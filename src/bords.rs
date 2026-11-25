//! Les bords des chaussées, tirés des voies et de leurs files seulement (pas de la
//! chaussée matérialisée) : le contour de l'union des chaussées des voies carrossables.
//!
//! Chaque voie a une bande : son axe (déporté près d'une fourche, comme les files)
//! élargi d'une demi-largeur de chaque côté. Ses deux bords sont gardés là où ils ne
//! sont dans la bande d'aucune autre voie du même niveau : aux carrefours, les bords
//! se coupent d'eux-mêmes en coins, ceux des courtes voies internes d'un carrefour de
//! nœuds proches disparaissent, et ce qui reste se lie en boucles (autour d'un pâté de
//! maisons). Deux compléments aux nœuds :
//! - un côté ouvert de plus d'un demi-tour (l'extérieur d'un coude, le côté plat d'un
//!   T) : les bords voisins sont reliés d'un trait (lui aussi gardé hors des bandes) ;
//! - une impasse : ses deux bords sont reliés en travers.
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

/// On cherche le croisement de deux bords sur au plus tant de mètres depuis le nœud.
const PORTEE: f32 = 40.0;

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

/// Deux bords partant d'un nœud se croisent-ils à moins de `PORTEE` mètres ?
fn croisent(p: &[[f32; 3]], q: &[[f32; 3]]) -> bool {
    let mut sp = 0.0;
    for a in p.windows(2) {
        if sp > PORTEE {
            break;
        }
        sp += (a[1][0] - a[0][0]).hypot(a[1][1] - a[0][1]);
        let mut sq = 0.0;
        for b in q.windows(2) {
            if sq > PORTEE {
                break;
            }
            sq += (b[1][0] - b[0][0]).hypot(b[1][1] - b[0][1]);
            let (r, s) = ([a[1][0] - a[0][0], a[1][1] - a[0][1]], [b[1][0] - b[0][0], b[1][1] - b[0][1]]);
            let den = r[0] * s[1] - r[1] * s[0];
            if den.abs() < 1e-9 {
                continue;
            }
            let (qx, qz) = (b[0][0] - a[0][0], b[0][1] - a[0][1]);
            let t = (qx * s[1] - qz * s[0]) / den;
            let u = (qx * r[1] - qz * r[0]) / den;
            if (0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u) {
                return true;
            }
        }
    }
    false
}

/// Les bords des chaussées des voies carrossables `aretes`, l'axe de chaque arête donné
/// par `axe` : des polylignes.
/// L'axe donne aussi la demi-largeur de la voie en chaque point.
pub fn bords(r: &Reseau, aretes: &[usize], axe: &dyn Fn(usize) -> (Poly, Vec<f32>)) -> Vec<Poly> {
    let carrossable = |k: usize| {
        let a = &r.aretes[k];
        sage_reel::carrossable(a.genre) && a.files.total > 0 && a.points.len() >= 2
    };
    let est_bus = |k: usize| r.aretes[k].genre == sage_reel::BUS;
    let utiles: Vec<usize> = aretes.iter().copied().filter(|&k| carrossable(k)).collect();
    // Les axes, les deux bords (gauche, droite du tracé) et les bandes
    let mut cotes: HashMap<usize, [Poly; 2]> = HashMap::new();
    let mut bandes = Bandes { bandes: Vec::new(), index: HashMap::new() };
    for &k in &utiles {
        let (p, ws) = axe(k);
        if p.len() < 2 || ws.len() != p.len() {
            continue;
        }
        let g = crate::files::normales(&p);
        let decale = |sg: f32| -> Poly { p.iter().zip(&g).zip(&ws).map(|((q, g), w)| [q[0] + g[0] * w * sg, q[1] + g[1] * w * sg, q[2]]).collect() };
        cotes.insert(k, [decale(1.0), decale(-1.0)]);
        let v = |i: usize| (ws[i] - EPS).max(0.0);
        for i in 0..p.len() - 1 {
            let j = i + 1;
            let (vi, vj) = (v(i), v(j));
            let q = [
                [p[i][0] + g[i][0] * vi, p[i][1] + g[i][1] * vi],
                [p[j][0] + g[j][0] * vj, p[j][1] + g[j][1] * vj],
                [p[j][0] - g[j][0] * vj, p[j][1] - g[j][1] * vj],
                [p[i][0] - g[i][0] * vi, p[i][1] - g[i][1] * vi],
            ];
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
    }
    let mut sortie: Vec<Poly> = Vec::new();
    // Les bords, hors des bandes des autres voies
    for &k in &utiles {
        let Some(c) = cotes.get(&k) else { continue };
        for p in c {
            sortie.extend(bandes.garder(p, r.aretes[k].passage, est_bus(k), &[k]));
        }
    }
    // Les liens aux nœuds : côtés ouverts et impasses
    let mut noeuds: Vec<usize> = Vec::new();
    for &k in &utiles {
        noeuds.push(r.aretes[k].a);
        noeuds.push(r.aretes[k].b);
    }
    noeuds.sort_unstable();
    noeuds.dedup();
    for (n, bus) in noeuds.into_iter().flat_map(|n| [(n, false), (n, true)]) {
        // Les bouts de cette sorte : (arête, au début, angle en s'éloignant du nœud)
        let mut bouts: Vec<(usize, bool, f32)> = Vec::new();
        for &k in &r.noeuds[n].aretes {
            if est_bus(k) != bus {
                continue;
            }
            let Some(c) = cotes.get(&k) else { continue };
            let a = &r.aretes[k];
            for debut in [true, false] {
                if (if debut { a.a } else { a.b }) != n {
                    continue;
                }
                let p = &c[0];
                let (o, s) = if debut { (p[0], p[1]) } else { (p[p.len() - 1], p[p.len() - 2]) };
                bouts.push((k, debut, (s[1] - o[1]).atan2(s[0] - o[0])));
            }
        }
        if bouts.is_empty() {
            continue;
        }
        bouts.sort_by(|x, y| x.2.total_cmp(&y.2));
        // Le bout d'un bord au nœud
        let bout = |k: usize, debut: bool, c: usize| {
            let p = &cotes[&k][c];
            if debut {
                (p[0], p[1])
            } else {
                (p[p.len() - 1], p[p.len() - 2])
            }
        };
        if bouts.len() == 1 {
            let (k, debut, _) = bouts[0];
            let (g, d) = (bout(k, debut, 0).0, bout(k, debut, 1).0);
            sortie.extend(bandes.garder(&[g, d], r.aretes[k].passage, bus, &[k]));
            continue;
        }
        let m = bouts.len();
        let c = r.noeuds[n].p;
        for i in 0..m {
            let (ki, di, ai) = bouts[i];
            let (kj, dj, aj) = bouts[(i + 1) % m];
            let mut ouverture = aj - ai;
            if ouverture <= 0.0 {
                ouverture += std::f32::consts::TAU;
            }
            let ouvert = ouverture >= std::f32::consts::PI - 0.05;
            let bis = ai + ouverture / 2.0;
            let bv = [bis.cos(), bis.sin()];
            // Le bord de chaque bout tourné vers le secteur : (côté, son bout au nœud)
            let vers = |k: usize, debut: bool| {
                let (g, d) = (bout(k, debut, 0), bout(k, debut, 1));
                let pos = |(_, s): ([f32; 3], [f32; 3])| (s[0] - c[0]) * bv[0] + (s[1] - c[1]) * bv[1];
                if pos(g) >= pos(d) {
                    (0, g.0)
                } else {
                    (1, d.0)
                }
            };
            let ((si, pi), (sj, pj)) = (vers(ki, di), vers(kj, dj));
            if (pi[0] - pj[0]).hypot(pi[1] - pj[1]) <= 0.05 {
                continue;
            }
            if ouvert {
                sortie.extend(bandes.garder(&[pi, pj], r.aretes[ki].passage, bus, &[ki, kj]));
                continue;
            }
            // Un angle : les deux bords se croisent d'habitude (le coin se fait seul) ; s'ils
            // ne se croisent pas (des voies qui se rejoignent côte à côte, l'entre-deux d'une
            // fourche de sens unique), leurs bouts sont reliés
            let depuis = |k: usize, debut: bool, cote: usize| -> Poly {
                let p = &cotes[&k][cote];
                if debut {
                    p.clone()
                } else {
                    p.iter().rev().copied().collect()
                }
            };
            if !croisent(&depuis(ki, di, si), &depuis(kj, dj, sj)) {
                let tous: Vec<usize> = bouts.iter().map(|b| b.0).collect();
                sortie.extend(bandes.garder(&[pi, pj], r.aretes[ki].passage, bus, &tous));
            }
        }
    }
    sortie
}
