//! Les contextes : les sous-applications que montrent les fenêtres de la surface.
//!
//! Chaque fenêtre porte une **instance** de contexte, avec son propre état (son
//! document, son dossier…) : deux fenêtres du même contexte travaillent
//! chacune sur le leur. Ce qui doit être partagé entre toutes (l'apparence, le
//! clavier, l'analyse du stockage, l'historique des radios…) est dans `Commun`, et
//! se règle dans le panneau des paramètres, pas dans un contexte.
//!
//! Un contexte se déclare par un `Genre` (sa clé, son nom, son icône et sa fonction
//! d'ouverture) dans la liste `contextes::GENRES`, et son instance implémente le trait
//! `Contexte`. L'état d'une instance s'enregistre en paires `clé = valeur` (`Etat`)
//! dans le fichier de l'espace de travail, et la relit à l'ouverture.
//!
//! Le guide d'intégration est dans `docs/contextes.md`.

use crate::clavier::{Clavier, Frappe};
use crate::disque::Analyse;
use crate::radio::Historique;
use crate::surface::{FenetreId, Placee};
use crate::ui::{Gesture, Id};
use crate::widgets::{Cx, Icon, Scrolls, Theme};
use std::path::{Path, PathBuf};

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

    /// Un geste (glissement d'un doigt, déplacement et zoom à deux doigts) commencé
    /// dans la fenêtre.
    fn geste(&mut self, _g: &Gesture, _p: &Placee, _c: &Commun) {}

    /// Une frappe du clavier virtuel pour le champ `cible`. Renvoie vrai si le champ
    /// est à cette instance (la frappe ne va alors à personne d'autre).
    fn frappe(&mut self, _f: FenetreId, _cible: Id, _frappe: Frappe, _c: &mut Commun) -> bool {
        false
    }

    /// Retour dans l'application : ce qui est lu sur le disque a pu changer.
    fn retour(&mut self, _c: &Commun) {}

    /// L'instance va disparaître (fenêtre fermée, autre contexte) ou l'application
    /// passe en arrière-plan : mettre à l'abri ce qui n'est pas enregistré (un
    /// brouillon dans `Commun::dossier`). `ecrire` est appelé ensuite.
    fn mettre_a_l_abri(&mut self, _c: &Commun) {}

    /// Le fichier que l'instance édite, s'il y en a un : un même fichier ne s'ouvre
    /// pas deux fois.
    fn document(&self) -> Option<&Path> {
        None
    }
}

/// Ce qu'une instance demande à l'application, traité après le dessin.
#[derive(Clone, Debug, PartialEq)]
pub enum Demande {
    /// Ouvrir un contexte, avec cet état, dans une fenêtre née du partage de
    /// `depuis` (par exemple l'éditeur de texte, depuis l'explorateur). Si une fenêtre
    /// édite déjà ce document (`etat` : `document`), rien ne s'ouvre.
    Ouvrir { depuis: FenetreId, cle: &'static str, etat: Etat },
}

/// Ce que partagent toutes les fenêtres.
pub struct Commun {
    /// L'apparence de l'interface, réglée dans le panneau des paramètres.
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
    /// Le dossier de Flow (réglages, espace, brouillons), s'il est connu.
    pub dossier: Option<PathBuf>,
    /// Les demandes des instances, traitées par l'application après le dessin.
    pub demandes: Vec<Demande>,
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
            dossier: None,
            demandes: Vec::new(),
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
}
