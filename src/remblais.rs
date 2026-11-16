//! Le contour à plat du remblai des quais, vu de dessus. Chaque tronçon de quai (une
//! suite de côtés d'une berge dont le type est un quai, `est_quai`) est traité comme un
//! trait épais, à la manière du « contour vers chemin » d'Inkscape : décalé de
//! `REMBLAI_EAU` mètres vers l'eau et de `REMBLAI_TERRE` mètres vers la terre, joints
//! arrondis. Les morceaux (un quadrilatère par côté, un éventail par sommet) sont
//! réunis par `i_overlay` : les recouvrements (un décrochement du quai) sont résolus à
//! plat, il reste un polygone simple par tronçon (avec ses trous éventuels). Ce contour
//! sera extrudé en un seul volume.

use crate::berges::Berge;
use crate::carte::Donnees;
use i_overlay::core::fill_rule::FillRule;
use i_overlay::float::simplify::SimplifyShape;

/// Le remblai déborde de la ligne de berge d'autant vers l'eau et vers la terre (m).
pub const REMBLAI_EAU: f32 = 2.0;
pub const REMBLAI_TERRE: f32 = 2.9;
/// Un tronçon ouvert est prolongé d'autant à chaque bout (m).
const PROLONGE: f32 = 0.5;
/// Chaque côté chevauche ses voisins d'autant (m).
const CHEVAUCHE: f32 = 0.1;
/// Les arcs des joints arrondis : un segment tous les `ARC` radians au plus.
const ARC: f32 = 0.35;

/// Une berge de type `q` est-elle un quai (le gris de la carte) ?
pub fn est_quai(q: f32) -> bool {
    q >= 0.5
}

/// Le contour d'un remblai : son anneau extérieur, puis ses trous ; et sa boîte.
#[derive(Clone, Debug, Default)]
pub struct Contour {
    pub anneaux: Vec<Vec<[f32; 2]>>,
    pub boite: [f32; 4],
}

/// Les contours des remblais de tous les quais du lieu : les morceaux de tous les
/// tronçons réunis d'un coup (deux tronçons qui se touchent, d'anneaux voisins, ne
/// font qu'un remblai).
pub fn calculer(d: &Donnees) -> Vec<Contour> {
    let types = d.berges();
    let mut morceaux = Vec::new();
    for g in types.iter().flatten() {
        for t in troncons(g) {
            morceaux.extend(morceaux_troncon(g, &t));
        }
    }
    reunir(morceaux)
}

/// Les tronçons de quai d'une berge : des suites de côtés (le côté `k` va du point `k`
/// au suivant) ; `fermee` si toute la berge est un quai.
pub struct Troncon {
    pub cotes: Vec<usize>,
    pub fermee: bool,
}

fn troncons(g: &Berge) -> Vec<Troncon> {
    let m = g.points.len();
    if m < 2 || g.quai.len() != m || g.terre.len() != m {
        return Vec::new();
    }
    let quai = |k: usize| est_quai((g.quai[k] + g.quai[(k + 1) % m]) / 2.0);
    let Some(debut) = (0..m).find(|&k| !quai(k)) else {
        return vec![Troncon { cotes: (0..m).collect(), fermee: true }];
    };
    let mut v = Vec::new();
    let mut cours: Vec<usize> = Vec::new();
    for j in 1..=m {
        let k = (debut + j) % m;
        if quai(k) {
            cours.push(k);
        } else if !cours.is_empty() {
            v.push(Troncon { cotes: std::mem::take(&mut cours), fermee: false });
        }
    }
    if !cours.is_empty() {
        v.push(Troncon { cotes: cours, fermee: false });
    }
    v
}

/// Un contour orienté dans le sens direct (aire positive).
fn direct(mut c: Vec<[f64; 2]>) -> Option<Vec<[f64; 2]>> {
    let mut a = 0.0;
    for i in 0..c.len() {
        let (p, q) = (c[i], c[(i + 1) % c.len()]);
        a += p[0] * q[1] - q[0] * p[1];
    }
    if a.abs() < 1e-6 {
        return None;
    }
    if a < 0.0 {
        c.reverse();
    }
    Some(c)
}

/// L'éventail arrondi en `p` de la normale `a` à la normale `b`, au rayon `r`.
fn eventail(p: [f32; 2], a: [f32; 2], b: [f32; 2], r: f32) -> Option<Vec<[f64; 2]>> {
    let (ta, tb) = (a[1].atan2(a[0]), b[1].atan2(b[0]));
    let mut d = tb - ta;
    while d > std::f32::consts::PI {
        d -= 2.0 * std::f32::consts::PI;
    }
    while d < -std::f32::consts::PI {
        d += 2.0 * std::f32::consts::PI;
    }
    if d.abs() < 1e-4 {
        return None;
    }
    let n = (d.abs() / ARC).ceil().max(1.0) as usize;
    let mut c = vec![[p[0] as f64, p[1] as f64]];
    for k in 0..=n {
        let t = ta + d * k as f32 / n as f32;
        c.push([(p[0] + r * t.cos()) as f64, (p[1] + r * t.sin()) as f64]);
    }
    direct(c)
}

/// Le contour du remblai d'un tronçon : ses morceaux réunis.
#[cfg(test)]
fn contour(g: &Berge, t: &Troncon) -> Vec<Contour> {
    reunir(morceaux_troncon(g, t))
}

/// Les morceaux du remblai d'un tronçon : un quadrilatère par côté, un éventail
/// arrondi de chaque côté à chaque sommet intérieur.
fn morceaux_troncon(g: &Berge, t: &Troncon) -> Vec<Vec<[f64; 2]>> {
    let m = g.points.len();
    let mut morceaux: Vec<Vec<[f64; 2]>> = Vec::new();
    let pt = |p: [f32; 2], n: [f32; 2], o: f32| [(p[0] + n[0] * o) as f64, (p[1] + n[1] * o) as f64];
    let dernier = t.cotes.len() - 1;
    for (i, &k) in t.cotes.iter().enumerate() {
        let (mut p, mut q, n) = (g.points[k], g.points[(k + 1) % m], g.terre[k]);
        // Un tronçon ouvert est prolongé à ses bouts : deux tronçons voisins se
        // chevauchent, la réunion les fond
        // Et chaque côté chevauche ses voisins de `CHEVAUCHE` : deux quadrilatères qui ne
        // se toucheraient que par une arête (deux côtés alignés, sans joint) laisseraient
        // une fente
        let l = ((q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2)).sqrt().max(1e-6);
        let u = [(q[0] - p[0]) / l, (q[1] - p[1]) / l];
        let (a, b) = (if i == 0 && !t.fermee { PROLONGE } else { CHEVAUCHE }, if i == dernier && !t.fermee { PROLONGE } else { CHEVAUCHE });
        p = [p[0] - u[0] * a, p[1] - u[1] * a];
        q = [q[0] + u[0] * b, q[1] + u[1] * b];
        if let Some(c) = direct(vec![pt(p, n, -REMBLAI_EAU), pt(q, n, -REMBLAI_EAU), pt(q, n, REMBLAI_TERRE), pt(p, n, REMBLAI_TERRE)]) {
            morceaux.push(c);
        }
    }
    // Les joints arrondis, des deux côtés, entre deux côtés qui se suivent
    let joints = t.cotes.len() - usize::from(!t.fermee);
    for i in 0..joints {
        let (a, b) = (t.cotes[i], t.cotes[(i + 1) % t.cotes.len()]);
        let p = g.points[b];
        let (na, nb) = (g.terre[a], g.terre[b]);
        let eau = |n: [f32; 2]| [-n[0], -n[1]];
        morceaux.extend(eventail(p, na, nb, REMBLAI_TERRE));
        morceaux.extend(eventail(p, eau(na), eau(nb), REMBLAI_EAU));
    }
    morceaux
}

/// Des morceaux réunis à plat (règle non nulle) : des polygones simples, avec leurs
/// trous.
fn reunir(morceaux: Vec<Vec<[f64; 2]>>) -> Vec<Contour> {
    if morceaux.is_empty() {
        return Vec::new();
    }
    morceaux
        .simplify_shape(FillRule::NonZero)
        .into_iter()
        .filter(|forme| !forme.is_empty())
        .map(|forme| {
            let anneaux: Vec<Vec<[f32; 2]>> = forme.iter().map(|c| c.iter().map(|p| [p[0] as f32, p[1] as f32]).collect()).collect();
            let boite = anneaux[0].iter().fold([f32::MAX, f32::MAX, f32::MIN, f32::MIN], |b, p| [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])]);
            Contour { anneaux, boite }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Une berge droite le long de x, la terre vers +z, avec un décrochement
    /// rectangulaire de 3 m vers la terre au milieu.
    fn berge_a_decrochement() -> Berge {
        let points = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 3.0], [13.0, 3.0], [13.0, 0.0], [25.0, 0.0]];
        let terre = vec![[0.0, 1.0], [-1.0, 0.0], [0.0, 1.0], [1.0, 0.0], [0.0, 1.0], [0.0, 1.0]];
        let m = points.len();
        Berge { points, quai: vec![1.0; m], terre, crete: vec![12.0; m] }
    }

    #[test]
    fn le_remblai_d_un_quai_a_decrochement_est_un_seul_polygone() {
        let g = berge_a_decrochement();
        let t = Troncon { cotes: (0..5).collect(), fermee: false };
        let c = contour(&g, &t);
        assert_eq!(c.len(), 1, "{c:?}");
        let ext = &c[0].anneaux[0];
        // Il déborde vers l'eau et vers la terre
        assert!(ext.iter().any(|p| p[1] < -REMBLAI_EAU + 0.01));
        assert!(ext.iter().any(|p| p[1] > 3.0 + REMBLAI_TERRE - 0.01));
        // Un polygone simple : pas deux côtés qui se croisent
        let n = ext.len();
        let croise = |a: [f32; 2], b: [f32; 2], c: [f32; 2], d: [f32; 2]| {
            let s = |p: [f32; 2], q: [f32; 2], r: [f32; 2]| (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0]);
            s(a, b, c) * s(a, b, d) < -1e-6 && s(c, d, a) * s(c, d, b) < -1e-6
        };
        for i in 0..n {
            for j in i + 2..n {
                if i == 0 && j == n - 1 {
                    continue;
                }
                assert!(!croise(ext[i], ext[(i + 1) % n], ext[j], ext[(j + 1) % n]), "côtés {i} et {j}");
            }
        }
    }

    #[test]
    fn les_troncons_suivent_le_type_de_la_berge() {
        let mut g = berge_a_decrochement();
        // Côtés quai (moyenne ≥ 0,5) : 1, 2 et 4, 5
        g.quai = vec![0.0, 0.0, 1.0, 0.0, 0.0, 1.0];
        let t = troncons(&g);
        assert_eq!(t.len(), 2);
        assert!(t.iter().all(|t| !t.fermee));
        g.quai = vec![1.0; 6];
        let t = troncons(&g);
        assert_eq!(t.len(), 1);
        assert!(t[0].fermee);
    }
}
