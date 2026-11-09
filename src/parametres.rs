//! Le panneau des paramètres de l'application : l'apparence de l'interface et le
//! clavier virtuel, appliqués dès l'image suivante et enregistrés dans le fichier des
//! réglages (`reglages`). Ce sont des réglages globaux : ils vivent dans le panneau
//! latéral, pas dans un contexte.

use crate::clavier::{self, Disposition, Edition};
use crate::contexte::Commun;
use crate::ui::{id, sub_id, Id};
use crate::widgets::{Body, Cx, Theme, ACCENTS};

/// Les onglets du panneau.
pub const ONGLETS: [&str; 2] = ["Interface", "Clavier"];

/// Le champ d'essai du clavier.
pub const ESSAI: Id = id("essai-clavier");

/// Le contenu de l'onglet `onglet`, dans le corps `b` (identifiant `scroll`).
pub fn panneau(cx: &mut Cx, b: &mut Body, scroll: Id, onglet: usize, c: &mut Commun, essai: &mut Edition) {
    if onglet == 0 {
        interface(cx, b, scroll, c);
    } else {
        clavier(cx, b, scroll, c, essai);
    }
    cx.section(b, "FICHIERS");
    let ou = match &c.fichier_reglages {
        Some(f) => format!("Ces réglages sont enregistrés dans {f}, un fichier texte qu'on peut modifier à la main."),
        None => "Ces réglages ne sont pas enregistrés.".to_string(),
    };
    cx.paragraph(b, &ou);
    if let Some(f) = &c.fichier_espace {
        cx.paragraph(b, &format!("Les fenêtres et leur contenu sont enregistrés dans {f}."));
    }
}

fn interface(cx: &mut Cx, b: &mut Body, scroll: Id, c: &mut Commun) {
    let t = &mut c.theme;
    cx.section(b, "COMMANDES");
    let shown = format!("{:.0} dp", t.commande);
    cx.slider(b, sub_id(scroll, 0), "Taille des commandes", &shown, &mut t.commande, 36.0, 64.0);
    t.commande = t.commande.round();
    let shown = format!("{:.0} %", t.arrondi * 100.0);
    cx.slider(b, sub_id(scroll, 1), "Arrondi", &shown, &mut t.arrondi, 0.0, 1.0);
    let shown = if t.marge < 0.5 { "aplati".to_string() } else { format!("{:.0} dp", t.marge) };
    cx.slider(b, sub_id(scroll, 30), "Marges", &shown, &mut t.marge, 0.0, 20.0);
    t.marge = t.marge.round();
    let shown = format!("{:.0} dp", t.padding);
    cx.slider(b, sub_id(scroll, 31), "Padding", &shown, &mut t.padding, 4.0, 24.0);
    t.padding = t.padding.round();
    cx.section(b, "LECTURE");
    let shown = format!("{:.0} %", t.texte * 100.0);
    cx.slider(b, sub_id(scroll, 2), "Taille du texte", &shown, &mut t.texte, 0.85, 1.3);
    t.texte = (t.texte * 20.0).round() / 20.0;
    let shown = format!("{:.0} dp", t.ligne);
    cx.slider(b, sub_id(scroll, 3), "Hauteur des lignes", &shown, &mut t.ligne, 40.0, 64.0);
    t.ligne = t.ligne.round();
    cx.section(b, "COULEURS");
    cx.toggle(b, sub_id(scroll, 4), "Thème sombre", &mut t.sombre);
    for (i, (nom, clair, fonce)) in ACCENTS.iter().enumerate() {
        let teinte = if t.sombre { *fonce } else { *clair };
        if cx.row(b, sub_id(scroll, 10 + i), nom, Some(teinte), i == t.accent) {
            t.accent = i;
        }
    }
    if cx.button(b, sub_id(scroll, 5), "Rétablir l'apparence") {
        *t = Theme::default();
    }
}

fn clavier(cx: &mut Cx, b: &mut Body, scroll: Id, c: &mut Commun, essai: &mut Edition) {
    let k = &mut c.clavier.reglages;
    cx.section(b, "DISPOSITION");
    for (i, disp) in Disposition::TOUTES.iter().enumerate() {
        if cx.row(b, sub_id(scroll, 20 + i), disp.nom(), None, *disp == k.disposition) {
            k.disposition = *disp;
        }
    }
    cx.section(b, "TOUCHES");
    let shown = format!("{:.0} dp", k.touche);
    cx.slider(b, sub_id(scroll, 6), "Hauteur des touches", &shown, &mut k.touche, 40.0, 64.0);
    k.touche = k.touche.round();
    let shown = format!("{:.0} ms", k.appui_long * 1000.0);
    cx.slider(b, sub_id(scroll, 7), "Appui long (variantes)", &shown, &mut k.appui_long, 0.2, 0.8);
    k.appui_long = (k.appui_long * 100.0).round() / 100.0;
    cx.toggle(b, sub_id(scroll, 8), "Aperçu des touches", &mut k.apercu);
    cx.toggle(b, sub_id(scroll, 9), "Clavier flottant", &mut k.flottant);
    cx.section(b, "ESSAI");
    clavier::champ(cx, b, ESSAI, "Essayer le clavier…", essai, &mut c.clavier);
}
