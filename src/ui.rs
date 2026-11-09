//! Tactile : à qui appartient chaque doigt.
//!
//! Chaque doigt reste à ce qu'il a touché en premier, jusqu'à ce qu'il se lève.
//! Les zones touchables sont celles déclarées à l'image précédente, dernière
//! déclarée au-dessus. Un doigt qui ne touche aucune zone n'appartient à personne.
//! Une zone à gestes (`Ui::gestes`) reçoit des événements élaborés : tap, appui,
//! appui long, saisie à curseur déporté, pincement (voir `gestes.rs`).

use crate::draw::Rect;
use crate::gestes::{Apercu, Geste, Moteur, ReglagesGestes};
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
    /// Zone à gestes élaborés : ses doigts vont à son moteur.
    Gestes,
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
    Gestes(Id),
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
    /// Un moteur par zone à gestes, et ses événements en attente d'être lus.
    moteurs: HashMap<Id, Moteur>,
    sorties: HashMap<Id, Vec<Geste>>,
    /// Les réglages des gestes (la distance de saisie en dp, les durées).
    pub reglages_gestes: ReglagesGestes,
    /// L'instant le plus récent connu, en s.
    maintenant: f64,
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
            moteurs: HashMap::new(),
            sorties: HashMap::new(),
            reglages_gestes: ReglagesGestes::default(),
            maintenant: 0.0,
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
        self.maintenant = self.maintenant.max(t);
        match ev {
            Event::Down { id, x, y } => {
                let owner = match self.hit(x, y) {
                    None => Owner::Nobody,
                    Some(r) => match r.sense {
                        Sense::Click { .. } => Owner::Widget(r.id),
                        Sense::Drag => {
                            self.drags.insert(r.id, Drag { x, y, moved: false });
                            Owner::Widget(r.id)
                        }
                        Sense::Scroll => Owner::Scroll(r.id),
                        Sense::Gestes => {
                            let (reg, d) = (self.reglages_gestes, self.dist());
                            let m = self.moteurs.entry(r.id).or_default();
                            m.poser(id, x, y, t, &reg, d);
                            Owner::Gestes(r.id)
                        }
                        Sense::Sink => Owner::Nobody,
                    },
                };
                self.pointers.retain(|p| p.id != id);
                self.pointers.push(Pointer { id, x, y, start: (x, y), owner, slid: false });
            }
            Event::Move { id, x, y } => self.moved(id, x, y, t),
            Event::Up { id } => {
                if let Some(i) = self.pointers.iter().position(|p| p.id == id) {
                    let p = self.pointers.remove(i);
                    self.released(&p, t);
                }
            }
            Event::Cancel => {
                self.pointers.clear();
                self.moteurs.values_mut().for_each(Moteur::annuler);
            }
        }
        self.recueillir();
    }

    /// La distance de saisie, en pixels.
    fn dist(&self) -> f32 {
        self.reglages_gestes.dist * self.dp
    }

    /// Range les événements des moteurs, pour les zones qui les liront.
    fn recueillir(&mut self) {
        for (id, m) in self.moteurs.iter_mut() {
            let g = m.prendre();
            if !g.is_empty() {
                self.sorties.entry(*id).or_default().extend(g);
            }
        }
    }

    /// Le temps passe (à chaque image) : les zones à gestes passent du tap à l'appui,
    /// puis à l'appui long, sans attendre un mouvement du doigt.
    pub fn horloge(&mut self, t: f64) {
        self.maintenant = self.maintenant.max(t);
        let r = self.reglages_gestes;
        for m in self.moteurs.values_mut() {
            m.temps(self.maintenant, &r);
        }
        self.recueillir();
    }

    /// Zone à gestes élaborés : les événements reçus depuis l'image précédente.
    pub fn gestes(&mut self, id: Id, rect: Rect) -> Vec<Geste> {
        self.region_add(id, rect, Sense::Gestes);
        self.sorties.remove(&id).unwrap_or_default()
    }

    /// L'état des gestes en cours, pour leur retour visuel.
    pub fn gestes_actifs(&self) -> Vec<Apercu> {
        self.moteurs.values().filter_map(Moteur::apercu).collect()
    }

    /// Vrai si un geste en cours change d'état avec le temps : il faut redessiner.
    pub fn gestes_en_attente(&self) -> bool {
        self.moteurs.values().any(Moteur::en_attente)
    }

    fn moved(&mut self, id: i32, x: f32, y: f32, t: f64) {
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
            Owner::Gestes(g) => {
                let (reg, d) = (self.reglages_gestes, self.dist());
                if let Some(m) = self.moteurs.get_mut(&g) {
                    m.bouger(id, x, y, t, &reg, d);
                }
            }
            _ => {}
        }
        let _ = ox;
    }

    fn released(&mut self, p: &Pointer, t: f64) {
        if let Owner::Gestes(g) = p.owner {
            let reg = self.reglages_gestes;
            if let Some(m) = self.moteurs.get_mut(&g) {
                m.lever(p.id, t, &reg);
            }
            return;
        }
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

    /// Fin d'image : les zones déclarées deviennent celles qui reçoivent les doigts.
    pub fn end_frame(&mut self) {
        std::mem::swap(&mut self.regions, &mut self.next);
        self.next.clear();
        self.clip = None;
        self.clicks.clear();
        self.clic_pos.clear();
        self.double_taps.clear();
        // Les événements qu'aucune zone n'a lus sont perdus ; les moteurs des zones
        // disparues aussi, une fois au repos
        self.sorties.clear();
        let zones: Vec<Id> = self.regions.iter().filter(|r| r.sense == Sense::Gestes).map(|r| r.id).collect();
        self.moteurs.retain(|id, m| zones.contains(id) || m.apercu().is_some());
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
    fn une_zone_a_gestes_recoit_tap_et_saisie() {
        const ZONE: Id = id("zone");
        let mut u = Ui::new(1.0);
        u.region_add(ZONE, Rect::new(400., 400., 400., 400.), Sense::Gestes);
        u.end_frame();
        down(&mut u, 0, 500., 500.);
        u.event(Event::Up { id: 0 }, 0.1);
        let g = u.gestes(ZONE, Rect::new(400., 400., 400., 400.));
        assert!(g.iter().any(|g| matches!(g, Geste::Tap { .. })), "{g:?}");
        u.end_frame();
        // Le temps fait passer à l'appui, sans mouvement
        u.event(Event::Down { id: 0, x: 500., y: 500. }, 1.0);
        assert!(u.gestes_en_attente());
        u.horloge(1.6);
        assert_eq!(u.gestes_actifs()[0].etat, crate::gestes::Etat::Appui);
        u.event(Event::Up { id: 0 }, 1.7);
        let g = u.gestes(ZONE, Rect::new(400., 400., 400., 400.));
        assert!(g.iter().any(|g| matches!(g, Geste::Appui { .. })), "{g:?}");
        u.end_frame();
        // Glisser : le curseur déporté
        u.event(Event::Down { id: 0, x: 500., y: 500. }, 2.0);
        u.event(Event::Move { id: 0, x: 600., y: 500. }, 2.1);
        assert_eq!(u.gestes_actifs()[0].curseur, Some((540., 500.)));
    }

    #[test]
    fn un_doigt_hors_de_toute_zone_ne_fait_rien() {
        let mut u = ui();
        down(&mut u, 0, 500., 500.);
        u.event(Event::Move { id: 0, x: 600., y: 600. }, 0.1);
        assert!(u.gestes_actifs().is_empty());
        assert!(!u.pressed(BTN));
    }

    #[test]
    fn deux_doigts_deux_boutons() {
        let mut u = ui();
        down(&mut u, 0, 20., 20.);
        down(&mut u, 1, 20., 110.);
        assert!(u.pressed(BTN) && u.pressed(SLIDER));
    }
}
