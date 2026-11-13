//! La carte d'un lieu réel (`sage_reel::Lieu`), pour le contexte Lieux : des tuiles de
//! 256 pixels, en pyramide (au niveau `z`, 2^z tuiles de côté sur toute la carte),
//! dessinées par deux threads à la demande et gardées tant qu'elles servent.
//!
//! Une tuile montre le relief ombré (lumière du nord-ouest), l'eau, l'occupation du sol,
//! les routes et les ponts (bordés), les bâtiments. Ce qui est choisi ou édité se
//! dessine par-dessus, en vecteurs, par le contexte.
//!
//! Les données sont partagées (`Arc<RwLock<Donnees>>`) : le contexte les modifie sous
//! le verrou d'écriture et signale la zone retouchée (`retoucher`) ; les tuiles qui la
//! touchent sont redessinées, l'ancienne image restant affichée en attendant.

use crate::draw::{Image, Rect};
use crate::widgets::Cx;
use sage_reel::{origine, Batiment, Lieu, Route, BOIS, CHAMP, CHEMIN, GRANDE, HABITE, PIETONNE, PRE, ROUTE, SENTIER, SERVICE, ZONE};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, RwLock};

/// Le côté d'une tuile, en pixels.
pub const TUILE: usize = 256;
/// Le niveau le plus fin : 2^8 tuiles de côté, une vingtaine de centimètres par pixel.
pub const NIVEAU_MAX: u8 = 8;
/// Les tuiles gardées au plus.
const GARDEES: usize = 360;
/// Le côté des cellules de l'index des bâtiments et des routes, en mètres.
const CELLULE: f32 = 128.0;

/// Une tuile : (niveau, colonne, ligne).
pub type Cle = (u8, i32, i32);

/// Le lieu et ses index : les bâtiments, les routes et les ponts rangés par cellule.
pub struct Donnees {
    pub lieu: Lieu,
    bati: HashMap<(i32, i32), Vec<u32>>,
    routes: HashMap<(i32, i32), Vec<u32>>,
    ponts: HashMap<(i32, i32), Vec<u32>>,
}

/// La boîte (x0, z0, x1, z1) d'une suite de points.
pub fn boite<'a>(pts: impl Iterator<Item = &'a [f32]>) -> [f32; 4] {
    let mut b = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    for p in pts {
        b = [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])];
    }
    b
}

pub fn boite_bati(b: &Batiment) -> [f32; 4] {
    boite(b.contour.iter().map(|p| &p[..]))
}

pub fn boite_route(r: &Route) -> [f32; 4] {
    let b = boite(r.points.iter().map(|p| &p[..]));
    let m = r.largeur / 2.0 + 2.0;
    [b[0] - m, b[1] - m, b[2] + m, b[3] + m]
}

fn cellules(b: [f32; 4]) -> impl Iterator<Item = (i32, i32)> {
    let (i0, i1) = ((b[0] / CELLULE).floor() as i32, (b[2] / CELLULE).floor() as i32);
    let (j0, j1) = ((b[1] / CELLULE).floor() as i32, (b[3] / CELLULE).floor() as i32);
    (j0..=j1).flat_map(move |j| (i0..=i1).map(move |i| (i, j)))
}

/// Les éléments de l'index qui approchent la boîte, sans doublon, dans l'ordre.
fn dans(index: &HashMap<(i32, i32), Vec<u32>>, b: [f32; 4], tous: usize) -> Vec<u32> {
    // Une grande boîte : tout, plutôt que des milliers de cellules
    let n = ((b[2] - b[0]) / CELLULE + 1.0) * ((b[3] - b[1]) / CELLULE + 1.0);
    if n > 4096.0 {
        return (0..tous as u32).collect();
    }
    let mut v: Vec<u32> = cellules(b).flat_map(|c| index.get(&c).map_or(&[][..], |l| l.as_slice())).copied().collect();
    v.sort_unstable();
    v.dedup();
    v
}

impl Donnees {
    pub fn new(lieu: Lieu) -> Donnees {
        let mut d = Donnees { lieu, bati: HashMap::new(), routes: HashMap::new(), ponts: HashMap::new() };
        d.reindexer();
        d
    }

    /// Refait les index (après un changement des bâtiments, des routes ou des ponts).
    pub fn reindexer(&mut self) {
        let mut bati: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        for (k, b) in self.lieu.batiments.iter().enumerate() {
            for c in cellules(boite_bati(b)) {
                bati.entry(c).or_default().push(k as u32);
            }
        }
        let ranger = |l: &[Route]| {
            let mut m: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
            for (k, r) in l.iter().enumerate() {
                for c in cellules(boite_route(r)) {
                    m.entry(c).or_default().push(k as u32);
                }
            }
            m
        };
        self.routes = ranger(&self.lieu.routes);
        self.ponts = ranger(&self.lieu.ponts);
        self.bati = bati;
    }

    /// Le carré de la pyramide des tuiles : son coin (x, z) et son côté, en mètres. Il
    /// part du coin du lieu et couvre son plus grand côté (un lieu peut être rectangulaire).
    pub fn monde(&self) -> (f32, f32, f32) {
        let [x0, z0, x1, z1] = self.lieu.etendue();
        (x0 as f32, z0 as f32, (x1 - x0).max(z1 - z0) as f32)
    }

    pub fn batiments_dans(&self, b: [f32; 4]) -> Vec<u32> {
        dans(&self.bati, b, self.lieu.batiments.len())
    }

    pub fn routes_dans(&self, b: [f32; 4], ponts: bool) -> Vec<u32> {
        if ponts {
            dans(&self.ponts, b, self.lieu.ponts.len())
        } else {
            dans(&self.routes, b, self.lieu.routes.len())
        }
    }
}

/// Les couleurs de la carte (de 0 à 1).
pub fn couleur_sol(classe: u8) -> [f32; 3] {
    match classe {
        BOIS => [0.62, 0.76, 0.52],
        PRE => [0.80, 0.89, 0.66],
        CHAMP => [0.93, 0.90, 0.70],
        HABITE => [0.90, 0.87, 0.85],
        ZONE => [0.86, 0.84, 0.89],
        _ => [0.86, 0.90, 0.76],
    }
}

pub const EAU: [f32; 3] = [0.62, 0.78, 0.92];

/// L'intérieur et le bord d'une voie de ce genre ; les ponts ont un bord sombre.
pub fn couleur_route(genre: u8, pont: bool) -> ([f32; 3], [f32; 3]) {
    let int = match genre {
        GRANDE => [0.98, 0.82, 0.50],
        ROUTE | SERVICE => [1.0, 1.0, 1.0],
        PIETONNE => [0.95, 0.93, 0.88],
        CHEMIN => [0.86, 0.74, 0.56],
        _ => [0.80, 0.64, 0.46],
    };
    let bord = if pont { [0.25, 0.25, 0.28] } else if genre == GRANDE { [0.78, 0.55, 0.25] } else { [0.62, 0.60, 0.58] };
    (int, bord)
}

pub fn couleur_bati(genre: u8) -> [f32; 3] {
    match genre {
        sage_reel::MAISON => [0.82, 0.70, 0.64],
        sage_reel::IMMEUBLE => [0.76, 0.66, 0.66],
        sage_reel::INDUSTRIEL => [0.74, 0.74, 0.78],
        sage_reel::EDIFICE => [0.80, 0.66, 0.52],
        _ => [0.78, 0.74, 0.68],
    }
}

/// Mélange une couleur dans le pixel `i` (RVBA, opaque).
fn melanger(px: &mut [u8], i: usize, c: [f32; 3], a: f32) {
    if a <= 0.0 {
        return;
    }
    let a = a.min(1.0);
    for k in 0..3 {
        let v = px[i * 4 + k] as f32 / 255.0;
        px[i * 4 + k] = ((v + (c[k] - v) * a) * 255.0).round().clamp(0.0, 255.0) as u8;
    }
}

/// La hauteur bilinéaire du relief en (x, z) (pour les vues lointaines).
fn bilineaire(l: &Lieu, x: f64, z: f64) -> f64 {
    let (gx, gz) = ((x - origine()) / l.pas, (z - origine()) / l.pas);
    let (a, b) = (gx.floor(), gz.floor());
    let (u, v) = (gx - a, gz - b);
    let (a, b) = (a as i64, b as i64);
    let p = |a: i64, b: i64| l.h_point(a, b) as f64;
    (p(a, b) * (1.0 - u) + p(a + 1, b) * u) * (1.0 - v) + (p(a, b + 1) * (1.0 - u) + p(a + 1, b + 1) * u) * v
}

/// La boîte d'une tuile, en mètres.
pub fn boite_tuile(d: &Donnees, (z, i, j): Cle) -> [f32; 4] {
    let (x0, z0, c) = d.monde();
    let t = c / (1u32 << z) as f32;
    [x0 + i as f32 * t, z0 + j as f32 * t, x0 + (i + 1) as f32 * t, z0 + (j + 1) as f32 * t]
}

/// Dessine une tuile : RVBA, `TUILE` × `TUILE`.
pub fn rendre(d: &Donnees, cle: Cle) -> Vec<u8> {
    let l = &d.lieu;
    let b = boite_tuile(d, cle);
    let m = (b[2] - b[0]) / TUILE as f32; // mètres par pixel
    let n = TUILE;
    let mut px = vec![255u8; n * n * 4];
    // Le relief, aux centres des pixels et sur un pixel de bord, pour l'ombrage
    let fin = (m as f64) < l.pas * 1.5;
    let mut h = vec![0f32; (n + 2) * (n + 2)];
    for y in 0..n + 2 {
        let z = b[1] as f64 + (y as f64 - 0.5) * m as f64;
        for x in 0..n + 2 {
            let xx = b[0] as f64 + (x as f64 - 0.5) * m as f64;
            h[y * (n + 2) + x] = if fin { l.relief(xx, z) } else { bilineaire(l, xx, z) } as f32;
        }
    }
    // La lumière, du nord-ouest et d'en haut ; le relief un peu exagéré au loin
    let exag = (1.0 + (m / 8.0).ln().max(0.0) * 0.6).min(3.0);
    let lum = {
        let v = [-1.0f32, 1.4, -1.0];
        let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        [v[0] / n, v[1] / n, v[2] / n]
    };
    for y in 0..n {
        let z = b[1] as f64 + (y as f64 + 0.5) * m as f64;
        for x in 0..n {
            let xx = b[0] as f64 + (x as f64 + 0.5) * m as f64;
            let i = y * n + x;
            if l.eau(xx, z) >= 0.5 {
                // Plus sombre au fond
                let p = ((l.niveau - h[(y + 1) * (n + 2) + x + 1]) / 8.0).clamp(0.0, 1.0);
                let c = [EAU[0] - 0.12 * p, EAU[1] - 0.10 * p, EAU[2] - 0.04 * p];
                for k in 0..3 {
                    px[i * 4 + k] = (c[k] * 255.0) as u8;
                }
                continue;
            }
            let hc = |dx: i32, dy: i32| h[((y as i32 + 1 + dy) as usize) * (n + 2) + (x as i32 + 1 + dx) as usize];
            // De près, la pente se prend sur deux pas de la grille : le relief de l'IGN,
            // rééchantillonné, garde des marches d'un point à l'autre
            let (gx, gz) = if fin {
                let e = 1.25 * l.pas;
                let r = |dx: f64, dz: f64| l.relief(xx + dx, z + dz) as f32;
                ((r(e, 0.0) - r(-e, 0.0)) / (2.0 * e as f32), (r(0.0, e) - r(0.0, -e)) / (2.0 * e as f32))
            } else {
                ((hc(1, 0) - hc(-1, 0)) / (2.0 * m), (hc(0, 1) - hc(0, -1)) / (2.0 * m))
            };
            let (gx, gz) = (gx * exag, gz * exag);
            let nn = (gx * gx + 1.0 + gz * gz).sqrt();
            let s = ((-gx * lum[0] + lum[1] - gz * lum[2]) / nn / lum[1]).clamp(0.0, 1.35);
            // Les classes des quatre points voisins, mêlées : pas d'escalier aux bords
            let c = if m < (l.pas * 2.0) as f32 {
                l.occupation(xx, z).iter().fold([0.0f32; 3], |a, &(k, w)| {
                    let c = couleur_sol(k);
                    [a[0] + c[0] * w as f32, a[1] + c[1] * w as f32, a[2] + c[2] * w as f32]
                })
            } else {
                couleur_sol(l.classe(xx, z))
            };
            let f = 0.42 + 0.58 * s;
            for k in 0..3 {
                px[i * 4 + k] = (c[k] * f * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    // En pixels de la tuile
    let vers = |p: [f32; 2]| ((p[0] - b[0]) / m, (p[1] - b[1]) / m);
    // Les voies : d'abord tous les bords, puis les intérieurs ; les ponts par-dessus
    let marge = [b[0] - 40.0, b[1] - 40.0, b[2] + 40.0, b[3] + 40.0];
    for pont in [false, true] {
        let liste = if pont { &l.ponts } else { &l.routes };
        let choisies = d.routes_dans(marge, pont);
        for passe in 0..2 {
            for &k in &choisies {
                let r = &liste[k as usize];
                let large = r.largeur / m;
                // Au loin, seules les grandes voies se voient
                let min = match r.genre {
                    GRANDE => 1.4,
                    ROUTE => 0.9,
                    SERVICE | PIETONNE if m < 6.0 => 0.7,
                    CHEMIN | SENTIER if m < 4.0 => 0.6,
                    _ if pont => 1.2,
                    _ => continue,
                };
                let demi = (large / 2.0).max(min / 2.0);
                let bord = if large > 3.0 { 1.0 } else { 0.6 };
                let (int, cb) = couleur_route(r.genre, pont);
                for s in r.points.windows(2) {
                    let (a, c) = (vers([s[0][0], s[0][1]]), vers([s[1][0], s[1][1]]));
                    let e = demi + bord + 1.0;
                    let (x0, x1) = ((a.0.min(c.0) - e).floor().max(0.0) as i32, (a.0.max(c.0) + e).ceil().min(n as f32 - 1.0) as i32);
                    let (y0, y1) = ((a.1.min(c.1) - e).floor().max(0.0) as i32, (a.1.max(c.1) + e).ceil().min(n as f32 - 1.0) as i32);
                    let (dx, dy) = (c.0 - a.0, c.1 - a.1);
                    let l2 = dx * dx + dy * dy;
                    for y in y0..=y1 {
                        for x in x0..=x1 {
                            let (qx, qy) = (x as f32 + 0.5, y as f32 + 0.5);
                            let t = if l2 > 0.0 { (((qx - a.0) * dx + (qy - a.1) * dy) / l2).clamp(0.0, 1.0) } else { 0.0 };
                            let dd = (qx - a.0 - dx * t).hypot(qy - a.1 - dy * t);
                            let i = y as usize * n + x as usize;
                            if passe == 0 {
                                melanger(&mut px, i, cb, demi + bord + 0.5 - dd);
                            } else {
                                melanger(&mut px, i, int, demi + 0.5 - dd);
                            }
                        }
                    }
                }
            }
        }
    }
    // Les bâtiments : remplis par lignes, bordés de près
    for k in d.batiments_dans(b) {
        let bt = &l.batiments[k as usize];
        let c = couleur_bati(bt.genre);
        let pts: Vec<(f32, f32)> = bt.contour.iter().map(|p| vers(*p)).collect();
        let bb = boite(pts.iter().map(|p| [p.0, p.1]).collect::<Vec<_>>().iter().map(|p| &p[..]));
        if bb[2] < 0.0 || bb[3] < 0.0 || bb[0] > n as f32 || bb[1] > n as f32 {
            continue;
        }
        // Plus petit qu'un pixel : un point à sa mesure
        if bb[2] - bb[0] < 1.0 && bb[3] - bb[1] < 1.0 {
            let (x, y) = (((bb[0] + bb[2]) / 2.0) as i32, ((bb[1] + bb[3]) / 2.0) as i32);
            if (0..n as i32).contains(&x) && (0..n as i32).contains(&y) {
                let a = ((bb[2] - bb[0]) * (bb[3] - bb[1])).sqrt().max(0.25);
                melanger(&mut px, y as usize * n + x as usize, [c[0] * 0.8, c[1] * 0.8, c[2] * 0.8], a);
            }
            continue;
        }
        let (y0, y1) = (bb[1].floor().max(0.0) as i32, bb[3].ceil().min(n as f32 - 1.0) as i32);
        let mut xs = Vec::new();
        for y in y0..=y1 {
            let yc = y as f32 + 0.5;
            xs.clear();
            for e in 0..pts.len() {
                let (p, q) = (pts[e], pts[(e + 1) % pts.len()]);
                if (p.1 <= yc) != (q.1 <= yc) {
                    xs.push(p.0 + (yc - p.1) / (q.1 - p.1) * (q.0 - p.0));
                }
            }
            xs.sort_by(|a, b| a.total_cmp(b));
            for s in xs.chunks(2) {
                if s.len() < 2 {
                    continue;
                }
                let (xa, xb) = (s[0].max(0.0), s[1].min(n as f32));
                let (ia, ib) = (xa.floor() as i32, (xb.ceil() as i32 - 1).min(n as i32 - 1));
                for x in ia.max(0)..=ib {
                    // Couverture horizontale aux bouts
                    let cov = ((x as f32 + 1.0).min(xb) - (x as f32).max(xa)).clamp(0.0, 1.0);
                    melanger(&mut px, y as usize * n + x as usize, c, cov);
                }
            }
        }
        // Le bord, quand il se voit
        if bb[2] - bb[0] > 5.0 || bb[3] - bb[1] > 5.0 {
            let sombre = [c[0] * 0.62, c[1] * 0.58, c[2] * 0.58];
            for e in 0..pts.len() {
                let (p, q) = (pts[e], pts[(e + 1) % pts.len()]);
                let pas = ((q.0 - p.0).abs().max((q.1 - p.1).abs()) * 1.5).ceil().max(1.0) as i32;
                for s in 0..=pas {
                    let t = s as f32 / pas as f32;
                    let (x, y) = ((p.0 + (q.0 - p.0) * t) as i32, (p.1 + (q.1 - p.1) * t) as i32);
                    if (0..n as i32).contains(&x) && (0..n as i32).contains(&y) {
                        melanger(&mut px, y as usize * n + x as usize, sombre, 0.7);
                    }
                }
            }
        }
    }
    // Hors du lieu : transparent
    let e = l.etendue();
    for y in 0..n {
        let z = b[1] as f64 + (y as f64 + 0.5) * m as f64;
        for x in 0..n {
            let xx = b[0] as f64 + (x as f64 + 0.5) * m as f64;
            if xx < e[0] || xx > e[2] || z < e[1] || z > e[3] {
                px[(y * n + x) * 4..(y * n + x) * 4 + 4].fill(0);
            }
        }
    }
    px
}

/// La vue : le point du monde au centre de la zone, et les pixels par mètre.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vue {
    pub cx: f64,
    pub cz: f64,
    pub echelle: f32,
}

impl Vue {
    pub fn ecran(&self, zone: Rect, x: f32, z: f32) -> [f32; 2] {
        [zone.cx() + ((x as f64 - self.cx) as f32) * self.echelle, zone.cy() + ((z as f64 - self.cz) as f32) * self.echelle]
    }

    pub fn monde(&self, zone: Rect, sx: f32, sy: f32) -> (f32, f32) {
        ((self.cx + ((sx - zone.cx()) / self.echelle) as f64) as f32, (self.cz + ((sy - zone.cy()) / self.echelle) as f64) as f32)
    }

    /// Zoome de `k` autour du point d'écran (sx, sy), qui reste sous le doigt.
    pub fn zoomer(&mut self, zone: Rect, k: f32, sx: f32, sy: f32, min: f32, max: f32) {
        let (wx, wz) = self.monde(zone, sx, sy);
        self.echelle = (self.echelle * k).clamp(min, max);
        self.cx = wx as f64 - ((sx - zone.cx()) / self.echelle) as f64;
        self.cz = wz as f64 - ((sy - zone.cy()) / self.echelle) as f64;
    }

    pub fn deplacer(&mut self, dx: f32, dy: f32) {
        self.cx -= (dx / self.echelle) as f64;
        self.cz -= (dy / self.echelle) as f64;
    }
}

/// Le travail partagé avec les threads de dessin.
struct Partage {
    file: Mutex<(Vec<(Cle, u64)>, HashSet<Cle>)>,
    faites: Mutex<Vec<(Cle, u64, Vec<u8>)>>,
    signal: Condvar,
    fin: AtomicBool,
}

struct Tuile {
    image: Image,
    /// La retouche jusqu'à laquelle elle est à jour, et la dernière image où elle a servi.
    faite: u64,
    vue: u64,
}

static INSTANCES: AtomicU64 = AtomicU64::new(1);

/// Les tuiles d'un lieu, leurs threads et leurs retouches.
pub struct Carte {
    pub donnees: Arc<RwLock<Donnees>>,
    partage: Arc<Partage>,
    tuiles: HashMap<Cle, Tuile>,
    /// Le compteur des retouches, et les zones retouchées (les plus récentes).
    pub compteur: u64,
    retouches: Vec<(u64, [f32; 4])>,
    image: u64,
    num: u64,
}

impl Drop for Carte {
    fn drop(&mut self) {
        self.partage.fin.store(true, Ordering::Relaxed);
        self.partage.signal.notify_all();
    }
}

impl Carte {
    pub fn new(lieu: Lieu) -> Carte {
        let donnees = Arc::new(RwLock::new(Donnees::new(lieu)));
        let partage = Arc::new(Partage { file: Mutex::new((Vec::new(), HashSet::new())), faites: Mutex::new(Vec::new()), signal: Condvar::new(), fin: AtomicBool::new(false) });
        for _ in 0..2 {
            let (d, p) = (donnees.clone(), partage.clone());
            std::thread::spawn(move || loop {
                let travail = {
                    let Ok(mut f) = p.file.lock() else { return };
                    loop {
                        if p.fin.load(Ordering::Relaxed) {
                            return;
                        }
                        if let Some((cle, v)) = f.0.pop() {
                            f.1.insert(cle);
                            break (cle, v);
                        }
                        f = match p.signal.wait(f) {
                            Ok(f) => f,
                            Err(_) => return,
                        };
                    }
                };
                let pixels = match d.read() {
                    Ok(d) => rendre(&d, travail.0),
                    Err(_) => return,
                };
                if let Ok(mut s) = p.faites.lock() {
                    s.push((travail.0, travail.1, pixels));
                }
                if let Ok(mut f) = p.file.lock() {
                    f.1.remove(&travail.0);
                }
                crate::radio::reveiller();
            });
        }
        Carte { donnees, partage, tuiles: HashMap::new(), compteur: 1, retouches: Vec::new(), image: 0, num: INSTANCES.fetch_add(1, Ordering::Relaxed) }
    }

    /// La zone (en mètres) a changé : les tuiles qui la touchent sont à refaire.
    pub fn retoucher(&mut self, zone: [f32; 4]) {
        self.compteur += 1;
        self.retouches.push((self.compteur, zone));
        if self.retouches.len() > 4000 {
            self.retouches.drain(..2000);
        }
    }

    /// Tout est à refaire (un autre lieu, une annulation lointaine…).
    pub fn tout_retoucher(&mut self) {
        self.retoucher([f32::MIN, f32::MIN, f32::MAX, f32::MAX]);
    }

    /// Ce qui a été fait à la retouche `faite` dans la boîte `b` est-il encore à jour ?
    pub fn a_jour(&self, faite: u64, b: [f32; 4]) -> bool {
        if self.retouches.first().is_some_and(|r| r.0 > faite + 1) && faite < self.retouches[0].0 {
            // Plus vieille que ce qu'on retient
            return false;
        }
        !self.retouches.iter().rev().take_while(|r| r.0 > faite).any(|(_, z)| z[0] <= b[2] && z[2] >= b[0] && z[1] <= b[3] && z[3] >= b[1])
    }

    /// Des tuiles attendent d'être dessinées.
    pub fn en_attente(&self) -> bool {
        self.partage.file.lock().map_or(false, |f| !f.0.is_empty() || !f.1.is_empty()) || self.partage.faites.lock().map_or(false, |f| !f.is_empty())
    }

    /// Le niveau qui donne au moins un pixel de tuile par pixel d'écran.
    pub fn niveau(&self, echelle: f32) -> u8 {
        let c = self.donnees.read().map_or(20000.0, |d| d.monde().2);
        let z = (echelle * c / TUILE as f32).max(1.0).log2().ceil();
        (z as i32).clamp(0, NIVEAU_MAX as i32) as u8
    }

    /// Range les tuiles finies par les threads.
    fn ranger(&mut self) {
        let faites = self.partage.faites.lock().map(|mut f| std::mem::take(&mut *f)).unwrap_or_default();
        for (cle, v, pixels) in faites {
            let id = 0x6361_7274_0000_0000u64 ^ (self.num << 40) ^ ((cle.0 as u64) << 32) ^ ((cle.1 as u64 & 0xFFFF) << 16) ^ (cle.2 as u64 & 0xFFFF);
            let image = Image { id, revision: v, largeur: TUILE as u32, hauteur: TUILE as u32, pixels: Arc::new(pixels) };
            let vue_ = self.tuiles.get(&cle).map_or(self.image, |t| t.vue);
            if self.tuiles.get(&cle).is_none_or(|t| t.faite <= v) {
                self.tuiles.insert(cle, Tuile { image, faite: v, vue: vue_ });
            }
        }
    }

    /// L'image de la tuile `cle` (de boîte `bt`), ou d'un ancêtre et la partie (u0, v0,
    /// largeur, hauteur) qui la couvre ; demandée (dans `demandes`) si elle manque ou
    /// n'est plus à jour.
    fn image_de(&mut self, cle: Cle, bt: [f32; 4], demandes: &mut Vec<(Cle, u64)>) -> Option<(Image, [f32; 4])> {
        if !self.tuiles.get(&cle).is_some_and(|tu| self.a_jour(tu.faite, bt)) {
            demandes.push((cle, self.compteur));
        }
        let (z, i, j) = cle;
        for k in 0..=z {
            let (pi, pj) = (i >> k, j >> k);
            if let Some(tu) = self.tuiles.get_mut(&(z - k, pi, pj)) {
                tu.vue = self.image;
                let f = 1.0 / (1 << k) as f32;
                return Some((tu.image.clone(), [(i - (pi << k)) as f32 * f, (j - (pj << k)) as f32 * f, f, f]));
            }
        }
        None
    }

    /// Les demandes (la plus importante d'abord) remplacent les anciennes, celles qui ne
    /// se voient plus tombent ; les tuiles qui n'ont pas servi depuis longtemps
    /// s'oublient.
    fn envoyer(&mut self, mut demandes: Vec<(Cle, u64)>) {
        if let Ok(mut f) = self.partage.file.lock() {
            let en_cours = f.1.clone();
            demandes.retain(|(k, _)| !en_cours.contains(k));
            // La file se vide par la fin
            demandes.reverse();
            f.0 = demandes;
        }
        self.partage.signal.notify_all();
        if self.tuiles.len() > GARDEES {
            let mut v: Vec<(u64, Cle)> = self.tuiles.iter().filter(|(k, _)| k.0 > 0).map(|(k, t)| (t.vue, *k)).collect();
            v.sort();
            for (_, k) in v.iter().take(self.tuiles.len() - GARDEES) {
                self.tuiles.remove(k);
            }
        }
    }

    /// Pour la 3D : les images des tuiles `cles` (la plus importante d'abord), ou d'un
    /// ancêtre et la partie qui les couvre. La tuile du niveau 0 est toujours demandée.
    pub fn textures(&mut self, cles: &[Cle]) -> Vec<Option<(Image, [f32; 4])>> {
        self.image += 1;
        self.ranger();
        let boites: Vec<[f32; 4]> = match self.donnees.read() {
            Ok(d) => std::iter::once((0, 0, 0)).chain(cles.iter().cloned()).map(|c| boite_tuile(&d, c)).collect(),
            Err(_) => return vec![None; cles.len()],
        };
        let mut demandes = Vec::new();
        let mut v: Vec<_> = cles.iter().zip(&boites[1..]).map(|(&c, &b)| self.image_de(c, b, &mut demandes)).collect();
        // La racine en premier : de quoi montrer en attendant
        let mut d0 = Vec::new();
        self.image_de((0, 0, 0), boites[0], &mut d0);
        d0.extend(demandes);
        self.envoyer(d0);
        v.shrink_to_fit();
        v
    }

    /// Dessine la carte vue par `vue` dans `zone`.
    pub fn dessiner(&mut self, cx: &mut Cx, zone: Rect, vue: &Vue) {
        self.image += 1;
        self.ranger();
        let (x0, z0, c) = match self.donnees.read() {
            Ok(d) => d.monde(),
            Err(_) => return,
        };
        let z = self.niveau(vue.echelle);
        let nt = 1i32 << z;
        let t = c / nt as f32;
        let (a, b) = (vue.monde(zone, zone.x, zone.y), vue.monde(zone, zone.right(), zone.bottom()));
        let (i0, i1) = (((a.0 - x0) / t).floor().max(0.0) as i32, (((b.0 - x0) / t).floor() as i32).min(nt - 1));
        let (j0, j1) = (((a.1 - z0) / t).floor().max(0.0) as i32, (((b.1 - z0) / t).floor() as i32).min(nt - 1));
        let mut demandes = Vec::new();
        // La tuile du niveau 0 toujours, pour avoir de quoi montrer en attendant
        let racine = (0u8, 0, 0);
        if !self.tuiles.get(&racine).is_some_and(|tu| self.a_jour(tu.faite, [x0, z0, x0 + c, z0 + c])) {
            demandes.push((racine, self.compteur));
        }
        cx.clip(Some(zone));
        // Le fond hors de la carte
        cx.dl.rrect(zone, 0.0, [0xD8, 0xDC, 0xD4, 255]);
        // Le fond du lieu (son rectangle, pas le carré des tuiles), en attendant les tuiles
        let e = self.donnees.read().map_or([x0, z0, x0 + c, z0 + c], |d| { let e = d.lieu.etendue(); [e[0] as f32, e[1] as f32, e[2] as f32, e[3] as f32] });
        let (ca, cb) = (vue.ecran(zone, e[0], e[1]), vue.ecran(zone, e[2], e[3]));
        cx.dl.rrect(Rect::new(ca[0], ca[1], cb[0] - ca[0], cb[1] - ca[1]), 0.0, [0xC8, 0xD4, 0xB8, 255]);
        if i0 <= i1 && j0 <= j1 {
            // Du centre vers les bords : les premières demandées sont au milieu
            let mut cles: Vec<(i32, i32)> = (j0..=j1).flat_map(|j| (i0..=i1).map(move |i| (i, j))).collect();
            let (mi, mj) = ((i0 + i1) as f32 / 2.0, (j0 + j1) as f32 / 2.0);
            cles.sort_by(|p, q| ((q.0 as f32 - mi).powi(2) + (q.1 as f32 - mj).powi(2)).total_cmp(&((p.0 as f32 - mi).powi(2) + (p.1 as f32 - mj).powi(2))));
            for (i, j) in cles {
                let cle = (z, i, j);
                let bt = [x0 + i as f32 * t, z0 + j as f32 * t, x0 + (i + 1) as f32 * t, z0 + (j + 1) as f32 * t];
                let (p, q) = (vue.ecran(zone, bt[0], bt[1]), vue.ecran(zone, bt[2], bt[3]));
                // Un quart de pixel de plus : pas de joint entre les tuiles
                let r = Rect::new(p[0].floor(), p[1].floor(), (q[0] - p[0].floor()).ceil() + 0.5, (q[1] - p[1].floor()).ceil() + 0.5);
                let a_jour = self.tuiles.get(&cle).is_some_and(|tu| self.a_jour(tu.faite, bt));
                if !a_jour {
                    demandes.push((cle, self.compteur));
                }
                if let Some(tu) = self.tuiles.get_mut(&cle) {
                    tu.vue = self.image;
                    cx.dl.image(r, tu.image.clone());
                    continue;
                }
                // En attendant : un ancêtre, agrandi
                let mut k = 1;
                while k <= z {
                    let (pi, pj) = (i >> k, j >> k);
                    if let Some(tu) = self.tuiles.get_mut(&(z - k, pi, pj)) {
                        tu.vue = self.image;
                        let f = 1.0 / (1 << k) as f32;
                        let (u, v) = ((i - (pi << k)) as f32 * f, (j - (pj << k)) as f32 * f);
                        cx.dl.image_partie(r, tu.image.clone(), [u, v], [u + f, v + f]);
                        break;
                    }
                    k += 1;
                }
            }
        }
        cx.clip(None);
        self.envoyer(demandes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sage_reel::{cote, cote_sol, MAISON};

    fn lieu() -> Lieu {
        let pas = 64.0;
        let n = cote(pas);
        let h: Vec<f32> = (0..n * n).map(|i| ((i % n) as f32 * 0.3).sin() * 20.0 + 50.0).collect();
        let eau: Vec<bool> = (0..n * n).map(|i| i % n < 20).collect();
        let b = Batiment::plat(MAISON, 49.0, 60.0, vec![[0.0, 0.0], [40.0, 0.0], [40.0, 30.0], [0.0, 30.0]]);
        let r = Route { genre: ROUTE, largeur: 8.0, points: vec![[-300.0, 50.0, 50.0], [300.0, 50.0, 50.0]] };
        let m = cote_sol(n);
        Lieu::new("Essai", 49.0, 1.0, 0.0, 0.0, pas, 40.0, &h, &eau, vec![r], vec![b], Vec::new()).avec_sol(vec![BOIS; m * m])
    }

    #[test]
    fn une_tuile_montre_l_eau_la_route_et_le_batiment() {
        let d = Donnees::new(lieu());
        let (x0, z0, c) = d.monde();
        assert!(x0 < 0.0 && z0 == x0 && c > 19_000.0);
        assert_eq!(d.batiments_dans([10.0, 10.0, 20.0, 20.0]), vec![0]);
        assert!(d.batiments_dans([1000.0, 1000.0, 1100.0, 1100.0]).is_empty());
        assert_eq!(d.routes_dans([0.0, 40.0, 10.0, 60.0], false), vec![0]);
        // Au niveau 8, la tuile qui contient (20, 15) : le bâtiment ; celle de (0, 50) : la route
        let tuile = |x: f32, z: f32| {
            let t = c / 256.0;
            (8u8, ((x - x0) / t).floor() as i32, ((z - x0) / t).floor() as i32)
        };
        let pixel = |cle: Cle, x: f32, z: f32| {
            let b = boite_tuile(&d, cle);
            let px = rendre(&d, cle);
            let m = (b[2] - b[0]) / TUILE as f32;
            let i = ((z - b[1]) / m) as usize * TUILE + ((x - b[0]) / m) as usize;
            [px[i * 4], px[i * 4 + 1], px[i * 4 + 2]]
        };
        let cb = couleur_bati(MAISON);
        let p = pixel(tuile(20.0, 15.0), 20.0, 15.0);
        assert!((p[0] as f32 - cb[0] * 255.0).abs() < 3.0, "{p:?}");
        let p = pixel(tuile(10.0, 50.0), 10.0, 50.0);
        assert_eq!(p, [255, 255, 255]);
        // L'eau, à l'ouest
        let xo = origine() as f32 + 64.0 * 5.0;
        let p = pixel(tuile(xo, 0.0), xo, 0.0);
        assert!(p[2] > p[0] && p[2] > 180, "{p:?}");
    }

    #[test]
    fn un_lieu_rectangulaire_a_ses_tuiles_transparentes_au_dela() {
        let l = Lieu::vierge_sur("R", 49.0, 1.0, 16.0, sage_reel::Grille::emprise(16.0, 3000.0, 1000.0), 100.0);
        let d = Donnees::new(l);
        let (x0, z0, c) = d.monde();
        let e = d.lieu.etendue();
        assert_eq!((x0, z0), (e[0] as f32, e[1] as f32));
        assert!((c - (e[2] - e[0]) as f32).abs() < 1.0);
        // La tuile du niveau 0 : le haut opaque (le lieu), le bas transparent
        let px = rendre(&d, (0, 0, 0));
        assert_eq!(px[(10 * TUILE + 10) * 4 + 3], 255);
        assert_eq!(px[((TUILE - 10) * TUILE + 10) * 4 + 3], 0);
    }

    #[test]
    fn la_vue_garde_le_point_sous_les_doigts() {
        let z = Rect::new(0.0, 0.0, 800.0, 600.0);
        let mut v = Vue { cx: 100.0, cz: -50.0, echelle: 0.5 };
        let avant = v.monde(z, 600.0, 100.0);
        v.zoomer(z, 3.0, 600.0, 100.0, 0.01, 50.0);
        let apres = v.monde(z, 600.0, 100.0);
        assert!((avant.0 - apres.0).abs() < 1e-3 && (avant.1 - apres.1).abs() < 1e-3);
        let e = v.ecran(z, 100.0, -50.0);
        let m = v.monde(z, e[0], e[1]);
        assert!((m.0 - 100.0).abs() < 1e-3 && (m.1 + 50.0).abs() < 1e-3);
    }
}
