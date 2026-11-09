//! La liste des contextes de Flow, et les outils qu'ils partagent (lignes de liste,
//! empreintes de dossiers).
//!
//! Pour ajouter un contexte : un module ici, et une ligne dans `GENRES`. Voir
//! `docs/contextes.md`.

pub mod aide;
pub mod disque;
pub mod fichiers;
pub mod radio;

use crate::contexte::Genre;
use crate::draw::{Color, Rect};
use crate::ui::Id;
use crate::widgets::{Body, Cx, Icon};

/// Les contextes, dans l'ordre du menu des fenêtres. Le premier est celui de la
/// première fenêtre ; leur place ne compte pas pour l'enregistrement (c'est la clé qui
/// est retenue), on peut donc les réordonner.
pub const GENRES: &[Genre] = &[fichiers::GENRE, disque::GENRE, radio::GENRE, aide::GENRE];

/// Le contexte d'une fenêtre dont la clé est inconnue (fichier d'une autre version).
pub const REPLI: &str = aide::CLE;

/// La place d'un contexte dans `GENRES`, d'après sa clé.
pub fn indice(cle: &str) -> Option<u32> {
    GENRES.iter().position(|g| g.cle == cle).map(|i| i as u32)
}

/// Le genre à la place `vue` (le repli si elle n'existe pas).
pub fn genre(vue: u32) -> &'static Genre {
    GENRES.get(vue as usize).unwrap_or_else(|| &GENRES[indice(REPLI).unwrap_or(0) as usize])
}

/// Le nom et l'icône de chaque contexte, pour la surface.
pub fn vues() -> Vec<(&'static str, Icon)> {
    GENRES.iter().map(|g| (g.nom, g.icone)).collect()
}

/// Les teintes des secteurs du premier anneau ; les anneaux suivants les éclaircissent.
pub const TEINTES: [Color; 8] = [
    [0x3B, 0x82, 0xF6, 255],
    [0xEF, 0x44, 0x44, 255],
    [0x22, 0xC5, 0x5E, 255],
    [0xF5, 0x9E, 0x0B, 255],
    [0xA8, 0x55, 0xF7, 255],
    [0x14, 0xB8, 0xA6, 255],
    [0xEC, 0x48, 0x99, 255],
    [0x84, 0xCC, 0x16, 255],
];

/// Ce qui précède le nom d'une ligne de liste : une icône ou une pastille de couleur.
pub enum Puce {
    Icone(Icon),
    Teinte(Color),
    /// L'empreinte d'un dossier : ses angles du premier anneau jusqu'à lui, et la
    /// teinte de sa branche.
    Empreinte(Vec<(f32, f32)>, Color),
}

/// Une ligne de liste de fichiers : puce, nom, et à droite une information en chasse
/// fixe. Hors de la zone visible, elle ne fait que réserver sa place.
#[allow(clippy::too_many_arguments)]
pub fn ligne_fichier(cx: &mut Cx, b: &mut Body, id: Id, puce: Puce, nom: &str, info: &str, choisie: bool) -> bool {
    let (d, col) = (cx.dp, cx.couleurs());
    let r = Rect::new(b.rect().x, b.y, b.rect().w, (cx.theme.ligne * d).round());
    b.y += r.h;
    let vue = b.rect();
    if r.bottom() < vue.y || r.y > vue.bottom() {
        return false;
    }
    let clic = cx.ui.click(id, r, Some(b.id()));
    if cx.ui.pressed(id) || choisie {
        cx.dl.rrect(r.inset(2.0 * d), cx.theme.rayon(r.h) * 0.45, col.puce);
    }
    let ic = (r.h * 0.5).round();
    match puce {
        Puce::Icone(icone) => {
            let teinte = if icone == Icon::Dossier || choisie { col.accent } else { col.pale };
            cx.icon(Rect::new(b.x, r.cy() - ic / 2.0, ic, ic), icone, teinte);
        }
        Puce::Teinte(t) => {
            let s = (ic * 0.8).round();
            cx.dl.rrect(Rect::new(b.x + (ic - s) / 2.0, r.cy() - s / 2.0, s, s), 5.0 * d * cx.theme.arrondi.max(0.2), t);
        }
        Puce::Empreinte(arcs, t) => empreinte(cx, Rect::new(b.x, r.cy() - ic / 2.0, ic, ic), &arcs, t),
    }
    let x = b.x + ic + 12.0 * d;
    let wi = if info.is_empty() { 0.0 } else { cx.mesure_mono(info, 12.0) + 12.0 * d };
    let fin = b.x + b.w - wi;
    cx.clip(Some(Rect::new(x, r.y, (fin - x).max(0.0), r.h).intersect(&vue)));
    cx.text_at(x, r.cy(), nom, 16.0, if choisie { col.accent } else { col.texte });
    cx.clip(Some(vue));
    if !info.is_empty() {
        cx.mono_at(b.x + b.w - wi + 12.0 * d, r.cy(), info, 12.0, col.pale);
    }
    clic
}

/// Niveaux montrés au plus dans une empreinte : au-delà, les plus proches du dossier.
const EMPREINTE_NIVEAUX: usize = 5;
/// Ouverture minimale d'un secteur d'empreinte, pour qu'il reste visible.
const EMPREINTE_MIN: f32 = 0.4;

/// Dessine une empreinte dans le carré `r` : un anneau par niveau, les ancêtres en
/// pâle, le dossier en pleine teinte.
pub fn empreinte(cx: &mut Cx, r: Rect, arcs: &[(f32, f32)], teinte: Color) {
    let (d, col) = (cx.dp, cx.couleurs());
    let arcs = &arcs[arcs.len().saturating_sub(EMPREINTE_NIVEAUX)..];
    if arcs.is_empty() {
        return;
    }
    let c = [r.cx(), r.cy()];
    let grand = r.w.min(r.h) / 2.0;
    let petit = grand * 0.3;
    let large = (grand - petit) / arcs.len() as f32;
    let (ecart, q, tau) = (0.8 * d, std::f32::consts::FRAC_PI_2, std::f32::consts::TAU);
    let pale = |k: f32| -> Color {
        let m = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * k) as u8;
        [m(teinte[0], col.surface[0]), m(teinte[1], col.surface[1]), m(teinte[2], col.surface[2]), 255]
    };
    for (n, &(a0, a1)) in arcs.iter().enumerate() {
        let (ra, rb) = (petit + n as f32 * large + ecart / 2.0, petit + (n + 1) as f32 * large - ecart / 2.0);
        // Le fond de l'anneau, puis le secteur, élargi s'il est trop fin
        cx.dl.secteur(c, ra, rb, -q, tau - q, 0.0, col.ligne);
        let (m, o) = ((a0 + a1) / 2.0, (a1 - a0).max(EMPREINTE_MIN) / 2.0);
        let t = if n + 1 == arcs.len() { teinte } else { pale(0.55) };
        cx.dl.secteur(c, ra, rb, m - o - q, m + o - q, 0.0, t);
    }
}

/// Un nombre tiré d'une valeur, pour donner à chaque dossier son défilement.
pub fn hachage<T: std::hash::Hash + ?Sized>(v: &T) -> usize {
    use std::hash::Hasher;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    v.hash(&mut h);
    h.finish() as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contexte::Etat;

    #[test]
    fn les_cles_sont_uniques_et_valides() {
        for (i, g) in GENRES.iter().enumerate() {
            assert!(!g.cle.is_empty() && g.cle.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_'), "{}", g.cle);
            assert_eq!(indice(g.cle), Some(i as u32), "clé en double : {}", g.cle);
            // L'instance ouverte se reconnaît à sa clé
            assert_eq!((g.ouvrir)(&Etat::default()).cle(), g.cle);
        }
        assert!(indice(REPLI).is_some());
        assert_eq!(genre(999).cle, REPLI);
    }
}
