//! Les applications Android, pour le contexte Fichiers : lire un `.apk` (son nom, sa
//! version, et celle de l'application installée), l'installer ou la mettre à jour,
//! lancer une application installée.
//!
//! Android ne laisse jamais une application installer en silence : Flow copie le
//! `.apk` dans une session de l'installateur du système, qui demande confirmation. Il
//! faut aussi que Flow ait le droit de demander des installations (« Installer des
//! applis inconnues », un réglage d'Android que `autoriser` ouvre). Une mise à jour
//! n'est acceptée que signée de la même clé, et pas plus ancienne.
//!
//! Le travail se fait en Java (`fr.s1pierro.flow.Installation`), appelé par JNI
//! (`acces::installation`) ; hors d'Android, tout répond « indisponible ».

use crate::acces::installation;
use std::path::Path;

/// Ce que dit un `.apk`.
#[derive(Clone, Debug, PartialEq)]
pub struct Apk {
    pub paquet: String,
    pub nom: String,
    pub version: String,
    pub code: i64,
    /// La version et le code de l'application installée, si elle l'est.
    pub installee: Option<(String, i64)>,
}

/// Ce que l'installation ferait.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    Installer,
    MettreAJour,
    Reinstaller,
    /// Plus ancienne que l'installée : Android refuse.
    PlusAncienne,
}

impl Action {
    /// Le nom du bouton ; rien pour une version refusée.
    pub fn bouton(self) -> Option<&'static str> {
        match self {
            Action::Installer => Some("Installer"),
            Action::MettreAJour => Some("Mettre à jour"),
            Action::Reinstaller => Some("Réinstaller"),
            Action::PlusAncienne => None,
        }
    }
}

impl Apk {
    /// Relit la réponse de `Installation.lire` : six lignes.
    pub fn depuis(t: &str) -> Option<Apk> {
        let l: Vec<&str> = t.split('\n').collect();
        if l.len() < 6 || l[0].is_empty() {
            return None;
        }
        let installee = l[5].trim().parse().ok().map(|c| (l[4].to_string(), c));
        Some(Apk { paquet: l[0].into(), version: l[1].into(), code: l[2].trim().parse().ok()?, nom: if l[3].is_empty() { l[0].into() } else { l[3].into() }, installee })
    }

    pub fn action(&self) -> Action {
        match &self.installee {
            None => Action::Installer,
            Some((_, c)) if self.code > *c => Action::MettreAJour,
            Some((_, c)) if self.code == *c => Action::Reinstaller,
            Some(_) => Action::PlusAncienne,
        }
    }
}

/// Où en est l'installation lancée par `installer`.
#[derive(Clone, Debug, PartialEq)]
pub enum Suivi {
    Rien,
    /// La copie vers l'installateur, de 0 à 1.
    Copie(f32),
    /// La fenêtre de confirmation du système est ouverte.
    Confirmation,
    Reussie,
    Echec(String),
    Annulee,
}

impl Suivi {
    /// Relit la réponse de `Installation.suivi` : état, octets copiés, total, message.
    pub fn depuis(t: &str) -> Suivi {
        let l: Vec<&str> = t.split('\n').collect();
        let n = |i: usize| l.get(i).and_then(|s| s.trim().parse::<f64>().ok()).unwrap_or(0.0);
        match l.first().map(|s| s.trim()) {
            Some("1") => Suivi::Copie(if n(2) > 0.0 { (n(1) / n(2)).clamp(0.0, 1.0) as f32 } else { 0.0 }),
            Some("2") => Suivi::Confirmation,
            Some("3") => Suivi::Reussie,
            Some("4") => Suivi::Echec(l.get(3).map_or(String::new(), |s| s.to_string())),
            Some("5") => Suivi::Annulee,
            _ => Suivi::Rien,
        }
    }

    /// Une installation est en cours : copie ou confirmation.
    pub fn en_cours(&self) -> bool {
        matches!(self, Suivi::Copie(_) | Suivi::Confirmation)
    }
}

/// Lit le `.apk` ; une erreur lisible sinon.
pub fn lire(chemin: &Path) -> Result<Apk, String> {
    let Some(t) = installation("lire", Some(&chemin.to_string_lossy())) else {
        return Err(if cfg!(target_os = "android") { "Ce fichier n'est pas une application lisible." } else { "Les applications ne s'installent que sous Android." }.into());
    };
    Apk::depuis(&t).ok_or_else(|| "Ce fichier n'est pas une application lisible.".into())
}

/// Vrai si Flow a le droit de demander des installations.
pub fn autorisee() -> bool {
    installation("autorisee", None).as_deref() == Some("1")
}

/// Ouvre le réglage d'Android qui autorise Flow à installer des applications.
pub fn autoriser() {
    installation("autoriser", None);
}

/// Lance l'installation du `.apk` ; à suivre par `suivi`.
pub fn installer(chemin: &Path) -> Result<(), String> {
    match installation("installer", Some(&chemin.to_string_lossy())) {
        Some(e) if e.is_empty() => Ok(()),
        Some(e) => Err(e),
        None => Err("L'installateur d'Android est injoignable.".into()),
    }
}

pub fn suivi() -> Suivi {
    installation("suivi", None).map_or(Suivi::Rien, |t| Suivi::depuis(&t))
}

/// Oublie une installation terminée, avant d'en suivre une autre.
pub fn oublier() {
    installation("oublier", None);
}

/// Lance l'application installée `paquet` ; faux si elle n'a pas d'écran d'accueil.
pub fn lancer(paquet: &str) -> bool {
    installation("lancer", Some(paquet)).as_deref() == Some("1")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_reponse_de_java_se_relit() {
        let a = Apk::depuis("fr.s1pierro.flow\n0.30\n37\nFlow\n0.29\n36").unwrap();
        assert_eq!((a.nom.as_str(), a.code, a.installee.clone()), ("Flow", 37, Some(("0.29".into(), 36))));
        assert_eq!(a.action(), Action::MettreAJour);
        let b = Apk::depuis("org.exemple\n1.0\n5\n\n\n").unwrap();
        assert_eq!((b.nom.as_str(), b.installee.clone(), b.action()), ("org.exemple", None, Action::Installer));
        assert_eq!(Apk::depuis("org.exemple\n1.0\n5\nX\n1.0\n5").unwrap().action(), Action::Reinstaller);
        assert_eq!(Apk::depuis("org.exemple\n1.0\n5\nX\n2.0\n9").unwrap().action(), Action::PlusAncienne);
        assert!(Apk::depuis("").is_none() && Apk::depuis("a\nb\nc\nd\ne\nf").is_none());
        assert_eq!(Suivi::depuis("1\n512\n1024\n"), Suivi::Copie(0.5));
        assert_eq!(Suivi::depuis("4\n0\n0\nINSTALL_FAILED_UPDATE_INCOMPATIBLE"), Suivi::Echec("INSTALL_FAILED_UPDATE_INCOMPATIBLE".into()));
        assert_eq!(Suivi::depuis("3\n1\n1\n"), Suivi::Reussie);
    }

    #[test]
    fn hors_d_android_rien_ne_s_installe() {
        assert!(lire(Path::new("/x.apk")).is_err());
        assert!(!autorisee() && installer(Path::new("/x.apk")).is_err() && !lancer("x"));
        assert_eq!(suivi(), Suivi::Rien);
    }
}
