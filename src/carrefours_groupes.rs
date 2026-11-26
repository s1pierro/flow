//! Les groupes de nœuds proches d'une seule pièce (0.143) : des nœuds de carrefour
//! reliés par des tronçons courts (au plus `LIEN_MAX` mètres : l'anneau d'un giratoire,
//! les chaussées séparées d'un boulevard, deux carrefours décalés). Une seule plaque
//! pour tout le groupe :
//! 1. l'union à plat des emprises : les tronçons intérieurs entiers, un disque à chaque
//!    nœud, le début de chaque bras jusqu'au-delà de sa bouche ;
//! 2. une fermeture (élargie de `BORDURE` aux coins ronds, puis rétrécie d'autant) :
//!    les coins rentrants deviennent des bordures arrondies, les trous plus étroits que
//!    deux bordures sont bouchés ;
//! 3. chaque bras coupé net à sa bouche (la différence avec son prolongement), la
//!    section remise au bit près pour que le ruban du bras s'y raccorde.
//!
//! Les tronçons intérieurs n'ont plus de ruban (`absorbees`) ; l'îlot d'un giratoire
//! reste un trou de la plaque. Un groupe qui ne s'y prête pas (trop grand, un bras trop
//! court, une plaque en plusieurs morceaux) garde les rubans et les dalles.

use crate::carrefours::{dist, droite, gauche, norm, plus, Bouche, Bras, Carrefour, BORDURE, PAS, RESTE, S_MAX, S_MIN};
use crate::champ_voirie::Chaine;
use crate::materialisation::{au_sol, plan, Pt};
use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::single::SingleFloatOverlay;
use i_overlay::float::simplify::SimplifyShape;
use i_overlay::mesh::float::outline::offset::OutlineOffset;
use i_overlay::mesh::float::style::{LineJoin, OutlineStyle};
use crate::reseau::Reseau;
use std::collections::{HashMap, HashSet};

/// Un tronçon entre deux nœuds de carrefour plus court que ça (m) les met dans le même
/// groupe.
const LIEN_MAX: f32 = 25.0;
/// Au plus tant de nœuds par groupe, et une boîte d'au plus tant de mètres de côté.
const NOEUDS_MAX: usize = 16;
const COTE_MAX: f32 = 120.0;
/// Le prolongement des bras au-delà de leur bouche dans l'union (m) : la fermeture n'y
/// touche pas la bouche.
const PROLONGE: f32 = 2.0 * BORDURE + 2.0;
/// Le dégagement d'une bouche : ses coins à au moins tant de fois la bordure des autres
/// emprises.
const DEGAGEMENT: f32 = 1.5;
/// Un tronçon dont les deux bouts sont dans le groupe y est intérieur jusqu'à cette
/// longueur (m) : l'anneau d'un giratoire plus long qu'un lien.
const INTERIEUR_MAX: f32 = 60.0;
/// Deux bras jumeaux (les chaussées séparées d'un boulevard qui sortent côte à côte) :
/// à moins de tant de degrés, bords à moins du dégagement l'un de l'autre. Leurs
/// bouches sont alignées et l'entre-deux coupé avec eux.
const JUMEAUX_ANGLE: f32 = 25.0;
/// Jusqu'où (m au-delà de `S_MAX`) reculer les bouches de deux jumeaux pour les aligner,
/// et de combien leurs nœuds peuvent être décalés le long de la voie.
const JUMEAUX_RECUL: f32 = 15.0;
const JUMEAUX_DECALAGE: f32 = 30.0;
/// Le jeu minimal entre deux bras jumeaux à leur bouche (m).
const JUMEAUX_JEU: f32 = 0.2;
/// La bouche d'un bras qui devient pont : tant de mètres avant le début du pont.
const AVANT_PONT: f32 = 0.5;

type Anneau = Vec<[f64; 2]>;

fn f64s(p: [f32; 2]) -> [f64; 2] {
    [p[0] as f64, p[1] as f64]
}

/// La distance de `p` au segment `a`–`b`.
fn au_segment(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let (ux, uz) = (b[0] - a[0], b[1] - a[1]);
    let l2 = ux * ux + uz * uz;
    let t = if l2 > 1e-12 { (((p[0] - a[0]) * ux + (p[1] - a[1]) * uz) / l2).clamp(0.0, 1.0) } else { 0.0 };
    (p[0] - a[0] - t * ux).hypot(p[1] - a[1] - t * uz)
}

/// L'emprise d'un tracé de l'abscisse `s0` à `s1` (demi-largeur des points, plus
/// `marge`) : un polygone, bords gauches puis droits à rebours.
fn bande(b: &Bras, s0: f32, s1: f32, marge: f32) -> Anneau {
    bande2(b, s0, s1, marge, marge)
}

/// L'axe d'un bras de `s0` à `s1`.
fn axe_bras(b: &Bras, s0: f32, s1: f32) -> Vec<Pt> {
    let mut axe: Vec<Pt> = vec![b.a(s0).0];
    for (p, &s) in b.pts.iter().zip(&b.s0) {
        if s > s0 + 0.05 && s < s1 - 0.05 {
            axe.push(*p);
        }
    }
    axe.push(b.a(s1.min(b.longueur())).0);
    axe
}

/// Les bords d'un axe (demi-largeur des points plus la marge de chaque côté) : gauche,
/// droit, dans le sens de l'axe.
fn bords(axe: &[Pt], mg: f32, md: f32) -> (Anneau, Anneau) {
    let n = axe.len();
    let mut g = Vec::with_capacity(n);
    let mut d = Vec::with_capacity(n);
    for i in 0..n {
        let (a, b) = (axe[i.saturating_sub(1)], axe[(i + 1).min(n - 1)]);
        let t = norm([b[0] - a[0], b[1] - a[1]]);
        let o = [axe[i][0], axe[i][1]];
        g.push(f64s(plus(o, gauche(t), axe[i][6] + mg)));
        d.push(f64s(plus(o, droite(t), axe[i][6] + md)));
    }
    (g, d)
}

/// L'emprise d'un bras, marges différentes à gauche et à droite.
fn bande2(b: &Bras, s0: f32, s1: f32, mg: f32, md: f32) -> Anneau {
    let (mut g, mut d) = bords(&axe_bras(b, s0, s1), mg, md);
    d.reverse();
    g.extend(d);
    g
}

#[allow(dead_code)]
fn bande_ancienne(b: &Bras, s0: f32, s1: f32, marge: f32) -> Anneau {
    let mut axe: Vec<Pt> = vec![b.a(s0).0];
    for (p, &s) in b.pts.iter().zip(&b.s0) {
        if s > s0 + 0.05 && s < s1 - 0.05 {
            axe.push(*p);
        }
    }
    axe.push(b.a(s1.min(b.longueur())).0);
    bande_axe(&axe, marge)
}

fn bande_axe(axe: &[Pt], marge: f32) -> Anneau {
    let n = axe.len();
    let mut g = Vec::with_capacity(n);
    let mut d = Vec::with_capacity(n);
    for i in 0..n {
        let (a, b) = (axe[i.saturating_sub(1)], axe[(i + 1).min(n - 1)]);
        let t = norm([b[0] - a[0], b[1] - a[1]]);
        let o = [axe[i][0], axe[i][1]];
        let w = axe[i][6] + marge;
        g.push(f64s(plus(o, gauche(t), w)));
        d.push(f64s(plus(o, droite(t), w)));
    }
    d.reverse();
    g.extend(d);
    g
}

fn disque(c: [f32; 2], r: f32) -> Anneau {
    (0..16).map(|k| {
        let a = k as f32 * std::f32::consts::TAU / 16.0;
        f64s(plus(c, [a.cos(), a.sin()], r))
    }).collect()
}

/// Les anneaux tous en sens direct.
fn orienter(v: &mut [Anneau]) {
    for a in v.iter_mut() {
        let n = a.len();
        let s: f64 = (0..n).map(|i| a[i][0] * a[(i + 1) % n][1] - a[(i + 1) % n][0] * a[i][1]).sum();
        if s < 0.0 {
            a.reverse();
        }
    }
}

/// L'aire signée d'un anneau (positive en sens direct).
fn aire(a: &[[f32; 2]]) -> f32 {
    let n = a.len();
    (0..n).map(|i| {
        let (p, q) = (a[i], a[(i + 1) % n]);
        0.5 * (p[0] * q[1] - q[0] * p[1])
    }).sum()
}

/// Un anneau recoupé tous les `PAS` mètres, les points confondus ôtés ; `fixes` : les
/// côtés à ne pas recouper (leur premier point), dont les points sont gardés.
fn recouper(a: &[[f32; 2]], fixes: &HashSet<usize>) -> (Vec<[f32; 2]>, HashMap<usize, usize>) {
    let n = a.len();
    let mut v: Vec<[f32; 2]> = Vec::with_capacity(n * 2);
    let mut nouveaux = HashMap::new();
    let garde = |i: usize| fixes.contains(&i) || fixes.contains(&((i + n - 1) % n));
    for i in 0..n {
        let p = a[i];
        if !garde(i) && v.last().is_some_and(|&l| dist(l, p) < 0.05) {
            continue;
        }
        nouveaux.insert(i, v.len());
        v.push(p);
        if fixes.contains(&i) {
            continue;
        }
        let q = a[(i + 1) % n];
        let k = (dist(p, q) / PAS).ceil() as usize;
        for t in 1..k {
            v.push(plus(p, [q[0] - p[0], q[1] - p[1]], t as f32 / k as f32));
        }
    }
    (v, nouveaux)
}

/// Les groupes de nœuds proches du réseau, faits en plaques (`carrefours`) ; leurs bras
/// coupés (`coupes`), leurs tronçons intérieurs absorbés (`absorbees`). Rend les nœuds
/// des groupes faits.
#[allow(clippy::too_many_arguments)]
pub fn construire(
    r: &Reseau,
    voies: &[Chaine],
    bouts: &[(usize, usize)],
    aux: &HashMap<usize, Vec<(usize, bool)>>,
    coupes: &mut [[Option<(f32, Bouche)>; 2]],
    absorbees: &mut [bool],
    carrefours: &mut Vec<Carrefour>,
) -> HashSet<usize> {
    let mut faits = HashSet::new();
    let longueur = |k: usize| -> f32 { voies[k].points.windows(2).map(|s| (s[1][0] - s[0][0]).hypot(s[1][1] - s[0][1])).sum() };
    // Les nœuds de carrefour : là où finissent au moins deux chaînes
    let carrefour = |n: usize| aux.get(&n).is_some_and(|v| v.len() >= 2);
    // Les liens courts, tout au sol, entre deux nœuds de carrefour
    let lien = |k: usize| {
        let (a, b) = bouts[k];
        a != b && carrefour(a) && carrefour(b) && voies[k].points.iter().all(au_sol) && longueur(k) <= LIEN_MAX
    };
    let mut parent: HashMap<usize, usize> = HashMap::new();
    fn racine(p: &mut HashMap<usize, usize>, i: usize) -> usize {
        let mut i = i;
        while let Some(&j) = p.get(&i) {
            if j == i {
                break;
            }
            let g = p.get(&j).copied().unwrap_or(j);
            p.insert(i, g);
            i = j;
        }
        i
    }
    for k in 0..voies.len() {
        if lien(k) {
            let (a, b) = bouts[k];
            parent.entry(a).or_insert(a);
            parent.entry(b).or_insert(b);
            let (ra, rb) = (racine(&mut parent, a), racine(&mut parent, b));
            parent.insert(ra, rb);
        }
    }
    let mut groupes: HashMap<usize, Vec<usize>> = HashMap::new();
    let cles: Vec<usize> = parent.keys().copied().collect();
    for n in cles {
        let g = racine(&mut parent, n);
        groupes.entry(g).or_default().push(n);
    }
    let mut groupes: Vec<Vec<usize>> = groupes.into_values().filter(|g| g.len() >= 2 && g.len() <= NOEUDS_MAX).collect();
    groupes.iter_mut().for_each(|g| g.sort_unstable());
    groupes.sort();
    for g in groupes {
        if let Some(c) = groupe(r, voies, bouts, aux, &g, coupes, absorbees, &lien) {
            faits.extend(g.iter().copied());
            carrefours.push(c);
        }
    }
    faits
}

/// La plaque d'un groupe, s'il s'y prête (ses coupes et ses absorptions posées).
#[allow(clippy::too_many_arguments)]
fn groupe(
    r: &Reseau,
    voies: &[Chaine],
    bouts: &[(usize, usize)],
    aux: &HashMap<usize, Vec<(usize, bool)>>,
    g: &[usize],
    coupes: &mut [[Option<(f32, Bouche)>; 2]],
    absorbees: &mut [bool],
    lien: &dyn Fn(usize) -> bool,
) -> Option<Carrefour> {
    let dans: HashSet<usize> = g.iter().copied().collect();
    // Le diagnostic : FLOW_GROUPE=n1,n2… dit pourquoi un groupe de ces nœuds échoue
    let suivi = std::env::var("FLOW_GROUPE").ok().is_some_and(|v| v.split(',').filter_map(|x| x.parse::<usize>().ok()).any(|n| dans.contains(&n)));
    let echec = |raison: &str| {
        if suivi {
            eprintln!("groupe {g:?} : {raison}");
        }
        None
    };
    let pos = |n: usize| [r.noeuds[n].p[0], r.noeuds[n].p[1]];
    // La taille du groupe
    let mut boite = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    for &n in g {
        let p = pos(n);
        boite = [boite[0].min(p[0]), boite[1].min(p[1]), boite[2].max(p[0]), boite[3].max(p[1])];
    }
    if boite[2] - boite[0] > COTE_MAX || boite[3] - boite[1] > COTE_MAX {
        return echec("trop grand");
    }
    // Les tronçons intérieurs (les liens, et ce qui relie deux nœuds du groupe sans être
    // trop long) et les bras
    let longueur = |k: usize| -> f32 { voies[k].points.windows(2).map(|s| (s[1][0] - s[0][0]).hypot(s[1][1] - s[0][1])).sum() };
    let interieur = |k: usize| {
        let (a, b) = bouts[k];
        dans.contains(&a) && dans.contains(&b) && (lien(k) || (a != b && voies[k].points.iter().all(au_sol) && longueur(k) <= INTERIEUR_MAX))
    };
    let mut interieurs: Vec<usize> = Vec::new();
    let mut bras: Vec<Bras> = Vec::new();
    for &n in g {
        for &(k, debut) in &aux[&n] {
            let (_a, _b) = bouts[k];
            if interieur(k) {
                if debut {
                    interieurs.push(k);
                }
                continue;
            }
            let v = &voies[k];
            let pts: Vec<Pt> = if debut { v.points.clone() } else { v.points.iter().rev().copied().collect() };
            if pts.len() < 2 || dist([pts[0][0], pts[0][1]], pos(n)) > 0.5 {
                return echec("bras sans point au nœud");
            }
            let mut s0 = vec![0.0f32; pts.len()];
            for i in 1..pts.len() {
                s0[i] = s0[i - 1] + (pts[i][0] - pts[i - 1][0]).hypot(pts[i][1] - pts[i - 1][1]);
            }
            let w = pts[0][6];
            bras.push(Bras { k, debut, pts, s0, u: [1.0, 0.0], w, ang: 0.0, s: S_MIN });
        }
    }
    if bras.is_empty() {
        return echec("sans bras");
    }
    // Les axes des emprises fixes (tronçons intérieurs, disques des nœuds) : (a, b, w)
    let mut fixes: Vec<([f32; 2], [f32; 2], f32)> = Vec::new();
    for &k in &interieurs {
        for s in voies[k].points.windows(2) {
            fixes.push(([s[0][0], s[0][1]], [s[1][0], s[1][1]], s[0][6].max(s[1][6])));
        }
    }
    let rayon_noeud = |n: usize| aux[&n].iter().map(|&(k, debut)| if debut { voies[k].points[0][6] } else { voies[k].points[voies[k].points.len() - 1][6] }).fold(0.0f32, f32::max);
    for &n in g {
        fixes.push((pos(n), pos(n), rayon_noeud(n)));
    }
    // Les bras jumeaux : (bras de droite, bras de gauche), chacun dans une paire au plus
    let marge = DEGAGEMENT * BORDURE;
    let cos_j = JUMEAUX_ANGLE.to_radians().cos();
    // (le jumeau de gauche, de droite)
    let mut jumeau: Vec<[Option<usize>; 2]> = vec![[None, None]; bras.len()];
    let mut paires: Vec<(usize, usize)> = Vec::new();
    {
        let mut candidats: Vec<(f32, usize, usize)> = Vec::new();
        for i in 0..bras.len() {
            for j in i + 1..bras.len() {
                let (bi, bj) = (&bras[i], &bras[j]);
                let s = 10.0f32.min(bi.longueur() - 0.1).min(bj.longueur() - 0.1);
                if s <= 0.0 {
                    continue;
                }
                // Parallèles tout du long de leur début, pas seulement en un point
                let paralleles = [10.0f32, 15.0, 20.0, S_MAX + 5.0].iter().all(|&x| {
                    let x = x.min(bi.longueur() - 0.1).min(bj.longueur() - 0.1);
                    let ((_, di), (_, dj)) = (bi.a(x), bj.a(x));
                    di[0] * dj[0] + di[1] * dj[1] >= cos_j
                });
                if !paralleles {
                    continue;
                }
                // L'écart de leurs bords : le plus petit, de l'un à l'autre et de l'autre à
                // l'un, à plusieurs abscisses (leurs nœuds peuvent être décalés)
                let ecart_de = |a: &Bras, b: &Bras| -> f32 {
                    [10.0f32, 15.0, 20.0, S_MAX + 5.0].iter().map(|&x| {
                        let q = a.a(x.min(a.longueur() - 0.1)).0;
                        let o = [q[0], q[1]];
                        b.pts.windows(2).zip(b.s0.windows(2)).take_while(|(_, l)| l[0] < S_MAX + JUMEAUX_DECALAGE).map(|(p, _)| au_segment(o, [p[0][0], p[0][1]], [p[1][0], p[1][1]]) - p[0][6].max(p[1][6])).fold(f32::MAX, f32::min) - q[6]
                    }).fold(f32::MAX, f32::min)
                };
                let ecart = ecart_de(bi, bj).min(ecart_de(bj, bi));
                if ecart < marge {
                    candidats.push((ecart, i, j));
                }
            }
        }
        candidats.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (_, i, j) in candidats {
            // Lequel est à gauche de l'autre
            let s = 10.0f32.min(bras[i].longueur() - 0.1);
            let (qi, di) = bras[i].a(s);
            let (qj, _) = bras[j].a(s.min(bras[j].longueur() - 0.1));
            let gj = (qj[0] - qi[0]) * gauche(di)[0] + (qj[1] - qi[1]) * gauche(di)[1];
            let (d, g) = if gj > 0.0 { (i, j) } else { (j, i) };
            if jumeau[d][0].is_some() || jumeau[g][1].is_some() {
                continue;
            }
            jumeau[d][0] = Some(g);
            jumeau[g][1] = Some(d);
            paires.push((d, g));
        }
    }
    // Les grappes de jumeaux : les composantes des paires
    let mut grappes: Vec<Vec<usize>> = Vec::new();
    {
        let mut vu = vec![false; bras.len()];
        for i in 0..bras.len() {
            if vu[i] || jumeau[i] == [None, None] {
                continue;
            }
            let mut g = vec![i];
            vu[i] = true;
            let mut k = 0;
            while k < g.len() {
                for j in jumeau[g[k]].into_iter().flatten() {
                    if !vu[j] {
                        vu[j] = true;
                        g.push(j);
                    }
                }
                k += 1;
            }
            grappes.push(g);
        }
    }
    // Les bouches : assez loin de toutes les autres emprises (bras voisins compris, sauf
    // le jumeau)
    let degage = |i: usize, s: f32, bras: &[Bras]| -> bool {
        let b = &bras[i];
        let (q, d) = b.a(s);
        let o = [q[0], q[1]];
        let coins = [plus(o, droite(d), q[6]), plus(o, gauche(d), q[6])];
        let loin = |a: [f32; 2], z: [f32; 2], w: f32| coins.iter().all(|&c| au_segment(c, a, z) - w > marge);
        // Le disque du nœud du bras : seulement en avant de lui
        fixes.iter().all(|&(a, z, w)| loin(a, z, w))
            && bras.iter().enumerate().filter(|&(j, _)| j != i && !jumeau[i].contains(&Some(j))).all(|(_, o)| {
                o.pts.windows(2).zip(o.s0.windows(2)).take_while(|(_, s)| s[0] < o.s + PROLONGE).all(|(p, _)| loin([p[0][0], p[0][1]], [p[1][0], p[1][1]], p[0][6].max(p[1][6])))
            })
    };
    for _ in 0..4 {
        for i in 0..bras.len() {
            let mut s = S_MIN;
            while s <= S_MAX && !degage(i, s, &bras) {
                s += 0.5;
            }
            if s > S_MAX {
                if suivi {
                    // Ce qui gêne le bras i à S_MAX : le bras voisin le plus proche
                    let (q, d) = bras[i].a(S_MAX);
                    let o = [q[0], q[1]];
                    for (_, b) in bras.iter().enumerate().filter(|&(j, _)| j != i) {
                        let ecart = b.pts.windows(2).map(|p| au_segment(o, [p[0][0], p[0][1]], [p[1][0], p[1][1]]) - p[0][6]).fold(f32::MAX, f32::min) - q[6];
                        let (_, db) = b.a(S_MAX.min(b.longueur() - 0.1));
                        let angle = (d[0] * db[0] + d[1] * db[1]).clamp(-1.0, 1.0).acos().to_degrees();
                        if ecart < 6.0 {
                            eprintln!("  bras {} (l {:.1}) gêné par bras {} (l {:.1}) : écart des bords {ecart:.1} m, angle {angle:.0}°", bras[i].k, q[6] * 2.0, b.k, b.w * 2.0);
                        }
                    }
                    let f = fixes.iter().map(|&(a, z, w)| au_segment(o, a, z) - w - q[6]).fold(f32::MAX, f32::min);
                    eprintln!("  bras {} : emprise fixe la plus proche à {f:.1} m", bras[i].k);
                }
                return echec("bouche sans dégagement");
            }
            bras[i].s = s;
        }
        // Les jumeaux, par grappes (deux, trois, quatre chaussées côte à côte) : leurs
        // bouches sur une même ligne en travers, la plus proche où toutes sont dégagées
        for grappe in &grappes {
            let t = {
                let (x, z) = grappe.iter().fold((0.0, 0.0), |a, &k| {
                    let d = bras[k].a(S_MAX.min(bras[k].longueur() - 0.1)).1;
                    (a.0 + d[0], a.1 + d[1])
                });
                norm([x, z])
            };
            let proj = |k: usize, s: f32, bras: &[Bras]| {
                let q = bras[k].a(s).0;
                q[0] * t[0] + q[1] * t[1]
            };
            let s_max = S_MAX + JUMEAUX_RECUL + JUMEAUX_DECALAGE;
            // L'abscisse du bras k où il passe la ligne l
            let s_de = |k: usize, l: f32, bras: &[Bras]| -> Option<f32> {
                let mut s = S_MIN;
                while s <= s_max.min(bras[k].longueur() - 0.1) {
                    if proj(k, s, bras) >= l {
                        return Some(s);
                    }
                    s += 0.05;
                }
                None
            };
            let mut l = grappe.iter().map(|&k| proj(k, bras[k].s, &bras)).fold(f32::MIN, f32::max);
            let mut dernier = String::new();
            let mut fait = false;
            for _ in 0..80 {
                let ss: Vec<Option<f32>> = grappe.iter().map(|&k| s_de(k, l, &bras)).collect();
                if ss.iter().any(|x| x.is_none()) {
                    dernier = format!("ligne {l:.1} hors d'un bras");
                    break;
                }
                let ss: Vec<f32> = ss.into_iter().flatten().collect();
                let degagees = grappe.iter().zip(&ss).all(|(&k, &s)| degage(k, s, &bras));
                // Le jeu entre voisins à leurs bouches
                let jeu = paires.iter().filter(|(d, _)| grappe.contains(d)).map(|&(d, g)| {
                    let sd = ss[grappe.iter().position(|&k| k == d).unwrap()];
                    let sg = ss[grappe.iter().position(|&k| k == g).unwrap()];
                    let (q, td) = bras[d].a(sd);
                    let (p, _) = bras[g].a(sg);
                    (p[0] - q[0]) * gauche(td)[0] + (p[1] - q[1]) * gauche(td)[1] - q[6] - p[6]
                }).fold(f32::MAX, f32::min);
                if suivi {
                    dernier = format!("ligne {l:.1} : abscisses {:?}, jeu {jeu:.2}, dégagées {degagees}", ss.iter().map(|x| (x * 10.0).round() / 10.0).collect::<Vec<_>>());
                }
                if degagees && jeu >= JUMEAUX_JEU {
                    for (&k, &s) in grappe.iter().zip(&ss) {
                        bras[k].s = s;
                    }
                    fait = true;
                    break;
                }
                l += 0.5;
            }
            if !fait {
                if suivi {
                    eprintln!("  grappe {:?} : {dernier}", grappe.iter().map(|&k| bras[k].k).collect::<Vec<_>>());
                }
                return echec("jumeaux sans bouches alignées");
            }
        }
    }
    // Assez de place sur chaque bras, et au sol jusqu'au-delà de la bouche et du
    // prolongement
    for b in &mut bras {
        let autre = coupes[b.k][usize::from(b.debut)].map_or(0.0, |c| c.0);
        if b.longueur() < b.s + autre + RESTE || b.longueur() < b.s + PROLONGE + 1.0 {
            return echec("bras trop court");
        }
        // Un bras qui devient pont peu après le nœud (un pont qui part du carrefour) :
        // sa bouche ramenée juste avant le pont (0.178 ; avant, le groupe était refusé
        // s'il n'était pas au sol jusqu'au-delà du prolongement)
        let s_pont = b.pts.iter().zip(&b.s0).find(|(p, _)| !au_sol(p)).map_or(f32::MAX, |(_, &s)| s);
        if b.s > s_pont - AVANT_PONT {
            if s_pont - AVANT_PONT < S_MIN {
                return echec("bras en pont dès le nœud");
            }
            b.s = s_pont - AVANT_PONT;
        }
        if coupes[b.k][usize::from(!b.debut)].is_some() {
            return echec("bras déjà coupé à l'autre bout");
        }
    }
    // 1. L'union des emprises
    let mut morceaux: Vec<Anneau> = Vec::new();
    for &k in &interieurs {
        morceaux.push(bande_axe(&voies[k].points, 0.0));
    }
    for &n in g {
        morceaux.push(disque(pos(n), rayon_noeud(n)));
    }
    for b in &bras {
        morceaux.push(bande(b, 0.0, b.s + PROLONGE, 0.0));
    }
    // Tous dans le même sens : en sens contraires, deux emprises qui se recouvrent
    // s'annuleraient (règle non nulle)
    orienter(&mut morceaux);
    let union = morceaux.simplify_shape(FillRule::NonZero);
    // 2. La fermeture
    let rb = BORDURE as f64;
    let large = union.outline(&OutlineStyle::new(rb).line_join(LineJoin::Round(0.2)));
    let ferme = large.outline(&OutlineStyle::new(-rb).line_join(LineJoin::Miter(0.1)));
    // La fermeture contient l'union : réunies au cas où l'érosion aurait rongé un lien
    let ferme = ferme.overlay(&union, OverlayRule::Union, FillRule::NonZero);
    if suivi {
        eprintln!("  union en {} morceaux, fermée en {}", union.len(), ferme.len());
    }
    // 3. Les bras coupés à leur bouche
    // (les jumeaux sans marge l'un vers l'autre, l'entre-deux coupé avec eux)
    let mut coupe: Vec<Anneau> = Vec::new();
    for (i, b) in bras.iter().enumerate() {
        let mg = if jumeau[i][0].is_some() { 0.0 } else { 0.5 };
        let md = if jumeau[i][1].is_some() { 0.0 } else { 0.5 };
        coupe.push(bande2(b, b.s, b.s + PROLONGE + 2.0, mg, md));
    }
    for &(d, g) in &paires {
        let (bd, bg) = (&bras[d], &bras[g]);
        let (gauche_d, _) = bords(&axe_bras(bd, bd.s, bd.s + PROLONGE + 2.0), 0.0, 0.0);
        let (_, mut droite_g) = bords(&axe_bras(bg, bg.s, bg.s + PROLONGE + 2.0), 0.0, 0.0);
        droite_g.reverse();
        let mut entre = gauche_d;
        entre.extend(droite_g);
        coupe.push(entre);
    }
    orienter(&mut coupe);
    let plaque = ferme.overlay(&coupe, OverlayRule::Difference, FillRule::NonZero);
    let plaque: Vec<Vec<Anneau>> = plaque.into_iter().filter(|f| !f.is_empty()).collect();
    if plaque.len() != 1 {
        if suivi {
            for f in &plaque {
                let a: Vec<[f32; 2]> = f[0].iter().map(|p| [p[0] as f32, p[1] as f32]).collect();
                eprintln!("  morceau de {:.1} m² en ({:.1}, {:.1})", aire(&a).abs(), a[0][0], a[0][1]);
            }
            for &k in &interieurs {
                let p = &voies[k].points;
                eprintln!("  intérieur {k} : ({:.1}, {:.1}) → ({:.1}, {:.1}), l {:.1}", p[0][0], p[0][1], p[p.len() - 1][0], p[p.len() - 1][1], 2.0 * p[0][6]);
            }
            for b in &bras {
                let (q, d) = b.a(b.s);
                eprintln!("  bras {} : bouche à {:.1} m en ({:.1}, {:.1}) vers ({:.2}, {:.2}), l {:.1}", b.k, b.s, q[0], q[1], d[0], d[1], 2.0 * q[6]);
            }
        }
        return echec(&format!("plaque en {} morceaux", plaque.len()));
    }
    let anneaux: Vec<Vec<[f32; 2]>> = plaque[0].iter().map(|a| a.iter().map(|p| [p[0] as f32, p[1] as f32]).collect()).collect();
    let mut exterieur = anneaux[0].clone();
    if aire(&exterieur) < 0.0 {
        exterieur.reverse();
    }
    // Les bouches remises au bit près : le bord droit suivi du bord gauche
    let mut sections = Vec::new();
    for b in &bras {
        let (q, d) = b.a(b.s);
        let o = [q[0], q[1]];
        let (bd, bg) = (plus(o, droite(d), q[6]), plus(o, gauche(d), q[6]));
        let proche = |c: [f32; 2]| (0..exterieur.len()).filter(|&i| dist(exterieur[i], c) < 0.05).min_by(|&i, &j| dist(exterieur[i], c).total_cmp(&dist(exterieur[j], c)));
        let (Some(id), Some(ig)) = (proche(bd), proche(bg)) else {
            return echec(&format!("coin de bouche introuvable (bras {} à {:.1} m)", b.k, b.s));
        };
        if (id + 1) % exterieur.len() != ig {
            return echec("bouche non consécutive");
        }
        sections.push((id, q, bd, bg));
    }
    // L'anneau extérieur commencé à une bouche, les sections posées
    let n = exterieur.len();
    let depart = sections[0].0;
    let mut contour: Vec<[f32; 2]> = Vec::with_capacity(n + sections.len());
    let mut fixes_c: HashSet<usize> = HashSet::new();
    let mut indices = Vec::new();
    for t in 0..n {
        let i = (depart + t) % n;
        if let Some((j, &(_, q, bd, _))) = sections.iter().enumerate().find(|(_, s)| s.0 == i) {
            indices.push((j, contour.len()));
            fixes_c.insert(contour.len());
            fixes_c.insert(contour.len() + 1);
            contour.push(bd);
            contour.push([q[0], q[1]]);
        } else if let Some(&(_, _, _, bg)) = sections.iter().find(|s| (s.0 + 1) % n == i) {
            contour.push(bg);
        } else {
            contour.push(exterieur[i]);
        }
    }
    let (contour, nouveaux) = recouper(&contour, &fixes_c);
    let mut bouches: Vec<(usize, Bouche)> = Vec::with_capacity(bras.len());
    let mut par_bras: Vec<Option<Bouche>> = vec![None; bras.len()];
    for (j, k) in indices {
        let b = &bras[j];
        let (_, q, bd, bg) = sections[j];
        let (g2, d2) = if b.debut { (bg, bd) } else { (bd, bg) };
        let bo = Bouche { q, g: g2, m: [q[0], q[1]], d: d2 };
        bouches.push((nouveaux[&k], bo));
        par_bras[j] = Some(bo);
    }
    let trous: Vec<Vec<[f32; 2]>> = anneaux[1..]
        .iter()
        .map(|a| {
            let mut a = a.clone();
            if aire(&a) > 0.0 {
                a.reverse();
            }
            recouper(&a, &HashSet::new()).0
        })
        .collect();
    let mut tous = vec![contour.clone()];
    tous.extend(trous.iter().cloned());
    if !crate::materialisation::maillage_valide(&tous) {
        return echec("maillage impossible");
    }
    let plans = g.iter().filter_map(|&n| plan(r, n)).collect::<Vec<_>>();
    if plans.is_empty() || par_bras.iter().any(|b| b.is_none()) {
        return echec("sans plan ou bouche manquante");
    }
    // Le groupe est fait : ses coupes et ses absorptions
    for (b, bo) in bras.iter().zip(&par_bras) {
        coupes[b.k][usize::from(!b.debut)] = Some((b.s, bo.unwrap()));
    }
    for &k in &interieurs {
        absorbees[k] = true;
    }
    let mut bt = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    for p in &contour {
        bt = [bt[0].min(p[0]), bt[1].min(p[1]), bt[2].max(p[0]), bt[3].max(p[1])];
    }
    let c = [(boite[0] + boite[2]) / 2.0, (boite[1] + boite[3]) / 2.0];
    Some(Carrefour { noeuds: g.to_vec(), c, plans, contour, trous, bouches, boite: bt, aplanir: true })
}
