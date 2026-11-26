//! Les files des voies en données vectorielles : pour chaque file d'une arête du réseau,
//! son axe, ses deux bords et son sens, d'après l'axe de la voie (déporté près d'une
//! fourche comme la chaussée), sa largeur et ses files (`sage_reel::Files`).
//!
//! Les files se partagent la largeur à parts égales, de la gauche du tracé (file 0) à
//! sa droite ; la file `i` est la bande entre les décalages `w − i·l` et `w − (i+1)·l`
//! (w la demi-largeur, l = largeur / nombre de files). Les files `0..arriere` vont à
//! contresens du tracé, les `total − avant..total` dans son sens, celles du milieu dans
//! les deux. Le décalage suit la normale à gauche moyenne des segments voisins, avec un
//! onglet borné (×2) aux coudes.
//!
//! C'est le calcul des lignes des files de la surimpression, gardé : les lignes en sont
//! tirées (`separations`), et le reste du programme peut lire les files elles-mêmes.

use crate::reseau::Arete;

/// Le sens d'une file par rapport au tracé de son arête.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sens {
    /// Dans le sens du tracé.
    Avant,
    /// À contresens du tracé.
    Arriere,
    /// Partagée : les deux sens (une voie étroite, une file centrale).
    Partagee,
}

/// Une file : son arête, son rang (de la gauche du tracé), son sens, et sa géométrie
/// (x, z, hauteur), dans le sens du tracé : l'axe et les deux bords (gauche et droite
/// du tracé).
#[derive(Clone, Debug)]
pub struct File {
    pub arete: usize,
    pub rang: usize,
    pub sens: Sens,
    pub bus: bool,
    /// Le nombre de files vient d'OSM (sinon il est déduit de la largeur).
    pub osm: bool,
    pub axe: Vec<[f32; 3]>,
    pub gauche: Vec<[f32; 3]>,
    pub droite: Vec<[f32; 3]>,
}

/// La nature d'une ligne entre deux files voisines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ligne {
    /// Entre les deux sens (ou au bord d'une file partagée).
    Sens,
    /// Entre deux files du même sens, lues dans OSM.
    Osm,
    /// Entre deux files du même sens, déduites.
    Deduite,
}

/// La normale à gauche en chaque point de la polyligne (moyenne des segments voisins ;
/// sa longueur porte l'onglet, 1 / cos² du demi-angle, borné à 2 ; un bout n'a qu'un
/// segment : sa normale telle quelle).
pub fn normales(pts: &[[f32; 3]]) -> Vec<[f32; 2]> {
    let m = pts.len();
    (0..m)
        .map(|i| {
            let (mut g, mut k) = ([0.0f32; 2], 0.0f32);
            for (u, v) in [(i.saturating_sub(1), i), (i, (i + 1).min(m - 1))] {
                let (dx, dz) = (pts[v][0] - pts[u][0], pts[v][1] - pts[u][1]);
                let l = dx.hypot(dz);
                if l > 1e-6 {
                    g[0] += dz / l;
                    g[1] += -dx / l;
                    k += 1.0;
                }
            }
            let g = [g[0] / k.max(1.0), g[1] / k.max(1.0)];
            let c = g[0].hypot(g[1]).max(0.5);
            [g[0] / (c * c), g[1] / (c * c)]
        })
        .collect()
}

/// La polyligne décalée de `y` à gauche du tracé (le long des normales `g`).
pub fn decaler_selon(pts: &[[f32; 3]], g: &[[f32; 2]], y: f32) -> Vec<[f32; 3]> {
    pts.iter().zip(g).map(|(p, g)| [p[0] + g[0] * y, p[1] + g[1] * y, p[2]]).collect()
}

/// La polyligne décalée de `y` à gauche du tracé.
pub fn decaler(pts: &[[f32; 3]], y: f32) -> Vec<[f32; 3]> {
    decaler_selon(pts, &normales(pts), y)
}

/// Le décalage (à gauche du tracé) du bord gauche de la file `i` de l'arête `a`.
pub fn bord(a: &Arete, i: usize) -> f32 {
    let n = a.files.total.max(1) as f32;
    a.largeur * 0.5 - i as f32 * a.largeur / n
}

/// Le décalage (à gauche du tracé) de l'axe de la file `i` de l'arête `a`.
pub fn decalage(a: &Arete, i: usize) -> f32 {
    let n = a.files.total.max(1) as f32;
    a.largeur * 0.5 - (i as f32 + 0.5) * a.largeur / n
}

/// Le sens de la file `i` de l'arête `a`.
pub fn sens(a: &Arete, i: usize) -> Sens {
    let f = a.files;
    let n = f.total as usize;
    let ar = (f.arriere as usize).min(n);
    let av = (f.avant as usize).min(n - ar);
    if i < ar {
        Sens::Arriere
    } else if i >= n - av {
        Sens::Avant
    } else {
        Sens::Partagee
    }
}

/// Les files de l'arête `k` (`a`), sur l'axe `pts` (celui de l'arête, ou déporté).
pub fn files(k: usize, a: &Arete, pts: &[[f32; 3]]) -> Vec<File> {
    let n = a.files.total as usize;
    if n == 0 || pts.len() < 2 {
        return Vec::new();
    }
    let g = normales(pts);
    (0..n)
        .map(|i| File {
            arete: k,
            rang: i,
            sens: sens(a, i),
            bus: a.files.est_bus(i),
            osm: a.files.osm,
            axe: decaler_selon(pts, &g, decalage(a, i)),
            gauche: decaler_selon(pts, &g, bord(a, i)),
            droite: decaler_selon(pts, &g, bord(a, i + 1)),
        })
        .collect()
}

/// Les lignes entre files voisines de l'arête `a` (sur l'axe `pts`), de gauche à
/// droite : leur nature et leur polyligne.
pub fn separations(a: &Arete, pts: &[[f32; 3]]) -> Vec<(Ligne, Vec<[f32; 3]>)> {
    let n = a.files.total as usize;
    if n < 2 || pts.len() < 2 {
        return Vec::new();
    }
    let g = normales(pts);
    let groupe = |i: usize| match sens(a, i) {
        Sens::Arriere => 0,
        Sens::Partagee => 1,
        Sens::Avant => 2,
    };
    (1..n)
        .map(|i| {
            let l = if groupe(i - 1) != groupe(i) {
                Ligne::Sens
            } else if a.files.osm {
                Ligne::Osm
            } else {
                Ligne::Deduite
            };
            (l, decaler_selon(pts, &g, bord(a, i)))
        })
        .collect()
}
