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
use sage_reel::reseau::Reseau;
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
    // Les tronçons intérieurs et les bras
    let mut interieurs: Vec<usize> = Vec::new();
    let mut bras: Vec<Bras> = Vec::new();
    for &n in g {
        for &(k, debut) in &aux[&n] {
            let (a, b) = bouts[k];
            if lien(k) && dans.contains(&a) && dans.contains(&b) {
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
    // Les bouches : assez loin de toutes les autres emprises (bras voisins compris)
    let degage = |i: usize, s: f32, bras: &[Bras]| -> bool {
        let b = &bras[i];
        let (q, d) = b.a(s);
        let o = [q[0], q[1]];
        let coins = [plus(o, droite(d), q[6]), plus(o, gauche(d), q[6])];
        let marge = DEGAGEMENT * BORDURE;
        let loin = |a: [f32; 2], z: [f32; 2], w: f32| coins.iter().all(|&c| au_segment(c, a, z) - w > marge);
        // Le disque du nœud du bras : seulement en avant de lui
        fixes.iter().all(|&(a, z, w)| loin(a, z, w))
            && bras.iter().enumerate().filter(|&(j, _)| j != i).all(|(_, o)| {
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
                return echec("bouche sans dégagement");
            }
            bras[i].s = s;
        }
    }
    // Assez de place sur chaque bras, et au sol jusqu'au-delà de la bouche et du
    // prolongement
    for b in &bras {
        let autre = coupes[b.k][usize::from(b.debut)].map_or(0.0, |c| c.0);
        if b.longueur() < b.s + autre + RESTE || b.longueur() < b.s + PROLONGE + 1.0 {
            return echec("bras trop court");
        }
        if !b.pts.iter().zip(&b.s0).take_while(|(_, &s)| s <= b.s + PROLONGE + 1.0).all(|(p, _)| au_sol(p)) {
            return echec("bras pas au sol jusqu'au prolongement");
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
    let union = morceaux.simplify_shape(FillRule::NonZero);
    // 2. La fermeture
    let rb = BORDURE as f64;
    let large = union.outline(&OutlineStyle::new(rb).line_join(LineJoin::Round(0.2)));
    let ferme = large.outline(&OutlineStyle::new(-rb).line_join(LineJoin::Miter(0.1)));
    // 3. Les bras coupés à leur bouche
    let coupe: Vec<Anneau> = bras.iter().map(|b| bande(b, b.s, b.s + PROLONGE + 2.0, 0.5)).collect();
    let plaque = ferme.overlay(&coupe, OverlayRule::Difference, FillRule::NonZero);
    let plaque: Vec<Vec<Anneau>> = plaque.into_iter().filter(|f| !f.is_empty()).collect();
    if plaque.len() != 1 {
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
    Some(Carrefour { noeuds: g.to_vec(), c, plans, contour, trous, bouches, boite: bt })
}
