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
//! une impasse est fermée d'un trait en travers.
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
/// La maille (m) de l'index des bandes.
const CASE: f32 = 10.0;
/// Une bande ne coupe un bord que s'ils sont à moins de tant de mètres en hauteur (un
/// pont ne coupe pas la rue qu'il enjambe).
const ECART_H: f32 = 2.0;

type Poly = Vec<[f32; 3]>;

/// Un morceau de bande : le quadrilatère d'un segment de voie.
struct Bande {
    arete: usize,
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
struct Bandes {
    bandes: Vec<Bande>,
    index: HashMap<(i32, i32), Vec<u32>>,
}

impl Bandes {
    /// Les morceaux de `p` hors des bandes (sauf celles des arêtes `sauf`) de son niveau.
    /// Le bord d'une voie de bus (`bus`) n'est coupé que par les voies de bus.
    fn garder(&self, p: &[[f32; 3]], passage: Passage, bus: bool, sauf: &[usize]) -> Vec<Poly> {
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
                if sauf.contains(&b.arete) || b.passage != passage || (bus && !b.bus) || (b.h - hm).abs() > ECART_H {
                    continue;
                }
                if let Some((t0, t1)) = dedans(p0, p1, b) {
                    if (t1 - t0) * l > 0.01 {
                        couverts.push((t0, t1));
                    }
                }
            }
            couverts.sort_by(|x, y| x.0.total_cmp(&y.0));
            // Les intervalles gardés : le complément
            let mut gardes: Vec<(f32, f32)> = Vec::new();
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
    let carrossable = |k: usize| {
        let a = &r.aretes[k];
        sage_reel::carrossable(a.genre) && a.files.total > 0 && a.points.len() >= 2
    };
    let est_bus = |k: usize| r.aretes[k].genre == sage_reel::BUS;
    let utiles: Vec<usize> = aretes.iter().copied().filter(|&k| carrossable(k)).collect();
    // Les deux bords (gauche, droite du tracé) : le bord extérieur des files extrêmes,
    // posé aux raccords sur celui des files du tronc qu'elles continuent ; et les bandes
    // entre eux
    let mut cotes: HashMap<usize, [Poly; 2]> = HashMap::new();
    let mut bandes = Bandes { bandes: Vec::new(), index: HashMap::new() };
    for &k in &utiles {
        let (p, ws, raccords) = axe(k);
        if p.len() < 2 || ws.len() != p.len() {
            continue;
        }
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
            let q = [serre(gauche[i], droite[i]), serre(gauche[j], droite[j]), serre(droite[j], gauche[j]), serre(droite[i], gauche[i])];
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
            bandes.bandes.push(Bande { arete: k, passage: r.aretes[k].passage, bus: est_bus(k), q, sens, h: (p[i][2] + p[j][2]) / 2.0 });
        }
        cotes.insert(k, [gauche, droite]);
    }
    let mut sortie: Vec<Poly> = Vec::new();
    // Les bords, hors des bandes des autres voies
    for &k in &utiles {
        let Some(c) = cotes.get(&k) else { continue };
        for p in c {
            sortie.extend(bandes.garder(p, r.aretes[k].passage, est_bus(k), &[k]));
        }
    }
    // Les impasses : leurs deux bords reliés en travers (aux autres nœuds, les bords des
    // files se rejoignent d'eux-mêmes)
    let mut noeuds: Vec<usize> = Vec::new();
    for &k in &utiles {
        noeuds.push(r.aretes[k].a);
        noeuds.push(r.aretes[k].b);
    }
    noeuds.sort_unstable();
    noeuds.dedup();
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
        let bout = |p: &Poly| if debut { p[0] } else { p[p.len() - 1] };
        sortie.extend(bandes.garder(&[bout(&c[0]), bout(&c[1])], r.aretes[k].passage, est_bus(k), &[k]));
    }
    sortie
}
