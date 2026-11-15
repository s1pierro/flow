//! Le rendu Maquette de la 3D d'un lieu : un nouveau rendu, fait en passes, dont chaque
//! tuile est un maillage texturé complet, fabriqué entièrement hors de l'interface.
//!
//! - **Première passe** (celle-ci) : chaque tuile est un **solide fermé** (`solide` :
//!   une `booleens::Forme`, variété aux faces tournées dehors) : le dessus est le relief,
//!   l'eau à plat à son niveau ; les murs de son pourtour descendent jusqu'à un socle
//!   commun, 50 m sous le point le plus bas du lieu, et le dessous la ferme. Au bord du
//!   lieu, la tuile s'arrête sur sa limite. Ce volume est provisoire : il sert aux
//!   opérations booléennes (la différence avec les volumes des voies, des lits des
//!   cours d'eau), puis s'aplatit en fin de cycle (`aplatir` : sans les sommets du
//!   socle ni leurs faces).
//! - **L'habillage** (`habiller`) fait du solide (avant ou après les opérations) le
//!   maillage à dessiner, texturé par la seule occupation du sol (bois, prés, champs,
//!   habité, zones) et l'eau.
//! - **La texture** : `TEX` × `TEX` pixels pour le dessus (les pixels du bord tombent
//!   sur le bord de la tuile, sans couture), et sous lui une bande de `BANDE` lignes, la
//!   coupe de la terre (de la terre végétale sombre à l'argile), où se prennent les murs
//!   et le dessous. Pas d'ombrage cuit : la lumière vient des normales (le nuanceur
//!   éclaire la matière `MAQUETTE` plus franchement).
//! - **La fabrique** (`Fabrique`) : un fil à part, de basse priorité, prend les tuiles
//!   demandées dans l'ordre (les plus grossières d'abord) et rend des tuiles finies ;
//!   l'interface ne fait que les prendre, quelques-unes par image. Le temps de
//!   fabrication n'est pas compté : la réactivité de l'interface d'abord.

use crate::booleens::Forme;
use crate::carte::{boite_tuile, couleur_sol, Cle, Donnees};
use crate::relief3d::{COTES, SOMMET};
use sage_reel::CLASSES;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, RwLock};

/// La matière du relief de la maquette (le nuanceur l'éclaire plus franchement).
pub const MAQUETTE: f32 = 4.0;
/// L'eau de la maquette (le rendu blanc la teinte en bleu pâle).
pub const MAQUETTE_EAU: f32 = 4.2;
/// Le côté de la texture du dessus, en pixels.
pub const TEX: usize = 256;
/// Les lignes de la coupe de la terre, sous le dessus.
pub const BANDE: usize = 8;
/// L'épaisseur sous le point le plus bas du lieu (m).
pub const EPAISSEUR: f32 = 50.0;

/// Une tuile finie : son maillage (sommets de `SOMMET` flottants) et sa texture (RVBA,
/// `TEX` × `TEX + BANDE`), et la retouche de la carte jusqu'à laquelle elle est à jour.
pub struct Tuile {
    pub cle: Cle,
    pub faite: u64,
    pub sommets: Vec<f32>,
    pub indices: Vec<u32>,
    pub pixels: Vec<u8>,
    /// Ses arêtes à tracer (le rendu blanc), en indices deux par deux.
    pub aretes: Vec<u32>,
    /// Le contour de son volume (la surbrillance d'une tuile sélectionnée).
    pub contour: Vec<u32>,
    /// Sa surface d'eau, à part : à plat au niveau, de la matière `MAQUETTE_EAU`,
    /// dessinée semi-transparente par-dessus le lit.
    pub eau_sommets: Vec<f32>,
    pub eau_indices: Vec<u32>,
    pub mesure: Mesure,
}

/// Ce que pèse une tuile finie : sommets, triangles, arêtes (paires de sommets
/// distinctes de ses triangles), mémoire (sommets, indices, texture, arêtes et
/// contour à tracer), en octets.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Mesure {
    pub sommets: usize,
    pub triangles: usize,
    pub aretes: usize,
    pub octets: usize,
}

pub fn mesurer(v: &[f32], ind: &[u32], pixels: usize, traits: usize) -> Mesure {
    let mut a: HashSet<(u32, u32)> = HashSet::with_capacity(ind.len());
    for t in ind.chunks_exact(3) {
        for (i, j) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            a.insert((i.min(j), i.max(j)));
        }
    }
    Mesure { sommets: v.len() / SOMMET, triangles: ind.len() / 3, aretes: a.len(), octets: v.len() * 4 + ind.len() * 4 + pixels + traits * 4 }
}

/// Le contour du volume d'une tuile finie : ses arêtes vives (plis de plus de 20°)
/// posées sur son pourtour (le tour du dessus, les coins des murs, le tour du dessous).
pub fn contour(v: &[f32], ind: &[u32]) -> Vec<u32> {
    let p = |i: u32| [v[i as usize * SOMMET], v[i as usize * SOMMET + 2]];
    let mut b = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    for s in v.chunks_exact(SOMMET) {
        b = [b[0].min(s[0]), b[1].min(s[2]), b[2].max(s[0]), b[3].max(s[2])];
    }
    let pres = |a: f32, c: f32| (a - c).abs() < 0.01;
    crate::relief3d::aretes(v, SOMMET, ind, true)
        .chunks_exact(2)
        .filter(|e| {
            let (a, c) = (p(e[0]), p(e[1]));
            (0..2).any(|k| (pres(a[k], b[k]) && pres(c[k], b[k])) || (pres(a[k], b[k + 2]) && pres(c[k], b[k + 2])))
        })
        .flatten()
        .copied()
        .collect()
}

/// Une tuile finie, avec ses arêtes, son contour et sa mesure.
pub fn tuile(d: &Donnees, cle: Cle, faite: u64) -> Tuile {
    let (sommets, indices, pixels) = fabriquer(d, cle);
    let aretes = aretes(&sommets, &indices);
    let contour = contour(&sommets, &indices);
    let (eau_sommets, eau_indices) = surface_eau(d, cle);
    let mut mesure = mesurer(&sommets, &indices, pixels.len(), aretes.len() + contour.len());
    mesure.sommets += eau_sommets.len() / SOMMET;
    mesure.triangles += eau_indices.len() / 3;
    mesure.octets += (eau_sommets.len() + eau_indices.len()) * 4;
    Tuile { cle, faite, sommets, indices, pixels, aretes, contour, eau_sommets, eau_indices, mesure }
}

/// Le fond du lit, sous l'eau : vase et sable.
pub const LIT: [f32; 3] = [0.50, 0.46, 0.36];

/// La coupe de la terre, du haut (terre végétale) au bas (argile).
fn couleur_coupe(t: f32) -> [f32; 3] {
    let (a, b) = ([0.42, 0.32, 0.22], [0.72, 0.60, 0.44]);
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// La profondeur du lit au milieu de l'eau (m) : les données s'arrêtent à la surface,
/// elle est choisie.
pub const PROFONDEUR: f32 = 3.0;
/// Le pied d'un quai : le lit est déjà sous l'eau d'autant contre son mur (m), puis il
/// descend de `PENTE_QUAI` mètres par mètre.
pub const PIED_QUAI: f32 = 1.5;
pub const PENTE_QUAI: f32 = 0.5;
/// Une berge naturelle : le lit part de `RIVE_NATURELLE` sous l'eau et descend
/// doucement, de `PENTE_NATURELLE` mètres par mètre ; la terre descend en talus sur
/// `TALUS` mètres jusqu'à `RAS` au-dessus de l'eau.
pub const RIVE_NATURELLE: f32 = 0.1;
pub const PENTE_NATURELLE: f32 = 0.12;
pub const TALUS: f32 = 12.0;
pub const RAS: f32 = 0.15;
/// Le remblai le long des berges : sa crête à plat sur `PLAT` mètres après le bord ou
/// le talus, puis un talus vers la terre (`PENTE_REMBLAI` mètres par mètre) jusqu'à sa
/// base, sous le lit. Le relief sous lui s'abaisse de `ENFOUI` : c'est son dessus,
/// régulier, qui se voit.
pub const PLAT: f32 = 4.0;
pub const PENTE_REMBLAI: f32 = 1.0;
pub const ENFOUI: f32 = 0.3;

/// Le profil du remblai d'une berge de type `q` et de crête `crete`, en travers : les
/// points (distance à la rive vers la terre, hauteur) de sa coupe, du pied sous le
/// lit, au bord de l'eau, à la crête, au talus vers la terre, au pied.
pub fn profil(niveau: f32, q: f32, crete: f32) -> [[f32; 2]; 5] {
    let bas = niveau - PROFONDEUR - 1.0;
    let h0 = niveau + RAS + (crete - niveau - RAS) * q;
    let w = ((1.0 - q) * TALUS).max(0.3);
    let t = (crete - bas) / PENTE_REMBLAI;
    [[0.0, bas], [0.0, h0], [w, crete], [w + PLAT, crete], [w + PLAT + t, bas]]
}

/// La hauteur du dessus du profil à la distance `s` de la rive.
pub fn hauteur_profil(p: &[[f32; 2]; 5], s: f32) -> f32 {
    for k in 1..4 {
        let (a, b) = (p[k], p[k + 1]);
        if s <= b[0] {
            let t = if b[0] > a[0] { ((s - a[0]) / (b[0] - a[0])).clamp(0.0, 1.0) } else { 1.0 };
            return a[1] + (b[1] - a[1]) * t;
        }
    }
    p[4][1]
}

/// Sous la rive, la profondeur du lit pour une berge de type `q` (0 naturelle, 1 quai).
pub fn sous_rive(q: f32) -> f32 {
    RIVE_NATURELLE + (PIED_QUAI - RIVE_NATURELLE) * q
}

pub fn pente_lit(q: f32) -> f32 {
    PENTE_NATURELLE + (PENTE_QUAI - PENTE_NATURELLE) * q
}

/// Les berges d'OSM autour d'une tuile : les anneaux des surfaces d'eau du lieu,
/// coupés à la boîte de la tuile élargie de la portée du lit et du talus. Où il y en a,
/// ils disent l'eau (pair-impair), la distance à la rive et le type de la berge (quai ou
/// naturelle, `berges`), au lieu des points d'eau de la grille (une rive en marches de
/// 2 m) ; ailleurs, la grille, et des quais.
pub struct Rives {
    anneaux: Vec<Vec<[f32; 2]>>,
    /// Les côtés des berges proches, et le type et la crête à leurs deux bouts.
    berges: Vec<([[f32; 2]; 2], [f32; 2], [f32; 2])>,
}

impl Rives {
    pub fn new(d: &Donnees, b: [f32; 4]) -> Rives {
        let marge = (PROFONDEUR - RIVE_NATURELLE) / PENTE_NATURELLE + TALUS + 4.0 * d.lieu.pas as f32;
        let bx = [b[0] - marge, b[1] - marge, b[2] + marge, b[3] + marge];
        let boite = |r: &[[f32; 2]]| r.iter().fold([f32::MAX, f32::MAX, f32::MIN, f32::MIN], |a, p| [a[0].min(p[0]), a[1].min(p[1]), a[2].max(p[0]), a[3].max(p[1])]);
        let hors = |bt: [f32; 4]| bt[2] < bx[0] || bt[0] > bx[2] || bt[3] < bx[1] || bt[1] > bx[3];
        let mut anneaux = Vec::new();
        for s in &d.lieu.surfaces_eau {
            for r in &s.anneaux {
                if hors(boite(r)) {
                    continue;
                }
                let c = sage_reel::cours_eau::couper_anneau(r, bx);
                if !c.is_empty() {
                    anneaux.push(c);
                }
            }
        }
        let mut berges = Vec::new();
        if !anneaux.is_empty() {
            let types = d.berges();
            for g in types.iter().flatten() {
                let r = &g.points;
                if r.len() < 2 || hors(boite(r)) {
                    continue;
                }
                for i in 0..r.len() {
                    let j = (i + 1) % r.len();
                    if !hors(boite(&[r[i], r[j]])) {
                        berges.push(([r[i], r[j]], [g.quai[i], g.quai[j]], [g.crete[i], g.crete[j]]));
                    }
                }
            }
        }
        Rives { anneaux, berges }
    }

    /// Les berges d'OSM couvrent-elles la tuile ?
    pub fn vectorielles(&self) -> bool {
        !self.anneaux.is_empty()
    }

    /// Dans l'eau : dans les anneaux (pair-impair), sinon sur les points d'eau de la grille.
    pub fn dans_eau(&self, d: &Donnees, x: f32, z: f32) -> bool {
        if !self.vectorielles() {
            return d.lieu.eau(x as f64, z as f64) >= 0.5;
        }
        let mut dedans = false;
        for r in &self.anneaux {
            for i in 0..r.len() {
                let (p, q) = (r[i], r[(i + 1) % r.len()]);
                if (p[1] > z) != (q[1] > z) && x < p[0] + (z - p[1]) / (q[1] - p[1]) * (q[0] - p[0]) {
                    dedans = !dedans;
                }
            }
        }
        dedans
    }

    /// Positif dans l'eau, négatif sur la terre, nul sur la rive : la distance signée à
    /// la berge d'OSM ; sur la grille, la part d'eau moins un demi.
    pub fn signe(&self, d: &Donnees, x: f32, z: f32) -> f32 {
        if !self.vectorielles() {
            return d.lieu.eau(x as f64, z as f64) as f32 - 0.5;
        }
        let dd = self.distance(x, z, 1e4).max(1e-4);
        if self.dans_eau(d, x, z) { dd } else { -dd }
    }

    /// La distance à la berge la plus proche (au plus `max`).
    pub fn distance(&self, x: f32, z: f32, max: f32) -> f32 {
        self.berge(x, z).0.min(max)
    }

    /// La berge la plus proche : sa distance et son type (0 naturelle, 1 quai) ; sans
    /// berge, loin, et un quai.
    pub fn berge(&self, x: f32, z: f32) -> (f32, f32) {
        let (d, q, _) = self.berge_crete(x, z);
        (d, q)
    }

    /// `berge`, et la crête du remblai en ce point de la berge.
    pub fn berge_crete(&self, x: f32, z: f32) -> (f32, f32, f32) {
        let mut r = (f32::MAX, 1.0, 0.0);
        for ([p, q], v, c) in &self.berges {
            let (dx, dz) = (q[0] - p[0], q[1] - p[1]);
            let l2 = dx * dx + dz * dz;
            let t = if l2 > 0.0 { (((x - p[0]) * dx + (z - p[1]) * dz) / l2).clamp(0.0, 1.0) } else { 0.0 };
            let (ex, ez) = (p[0] + dx * t - x, p[1] + dz * t - z);
            let d2 = ex * ex + ez * ez;
            if d2 < r.0 {
                r = (d2, v[0] + (v[1] - v[0]) * t, c[0] + (c[1] - c[0]) * t);
            }
        }
        (r.0.sqrt(), r.1, r.2)
    }
}

/// La profondeur du lit en (x, z) : nulle hors de l'eau ; dans l'eau, elle croît depuis
/// la rive (la berge d'OSM, ou le point de la carte hors de l'eau le plus proche)
/// jusqu'à `PROFONDEUR`, vite contre un quai, doucement le long d'une berge naturelle.
pub fn profondeur(d: &Donnees, rives: &Rives, x: f32, z: f32) -> f32 {
    if !rives.dans_eau(d, x, z) {
        return 0.0;
    }
    profondeur_lit(d, rives, x, z)
}

/// La profondeur du lit en (x, z) comme si on était dans l'eau (le prolongement du lit
/// sous la rive : pour les normales et la coupe des triangles).
pub fn profondeur_lit(d: &Donnees, rives: &Rives, x: f32, z: f32) -> f32 {
    let l = &d.lieu;
    if rives.vectorielles() {
        let (dd, q) = rives.berge(x, z);
        return (sous_rive(q) + dd * pente_lit(q)).min(PROFONDEUR);
    }
    // Sur la grille : des quais
    let portee = (PROFONDEUR - PIED_QUAI) / PENTE_QUAI;
    let pas = l.pas as f32;
    let r = (portee / pas).ceil() as i64 + 1;
    let o = sage_reel::origine() as f32;
    let (ga, gb) = (((x - o) / pas).round() as i64, ((z - o) / pas).round() as i64);
    let mut dmin = portee;
    for b in gb - r..=gb + r {
        for a in ga - r..=ga + r {
            if l.est_eau(a, b) || l.grille.indice(a, b).is_none() {
                continue;
            }
            let (px, pz) = (o + a as f32 * pas, o + b as f32 * pas);
            dmin = dmin.min(((px - x).powi(2) + (pz - z).powi(2)).sqrt());
        }
    }
    // La rive est à mi-chemin entre le dernier point sec et le premier mouillé
    (PIED_QUAI + (dmin - pas / 2.0).max(0.0) * PENTE_QUAI).min(PROFONDEUR)
}

/// La terre en (x, z) : le relief sans les points d'eau ; sous la crête et le bord du
/// remblai d'une berge, abaissé sous son dessus (le remblai, uni au volume de la
/// tuile, s'y montre seul).
pub fn terre(d: &Donnees, rives: &Rives, x: f32, z: f32) -> f32 {
    let r = relief_sec(d, x, z);
    if !rives.vectorielles() {
        return r;
    }
    let (dd, q, c) = rives.berge_crete(x, z);
    let p = profil(d.lieu.niveau, q, c);
    if dd >= p[3][0] {
        return r;
    }
    r.min(hauteur_profil(&p, dd) - ENFOUI)
}

/// La hauteur du dessus en (x, z) : le relief ; sous l'eau, le lit, creusé sous le
/// niveau de l'eau (la surface de l'eau est un maillage à part).
fn haut(d: &Donnees, rives: &Rives, x: f32, z: f32) -> f32 {
    if rives.signe(d, x, z) > 0.0 { lit(d, rives, x, z) } else { terre(d, rives, x, z) }
}

/// Le fond du lit en (x, z), prolongé sous la rive.
fn lit(d: &Donnees, rives: &Rives, x: f32, z: f32) -> f32 {
    d.lieu.niveau - profondeur_lit(d, rives, x, z)
}

/// Le relief de la terre en (x, z), sans les points d'eau de la carte (mis à plat au
/// niveau de l'eau, ils tireraient la rive vers le bas) : loin de l'eau, le relief ;
/// près d'elle, les seuls points secs autour, pondérés.
pub fn relief_sec(d: &Donnees, x: f32, z: f32) -> f32 {
    let l = &d.lieu;
    let pas = l.pas as f32;
    let o = sage_reel::origine() as f32;
    let (gx, gz) = ((x - o) / pas, (z - o) / pas);
    let (fa, fb) = (gx.floor(), gz.floor());
    let (u, v) = (gx - fa, gz - fb);
    let (a, b) = (fa as i64, fb as i64);
    // Le relief doux lit 4 × 4 points : sans eau parmi eux, il est juste
    if !(-1..=2).any(|db| (-1..=2).any(|da| l.est_eau(a + da, b + db))) {
        return l.relief(x as f64, z as f64) as f32;
    }
    let (mut s, mut w) = (0f32, 0f32);
    for (da, db, p) in [(0, 0, (1.0 - u) * (1.0 - v)), (1, 0, u * (1.0 - v)), (0, 1, (1.0 - u) * v), (1, 1, u * v)] {
        if p > 0.0 && !l.est_eau(a + da, b + db) {
            s += p * l.h_point(a + da, b + db);
            w += p;
        }
    }
    if w > 1e-4 {
        return s / w;
    }
    for r in 1..=3i64 {
        let (mut s, mut n) = (0f32, 0);
        for db in -r..=r + 1 {
            for da in -r..=r + 1 {
                if (da == -r || da == r + 1 || db == -r || db == r + 1) && !l.est_eau(a + da, b + db) && l.grille.indice(a + da, b + db).is_some() {
                    s += l.h_point(a + da, b + db);
                    n += 1;
                }
            }
        }
        if n > 0 {
            return s / n as f32;
        }
    }
    l.niveau
}

/// Le socle commun des tuiles du lieu.
pub fn socle(d: &Donnees) -> f32 {
    d.lieu.base - EPAISSEUR
}

/// Le solide fermé de la tuile `cle`, pour les opérations booléennes : le dessus (une
/// grille de `COTES` cases, bornée au lieu : au bord, sa dernière rangée tombe sur la
/// limite), les murs de son pourtour jusqu'au socle, le dessous en éventail autour de
/// son centre. Sommets soudés, faces tournées vers le dehors : une variété fermée.
/// Les points de la grille se calculent depuis celle de toute la carte : deux tuiles
/// voisines de même niveau ont leurs points communs identiques.
pub fn solide(d: &Donnees, cle: Cle) -> Forme {
    let (xs, zs) = grille(d, cle);
    let (nx, nz) = (xs.len(), zs.len());
    if nx < 2 || nz < 2 {
        return Forme::default();
    }
    let socle = socle(d);
    let rives = Rives::new(d, boite_tuile(d, cle));
    let mut f = Forme::default();
    let mut signe: Vec<f32> = Vec::with_capacity(nx * nz);
    for &z in &zs {
        for &x in &xs {
            let s = rives.signe(d, x, z);
            signe.push(s);
            f.sommets.push([x, if s > 0.0 { lit(d, &rives, x, z) } else { terre(d, &rives, x, z) }, z]);
        }
    }
    // Les côtés de la grille que la rive coupe : le point de la rive, deux fois (sur la
    // terre, et au pied de la berge dans le lit)
    let mut coupes: HashMap<(u32, u32), (u32, u32)> = HashMap::new();
    let mut couper = |f: &mut Forme, a: u32, b: u32| -> (u32, u32) {
        *coupes.entry((a.min(b), a.max(b))).or_insert_with(|| {
            let (sa, sb) = (signe[a as usize], signe[b as usize]);
            let t = (sa / (sa - sb)).clamp(0.0, 1.0);
            let (pa, pb) = (f.sommets[a as usize], f.sommets[b as usize]);
            let (x, z) = (pa[0] + (pb[0] - pa[0]) * t, pa[2] + (pb[2] - pa[2]) * t);
            let terre = terre(d, &rives, x, z);
            let n = f.sommets.len() as u32;
            f.sommets.push([x, terre, z]);
            f.sommets.push([x, lit(d, &rives, x, z).min(terre), z]);
            (n, n + 1)
        })
    };
    let w = nx as u32;
    let mouille = |i: u32| signe[i as usize] > 0.0;
    for k in 0..nz as u32 - 1 {
        for a in 0..w - 1 {
            let p = k * w + a;
            for t in [[p, p + w, p + 1], [p + 1, p + w, p + w + 1]] {
                let m = t.map(mouille);
                if m[0] == m[1] && m[1] == m[2] {
                    f.faces.push(t.to_vec());
                    continue;
                }
                // Le sommet seul de son côté de la rive en tête (l'ordre tournant gardé)
                let r = (0..3).find(|&i| m[i] != m[(i + 1) % 3] && m[i] != m[(i + 2) % 3]).unwrap();
                let (sa, sb, sc) = (t[r], t[(r + 1) % 3], t[(r + 2) % 3]);
                let (xab, xac) = (couper(&mut f, sa, sb), couper(&mut f, sa, sc));
                let cote = |x: (u32, u32), eau: bool| if eau { x.1 } else { x.0 };
                let ea = m[r];
                f.faces.push(vec![sa, cote(xab, ea), cote(xac, ea)]);
                f.faces.push(vec![cote(xab, !ea), sb, sc, cote(xac, !ea)]);
                // La berge, debout entre la terre et le lit
                f.faces.push(vec![cote(xac, ea), cote(xab, ea), cote(xab, !ea), cote(xac, !ea)]);
            }
        }
    }
    murs_et_dessous(&mut f, &xs, &zs, socle, &coupes, &signe);
    f
}

/// Les abscisses et les profondeurs de la grille du dessus de la tuile `cle`, bornées
/// au lieu, sans doublon (ni tranche de moins de 5 cm).
fn grille(d: &Donnees, cle: Cle) -> (Vec<f32>, Vec<f32>) {
    let e = d.lieu.etendue();
    let (ex0, ez0, ex1, ez1) = (e[0] as f32, e[1] as f32, e[2] as f32, e[3] as f32);
    let n = COTES;
    let (x0, z0, c) = d.monde();
    let pas_niveau = c as f64 / (1u64 << cle.0) as f64 / n as f64;
    // Les abscisses et les profondeurs de la grille, bornées au lieu, sans doublon (ni
    // tranche de moins de 5 cm)
    let ligne = |o: f32, i: i32, lo: f32, hi: f32| -> Vec<f32> {
        let mut v: Vec<f32> = Vec::with_capacity(n + 1);
        for a in 0..=n as i64 {
            let x = ((o as f64 + (i as i64 * n as i64 + a) as f64 * pas_niveau) as f32).clamp(lo, hi);
            match v.last() {
                Some(&p) if x - p < 0.05 => {
                    // Le bord du lieu l'emporte sur un point de grille trop proche
                    if x == hi {
                        *v.last_mut().unwrap() = x;
                    }
                }
                _ => v.push(x),
            }
        }
        v
    };
    (ligne(x0, cle.1, ex0, ex1), ligne(z0, cle.2, ez0, ez1))
}

/// Les murs et le dessous d'un dessus en grille `xs` × `zs` (ses premiers sommets).
fn murs_et_dessous(f: &mut Forme, xs: &[f32], zs: &[f32], socle: f32, coupes: &HashMap<(u32, u32), (u32, u32)>, signe: &[f32]) {
    let (nx, nz) = (xs.len(), zs.len());
    // Le pourtour du dessus, dans le sens où les murs regardent dehors
    let id = |a: usize, k: usize| (k * nx + a) as u32;
    let mut bord: Vec<u32> = Vec::new();
    bord.extend((0..nx - 1).map(|a| id(a, 0)));
    bord.extend((0..nz - 1).map(|k| id(nx - 1, k)));
    bord.extend((1..nx).rev().map(|a| id(a, nz - 1)));
    bord.extend((1..nz).rev().map(|k| id(0, k)));
    // Avec les points de la rive là où elle passe le bord
    let mut tour: Vec<u32> = Vec::with_capacity(bord.len() + 8);
    for i in 0..bord.len() {
        let (u, v) = (bord[i], bord[(i + 1) % bord.len()]);
        tour.push(u);
        if let Some(&(terre, eau)) = coupes.get(&(u.min(v), u.max(v))) {
            if signe[u as usize] > 0.0 {
                tour.extend_from_slice(&[eau, terre]);
            } else {
                tour.extend_from_slice(&[terre, eau]);
            }
        }
    }
    let bas0 = f.sommets.len() as u32;
    for &t in &tour {
        let p = f.sommets[t as usize];
        f.sommets.push([p[0], socle, p[2]]);
    }
    let m = tour.len() as u32;
    for i in 0..m {
        let j = (i + 1) % m;
        f.faces.push(vec![tour[i as usize], tour[j as usize], bas0 + j, bas0 + i]);
    }
    let centre = f.sommets.len() as u32;
    f.sommets.push([(xs[0] + xs[nx - 1]) / 2.0, socle, (zs[0] + zs[nz - 1]) / 2.0]);
    for i in 0..m {
        f.faces.push(vec![centre, bas0 + i, bas0 + (i + 1) % m]);
    }
}

/// La surface d'eau de la tuile `cle`, à part du solide : les cases de sa grille qui
/// touchent l'eau, à plat au niveau (sommets de `SOMMET` flottants, normale en haut,
/// matière `MAQUETTE_EAU`). Elle dépasse un peu sur la rive : la berge, plus haute, la
/// cache ; on voit le lit au travers.
pub fn surface_eau(d: &Donnees, cle: Cle) -> (Vec<f32>, Vec<u32>) {
    let l = &d.lieu;
    let (xs, zs) = grille(d, cle);
    let (nx, nz) = (xs.len(), zs.len());
    let (mut v, mut ind) = (Vec::new(), Vec::new());
    if nx < 2 || nz < 2 {
        return (v, ind);
    }
    // Mouillés : dans l'eau, ou à moins d'une case de la rive (la berge cache le reste)
    let rives = Rives::new(d, boite_tuile(d, cle));
    let case = ((xs[nx - 1] - xs[0]) / (nx - 1) as f32).max((zs[nz - 1] - zs[0]) / (nz - 1) as f32) * 1.5;
    let mouille: Vec<bool> = zs.iter().flat_map(|&z| xs.iter().map(move |&x| (x, z))).map(|(x, z)| if rives.vectorielles() { rives.dans_eau(d, x, z) || rives.distance(x, z, case) < case } else { l.eau(x as f64, z as f64) > 0.0 }).collect();
    let mut numero: HashMap<usize, u32> = HashMap::new();
    for k in 0..nz - 1 {
        for a in 0..nx - 1 {
            let coins = [k * nx + a, (k + 1) * nx + a, k * nx + a + 1, (k + 1) * nx + a + 1];
            if !coins.iter().any(|&c| mouille[c]) {
                continue;
            }
            let n: Vec<u32> = coins.iter().map(|&c| *numero.entry(c).or_insert_with(|| {
                let (x, z) = (xs[c % nx], zs[c / nx]);
                v.extend_from_slice(&[x, l.niveau, z, 0.0, 1.0, 0.0, 0.0, 0.0, MAQUETTE_EAU]);
                (v.len() / SOMMET - 1) as u32
            })).collect();
            ind.extend_from_slice(&[n[0], n[1], n[2], n[2], n[1], n[3]]);
        }
    }
    (v, ind)
}

/// Le solide aplati (en fin de cycle) : sans les sommets du socle ni leurs faces.
pub fn aplatir(f: &Forme, socle: f32) -> Forme {
    let garde: Vec<bool> = f.sommets.iter().map(|p| (p[1] - socle).abs() > 1e-3).collect();
    let mut nouveau = vec![u32::MAX; f.sommets.len()];
    let mut r = Forme::default();
    for (k, p) in f.sommets.iter().enumerate().filter(|(k, _)| garde[*k]) {
        nouveau[k] = r.sommets.len() as u32;
        r.sommets.push(*p);
    }
    r.faces = f.faces.iter().filter(|p| p.iter().all(|&i| garde[i as usize])).map(|p| p.iter().map(|&i| nouveau[i as usize]).collect()).collect();
    r
}

/// Le maillage à dessiner d'un solide de la tuile de boîte `b` (sommets de `SOMMET`
/// flottants) : les faces tournées vers le haut sont le dessus, texturé d'en haut, à la
/// normale du relief en leurs points (la même d'une tuile à l'autre : pas de couture
/// de lumière) ; les autres (murs, dessous) prennent la coupe de la terre, selon leur
/// profondeur, à la normale de leur face.
pub fn habiller(d: &Donnees, f: &Forme, b: [f32; 4]) -> (Vec<f32>, Vec<u32>) {
    let cote = b[2] - b[0];
    let h_tex = TEX + BANDE;
    let socle = socle(d);
    let haut_max = f.sommets.iter().map(|p| p[1]).fold(socle + 1.0, f32::max);
    let uv = |x: f32, z: f32| -> (f32, f32) {
        let (fu, fv) = (((x - b[0]) / cote).clamp(0.0, 1.0), ((z - b[1]) / cote).clamp(0.0, 1.0));
        ((0.5 + fu * (TEX - 1) as f32) / TEX as f32, (0.5 + fv * (TEX - 1) as f32) / h_tex as f32)
    };
    let v_coupe = |y: f32| (TEX as f32 + 0.5 + ((haut_max - y) / (haut_max - socle)).clamp(0.0, 1.0) * (BANDE - 1) as f32) / h_tex as f32;
    let ecart = cote / COTES as f32 / 2.0;
    let rives = Rives::new(d, b);
    // La normale de la terre ou du lit (chacun prolongé au-delà de la rive : pas de
    // pli de lumière le long d'elle) ; un sommet est du lit s'il est à sa hauteur
    let normale_relief = |x: f32, y: f32, z: f32| -> [f32; 3] {
        let du_lit = (y - lit(d, &rives, x, z)).abs() < 1e-3 && y < d.lieu.niveau + 1e-3;
        let h = |x: f32, z: f32| if du_lit { lit(d, &rives, x, z) } else { terre(d, &rives, x, z) };
        let gx = (h(x + ecart, z) - h(x - ecart, z)) / (2.0 * ecart);
        let gz = (h(x, z + ecart) - h(x, z - ecart)) / (2.0 * ecart);
        let nn = (gx * gx + 1.0 + gz * gz).sqrt();
        [-gx / nn, 1.0 / nn, -gz / nn]
    };
    let mut v: Vec<f32> = Vec::with_capacity(f.sommets.len() * 2 * SOMMET);
    let mut ind: Vec<u32> = Vec::with_capacity(f.faces.len() * 4);
    // Les sommets du dessus, partagés entre ses faces
    let mut dessus: HashMap<u32, u32> = HashMap::new();
    for p in &f.faces {
        let q: Vec<[f32; 3]> = p.iter().map(|&i| f.sommets[i as usize]).collect();
        // La normale de la face (Newell)
        let mut no = [0f32; 3];
        for i in 0..q.len() {
            let (a, c) = (q[i], q[(i + 1) % q.len()]);
            no[0] += (a[1] - c[1]) * (a[2] + c[2]);
            no[1] += (a[2] - c[2]) * (a[0] + c[0]);
            no[2] += (a[0] - c[0]) * (a[1] + c[1]);
        }
        let l = (no[0] * no[0] + no[1] * no[1] + no[2] * no[2]).sqrt();
        if l < 1e-9 {
            continue;
        }
        let no = no.map(|c| c / l);
        let coins: Vec<u32> = if no[1] > 0.05 {
            p.iter().map(|&i| {
                let s = f.sommets[i as usize];
                // Sur le relief ou le lit : la normale douce, sommet partagé ; ailleurs (le
                // remblai), la normale de la face
                if (s[1] - haut(d, &rives, s[0], s[2])).abs() > 0.01 {
                    let (u, w) = uv(s[0], s[2]);
                    v.extend_from_slice(&[s[0], s[1], s[2], no[0], no[1], no[2], u, w, MAQUETTE]);
                    return (v.len() / SOMMET - 1) as u32;
                }
                *dessus.entry(i).or_insert_with(|| {
                    let (u, w) = uv(s[0], s[2]);
                    let nr = normale_relief(s[0], s[1], s[2]);
                    v.extend_from_slice(&[s[0], s[1], s[2], nr[0], nr[1], nr[2], u, w, MAQUETTE]);
                    (v.len() / SOMMET - 1) as u32
                })
            }).collect()
        } else {
            q.iter().map(|s| {
                let u = uv(s[0], s[2]).0;
                v.extend_from_slice(&[s[0], s[1], s[2], no[0], no[1], no[2], u, v_coupe(s[1]), MAQUETTE]);
                (v.len() / SOMMET - 1) as u32
            }).collect()
        };
        for k in 1..coins.len() - 1 {
            ind.extend_from_slice(&[coins[0], coins[k], coins[k + 1]]);
        }
    }
    (v, ind)
}

/// Les arêtes à tracer d'une tuile finie (le rendu blanc), comme celles du relief de
/// la Carte : toute la grille du dessus, sans les faces debout (les murs).
pub fn aretes(v: &[f32], ind: &[u32]) -> Vec<u32> {
    crate::relief3d::aretes(v, SOMMET, ind, false)
}

/// Fabrique la tuile `cle` : son solide, habillé, et sa texture.
pub fn fabriquer(d: &Donnees, cle: Cle) -> (Vec<f32>, Vec<u32>, Vec<u8>) {
    let b = boite_tuile(d, cle);
    // Le solide uni au remblai, ses plans hachés par l'union redevenus simples
    let f = crate::dissolution::dissoudre(&remblayer(d, cle, solide(d, cle)));
    let (v, ind) = habiller(d, &f, b);
    (v, ind, texture(d, b))
}

/// Le solide de la tuile uni au remblai de ses berges ; tel quel si l'union échoue.
pub fn remblayer(d: &Donnees, cle: Cle, f: Forme) -> Forme {
    let (r, _) = remblai(d, cle);
    if r.vide() {
        return f;
    }
    crate::booleens::operation(&f, &r, crate::booleens::Op::Union).unwrap_or(f)
}

/// Le recul du mur du remblai dans l'eau (m) : pas de face confondue avec la berge du
/// solide de la tuile.
const AVANCE: f32 = 0.1;
/// L'allongement de chaque morceau de remblai à ses bouts (m) : ils se chevauchent.
const ALLONGE: f32 = 0.3;

/// Le remblai des berges qui touchent la tuile `cle` : le long de chaque berge, côté
/// terre, un volume de la coupe `profil` (son type et sa crête, point par point). Une
/// berge se découpe en morceaux droits (les côtés de l'anneau d'OSM, redécoupés en
/// stations) : chacun est un volume simple, fermé ; ils sont unis, puis coupés à la
/// tuile (un millimètre en deçà de son bord : pas de face confondue avec ses murs).
/// Rend le remblai et le nombre d'unions qui ont échoué (leur morceau est laissé).
pub fn remblai(d: &Donnees, cle: Cle) -> (Forme, usize) {
    use crate::booleens::{operation, Op};
    let (xs, zs) = grille(d, cle);
    if xs.len() < 2 || zs.len() < 2 {
        return (Forme::default(), 0);
    }
    let bt = [xs[0], zs[0], xs[xs.len() - 1], zs[zs.len() - 1]];
    let niveau = d.lieu.niveau;
    let types = d.berges();
    let mut morceaux: Vec<Forme> = Vec::new();
    for g in types.iter().flatten() {
        let m = g.points.len();
        if m < 2 || g.terre.len() != m {
            continue;
        }
        // Les suites de côtés de même normale (à moins de 0,8° : deux morceaux presque
        // alignés auraient des murs presque confondus, que l'union réussit mal), à
        // partir d'un changement de normale
        let pareil = |a: [f32; 2], b: [f32; 2]| a[0] * b[0] + a[1] * b[1] > 0.9999;
        let debut = (0..m).find(|&k| !pareil(g.terre[k], g.terre[(k + m - 1) % m])).unwrap_or(0);
        let mut k = 0;
        while k < m {
            let a = (debut + k) % m;
            let mut fin = k + 1;
            while fin < m && pareil(g.terre[(debut + fin) % m], g.terre[a]) {
                fin += 1;
            }
            let stations: Vec<usize> = (k..=fin).map(|j| (debut + j) % m).collect();
            k = fin;
            let pts: Vec<[f32; 2]> = stations.iter().map(|&i| g.points[i]).collect();
            let large = TALUS + PLAT + (g.crete.iter().cloned().fold(niveau, f32::max) - niveau + PROFONDEUR + 1.0) / PENTE_REMBLAI + 2.0;
            let b = pts.iter().fold([f32::MAX, f32::MAX, f32::MIN, f32::MIN], |b, p| [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])]);
            if b[2] + large < bt[0] || b[0] - large > bt[2] || b[3] + large < bt[1] || b[1] - large > bt[3] {
                continue;
            }
            let morceau = troncon(&pts, &stations.iter().map(|&i| profil(niveau, g.quai[i], g.crete[i])).collect::<Vec<_>>(), g.terre[a]);
            if !morceau.vide() {
                morceaux.push(morceau);
            }
        }
    }
    if morceaux.is_empty() {
        return (Forme::default(), 0);
    }
    let mut echecs = 0;
    let mut r = morceaux.swap_remove(0);
    for p in &morceaux {
        match operation(&r, p, Op::Union) {
            Ok(u) => r = u,
            Err(_) => echecs += 1,
        }
    }
    let haut = r.sommets.iter().map(|p| p[1]).fold(niveau, f32::max) + 10.0;
    let e = 0.001;
    let boite = pave([bt[0] + e, niveau - PROFONDEUR - 3.0, bt[1] + e], [bt[2] - e, haut, bt[3] - e]);
    match operation(&r, &boite, Op::Intersection) {
        Ok(c) => (c, echecs),
        Err(_) => (Forme::default(), echecs + 1),
    }
}

/// Un pavé fermé, faces dehors.
fn pave(a: [f32; 3], b: [f32; 3]) -> Forme {
    let p = |i: usize| [if i & 1 == 0 { a[0] } else { b[0] }, if i & 2 == 0 { a[1] } else { b[1] }, if i & 4 == 0 { a[2] } else { b[2] }];
    let mut f = Forme { sommets: (0..8).map(p).collect(), faces: vec![vec![0, 4, 6, 2], vec![1, 3, 7, 5], vec![0, 1, 5, 4], vec![2, 6, 7, 3], vec![0, 2, 3, 1], vec![4, 5, 7, 6]] };
    if f.volume() < 0.0 {
        f.faces.iter_mut().for_each(|p| p.reverse());
    }
    f
}

/// Le volume d'un morceau droit de remblai : les coupes `profils` posées aux stations
/// `pts` (alignées), tournées vers la terre `n`, reliées de l'une à l'autre ; le
/// morceau est allongé de `ALLONGE` à ses bouts, et son mur avance de `AVANCE` dans
/// l'eau.
fn troncon(pts: &[[f32; 2]], profils: &[[[f32; 2]; 5]], n: [f32; 2]) -> Forme {
    let s = pts.len();
    if s < 2 {
        return Forme::default();
    }
    let (dx, dz) = (pts[s - 1][0] - pts[0][0], pts[s - 1][1] - pts[0][1]);
    let l = (dx * dx + dz * dz).sqrt();
    if l < 1e-3 {
        return Forme::default();
    }
    let dir = [dx / l, dz / l];
    let mut f = Forme::default();
    for (k, (p, pr)) in pts.iter().zip(profils).enumerate() {
        let e = if k == 0 { -ALLONGE } else if k == s - 1 { ALLONGE } else { 0.0 };
        for (j, c) in pr.iter().enumerate() {
            let o = if j < 2 { c[0] - AVANCE } else { c[0] };
            f.sommets.push([p[0] + n[0] * o + dir[0] * e, c[1], p[1] + n[1] * o + dir[1] * e]);
        }
    }
    let id = |k: usize, j: usize| (k * 5 + j % 5) as u32;
    f.faces.push((0..5).map(|j| id(0, j)).collect());
    f.faces.push((0..5).rev().map(|j| id(s - 1, j)).collect());
    for k in 0..s - 1 {
        for j in 0..5 {
            f.faces.push(vec![id(k, j), id(k + 1, j), id(k + 1, j + 1)]);
            f.faces.push(vec![id(k, j), id(k + 1, j + 1), id(k, j + 1)]);
        }
    }
    if f.volume() < 0.0 {
        f.faces.iter_mut().for_each(|p| p.reverse());
    }
    f
}

/// La texture d'une tuile de boîte `b` : l'occupation du sol (les classes des points
/// voisins mêlées, de près), l'eau ; hors du lieu, transparent. Sous le dessus, la
/// coupe de la terre.
pub fn texture(d: &Donnees, b: [f32; 4]) -> Vec<u8> {
    let l = &d.lieu;
    let e = l.etendue();
    let pas = (b[2] - b[0]) / (TEX - 1) as f32;
    let fin = pas < (l.pas * 2.0) as f32;
    let rives = Rives::new(d, b);
    let mut px = vec![0u8; TEX * (TEX + BANDE) * 4];
    let mut mettre = |i: usize, c: [f32; 3], a: u8| {
        for k in 0..3 {
            px[i * 4 + k] = (c[k] * 255.0).round().clamp(0.0, 255.0) as u8;
        }
        px[i * 4 + 3] = a;
    };
    for y in 0..TEX {
        let z = (b[1] + y as f32 * pas) as f64;
        for x in 0..TEX {
            let xx = (b[0] + x as f32 * pas) as f64;
            let i = y * TEX + x;
            // Un demi-pixel de tolérance : le bord du lieu reste peint
            let marge = pas as f64;
            if xx < e[0] - marge || xx > e[2] + marge || z < e[1] - marge || z > e[3] + marge {
                continue;
            }
            if rives.dans_eau(d, xx as f32, z as f32) {
                mettre(i, LIT, 255);
                continue;
            }
            let c = if fin {
                let mut poids = [0f32; CLASSES as usize];
                for (k, w) in l.occupation(xx, z) {
                    poids[(k as usize).min(CLASSES as usize - 1)] += w as f32;
                }
                let t: f32 = poids.iter().sum::<f32>().max(1e-6);
                let mut c = [0f32; 3];
                for (k, w) in poids.iter().enumerate() {
                    let cs = couleur_sol(k as u8);
                    for j in 0..3 {
                        c[j] += cs[j] * w / t;
                    }
                }
                c
            } else {
                couleur_sol(l.classe(xx, z))
            };
            mettre(i, c, 255);
        }
    }
    for y in 0..BANDE {
        let c = couleur_coupe(y as f32 / (BANDE - 1) as f32);
        for x in 0..TEX {
            mettre((TEX + y) * TEX + x, c, 255);
        }
    }
    px
}

/// Ce que l'interface demande et ce que la fabrique rend.
#[derive(Default)]
struct Echange {
    /// Les tuiles voulues, dans l'ordre où les faire, avec la retouche de la carte.
    voulues: Vec<(Cle, u64)>,
    /// Celle en cours.
    en_cours: Option<Cle>,
    faites: Vec<Tuile>,
}

/// Le fil qui fabrique les tuiles de la maquette.
pub struct Fabrique {
    echange: Arc<(Mutex<Echange>, Condvar)>,
    arret: Arc<AtomicBool>,
    fil: Option<std::thread::JoinHandle<()>>,
}

/// Baisse la priorité du fil courant (Linux, Android : `setpriority` sur le fil).
fn baisser_priorite() {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        extern "C" {
            fn setpriority(which: i32, who: u32, prio: i32) -> i32;
        }
        // PRIO_PROCESS, 0 : le fil appelant (sous Linux) ; 10 : bien en dessous de l'interface
        unsafe {
            setpriority(0, 0, 10);
        }
    }
}

impl Fabrique {
    pub fn new(donnees: Arc<RwLock<Donnees>>) -> Fabrique {
        let echange: Arc<(Mutex<Echange>, Condvar)> = Default::default();
        let arret = Arc::new(AtomicBool::new(false));
        let (e, a) = (echange.clone(), arret.clone());
        let fil = std::thread::Builder::new()
            .name("maquette".into())
            .spawn(move || {
                baisser_priorite();
                loop {
                    // La prochaine tuile voulue
                    let (cle, faite) = {
                        let (m, c) = &*e;
                        let Ok(mut g) = m.lock() else { return };
                        loop {
                            if a.load(Ordering::Relaxed) {
                                return;
                            }
                            if !g.voulues.is_empty() {
                                break;
                            }
                            g = match c.wait(g) {
                                Ok(g) => g,
                                Err(_) => return,
                            };
                        }
                        let t = g.voulues.remove(0);
                        g.en_cours = Some(t.0);
                        t
                    };
                    let fait = donnees.read().ok().map(|d| tuile(&d, cle, faite));
                    let (m, _) = &*e;
                    let Ok(mut g) = m.lock() else { return };
                    g.en_cours = None;
                    if let Some(t) = fait {
                        g.faites.push(t);
                    }
                }
            })
            .ok();
        Fabrique { echange, arret, fil }
    }

    /// Les tuiles voulues, dans l'ordre (remplace la demande d'avant).
    pub fn demander(&self, voulues: Vec<(Cle, u64)>) {
        let (m, c) = &*self.echange;
        if let Ok(mut g) = m.lock() {
            let en_cours = g.en_cours;
            g.voulues = voulues.into_iter().filter(|v| Some(v.0) != en_cours).collect();
            c.notify_one();
        }
    }

    /// Prend au plus `n` tuiles finies.
    pub fn prendre(&self, n: usize) -> Vec<Tuile> {
        let (m, _) = &*self.echange;
        m.lock().map(|mut g| {
            let k = g.faites.len().min(n);
            g.faites.drain(..k).collect()
        }).unwrap_or_default()
    }

    /// Des tuiles demandées, en cours ou finies attendent.
    pub fn occupee(&self) -> bool {
        let (m, _) = &*self.echange;
        m.lock().is_ok_and(|g| !g.voulues.is_empty() || g.en_cours.is_some() || !g.faites.is_empty())
    }

    /// Les clés en cours ou demandées (pour ne pas les redemander).
    pub fn attendues(&self) -> HashSet<Cle> {
        let (m, _) = &*self.echange;
        m.lock().map(|g| g.voulues.iter().map(|v| v.0).chain(g.en_cours).collect()).unwrap_or_default()
    }
}

impl Drop for Fabrique {
    fn drop(&mut self) {
        self.arret.store(true, Ordering::Relaxed);
        self.echange.1.notify_all();
        // Sans attendre : la tuile en cours finit seule
        self.fil.take();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sage_reel::{cote, cote_sol, Lieu, BOIS};

    fn donnees() -> Donnees {
        let pas = 64.0;
        let n = cote(pas);
        let h: Vec<f32> = (0..n * n).map(|i| ((i % n) as f32 * 0.3).sin() * 20.0 + 50.0).collect();
        let eau: Vec<bool> = (0..n * n).map(|i| i % n < 20).collect();
        let m = cote_sol(n);
        Donnees::new(Lieu::new("Essai", 49.0, 1.0, 0.0, 0.0, pas, 40.0, &h, &eau, Vec::new(), Vec::new(), Vec::new()).avec_sol(vec![BOIS; m * m]))
    }

    #[test]
    fn une_tuile_a_un_dessus_des_murs_et_un_dessous() {
        let d = donnees();
        let (v, ind, px) = fabriquer(&d, (3, 4, 4));
        let socle = d.lieu.base - EPAISSEUR;
        assert_eq!(px.len(), TEX * (TEX + BANDE) * 4);
        assert!(ind.iter().all(|&i| (i as usize) < v.len() / SOMMET));
        let ys: Vec<f32> = v.chunks(SOMMET).map(|s| s[1]).collect();
        // Le dessus suit le relief, les murs descendent au socle
        assert!(ys.iter().any(|&y| (y - socle).abs() < 1e-3));
        assert!(ys.iter().cloned().fold(f32::MIN, f32::max) > socle + EPAISSEUR);
        // Tous les triangles du dessus regardent en haut, ceux du dessous en bas
        let p = |i: u32| { let k = i as usize * SOMMET; [v[k], v[k + 1], v[k + 2]] };
        let normale = |t: &[u32]| {
            let (a, b, c) = (p(t[0]), p(t[1]), p(t[2]));
            let (u, w) = ([b[0] - a[0], b[1] - a[1], b[2] - a[2]], [c[0] - a[0], c[1] - a[1], c[2] - a[2]]);
            [u[1] * w[2] - u[2] * w[1], u[2] * w[0] - u[0] * w[2], u[0] * w[1] - u[1] * w[0]]
        };
        let haut = |t: &[u32]| t.iter().all(|&i| p(i)[1] > socle + 1.0);
        assert!(ind.chunks(3).filter(|t| haut(t)).all(|t| normale(t)[1] >= -1e-6), "le dessus vu d'en haut");
        assert!(ind.chunks(3).any(|t| haut(t)), "un dessus");
        assert!(ind.chunks(3).any(|t| t.iter().all(|&i| (p(i)[1] - socle).abs() < 1e-3) && normale(t)[1] < 0.0), "le dessous à l'inverse du dessus");
        // En blanc : les arêtes du dessus, aucune debout (les murs)
        let a = aretes(&v, &ind);
        assert!(!a.is_empty());
        let x = |i: u32| v[i as usize * SOMMET];
        let z = |i: u32| v[i as usize * SOMMET + 2];
        assert!(a.chunks(2).all(|e| !((x(e[0]) - x(e[1])).abs() < 1e-4 && (z(e[0]) - z(e[1])).abs() < 1e-4)), "pas d'arête verticale de mur");
        // Le contour : le tour du dessus et du dessous, les quatre coins debout
        let c = contour(&v, &ind);
        let debout = c.chunks(2).filter(|e| (x(e[0]) - x(e[1])).abs() < 1e-4 && (z(e[0]) - z(e[1])).abs() < 1e-4).count();
        assert_eq!(debout, 4);
        assert!(c.len() / 2 >= 4 + 2 * 4);
        let m = mesurer(&v, &ind, px.len(), 0);
        assert_eq!((m.sommets, m.triangles), (v.len() / SOMMET, ind.len() / 3));
        assert!(m.aretes > m.triangles && m.octets > px.len());
    }

    /// Une boîte (x0, y0, z0) – (x1, y1, z1), faces tournées dehors.
    fn boite(a: [f32; 3], b: [f32; 3]) -> Forme {
        let s = |i: usize| [if i & 1 == 0 { a[0] } else { b[0] }, if i & 2 == 0 { a[1] } else { b[1] }, if i & 4 == 0 { a[2] } else { b[2] }];
        Forme { sommets: (0..8).map(s).collect(), faces: vec![vec![2, 3, 1, 0], vec![5, 7, 6, 4], vec![1, 5, 4, 0], vec![6, 7, 3, 2], vec![4, 6, 2, 0], vec![3, 7, 5, 1]] }
    }

    #[test]
    fn le_solide_est_ferme_se_creuse_et_s_aplatit() {
        let d = donnees();
        let cle = (5, 13, 15);
        let f = solide(&d, cle);
        let b = boite_tuile(&d, cle);
        let socle = socle(&d);
        // Son volume : la colonne de terre sous le dessus, à peu près
        let (cx, cz) = ((b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0);
        let attendu = (haut(&d, &Rives::new(&d, b), cx, cz) - socle) as f64 * ((b[2] - b[0]) * (b[3] - b[1])) as f64;
        let vol = f.volume();
        assert!(vol > 0.0 && (vol - attendu).abs() / attendu < 0.1, "{vol} {attendu}");
        // Une voie en creux : la différence passe, le volume baisse de celui de la voie
        let y = haut(&d, &Rives::new(&d, b), cx, cz);
        let voie = boite([b[0] - 10.0, y - 2.0, cz - 4.0], [b[2] + 10.0, y + 30.0, cz + 4.0]);
        let r = crate::booleens::operation(&f, &voie, crate::booleens::Op::Difference).expect("le solide est une variété fermée");
        assert!(r.volume() < vol && r.volume() > vol * 0.5, "{} {vol}", r.volume());
        // Aplati : plus rien au socle, et rien que des faces du dessus
        let a = aplatir(&f, socle);
        assert!(a.sommets.iter().all(|p| p[1] > socle + 1.0));
        assert_eq!(a.faces.len(), COTES * COTES * 2);
        // Habillé : des triangles valides
        let (v, ind) = habiller(&d, &r, b);
        assert!(!ind.is_empty() && ind.iter().all(|&i| (i as usize) < v.len() / SOMMET));
    }

    #[test]
    fn au_bord_du_lieu_la_tuile_s_arrete_et_reste_fermee() {
        let d = donnees();
        let petit = d.lieu.recadrer(sage_reel::Grille::emprise(64.0, 3000.0, 2000.0));
        let d = Donnees::new(petit);
        let e = d.lieu.etendue();
        // Le lieu est plus large que haut : une tuile déborde au sud
        let (_, z0, c) = d.monde();
        let t = c / 8.0;
        let j = ((e[3] as f32 - z0) / t).floor() as i32;
        let cle = (3u8, 2, j);
        assert!(boite_tuile(&d, cle)[3] > e[3] as f32 + 1.0);
        let f = solide(&d, cle);
        assert!(f.sommets.iter().all(|p| p[2] as f64 <= e[3] + 1e-3 && p[2] as f64 >= e[1] - 1e-3));
        assert!(f.volume() > 0.0, "{} sommets, {} faces, vol {}, e {e:?}, cle {cle:?}", f.sommets.len(), f.faces.len(), f.volume());
        let b = boite_tuile(&d, cle);
        let voie = boite([b[0], -1000.0, b[1]], [b[0] + 20.0, 1000.0, b[1] + 20.0]);
        assert!(crate::booleens::operation(&f, &voie, crate::booleens::Op::Difference).is_ok());
    }

    #[test]
    fn la_texture_est_le_sol_et_le_lit() {
        let d = donnees();
        let b = boite_tuile(&d, (2, 0, 1));
        let px = texture(&d, b);
        let bois = couleur_sol(BOIS).map(|c| (c * 255.0).round() as u8);
        let eau = LIT.map(|c| (c * 255.0).round() as u8);
        let pixels: Vec<[u8; 4]> = px.chunks(4).take(TEX * TEX).map(|c| [c[0], c[1], c[2], c[3]]).collect();
        assert!(pixels.iter().any(|p| p[..3] == bois[..]));
        assert!(pixels.iter().any(|p| p[..3] == eau[..]));
        // La coupe, sous le dessus, est opaque
        assert!(px[TEX * TEX * 4..].chunks(4).all(|c| c[3] == 255));
        // Le lit est creusé sous le niveau ; la surface d'eau, à part, est à plat au
        // niveau, de sa matière
        let (v, _, _) = fabriquer(&d, (2, 0, 1));
        let niveau = d.lieu.niveau;
        assert!(v.chunks(SOMMET).all(|s| s[8] == MAQUETTE));
        assert!(v.chunks(SOMMET).any(|s| s[1] <= niveau - RIVE_NATURELLE + 0.01 && s[1] >= niveau - PROFONDEUR - 0.01));
        let (e, ie) = surface_eau(&d, (2, 0, 1));
        assert!(!ie.is_empty() && e.chunks(SOMMET).all(|s| s[8] == MAQUETTE_EAU && s[1] == niveau));
    }

    #[test]
    fn les_berges_d_osm_disent_l_eau_et_la_pente_du_lit() {
        let mut d = donnees();
        let carre = vec![[-100.0, -100.0], [100.0, -100.0], [100.0, 100.0], [-100.0, 100.0]];
        d.lieu.surfaces_eau = vec![sage_reel::cours_eau::SurfaceEau { nature: 0, anneaux: vec![carre] }];
        let r = Rives::new(&d, [-50.0, -50.0, 50.0, 50.0]);
        assert!(r.vectorielles());
        assert!(r.dans_eau(&d, 0.0, 0.0) && !r.dans_eau(&d, 120.0, 0.0));
        assert_eq!(profondeur(&d, &r, 0.0, 0.0), PROFONDEUR);
        assert_eq!(profondeur(&d, &r, 120.0, 0.0), 0.0);
        let pres = profondeur(&d, &r, 99.0, 0.0);
        let q = r.berge(99.0, 0.0).1;
        assert!((pres - (sous_rive(q) + pente_lit(q))).abs() < 1e-3, "{pres} {q}");
        // Loin de toute surface : la grille
        assert!(!Rives::new(&d, [5000.0, 5000.0, 5100.0, 5100.0]).vectorielles());
        // Une tuile que la rive traverse : coupée sur elle, avec sa berge debout, et
        // toujours fermée
        let (x0, z0, c) = d.monde();
        let n = 7u8;
        let t = c / (1u32 << n) as f32;
        let cle = (n, ((100.0 - x0) / t).floor() as i32, ((0.0 - z0) / t).floor() as i32);
        let f = solide(&d, cle);
        let berge = f.sommets.iter().filter(|p| (p[0] - 100.0).abs() < 1e-3 && p[1] < d.lieu.niveau).count();
        assert!(berge >= 2, "{berge}");
        assert!(crate::booleens::operation(&f, &boite([-1e4, -1e4, -1e4], [1e4, 1e4, 1e4]), crate::booleens::Op::Intersection).is_ok());
    }

    #[test]
    fn le_remblai_borde_la_rive_et_s_unit_a_la_tuile() {
        let mut d = donnees();
        let carre = vec![[-100.0, -100.0], [100.0, -100.0], [100.0, 100.0], [-100.0, 100.0]];
        d.lieu.surfaces_eau = vec![sage_reel::cours_eau::SurfaceEau { nature: 0, anneaux: vec![carre] }];
        let (x0, z0, c) = d.monde();
        let n = 7u8;
        let t = c / (1u32 << n) as f32;
        let cle = (n, ((100.0 - x0) / t).floor() as i32, ((0.0 - z0) / t).floor() as i32);
        let (r, echecs) = remblai(&d, cle);
        assert_eq!(echecs, 0);
        assert!(r.volume() > 0.0);
        // Il reste dans la tuile, côté terre (son mur avance à peine dans l'eau)
        let b = boite_tuile(&d, cle);
        assert!(r.sommets.iter().all(|p| p[0] >= b[0] && p[0] <= b[2] && p[2] >= b[1] && p[2] <= b[3]));
        let dedans = |p: &[f32; 3]| p[0].abs() < 100.0 - AVANCE - 1e-3 && p[2].abs() < 100.0 - AVANCE - 1e-3;
        assert!(!r.sommets.iter().any(dedans), "{:?}", r.sommets.iter().find(|p| dedans(p)));
        let f = remblayer(&d, cle, solide(&d, cle));
        assert!(f.volume() > solide(&d, cle).volume() * 0.99);
        let (v, ind, _) = fabriquer(&d, cle);
        assert!(!v.is_empty() && !ind.is_empty());
    }

    /// Banc : FLOW_LIEU ; le remblai des tuiles le long d'une berge.
    #[test]
    #[ignore]
    fn banc_remblai() {
        let l = sage_reel::Lieu::lire(std::path::Path::new(&std::env::var("FLOW_LIEU").unwrap())).unwrap();
        let d = Donnees::new(l);
        let b = d.berges();
        let g = b.iter().flatten().max_by_key(|g| g.points.len()).unwrap();
        let (x0, z0, c) = d.monde();
        for n in [5u8, 6, 7] {
            let t = c / (1u32 << n) as f32;
            let (mut total, mut pire, mut echecs, mut nb, mut ratees) = (0.0, 0.0f64, 0, 0, 0);
            let mut vues = std::collections::HashSet::new();
            for p in g.points.iter().step_by(6).take(40) {
                let cle = (n, ((p[0] - x0) / t).floor() as i32, ((p[1] - z0) / t).floor() as i32);
                if !vues.insert(cle) {
                    continue;
                }
                let t0 = std::time::Instant::now();
                let f = solide(&d, cle);
                let (r, e) = remblai(&d, cle);
                let u = crate::booleens::operation(&f, &r, crate::booleens::Op::Union);
                let dt = t0.elapsed().as_secs_f64() * 1000.0;
                total += dt;
                pire = pire.max(dt);
                echecs += e;
                if let Err(e) = &u {
                    eprintln!("  {cle:?} : {e} ; remblai {} sommets, vol {:.0}, solide vol {:.0}", r.sommets.len(), r.volume(), f.volume());
                }
                ratees += u.is_err() as usize;
                nb += 1;
            }
            eprintln!("niveau {n} : {nb} tuiles, {:.0} ms en moyenne, pire {pire:.0} ms, {echecs} morceaux ratés, {ratees} unions ratées", total / nb as f64);
        }
    }
}