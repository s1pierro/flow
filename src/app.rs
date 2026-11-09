//! L'application : l'espace de travail partagé en fenêtres, chacune portant une
//! instance de contexte (`contexte`, `contextes`), un panneau de chaque côté, et en bas
//! de l'écran la barre de l'application : ses pastilles aplaties (le logo, les
//! panneaux), l'heure et la batterie. Le bas de l'écran est réservé aux gestes du
//! système, mais les appuis y restent disponibles.
//!
//! L'espace (disposition, contexte et état de chaque fenêtre) s'écrit en texte avec
//! `espace` et se relit avec `ouvrir_espace` ; c'est le fichier `espace.txt`.

use crate::contexte::{Atelier, Commun, Contexte, Etat, Scene, COULEURS};
use crate::contextes::{self, GENRES};
use crate::draw::Rect;
use crate::espace::Espace;
use crate::reglages::Reglages;
use crate::shell::{Layout, Shell, Side};
use crate::surface::{Changement, FenetreId, Placee, Surface};
use crate::ui::{id, sub_id, Id};
use crate::widgets::{Cx, Icon, Theme};
use std::collections::HashMap;
use std::path::PathBuf;

pub use crate::contexte::Vue3d;

const OBJECTS: Id = id("objets");
const DISPLAY: Id = id("affichage");
const HELP: Id = id("aide");
const BARRE: Id = id("barre-application");
const LOGO: Id = id("logo");

#[derive(Clone, Copy, PartialEq)]
enum EndContent {
    Display,
    Help,
    APropos,
}

pub struct App {
    pub shell: Shell,
    /// Ce que partagent toutes les fenêtres : apparence, clavier, stockage…
    pub commun: Commun,
    /// L'espace de travail, partagé en fenêtres.
    pub surface: Surface,
    /// L'instance de contexte de chaque fenêtre.
    fenetres: HashMap<FenetreId, Box<dyn Contexte>>,
    /// L'heure (heures, minutes) et la batterie (niveau, en charge), données par le
    /// système ; rien hors d'Android.
    pub heure: Option<(u8, u8)>,
    pub batterie: Option<(u8, bool)>,
    end_content: EndContent,
    display_tab: usize,
}

impl App {
    pub fn new(dp: f32) -> App {
        App {
            shell: Shell::new(dp),
            commun: Commun::default(),
            surface: Surface::new(0),
            fenetres: HashMap::new(),
            heure: None,
            batterie: None,
            end_content: EndContent::Display,
            display_tab: 0,
        }
    }

    /// Vrai s'il faut redessiner sans attendre un événement.
    pub fn animating(&self) -> bool {
        self.shell.animating()
            || self.commun.clavier.anime()
            || self.surface.placees().iter().any(|p| self.fenetres.get(&p.id).is_some_and(|i| i.anime(&self.commun)))
    }

    /// Une pastille ouvre son contenu, ou referme le panneau s'il l'affiche déjà.
    fn toggle_end(&mut self, content: EndContent) {
        if self.shell.end.open && self.end_content == content {
            self.shell.end.open = false;
        } else {
            self.end_content = content;
            self.shell.end.open = true;
        }
    }

    /// Les réglages de l'interface à enregistrer : l'apparence et le clavier.
    pub fn reglages(&self) -> Reglages {
        Reglages { theme: self.commun.theme, clavier: self.commun.clavier.reglages }
    }

    /// Applique des réglages relus.
    pub fn appliquer(&mut self, r: Reglages) {
        self.commun.theme = r.theme;
        self.commun.clavier.reglages = r.clavier;
    }

    /// L'espace de travail en texte : la disposition, ce qui est commun, et l'état de
    /// chaque fenêtre.
    pub fn espace(&self) -> String {
        let fenetres = self.surface.ecrire_avec(&|v| contextes::genre(v).cle.to_string());
        let mut commun = Etat::default();
        self.commun.atelier.ecrire(&mut commun);
        let sections = self
            .surface
            .disposer(Rect::default())
            .into_iter()
            .map(|(f, _, _)| {
                let mut e = Etat::default();
                if let Some(i) = self.fenetres.get(&f) {
                    i.ecrire(&mut e);
                }
                (f, e)
            })
            .collect();
        Espace { fenetres: Some(fenetres), commun, sections }.ecrire()
    }

    /// Rouvre un espace de travail écrit par `espace`. Une disposition illisible garde
    /// les fenêtres actuelles ; un contexte inconnu devient le contexte de repli.
    pub fn ouvrir_espace(&mut self, texte: &str) {
        let e = Espace::lire(texte);
        self.commun.atelier = Atelier::lire(&e.commun);
        let repli = contextes::indice(contextes::REPLI).unwrap_or(0);
        let Some(s) = e.fenetres.as_deref().and_then(|t| Surface::lire_avec(t, &|n| Some(contextes::indice(n).unwrap_or(repli)))) else { return };
        self.surface = s;
        self.fenetres.clear();
        for (f, vue, _) in self.surface.disposer(Rect::default()) {
            self.fenetres.insert(f, (contextes::genre(vue).ouvrir)(&e.section(f)));
        }
    }

    /// La racine du stockage, donnée par Android ; au retour dans l'application, les
    /// contextes relisent ce qui a pu changer sur le disque.
    pub fn racine_en(&mut self, racine: Option<PathBuf>) {
        self.commun.racine = racine;
        for i in self.fenetres.values_mut() {
            i.retour(&self.commun);
        }
    }

    /// (Re)lit l'historique des radios, rangé à côté des réglages.
    pub fn historique_en(&mut self, chemin: PathBuf) {
        self.commun.historique = crate::radio::Historique::ouvrir(chemin);
    }

    /// Les frappes du clavier vont à l'instance qui possède le champ ciblé.
    fn frappes(&mut self, cx: &mut Cx, zone: Rect, dt: f32) {
        for frappe in self.commun.clavier.saisir(cx, zone, dt) {
            let Some(cible) = self.commun.clavier.cible else { continue };
            for (f, i) in self.fenetres.iter_mut() {
                if i.frappe(*f, cible, frappe, &mut self.commun) {
                    break;
                }
            }
        }
    }

    /// Chaque fenêtre a une instance du contexte qu'elle montre : la fenêtre née d'un
    /// partage reçoit une copie, une fenêtre fermée perd la sienne, et une fenêtre qui
    /// change de contexte en reçoit une nouvelle.
    fn suivre_fenetres(&mut self, placees: &[Placee]) {
        for c in self.surface.changements() {
            match c {
                Changement::Partagee { de, nouvelle } => {
                    if let Some(copie) = self.fenetres.get(&de).map(|i| i.dupliquer()) {
                        self.fenetres.insert(nouvelle, copie);
                    }
                }
                Changement::Fermee(f) => {
                    self.fenetres.remove(&f);
                }
                Changement::Vue { .. } => {}
            }
        }
        for p in placees {
            let g = contextes::genre(p.vue);
            if self.fenetres.get(&p.id).is_none_or(|i| i.cle() != g.cle) {
                self.fenetres.insert(p.id, (g.ouvrir)(&Etat::default()));
            }
        }
    }

    pub fn frame(&mut self, cx: &mut Cx, screen: Rect, dt: f32) -> Scene {
        cx.theme = self.commun.theme;
        // La barre de l'application, tout en bas
        let hb = cx.commande().min(screen.h / 4.0);
        let barre = Rect::new(screen.x, screen.bottom() - hb, screen.w, hb);
        let screen = Rect::new(screen.x, screen.y, screen.w, screen.h - hb);
        // Le clavier ouvert prend le bas de l'écran, ou flotte par-dessus ; ses frappes
        // vont au champ qui a le focus
        let (screen, zone_clavier) = self.commun.clavier.reserver(cx, screen);
        let flottant = self.commun.clavier.reglages.flottant;
        if !flottant {
            self.frappes(cx, zone_clavier, dt);
        }
        let l = self.shell.layout(screen, dt);
        let vues = contextes::vues();
        let placees = self.surface.frame(cx, l.workspace, &vues);
        self.suivre_fenetres(&placees);
        // Les gestes de l'espace de travail vont à la fenêtre où ils ont commencé
        let g = cx.ui.take_gesture();
        if let Some(p) = g.at.and_then(|(x, y)| self.surface.fenetre_en(x, y).copied()) {
            if let Some(i) = self.fenetres.get_mut(&p.id) {
                i.geste(&g, &p, &self.commun);
            }
        }
        for p in &placees {
            if let Some(i) = self.fenetres.get_mut(&p.id) {
                i.avancer(dt, &self.commun);
            }
        }
        for p in &placees {
            let Some(i) = self.fenetres.get_mut(&p.id) else { continue };
            let etat = i.dessiner(cx, &mut self.commun, p);
            // La ligne d'état, à droite du titre, en chasse fixe
            let (d, col, e) = (cx.dp, cx.couleurs(), p.etat);
            cx.clip(Some(e));
            cx.mono_at(e.x + 4.0 * d, e.cy(), &etat, 12.0, col.pale);
            cx.clip(None);
        }
        self.surface.dessus(cx, &vues);
        self.barre(cx, barre);

        self.start_panel(cx, &l);
        self.end_panel(cx, &l);
        self.handles(cx, &l, screen);
        // Flottant, le clavier déclare ses touches en dernier : elles passent devant tout
        if flottant {
            self.frappes(cx, zone_clavier, dt);
        }
        self.commun.clavier.dessiner(cx);
        let mut scene = self.scene(l.workspace, &placees);
        scene.fond = cx.couleurs().fond;
        scene
    }

    /// La barre de l'application : pastilles aplaties à gauche (le logo, puis les
    /// panneaux), l'heure et la batterie à droite.
    fn barre(&mut self, cx: &mut Cx, r: Rect) {
        if r.h < 1.0 {
            return;
        }
        let (d, col) = (cx.dp, cx.couleurs());
        cx.ui.region_add(BARRE, r, crate::ui::Sense::Sink);
        cx.dl.rrect(r, 0.0, col.surface);
        cx.dl.rrect(Rect::new(r.x, r.y, r.w, d.max(1.0)), 0.0, col.ligne);
        let case = |i: f32| Rect::new(r.x + i * r.h, r.y, r.h, r.h);
        let (end, content) = (self.shell.end.open, self.end_content);
        if cx.pastille_plate(LOGO, case(0.0), Icon::Rustine, end && content == EndContent::APropos) {
            self.toggle_end(EndContent::APropos);
        }
        if cx.pastille_plate(OBJECTS, case(1.0), Icon::List, self.shell.start.open) {
            self.shell.start.toggle();
        }
        if cx.pastille_plate(DISPLAY, case(2.0), Icon::Sliders, end && content == EndContent::Display) {
            self.toggle_end(EndContent::Display);
        }
        if cx.pastille_plate(HELP, case(3.0), Icon::Help, end && content == EndContent::Help) {
            self.toggle_end(EndContent::Help);
        }
        // À droite : la batterie, puis l'heure tout au bord
        let mut x = r.right() - cx.pad();
        if let Some((h, m)) = self.heure {
            let s = format!("{h:02}:{m:02}");
            let w = cx.measure(&s, 15.0);
            x -= w;
            cx.text_at(x, r.cy(), &s, 15.0, col.texte);
            x -= cx.pad();
        }
        if let Some((niveau, charge)) = self.batterie {
            let s = format!("{niveau} %");
            let w = cx.measure(&s, 13.0);
            x -= w;
            cx.text_at(x, r.cy(), &s, 13.0, col.pale);
            let ic = (r.h * 0.5).round();
            x -= ic + 4.0 * d;
            cx.icon(Rect::new(x, r.cy() - ic / 2.0, ic, ic), Icon::Batterie { niveau, charge }, col.texte);
        }
    }

    fn panel_frame(&self, cx: &mut Cx, id: Id, r: Rect, side: Side, landscape: bool) {
        cx.ui.region_add(id, r, crate::ui::Sense::Sink);
        let col = cx.couleurs();
        cx.dl.rrect(r, 0.0, col.surface);
        let t = cx.dp.max(1.0);
        let line = match (side, landscape) {
            (Side::Start, true) => Rect::new(r.right() - t, r.y, t, r.h),
            (Side::End, true) => Rect::new(r.x, r.y, t, r.h),
            (Side::Start, false) => Rect::new(r.x, r.bottom() - t, r.w, t),
            (Side::End, false) => Rect::new(r.x, r.y, r.w, t),
        };
        cx.dl.rrect(line, 0.0, col.ligne);
    }

    /// En-tête d'un panneau : renvoie le rectangle du corps, et vrai si ✕ a été touché.
    fn panel_header(&self, cx: &mut Cx, id_close: Id, r: Rect) -> (Rect, Rect, bool) {
        let (d, col, h) = (cx.dp, cx.couleurs(), cx.commande());
        let head = h + 12.0 * d;
        let close = Rect::new(r.right() - h - 6.0 * d, r.y + 6.0 * d, h, h);
        let clicked = cx.ui.click(id_close, close, None);
        cx.dl.rrect(close, cx.theme.rayon(h), col.puce);
        cx.icon(close.inset(h * 0.25), Icon::Close, col.texte);
        cx.dl.rrect(Rect::new(r.x, r.y + head - d, r.w, d.max(1.0)), 0.0, col.ligne);
        // Les onglets partent de (x, y) ; le clip les garde à gauche du ✕
        (Rect::new(r.x + 12.0 * d, r.y + 6.0 * d, r.w - h - 30.0 * d, h), Rect::new(r.x, r.y + head, r.w, r.h - head), clicked)
    }

    fn start_panel(&mut self, cx: &mut Cx, l: &Layout) {
        if self.shell.start.anim == 0.0 {
            return;
        }
        let r = l.start;
        self.panel_frame(cx, id("panneau-debut"), r, Side::Start, l.landscape);
        let (tabs, body, close) = self.panel_header(cx, id("fermer-debut"), r);
        if close {
            self.shell.start.open = false;
        }
        cx.tabs(id("onglets-debut"), tabs.x, tabs.y, &["Couleurs"], 0);
        let scroll = id("liste-couleurs");
        let c = &mut self.commun;
        let mut b = cx.body(scroll, body, &c.scrolls);
        for (i, (name, color)) in COULEURS.iter().enumerate() {
            if cx.row(&mut b, sub_id(scroll, i), name, Some(*color), i == c.atelier.couleur) {
                c.atelier.couleur = i;
            }
        }
        cx.body_end(b, &mut c.scrolls);
    }

    fn end_panel(&mut self, cx: &mut Cx, l: &Layout) {
        if self.shell.end.anim == 0.0 {
            return;
        }
        let r = l.end;
        self.panel_frame(cx, id("panneau-fin"), r, Side::End, l.landscape);
        let (tabs, body, close) = self.panel_header(cx, id("fermer-fin"), r);
        if close {
            self.shell.end.open = false;
        }
        let c = &mut self.commun;
        match self.end_content {
            EndContent::Display => {
                self.display_tab = cx.tabs(id("onglets-affichage"), tabs.x, tabs.y, &["Vue", "Cube"], self.display_tab);
                let scroll = sub_id(id("corps-affichage"), self.display_tab);
                let a = &mut c.atelier;
                let mut b = cx.body(scroll, body, &c.scrolls);
                if self.display_tab == 0 {
                    cx.section(&mut b, "CAMÉRA");
                    let shown = format!("{:.0}°", a.champ);
                    cx.slider(&mut b, id("champ"), "Champ de vision", &shown, &mut a.champ, 25.0, 90.0);
                    if cx.button(&mut b, id("recadrer-panneau"), "Recadrer") {
                        a.recadrage = a.recadrage.wrapping_add(1);
                    }
                    cx.section(&mut b, "ROTATION");
                    cx.toggle(&mut b, id("rotation-panneau"), "Rotation automatique", &mut a.rotation);
                    let shown = format!("{:.0}°/s", a.vitesse);
                    cx.slider(&mut b, id("vitesse"), "Vitesse", &shown, &mut a.vitesse, 5.0, 180.0);
                } else {
                    cx.section(&mut b, "FORME");
                    let shown = format!("{:.2}", a.taille);
                    cx.slider(&mut b, id("taille"), "Taille", &shown, &mut a.taille, 0.4, 2.0);
                    cx.section(&mut b, "APPARENCE");
                    cx.toggle(&mut b, id("faces"), "Faces colorées", &mut a.faces);
                }
                cx.body_end(b, &mut c.scrolls);
            }
            EndContent::APropos => {
                cx.tabs(id("onglets-a-propos"), tabs.x, tabs.y, &["Flow"], 0);
                let mut b = cx.body(id("corps-a-propos"), body, &c.scrolls);
                cx.section(&mut b, &format!("FLOW {}", env!("CARGO_PKG_VERSION")));
                cx.paragraph(&mut b, "Une application d'exploitation : des contextes (sous-applications) à disposer dans les fenêtres, pour composer son espace de travail.");
                cx.paragraph(&mut b, &format!("{} contextes : {}.", GENRES.len(), GENRES.iter().map(|g| g.nom).collect::<Vec<_>>().join(", ")));
                cx.paragraph(&mut b, "Réutilisable à des fins non commerciales, selon la licence PolyForm Noncommercial 1.0.0.");
                cx.section(&mut b, "BARRE");
                cx.paragraph(&mut b, "Le centre du bas de l'écran est réservé aux gestes du système, mais les appuis y restent libres : la barre de l'application y range ses pastilles.");
                if c.fichier_reglages.is_some() || c.fichier_espace.is_some() {
                    cx.section(&mut b, "FICHIERS");
                    for f in [&c.fichier_reglages, &c.fichier_espace].into_iter().flatten() {
                        cx.paragraph(&mut b, f);
                    }
                }
                cx.body_end(b, &mut c.scrolls);
            }
            EndContent::Help => {
                cx.tabs(id("onglets-aide"), tabs.x, tabs.y, &["Aide"], 0);
                let mut b = cx.body(id("corps-aide"), body, &c.scrolls);
                cx.section(&mut b, "GESTES");
                cx.paragraph(&mut b, "Un doigt sur une vue 3D la fait tourner.");
                cx.paragraph(&mut b, "Deux doigts zooment et déplacent la vue.");
                cx.section(&mut b, "PANNEAUX");
                cx.paragraph(&mut b, "Les pastilles de la barre du bas déploient les panneaux. L'espace de travail s'adapte à la place restante.");
                cx.paragraph(&mut b, "Glissez la frontière d'un panneau pour le redimensionner ; un double appui rétablit sa taille.");
                cx.body_end(b, &mut c.scrolls);
            }
        }
    }

    /// Poignées de redimensionnement, sur la frontière de chaque panneau ouvert.
    fn handles(&mut self, cx: &mut Cx, l: &Layout, screen: Rect) {
        let d = cx.dp;
        for (side, key, shown) in [(Side::Start, id("poignee-debut"), self.shell.start.anim), (Side::End, id("poignee-fin"), self.shell.end.anim)] {
            if shown < 1.0 {
                continue;
            }
            let band = self.shell.handle(l, side);
            if let Some(drag) = cx.ui.drag(key, band) {
                self.shell.resize(side, screen, drag.x, drag.y);
            }
            if cx.ui.double_tapped(key) {
                self.shell.reset_size(side, screen);
            }
            let pressed = cx.ui.pressed(key);
            let color = if pressed { cx.couleurs().accent } else { [0x6C, 0x6C, 0x80, 140] };
            let (long, thick) = (52.0 * d, 5.0 * d);
            let grip = if l.landscape {
                Rect::new(band.cx() - thick / 2.0, band.cy() - long / 2.0, thick, long)
            } else {
                Rect::new(band.cx() - long / 2.0, band.cy() - thick / 2.0, long, thick)
            };
            cx.dl.rrect(grip, thick / 2.0, color);
        }
    }

    fn scene(&self, w: Rect, placees: &[Placee]) -> Scene {
        let vues = placees.iter().filter_map(|p| self.fenetres.get(&p.id)?.vue3d(p, &self.commun)).collect();
        let a = &self.commun.atelier;
        let [r, g, b, _] = COULEURS[a.couleur.min(COULEURS.len() - 1)].1;
        Scene { workspace: w, vues, fond: Theme::default().couleurs().fond, color: [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0], tint: if a.faces { 0.25 } else { 0.0 } }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draw::DrawList;
    use crate::text::Text;
    use crate::ui::{Event, Ui};

    const SCREEN: Rect = Rect::new(0., 0., 1920., 1200.);

    struct Rig {
        app: App,
        ui: Ui,
        text: Text,
        dl: DrawList,
    }

    impl Rig {
        fn new() -> Rig {
            let mut r = Rig { app: App::new(1.5), ui: Ui::new(1.5), text: Text::new(), dl: DrawList::default() };
            r.frames(1);
            r
        }
        fn frame(&mut self, dt: f32) -> Scene {
            self.dl.clear();
            let mut cx = Cx { ui: &mut self.ui, text: &mut self.text, dl: &mut self.dl, dp: 1.5, theme: Default::default() };
            let s = self.app.frame(&mut cx, SCREEN, dt);
            self.ui.end_frame();
            s
        }
        fn frames(&mut self, n: usize) -> Scene {
            let mut s = self.frame(0.05);
            for _ in 1..n {
                s = self.frame(0.05);
            }
            s
        }
        fn tap(&mut self, x: f32, y: f32) {
            self.ui.event(Event::Down { id: 0, x, y }, 0.0);
            self.ui.event(Event::Up { id: 0 }, 0.05);
            self.frames(1);
        }
        fn glisser(&mut self, x: f32, y: f32) {
            self.ui.event(Event::Down { id: 0, x, y }, 0.0);
            self.ui.event(Event::Move { id: 0, x: x + 100., y }, 0.1);
            self.ui.event(Event::Move { id: 0, x: x + 200., y }, 0.1);
            self.frames(1);
            self.ui.event(Event::Up { id: 0 }, 0.2);
            self.frames(1);
        }
    }

    fn vue(cle: &str) -> u32 {
        contextes::indice(cle).unwrap()
    }

    // Centre de la n-ième pastille des panneaux, dans la barre du bas (après le logo).
    fn pastille(n: f32) -> (f32, f32) {
        (72.0 * (n + 1.5), 1200.0 - 36.0)
    }

    #[test]
    fn la_taille_des_commandes_suit_le_theme() {
        let mut r = Rig::new();
        r.app.commun.theme.commande = 60.0;
        r.frames(1);
        // Les pastilles font 60 dp : la barre est plus haute, la première pastille des
        // panneaux plus loin
        let (x, y) = (90.0 * 1.5, 1200.0 - 45.0);
        r.tap(x, y);
        assert!(r.app.shell.start.open);
    }

    #[test]
    fn une_pastille_deploie_son_panneau_et_l_espace_s_adapte() {
        let mut r = Rig::new();
        let (x, y) = pastille(1.0);
        r.tap(x, y);
        assert!(r.app.shell.end.open);
        let s = r.frames(60);
        assert_eq!(s.workspace.w, 1920.0 - 576.0);
        assert!(!r.app.animating());
        let (x, y) = pastille(1.0);
        r.tap(x, y); // nouvel appui : referme
        assert!(!r.app.shell.end.open);
        assert_eq!(r.frames(60).workspace, Rect::new(0., 0., 1920., 1200. - 72.));
    }

    #[test]
    fn la_pastille_d_aide_remplace_le_contenu_sans_fermer() {
        let mut r = Rig::new();
        let (x, y) = pastille(1.0);
        r.tap(x, y);
        r.frames(60);
        let (x, y) = pastille(2.0);
        r.tap(x, y);
        assert!(r.app.shell.end.open && r.app.end_content == EndContent::Help);
    }

    #[test]
    fn toucher_la_liste_change_la_couleur_sans_tourner_la_vue() {
        let mut r = Rig::new();
        let (x, y) = pastille(0.0);
        r.tap(x, y);
        let avant = r.frames(60).vues[0].mvp;
        r.tap(200.0, 1.5 * (60.0 + 8.0 + 48.0 * 3.5));
        assert_eq!(r.app.commun.atelier.couleur, 3);
        assert_eq!(r.frames(1).vues[0].mvp, avant);
    }

    #[test]
    fn glisser_sur_l_espace_de_travail_tourne_la_vue() {
        let mut r = Rig::new();
        let avant = r.frames(1).vues[0].mvp;
        r.glisser(900., 600.);
        assert_ne!(r.frames(1).vues[0].mvp, avant);
    }

    #[test]
    fn la_poignee_redimensionne_dans_l_application() {
        let mut r = Rig::new();
        r.app.surface.partager(1, crate::surface::Sens::Colonnes, 0.5);
        r.frames(1);
        // La poignée de la fenêtre de gauche : 54 px de côté, dans son coin bas droit, au-dessus de la barre de l'application
        r.ui.event(Event::Down { id: 0, x: 933., y: 1101. }, 0.0);
        r.frames(1);
        for k in 1..=10 {
            r.ui.event(Event::Move { id: 0, x: 933. + 30. * k as f32, y: 1101. }, 0.1);
            r.frames(1);
        }
        r.ui.event(Event::Up { id: 0 }, 0.2);
        let s = r.frames(2);
        assert!((s.vues[0].rect.w - 1260.).abs() <= 1.0, "{:?}", s.vues[0].rect);
    }

    #[test]
    fn chaque_fenetre_a_sa_propre_instance() {
        let mut r = Rig::new();
        let nouvelle = r.app.surface.partager(1, crate::surface::Sens::Colonnes, 0.5).unwrap();
        let s = r.frames(1);
        assert_eq!(s.vues.len(), 2);
        assert_eq!(s.vues[0].rect.w, 960.0);
        // Un geste dans la fenêtre de droite (la n°1) ne tourne qu'elle
        let (a, b) = (s.vues[0].mvp, s.vues[1].mvp);
        r.glisser(1400., 600.);
        let s = r.frames(1);
        assert_eq!(s.vues[0].mvp, a);
        assert_ne!(s.vues[1].mvp, b);
        // Une fenêtre passée en Couleurs n'a plus de 3D, et change d'instance
        r.app.surface.changer_vue(nouvelle, vue("couleurs"));
        assert_eq!(r.frames(1).vues.len(), 1);
        assert_eq!(r.app.fenetres[&nouvelle].cle(), "couleurs");
        // Fermée, elle n'en a plus
        r.app.surface.fermer(nouvelle);
        r.frames(1);
        assert!(!r.app.fenetres.contains_key(&nouvelle));
    }

    #[test]
    fn l_espace_de_travail_se_retrouve() {
        let texte = "fenetres = C0.4000(F3:fichiers,L0.5000(F2:fichiers,F1:cube))\n\
                     [commun]\ncube_couleur = rose\n\
                     [fenetre 3]\ndossier = /tmp\ncaches = oui\n\
                     [fenetre 2]\ndossier = /\n\
                     [fenetre 1]\ncamera = 1.000 0.200 9.000 0.000 1.000 0.000\n";
        let mut r = Rig::new();
        r.app.ouvrir_espace(texte);
        assert_eq!(r.app.commun.atelier.couleur, 10);
        assert_eq!(r.app.surface.nombre(), 3);
        r.frames(1);
        // Les deux explorateurs gardent chacun leur dossier
        let t = r.app.espace();
        assert!(t.contains("[fenetre 3]\ndossier = /tmp\ncaches = oui\n"), "{t}");
        assert!(t.contains("[fenetre 2]\ndossier = /\ncaches = non\n"), "{t}");
        assert!(t.contains("camera = 1.000 0.200 9.000 0.000 1.000 0.000"), "{t}");
        // Une autre application qui relit ce texte écrit la même chose
        let mut autre = Rig::new();
        autre.app.ouvrir_espace(&t);
        autre.frames(1);
        assert_eq!(autre.app.espace(), t);
        // Un contexte inconnu devient le repli, une disposition illisible ne change rien
        autre.app.ouvrir_espace("fenetres = F1:inconnu\n");
        assert_eq!(autre.app.surface.ecrire_avec(&|v| contextes::genre(v).cle.into()), format!("F1:{}", contextes::REPLI));
        autre.app.ouvrir_espace("fenetres = C0.5(F1:cube\n");
        assert_eq!(autre.app.surface.nombre(), 1);
    }

    #[test]
    fn un_partage_copie_l_etat_de_la_fenetre() {
        let mut r = Rig::new();
        r.app.ouvrir_espace("fenetres = F1:fichiers\n[fenetre 1]\ndossier = /tmp\n");
        r.frames(1);
        r.app.surface.partager(1, crate::surface::Sens::Colonnes, 0.5);
        r.frames(1);
        let t = r.app.espace();
        assert!(t.contains("[fenetre 2]\ndossier = /tmp\n"), "{t}");
    }

    #[test]
    fn rien_ne_redessine_au_repos() {
        let mut r = Rig::new();
        r.frames(3);
        assert!(!r.app.animating());
    }
}
