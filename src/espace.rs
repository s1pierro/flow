//! L'espace de travail sur le disque : la disposition des fenêtres, le contexte et
//! l'état de chacune, dans un fichier texte qu'on peut lire et retoucher à la main.
//!
//! ```text
//! # Espace de travail de Flow
//! fenetres = C0.6000(F2:fichiers,F1:cube)
//!
//! [commun]
//! cube_couleur = indigo
//!
//! [fenetre 2]
//! dossier = /storage/emulated/0/Download
//! caches = non
//!
//! [fenetre 1]
//! camera = 0.650 0.450 6.000 0.000 0.000 0.000
//! ```
//!
//! Les lignes qui commencent par `#` sont des commentaires (un `#` plus loin fait
//! partie de la valeur : un chemin peut en contenir). Une section `[fenetre N]` porte
//! l'état de la fenêtre N, tel que son contexte l'écrit (`Contexte::ecrire`).

use crate::contexte::Etat;
use crate::surface::FenetreId;
use std::path::PathBuf;

/// Le nom du fichier, à côté des réglages de l'interface.
pub const NOM: &str = "espace.txt";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Espace {
    /// La disposition, telle que `Surface::ecrire_avec` l'écrit (contextes par clé).
    pub fenetres: Option<String>,
    /// Ce qui est commun à toutes les fenêtres.
    pub commun: Etat,
    /// L'état de chaque fenêtre, dans l'ordre de la disposition.
    pub sections: Vec<(FenetreId, Etat)>,
}

impl Espace {
    pub fn lire(texte: &str) -> Espace {
        let mut e = Espace::default();
        // Où vont les paires : en tête, dans [commun], ou dans une fenêtre
        enum Ou {
            Tete,
            Commun,
            Fenetre(usize),
            Ailleurs,
        }
        let mut ou = Ou::Tete;
        for l in texte.lines() {
            let l = l.trim();
            if l.is_empty() || l.starts_with('#') {
                continue;
            }
            if let Some(nom) = l.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                let nom = nom.trim();
                ou = if nom == "commun" {
                    Ou::Commun
                } else if let Some(n) = nom.strip_prefix("fenetre").and_then(|n| n.trim().parse::<FenetreId>().ok()) {
                    e.sections.push((n, Etat::default()));
                    Ou::Fenetre(e.sections.len() - 1)
                } else {
                    Ou::Ailleurs
                };
                continue;
            }
            let Some((k, v)) = l.split_once('=') else { continue };
            let (k, v) = (k.trim().to_lowercase(), v.trim());
            match ou {
                Ou::Tete if k == "fenetres" && !v.is_empty() => e.fenetres = Some(v.to_string()),
                Ou::Tete | Ou::Ailleurs => {}
                Ou::Commun => e.commun.mettre(&k, v),
                Ou::Fenetre(i) => e.sections[i].1.mettre(&k, v),
            }
        }
        e
    }

    pub fn ecrire(&self) -> String {
        let mut s = String::new();
        s += "# Espace de travail de Flow : les fenêtres, leur contexte et leur état.\n";
        s += "#\n";
        s += "# L'application réécrit ce fichier quand l'espace change, et le relit au retour\n";
        s += "# si on l'a modifié à la main. Une valeur absente ou illisible reprend sa valeur\n";
        s += "# par défaut.\n";
        s += "#\n";
        s += "# Disposition : F<numéro>:<contexte> pour une fenêtre ; C<part>(…,…) pour deux\n";
        s += "# fenêtres côte à côte, L<part>(…,…) pour l'une sur l'autre (part de la\n";
        s += "# première, de 0.1 à 0.9).\n";
        if let Some(f) = &self.fenetres {
            s += &format!("fenetres = {f}\n");
        }
        if !self.commun.est_vide() {
            s += "\n[commun]\n";
            for (k, v) in &self.commun.paires {
                s += &format!("{k} = {v}\n");
            }
        }
        for (id, etat) in &self.sections {
            if etat.est_vide() {
                continue;
            }
            s += &format!("\n[fenetre {id}]\n");
            for (k, v) in &etat.paires {
                s += &format!("{k} = {v}\n");
            }
        }
        s
    }

    /// L'état enregistré de la fenêtre `f` (vide s'il n'y en a pas).
    pub fn section(&self, f: FenetreId) -> Etat {
        self.sections.iter().find(|s| s.0 == f).map(|s| s.1.clone()).unwrap_or_default()
    }
}

/// Le fichier de l'espace sur le disque : relu au démarrage et au retour dans
/// l'application, réécrit quand l'espace change.
pub struct Fichier {
    pub chemin: PathBuf,
    /// Ce qu'on a lu ou écrit en dernier.
    connu: String,
}

impl Fichier {
    pub fn new(chemin: PathBuf) -> Fichier {
        Fichier { chemin, connu: String::new() }
    }

    /// Le texte du fichier, s'il a changé depuis la dernière lecture ou écriture.
    pub fn relire(&mut self) -> Option<String> {
        let texte = std::fs::read_to_string(&self.chemin).ok()?;
        (texte != self.connu).then(|| {
            self.connu = texte.clone();
            texte
        })
    }

    /// Ce que l'application écrirait après avoir relu le fichier : un fichier
    /// simplement reformaté n'est pas réécrit pour autant.
    pub fn connaitre(&mut self, texte: String) {
        self.connu = texte;
    }

    /// Écrit le texte s'il a changé.
    pub fn suivre(&mut self, texte: &str) -> Result<(), String> {
        if texte == self.connu {
            return Ok(());
        }
        if let Some(d) = self.chemin.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        // Écrit à côté puis renomme : un fichier n'est jamais à moitié écrit
        let tmp = self.chemin.with_extension("txt~");
        std::fs::write(&tmp, texte).and_then(|_| std::fs::rename(&tmp, &self.chemin)).map_err(|e| e.to_string())?;
        self.connu = texte.to_string();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn l_espace_se_relit() {
        let mut e = Espace { fenetres: Some("C0.6000(F2:fichiers,F1:cube)".into()), ..Espace::default() };
        e.commun.mettre("cube_couleur", "vert");
        let mut f = Etat::default();
        f.mettre("dossier", "/sdcard/Musique #1");
        e.sections.push((2, f));
        e.sections.push((1, Etat::default()));
        let t = e.ecrire();
        assert!(!t.contains("[fenetre 1]"), "une fenêtre sans état n'a pas de section");
        let lu = Espace::lire(&t);
        assert_eq!(lu.fenetres, e.fenetres);
        assert_eq!(lu.commun, e.commun);
        assert_eq!(lu.section(2).texte("dossier"), Some("/sdcard/Musique #1"));
        assert!(lu.section(1).est_vide());
    }

    #[test]
    fn un_fichier_abime_est_tolere() {
        let lu = Espace::lire("n'importe quoi\n[inconnue]\na = 1\n[fenetre x]\nb = 2\n[fenetre 3]\n  Dossier =  /a  \n");
        assert_eq!(lu.fenetres, None);
        assert!(lu.commun.est_vide());
        assert_eq!(lu.sections.len(), 1);
        assert_eq!(lu.section(3).texte("dossier"), Some("/a"));
    }

    #[test]
    fn le_fichier_ne_s_ecrit_que_s_il_change() {
        let dir = std::env::temp_dir().join(format!("flow-espace-{}", std::process::id()));
        let mut f = Fichier::new(dir.join(NOM));
        assert!(f.relire().is_none());
        f.suivre("fenetres = F1:cube\n").unwrap();
        assert!(f.relire().is_none(), "on relit ce qu'on vient d'écrire : rien de neuf");
        std::fs::write(&f.chemin, "fenetres = F1:aide\n").unwrap();
        assert_eq!(f.relire().as_deref(), Some("fenetres = F1:aide\n"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
