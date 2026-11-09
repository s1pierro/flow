//! Le contexte Paramètres : l'apparence de l'interface et le clavier, appliqués dès
//! l'image suivante et enregistrés dans le fichier des réglages.

use crate::clavier::{self, Disposition, Edition, Frappe};
use crate::contexte::{Commun, Contexte, Etat, Genre};
use crate::surface::{FenetreId, Placee};
use crate::ui::{id, sub_id, Id};
use crate::widgets::{Cx, Icon, Theme, ACCENTS};

pub const CLE: &str = "parametres";
pub const GENRE: Genre = Genre { cle: CLE, nom: "Paramètres", icone: Icon::Sliders, ouvrir };

const PARAMS: Id = id("parametres");
const ESSAI: Id = id("essai-clavier");

/// Le champ d'essai du clavier, propre à chaque fenêtre.
#[derive(Clone, Default)]
pub struct Parametres {
    essai: Edition,
}

fn ouvrir(_: &Etat) -> Box<dyn Contexte> {
    Box::new(Parametres::default())
}

impl Contexte for Parametres {
    fn cle(&self) -> &'static str {
        CLE
    }

    fn dupliquer(&self) -> Box<dyn Contexte> {
        Box::new(Parametres::default())
    }

    fn frappe(&mut self, f: FenetreId, cible: Id, frappe: Frappe, c: &mut Commun) -> bool {
        if cible != sub_id(ESSAI, f as usize) {
            return false;
        }
        if self.essai.appliquer(frappe) {
            c.clavier.fermer();
        }
        true
    }

    fn dessiner(&mut self, cx: &mut Cx, c: &mut Commun, p: &Placee) -> String {
        let r = p.contenu;
        cx.dl.rrect(r, 0.0, cx.couleurs().surface);
        let scroll = sub_id(PARAMS, p.id as usize);
        let t = &mut c.theme;
        let mut b = cx.body(scroll, r, &c.scrolls);
        cx.section(&mut b, "COMMANDES");
        let shown = format!("{:.0} dp", t.commande);
        cx.slider(&mut b, sub_id(scroll, 0), "Taille des commandes", &shown, &mut t.commande, 36.0, 64.0);
        t.commande = t.commande.round();
        let shown = format!("{:.0} %", t.arrondi * 100.0);
        cx.slider(&mut b, sub_id(scroll, 1), "Arrondi", &shown, &mut t.arrondi, 0.0, 1.0);
        let shown = if t.marge < 0.5 { "aplati".to_string() } else { format!("{:.0} dp", t.marge) };
        cx.slider(&mut b, sub_id(scroll, 30), "Marges", &shown, &mut t.marge, 0.0, 20.0);
        t.marge = t.marge.round();
        let shown = format!("{:.0} dp", t.padding);
        cx.slider(&mut b, sub_id(scroll, 31), "Padding", &shown, &mut t.padding, 4.0, 24.0);
        t.padding = t.padding.round();
        cx.section(&mut b, "LECTURE");
        let shown = format!("{:.0} %", t.texte * 100.0);
        cx.slider(&mut b, sub_id(scroll, 2), "Taille du texte", &shown, &mut t.texte, 0.85, 1.3);
        t.texte = (t.texte * 20.0).round() / 20.0;
        let shown = format!("{:.0} dp", t.ligne);
        cx.slider(&mut b, sub_id(scroll, 3), "Hauteur des lignes", &shown, &mut t.ligne, 40.0, 64.0);
        t.ligne = t.ligne.round();
        cx.section(&mut b, "COULEURS");
        cx.toggle(&mut b, sub_id(scroll, 4), "Thème sombre", &mut t.sombre);
        for (i, (nom, clair, fonce)) in ACCENTS.iter().enumerate() {
            let teinte = if t.sombre { *fonce } else { *clair };
            if cx.row(&mut b, sub_id(scroll, 10 + i), nom, Some(teinte), i == t.accent) {
                t.accent = i;
            }
        }
        if cx.button(&mut b, sub_id(scroll, 5), "Rétablir l'apparence") {
            *t = Theme::default();
        }
        cx.section(&mut b, "CLAVIER");
        let k = &mut c.clavier.reglages;
        for (i, disp) in Disposition::TOUTES.iter().enumerate() {
            if cx.row(&mut b, sub_id(scroll, 20 + i), disp.nom(), None, *disp == k.disposition) {
                k.disposition = *disp;
            }
        }
        let shown = format!("{:.0} dp", k.touche);
        cx.slider(&mut b, sub_id(scroll, 6), "Hauteur des touches", &shown, &mut k.touche, 40.0, 64.0);
        k.touche = k.touche.round();
        let shown = format!("{:.0} ms", k.appui_long * 1000.0);
        cx.slider(&mut b, sub_id(scroll, 7), "Appui long (variantes)", &shown, &mut k.appui_long, 0.2, 0.8);
        k.appui_long = (k.appui_long * 100.0).round() / 100.0;
        cx.toggle(&mut b, sub_id(scroll, 8), "Aperçu des touches", &mut k.apercu);
        cx.toggle(&mut b, sub_id(scroll, 9), "Clavier flottant", &mut k.flottant);
        clavier::champ(cx, &mut b, sub_id(ESSAI, p.id as usize), "Essayer le clavier…", &mut self.essai, &mut c.clavier);
        cx.section(&mut b, "FICHIERS");
        let ou = match &c.fichier_reglages {
            Some(f) => format!("Ces réglages sont enregistrés dans {f}, un fichier texte qu'on peut modifier à la main."),
            None => "Ces réglages ne sont pas enregistrés.".to_string(),
        };
        cx.paragraph(&mut b, &ou);
        if let Some(f) = &c.fichier_espace {
            cx.paragraph(&mut b, &format!("Les fenêtres et leur contenu sont enregistrés dans {f}."));
        }
        cx.body_end(b, &mut c.scrolls);
        "Apparence de l'interface".to_string()
    }
}
