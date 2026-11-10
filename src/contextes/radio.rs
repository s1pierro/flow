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
pub const GENRE: Genre = Genre { cle: CLE, nom: "Radio", legende: "Écouter un flux en ligne", icone: Icon::Radio, ouvrir };

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
