//! L'application : l'espace de travail partagé en fenêtres, chacune portant une
//! instance de contexte (`contexte`, `contextes`), le panneau latéral (paramètres de
//! l'application, aide, à propos), et en bas de l'écran la barre de l'application :
//! ses pastilles aplaties (le logo, les paramètres, l'aide), l'heure et la batterie.
//! Le bas de l'écran est réservé aux gestes du système, mais les appuis y restent
//! disponibles.
//!
//! L'espace (disposition, contexte et état de chaque fenêtre) s'écrit en texte avec
//! `espace` et se relit avec `ouvrir_espace` ; c'est le fichier `espace.txt`.

use crate::clavier::Edition;
use crate::contexte::{Commun, Contexte, Etat};
use crate::contextes::{self, GENRES};
use crate::draw::{Color, Rect};
use crate::espace::Espace;
use crate::parametres;
use crate::reglages::Reglages;
use crate::shell::{Layout, Shell, Side};
use crate::surface::{Changement, FenetreId, Placee, Surface};
use crate::ui::{id, sub_id, Id};
use crate::widgets::{Cx, Icon};
use std::collections::HashMap;
use std::path::PathBuf;

const PARAMETRES: Id = id("parametres");
const HELP: Id = id("aide");
const BARRE: Id = id("barre-application");
const LOGO: Id = id("logo");

/// Ce que le moteur de rendu doit savoir de l'image : l'espace de travail, et le
/// fond de l'écran.
#[derive(Clone, Copy, Debug)]
pub struct Scene {
    pub workspace: Rect,
    pub fond: Color,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum EndContent {
    Parametres,
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
    /// L'onglet ouvert du panneau des paramètres, et son champ d'essai du clavier.
    onglet_parametres: usize,
    essai: Edition,
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
            end_content: EndContent::Parametres,
            onglet_parametres: 0,
            essai: Edition::default(),
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

    /// L'espace de travail en texte : la disposition et l'état de chaque fenêtre.
    pub fn espace(&self) -> String {
        let fenetres = self.surface.ecrire_avec(&|v| contextes::genre(v).cle.to_string());
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
        Espace { fenetres: Some(fenetres), commun: Etat::default(), sections }.ecrire()
    }

    /// Rouvre un espace de travail écrit par `espace`. Une disposition illisible garde
    /// les fenêtres actuelles ; un contexte inconnu devient le contexte de repli.
    pub fn ouvrir_espace(&mut self, texte: &str) {
        let e = Espace::lire(texte);
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

    /// Les frappes du clavier vont au champ ciblé : celui d'essai du panneau des
    /// paramètres, ou celui de l'instance qui le possède.
    fn frappes(&mut self, cx: &mut Cx, zone: Rect, dt: f32) {
        for frappe in self.commun.clavier.saisir(cx, zone, dt) {
            let Some(cible) = self.commun.clavier.cible else { continue };
            if cible == parametres::ESSAI {
                if self.essai.appliquer(frappe) {
                    self.commun.clavier.fermer();
                }
                continue;
            }
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

        self.end_panel(cx, &l);
        self.handles(cx, &l, screen);
        // Flottant, le clavier déclare ses touches en dernier : elles passent devant tout
        if flottant {
            self.frappes(cx, zone_clavier, dt);
        }
        self.commun.clavier.dessiner(cx);
        Scene { workspace: l.workspace, fond: cx.couleurs().fond }
    }

    /// La barre de l'application : pastilles aplaties à gauche (le logo, les
    /// paramètres, l'aide), l'heure et la batterie à droite.
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
        if cx.pastille_plate(PARAMETRES, case(1.0), Icon::Sliders, end && content == EndContent::Parametres) {
            self.toggle_end(EndContent::Parametres);
        }
        if cx.pastille_plate(HELP, case(2.0), Icon::Help, end && content == EndContent::Help) {
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
            EndContent::Parametres => {
                self.onglet_parametres = cx.tabs(id("onglets-parametres"), tabs.x, tabs.y, &parametres::ONGLETS, self.onglet_parametres);
                let scroll = sub_id(id("corps-parametres"), self.onglet_parametres);
                let mut b = cx.body(scroll, body, &c.scrolls);
                parametres::panneau(cx, &mut b, scroll, self.onglet_parametres, c, &mut self.essai);
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
                cx.body_end(b, &mut c.scrolls);
            }
            EndContent::Help => {
                cx.tabs(id("onglets-aide"), tabs.x, tabs.y, &["Aide"], 0);
                let mut b = cx.body(id("corps-aide"), body, &c.scrolls);
                cx.section(&mut b, "FENÊTRES");
                cx.paragraph(&mut b, "Glissez la pastille + d'une fenêtre pour la partager ; touchez son titre pour choisir son contexte ou la fermer.");
                cx.section(&mut b, "PANNEAU");
                cx.paragraph(&mut b, "Les pastilles de la barre du bas ouvrent ce panneau : à propos, paramètres de l'application, aide. L'espace de travail s'adapte à la place restante.");
                cx.paragraph(&mut b, "Glissez la frontière du panneau pour le redimensionner ; un double appui rétablit sa taille.");
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
    }

    fn vue(cle: &str) -> u32 {
        contextes::indice(cle).unwrap()
    }

    // Centre de la n-ième pastille de la barre du bas : 0 le logo, 1 les paramètres, 2 l'aide.
    fn pastille(n: f32) -> (f32, f32) {
        (72.0 * (n + 0.5), 1200.0 - 36.0)
    }

    #[test]
    fn la_taille_des_commandes_suit_le_theme() {
        let mut r = Rig::new();
        r.app.commun.theme.commande = 60.0;
        r.frames(1);
        // Les pastilles font 60 dp : la barre est plus haute, celle des paramètres plus loin
        r.tap(90.0 * 1.5, 1200.0 - 45.0);
        assert!(r.app.shell.end.open && r.app.end_content == EndContent::Parametres);
    }

    #[test]
    fn la_pastille_des_parametres_deploie_le_panneau_et_l_espace_s_adapte() {
        let mut r = Rig::new();
        let (x, y) = pastille(1.0);
        r.tap(x, y);
        assert!(r.app.shell.end.open && r.app.end_content == EndContent::Parametres);
        let s = r.frames(60);
        assert_eq!(s.workspace.w, 1920.0 - 576.0);
        assert!(!r.app.animating());
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
        r.frames(2);
        let w = r.app.surface.placees()[0].cadre.w;
        assert!((w - 1260.).abs() <= 1.0, "{w}");
    }

    #[test]
    fn chaque_fenetre_a_sa_propre_instance() {
        let mut r = Rig::new();
        r.app.ouvrir_espace("fenetres = C0.5000(F2:fichiers,F1:fichiers)\n[fenetre 2]\ndossier = /tmp\n[fenetre 1]\ndossier = /\n");
        r.frames(1);
        let t = r.app.espace();
        assert!(t.contains("[fenetre 2]\ndossier = /tmp\n") && t.contains("[fenetre 1]\ndossier = /\n"), "{t}");
        // Une fenêtre qui change de contexte change d'instance
        r.app.surface.changer_vue(2, vue("aide"));
        r.frames(1);
        assert_eq!(r.app.fenetres[&2].cle(), "aide");
        assert_eq!(r.app.fenetres[&1].cle(), "fichiers");
        // Fermée, elle n'en a plus
        r.app.surface.fermer(2);
        r.frames(1);
        assert!(!r.app.fenetres.contains_key(&2));
    }

    #[test]
    fn l_espace_de_travail_se_retrouve() {
        let texte = "fenetres = C0.4000(F3:fichiers,L0.5000(F2:radio,F1:disque))\n\
                     [fenetre 3]\ndossier = /tmp\ncaches = oui\n\
                     [fenetre 2]\nadresse = https://exemple.fr/flux\n\
                     [fenetre 1]\ncentre = Musique/Albums\n";
        let mut r = Rig::new();
        r.app.ouvrir_espace(texte);
        assert_eq!(r.app.surface.nombre(), 3);
        r.frames(1);
        let t = r.app.espace();
        assert!(t.contains("[fenetre 3]\ndossier = /tmp\ncaches = oui\n"), "{t}");
        assert!(t.contains("[fenetre 2]\nadresse = https://exemple.fr/flux\n"), "{t}");
        assert!(t.contains("[fenetre 1]\ncentre = Musique/Albums\n"), "{t}");
        // Une autre application qui relit ce texte écrit la même chose
        let mut autre = Rig::new();
        autre.app.ouvrir_espace(&t);
        autre.frames(1);
        assert_eq!(autre.app.espace(), t);
        // Un contexte inconnu (le cube d'une version précédente) devient le repli, une
        // disposition illisible ne change rien
        autre.app.ouvrir_espace("fenetres = F1:cube\n");
        assert_eq!(autre.app.surface.ecrire_avec(&|v| contextes::genre(v).cle.into()), format!("F1:{}", contextes::REPLI));
        autre.app.ouvrir_espace("fenetres = C0.5(F1:aide\n");
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
