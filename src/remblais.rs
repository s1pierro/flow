//! Le contour à plat du remblai des quais, vu de dessus. Chaque tronçon de quai (une
//! suite de côtés d'une berge dont le type est un quai, `est_quai`) est traité comme un
//! trait épais, à la manière du « contour vers chemin » d'Inkscape : décalé de
//! `REMBLAI_EAU` mètres vers l'eau et de `REMBLAI_TERRE` mètres vers la terre, joints
//! arrondis. Les morceaux (un quadrilatère par côté, un éventail par sommet) sont
//! réunis par `i_overlay` : les recouvrements (un décrochement du quai) sont résolus à
//! plat, il reste un polygone simple par tronçon (avec ses trous éventuels). Ce contour
//! est extrudé en un seul volume (`Remblais::volume`) : son plan triangulé par
//! `i_triangle`, le dessus à la hauteur du quai, le dessous sous le lit, des murs sur
//! son pourtour.

use crate::berges::Berge;
use crate::booleens::Forme;
use crate::carte::Donnees;
use i_overlay::core::fill_rule::FillRule;
use i_overlay::float::simplify::SimplifyShape;
use i_triangle::float::triangulatable::Triangulatable;
use std::collections::HashMap;

/// Le remblai déborde de la ligne de berge d'autant vers l'eau et vers la terre (m).
pub const REMBLAI_EAU: f32 = 2.0;
pub const REMBLAI_TERRE: f32 = 2.9;
/// Le quai : sa hauteur au-dessus de la crête de la berge (le relief à 8 m de la rive,
/// lissé ; réglable). Le dessus du remblai la dépasse de `REMBLAI_HAUT` ;
/// l'épaulement, plus large que lui vers la terre, l'arase à la hauteur du quai.
pub const DECALAGE_QUAI: f32 = 0.0;
pub const REMBLAI_HAUT: f32 = 1.0;
/// L'épaulement des quais : la terre arasée à la hauteur du quai sur `EPAULEMENT`
/// mètres derrière sa ligne (au-delà de la largeur du remblai vers la terre) ; sa
/// bande déborde de `EPAULEMENT_EAU` dans l'eau, au-delà du remblai : elle l'englobe
/// tout entier (la défonce passe ensuite dans l'eau).
pub const EPAULEMENT: f32 = 3.0;
pub const EPAULEMENT_EAU: f32 = REMBLAI_EAU + 0.5;
/// La case de l'index des stations des quais (m).
const CASE: f32 = 16.0;
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

/// Les remblais des quais du lieu : leurs contours, et les stations des quais (point,
/// crête) indexées par case, pour la hauteur de leur dessus.
#[derive(Debug, Default)]
pub struct Remblais {
    pub contours: Vec<Contour>,
    /// Les contours de l'épaulement des quais : la bande de `EPAULEMENT` mètres derrière
    /// la ligne du quai, arasée à sa hauteur.
    pub epaulements: Vec<Contour>,
    stations: HashMap<(i32, i32), Vec<([f32; 2], f32)>>,
    /// Les côtés des lignes de quai (début, fin, abscisse du début le long de son
    /// tronçon), indexés par case : le repère du dessus du quai (`repere`).
    cotes: HashMap<(i32, i32), Vec<([f32; 2], [f32; 2], f32)>>,
}

/// Les remblais de tous les quais du lieu : les morceaux de tous les tronçons réunis
/// d'un coup (deux tronçons qui se touchent, d'anneaux voisins, ne font qu'un
/// remblai).
pub fn calculer(d: &Donnees) -> Remblais {
    let types = d.berges();
    let mut morceaux = Vec::new();
    let mut epaulement = Vec::new();
    let mut stations: HashMap<(i32, i32), Vec<([f32; 2], f32)>> = HashMap::new();
    let mut cotes: HashMap<(i32, i32), Vec<([f32; 2], [f32; 2], f32)>> = HashMap::new();
    for g in types.iter().flatten() {
        let m = g.points.len();
        for t in troncons(g) {
            // Les côtés du tronçon, et leur abscisse le long de lui
            let mut s = 0.0;
            for &k in &t.cotes {
                let (a, b) = (g.points[k], g.points[(k + 1) % m]);
                let (c0, c1) = (case([a[0].min(b[0]), a[1].min(b[1])]), case([a[0].max(b[0]), a[1].max(b[1])]));
                for i in c0.0..=c1.0 {
                    for j in c0.1..=c1.1 {
                        cotes.entry((i, j)).or_default().push((a, b, s));
                    }
                }
                s += ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
            }
            morceaux.extend(morceaux_troncon(g, &t, REMBLAI_EAU, REMBLAI_TERRE));
            epaulement.extend(morceaux_troncon(g, &t, EPAULEMENT_EAU, EPAULEMENT));
            for &k in &t.cotes {
                for i in [k, (k + 1) % m] {
                    let p = g.points[i];
                    stations.entry(case(p)).or_default().push((p, g.crete[i]));
                }
            }
        }
    }
    Remblais { contours: reunir(morceaux), epaulements: reunir(epaulement), stations, cotes }
}

fn case(p: [f32; 2]) -> (i32, i32) {
    ((p[0] / CASE).floor() as i32, (p[1] / CASE).floor() as i32)
}

impl Remblais {
    /// La hauteur du dessus du remblai en `p` : celle du quai à la station la plus
    /// proche, plus `REMBLAI_HAUT`.
    pub fn hauteur(&self, p: [f32; 2]) -> Option<f32> {
        self.quai(p).map(|h| h + REMBLAI_HAUT)
    }

    /// Le repère du dessus du quai en `p` : l'abscisse le long de la ligne de quai la
    /// plus proche (m), et la distance à elle (m) ; rien à plus de deux cases d'elle.
    /// Les pavés s'y alignent sur le quai, la bordure suit son bord.
    pub fn repere(&self, p: [f32; 2]) -> Option<(f32, f32)> {
        let (ci, cj) = case(p);
        let mut best: Option<(f32, f32)> = None;
        for i in ci - 1..=ci + 1 {
            for j in cj - 1..=cj + 1 {
                for &(a, b, s) in self.cotes.get(&(i, j)).into_iter().flatten() {
                    let (ux, uz) = (b[0] - a[0], b[1] - a[1]);
                    let l2 = ux * ux + uz * uz;
                    let t = if l2 > 1e-12 { (((p[0] - a[0]) * ux + (p[1] - a[1]) * uz) / l2).clamp(0.0, 1.0) } else { 0.0 };
                    let (qx, qz) = (a[0] + ux * t, a[1] + uz * t);
                    let d = ((p[0] - qx).powi(2) + (p[1] - qz).powi(2)).sqrt();
                    if best.is_none_or(|b| d < b.1) {
                        best = Some((s + t * l2.sqrt(), d));
                    }
                }
            }
        }
        best
    }

    /// La hauteur du quai en `p` : la crête à la station la plus proche, plus
    /// `DECALAGE_QUAI`.
    pub fn quai(&self, p: [f32; 2]) -> Option<f32> {
        let (ci, cj) = case(p);
        for r in 1..6 {
            let mut best: Option<(f32, f32)> = None;
            for i in ci - r..=ci + r {
                for j in cj - r..=cj + r {
                    for &(q, c) in self.stations.get(&(i, j)).into_iter().flatten() {
                        let d2 = (q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2);
                        if best.is_none_or(|b| d2 < b.0) {
                            best = Some((d2, c));
                        }
                    }
                }
            }
            if let Some((_, c)) = best {
                return Some(c + DECALAGE_QUAI);
            }
        }
        None
    }

    /// Le volume du remblai de contour `c` : son plan triangulé, le dessus à la hauteur
    /// du quai (point par point), le dessous à `bas`, des murs sur les côtés de bord. Une
    /// variété fermée, faces dehors ; vide si la triangulation échoue.
    pub fn volume(&self, c: &Contour, bas: f32) -> Forme {
        extruder(c, |q| self.hauteur(q).unwrap_or(bas + 1.0).max(bas + 0.5), |_| bas).en(crate::maquette::matiere::REMBLAI)
    }

    /// Le volume de l'épaulement de contour `c` : le dessous à la hauteur du quai
    /// (point par point), le dessus à `haut`. Ôté au solide, il arase la terre et le
    /// remblai à la hauteur du quai.
    pub fn volume_epaulement(&self, c: &Contour, haut: f32) -> Forme {
        use crate::maquette::matiere;
        let f = extruder(c, |_| haut, |q| self.quai(q).unwrap_or(haut - 1.0).min(haut - 0.5));
        // Le dessus (en haut), le fond (le quai), les murs : selon la pente des faces
        let mut m = Vec::with_capacity(f.faces.len());
        for p in &f.faces {
            let ys: Vec<f32> = p.iter().map(|&i| f.sommets[i as usize][1]).collect();
            m.push(if ys.iter().all(|&y| y >= haut - 1e-3) {
                matiere::HAUT
            } else if p.len() == 3 {
                matiere::QUAI
            } else {
                matiere::QUAI_MUR
            });
        }
        Forme { matieres: m, ..f }
    }
}

/// Le contour `c` extrudé : son plan triangulé (`i_triangle`), le dessus à `dessus(p)`,
/// le dessous à `dessous(p)`, des murs sur les côtés de bord. Une variété fermée, faces
/// dehors ; vide si la triangulation échoue.
pub fn extruder(c: &Contour, dessus: impl Fn([f32; 2]) -> f32, dessous: impl Fn([f32; 2]) -> f32) -> Forme {
    {
        let forme: Vec<Vec<[f64; 2]>> = c.anneaux.iter().map(|a| a.iter().map(|p| [p[0] as f64, p[1] as f64]).collect()).collect();
        let t = forme.triangulate().to_triangulation::<u32>();
        let n = t.points.len() as u32;
        if n < 3 || t.indices.len() < 3 {
            return Forme::default();
        }
        let mut f = Forme::default();
        for p in &t.points {
            let q = [p[0] as f32, p[1] as f32];
            f.sommets.push([q[0], dessus(q), q[1]]);
        }
        for p in &t.points {
            let q = [p[0] as f32, p[1] as f32];
            f.sommets.push([q[0], dessous(q), q[1]]);
        }
        // Les triangles du plan : vus d'en haut dans le sens direct (x, z), donc
        // tournés vers le bas en 3D (y vers le haut, z vers le bas de l'écran) ; on
        // corrige l'orientation à la fin par le signe du volume
        let mut aretes: HashMap<(u32, u32), u32> = HashMap::new();
        for tri in t.indices.chunks(3) {
            let (a, b, c) = (tri[0], tri[1], tri[2]);
            f.faces.push(vec![a, b, c]);
            f.faces.push(vec![c + n, b + n, a + n]);
            for (u, v) in [(a, b), (b, c), (c, a)] {
                *aretes.entry((u, v)).or_insert(0) += 1;
            }
        }
        // Les murs : sur les côtés de bord (sans leur retourné), dans le sens du dessus
        for &(u, v) in aretes.keys() {
            if !aretes.contains_key(&(v, u)) {
                f.faces.push(vec![v, u, u + n, v + n]);
            }
        }
        if f.volume() < 0.0 {
            f.faces.iter_mut().for_each(|p| p.reverse());
        }
        f
    }
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
    reunir(morceaux_troncon(g, t, REMBLAI_EAU, REMBLAI_TERRE))
}

/// Les morceaux du remblai d'un tronçon : un quadrilatère par côté, un éventail
/// arrondi de chaque côté à chaque sommet intérieur.
fn morceaux_troncon(g: &Berge, t: &Troncon, vers_eau: f32, vers_terre: f32) -> Vec<Vec<[f64; 2]>> {
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
        if let Some(c) = direct(vec![pt(p, n, -vers_eau), pt(q, n, -vers_eau), pt(q, n, vers_terre), pt(p, n, vers_terre)]) {
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
        morceaux.extend(eventail(p, na, nb, vers_terre));
        morceaux.extend(eventail(p, eau(na), eau(nb), vers_eau));
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
    fn le_remblai_extrude_est_un_volume_ferme() {
        let g = berge_a_decrochement();
        let t = Troncon { cotes: (0..5).collect(), fermee: false };
        let contours = contour(&g, &t);
        let mut stations: HashMap<(i32, i32), Vec<([f32; 2], f32)>> = HashMap::new();
        for p in &g.points {
            stations.entry(case(*p)).or_default().push((*p, 12.0));
        }
        let r = Remblais { contours, epaulements: Vec::new(), stations, cotes: HashMap::new() };
        let f = r.volume(&r.contours[0], 5.0);
        assert!(f.volume() > 0.0);
        // Fermé : chaque côté une fois dans chaque sens
        let mut cotes = HashMap::new();
        for p in &f.faces {
            for k in 0..p.len() {
                *cotes.entry((p[k], p[(k + 1) % p.len()])).or_insert(0) += 1;
            }
        }
        assert!(cotes.iter().all(|(&(a, b), &n)| n == 1 && cotes.get(&(b, a)) == Some(&1)));
        assert!(f.sommets.iter().any(|p| (p[1] - 12.0 - REMBLAI_HAUT).abs() < 1e-4) && f.sommets.iter().any(|p| p[1] == 5.0));
        // Les booléens le prennent
        let boite = crate::booleens::operation(&f, &f, crate::booleens::Op::Intersection);
        assert!(boite.is_ok(), "{boite:?}");
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
