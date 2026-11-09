//! Réglages de l'interface, dans un fichier texte qu'on peut ouvrir et modifier à la
//! main : une valeur par ligne, `clé = valeur`, des commentaires après `#`.
//!
//! Seule l'interface y figure : l'apparence (`Theme`) et le clavier virtuel. Les
//! fenêtres et leur contenu sont dans le fichier de l'espace de travail (`espace`).
//! Une clé inconnue, absente ou mal écrite garde sa valeur par défaut, et les nombres
//! sont ramenés dans leurs bornes : un fichier abîmé ne casse rien. Le fichier est
//! réécrit en entier, avec ses commentaires, quand un réglage change dans l'application.

use crate::clavier::{Disposition, ReglagesClavier};
use crate::widgets::{Theme, ACCENTS};
use std::path::PathBuf;

/// Le nom du fichier, dans le dossier de l'application.
pub const NOM: &str = "interface.txt";

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Reglages {
    pub theme: Theme,
    pub clavier: ReglagesClavier,
}

/// Les paires `clé = valeur` d'un texte, sans les commentaires ni les lignes vides.
pub fn cles(texte: &str) -> Vec<(String, String)> {
    texte
        .lines()
        .filter_map(|l| {
            let l = l.split('#').next().unwrap_or("").trim();
            let (k, v) = l.split_once('=')?;
            Some((k.trim().to_lowercase(), v.trim().to_string()))
        })
        .collect()
}

fn nombre(v: &str) -> Option<f32> {
    v.trim_end_matches('%').trim().replace(',', ".").parse().ok().filter(|x: &f32| x.is_finite())
}

fn oui(v: &str) -> Option<bool> {
    match v.to_lowercase().as_str() {
        "oui" | "vrai" | "1" | "yes" | "true" => Some(true),
        "non" | "faux" | "0" | "no" | "false" => Some(false),
        _ => None,
    }
}

impl Reglages {
    pub fn lire(texte: &str) -> Reglages {
        let mut r = Reglages::default();
        let t = &mut r.theme;
        let c = &mut r.clavier;
        for (k, v) in cles(texte) {
            match k.as_str() {
                "commandes" => t.commande = nombre(&v).map_or(t.commande, |x| x.round().clamp(36.0, 64.0)),
                "lignes" => t.ligne = nombre(&v).map_or(t.ligne, |x| x.round().clamp(40.0, 64.0)),
                "texte" => t.texte = nombre(&v).map_or(t.texte, |x| (x / 100.0).clamp(0.85, 1.3)),
                "arrondi" => t.arrondi = nombre(&v).map_or(t.arrondi, |x| (x / 100.0).clamp(0.0, 1.0)),
                "sombre" => t.sombre = oui(&v).unwrap_or(t.sombre),
                "marges" => t.marge = nombre(&v).map_or(t.marge, |x| x.round().clamp(0.0, 20.0)),
                "padding" => t.padding = nombre(&v).map_or(t.padding, |x| x.round().clamp(4.0, 24.0)),
                "accent" => t.accent = ACCENTS.iter().position(|a| a.0.eq_ignore_ascii_case(&v)).unwrap_or(t.accent),
                "clavier" => c.disposition = Disposition::TOUTES.into_iter().find(|d| d.nom().eq_ignore_ascii_case(&v)).unwrap_or(c.disposition),
                "clavier_touches" => c.touche = nombre(&v).map_or(c.touche, |x| x.round().clamp(40.0, 64.0)),
                "clavier_appui_long" => c.appui_long = nombre(&v).map_or(c.appui_long, |x| (x / 1000.0).clamp(0.2, 0.8)),
                "clavier_apercu" => c.apercu = oui(&v).unwrap_or(c.apercu),
                "clavier_flottant" => c.flottant = oui(&v).unwrap_or(c.flottant),
                "clavier_position" => {
                    let n: Vec<f32> = v.split_whitespace().filter_map(nombre).collect();
                    if let [x, y] = n[..] {
                        c.position = (x.clamp(0.0, 1.0), y.clamp(0.0, 1.0));
                    }
                }
                _ => {}
            }
        }
        r
    }

    pub fn ecrire(&self) -> String {
        let t = &self.theme;
        let accents: Vec<String> = ACCENTS.iter().map(|a| a.0.to_lowercase()).collect();
        let mut s = String::new();
        s += "# Réglages de l'interface de Flow\n";
        s += "#\n";
        s += "# Ce fichier se modifie à la main : une valeur par ligne, « clé = valeur ».\n";
        s += "# Tout ce qui suit # est ignoré. Une valeur absente ou mal écrite reprend sa\n";
        s += "# valeur par défaut. L'application réécrit ce fichier quand un réglage change.\n\n";
        s += "# Taille des pastilles, pilules, boutons et onglets, en dp (36 à 64 ; 48)\n";
        s += &format!("commandes = {:.0}\n\n", t.commande);
        s += "# Hauteur des lignes des listes, en dp (40 à 64 ; 48)\n";
        s += &format!("lignes = {:.0}\n\n", t.ligne);
        s += "# Taille du texte, en % (85 à 130 ; 100)\n";
        s += &format!("texte = {:.0}\n\n", t.texte * 100.0);
        s += "# Arrondi des commandes, en % (0 : coins presque droits ; 100 : bouts ronds)\n";
        s += &format!("arrondi = {:.0}\n\n", t.arrondi * 100.0);
        s += "# Marges autour des boutons et pastilles, en dp (0 à 20 ; 10). À 0, l'affichage\n";
        s += "# est aplati : commandes jointives, sans ombre\n";
        s += &format!("marges = {:.0}\n\n", t.marge);
        s += "# Padding, l'espace entre le bord d'un bouton et son contenu, en dp (4 à 24 ; 16)\n";
        s += &format!("padding = {:.0}\n\n", t.padding);
        s += "# Thème sombre : oui ou non\n";
        s += &format!("sombre = {}\n\n", if t.sombre { "oui" } else { "non" });
        s += &format!("# Couleur d'accent : {}\n", accents.join(", "));
        s += &format!("accent = {}\n\n", accents[t.accent.min(accents.len() - 1)]);
        let c = &self.clavier;
        let dispositions: Vec<String> = Disposition::TOUTES.iter().map(|d| d.nom().to_lowercase()).collect();
        s += &format!("# Clavier virtuel : {}\n", dispositions.join(" ou "));
        s += &format!("clavier = {}\n\n", c.disposition.nom().to_lowercase());
        s += "# Hauteur des touches du clavier, en dp (40 à 64 ; 50)\n";
        s += &format!("clavier_touches = {:.0}\n\n", c.touche);
        s += "# Durée de l'appui long qui propose les variantes (é, è…), en ms (200 à 800 ; 350)\n";
        s += &format!("clavier_appui_long = {:.0}\n\n", c.appui_long * 1000.0);
        s += "# Bulle qui montre la touche sous le doigt : oui ou non\n";
        s += &format!("clavier_apercu = {}\n\n", if c.apercu { "oui" } else { "non" });
        s += "# Clavier flottant, par-dessus l'interface, qu'on déplace en tirant son bandeau : oui ou non\n";
        s += &format!("clavier_flottant = {}\n\n", if c.flottant { "oui" } else { "non" });
        s += "# Place du clavier flottant : horizontale puis verticale, de 0 (gauche, haut) à 1 (droite, bas)\n";
        s += &format!("clavier_position = {:.3} {:.3}\n", c.position.0, c.position.1);
        s
    }
}

/// Le fichier des réglages sur le disque. Il est relu au démarrage et au retour dans
/// l'application (il a pu être modifié entre-temps), et réécrit quand les réglages
/// changent.
pub struct Fichier {
    pub chemin: PathBuf,
    /// Ce qu'on a lu ou écrit en dernier : rien à faire tant que les réglages s'écrivent pareil.
    connu: String,
}

impl Fichier {
    pub fn new(chemin: PathBuf) -> Fichier {
        Fichier { chemin, connu: String::new() }
    }

    /// Le contenu du fichier, s'il a changé depuis la dernière lecture ou écriture.
    pub fn relire(&mut self) -> Option<Reglages> {
        let texte = std::fs::read_to_string(&self.chemin).ok()?;
        if texte == self.connu {
            return None;
        }
        let r = Reglages::lire(&texte);
        // Ce qu'on réécrirait à partir de ces réglages : un fichier simplement
        // reformaté n'est pas réécrit pour autant
        self.connu = r.ecrire();
        Some(r)
    }

    /// Écrit les réglages s'ils ont changé. Renvoie l'erreur éventuelle.
    pub fn suivre(&mut self, r: &Reglages) -> Result<(), String> {
        let texte = r.ecrire();
        if texte == self.connu {
            return Ok(());
        }
        if let Some(d) = self.chemin.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        // Écrit à côté puis renomme : un fichier n'est jamais à moitié écrit
        let tmp = self.chemin.with_extension("txt~");
        std::fs::write(&tmp, &texte).and_then(|_| std::fs::rename(&tmp, &self.chemin)).map_err(|e| e.to_string())?;
        self.connu = texte;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn les_reglages_se_relisent() {
        let r = Reglages {
            theme: Theme { commande: 56.0, ligne: 44.0, texte: 1.15, arrondi: 0.3, accent: 4, sombre: true, marge: 0.0, padding: 6.0 },
            clavier: ReglagesClavier { disposition: Disposition::Qwerty, touche: 58.0, appui_long: 0.5, apercu: false, flottant: true, position: (0.25, 0.5) },
        };
        let lu = Reglages::lire(&r.ecrire());
        assert_eq!(lu.clavier, r.clavier);
        let t = lu.theme;
        assert_eq!((t.commande, t.ligne, t.accent, t.sombre, t.marge, t.padding), (56.0, 44.0, 4, true, 0.0, 6.0));
        assert!((t.texte - 1.15).abs() < 1e-4 && (t.arrondi - 0.3).abs() < 1e-4);
    }

    #[test]
    fn un_fichier_ecrit_a_la_main_est_tolere() {
        let r = Reglages::lire(
            "# essai\n  COMMANDES=200   # trop grand\ntexte = 110 %\narrondi = abc\naccent = Vert\nsombre = oui\ninconnue = 3\nligne sans égal\n",
        );
        let t = r.theme;
        assert_eq!(t.commande, 64.0);
        assert!((t.texte - 1.1).abs() < 1e-4);
        assert_eq!(t.arrondi, Theme::default().arrondi);
        assert_eq!(t.accent, 3);
        assert!(t.sombre);
        // Le padding n'est jamais nul
        assert_eq!(Reglages::lire("padding = 0").theme.padding, 4.0);
        assert_eq!(Reglages::lire(""), Reglages::default());
    }

    #[test]
    fn le_fichier_ne_s_ecrit_que_s_il_change() {
        let dir = std::env::temp_dir().join(format!("flow-reglages-{}", std::process::id()));
        let mut f = Fichier::new(dir.join(NOM));
        assert!(f.relire().is_none());
        let mut r = Reglages::default();
        f.suivre(&r).unwrap();
        assert!(f.relire().is_none(), "on relit ce qu'on vient d'écrire : rien de neuf");
        // Une retouche à la main est relue
        std::fs::write(&f.chemin, "accent = rose\n").unwrap();
        assert_eq!(f.relire().unwrap().theme.accent, 5);
        r.theme.accent = 5;
        // …et ne provoque pas de réécriture tant que rien ne change dans l'application
        f.suivre(&r).unwrap();
        assert_eq!(std::fs::read_to_string(&f.chemin).unwrap(), "accent = rose\n");
        let _ = std::fs::remove_dir_all(dir);
    }
}
