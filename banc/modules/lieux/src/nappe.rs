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
use std::sync::Mutex;
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
    segments_si(r, b, marge, |a| a.passage == Passage::Sol)
}

/// Comme `segments`, pour les seules voies que `garder` retient (ponts et tunnels compris).
fn segments_si(r: &Reseau, b: [f32; 4], marge: f32, garder: impl Fn(&sage_reel::reseau::Arete) -> bool) -> Vec<([f32; 3], [f32; 3], f32)> {
    let mut v = Vec::new();
    for a in r.aretes.iter().filter(|a| garder(a)) {
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

    /// La hauteur fondue : la moyenne des segments à moins de `FONDU` mètres de plus que
    /// le plus proche, pondérée par cet écart. Continue là où deux voies voisines (deux
    /// chaussées d'un boulevard) ont des hauteurs différentes.
    fn hauteur_douce(&self, x: [f32; 2]) -> f32 {
        const FONDU: f32 = 4.0;
        let (ci, cj) = ((x[0] / CASE).floor() as i32, (x[1] / CASE).floor() as i32);
        let mut vus: Vec<(f32, f32)> = Vec::new();
        let mut trouve = None;
        for r in 0..64 {
            if trouve.is_some_and(|t| r > t + 1) {
                break;
            }
            for i in ci - r..=ci + r {
                for j in cj - r..=cj + r {
                    if (i - ci).abs() != r && (j - cj).abs() != r {
                        continue;
                    }
                    for &k in self.cases.get(&(i, j)).map(|v| v.as_slice()).unwrap_or(&[]) {
                        vus.push(self.proche(k, x));
                    }
                }
            }
            if trouve.is_none() && !vus.is_empty() {
                trouve = Some(r);
            }
        }
        let dmin = vus.iter().map(|v| v.0).fold(f32::MAX, f32::min);
        let (mut somme, mut poids) = (0.0, 0.0);
        for (d, h) in vus {
            let w = (1.0 - (d - dmin) / FONDU).max(0.0).powi(2);
            somme += w * h;
            poids += w;
        }
        if poids > 0.0 { somme / poids } else { self.hauteur(x) }
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
    top("habillage, texture", &mut t);
    let anneaux: usize = contours.iter().map(|c| c.anneaux.len()).sum();
    let total: f32 = etapes_t.iter().map(|e| e.1).sum();
    let resume = format!(
        "tuile {}/{}/{} : {} segments → {} contours ({} anneaux), dégagement {} faces, chaussée {} faces ; {} étapes, {} sautées ; {:.2} s",
        cle.0, cle.1, cle.2, segs.len(), contours.len(), anneaux, deg.faces.len(), ch.faces.len(), etapes.len(), sautees, total
    );
    *DERNIERE.lock().unwrap() = (etapes_t, resume);
    (v, ind, px)
}

/// Ce que le remblai descend sous le terrain (m), comme les remblais en surimpression.
const SOUS: f32 = 0.3;
/// La défonce des voies par le dessus : de la voie (un peu dessous, pour ne pas
/// laisser de face confondue avec le dessus du remblai) jusqu'à `DEFONCE_HAUT` au-dessus.
const DEFONCE_HAUT: f32 = 3.2;
const DEFONCE_SOUS: f32 = 0.05;
/// Le remblai d'un pont (bretelle en l'air comprise) ou d'un tunnel : une dalle de
/// `OUVRAGE` mètres sous la chaussée, comme en surimpression.
const OUVRAGE: f32 = 1.0;

/// Les volumes refusés par les booléens à la matérialisation (les plus récents), et le
/// nombre de tuiles faites.
pub static ECHECS: Mutex<(Vec<String>, usize)> = Mutex::new((Vec::new(), 0));

/// Vrai si les booléens acceptent `f` (sa conversion, contre un petit cube loin).
fn accepte(f: &Forme) -> bool {
    let c = pave([1.0e5, 0.0, 1.0e5], [1.0e5 + 1.0, 1.0, 1.0e5 + 1.0]);
    flow::booleens::operation(&c, f, Op::Union).is_ok()
}

/// La matière d'un volume de défonce des voies : son dessous (tourné vers le bas, il
/// devient le fond, la chaussée) en bitume ; ses côtés et son dessus en béton clair.
fn chaussee(f: Forme) -> Forme {
    let mut f = f.en(mq::matiere::BETON);
    for k in 0..f.faces.len() {
        let p = &f.faces[k];
        let mut ny = 0.0f32;
        let mut l2 = [0f32; 3];
        for i in 0..p.len() {
            let (a, b) = (f.sommets[p[i] as usize], f.sommets[p[(i + 1) % p.len()] as usize]);
            l2[0] += (a[1] - b[1]) * (a[2] + b[2]);
            ny += (a[2] - b[2]) * (a[0] + b[0]);
            l2[2] += (a[0] - b[0]) * (a[1] + b[1]);
        }
        let l = (l2[0] * l2[0] + ny * ny + l2[2] * l2[2]).sqrt();
        if l > 1e-9 && ny / l < -0.6 {
            f.matieres[k] = mq::matiere::CHAUSSEE;
        }
    }
    f
}

/// L'écart de hauteur (m) au-delà duquel deux voies qui se chevauchent ne vont pas
/// dans le même plan (une trémie le long d'un boulevard).
const ECART_NIVEAUX: f32 = 0.5;

/// Les segments des voies que `garder` retient, en groupes : d'abord toutes les voies
/// fusionnables ensemble, puis, chacune à part, celles dont l'emprise chevauche celle
/// d'une autre voie à plus de `ECART_NIVEAUX` de hauteur (leurs plans fusionnés
/// devraient avoir une marche ; à part, chacune reste plate en travers).
fn groupes(r: &Reseau, b: [f32; 4], garder: impl Fn(&sage_reel::reseau::Arete) -> bool) -> Vec<Vec<([f32; 3], [f32; 3], f32)>> {
    let marge = 10.0;
    let mut v: Vec<(usize, ([f32; 3], [f32; 3], f32))> = Vec::new();
    for (k, a) in r.aretes.iter().enumerate().filter(|(_, a)| garder(a)) {
        let w = (a.largeur * 0.5).max(1.0);
        for s in a.points.windows(2) {
            let (p, q) = (s[0], s[1]);
            if p[0].max(q[0]) >= b[0] - marge - w && p[0].min(q[0]) <= b[2] + marge + w && p[1].max(q[1]) >= b[1] - marge - w && p[1].min(q[1]) <= b[3] + marge + w && (q[0] - p[0]).hypot(q[1] - p[1]) > 0.05 {
                v.push((k, (p, q, w)));
            }
        }
    }
    if v.is_empty() {
        return Vec::new();
    }
    // Les cases des segments, pour ne comparer que les voisins
    let mut cases: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
    for (n, &(_, (p, q, w))) in v.iter().enumerate() {
        for i in ((p[0].min(q[0]) - w) / CASE).floor() as i32..=((p[0].max(q[0]) + w) / CASE).floor() as i32 {
            for j in ((p[1].min(q[1]) - w) / CASE).floor() as i32..=((p[1].max(q[1]) + w) / CASE).floor() as i32 {
                cases.entry((i, j)).or_default().push(n);
            }
        }
    }
    // Le point de s le plus proche de x : sa distance et sa hauteur
    let proche = |s: ([f32; 3], [f32; 3], f32), x: [f32; 2]| {
        let (p, q, _) = s;
        let (dx, dz) = (q[0] - p[0], q[1] - p[1]);
        let t = (((x[0] - p[0]) * dx + (x[1] - p[1]) * dz) / (dx * dx + dz * dz).max(1e-9)).clamp(0.0, 1.0);
        ((x[0] - p[0] - t * dx).hypot(x[1] - p[1] - t * dz), p[2] + t * (q[2] - p[2]))
    };
    let mut a_part = std::collections::HashSet::new();
    for liste in cases.values() {
        for (x, &m) in liste.iter().enumerate() {
            for &n in &liste[x + 1..] {
                let ((ka, sa), (kb, sb)) = (v[m], v[n]);
                if ka == kb || (a_part.contains(&ka) && a_part.contains(&kb)) {
                    continue;
                }
                // Quelques points de chaque segment contre l'autre
                let mut conflit = false;
                for (s, o) in [(sa, sb), (sb, sa)] {
                    for t in [0.0f32, 0.25, 0.5, 0.75, 1.0] {
                        let x = [s.0[0] + t * (s.1[0] - s.0[0]), s.0[1] + t * (s.1[1] - s.0[1])];
                        let h = s.0[2] + t * (s.1[2] - s.0[2]);
                        let (d, ho) = proche(o, x);
                        if d < s.2 + o.2 && (h - ho).abs() > ECART_NIVEAUX {
                            conflit = true;
                        }
                    }
                }
                if conflit {
                    a_part.insert(ka);
                    a_part.insert(kb);
                }
            }
        }
    }
    let mut g: Vec<Vec<([f32; 3], [f32; 3], f32)>> = vec![Vec::new()];
    let mut seules: HashMap<usize, usize> = HashMap::new();
    for &(k, s) in &v {
        if a_part.contains(&k) {
            let n = g.len();
            let i = *seules.entry(k).or_insert(n);
            if i == n {
                g.push(Vec::new());
            }
            g[i].push(s);
        } else {
            g[0].push(s);
        }
    }
    g.retain(|x| !x.is_empty());
    g
}

/// L'espacement des points ajoutés le long du tracé dans la triangulation du plan (m).
const PAS_TRACE: f32 = 3.0;

/// Les points du tracé (bouts des segments et tous les `PAS_TRACE` mètres) qui sont
/// dans le contour `c` : ajoutés à la triangulation, ils empêchent un triangle de
/// relier deux bords de voie pris à des hauteurs différentes — la chaussée reste
/// horizontale en travers.
fn points_trace(c: &Contour, segs: &[([f32; 3], [f32; 3], f32)]) -> Vec<[f64; 2]> {
    let dedans = |x: f32, z: f32| {
        if x < c.boite[0] || x > c.boite[2] || z < c.boite[1] || z > c.boite[3] {
            return false;
        }
        let mut n = false;
        for a in &c.anneaux {
            for i in 0..a.len() {
                let (p, q) = (a[i], a[(i + 1) % a.len()]);
                if (p[1] > z) != (q[1] > z) && x < p[0] + (z - p[1]) / (q[1] - p[1]) * (q[0] - p[0]) {
                    n = !n;
                }
            }
        }
        n
    };
    let mut vus = std::collections::HashSet::new();
    let mut v = Vec::new();
    for &(p, q, _) in segs {
        let l = (q[0] - p[0]).hypot(q[1] - p[1]);
        let n = (l / PAS_TRACE).ceil().max(1.0) as usize;
        for k in 0..=n {
            let t = k as f32 / n as f32;
            let (x, z) = (p[0] + t * (q[0] - p[0]), p[1] + t * (q[1] - p[1]));
            if dedans(x, z) && vus.insert(((x * 10.0).round() as i64, (z * 10.0).round() as i64)) {
                v.push([x as f64, z as f64]);
            }
        }
    }
    v
}

/// Comme `flow::remblais::extruder`, la triangulation du plan passant aussi par les
/// points du tracé (`points_trace`).
fn extruder_plat(c: &Contour, segs: &[([f32; 3], [f32; 3], f32)], dessus: impl Fn([f32; 2]) -> f32, dessous: impl Fn([f32; 2]) -> f32) -> Forme {
    use i_triangle::float::triangulatable::Triangulatable;
    // Les bords aussi, redécoupés tous les `PAS_TRACE` mètres
    let forme: Vec<Vec<[f64; 2]>> = c
        .anneaux
        .iter()
        .map(|a| {
            let mut v = Vec::with_capacity(a.len() * 2);
            for i in 0..a.len() {
                let (p, q) = (a[i], a[(i + 1) % a.len()]);
                let n = ((q[0] - p[0]).hypot(q[1] - p[1]) / PAS_TRACE).ceil().max(1.0) as usize;
                for k in 0..n {
                    let t = k as f32 / n as f32;
                    v.push([(p[0] + t * (q[0] - p[0])) as f64, (p[1] + t * (q[1] - p[1])) as f64]);
                }
            }
            v
        })
        .collect();
    let pts = points_trace(c, segs);
    let t = forme.triangulate_with_steiner_points(&pts).to_triangulation::<u32>();
    let n = t.points.len() as u32;
    if n < 3 || t.indices.len() < 3 {
        return extruder(c, dessus, dessous);
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
    let mut aretes: HashMap<(u32, u32), u32> = HashMap::new();
    for tri in t.indices.chunks(3) {
        let (a, b, c) = (tri[0], tri[1], tri[2]);
        f.faces.push(vec![a, b, c]);
        f.faces.push(vec![c + n, b + n, a + n]);
        for (u, v) in [(a, b), (b, c), (c, a)] {
            *aretes.entry((u, v)).or_insert(0) += 1;
        }
    }
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

/// La Matérialisation (et la variante `remblai`) : 1. la forme brute de Flow (solide,
/// remblai et épaulement des quais, défonce des lits) ; 2. l'union des remblais : le
/// plan des voies au sol (comme la nappe) extrudé du dessus de la chaussée nivelée
/// jusqu'à `SOUS` sous le terrain (ou sous la chaussée) ; 3. la défonce des voies par le
/// dessus, sauf les piétonnes : de la voie jusqu'à `DEFONCE_HAUT` au-dessus. Le remblai
/// comble où la voie passe au-dessus du terrain, la défonce creuse où elle passe
/// dessous : les voies sont continues. Ponts, bretelles et tunnels compris : leur
/// remblai est une dalle de `OUVRAGE` sous la chaussée, chaque passage nivelé à part.
pub fn tuile_remblai(d: &Donnees, cle: (u8, i32, i32)) -> (Vec<f32>, Vec<u32>, Vec<u8>) {
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
    top("défonce, remblai des quais, épaulement", &mut t);
    // Par passage (au sol, ponts, tunnels), chacun son plan et ses hauteurs : un pont
    // qui croise une voie au sol ne mêle pas leurs niveaux
    let terre = |p: [f32; 2]| d.lieu.relief(p[0] as f64, p[1] as f64) as f32;
    let (mut remblais, mut defonces) = (Vec::new(), Vec::new());
    let (mut passages, mut defonce_par): (Vec<usize>, Vec<Option<Forme>>) = (Vec::new(), Vec::new());
    let (mut n_segs, mut n_segs_d, mut contours) = (0, 0, Vec::new());
    for (np, passage) in [Passage::Sol, Passage::Pont, Passage::Tunnel].into_iter().enumerate() {
        let segs = segments_si(&r, b, 10.0, |a| a.passage == passage);
        if segs.is_empty() {
            continue;
        }
        n_segs += segs.len();
        // Le remblai : au sol jusque sous le terrain ; un ouvrage, en dalle. Un volume
        // par groupe (les voies qui ne se chevauchent pas à des hauteurs différentes
        // ensemble ; les autres chacune à part, plates en travers)
        // Les routes (comme leur défonce : mêmes groupes, mêmes hauteurs, le fond de la
        // défonce tombe toujours dans le remblai) ; les piétonnes à part, sauf au sol où
        // elles ne sont pas remblayées (leurs hauteurs, bruitées, tordaient le remblai)
        let routes = groupes(&r, b, |a| a.passage == passage && a.genre != sage_reel::PIETONNE);
        let pietonnes = if passage == Passage::Sol { Vec::new() } else { groupes(&r, b, |a| a.passage == passage && a.genre == sage_reel::PIETONNE) };
        for g in routes.into_iter().chain(pietonnes) {
            let h = Hauteurs::new(&g);
            let cs = plan(&g);
            let mut rp = Vec::new();
            for c in &cs {
                let f = if passage == Passage::Sol {
                    extruder_plat(c, &g, |p| h.hauteur_douce(p), |p| terre(p).min(h.hauteur_douce(p)) - SOUS)
                } else {
                    extruder_plat(c, &g, |p| h.hauteur_douce(p), |p| h.hauteur_douce(p) - OUVRAGE)
                };
                rp.push(f.en(mq::matiere::REMBLAI));
            }
            remblais.push(reunir_formes(rp));
            passages.push(np);
            contours.extend(cs);
            defonce_par.push(None);
        }
        // La défonce : sauf les piétonnes, par groupe de même
        for g in groupes(&r, b, |a| a.passage == passage && a.genre != sage_reel::PIETONNE) {
            n_segs_d += g.len();
            let hd = Hauteurs::new(&g);
            let dp = plan(&g).iter().map(|c| extruder_plat(c, &g, |p| hd.hauteur_douce(p) + DEFONCE_HAUT, |p| hd.hauteur_douce(p) - DEFONCE_SOUS)).collect();
            let f = chaussee(reunir_formes(dp));
            remblais.push(Forme::default());
            passages.push(np);
            defonce_par.push(Some(f.clone()));
            defonces.push(f);
        }
    }
    top("plan des voies", &mut t);
    let voies = remblais;
    let defonce = defonces;
    top("volumes des voies", &mut t);
    let socle = mq::socle(d);
    let boite = pave([b[0], socle - 50.0, b[1]], [b[2], 2000.0, b[3]]);
    // Comme Flow (remblai des quais, épaulement, défonce), puis les voies unies
    {
        let mut refus = Vec::new();
        for (k, (r, d)) in voies.iter().zip(&defonce_par).enumerate() {
            let nom = ["sol", "pont", "tunnel"][passages[k]];
            if !r.vide() && !accepte(r) {
                refus.push(format!("remblai {nom}"));
            }
            if let Some(d) = d {
                if !d.vide() && !accepte(d) {
                    refus.push(format!("défonce {nom}"));
                }
            }
        }
        let mut e = ECHECS.lock().unwrap();
        e.1 += 1;
        if !refus.is_empty() {
            e.0.push(format!("{}/{}/{} : {}", cle.0, cle.1, cle.2, refus.join(", ")));
            if e.0.len() > 40 {
                e.0.remove(0);
            }
        }
    }
    let mut etapes: Vec<(Op, &Forme)> = rem.iter().map(|m| (Op::Union, m)).collect();
    etapes.extend(epa.iter().map(|m| (Op::Difference, m)));
    etapes.push((Op::Difference, &def));
    etapes.extend(voies.iter().filter(|f| !f.vide()).map(|f| (Op::Union, f)));
    etapes.extend(defonce.iter().filter(|f| !f.vide()).map(|f| (Op::Difference, f)));
    etapes.push((Op::Intersection, &boite));
    let solide = mq::solide(d, cle);
    top("solide", &mut t);
    let (f, sautees) = match flow::booleens::enchainer(&solide, &etapes) {
        Ok(x) => x,
        Err(_) => (solide, etapes.len()),
    };
    top("booléens", &mut t);
    // Sans dissolution : elle réunit de proche en proche les triangles presque
    // coplanaires d'une longue chaussée en pente, et le grand polygone, retriangulé en
    // éventail, n'est plus plan (dents de scie)
    let (mut v, mut ind) = mq::habiller(d, &f, b);
    // Les volumes de défonce des voies, coupés à la tuile, joints en matière
    // MAQUETTE_DEFONCE : Flow les montre avec la défonce (pilule Défonce)
    for g in &defonce {
        let Ok(g) = flow::booleens::operation(g, &boite, Op::Intersection) else { continue };
        let (dv, di, _) = mq::maillage_defonce(&g);
        let base = (v.len() / flow::relief3d::SOMMET) as u32;
        v.extend_from_slice(&dv);
        ind.extend(di.iter().map(|k| k + base));
    }
    let px = mq::texture(d, b);
    top("dissolution, habillage, texture", &mut t);
    let anneaux: usize = contours.iter().map(|c| c.anneaux.len()).sum();
    let total: f32 = etapes_t.iter().map(|e| e.1).sum();
    let resume = format!(
        "remblai {}/{}/{} : {} segments → {} contours ({} anneaux), remblai {} faces, défonce {} faces ({} segments) ; {} étapes, {} sautées ; {:.2} s",
        cle.0, cle.1, cle.2, n_segs, contours.len(), anneaux, voies.iter().map(|f| f.faces.len()).sum::<usize>(), defonce.iter().map(|f| f.faces.len()).sum::<usize>(), n_segs_d, etapes.len(), sautees, total
    );
    *DERNIERE.lock().unwrap() = (etapes_t, resume);
    (v, ind, px)
}
