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
    /// Le volume de défonce de ses lits (`maillage_defonce`) : faces et arêtes.
    pub defonce_sommets: Vec<f32>,
    pub defonce_indices: Vec<u32>,
    pub defonce_aretes: Vec<u32>,
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
    tuile_pour(d, cle, faite, false)
}

/// Une tuile finie pour la Maquette, ou pour la vue Matérialisation (`materialiser`) :
/// celle-ci est faite par `materialisation::tuile`.
pub fn tuile_pour(d: &Donnees, cle: Cle, faite: u64, materialiser: bool) -> Tuile {
    let mut suivi = crate::suivi::tache("tuile");
    suivi.detail(format!("{}/{}/{}", cle.0, cle.1, cle.2));
    let _ = faite;
    // La Matérialisation (`materialisation`) : ses volumes de défonce des voies sont
    // joints en matière `MAQUETTE_DEFONCE`, mis à part pour la défonce
    let fait: Option<Result<(Vec<f32>, Vec<u32>, Vec<u8>), String>> = materialiser.then(|| Ok(crate::materialisation::tuile(d, cle)));
    let mut apart = None;
    let (sommets, indices, pixels, def) = match fait {
        Some(Ok((s, i, p))) => {
            let _t = crate::suivi::tache("défonce");
            // Les triangles de matière `MAQUETTE_DEFONCE` (les volumes de défonce des
            // voies) : montrés avec la défonce
            let (i, a) = separer_defonce(&s, i);
            apart = a;
            (s, i, p, defonce(d, cle))
        }
        _ => fabriquer_avec_defonce(d, cle),
    };
    let t = crate::suivi::tache("finitions");
    let (mut defonce_sommets, mut defonce_indices, mut defonce_aretes) = maillage_defonce(&def);
    if let Some((v, ind)) = apart {
        let base = (defonce_sommets.len() / SOMMET) as u32;
        let ar = crate::relief3d::aretes(&v, SOMMET, &ind, true);
        defonce_sommets.extend_from_slice(&v);
        defonce_indices.extend(ind.iter().map(|k| k + base));
        defonce_aretes.extend(ar.iter().map(|k| k + base));
    }
    let aretes = aretes(&sommets, &indices);
    let contour = contour(&sommets, &indices);
    let (eau_sommets, eau_indices) = {
        let _t = crate::suivi::tache("eau");
        surface_eau(d, cle)
    };
    drop(t);
    let mut mesure = mesurer(&sommets, &indices, pixels.len(), aretes.len() + contour.len());
    mesure.sommets += eau_sommets.len() / SOMMET;
    mesure.triangles += eau_indices.len() / 3;
    mesure.octets += (eau_sommets.len() + eau_indices.len()) * 4;
    suivi.detail(format!("{}/{}/{} · {} triangles", cle.0, cle.1, cle.2, mesure.triangles));
    Tuile { cle, faite, sommets, indices, pixels, aretes, contour, eau_sommets, eau_indices, defonce_sommets, defonce_indices, defonce_aretes, mesure }
}

/// Sépare des triangles d'une tuile ceux dont les trois sommets sont de matière
/// `MAQUETTE_DEFONCE` : les indices restants, et les triangles ôtés (leurs sommets
/// recopiés, réindexés), s'il y en a.
fn separer_defonce(s: &[f32], ind: Vec<u32>) -> (Vec<u32>, Option<(Vec<f32>, Vec<u32>)>) {
    let est = |k: u32| (s[k as usize * SOMMET + 8] - MAQUETTE_DEFONCE).abs() < 1e-3;
    if !ind.chunks(3).any(|t| t.iter().all(|&k| est(k))) {
        return (ind, None);
    }
    let (mut garde, mut v, mut vi) = (Vec::with_capacity(ind.len()), Vec::new(), Vec::new());
    let mut nouveaux: HashMap<u32, u32> = HashMap::new();
    for t in ind.chunks(3) {
        if t.iter().all(|&k| est(k)) {
            for &k in t {
                let n = *nouveaux.entry(k).or_insert_with(|| {
                    v.extend_from_slice(&s[k as usize * SOMMET..(k as usize + 1) * SOMMET]);
                    (v.len() / SOMMET - 1) as u32
                });
                vi.push(n);
            }
        } else {
            garde.extend_from_slice(t);
        }
    }
    (garde, Some((v, vi)))
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
pub fn terre(d: &Donnees, _rives: &Rives, x: f32, z: f32) -> f32 {
    // Le remblai des quais (`remblais`), uni ensuite, se montre là où il dépasse
    relief_sec(d, x, z)
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
    let rives = Rives::new(d, boite_tuile(d, cle));
    let mut f = Forme::default();
    let mut signe: Vec<f32> = Vec::with_capacity(nx * nz);
    for &z in &zs {
        for &x in &xs {
            signe.push(rives.signe(d, x, z));
            f.sommets.push([x, non_creuse(d, &rives, x, z), z]);
        }
    }
    let w = nx as u32;
    for k in 0..nz as u32 - 1 {
        for a in 0..w - 1 {
            let p = k * w + a;
            f.faces.push(vec![p, p + w, p + 1]);
            f.faces.push(vec![p + 1, p + w, p + w + 1]);
        }
    }
    murs_et_dessous(&mut f, &xs, &zs, socle(d), &HashMap::new(), &signe);
    f
}

/// Le dessus non creusé en (x, z) : la terre ; dans l'eau, à plat à son niveau (le lit
/// s'y fait par la défonce).
fn non_creuse(d: &Donnees, rives: &Rives, x: f32, z: f32) -> f32 {
    if rives.signe(d, x, z) > 0.0 { d.lieu.niveau } else { terre(d, rives, x, z) }
}

/// Le dépassement du dessus de la défonce au-dessus du plus haut point qu'elle
/// traverse (m).
pub const DEPASSE: f32 = 0.5;

/// Le solide de la tuile `cle`, creusé de ses lits (la différence avec `defonce`) ; tel
/// quel s'il n'y a pas d'eau ou si la différence échoue.
pub fn creuser(f: Forme, def: &Forme) -> Forme {
    if def.vide() {
        return f;
    }
    match crate::booleens::operation(&f, def, crate::booleens::Op::Difference) {
        Ok(c) => {
            if std::env::var_os("FLOW_TRACE_DEFONCE").is_some() {
                eprintln!("creuser : volume {:.0} -> {:.0} m3 (défonce {:.0} m3)", f.volume(), c.volume(), def.volume());
            }
            c
        }
        Err(e) => {
            if std::env::var_os("FLOW_TRACE_DEFONCE").is_some() {
                eprintln!("creuser : la différence a échoué : {e:?}");
            }
            f
        }
    }
}

/// Le volume positif de la défonce des lits de la tuile `cle` : ce qu'on ôte au
/// solide non creusé pour y faire le lit. Son plan suit les berges d'OSM (la rive
/// coupe les cases de la grille de la tuile, prolongée d'une case tout autour : le
/// volume déborde de la tuile, pas de face confondue avec ses murs) ; son fond est le
/// lit (`lit` : la profondeur selon la distance à la rive et le type de la berge) ;
/// son dessus, à plat, passe `DEPASSE` au-dessus du plus haut point du dessus non
/// creusé qu'il couvre ; des murs debout sur son pourtour. Une variété fermée par
/// étendue d'eau, faces tournées dehors.
pub fn defonce(d: &Donnees, cle: Cle) -> Forme {
    let (mut xs, mut zs) = grille(d, cle);
    let mut r = Forme::default();
    if xs.len() < 2 || zs.len() < 2 {
        return r;
    }
    for v in [&mut xs, &mut zs] {
        let (pa, pb) = (v[1] - v[0], v[v.len() - 1] - v[v.len() - 2]);
        v.insert(0, v[0] - pa);
        v.push(v[v.len() - 1] + pb);
    }
    let (nx, nz) = (xs.len(), zs.len());
    let rives = Rives::new(d, boite_tuile(d, cle));
    let mut sommets: Vec<[f32; 3]> = Vec::with_capacity(nx * nz);
    let mut signe: Vec<f32> = Vec::with_capacity(nx * nz);
    for &z in &zs {
        for &x in &xs {
            let s = rives.signe(d, x, z);
            signe.push(s);
            sommets.push([x, if s > 0.0 { lit(d, &rives, x, z) } else { terre(d, &rives, x, z) }, z]);
        }
    }
    // Le plus haut du dessus non creusé sous la défonce : l'eau, et la terre des
    // cases que la rive coupe
    let mut haut = d.lieu.niveau;
    // Le fond : les cases (ou leurs morceaux) dans l'eau ; la rive coupe les côtés
    let mut coupes: HashMap<(u32, u32), u32> = HashMap::new();
    let mut fond: Vec<Vec<u32>> = Vec::new();
    let w = nx as u32;
    let mouille = |i: u32| signe[i as usize] > 0.0;
    for k in 0..nz as u32 - 1 {
        for a in 0..w - 1 {
            let p = k * w + a;
            for t in [[p, p + w, p + 1], [p + 1, p + w, p + w + 1]] {
                let m = t.map(mouille);
                if m.iter().all(|&e| e) {
                    fond.push(t.to_vec());
                    continue;
                }
                if !m.iter().any(|&e| e) {
                    continue;
                }
                for &i in &t {
                    if !mouille(i) {
                        haut = haut.max(sommets[i as usize][1]);
                    }
                }
                // Le sommet seul de son côté de la rive en tête (l'ordre tournant gardé)
                let r0 = (0..3).find(|&i| m[i] != m[(i + 1) % 3] && m[i] != m[(i + 2) % 3]).unwrap();
                let (sa, sb, sc) = (t[r0], t[(r0 + 1) % 3], t[(r0 + 2) % 3]);
                let mut couper = |u: u32, v: u32| -> u32 {
                    *coupes.entry((u.min(v), u.max(v))).or_insert_with(|| {
                        let (su, sv) = (signe[u as usize], signe[v as usize]);
                        let t = (su / (su - sv)).clamp(0.0, 1.0);
                        let (pu, pv) = (sommets[u as usize], sommets[v as usize]);
                        let (x, z) = (pu[0] + (pv[0] - pu[0]) * t, pu[2] + (pv[2] - pu[2]) * t);
                        sommets.push([x, lit(d, &rives, x, z), z]);
                        (sommets.len() - 1) as u32
                    })
                };
                let (xab, xac) = (couper(sa, sb), couper(sa, sc));
                if m[r0] {
                    fond.push(vec![sa, xab, xac]);
                } else {
                    fond.push(vec![xab, sb, sc, xac]);
                }
            }
        }
    }
    if fond.is_empty() {
        return r;
    }
    let haut = haut + DEPASSE;
    // Les sommets du fond, puis ceux du dessus (`n` plus loin)
    let mut numero: HashMap<u32, u32> = HashMap::new();
    for p in &fond {
        for &i in p.iter() {
            numero.entry(i).or_insert_with(|| {
                r.sommets.push(sommets[i as usize]);
                (r.sommets.len() - 1) as u32
            });
        }
    }
    let n = r.sommets.len() as u32;
    for k in 0..n as usize {
        let p = r.sommets[k];
        r.sommets.push([p[0], haut, p[2]]);
    }
    let mut cotes: HashSet<(u32, u32)> = HashSet::new();
    for p in &fond {
        let q: Vec<u32> = p.iter().map(|i| numero[i]).collect();
        for k in 0..q.len() {
            cotes.insert((q[k], q[(k + 1) % q.len()]));
        }
        r.faces.push(q.iter().rev().copied().collect());
        r.matieres.push(matiere::LIT);
        r.faces.push(q.iter().map(|&i| i + n).collect());
        r.matieres.push(matiere::HAUT);
    }
    // Les murs : sur chaque côté du pourtour (sans son côté retourné)
    for &(a, b) in &cotes {
        if !cotes.contains(&(b, a)) {
            r.faces.push(vec![a, b, b + n, a + n]);
            r.matieres.push(matiere::BERGE);
        }
    }
    r
}

/// Les matières des faces de la maquette (`Forme::matieres`), que les opérations
/// transportent : chaque volume marque les faces qu'il laisse dans la tuile.
pub mod matiere {
    /// Le solide de la tuile (le relief, ses murs et son dessous).
    pub const SOL: u16 = 0;
    /// Le fond de la défonce : le lit.
    pub const LIT: u16 = 1;
    /// Les murs de la défonce : la berge (le mur du quai).
    pub const BERGE: u16 = 2;
    /// Le remblai des quais.
    pub const REMBLAI: u16 = 3;
    /// Le fond de l'épaulement : le dessus du quai.
    pub const QUAI: u16 = 4;
    /// Les murs de l'épaulement.
    pub const QUAI_MUR: u16 = 5;
    /// Le dessus d'un volume ôté : ne devrait jamais rester visible.
    pub const HAUT: u16 = 6;
    /// La chaussée laissée par la défonce des voies (la Matérialisation) : le bitume.
    pub const CHAUSSEE: u16 = 7;
    /// Les côtés et le dessus de la défonce des voies : le béton clair.
    pub const BETON: u16 = 8;
    /// Les murs et toits des bâtiments : huit tons de pierre et de gris clair, de
    /// `BATI` à `BATI + TONS_BATI - 1` ; chaque bâtiment tire le sien de son numéro.
    pub const BATI: u16 = 9;
    pub const TONS_BATI: u16 = 8;
    /// La chaussée des voies de bus en site propre (le TEOR à Rouen) : le bitume
    /// bordeaux.
    pub const CHAUSSEE_BUS: u16 = BATI + TONS_BATI;
    /// La chaussée sous terre (dans un tunnel) : le bitume, plus sombre ; la carte le
    /// dit à qui roule dessus (la réverbération de la voûte).
    pub const CHAUSSEE_TUNNEL: u16 = CHAUSSEE_BUS + 1;
    /// Les chaussées selon leur revêtement d'OSM (`surface=*`, lu dans le lieu) : béton,
    /// pavés, gravier, terre, herbe (le bitume est `CHAUSSEE`).
    pub const CHAUSSEE_BETON: u16 = CHAUSSEE_TUNNEL + 1;
    pub const CHAUSSEE_PAVES: u16 = CHAUSSEE_BETON + 1;
    pub const CHAUSSEE_GRAVIER: u16 = CHAUSSEE_PAVES + 1;
    pub const CHAUSSEE_TERRE: u16 = CHAUSSEE_GRAVIER + 1;
    pub const CHAUSSEE_HERBE: u16 = CHAUSSEE_TERRE + 1;

    /// La chaussée d'un revêtement du lieu (`sage_reel::BITUME`…).
    pub fn chaussee_de(revetement: u8) -> u16 {
        match revetement {
            sage_reel::BETON => CHAUSSEE_BETON,
            sage_reel::PAVES => CHAUSSEE_PAVES,
            sage_reel::GRAVIER => CHAUSSEE_GRAVIER,
            sage_reel::TERRE => CHAUSSEE_TERRE,
            sage_reel::HERBE => CHAUSSEE_HERBE,
            _ => CHAUSSEE,
        }
    }
    /// Les tons des bâtiments (le shader et la carte les reprennent), proches du blanc
    /// cassé : calcaire, pierre de Caen, sable pâle, pierre grise, gris clair, gris perle,
    /// gris chaud, ocre très pâle.
    pub const TEINTES_BATI: [[f32; 3]; TONS_BATI as usize] = [
        [0.92, 0.90, 0.85],
        [0.91, 0.88, 0.80],
        [0.89, 0.86, 0.79],
        [0.85, 0.84, 0.81],
        [0.88, 0.88, 0.87],
        [0.93, 0.93, 0.92],
        [0.87, 0.85, 0.82],
        [0.91, 0.87, 0.78],
    ];
    /// La matière du bâtiment `k` : un ton tiré au hasard, toujours le même pour lui.
    pub fn bati(k: u32) -> u16 {
        let mut h = k.wrapping_mul(0x9E37_79B9);
        h ^= h >> 15;
        h = h.wrapping_mul(0x85EB_CA6B);
        h ^= h >> 13;
        BATI + (h % TONS_BATI as u32) as u16
    }
    /// Le flottant « matière » d'un sommet qui montre la matière `m` (le shader la
    /// teinte : 5,1 le lit, 5,2 la berge…).
    pub fn sommet(m: u16) -> f32 {
        5.0 + m as f32 * 0.1
    }
}

/// La largeur de la bordure du quai (les pierres de taille le long de son bord, avant
/// les pavés), en mètres : le shader la lit dans `u_bordure`.
pub const BORDURE: f32 = 0.6;

/// La matière des volumes de défonce (dessinés à part, semi-transparents).
pub const MAQUETTE_DEFONCE: f32 = 4.3;

/// Le maillage à dessiner d'un volume de défonce (sommets de `SOMMET` flottants, à
/// plat : la normale de chaque face, matière `MAQUETTE_DEFONCE`) et ses arêtes vives
/// (en indices deux par deux).
pub fn maillage_defonce(f: &Forme) -> (Vec<f32>, Vec<u32>, Vec<u32>) {
    let (mut v, mut ind, mut aretes) = (Vec::new(), Vec::new(), Vec::new());
    if f.vide() {
        return (v, ind, aretes);
    }
    for p in &f.faces {
        // La normale de Newell (les faces à quatre côtés ne sont pas toujours planes)
        let mut nm = [0f32; 3];
        for k in 0..p.len() {
            let (a, b) = (f.sommets[p[k] as usize], f.sommets[p[(k + 1) % p.len()] as usize]);
            nm[0] += (a[1] - b[1]) * (a[2] + b[2]);
            nm[1] += (a[2] - b[2]) * (a[0] + b[0]);
            nm[2] += (a[0] - b[0]) * (a[1] + b[1]);
        }
        let l = (nm[0] * nm[0] + nm[1] * nm[1] + nm[2] * nm[2]).sqrt().max(1e-9);
        let base = (v.len() / SOMMET) as u32;
        for &i in p {
            let s = f.sommets[i as usize];
            v.extend_from_slice(&[s[0], s[1], s[2], nm[0] / l, nm[1] / l, nm[2] / l, 0.0, 0.0, MAQUETTE_DEFONCE]);
        }
        for k in 1..p.len() as u32 - 1 {
            ind.extend_from_slice(&[base, base + k, base + k + 1]);
        }
    }
    // Ses arêtes vives : les bords et les plis de plus de 20°
    aretes = crate::relief3d::aretes(&v, SOMMET, &ind, true);
    (v, ind, aretes)
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
    for (k, p) in f.faces.iter().enumerate().filter(|(_, p)| p.iter().all(|&i| garde[i as usize])) {
        r.faces.push(p.iter().map(|&i| nouveau[i as usize]).collect());
        r.matieres.push(f.matiere(k));
    }
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
    for (kf, p) in f.faces.iter().enumerate() {
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
        let m = f.matiere(kf);
        let coins: Vec<u32> = if m != matiere::SOL {
            // Une face laissée par un volume : sa matière, à la normale de la face, ses
            // coordonnées de texture en mètres (vue d'en haut à plat ; debout, le long
            // du mur et en hauteur)
            // ; le dessus du quai, dans le repère de sa ligne : le long d'elle, et la
            // distance à elle (les pavés alignés, la bordure au bord)
            let l = (no[0] * no[0] + no[2] * no[2]).sqrt();
            let debout = no[1].abs() < 0.6 && l > 1e-6;
            let quai = (m == matiere::QUAI).then(|| d.remblais());
            q.iter().map(|s| {
                let (u, w) = match &quai {
                    Some(r) if !debout => r.repere([s[0], s[2]]).unwrap_or((s[0], s[2] + BORDURE)),
                    _ if debout => ((s[0] * no[2] - s[2] * no[0]) / l, -s[1]),
                    _ => (s[0], s[2]),
                };
                v.extend_from_slice(&[s[0], s[1], s[2], no[0], no[1], no[2], u, w, matiere::sommet(m)]);
                (v.len() / SOMMET - 1) as u32
            }).collect()
        } else if no[1] > 0.05 {
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
    let (v, ind, px, _) = fabriquer_avec_defonce(d, cle);
    (v, ind, px)
}

/// `fabriquer`, et le volume de défonce des lits de la tuile.
pub fn fabriquer_avec_defonce(d: &Donnees, cle: Cle) -> (Vec<f32>, Vec<u32>, Vec<u8>, Forme) {
    // La chaussée en volume sur le réseau nivelé (FLOW_SANS_CHAUSSEE=1 : sans),
    // seulement sur les tuiles assez fines pour qu'elle s'y voie (`chaussee::pour`) :
    // au-dessus, pas de voies du tout (la texture de la maquette ne les peint pas)
    // Mise de côté le 2026-11-16 (le réseau est tracé en surimpression à la place) :
    // FLOW_CHAUSSEE=1 pour la remettre
    if std::env::var("FLOW_CHAUSSEE").is_ok_and(|v| v == "1") && crate::chaussee::pour(d, cle) {
        return crate::chaussee::tuile(d, cle);
    }
    let b = boite_tuile(d, cle);
    let def = {
        let _t = crate::suivi::tache("défonce");
        defonce(d, cle)
    };
    // Le solide uni au remblai de ses berges, puis creusé de ses lits (la défonce
    // coupe aussi ce que le remblai avance dans l'eau), ses plans hachés par les
    // opérations redevenus simples
    // Les quais (remblais, épaulements) seulement sur les tuiles assez fines pour qu'ils
    // s'y voient (`quais_pour`) : la vue d'ensemble n'a que le solide creusé de ses lits
    let quais = quais_pour(d, cle);
    let rem = if quais {
        let _t = crate::suivi::tache("remblai");
        remblai_morceaux(d, cle)
    } else {
        Vec::new()
    };
    let epa = if quais {
        let _t = crate::suivi::tache("épaulement");
        epaulement_morceaux(d, cle)
    } else {
        Vec::new()
    };
    let mut etapes: Vec<(crate::booleens::Op, &Forme)> = rem.iter().map(|m| (crate::booleens::Op::Union, m)).collect();
    etapes.extend(epa.iter().map(|m| (crate::booleens::Op::Difference, m)));
    etapes.push((crate::booleens::Op::Difference, &def));
    let s = {
        let _t = crate::suivi::tache("solide");
        solide(d, cle)
    };
    let c = {
        let mut t = crate::suivi::tache("booléens");
        t.detail(format!("{} étapes", etapes.len()));
        composer(s, &etapes)
    };
    let f = {
        let _t = crate::suivi::tache("dissolution");
        crate::dissolution::dissoudre(&c)
    };
    let (v, ind) = {
        let _t = crate::suivi::tache("habillage");
        habiller(d, &f, b)
    };
    let px = {
        let _t = crate::suivi::tache("texture");
        texture(d, b)
    };
    (v, ind, px, def)
}

/// Le plus grand côté de tuile (m) qui reçoit les quais (remblais et épaulements ;
/// FLOW_QUAIS_COTE pour essayer) : au-delà, un pas de la grille fait des dizaines de
/// mètres, les quais ne s'y verraient pas et coûteraient des dizaines de secondes (ceux
/// de toute la ville, à la racine).
pub const QUAIS_COTE_MAX: f32 = 1500.0;

/// La tuile `cle` reçoit-elle les quais ?
pub fn quais_pour(d: &Donnees, cle: Cle) -> bool {
    let max = std::env::var("FLOW_QUAIS_COTE").ok().and_then(|s| s.parse::<f32>().ok()).unwrap_or(QUAIS_COTE_MAX);
    let b = boite_tuile(d, cle);
    (b[2] - b[0]).max(b[3] - b[1]) <= max
}

/// `f`, puis les `etapes` à la suite (`booleens::enchainer`, sans repasser par les
/// formes) ; tel quel si rien ne marche. Avec FLOW_TRACE_DEFONCE, le volume avant et
/// après, et les étapes sautées.
pub fn composer(f: Forme, etapes: &[(crate::booleens::Op, &Forme)]) -> Forme {
    let trace = std::env::var_os("FLOW_TRACE_DEFONCE").is_some();
    match crate::booleens::enchainer(&f, etapes) {
        Ok((c, sautees)) => {
            if trace {
                eprintln!("composer : volume {:.0} -> {:.0} m3, {} étapes, {sautees} sautées", f.volume(), c.volume(), etapes.len());
            }
            c
        }
        Err(e) => {
            if trace {
                eprintln!("composer : échec : {e}");
            }
            f
        }
    }
}

/// Le solide de la tuile uni au remblai de ses quais.
pub fn remblayer(d: &Donnees, cle: Cle, f: Forme) -> Forme {
    let rem = remblai_morceaux(d, cle);
    let etapes: Vec<(crate::booleens::Op, &Forme)> = rem.iter().map(|m| (crate::booleens::Op::Union, m)).collect();
    composer(f, &etapes)
}

/// Les remblais des quais qui touchent la tuile `cle` (`remblais` : un volume par
/// contour, extrudé de sous le lit à la hauteur du quai), chacun coupé à la tuile (un
/// millimètre en deçà de son bord : pas de face confondue avec ses murs).
pub fn remblai_morceaux(d: &Donnees, cle: Cle) -> Vec<Forme> {
    use crate::booleens::{operation, Op};
    let (xs, zs) = grille(d, cle);
    if xs.len() < 2 || zs.len() < 2 {
        return Vec::new();
    }
    let bt = [xs[0], zs[0], xs[xs.len() - 1], zs[zs.len() - 1]];
    let bas = d.lieu.niveau - PROFONDEUR - 1.0;
    let rem = d.remblais();
    let mut v = Vec::new();
    for c in &rem.contours {
        if crate::booleens::arrete() {
            break;
        }
        let b = c.boite;
        if b[2] < bt[0] || b[0] > bt[2] || b[3] < bt[1] || b[1] > bt[3] {
            continue;
        }
        let vol = rem.volume(c, bas);
        if vol.vide() {
            continue;
        }
        let haut = vol.sommets.iter().map(|p| p[1]).fold(bas, f32::max) + 10.0;
        let e = 0.001;
        let boite = pave([bt[0] + e, bas - 2.0, bt[1] + e], [bt[2] - e, haut, bt[3] - e]).en(matiere::REMBLAI);
        if let Ok(m) = operation(&vol, &boite, Op::Intersection) {
            if !m.vide() {
                v.push(m);
            }
        }
    }
    v
}

/// Les épaulements des quais qui touchent la tuile `cle` (`remblais` : un volume par
/// contour, du quai jusqu'au-dessus du relief), chacun coupé à la tuile élargie d'un
/// mètre (ôtés du solide, ils peuvent déborder).
pub fn epaulement_morceaux(d: &Donnees, cle: Cle) -> Vec<Forme> {
    use crate::booleens::{operation, Op};
    // Pour les essais : FLOW_SANS_EPAULEMENT=1 le retire
    if std::env::var("FLOW_SANS_EPAULEMENT").is_ok_and(|v| v == "1") {
        return Vec::new();
    }
    let b = boite_tuile(d, cle);
    let rem = d.remblais();
    let mut v = Vec::new();
    for c in &rem.epaulements {
        if crate::booleens::arrete() {
            break;
        }
        let bc = c.boite;
        if bc[2] < b[0] || bc[0] > b[2] || bc[3] < b[1] || bc[1] > b[3] {
            continue;
        }
        // Le dessus : au-dessus de tout le relief de la tuile qu'il couvre
        let mut haut = d.lieu.niveau;
        for a in &c.anneaux {
            for p in a {
                haut = haut.max(relief_sec(d, p[0], p[1]));
            }
        }
        let haut = haut + 20.0;
        let vol = rem.volume_epaulement(c, haut);
        if vol.vide() {
            continue;
        }
        let boite = pave([b[0] - 1.0, d.lieu.niveau - PROFONDEUR - 10.0, b[1] - 1.0], [b[2] + 1.0, haut + 1.0, b[3] + 1.0]).en(matiere::QUAI_MUR);
        if let Ok(m) = operation(&vol, &boite, Op::Intersection) {
            if !m.vide() {
                v.push(m);
            }
        }
    }
    v
}

/// Le remblai de la tuile `cle` en une forme (ses morceaux unis) et le nombre d'unions
/// sautées (pour les essais).
pub fn remblai(d: &Donnees, cle: Cle) -> (Forme, usize) {
    let mut m = remblai_morceaux(d, cle);
    if m.is_empty() {
        return (Forme::default(), 0);
    }
    let premier = m.remove(0);
    let etapes: Vec<(crate::booleens::Op, &Forme)> = m.iter().map(|x| (crate::booleens::Op::Union, x)).collect();
    crate::booleens::enchainer(&premier, &etapes).unwrap_or((premier, m.len()))
}

/// Un pavé fermé, faces dehors.
fn pave(a: [f32; 3], b: [f32; 3]) -> Forme {
    let p = |i: usize| [if i & 1 == 0 { a[0] } else { b[0] }, if i & 2 == 0 { a[1] } else { b[1] }, if i & 4 == 0 { a[2] } else { b[2] }];
    let mut f = Forme { sommets: (0..8).map(p).collect(), faces: vec![vec![0, 4, 6, 2], vec![1, 3, 7, 5], vec![0, 1, 5, 4], vec![2, 6, 7, 3], vec![0, 2, 3, 1], vec![4, 5, 7, 6]], matieres: Vec::new() };
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
    /// Celles en cours, une par fil occupé.
    en_cours: Vec<Cle>,
    faites: Vec<Tuile>,
}

/// Le nombre de fils de la fabrique (`FLOW_FILS`, 3 par défaut, de 1 à 6).
pub fn fils() -> usize {
    std::env::var("FLOW_FILS").ok().and_then(|v| v.parse().ok()).unwrap_or(3).clamp(1, 6)
}

/// Les fils qui fabriquent les tuiles de la maquette : chacun prend la première tuile
/// voulue qui n'est pas déjà en cours.
pub struct Fabrique {
    echange: Arc<(Mutex<Echange>, Condvar)>,
    arret: Arc<AtomicBool>,
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
        Fabrique::pour(donnees, false)
    }

    /// Une fabrique de tuiles pour la Maquette, ou pour la vue Matérialisation.
    pub fn pour(donnees: Arc<RwLock<Donnees>>, materialiser: bool) -> Fabrique {
        let echange: Arc<(Mutex<Echange>, Condvar)> = Default::default();
        let arret = Arc::new(AtomicBool::new(false));
        for k in 0..fils() {
            let (e, a, donnees) = (echange.clone(), arret.clone(), donnees.clone());
            let _ = std::thread::Builder::new().name(format!("maquette {}", k + 1)).spawn(move || {
                baisser_priorite();
                // Abandonnée (`Drop`), la tuile en cours s'arrête à la prochaine étape
                crate::booleens::arreter_avec(Some(a.clone()));
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
                        g.en_cours.push(t.0);
                        t
                    };
                    let attente = crate::suivi::tache_min("attente des données", 5.0);
                    let lu = donnees.read();
                    drop(attente);
                    let fait = lu.ok().map(|d| tuile_pour(&d, cle, faite, materialiser));
                    if a.load(Ordering::Relaxed) {
                        crate::suivi::noter(format!("fabrique arrêtée : tuile {}/{}/{} abandonnée", cle.0, cle.1, cle.2));
                        return;
                    }
                    let (m, _) = &*e;
                    let Ok(mut g) = m.lock() else { return };
                    g.en_cours.retain(|&c| c != cle);
                    if let Some(t) = fait {
                        g.faites.push(t);
                    }
                }
            });
        }
        Fabrique { echange, arret }
    }

    /// Les tuiles voulues, dans l'ordre (remplace la demande d'avant).
    pub fn demander(&self, voulues: Vec<(Cle, u64)>) {
        let (m, c) = &*self.echange;
        if let Ok(mut g) = m.lock() {
            // Ni celles en cours, ni celles finies pas encore prises (sinon refaites)
            let deja: HashSet<Cle> = g.en_cours.iter().copied().chain(g.faites.iter().map(|t| t.cle)).collect();
            g.voulues = voulues.into_iter().filter(|v| !deja.contains(&v.0)).collect();
            c.notify_all();
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
        m.lock().is_ok_and(|g| !g.voulues.is_empty() || !g.en_cours.is_empty() || !g.faites.is_empty())
    }

    /// Les clés en cours ou demandées (pour ne pas les redemander).
    pub fn attendues(&self) -> HashSet<Cle> {
        let (m, _) = &*self.echange;
        m.lock().map(|g| g.voulues.iter().map(|v| v.0).chain(g.en_cours.iter().copied()).collect()).unwrap_or_default()
    }
}

impl Drop for Fabrique {
    fn drop(&mut self) {
        self.arret.store(true, Ordering::Relaxed);
        // Sans attendre : les tuiles en cours finissent seules
        self.echange.1.notify_all();
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
        Forme { sommets: (0..8).map(s).collect(), faces: vec![vec![2, 3, 1, 0], vec![5, 7, 6, 4], vec![1, 5, 4, 0], vec![6, 7, 3, 2], vec![4, 6, 2, 0], vec![3, 7, 5, 1]], matieres: Vec::new() }
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
        // Le sol de la tuile, et ce que la défonce laisse : le lit et la berge, de leurs
        // matières (rien du dessus ôté)
        let mats: Vec<f32> = v.chunks(SOMMET).map(|s| s[8]).collect();
        assert!(mats.iter().all(|&m| m == MAQUETTE || m == matiere::sommet(matiere::LIT) || m == matiere::sommet(matiere::BERGE)), "{:?}", mats.iter().filter(|&&m| m != MAQUETTE).take(5).collect::<Vec<_>>());
        assert!(mats.contains(&matiere::sommet(matiere::LIT)));
        assert!(!mats.contains(&matiere::sommet(matiere::HAUT)));
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
        let f = creuser(solide(&d, cle), &defonce(&d, cle));
        let berge = f.sommets.iter().filter(|p| (p[0] - 100.0).abs() < 1e-3 && p[1] < d.lieu.niveau).count();
        assert!(berge >= 2, "{berge}");
        assert!(crate::booleens::operation(&f, &boite([-1e4, -1e4, -1e4], [1e4, 1e4, 1e4]), crate::booleens::Op::Intersection).is_ok());
    }

    #[test]
    fn la_defonce_remplit_le_lit_et_est_fermee() {
        let mut d = donnees();
        let carre = vec![[-100.0, -100.0], [100.0, -100.0], [100.0, 100.0], [-100.0, 100.0]];
        d.lieu.surfaces_eau = vec![sage_reel::cours_eau::SurfaceEau { nature: 0, anneaux: vec![carre] }];
        let (x0, z0, c) = d.monde();
        let n = 7u8;
        let t = c / (1u32 << n) as f32;
        let cle = (n, ((100.0 - x0) / t).floor() as i32, ((0.0 - z0) / t).floor() as i32);
        let def = defonce(&d, cle);
        let niveau = d.lieu.niveau;
        // Seulement dans l'eau ; le dessus au-dessus de l'eau, le fond en dessous
        assert!(!def.vide());
        assert!(def.sommets.iter().all(|p| p[0] <= 100.0 + 1e-3));
        assert!(def.sommets.iter().any(|p| p[1] >= niveau + DEPASSE - 1e-4) && def.sommets.iter().any(|p| p[1] < niveau - 1.0));
        // Fermée : chaque côté de face une fois dans chaque sens
        let mut cotes = std::collections::HashMap::new();
        for p in &def.faces {
            for k in 0..p.len() {
                *cotes.entry((p[k], p[(k + 1) % p.len()])).or_insert(0) += 1;
            }
        }
        assert!(cotes.iter().all(|(&(a, b), &n)| n == 1 && cotes.get(&(b, a)) == Some(&1)));
        // Un volume positif (faces dehors), que les booléens prennent
        assert!(def.volume() > 0.0);
        assert!(crate::booleens::operation(&def, &boite([-1e4, -1e4, -1e4], [1e4, 1e4, 1e4]), crate::booleens::Op::Intersection).is_ok());
        let (v, ind, aretes) = maillage_defonce(&def);
        assert!(!ind.is_empty() && !aretes.is_empty() && aretes.iter().chain(&ind).all(|&i| (i as usize) < v.len() / SOMMET));
        // Loin de l'eau : rien
        assert!(defonce(&d, (n, ((1000.0 - x0) / t).floor() as i32, ((1000.0 - z0) / t).floor() as i32)).vide());
        // Le solide creusé : moins de volume, le lit sous l'eau, toujours fermé
        let f = solide(&d, cle);
        let c = creuser(f.clone(), &def);
        assert!(c.volume() < f.volume() - 1.0, "{} {}", c.volume(), f.volume());
        assert!(c.sommets.iter().any(|p| p[1] < niveau - 1.0));
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
        let dedans = |p: &[f32; 3]| p[0].abs() < 100.0 - crate::remblais::REMBLAI_EAU - 1.0 && p[2].abs() < 100.0 - crate::remblais::REMBLAI_EAU - 1.0;
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