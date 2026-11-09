//! Le contexte Encre : dessiner à main levée sur une feuille sans bords, enregistrée en
//! SVG.
//!
//! - **Un doigt** trace, au bout du curseur déporté (voir `crate::gestes`) ; un tap
//!   (ou un appui tenu sans glisser) pose un point. Avec la gomme, le curseur efface les traits qu'il touche.
//! - **Deux doigts** naviguent : pincer zoome autour du centre des doigts, les glisser
//!   déplace la feuille. Les deux doigts se posent ensemble : un second doigt arrivé
//!   pendant un tracé est ignoré.
//! - **La barre du bas** choisit l'épaisseur, la couleur, la gomme ; elle annule et
//!   rétablit (l'historique dure le temps de la session) ; le zoom affiché y ramène la
//!   vue à 100 %.
//!
//! Le dessin est un document SVG (`Documents/Encre/dessin-N.svg` pour un nouveau),
//! enregistré à chaque trait terminé. Chaque trait y est un `<path>` fait de segments
//! droits, en unités qui valent un dp à 100 %. Un SVG venu d'ailleurs s'ouvre en
//! lecture seule : Encre n'en montre que les tracés simples qu'il comprend, et ne
//! l'écrase jamais.

use crate::contexte::{Commun, Contexte, Etat, Genre};
use crate::draw::{Color, Rect};
use crate::gestes::Geste;
use crate::surface::Placee;
use crate::ui::{id, sub_id, Id};
use crate::widgets::{Cx, Icon};
use std::path::{Path, PathBuf};

pub const CLE: &str = "encre";
pub const GENRE: Genre = Genre { cle: CLE, nom: "Encre", icone: Icon::Encre, ouvrir };

const ENCRE: Id = id("encre");

/// Les épaisseurs proposées, en unités du dessin (dp à 100 %).
pub const TAILLES: [f32; 4] = [2.0, 4.0, 8.0, 16.0];
/// Les encres proposées : sur la feuille, toujours claire, quel que soit le thème.
pub const COULEURS: [Color; 8] = [
    [0x1E, 0x1E, 0x25, 255],
    [0xFF, 0xFF, 0xFF, 255],
    [0x6B, 0x72, 0x80, 255],
    [0xDC, 0x26, 0x26, 255],
    [0xEA, 0x8A, 0x0C, 255],
    [0x16, 0xA3, 0x4A, 255],
    [0x25, 0x63, 0xEB, 255],
    [0x93, 0x33, 0xEA, 255],
];
const PAPIER: Color = [0xFB, 0xFA, 0xF6, 255];
const ZOOM: (f32, f32) = (0.1, 12.0);
/// Le rayon de la gomme, à l'écran, en dp.
const GOMME: f32 = 12.0;

#[derive(Clone, Debug, PartialEq)]
pub struct Trait {
    pub couleur: Color,
    pub largeur: f32,
    /// Les points, en unités du dessin.
    pub points: Vec<(f32, f32)>,
}

/// Une modification du dessin, pour annuler et rétablir.
#[derive(Clone, Debug, PartialEq)]
enum Action {
    /// Un trait ajouté (au bout de la liste).
    Ajout(Trait),
    /// Des traits gommés, dans l'ordre : chacun à sa place au moment où il est parti.
    Effacement(Vec<(usize, Trait)>),
}

/// Le nombre d'actions qu'on peut annuler.
const HISTORIQUE: usize = 500;

/// La vue : le point du dessin au coin haut gauche de la feuille, et le zoom.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Vue {
    x: f32,
    y: f32,
    zoom: f32,
}

#[derive(Clone)]
pub struct Encre {
    chemin: Option<PathBuf>,
    traits: Vec<Trait>,
    charge: bool,
    /// Un SVG venu d'ailleurs : on ne l'écrase pas.
    lecture_seule: bool,
    message: Option<String>,
    taille: usize,
    couleur: usize,
    gomme: bool,
    vue: Vue,
    /// Un trait en cours (le dernier de `traits`), ou ce que la gomme en cours a effacé.
    trace: bool,
    gommes: Vec<(usize, Trait)>,
    /// L'historique : ce qu'on peut annuler, puis rétablir.
    annuler: Vec<Action>,
    refaire: Vec<Action>,
    /// Le dernier SVG lu ou écrit : un fichier inchangé n'est pas relu (l'historique reste).
    connu: String,
    /// L'échelle du pincement à l'événement précédent.
    pincement: f32,
    /// La feuille et le dp à l'image précédente, pour passer de l'écran au dessin.
    zone: Rect,
    dp: f32,
}

fn ouvrir(e: &Etat) -> Box<dyn Contexte> {
    Box::new(lire(e))
}

/// L'instance, depuis son état enregistré.
fn lire(e: &Etat) -> Encre {
    let v = e.nombres("vue");
    let vue = match v[..] {
        [x, y, z] => Vue { x, y, zoom: z.clamp(ZOOM.0, ZOOM.1) },
        _ => Vue { x: 0.0, y: 0.0, zoom: 1.0 },
    };
    Encre {
        chemin: e.texte("document").filter(|t| !t.is_empty()).map(PathBuf::from),
        traits: Vec::new(),
        charge: false,
        lecture_seule: false,
        message: None,
        taille: (e.nombre("taille").unwrap_or(1.0) as usize).min(TAILLES.len() - 1),
        // La couleur par son code (#rrggbb) ; un indice venu de la 0.8 donne le noir
        couleur: e.texte("couleur").and_then(couleur_svg).and_then(|c| COULEURS.iter().position(|k| *k == c)).unwrap_or(0),
        gomme: e.oui("gomme").unwrap_or(false),
        vue,
        trace: false,
        gommes: Vec::new(),
        annuler: Vec::new(),
        refaire: Vec::new(),
        connu: String::new(),
        pincement: 1.0,
        zone: Rect::new(0.0, 0.0, 1.0, 1.0),
        dp: 1.0,
    }
}

fn hex(c: Color) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

/// Le dessin en SVG : un `<path>` par trait, la vue ajustée à ce qui est dessiné.
pub fn vers_svg(traits: &[Trait]) -> String {
    let mut b = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    for t in traits {
        for p in &t.points {
            let m = t.largeur / 2.0;
            b = [b[0].min(p.0 - m), b[1].min(p.1 - m), b[2].max(p.0 + m), b[3].max(p.1 + m)];
        }
    }
    if traits.is_empty() {
        b = [0.0, 0.0, 100.0, 100.0];
    }
    let (x, y, w, h) = (b[0] - 10.0, b[1] - 10.0, b[2] - b[0] + 20.0, b[3] - b[1] + 20.0);
    let mut s = format!("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"{x:.1} {y:.1} {w:.1} {h:.1}\" width=\"{w:.0}\" height=\"{h:.0}\">\n<!-- Dessiné avec Encre, de Flow -->\n");
    for t in traits {
        let mut d = String::new();
        for (i, p) in t.points.iter().enumerate() {
            d += &format!("{}{:.1} {:.1}", if i == 0 { "M" } else { " L" }, p.0, p.1);
        }
        if t.points.len() == 1 {
            // Un point : un segment nul, que les bouts ronds rendent visible
            d += &format!(" L{:.1} {:.1}", t.points[0].0, t.points[0].1);
        }
        s += &format!("<path d=\"{d}\" fill=\"none\" stroke=\"{}\" stroke-width=\"{:.1}\" stroke-linecap=\"round\" stroke-linejoin=\"round\"/>\n", hex(t.couleur), t.largeur);
    }
    s += "</svg>\n";
    s
}

fn attribut<'a>(balise: &'a str, nom: &str) -> Option<&'a str> {
    let i = balise.find(&format!(" {nom}=\""))? + nom.len() + 3;
    let fin = balise[i..].find('"')?;
    Some(&balise[i..i + fin])
}

fn couleur_svg(v: &str) -> Option<Color> {
    let h = v.trim().strip_prefix('#')?;
    let n = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok();
    match h.len() {
        6 => Some([n(0)?, n(2)?, n(4)?, 255]),
        3 => {
            let c = |i: usize| u8::from_str_radix(h.get(i..i + 1)?, 16).ok().map(|x| x * 17);
            Some([c(0)?, c(1)?, c(2)?, 255])
        }
        _ => None,
    }
}

/// Les points d'un chemin fait de M et L (absolus) ; `None` pour toute autre commande.
fn chemin_simple(d: &str) -> Option<Vec<(f32, f32)>> {
    let d = d.replace(',', " ").replace('M', " M ").replace('L', " L ");
    let mut nombres = Vec::new();
    for mot in d.split_whitespace() {
        match mot {
            "M" | "L" => {}
            n => nombres.push(n.parse::<f32>().ok().filter(|x| x.is_finite())?),
        }
    }
    if nombres.len() < 2 || nombres.len() % 2 == 1 {
        return None;
    }
    let mut points: Vec<(f32, f32)> = nombres.chunks_exact(2).map(|c| (c[0], c[1])).collect();
    // Le segment nul d'un point
    if points.len() == 2 && points[0] == points[1] {
        points.pop();
    }
    Some(points)
}

/// Relit un SVG : les traits compris, et vrai si tout l'a été (sinon, lecture seule).
pub fn depuis_svg(texte: &str) -> (Vec<Trait>, bool) {
    let mut traits = Vec::new();
    let mut tout = texte.contains("<svg");
    for (k, morceau) in texte.split('<').enumerate() {
        if k == 0 {
            tout &= morceau.trim().is_empty();
            continue;
        }
        let nom: String = morceau.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '?' || *c == '!' || *c == '/').collect();
        match nom.as_str() {
            "svg" | "/svg" | "?xml" | "!--" => {}
            _ if morceau.starts_with("!--") => {}
            "path" => {
                let balise = morceau.split('>').next().unwrap_or("");
                let lu = (|| {
                    let points = chemin_simple(attribut(balise, "d")?)?;
                    let couleur = couleur_svg(attribut(balise, "stroke")?)?;
                    let largeur = attribut(balise, "stroke-width").map_or(Some(1.0), |w| w.trim().parse::<f32>().ok())?;
                    Some(Trait { couleur, largeur: largeur.clamp(0.1, 500.0), points })
                })();
                match lu {
                    Some(t) => traits.push(t),
                    None => tout = false,
                }
                tout &= !balise.contains("transform");
            }
            _ => tout = false,
        }
    }
    (traits, tout)
}

/// La distance du point `p` au segment `a`-`b`.
fn distance_segment(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let l = dx * dx + dy * dy;
    let k = if l > 0.0 { (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / l).clamp(0.0, 1.0) } else { 0.0 };
    (p.0 - a.0 - k * dx).hypot(p.1 - a.1 - k * dy)
}

impl Trait {
    fn touche(&self, p: (f32, f32), rayon: f32) -> bool {
        let r = rayon + self.largeur / 2.0;
        match self.points[..] {
            [a] => (p.0 - a.0).hypot(p.1 - a.1) <= r,
            _ => self.points.windows(2).any(|w| distance_segment(p, w[0], w[1]) <= r),
        }
    }
}

impl Encre {
    /// Un point de l'écran, dans le dessin.
    fn vers_dessin(&self, x: f32, y: f32) -> (f32, f32) {
        let k = self.vue.zoom * self.dp;
        (self.vue.x + (x - self.zone.x) / k, self.vue.y + (y - self.zone.y) / k)
    }

    fn vers_ecran(&self, p: (f32, f32)) -> [f32; 2] {
        let k = self.vue.zoom * self.dp;
        [self.zone.x + (p.0 - self.vue.x) * k, self.zone.y + (p.1 - self.vue.y) * k]
    }

    fn charger(&mut self) {
        self.charge = true;
        let Some(c) = &self.chemin else { return };
        match std::fs::read_to_string(c) {
            Ok(t) if t == self.connu => {}
            Ok(t) => {
                let (traits, tout) = depuis_svg(&t);
                // Un autre dessin : l'historique ne s'y applique plus
                self.annuler.clear();
                self.refaire.clear();
                self.connu = t;
                self.traits = traits;
                self.lecture_seule = !tout;
                self.message = (!tout).then(|| "Ce SVG ne vient pas d'Encre : lecture seule, seuls ses tracés simples sont montrés.".to_string());
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                self.lecture_seule = true;
                self.message = Some(e.to_string());
            }
        }
    }

    /// Le fichier d'un nouveau dessin : `Documents/Encre/dessin-N.svg`, sans écraser.
    fn destination(c: &Commun) -> Option<PathBuf> {
        let d = c.racine.as_ref()?.join("Documents").join("Encre");
        (1..).map(|n| d.join(format!("dessin-{n}.svg"))).find(|p| !p.exists())
    }

    fn enregistrer(&mut self, c: &Commun) {
        if self.lecture_seule {
            return;
        }
        if self.chemin.is_none() {
            self.chemin = Encre::destination(c);
        }
        let Some(ch) = self.chemin.clone() else {
            self.message = Some("Pas de stockage : le dessin n'est pas enregistré.".into());
            return;
        };
        let svg = vers_svg(&self.traits);
        self.message = super::texte::ecrire_fichier(&ch, &svg).err();
        self.connu = svg;
    }

    fn noter(&mut self, a: Action) {
        self.annuler.push(a);
        if self.annuler.len() > HISTORIQUE {
            self.annuler.remove(0);
        }
        self.refaire.clear();
    }

    /// Défait la dernière action (ou rétablit la dernière annulée). Vrai si le dessin a changé.
    fn defaire(&mut self, retablir: bool) -> bool {
        let (de, vers) = if retablir { (&mut self.refaire, &mut self.annuler) } else { (&mut self.annuler, &mut self.refaire) };
        let Some(a) = de.pop() else { return false };
        match (&a, retablir) {
            (Action::Ajout(_), false) => {
                self.traits.pop();
            }
            (Action::Ajout(t), true) => self.traits.push(t.clone()),
            (Action::Effacement(v), false) => {
                for (i, t) in v.iter().rev() {
                    self.traits.insert((*i).min(self.traits.len()), t.clone());
                }
            }
            (Action::Effacement(v), true) => {
                for (i, _) in v {
                    if *i < self.traits.len() {
                        self.traits.remove(*i);
                    }
                }
            }
        }
        vers.push(a);
        true
    }

    /// Termine un trait ou un coup de gomme : il entre dans l'historique.
    fn conclure(&mut self) -> bool {
        if std::mem::take(&mut self.trace) {
            if let Some(t) = self.traits.last().cloned() {
                self.noter(Action::Ajout(t));
            }
            return true;
        }
        let g = std::mem::take(&mut self.gommes);
        let change = !g.is_empty();
        if change {
            self.noter(Action::Effacement(g));
        }
        change
    }

    fn gommer(&mut self, x: f32, y: f32) {
        let p = self.vers_dessin(x, y);
        let r = GOMME / self.vue.zoom;
        // De la fin vers le début : chaque place notée reste juste au moment du retrait
        for i in (0..self.traits.len()).rev() {
            if self.traits[i].touche(p, r) {
                let t = self.traits.remove(i);
                self.gommes.push((i, t));
            }
        }
    }

    /// Un événement de la feuille. Renvoie vrai si le dessin est à enregistrer.
    fn recevoir(&mut self, g: Geste) -> bool {
        let modifiable = !self.lecture_seule;
        match g {
            // Un contact sans glissement, bref ou tenu, pose un point (ou gomme)
            Geste::Tap { x, y, .. } | Geste::Appui { x, y, .. } | Geste::AppuiLong { x, y, .. } if modifiable => {
                if self.gomme {
                    self.gommes.clear();
                    self.gommer(x, y);
                } else {
                    let p = self.vers_dessin(x, y);
                    self.traits.push(Trait { couleur: COULEURS[self.couleur], largeur: TAILLES[self.taille], points: vec![p] });
                    self.trace = true;
                }
                self.conclure()
            }
            Geste::SaisieDebut { x, y, .. } if modifiable => {
                if self.gomme {
                    self.gommes.clear();
                    self.gommer(x, y);
                } else {
                    let p = self.vers_dessin(x, y);
                    self.traits.push(Trait { couleur: COULEURS[self.couleur], largeur: TAILLES[self.taille], points: vec![p] });
                    self.trace = true;
                }
                false
            }
            Geste::SaisieDeplace { x, y, .. } => {
                if self.gomme && modifiable {
                    self.gommer(x, y);
                } else if self.trace {
                    let p = self.vers_dessin(x, y);
                    let pas = 1.0 / self.vue.zoom;
                    if let Some(t) = self.traits.last_mut() {
                        if t.points.last().is_none_or(|q| (q.0 - p.0).hypot(q.1 - p.1) >= pas) {
                            t.points.push(p);
                        }
                    }
                }
                false
            }
            Geste::SaisieFin { .. } => self.conclure(),
            Geste::SaisieAnnulee { .. } => {
                // Cinq doigts : le trait en cours disparaît, les traits gommés reviennent
                if std::mem::take(&mut self.trace) {
                    self.traits.pop();
                }
                for (i, t) in std::mem::take(&mut self.gommes).into_iter().rev() {
                    self.traits.insert(i.min(self.traits.len()), t);
                }
                false
            }
            Geste::PincementDebut { .. } => {
                self.pincement = 1.0;
                false
            }
            Geste::Pincement { echelle, centre, deplacement } => {
                // Glisser : la feuille suit les doigts ; pincer : le point sous leur centre reste en place
                let k = self.vue.zoom * self.dp;
                self.vue.x -= deplacement.0 / k;
                self.vue.y -= deplacement.1 / k;
                let sous = self.vers_dessin(centre.0, centre.1);
                let f = echelle / self.pincement.max(1e-3);
                self.pincement = echelle;
                self.vue.zoom = (self.vue.zoom * f).clamp(ZOOM.0, ZOOM.1);
                let k = self.vue.zoom * self.dp;
                self.vue.x = sous.0 - (centre.0 - self.zone.x) / k;
                self.vue.y = sous.1 - (centre.1 - self.zone.y) / k;
                false
            }
            _ => false,
        }
    }

    fn feuille(&mut self, cx: &mut Cx, z: Rect) {
        let d = cx.dp;
        cx.dl.rrect(z, 0.0, PAPIER);
        cx.clip(Some(z));
        // Une grille légère, tous les 40 unités, tant qu'elle ne devient pas trop serrée
        let k = self.vue.zoom * d;
        let mut pas = 40.0;
        while pas * k < 16.0 * d {
            pas *= 5.0;
        }
        let grille = [0x1E, 0x1E, 0x25, 18];
        let mut gx = (self.vue.x / pas).ceil() * pas;
        while self.vers_ecran((gx, 0.0))[0] < z.right() {
            cx.dl.rrect(Rect::new(self.vers_ecran((gx, 0.0))[0], z.y, 1.0, z.h), 0.0, grille);
            gx += pas;
        }
        let mut gy = (self.vue.y / pas).ceil() * pas;
        while self.vers_ecran((0.0, gy))[1] < z.bottom() {
            cx.dl.rrect(Rect::new(z.x, self.vers_ecran((0.0, gy))[1], z.w, 1.0), 0.0, grille);
            gy += pas;
        }
        for t in &self.traits {
            let e = (t.largeur * k).max(1.0);
            // Hors de la vue, le trait ne se dessine pas
            let marge = Rect::new(z.x - e, z.y - e, z.w + 2.0 * e, z.h + 2.0 * e);
            if let [p] = t.points[..] {
                let [x, y] = self.vers_ecran(p);
                if marge.contains(x, y) {
                    cx.dl.rrect(Rect::new(x - e / 2.0, y - e / 2.0, e, e), e / 2.0, t.couleur);
                }
                continue;
            }
            for w in t.points.windows(2) {
                let (a, b) = (self.vers_ecran(w[0]), self.vers_ecran(w[1]));
                if marge.contains(a[0], a[1]) || marge.contains(b[0], b[1]) {
                    cx.dl.line(a, b, e, t.couleur);
                }
            }
        }
        cx.clip(None);
    }

    /// La barre du bas : les épaisseurs, les couleurs, la gomme, le zoom.
    /// Renvoie vrai si le dessin a changé (annuler, rétablir).
    fn barre(&mut self, cx: &mut Cx, r: Rect, base: Id) -> bool {
        let (d, col) = (cx.dp, cx.couleurs());
        cx.dl.rrect(r, 0.0, col.surface);
        cx.dl.rrect(Rect::new(r.x, r.y, r.w, d.max(1.0)), 0.0, col.ligne);
        cx.clip(Some(r));
        // Étroite, la barre resserre ses cases (le zoom et les outils restent touchables)
        let largeur = r.h * (TAILLES.len() as f32 + COULEURS.len() as f32 * 0.8 + 4.6) + 4.0 * cx.ecart() + cx.measure("1200 %", 13.0) + 2.0 * cx.pad();
        let h = r.h * (r.w / largeur).clamp(0.6, 1.0);
        let mut x = r.x + cx.ecart();
        let mut change = false;
        let contour = [col.pale[0], col.pale[1], col.pale[2], 140];
        let case = |x: f32| Rect::new(x, r.y + (r.h - h) / 2.0, h, h);
        let choisi = |cx: &mut Cx, c: Rect, oui: bool| {
            if oui {
                cx.dl.rrect(c.inset(4.0 * d), cx.theme.rayon(c.h - 8.0 * d), col.puce);
            }
        };
        for (i, t) in TAILLES.iter().enumerate() {
            let c = case(x);
            if cx.ui.click(sub_id(base, 10 + i), c, None) {
                self.taille = i;
                self.gomme = false;
            }
            choisi(cx, c, i == self.taille && !self.gomme);
            // L'encre sur un rond de papier : visible quel que soit le thème
            let fond = h * 0.3;
            cx.dl.rrect(Rect::new(c.cx() - fond, c.cy() - fond, 2.0 * fond, 2.0 * fond), fond, PAPIER);
            cx.dl.secteur([c.cx(), c.cy()], fond, fond + d, 0.0, std::f32::consts::TAU, 0.0, contour);
            let e = (t * 1.2).clamp(3.0, 2.0 * fond / d - 4.0) * d;
            cx.dl.rrect(Rect::new(c.cx() - e / 2.0, c.cy() - e / 2.0, e, e), e / 2.0, if self.gomme { col.pale } else { COULEURS[self.couleur] });
            x += h;
        }
        x += cx.ecart();
        for (i, couleur) in COULEURS.iter().enumerate() {
            let c = case(x);
            if cx.ui.click(sub_id(base, 20 + i), c, None) {
                self.couleur = i;
                self.gomme = false;
            }
            let rayon = h * 0.28;
            if i == self.couleur && !self.gomme {
                cx.dl.secteur([c.cx(), c.cy()], rayon + 3.0 * d, rayon + 5.0 * d, 0.0, std::f32::consts::TAU, 0.0, col.texte);
            }
            cx.dl.rrect(Rect::new(c.cx() - rayon, c.cy() - rayon, 2.0 * rayon, 2.0 * rayon), rayon, *couleur);
            // Un liseré : le noir sur fond sombre, le blanc sur fond clair
            cx.dl.secteur([c.cx(), c.cy()], rayon - d, rayon, 0.0, std::f32::consts::TAU, 0.0, contour);
            x += h * 0.8;
        }
        x += h * 0.2 + cx.ecart();
        let c = case(x);
        if cx.ui.click(sub_id(base, 30), c, None) {
            self.gomme = !self.gomme;
        }
        if self.gomme {
            cx.dl.rrect(c.inset(4.0 * d), cx.theme.rayon(c.h - 8.0 * d), col.accent);
        }
        cx.icon(c.inset(cx.pad() * 0.6), Icon::Gomme, if self.gomme { col.sur_accent } else { col.texte });
        x += h + cx.ecart();
        // Annuler, rétablir
        let ecrit = !self.lecture_seule;
        for (k, (icone, retablir)) in [(Icon::Annuler, false), (Icon::Retablir, true)].into_iter().enumerate() {
            let c = case(x);
            let possible = ecrit && !if retablir { &self.refaire } else { &self.annuler }.is_empty();
            if cx.ui.click(sub_id(base, 32 + k), c, None) && possible {
                change |= self.defaire(retablir);
            }
            if possible && cx.ui.pressed(sub_id(base, 32 + k)) {
                cx.dl.rrect(c.inset(4.0 * d), cx.theme.rayon(c.h - 8.0 * d), col.puce);
            }
            cx.icon(c.inset(cx.pad() * 0.6), icone, if possible { col.texte } else { contour });
            x += h;
        }
        let x = x - h;
        // Le zoom, à droite : le toucher ramène à 100 %
        let texte = format!("{:.0} %", self.vue.zoom * 100.0);
        let w = cx.measure(&texte, 13.0) + 2.0 * cx.pad();
        let zr = Rect::new((r.right() - w - cx.ecart()).max(x + h), r.y, w, h);
        if cx.ui.click(sub_id(base, 31), zr, None) {
            let centre = self.vers_dessin(self.zone.cx(), self.zone.cy());
            self.vue.zoom = 1.0;
            self.vue.x = centre.0 - self.zone.w / 2.0 / self.dp;
            self.vue.y = centre.1 - self.zone.h / 2.0 / self.dp;
        }
        cx.centered(zr, &texte, 13.0, col.pale);
        cx.clip(None);
        change
    }
}

impl Contexte for Encre {
    fn cle(&self) -> &'static str {
        CLE
    }

    /// Un nouveau dessin, avec les mêmes outils.
    fn dupliquer(&self) -> Box<dyn Contexte> {
        let mut e = Etat::default();
        self.outils(&mut e);
        ouvrir(&e)
    }

    fn document(&self) -> Option<&Path> {
        self.chemin.as_deref()
    }

    fn ecrire(&self, e: &mut Etat) {
        if let Some(c) = &self.chemin {
            e.mettre("document", c.display());
        }
        self.outils(e);
        e.mettre("vue", format!("{:.1} {:.1} {:.3}", self.vue.x, self.vue.y, self.vue.zoom));
    }

    fn retour(&mut self, _c: &Commun) {
        // Le fichier a pu changer entre-temps
        if self.charge && !self.trace {
            self.charger();
        }
    }

    fn dessiner(&mut self, cx: &mut Cx, c: &mut Commun, p: &Placee) -> String {
        if !self.charge {
            self.charger();
        }
        let r = p.contenu;
        let base = sub_id(ENCRE, p.id as usize);
        let hb = cx.commande().min(r.h / 3.0);
        let feuille = Rect::new(r.x, r.y, r.w, r.h - hb);
        self.zone = feuille;
        self.dp = cx.dp;
        let mut changer = false;
        for g in cx.ui.gestes(sub_id(base, 1), feuille) {
            changer |= self.recevoir(g);
        }
        if changer {
            self.enregistrer(c);
        }
        self.feuille(cx, feuille);
        if let Some(m) = &self.message {
            let (d, col) = (cx.dp, cx.couleurs());
            let w = (cx.measure(m, 13.0) + 24.0 * d).min(feuille.w - 16.0 * d);
            let b = Rect::new(feuille.right() - w - 8.0 * d, feuille.y + 8.0 * d, w, 28.0 * d);
            cx.dl.rrect(b, 14.0 * d, col.surface);
            cx.clip(Some(b));
            cx.centered(b, m, 13.0, col.pale);
            cx.clip(None);
        }
        if self.barre(cx, Rect::new(r.x, r.bottom() - hb, r.w, hb), base) {
            self.enregistrer(c);
        }
        let nom = self.chemin.as_ref().and_then(|c| c.file_name()).map_or("nouveau dessin".into(), |n| n.to_string_lossy().into_owned());
        let n = self.traits.len();
        format!("{nom} · {n} trait{}{}", if n > 1 { "s" } else { "" }, if self.lecture_seule { " · lecture seule" } else { "" })
    }
}

impl Encre {
    fn outils(&self, e: &mut Etat) {
        e.mettre("taille", self.taille);
        e.mettre("couleur", hex(COULEURS[self.couleur]));
        e.mettre_oui("gomme", self.gomme);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encre() -> Encre {
        let mut e = lire(&Etat::default());
        e.charge = true;
        e.zone = Rect::new(100.0, 100.0, 800.0, 600.0);
        e.dp = 2.0;
        e
    }

    #[test]
    fn le_svg_se_relit_a_l_identique() {
        let traits = vec![
            Trait { couleur: COULEURS[2], largeur: 4.0, points: vec![(0.0, 0.0), (10.5, 20.0), (30.0, -4.5)] },
            Trait { couleur: COULEURS[5], largeur: 16.0, points: vec![(50.0, 50.0)] },
        ];
        let (lu, tout) = depuis_svg(&vers_svg(&traits));
        assert!(tout);
        assert_eq!(lu, traits);
    }

    #[test]
    fn un_svg_etranger_reste_en_lecture_seule() {
        let (lu, tout) = depuis_svg("<svg><rect width=\"3\"/><path d=\"M0 0 L5 5\" stroke=\"#f00\"/></svg>");
        assert!(!tout);
        assert_eq!(lu.len(), 1);
        assert!(!depuis_svg("<svg><path d=\"M0 0 C1 1 2 2 3 3\" stroke=\"#000\"/></svg>").1);
        assert!(!depuis_svg("du texte").1);
    }

    #[test]
    fn tracer_puis_gommer() {
        let mut e = encre();
        e.recevoir(Geste::SaisieDebut { x: 100.0, y: 100.0, doigt: (0.0, 0.0) });
        e.recevoir(Geste::SaisieDeplace { x: 300.0, y: 100.0, doigt: (0.0, 0.0) });
        assert!(e.recevoir(Geste::SaisieFin { x: 300.0, y: 100.0, depart: (0.0, 0.0), vecteur: (0.0, 0.0) }));
        // À 100 % et 2 px par dp : de (0, 0) à (100, 0) dans le dessin
        assert_eq!(e.traits[0].points, vec![(0.0, 0.0), (100.0, 0.0)]);
        e.gomme = true;
        assert!(!e.recevoir(Geste::Tap { x: 200.0, y: 500.0, intensite: 0.0, precision: 0.0 }));
        assert!(e.recevoir(Geste::Tap { x: 200.0, y: 110.0, intensite: 0.0, precision: 0.0 }));
        assert!(e.traits.is_empty());
    }

    #[test]
    fn annuler_et_retablir() {
        let mut e = encre();
        let point = |x: f32| Geste::Tap { x, y: 150.0, intensite: 0.0, precision: 0.0 };
        for x in [150.0, 250.0, 350.0] {
            assert!(e.recevoir(point(x)));
        }
        let trois = e.traits.clone();
        // Gommer celui du milieu, puis annuler : il revient à sa place
        e.gomme = true;
        assert!(e.recevoir(point(250.0)));
        assert_eq!(e.traits.len(), 2);
        assert!(e.defaire(false));
        assert_eq!(e.traits, trois);
        assert!(e.defaire(true));
        assert_eq!(e.traits.len(), 2);
        // Annuler deux fois : le gommage, puis le dernier point
        e.defaire(false);
        e.defaire(false);
        assert_eq!(e.traits[..], trois[..2]);
        // Une nouvelle action efface ce qu'on pouvait rétablir
        e.gomme = false;
        e.recevoir(point(500.0));
        assert!(!e.defaire(true));
        while e.defaire(false) {}
        assert!(e.traits.is_empty());
    }

    #[test]
    fn cinq_doigts_rendent_ce_que_la_gomme_a_pris() {
        let mut e = encre();
        e.recevoir(Geste::Tap { x: 150.0, y: 150.0, intensite: 0.0, precision: 0.0 });
        e.gomme = true;
        e.recevoir(Geste::SaisieDebut { x: 150.0, y: 150.0, doigt: (0.0, 0.0) });
        assert!(e.traits.is_empty());
        assert!(!e.recevoir(Geste::SaisieAnnulee { x: 0.0, y: 0.0 }));
        assert_eq!(e.traits.len(), 1);
        assert_eq!(e.annuler.len(), 1);
    }

    #[test]
    fn pincer_garde_le_point_sous_les_doigts() {
        let mut e = encre();
        let centre = (500.0, 400.0);
        let avant = e.vers_dessin(centre.0, centre.1);
        e.recevoir(Geste::PincementDebut { centre });
        e.recevoir(Geste::Pincement { echelle: 2.0, centre, deplacement: (0.0, 0.0) });
        assert_eq!(e.vue.zoom, 2.0);
        let apres = e.vers_dessin(centre.0, centre.1);
        assert!((avant.0 - apres.0).abs() < 1e-3 && (avant.1 - apres.1).abs() < 1e-3);
        // Glisser les deux doigts déplace la feuille avec eux
        e.recevoir(Geste::Pincement { echelle: 2.0, centre: (540.0, 400.0), deplacement: (40.0, 0.0) });
        let p = e.vers_dessin(540.0, 400.0);
        assert!((p.0 - avant.0).abs() < 1e-3);
    }

    #[test]
    fn un_dessin_s_enregistre_dans_documents_encre() {
        let d = std::env::temp_dir().join(format!("flow-encre-{}", std::process::id()));
        let c = Commun { racine: Some(d.clone()), ..Commun::default() };
        let mut e = encre();
        e.recevoir(Geste::Tap { x: 150.0, y: 150.0, intensite: 0.0, precision: 0.0 });
        e.enregistrer(&c);
        let f = d.join("Documents/Encre/dessin-1.svg");
        assert_eq!(e.chemin.as_deref(), Some(f.as_path()));
        let mut et = Etat::default();
        e.ecrire(&mut et);
        let mut e2 = lire(&et);
        e2.charger();
        assert_eq!(e2.traits, e.traits);
        assert!(!e2.lecture_seule);
        let _ = std::fs::remove_dir_all(d);
    }
}
