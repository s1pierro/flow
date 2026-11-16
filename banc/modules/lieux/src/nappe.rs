//! Variante `nappe` : la chaussée de la tuile en UN volume (piste b).
//!
//! Les voies au sol de la tuile sont réunies à plat (i_overlay : un quadrilatère par
//! segment, un disque à chaque point, pour les joints et les carrefours), puis le
//! contour est triangulé (i_triangle) et extrudé : chaque sommet prend la hauteur du
//! réseau nivelé au point le plus proche. Deux volumes seulement — le dégagement
//! (ôté) et la chaussée (unie) — au lieu de deux par morceau de voie.

use flow::booleens::{Forme, Op};
use flow::carte::Donnees;
use flow::maquette as mq;
use flow::remblais::{extruder, Contour};
use i_overlay::core::fill_rule::FillRule;
use i_overlay::float::simplify::SimplifyShape;
use sage_reel::reseau::{Passage, Reseau};
use std::collections::HashMap;
use std::time::Instant;

/// L'épaisseur de la chaussée, la hauteur du dégagement (m), comme `flow::chaussee`.
const EPAISSEUR: f32 = flow::chaussee::EPAISSEUR;
const DEGAGE: f32 = 40.0;
/// Les côtés des disques posés à chaque point des voies.
const DISQUE: usize = 8;
/// La case de l'index des segments (m).
const CASE: f32 = 16.0;

/// Ce que la dernière tuile a coûté, par étape (nom, secondes) ; et le résumé.
pub static DERNIERE: std::sync::Mutex<(Vec<(&'static str, f32)>, String)> = std::sync::Mutex::new((Vec::new(), String::new()));

/// Les segments (a, b, demi-largeur) des voies au sol qui touchent la boîte `b`
/// élargie de `marge` ; a et b en (x, z, h).
fn segments(r: &Reseau, b: [f32; 4], marge: f32) -> Vec<([f32; 3], [f32; 3], f32)> {
    let mut v = Vec::new();
    for a in r.aretes.iter().filter(|a| a.passage == Passage::Sol) {
        let w = (a.largeur * 0.5).max(1.0);
        for s in a.points.windows(2) {
            let (p, q) = (s[0], s[1]);
            if p[0].max(q[0]) >= b[0] - marge - w && p[0].min(q[0]) <= b[2] + marge + w && p[1].max(q[1]) >= b[1] - marge - w && p[1].min(q[1]) <= b[3] + marge + w && (q[0] - p[0]).hypot(q[1] - p[1]) > 0.05 {
                v.push((p, q, w));
            }
        }
    }
    v
}

/// Un anneau dans le sens direct (aire positive), ou rien s'il est plat.
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

/// Le plan des voies : quadrilatères des segments et disques des points, réunis.
fn plan(segs: &[([f32; 3], [f32; 3], f32)]) -> Vec<Contour> {
    let mut morceaux: Vec<Vec<[f64; 2]>> = Vec::with_capacity(segs.len() * 2);
    let disque = |p: [f32; 3], w: f32| {
        // Le rayon circonscrit : le disque couvre au moins la demi-largeur
        let r = w / (std::f32::consts::PI / DISQUE as f32).cos();
        (0..DISQUE).map(|k| {
            let t = k as f32 * std::f32::consts::TAU / DISQUE as f32;
            [(p[0] + r * t.cos()) as f64, (p[1] + r * t.sin()) as f64]
        }).collect::<Vec<_>>()
    };
    for &(p, q, w) in segs {
        let (dx, dz) = (q[0] - p[0], q[1] - p[1]);
        let l = dx.hypot(dz);
        let (nx, nz) = (-dz / l * w, dx / l * w);
        let quad = vec![[(p[0] + nx) as f64, (p[1] + nz) as f64], [(q[0] + nx) as f64, (q[1] + nz) as f64], [(q[0] - nx) as f64, (q[1] - nz) as f64], [(p[0] - nx) as f64, (p[1] - nz) as f64]];
        morceaux.extend(direct(quad));
        morceaux.extend(direct(disque(p, w)));
        morceaux.extend(direct(disque(q, w)));
    }
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

/// La hauteur du réseau au point le plus proche : les segments indexés par case.
struct Hauteurs<'a> {
    segs: &'a [([f32; 3], [f32; 3], f32)],
    cases: HashMap<(i32, i32), Vec<u32>>,
}

impl<'a> Hauteurs<'a> {
    fn new(segs: &'a [([f32; 3], [f32; 3], f32)]) -> Self {
        let mut cases: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        for (k, &(p, q, w)) in segs.iter().enumerate() {
            let (x0, x1) = (((p[0].min(q[0]) - w) / CASE).floor() as i32, ((p[0].max(q[0]) + w) / CASE).floor() as i32);
            let (z0, z1) = (((p[1].min(q[1]) - w) / CASE).floor() as i32, ((p[1].max(q[1]) + w) / CASE).floor() as i32);
            for i in x0..=x1 {
                for j in z0..=z1 {
                    cases.entry((i, j)).or_default().push(k as u32);
                }
            }
        }
        Hauteurs { segs, cases }
    }

    fn proche(&self, k: u32, x: [f32; 2]) -> (f32, f32) {
        let (p, q, _) = self.segs[k as usize];
        let (dx, dz) = (q[0] - p[0], q[1] - p[1]);
        let t = (((x[0] - p[0]) * dx + (x[1] - p[1]) * dz) / (dx * dx + dz * dz).max(1e-9)).clamp(0.0, 1.0);
        let (cx, cz) = (p[0] + t * dx, p[1] + t * dz);
        ((x[0] - cx).hypot(x[1] - cz), p[2] + t * (q[2] - p[2]))
    }

    fn hauteur(&self, x: [f32; 2]) -> f32 {
        let (ci, cj) = ((x[0] / CASE).floor() as i32, (x[1] / CASE).floor() as i32);
        let mut best = (f32::MAX, 0.0);
        // La case, puis des anneaux de cases jusqu'à trouver un segment
        for r in 0..64 {
            for i in ci - r..=ci + r {
                for j in cj - r..=cj + r {
                    if (i - ci).abs() != r && (j - cj).abs() != r {
                        continue;
                    }
                    for &k in self.cases.get(&(i, j)).map(|v| v.as_slice()).unwrap_or(&[]) {
                        let d = self.proche(k, x);
                        if d.0 < best.0 {
                            best = d;
                        }
                    }
                }
            }
            if best.0 < r as f32 * CASE {
                break;
            }
        }
        best.1
    }
}

/// Des volumes disjoints en une seule forme (plusieurs coques).
fn reunir_formes(v: Vec<Forme>) -> Forme {
    let mut f = Forme::default();
    for g in v {
        let n = f.sommets.len() as u32;
        f.sommets.extend(g.sommets);
        f.faces.extend(g.faces.into_iter().map(|p| p.into_iter().map(|i| i + n).collect()));
        f.matieres.extend(g.matieres);
    }
    f
}

fn pave(a: [f32; 3], b: [f32; 3]) -> Forme {
    let p = |i: usize| [if i & 1 == 0 { a[0] } else { b[0] }, if i & 2 == 0 { a[1] } else { b[1] }, if i & 4 == 0 { a[2] } else { b[2] }];
    let mut f = Forme { sommets: (0..8).map(p).collect(), faces: vec![vec![0, 4, 6, 2], vec![1, 3, 7, 5], vec![0, 1, 5, 4], vec![2, 6, 7, 3], vec![0, 2, 3, 1], vec![4, 5, 7, 6]], matieres: Vec::new() };
    if f.volume() < 0.0 {
        f.faces.iter_mut().for_each(|p| p.reverse());
    }
    f
}

/// La tuile : comme `flow::chaussee::tuile`, mais la chaussée en une nappe.
pub fn tuile(d: &Donnees, cle: (u8, i32, i32)) -> (Vec<f32>, Vec<u32>, Vec<u8>) {
    if !flow::chaussee::pour(d, cle) {
        let (s, i, p, _) = mq::fabriquer_avec_defonce(d, cle);
        return (s, i, p);
    }
    let mut etapes_t: Vec<(&'static str, f32)> = Vec::new();
    let mut t = Instant::now();
    let mut top = |nom: &'static str, t: &mut Instant| {
        etapes_t.push((nom, t.elapsed().as_secs_f32()));
        *t = Instant::now();
    };
    let r = d.reseau();
    top("réseau", &mut t);
    let b = flow::carte::boite_tuile(d, cle);
    let def = mq::defonce(d, cle);
    let rem = mq::remblai_morceaux(d, cle);
    let epa = mq::epaulement_morceaux(d, cle);
    top("défonce, remblai, épaulement", &mut t);
    let segs = segments(&r, b, 10.0);
    let contours = plan(&segs);
    top("plan des voies", &mut t);
    let h = Hauteurs::new(&segs);
    let deg = reunir_formes(contours.iter().map(|c| extruder(c, |p| h.hauteur(p) + DEGAGE, |p| h.hauteur(p) - 0.01).en(mq::matiere::REMBLAI)).collect());
    let ch = reunir_formes(contours.iter().map(|c| extruder(c, |p| h.hauteur(p), |p| h.hauteur(p) - EPAISSEUR).en(mq::matiere::SOL)).collect());
    top("nappes", &mut t);
    let socle = mq::socle(d);
    let boite = pave([b[0], socle - 50.0, b[1]], [b[2], 2000.0, b[3]]);
    let mut etapes: Vec<(Op, &Forme)> = rem.iter().map(|m| (Op::Union, m)).collect();
    etapes.extend(epa.iter().map(|m| (Op::Difference, m)));
    etapes.push((Op::Difference, &def));
    if !deg.faces.is_empty() {
        etapes.push((Op::Difference, &deg));
        etapes.push((Op::Union, &ch));
    }
    etapes.push((Op::Intersection, &boite));
    let solide = mq::solide(d, cle);
    top("solide", &mut t);
    let (f, sautees) = match flow::booleens::enchainer(&solide, &etapes) {
        Ok(x) => x,
        Err(_) => (solide, etapes.len()),
    };
    top("booléens", &mut t);
    let f = flow::dissolution::dissoudre(&f);
    let (v, ind) = mq::habiller(d, &f, b);
    let px = mq::texture(d, b);
    top("dissolution, habillage, texture", &mut t);
    let anneaux: usize = contours.iter().map(|c| c.anneaux.len()).sum();
    let total: f32 = etapes_t.iter().map(|e| e.1).sum();
    let resume = format!(
        "tuile {}/{}/{} : {} segments → {} contours ({} anneaux), dégagement {} faces, chaussée {} faces ; {} étapes, {} sautées ; {:.2} s",
        cle.0, cle.1, cle.2, segs.len(), contours.len(), anneaux, deg.faces.len(), ch.faces.len(), etapes.len(), sautees, total
    );
    *DERNIERE.lock().unwrap() = (etapes_t, resume);
    (v, ind, px)
}
