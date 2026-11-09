//! Démonstration : l'espace de travail partagé en fenêtres (un cube, une liste, une
//! aide), un panneau de chaque côté, et en bas de l'écran la barre de l'application :
//! ses pastilles aplaties (la rustine, les panneaux), l'heure et la batterie. Le bas de
//! l'écran est réservé aux gestes du système, mais les appuis y restent disponibles.

use crate::draw::{Color, Rect};
use crate::disque::{self, Analyse};
use crate::explorateur::{self, Dossier, Nature};
use crate::math::{self, Mat4};
use crate::clavier::{self, Clavier, Disposition, Edition};
use crate::mixage::{self, mixeur, Piste, Reglage};
use crate::radio::{self, Etat, Historique};
use crate::reglages::Reglages;
use crate::shell::{Layout, Shell, Side};
use crate::surface::{Changement, FenetreId, Placee, Surface};
use crate::ui::{id, sub_id, Id};
use crate::widgets::{Cx, Icon, Scrolls, Theme, ACCENTS};
use std::collections::HashMap;
use std::path::PathBuf;

const COLORS: [(&str, Color); 14] = [
    ("Indigo", [0x4A, 0x45, 0xE0, 255]),
    ("Bleu", [0x3B, 0x82, 0xF6, 255]),
    ("Ciel", [0x38, 0xBD, 0xF8, 255]),
    ("Turquoise", [0x14, 0xB8, 0xA6, 255]),
    ("Vert", [0x22, 0xC5, 0x5E, 255]),
    ("Olive", [0x84, 0xA9, 0x1C, 255]),
    ("Jaune", [0xEA, 0xB3, 0x08, 255]),
    ("Orange", [0xF9, 0x73, 0x16, 255]),
    ("Rouille", [0xB9, 0x4A, 0x18, 255]),
    ("Rouge", [0xEF, 0x44, 0x44, 255]),
    ("Rose", [0xEC, 0x48, 0x99, 255]),
    ("Violet", [0xA8, 0x55, 0xF7, 255]),
    ("Ardoise", [0x64, 0x74, 0x8B, 255]),
    ("Sable", [0xC8, 0xB0, 0x8A, 255]),
];

const OBJECTS: Id = id("objets");
const DISPLAY: Id = id("affichage");
const HELP: Id = id("aide");
const FIT: Id = id("recadrer");
const SPIN: Id = id("rotation");
const LISTE: Id = id("liste-fenetre");
const AIDE: Id = id("aide-fenetre");

const PARAMS: Id = id("parametres");
const BARRE: Id = id("barre-application");
const RUSTINE: Id = id("rustine");
const ESSAI: Id = id("essai-clavier");
const RADIO: Id = id("radio");
const RADIO_CHAMP: Id = id("radio-adresse");
const FICHIERS: Id = id("fichiers");
const DISQUE: Id = id("disque");

/// Les teintes des secteurs du premier anneau ; les anneaux suivants les éclaircissent.
const TEINTES: [Color; 8] = [
    [0x3B, 0x82, 0xF6, 255],
    [0xEF, 0x44, 0x44, 255],
    [0x22, 0xC5, 0x5E, 255],
    [0xF5, 0x9E, 0x0B, 255],
    [0xA8, 0x55, 0xF7, 255],
    [0x14, 0xB8, 0xA6, 255],
    [0xEC, 0x48, 0x99, 255],
    [0x84, 0xCC, 0x16, 255],
];
/// Anneaux du graphique, autour du dossier central.
const ANNEAUX: usize = 4;

/// Deux flux pour essayer, tant que l'historique est vide.
const SUGGESTIONS: [&str; 2] = ["https://icecast.radiofrance.fr/fip-midfi.mp3", "https://stream.radioparadise.com/aac-128"];

/// Les vues qu'une fenêtre de la surface peut montrer.
pub const VUES: [(&str, Icon); 7] = [("Cube", Icon::Cube), ("Couleurs", Icon::List), ("Aide", Icon::Help), ("Paramètres", Icon::Sliders), ("Radio", Icon::Radio), ("Fichiers", Icon::Dossier), ("Disque", Icon::Anneau)];
pub const VUE_CUBE: u32 = 0;
pub const VUE_COULEURS: u32 = 1;
pub const VUE_AIDE: u32 = 2;
pub const VUE_PARAMETRES: u32 = 3;
pub const VUE_RADIO: u32 = 4;
pub const VUE_FICHIERS: u32 = 5;
pub const VUE_DISQUE: u32 = 6;

/// Une vue 3D : son rectangle à l'écran et sa matrice.
#[derive(Clone, Copy)]
pub struct Vue3d {
    pub rect: Rect,
    pub mvp: Mat4,
}

/// Ce que le moteur de rendu doit dessiner : un cube dans chaque vue 3D.
#[derive(Clone)]
pub struct Scene {
    pub workspace: Rect,
    pub vues: Vec<Vue3d>,
    /// Le fond de l'écran, que la 3D laisse voir.
    pub fond: Color,
    pub color: [f32; 3],
    pub tint: f32,
}

#[derive(Clone, Copy)]
struct Camera {
    yaw: f32,
    pitch: f32,
    dist: f32,
    target: [f32; 3],
}

const HOME: Camera = Camera { yaw: 0.65, pitch: 0.45, dist: 6.0, target: [0.0, 0.0, 0.0] };

#[derive(Clone, Copy, PartialEq)]
enum EndContent {
    Display,
    Help,
    APropos,
}

pub struct App {
    pub shell: Shell,
    /// L'apparence de l'interface, réglée dans la vue Paramètres.
    pub theme: Theme,
    /// Le clavier virtuel, et le champ d'essai de la vue Paramètres.
    pub clavier: Clavier,
    essai: Edition,
    /// L'explorateur : la racine du stockage partagé, le dossier ouvert dans chaque
    /// fenêtre Fichiers, et l'affichage des fichiers cachés.
    racine: Option<PathBuf>,
    dossiers: HashMap<FenetreId, Dossier>,
    caches: bool,
    /// L'occupation du disque : l'analyse du stockage (une pour toutes les fenêtres),
    /// et dans chaque fenêtre Disque le dossier au centre et l'élément choisi.
    analyse: Option<Analyse>,
    centres: HashMap<FenetreId, Vec<usize>>,
    choisis: HashMap<FenetreId, Vec<usize>>,
    /// Une vue Disque est à l'écran : elle s'anime pendant l'analyse.
    disque_visible: bool,
    /// Une vue Radio est à l'écran : ses vumètres s'animent tant qu'une voie joue.
    vumetres: bool,
    /// La radio : l'adresse saisie et les derniers flux écoutés ; le lecteur est
    /// `radio::lecteur()`, partagé avec la notification.
    adresse: Edition,
    historique: Historique,
    /// L'heure (heures, minutes) et la batterie (niveau, en charge), données par le
    /// système ; rien hors d'Android.
    pub heure: Option<(u8, u8)>,
    pub batterie: Option<(u8, bool)>,
    /// Où les réglages sont enregistrés, pour l'indiquer dans la vue Paramètres.
    pub fichier_reglages: Option<String>,
    /// L'espace de travail, partagé en fenêtres.
    pub surface: Surface,
    /// Une caméra par fenêtre 3D.
    cams: HashMap<FenetreId, Camera>,
    end_content: EndContent,
    display_tab: usize,
    color: usize,
    spin: bool,
    speed: f32,
    fov: f32,
    size: f32,
    faces: bool,
    scrolls: Scrolls,
}

impl App {
    pub fn new(dp: f32) -> App {
        App {
            shell: Shell::new(dp),
            theme: Theme::default(),
            fichier_reglages: None,
            heure: None,
            batterie: None,
            clavier: Clavier::default(),
            essai: Edition::default(),
            adresse: Edition::default(),
            vumetres: false,
            racine: None,
            analyse: None,
            centres: HashMap::new(),
            choisis: HashMap::new(),
            disque_visible: false,
            dossiers: HashMap::new(),
            caches: false,
            historique: Historique::default(),
            surface: Surface::new(VUE_CUBE),
            cams: HashMap::new(),
            end_content: EndContent::Display,
            display_tab: 0,
            color: 0,
            spin: false,
            speed: 30.0,
            fov: 45.0,
            size: 1.0,
            faces: true,
            scrolls: Scrolls::new(),
        }
    }

    /// Vrai s'il faut redessiner sans attendre un événement.
    pub fn animating(&self) -> bool {
        self.spin || self.shell.animating() || self.clavier.anime() || (self.vumetres && mixeur().actif() && !mixeur().en_pause())
            || (self.disque_visible && self.analyse.as_ref().is_some_and(|a| a.en_cours()))
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

    /// Les réglages de l'interface à enregistrer : l'apparence et les fenêtres.
    pub fn reglages(&self) -> Reglages {
        Reglages { theme: self.theme, clavier: self.clavier.reglages, fenetres: Some(self.surface.ecrire()) }
    }

    /// Applique des réglages relus. Une disposition illisible garde les fenêtres.
    pub fn appliquer(&mut self, r: Reglages) {
        self.theme = r.theme;
        self.clavier.reglages = r.clavier;
        if let Some(s) = r.fenetres.as_deref().and_then(Surface::lire) {
            if s.racine != self.surface.racine {
                self.surface = s;
                self.cams.clear();
            }
        }
    }

    /// La racine du stockage, donnée par Android ; au retour dans l'application, les
    /// dossiers ouverts sont relus (ils ont pu changer entre-temps).
    pub fn racine_en(&mut self, racine: Option<PathBuf>) {
        self.racine = racine;
        let caches = self.caches;
        for d in self.dossiers.values_mut() {
            *d = Dossier::lire(&d.chemin.clone(), caches);
        }
    }

    /// Ouvre un dossier dans une fenêtre Fichiers.
    fn ouvrir_dossier(&mut self, f: FenetreId, chemin: PathBuf) {
        self.dossiers.insert(f, Dossier::lire(&chemin, self.caches));
    }

    /// (Re)lit l'historique des radios, rangé à côté des réglages.
    pub fn historique_en(&mut self, chemin: PathBuf) {
        self.historique = Historique::ouvrir(chemin);
    }

    /// Joue un flux et le place en tête de l'historique.
    fn jouer(&mut self, adresse: &str) {
        let adresse = adresse.trim().to_string();
        if adresse.is_empty() {
            return;
        }
        radio::lecteur().jouer(&adresse);
        if let Err(e) = self.historique.ajouter(&adresse) {
            crate::android::log(&format!("historique des radios non enregistré : {e}"));
        }
    }

    fn frappes(&mut self, cx: &mut Cx, zone: Rect, dt: f32) {
        for f in self.clavier.saisir(cx, zone, dt) {
            match self.clavier.cible {
                Some(ESSAI) => {
                    if self.essai.appliquer(f) {
                        self.clavier.fermer();
                    }
                }
                // Les autres champs sont ceux de la radio : Entrée lance le flux
                Some(_) => {
                    if self.adresse.appliquer(f) {
                        self.clavier.fermer();
                        let a = self.adresse.texte.clone();
                        self.jouer(&a);
                    }
                }
                None => {}
            }
        }
    }

    fn cam(&mut self, f: FenetreId) -> &mut Camera {
        self.cams.entry(f).or_insert(HOME)
    }

    pub fn frame(&mut self, cx: &mut Cx, screen: Rect, dt: f32) -> Scene {
        cx.theme = self.theme;
        // La barre de l'application, tout en bas
        let hb = cx.commande().min(screen.h / 4.0);
        let barre = Rect::new(screen.x, screen.bottom() - hb, screen.w, hb);
        let screen = Rect::new(screen.x, screen.y, screen.w, screen.h - hb);
        // Le clavier ouvert prend le bas de l'écran, ou flotte par-dessus ; ses frappes
        // vont au champ qui a le focus
        let (screen, zone_clavier) = self.clavier.reserver(cx, screen);
        let flottant = self.clavier.reglages.flottant;
        if !flottant {
            self.frappes(cx, zone_clavier, dt);
        }
        let l = self.shell.layout(screen, dt);
        let placees = self.surface.frame(cx, l.workspace, &VUES);
        self.vumetres = placees.iter().any(|p| p.vue == VUE_RADIO);
        self.disque_visible = placees.iter().any(|p| p.vue == VUE_DISQUE);
        for c in self.surface.changements() {
            match c {
                // La nouvelle fenêtre part de la caméra de celle qu'on a partagée
                Changement::Partagee { de, nouvelle } => {
                    let cam = *self.cam(de);
                    self.cams.insert(nouvelle, cam);
                }
                Changement::Fermee(f) => {
                    self.cams.remove(&f);
                    self.dossiers.remove(&f);
                    self.centres.remove(&f);
                    self.choisis.remove(&f);
                }
                Changement::Vue { .. } => {}
            }
        }
        self.gestures(cx);
        if self.spin {
            let a = self.speed.to_radians() * dt;
            for p in placees.iter().filter(|p| p.vue == VUE_CUBE) {
                self.cam(p.id).yaw += a;
            }
        }
        for p in &placees {
            self.fenetre(cx, p);
        }
        self.surface.dessus(cx, &VUES);
        self.barre(cx, barre);

        self.start_panel(cx, &l);
        self.end_panel(cx, &l);
        self.handles(cx, &l, screen);
        // Flottant, le clavier déclare ses touches en dernier : elles passent devant tout
        if flottant {
            self.frappes(cx, zone_clavier, dt);
        }
        self.clavier.dessiner(cx);
        let mut scene = self.scene(l.workspace, &placees);
        scene.fond = cx.couleurs().fond;
        scene
    }

    /// Les gestes de l'espace de travail pilotent la vue 3D où ils ont commencé.
    fn gestures(&mut self, cx: &mut Cx) {
        let g = cx.ui.take_gesture();
        let Some(p) = g.at.and_then(|(x, y)| self.surface.fenetre_en(x, y).copied()).filter(|p| p.vue == VUE_CUBE) else { return };
        let fov = self.fov;
        let c = self.cam(p.id);
        c.yaw -= g.orbit.0 * 0.008;
        c.pitch = (c.pitch + g.orbit.1 * 0.008).clamp(-1.5, 1.5);
        c.dist = (c.dist / g.zoom).clamp(1.8, 24.0);
        // Déplacement à l'échelle de l'image : le cube suit les doigts.
        let k = 2.0 * c.dist * (fov.to_radians() / 2.0).tan() / p.contenu.h.max(1.0);
        let (sy, cy, sp, cp) = (c.yaw.sin(), c.yaw.cos(), c.pitch.sin(), c.pitch.cos());
        let (right, up) = ([cy, 0.0, -sy], [-sy * sp, cp, -cy * sp]);
        for i in 0..3 {
            c.target[i] += -g.pan.0 * k * right[i] + g.pan.1 * k * up[i];
        }
    }

    /// Le contenu d'une fenêtre, selon sa vue, et sa ligne d'état.
    fn fenetre(&mut self, cx: &mut Cx, p: &Placee) {
        let (d, col) = (cx.dp, cx.couleurs());
        let k = p.id as usize;
        let r = p.contenu;
        let etat = match p.vue {
            VUE_CUBE => {
                let (gap, m, h) = (cx.ecart(), cx.coin(), cx.commande());
                let (w1, w2) = (cx.pill_width("Recadrer"), cx.pill_width("Rotation"));
                if r.h > 3.0 * h && r.w > w1 + w2 + gap + 2.0 * m {
                    let x = r.cx() - (w1 + w2 + gap) / 2.0;
                    let y = r.bottom() - h - m;
                    if cx.pill(sub_id(FIT, k), x, y, "Recadrer", false) {
                        self.cams.insert(p.id, HOME);
                    }
                    if cx.pill(sub_id(SPIN, k), x + w1 + gap, y, "Rotation", self.spin) {
                        self.spin = !self.spin;
                    }
                }
                format!("1 objet · 12 tri · n°{}", p.id)
            }
            VUE_COULEURS => {
                cx.dl.rrect(r, 0.0, col.surface);
                let scroll = sub_id(LISTE, k);
                let mut b = cx.body(scroll, r, &self.scrolls);
                for (i, (name, color)) in COLORS.iter().enumerate() {
                    if cx.row(&mut b, sub_id(scroll, i), name, Some(*color), i == self.color) {
                        self.color = i;
                    }
                }
                cx.body_end(b, &mut self.scrolls);
                format!("{} couleurs · {}", COLORS.len(), COLORS[self.color].0)
            }
            VUE_PARAMETRES => {
                cx.dl.rrect(r, 0.0, col.surface);
                self.parametres(cx, sub_id(PARAMS, k), r);
                "Apparence de l'interface".to_string()
            }
            VUE_FICHIERS => {
                cx.dl.rrect(r, 0.0, col.surface);
                self.fichiers(cx, p.id, r)
            }
            VUE_DISQUE => {
                cx.dl.rrect(r, 0.0, col.surface);
                self.disque(cx, p.id, r)
            }
            VUE_RADIO => {
                cx.dl.rrect(r, 0.0, col.surface);
                self.radio(cx, sub_id(RADIO, k), sub_id(RADIO_CHAMP, k), r)
            }
            _ => {
                cx.dl.rrect(r, 0.0, col.surface);
                let mut b = cx.body(sub_id(AIDE, k), r, &self.scrolls);
                cx.section(&mut b, "FENÊTRES");
                cx.paragraph(&mut b, "Attrapez la pastille + en haut à gauche d'une fenêtre et glissez-la pour la partager : vers le côté, les fenêtres se placent côte à côte ; vers le haut ou le bas, l'une sur l'autre.");
                cx.paragraph(&mut b, "La poignée du coin bas droit déplace les bords de la fenêtre : un tiret montre chaque bord qu'elle peut déplacer.");
                cx.paragraph(&mut b, "Touchez l'icône ou le titre pour choisir ce que montre la fenêtre, ou la fermer.");
                cx.section(&mut b, "GESTES");
                cx.paragraph(&mut b, "Un doigt sur une vue 3D la fait tourner ; deux doigts zooment et déplacent.");
                cx.body_end(b, &mut self.scrolls);
                "Aide".to_string()
            }
        };
        // La ligne d'état, à droite du titre, en chasse fixe
        let e = p.etat;
        cx.clip(Some(e));
        cx.mono_at(e.x + 4.0 * d, e.cy(), &etat, 12.0, col.pale);
        cx.clip(None);
    }

    /// La vue Paramètres : l'apparence, appliquée dès l'image suivante.
    fn parametres(&mut self, cx: &mut Cx, scroll: Id, r: Rect) {
        let t = &mut self.theme;
        let mut b = cx.body(scroll, r, &self.scrolls);
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
        let c = &mut self.clavier.reglages;
        for (i, disp) in Disposition::TOUTES.iter().enumerate() {
            if cx.row(&mut b, sub_id(scroll, 20 + i), disp.nom(), None, *disp == c.disposition) {
                c.disposition = *disp;
            }
        }
        let shown = format!("{:.0} dp", c.touche);
        cx.slider(&mut b, sub_id(scroll, 6), "Hauteur des touches", &shown, &mut c.touche, 40.0, 64.0);
        c.touche = c.touche.round();
        let shown = format!("{:.0} ms", c.appui_long * 1000.0);
        cx.slider(&mut b, sub_id(scroll, 7), "Appui long (variantes)", &shown, &mut c.appui_long, 0.2, 0.8);
        c.appui_long = (c.appui_long * 100.0).round() / 100.0;
        cx.toggle(&mut b, sub_id(scroll, 8), "Aperçu des touches", &mut c.apercu);
        cx.toggle(&mut b, sub_id(scroll, 9), "Clavier flottant", &mut c.flottant);
        clavier::champ(cx, &mut b, ESSAI, "Essayer le clavier…", &mut self.essai, &mut self.clavier);
        cx.section(&mut b, "FICHIER");
        let ou = match &self.fichier_reglages {
            Some(f) => format!("Ces réglages sont enregistrés dans {f}, un fichier texte qu'on peut modifier à la main."),
            None => "Ces réglages ne sont pas enregistrés.".to_string(),
        };
        cx.paragraph(&mut b, &ou);
        cx.body_end(b, &mut self.scrolls);
    }

    /// La vue Radio : une adresse à saisir, lecture et arrêt, le volume, et les derniers
    /// flux écoutés. Renvoie la ligne d'état.
    fn radio(&mut self, cx: &mut Cx, scroll: Id, champ: Id, r: Rect) -> String {
        let (col, h) = (cx.couleurs(), cx.commande());
        let (etat, actif, en_cours) = {
            let l = radio::lecteur();
            (l.etat(), l.actif(), l.adresse.clone())
        };
        let mut b = cx.body(scroll, r, &self.scrolls);
        cx.section(&mut b, "FLUX");
        clavier::champ(cx, &mut b, champ, "Adresse du flux (http…)", &mut self.adresse, &mut self.clavier);
        // Lecture ou arrêt, et l'état à côté
        let y = b.y + cx.ecart();
        if cx.pastille(sub_id(scroll, 1), b.x, y, if actif { Icon::Stop } else { Icon::Lecture }, actif) {
            if actif {
                radio::lecteur().arreter();
            } else {
                let a = if self.adresse.texte.trim().is_empty() { self.historique.liste.first().cloned().unwrap_or_default() } else { self.adresse.texte.clone() };
                self.jouer(&a);
            }
        }
        let x = b.x + h + cx.pad();
        let (titre, detail) = match &etat {
            Etat::Arret => ("Arrêt".to_string(), String::new()),
            Etat::Connexion => ("Connexion…".to_string(), en_cours.as_deref().map(radio::courte).unwrap_or("").to_string()),
            Etat::Lecture { .. } => ("Lecture".to_string(), etat.detail()),
            Etat::Erreur(e) => ("Erreur".to_string(), e.clone()),
        };
        cx.clip(Some(Rect::new(x, y, (b.x + b.w - x).max(0.0), h).intersect(&b.rect())));
        cx.text_at(x, y + h * 0.32, &titre, 16.0, if matches!(etat, Etat::Erreur(_)) { [0xE0, 0x3A, 0x3A, 255] } else { col.texte });
        cx.text_at(x, y + h * 0.75, &detail, 13.0, col.pale);
        cx.clip(Some(b.rect()));
        b.y = y + h + cx.ecart();
        self.mixage(cx, &mut b, scroll);
        // Les derniers flux : toucher pour écouter
        let (titre, liste) = if self.historique.liste.is_empty() { ("SUGGESTIONS", SUGGESTIONS.iter().map(|s| s.to_string()).collect()) } else { ("RÉCENTS", self.historique.liste.clone()) };
        cx.section(&mut b, titre);
        let mut choisi = None;
        for (i, a) in liste.iter().enumerate() {
            let joue = actif && en_cours.as_deref() == Some(a.as_str());
            if cx.row(&mut b, sub_id(scroll, 100 + i), radio::courte(a), None, joue) {
                choisi = Some(a.clone());
            }
        }
        if let Some(a) = choisi {
            self.adresse = Edition { curseur: a.chars().count(), texte: a.clone() };
            self.jouer(&a);
        }
        if let Some(f) = &self.historique.chemin {
            cx.paragraph(&mut b, &format!("Historique enregistré dans {}.", f.display()));
        }
        cx.body_end(b, &mut self.scrolls);
        match etat {
            Etat::Lecture { .. } => {
                let t = mixeur().pistes().into_iter().find(|p| p.nom == radio::PISTE).map_or(0.0, |p| p.tampon);
                format!("tampon {:.0} %", t * 100.0)
            }
            Etat::Connexion => "connexion".into(),
            Etat::Erreur(_) => "erreur".into(),
            Etat::Arret => "arrêt".into(),
        }
    }

    /// La vue Fichiers : le fil d'Ariane, le dossier parent, puis le contenu du
    /// dossier. Un dossier s'ouvre au toucher, un fichier audio se lit dans la piste
    /// « Fichier » du mixeur. Renvoie la ligne d'état.
    fn fichiers(&mut self, cx: &mut Cx, f: FenetreId, r: Rect) -> String {
        let (d, col) = (cx.dp, cx.couleurs());
        if !self.dossiers.contains_key(&f) {
            let depart = self.racine.clone().unwrap_or_else(|| PathBuf::from("/"));
            self.ouvrir_dossier(f, depart);
        }
        let dossier = self.dossiers[&f].clone();
        let base = sub_id(FICHIERS, f as usize);
        // Les empreintes viennent de l'analyse du stockage, lancée au besoin
        if self.analyse.is_none() {
            if let Some(racine) = self.racine.clone() {
                self.analyse = Some(Analyse::lancer(racine, radio::reveiller));
            }
        }
        let arbre = self.analyse.as_ref().and_then(|a| a.resultat().map(|(n, _)| (n, a.racine.clone())));
        let situation = arbre.as_ref().and_then(|(n, racine)| {
            let rel = dossier.chemin.strip_prefix(racine).ok()?;
            let noms: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
            disque::placer(n, &noms)
        });
        // Un défilement par dossier : en revenant au parent, on retrouve sa position
        let scroll = sub_id(base, hachage(&dossier.chemin));
        let lecture = radio::lecteur_fichiers().adresse.clone();
        let ligne = (cx.theme.ligne * d).round();
        let mut aller: Option<PathBuf> = None;
        let mut b = cx.body(scroll, r, &self.scrolls);

        // Le fil d'Ariane : chaque étape s'ouvre au toucher ; trop long, on voit la fin
        let etapes = explorateur::ariane(&dossier.chemin, self.racine.as_deref());
        let sep = "  ›  ";
        let largeur: f32 = etapes.iter().map(|(n, _)| cx.measure(n, 15.0)).sum::<f32>() + cx.measure(sep, 15.0) * (etapes.len() as f32 - 1.0);
        // La pastille + de la fenêtre occupe le coin haut gauche : le fil part à sa droite
        let x0 = (r.x + cx.coin() + cx.commande() + cx.ecart()).max(b.x);
        let place = b.x + b.w - x0;
        let mut x = x0 + (place - largeur).min(0.0);
        cx.clip(Some(Rect::new(x0, b.y, place.max(0.0), ligne).intersect(&b.rect())));
        for (i, (nom, chemin)) in etapes.iter().enumerate() {
            let w = cx.measure(nom, 15.0);
            let zone = Rect::new(x - 4.0 * d, b.y, w + 8.0 * d, ligne);
            let derniere = i + 1 == etapes.len();
            if cx.ui.click(sub_id(base, 1_000_000 + i), zone, Some(b.id())) && !derniere {
                aller = Some(chemin.clone());
            }
            cx.text_at(x, b.y + ligne / 2.0, nom, 15.0, if derniere { col.texte } else { col.accent });
            x += w;
            if !derniere {
                cx.text_at(x, b.y + ligne / 2.0, sep, 15.0, col.pale);
                x += cx.measure(sep, 15.0);
            }
        }
        cx.clip(Some(b.rect()));
        b.y += ligne;
        cx.dl.rrect(Rect::new(b.rect().x, b.y, b.rect().w, d.max(1.0)), 0.0, col.ligne);

        if let Some(parent) = dossier.chemin.parent() {
            if self.ligne_fichier(cx, &mut b, sub_id(base, 999_999), Puce::Icone(Icon::Haut), "Dossier parent", "", false) {
                aller = Some(parent.to_path_buf());
            }
        }
        if let Some(e) = &dossier.erreur {
            let aide = if e == "Accès refusé" { format!("{e}. Pour parcourir tout le stockage, accordez à Flow l'accès à tous les fichiers dans les réglages d'Android.") } else { format!("{e}.") };
            cx.paragraph(&mut b, &aide);
        } else if dossier.entrees.is_empty() {
            cx.paragraph(&mut b, "Dossier vide.");
        }
        for (i, e) in dossier.entrees.iter().enumerate() {
            let chemin = dossier.chemin.join(&e.nom);
            let icone = match e.nature {
                Nature::Dossier => Icon::Dossier,
                Nature::Audio => Icon::Note,
                Nature::Image => Icon::Photo,
                Nature::Video => Icon::Film,
                _ => Icon::Fichier,
            };
            let info = if e.dossier() { String::new() } else { explorateur::taille(e.taille) };
            let joue = lecture.as_deref() == chemin.to_str();
            // L'empreinte d'un dossier : sa place dans le graphique du stockage
            let puce = match (&situation, e.dossier()) {
                (Some(p), true) => p.noeud.enfants.iter().position(|n| n.nom == e.nom && n.dossier).map(|k| {
                    let span = p.arcs.last().copied().unwrap_or((0.0, std::f32::consts::TAU));
                    let mut arcs = p.arcs.clone();
                    arcs.push(disque::arc_enfant(p.noeud, span, k));
                    Puce::Empreinte(arcs, TEINTES[p.teinte.unwrap_or(k) % TEINTES.len()])
                }),
                _ => None,
            };
            let puce = puce.unwrap_or(Puce::Icone(icone));
            if self.ligne_fichier(cx, &mut b, sub_id(base, i), puce, &e.nom, &info, joue) {
                match e.nature {
                    Nature::Dossier => aller = Some(chemin),
                    Nature::Audio if joue => radio::lecteur_fichiers().arreter(),
                    Nature::Audio => radio::lecteur_fichiers().jouer(&chemin.to_string_lossy()),
                    _ => {}
                }
            }
        }
        b.y += cx.ecart();
        let mut caches = self.caches;
        cx.toggle(&mut b, sub_id(base, 999_998), "Fichiers cachés", &mut caches);
        cx.body_end(b, &mut self.scrolls);
        if caches != self.caches {
            self.caches = caches;
            aller = Some(dossier.chemin.clone());
        }
        if let Some(c) = aller {
            self.ouvrir_dossier(f, c);
        }
        let (nd, nf) = dossier.compte();
        let pluriel = |n: usize, mot: &str| format!("{n} {mot}{}", if n > 1 { "s" } else { "" });
        format!("{} · {}", pluriel(nd, "dossier"), pluriel(nf, "fichier"))
    }

    /// Une ligne de l'explorateur : icône, nom, et à droite une information en chasse
    /// fixe. Hors de la zone visible, elle ne fait que réserver sa place.
    #[allow(clippy::too_many_arguments)]
    fn ligne_fichier(&self, cx: &mut Cx, b: &mut crate::widgets::Body, id: Id, puce: Puce, nom: &str, info: &str, choisie: bool) -> bool {
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

    /// La vue Disque, à la façon de l'analyseur d'utilisation des disques de GNOME :
    /// l'occupation du stockage en haut, puis un graphique en anneaux (le dossier au
    /// centre, ses enfants autour, sur quatre niveaux) et la liste de ses enfants.
    /// Toucher un dossier le met au centre ; toucher le centre remonte.
    fn disque(&mut self, cx: &mut Cx, f: FenetreId, r: Rect) -> String {
        let (d, col) = (cx.dp, cx.couleurs());
        let base = sub_id(DISQUE, f as usize);
        if self.analyse.is_none() {
            if let Some(racine) = self.racine.clone() {
                self.analyse = Some(Analyse::lancer(racine, radio::reveiller));
            }
        }
        let Some(analyse) = &self.analyse else { return "aucun stockage".into() };
        let ligne = (cx.theme.ligne * d).round();
        let marge = cx.pad();

        // L'occupation du système de fichiers : texte et barre
        let mut haut = r.y + marge;
        if let Some((total, libre)) = disque::espace(&analyse.racine) {
            let utilise = total.saturating_sub(libre);
            let t = format!("{} utilisés sur {} · {} libres", explorateur::taille(utilise), explorateur::taille(total), explorateur::taille(libre));
            // À droite de la pastille + du coin de la fenêtre
            let x0 = r.x + cx.coin() + cx.commande() + cx.ecart();
            cx.clip(Some(r));
            cx.text_at(x0, haut + ligne * 0.35, &t, 14.0, col.texte);
            let barre = Rect::new(x0, haut + ligne * 0.7, (r.right() - marge - x0).max(0.0), 6.0 * d);
            cx.dl.rrect(barre, 3.0 * d, col.ligne);
            let k = if total > 0 { utilise as f32 / total as f32 } else { 0.0 };
            cx.dl.rrect(Rect::new(barre.x, barre.y, barre.w * k, barre.h), 3.0 * d, col.accent);
            cx.clip(None);
            haut += ligne + marge * 0.5;
        }
        let reste = Rect::new(r.x, haut, r.w, (r.bottom() - haut).max(0.0));

        let Some((arbre, duree)) = analyse.resultat() else {
            // Pendant l'analyse : ce qui est déjà compté
            let (n, o) = (analyse.progres.fichiers.load(std::sync::atomic::Ordering::Relaxed), analyse.progres.octets.load(std::sync::atomic::Ordering::Relaxed));
            cx.clip(Some(reste));
            cx.centered(Rect::new(reste.x, reste.y, reste.w, reste.h * 0.5), "Analyse du stockage…", 16.0, col.texte);
            cx.centered(Rect::new(reste.x, reste.cy() - ligne * 0.5, reste.w, ligne), &format!("{n} fichiers · {}", explorateur::taille(o)), 13.0, col.pale);
            cx.clip(None);
            return format!("analyse · {n} fichiers");
        };

        // Le dossier au centre : un chemin d'indices dans l'arbre (remis à la racine
        // s'il ne mène plus nulle part, après une nouvelle analyse)
        let mut centre = self.centres.get(&f).cloned().unwrap_or_default();
        if arbre.suivre(&centre).is_none() {
            centre.clear();
        }
        let noeud = arbre.suivre(&centre).unwrap_or(&arbre);
        let mut choisi = self.choisis.get(&f).cloned().filter(|c| arbre.suivre(c).is_some());

        // Le graphique et la liste : côte à côte en paysage, l'un sur l'autre sinon
        let paysage = reste.w > reste.h * 1.15;
        let (zone, liste) = if paysage {
            let c = reste.h.min(reste.w * 0.6);
            (Rect::new(reste.x, reste.y, c, reste.h), Rect::new(reste.x + c, reste.y, reste.w - c, reste.h))
        } else {
            let c = reste.w.min(reste.h * 0.6);
            (Rect::new(reste.x, reste.y, reste.w, c), Rect::new(reste.x, reste.y + c, reste.w, reste.h - c))
        };
        let rayon = (zone.w.min(zone.h) / 2.0 - marge).max(0.0);
        let c = [zone.cx(), zone.cy()];
        let r0 = rayon * 0.28;
        let large = (rayon - r0) / ANNEAUX as f32;
        let ecart = 1.5 * d;
        let teinte = |s: &disque::Secteur| -> Color {
            let t = TEINTES[s.teinte % TEINTES.len()];
            // Plus loin du centre, plus pâle ; les fichiers un peu éteints
            let k = 0.17 * (s.anneau as f32 - 1.0) + if s.dossier { 0.0 } else { 0.25 };
            let m = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * k.min(0.8)) as u8;
            [m(t[0], col.surface[0]), m(t[1], col.surface[1]), m(t[2], col.surface[2]), 255]
        };
        let secteurs = if rayon > 0.0 { disque::secteurs(noeud, ANNEAUX, 3.0 * d / rayon) } else { Vec::new() };
        let q = std::f32::consts::FRAC_PI_2;
        cx.clip(Some(zone));
        cx.dl.rrect(Rect::new(c[0] - r0 + ecart, c[1] - r0 + ecart, 2.0 * (r0 - ecart), 2.0 * (r0 - ecart)), r0, col.puce);
        for s in &secteurs {
            let a = r0 + (s.anneau as f32 - 1.0) * large;
            // L'élément choisi prend la couleur d'accent
            let plein: Vec<usize> = centre.iter().chain(&s.chemin).copied().collect();
            let t = if choisi.as_ref() == Some(&plein) { col.accent } else { teinte(s) };
            cx.dl.secteur(c, a + ecart / 2.0, a + large - ecart / 2.0, s.a0 - q, s.a1 - q, ecart, t);
        }
        // Au centre : le nom et la taille du dossier
        let nom = if centre.is_empty() { "Stockage" } else { noeud.nom.as_str() };
        let interieur = Rect::new(c[0] - r0 * 0.85, c[1] - r0 * 0.6, r0 * 1.7, r0 * 1.2);
        cx.clip(Some(interieur.intersect(&zone)));
        cx.centered(Rect::new(interieur.x, c[1] - ligne * 0.45, interieur.w, ligne * 0.5), nom, 14.0, col.texte);
        cx.centered(Rect::new(interieur.x, c[1] + ligne * 0.0, interieur.w, ligne * 0.45), &explorateur::taille(noeud.taille), 12.0, col.pale);
        cx.clip(None);

        // Toucher le graphique : un dossier vient au centre, un fichier est choisi, le
        // centre remonte d'un niveau
        let graphique = sub_id(base, 1);
        cx.ui.region_add(graphique, zone, crate::ui::Sense::Click { scroll: None });
        let mut nouveau_centre: Option<Vec<usize>> = None;
        if let Some((x, y)) = cx.ui.clic_en(graphique) {
            let (dx, dy) = (x - c[0], y - c[1]);
            let dist = (dx * dx + dy * dy).sqrt();
            if dist < r0 {
                if !centre.is_empty() {
                    let mut p = centre.clone();
                    p.pop();
                    nouveau_centre = Some(p);
                }
            } else if large > 0.0 {
                let anneau = ((dist - r0) / large) as usize + 1;
                let angle = (dy.atan2(dx) + q).rem_euclid(std::f32::consts::TAU);
                if let Some(s) = disque::toucher(&secteurs, anneau, angle) {
                    let plein: Vec<usize> = centre.iter().chain(&s.chemin).copied().collect();
                    if s.dossier {
                        nouveau_centre = Some(plein);
                    } else {
                        choisi = Some(plein);
                    }
                }
            }
        }

        // La liste des enfants du centre, avec la teinte de leur secteur
        let scroll = sub_id(base, hachage_indices(&centre));
        let mut b = cx.body(scroll, liste, &self.scrolls);
        if !centre.is_empty() && self.ligne_fichier(cx, &mut b, sub_id(scroll, 999_999), Puce::Icone(Icon::Haut), "Dossier parent", "", false) {
            let mut p = centre.clone();
            p.pop();
            nouveau_centre = Some(p);
        }
        for (i, e) in noeud.enfants.iter().enumerate() {
            let s = disque::Secteur { chemin: vec![i], anneau: 1, a0: 0.0, a1: 0.0, teinte: i, dossier: e.dossier };
            let part = if noeud.taille > 0 { 100.0 * e.taille as f32 / noeud.taille as f32 } else { 0.0 };
            let info = format!("{} · {:>3.0} %", explorateur::taille(e.taille), part);
            let plein: Vec<usize> = centre.iter().copied().chain([i]).collect();
            let est_choisi = choisi.as_ref() == Some(&plein);
            let puce = if e.dossier || est_choisi { Puce::Teinte(teinte(&s)) } else { Puce::Icone(Icon::Fichier) };
            if self.ligne_fichier(cx, &mut b, sub_id(scroll, i), puce, &e.nom, &info, est_choisi) {
                if e.dossier {
                    nouveau_centre = Some(plein);
                } else {
                    choisi = Some(plein);
                }
            }
        }
        b.y += cx.ecart();
        let relancer = cx.button(&mut b, sub_id(scroll, 999_998), "Relancer l'analyse");
        cx.body_end(b, &mut self.scrolls);

        let statut = match choisi.as_ref().and_then(|c| arbre.suivre(c)) {
            Some(n) => format!("{} · {}", n.nom, explorateur::taille(n.taille)),
            None => format!("{} fichiers · {} · {:.1} s", arbre.fichiers, explorateur::taille(arbre.taille), duree),
        };
        if let Some(p) = nouveau_centre {
            self.centres.insert(f, p);
            choisi = None;
        }
        match choisi {
            Some(c) => self.choisis.insert(f, c),
            None => self.choisis.remove(&f),
        };
        if relancer {
            let racine = analyse.racine.clone();
            self.analyse = Some(Analyse::lancer(racine, radio::reveiller));
        }
        statut
    }

    /// Le tableau du mixeur : la voie d'essai à lancer, chaque piste connue avec son
    /// vumètre, son volume, sa balance et ses bascules, puis le volume général.
    fn mixage(&mut self, cx: &mut Cx, b: &mut crate::widgets::Body, scroll: Id) {
        let m = mixeur();
        cx.section(b, "MIXAGE");
        let mut essai = mixage::essai_actif();
        cx.toggle(b, sub_id(scroll, 3), "Voie d'essai (arpège)", &mut essai);
        if essai != mixage::essai_actif() {
            if let Err(e) = mixage::essai(essai) {
                crate::android::log(&format!("voie d'essai : {e}"));
            }
        }
        for (k, p) in m.pistes().into_iter().enumerate() {
            let base = sub_id(scroll, 200 + 10 * k);
            self.piste(cx, b, base, &p);
        }
        // Le volume général, avec le vumètre de la sortie
        let ligne = (cx.theme.ligne * cx.dp).round();
        let info = match (m.niveau(), m.erreur()) {
            (_, Some(e)) => e,
            (Some(_), None) if m.en_pause() => "en pause".into(),
            (Some(_), None) => format!("sortie {:.1} kHz", m.frequence() as f32 / 1000.0),
            (None, None) => "sortie fermée".into(),
        };
        self.entete(cx, b, "Général", &info, m.niveau(), ligne);
        let mut g = m.general.get();
        let avant = g;
        let shown = format!("{:.0} %", g * 100.0);
        cx.slider(b, sub_id(scroll, 4), "Volume général", &shown, &mut g, 0.0, 1.0);
        if g != avant {
            m.general.set((g * 20.0).round() / 20.0);
        }
        if let Some(e) = m.erreur() {
            cx.paragraph(b, &e);
        }
    }

    /// Le nom d'une piste, un texte pâle et son vumètre (deux barres, gauche et droite).
    fn entete(&self, cx: &mut Cx, b: &mut crate::widgets::Body, nom: &str, info: &str, niveau: Option<[f32; 2]>, ligne: f32) {
        let (d, col) = (cx.dp, cx.couleurs());
        let y = b.y;
        let w = cx.measure(nom, 16.0);
        cx.text_at(b.x, y + ligne / 2.0, nom, 16.0, col.texte);
        cx.text_at(b.x + w + 8.0 * d, y + ligne / 2.0, info, 12.0, col.pale);
        let x0 = b.x + (b.w * 0.5).max(w + 8.0 * d + cx.measure(info, 12.0) + 12.0 * d).min(b.w * 0.7);
        let large = b.x + b.w - x0;
        let (e, ep) = (5.0 * d, 3.0 * d);
        for ch in 0..2 {
            let r = Rect::new(x0, y + ligne / 2.0 - e - ep / 2.0 + ch as f32 * (e + ep), large, e);
            cx.dl.rrect(r, e / 2.0, col.ligne);
            if let Some(n) = niveau {
                let v = mixage::vumetre(n[ch]);
                // Vert, puis l'accent près du maximum, rouge à la saturation
                let teinte = if n[ch] >= 0.99 { [0xE0, 0x3A, 0x3A, 255] } else if v > 0.85 { col.accent } else { [0x22, 0xC5, 0x5E, 255] };
                if v > 0.0 {
                    cx.dl.rrect(Rect::new(x0, r.y, (large * v).max(e), e), e / 2.0, teinte);
                }
            }
        }
        b.y += ligne;
    }

    fn piste(&mut self, cx: &mut Cx, b: &mut crate::widgets::Body, base: Id, p: &Piste) {
        let m = mixeur();
        let ligne = (cx.theme.ligne * cx.dp).round();
        let info = if p.niveau.is_some() { "joue" } else { "silencieuse" };
        self.entete(cx, b, &p.nom, info, p.niveau, ligne);
        let mut r: Reglage = p.reglage;
        let shown = format!("{:.0} %", r.gain * 100.0);
        cx.slider(b, sub_id(base, 0), "Volume", &shown, &mut r.gain, 0.0, 1.0);
        r.gain = (r.gain * 20.0).round() / 20.0;
        let shown = match (r.balance * 100.0).round() as i32 {
            0 => "centre".to_string(),
            v if v < 0 => format!("G {}", -v),
            v => format!("D {v}"),
        };
        cx.slider(b, sub_id(base, 1), "Balance", &shown, &mut r.balance, -1.0, 1.0);
        r.balance = (r.balance * 20.0).round() / 20.0;
        // Les bascules, en pilules, sous les curseurs
        let (gap, h) = (cx.ecart().max(4.0 * cx.dp), cx.commande());
        let mut x = b.x;
        let y = b.y + cx.ecart() * 0.5;
        for (i, (label, v)) in [("Muet", &mut r.muet), ("Solo", &mut r.solo), ("Prioritaire", &mut r.prioritaire)].into_iter().enumerate() {
            if cx.pill(sub_id(base, 2 + i), x, y, label, *v) {
                *v = !*v;
            }
            x += cx.pill_width(label) + gap;
        }
        b.y = y + h + cx.ecart();
        if r != p.reglage {
            m.regler(&p.nom, r);
        }
    }

    /// La barre de l'application : pastilles aplaties à gauche (la rustine, puis les
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
        if cx.pastille_plate(RUSTINE, case(0.0), Icon::Rustine, end && content == EndContent::APropos) {
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
        let mut b = cx.body(scroll, body, &self.scrolls);
        for (i, (name, color)) in COLORS.iter().enumerate() {
            if cx.row(&mut b, sub_id(scroll, i), name, Some(*color), i == self.color) {
                self.color = i;
            }
        }
        cx.body_end(b, &mut self.scrolls);
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
        match self.end_content {
            EndContent::Display => {
                self.display_tab = cx.tabs(id("onglets-affichage"), tabs.x, tabs.y, &["Vue", "Cube"], self.display_tab);
                let scroll = sub_id(id("corps-affichage"), self.display_tab);
                let mut b = cx.body(scroll, body, &self.scrolls);
                if self.display_tab == 0 {
                    cx.section(&mut b, "CAMÉRA");
                    let shown = format!("{:.0}°", self.fov);
                    cx.slider(&mut b, id("champ"), "Champ de vision", &shown, &mut self.fov, 25.0, 90.0);
                    if cx.button(&mut b, id("recadrer-panneau"), "Recadrer") {
                        self.cams.clear();
                    }
                    cx.section(&mut b, "ROTATION");
                    cx.toggle(&mut b, id("rotation-panneau"), "Rotation automatique", &mut self.spin);
                    let shown = format!("{:.0}°/s", self.speed);
                    cx.slider(&mut b, id("vitesse"), "Vitesse", &shown, &mut self.speed, 5.0, 180.0);
                } else {
                    cx.section(&mut b, "FORME");
                    let shown = format!("{:.2}", self.size);
                    cx.slider(&mut b, id("taille"), "Taille", &shown, &mut self.size, 0.4, 2.0);
                    cx.section(&mut b, "APPARENCE");
                    cx.toggle(&mut b, id("faces"), "Faces colorées", &mut self.faces);
                }
                cx.body_end(b, &mut self.scrolls);
            }
            EndContent::APropos => {
                cx.tabs(id("onglets-a-propos"), tabs.x, tabs.y, &["Flow"], 0);
                let mut b = cx.body(id("corps-a-propos"), body, &self.scrolls);
                cx.section(&mut b, &format!("FLOW {}", env!("CARGO_PKG_VERSION")));
                cx.paragraph(&mut b, "Socle d'applications Android en Rust : interface tactile et 3D en OpenGL ES 3 ; un seul service Java, pour la lecture en arrière-plan.");
                cx.paragraph(&mut b, "Réutilisable à des fins non commerciales, selon la licence PolyForm Noncommercial 1.0.0.");
                cx.section(&mut b, "BARRE");
                cx.paragraph(&mut b, "Le centre du bas de l'écran est réservé aux gestes du système, mais les appuis y restent libres : la barre de l'application y range ses pastilles.");
                if let Some(f) = &self.fichier_reglages {
                    cx.section(&mut b, "RÉGLAGES");
                    cx.paragraph(&mut b, f);
                }
                cx.body_end(b, &mut self.scrolls);
            }
            EndContent::Help => {
                cx.tabs(id("onglets-aide"), tabs.x, tabs.y, &["Aide"], 0);
                let mut b = cx.body(id("corps-aide"), body, &self.scrolls);
                cx.section(&mut b, "GESTES");
                cx.paragraph(&mut b, "Un doigt sur l'espace de travail fait tourner la vue.");
                cx.paragraph(&mut b, "Deux doigts zooment et déplacent la vue.");
                cx.section(&mut b, "PANNEAUX");
                cx.paragraph(&mut b, "Les pastilles de la barre du bas déploient les panneaux. L'espace de travail s'adapte à la place restante.");
                cx.paragraph(&mut b, "Glissez la frontière d'un panneau pour le redimensionner ; un double appui rétablit sa taille.");
                cx.body_end(b, &mut self.scrolls);
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
        let model = math::scale(self.size);
        let vues = placees
            .iter()
            .filter(|p| p.vue == VUE_CUBE && p.contenu.w >= 1.0 && p.contenu.h >= 1.0)
            .map(|p| {
                let c = self.cams.get(&p.id).copied().unwrap_or(HOME);
                let (sy, cy, sp, cp) = (c.yaw.sin(), c.yaw.cos(), c.pitch.sin(), c.pitch.cos());
                let eye = [c.target[0] + c.dist * sy * cp, c.target[1] + c.dist * sp, c.target[2] + c.dist * cy * cp];
                let view = math::look_at(eye, c.target, [0.0, 1.0, 0.0]);
                // Le rapport suit la fenêtre : c'est lui qui « adapte » la 3D à la place.
                let proj = math::perspective(self.fov.to_radians(), p.contenu.w / p.contenu.h, 0.1, 100.0);
                Vue3d { rect: p.contenu, mvp: math::mul(&proj, &math::mul(&view, &model)) }
            })
            .collect();
        let [r, g, b, _] = COLORS[self.color].1;
        Scene { workspace: w, vues, fond: Theme::default().couleurs().fond, color: [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0], tint: if self.faces { 0.25 } else { 0.0 } }
    }
}

/// Ce qui précède le nom d'une ligne de liste : une icône ou une pastille de couleur.
enum Puce {
    Icone(Icon),
    Teinte(Color),
    /// L'empreinte d'un dossier : ses angles du premier anneau jusqu'à lui, et la
    /// teinte de sa branche.
    Empreinte(Vec<(f32, f32)>, Color),
}

/// Niveaux montrés au plus dans une empreinte : au-delà, les plus proches du dossier.
const EMPREINTE_NIVEAUX: usize = 5;
/// Ouverture minimale d'un secteur d'empreinte, pour qu'il reste visible.
const EMPREINTE_MIN: f32 = 0.4;

/// Dessine une empreinte dans le carré `r` : un anneau par niveau, les ancêtres en
/// pâle, le dossier en pleine teinte.
fn empreinte(cx: &mut Cx, r: Rect, arcs: &[(f32, f32)], teinte: Color) {
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

/// Un nombre tiré d'un chemin d'indices, pour donner à chaque dossier son défilement.
fn hachage_indices(c: &[usize]) -> usize {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    c.hash(&mut h);
    h.finish() as usize
}

/// Un nombre tiré d'un chemin, pour donner à chaque dossier son défilement.
fn hachage(p: &std::path::Path) -> usize {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    p.hash(&mut h);
    h.finish() as usize
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

    // Centre de la n-ième pastille des panneaux, dans la barre du bas (après la rustine).
    fn pastille(n: f32) -> (f32, f32) {
        (72.0 * (n + 1.5), 1200.0 - 36.0)
    }

    #[test]
    fn la_taille_des_commandes_suit_le_theme() {
        let mut r = Rig::new();
        r.app.theme.commande = 60.0;
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
        r.frames(60);
        let yaw = r.app.cam(1).yaw;
        r.tap(200.0, 1.5 * (60.0 + 8.0 + 48.0 * 3.5));
        assert_eq!(r.app.color, 3);
        assert_eq!(r.app.cam(1).yaw, yaw);
    }

    #[test]
    fn glisser_sur_l_espace_de_travail_tourne_la_vue() {
        let mut r = Rig::new();
        let yaw = r.app.cam(1).yaw;
        r.ui.event(Event::Down { id: 0, x: 900., y: 600. }, 0.0);
        r.ui.event(Event::Move { id: 0, x: 1000., y: 600. }, 0.1);
        r.ui.event(Event::Move { id: 0, x: 1100., y: 600. }, 0.1);
        r.frames(1);
        assert!(r.app.cam(1).yaw < yaw);
    }

    #[test]
    fn la_poignee_redimensionne_dans_la_demo() {
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
    fn chaque_fenetre_3d_a_sa_camera() {
        let mut r = Rig::new();
        let nouvelle = r.app.surface.partager(1, crate::surface::Sens::Colonnes, 0.5).unwrap();
        let s = r.frames(1);
        assert_eq!(s.vues.len(), 2);
        assert_eq!(s.vues[0].rect.w, 960.0);
        // Un geste dans la fenêtre de droite ne tourne qu'elle
        let (a, b) = (r.app.cam(nouvelle).yaw, r.app.cam(1).yaw);
        r.ui.event(Event::Down { id: 0, x: 1400., y: 600. }, 0.0);
        r.ui.event(Event::Move { id: 0, x: 1500., y: 600. }, 0.1);
        r.ui.event(Event::Move { id: 0, x: 1600., y: 600. }, 0.1);
        r.frames(1);
        assert_eq!(r.app.cam(nouvelle).yaw, a);
        assert!(r.app.cam(1).yaw < b);
        // Une fenêtre passée en liste n'a plus de 3D
        r.app.surface.changer_vue(nouvelle, VUE_COULEURS);
        assert_eq!(r.frames(1).vues.len(), 1);
    }

    #[test]
    fn rien_ne_redessine_au_repos() {
        let mut r = Rig::new();
        r.frames(3);
        assert!(!r.app.animating());
    }
}
