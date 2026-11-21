//! Les carrefours simples d'une seule pièce (0.140) : un nœud où se rejoignent au moins
//! trois routes au sol, assez loin des autres carrefours pour que leurs bras aient de
//! la place. Au lieu des rubans prolongés qui s'y recouvrent et d'une dalle octogonale
//! (des éclats, des aiguilles et des petits murs où les volumes se chevauchent presque
//! à la même hauteur, des rectangles en saillie au bout des rubans, des dents de scie
//! dans les coins), le carrefour est une plaque :
//! - son contour : les bras triés autour du nœud ; chacun coupé net à sa bouche, une
//!   section en travers à `s` mètres du nœud ; entre deux bras, une bordure arrondie de
//!   rayon `BORDURE` (raccourcie s'il le faut), ou le tour du nœud du côté où les bras
//!   s'ouvrent de plus d'un demi-tour ;
//! - les rubans des bras s'arrêtent à leur bouche, sur les mêmes sommets que le
//!   contour (`Bouche`) : la plaque et le ruban se touchent par une face commune ;
//! - le dessus est maillé en anneaux autour du nœud (`materialisation::plaque`), aux
//!   hauteurs du champ de la voirie.
//!
//! Les groupes de nœuds proches (bras trop courts) gardent les rubans et les dalles.

use crate::champ_voirie::Chaine;
use crate::materialisation::{au_sol, plan, Plan, Pt};
use sage_reel::reseau::{Passage, Reseau};
use std::collections::HashMap;
use std::f32::consts::{PI, TAU};

/// Le rayon (m) des bordures arrondies entre deux bras.
pub const BORDURE: f32 = 3.0;
/// La bouche d'un bras : au moins à `S_MIN`, au plus à `S_MAX` mètres du nœud.
pub(crate) const S_MIN: f32 = 2.0;
pub(crate) const S_MAX: f32 = 20.0;
/// Ce que la bouche laisse au-delà de la bordure (m).
pub(crate) const MARGE: f32 = 0.5;
/// Ce qu'il doit rester d'une chaîne entre ses coupes (m).
pub(crate) const RESTE: f32 = 3.0;
/// Le plus grand écart (m) entre deux points du contour (hors bouches).
pub const PAS: f32 = 2.0;
/// Le pas d'angle des arcs.
const PAS_ANGLE: f32 = 15.0 * PI / 180.0;
/// Un angle entre deux bras plus ouvert que ça est un demi-tour (le côté plat d'un T).
const DEMI_TOUR: f32 = PI - 0.05;

/// Le bout d'une chaîne coupé à la bouche d'un carrefour : le point de tracé de la
/// coupe et la section du ruban à cet endroit (bord gauche, axe, bord droit, dans le
/// sens de parcours de la chaîne).
#[derive(Clone, Copy, Debug)]
pub struct Bouche {
    pub q: Pt,
    pub g: [f32; 2],
    pub m: [f32; 2],
    pub d: [f32; 2],
}

/// Un carrefour simple : son nœud, son plan, son contour (sens direct autour du
/// nœud) et ses bouches : `contour[k..k + 3]` est la section de la bouche (bord, axe,
/// bord).
pub struct Carrefour {
    /// Ses nœuds (un seul pour un carrefour simple, tous ceux d'un groupe).
    pub noeuds: Vec<usize>,
    pub c: [f32; 2],
    /// Les plans de ses nœuds : un point lit le plus proche.
    pub(crate) plans: Vec<Plan>,
    pub contour: Vec<[f32; 2]>,
    /// Les trous de la plaque (l'îlot d'un giratoire), en sens indirect.
    pub trous: Vec<Vec<[f32; 2]>>,
    pub bouches: Vec<(usize, Bouche)>,
    pub boite: [f32; 4],
}

impl Carrefour {
    /// Le plan du nœud le plus proche de (x, z).
    pub(crate) fn plan(&self, x: f32, z: f32) -> &Plan {
        self.plans.iter().min_by(|a, b| (a.c[0] - x).hypot(a.c[1] - z).total_cmp(&(b.c[0] - x).hypot(b.c[1] - z))).unwrap()
    }
}

/// Un bras d'un carrefour : la chaîne, si elle part du nœud (son début), ses points
/// vers l'extérieur et leurs abscisses, sa direction au nœud, sa demi-largeur, son
/// angle et sa bouche.
pub(crate) struct Bras {
    pub(crate) k: usize,
    pub(crate) debut: bool,
    pub(crate) pts: Vec<Pt>,
    pub(crate) s0: Vec<f32>,
    pub(crate) u: [f32; 2],
    pub(crate) w: f32,
    pub(crate) ang: f32,
    pub(crate) s: f32,
}

pub(crate) fn norm(v: [f32; 2]) -> [f32; 2] {
    let l = v[0].hypot(v[1]).max(1e-9);
    [v[0] / l, v[1] / l]
}
pub(crate) fn gauche(d: [f32; 2]) -> [f32; 2] {
    [-d[1], d[0]]
}
pub(crate) fn droite(d: [f32; 2]) -> [f32; 2] {
    [d[1], -d[0]]
}
pub(crate) fn plus(a: [f32; 2], b: [f32; 2], t: f32) -> [f32; 2] {
    [a[0] + b[0] * t, a[1] + b[1] * t]
}
pub(crate) fn dist(a: [f32; 2], b: [f32; 2]) -> f32 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}
/// L'angle de `b` après `a`, dans [0, 2π).
fn apres(a: f32, b: f32) -> f32 {
    (b - a).rem_euclid(TAU)
}

/// Les droites p + d·a et q + e·b se croisent en (a, b).
fn croiser(p: [f32; 2], d: [f32; 2], q: [f32; 2], e: [f32; 2]) -> Option<(f32, f32)> {
    let det = e[0] * d[1] - d[0] * e[1];
    if det.abs() < 1e-6 {
        return None;
    }
    let r = [q[0] - p[0], q[1] - p[1]];
    Some(((e[0] * r[1] - r[0] * e[1]) / det, (d[0] * r[1] - d[1] * r[0]) / det))
}

impl Bras {
    pub(crate) fn longueur(&self) -> f32 {
        self.s0[self.s0.len() - 1]
    }

    /// Le point de tracé à l'abscisse `s` et la direction (vers l'extérieur) du
    /// segment qui le porte.
    pub(crate) fn a(&self, s: f32) -> (Pt, [f32; 2]) {
        let j = self.s0.partition_point(|&x| x <= s).clamp(1, self.pts.len() - 1);
        let (a, b) = (&self.pts[j - 1], &self.pts[j]);
        let t = ((s - self.s0[j - 1]) / (self.s0[j] - self.s0[j - 1]).max(1e-9)).clamp(0.0, 1.0);
        let mut q: Pt = std::array::from_fn(|c| a[c] + t * (b[c] - a[c]));
        q[5] = 0.0;
        (q, norm([b[0] - a[0], b[1] - a[1]]))
    }
}

/// Les carrefours simples du réseau et, pour chaque chaîne (`voies`, de nœuds
/// `bouts` : début, fin), ses coupes : (abscisse depuis le bout, bouche) au début et à
/// la fin.
#[allow(clippy::type_complexity)]
pub fn construire(r: &Reseau, voies: &[Chaine], bouts: &[(usize, usize)]) -> (Vec<Carrefour>, Vec<[Option<(f32, Bouche)>; 2]>, Vec<bool>) {
    let mut coupes: Vec<[Option<(f32, Bouche)>; 2]> = vec![[None, None]; voies.len()];
    // Les bouts des chaînes à chaque nœud
    let mut aux: HashMap<usize, Vec<(usize, bool)>> = HashMap::new();
    for (k, &(a, b)) in bouts.iter().enumerate() {
        aux.entry(a).or_default().push((k, true));
        aux.entry(b).or_default().push((k, false));
    }
    // 0. Les groupes de nœuds proches, d'abord
    let mut carrefours = Vec::new();
    let mut absorbees = vec![false; voies.len()];
    let en_groupe = crate::carrefours_groupes::construire(r, voies, bouts, &aux, &mut coupes, &mut absorbees, &mut carrefours);
    // 1. Les candidats et leurs bras, les bouches posées
    let mut candidats: HashMap<usize, Vec<Bras>> = HashMap::new();
    'noeuds: for (&n, bouts_n) in &aux {
        if en_groupe.contains(&n) {
            continue;
        }
        let routes: Vec<usize> = r.noeuds[n].aretes.iter().copied().filter(|&k| r.aretes[k].genre != sage_reel::PIETONNE).collect();
        if routes.len() < 3 || bouts_n.len() != routes.len() || routes.iter().any(|&k| r.aretes[k].passage != Passage::Sol) {
            continue;
        }
        let c = [r.noeuds[n].p[0], r.noeuds[n].p[1]];
        let mut bras = Vec::new();
        for &(k, debut) in bouts_n {
            let v = &voies[k];
            let pts: Vec<Pt> = if debut { v.points.clone() } else { v.points.iter().rev().copied().collect() };
            if pts.len() < 2 || dist([pts[0][0], pts[0][1]], c) > 0.5 {
                continue 'noeuds;
            }
            let mut s0 = vec![0.0f32; pts.len()];
            for i in 1..pts.len() {
                s0[i] = s0[i - 1] + (pts[i][0] - pts[i - 1][0]).hypot(pts[i][1] - pts[i - 1][1]);
            }
            let l = s0[s0.len() - 1];
            if l < S_MIN + RESTE {
                continue 'noeuds;
            }
            let w = pts[0][6];
            let mut b = Bras { k, debut, pts, s0, u: [1.0, 0.0], w, ang: 0.0, s: S_MIN };
            let (q, _) = b.a(6.0f32.min(l));
            b.u = norm([q[0] - c[0], q[1] - c[1]]);
            b.ang = b.u[1].atan2(b.u[0]);
            bras.push(b);
        }
        bras.sort_by(|a, b| a.ang.total_cmp(&b.ang));
        let m = bras.len();
        // Les bouches : au-delà des bordures des deux coins de chaque bras
        for i in 0..m {
            let j = (i + 1) % m;
            let theta = apres(bras[i].ang, bras[j].ang);
            if theta >= DEMI_TOUR || theta < 1e-3 {
                continue;
            }
            let (bi, bj) = (&bras[i], &bras[j]);
            let pi = plus(c, gauche(bi.u), bi.w);
            let pj = plus(c, droite(bj.u), bj.w);
            if let Some((a, b)) = croiser(pi, bi.u, pj, bj.u) {
                let tau = BORDURE / (theta / 2.0).tan();
                if a > 0.0 {
                    bras[i].s = bras[i].s.max(a + tau + MARGE);
                }
                if b > 0.0 {
                    bras[j].s = bras[j].s.max(b + tau + MARGE);
                }
            }
        }
        for b in &mut bras {
            b.s = b.s.clamp(S_MIN, S_MAX);
        }
        candidats.insert(n, bras);
    }
    // 2. Assez de place sur chaque bras (l'autre bout coupé aussi, s'il l'est) et au sol
    // jusqu'au-delà de la bouche
    let coupe_autre = |k: usize, debut: bool| -> f32 {
        let n = if debut { bouts[k].1 } else { bouts[k].0 };
        let groupe = coupes[k][usize::from(debut)].map_or(0.0, |c| c.0);
        candidats.get(&n).and_then(|v| v.iter().find(|b| b.k == k && b.debut != debut)).map_or(groupe, |b| b.s)
    };
    let valides: Vec<usize> = candidats
        .iter()
        .filter(|(_, bras)| {
            bras.iter().all(|b| {
                b.longueur() >= b.s + coupe_autre(b.k, b.debut) + RESTE && b.pts.iter().zip(&b.s0).take_while(|(_, &s)| s <= b.s + 1.0).all(|(p, _)| au_sol(p))
            })
        })
        .map(|(&n, _)| n)
        .collect();
    // 3. Les contours
    for n in valides {
        let bras = &candidats[&n];
        let Some(pl) = plan(r, n) else { continue };
        let c = [r.noeuds[n].p[0], r.noeuds[n].p[1]];
        let m = bras.len();
        // Par bras : la coupe, sa direction, ses bords droit et gauche
        let coupes_n: Vec<(Pt, [f32; 2], [f32; 2], [f32; 2])> = bras
            .iter()
            .map(|b| {
                let (q, d) = b.a(b.s);
                let o = [q[0], q[1]];
                (q, d, plus(o, droite(d), q[6]), plus(o, gauche(d), q[6]))
            })
            .collect();
        // Les sommets : (point, côté de bouche) ; un côté de bouche n'est pas recoupé
        let mut v: Vec<([f32; 2], bool)> = Vec::new();
        let mut indices_bouches = Vec::new();
        for i in 0..m {
            let j = (i + 1) % m;
            let (q, di, bdi, bgi) = coupes_n[i];
            indices_bouches.push(v.len());
            v.push((bdi, true));
            v.push(([q[0], q[1]], true));
            v.push((bgi, false));
            let (_, dj, bdj, _) = coupes_n[j];
            let theta = apres(bras[i].ang, bras[j].ang);
            if theta >= DEMI_TOUR {
                // Le tour du nœud, du bord gauche de l'un au bord droit de l'autre
                let ai = gauche(di);
                let a0 = ai[1].atan2(ai[0]);
                let ecart = (theta - PI).max(0.0);
                let k = (ecart / PAS_ANGLE).ceil().max(1.0) as usize;
                for t in 0..=k {
                    let f = t as f32 / k as f32;
                    let a = a0 + ecart * f;
                    let rr = bras[i].w + (bras[j].w - bras[i].w) * f;
                    v.push((plus(c, [a.cos(), a.sin()], rr), false));
                }
                continue;
            }
            // Le coin : les bords se croisent en P, en deçà des deux bouches
            let Some((a, b)) = croiser(bgi, di, bdj, dj) else { continue };
            let (ri, rj) = (-a, -b);
            if ri <= 0.05 || rj <= 0.05 {
                // Un Y trop aigu : la pointe est comblée, d'une bouche à l'autre
                continue;
            }
            let p = plus(bgi, di, a);
            let phi = (di[0] * dj[0] + di[1] * dj[1]).clamp(-1.0, 1.0).acos();
            let bis = [di[0] + dj[0], di[1] + dj[1]];
            let lb = bis[0].hypot(bis[1]);
            let mut tau = BORDURE / (phi / 2.0).tan().max(1e-3);
            let tau_max = ri.min(rj) - 0.05;
            if tau > tau_max {
                tau = tau_max;
            }
            let rayon = tau * (phi / 2.0).tan();
            if lb < 1e-3 || rayon < 0.2 || tau < 0.1 {
                v.push((p, false));
                continue;
            }
            // La bordure : un arc tangent aux deux bords
            let (ti, tj) = (plus(p, di, tau), plus(p, dj, tau));
            let o = plus(p, [bis[0] / lb, bis[1] / lb], rayon / (phi / 2.0).sin());
            let (bi, bj) = ((ti[1] - o[1]).atan2(ti[0] - o[0]), (tj[1] - o[1]).atan2(tj[0] - o[0]));
            let mut delta = (bj - bi).rem_euclid(TAU);
            if delta > PI {
                delta -= TAU;
            }
            let k = (delta.abs() / PAS_ANGLE).ceil().max(1.0) as usize;
            for t in 0..=k {
                let a = bi + delta * t as f32 / k as f32;
                v.push((plus(o, [a.cos(), a.sin()], rayon), false));
            }
        }
        // Les côtés recoupés tous les PAS mètres, les points confondus ôtés
        let nv = v.len();
        let mut contour: Vec<[f32; 2]> = Vec::with_capacity(nv * 2);
        let mut bouches_k = Vec::new();
        // Les points libres posés depuis la dernière bouche (ceux qu'on peut ôter)
        let mut libres = 0usize;
        for i in 0..nv {
            let (p, cote_bouche) = v[i];
            if indices_bouches.contains(&i) {
                // Un point libre confondu avec le bord de la bouche : ôté
                while libres > 0 && contour.last().is_some_and(|&l| dist(l, p) < 0.05) {
                    contour.pop();
                    libres -= 1;
                }
                bouches_k.push(contour.len());
                libres = 0;
            }
            let est_bouche = indices_bouches.iter().any(|&b| i >= b && i <= b + 2);
            if !est_bouche && contour.last().is_some_and(|&l| dist(l, p) < 0.05) {
                continue;
            }
            contour.push(p);
            if !est_bouche {
                libres += 1;
            }
            if cote_bouche {
                continue;
            }
            let q = v[(i + 1) % nv].0;
            let l = dist(p, q);
            let k = (l / PAS).ceil() as usize;
            for t in 1..k {
                contour.push(plus(p, [q[0] - p[0], q[1] - p[1]], t as f32 / k as f32));
                libres += 1;
            }
        }
        // Les derniers points libres confondus avec le premier (une bouche) : ôtés
        while libres > 0 && contour.len() > 3 && dist(contour[contour.len() - 1], contour[0]) < 0.05 {
            contour.pop();
            libres -= 1;
        }
        // Un contour en étoile autour du nœud, sans quoi les anneaux se croiseraient
        let mut total = 0.0;
        let mut etoile = true;
        for i in 0..contour.len() {
            let (p, q) = (contour[i], contour[(i + 1) % contour.len()]);
            if dist(p, c) < 0.3 {
                etoile = false;
                break;
            }
            let e = apres((p[1] - c[1]).atan2(p[0] - c[0]), (q[1] - c[1]).atan2(q[0] - c[0]));
            if e < 1e-4 || e > PI {
                etoile = false;
                break;
            }
            total += e;
        }
        if !etoile || (total - TAU).abs() > 0.01 {
            continue;
        }
        let mut boite = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
        for p in &contour {
            boite = [boite[0].min(p[0]), boite[1].min(p[1]), boite[2].max(p[0]), boite[3].max(p[1])];
        }
        let mut bouches = Vec::new();
        for (i, b) in bras.iter().enumerate() {
            let (q, _, bd, bg) = coupes_n[i];
            let k = bouches_k[i];
            // Dans le sens de parcours de la chaîne : vers l'extérieur à son début
            let (g, d) = if b.debut { (bg, bd) } else { (bd, bg) };
            let bo = Bouche { q, g, m: [q[0], q[1]], d };
            bouches.push((k, bo));
            coupes[b.k][usize::from(!b.debut)] = Some((b.s, bo));
        }
        carrefours.push(Carrefour { noeuds: vec![n], c, plans: vec![pl], contour, trous: Vec::new(), bouches, boite });
    }
    (carrefours, coupes, absorbees)
}

/// Le tracé d'une chaîne coupé à ses bouches : de la bouche du début (son point) à
/// celle de la fin, sans les points à moins de 10 cm d'une coupe.
pub fn couper(points: &[Pt], coupes: &[Option<(f32, Bouche)>; 2]) -> Vec<Pt> {
    if coupes[0].is_none() && coupes[1].is_none() {
        return points.to_vec();
    }
    let mut s = vec![0.0f32; points.len()];
    for i in 1..points.len() {
        s[i] = s[i - 1] + (points[i][0] - points[i - 1][0]).hypot(points[i][1] - points[i - 1][1]);
    }
    let l = s[s.len() - 1];
    let (a, b) = (coupes[0].map_or(-1.0, |c| c.0 + 0.1), coupes[1].map_or(l + 1.0, |c| l - c.0 - 0.1));
    let mut v = Vec::with_capacity(points.len());
    if let Some((_, bo)) = coupes[0] {
        v.push(bo.q);
    }
    v.extend(points.iter().zip(&s).filter(|(_, &x)| x > a && x < b).map(|(p, _)| *p));
    if let Some((_, bo)) = coupes[1] {
        v.push(bo.q);
    }
    v
}
