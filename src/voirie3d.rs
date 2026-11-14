//! La voirie en volume, pour la 3D des lieux : le réseau des voies (hors ponts) nivelé,
//! le terrain terrassé le long de son tracé, puis chaque voie maillée en ruban et cousue
//! au relief.
//!
//! - **Le réseau** : les points des voies, réunis là où elles se touchent (OSM partage
//!   ses nœuds : un carrefour est un même point dans chaque voie). Un nœud à trois
//!   voisins ou plus est un **carrefour** ; les **branches** vont d'un carrefour ou d'un
//!   bout à l'autre, à travers les nœuds de passage (deux voies bout à bout n'en font
//!   qu'une).
//! - **Le nivellement** : la hauteur de chaque nœud (la moyenne des voies qui y passent)
//!   se lisse sur le réseau (`PASSES` moyennes avec les voisins : une pente régulière
//!   reste, les à-coups s'effacent), les rues d'abord, puis les voies piétonnes, chemins
//!   et sentiers, accrochés aux rues qu'ils rejoignent. Un bout de voie au pied d'un pont
//!   prend la hauteur de son tablier. Puis chaque carrefour se met à plat : les nœuds à
//!   moins de son rayon prennent sa hauteur, et les approches se relissent jusqu'à lui.
//! - **Les rubans** : chaque branche, la chaussée au milieu et la plateforme de chaque
//!   côté (`plateforme` : trottoir ou accotement), à plat en travers, à la hauteur du
//!   profil ; ses angles en onglet. Elle s'arrête au rayon des carrefours, que remplit une
//!   **place** : l'enveloppe des bouts des branches, à la hauteur du carrefour.
//! - **Le terrassement** (`sol`) : sous la plateforme, le sol à la hauteur de la voie ;
//!   au-delà, il rejoint le relief par un talus d'au plus `PENTE`, sur `PORTEE` mètres,
//!   fondu sur les `FONDU` derniers. Dans l'eau, pas de talus (la berge reste un quai).
//! - **La couture** (`sur`, `hauteur`, `monter`) : le relief se découpe exactement au bord
//!   des rubans et des places (le point de coupe cherché sur l'arête), ce qu'ils couvrent
//!   s'en va, et le bord du relief prend la hauteur de la voie : le ruban s'y pose bord à
//!   bord. Une petite jupe sous chaque bord cache les fentes qui resteraient aux angles.

use crate::relief3d::{BORD, CHAUSSEE, SOMMET};
use sage_reel::tabliers::Tablier;
use sage_reel::{carrossable, Lieu, Route};
use std::collections::HashMap as Table;
use std::hash::{BuildHasherDefault, Hasher};

/// Un hachage rapide (à la manière de FxHash) : les clés sont des entiers, pas besoin de
/// résister aux collisions voulues.
#[derive(Default, Clone, Copy)]
struct Rapide(u64);

impl Hasher for Rapide {
    fn write(&mut self, octets: &[u8]) {
        for &o in octets {
            self.write_u64(o as u64);
        }
    }
    fn write_u64(&mut self, x: u64) {
        self.0 = (self.0.rotate_left(5) ^ x).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
    fn write_u32(&mut self, x: u32) {
        self.write_u64(x as u64);
    }
    fn write_i32(&mut self, x: i32) {
        self.write_u64(x as u32 as u64);
    }
    fn write_i64(&mut self, x: i64) {
        self.write_u64(x as u64);
    }
    fn write_usize(&mut self, x: usize) {
        self.write_u64(x as u64);
    }
    fn finish(&self) -> u64 {
        self.0
    }
}

type HashMap<K, V> = Table<K, V, BuildHasherDefault<Rapide>>;
type HashSet<K> = std::collections::HashSet<K, BuildHasherDefault<Rapide>>;

/// La place à plat de chaque côté de la chaussée (m) : trottoir ou accotement.
pub fn plateforme(genre: u8) -> f32 {
    if carrossable(genre) {
        2.0
    } else {
        0.5
    }
}

/// La pente des talus : 2 de haut pour 3 de long.
pub const PENTE: f32 = 2.0 / 3.0;
/// Jusqu'où va un talus au-delà de la plateforme (m), et sur combien de ses derniers
/// mètres il se fond dans le relief.
pub const PORTEE: f32 = 20.0;
pub const FONDU: f32 = 5.0;
/// Les moyennes du lissage du profil (les points sont à 8 m au plus : environ 20 m).
pub const PASSES: usize = 10;
/// Les moyennes qui relissent les approches des carrefours.
pub const PASSES_APPROCHE: usize = 6;
/// La marge du rayon d'un carrefour au-delà de la plus large de ses branches (m).
pub const MARGE_CARREFOUR: f32 = 1.0;
/// La jupe sous les bords des rubans (m).
pub const JUPE: f32 = 0.6;
/// La hauteur des trottoirs au-dessus de la chaussée, le long des rues (m).
pub const TROTTOIR: f32 = 0.15;
/// Le sol terrassé au bord des voies, un peu sous elles : la jupe cache la marche, et
/// le relief ne se mêle pas aux rubans là où il passe dessous sans être coupé (m).
pub const ENFOUI: f32 = 0.1;
/// Les voies piétonnes un peu au-dessus des rues, là où elles se recouvrent (m).
const DESSUS_PIETON: f32 = 0.02;
/// La plus grande marge à plat de `sol` (une case des tuiles fines, m).
pub const MARGE_MAX: f32 = 6.0;
/// Le côté des cellules de l'index (m).
const CELLULE: f32 = 16.0;

/// Une branche maillée : ses points (x, z, hauteur), la demi-largeur de la chaussée et
/// celle de la plateforme comprise, son genre.
#[derive(Clone, Debug)]
pub struct Ruban {
    pub genre: u8,
    pub chaussee: f32,
    pub large: f32,
    pub points: Vec<[f32; 3]>,
}

/// Une place de carrefour : son contour (x, z), convexe, et sa hauteur.
#[derive(Clone, Debug)]
pub struct Place {
    pub contour: Vec<[f32; 2]>,
    pub hauteur: f32,
}

#[derive(Clone, Copy, Debug)]
enum Piece {
    /// Le segment `i` du ruban `r`.
    Segment(u32, u32),
    Place(u32),
}

/// Les bords d'un ruban en chaque point : dehors gauche, chaussée gauche, chaussée
/// droite, dehors droit (x, z), en onglet aux angles.
fn bords(r: &Ruban) -> Vec<[[f32; 2]; 4]> {
    let p = &r.points;
    let n = p.len();
    let normale = |i: usize| {
        let (a, b) = (p[i], p[i + 1]);
        let l = (b[0] - a[0]).hypot(b[1] - a[1]).max(1e-6);
        [-(b[1] - a[1]) / l, (b[0] - a[0]) / l]
    };
    (0..n)
        .map(|i| {
            let (m, k) = if i == 0 {
                (normale(0), 1.0)
            } else if i == n - 1 {
                (normale(n - 2), 1.0)
            } else {
                let (a, b) = (normale(i - 1), normale(i));
                let s = [a[0] + b[0], a[1] + b[1]];
                let l = s[0].hypot(s[1]);
                if l < 1e-3 {
                    (b, 1.0)
                } else {
                    let m = [s[0] / l, s[1] / l];
                    (m, 1.0 / (m[0] * b[0] + m[1] * b[1]).max(0.5))
                }
            };
            let o = |d: f32| [p[i][0] + m[0] * d * k, p[i][1] + m[1] * d * k];
            [o(r.large), o(r.chaussee), o(-r.chaussee), o(-r.large)]
        })
        .collect()
}

/// Le point (x, z) dans le polygone (quelconque) ?
fn dedans(poly: &[[f32; 2]], q: [f32; 2]) -> bool {
    let mut c = false;
    let n = poly.len();
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + n - 1) % n]);
        if (a[1] > q[1]) != (b[1] > q[1]) && q[0] < (b[0] - a[0]) * (q[1] - a[1]) / (b[1] - a[1]) + a[0] {
            c = !c;
        }
    }
    c
}

/// (la fraction le long de a → b du point le plus proche de q, la distance).
fn projeter(a: [f32; 3], b: [f32; 3], q: [f32; 2]) -> (f32, f32) {
    let (ex, ez) = (b[0] - a[0], b[1] - a[1]);
    let l2 = ex * ex + ez * ez;
    let u = if l2 > 1e-9 { (((q[0] - a[0]) * ex + (q[1] - a[1]) * ez) / l2).clamp(0.0, 1.0) } else { 0.0 };
    (u, (q[0] - a[0] - ex * u).hypot(q[1] - a[1] - ez * u))
}

/// L'enveloppe convexe de points (x, z), dans le sens trigonométrique.
fn enveloppe(mut p: Vec<[f32; 2]>) -> Vec<[f32; 2]> {
    p.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    p.dedup_by(|a, b| (a[0] - b[0]).abs() < 1e-4 && (a[1] - b[1]).abs() < 1e-4);
    if p.len() < 3 {
        return p;
    }
    let tour = |o: [f32; 2], a: [f32; 2], b: [f32; 2]| (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0]);
    let mut h: Vec<[f32; 2]> = Vec::new();
    for pass in 0..2 {
        let debut = h.len();
        let it: Box<dyn Iterator<Item = &[f32; 2]>> = if pass == 0 { Box::new(p.iter()) } else { Box::new(p.iter().rev()) };
        for &q in it {
            while h.len() >= debut + 2 && tour(h[h.len() - 2], h[h.len() - 1], q) <= 0.0 {
                h.pop();
            }
            h.push(q);
        }
        h.pop();
    }
    h
}

/// La voirie d'un lieu : ses rubans, ses places, et leur index.
#[derive(Default)]
pub struct Voirie {
    pub rubans: Vec<Ruban>,
    pub places: Vec<Place>,
    /// Les bords de chaque ruban (`bords`).
    contours: Vec<Vec<[[f32; 2]; 4]>>,
    /// Les pièces de chaque cellule (à 1 m près).
    index: HashMap<(i32, i32), Vec<Piece>>,
}

impl Voirie {
    /// La voirie des routes du lieu (hors ponts), au pied des tabliers `tabliers`.
    pub fn new(lieu: &Lieu, tabliers: &[Tablier]) -> Voirie {
        Voirie::depuis(&lieu.routes, &lieu.ponts, tabliers)
    }

    pub fn depuis(routes: &[Route], ponts: &[Route], tabliers: &[Tablier]) -> Voirie {
        // 1. Le réseau : les nœuds (points réunis au centimètre), leurs voisins, et la
        // route de chaque arête
        let cle = |p: [f32; 3]| ((p[0] * 100.0).round() as i64, (p[1] * 100.0).round() as i64);
        let mut ids: HashMap<(i64, i64), usize> = HashMap::default();
        let mut pos: Vec<[f32; 2]> = Vec::new();
        let mut somme: Vec<(f32, u32)> = Vec::new();
        let mut voisins: Vec<Vec<usize>> = Vec::new();
        let mut arete: HashMap<(usize, usize), usize> = HashMap::default();
        for (k, r) in routes.iter().enumerate() {
            let mut avant: Option<usize> = None;
            for &p in &r.points {
                let id = *ids.entry(cle(p)).or_insert_with(|| {
                    pos.push([p[0], p[1]]);
                    somme.push((0.0, 0));
                    voisins.push(Vec::new());
                    pos.len() - 1
                });
                somme[id].0 += p[2];
                somme[id].1 += 1;
                if let Some(a) = avant.filter(|&a| a != id) {
                    let e = (a.min(id), a.max(id));
                    if !arete.contains_key(&e) {
                        arete.insert(e, k);
                        voisins[a].push(id);
                        voisins[id].push(a);
                    }
                }
                avant = Some(id);
            }
        }
        let n = pos.len();
        if n == 0 {
            return Voirie::default();
        }
        let mut h: Vec<f32> = somme.iter().map(|s| s.0 / s.1.max(1) as f32).collect();
        let route_de = |a: usize, b: usize| arete.get(&(a.min(b), a.max(b))).map(|&k| &routes[k]);
        let rue = |a: usize, b: usize| route_de(a, b).is_some_and(|r| carrossable(r.genre));
        let large = |a: usize, b: usize| route_de(a, b).map_or(1.0, |r| r.largeur / 2.0 + plateforme(r.genre));
        let mut fixe = vec![false; n];
        // Au pied d'un pont : la hauteur du tablier
        let case = |x: f32, z: f32| ((x / 2.0).floor() as i32, (z / 2.0).floor() as i32);
        let mut pieds: HashMap<(i32, i32), Vec<[f32; 2]>> = HashMap::default();
        for p in ponts.iter().filter(|p| !p.points.is_empty()) {
            for b in [p.points[0], p.points[p.points.len() - 1]] {
                pieds.entry(case(b[0], b[1])).or_default().push([b[0], b[1]]);
            }
        }
        for i in 0..n {
            if voisins[i].len() != 1 {
                continue;
            }
            let q = pos[i];
            let (ci, cj) = case(q[0], q[1]);
            let pres = (cj - 1..=cj + 1).any(|j| (ci - 1..=ci + 1).any(|i| pieds.get(&(i, j)).is_some_and(|v| v.iter().any(|b| (b[0] - q[0]).hypot(b[1] - q[1]) < 2.0))));
            if !pres {
                continue;
            }
            if let Some((y, _)) = tabliers.iter().filter_map(|t| t.hauteur_en(q, 3.0)).min_by(|a, b| a.1.total_cmp(&b.1)) {
                h[i] = y;
                fixe[i] = true;
            }
        }
        // 2. Le lissage : les rues entre elles, puis le reste accroché aux rues
        // (les arêtes des rues, une fois pour toutes)
        let rue_v: Vec<Vec<bool>> = (0..n).map(|i| voisins[i].iter().map(|&j| rue(i, j)).collect()).collect();
        let lisser = |h: &mut Vec<f32>, fixe: &[bool], passes: usize, rues: bool, libre: &[bool]| {
            let mut avant = h.clone();
            for _ in 0..passes {
                avant.copy_from_slice(h);
                for i in 0..n {
                    if fixe[i] || !libre[i] {
                        continue;
                    }
                    let (mut s, mut c) = (avant[i], 1.0);
                    for (k, &j) in voisins[i].iter().enumerate() {
                        if !rues || rue_v[i][k] {
                            s += avant[j];
                            c += 1.0;
                        }
                    }
                    h[i] = s / c;
                }
            }
        };
        let est_rue: Vec<bool> = rue_v.iter().map(|v| v.iter().any(|&r| r)).collect();
        let tous = vec![true; n];
        lisser(&mut h, &fixe, PASSES, true, &est_rue);
        let mut fixe2 = fixe.clone();
        for i in 0..n {
            fixe2[i] |= est_rue[i];
        }
        lisser(&mut h, &fixe2, PASSES, false, &tous);
        // 3. Les carrefours à plat : leur rayon, leurs nœuds proches, puis les approches
        let carrefour: Vec<bool> = (0..n).map(|i| voisins[i].len() >= 3).collect();
        let rayon: Vec<f32> = (0..n).map(|i| if carrefour[i] { voisins[i].iter().map(|&j| large(i, j)).fold(0.0, f32::max) + MARGE_CARREFOUR } else { 0.0 }).collect();
        let mut fixe3 = fixe.clone();
        for c in (0..n).filter(|&i| carrefour[i]) {
            fixe3[c] = true;
            // Le long de chaque branche, jusqu'au rayon
            for &d in &voisins[c] {
                let (mut a, mut b) = (c, d);
                loop {
                    if (pos[b][0] - pos[c][0]).hypot(pos[b][1] - pos[c][1]) > rayon[c] || carrefour[b] || fixe[b] {
                        break;
                    }
                    h[b] = h[c];
                    fixe3[b] = true;
                    let Some(&s) = voisins[b].iter().find(|&&s| s != a) else { break };
                    (a, b) = (b, s);
                }
            }
        }
        lisser(&mut h, &fixe3, PASSES_APPROCHE, false, &tous);
        // 4. Les branches : d'un carrefour ou d'un bout à l'autre, à travers les nœuds de
        // passage (et les boucles sans carrefour)
        let mut vue: HashSet<(usize, usize)> = Default::default();
        let mut branches: Vec<Vec<usize>> = Vec::new();
        let arret = |i: usize| voisins[i].len() != 2;
        let mut departs: Vec<usize> = (0..n).filter(|&i| arret(i)).collect();
        departs.extend((0..n).filter(|&i| !arret(i)));
        for &s in &departs {
            for &t in &voisins[s] {
                if vue.contains(&(s.min(t), s.max(t))) {
                    continue;
                }
                let mut b = vec![s];
                let (mut a, mut c) = (s, t);
                loop {
                    vue.insert((a.min(c), a.max(c)));
                    b.push(c);
                    if arret(c) || c == s {
                        break;
                    }
                    let Some(&d) = voisins[c].iter().find(|&&d| d != a && !vue.contains(&(c.min(d), c.max(d)))) else { break };
                    (a, c) = (c, d);
                }
                branches.push(b);
            }
        }
        // 5. Les rubans, arrêtés au rayon des carrefours ; les places
        let mut rubans = Vec::new();
        let mut bouts: HashMap<usize, Vec<[f32; 2]>> = HashMap::default();
        for b in &branches {
            if b.len() < 2 {
                continue;
            }
            // La route la plus large de la branche
            let r = b.windows(2).filter_map(|w| route_de(w[0], w[1])).max_by(|x, y| x.largeur.total_cmp(&y.largeur));
            let Some(r) = r else { continue };
            let pieton = if carrossable(r.genre) { 0.0 } else { DESSUS_PIETON };
            let mut pts: Vec<[f32; 3]> = b.iter().map(|&i| [pos[i][0], pos[i][1], h[i] + pieton]).collect();
            let longueur: f32 = pts.windows(2).map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1])).sum();
            let (d0, d1) = (if carrefour[b[0]] { rayon[b[0]].min(longueur * 0.45) } else { 0.0 }, if carrefour[b[b.len() - 1]] { rayon[b[b.len() - 1]].min(longueur * 0.45) } else { 0.0 });
            pts = rogner(&pts, d0, d1);
            if pts.len() < 2 {
                continue;
            }
            // Au bord d'une place : sa hauteur
            let k = pts.len() - 1;
            if d0 > 0.0 {
                pts[0][2] = h[b[0]] + pieton;
            }
            if d1 > 0.0 {
                pts[k][2] = h[b[b.len() - 1]] + pieton;
            }
            let ruban = Ruban { genre: r.genre, chaussee: r.largeur / 2.0, large: r.largeur / 2.0 + plateforme(r.genre), points: pts };
            let bo = bords(&ruban);
            for (k, i) in [(0, b[0]), (bo.len() - 1, b[b.len() - 1])] {
                if carrefour[i] {
                    bouts.entry(i).or_default().extend([bo[k][0], bo[k][3]]);
                }
            }
            rubans.push(ruban);
        }
        let mut places = Vec::new();
        let mut cles: Vec<usize> = bouts.keys().copied().collect();
        cles.sort_unstable();
        for c in cles {
            let mut p = bouts[&c].clone();
            p.push(pos[c]);
            let contour = enveloppe(p);
            if contour.len() >= 3 {
                places.push(Place { contour, hauteur: h[c] });
            }
        }
        Voirie::ranger(rubans, places)
    }

    /// Les rubans et les places, rangés par cellule.
    pub fn ranger(rubans: Vec<Ruban>, places: Vec<Place>) -> Voirie {
        let contours: Vec<_> = rubans.iter().map(bords).collect();
        let mut index: HashMap<(i32, i32), Vec<Piece>> = HashMap::default();
        let mut poser = |b: [f32; 4], p: Piece| {
            let m = 1.0;
            let (i0, i1) = (((b[0] - m) / CELLULE).floor() as i32, ((b[2] + m) / CELLULE).floor() as i32);
            let (j0, j1) = (((b[1] - m) / CELLULE).floor() as i32, ((b[3] + m) / CELLULE).floor() as i32);
            for j in j0..=j1 {
                for i in i0..=i1 {
                    index.entry((i, j)).or_default().push(p);
                }
            }
        };
        for (r, c) in contours.iter().enumerate() {
            for i in 0..c.len().saturating_sub(1) {
                let q = [c[i][0], c[i][3], c[i + 1][0], c[i + 1][3]];
                let b = q.iter().fold([f32::MAX, f32::MAX, f32::MIN, f32::MIN], |b, p| [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])]);
                poser(b, Piece::Segment(r as u32, i as u32));
            }
        }
        for (k, p) in places.iter().enumerate() {
            let b = p.contour.iter().fold([f32::MAX, f32::MAX, f32::MIN, f32::MIN], |b, q| [b[0].min(q[0]), b[1].min(q[1]), b[2].max(q[0]), b[3].max(q[1])]);
            poser(b, Piece::Place(k as u32));
        }
        Voirie { rubans, places, contours, index }
    }

    fn pieces(&self, x: f32, z: f32) -> &[Piece] {
        self.index.get(&((x / CELLULE).floor() as i32, (z / CELLULE).floor() as i32)).map_or(&[], |v| v.as_slice())
    }

    /// Les pièces des cellules à moins de `r` de (x, z) (une pièce peut revenir).
    fn autour(&self, x: f32, z: f32, r: f32) -> impl Iterator<Item = Piece> + '_ {
        let (i0, i1) = (((x - r) / CELLULE).floor() as i32, ((x + r) / CELLULE).floor() as i32);
        let (j0, j1) = (((z - r) / CELLULE).floor() as i32, ((z + r) / CELLULE).floor() as i32);
        (j0..=j1).flat_map(move |j| (i0..=i1).flat_map(move |i| self.index.get(&(i, j)).map_or(&[][..], |v| v.as_slice()).iter().copied()))
    }

    /// Des voies passent-elles près de la boîte (x0, z0, x1, z1) (à la portée des talus) ?
    pub fn pres(&self, b: [f32; 4]) -> bool {
        let r = PORTEE + MARGE_MAX + 1.0;
        let b = [b[0] - r, b[1] - r, b[2] + r, b[3] + r];
        let (i0, i1) = ((b[0] / CELLULE).floor() as i32, (b[2] / CELLULE).floor() as i32);
        let (j0, j1) = ((b[1] / CELLULE).floor() as i32, (b[3] / CELLULE).floor() as i32);
        (j0..=j1).any(|j| (i0..=i1).any(|i| self.index.contains_key(&(i, j))))
    }

    /// Le contour (x, z) du segment `i` du ruban `r` (le dehors de la plateforme).
    fn quad(&self, r: u32, i: u32) -> [[f32; 2]; 4] {
        let c = &self.contours[r as usize];
        let (a, b) = (c[i as usize], c[i as usize + 1]);
        [a[0], b[0], b[3], a[3]]
    }

    /// La hauteur de la voie sur laquelle tombe (x, z), s'il y en a une.
    pub fn sur(&self, x: f32, z: f32) -> Option<f32> {
        let q = [x, z];
        let mut mieux: Option<(f32, f32)> = None;
        for &p in self.pieces(x, z) {
            match p {
                Piece::Segment(r, i) => {
                    if dedans(&self.quad(r, i), q) {
                        let pts = &self.rubans[r as usize].points;
                        let (a, b) = (pts[i as usize], pts[i as usize + 1]);
                        let (u, d) = projeter(a, b, q);
                        if mieux.is_none_or(|m| d < m.1) {
                            mieux = Some((a[2] + (b[2] - a[2]) * u, d));
                        }
                    }
                }
                Piece::Place(k) => {
                    let pl = &self.places[k as usize];
                    if dedans(&pl.contour, q) {
                        return Some(pl.hauteur);
                    }
                }
            }
        }
        mieux.map(|m| m.0)
    }

    /// (l'écart au dehors de la plateforme la plus proche, négatif dessus ; la hauteur
    /// de la voie là), à `PORTEE` (plus la marge) près.
    fn proche(&self, x: f32, z: f32) -> Option<(f32, f32)> {
        let q = [x, z];
        let mut mieux: Option<(f32, f32)> = None;
        for p in self.autour(x, z, PORTEE + MARGE_MAX) {
            match p {
                Piece::Segment(r, i) => {
                    let rb = &self.rubans[r as usize];
                    let (a, b) = (rb.points[i as usize], rb.points[i as usize + 1]);
                    let (u, d) = projeter(a, b, q);
                    let e = d - rb.large;
                    if mieux.is_none_or(|m| e < m.0) {
                        mieux = Some((e, a[2] + (b[2] - a[2]) * u));
                    }
                }
                Piece::Place(k) => {
                    let pl = &self.places[k as usize];
                    if dedans(&pl.contour, q) {
                        return Some((-1.0, pl.hauteur));
                    }
                }
            }
        }
        mieux
    }

    /// La hauteur de la voie la plus proche de (x, z) (au bord d'un ruban : la sienne).
    pub fn hauteur(&self, x: f32, z: f32) -> Option<f32> {
        self.sur(x, z).or_else(|| self.proche(x, z).map(|p| p.1))
    }

    /// Le sol terrassé en (x, z), d'après le relief `relief` : à la hauteur de la voie
    /// sous sa plateforme, à plat (`ENFOUI` dessous) sur `marge` mètres au-delà (une
    /// case de la grille : le relief entre deux points reste au niveau de la voie), puis
    /// un talus jusqu'au relief (pas dans l'eau).
    pub fn sol(&self, x: f32, z: f32, relief: f32, eau: bool, marge: f32) -> f32 {
        if eau {
            return relief;
        }
        if let Some(y) = self.sur(x, z) {
            return y;
        }
        let Some((e, y)) = self.proche(x, z) else { return relief };
        if e <= 0.0 {
            return y;
        }
        let (e, y) = (e - marge, y - ENFOUI);
        if e <= 0.0 {
            return y;
        }
        if e >= PORTEE {
            return relief;
        }
        let t = relief.clamp(y - PENTE * e, y + PENTE * e);
        if e > PORTEE - FONDU {
            let k = (e - (PORTEE - FONDU)) / FONDU;
            t + (relief - t) * k
        } else {
            t
        }
    }

    /// Les rubans et les places dans la boîte `b` (x0, z0, x1, z1), découpés à son bord,
    /// ajoutés au maillage (sommets de `SOMMET` flottants, la texture prise dans la boîte).
    pub fn monter(&self, b: [f32; 4], v: &mut Vec<f32>, ind: &mut Vec<u32>) {
        let mut faits: HashSet<(u8, u32, u32)> = HashSet::default();
        let (i0, i1) = ((b[0] / CELLULE).floor() as i32, (b[2] / CELLULE).floor() as i32);
        let (j0, j1) = ((b[1] / CELLULE).floor() as i32, (b[3] / CELLULE).floor() as i32);
        let sommet = |x: f32, y: f32, z: f32, n: [f32; 3], mat: f32| -> [f32; SOMMET] { [x, y, z, n[0], n[1], n[2], (x - b[0]) / (b[2] - b[0]), (z - b[1]) / (b[3] - b[1]), mat] };
        let poly = |p: Vec<[f32; SOMMET]>, v: &mut Vec<f32>, ind: &mut Vec<u32>| {
            let p = decouper(p, b);
            if p.len() < 3 {
                return;
            }
            let d = (v.len() / SOMMET) as u32;
            for s in &p {
                v.extend_from_slice(s);
            }
            for k in 1..p.len() as u32 - 1 {
                ind.extend_from_slice(&[d, d + k, d + k + 1]);
            }
        };
        let haut = [0.0, 1.0, 0.0];
        for j in j0..=j1 {
            for i in i0..=i1 {
                for &p in self.index.get(&(i, j)).map_or(&[][..], |v| v.as_slice()) {
                    let cle = match p {
                        Piece::Segment(r, k) => (0, r, k),
                        Piece::Place(k) => (1, k, 0),
                    };
                    if !faits.insert(cle) {
                        continue;
                    }
                    match p {
                        Piece::Segment(r, k) => {
                            let rb = &self.rubans[r as usize];
                            let c = &self.contours[r as usize];
                            let (a, bb) = (c[k as usize], c[k as usize + 1]);
                            let (ya, yb) = (rb.points[k as usize][2], rb.points[k as usize + 1][2]);
                            // Le trottoir, plus haut que la chaussée (les rues seulement)
                            let t = if carrossable(rb.genre) { TROTTOIR } else { 0.0 };
                            let dernier = k as usize + 2 == rb.points.len();
                            // Un mur de p à q, de haut en bas (le dehors : à gauche de p → q)
                            let mur = |p: [f32; 2], q: [f32; 2], (hp, hq): (f32, f32), (bp, bq): (f32, f32), v: &mut Vec<f32>, ind: &mut Vec<u32>| {
                                let (ex, ez) = (q[0] - p[0], q[1] - p[1]);
                                let l = ex.hypot(ez);
                                if l < 1e-3 {
                                    return;
                                }
                                let n = [-ez / l, 0.0, ex / l];
                                poly(vec![sommet(p[0], hp, p[1], n, BORD), sommet(q[0], hq, q[1], n, BORD), sommet(q[0], bq, q[1], n, BORD), sommet(p[0], bp, p[1], n, BORD)], v, ind);
                            };
                            // Les trois bandes : trottoir gauche, chaussée, trottoir droit
                            // (le sens : vers le haut)
                            for (g, d, mat, h) in [(0, 1, BORD, t), (1, 2, CHAUSSEE, 0.0), (2, 3, BORD, t)] {
                                if (a[g][0] - a[d][0]).hypot(a[g][1] - a[d][1]) < 1e-3 {
                                    continue;
                                }
                                poly(vec![sommet(a[g][0], ya + h, a[g][1], haut, mat), sommet(bb[g][0], yb + h, bb[g][1], haut, mat), sommet(bb[d][0], yb + h, bb[d][1], haut, mat), sommet(a[d][0], ya + h, a[d][1], haut, mat)], v, ind);
                                // Les bouts du ruban, fermés
                                if k == 0 {
                                    mur(a[d], a[g], (ya + h, ya + h), (ya - JUPE, ya - JUPE), v, ind);
                                }
                                if dernier {
                                    mur(bb[g], bb[d], (yb + h, yb + h), (yb - JUPE, yb - JUPE), v, ind);
                                }
                            }
                            // Les bordures, de la chaussée au trottoir
                            if t > 0.0 {
                                mur(bb[1], a[1], (yb + t, ya + t), (yb, ya), v, ind);
                                mur(a[2], bb[2], (ya + t, yb + t), (ya, yb), v, ind);
                            }
                            // Les jupes des deux bords, sous le trottoir
                            mur(a[0], bb[0], (ya + t, yb + t), (ya - JUPE, yb - JUPE), v, ind);
                            mur(bb[3], a[3], (yb + t, ya + t), (yb - JUPE, ya - JUPE), v, ind);
                        }
                        Piece::Place(k) => {
                            let pl = &self.places[k as usize];
                            let y = pl.hauteur;
                            // Le contour tourne dans le sens trigonométrique en (x, z) : vers
                            // le bas en (x, y, z) ; on le parcourt à l'envers
                            poly(pl.contour.iter().rev().map(|q| sommet(q[0], y, q[1], haut, CHAUSSEE)).collect(), v, ind);
                            let n = pl.contour.len();
                            for e in 0..n {
                                let (p, q) = (pl.contour[e], pl.contour[(e + 1) % n]);
                                let (ex, ez) = (q[0] - p[0], q[1] - p[1]);
                                let l = ex.hypot(ez).max(1e-6);
                                let nn = [ez / l, 0.0, -ex / l];
                                poly(vec![sommet(p[0], y, p[1], nn, BORD), sommet(q[0], y, q[1], nn, BORD), sommet(q[0], y - JUPE, q[1], nn, BORD), sommet(p[0], y - JUPE, p[1], nn, BORD)], v, ind);
                            }
                        }
                    }
                }
            }
        }
    }
}

/// La ligne raccourcie de `d0` mètres au début et de `d1` à la fin.
fn rogner(p: &[[f32; 3]], d0: f32, d1: f32) -> Vec<[f32; 3]> {
    let couper = |p: &[[f32; 3]], d: f32| -> Vec<[f32; 3]> {
        if d <= 0.0 {
            return p.to_vec();
        }
        let mut reste = d;
        for i in 0..p.len() - 1 {
            let (a, b) = (p[i], p[i + 1]);
            let l = (b[0] - a[0]).hypot(b[1] - a[1]);
            if l >= reste && l > 1e-6 {
                let t = reste / l;
                let m = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t];
                let mut v = vec![m];
                v.extend_from_slice(&p[i + 1..]);
                if (v[0][0] - v[1][0]).hypot(v[0][1] - v[1][1]) < 0.05 && v.len() > 2 {
                    v.remove(0);
                    v[0] = m;
                }
                return v;
            }
            reste -= l;
        }
        vec![p[p.len() - 1]]
    };
    let mut v = couper(p, d0);
    v.reverse();
    let mut v = couper(&v, d1);
    v.reverse();
    v
}

/// Le polygone découpé à la boîte (x0, z0, x1, z1) (Sutherland–Hodgman, en x et z).
fn decouper(mut p: Vec<[f32; SOMMET]>, b: [f32; 4]) -> Vec<[f32; SOMMET]> {
    for (axe, borne, garder_plus) in [(0, b[0], true), (0, b[2], false), (2, b[1], true), (2, b[3], false)] {
        if p.is_empty() {
            break;
        }
        let dans = |s: &[f32; SOMMET]| if garder_plus { s[axe] >= borne } else { s[axe] <= borne };
        let mut q = Vec::with_capacity(p.len() + 2);
        for i in 0..p.len() {
            let (a, c) = (p[i], p[(i + 1) % p.len()]);
            if dans(&a) {
                q.push(a);
            }
            if dans(&a) != dans(&c) {
                let t = (borne - a[axe]) / (c[axe] - a[axe]);
                let mut m = [0f32; SOMMET];
                for k in 0..SOMMET {
                    m[k] = a[k] + (c[k] - a[k]) * t;
                }
                m[axe] = borne;
                q.push(m);
            }
        }
        p = q;
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use sage_reel::{GRANDE, PIETONNE, ROUTE};

    fn route(genre: u8, largeur: f32, pts: &[[f32; 3]]) -> Route {
        Route { genre, largeur, points: pts.to_vec() }
    }

    #[test]
    fn un_carrefour_a_une_seule_hauteur_et_se_met_a_plat() {
        // Trois routes qui se rejoignent en (0, 0), chacune à sa hauteur
        let a = route(ROUTE, 6.0, &[[-80.0, 0.0, 10.0], [-40.0, 0.0, 10.0], [-8.0, 0.0, 10.0], [0.0, 0.0, 10.0]]);
        let b = route(ROUTE, 6.0, &[[0.0, 0.0, 12.0], [8.0, 0.0, 12.0], [40.0, 0.0, 12.0], [80.0, 0.0, 12.0]]);
        let c = route(ROUTE, 6.0, &[[0.0, 0.0, 14.0], [0.0, 8.0, 14.0], [0.0, 40.0, 14.0], [0.0, 80.0, 14.0]]);
        let v = Voirie::depuis(&[a, b, c], &[], &[]);
        assert_eq!(v.places.len(), 1);
        assert_eq!(v.rubans.len(), 3);
        // La place à une hauteur entre les trois, et chaque ruban y arrive à cette hauteur
        let y = v.places[0].hauteur;
        assert!(y > 10.0 && y < 14.0, "{y}");
        for r in &v.rubans {
            let p = r.points.iter().min_by(|p, q| p[0].hypot(p[1]).total_cmp(&q[0].hypot(q[1]))).unwrap();
            assert!((p[2] - y).abs() < 1e-3, "{p:?} {y}");
            // Arrêté au rayon : 3 + 2 + 1 = 6 m du carrefour
            assert!((p[0].hypot(p[1]) - 6.0).abs() < 0.05, "{p:?}");
        }
        // La place couvre le carrefour, à sa hauteur ; le sol aussi
        assert_eq!(v.sur(0.0, 0.0), Some(y));
        assert_eq!(v.sol(0.5, 0.5, 0.0, false, 0.0), y);
        // Les profils sont doux : pas plus de 0,6 m entre deux points voisins
        for r in &v.rubans {
            assert!(r.points.windows(2).all(|w| (w[1][2] - w[0][2]).abs() < 0.6 * (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]).max(1.0)), "{:?}", r.points);
        }
    }

    #[test]
    fn deux_voies_bout_a_bout_font_un_seul_ruban_lisse() {
        // Une marche de 3 m au raccord : le profil l'étale
        let pts = |x0: f32, y: f32| -> Vec<[f32; 3]> { (0..=10).map(|i| [x0 + i as f32 * 8.0, 0.0, y]).collect() };
        let v = Voirie::depuis(&[route(ROUTE, 6.0, &pts(0.0, 10.0)), route(ROUTE, 6.0, &pts(80.0, 13.0))], &[], &[]);
        assert_eq!(v.rubans.len(), 1);
        assert!(v.places.is_empty());
        let p = &v.rubans[0].points;
        let saut = p.windows(2).map(|w| (w[1][2] - w[0][2]).abs()).fold(0.0, f32::max);
        assert!(saut < 0.8, "{saut} {p:?}");
    }

    #[test]
    fn le_pied_d_un_pont_prend_la_hauteur_du_tablier() {
        let pont = route(GRANDE, 8.0, &[[100.0, 0.0, 15.0], [200.0, 0.0, 15.0]]);
        let tabliers = sage_reel::tabliers::tabliers(std::slice::from_ref(&pont));
        let r = route(GRANDE, 8.0, &(0..=12).map(|i| [i as f32 * 100.0 / 12.0, 0.0, 9.0]).collect::<Vec<_>>());
        let v = Voirie::depuis(&[r], &[pont], &tabliers);
        let p = &v.rubans[0].points;
        let bout = p.iter().max_by(|a, b| a[0].total_cmp(&b[0])).unwrap();
        assert!((bout[2] - 15.0).abs() < 1e-3, "{bout:?}");
    }

    #[test]
    fn le_sol_se_terrasse_en_talus() {
        let v = Voirie::depuis(&[route(ROUTE, 6.0, &(0..=20).map(|i| [i as f32 * 8.0, 0.0, 10.0]).collect::<Vec<_>>())], &[], &[]);
        // Sur la plateforme (3 + 2 m) : la voie ; au-delà, 2/3 par mètre ; loin : le relief
        let y = 10.0 - ENFOUI;
        assert!((v.sol(80.0, 4.5, 30.0, false, 0.0) - 10.0).abs() < 1e-3);
        assert!((v.sol(80.0, 8.0, 30.0, false, 0.0) - (y + 2.0)).abs() < 1e-3);
        assert!((v.sol(80.0, 8.0, 0.0, false, 0.0) - (y - 2.0)).abs() < 1e-3);
        assert!((v.sol(80.0, 8.0, 10.5, false, 0.0) - 10.5).abs() < 1e-3);
        assert!((v.sol(80.0, 40.0, 30.0, false, 0.0) - 30.0).abs() < 1e-3);
        // Une marge à plat d'une case (4 m), puis le talus
        assert!((v.sol(80.0, 8.0, 30.0, false, 4.0) - y).abs() < 1e-3);
        assert!((v.sol(80.0, 12.0, 30.0, false, 4.0) - (y + 2.0)).abs() < 1e-3);
        // Dans l'eau : le relief
        assert_eq!(v.sol(80.0, 8.0, 30.0, true, 0.0), 30.0);
    }

    #[test]
    fn les_rubans_se_montent_dans_la_boite() {
        let v = Voirie::depuis(&[route(ROUTE, 6.0, &[[-50.0, 10.0, 10.0], [50.0, 10.0, 10.0]]), route(PIETONNE, 2.0, &[[-50.0, 30.0, 10.0], [50.0, 30.0, 10.0]])], &[], &[]);
        let (mut s, mut ind) = (Vec::new(), Vec::new());
        v.monter([0.0, 0.0, 32.0, 32.0], &mut s, &mut ind);
        assert!(!ind.is_empty());
        for m in s.chunks(SOMMET) {
            assert!(m[0] >= -1e-3 && m[0] <= 32.001 && m[2] >= -1e-3 && m[2] <= 32.001, "{m:?}");
            assert!(m[6] >= -1e-3 && m[6] <= 1.001);
        }
        // Le dessus de la chaussée, tourné vers le haut (le sens des triangles)
        let p = |i: u32| [s[i as usize * SOMMET], s[i as usize * SOMMET + 1], s[i as usize * SOMMET + 2]];
        for t in ind.chunks(3) {
            if s[t[0] as usize * SOMMET + 4] > 0.9 {
                let (a, b, c) = (p(t[0]), p(t[1]), p(t[2]));
                let ny = (b[2] - a[2]) * (c[0] - a[0]) - (b[0] - a[0]) * (c[2] - a[2]);
                assert!(ny >= -1e-6, "{a:?} {b:?} {c:?}");
            }
        }
    }
}
