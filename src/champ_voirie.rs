//! Le champ de hauteur de la voirie au sol (0.135) : une seule surface pour toutes les
//! routes, lue par chaque volume de la Matérialisation (`materialisation::ruban`,
//! `dalle`) au lieu de la hauteur propre de chaque voie. Deux volumes qui se
//! recouvrent (tronçons d'une même rue, voies parallèles, couloirs de bus, carrefours)
//! lisent la même hauteur au même endroit : plus de marche entre eux.
//!
//! - Les hauteurs viennent des tracés lissés (`materialisation::lisser`, plans des
//!   carrefours compris), échantillonnés tous les `PAS_ECHANTILLON` mètres.
//! - Chaque nœud de la grille (un tous les `PAS` mètres) prend la moyenne des
//!   échantillons proches, pondérée par la distance (plein poids sur la largeur de la
//!   voie, puis décroissance sur `FRANGE` mètres) et par le rang de la voie (la route
//!   principale garde sa pente), puis deux passes de flou 3 × 3.
//! - La grille est faite par blocs de `BLOC` nœuds, à la demande, et gardée en mémoire
//!   (au plus `BLOCS_MAX`, puis tout est oublié) : un bloc ne dépend que des voies, il
//!   est le même quel que soit l'ordre des tuiles ou l'appareil (le renfort).
//!
//! Les ponts et les tunnels n'y sont pas : ils gardent leur propre profil.

use crate::materialisation::{lisser, plan, rang};
use sage_reel::reseau::{Passage, Reseau};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// L'écart entre deux nœuds de la grille (m).
pub const PAS: f32 = 1.0;
/// Les nœuds d'un bloc, de côté.
const BLOC: i32 = 32;
/// La marge d'un bloc (en nœuds) pour le flou (deux passes 3 × 3).
const MARGE: i32 = 2;
/// L'écart entre deux échantillons le long d'une voie (m).
const PAS_ECHANTILLON: f32 = 1.0;
/// Au-delà du bord d'une voie, son poids décroît jusqu'à zéro sur cette distance (m).
pub const FRANGE: f32 = 3.0;
/// Les blocs gardés en mémoire (16 Kio chacun) avant de tout oublier.
const BLOCS_MAX: usize = 6000;

/// Une voie au sol échantillonnée : points (x, z, h), demi-largeur, poids.
struct Voie {
    points: Vec<[f32; 3]>,
    w: f32,
    poids: f32,
}

pub struct Champ {
    voies: Vec<Voie>,
    /// Les voies dont la zone d'influence touche chaque bloc.
    index: HashMap<(i32, i32), Vec<u32>>,
    blocs: Mutex<HashMap<(i32, i32), Arc<Vec<f32>>>>,
}

/// Le poids d'une voie selon son rang : la principale l'emporte nettement.
fn poids(genre: u8) -> f32 {
    match rang(genre) {
        0 => 8.0,
        1 => 4.0,
        2 => 2.0,
        _ => 1.0,
    }
}

/// Le bloc d'un nœud de la grille.
fn bloc_de(i: i32) -> i32 {
    i.div_euclid(BLOC)
}

impl Champ {
    /// Le champ des voies au sol du réseau nivelé (sans les piétonnes).
    pub fn new(r: &Reseau) -> Champ {
        let mut voies = Vec::new();
        for a in r.aretes.iter().filter(|a| a.passage == Passage::Sol && a.genre != sage_reel::PIETONNE) {
            if a.points.len() < 2 {
                continue;
            }
            let l = lisser(&a.points, plan(r, a.a), plan(r, a.b));
            // Rééchantillonné tous les PAS_ECHANTILLON mètres
            let mut pts = vec![[l[0][0], l[0][1], l[0][2]]];
            for s in l.windows(2) {
                let d = (s[1][0] - s[0][0]).hypot(s[1][1] - s[0][1]);
                let n = (d / PAS_ECHANTILLON).ceil().max(1.0) as usize;
                for k in 1..=n {
                    let t = k as f32 / n as f32;
                    pts.push([s[0][0] + t * (s[1][0] - s[0][0]), s[0][1] + t * (s[1][1] - s[0][1]), s[0][2] + t * (s[1][2] - s[0][2])]);
                }
            }
            voies.push(Voie { points: pts, w: (a.largeur * 0.5).max(1.0), poids: poids(a.genre) });
        }
        let mut index: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        for (k, v) in voies.iter().enumerate() {
            let r = v.w + FRANGE + (MARGE as f32 + 1.0) * PAS;
            let mut b = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
            for p in &v.points {
                b = [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])];
            }
            let (i0, i1) = (bloc_de(((b[0] - r) / PAS).floor() as i32), bloc_de(((b[2] + r) / PAS).ceil() as i32));
            let (j0, j1) = (bloc_de(((b[1] - r) / PAS).floor() as i32), bloc_de(((b[3] + r) / PAS).ceil() as i32));
            // Seulement les blocs que la voie approche vraiment (les longues voies en
            // diagonale ne remplissent pas toute leur boîte)
            for bi in i0..=i1 {
                for bj in j0..=j1 {
                    let (x0, z0) = ((bi * BLOC) as f32 * PAS - r, (bj * BLOC) as f32 * PAS - r);
                    let (x1, z1) = (x0 + BLOC as f32 * PAS + 2.0 * r, z0 + BLOC as f32 * PAS + 2.0 * r);
                    if v.points.iter().any(|p| p[0] >= x0 && p[0] <= x1 && p[1] >= z0 && p[1] <= z1) {
                        index.entry((bi, bj)).or_default().push(k as u32);
                    }
                }
            }
        }
        Champ { voies, index, blocs: Mutex::new(HashMap::new()) }
    }

    /// Les hauteurs d'un bloc (NaN loin des voies), faites si besoin.
    fn bloc(&self, b: (i32, i32)) -> Option<Arc<Vec<f32>>> {
        let ids = self.index.get(&b)?;
        if let Some(v) = self.blocs.lock().ok()?.get(&b) {
            return Some(v.clone());
        }
        let v = Arc::new(self.faire_bloc(b, ids));
        if let Ok(mut m) = self.blocs.lock() {
            if m.len() >= BLOCS_MAX {
                m.clear();
            }
            m.insert(b, v.clone());
        }
        Some(v)
    }

    fn faire_bloc(&self, b: (i32, i32), ids: &[u32]) -> Vec<f32> {
        let n = (BLOC + 2 * MARGE) as usize;
        let (i0, j0) = (b.0 * BLOC - MARGE, b.1 * BLOC - MARGE);
        let mut somme = vec![0.0f64; n * n];
        let mut poids = vec![0.0f64; n * n];
        for &k in ids {
            let v = &self.voies[k as usize];
            let r = v.w + FRANGE;
            for p in &v.points {
                let (ia, ib) = (((p[0] - r) / PAS).floor() as i32 - i0, ((p[0] + r) / PAS).ceil() as i32 - i0);
                let (ja, jb) = (((p[1] - r) / PAS).floor() as i32 - j0, ((p[1] + r) / PAS).ceil() as i32 - j0);
                if ib < 0 || jb < 0 || ia >= n as i32 || ja >= n as i32 {
                    continue;
                }
                for j in ja.max(0)..=jb.min(n as i32 - 1) {
                    for i in ia.max(0)..=ib.min(n as i32 - 1) {
                        let (x, z) = ((i + i0) as f32 * PAS, (j + j0) as f32 * PAS);
                        let d = (x - p[0]).hypot(z - p[1]);
                        if d >= r {
                            continue;
                        }
                        let k = if d <= v.w { 1.0 } else { (1.0 - (d - v.w) / FRANGE).powi(2) };
                        let w = (k * v.poids) as f64;
                        let c = j as usize * n + i as usize;
                        somme[c] += w * p[2] as f64;
                        poids[c] += w;
                    }
                }
            }
        }
        let mut h: Vec<f32> = (0..n * n).map(|c| if poids[c] > 1e-9 { (somme[c] / poids[c]) as f32 } else { f32::NAN }).collect();
        // Deux passes de flou 3 × 3 (sur les nœuds tenus seulement)
        for _ in 0..2 {
            let mut f = h.clone();
            for j in 0..n {
                for i in 0..n {
                    let c = j * n + i;
                    if h[c].is_nan() {
                        continue;
                    }
                    let (mut s, mut m) = (0.0, 0.0);
                    for dj in -1i32..=1 {
                        for di in -1i32..=1 {
                            let (a, bb) = (i as i32 + di, j as i32 + dj);
                            if a < 0 || bb < 0 || a >= n as i32 || bb >= n as i32 {
                                continue;
                            }
                            let v = h[bb as usize * n + a as usize];
                            if !v.is_nan() {
                                s += v;
                                m += 1.0;
                            }
                        }
                    }
                    f[c] = s / m;
                }
            }
            h = f;
        }
        // Le bloc sans sa marge
        let m = MARGE as usize;
        let mut out = Vec::with_capacity((BLOC * BLOC) as usize);
        for j in 0..BLOC as usize {
            out.extend_from_slice(&h[(j + m) * n + m..(j + m) * n + m + BLOC as usize]);
        }
        out
    }

    /// La hauteur d'un nœud de la grille (NaN loin des voies).
    fn noeud(&self, i: i32, j: i32) -> f32 {
        let b = (bloc_de(i), bloc_de(j));
        match self.bloc(b) {
            Some(v) => v[(j - b.1 * BLOC) as usize * BLOC as usize + (i - b.0 * BLOC) as usize],
            None => f32::NAN,
        }
    }

    /// La hauteur de la voirie en (x, z) : bilinéaire entre les nœuds tenus ; rien loin
    /// des voies.
    pub fn h(&self, x: f32, z: f32) -> Option<f32> {
        let (fx, fz) = (x / PAS, z / PAS);
        let (i, j) = (fx.floor() as i32, fz.floor() as i32);
        let (tx, tz) = (fx - i as f32, fz - j as f32);
        let (mut s, mut m) = (0.0, 0.0);
        for (di, dj, w) in [(0, 0, (1.0 - tx) * (1.0 - tz)), (1, 0, tx * (1.0 - tz)), (0, 1, (1.0 - tx) * tz), (1, 1, tx * tz)] {
            let v = self.noeud(i + di, j + dj);
            if !v.is_nan() {
                s += w * v;
                m += w;
            }
        }
        (m > 1e-6).then(|| s / m)
    }
}
