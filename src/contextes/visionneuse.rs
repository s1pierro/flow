//! La visionneuse d'images de Fichiers : l'image touchée, sur toute la fenêtre, et
//! les autres images du dossier. Le pincement zoome (autour des doigts), le glissement
//! déplace l'image agrandie ou, à sa taille, passe à la suivante ou à la précédente ;
//! un toucher sur un bord en fait autant, au milieu il agrandit ou revient à la
//! taille. Une barre en haut : fermer, le nom, la place dans le dossier.

use crate::draw::{Image, Rect};
use crate::gestes::Geste;
use crate::images::{self, Reponse};
use crate::ui::{sub_id, Id};
use crate::widgets::{Cx, Icon};
use std::path::PathBuf;
use std::time::SystemTime;

/// Le plus grand côté des images montrées en grand, en pixels.
pub const GRANDE: u32 = 2560;
/// Celui des vignettes des cartes.
pub const VIGNETTE: u32 = 256;

#[derive(Clone, Debug, PartialEq)]
pub struct Visionneuse {
    /// Les images du dossier, dans l'ordre de la liste, et leur date.
    pub images: Vec<(PathBuf, Option<SystemTime>)>,
    pub i: usize,
    /// L'agrandissement (1 : l'image tient dans la fenêtre) et le décalage de son
    /// centre par rapport à celui de la fenêtre, en pixels.
    zoom: f32,
    decalage: (f32, f32),
    pincement: f32,
    /// Le glissement en cours : son départ et le dernier point.
    glisse: Option<((f32, f32), (f32, f32))>,
}

/// Ce que la visionneuse demande à Fichiers.
pub enum Issue {
    Rien,
    Fermer,
}

impl Visionneuse {
    pub fn new(images: Vec<(PathBuf, Option<SystemTime>)>, i: usize) -> Visionneuse {
        Visionneuse { i: i.min(images.len().saturating_sub(1)), images, zoom: 1.0, decalage: (0.0, 0.0), pincement: 1.0, glisse: None }
    }

    fn aller(&mut self, pas: isize) {
        let n = self.images.len() as isize;
        if n == 0 {
            return;
        }
        let j = self.i as isize + pas;
        if (0..n).contains(&j) {
            self.i = j as usize;
            self.zoom = 1.0;
            self.decalage = (0.0, 0.0);
        }
    }

    /// La place de l'image dans la zone : ajustée, agrandie, décalée.
    fn place(&self, zone: Rect, l: u32, h: u32) -> Rect {
        let (l, h) = (l.max(1) as f32, h.max(1) as f32);
        let s = (zone.w / l).min(zone.h / h) * self.zoom;
        let (w, hh) = (l * s, h * s);
        Rect::new(zone.cx() + self.decalage.0 - w / 2.0, zone.cy() + self.decalage.1 - hh / 2.0, w, hh)
    }

    /// Garde l'image agrandie sur la zone : pas de vide d'un côté s'il y a de l'image
    /// de l'autre.
    fn borner(&mut self, zone: Rect, l: u32, h: u32) {
        let r = self.place(zone, l, h);
        let bx = ((r.w - zone.w) / 2.0).max(0.0);
        let by = ((r.h - zone.h) / 2.0).max(0.0);
        self.decalage = (self.decalage.0.clamp(-bx, bx), self.decalage.1.clamp(-by, by));
    }

    fn recevoir(&mut self, g: Geste, zone: Rect, taille: (u32, u32)) {
        match g {
            Geste::Tap { x, y, .. } => {
                let bord = zone.w * 0.2;
                if x < zone.x + bord && self.zoom <= 1.0 {
                    self.aller(-1);
                } else if x > zone.right() - bord && self.zoom <= 1.0 {
                    self.aller(1);
                } else if self.zoom > 1.0 {
                    self.zoom = 1.0;
                    self.decalage = (0.0, 0.0);
                } else {
                    // Agrandir autour du point touché
                    let k = 2.5;
                    self.decalage = ((zone.cx() - x) * (k - 1.0), (zone.cy() - y) * (k - 1.0));
                    self.zoom = k;
                }
            }
            Geste::SaisieDebut { doigt, .. } => self.glisse = Some((doigt, doigt)),
            Geste::SaisieDeplace { doigt, .. } => {
                if let Some((depart, avant)) = self.glisse {
                    if self.zoom > 1.0 {
                        self.decalage.0 += doigt.0 - avant.0;
                        self.decalage.1 += doigt.1 - avant.1;
                    }
                    self.glisse = Some((depart, doigt));
                }
            }
            Geste::SaisieFin { .. } | Geste::SaisieAnnulee { .. } => {
                if let Some((depart, fin)) = self.glisse.take() {
                    let dx = fin.0 - depart.0;
                    if self.zoom <= 1.0 && dx.abs() > zone.w * 0.12 && dx.abs() > (fin.1 - depart.1).abs() {
                        self.aller(if dx < 0.0 { 1 } else { -1 });
                    }
                }
            }
            Geste::PincementDebut { .. } => self.pincement = 1.0,
            Geste::Pincement { echelle, centre, deplacement } => {
                let k = echelle / self.pincement.max(1e-3);
                self.pincement = echelle;
                let z = (self.zoom * k).clamp(1.0, 12.0);
                let k = z / self.zoom;
                // Le point sous les doigts reste sous les doigts
                let (cx, cy) = (centre.0 - zone.cx(), centre.1 - zone.cy());
                self.decalage = ((self.decalage.0 - cx) * k + cx + deplacement.0, (self.decalage.1 - cy) * k + cy + deplacement.1);
                self.zoom = z;
            }
            Geste::PincementFin { .. } => {
                if self.zoom <= 1.01 {
                    self.zoom = 1.0;
                    self.decalage = (0.0, 0.0);
                }
            }
            _ => {}
        }
        self.borner(zone, taille.0, taille.1);
    }

    /// Dessine la visionneuse sur `r` ; la ligne d'état et ce qu'il faut faire.
    pub fn dessiner(&mut self, cx: &mut Cx, r: Rect, base: Id) -> (String, Issue) {
        let (d, col) = (cx.dp, cx.couleurs());
        let Some((chemin, date)) = self.images.get(self.i).cloned() else { return (String::new(), Issue::Fermer) };
        cx.dl.rrect(r, 0.0, [0x0C, 0x0C, 0x10, 255]);
        let h = cx.commande();
        let barre = Rect::new(r.x, r.y, r.w, h.min(r.h));
        let zone = Rect::new(r.x, barre.bottom(), r.w, (r.bottom() - barre.bottom()).max(0.0));
        // Les voisines d'abord : la courante, demandée en dernier, passe devant
        for j in [self.i + 1, self.i.wrapping_sub(1)] {
            if let Some((p, t)) = self.images.get(j) {
                images::demander(p, *t, GRANDE);
            }
        }
        let grande = images::demander(&chemin, date, GRANDE);
        let (image, taille, etat): (Option<Image>, (u32, u32), String) = match grande {
            Reponse::Prete(i, o) => {
                let t = (i.largeur, i.hauteur);
                (Some(i), t, format!("{} × {}", o.0, o.1))
            }
            Reponse::Attente => match images::demander(&chemin, date, VIGNETTE) {
                // La vignette en attendant, étirée
                Reponse::Prete(i, o) => {
                    let t = (i.largeur, i.hauteur);
                    (Some(i), t, format!("{} × {} · chargement…", o.0, o.1))
                }
                _ => (None, (1, 1), "chargement…".into()),
            },
            Reponse::Erreur(e) => (None, (1, 1), e),
        };
        // Les doigts
        let gk = sub_id(base, 1);
        for g in cx.ui.gestes(gk, zone) {
            self.recevoir(g, zone, taille);
        }
        cx.clip(Some(zone));
        match image {
            Some(i) => {
                let p = self.place(zone, taille.0, taille.1);
                cx.dl.image(p, i);
            }
            None => cx.text_at(zone.x + cx.pad(), zone.cy(), &etat, 15.0, [0xCC, 0xCC, 0xD6, 255]),
        }
        cx.clip(None);
        // La barre : fermer, le nom, précédente et suivante
        cx.dl.rrect(barre, 0.0, [0x0C, 0x0C, 0x10, 230]);
        let blanc = [0xEE, 0xEE, 0xF4, 255];
        let mut issue = Issue::Rien;
        let bouton = |cx: &mut Cx, i: usize, x: f32, icone: Icon, actif: bool| -> bool {
            let b = Rect::new(x, barre.y, h, h);
            let k = sub_id(base, 10 + i);
            let clic = cx.ui.click(k, b, None);
            if cx.ui.pressed(k) && actif {
                cx.dl.rrect(b.inset(4.0 * d), cx.theme.rayon(h) * 0.5, [255, 255, 255, 40]);
            }
            cx.icon(b.inset((cx.pad() * 0.6).min(h * 0.3)), icone, if actif { blanc } else { [blanc[0], blanc[1], blanc[2], 90] });
            clic && actif
        };
        if bouton(cx, 0, barre.x, Icon::Close, true) {
            issue = Issue::Fermer;
        }
        let n = self.images.len();
        if bouton(cx, 2, barre.right() - h, Icon::Suivant, self.i + 1 < n) {
            self.aller(1);
        }
        if bouton(cx, 1, barre.right() - 2.0 * h, Icon::Precedent, self.i > 0) {
            self.aller(-1);
        }
        let nom = chemin.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned());
        let x = barre.x + h + 4.0 * d;
        let place = (barre.right() - 2.0 * h - x - 8.0 * d).max(0.0);
        cx.clip(Some(Rect::new(x, barre.y, place, barre.h)));
        let nom = super::abreger(cx, &nom, 15.0, place);
        cx.text_at(x, barre.cy(), &nom, 15.0, blanc);
        cx.clip(None);
        let _ = col;
        (format!("{} / {n} · {etat}{}", self.i + 1, if self.zoom > 1.0 { format!(" · ×{:.1}", self.zoom) } else { String::new() }), issue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_visionneuse_zoome_se_borne_et_passe_d_une_image_a_l_autre() {
        let l = vec![(PathBuf::from("/a.jpg"), None), (PathBuf::from("/b.jpg"), None), (PathBuf::from("/c.jpg"), None)];
        let mut v = Visionneuse::new(l, 1);
        let z = Rect::new(0.0, 0.0, 1000.0, 500.0);
        // Ajustée : une image 2:1 remplit la zone
        assert_eq!(v.place(z, 2000, 1000), z);
        // Un toucher au bord droit : la suivante ; au gauche, deux fois : la première
        v.recevoir(Geste::Tap { x: 990.0, y: 250.0, intensite: 0.0, precision: 0.0 }, z, (2000, 1000));
        assert_eq!(v.i, 2);
        v.recevoir(Geste::Tap { x: 10.0, y: 250.0, intensite: 0.0, precision: 0.0 }, z, (2000, 1000));
        v.recevoir(Geste::Tap { x: 10.0, y: 250.0, intensite: 0.0, precision: 0.0 }, z, (2000, 1000));
        v.recevoir(Geste::Tap { x: 10.0, y: 250.0, intensite: 0.0, precision: 0.0 }, z, (2000, 1000));
        assert_eq!(v.i, 0);
        // Au milieu : agrandie autour du point, puis bornée ; un second toucher revient
        v.recevoir(Geste::Tap { x: 500.0, y: 250.0, intensite: 0.0, precision: 0.0 }, z, (2000, 1000));
        assert_eq!(v.zoom, 2.5);
        v.decalage = (5000.0, 0.0);
        v.borner(z, 2000, 1000);
        assert_eq!(v.decalage.0, 750.0);
        v.recevoir(Geste::Tap { x: 500.0, y: 250.0, intensite: 0.0, precision: 0.0 }, z, (2000, 1000));
        assert_eq!((v.zoom, v.decalage), (1.0, (0.0, 0.0)));
        // Un glissement vers la gauche, à la taille : la suivante
        v.recevoir(Geste::SaisieDebut { x: 800.0, y: 250.0, doigt: (800.0, 250.0) }, z, (2000, 1000));
        v.recevoir(Geste::SaisieDeplace { x: 800.0, y: 250.0, doigt: (500.0, 260.0) }, z, (2000, 1000));
        v.recevoir(Geste::SaisieFin { x: 800.0, y: 250.0, depart: (800.0, 250.0), vecteur: (-300.0, 10.0) }, z, (2000, 1000));
        assert_eq!(v.i, 1);
    }
}
