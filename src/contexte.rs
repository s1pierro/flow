//! Les contextes : les sous-applications que montrent les fenêtres de la surface.
//!
//! Chaque fenêtre porte une **instance** de contexte, avec son propre état (son
//! document, son dossier, sa caméra…) : deux fenêtres du même contexte travaillent
//! chacune sur le leur. Ce qui doit être partagé entre toutes (l'apparence, le
//! clavier, l'analyse du stockage, l'historique des radios…) est dans `Commun`.
//!
//! Un contexte se déclare par un `Genre` (sa clé, son nom, son icône et sa fonction
//! d'ouverture) dans la liste `contextes::GENRES`, et son instance implémente le trait
//! `Contexte`. L'état d'une instance s'enregistre en paires `clé = valeur` (`Etat`)
//! dans le fichier de l'espace de travail, et la relit à l'ouverture.
//!
//! Le guide d'intégration est dans `docs/contextes.md`.

use crate::clavier::{Clavier, Frappe};
use crate::disque::Analyse;
use crate::draw::{Color, Rect};
use crate::math::Mat4;
use crate::radio::Historique;
use crate::surface::{FenetreId, Placee};
use crate::ui::{Gesture, Id};
use crate::widgets::{Cx, Icon, Scrolls, Theme};
use std::path::PathBuf;

/// Une sorte de contexte, telle qu'elle apparaît dans le menu des fenêtres.
pub struct Genre {
    /// La clé, stable : c'est elle que retient le fichier de l'espace de travail.
    /// Minuscules, chiffres, `-` et `_`.
    pub cle: &'static str,
    /// Le nom affiché dans la barre de la fenêtre et le menu.
    pub nom: &'static str,
    pub icone: Icon,
    /// Crée une instance à partir de son état enregistré (vide pour une fenêtre
    /// nouvelle). Les valeurs absentes ou illisibles prennent leur valeur par défaut.
    pub ouvrir: fn(&Etat) -> Box<dyn Contexte>,
}

/// L'instance d'un contexte, dans une fenêtre.
///
/// Seules `cle`, `dessiner` et `dupliquer` sont obligatoires. `Commun` est passé à
/// chaque appel : une instance ne garde pas de référence vers lui.
pub trait Contexte {
    /// La clé de son `Genre`.
    fn cle(&self) -> &'static str;

    /// Dessine le contenu de la fenêtre dans `p.contenu` et renvoie la ligne d'état,
    /// affichée dans la barre de la fenêtre. Les identifiants d'interface se dérivent
    /// de `p.id` (avec `sub_id`), pour ne pas se mêler à ceux d'une autre fenêtre.
    fn dessiner(&mut self, cx: &mut Cx, c: &mut Commun, p: &Placee) -> String;

    /// Une copie pour la fenêtre née d'un partage. Un contexte à document peut
    /// renvoyer une instance vide plutôt que d'ouvrir deux fois le même fichier.
    fn dupliquer(&self) -> Box<dyn Contexte>;

    /// Écrit dans `e` ce qu'il faut pour rouvrir l'instance au prochain lancement.
    fn ecrire(&self, _e: &mut Etat) {}

    /// Avance le temps, avant le dessin (animations).
    fn avancer(&mut self, _dt: f32, _c: &Commun) {}

    /// Vrai s'il faut redessiner sans attendre d'événement.
    fn anime(&self, _c: &Commun) -> bool {
        false
    }

    /// Un geste (orbite, déplacement, zoom) commencé dans la fenêtre.
    fn geste(&mut self, _g: &Gesture, _p: &Placee, _c: &Commun) {}

    /// La vue 3D à dessiner sous l'interface, dans le contenu de la fenêtre.
    fn vue3d(&self, _p: &Placee, _c: &Commun) -> Option<Vue3d> {
        None
    }

    /// Une frappe du clavier virtuel pour le champ `cible`. Renvoie vrai si le champ
    /// est à cette instance (la frappe ne va alors à personne d'autre).
    fn frappe(&mut self, _f: FenetreId, _cible: Id, _frappe: Frappe, _c: &mut Commun) -> bool {
        false
    }

    /// Retour dans l'application : ce qui est lu sur le disque a pu changer.
    fn retour(&mut self, _c: &Commun) {}
}

/// Une vue 3D : son rectangle à l'écran et sa matrice.
#[derive(Clone, Copy)]
pub struct Vue3d {
    pub rect: Rect,
    pub mvp: Mat4,
}

/// Ce que le moteur de rendu doit dessiner : un cube dans chaque vue 3D.
#[derive(Clone)]
pub struct Scene {
    pub workspace: Rect,
    pub vues: Vec<Vue3d>,
    /// Le fond de l'écran, que la 3D laisse voir.
    pub fond: Color,
    pub color: [f32; 3],
    pub tint: f32,
}

/// Les couleurs du cube, communes à la vue Couleurs et au panneau de gauche.
pub const COULEURS: [(&str, Color); 14] = [
    ("Indigo", [0x4A, 0x45, 0xE0, 255]),
    ("Bleu", [0x3B, 0x82, 0xF6, 255]),
    ("Ciel", [0x38, 0xBD, 0xF8, 255]),
    ("Turquoise", [0x14, 0xB8, 0xA6, 255]),
    ("Vert", [0x22, 0xC5, 0x5E, 255]),
    ("Olive", [0x84, 0xA9, 0x1C, 255]),
    ("Jaune", [0xEA, 0xB3, 0x08, 255]),
    ("Orange", [0xF9, 0x73, 0x16, 255]),
    ("Rouille", [0xB9, 0x4A, 0x18, 255]),
    ("Rouge", [0xEF, 0x44, 0x44, 255]),
    ("Rose", [0xEC, 0x48, 0x99, 255]),
    ("Violet", [0xA8, 0x55, 0xF7, 255]),
    ("Ardoise", [0x64, 0x74, 0x8B, 255]),
    ("Sable", [0xC8, 0xB0, 0x8A, 255]),
];

/// Les réglages de la 3D, communs à toutes les fenêtres Cube.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Atelier {
    pub couleur: usize,
    pub rotation: bool,
    /// Degrés par seconde.
    pub vitesse: f32,
    /// Champ de vision, en degrés.
    pub champ: f32,
    pub taille: f32,
    pub faces: bool,
    /// Augmente pour demander à toutes les caméras de revenir à leur position de départ.
    pub recadrage: u32,
}

impl Default for Atelier {
    fn default() -> Atelier {
        Atelier { couleur: 0, rotation: false, vitesse: 30.0, champ: 45.0, taille: 1.0, faces: true, recadrage: 0 }
    }
}

impl Atelier {
    pub fn ecrire(&self, e: &mut Etat) {
        e.mettre("cube_couleur", COULEURS[self.couleur.min(COULEURS.len() - 1)].0.to_lowercase());
        e.mettre_oui("cube_rotation", self.rotation);
        e.mettre("cube_vitesse", format!("{:.0}", self.vitesse));
        e.mettre("cube_champ", format!("{:.0}", self.champ));
        e.mettre("cube_taille", format!("{:.2}", self.taille));
        e.mettre_oui("cube_faces", self.faces);
    }

    pub fn lire(e: &Etat) -> Atelier {
        let d = Atelier::default();
        Atelier {
            couleur: e.texte("cube_couleur").and_then(|v| COULEURS.iter().position(|c| c.0.eq_ignore_ascii_case(v))).unwrap_or(d.couleur),
            rotation: e.oui("cube_rotation").unwrap_or(d.rotation),
            vitesse: e.nombre("cube_vitesse").map_or(d.vitesse, |v| v.clamp(5.0, 180.0)),
            champ: e.nombre("cube_champ").map_or(d.champ, |v| v.clamp(25.0, 90.0)),
            taille: e.nombre("cube_taille").map_or(d.taille, |v| v.clamp(0.4, 2.0)),
            faces: e.oui("cube_faces").unwrap_or(d.faces),
            recadrage: 0,
        }
    }
}

/// Ce que partagent toutes les fenêtres.
pub struct Commun {
    /// L'apparence de l'interface, réglée dans la vue Paramètres.
    pub theme: Theme,
    /// Le clavier virtuel : un seul, pour le champ qui a le focus.
    pub clavier: Clavier,
    /// Les positions de défilement, par identifiant de corps.
    pub scrolls: Scrolls,
    /// La racine du stockage partagé, donnée par Android.
    pub racine: Option<PathBuf>,
    /// L'analyse du stockage, une pour toutes les fenêtres (Disque, Fichiers).
    pub analyse: Option<Analyse>,
    /// Les derniers flux radio écoutés.
    pub historique: Historique,
    /// Où sont enregistrés les réglages et l'espace de travail, pour l'indiquer.
    pub fichier_reglages: Option<String>,
    pub fichier_espace: Option<String>,
    pub atelier: Atelier,
}

impl Default for Commun {
    fn default() -> Commun {
        Commun {
            theme: Theme::default(),
            clavier: Clavier::default(),
            scrolls: Scrolls::new(),
            racine: None,
            analyse: None,
            historique: Historique::default(),
            fichier_reglages: None,
            fichier_espace: None,
            atelier: Atelier::default(),
        }
    }
}

impl Commun {
    /// L'analyse du stockage, lancée à la première demande.
    pub fn analyser(&mut self) -> Option<&Analyse> {
        if self.analyse.is_none() {
            if let Some(racine) = self.racine.clone() {
                self.analyse = Some(Analyse::lancer(racine, crate::radio::reveiller));
            }
        }
        self.analyse.as_ref()
    }
}

/// L'état enregistré d'une instance : des paires `clé = valeur`, dans l'ordre où on
/// les a mises. Les valeurs tiennent sur une ligne.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Etat {
    pub paires: Vec<(String, String)>,
}

impl Etat {
    pub fn est_vide(&self) -> bool {
        self.paires.is_empty()
    }

    /// Met une valeur ; une clé déjà présente est remplacée. Les retours à la ligne
    /// deviennent des espaces.
    pub fn mettre(&mut self, cle: &str, valeur: impl ToString) {
        let v = valeur.to_string().replace(['\n', '\r'], " ");
        match self.paires.iter_mut().find(|p| p.0 == cle) {
            Some(p) => p.1 = v,
            None => self.paires.push((cle.to_string(), v)),
        }
    }

    pub fn mettre_oui(&mut self, cle: &str, v: bool) {
        self.mettre(cle, if v { "oui" } else { "non" });
    }

    pub fn texte(&self, cle: &str) -> Option<&str> {
        self.paires.iter().find(|p| p.0 == cle).map(|p| p.1.as_str())
    }

    pub fn nombre(&self, cle: &str) -> Option<f32> {
        self.texte(cle)?.trim().replace(',', ".").parse().ok().filter(|x: &f32| x.is_finite())
    }

    /// Plusieurs nombres séparés par des espaces.
    pub fn nombres(&self, cle: &str) -> Vec<f32> {
        self.texte(cle).map_or(Vec::new(), |v| v.split_whitespace().filter_map(|n| n.parse().ok()).filter(|x: &f32| x.is_finite()).collect())
    }

    pub fn oui(&self, cle: &str) -> Option<bool> {
        match self.texte(cle)?.trim().to_lowercase().as_str() {
            "oui" | "vrai" | "1" | "yes" | "true" => Some(true),
            "non" | "faux" | "0" | "no" | "false" => Some(false),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn l_etat_se_lit_et_s_ecrit() {
        let mut e = Etat::default();
        e.mettre("dossier", "/sdcard/a\nb");
        e.mettre_oui("caches", true);
        e.mettre("camera", "0.5 1 -2");
        e.mettre("dossier", "/sdcard/c");
        assert_eq!(e.paires.len(), 3);
        assert_eq!(e.texte("dossier"), Some("/sdcard/c"));
        assert_eq!(e.oui("caches"), Some(true));
        assert_eq!(e.nombres("camera"), vec![0.5, 1.0, -2.0]);
        assert_eq!(e.nombre("absente"), None);
    }

    #[test]
    fn l_atelier_se_relit() {
        let a = Atelier { couleur: 4, rotation: true, vitesse: 60.0, champ: 70.0, taille: 1.5, faces: false, recadrage: 0 };
        let mut e = Etat::default();
        a.ecrire(&mut e);
        assert_eq!(Atelier::lire(&e), a);
        assert_eq!(Atelier::lire(&Etat::default()), Atelier::default());
    }
}
