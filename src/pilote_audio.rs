//! Les commandes audio du pilote (`pilote.rs`) : la radio de la Boombox, son
//! magnétophone et le mixeur. Elles agissent sur les lecteurs et le mixeur partagés :
//! aucune fenêtre n'a besoin d'être ouverte.
//!
//! - `radio` : l'état (lecture, veille, enregistrement, tampon) ;
//! - `radio liste` : l'historique, numéroté ;
//! - `radio jouer <n | adresse | mot>` : le n-ième de l'historique, une adresse, ou le
//!   premier de l'historique qui contient ce mot (« nova ») ;
//! - `radio derniere` : relance le dernier flux (ou le premier de l'historique) ;
//! - `radio arreter` ;
//! - `enregistrer [maintenant | 30s | 1min | 2min | 5min | tout | <secondes>]` : commence
//!   un enregistrement, en partant d'autant dans le passé (le tampon) ;
//! - `enregistrer arreter` ;
//! - `mixeur` : la sortie et chaque piste ;
//! - `mixeur pause | lecture` ;
//! - `mixeur <piste|sortie> gain <0..1 | n %>`, `balance <-1..1>`, `muet|solo|prioritaire
//!   oui|non`, `eq oui|non`, `eq <bande 1..5> <dB>`.

use crate::contexte::Commun;
use crate::contextes::boombox;
use crate::magneto::{self, magneto};
use crate::mixage::{commande, mixeur, Reglage};
use crate::radio::{self, Etat};

/// Exécute la commande si elle est audio ; sinon rien.
pub fn executer(c: &mut Commun, m: &[String]) -> Option<Result<String, String>> {
    match m[0].as_str() {
        "radio" => Some(radio_(c, m)),
        "enregistrer" => Some(enregistrer(c, m)),
        "mixeur" => Some(mixeur_(m)),
        _ => None,
    }
}

fn etat_radio() -> String {
    let l = radio::lecteur();
    let mut r = match (l.etat(), &l.adresse, &l.veille) {
        (Etat::Lecture { .. }, Some(a), _) => format!("lecture {a}\n{}", l.etat().detail()),
        (Etat::Connexion, Some(a), _) => format!("connexion {a}"),
        (Etat::Erreur(e), a, _) => format!("erreur {} : {e}", a.as_deref().unwrap_or("")),
        (_, _, Some(v)) => format!("veille {v}"),
        _ => "arret".to_string(),
    };
    drop(l);
    if let Some(u) = boombox::source() {
        r.push_str(&format!("\ntampon {}", magneto::duree(magneto().passe_s(&u))));
    }
    let mg = magneto();
    if let Some(p) = &mg.prise {
        r.push_str(&format!("\nenregistrement {} ({})", p.chemin.display(), magneto::duree(p.duree_s())));
    }
    if let Some(e) = &mg.erreur {
        r.push_str(&format!("\nerreur magnéto : {e}"));
    }
    r
}

/// Le flux désigné par `q` : un numéro de l'historique (à partir de 1), une adresse,
/// ou le premier de l'historique qui contient le mot.
fn designer(c: &Commun, q: &str) -> Result<String, String> {
    let h = &c.historique.liste;
    if let Ok(n) = q.parse::<usize>() {
        return h.get(n.wrapping_sub(1)).cloned().ok_or(format!("pas de n° {n} dans l'historique ({} flux)", h.len()));
    }
    if q.contains("://") {
        return Ok(q.to_string());
    }
    let mot = q.to_lowercase();
    h.iter()
        .find(|a| a.to_lowercase().contains(&mot) || radio::titre(a).to_lowercase().contains(&mot))
        .cloned()
        .ok_or(format!("aucun flux de l'historique ne contient « {q} » (radio liste)"))
}

fn radio_(c: &mut Commun, m: &[String]) -> Result<String, String> {
    match m.get(1).map(|s| s.as_str()) {
        None | Some("etat") => Ok(etat_radio()),
        Some("liste") => Ok(c.historique.liste.iter().enumerate().map(|(i, a)| format!("{} {}", i + 1, a)).collect::<Vec<_>>().join("\n")),
        Some("jouer") => {
            let q = m.get(2..).map(|r| r.join(" ")).filter(|q| !q.is_empty()).ok_or("radio jouer <n | adresse | mot>")?;
            let a = designer(c, &q)?;
            boombox::jouer(c, &a);
            Ok(format!("jouer {a}"))
        }
        Some("derniere") => {
            let a = radio::lecteur().dernier.clone().or_else(|| c.historique.liste.first().cloned()).ok_or("aucun flux connu")?;
            boombox::jouer(c, &a);
            Ok(format!("jouer {a}"))
        }
        Some("arreter") | Some("stop") => {
            radio::lecteur().arreter();
            Ok("arret".into())
        }
        Some(x) => Err(format!("radio : « {x} » ? (etat, liste, jouer, derniere, arreter)")),
    }
}

/// Une durée de départ dans le passé : « maintenant », « 30s », « 1min », « 2min30 »,
/// « tout », ou des secondes.
fn depart(q: &str, tampon: f32) -> Result<f32, String> {
    let q = q.trim().to_lowercase();
    match q.as_str() {
        "maintenant" | "0" => return Ok(0.0),
        "tout" => return Ok(tampon),
        _ => {}
    }
    if let Ok(s) = q.parse::<f32>() {
        return Ok(s.max(0.0));
    }
    let (min, reste) = match q.split_once("min") {
        Some((a, b)) => (a.trim().parse::<f32>().map_err(|_| format!("durée illisible : {q}"))?, b),
        None => (0.0, q.as_str()),
    };
    let sec = reste.trim().trim_end_matches('s').trim();
    let sec = if sec.is_empty() { 0.0 } else { sec.parse::<f32>().map_err(|_| format!("durée illisible : {q}"))? };
    Ok(min * 60.0 + sec)
}

fn enregistrer(c: &mut Commun, m: &[String]) -> Result<String, String> {
    match m.get(1).map(|s| s.as_str()) {
        Some("arreter") | Some("stop") => match magneto().arreter() {
            Some((p, s)) => Ok(format!("enregistré {} ({})", p.display(), magneto::duree(s))),
            None => Err("aucun enregistrement en cours".into()),
        },
        q => {
            if magneto().prise.is_some() {
                return Err("un enregistrement est déjà en cours (enregistrer arreter)".into());
            }
            let tampon = boombox::source().map_or(0.0, |u| magneto().passe_s(&u));
            let passe = depart(q.unwrap_or("maintenant"), tampon)?;
            let p = boombox::commencer_enregistrement(c, passe.min(tampon))?;
            Ok(format!("enregistrement {} (depuis {} dans le passé ; tampon {})", p.display(), magneto::duree(passe.min(tampon)), magneto::duree(tampon)))
        }
    }
}

fn oui(v: Option<&String>) -> Result<bool, String> {
    match v.map(|s| s.as_str()) {
        Some("oui" | "1" | "vrai" | "on") => Ok(true),
        Some("non" | "0" | "faux" | "off") => Ok(false),
        _ => Err("oui ou non".into()),
    }
}

fn decrire(nom: &str, r: &Reglage, niveau: Option<[f32; 2]>) -> String {
    let b = |x: bool| if x { "oui" } else { "non" };
    let eq: Vec<String> = r.eq.bandes.iter().map(|x| format!("{:+.1}", x.gain)).collect();
    let son = niveau.map_or("muette".to_string(), |n| format!("crête {:.2} {:.2}", n[0], n[1]));
    format!("piste {nom} gain {:.2} balance {:.2} muet {} solo {} prioritaire {} eq {} [{}] {son}", r.gain, r.balance, b(r.muet), b(r.solo), b(r.prioritaire), b(r.eq.actif), eq.join(" "))
}

fn mixeur_(m: &[String]) -> Result<String, String> {
    let mx = mixeur();
    let Some(cible) = m.get(1) else {
        let e = mx.egaliseur();
        let eq: Vec<String> = e.bandes.iter().map(|x| format!("{:+.1}", x.gain)).collect();
        let mut r = format!("sortie {} Hz pause {} eq {} [{}]", mx.frequence(), if mx.en_pause() { "oui" } else { "non" }, if e.actif { "oui" } else { "non" }, eq.join(" "));
        for p in mx.pistes() {
            r.push('\n');
            r.push_str(&decrire(&p.nom, &p.reglage, p.niveau));
        }
        return Ok(r);
    };
    match cible.as_str() {
        "pause" => {
            mx.commander(commande::SUSPENDRE);
            return Ok("pause".into());
        }
        "lecture" | "reprendre" => {
            mx.commander(commande::JOUER);
            return Ok("lecture".into());
        }
        _ => {}
    }
    let prop = m.get(2).ok_or("mixeur <piste|sortie> <gain|balance|muet|solo|prioritaire|eq> <valeur>")?;
    let nombre = |i: usize| -> Result<f32, String> {
        let v = m.get(i).ok_or("valeur manquante")?;
        let pc = v.ends_with('%') || m.get(i + 1).is_some_and(|x| x == "%");
        let x = v.trim_end_matches('%').replace(',', ".").parse::<f32>().map_err(|_| format!("« {v} » n'est pas un nombre"))?;
        Ok(if pc { x / 100.0 } else { x })
    };
    // L'égaliseur, de la sortie ou d'une piste
    let regler_eq = |e: &mut crate::egaliseur::Egaliseur| -> Result<(), String> {
        match m.get(3).map(|s| s.as_str()) {
            Some("oui" | "non" | "1" | "0" | "on" | "off") => e.actif = oui(m.get(3))?,
            _ => {
                let b = m.get(3).and_then(|s| s.parse::<usize>().ok()).filter(|b| (1..=e.bandes.len()).contains(b)).ok_or(format!("eq oui|non, ou eq <bande 1..{}> <dB>", e.bandes.len()))?;
                e.bandes[b - 1].gain = nombre(4)?;
                e.actif = true;
            }
        }
        Ok(())
    };
    if cible == "sortie" {
        if prop != "eq" {
            return Err("la sortie n'a que son égaliseur : mixeur sortie eq …".into());
        }
        let mut e = mx.egaliseur();
        regler_eq(&mut e)?;
        mx.regler_egaliseur(&e);
        return Ok("sortie eq réglé".into());
    }
    // La piste : son nom exact, ou le premier qui commence par ce mot
    let pistes = mx.pistes();
    let nom = pistes
        .iter()
        .find(|p| p.nom.eq_ignore_ascii_case(cible))
        .or_else(|| pistes.iter().find(|p| p.nom.to_lowercase().starts_with(&cible.to_lowercase())))
        .map(|p| p.nom.clone())
        .ok_or(format!("piste inconnue : {cible} ({})", pistes.iter().map(|p| p.nom.as_str()).collect::<Vec<_>>().join(", ")))?;
    let mut r = mx.reglage(&nom);
    match prop.as_str() {
        "gain" | "volume" => r.gain = nombre(3)?.clamp(0.0, 1.0),
        "balance" => r.balance = nombre(3)?.clamp(-1.0, 1.0),
        "muet" => r.muet = oui(m.get(3))?,
        "solo" => r.solo = oui(m.get(3))?,
        "prioritaire" => r.prioritaire = oui(m.get(3))?,
        "eq" => regler_eq(&mut r.eq)?,
        x => return Err(format!("réglage inconnu : {x} (gain, balance, muet, solo, prioritaire, eq)")),
    }
    mx.regler(&nom, r);
    Ok(decrire(&nom, &mx.reglage(&nom), None))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn les_departs_se_lisent() {
        assert_eq!(depart("maintenant", 300.0), Ok(0.0));
        assert_eq!(depart("30s", 300.0), Ok(30.0));
        assert_eq!(depart("1min", 300.0), Ok(60.0));
        assert_eq!(depart("2min30", 300.0), Ok(150.0));
        assert_eq!(depart("tout", 212.0), Ok(212.0));
        assert_eq!(depart("45", 300.0), Ok(45.0));
        assert!(depart("bientôt", 300.0).is_err());
    }
}
