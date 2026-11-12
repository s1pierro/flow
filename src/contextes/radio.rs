//! Le contexte Radio : une adresse de flux à saisir, lecture et arrêt, et les derniers
//! flux écoutés. Le volume de la piste Radio se règle dans le contexte Mixeur. Le lecteur (`radio::lecteur()`) et l'historique sont
//! partagés ; l'adresse saisie est propre à chaque fenêtre.

use crate::clavier::{self, Edition, Frappe};
use crate::contexte::{Commun, Contexte, Etat, Genre};
use crate::draw::Rect;
use crate::mixage::mixeur;
use crate::radio::{self, Etat as Lecture};
use crate::surface::{FenetreId, Placee};
use crate::ui::{id, sub_id, Id};
use crate::widgets::{Cx, Icon};

pub const CLE: &str = "radio";
pub const GENRE: Genre = Genre { cle: CLE, nom: "Radio", legende: "Écouter un flux en ligne", icone: Icon::Radio, ouvrir, applet: Some(applet) };

const RADIO: Id = id("radio");
const RADIO_CHAMP: Id = id("radio-adresse");

/// Deux flux pour essayer, tant que l'historique est vide.
const SUGGESTIONS: [&str; 2] = ["https://icecast.radiofrance.fr/fip-midfi.mp3", "https://stream.radioparadise.com/aac-128"];

#[derive(Clone, Default)]
pub struct Radio {
    adresse: Edition,
}

fn ouvrir(e: &Etat) -> Box<dyn Contexte> {
    Box::new(Radio { adresse: Edition::new(e.texte("adresse").unwrap_or("")) })
}

/// Joue un flux et le place en tête de l'historique.
fn jouer(c: &mut Commun, adresse: &str) {
    let adresse = adresse.trim().to_string();
    if adresse.is_empty() {
        return;
    }
    radio::lecteur().jouer(&adresse);
    if let Err(e) = c.historique.ajouter(&adresse) {
        crate::android::log(&format!("historique des radios non enregistré : {e}"));
    }
}

impl Contexte for Radio {
    fn cle(&self) -> &'static str {
        CLE
    }

    fn dupliquer(&self) -> Box<dyn Contexte> {
        Box::new(self.clone())
    }

    fn ecrire(&self, e: &mut Etat) {
        if !self.adresse.texte.is_empty() {
            e.mettre("adresse", &self.adresse.texte);
        }
    }

    /// La ligne d'état (le tampon) suit la lecture.
    fn anime(&self, _c: &Commun) -> bool {
        mixeur().actif() && !mixeur().en_pause()
    }

    /// Entrée lance le flux saisi.
    fn frappe(&mut self, f: FenetreId, cible: Id, frappe: Frappe, c: &mut Commun) -> bool {
        if cible != sub_id(RADIO_CHAMP, f as usize) {
            return false;
        }
        if self.adresse.appliquer(frappe) {
            c.clavier.fermer();
            let a = self.adresse.texte.clone();
            jouer(c, &a);
        }
        true
    }

    fn dessiner(&mut self, cx: &mut Cx, c: &mut Commun, p: &Placee) -> String {
        let r = p.contenu;
        let (col, h) = (cx.couleurs(), cx.commande());
        cx.dl.rrect(r, 0.0, col.surface);
        let (scroll, champ) = (sub_id(RADIO, p.id as usize), sub_id(RADIO_CHAMP, p.id as usize));
        let (etat, actif, en_cours) = {
            let l = radio::lecteur();
            (l.etat(), l.actif(), l.adresse.clone())
        };
        let mut b = cx.body(scroll, r, &c.scrolls);
        cx.section(&mut b, "FLUX");
        clavier::champ(cx, &mut b, champ, "Adresse du flux (http…)", &mut self.adresse, &mut c.clavier);
        // Lecture ou arrêt, et l'état à côté
        let y = b.y + cx.ecart();
        if cx.pastille(sub_id(scroll, 1), b.x, y, if actif { Icon::Stop } else { Icon::Lecture }, actif) {
            if actif {
                radio::lecteur().arreter();
            } else {
                let a = if self.adresse.texte.trim().is_empty() { c.historique.liste.first().cloned().unwrap_or_default() } else { self.adresse.texte.clone() };
                jouer(c, &a);
            }
        }
        let x = b.x + h + cx.pad();
        let (titre, detail) = match &etat {
            Lecture::Arret => ("Arrêt".to_string(), String::new()),
            Lecture::Connexion => ("Connexion…".to_string(), en_cours.as_deref().map(radio::courte).unwrap_or("").to_string()),
            Lecture::Lecture { .. } => ("Lecture".to_string(), etat.detail()),
            Lecture::Erreur(e) => ("Erreur".to_string(), e.clone()),
        };
        cx.clip(Some(Rect::new(x, y, (b.x + b.w - x).max(0.0), h).intersect(&b.rect())));
        cx.text_at(x, y + h * 0.32, &titre, 16.0, if matches!(etat, Lecture::Erreur(_)) { [0xE0, 0x3A, 0x3A, 255] } else { col.texte });
        cx.text_at(x, y + h * 0.75, &detail, 13.0, col.pale);
        cx.clip(Some(b.rect()));
        b.y = y + h + cx.ecart();
        // Les derniers flux : toucher pour écouter
        let (titre, liste) = if c.historique.liste.is_empty() { ("SUGGESTIONS", SUGGESTIONS.iter().map(|s| s.to_string()).collect()) } else { ("RÉCENTS", c.historique.liste.clone()) };
        cx.section(&mut b, titre);
        let mut choisi = None;
        for (i, a) in liste.iter().enumerate() {
            let joue = actif && en_cours.as_deref() == Some(a.as_str());
            if cx.row(&mut b, sub_id(scroll, 100 + i), radio::courte(a), None, joue) {
                choisi = Some(a.clone());
            }
        }
        if let Some(a) = choisi {
            self.adresse = Edition::new(&a);
            jouer(c, &a);
        }
        if let Some(f) = &c.historique.chemin {
            cx.paragraph(&mut b, &format!("Historique enregistré dans {}.", f.display()));
        }
        cx.body_end(b, &mut c.scrolls);
        match etat {
            Lecture::Lecture { .. } => {
                let t = mixeur().pistes().into_iter().find(|p| p.nom == radio::PISTE).map_or(0.0, |p| p.tampon);
                format!("tampon {:.0} %", t * 100.0)
            }
            Lecture::Connexion => "connexion".into(),
            Lecture::Erreur(_) => "erreur".into(),
            Lecture::Arret => "arrêt".into(),
        }
    }
}

// --- L'applet de la barre -------------------------------------------------------

/// L'applet Radio : dans la barre, le poste, le flux en cours et un bouton lecture ou
/// arrêt ; dans le volet, l'état, le volume de la piste Radio et les derniers flux.
struct AppletRadio;

fn applet() -> Box<dyn crate::applet::Applet> {
    Box::new(AppletRadio)
}

/// Ce que le bouton lecture relancerait : le dernier flux, ou le premier de l'historique.
fn a_relancer(c: &Commun) -> Option<String> {
    radio::lecteur().dernier.clone().or_else(|| c.historique.liste.first().cloned())
}

/// Le titre court montré dans la barre, s'il y a une lecture.
fn titre_barre() -> Option<(String, bool)> {
    let l = radio::lecteur();
    let a = l.adresse.clone()?;
    let erreur = matches!(l.etat(), Lecture::Erreur(_));
    Some((radio::titre(&a).to_string(), erreur))
}

impl crate::applet::Applet for AppletRadio {
    fn largeur(&mut self, cx: &mut Cx, c: &Commun, h: f32, compact: bool) -> f32 {
        let texte = titre_barre().filter(|_| !compact).map_or(0.0, |(t, _)| cx.measure(&t, 13.0).min(150.0 * cx.dp) + 4.0 * cx.dp);
        // En compact, le poste seul : le volet garde la lecture et l'arrêt
        let bouton = if !compact && (radio::lecteur().actif() || a_relancer(c).is_some()) { h } else { 0.0 };
        h * 0.9 + texte + bouton
    }

    fn barre(&mut self, cx: &mut Cx, c: &mut Commun, r: Rect, id: Id, compact: bool) {
        let (d, col) = (cx.dp, cx.couleurs());
        let ic = (r.h * 0.5).round();
        let actif = radio::lecteur().actif();
        // Le poste prend l'accent pendant la lecture
        cx.icon(Rect::new(r.x + (r.h * 0.9 - ic) / 2.0, r.cy() - ic / 2.0, ic, ic), Icon::Radio, if actif { col.accent } else { col.texte });
        let mut x = r.x + r.h * 0.9;
        let bouton = !compact && (actif || a_relancer(c).is_some());
        if let Some((t, erreur)) = titre_barre().filter(|_| !compact) {
            let fin = r.right() - if bouton { r.h } else { 0.0 };
            cx.clip(Some(Rect::new(x, r.y, (fin - x).max(0.0), r.h)));
            let t = crate::contextes::abreger(cx, &t, 13.0, (fin - x - 4.0 * d).max(0.0));
            cx.text_at(x, r.cy(), &t, 13.0, if erreur { [0xE0, 0x3A, 0x3A, 255] } else { col.texte });
            cx.clip(None);
            x = fin;
        }
        if bouton && cx.pastille_plate(sub_id(id, 1), Rect::new(x, r.y, r.h, r.h), if actif { Icon::Stop } else { Icon::Lecture }, false) {
            if actif {
                radio::lecteur().arreter();
            } else if let Some(a) = a_relancer(c) {
                jouer(c, &a);
            }
        }
    }

    fn hauteur_volet(&mut self, cx: &mut Cx, c: &Commun, _w: f32) -> f32 {
        let d = cx.dp;
        let ligne = (cx.theme.ligne * d).round();
        let n = c.historique.liste.len().clamp(1, 6) as f32;
        let section = (20.0 + 18.0 * cx.theme.texte) * d;
        8.0 * d + 2.0 * section + cx.commande() + 2.0 * cx.ecart() + (64.0 * d * cx.theme.texte.max(1.0)).round() + n * ligne + cx.commande() + 2.0 * (cx.theme.marge * d).round() + 20.0 * d
    }

    fn volet(&mut self, cx: &mut Cx, c: &mut Commun, r: Rect, id: Id) -> bool {
        let (col, h) = (cx.couleurs(), cx.commande());
        let scroll = sub_id(id, 0);
        let (etat, actif, en_cours) = {
            let l = radio::lecteur();
            (l.etat(), l.actif(), l.adresse.clone())
        };
        let mut b = cx.body(scroll, r, &c.scrolls);
        cx.section(&mut b, "RADIO");
        let y = b.y + cx.ecart();
        let relance = a_relancer(c);
        if cx.pastille(sub_id(scroll, 1), b.x, y, if actif { Icon::Stop } else { Icon::Lecture }, actif) {
            if actif {
                radio::lecteur().arreter();
            } else if let Some(a) = &relance {
                jouer(c, a);
            }
        }
        let x = b.x + h + cx.pad();
        let (titre, detail) = match &etat {
            Lecture::Arret => ("Arrêt".to_string(), relance.as_deref().map(radio::courte).unwrap_or("aucun flux écouté").to_string()),
            Lecture::Connexion => ("Connexion…".to_string(), en_cours.as_deref().map(radio::courte).unwrap_or("").to_string()),
            Lecture::Lecture { .. } => (en_cours.as_deref().map(radio::titre).unwrap_or("Lecture").to_string(), etat.detail()),
            Lecture::Erreur(e) => ("Erreur".to_string(), e.clone()),
        };
        cx.clip(Some(Rect::new(x, y, (b.x + b.w - x).max(0.0), h).intersect(&b.rect())));
        cx.text_at(x, y + h * 0.32, &titre, 16.0, if matches!(etat, Lecture::Erreur(_)) { [0xE0, 0x3A, 0x3A, 255] } else { col.texte });
        cx.text_at(x, y + h * 0.75, &detail, 13.0, col.pale);
        cx.clip(Some(b.rect()));
        b.y = y + h + cx.ecart();
        // Le volume de la piste Radio, comme dans le Mixeur
        let mut reglage = mixeur().reglage(radio::PISTE);
        let avant = reglage.gain;
        let shown = format!("{:.0} %", reglage.gain * 100.0);
        cx.slider(&mut b, sub_id(scroll, 2), "Volume", &shown, &mut reglage.gain, 0.0, 1.0);
        reglage.gain = (reglage.gain * 20.0).round() / 20.0;
        if reglage.gain != avant {
            mixeur().regler(radio::PISTE, reglage);
        }
        let (titre, liste) = if c.historique.liste.is_empty() { ("SUGGESTIONS", SUGGESTIONS.iter().map(|s| s.to_string()).collect::<Vec<_>>()) } else { ("RÉCENTS", c.historique.liste.iter().take(6).cloned().collect()) };
        cx.section(&mut b, titre);
        let mut choisi = None;
        for (i, a) in liste.iter().enumerate() {
            let joue = actif && en_cours.as_deref() == Some(a.as_str());
            if cx.row(&mut b, sub_id(scroll, 100 + i), radio::courte(a), None, joue) {
                choisi = Some(a.clone());
            }
        }
        if let Some(a) = choisi {
            jouer(c, &a);
        }
        let ouvrir = cx.button(&mut b, sub_id(scroll, 3), "Ouvrir la Radio");
        if ouvrir {
            c.demandes.push(crate::contexte::Demande::Montrer { cle: CLE });
        }
        cx.body_end(b, &mut c.scrolls);
        ouvrir
    }
}
