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
    /// Le dessus posé sur un plan (`materialisation::plaque`) : les carrefours, pas les
    /// fourches (longues, elles suivent la pente de leurs voies ; 0.183).
    pub aplanir: bool,
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
pub fn construire(r: &Reseau, voies: &[Chaine], bouts: &[(usize, usize)], fourches: &std::collections::HashSet<usize>, deja: &[(usize, bool, f32, Bouche)]) -> (Vec<Carrefour>, Vec<[Option<(f32, Bouche)>; 2]>, Vec<bool>) {
    let mut coupes: Vec<[Option<(f32, Bouche)>; 2]> = vec![[None, None]; voies.len()];
    // Les coupes déjà faites (les plaques des fourches) : les bouches des autres
    // carrefours en tiennent compte
    for &(k, debut, s, bo) in deja {
        coupes[k][usize::from(!debut)] = Some((s, bo));
    }
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
        // Une fourche se fait bord à bord (`champ_voirie::fourches`), sans plaque
        if en_groupe.contains(&n) || fourches.contains(&n) {
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
        carrefours.push(Carrefour { noeuds: vec![n], c, plans: vec![pl], contour, trous: Vec::new(), bouches, boite, aplanir: true });
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

/// Une fourche en plaque (0.176) : du tronc (coupé à `S_MIN` du nœud) jusqu'au nez,
/// là où les rubans des deux branches cessent de se chevaucher ; les branches coupées
/// au nez. Les bords extérieurs passent en douceur de ceux du tronc à ceux des
/// branches. Rend la plaque et les coupes (chaîne, au début, abscisse, bouche), ou rien
/// (pas de nez assez tôt, contour qui ne fait pas l'étoile) : la fourche se fait alors
/// en décalant les branches.
pub fn plaque_fourche(r: &Reseau, voies: &[Chaine], n: usize, tronc: (usize, bool), b1: (usize, bool), b2: (usize, bool), autre: &dyn Fn(usize, bool) -> f32) -> Option<(Carrefour, Vec<(usize, bool, f32, Bouche)>)> {
    let c = [r.noeuds[n].p[0], r.noeuds[n].p[1]];
    let bras_de = |(k, debut): (usize, bool)| -> Option<Bras> {
        let v = &voies[k];
        let pts: Vec<Pt> = if debut { v.points.clone() } else { v.points.iter().rev().copied().collect() };
        if pts.len() < 2 || dist([pts[0][0], pts[0][1]], c) > 0.5 {
            return None;
        }
        let mut s0 = vec![0.0f32; pts.len()];
        for i in 1..pts.len() {
            s0[i] = s0[i - 1] + (pts[i][0] - pts[i - 1][0]).hypot(pts[i][1] - pts[i - 1][1]);
        }
        let w = pts[0][6];
        let mut b = Bras { k, debut, pts, s0, u: [1.0, 0.0], w, ang: 0.0, s: S_MIN };
        let (q, _) = b.a(6.0f32.min(b.longueur()));
        b.u = norm([q[0] - c[0], q[1] - c[1]]);
        b.ang = b.u[1].atan2(b.u[0]);
        Some(b)
    };
    let Some(bt) = bras_de(tronc) else { echec(0); return None };
    let (Some(mut x1), Some(mut x2)) = (bras_de(b1), bras_de(b2)) else { echec(0); return None };
    let trace = std::env::var("FLOW_FOURCHE").is_ok_and(|v| {
        let x: Vec<f32> = v.split_whitespace().filter_map(|x| x.parse().ok()).collect();
        x.len() == 2 && dist([x[0], x[1]], c) < 3.0
    });
    // La place de chaque bras : sa longueur moins la coupe de son autre bout
    let place = |b: &Bras| b.longueur() - autre(b.k, !b.debut) - RESTE_FOURCHE;
    if place(&bt) < 0.5 {
        if trace {
            eprintln!("  plaque : tronc trop court ({:.1} m)", bt.longueur());
        }
        echec(1); return None;
    }
    // Le nez : le premier pas où les deux rubans se quittent
    let lmax = NEZ_MAX.min(place(&x1)).min(place(&x2));
    if lmax < S_MIN + 2.0 {
        if trace {
            eprintln!("  plaque : pas de place pour le nez ({lmax:.1} m)");
        }
        echec(2); return None;
    }
    let mut s = S_MIN;
    let mut nez = None;
    while s <= lmax {
        let (p, _) = x1.a(s);
        let (q, _) = x2.a(s);
        // Les rubans se quittent à plat, ou l'un descend (ou monte) loin de l'autre :
        // une rampe qui longe sa voie (0.184 ; avant, la plaque la couvrait jusqu'à
        // `NEZ_MAX` et bouchait la descente)
        if dist([p[0], p[1]], [q[0], q[1]]) >= p[6] + q[6] || (p[2] - q[2]).abs() > NEZ_DENIVELE {
            nez = Some(s);
            break;
        }
        s += 1.0;
    }
    if trace {
        eprintln!("  plaque : nez {nez:?} (au plus {lmax:.1} m)");
    }
    // Pas de nez avant la fin de la place : la plaque s'arrête là, les deux bouches
    // rognées du côté où les rubans se chevauchent encore
    let s_nez = nez.map_or(lmax, |s| (s + NEZ_MARGE).min(lmax));
    x1.s = s_nez;
    x2.s = s_nez;
    let mut bt = bt;
    bt.s = S_MIN.min(place(&bt));
    let Some(pl) = plan(r, n) else { echec(3); return None };
    let ut = bt.u;
    let wt = bt.w;
    let it_tronc = 0usize;
    let mut bras = vec![bt, x1, x2];
    // Le tronc d'abord, puis les branches dans le sens direct
    let a0 = bras[0].ang;
    bras[1..].sort_by(|a, b| apres(a0, a.ang).total_cmp(&apres(a0, b.ang)));
    let m = 3;
    let coupes_n: Vec<(Pt, [f32; 2], [f32; 2], [f32; 2])> = bras
        .iter()
        .map(|b| {
            let (q, d) = b.a(b.s);
            let o = [q[0], q[1]];
            (q, d, plus(o, droite(d), q[6]), plus(o, gauche(d), q[6]))
        })
        .collect();
    let mut coupes_n = coupes_n;
    {
        // Les bords intérieurs des deux branches (bras 1 et 2), rognés s'ils se croisent
        let (p1, p2) = (coupes_n[1].0, coupes_n[2].0);
        let (a, b) = ([p1[0], p1[1]], [p2[0], p2[1]]);
        let d = dist(a, b);
        if d < p1[6] + p2[6] {
            let u = norm([b[0] - a[0], b[1] - a[1]]);
            let t = d * p1[6] / (p1[6] + p2[6]);
            coupes_n[1].3 = plus(a, u, (t - 0.05).max(0.05));
            coupes_n[2].2 = plus(a, u, (t + 0.05).min(d - 0.05));
        }
    }
    let lisse = |u: f32| {
        let u = u.clamp(0.0, 1.0);
        u * u * (3.0 - 2.0 * u)
    };
    let mut v: Vec<([f32; 2], bool)> = Vec::new();
    let mut indices_bouches = Vec::new();
    for i in 0..m {
        let j = (i + 1) % m;
        let (_, _, bdi, bgi) = coupes_n[i];
        indices_bouches.push(v.len());
        v.push((bdi, true));
        v.push(([coupes_n[i].0[0], coupes_n[i].0[1]], true));
        v.push((bgi, false));
        if i == it_tronc {
            // Du bord gauche du tronc au bord droit de la branche j, jusqu'au nez
            let bord = plus(c, gauche(ut), wt);
            v.push((bord, false));
            let b = &bras[j];
            let mut s = 1.0;
            while s < b.s - 0.5 {
                let (p, d) = b.a(s);
                let propre = plus([p[0], p[1]], droite(d), p[6]);
                let cible = plus(bord, ut, -s);
                let k = 1.0 - lisse(s / b.s);
                v.push(([propre[0] + (cible[0] - propre[0]) * k, propre[1] + (cible[1] - propre[1]) * k], false));
                s += 1.0;
            }
        } else if j == it_tronc {
            // Du nez, le long du bord gauche de la branche i, au bord droit du tronc
            let bord = plus(c, droite(ut), wt);
            let b = &bras[i];
            let mut s = (b.s - 1.0).floor();
            while s >= 1.0 {
                let (p, d) = b.a(s);
                let propre = plus([p[0], p[1]], gauche(d), p[6]);
                let cible = plus(bord, ut, -s);
                let k = 1.0 - lisse(s / b.s);
                v.push(([propre[0] + (cible[0] - propre[0]) * k, propre[1] + (cible[1] - propre[1]) * k], false));
                s -= 1.0;
            }
            v.push((bord, false));
        }
        // Entre les deux branches : le nez, d'une bouche à l'autre
    }
    // Les côtés recoupés tous les PAS mètres, les points confondus ôtés
    let nv = v.len();
    let mut contour: Vec<[f32; 2]> = Vec::with_capacity(nv * 2);
    let mut bouches_k = Vec::new();
    for i in 0..nv {
        let (p, cote_bouche) = v[i];
        let est_bouche = indices_bouches.iter().any(|&b| i >= b && i <= b + 2);
        if indices_bouches.contains(&i) {
            bouches_k.push(contour.len());
        } else if !est_bouche && contour.last().is_some_and(|&l| dist(l, p) < 0.05) {
            continue;
        }
        contour.push(p);
        if cote_bouche {
            continue;
        }
        let q = v[(i + 1) % nv].0;
        let k = (dist(p, q) / PAS).ceil() as usize;
        for t in 1..k {
            contour.push(plus(p, [q[0] - p[0], q[1] - p[1]], t as f32 / k as f32));
        }
    }
    // Un polygone simple, dans le sens direct (maillé par Delaunay, pas en anneaux)
    if !polygone_simple(&contour) {
        if trace {
            eprintln!("  plaque : contour qui se croise");
        }
        echec(4);
        return None;
    }
    let o = contour[0];
    let aire: f32 = (0..contour.len()).map(|i| {
        let (p, q) = (contour[i], contour[(i + 1) % contour.len()]);
        (p[0] - o[0]) * (q[1] - o[1]) - (q[0] - o[0]) * (p[1] - o[1])
    }).sum();
    if aire <= 0.0 {
        echec(5);
        return None;
    }
    if !crate::materialisation::maillage_valide(&[contour.clone()]) {
        if std::env::var_os("FLOW_DIAG").is_some() {
            eprintln!("maillage refusé : fourche en ({:.1} ; {:.1}), {} points", c[0], c[1], contour.len());
        }
        echec(6);
        return None;
    }
    let mut boite = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    for p in &contour {
        boite = [boite[0].min(p[0]), boite[1].min(p[1]), boite[2].max(p[0]), boite[3].max(p[1])];
    }
    let mut bouches = Vec::new();
    let mut coupes = Vec::new();
    for (i, b) in bras.iter().enumerate() {
        let (q, _, bd, bg) = coupes_n[i];
        let (g, d) = if b.debut { (bg, bd) } else { (bd, bg) };
        let bo = Bouche { q, g, m: [q[0], q[1]], d };
        bouches.push((bouches_k[i], bo));
        coupes.push((b.k, b.debut, b.s, bo));
    }
    Some((Carrefour { noeuds: vec![n], c, plans: vec![pl], contour, trous: Vec::new(), bouches, boite, aplanir: false }, coupes))
}

/// Le nez d'une fourche : au plus à tant de mètres du nœud, et ce qu'il laisse au-delà
/// du point où les rubans se quittent.
const NEZ_MAX: f32 = 60.0;
const NEZ_MARGE: f32 = 1.0;
/// Le dénivelé (m) entre les deux branches qui arrête la plaque d'une fourche.
const NEZ_DENIVELE: f32 = 0.3;
/// Ce qu'il doit rester (m) d'une chaîne au-delà de la coupe d'une fourche.
const RESTE_FOURCHE: f32 = 3.0;
/// La coupe supposée (m) à l'autre bout d'une branche ou d'un tronc, s'il n'y en a pas
/// déjà une (un carrefour la règle ensuite sur la place qui reste).
pub const AUTRE_FOURCHE: f32 = 6.0;

/// Les fourches sans plaque, par raison : bras, tronc trop court, pas de place pour le
/// nez, pas de plan, contour qui se croise, à l'envers ou non maillable (pour le diagnostic).
pub static ECHECS: std::sync::Mutex<[usize; 7]> = std::sync::Mutex::new([0; 7]);
fn echec(i: usize) {
    if let Ok(mut e) = ECHECS.lock() {
        e[i] += 1;
    }
}

/// Le contour fermé ne se croise pas (deux côtés non voisins ne se coupent pas).
fn polygone_simple(c: &[[f32; 2]]) -> bool {
    let n = c.len();
    if n < 3 {
        return false;
    }
    let cote = |a: [f32; 2], b: [f32; 2], p: [f32; 2]| (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]);
    let boites: Vec<[f32; 4]> = (0..n).map(|i| {
        let (p, q) = (c[i], c[(i + 1) % n]);
        [p[0].min(q[0]), p[1].min(q[1]), p[0].max(q[0]), p[1].max(q[1])]
    }).collect();
    for i in 0..n {
        let (a, b) = (c[i], c[(i + 1) % n]);
        for j in i + 2..n {
            if i == 0 && j == n - 1 {
                continue;
            }
            let (bi, bj) = (boites[i], boites[j]);
            if bi[2] < bj[0] || bj[2] < bi[0] || bi[3] < bj[1] || bj[3] < bi[1] {
                continue;
            }
            let (p, q) = (c[j], c[(j + 1) % n]);
            let (d1, d2, d3, d4) = (cote(a, b, p), cote(a, b, q), cote(p, q, a), cote(p, q, b));
            if d1 * d2 < 0.0 && d3 * d4 < 0.0 {
                return false;
            }
        }
    }
    true
}
