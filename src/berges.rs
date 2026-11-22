//! Le type des berges : quai ou berge naturelle, le long des contours des surfaces
//! d'eau d'OSM. Chaque côté d'un anneau est regardé tous les `ECHANTILLON` mètres, du
//! côté de la terre : c'est un quai si la ville y est (sol habité ou zone d'activité,
//! une route ou un bâtiment tout près) ou si la rive est raide ; sinon une berge
//! naturelle (bois, prés, champs, herbe). Les valeurs, lissées le long de l'anneau sur
//! `FENETRE` mètres, passent de l'un à l'autre en douceur : 1 pour un quai, 0 pour une
//! berge naturelle, une valeur entre deux sur la transition.

use crate::carte::Donnees;
use sage_reel::{CHEMIN, HABITE, PIETONNE, SENTIER, ZONE};

/// Le pas des échantillons le long d'une berge (m).
pub const ECHANTILLON: f32 = 8.0;
/// À quelle distance de la rive on regarde le sol (m).
pub const COTE: f32 = 5.0;
/// Une route ou un bâtiment à moins de cette distance du point regardé : un quai (m).
pub const PROCHE: f32 = 12.0;
/// La terre plus haute que l'eau d'autant, à 8 m de la rive : une rive raide (m).
pub const RAIDE: f32 = 3.0;
/// La longueur du lissage le long de la rive (m).
pub const FENETRE: f32 = 30.0;

/// À quelle distance de la rive on prend la hauteur de la terre pour la crête (m).
pub const CRETE_A: f32 = 8.0;

/// Une berge : un anneau d'OSM redécoupé en points d'au plus `ECHANTILLON` mètres
/// (les siens gardés), et en chacun sa valeur, de 0 (berge naturelle) à 1 (quai) ;
/// la normale du pas qui le suit, tournée vers la terre ; la hauteur de la crête du
/// remblai, lissée le long de la rive comme le type.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Berge {
    pub points: Vec<[f32; 2]>,
    pub quai: Vec<f32>,
    pub terre: Vec<[f32; 2]>,
    pub crete: Vec<f32>,
}

/// Pour chaque surface d'eau du lieu, chacun de ses anneaux.
pub type Types = Vec<Vec<Berge>>;

pub fn calculer(d: &Donnees) -> Types {
    d.lieu.surfaces_eau.iter().map(|s| s.anneaux.iter().map(|r| anneau(d, r, &s.anneaux)).collect()).collect()
}

fn dist_segment(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
    let l2 = dx * dx + dz * dz;
    let t = if l2 > 0.0 { (((p[0] - a[0]) * dx + (p[1] - a[1]) * dz) / l2).clamp(0.0, 1.0) } else { 0.0 };
    ((a[0] + dx * t - p[0]).powi(2) + (a[1] + dz * t - p[1]).powi(2)).sqrt()
}

/// Le point est-il dans la surface d'anneaux `tous` (pair-impair) ?
fn dans(tous: &[Vec<[f32; 2]>], x: f32, z: f32) -> bool {
    let mut dedans = false;
    for r in tous {
        for i in 0..r.len() {
            let (p, q) = (r[i], r[(i + 1) % r.len()]);
            if (p[1] > z) != (q[1] > z) && x < p[0] + (z - p[1]) / (q[1] - p[1]) * (q[0] - p[0]) {
                dedans = !dedans;
            }
        }
    }
    dedans
}

/// Le côté de la terre de la normale `n` en `p` : 1 ou -1. Celui qui sort de la
/// surface d'eau `tous` ; s'ils en sortent tous deux (une surface étroite), celui où la
/// grille a le moins d'eau.
pub fn cote_terre(d: &Donnees, tous: &[Vec<[f32; 2]>], p: [f32; 2], n: [f32; 2]) -> f32 {
    let (a, b) = ([p[0] + n[0] * 0.5, p[1] + n[1] * 0.5], [p[0] - n[0] * 0.5, p[1] - n[1] * 0.5]);
    match (dans(tous, a[0], a[1]), dans(tous, b[0], b[1])) {
        (true, false) => return -1.0,
        (false, true) => return 1.0,
        _ => {}
    }
    let eau = |q: [f32; 2]| d.lieu.eau(q[0] as f64, q[1] as f64);
    let (a, b) = ([p[0] + n[0] * COTE, p[1] + n[1] * COTE], [p[0] - n[0] * COTE, p[1] - n[1] * COTE]);
    if eau(a) <= eau(b) { 1.0 } else { -1.0 }
}

/// 1 si le point `p` de la rive, de normale `n`, borde la ville ou une rive raide.
pub fn quai_en(d: &Donnees, tous: &[Vec<[f32; 2]>], p: [f32; 2], n: [f32; 2]) -> f32 {
    let l = &d.lieu;
    let s = cote_terre(d, tous, p, n);
    let t = [p[0] + n[0] * COTE * s, p[1] + n[1] * COTE * s];
    let classe = l.classe(t[0] as f64, t[1] as f64);
    if classe == HABITE || classe == ZONE {
        return 1.0;
    }
    let loin = [p[0] + n[0] * 8.0 * s, p[1] + n[1] * 8.0 * s];
    if l.relief(loin[0] as f64, loin[1] as f64) as f32 - l.niveau > RAIDE {
        return 1.0;
    }
    let b = [t[0] - PROCHE - 20.0, t[1] - PROCHE - 20.0, t[0] + PROCHE + 20.0, t[1] + PROCHE + 20.0];
    for k in d.routes_dans(b, false) {
        let r = &l.routes[k as usize];
        if r.genre == CHEMIN || r.genre == SENTIER || r.genre == PIETONNE {
            continue;
        }
        let m = PROCHE + r.largeur / 2.0;
        if r.points.windows(2).any(|w| dist_segment(t, [w[0][0], w[0][1]], [w[1][0], w[1][1]]) < m) {
            return 1.0;
        }
    }
    for k in d.batiments_dans(b) {
        let c = &l.batiments[k as usize].contour;
        if (0..c.len()).any(|i| dist_segment(t, c[i], c[(i + 1) % c.len()]) < PROCHE) {
            return 1.0;
        }
    }
    0.0
}

/// La berge d'un anneau (fermé sans répéter le premier point).
pub fn anneau(d: &Donnees, r: &[[f32; 2]], tous: &[Vec<[f32; 2]>]) -> Berge {
    let n = r.len();
    let mut points = Vec::new();
    let mut brut = Vec::new();
    let mut terre = Vec::new();
    let mut hauts = Vec::new();
    let l_ = &d.lieu;
    for i in 0..n {
        let (a, b) = (r[i], r[(i + 1) % n]);
        let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
        let l = (dx * dx + dz * dz).sqrt();
        if l < 1e-6 {
            continue;
        }
        let nn = [-dz / l, dx / l];
        let k = (l / ECHANTILLON).ceil().max(1.0) as usize;
        for j in 0..k {
            let t = j as f32 / k as f32;
            let p = [a[0] + dx * t, a[1] + dz * t];
            // Le point regardé : au milieu du pas qui suit (le coin regarde son côté)
            let t2 = (j as f32 + 0.5) / k as f32;
            let pm = [a[0] + dx * t2, a[1] + dz * t2];
            points.push(p);
            brut.push(quai_en(d, tous, pm, nn));
            let s = cote_terre(d, tous, pm, nn);
            let tn = [nn[0] * s, nn[1] * s];
            terre.push(tn);
            hauts.push(l_.relief((p[0] + tn[0] * CRETE_A) as f64, (p[1] + tn[1] * CRETE_A) as f64) as f32);
        }
    }
    let m = points.len();
    if m < 2 {
        return Berge { quai: vec![1.0; m], crete: hauts, terre, points };
    }
    // Le long de l'anneau, la moyenne à moins d'une demi-fenêtre de chaque point
    let pas: Vec<f32> = (0..m).map(|i| {
        let (a, b) = (points[i], points[(i + 1) % m]);
        ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt()
    }).collect();
    let tour: f32 = pas.iter().sum();
    let demi = (FENETRE / 2.0).min(tour / 2.0);
    let lisser = |v: &[f32]| -> Vec<f32> {
        (0..m).map(|i| {
            let (mut somme, mut nb) = (v[i], 1.0f32);
            for sens in [true, false] {
                let (mut j, mut parcouru) = (i, 0.0);
                for _ in 0..m {
                    let suivant = if sens { (j + 1) % m } else { (j + m - 1) % m };
                    parcouru += if sens { pas[j] } else { pas[suivant] };
                    if parcouru > demi || suivant == i {
                        break;
                    }
                    j = suivant;
                    somme += v[j];
                    nb += 1.0;
                }
            }
            somme / nb
        }).collect()
    };
    // Une transition franche plutôt qu'un entre-deux sur de longues distances
    let quai = lisser(&brut).into_iter().map(|v| {
        let t = ((v - 0.3) / 0.4).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }).collect();
    let crete = lisser(&hauts).into_iter().map(|h| h.max(l_.niveau + 0.15)).collect();
    Berge { points, quai, terre, crete }
}

/// Les longueurs de quais et de berges naturelles (m) : un côté compte pour sa valeur
/// moyenne.
pub fn longueurs(d: &Donnees) -> (f32, f32) {
    let types = d.berges();
    let (mut q, mut n) = (0.0, 0.0);
    for b in types.iter().flatten() {
        let (r, t) = (&b.points, &b.quai);
        {
            for i in 0..r.len() {
                let j = (i + 1) % r.len();
                let l = ((r[j][0] - r[i][0]).powi(2) + (r[j][1] - r[i][1]).powi(2)).sqrt();
                let v = (t[i] + t[j]) / 2.0;
                q += l * v;
                n += l * (1.0 - v);
            }
        }
    }
    (q, n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sage_reel::{cote, cours_eau::SurfaceEau, Lieu, Route, ROUTE};

    #[test]
    fn la_route_fait_le_quai_et_le_reste_est_naturel() {
        let pas = 4.0;
        let n = cote(pas);
        let h = vec![1.0; n * n];
        let mut eau = vec![false; n * n];
        // L'eau : une bande au milieu, de x = -100 à 100 (la grille, en points)
        let o = sage_reel::origine();
        for b in 0..n {
            for a in 0..n {
                let x = o + a as f64 * pas;
                if (-100.0..=100.0).contains(&x) {
                    eau[b * n + a] = true;
                }
            }
        }
        let mut l = Lieu::new("Essai", 49.0, 1.0, 0.0, 0.0, pas, 400.0, &h, &eau, Vec::new(), Vec::new(), Vec::new());
        l.niveau = 0.0;
        l.surfaces_eau = vec![SurfaceEau { nature: 0, anneaux: vec![vec![[-100.0, -180.0], [100.0, -180.0], [100.0, 180.0], [-100.0, 180.0]]] }];
        // Une route le long de la rive est, sur sa moitié nord
        l.routes = vec![Route { revetement: 0, files: Default::default(), genre: ROUTE, largeur: 8.0, points: vec![[110.0, -180.0, 1.0], [110.0, 0.0, 1.0]] }];
        let d = Donnees::new(l);
        let b = &d.berges()[0][0];
        let en = |x: f32, z: f32| {
            let i = (0..b.points.len()).min_by(|&i, &j| {
                let e = |k: usize| (b.points[k][0] - x).powi(2) + (b.points[k][1] - z).powi(2);
                e(i).total_cmp(&e(j))
            }).unwrap();
            b.quai[i]
        };
        assert!(b.points.len() > 100);
        // La rive est longe la route au nord (z < 0) : quai ; au sud et à l'ouest, naturelle
        assert!(en(100.0, -100.0) > 0.9, "{b:?}");
        assert!(en(100.0, 100.0) < 0.1 && en(-100.0, 0.0) < 0.1, "{b:?}");
        let (q, nat) = longueurs(&d);
        assert!(q > 100.0 && nat > 500.0, "{q} {nat}");
    }

    /// Banc : FLOW_LIEU ; les longueurs de quais et de berges naturelles.
    #[test]
    #[ignore]
    fn banc_berges() {
        let l = Lieu::lire(std::path::Path::new(&std::env::var("FLOW_LIEU").unwrap())).unwrap();
        let d = Donnees::new(l);
        let t0 = std::time::Instant::now();
        let b = d.berges();
        eprintln!("calcul {:?}", t0.elapsed());
        let (q, n) = longueurs(&d);
        eprintln!("quais {q:.0} m, naturelles {n:.0} m");
        for g in b.iter().flatten() {
            let mut morceaux = Vec::new();
            let mut debut = 0;
            for i in 1..=g.points.len() {
                let fin = i == g.points.len();
                if fin || (g.quai[i] >= 0.5) != (g.quai[debut] >= 0.5) {
                    let p = g.points[debut];
                    morceaux.push(format!("{}@{:.0},{:.0}x{}", if g.quai[debut] >= 0.5 { "Q" } else { "N" }, p[0], p[1], i - debut));
                    debut = i;
                }
            }
            eprintln!("{} points : {}", g.points.len(), morceaux.join(" "));
        }
    }
}