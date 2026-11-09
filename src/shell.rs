//! Coque de l'application : espace de travail et panneaux latéraux, repris de
//! `PanneauOnglets` de GDA. En paysage, panneau de début à gauche et de fin à
//! droite ; en portrait, en haut et en bas. L'espace de travail se réduit d'autant,
//! et garde toujours au moins 30 % de l'écran.

use crate::draw::Rect;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Side {
    Start,
    End,
}

#[derive(Default)]
pub struct Panel {
    pub open: bool,
    /// Déploiement, de 0 (fermé) à 1 (ouvert).
    pub anim: f32,
    /// Taille choisie à la poignée : [paysage, portrait].
    custom: [Option<f32>; 2],
}

impl Panel {
    fn step(&mut self, dt: f32) {
        let target = if self.open { 1.0 } else { 0.0 };
        self.anim += (target - self.anim) * (1.0 - (-dt * 16.0).exp());
        if (target - self.anim).abs() < 0.004 {
            self.anim = target;
        }
    }

    fn settled(&self) -> bool {
        self.anim == if self.open { 1.0 } else { 0.0 }
    }

    pub fn toggle(&mut self) {
        self.open = !self.open;
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    pub workspace: Rect,
    pub start: Rect,
    pub end: Rect,
    pub landscape: bool,
}

pub struct Shell {
    pub start: Panel,
    pub end: Panel,
    pub dp: f32,
}

const MIN_PANEL_DP: f32 = 200.0;
const HANDLE_DP: f32 = 28.0;

impl Shell {
    pub fn new(dp: f32) -> Shell {
        Shell { start: Panel::default(), end: Panel::default(), dp }
    }

    pub fn animating(&self) -> bool {
        !self.start.settled() || !self.end.settled()
    }

    /// Tailles des panneaux une fois ouverts.
    fn sizes(&self, screen: Rect) -> (f32, f32) {
        let landscape = screen.w >= screen.h;
        let (avail, def_start, def_end) = if landscape {
            (screen.w, (288.0 * self.dp).min(0.30 * screen.w), (384.0 * self.dp).min(0.40 * screen.w))
        } else {
            (screen.h, (320.0 * self.dp).min(0.32 * screen.h), (448.0 * self.dp).min(0.45 * screen.h))
        };
        let both = self.start.open && self.end.open;
        let (max_start, max_end) = if both { (0.28, 0.42) } else { (0.70, 0.70) };
        let o = if landscape { 0 } else { 1 };
        (self.start.custom[o].unwrap_or(def_start).min(max_start * avail), self.end.custom[o].unwrap_or(def_end).min(max_end * avail))
    }

    /// Avance les animations de `dt` secondes et place chaque zone.
    pub fn layout(&mut self, screen: Rect, dt: f32) -> Layout {
        self.start.step(dt);
        self.end.step(dt);
        let (s, e) = self.sizes(screen);
        let (shown_s, shown_e) = (s * self.start.anim, e * self.end.anim);
        if screen.w >= screen.h {
            Layout {
                workspace: Rect::new(screen.x + shown_s, screen.y, screen.w - shown_s - shown_e, screen.h),
                start: Rect::new(screen.x + shown_s - s, screen.y, s, screen.h),
                end: Rect::new(screen.right() - shown_e, screen.y, e, screen.h),
                landscape: true,
            }
        } else {
            Layout {
                workspace: Rect::new(screen.x, screen.y + shown_s, screen.w, screen.h - shown_s - shown_e),
                start: Rect::new(screen.x, screen.y + shown_s - s, screen.w, s),
                end: Rect::new(screen.x, screen.bottom() - shown_e, screen.w, e),
                landscape: false,
            }
        }
    }

    /// Bande tactile à cheval sur la frontière du panneau.
    pub fn handle(&self, l: &Layout, side: Side) -> Rect {
        let band = HANDLE_DP * self.dp;
        let w = l.workspace;
        match (side, l.landscape) {
            (Side::Start, true) => Rect::new(w.x - band / 2.0, w.y, band, w.h),
            (Side::End, true) => Rect::new(w.right() - band / 2.0, w.y, band, w.h),
            (Side::Start, false) => Rect::new(w.x, w.y - band / 2.0, w.w, band),
            (Side::End, false) => Rect::new(w.x, w.bottom() - band / 2.0, w.w, band),
        }
    }

    /// Le doigt tient la poignée en (x, y).
    pub fn resize(&mut self, side: Side, screen: Rect, x: f32, y: f32) {
        let landscape = screen.w >= screen.h;
        let size = match (side, landscape) {
            (Side::Start, true) => x - screen.x,
            (Side::End, true) => screen.right() - x,
            (Side::Start, false) => y - screen.y,
            (Side::End, false) => screen.bottom() - y,
        };
        let panel = if side == Side::Start { &mut self.start } else { &mut self.end };
        panel.custom[if landscape { 0 } else { 1 }] = Some(size.max(MIN_PANEL_DP * self.dp));
    }

    /// Double appui sur la poignée : retour à la taille par défaut de cette orientation.
    pub fn reset_size(&mut self, side: Side, screen: Rect) {
        let panel = if side == Side::Start { &mut self.start } else { &mut self.end };
        panel.custom[if screen.w >= screen.h { 0 } else { 1 }] = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LAND: Rect = Rect::new(0., 0., 1920., 1200.);
    const PORT: Rect = Rect::new(0., 0., 1200., 1920.);

    fn opened(start: bool, end: bool) -> Shell {
        let mut s = Shell::new(1.5);
        s.start.open = start;
        s.end.open = end;
        s
    }

    fn settle(s: &mut Shell, screen: Rect) -> Layout {
        let mut l = s.layout(screen, 0.016);
        for _ in 0..200 {
            l = s.layout(screen, 0.016);
        }
        assert!(!s.animating());
        l
    }

    #[test]
    fn fermes_l_espace_prend_tout() {
        assert_eq!(settle(&mut opened(false, false), LAND).workspace, LAND);
    }

    #[test]
    fn panneaux_et_espace_pavent_l_ecran() {
        for (screen, x) in [(LAND, true), (PORT, false)] {
            let l = settle(&mut opened(true, true), screen);
            let w = l.workspace;
            if x {
                assert!((l.start.right() - w.x).abs() < 0.01 && (w.right() - l.end.x).abs() < 0.01);
            } else {
                assert!((l.start.bottom() - w.y).abs() < 0.01 && (w.bottom() - l.end.y).abs() < 0.01);
            }
        }
    }

    #[test]
    fn l_espace_garde_30_pourcent_meme_tire_au_maximum() {
        for screen in [LAND, PORT] {
            let mut s = opened(true, true);
            s.resize(Side::Start, screen, 1900., 1900.);
            s.resize(Side::End, screen, 0., 0.);
            let l = settle(&mut s, screen);
            assert!(l.workspace.w * l.workspace.h >= 0.299 * screen.w * screen.h, "{l:?}");
            s.start.open = false;
            let l = settle(&mut s, screen);
            assert!(l.workspace.w * l.workspace.h >= 0.299 * screen.w * screen.h, "{l:?}");
        }
    }

    #[test]
    fn taille_par_orientation_et_double_appui() {
        let mut s = opened(false, true);
        s.resize(Side::End, LAND, 1000., 0.);
        assert_eq!(settle(&mut s, LAND).end.w, 920.);
        assert_eq!(settle(&mut s, PORT).end.h, 672.); // l'autre orientation n'a pas bougé
        s.reset_size(Side::End, LAND);
        assert!(settle(&mut s, LAND).end.w < 768.1);
    }

    #[test]
    fn poignee_ne_descend_pas_sous_le_minimum() {
        let mut s = opened(false, true);
        s.resize(Side::End, LAND, 1919., 0.);
        assert_eq!(settle(&mut s, LAND).end.w, 300.);
    }
}
