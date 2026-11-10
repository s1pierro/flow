//! Le contexte Orchestre : la face visible de l'orchestrateur (`crate::orchestre`).
//! Il montre l'état de ses tâches (empreintes du stockage, batterie), le recensement
//! des contextes et le journal, et en règle le fonctionnement. Il n'a pas d'état à
//! lui : tout est dans `Commun::orchestre`, partagé par toutes les fenêtres.

use crate::contexte::{Commun, Contexte, Etat, EtatEmpreintes, Genre};
use crate::contextes::{ligne_fichier, Puce, GENRES};
use crate::disque;
use crate::draw::Rect;
use crate::orchestre::{duree, HISTOIRE};
use crate::surface::Placee;
use crate::ui::{id, sub_id, Id};
use crate::widgets::{Body, Cx, Icon};
use std::sync::atomic::Ordering;

pub const CLE: &str = "orchestre";
pub const GENRE: Genre = Genre { cle: CLE, nom: "Orchestre", legende: "Empreintes, batterie, contextes : l'état de Flow", icone: Icon::Orchestre, ouvrir };

const ORCHESTRE: Id = id("orchestre");
/// La durée montrée par la courbe de la batterie.
const COURBE: u64 = 24 * 3600;
/// Lignes du journal montrées.
const JOURNAL: usize = 40;
const ROUGE: [u8; 4] = [0xE0, 0x3A, 0x3A, 255];

#[derive(Clone, Default)]
pub struct Tableau;

fn ouvrir(_e: &Etat) -> Box<dyn Contexte> {
    Box::new(Tableau)
}

impl Contexte for Tableau {
    fn cle(&self) -> &'static str {
        CLE
    }

    fn dupliquer(&self) -> Box<dyn Contexte> {
        Box::new(Tableau)
    }

    /// La progression d'une analyse se suit en direct.
    fn anime(&self, c: &Commun) -> bool {
        c.analyse.as_ref().is_some_and(|a| a.en_cours())
    }

    fn dessiner(&mut self, cx: &mut Cx, c: &mut Commun, p: &Placee) -> String {
        let r = p.contenu;
        cx.dl.rrect(r, 0.0, cx.couleurs().surface);
        let scroll = sub_id(ORCHESTRE, p.id as usize);
        let mut b = cx.body(scroll, r, &c.scrolls);
        let maintenant = disque::maintenant();
        let etat = empreintes(cx, &mut b, scroll, c, maintenant);
        batterie(cx, &mut b, scroll, c, maintenant);
        recensement(cx, &mut b, scroll, c);
        journal(cx, &mut b, c);
        if let Some(d) = &c.dossier {
            cx.section(&mut b, "FICHIERS");
            cx.paragraph(&mut b, &format!("Réglages : {}, modifiable à la main. Historique de la batterie : {}.", d.join(crate::orchestre::FICHIER).display(), d.join(crate::orchestre::BATTERIE).display()));
        }
        cx.body_end(b, &mut c.scrolls);
        match c.orchestre.batterie() {
            Some((n, charge)) => format!("{etat} · {n} %{}", if charge { " en charge" } else { "" }),
            None => etat,
        }
    }
}

/// Une ligne d'état : icône, nom, valeur à droite.
fn info(cx: &mut Cx, b: &mut Body, id: Id, icone: Icon, nom: &str, valeur: &str) {
    ligne_fichier(cx, b, id, Puce::Icone(icone), nom, valeur, false);
}

/// La tâche des empreintes : état, réglages, et l'analyse à la demande. Renvoie un
/// résumé pour la ligne d'état.
fn empreintes(cx: &mut Cx, b: &mut Body, scroll: Id, c: &mut Commun, maintenant: u64) -> String {
    cx.section(b, "EMPREINTES DU STOCKAGE");
    let etat = c.etat_empreintes();
    let (texte, court) = match etat {
        EtatEmpreintes::SansStockage => ("pas d'accès au stockage".to_string(), "sans stockage".to_string()),
        EtatEmpreintes::EnCours => {
            let n = c.analyse.as_ref().map_or(0, |a| a.progres.fichiers.load(Ordering::Relaxed));
            (format!("analyse · {n} fichiers"), "analyse".to_string())
        }
        EtatEmpreintes::AJour { .. } => ("à jour".to_string(), "empreintes à jour".to_string()),
        EtatEmpreintes::Manuelle => ("à la demande".to_string(), "empreintes à la demande".to_string()),
        EtatEmpreintes::Reportee => ("reportée : batterie basse".to_string(), "analyse reportée".to_string()),
        EtatEmpreintes::Due => ("à refaire".to_string(), "empreintes à refaire".to_string()),
    };
    info(cx, b, sub_id(scroll, 1), Icon::Anneau, "État", &texte);
    let age = c.age_empreintes().map_or("aucune".to_string(), |a| format!("il y a {}", duree(a)));
    info(cx, b, sub_id(scroll, 2), Icon::Fichier, "Dernière synthèse", &age);
    let prochaine = match etat {
        EtatEmpreintes::AJour { dans } => format!("dans {}", duree(dans)),
        EtatEmpreintes::Manuelle => "à la demande".into(),
        EtatEmpreintes::Reportee => "à la recharge".into(),
        EtatEmpreintes::EnCours => "en cours".into(),
        _ => "—".into(),
    };
    info(cx, b, sub_id(scroll, 3), Icon::Haut, "Prochaine", &prochaine);
    let r = &mut c.orchestre.reglages;
    cx.toggle(b, sub_id(scroll, 4), "Les refaire de soi-même", &mut r.empreintes_auto);
    let shown = duree(r.periode());
    cx.slider(b, sub_id(scroll, 5), "Période", &shown, &mut r.empreintes_heures, 1.0, 168.0);
    r.empreintes_heures = r.empreintes_heures.round();
    cx.toggle(b, sub_id(scroll, 6), "Attendre si la batterie est basse", &mut r.economie);
    if cx.button(b, sub_id(scroll, 7), "Analyser maintenant") && etat != EtatEmpreintes::EnCours && c.racine.is_some() {
        c.orchestre.noter(maintenant, "analyse du stockage demandée");
        c.reanalyser();
    }
    court
}

/// La batterie : courbe du dernier jour, niveau, débit, autonomie, réglages.
fn batterie(cx: &mut Cx, b: &mut Body, scroll: Id, c: &mut Commun, maintenant: u64) {
    cx.section(b, "BATTERIE");
    let o = &c.orchestre;
    let Some((niveau, charge)) = o.batterie() else {
        cx.paragraph(b, "Pas encore de relevé (la batterie ne se lit que sur Android).");
        reglages_batterie(cx, b, scroll, c);
        return;
    };
    courbe(cx, b, c, maintenant);
    let o = &c.orchestre;
    let n = format!("{niveau} %{}", if charge { " · en charge" } else if o.basse() { " · basse" } else { "" });
    info(cx, b, sub_id(scroll, 20), Icon::Batterie { niveau, charge }, "Niveau", &n);
    let debit = o.debit().map_or("pas assez de recul".to_string(), |d| format!("{d:+.1} %/h").replace('.', ","));
    info(cx, b, sub_id(scroll, 21), Icon::Sliders, "Débit", &debit);
    let autonomie = match (o.autonomie(), charge) {
        (Some(h), true) => format!("pleine dans {}", duree((h * 3600.0) as u64)),
        (Some(h), false) => format!("vide dans {}", duree((h * 3600.0) as u64)),
        (None, _) => "—".to_string(),
    };
    info(cx, b, sub_id(scroll, 22), Icon::Haut, "Autonomie", &autonomie);
    let releves = format!("{} sur {}", o.releves.len(), duree(HISTOIRE));
    info(cx, b, sub_id(scroll, 23), Icon::List, "Relevés", &releves);
    reglages_batterie(cx, b, scroll, c);
}

fn reglages_batterie(cx: &mut Cx, b: &mut Body, scroll: Id, c: &mut Commun) {
    let r = &mut c.orchestre.reglages;
    let shown = format!("{:.0} min", r.batterie_minutes);
    cx.slider(b, sub_id(scroll, 24), "Intervalle des relevés", &shown, &mut r.batterie_minutes, 1.0, 30.0);
    r.batterie_minutes = r.batterie_minutes.round();
    let shown = format!("{:.0} %", r.seuil_batterie);
    cx.slider(b, sub_id(scroll, 25), "Seuil bas", &shown, &mut r.seuil_batterie, 5.0, 50.0);
    r.seuil_batterie = r.seuil_batterie.round();
}

/// La courbe du niveau sur le dernier jour : en paliers, l'accent pendant la charge,
/// le seuil bas en rouge.
fn courbe(cx: &mut Cx, b: &mut Body, c: &Commun, maintenant: u64) {
    let (d, col) = (cx.dp, cx.couleurs());
    let r = Rect::new(b.x, b.y + cx.ecart(), b.w, (96.0 * d).round());
    b.y = r.bottom() + 18.0 * d;
    let vue = b.rect();
    if r.bottom() + 18.0 * d < vue.y || r.y > vue.bottom() {
        return;
    }
    cx.dl.rrect(r, cx.coin().min(8.0 * d), col.puce);
    let debut = maintenant.saturating_sub(COURBE);
    let x = |t: u64| r.x + (t.clamp(debut, maintenant) - debut) as f32 / COURBE as f32 * r.w;
    let y = |n: u8| r.bottom() - 4.0 * d - n as f32 / 100.0 * (r.h - 8.0 * d);
    let seuil = r.bottom() - 4.0 * d - c.orchestre.reglages.seuil_batterie / 100.0 * (r.h - 8.0 * d);
    cx.dl.line([r.x, seuil], [r.right(), seuil], d.max(1.0), ROUGE);
    let pts: Vec<_> = c.orchestre.releves.iter().filter(|p| p.date <= maintenant).collect();
    let epais = 2.0 * d;
    for (i, p) in pts.iter().enumerate() {
        let fin = pts.get(i + 1).map_or(maintenant, |s| s.date);
        if fin < debut {
            continue;
        }
        let teinte = if p.charge { col.accent } else { col.texte };
        let (x0, x1, y0) = (x(p.date), x(fin), y(p.niveau));
        cx.dl.line([x0, y0], [x1.max(x0 + 1.0), y0], epais, teinte);
        if let Some(s) = pts.get(i + 1) {
            cx.dl.line([x1, y0], [x1, y(s.niveau)], epais, teinte);
        }
    }
    let ly = r.bottom() + 9.0 * d;
    cx.mono_at(r.x, ly, "-24 h", 11.0, col.pale);
    let w = cx.mesure_mono("maintenant", 11.0);
    cx.mono_at(r.right() - w, ly, "maintenant", 11.0, col.pale);
}

/// Les contextes disponibles, avec leurs instances ouvertes et leurs documents.
fn recensement(cx: &mut Cx, b: &mut Body, scroll: Id, c: &Commun) {
    let o = &c.orchestre;
    cx.section(b, &format!("CONTEXTES · {} DISPONIBLES, {} OUVERTS", GENRES.len(), o.instances.len()));
    for (i, g) in GENRES.iter().enumerate() {
        let n = o.ouvertes(g.cle);
        let ouvertes = match n {
            0 => "—".to_string(),
            1 => "1 fenêtre".to_string(),
            n => format!("{n} fenêtres"),
        };
        ligne_fichier(cx, b, sub_id(scroll, 100 + i), Puce::Icone(g.icone), g.nom, &ouvertes, n > 0);
    }
    let docs: Vec<_> = o.instances.iter().filter_map(|i| Some((i.fenetre, i.cle, i.document.as_ref()?))).collect();
    if !docs.is_empty() {
        cx.section(b, "DOCUMENTS OUVERTS");
        for (k, (f, cle, doc)) in docs.into_iter().enumerate() {
            let nom = doc.file_name().map_or_else(|| doc.display().to_string(), |n| n.to_string_lossy().into_owned());
            let genre = GENRES.iter().find(|g| g.cle == cle);
            let icone = genre.map_or(Icon::Fichier, |g| g.icone);
            ligne_fichier(cx, b, sub_id(scroll, 200 + k), Puce::Icone(icone), &nom, &format!("fenêtre {f}"), false);
        }
    }
}

/// Le journal, le plus récent en haut.
fn journal(cx: &mut Cx, b: &mut Body, c: &Commun) {
    cx.section(b, "JOURNAL");
    if c.orchestre.journal.is_empty() {
        cx.paragraph(b, "Rien encore depuis le lancement.");
        return;
    }
    for (t, texte) in c.orchestre.journal.iter().rev().take(JOURNAL) {
        let (h, m) = crate::android::heure_de(*t);
        cx.paragraph(b, &format!("{h:02}:{m:02}  {texte}"));
    }
}
