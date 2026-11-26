//! Les bords des chaussées, tirés des voies et de leurs files seulement (pas de la
//! chaussée matérialisée) : le contour de l'union des chaussées des voies carrossables.
//!
//! Chaque voie a une bande : son axe (déporté près d'une fourche, comme les files)
//! élargi d'une demi-largeur de chaque côté. Ses deux bords sont gardés là où ils ne
//! sont dans la bande d'aucune autre voie du même niveau : aux carrefours, les bords
//! se coupent d'eux-mêmes en coins, ceux des courtes voies internes d'un carrefour de
//! nœuds proches disparaissent, et ce qui reste se lie en boucles (autour d'un pâté de
//! maisons). Les bords suivent le bord extérieur des files extrêmes : aux raccords
//! des fourches, ils finissent sur celui des files du tronc qu'elles continuent. Seule
//! une impasse est fermée d'un trait en travers, et les micro coupures près d'un nœud
//! (deux bouts libres à moins de `SOUDURE`) sont refermées : les deux bouts sont
//! confondus en leur milieu.
//!
//! Les voies de bus en site propre (`sage_reel::BUS`, le TEOR) sont une surface à part,
//! qui passe sans être coupée : leurs bords ne s'arrêtent que dans une autre voie de
//! bus ; les voies qui les traversent s'arrêtent à leur bord et s'y referment. Les
//! liens aux nœuds se font entre voies de même sorte.

use sage_reel::reseau::{Passage, Reseau};
use std::collections::HashMap;

/// Les bandes sont rétrécies d'autant (m) : un bord posé sur le bord d'une autre bande
/// (une voie qui en continue une autre) n'est pas « dedans ».
const EPS: f32 = 0.02;
/// Deux bouts de bords près d'un nœud à moins de tant (m) l'un de l'autre sont confondus.
const SOUDURE: f32 = 1.0;
/// Partout, deux bouts à moins de tant (m) l'un de l'autre sont confondus.
/// Le biseau dans l'alignement : l'écart d'angle admis (degrés), sa longueur par
/// mètre de demi-largeur perdu, et au moins.
const BISEAU_ANGLE: f32 = 20.0;
const BISEAU_PENTE: f32 = 5.0;
const BISEAU_MIN: f32 = 4.0;
/// Le dépassement d'une bande au-delà d'un nœud où une autre voie la prolonge (m).
const RECOUVRE: f32 = 0.5;
const SOUDURE_FINE: f32 = 0.15;
/// La maille (m) de l'index des bandes.
const CASE: f32 = 10.0;
/// Une bande ne coupe un bord que s'ils sont à moins de tant de mètres en hauteur (un
/// pont ne coupe pas la rue qu'il enjambe).
const ECART_H: f32 = 2.0;

/// Le point de croisement des segments `a`–`b` et `c`–`d`, s'ils se croisent.
fn croisement(a: [f32; 2], b: [f32; 2], c: [f32; 2], d: [f32; 2]) -> Option<[f32; 2]> {
    let r = [b[0] - a[0], b[1] - a[1]];
    let s = [d[0] - c[0], d[1] - c[1]];
    let den = r[0] * s[1] - r[1] * s[0];
    if den.abs() < 1e-9 {
        return None;
    }
    let q = [c[0] - a[0], c[1] - a[1]];
    let t = (q[0] * s[1] - q[1] * s[0]) / den;
    let u = (q[0] * r[1] - q[1] * r[0]) / den;
    ((0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u)).then(|| [a[0] + r[0] * t, a[1] + r[1] * t])
}

/// La part (t0, t1) du segment `a`–`b` dans le cercle de centre `c` et de rayon `ray`.
fn dans_cercle(a: [f32; 2], b: [f32; 2], c: [f32; 2], ray: f32) -> Option<(f32, f32)> {
    let d = [b[0] - a[0], b[1] - a[1]];
    let f = [a[0] - c[0], a[1] - c[1]];
    let qa = d[0] * d[0] + d[1] * d[1];
    let qb = 2.0 * (f[0] * d[0] + f[1] * d[1]);
    let qc = f[0] * f[0] + f[1] * f[1] - ray * ray;
    if qa < 1e-12 {
        return (qc <= 0.0).then_some((0.0, 1.0));
    }
    let disc = qb * qb - 4.0 * qa * qc;
    if disc < 0.0 {
        return None;
    }
    let sq = disc.sqrt();
    let (t0, t1) = (((-qb - sq) / (2.0 * qa)).max(0.0), ((-qb + sq) / (2.0 * qa)).min(1.0));
    (t1 > t0).then_some((t0, t1))
}

type Poly = Vec<[f32; 3]>;

/// Un morceau de bande : le quadrilatère d'un segment de voie.
struct Bande {
    arete: usize,
    /// Les voies dont les bords ne sont pas coupés par ce morceau (un joint : celles qui
    /// arrivent à son nœud).
    exclues: Vec<usize>,
    /// Le nœud d'un changement de passage près duquel ce morceau se trouve : là, il
    /// coupe aussi les bords des voies d'autres passages qui y arrivent.
    portail: Option<usize>,
    passage: Passage,
    bus: bool,
    q: [[f32; 2]; 4],
    /// +1 si `q` tourne dans le sens direct, −1 sinon.
    sens: f32,
    h: f32,
}

/// L'intervalle (t0, t1) du segment p0 → p1 à l'intérieur du quadrilatère convexe `b`.
fn dedans(p0: [f32; 2], p1: [f32; 2], b: &Bande) -> Option<(f32, f32)> {
    let (mut te, mut tl) = (0.0f32, 1.0f32);
    let d = [p1[0] - p0[0], p1[1] - p0[1]];
    for i in 0..4 {
        let (a, c) = (b.q[i], b.q[(i + 1) % 4]);
        let e = [c[0] - a[0], c[1] - a[1]];
        // La normale vers l'intérieur
        let n = [-e[1] * b.sens, e[0] * b.sens];
        let num = n[0] * (p0[0] - a[0]) + n[1] * (p0[1] - a[1]);
        let den = n[0] * d[0] + n[1] * d[1];
        if den.abs() < 1e-12 {
            if num <= 0.0 {
                return None;
            }
            continue;
        }
        let t = -num / den;
        if den > 0.0 {
            te = te.max(t);
        } else {
            tl = tl.min(t);
        }
        if te >= tl {
            return None;
        }
    }
    Some((te, tl))
}

/// Le quadrilatère est-il convexe (et non plat) ? Son sens.
fn convexe(q: &[[f32; 2]; 4]) -> Option<f32> {
    let mut signe = 0.0f32;
    for i in 0..4 {
        let (a, b, c) = (q[i], q[(i + 1) % 4], q[(i + 2) % 4]);
        let x = (b[0] - a[0]) * (c[1] - b[1]) - (b[1] - a[1]) * (c[0] - b[0]);
        if x.abs() < 1e-9 {
            continue;
        }
        if signe == 0.0 {
            signe = x.signum();
        } else if x.signum() != signe {
            return None;
        }
    }
    (signe != 0.0).then_some(signe)
}

fn case(x: f32, z: f32) -> (i32, i32) {
    ((x / CASE).floor() as i32, (z / CASE).floor() as i32)
}

/// Les bandes et leur index.
pub struct Bandes {
    bandes: Vec<Bande>,
    index: HashMap<(i32, i32), Vec<u32>>,
    /// Les deux nœuds de chaque arête (une bouche de tunnel, une tête de pont : les
    /// voies qui s'y rejoignent s'y coupent malgré leurs passages différents).
    noeuds: HashMap<usize, [usize; 2]>,
    /// Le cercle de chaque portail (centre, rayon) : un autre passage n'y coupe que dedans.
    cercles: HashMap<usize, ([f32; 2], f32)>,
    /// L'origine de chaque bord de la sortie (diagnostic) : voie, côté, raccords, girations.
    pub origines: Vec<String>,
}

impl Bandes {
    /// Les morceaux de `p` hors des bandes (sauf celles des arêtes `sauf`) de son niveau.
    /// Le bord d'une voie de bus (`bus`) n'est coupé que par les voies de bus.
    fn garder(&self, p: &[[f32; 3]], passage: Passage, bus: bool, sauf: &[usize]) -> Vec<Poly> {
        self.decouper(p, passage, bus, sauf, false)
    }

    /// `garder`, ou à l'inverse (`dedans`) les morceaux de `p` dans les bandes des voies
    /// qui ne sont pas de bus.
    fn decouper(&self, p: &[[f32; 3]], passage: Passage, bus: bool, sauf: &[usize], dedans_: bool) -> Vec<Poly> {
        let mut sortie = Vec::new();
        let mut cur: Poly = Vec::new();
        let mut ouvert = false;
        let mut vus: Vec<u32> = Vec::new();
        for w in p.windows(2) {
            let (p0, p1) = ([w[0][0], w[0][1]], [w[1][0], w[1][1]]);
            let l = (p1[0] - p0[0]).hypot(p1[1] - p0[1]);
            let at = |t: f32| [w[0][0] + (w[1][0] - w[0][0]) * t, w[0][1] + (w[1][1] - w[0][1]) * t, w[0][2] + (w[1][2] - w[0][2]) * t];
            // Les intervalles couverts
            let (a, z) = (case(p0[0].min(p1[0]), p0[1].min(p1[1])), case(p0[0].max(p1[0]), p0[1].max(p1[1])));
            vus.clear();
            for ci in a.0..=z.0 {
                for cj in a.1..=z.1 {
                    if let Some(v) = self.index.get(&(ci, cj)) {
                        vus.extend_from_slice(v);
                    }
                }
            }
            vus.sort_unstable();
            vus.dedup();
            let hm = (w[0][2] + w[1][2]) / 2.0;
            let mut couverts: Vec<(f32, f32)> = Vec::new();
            for &i in &vus {
                let b = &self.bandes[i as usize];
                // Un autre passage ne coupe qu'au portail que les deux voies partagent
                let au_portail = || b.portail.is_some_and(|n| sauf.first().and_then(|k| self.noeuds.get(k)).is_some_and(|ns| ns.contains(&n)));
                if sauf.contains(&b.arete) || b.exclues.iter().any(|e| sauf.contains(e)) || (b.passage != passage && !au_portail()) || (b.h - hm).abs() > ECART_H {
                    continue;
                }
                if if dedans_ { b.bus || b.arete == usize::MAX } else { bus && !b.bus } {
                    continue;
                }
                if let Some((t0, t1)) = dedans(p0, p1, b) {
                    // Un autre passage ne coupe que dans le cercle du portail
                    let (t0, t1) = if b.passage != passage {
                        match b.portail.and_then(|n| self.cercles.get(&n)).and_then(|&(c, ray)| dans_cercle(p0, p1, c, ray)) {
                            Some((c0, c1)) => (t0.max(c0), t1.min(c1)),
                            None => continue,
                        }
                    } else {
                        (t0, t1)
                    };
                    if (t1 - t0) * l > 0.01 {
                        couverts.push((t0, t1));
                    }
                }
            }
            couverts.sort_by(|x, y| x.0.total_cmp(&y.0));
            // Les intervalles gardés : le complément (ou, à l'inverse, leur réunion)
            let mut gardes: Vec<(f32, f32)> = Vec::new();
            if dedans_ {
                for (t0, t1) in couverts {
                    match gardes.last_mut() {
                        Some(g) if t0 <= g.1 => g.1 = g.1.max(t1),
                        _ => gardes.push((t0, t1)),
                    }
                }
            } else {
                let mut t = 0.0f32;
                for (t0, t1) in couverts {
                    if t0 > t {
                        gardes.push((t, t0));
                    }
                    t = t.max(t1);
                }
                if t < 1.0 {
                    gardes.push((t, 1.0));
                }
            }
            if gardes.is_empty() {
                if cur.len() >= 2 {
                    sortie.push(std::mem::take(&mut cur));
                }
                cur.clear();
                ouvert = false;
                continue;
            }
            for &(t0, t1) in &gardes {
                if t0 <= 1e-5 && ouvert {
                    cur.push(at(t1));
                } else {
                    if cur.len() >= 2 {
                        sortie.push(std::mem::take(&mut cur));
                    }
                    cur = vec![at(t0), at(t1)];
                }
                ouvert = t1 >= 1.0 - 1e-5;
            }
            if !ouvert && cur.len() >= 2 {
                sortie.push(std::mem::take(&mut cur));
                cur.clear();
            }
        }
        if cur.len() >= 2 {
            sortie.push(cur);
        }
        sortie
    }
}


/// Les bords des chaussées des voies carrossables `aretes` : des polylignes. `axe`
/// donne l'axe de chaque arête (celui de ses files), sa demi-largeur en chaque point et
/// ses raccords aux fourches.
pub fn bords(r: &Reseau, aretes: &[usize], axe: &dyn Fn(usize) -> (Poly, Vec<f32>, [Option<crate::fils::Raccord>; 2])) -> Vec<Poly> {
    bords_et_bandes(r, aretes, axe).0
}

impl Bandes {
    /// Ce qui couvre le point `p` (x, z, hauteur) : les voies dont la bande le contient
    /// (« aN »), les joints de nœud, les morceaux près d'un portail (diagnostic).
    /// Le point `p` est sur un site propre de bus (de son niveau).
    pub fn sur_bus(&self, p: [f32; 3]) -> bool {
        let q = [p[0], p[1]];
        self.index.get(&case(p[0], p[1])).map(|x| x.as_slice()).unwrap_or(&[]).iter().any(|i| {
            let b = &self.bandes[*i as usize];
            b.bus && (b.h - p[2]).abs() <= ECART_H && dedans(q, q, b).is_some()
        })
    }

    pub fn couvrent(&self, p: [f32; 3]) -> Vec<String> {
        let q = [p[0], p[1]];
        let mut v: Vec<String> = Vec::new();
        for i in self.index.get(&case(p[0], p[1])).map(|x| x.as_slice()).unwrap_or(&[]) {
            let b = &self.bandes[*i as usize];
            if (b.h - p[2]).abs() > ECART_H || dedans(q, q, b).is_none() {
                continue;
            }
            let t = if b.arete == usize::MAX {
                "joint".to_string()
            } else if b.portail.is_some() {
                format!("a{} (portail)", b.arete)
            } else {
                format!("a{}", b.arete)
            };
            if !v.contains(&t) {
                v.push(t);
            }
        }
        v
    }
}

/// `bords`, et les bandes qui les ont coupés.
pub fn bords_et_bandes(r: &Reseau, aretes: &[usize], axe: &dyn Fn(usize) -> (Poly, Vec<f32>, [Option<crate::fils::Raccord>; 2])) -> (Vec<Poly>, Bandes) {
    let carrossable = |k: usize| {
        let a = &r.aretes[k];
        sage_reel::carrossable(a.genre) && a.files.total > 0 && a.points.len() >= 2 && !crate::fils::est_amorce(r, k)
    };
    let est_bus = |k: usize| r.aretes[k].genre == sage_reel::BUS;
    let utiles: Vec<usize> = aretes.iter().copied().filter(|&k| carrossable(k)).collect();
    // Les deux bords (gauche, droite du tracé) : le bord extérieur des files extrêmes,
    // posé aux raccords sur celui des files du tronc qu'elles continuent ; et les bandes
    // entre eux
    let mut cotes: HashMap<usize, [Poly; 2]> = HashMap::new();
    let mut infos: HashMap<usize, String> = HashMap::new();
    let mut bandes = Bandes { bandes: Vec::new(), index: HashMap::new(), noeuds: HashMap::new(), cercles: HashMap::new(), origines: Vec::new() };
    // Les nœuds où se rejoignent des voies de passages différents, et leur rayon (la plus
    // grande demi-largeur, plus une marge)
    let mut portails: HashMap<usize, f32> = HashMap::new();
    let mut axes: HashMap<usize, (Poly, Vec<f32>, [Option<crate::fils::Raccord>; 2])> = utiles.iter().map(|&k| (k, axe(k))).collect();
    {
        let mut vus: HashMap<usize, (Passage, bool, f32)> = HashMap::new();
        for &k in &utiles {
            let a = &r.aretes[k];
            for n in [a.a, a.b] {
                let e = vus.entry(n).or_insert((a.passage, false, 0.0));
                if e.0 != a.passage {
                    e.1 = true;
                }
                e.2 = e.2.max(a.largeur * 0.5);
            }
        }
        // Un portail qui est aussi une fourche : jusqu'aux cassures de ses branches (leurs
        // bords intérieurs s'y coupent jusqu'au nez)
        let mut fourches: HashMap<usize, f32> = HashMap::new();
        for &k in &utiles {
            let a = &r.aretes[k];
            for (i, rc) in axes[&k].2.iter().enumerate() {
                if let Some(rc) = rc {
                    let n = if i == 0 { a.a } else { a.b };
                    let d = rc.noeuds.iter().map(|x| x.distance).fold(0.0, f32::max);
                    let e = fourches.entry(n).or_insert(0.0);
                    *e = e.max(d);
                }
            }
        }
        // Les deux bords d'une voie (sans giration), comme plus bas
        let bords_de = |k: usize| -> [Poly; 2] {
            let (p, ws, raccords) = &axes[&k];
            let nf = r.aretes[k].files.total as usize;
            let g = crate::files::normales(p);
            let decale = |sg: f32| -> Poly { p.iter().zip(&g).zip(ws).map(|((q, g), w)| [q[0] + g[0] * w * sg, q[1] + g[1] * w * sg, q[2]]).collect() };
            let (mut gauche, mut droite) = (decale(1.0), decale(-1.0));
            crate::fils::raccorder_bouts(raccords, &mut gauche, |rc| rc.bout_bord(true, nf));
            crate::fils::raccorder_bouts(raccords, &mut droite, |rc| rc.bout_bord(false, nf));
            [gauche, droite]
        };
        // Le nez d'un portail-fourche : le croisement des bords de deux de ses branches le
        // plus loin du nœud, en deçà des cassures (au-delà, un chevauchement des données)
        let nez = |n: usize, limite: f32| -> Option<f32> {
            let c = [r.noeuds[n].p[0], r.noeuds[n].p[1]];
            let branches: Vec<usize> = utiles
                .iter()
                .copied()
                .filter(|&k| {
                    let a = &r.aretes[k];
                    (a.a == n && axes[&k].2[0].is_some()) || (a.b == n && axes[&k].2[1].is_some())
                })
                .collect();
            let bs: Vec<[Poly; 2]> = branches.iter().map(|&k| bords_de(k)).collect();
            let mut loin: Option<f32> = None;
            let proche = |q: &[f32; 3]| (q[0] - c[0]).hypot(q[1] - c[1]) <= limite + 1.0;
            for i in 0..bs.len() {
                for j in i + 1..bs.len() {
                    if r.aretes[branches[i]].passage == r.aretes[branches[j]].passage {
                        continue;
                    }
                    for l1 in &bs[i] {
                        for l2 in &bs[j] {
                            for s1 in l1.windows(2).filter(|s| proche(&s[0]) || proche(&s[1])) {
                                for s2 in l2.windows(2).filter(|s| proche(&s[0]) || proche(&s[1])) {
                                    if let Some(x) = croisement([s1[0][0], s1[0][1]], [s1[1][0], s1[1][1]], [s2[0][0], s2[0][1]], [s2[1][0], s2[1][1]]) {
                                        let d = (x[0] - c[0]).hypot(x[1] - c[1]);
                                        if d <= limite && loin.is_none_or(|v| d > v) {
                                            loin = Some(d);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            loin
        };
        for (n, (_, mixte, w)) in vus {
            if mixte {
                let ray = (w + 1.5).max(fourches.get(&n).and_then(|d| nez(n, d + w)).map_or(0.0, |d| d + 0.3));
                portails.insert(n, ray);
                bandes.cercles.insert(n, ([r.noeuds[n].p[0], r.noeuds[n].p[1]], ray));
            }
        }
    }
    // Le bout d'arête `k` (à sa fin si `fin`) est prolongé, au-delà de son nœud, par
    // une autre voie carrossable à moins de TOUT_DROIT : une continuation, simple ou
    // dans une fourche
    let direction = |k: usize, fin: bool| -> Option<[f32; 2]> {
        let q = &r.aretes[k].points;
        if q.len() < 2 {
            return None;
        }
        let it: Vec<[f32; 2]> = if fin { q.iter().rev().map(|p| [p[0], p[1]]).collect() } else { q.iter().map(|p| [p[0], p[1]]).collect() };
        let o = it[0];
        let mut s_ = it[1];
        let mut cumul = 0.0f32;
        for i in 1..it.len() {
            cumul += (it[i][0] - it[i - 1][0]).hypot(it[i][1] - it[i - 1][1]);
            s_ = it[i];
            if cumul >= 5.0 {
                break;
            }
        }
        let l = (s_[0] - o[0]).hypot(s_[1] - o[1]);
        (l > 1e-3).then(|| [(s_[0] - o[0]) / l, (s_[1] - o[1]) / l])
    };
    let prolongee = |k: usize, fin: bool| -> bool {
        let n = if fin { r.aretes[k].b } else { r.aretes[k].a };
        let Some(d) = direction(k, fin) else { return false };
        let cos = crate::fils::TOUT_DROIT.to_radians().cos();
        r.noeuds[n].aretes.iter().any(|&j| {
            let b = &r.aretes[j];
            if !sage_reel::carrossable(b.genre) || b.files.total == 0 {
                return false;
            }
            [false, true].into_iter().any(|f| {
                (if f { b.b } else { b.a }) == n && (j, f) != (k, fin) && direction(j, f).is_some_and(|e| -(d[0] * e[0] + d[1] * e[1]) >= cos)
            })
        })
    };
    for &k in &utiles {
        let (mut p, mut ws, raccords) = axes.remove(&k).unwrap();
        if p.len() < 2 || ws.len() != p.len() {
            continue;
        }
        // Le biseau dans l'alignement : la voie la plus large, prolongée tout droit par
        // une plus étroite (sans raccord de files), se rétrécit jusqu'à elle au nœud
        for (idx, fin) in [(0usize, false), (1usize, true)] {
            if raccords[idx].is_some() {
                continue;
            }
            let n = if fin { r.aretes[k].b } else { r.aretes[k].a };
            let Some(d) = direction(k, fin) else { continue };
            let wk = r.aretes[k].largeur * 0.5;
            let mut cible: Option<f32> = None;
            let mut alignees = 0usize;
            for &j in &r.noeuds[n].aretes {
                if !carrossable(j) || r.aretes[j].a == r.aretes[j].b {
                    continue;
                }
                for f in [false, true] {
                    if (j, f) == (k, fin) || (if f { r.aretes[j].b } else { r.aretes[j].a }) != n {
                        continue;
                    }
                    let Some(e) = direction(j, f) else { continue };
                    let wj = r.aretes[j].largeur * 0.5;
                    if -(d[0] * e[0] + d[1] * e[1]) >= BISEAU_ANGLE.to_radians().cos() {
                        alignees += 1;
                        if wk - wj > 0.3 {
                            cible = Some(wj);
                        }
                    }
                }
            }
            // Une seule voie dans l'alignement (deux : une fourche, ses branches posées
            // côte à côte sur le tronc, qui garde sa largeur)
            let Some(wt) = cible.filter(|_| alignees == 1) else { continue };
            // Le sens de parcours depuis le nœud
            if !fin {
                p.reverse();
                ws.reverse();
            }
            let m = p.len();
            let total: f32 = p.windows(2).map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1])).sum();
            let l = (BISEAU_PENTE * (wk - wt)).clamp(BISEAU_MIN, BISEAU_MIN.max(0.45 * total));
            // Un point à la longueur du biseau, pour qu'il ne s'étire pas jusqu'au
            // point suivant du tracé
            let mut cumul = 0.0f32;
            for i in (1..m).rev() {
                let sg = (p[i][0] - p[i - 1][0]).hypot(p[i][1] - p[i - 1][1]);
                if cumul + sg > l + 0.3 && cumul < l - 0.3 {
                    let t = (l - cumul) / sg;
                    let q = [p[i][0] + (p[i - 1][0] - p[i][0]) * t, p[i][1] + (p[i - 1][1] - p[i][1]) * t, p[i][2] + (p[i - 1][2] - p[i][2]) * t];
                    let wq = ws[i] + (ws[i - 1] - ws[i]) * t;
                    p.insert(i, q);
                    ws.insert(i, wq);
                    break;
                }
                cumul += sg;
                if cumul >= l {
                    break;
                }
            }
            let m = p.len();
            let mut s_ = 0.0f32;
            for i in (0..m).rev() {
                if i < m - 1 {
                    s_ += (p[i + 1][0] - p[i][0]).hypot(p[i + 1][1] - p[i][1]);
                }
                if s_ >= l {
                    break;
                }
                let u = s_ / l;
                let lisse = u * u * (3.0 - 2.0 * u);
                ws[i] = wt + (ws[i] - wt) * lisse;
            }
            if !fin {
                p.reverse();
                ws.reverse();
            }
        }
        bandes.noeuds.insert(k, [r.aretes[k].a, r.aretes[k].b]);
        let portail_de = |a: [f32; 3], b: [f32; 3]| -> Option<usize> {
            [r.aretes[k].a, r.aretes[k].b].into_iter().find(|n| {
                portails.get(n).is_some_and(|&ray| {
                    let c = r.noeuds[*n].p;
                    (a[0] - c[0]).hypot(a[1] - c[1]) <= ray || (b[0] - c[0]).hypot(b[1] - c[1]) <= ray
                })
            })
        };
        let n = r.aretes[k].files.total as usize;
        let g = crate::files::normales(&p);
        let decale = |sg: f32| -> Poly { p.iter().zip(&g).zip(&ws).map(|((q, g), w)| [q[0] + g[0] * w * sg, q[1] + g[1] * w * sg, q[2]]).collect() };
        let (mut gauche, mut droite) = (decale(1.0), decale(-1.0));
        crate::fils::raccorder_bouts(&raccords, &mut gauche, |rc| rc.bout_bord(true, n));
        crate::fils::raccorder_bouts(&raccords, &mut droite, |rc| rc.bout_bord(false, n));
        for i in 0..p.len() - 1 {
            let j = i + 1;
            // Rétrécie de EPS de chaque côté
            let serre = |a: [f32; 3], b: [f32; 3]| {
                let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
                let l = dx.hypot(dz);
                let f = if l > 2.0 * EPS { EPS / l } else { 0.5 };
                [a[0] + dx * f, a[1] + dz * f]
            };
            let mut q = [serre(gauche[i], droite[i]), serre(gauche[j], droite[j]), serre(droite[j], gauche[j]), serre(droite[i], gauche[i])];
            // Aux continuations (une autre voie la prolonge au-delà du nœud), la bande
            // dépasse le nœud de RECOUVRE : celle de la voie suivante la recouvre, sans fente entre
            // elles où un bord voisin survivrait
            let (dx, dz) = (p[j][0] - p[i][0], p[j][1] - p[i][1]);
            let ld = dx.hypot(dz);
            if ld > 1e-3 {
                let (ux, uz) = (dx / ld * RECOUVRE, dz / ld * RECOUVRE);
                if i == 0 && prolongee(k, false) {
                    for v in [0, 3] {
                        q[v] = [q[v][0] - ux, q[v][1] - uz];
                    }
                }
                if j == p.len() - 1 && prolongee(k, true) {
                    for v in [1, 2] {
                        q[v] = [q[v][0] + ux, q[v][1] + uz];
                    }
                }
            }
            let Some(sens) = convexe(&q) else { continue };
            let id = bandes.bandes.len() as u32;
            let (x0, z0) = q.iter().fold((f32::MAX, f32::MAX), |m, a| (m.0.min(a[0]), m.1.min(a[1])));
            let (x1, z1) = q.iter().fold((f32::MIN, f32::MIN), |m, a| (m.0.max(a[0]), m.1.max(a[1])));
            let (a, z) = (case(x0, z0), case(x1, z1));
            for ci in a.0..=z.0 {
                for cj in a.1..=z.1 {
                    bandes.index.entry((ci, cj)).or_default().push(id);
                }
            }
            bandes.bandes.push(Bande { arete: k, exclues: Vec::new(), portail: portail_de(p[i], p[j]), passage: r.aretes[k].passage, bus: est_bus(k), q, sens, h: (p[i][2] + p[j][2]) / 2.0 });
        }
        // Les girations (deux voies bout à bout en angle fort) : coin ou arc, après
        // les bandes
        let gs = crate::fils::girations(r, k, &p, &raccords);
        crate::fils::girer(&gs, &mut gauche);
        crate::fils::girer(&gs, &mut droite);
        cotes.insert(k, [gauche, droite]);
        {
            let mut t = Vec::new();
            for (i, nom) in ["début", "fin"].iter().enumerate() {
                if let Some(rc) = &raccords[i] {
                    t.push(format!("raccord {nom} ({}{})", rc.regle, if rc.entier { ", entier" } else { "" }));
                }
                if gs[i].is_some() {
                    t.push(format!("giration {nom}"));
                }
            }
            infos.insert(k, t.join(", "));
        }
    }
    // Les joints : à chaque nœud, l'enveloppe des bouts des bandes qui y arrivent (de
    // même niveau). Elle comble les coins vides entre deux bandes coupées d'équerre qui
    // font un angle ; un bord qui les traverserait y est coupé.
    {
        let mut aux: HashMap<(usize, Passage), (Vec<[f32; 2]>, f32, Vec<usize>, bool, Passage)> = HashMap::new();
        for &k in &utiles {
            let Some([g, d]) = cotes.get(&k) else { continue };
            // Un bord effacé par une giration (un petit tronçon traversé)
            if g.is_empty() || d.is_empty() {
                continue;
            }
            let a = &r.aretes[k];
            // Les deux bords n'ont pas toujours autant de points (girations)
            for (n, fin) in [(a.a, false), (a.b, true)] {
                let (gi, di) = if fin { (g[g.len() - 1], d[d.len() - 1]) } else { (g[0], d[0]) };
                let e = aux.entry((n, a.passage)).or_insert((Vec::new(), 0.0, Vec::new(), true, a.passage));
                e.0.push([gi[0], gi[1]]);
                e.0.push([di[0], di[1]]);
                e.1 += (gi[2] + di[2]) / 2.0;
                e.2.push(k);
                e.3 &= est_bus(k);
            }
        }
        for (_, (pts, hs, voies, bus, passage)) in aux {
            let nb = voies.len();
            if nb < 2 {
                continue;
            }
            let h = hs / nb as f32;
            // L'enveloppe convexe (chaîne monotone), dans le sens direct
            let mut v = pts.clone();
            v.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
            v.dedup_by(|a, b| (a[0] - b[0]).hypot(a[1] - b[1]) < 1e-4);
            if v.len() < 3 {
                continue;
            }
            let cross = |o: [f32; 2], a: [f32; 2], b: [f32; 2]| (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0]);
            let mut env: Vec<[f32; 2]> = Vec::new();
            for pass in 0..2 {
                let debut = env.len();
                let it: Vec<[f32; 2]> = if pass == 0 { v.clone() } else { v.iter().rev().copied().collect() };
                for q in it {
                    while env.len() >= debut + 2 && cross(env[env.len() - 2], env[env.len() - 1], q) <= 1e-6 {
                        env.pop();
                    }
                    env.push(q);
                }
                env.pop();
            }
            if env.len() < 3 {
                continue;
            }
            // Rétrécie de EPS vers son centre
            let c = env.iter().fold([0.0f32; 2], |s, q| [s[0] + q[0] / env.len() as f32, s[1] + q[1] / env.len() as f32]);
            let env: Vec<[f32; 2]> = env
                .iter()
                .map(|q| {
                    let (dx, dz) = (c[0] - q[0], c[1] - q[1]);
                    let l = dx.hypot(dz);
                    if l > 2.0 * EPS { [q[0] + dx * EPS / l, q[1] + dz * EPS / l] } else { *q }
                })
                .collect();
            // En quadrilatères en éventail depuis le premier point ; le dernier, s'il ne
            // reste qu'un triangle, avec le milieu d'un côté
            let mut i = 1;
            while i + 1 < env.len() {
                let q = if i + 2 < env.len() {
                    [env[0], env[i], env[i + 1], env[i + 2]]
                } else {
                    let m = [(env[i][0] + env[i + 1][0]) / 2.0, (env[i][1] + env[i + 1][1]) / 2.0];
                    [env[0], env[i], m, env[i + 1]]
                };
                i += 2;
                let Some(sens) = convexe(&q) else { continue };
                let id = bandes.bandes.len() as u32;
                let (x0, z0) = q.iter().fold((f32::MAX, f32::MAX), |m, a| (m.0.min(a[0]), m.1.min(a[1])));
                let (x1, z1) = q.iter().fold((f32::MIN, f32::MIN), |m, a| (m.0.max(a[0]), m.1.max(a[1])));
                let (a, z) = (case(x0, z0), case(x1, z1));
                for ci in a.0..=z.0 {
                    for cj in a.1..=z.1 {
                        bandes.index.entry((ci, cj)).or_default().push(id);
                    }
                }
                bandes.bandes.push(Bande { arete: usize::MAX, exclues: voies.clone(), portail: None, passage, bus, q, sens, h });
            }
        }
    }
    let mut sortie: Vec<Poly> = Vec::new();
    let mut origines: Vec<String> = Vec::new();
    let origine = |k: usize, quoi: &str| {
        let i = infos.get(&k).map(|s| s.as_str()).unwrap_or("");
        if i.is_empty() { format!("a{k} {quoi}") } else { format!("a{k} {quoi} · {i}") }
    };
    // Les bords, hors des bandes des autres voies
    for &k in &utiles {
        let Some(c) = cotes.get(&k) else { continue };
        for (cote, p) in c.iter().enumerate() {
            let nom = if cote == 0 { "gauche" } else { "droit" };
            // Un bord tout entier dans une giration (un petit tronçon traversé)
            if p.len() < 2 {
                continue;
            }
            let v = bandes.garder(p, r.aretes[k].passage, est_bus(k), &[k]);
            origines.extend(v.iter().map(|_| origine(k, &format!("bord {nom}"))));
            sortie.extend(v);
            // Une chaussée qui rencontre une voie de bus se ferme sur elle-même, le long
            // du bord de la voie de bus (en travers de ses files)
            if est_bus(k) {
                let v = bandes.decouper(p, r.aretes[k].passage, true, &[k], true);
                origines.extend(v.iter().map(|_| origine(k, &format!("fermeture bus {nom}"))));
                sortie.extend(v);
            }
        }
    }
    let mut noeuds: Vec<usize> = Vec::new();
    for &k in &utiles {
        noeuds.push(r.aretes[k].a);
        noeuds.push(r.aretes[k].b);
    }
    noeuds.sort_unstable();
    noeuds.dedup();
    // Les micro coupures : près d'un nœud, deux bouts libres de bords à moins de
    // SOUDURE l'un de l'autre (un coude, un changement de largeur) sont confondus en
    // leur milieu, les plus proches d'abord
    let mut deplacements: Vec<(usize, usize, [f32; 3])> = Vec::new();
    {
        let mut pris: std::collections::HashSet<(usize, usize)> = std::collections::HashSet::new();
        // D'abord, partout, les bouts presque confondus (deux bords qui se coupent en un
        // coin, à l'écart près des bandes rétrécies)
        {
            let mut grille: HashMap<(i32, i32), Vec<(usize, usize)>> = HashMap::new();
            for (i, l) in sortie.iter().enumerate() {
                for (j, q) in [(0, l[0]), (1, l[l.len() - 1])] {
                    grille.entry(((q[0] / 0.5).floor() as i32, (q[1] / 0.5).floor() as i32)).or_default().push((i, j));
                }
            }
            let bout = |(i, j): (usize, usize)| if j == 0 { sortie[i][0] } else { sortie[i][sortie[i].len() - 1] };
            let mut paires: Vec<(f32, (usize, usize), (usize, usize))> = Vec::new();
            for (&(gx, gz), v) in &grille {
                for &a in v {
                    for dx in -1..=1 {
                        for dz in -1..=1 {
                            for &b in grille.get(&(gx + dx, gz + dz)).map(|w| w.as_slice()).unwrap_or(&[]) {
                                if a < b && a.0 != b.0 {
                                    let (p, q) = (bout(a), bout(b));
                                    let d = (p[0] - q[0]).hypot(p[1] - q[1]);
                                    if d <= SOUDURE_FINE && (p[2] - q[2]).abs() <= ECART_H {
                                        paires.push((d, a, b));
                                    }
                                }
                            }
                        }
                    }
                }
            }
            paires.sort_by(|u, v| u.0.total_cmp(&v.0));
            for (d, a, b) in paires {
                if pris.contains(&a) || pris.contains(&b) {
                    continue;
                }
                pris.insert(a);
                pris.insert(b);
                if d > 0.001 {
                    let (p, q) = (bout(a), bout(b));
                    let m = [(p[0] + q[0]) / 2.0, (p[1] + q[1]) / 2.0, (p[2] + q[2]) / 2.0];
                    deplacements.push((a.0, a.1, m));
                    deplacements.push((b.0, b.1, m));
                }
            }
        }
        for &n in &noeuds {
            let c = r.noeuds[n].p;
            let rayon = r.noeuds[n].aretes.iter().map(|&k| r.aretes[k].largeur * 0.5).fold(0.0f32, f32::max) + 1.5;
            let mut bouts: Vec<(usize, usize, [f32; 3])> = Vec::new();
            for (i, l) in sortie.iter().enumerate() {
                for (j, q) in [(0, l[0]), (1, l[l.len() - 1])] {
                    if (q[0] - c[0]).hypot(q[1] - c[1]) <= rayon && !pris.contains(&(i, j)) {
                        bouts.push((i, j, q));
                    }
                }
            }
            let mut paires: Vec<(f32, usize, usize)> = Vec::new();
            for x in 0..bouts.len() {
                for y in x + 1..bouts.len() {
                    let (a, b) = (bouts[x], bouts[y]);
                    if a.0 == b.0 || (a.2[2] - b.2[2]).abs() > ECART_H {
                        continue;
                    }
                    let d = (a.2[0] - b.2[0]).hypot(a.2[1] - b.2[1]);
                    if d <= SOUDURE {
                        paires.push((d, x, y));
                    }
                }
            }
            paires.sort_by(|u, v| u.0.total_cmp(&v.0));
            for (d, x, y) in paires {
                let (a, b) = (bouts[x], bouts[y]);
                if pris.contains(&(a.0, a.1)) || pris.contains(&(b.0, b.1)) {
                    continue;
                }
                pris.insert((a.0, a.1));
                pris.insert((b.0, b.1));
                if d > 0.001 {
                    let m = [(a.2[0] + b.2[0]) / 2.0, (a.2[1] + b.2[1]) / 2.0, (a.2[2] + b.2[2]) / 2.0];
                    deplacements.push((a.0, a.1, m));
                    deplacements.push((b.0, b.1, m));
                }
            }
        }
    }
    for (i, j, m) in deplacements {
        let l = &mut sortie[i];
        let k = if j == 0 { 0 } else { l.len() - 1 };
        l[k] = m;
    }
    // Les impasses : leurs deux bords reliés en travers
    for n in noeuds {
        let mut bouts: Vec<(usize, bool)> = Vec::new();
        for &k in &r.noeuds[n].aretes {
            if !cotes.contains_key(&k) {
                continue;
            }
            let a = &r.aretes[k];
            for debut in [true, false] {
                if (if debut { a.a } else { a.b }) == n {
                    bouts.push((k, debut));
                }
            }
        }
        // Une voie de bus qui finit sur une rue n'est pas une impasse, ni l'inverse
        if bouts.len() != 1 || r.noeuds[n].aretes.iter().any(|&k| !cotes.contains_key(&k) && carrossable(k)) {
            continue;
        }
        let (k, debut) = bouts[0];
        let c = &cotes[&k];
        if c[0].is_empty() || c[1].is_empty() {
            continue;
        }
        let bout = |p: &Poly| if debut { p[0] } else { p[p.len() - 1] };
        let v = bandes.garder(&[bout(&c[0]), bout(&c[1])], r.aretes[k].passage, est_bus(k), &[k]);
        origines.extend(v.iter().map(|_| origine(k, "impasse")));
        sortie.extend(v);
    }
    bandes.origines = origines;
    (sortie, bandes)
}
