//! Tactile : à qui appartient chaque doigt.
//!
//! Chaque doigt reste à ce qu'il a touché en premier, jusqu'à ce qu'il se lève.
//! Les zones touchables sont celles déclarées à l'image précédente, dernière
//! déclarée au-dessus. Un doigt qui ne touche aucune zone part vers l'espace de
//! travail, où il devient un geste : un doigt tourne, deux doigts zooment et déplacent.

use crate::draw::Rect;
use std::collections::HashMap;

pub type Id = u64;

/// Identifiant stable tiré d'un texte (FNV-1a).
pub const fn id(s: &str) -> Id {
    let b = s.as_bytes();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut i = 0;
    while i < b.len() {
        h = (h ^ b[i] as u64).wrapping_mul(0x0000_0100_0000_01B3);
        i += 1;
    }
    h
}

pub fn sub_id(parent: Id, i: usize) -> Id {
    (parent ^ i as u64).wrapping_mul(0x0000_0100_0000_01B3)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    Down { id: i32, x: f32, y: f32 },
    Move { id: i32, x: f32, y: f32 },
    Up { id: i32 },
    Cancel,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Sense {
    /// Appui bref. Si `scroll` est donné, un glissement passe à cette zone défilante.
    Click { scroll: Option<Id> },
    /// Le doigt pilote directement la zone (curseur, poignée).
    Drag,
    Scroll,
    /// Absorbe le doigt sans rien faire : le fond d'un panneau.
    Sink,
}

#[derive(Clone, Copy)]
struct Region {
    id: Id,
    rect: Rect,
    clip: Option<Rect>,
    sense: Sense,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Owner {
    Widget(Id),
    Scroll(Id),
    Workspace,
    Nobody,
}

#[derive(Clone, Copy)]
struct Pointer {
    id: i32,
    x: f32,
    y: f32,
    start: (f32, f32),
    owner: Owner,
    slid: bool,
}

#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Drag {
    pub x: f32,
    pub y: f32,
    pub moved: bool,
}

/// Geste accumulé sur l'espace de travail depuis la dernière image.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gesture {
    pub orbit: (f32, f32),
    pub pan: (f32, f32),
    pub zoom: f32,
    /// Où le geste a commencé (le premier doigt posé), pour savoir quelle vue il pilote.
    pub at: Option<(f32, f32)>,
}

impl Default for Gesture {
    fn default() -> Gesture {
        Gesture { orbit: (0.0, 0.0), pan: (0.0, 0.0), zoom: 1.0, at: None }
    }
}

const DOUBLE_TAP_S: f64 = 0.35;

pub struct Ui {
    pub dp: f32,
    regions: Vec<Region>,
    next: Vec<Region>,
    clip: Option<Rect>,
    pointers: Vec<Pointer>,
    clicks: Vec<Id>,
    /// Où chaque clic de l'image s'est levé.
    clic_pos: Vec<(Id, f32, f32)>,
    double_taps: Vec<Id>,
    last_tap: Option<(Id, f64)>,
    drags: HashMap<Id, Drag>,
    scrolls: HashMap<Id, f32>,
    gesture: Gesture,
    /// Un événement vient d'être traité : voir `a_redessiner`.
    agite: bool,
}

impl Ui {
    pub fn new(dp: f32) -> Ui {
        Ui {
            dp,
            regions: Vec::new(),
            next: Vec::new(),
            clip: None,
            pointers: Vec::new(),
            clicks: Vec::new(),
            clic_pos: Vec::new(),
            double_taps: Vec::new(),
            last_tap: None,
            drags: HashMap::new(),
            scrolls: HashMap::new(),
            gesture: Gesture::default(),
            agite: false,
        }
    }

    fn slop(&self) -> f32 {
        8.0 * self.dp
    }

    fn hit(&self, x: f32, y: f32) -> Option<Region> {
        self.regions.iter().rev().find(|r| r.rect.contains(x, y) && r.clip.is_none_or(|c| c.contains(x, y))).copied()
    }

    fn region(&self, id: Id) -> Option<Region> {
        self.regions.iter().rev().find(|r| r.id == id).copied()
    }

    /// `t` : instant de l'événement, en secondes.
    pub fn event(&mut self, ev: Event, t: f64) {
        self.agite = true;
        match ev {
            Event::Down { id, x, y } => {
                let owner = match self.hit(x, y) {
                    None => Owner::Workspace,
                    Some(r) => match r.sense {
                        Sense::Click { .. } => Owner::Widget(r.id),
                        Sense::Drag => {
                            self.drags.insert(r.id, Drag { x, y, moved: false });
                            Owner::Widget(r.id)
                        }
                        Sense::Scroll => Owner::Scroll(r.id),
                        Sense::Sink => Owner::Nobody,
                    },
                };
                self.pointers.retain(|p| p.id != id);
                self.pointers.push(Pointer { id, x, y, start: (x, y), owner, slid: false });
            }
            Event::Move { id, x, y } => self.moved(id, x, y),
            Event::Up { id } => {
                if let Some(i) = self.pointers.iter().position(|p| p.id == id) {
                    let p = self.pointers.remove(i);
                    self.released(&p, t);
                }
            }
            Event::Cancel => self.pointers.clear(),
        }
    }

    fn moved(&mut self, id: i32, x: f32, y: f32) {
        let Some(i) = self.pointers.iter().position(|p| p.id == id) else { return };
        let (ox, oy) = (self.pointers[i].x, self.pointers[i].y);
        let (sx, sy) = self.pointers[i].start;
        self.pointers[i].x = x;
        self.pointers[i].y = y;
        if !self.pointers[i].slid && (x - sx).hypot(y - sy) > self.slop() {
            self.pointers[i].slid = true;
        }
        let slid = self.pointers[i].slid;
        match self.pointers[i].owner {
            Owner::Widget(w) => match self.region(w).map(|r| r.sense) {
                Some(Sense::Click { scroll: Some(s) }) if slid => self.pointers[i].owner = Owner::Scroll(s),
                Some(Sense::Drag) => {
                    self.drags.insert(w, Drag { x, y, moved: slid });
                }
                _ => {}
            },
            Owner::Scroll(s) if slid => *self.scrolls.entry(s).or_insert(0.0) += y - oy,
            Owner::Workspace => self.workspace_moved(i, ox, oy),
            _ => {}
        }
    }

    fn workspace_moved(&mut self, i: usize, ox: f32, oy: f32) {
        let me = self.pointers[i];
        let others: Vec<Pointer> = self.pointers.iter().filter(|p| p.owner == Owner::Workspace && p.id != me.id).copied().collect();
        if self.gesture.at.is_none() && (me.slid || !others.is_empty()) {
            self.gesture.at = Some(others.first().map_or(me.start, |o| o.start));
        }
        match others.first() {
            None if me.slid => {
                self.gesture.orbit.0 += me.x - ox;
                self.gesture.orbit.1 += me.y - oy;
            }
            None => {}
            Some(o) => {
                let before = (ox - o.x).hypot(oy - o.y).max(1.0);
                let after = (me.x - o.x).hypot(me.y - o.y).max(1.0);
                self.gesture.zoom *= after / before;
                self.gesture.pan.0 += (me.x - ox) / 2.0;
                self.gesture.pan.1 += (me.y - oy) / 2.0;
            }
        }
    }

    fn released(&mut self, p: &Pointer, t: f64) {
        let Owner::Widget(w) = p.owner else { return };
        if matches!(self.region(w).map(|r| r.sense), Some(Sense::Drag)) {
            self.drags.remove(&w);
        }
        if p.slid {
            return;
        }
        self.clicks.push(w);
        self.clic_pos.push((w, p.x, p.y));
        match self.last_tap {
            Some((prev, at)) if prev == w && t - at < DOUBLE_TAP_S => {
                self.double_taps.push(w);
                self.last_tap = None;
            }
            _ => self.last_tap = Some((w, t)),
        }
    }

    /// Vrai une fois après une image qui a reçu un événement. Un widget apprend qu'il
    /// est touché en se dessinant : l'état change donc après son dessin, et sans une
    /// image de plus, l'écran resterait en retard jusqu'au prochain événement.
    pub fn a_redessiner(&mut self) -> bool {
        std::mem::take(&mut self.agite)
    }

    pub fn set_clip(&mut self, clip: Option<Rect>) {
        self.clip = clip;
    }

    pub fn region_add(&mut self, id: Id, rect: Rect, sense: Sense) {
        self.next.push(Region { id, rect, clip: self.clip, sense });
    }

    /// Zone cliquable ; vrai à l'image qui suit l'appui.
    pub fn click(&mut self, id: Id, rect: Rect, scroll: Option<Id>) -> bool {
        self.region_add(id, rect, Sense::Click { scroll });
        self.clicked(id)
    }

    pub fn clicked(&self, id: Id) -> bool {
        self.clicks.contains(&id)
    }

    /// Où le doigt s'est levé, si `id` vient d'être cliqué.
    pub fn clic_en(&self, id: Id) -> Option<(f32, f32)> {
        self.clic_pos.iter().rev().find(|c| c.0 == id).map(|c| (c.1, c.2))
    }

    pub fn double_tapped(&self, id: Id) -> bool {
        self.double_taps.contains(&id)
    }

    /// Zone pilotée au doigt : position courante du doigt tant qu'il est posé.
    pub fn drag(&mut self, id: Id, rect: Rect) -> Option<Drag> {
        self.region_add(id, rect, Sense::Drag);
        self.drags.get(&id).copied()
    }

    /// Le nombre de doigts posés.
    pub fn doigts(&self) -> usize {
        self.pointers.len()
    }

    pub fn pressed(&self, id: Id) -> bool {
        self.pointers.iter().any(|p| p.owner == Owner::Widget(id))
    }

    pub fn take_scroll(&mut self, id: Id) -> f32 {
        self.scrolls.remove(&id).unwrap_or(0.0)
    }

    pub fn take_gesture(&mut self) -> Gesture {
        std::mem::take(&mut self.gesture)
    }

    /// Fin d'image : les zones déclarées deviennent celles qui reçoivent les doigts.
    pub fn end_frame(&mut self) {
        std::mem::swap(&mut self.regions, &mut self.next);
        self.next.clear();
        self.clip = None;
        self.clicks.clear();
        self.clic_pos.clear();
        self.double_taps.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BTN: Id = id("btn");
    const SCROLL: Id = id("scroll");
    const SLIDER: Id = id("slider");

    fn ui() -> Ui {
        let mut u = Ui::new(1.0);
        u.region_add(id("panel"), Rect::new(0., 0., 300., 300.), Sense::Sink);
        u.region_add(SCROLL, Rect::new(0., 0., 300., 200.), Sense::Scroll);
        u.region_add(BTN, Rect::new(10., 10., 100., 40.), Sense::Click { scroll: Some(SCROLL) });
        u.region_add(SLIDER, Rect::new(10., 100., 100., 40.), Sense::Drag);
        u.end_frame();
        u
    }

    fn down(u: &mut Ui, i: i32, x: f32, y: f32) {
        u.event(Event::Down { id: i, x, y }, 0.0);
    }

    #[test]
    fn une_image_de_plus_suit_chaque_evenement() {
        let mut u = ui();
        assert!(!u.a_redessiner());
        down(&mut u, 0, 20., 20.);
        u.event(Event::Up { id: 0 }, 0.1);
        assert!(u.clicked(BTN));
        u.end_frame();
        assert!(u.a_redessiner());
        // L'image suivante n'a rien reçu : la boucle peut s'endormir
        assert!(!u.a_redessiner());
    }

    #[test]
    fn appui_bref_sur_un_bouton() {
        let mut u = ui();
        down(&mut u, 0, 20., 20.);
        assert!(u.pressed(BTN));
        u.event(Event::Up { id: 0 }, 0.1);
        assert!(u.clicked(BTN));
        assert!(!u.pressed(BTN));
    }

    #[test]
    fn glisser_depuis_un_bouton_fait_defiler_sans_cliquer() {
        let mut u = ui();
        down(&mut u, 0, 20., 20.);
        u.event(Event::Move { id: 0, x: 20., y: 40. }, 0.1);
        u.event(Event::Move { id: 0, x: 20., y: 60. }, 0.1);
        assert_eq!(u.take_scroll(SCROLL), 20.0);
        u.event(Event::Up { id: 0 }, 0.2);
        assert!(!u.clicked(BTN));
    }

    #[test]
    fn curseur_capture_son_doigt_hors_de_la_zone() {
        let mut u = ui();
        down(&mut u, 0, 20., 110.);
        assert_eq!(u.drags.get(&SLIDER).map(|d| d.x), Some(20.));
        u.event(Event::Move { id: 0, x: 900., y: 900. }, 0.1);
        assert_eq!(u.drags.get(&SLIDER).map(|d| d.x), Some(900.));
        u.event(Event::Up { id: 0 }, 0.2);
        assert!(u.drags.is_empty());
    }

    #[test]
    fn double_appui() {
        let mut u = ui();
        for t in [0.0, 0.2] {
            down(&mut u, 0, 20., 20.);
            u.event(Event::Up { id: 0 }, t);
        }
        assert!(u.double_tapped(BTN));
    }

    #[test]
    fn un_doigt_tourne_deux_doigts_zooment() {
        let mut u = ui();
        down(&mut u, 0, 500., 500.);
        u.event(Event::Move { id: 0, x: 530., y: 500. }, 0.1);
        u.event(Event::Move { id: 0, x: 540., y: 510. }, 0.1);
        let g = u.take_gesture();
        assert_eq!(g.orbit, (40., 10.)); // le geste démarre avec le mouvement qui franchit le seuil
        assert_eq!(g.at, Some((500., 500.)));
        down(&mut u, 1, 640., 510.);
        u.event(Event::Move { id: 1, x: 740., y: 510. }, 0.2);
        let g = u.take_gesture();
        assert_eq!(g.orbit, (0., 0.));
        assert!((g.zoom - 2.0).abs() < 1e-5);
    }

    #[test]
    fn le_fond_d_un_panneau_ne_touche_pas_la_3d() {
        let mut u = ui();
        down(&mut u, 0, 250., 250.);
        u.event(Event::Move { id: 0, x: 290., y: 290. }, 0.1);
        assert_eq!(u.take_gesture(), Gesture::default());
    }

    #[test]
    fn deux_doigts_deux_boutons() {
        let mut u = ui();
        down(&mut u, 0, 20., 20.);
        down(&mut u, 1, 20., 110.);
        assert!(u.pressed(BTN) && u.pressed(SLIDER));
    }
}
