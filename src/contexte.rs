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
use crate::disque::{self, Analyse, Noeud, Synthese};
use std::sync::Arc;
use crate::orchestre::Orchestre;
use crate::radio::Historique;
use crate::surface::{FenetreId, Placee};
use crate::gestes::ReglagesGestes;
use crate::ui::Id;
use crate::widgets::{Cx, Icon, Scrolls, Theme};
use std::path::{Path, PathBuf};

/// Une sorte de contexte, telle qu'elle apparaît dans le menu des fenêtres.
pub struct Genre {
    /// La clé, stable : c'est elle que retient le fichier de l'espace de travail.
    /// Minuscules, chiffres, `-` et `_`.
    pub cle: &'static str,
    /// Le nom affiché dans la barre de la fenêtre et le menu.
    pub nom: &'static str,
    /// Une phrase courte qui dit à quoi il sert (cartes de l'Accueil).
    pub legende: &'static str,
    pub icone: Icon,
    /// Crée une instance à partir de son état enregistré (vide pour une fenêtre
    /// nouvelle). Les valeurs absentes ou illisibles prennent leur valeur par défaut.
    pub ouvrir: fn(&Etat) -> Box<dyn Contexte>,
    /// Son applet dans la barre de l'application (`applet`), s'il en a un : un seul pour
    /// toute l'application, créé au lancement.
    pub applet: Option<fn() -> Box<dyn crate::applet::Applet>>,
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
    /// Ouvrir un contexte, avec cet état, à la place de celui de la fenêtre `depuis`
    /// (par exemple l'éditeur de texte, depuis l'explorateur). Si une fenêtre édite
    /// déjà ce document (`etat` : `document`), rien ne s'ouvre.
    Ouvrir { depuis: FenetreId, cle: &'static str, etat: Etat },
    /// Montrer un contexte (depuis un applet) : rien si une fenêtre le montre déjà,
    /// sinon la plus grande fenêtre se scinde pour lui faire place.
    Montrer { cle: &'static str },
}

/// Ce que partagent toutes les fenêtres.
pub struct Commun {
    /// L'apparence de l'interface, réglée dans le panneau des paramètres.
    pub theme: Theme,
    /// Le clavier virtuel : un seul, pour le champ qui a le focus.
    pub clavier: Clavier,
    /// Les réglages des gestes élaborés (`cx.ui.gestes`).
    pub gestes: ReglagesGestes,
    /// Les positions de défilement, par identifiant de corps.
    pub scrolls: Scrolls,
    /// La racine du stockage partagé, donnée par Android.
    pub racine: Option<PathBuf>,
    /// L'analyse du stockage, une pour toutes les fenêtres (Disque, Fichiers).
    pub analyse: Option<Analyse>,
    /// La synthèse de la dernière analyse (les empreintes des dossiers), relue de
    /// `Flow/empreintes.txt` ou tirée d'une analyse finie.
    pub empreintes: Option<Arc<Synthese>>,
    pub(crate) empreintes_lues: bool,
    /// L'arbre dont la synthèse est tirée : une analyse finie n'est résumée qu'une fois.
    pub(crate) resume: Option<Arc<Noeud>>,
    /// Les derniers flux radio écoutés.
    pub historique: Historique,
    /// Où sont enregistrés les réglages et l'espace de travail, pour l'indiquer.
    pub fichier_reglages: Option<String>,
    pub fichier_espace: Option<String>,
    /// Le dossier de Flow (réglages, espace, brouillons), s'il est connu.
    pub dossier: Option<PathBuf>,
    /// Les demandes des instances, traitées par l'application après le dessin.
    pub demandes: Vec<Demande>,
    /// Les fenêtres libres (« free flow ») plutôt que le pavage (« full flow ») : réglé
    /// dans les paramètres, retenu avec l'espace de travail.
    pub libre: bool,
    /// L'orchestrateur : empreintes, batterie, recensement des contextes.
    pub orchestre: Orchestre,
    /// Les dictionnaires de coloration syntaxique, lus à la première demande et relus
    /// au retour dans l'application.
    pub syntaxes: Option<Arc<crate::syntaxe::Bibliotheque>>,
    /// Le presse-papier des fichiers et l'opération en cours, communs aux fenêtres
    /// Fichiers.
    pub fichiers: crate::operations::Manutention,
    /// Les applets montrés dans la barre (clés des contextes), réglés dans les
    /// paramètres.
    pub applets: Vec<String>,
    /// Le dernier texte de `mixeur.txt` lu ou écrit : il ne se réécrit que s'il change.
    pub(crate) mixeur_ecrit: Option<String>,
}

impl Default for Commun {
    fn default() -> Commun {
        Commun {
            theme: Theme::default(),
            clavier: Clavier::default(),
            gestes: ReglagesGestes::default(),
            scrolls: Scrolls::new(),
            racine: None,
            analyse: None,
            empreintes: None,
            empreintes_lues: false,
            resume: None,
            historique: Historique::default(),
            fichier_reglages: None,
            fichier_espace: None,
            dossier: None,
            demandes: Vec::new(),
            libre: false,
            orchestre: Orchestre::default(),
            syntaxes: None,
            fichiers: Default::default(),
            applets: crate::applet::par_defaut(),
            mixeur_ecrit: None,
        }
    }
}

impl Commun {
    /// Les dictionnaires de coloration syntaxique (`Flow/syntaxes/`).
    pub fn syntaxes(&mut self) -> Arc<crate::syntaxe::Bibliotheque> {
        let dossier = self.dossier.as_ref().map(|d| d.join(crate::syntaxe::DOSSIER));
        self.syntaxes.get_or_insert_with(|| Arc::new(crate::syntaxe::Bibliotheque::charger(dossier.as_deref()))).clone()
    }

    /// L'analyse du stockage, lancée à la première demande.
    pub fn analyser(&mut self) -> Option<&Analyse> {
        if self.analyse.is_none() {
            if let Some(racine) = self.racine.clone() {
                self.analyse = Some(Analyse::lancer(racine, crate::radio::reveiller));
            }
        }
        self.analyse.as_ref()
    }

    /// Une analyse vient de finir : sa synthèse remplace l'ancienne et s'enregistre
    /// (dans un thread). Ne coûte rien le reste du temps.
    pub fn suivre_analyse(&mut self) {
        let Some(a) = &self.analyse else { return };
        let Some((n, duree)) = a.resultat() else { return };
        if self.resume.as_ref().is_some_and(|r| Arc::ptr_eq(r, &n)) {
            return;
        }
        let s = Synthese { racine: a.racine.clone(), date: disque::maintenant(), arbre: disque::Empreinte::depuis(&n) };
        if let Some(f) = self.dossier.as_ref().map(|d| d.join(disque::EMPREINTES)) {
            let texte = s.ecrire();
            std::thread::spawn(move || {
                let r = ecrire_fichier(&f, &texte);
                if let Err(e) = r {
                    crate::android::log(&format!("empreintes non enregistrées : {e}"));
                }
            });
        }
        fn compter(e: &disque::Empreinte) -> usize {
            1 + e.enfants.iter().map(compter).sum::<usize>()
        }
        let texte = format!("empreintes refaites : {} dossiers, {} parcourus en {:.0} s", compter(&s.arbre), crate::explorateur::taille(n.taille), duree);
        self.orchestre.noter(s.date, texte);
        self.empreintes = Some(Arc::new(s));
        self.empreintes_lues = true;
        self.resume = Some(n);
    }

    /// Relit une fois la synthèse enregistrée (celle d'un autre stockage ne sert à rien).
    fn lire_empreintes(&mut self) {
        if !std::mem::replace(&mut self.empreintes_lues, true) {
            let lue = self.dossier.as_ref().and_then(|d| std::fs::read_to_string(d.join(disque::EMPREINTES)).ok()).and_then(|t| Synthese::lire(&t));
            self.empreintes = lue.filter(|s| Some(&s.racine) == self.racine.as_ref()).map(Arc::new);
        }
    }

    /// Les empreintes des dossiers, pour Fichiers : la synthèse enregistrée, relue une
    /// fois. C'est l'orchestrateur qui la tient à jour (`orchestrer`).
    pub fn empreintes(&mut self) -> Option<Arc<Synthese>> {
        self.lire_empreintes();
        self.empreintes.clone()
    }

    /// Une analyse lancée et pas encore résumée.
    pub fn analyse_en_vol(&self) -> bool {
        self.analyse.as_ref().is_some_and(|a| a.resultat().is_none_or(|(n, _)| !self.resume.as_ref().is_some_and(|r| Arc::ptr_eq(r, &n))))
    }

    /// L'âge de la synthèse, en secondes, si elle existe.
    pub fn age_empreintes(&self) -> Option<u64> {
        self.empreintes.as_ref().map(|s| disque::maintenant().saturating_sub(s.date))
    }

    /// Où en est la tâche des empreintes.
    pub fn etat_empreintes(&self) -> EtatEmpreintes {
        let r = &self.orchestre.reglages;
        if self.racine.is_none() {
            EtatEmpreintes::SansStockage
        } else if self.analyse_en_vol() {
            EtatEmpreintes::EnCours
        } else if !r.empreintes_auto {
            EtatEmpreintes::Manuelle
        } else if self.age_empreintes().is_some_and(|a| a < r.periode()) {
            EtatEmpreintes::AJour { dans: r.periode() - self.age_empreintes().unwrap_or(0) }
        } else if r.economie && self.orchestre.basse() {
            EtatEmpreintes::Reportee
        } else {
            EtatEmpreintes::Due
        }
    }

    /// Relance l'analyse du stockage (l'ancienne, si elle court encore, est abandonnée).
    pub fn reanalyser(&mut self) {
        self.analyse = None;
        self.analyser();
    }

    /// Le travail de l'orchestrateur, à chaque image : relire ses réglages et
    /// l'historique de la batterie quand le dossier de Flow change, résumer une analyse
    /// finie, lancer celle des empreintes quand elles ont vieilli, et enregistrer ce
    /// qui a changé. `ecrire` : les réglages peuvent s'enregistrer (doigt levé).
    pub fn orchestrer(&mut self, ecrire: bool) {
        let maintenant = disque::maintenant();
        if self.orchestre.dossier != self.dossier {
            self.orchestre.dossier = self.dossier.clone();
            if let Some(d) = &self.dossier {
                let t = std::fs::read_to_string(d.join(crate::orchestre::FICHIER)).ok();
                if let Some(t) = &t {
                    self.orchestre.reglages = crate::orchestre::ReglagesOrchestre::lire(t);
                }
                self.orchestre.ecrits = t;
                let lus = std::fs::read_to_string(d.join(crate::orchestre::BATTERIE)).map_or(Vec::new(), |t| crate::orchestre::Orchestre::lire_releves(&t));
                self.orchestre.reprendre(lus, maintenant);
                self.empreintes_lues = false;
                // Les réglages du mixeur
                let t = std::fs::read_to_string(d.join(crate::mixage::FICHIER)).ok();
                if let Some(t) = &t {
                    crate::mixage::mixeur().lire_reglages(t);
                }
                self.mixeur_ecrit = t;
            }
        }
        self.suivre_analyse();
        self.lire_empreintes();
        match self.etat_empreintes() {
            EtatEmpreintes::Due => {
                let pourquoi = match self.age_empreintes() {
                    Some(a) => format!("empreintes vieilles de {} : analyse du stockage", crate::orchestre::duree(a)),
                    None => "pas d'empreintes : analyse du stockage".to_string(),
                };
                self.orchestre.noter(maintenant, pourquoi);
                self.orchestre.report_annonce = false;
                self.reanalyser();
            }
            EtatEmpreintes::Reportee if !self.orchestre.report_annonce => {
                self.orchestre.report_annonce = true;
                self.orchestre.noter(maintenant, "analyse du stockage reportée : batterie basse");
            }
            _ => {}
        }
        let Some(d) = self.dossier.clone() else { return };
        // Les réglages, s'ils ont changé (bornés d'abord : le fichier peut venir d'une main)
        self.orchestre.reglages = self.orchestre.reglages.borner();
        let texte = self.orchestre.reglages.ecrire();
        if ecrire && self.orchestre.ecrits.as_deref() != Some(texte.as_str()) {
            match ecrire_fichier(&d.join(crate::orchestre::FICHIER), &texte) {
                Ok(()) => self.orchestre.ecrits = Some(texte),
                Err(e) => {
                    crate::android::log(&format!("réglages de l'orchestre non enregistrés : {e}"));
                    self.orchestre.ecrits = Some(texte);
                }
            }
        }
        // Les réglages du mixeur, s'ils ont changé
        if ecrire {
            let texte = crate::mixage::mixeur().ecrire_reglages();
            if self.mixeur_ecrit.as_deref() != Some(texte.as_str()) {
                if let Err(e) = ecrire_fichier(&d.join(crate::mixage::FICHIER), &texte) {
                    crate::android::log(&format!("réglages du mixeur non enregistrés : {e}"));
                }
                self.mixeur_ecrit = Some(texte);
            }
        }
        if std::mem::take(&mut self.orchestre.batterie_a_ecrire) {
            let (f, texte) = (d.join(crate::orchestre::BATTERIE), self.orchestre.ecrire_releves());
            std::thread::spawn(move || {
                if let Err(e) = ecrire_fichier(&f, &texte) {
                    crate::android::log(&format!("historique de la batterie non enregistré : {e}"));
                }
            });
        }
    }
}

/// Où en est la tâche des empreintes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EtatEmpreintes {
    /// Pas de stockage connu (accès non accordé).
    SansStockage,
    /// Une analyse court.
    EnCours,
    /// À jour, refaites dans tant de secondes.
    AJour { dans: u64 },
    /// L'orchestrateur ne les refait pas de lui-même.
    Manuelle,
    /// À refaire, mais la batterie est basse.
    Reportee,
    /// À refaire maintenant.
    Due,
}

/// Écrit un fichier d'un coup : d'abord à côté, puis renommé.
pub fn ecrire_fichier(f: &Path, texte: &str) -> std::io::Result<()> {
    let tmp = f.with_extension("txt~");
    f.parent().map_or(Ok(()), std::fs::create_dir_all)?;
    std::fs::write(&tmp, texte)?;
    std::fs::rename(&tmp, f)
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

#[cfg(test)]
mod tests_empreintes {
    use super::*;

    #[test]
    fn la_synthese_est_relue_et_refaite_au_plus_une_fois_par_jour() {
        let d = std::env::temp_dir().join(format!("flow-empreintes-{}", std::process::id()));
        let racine = d.join("stockage");
        std::fs::create_dir_all(racine.join("Musique")).unwrap();
        std::fs::write(racine.join("Musique/a.ogg"), vec![1u8; 9000]).unwrap();
        let mut c = Commun { racine: Some(racine.clone()), dossier: Some(d.join("Flow")), ..Commun::default() };
        // Rien sur le disque : l'orchestrateur lance une analyse, puis la résume et l'enregistre
        c.orchestrer(true);
        assert!(c.empreintes().is_none());
        assert!(c.analyse.is_some());
        while c.analyse.as_ref().unwrap().en_cours() {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        c.orchestrer(true);
        let s = c.empreintes().unwrap();
        assert!(matches!(c.etat_empreintes(), EtatEmpreintes::AJour { .. }));
        assert!(d.join("Flow").join(crate::orchestre::FICHIER).exists());
        assert!(disque::situer(&s.arbre, &["Musique"]).is_some());
        let f = d.join("Flow").join(disque::EMPREINTES);
        for _ in 0..200 {
            if f.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        // Au lancement suivant : la synthèse du jour est relue, sans nouvelle analyse
        let mut c2 = Commun { racine: Some(racine.clone()), dossier: Some(d.join("Flow")), ..Commun::default() };
        c2.orchestrer(true);
        assert_eq!(c2.empreintes().as_deref(), Some(&*s));
        assert!(c2.analyse.is_none());
        // Vieille d'un jour : une analyse repart
        let mut vieille = (*s).clone();
        vieille.date -= disque::UN_JOUR;
        std::fs::write(&f, vieille.ecrire()).unwrap();
        let mut c3 = Commun { racine: Some(racine.clone()), dossier: Some(d.join("Flow")), ..Commun::default() };
        c3.orchestrer(true);
        assert!(c3.empreintes().is_some());
        assert!(c3.analyse.is_some());
        // Batterie basse et hors charge : l'analyse est reportée ; sans réglage auto, rien
        let mut c4 = Commun { racine: Some(racine.clone()), dossier: Some(d.join("Flow")), ..Commun::default() };
        c4.orchestre.relever(5, false, disque::maintenant());
        c4.orchestrer(true);
        assert_eq!(c4.etat_empreintes(), EtatEmpreintes::Reportee);
        assert!(c4.analyse.is_none());
        let mut c5 = Commun { racine: Some(racine), dossier: Some(d.join("Flow")), ..Commun::default() };
        c5.orchestre.dossier = c5.dossier.clone();
        c5.orchestre.reglages.empreintes_auto = false;
        c5.orchestrer(true);
        assert_eq!(c5.etat_empreintes(), EtatEmpreintes::Manuelle);
        assert!(c5.analyse.is_none());
        let _ = std::fs::remove_dir_all(d);
    }
}

