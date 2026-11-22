//! Les remblais des voies en surimpression sur la maquette : sous chaque voie du réseau
//! nivelé, une bande de la largeur de la voie, colorée selon son genre, du dessus (à
//! `DESSUS` + `HAUT` au-dessus de la chaussée) jusqu'à `SOUS` sous le terrain pour une
//! voie au sol ; pour un pont (une bretelle en l'air comprise) ou un tunnel, une dalle
//! de `OUVRAGE` mètres sous la chaussée, qui suit la voie sans descendre au terrain (un
//! tunnel, plus sombre, ne se voit qu'aux têtes et là où le relief le découvre). Rien
//! n'entre dans les tuiles : la 3D la pose par-dessus, tuile par tuile.
//!

use crate::carte::{boite_tuile, Cle, Donnees};
use sage_reel::reseau::{Passage, Reseau};

/// Des triangles aux sommets de `relief3d::SOMMET` flottants (position, normale,
/// couleur).
#[derive(Default)]
struct Scene {
    sommets: Vec<f32>,
    indices: Vec<u32>,
}

impl Scene {
    /// Un triangle à plat, de couleur `c` (normale de sa face).
    fn triangle(&mut self, a: [f32; 3], b: [f32; 3], d: [f32; 3], c: [f32; 3]) {
        let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let v = [d[0] - a[0], d[1] - a[1], d[2] - a[2]];
        let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
        let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-12);
        let n = n.map(|x| x / l);
        let i = (self.sommets.len() / crate::relief3d::SOMMET) as u32;
        for p in [a, b, d] {
            self.sommets.extend_from_slice(&[p[0], p[1], p[2], n[0], n[1], n[2], c[0], c[1], c[2]]);
        }
        self.indices.extend_from_slice(&[i, i + 1, i + 2]);
    }

    /// Un polygone plan (en éventail), de couleur `c`.
    fn polygone(&mut self, p: &[[f32; 3]], c: [f32; 3]) {
        for k in 1..p.len().saturating_sub(1) {
            self.triangle(p[0], p[k], p[k + 1], c);
        }
    }
}

/// Le dessus du tracé au-dessus de la chaussée (m), ce que le remblai dépasse au-dessus
/// de lui, ce qu'il descend sous le terrain, ce qu'il déborde de la voie de chaque côté.
pub const DESSUS: f32 = 1.0;
pub const HAUT: f32 = 0.5;
pub const SOUS: f32 = 0.3;
pub const MARGE: f32 = 0.0;
/// Ce que la dalle d'un pont ou d'un tunnel descend sous la chaussée (m), et
/// l'assombrissement d'un tunnel.
pub const OUVRAGE: f32 = 1.0;
pub const SOMBRE_TUNNEL: f32 = 0.6;

/// Le segment p → q (x, z, h) coupé à la boîte b (Liang–Barsky), ou rien.
fn couper(p: [f32; 3], q: [f32; 3], b: [f32; 4]) -> Option<([f32; 3], [f32; 3])> {
    let (dx, dz) = (q[0] - p[0], q[1] - p[1]);
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    for (pk, qk) in [(-dx, p[0] - b[0]), (dx, b[2] - p[0]), (-dz, p[1] - b[1]), (dz, b[3] - p[1])] {
        if pk == 0.0 {
            if qk < 0.0 {
                return None;
            }
        } else {
            let t = qk / pk;
            if pk < 0.0 {
                t0 = t0.max(t);
            } else {
                t1 = t1.min(t);
            }
        }
    }
    if t0 > t1 {
        return None;
    }
    let at = |t: f32| [p[0] + t * dx, p[1] + t * dz, p[2] + t * (q[2] - p[2])];
    Some((at(t0), at(t1)))
}

/// La couleur de la bande de remblai selon le genre de la voie (même ordre que
/// des genres de `sage_reel`), la plus distincte possible : terres pour les voies carrossables, gris
/// pour les chemins.
pub const REMBLAIS: [([f32; 3], &str); sage_reel::GENRES as usize] = [
    ([0.50, 0.33, 0.18], "grande : brun foncé"),
    ([0.74, 0.58, 0.38], "route : brun"),
    ([0.90, 0.62, 0.22], "service : ocre"),
    ([0.95, 0.89, 0.76], "piétonne : beige clair"),
    ([0.58, 0.58, 0.60], "chemin : gris"),
    ([0.52, 0.62, 0.30], "sentier : olive"),
    ([0.40, 0.50, 0.70], "bus : ardoise"),
    ([0.85, 0.75, 0.62], "rue piétonne : sable"),
];

/// Les suites de points (x, z, h) des arêtes au sol coupées à la boîte `b` : un
/// segment coupé à l'entrée ou à la sortie commence ou finit une suite.
fn suites(a: &sage_reel::reseau::Arete, b: [f32; 4]) -> Vec<Vec<[f32; 3]>> {
    let mut v: Vec<Vec<[f32; 3]>> = Vec::new();
    let mut cur: Vec<[f32; 3]> = Vec::new();
    for w in a.points.windows(2) {
        match couper(w[0], w[1], b) {
            Some((p, q)) => {
                if cur.last() != Some(&p) {
                    if cur.len() >= 2 {
                        v.push(std::mem::take(&mut cur));
                    }
                    cur.clear();
                    cur.push(p);
                }
                cur.push(q);
            }
            None => {
                if cur.len() >= 2 {
                    v.push(std::mem::take(&mut cur));
                }
                cur.clear();
            }
        }
    }
    if cur.len() >= 2 {
        v.push(cur);
    }
    v
}

/// La bande d'une suite de points, de demi-largeur `w` : dessus à la hauteur de la
/// voie, dessous `sous` mètres sous le terrain (ou sous la voie, si plus bas), murs sur
/// les côtés et aux bouts ; dans la scène `s`. Avec `dalle`, le dessous suit la voie à
/// `dalle` mètres sous elle (un pont, un tunnel).
fn bande(s: &mut Scene, d: &Donnees, q: &[[f32; 3]], w: f32, sous: f32, haut: f32, couleur: [f32; 3], dalle: Option<f32>) {
    let n = q.len();
    let normale = |a: [f32; 3], b: [f32; 3]| {
        let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
        let l = dx.hypot(dz).max(1e-6);
        (-dz / l, dx / l)
    };
    // Pour chaque point : gauche et droite, en haut et en bas
    let mut g = Vec::with_capacity(n);
    for i in 0..n {
        let n0 = if i > 0 { normale(q[i - 1], q[i]) } else { normale(q[0], q[1]) };
        let n1 = if i + 1 < n { normale(q[i], q[i + 1]) } else { n0 };
        let (sx, sz) = (n0.0 + n1.0, n0.1 + n1.1);
        let l = sx.hypot(sz);
        let (ux, uz) = if l < 1e-3 { n0 } else { (sx / l, sz / l) };
        let c = (ux * n0.0 + uz * n0.1).max(0.5);
        let (nx, nz) = (ux * w / c, uz * w / c);
        let (x, z, h) = (q[i][0], q[i][1], q[i][2]);
        let terre = |x: f32, z: f32| d.lieu.relief(x as f64, z as f64) as f32;
        let gh = [x + nx, h + haut, z + nz];
        let dh = [x - nx, h + haut, z - nz];
        // Au sol, jusque sous le terrain ; un ouvrage, en dalle sous la chaussée
        let bas = |x: f32, z: f32| match dalle {
            Some(e) => h - e,
            None => terre(x, z).min(h) - sous,
        };
        let gb = [gh[0], bas(gh[0], gh[2]), gh[2]];
        let db = [dh[0], bas(dh[0], dh[2]), dh[2]];
        g.push((gh, dh, gb, db));
    }
    for i in 0..n - 1 {
        let (a, b) = (g[i], g[i + 1]);
        // dessus, côtés (vus de dehors : l'ordre est corrigé par la normale du triangle,
        // tracé des deux côtés)
        s.polygone(&[a.0, b.0, b.1, a.1], couleur);
        s.polygone(&[a.0, a.2, b.2, b.0], couleur.map(|x| x * 0.8));
        s.polygone(&[a.1, b.1, b.3, a.3], couleur.map(|x| x * 0.8));
    }
    let (a, z) = (g[0], g[n - 1]);
    s.polygone(&[a.0, a.1, a.3, a.2], couleur.map(|x| x * 0.8));
    s.polygone(&[z.0, z.2, z.3, z.1], couleur.map(|x| x * 0.8));
}

/// Les faces des remblais de la tuile `cle` (sommets de 9 flottants : position,
/// normale, couleur ; triangles).
pub fn tuile(d: &Donnees, r: &Reseau, cle: Cle) -> (Vec<f32>, Vec<u32>) {
    let _t = crate::suivi::tache("remblais des voies (tuile)");
    let b = boite_tuile(d, cle);
    let mut s = Scene::default();
    for a in &r.aretes {
        let w = a.largeur * 0.5 + MARGE;
        let mut c = REMBLAIS[(a.genre as usize).min(REMBLAIS.len() - 1)].0;
        let dalle = match a.passage {
            Passage::Sol => None,
            Passage::Pont => Some(OUVRAGE),
            Passage::Tunnel => {
                c = c.map(|x| x * SOMBRE_TUNNEL);
                Some(OUVRAGE)
            }
        };
        for q in suites(a, b) {
            bande(&mut s, d, &q, w, SOUS, DESSUS + HAUT, c, dalle);
        }
    }
    (s.sommets, s.indices)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Banc : FLOW_LIEU ; les remblais de quelques tuiles du centre.
    #[test]
    #[ignore]
    fn banc_remblais_voies() {
        let l = sage_reel::Lieu::lire(std::path::Path::new(&std::env::var("FLOW_LIEU").unwrap())).unwrap();
        let d = Donnees::new(l);
        let t = std::time::Instant::now();
        let r = d.reseau();
        println!("réseau : {} arêtes en {:.1} s", r.aretes.len(), t.elapsed().as_secs_f32());
        for cle in [(4u8, 7, 7), (4, 8, 8), (5, 16, 16)] {
            let t = std::time::Instant::now();
            let (s, i) = tuile(&d, &r, cle);
            assert_eq!(s.len() % 9, 0);
            assert!(i.iter().all(|&k| (k as usize) < s.len() / 9));
            assert!(s.iter().all(|x| x.is_finite()));
            println!("tuile {cle:?} : {} triangles en {:.0} ms", i.len() / 3, t.elapsed().as_secs_f32() * 1000.0);
        }
    }
}
