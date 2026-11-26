//! Les fils de circulation : un réseau concurrent, d'essai, montré seulement en
//! surimpression. Un fil par file (`sage_reel::Files`), sur l'axe de la file.
//!
//! Aux nœuds à 2 et 3 bouts carrossables, chaque file qui arrive est associée aux
//! files qui repartent (de gauche à droite, sans croisement). Au lieu de se rejoindre
//! au milieu du nœud, une des deux files d'une paire garde sa place et l'autre la
//! prolonge : elle part de son bout, dans sa direction, et glisse en douceur vers sa
//! propre place (la partie fondue). Celle qui glisse : celle de la voie qui a le moins
//! de files ; d'une file qui se partage en deux, les deux qui en partent ; de deux qui
//! se rejoignent, les deux qui arrivent. Ainsi, à une fourche, les files des branches
//! continuent celles du tronc, parallèles, au lieu de converger sur l'axe.
//!
//! Une file ne continue que vers les voies à moins de `TOUT_DROIT` de sa direction
//! (fourches, jonctions, passages). Vers les autres, ce sont des virages : une courbe
//! à part, qui ne déplace aucune file, de la file de droite (hors bus) pour tourner à
//! droite, de celle de gauche pour tourner à gauche. Les files d'une voie qui ne fait que
//! tourner (une rue perpendiculaire) s'arrêtent au bord des autres.
//!
//! Le réseau (`sage_reel::reseau::Reseau`) est seulement lu : ni lui, ni ses files, ni
//! la chaussée ne changent. Les axes sont ceux du réseau, sans les déports des fourches.
//! Les points sont ceux du réseau : x, z du jeu, et la hauteur.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use sage_reel::reseau::Reseau;
use std::collections::HashMap;

/// Au plus cette part de la longueur d'une arête reculée ou fondue à chaque bout.
const PART_MAX: f32 = 0.45;
/// Une file continue (fourche, jonction, passage) vers les bouts à moins de cet écart
/// de sa direction (degrés) ; au-delà, jusqu'à `VIRAGE_MAX`, c'est un virage.
pub const TOUT_DROIT: f32 = 40.0;
/// Un tronçon plus court que cela (m) est traversé par la giration de ses voisines.
/// Désactivé (0) : chaque nœud se raccorde pour lui-même — un angle droit par
/// prolongement des bords, un quasi-alignement par arc ou intersection — ce qui tient
/// dans un tronçon de quelques mètres.
const GIR_COURT: f32 = 0.0;
/// La longueur au-delà de laquelle une petite impasse n'est plus une amorce (m).
const AMORCE_MAX: f32 = 12.0;
/// L'écart à l'angle droit (degrés) en deçà duquel deux voies bout à bout se
/// rejoignent en coin des deux côtés, sans arc.
const ANGLE_DROIT: f32 = 25.0;

/// La portée (m) de la direction d'un bout au nœud.
const DIR_PORTEE: f32 = 15.0;
const VIRAGE_MAX: f32 = 160.0;
/// Les files d'un bout qui ne fait que tourner s'arrêtent à la demi-largeur de la plus
/// large des autres voies, plus cette marge (m).
const VIRAGE_MARGE: f32 = 1.0;
/// Un virage rejoint (ou quitte) une voie qui continue à tant de la file en face (m).
const VIRAGE_RAYON: f32 = 4.0;
/// Le rayon des virages en arc, à droite et à gauche (m), et le plus loin de la fin
/// d'une file où un arc peut la toucher.
pub const RAYON_DROITE: f32 = 4.5;
pub const RAYON_GAUCHE: f32 = 4.5;
/// Le lissage des fils (essai) : un graphe adouci tiré du graphe dur, sans le
/// modifier. Les fils qui se prolongent sont mis bout à bout en chaînes ; chaque
/// chaîne devient une courbe qui passe par tous ses nœuds de files (aucun ne bouge) et
/// par les sommets du tracé dur qui s'écartent de plus de la tolérance.
pub const TOLERANCE_DEFAUT: f32 = 0.02;
static LISSE: AtomicBool = AtomicBool::new(true);
static TOLERANCE: AtomicU32 = AtomicU32::new(0x3ca3_d70a); // 0.02

/// Le lissage et sa tolérance (m).
pub fn regler_lissage(lisse: bool, tolerance: f32) {
    LISSE.store(lisse, Ordering::Relaxed);
    TOLERANCE.store(tolerance.to_bits(), Ordering::Relaxed);
}

fn lisse() -> bool {
    LISSE.load(Ordering::Relaxed)
}

fn tolerance() -> f32 {
    f32::from_bits(TOLERANCE.load(Ordering::Relaxed))
}

/// Le pas des points d'une chaîne lissée (m).
const LISSE_PAS: f32 = 1.0;
/// Deux bouts de fils à moins de tant se touchent.
const LISSE_JOINT: f32 = 0.05;

/// Le graphe adouci : les fils (hors virages, continuités et marques) mis bout à bout en
/// chaînes là où exactement deux se touchent, chaque chaîne en une courbe par tous ses
/// nœuds.
fn lisser(v: &mut [Fil]) {
    let tol = tolerance();
    let lissable = |f: &Fil| !f.marque && !f.virage && !f.traverse && f.points.len() >= 2;
    // Les points où un virage ou une continuité s'accroche au milieu d'un fil (l'axe
    // prioritaire d'un carrefour) : ajoutés au fil comme sommets fixes
    let mut epingles: Vec<[f32; 3]> = Vec::new();
    {
        let accroches: Vec<[f32; 3]> = v.iter().filter(|f| (f.virage || f.traverse) && f.points.len() >= 2).flat_map(|f| [f.points[0], f.points[f.points.len() - 1]]).collect();
        for p in accroches {
            'fils: for f in v.iter_mut().filter(|f| lissable(f)) {
                for k in 0..f.points.len() - 1 {
                    let (a, b) = (f.points[k], f.points[k + 1]);
                    if (a[0] - p[0]).hypot(a[1] - p[1]) < LISSE_JOINT || (b[0] - p[0]).hypot(b[1] - p[1]) < LISSE_JOINT {
                        epingles.push(p);
                        break 'fils;
                    }
                    if ecart_corde(a, p, b) < LISSE_JOINT {
                        f.points.insert(k + 1, p);
                        epingles.push(p);
                        break 'fils;
                    }
                }
            }
        }
    }
    let epingle = |q: [f32; 3]| epingles.iter().any(|p| (p[0] - q[0]).hypot(p[1] - q[1]) < LISSE_JOINT);
    // Les bouts, regroupés par proximité (une grille de cases)
    let case = |p: [f32; 3]| ((p[0] / 0.5).floor() as i64, (p[1] / 0.5).floor() as i64);
    let mut grille: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    let mut joints: Vec<[f32; 3]> = Vec::new();
    let mut trouver = |p: [f32; 3], grille: &mut HashMap<(i64, i64), Vec<usize>>, joints: &mut Vec<[f32; 3]>| -> usize {
        let (a, b) = case(p);
        for da in -1..=1 {
            for db in -1..=1 {
                if let Some(l) = grille.get(&(a + da, b + db)) {
                    for &j in l {
                        if (joints[j][0] - p[0]).hypot(joints[j][1] - p[1]) < LISSE_JOINT {
                            return j;
                        }
                    }
                }
            }
        }
        joints.push(p);
        grille.entry((a, b)).or_default().push(joints.len() - 1);
        joints.len() - 1
    };
    // (fil, bout de fin ?) par joint ; et les joints touchés par un virage ou une continuité
    let mut ends: Vec<[usize; 2]> = vec![[usize::MAX; 2]; v.len()];
    let mut par_joint: Vec<Vec<(usize, bool)>> = Vec::new();
    let mut fixe: Vec<bool> = Vec::new();
    for (i, f) in v.iter().enumerate() {
        if f.marque || f.points.len() < 2 {
            continue;
        }
        for (k, p) in [f.points[0], f.points[f.points.len() - 1]].into_iter().enumerate() {
            let j = trouver(p, &mut grille, &mut joints);
            if par_joint.len() < joints.len() {
                par_joint.resize(joints.len(), Vec::new());
                fixe.resize(joints.len(), false);
            }
            if lissable(f) {
                ends[i][k] = j;
                par_joint[j].push((i, k == 1));
            } else {
                fixe[j] = true;
            }
        }
    }
    let traverse = |j: usize| !fixe[j] && par_joint[j].len() == 2 && par_joint[j][0].0 != par_joint[j][1].0;
    let mut vu = vec![false; v.len()];
    let departs: Vec<usize> = (0..v.len()).filter(|&i| lissable(&v[i])).collect();
    // Les chaînes ouvertes d'abord (d'un bout fixe), puis les boucles
    for boucles in [false, true] {
        for &i0 in &departs {
            if vu[i0] {
                continue;
            }
            // Le bout par lequel commencer : un bout qui n'est pas un passage
            let debut_fin = if !traverse(ends[i0][0]) {
                false
            } else if !traverse(ends[i0][1]) {
                true
            } else if boucles {
                false
            } else {
                continue;
            };
            // La chaîne : (fil, à l'envers ?)
            let mut chaine: Vec<(usize, bool)> = Vec::new();
            let (mut i, mut envers) = (i0, debut_fin);
            loop {
                vu[i] = true;
                chaine.push((i, envers));
                let sortie = ends[i][if envers { 0 } else { 1 }];
                if !traverse(sortie) {
                    break;
                }
                let &(n, fin) = par_joint[sortie].iter().find(|x| x.0 != i).unwrap();
                if vu[n] {
                    break;
                }
                // On entre dans `n` par son bout `fin` : à l'envers si c'est sa fin
                i = n;
                envers = fin;
            }
            // Les points bout à bout ; `bornes` : les joints, qui ne bougent pas
            let mut dur: Vec<[f32; 3]> = Vec::new();
            let mut bornes: Vec<usize> = vec![0];
            for &(i, envers) in &chaine {
                let mut p = v[i].points.clone();
                if envers {
                    p.reverse();
                }
                if !dur.is_empty() {
                    p.remove(0);
                }
                dur.extend(p);
                bornes.push(dur.len() - 1);
            }
            if dur.len() < 3 {
                continue;
            }
            // Les sommets gardés : les joints, et entre eux ceux qui s'écartent de plus de
            // la tolérance (Douglas-Peucker) ; les points alignés tombent
            // (jugés chacun avec ses voisins du tracé dur : un critère local, le même
            // quelle que soit l'étendue de la chaîne dans la tuile)
            let mut garde = vec![true; dur.len()];
            for k in 1..dur.len() - 1 {
                garde[k] = ecart_corde(dur[k - 1], dur[k], dur[k + 1]) > tol || epingle(dur[k]);
            }
            for &k in &bornes {
                garde[k] = true;
            }
            let som: Vec<usize> = (0..dur.len()).filter(|&k| garde[k]).collect();
            // Une courbe de Catmull-Rom centripète par tous ces sommets : elle passe par
            // chacun (les nœuds ne bougent pas), sans cassure ; aux bouts de la chaîne, la
            // direction du fil dur
            let pt = |k: usize| dur[som[k]];
            let fantome_debut = {
                let (a, b) = (dur[0], dur[1]);
                let l = (b[0] - a[0]).hypot(b[1] - a[1]).max(1e-4);
                let m = (pt(1)[0] - a[0]).hypot(pt(1)[1] - a[1]);
                [a[0] - (b[0] - a[0]) / l * m, a[1] - (b[1] - a[1]) / l * m, a[2] - (pt(1)[2] - a[2])]
            };
            let fantome_fin = {
                let n = dur.len();
                let (a, b) = (dur[n - 1], dur[n - 2]);
                let l = (b[0] - a[0]).hypot(b[1] - a[1]).max(1e-4);
                let q = pt(som.len() - 2);
                let m = (q[0] - a[0]).hypot(q[1] - a[1]);
                [a[0] - (b[0] - a[0]) / l * m, a[1] - (b[1] - a[1]) / l * m, a[2] - (q[2] - a[2])]
            };
            let mut q: Vec<[f32; 3]> = vec![pt(0)];
            // Où tombe chaque sommet dans `q`
            let mut ou = vec![0usize; dur.len()];
            for k in 0..som.len() - 1 {
                let p0 = if k == 0 { fantome_debut } else { pt(k - 1) };
                let p3 = if k + 2 >= som.len() { fantome_fin } else { pt(k + 2) };
                let (p1, p2) = (pt(k), pt(k + 1));
                let l = (p2[0] - p1[0]).hypot(p2[1] - p1[1]);
                let n = ((l / LISSE_PAS).ceil() as usize).max(1);
                for j in 1..n {
                    q.push(catmull_rom(p0, p1, p2, p3, j as f32 / n as f32));
                }
                q.push(p2);
                ou[som[k + 1]] = q.len() - 1;
            }
            // Rendus à leurs fils
            for (c, &(i, envers)) in chaine.iter().enumerate() {
                let mut p = q[ou[bornes[c]]..=ou[bornes[c + 1]]].to_vec();
                if envers {
                    p.reverse();
                }
                if p.len() >= 2 {
                    v[i].points = p;
                }
            }
        }
    }
}

/// La distance de `m` à la corde de `a` à `b`.
fn ecart_corde(a: [f32; 3], m: [f32; 3], b: [f32; 3]) -> f32 {
    let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
    let l2 = dx * dx + dz * dz;
    let t = if l2 > 1e-9 { (((m[0] - a[0]) * dx + (m[1] - a[1]) * dz) / l2).clamp(0.0, 1.0) } else { 0.0 };
    (m[0] - a[0] - dx * t).hypot(m[1] - a[1] - dz * t)
}

#[allow(dead_code)]
/// Douglas-Peucker : garde entre `a` et `b` les sommets de `p` à plus de `tol` de la
/// corde.
fn simplifier(p: &[[f32; 3]], a: usize, b: usize, tol: f32, garde: &mut [bool]) {
    if b <= a + 1 {
        return;
    }
    let (pa, pb) = (p[a], p[b]);
    let (dx, dz) = (pb[0] - pa[0], pb[1] - pa[1]);
    let l2 = dx * dx + dz * dz;
    let (mut loin, mut k) = (0.0f32, a);
    for i in a + 1..b {
        let t = if l2 > 1e-9 { (((p[i][0] - pa[0]) * dx + (p[i][1] - pa[1]) * dz) / l2).clamp(0.0, 1.0) } else { 0.0 };
        let d = (p[i][0] - pa[0] - dx * t).hypot(p[i][1] - pa[1] - dz * t);
        if d > loin {
            loin = d;
            k = i;
        }
    }
    if loin > tol {
        garde[k] = true;
        simplifier(p, a, k, tol, garde);
        simplifier(p, k, b, tol, garde);
    }
}

/// Le point à `t` (0 à 1) entre `p1` et `p2` d'une courbe de Catmull-Rom centripète.
fn catmull_rom(p0: [f32; 3], p1: [f32; 3], p2: [f32; 3], p3: [f32; 3], t: f32) -> [f32; 3] {
    let d = |a: [f32; 3], b: [f32; 3]| ((a[0] - b[0]).hypot(a[1] - b[1])).sqrt().max(1e-3);
    let t0 = 0.0;
    let t1 = t0 + d(p0, p1);
    let t2 = t1 + d(p1, p2);
    let t3 = t2 + d(p2, p3);
    let u = t1 + (t2 - t1) * t;
    let mix = |a: [f32; 3], b: [f32; 3], ta: f32, tb: f32| {
        let (wa, wb) = ((tb - u) / (tb - ta), (u - ta) / (tb - ta));
        [a[0] * wa + b[0] * wb, a[1] * wa + b[1] * wb, a[2] * wa + b[2] * wb]
    };
    let a1 = mix(p0, p1, t0, t1);
    let a2 = mix(p1, p2, t1, t2);
    let a3 = mix(p2, p3, t2, t3);
    let b1 = mix(a1, a2, t0, t2);
    let b2 = mix(a2, a3, t1, t3);
    mix(b1, b2, t1, t2)
}

/// Les branches de fourches déportées (`champ_voirie`), par arête : les fils partent
/// de l'axe déporté, comme la chaussée et les lignes des files.
static DEPORTS: std::sync::RwLock<Option<Arc<HashMap<usize, Vec<crate::champ_voirie::Deport>>>>> = std::sync::RwLock::new(None);

/// Règle les déports des arêtes (une fois le champ de la voirie fait).
pub fn regler_deports(d: Arc<HashMap<usize, Vec<crate::champ_voirie::Deport>>>) {
    if let Ok(mut g) = DEPORTS.write() {
        *g = Some(d);
    }
}

/// Les déports des branches de fourche, par réseau (sa clé : son adresse).
static DEPORTS_BRANCHES: std::sync::Mutex<Option<(usize, Arc<HashMap<usize, Vec<crate::champ_voirie::Deport>>>)>> = std::sync::Mutex::new(None);

/// Les branches d'une fourche (ou d'une séparation de chaussée, d'une voie qui se
/// greffe) qui tiennent chacune dans le tronc et le couvrent à elles deux : OSM les fait partir du même point,
/// elles se chevauchent alors sur des dizaines de mètres. Chacune est déportée sur ses
/// files du tronc (le centre de leur portion de la largeur), le déport décroissant
/// jusqu'à zéro là où les deux axes, non déportés, sont assez écartés pour que les
/// chaussées soient côte à côte.
pub fn deports_branches(r: &Reseau, cle: usize) -> Arc<HashMap<usize, Vec<crate::champ_voirie::Deport>>> {
    if let Ok(g) = DEPORTS_BRANCHES.lock() {
        if let Some((c, m)) = g.as_ref() {
            if *c == cle {
                return m.clone();
            }
        }
    }
    let mut m: HashMap<usize, Vec<crate::champ_voirie::Deport>> = HashMap::new();
    // Les points d'un bout depuis le nœud
    let depuis = |k: usize, f: bool| -> Vec<[f32; 2]> {
        let q = &r.aretes[k].points;
        let v: Vec<[f32; 2]> = q.iter().map(|p| [p[0], p[1]]).collect();
        if f { v.into_iter().rev().collect() } else { v }
    };
    let corde = |pts: &[[f32; 2]]| -> Option<[f32; 2]> {
        let o = pts[0];
        let mut s_ = pts[1];
        let mut cumul = 0.0f32;
        for i in 1..pts.len() {
            cumul += (pts[i][0] - pts[i - 1][0]).hypot(pts[i][1] - pts[i - 1][1]);
            s_ = pts[i];
            if cumul >= DIR_PORTEE {
                break;
            }
        }
        let l = (s_[0] - o[0]).hypot(s_[1] - o[1]);
        (l > 1e-3).then(|| [(s_[0] - o[0]) / l, (s_[1] - o[1]) / l])
    };
    // La distance d'un point à une polyligne
    let distance = |p: [f32; 2], l: &[[f32; 2]]| -> f32 {
        l.windows(2)
            .map(|w| {
                let (a, b) = (w[0], w[1]);
                let d = [b[0] - a[0], b[1] - a[1]];
                let ll = d[0] * d[0] + d[1] * d[1];
                let t = if ll < 1e-9 { 0.0 } else { (((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / ll).clamp(0.0, 1.0) };
                (p[0] - a[0] - t * d[0]).hypot(p[1] - a[1] - t * d[1])
            })
            .fold(f32::MAX, f32::min)
    };
    let longueur = |l: &[[f32; 2]]| l.windows(2).map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1])).sum::<f32>();
    for n in 0..r.noeuds.len() {
        let mut bouts: Vec<(usize, bool)> = Vec::new();
        for &k in &r.noeuds[n].aretes {
            let a = &r.aretes[k];
            if !sage_reel::carrossable(a.genre) || a.points.len() < 2 || a.files.total == 0 || a.a == a.b {
                continue;
            }
            for f in [false, true] {
                if (if f { a.b } else { a.a }) == n {
                    bouts.push((k, f));
                }
            }
        }
        if bouts.len() != 3 {
            continue;
        }
        let c = [r.noeuds[n].p[0], r.noeuds[n].p[1]];
        for t in 0..3 {
            let (tk, tf) = bouts[t];
            let ta = &r.aretes[tk];
            let (bi, bj) = (bouts[(t + 1) % 3], bouts[(t + 2) % 3]);
            let (fa, fb) = (r.aretes[bi.0].files.total as usize, r.aretes[bj.0].files.total as usize);
            let nt = ta.files.total as usize;
            // Chaque branche tient dans le tronc, et à elles deux elles le couvrent :
            // chacune est posée contre le bord du tronc de son côté, sur ses files (une
            // somme exacte, une file partagée, une voie qui se greffe sur une autre)
            if fa > nt || fb > nt || fa + fb < nt || ta.genre != r.aretes[bi.0].genre || ta.genre != r.aretes[bj.0].genre {
                continue;
            }
            let (pt, pa, pb) = (depuis(tk, tf), depuis(bi.0, bi.1), depuis(bj.0, bj.1));
            let (Some(dt), Some(da), Some(db)) = (corde(&pt), corde(&pa), corde(&pb)) else { continue };
            // Le prolongement du tronc, et les branches à moins de TOUT_DROIT de lui
            let f_ = [-dt[0], -dt[1]];
            let ecart = |d: [f32; 2]| (f_[0] * d[0] + f_[1] * d[1]).clamp(-1.0, 1.0).acos();
            if ecart(da) > TOUT_DROIT.to_radians() || ecart(db) > TOUT_DROIT.to_radians() {
                continue;
            }
            // À gauche du prolongement (+g) : la plus « à gauche » des deux
            let g = [-f_[1], f_[0]];
            let cote = |d: [f32; 2]| f_[0] * d[1] - f_[1] * d[0];
            let ((ga, gpa), (dr, dpa), (kg, kd)) = if cote(da) >= cote(db) { ((bi, pa.clone()), (bj, pb.clone()), (fa, fb)) } else { ((bj, pb.clone()), (bi, pa.clone()), (fb, fa)) };
            // Les centres des files de chaque branche sur le tronc, en travers (vers +g)
            let w = ta.largeur * 0.5;
            let lw = ta.largeur / nt as f32;
            let og = w - kg as f32 * lw / 2.0;
            let od = -(w - kd as f32 * lw / 2.0);
            // Le chevauchement : jusqu'où les axes non déportés sont à moins de la
            // somme des demi-largeurs des chaussées (sur leurs files)
            let besoin = (kg + kd) as f32 * lw / 2.0;
            // Mesuré le long de la plus longue des deux (l'autre peut n'être qu'un bout
            // de quelques mètres avant le nœud suivant)
            let (gpa, dpa) = if longueur(&gpa) >= longueur(&dpa) { (gpa, dpa) } else { (dpa, gpa) };
            let mut l = 0.0f32;
            let mut cumul = 0.0f32;
            for i in 1..gpa.len() {
                cumul += (gpa[i][0] - gpa[i - 1][0]).hypot(gpa[i][1] - gpa[i - 1][1]);
                // Des points tous les mètres sur le segment
                let seg = (gpa[i][0] - gpa[i - 1][0]).hypot(gpa[i][1] - gpa[i - 1][1]);
                let pas = seg.ceil().max(1.0) as usize;
                let mut fini = false;
                for j in 1..=pas {
                    let u = j as f32 / pas as f32;
                    let p = [gpa[i - 1][0] + (gpa[i][0] - gpa[i - 1][0]) * u, gpa[i - 1][1] + (gpa[i][1] - gpa[i - 1][1]) * u];
                    if distance(p, &dpa) >= besoin {
                        l = cumul - seg * (1.0 - u);
                        fini = true;
                        break;
                    }
                }
                if fini {
                    break;
                }
                l = cumul;
            }
            if og.abs() < 0.05 && od.abs() < 0.05 {
                continue;
            }
            // Chaque branche : jusqu'à 90 % de sa propre longueur ; celle qui garde déjà
            // le centre du tronc (une file partagée, 3 + 1 sur 3) n'est pas déportée
            for (b, o) in [(ga, og), (dr, od)] {
                let lb = l.min(0.9 * longueur(&depuis(b.0, b.1)));
                if o.abs() < 0.05 || lb < 3.0 {
                    continue;
                }
                m.entry(b.0).or_default().push(crate::champ_voirie::Deport { aretes: vec![b.0], c, decal: [g[0] * o, g[1] * o], l: lb });
            }
            break;
        }
    }
    let m = Arc::new(m);
    if let Ok(mut g) = DEPORTS_BRANCHES.lock() {
        *g = Some((cle, m.clone()));
    }
    m
}

/// L'axe de l'arête `k`, déporté près d'une fourche.
/// L'axe de l'arête `k` déporté près d'une fourche (`regler_deports`).
fn deporte(r: &Reseau, k: usize) -> Vec<[f32; 3]> {
    let a = &r.aretes[k];
    let ds = DEPORTS.read().ok().and_then(|g| g.as_ref().and_then(|m| m.get(&k).cloned()));
    match ds {
        None => a.points.clone(),
        Some(ds) => a
            .points
            .iter()
            .map(|p| {
                let mut q = *p;
                for dp in &ds {
                    let o = dp.en([p[0], p[1]]);
                    q[0] += o[0];
                    q[1] += o[1];
                }
                q
            })
            .collect(),
    }
}

/// La tolérance (°) d'une section strictement droite aux fourches de sens unique, par
/// défaut : au-delà, un coude est la cassure.
pub const RACCOURCI_TOLERANCE: f32 = 0.5;
static RACC_TOLERANCE: AtomicU32 = AtomicU32::new(0x3f00_0000); // 0.5

/// Règle la tolérance de la section droite des branches de fourche.
pub fn regler_raccourcis(tolerance: f32) {
    RACC_TOLERANCE.store(tolerance.to_bits(), Ordering::Relaxed);
}

/// Le sens d'un bout de voie à sens unique : vrai si la circulation entre au nœud
/// (rien pour une voie à double sens ou sans file).
fn entrant(a: &sage_reel::reseau::Arete, fin: bool) -> Option<bool> {
    let f = a.files;
    if f.total == 0 {
        return None;
    }
    let avant = f.avant > 0 && f.arriere == 0;
    let arriere = f.arriere > 0 && f.avant == 0;
    if !(avant || arriere) {
        return None;
    }
    Some(avant == fin)
}

/// Le raccourci d'une branche de fourche de sens unique : à un nœud où une voie (le
/// tronc) se sépare en voies à sens unique (les branches), chaque branche arrive par une
/// section droite puis une cassure, et un dernier segment qui plonge vers le nœud (OSM
/// réunit les chaussées en un point). Le tronc peut être à sens unique (les branches
/// vont toutes dans le même sens) ou à double sens (une chaussée par sens) : dans chaque
/// sens, ses files doivent être aussi nombreuses que celles des branches qui les
/// continuent. Le dernier segment est sauté : de la cassure, l'axe de la branche va
/// droit à ses files sur le tronc (au nœud, les branches d'un sens prennent les files
/// de ce sens dans l'ordre où elles arrivent en travers). `p` : l'axe de l'arête `k`,
/// modifié au bout `fin`. Rend la demi-largeur de ses files sur le tronc (celle de la
/// branche au nœud) si l'axe est raccourci.
fn raccourcir(r: &Reseau, k: usize, fin: bool, p: &mut Vec<[f32; 3]>, tolerance: f32, arret: usize) -> Option<Raccord> {
    let a = &r.aretes[k];
    let n = if fin { a.b } else { a.a };
    if a.a == a.b || p.len() < 2 {
        return None;
    }
    let Some(e) = entrant(a, fin) else {
        return raccourcir_double(r, k, fin, p, tolerance, arret);
    };
    // Les bouts carrossables au nœud
    let mut tous: Vec<(usize, bool)> = Vec::new();
    for &j in &r.noeuds[n].aretes {
        let b = &r.aretes[j];
        if !sage_reel::carrossable(b.genre) || b.points.len() < 2 || b.files.total == 0 {
            continue;
        }
        for f in [false, true] {
            if (if f { b.b } else { b.a }) == n {
                tous.push((j, f));
            }
        }
    }
    // La direction d'un bout en s'éloignant du nœud : la corde vers le point à
    // DIR_PORTEE le long du tracé (un premier segment court et coudé, le dessin du
    // carrefour, ne la fausse pas)
    let dir = |j: usize, f: bool| -> [f32; 2] {
        let q = &r.aretes[j].points;
        let m = q.len();
        let pt = |i: usize| if f { q[m - 1 - i] } else { q[i] };
        let o = pt(0);
        let mut s_ = pt(1);
        let mut cumul = 0.0f32;
        for i in 1..m {
            cumul += (pt(i)[0] - pt(i - 1)[0]).hypot(pt(i)[1] - pt(i - 1)[1]);
            s_ = pt(i);
            if cumul >= DIR_PORTEE {
                break;
            }
        }
        let l = (s_[0] - o[0]).hypot(s_[1] - o[1]).max(1e-6);
        [(s_[0] - o[0]) / l, (s_[1] - o[1]) / l]
    };
    // Le sens d'une file du tronc au nœud : vrai si elle y entre (rien : partagée)
    let file_entre = |t: &sage_reel::reseau::Arete, tf: bool, i: usize| match crate::files::sens(t, i) {
        crate::files::Sens::Avant => Some(tf),
        crate::files::Sens::Arriere => Some(!tf),
        crate::files::Sens::Partagee => None,
    };
    // Les files d'un bout qui entrent au nœud, et celles qui en sortent
    let files_du_sens = |j: usize, f: bool| -> (Vec<usize>, Vec<usize>) {
        let b = &r.aretes[j];
        let mut ei = Vec::new();
        let mut so = Vec::new();
        for i in 0..b.files.total as usize {
            match file_entre(b, f, i) {
                Some(true) => ei.push(i),
                Some(false) => so.push(i),
                None => {}
            }
        }
        (ei, so)
    };
    // Le tronc parmi des bouts gardés par `garde(tronc, bout)` : le seul bout tel que
    // les autres soient à sens unique, et que dans chaque sens ses files égalent celles
    // des branches qui les continuent. Rend aussi les bouts gardés pour ce tronc.
    let chercher = |garde: &dyn Fn((usize, bool), (usize, bool)) -> bool| -> Option<((usize, bool), Vec<(usize, bool)>)> {
        let mut trouve = None;
        for &(t, tf) in &tous {
            if t == k {
                continue;
            }
            let bouts: Vec<(usize, bool)> = tous.iter().copied().filter(|&x| x == (t, tf) || garde((t, tf), x)).collect();
            if bouts.len() < 3 || !bouts.contains(&(k, fin)) {
                continue;
            }
            let ta = &r.aretes[t];
            let (mut sorties, mut entrees, mut ok) = (0usize, 0usize, true);
            let (mut bus_sorties, mut bus_entrees) = (0usize, 0usize);
            for i in 0..ta.files.total as usize {
                let bus = ta.files.est_bus(i) as usize;
                match file_entre(ta, tf, i) {
                    Some(true) => {
                        entrees += 1;
                        bus_entrees += bus;
                    }
                    Some(false) => {
                        sorties += 1;
                        bus_sorties += bus;
                    }
                    None => ok = false,
                }
            }
            // Les branches qui entrent continuent les files qui sortent, et l'inverse ;
            // à chaque séparation entre deux branches d'un sens, une file du tronc peut
            // être partagée (elle mène à l'une ou à l'autre)
            // Une branche à double sens (une seule) apporte à chaque sens ses files de ce
            // sens
            let (mut b_in, mut b_out, mut n_in, mut n_out, mut doubles) = (0usize, 0usize, 0usize, 0usize, 0usize);
            for &(j, f) in &bouts {
                if (j, f) == (t, tf) {
                    continue;
                }
                match entrant(&r.aretes[j], f) {
                    Some(true) => {
                        b_in += r.aretes[j].files.total as usize;
                        n_in += 1;
                    }
                    Some(false) => {
                        b_out += r.aretes[j].files.total as usize;
                        n_out += 1;
                    }
                    None => {
                        doubles += 1;
                        let (ei, so) = files_du_sens(j, f);
                        if ei.len() + so.len() < r.aretes[j].files.total as usize {
                            ok = false;
                        }
                        if !ei.is_empty() {
                            b_in += ei.len();
                            n_in += 1;
                        }
                        if !so.is_empty() {
                            b_out += so.len();
                            n_out += 1;
                        }
                    }
                }
            }
            if doubles > 1 {
                ok = false;
            }
            // Juste : autant de files ; ou une partagée à chaque séparation ; ou, pour une
            // seule branche, les files du tronc moins ses files de bus (qui se ferment)
            let juste = |b: usize, n: usize, t_: usize, bus: usize| b == t_ || (n >= 2 && b == t_ + n - 1) || (n == 1 && bus > 0 && b + bus == t_);
            // Seul compte le sens de notre bout : l'autre sens peut ne pas se partager
            let dans_notre_sens = if e { juste(b_in, n_in, sorties, bus_sorties) } else { juste(b_out, n_out, entrees, bus_entrees) };
            if ok && dans_notre_sens {
                if trouve.is_some() {
                    return None;
                }
                trouve = Some(((t, tf), bouts));
            }
        }
        trouve
    };
    // D'abord les voies du même genre que le tronc (une rue qui s'y greffe n'en est
    // pas) ; à défaut, celles à moins de TOUT_DROIT du prolongement du tronc
    // Une voie naissante : le tronc continue tel quel dans une branche (autant de files
    // dans notre sens, la plus alignée), et d'autres branches de notre sens naissent à
    // son côté (ou, à l'inverse, s'y fondent). Rend le tronc, les bouts gardés et la
    // continuation.
    let naitre = |garde: &dyn Fn((usize, bool), (usize, bool)) -> bool| -> Option<((usize, bool), Vec<(usize, bool)>, (usize, bool))> {
        let mut trouve = None;
        for &(t, tf) in &tous {
            if t == k {
                continue;
            }
            let bouts: Vec<(usize, bool)> = tous.iter().copied().filter(|&x| x == (t, tf) || garde((t, tf), x)).collect();
            if bouts.len() < 3 || !bouts.contains(&(k, fin)) {
                continue;
            }
            let ta = &r.aretes[t];
            let nt = (0..ta.files.total as usize).filter(|&i| file_entre(ta, tf, i) == Some(!e)).count();
            if nt == 0 {
                continue;
            }
            let branches: Vec<(usize, bool)> = bouts.iter().copied().filter(|&(j, f)| (j, f) != (t, tf) && entrant(&r.aretes[j], f) == Some(e)).collect();
            if !branches.contains(&(k, fin)) || branches.iter().map(|&(j, _)| r.aretes[j].files.total as usize).sum::<usize>() <= nt {
                continue;
            }
            let u = dir(t, tf);
            let ecart = |(j, f): (usize, bool)| {
                let v = dir(j, f);
                (-(u[0] * v[0] + u[1] * v[1])).clamp(-1.0, 1.0).acos()
            };
            let suite = branches
                .iter()
                .copied()
                .filter(|&(j, _)| r.aretes[j].files.total as usize == nt)
                .min_by(|&x, &y| ecart(x).total_cmp(&ecart(y)));
            let Some(suite) = suite else { continue };
            if suite == (k, fin) || ecart(suite) > TOUT_DROIT.to_radians() {
                continue;
            }
            if trouve.is_some() {
                return None;
            }
            trouve = Some(((t, tf), bouts, suite));
        }
        trouve
    };
    // D'abord les voies du même genre que le tronc (une rue qui s'y greffe n'en est
    // pas) ; à défaut, celles à moins de TOUT_DROIT du prolongement du tronc
    let alignee = |(t, tf): (usize, bool), (j, f): (usize, bool)| {
        let (u, v) = (dir(t, tf), dir(j, f));
        // Une branche part à l'opposé du tronc
        (-(u[0] * v[0] + u[1] * v[1])).clamp(-1.0, 1.0).acos() <= TOUT_DROIT.to_radians()
    };
    // Une voie qui tourne (une rue en travers) n'est jamais une branche, même du même
    // genre
    let meme_genre = |t: (usize, bool), j: (usize, bool)| r.aretes[j.0].genre == r.aretes[t.0].genre && alignee(t, j);
    // Le côté où naît une file, d'après la fourche d'où sort la voie `t` (bout `tf` au
    // nœud) : on la remonte (à travers les nœuds à 2 bouts de la même voie) jusqu'à une
    // fourche ; la file naît du côté de la branche sœur, la place qu'elle a libérée.
    // Vrai : du côté des grands « travers » au nœud (rien : pas de fourche trouvée).
    let cote_libere = |t: usize, tf: bool| -> Option<bool> {
        let (mut j, mut jf) = (t, !tf);
        for _ in 0..16 {
            let aj = &r.aretes[j];
            let m = if jf { aj.b } else { aj.a };
            let mut autres: Vec<(usize, bool)> = Vec::new();
            for &x in &r.noeuds[m].aretes {
                let b = &r.aretes[x];
                if !sage_reel::carrossable(b.genre) || b.points.len() < 2 || b.files.total == 0 {
                    continue;
                }
                for xf in [false, true] {
                    if (if xf { b.b } else { b.a }) == m && (x, xf) != (j, jf) {
                        autres.push((x, xf));
                    }
                }
            }
            // La direction de la voie en s'éloignant de m (vers notre nœud)
            let dj = dir(j, jf);
            let ecart = |x: usize, xf: bool, sens: f32| {
                let v = dir(x, xf);
                (sens * (v[0] * dj[0] + v[1] * dj[1])).clamp(-1.0, 1.0).acos()
            };
            let ej = entrant(aj, jf)?;
            if autres.len() == 1 {
                // La même voie, qui continue : on la remonte
                let (x, xf) = autres[0];
                if r.aretes[x].genre != aj.genre || entrant(&r.aretes[x], xf) != Some(!ej) || ecart(x, xf, -1.0) > TOUT_DROIT.to_radians() {
                    return None;
                }
                (j, jf) = (x, !xf);
                continue;
            }
            // Une fourche : la sœur, de même sens (de tout genre : une rampe, une route
            // qui s'en va), la plus proche en angle
            let soeur = autres
                .iter()
                .copied()
                .filter(|&(x, xf)| entrant(&r.aretes[x], xf) == Some(ej) && ecart(x, xf, 1.0) <= TOUT_DROIT.to_radians())
                .min_by(|&(x, xf), &(y, yf)| ecart(x, xf, 1.0).total_cmp(&ecart(y, yf, 1.0)))?;
            let v = dir(soeur.0, soeur.1);
            let c = v[0] * dj[1] - v[1] * dj[0];
            if c.abs() < 1e-4 {
                return None;
            }
            return Some(c > 0.0);
        }
        None
    };
    // Une voie qui gagne une file au nœud (n → n + 1, deux bouts de la même voie) : la
    // voie étroite y sert de tronc
    let elargie = if tous.len() == 2 {
        let (t, tf) = if tous[0] == (k, fin) { tous[1] } else { tous[0] };
        let ta_ = &r.aretes[t];
        if t != k && tous.contains(&(k, fin)) && entrant(ta_, tf) == Some(!e) && ta_.genre == a.genre && alignee((t, tf), (k, fin)) && a.files.total > ta_.files.total {
            Some((t, tf))
        } else {
            None
        }
    } else {
        None
    };
    let ((t, tf), bouts, mode) = match elargie {
        // Une seule file de plus : elle naît du côté libéré s'il est connu ; plusieurs
        // (un élargissement, 2 → 4) : les files se répartissent en symétrie
        Some((t, tf)) => ((t, tf), tous.clone(), Mode::Elargie(if a.files.total == r.aretes[t].files.total + 1 { cote_libere(t, tf) } else { None })),
        None => match chercher(&meme_genre).or_else(|| chercher(&alignee)) {
            Some((t, b)) => (t, b, Mode::Fourche),
            None => {
                let (t, b, s_) = naitre(&meme_genre).or_else(|| naitre(&alignee))?;
                (t, b, Mode::Naissante(s_))
            }
        },
    };
    let ta = &r.aretes[t];
    // Les branches de notre sens
    let dans_sens = |j: usize, f: bool| -> usize {
        match entrant(&r.aretes[j], f) {
            Some(x) if x == e => r.aretes[j].files.total as usize,
            Some(_) => 0,
            None => {
                let (ei, so) = files_du_sens(j, f);
                if e { ei.len() } else { so.len() }
            }
        }
    };
    let branches: Vec<(usize, bool)> = if matches!(mode, Mode::Fourche) {
        bouts.iter().copied().filter(|&(j, f)| (j, f) != (t, tf) && dans_sens(j, f) > 0).collect()
    } else {
        bouts.iter().copied().filter(|&(j, f)| (j, f) != (t, tf) && entrant(&r.aretes[j], f) == Some(e)).collect()
    };
    // Le tronc depuis le nœud : sa direction, et l'axe de ses files de notre sens (qui
    // entrent au nœud si nous en sortons) au nœud, en travers
    let tp = deporte(r, t);
    if tp.len() < 2 {
        return None;
    }
    let tg = crate::files::normales(&tp);
    let (i0, i1) = if tf { (tp.len() - 1, tp.len() - 2) } else { (0, 1) };
    let o = tp[i0];
    let dl = (tp[i1][0] - o[0]).hypot(tp[i1][1] - o[1]).max(1e-6);
    let d = [(tp[i1][0] - o[0]) / dl, (tp[i1][1] - o[1]) / dl];
    let travers = |q: [f32; 2]| (q[0] - o[0]) * -d[1] + (q[1] - o[1]) * d[0];
    let mut fs: Vec<(usize, [f32; 2])> = (0..ta.files.total as usize)
        .filter(|&i| file_entre(ta, tf, i) == Some(!e))
        .map(|i| {
            let y = crate::files::decalage(ta, i);
            (i, [o[0] + tg[i0][0] * y, o[1] + tg[i0][1] * y])
        })
        .collect();
    fs.sort_by(|x, y| travers(x.1).total_cmp(&travers(y.1)));
    // Une branche, ses points vers le nœud : sa cassure (rang, point), et son repère en
    // travers (la cassure, ou à défaut l'avant-dernier point)
    let cassure = |q: &[[f32; 3]], arret: usize| -> (usize, [f32; 2]) {
        let m = q.len();
        // L'écart se mesure à la direction du dernier segment (au nœud), pas d'angle
        // à angle : une courbe faite de petits coudes n'est pas une droite
        let u2 = [q[m - 1][0] - q[m - 2][0], q[m - 1][1] - q[m - 2][1]];
        for i in (1..m - 1).rev() {
            if i <= arret {
                return (i, [q[i][0], q[i][1]]);
            }
            // La droite du raccord s'écarterait trop du tracé : la cassure au point d'avant
            if fleche(q, i) > FLECHE_MAX {
                let j = (i + 1).min(m - 2);
                return (j, [q[j][0], q[j][1]]);
            }
            let u1 = [q[i][0] - q[i - 1][0], q[i][1] - q[i - 1][1]];
            let (l1, l2) = (u1[0].hypot(u1[1]), u2[0].hypot(u2[1]));
            if l1 < 1e-6 || l2 < 1e-6 {
                continue;
            }
            let c = ((u1[0] * u2[0] + u1[1] * u2[1]) / (l1 * l2)).clamp(-1.0, 1.0);
            // La fin de la section strictement droite depuis le nœud
            if c.acos() > tolerance {
                return (i, [q[i][0], q[i][1]]);
            }
        }
        // Droite jusqu'au bout : toute la voie est déviée
        (0, [q[0][0], q[0][1]])
    };
    let vers_noeud = |pts: &[[f32; 3]], f: bool| -> Vec<[f32; 3]> { if f { pts.to_vec() } else { pts.iter().rev().copied().collect() } };
    let mut bs: Vec<(usize, bool, usize, f32)> = Vec::new();
    let mut mien: Option<usize> = None;
    for &(j, f) in &branches {
        let q = if j == k { vers_noeud(p, f) } else { vers_noeud(&deporte(r, j), f) };
        if q.len() < 2 {
            return None;
        }
        let (i, mut rep) = cassure(&q, if j == k && f == fin { arret } else { 0 });
        if j == k && f == fin {
            mien = Some(i);
        }
        let mut nb = r.aretes[j].files.total as usize;
        // Une branche à double sens : ses seules files de notre sens, repérées à leur
        // place en travers
        if entrant(&r.aretes[j], f).is_none() {
            let (ei, so) = files_du_sens(j, f);
            let mes = if e { ei } else { so };
            nb = mes.len();
            let pts = deporte(r, j);
            let tg_ = crate::files::normales(&pts);
            let ii = if f { i } else { pts.len() - 1 - i };
            let y = mes.iter().map(|&m| crate::files::decalage(&r.aretes[j], m)).sum::<f32>() / nb.max(1) as f32;
            rep = [pts[ii][0] + tg_[ii][0] * y, pts[ii][1] + tg_[ii][1] * y];
        }
        bs.push((j, f, nb, travers(rep)));
    }
    let i = mien?;
    bs.sort_by(|x, y| x.3.total_cmp(&y.3));
    let mut cibles: Option<Vec<(usize, [f32; 2])>> = None;
    let mut partagee = false;
    let mut bus_ferme = false;
    match mode {
        // Une fourche : les branches prennent les files du tronc dans l'ordre
        Mode::Fourche => {
            // Partagée : à chaque séparation, la dernière file d'une branche est aussi la
            // première de la suivante
            let somme: usize = bs.iter().map(|b| b.2).sum();
            // Une seule branche plus étroite : les files de bus du tronc s'y ferment, les
            // autres continuent dans l'ordre
            let sans_bus: Vec<(usize, [f32; 2])> = fs.iter().copied().filter(|x| !ta.files.est_bus(x.0)).collect();
            let (fs_, partage) = if somme == fs.len() {
                (fs.clone(), 0)
            } else if bs.len() >= 2 && somme == fs.len() + bs.len() - 1 {
                (fs.clone(), 1)
            } else if bs.len() == 1 && sans_bus.len() < fs.len() && somme == sans_bus.len() {
                bus_ferme = true;
                (sans_bus, 0)
            } else {
                return None;
            };
            let mut rang = 0;
            for &(j, f, nf, _) in &bs {
                if nf == 0 || rang + nf > fs_.len() {
                    return None;
                }
                if j == k && f == fin {
                    cibles = Some(fs_[rang..rang + nf].to_vec());
                }
                rang += nf - partage;
            }
            partagee = partage == 1;
        }
        // Une voie naissante : piquée sur les files extrêmes du tronc de son côté (les
        // premières qui ne sont pas réservées au bus)
        Mode::Naissante(s_) => {
            let x = |jf: (usize, bool)| bs.iter().find(|b| (b.0, b.1) == jf).map(|b| b.3);
            let (mien_x, suite_x) = (x((k, fin))?, x(s_)?);
            let nf = a.files.total as usize;
            if nf == 0 || nf > fs.len() {
                return None;
            }
            let libre = |d0: usize| (d0..d0 + nf).all(|m| !ta.files.est_bus(fs[m].0));
            let debuts: Vec<usize> = if mien_x < suite_x { (0..=fs.len() - nf).collect() } else { (0..=fs.len() - nf).rev().collect() };
            let d0 = debuts.iter().copied().find(|&d0| libre(d0)).unwrap_or(debuts[0]);
            cibles = Some(fs[d0..d0 + nf].to_vec());
        }
        // Une file naissante dans la même voie : les n files gardent leur place ; la
        // nouvelle est piquée sur la file extrême de son côté, ou, sans côté connu, les
        // n + 1 files se répartissent en symétrie sur les n (les extrêmes sur les
        // extrêmes, celles du milieu entre deux)
        Mode::Elargie(cote) => {
            let n = fs.len();
            if n == 0 {
                return None;
            }
            cibles = Some(match cote {
                Some(true) => fs.iter().copied().chain(std::iter::once(fs[n - 1])).collect(),
                Some(false) => std::iter::once(fs[0]).chain(fs.iter().copied()).collect(),
                // Plusieurs files de plus (un élargissement) : chaque file étroite en
                // nourrit plusieurs larges, groupées dans l'ordre (2 → 4 : f0 f1 sur la
                // première, f2 f3 sur la seconde)
                None if a.files.total as usize > n + 1 => (0..a.files.total as usize).map(|j| fs[j * n / a.files.total as usize]).collect(),
                None => (0..=n)
                    .map(|j| {
                        let x = j as f32 * (n - 1) as f32 / n as f32;
                        let (a0, f_) = (x.floor() as usize, x.fract());
                        let a1 = (a0 + 1).min(n - 1);
                        let (p0, p1) = (fs[a0].1, fs[a1].1);
                        (fs[a0].0, [p0[0] + (p1[0] - p0[0]) * f_, p0[1] + (p1[1] - p0[1]) * f_])
                    })
                    .collect(),
            });
        }
    }
    let cibles = cibles?;
    let nf = cibles.len();
    // Le centre au nœud : celui des cibles (fourche, voie naissante), ou celui de la
    // voie étroite (file naissante) ; et le nombre de files de la largeur au nœud
    let (c, nl) = match mode {
        Mode::Elargie(_) => (fs.iter().fold([0.0f32; 2], |s, x| [s[0] + x.1[0] / fs.len() as f32, s[1] + x.1[1] / fs.len() as f32]), fs.len()),
        _ => (cibles.iter().fold([0.0f32; 2], |s, x| [s[0] + x.1[0] / nf as f32, s[1] + x.1[1] / nf as f32]), nf),
    };
    let regle = match mode {
        Mode::Fourche if bs.len() == 1 && (0..ta.files.total as usize).any(|x| file_entre(ta, tf, x) == Some(e)) => "séparation de chaussée",
        Mode::Fourche if partagee => "fourche, file partagée",
        Mode::Fourche if bus_ferme => "fourche, file de bus fermée",
        Mode::Fourche => "fourche",
        Mode::Naissante(_) => "voie naissante",
        Mode::Elargie(Some(_)) => "file naissante, côté libéré",
        Mode::Elargie(None) if a.files.total as usize > fs.len() + 1 => "élargissement",
        Mode::Elargie(None) => "file naissante, symétrie",
    };
    // Le raccord : une droite de la cassure au centre des files sur le tronc
    let mut q = vers_noeud(p, fin);
    let (zc, zn) = (q[i][2], q[q.len() - 1][2]);
    let pc = q[i];
    let u = {
        let v = if i == 0 { [q[1][0] - q[0][0], q[1][1] - q[0][1]] } else { [q[i][0] - q[i - 1][0], q[i][1] - q[i - 1][1]] };
        let l = v[0].hypot(v[1]).max(1e-6);
        [v[0] / l, v[1] / l]
    };
    let dist: f32 = (i..q.len() - 1).map(|m| (q[m + 1][0] - q[m][0]).hypot(q[m + 1][1] - q[m][1])).sum();
    q.truncate(i + 1);
    q.push([c[0], c[1], zn]);
    if !fin {
        q.reverse();
    }
    *p = q;
    // Les nœuds de files : chaque file de la branche, à la cassure et sur le tronc
    let demi = ta.largeur / ta.files.total.max(1) as f32 * nl as f32 * 0.5;
    let g0 = [u[1], -u[0]];
    let noeuds = (0..nf)
        .map(|r_| {
            // La file r_ de la branche, de sa gauche à sa droite dans le sens du tracé
            let y0 = crate::files::decalage(a, r_);
            let y0 = if fin { y0 } else { -y0 };
            let cassure = [pc[0] + g0[0] * y0, pc[1] + g0[1] * y0, zc];
            NoeudsDeFile { rang: r_, cassure, distance: 0.0, tronc: t, rang_tronc: 0, sur_tronc: [0.0, 0.0, zn], bords: [[0.0; 2]; 2] }
        })
        .collect::<Vec<_>>();
    // Les files de la branche prises en travers du tronc dans le même ordre que ses cibles
    let mut noeuds = noeuds;
    noeuds.sort_by(|x, y| travers([x.cassure[0], x.cassure[1]]).total_cmp(&travers([y.cassure[0], y.cassure[1]])));
    // Les bords de chaque file au nœud : ceux d'une file du tronc autour de sa cible,
    // à gauche et à droite du tracé de la branche
    let fw = ta.largeur / ta.files.total.max(1) as f32;
    let sg = if fin { 1.0 } else { -1.0 };
    let gb = [sg * d[1], -sg * d[0]];
    for (nd, cb) in noeuds.iter_mut().zip(&cibles) {
        nd.distance = dist;
        nd.rang_tronc = cb.0;
        nd.sur_tronc = [cb.1[0], cb.1[1], zn];
        let h = fw * 0.5;
        nd.bords = [[cb.1[0] + gb[0] * h, cb.1[1] + gb[1] * h], [cb.1[0] - gb[0] * h, cb.1[1] - gb[1] * h]];
    }
    noeuds.sort_by_key(|x| x.rang);
    Some(Raccord { demi, points: RACCORD_POINTS, noeuds, entier: i == 0, regle })
}

/// Une file naissante dans une voie à double sens (n → n + 1, deux bouts de la même
/// voie alignés) : un sens garde ses files, l'autre en gagne une, qui naît du côté
/// extérieur (à l'opposé de l'autre sens : une file pour tourner). La voie large est
/// raccordée sur l'étroite, qui sert de tronc : ses files continuent celles du tronc
/// dans l'ordre, la nouvelle est piquée sur sa voisine.
fn raccourcir_double(r: &Reseau, k: usize, fin: bool, p: &mut Vec<[f32; 3]>, tolerance: f32, arret: usize) -> Option<Raccord> {
    let a = &r.aretes[k];
    let n = if fin { a.b } else { a.a };
    let mut tous: Vec<(usize, bool)> = Vec::new();
    for &j in &r.noeuds[n].aretes {
        let b = &r.aretes[j];
        if !sage_reel::carrossable(b.genre) || b.points.len() < 2 || b.files.total == 0 {
            continue;
        }
        for f in [false, true] {
            if (if f { b.b } else { b.a }) == n {
                tous.push((j, f));
            }
        }
    }
    if tous.len() != 2 || !tous.contains(&(k, fin)) {
        return None;
    }
    let (t, tf) = if tous[0] == (k, fin) { tous[1] } else { tous[0] };
    let ta = &r.aretes[t];
    if t == k || ta.genre != a.genre || a.files.total != ta.files.total + 1 {
        return None;
    }
    // Le sens de chaque file au nœud : vrai si elle y entre
    let sens = |b: &sage_reel::reseau::Arete, f: bool, i: usize| match crate::files::sens(b, i) {
        crate::files::Sens::Avant => Some(f),
        crate::files::Sens::Arriere => Some(!f),
        crate::files::Sens::Partagee => None,
    };
    let compte = |b: &sage_reel::reseau::Arete, f: bool| -> Option<(usize, usize)> {
        let (mut e_, mut s_) = (0, 0);
        for i in 0..b.files.total as usize {
            if sens(b, f, i)? {
                e_ += 1
            } else {
                s_ += 1
            }
        }
        Some((e_, s_))
    };
    let ((ek, sk), (et, st)) = (compte(a, fin)?, compte(ta, tf)?);
    // Nos files qui entrent continuent celles du tronc qui sortent, et l'inverse ; le
    // sens qui gagne une file
    let gagne_entrant = if ek == st + 1 && sk == et {
        true
    } else if sk == et + 1 && ek == st {
        false
    } else {
        return None;
    };
    if ek == 0 || sk == 0 {
        return None;
    }
    // Le tronc depuis le nœud
    let tp = deporte(r, t);
    let m = tp.len();
    if m < 2 {
        return None;
    }
    let tg = crate::files::normales(&tp);
    let (i0, i1) = if tf { (m - 1, m - 2) } else { (0, 1) };
    let o = tp[i0];
    let dl = (tp[i1][0] - o[0]).hypot(tp[i1][1] - o[1]).max(1e-6);
    let d = [(tp[i1][0] - o[0]) / dl, (tp[i1][1] - o[1]) / dl];
    // Alignées : notre voie part à l'opposé du tronc
    {
        let q = &a.points;
        let (x0, x1) = if fin { (q[q.len() - 1], q[q.len() - 2]) } else { (q[0], q[1]) };
        let l = (x1[0] - x0[0]).hypot(x1[1] - x0[1]).max(1e-6);
        let v = [(x1[0] - x0[0]) / l, (x1[1] - x0[1]) / l];
        if (-(d[0] * v[0] + d[1] * v[1])).clamp(-1.0, 1.0).acos() > TOUT_DROIT.to_radians() {
            return None;
        }
    }
    let travers = |q: [f32; 2]| (q[0] - o[0]) * -d[1] + (q[1] - o[1]) * d[0];
    let mut fs: Vec<(usize, [f32; 2], bool)> = (0..ta.files.total as usize)
        .map(|i| {
            let y = crate::files::decalage(ta, i);
            (i, [o[0] + tg[i0][0] * y, o[1] + tg[i0][1] * y], sens(ta, tf, i).unwrap_or(false))
        })
        .collect();
    fs.sort_by(|x, y| travers(x.1).total_cmp(&travers(y.1)));
    // Notre cassure
    let vers_noeud = |pts: &[[f32; 3]]| -> Vec<[f32; 3]> { if fin { pts.to_vec() } else { pts.iter().rev().copied().collect() } };
    let mut q = vers_noeud(p);
    let mq = q.len();
    if mq < 2 {
        return None;
    }
    // La cassure : en remontant depuis le nœud, le premier point où le tracé s'écarte
    // de la direction de son dernier segment (écart cumulé, pas d'angle à angle : une
    // courbe douce n'est pas droite), ou au plus RACC_DOUBLE_MAX m du nœud
    let fin_dir = {
        let v = [q[mq - 1][0] - q[mq - 2][0], q[mq - 1][1] - q[mq - 2][1]];
        let l = v[0].hypot(v[1]).max(1e-6);
        [v[0] / l, v[1] / l]
    };
    let mut i = 0usize;
    let mut cumul = 0.0f32;
    for j in (1..mq - 1).rev() {
        cumul += (q[j + 1][0] - q[j][0]).hypot(q[j + 1][1] - q[j][1]);
        if j <= arret || cumul >= RACC_DOUBLE_MAX {
            i = j;
            break;
        }
        if fleche(&q, j) > FLECHE_MAX {
            i = (j + 1).min(mq - 2);
            break;
        }
        let u1 = [q[j][0] - q[j - 1][0], q[j][1] - q[j - 1][1]];
        let l1 = u1[0].hypot(u1[1]);
        if l1 < 1e-6 {
            continue;
        }
        if ((u1[0] * fin_dir[0] + u1[1] * fin_dir[1]) / l1).clamp(-1.0, 1.0).acos() > tolerance {
            i = j;
            break;
        }
    }
    if i == 0 && cumul + (q[1][0] - q[0][0]).hypot(q[1][1] - q[0][1]) > RACC_DOUBLE_MAX {
        // Une seule longue droite : la cassure à RACC_DOUBLE_MAX du nœud
        let (a_, b_) = (q[mq - 2], q[mq - 1]);
        let l = (b_[0] - a_[0]).hypot(b_[1] - a_[1]).max(1e-6);
        if mq == 2 && l > RACC_DOUBLE_MAX {
            let t_ = 1.0 - RACC_DOUBLE_MAX / l;
            q.insert(1, [a_[0] + (b_[0] - a_[0]) * t_, a_[1] + (b_[1] - a_[1]) * t_, a_[2] + (b_[2] - a_[2]) * t_]);
            i = 1;
        }
    }
    let mq = q.len();
    let (zc, zn) = (q[i][2], q[mq - 1][2]);
    let pc = q[i];
    let u = {
        let v = if i == 0 { [q[1][0] - q[0][0], q[1][1] - q[0][1]] } else { [q[i][0] - q[i - 1][0], q[i][1] - q[i - 1][1]] };
        let l = v[0].hypot(v[1]).max(1e-6);
        [v[0] / l, v[1] / l]
    };
    let g0 = [u[1], -u[0]];
    // Nos files à la cassure, en travers du tronc
    let nf = a.files.total as usize;
    let mut noeuds: Vec<NoeudsDeFile> = (0..nf)
        .map(|r_| {
            let y0 = crate::files::decalage(a, r_);
            let y0 = if fin { y0 } else { -y0 };
            let cassure = [pc[0] + g0[0] * y0, pc[1] + g0[1] * y0, zc];
            NoeudsDeFile { rang: r_, cassure, distance: 0.0, tronc: t, rang_tronc: 0, sur_tronc: [0.0, 0.0, zn], bords: [[0.0; 2]; 2] }
        })
        .collect();
    noeuds.sort_by(|x, y| travers([x.cassure[0], x.cassure[1]]).total_cmp(&travers([y.cassure[0], y.cassure[1]])));
    // Le groupe qui gagne une file est à un bout en travers ; sa file extrême naît
    let entre = |r_: usize| sens(a, fin, r_).unwrap_or(false);
    let premier_gagne = entre(noeuds[0].rang) == gagne_entrant;
    let dernier_gagne = entre(noeuds[nf - 1].rang) == gagne_entrant;
    if premier_gagne == dernier_gagne {
        return None;
    }
    let nouvelle = if premier_gagne { 0 } else { nf - 1 };
    // Les cibles : les autres files dans l'ordre du tronc, la nouvelle sur sa voisine
    let mut cibles: Vec<(usize, [f32; 2])> = Vec::with_capacity(nf);
    let mut it = fs.iter();
    for x in 0..nf {
        if x == nouvelle {
            cibles.push((0, [0.0, 0.0]));
            continue;
        }
        let f_ = it.next()?;
        cibles.push((f_.0, f_.1));
    }
    let voisine = if nouvelle == 0 { 1 } else { nf - 2 };
    cibles[nouvelle] = cibles[voisine];
    // Les sens doivent correspondre : nos files entrantes sur les sortantes du tronc
    for (x, nd) in noeuds.iter().enumerate() {
        if x == nouvelle {
            continue;
        }
        let ft = fs.iter().find(|f_| f_.0 == cibles[x].0)?;
        if entre(nd.rang) == ft.2 {
            return None;
        }
    }
    // Le raccord : une droite de la cassure au centre du tronc
    let c = fs.iter().fold([0.0f32; 2], |s_, x| [s_[0] + x.1[0] / fs.len() as f32, s_[1] + x.1[1] / fs.len() as f32]);
    let dist: f32 = (i..mq - 1).map(|j| (q[j + 1][0] - q[j][0]).hypot(q[j + 1][1] - q[j][1])).sum();
    q.truncate(i + 1);
    q.push([c[0], c[1], zn]);
    if !fin {
        q.reverse();
    }
    *p = q;
    let fw = ta.largeur / ta.files.total.max(1) as f32;
    let sg = if fin { 1.0 } else { -1.0 };
    let gb = [sg * d[1], -sg * d[0]];
    for (nd, cb) in noeuds.iter_mut().zip(&cibles) {
        nd.distance = dist;
        nd.rang_tronc = cb.0;
        nd.sur_tronc = [cb.1[0], cb.1[1], zn];
        let h = fw * 0.5;
        nd.bords = [[cb.1[0] + gb[0] * h, cb.1[1] + gb[1] * h], [cb.1[0] - gb[0] * h, cb.1[1] - gb[1] * h]];
    }
    noeuds.sort_by_key(|x| x.rang);
    Some(Raccord { demi: ta.largeur * 0.5, points: RACCORD_POINTS, noeuds, entier: i == 0, regle: "file naissante à double sens, côté extérieur" })
}

/// La plus grande flèche (m) tolérée entre la droite d'un raccord et le tracé brut :
/// au-delà, la cassure est avancée vers le nœud.
const FLECHE_MAX: f32 = 0.15;

/// La flèche de la corde de `q[i]` au dernier point (le nœud) : le plus grand écart des
/// points entre eux à cette droite.
fn fleche(q: &[[f32; 3]], i: usize) -> f32 {
    let (a, b) = (q[i], q[q.len() - 1]);
    let d = [b[0] - a[0], b[1] - a[1]];
    let l = d[0].hypot(d[1]);
    if l < 1e-6 {
        return 0.0;
    }
    q[i + 1..q.len() - 1].iter().map(|p| ((p[0] - a[0]) * d[1] - (p[1] - a[1]) * d[0]).abs() / l).fold(0.0, f32::max)
}

/// La plus grande longueur (m) du biseau d'une file naissante à double sens.
const RACC_DOUBLE_MAX: f32 = 25.0;

/// Les points du raccord d'une branche de fourche après la cassure (une droite).
const RACCORD_POINTS: usize = 1;

/// Une file d'une branche raccordée à une fourche : ses deux nœuds de files, à la
/// cassure de la branche et sur la file du tronc qu'elle continue (au nœud).
#[derive(Clone, Debug)]
pub struct NoeudsDeFile {
    pub rang: usize,
    pub cassure: [f32; 3],
    /// La distance de la cassure au nœud, le long du tracé (m).
    pub distance: f32,
    pub tronc: usize,
    pub rang_tronc: usize,
    pub sur_tronc: [f32; 3],
    /// Les bords de la file au nœud (gauche, droite du tracé de la branche).
    pub bords: [[f32; 2]; 2],
}

/// Le raccord d'un bout de branche : la demi-largeur au nœud, le nombre de points de la
/// courbe après la cassure, et les nœuds de files.
#[derive(Clone, Debug)]
pub struct Raccord {
    pub demi: f32,
    pub points: usize,
    pub noeuds: Vec<NoeudsDeFile>,
    /// La voie est droite jusqu'à son autre bout : toute déviée.
    pub entier: bool,
    /// La règle appliquée (pour l'encart).
    pub regle: &'static str,
}

/// Une amorce : une petite impasse carrossable (moins de AMORCE_MAX), dont un bout ne
/// rejoint qu'une voie non carrossable (une passerelle, un chemin) et l'autre un
/// carrefour d'au moins deux voies. Elle fait partie du carrefour (le sommet d'une
/// épingle) : ni fourche, ni chaussée à part.
pub fn est_amorce(r: &Reseau, k: usize) -> bool {
    let a = &r.aretes[k];
    if !sage_reel::carrossable(a.genre) || a.files.total == 0 || a.points.len() < 2 || a.a == a.b {
        return false;
    }
    let l: f32 = a.points.windows(2).map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1])).sum();
    if l >= AMORCE_MAX {
        return false;
    }
    // Les autres bouts carrossables à un nœud
    let autres = |n: usize| -> usize {
        r.noeuds[n].aretes.iter().filter(|&&j| j != k).map(|&j| {
            let b = &r.aretes[j];
            if !sage_reel::carrossable(b.genre) || b.files.total == 0 || b.points.len() < 2 {
                0
            } else {
                (b.a == n) as usize + (b.b == n) as usize
            }
        }).sum()
    };
    // Une voie non carrossable (passerelle, chemin, escalier) arrive à son bout : une
    // impasse qui ne mène à rien est une vraie petite rue, pas une amorce
    let pieton = |n: usize| r.noeuds[n].aretes.iter().any(|&j| j != k && !sage_reel::carrossable(r.aretes[j].genre));
    let (na, nb) = (autres(a.a), autres(a.b));
    (na == 0 && nb >= 2 && pieton(a.a)) || (nb == 0 && na >= 2 && pieton(a.b))
}

/// Retire de la polyligne `l` la longueur `d` (m) à son bout (`fin`) ou à son début.
pub fn rogner_bout(l: &mut Vec<[f32; 3]>, fin: bool, d: f32) {
    if d <= 0.0 || l.len() < 2 {
        return;
    }
    if !fin {
        l.reverse();
    }
    let mut reste = d;
    while l.len() >= 2 {
        let (a, b) = (l[l.len() - 2], l[l.len() - 1]);
        let s = (b[0] - a[0]).hypot(b[1] - a[1]);
        if s > reste + 1e-4 {
            let t = (s - reste) / s;
            let m = l.len();
            l[m - 1] = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t];
            break;
        }
        reste -= s;
        l.pop();
    }
    if l.len() < 2 {
        l.clear();
    }
    if !fin {
        l.reverse();
    }
}

/// Pose les bouts raccordés d'une polyligne de l'arête (une file, une ligne entre
/// files) : au début et à la fin, le point que `bout` donne pour le raccord de ce bout.
pub fn raccorder_bouts(raccords: &[Option<Raccord>; 2], l: &mut [[f32; 3]], bout: impl Fn(&Raccord) -> Option<[f32; 2]>) {
    let m = l.len();
    if m == 0 {
        return;
    }
    for (i, j) in [(0, 0), (1, m - 1)] {
        if let Some(q) = raccords[i].as_ref().and_then(&bout) {
            l[j][0] = q[0];
            l[j][1] = q[1];
        }
    }
}

/// La règle d'un raccord.
#[derive(Clone, Copy)]
enum Mode {
    Fourche,
    /// Une voie naissante, et la voie qui continue le tronc.
    Naissante((usize, bool)),
    /// Une file naissante dans la même voie, et son côté s'il est connu.
    Elargie(Option<bool>),
}

impl Raccord {
    /// Le point au nœud de la file de rang `i` (sur le tronc).
    pub fn bout_file(&self, i: usize) -> Option<[f32; 2]> {
        self.noeuds.iter().find(|n| n.rang == i).map(|n| [n.sur_tronc[0], n.sur_tronc[1]])
    }
    /// Le point au nœud du bord gauche (de la file 0) ou droit (de la file `n − 1`) de
    /// la voie de `n` files.
    pub fn bout_bord(&self, gauche: bool, n: usize) -> Option<[f32; 2]> {
        let i = if gauche { 0 } else { n.checked_sub(1)? };
        self.noeuds.iter().find(|x| x.rang == i).map(|x| x.bords[if gauche { 0 } else { 1 }])
    }
    /// Les files `i − 1` et `i` aboutissent à la même file du tronc.
    pub fn fondues(&self, i: usize) -> bool {
        let r = |j: usize| self.noeuds.iter().find(|x| x.rang == j).map(|x| x.rang_tronc);
        i >= 1 && r(i - 1).is_some() && r(i - 1) == r(i)
    }
    /// Le point au nœud de la ligne entre les files `i − 1` et `i`.
    pub fn bout_ligne(&self, i: usize) -> Option<[f32; 2]> {
        let (a, b) = (self.bout_file(i.checked_sub(1)?)?, self.bout_file(i)?);
        Some([(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0])
    }
}

/// L'axe de l'arête `k` pour ses files : déporté près d'une fourche, et raccourci aux
/// fourches de sens unique (`raccourcir`).
pub fn trace(r: &Reseau, k: usize) -> Vec<[f32; 3]> {
    trace_largeurs(r, k).0
}

/// L'axe de `trace` et la demi-largeur de la voie en chacun de ses points : celle de la
/// voie, sauf au bout raccourci d'une branche de fourche, celle de ses files sur le
/// tronc (la largeur change en biseau depuis la cassure : les bords suivent les files).
pub fn trace_largeurs(r: &Reseau, k: usize) -> (Vec<[f32; 3]>, Vec<f32>) {
    let (p, w, _) = trace_raccords(r, k);
    (p, w)
}

/// `trace_largeurs`, avec les raccords des deux bouts (début, fin).
pub fn trace_raccords(r: &Reseau, k: usize) -> (Vec<[f32; 3]>, Vec<f32>, [Option<Raccord>; 2]) {
    let mut p = deporte(r, k);
    let tol = f32::from_bits(RACC_TOLERANCE.load(Ordering::Relaxed)).max(0.0).to_radians();
    // Une voie droite d'un bout à l'autre, déviée à ses deux bouts : depuis son milieu
    let essai = |fin: bool| raccourcir(r, k, fin, &mut p.clone(), tol, 0);
    let (arret0, arret1) = match (essai(false), essai(true)) {
        (Some(x), Some(y)) if x.entier && y.entier => {
            let m = p.len();
            let mi = if m >= 3 {
                m / 2
            } else {
                let (u, v) = (p[0], p[1]);
                p.insert(1, [(u[0] + v[0]) / 2.0, (u[1] + v[1]) / 2.0, (u[2] + v[2]) / 2.0]);
                1
            };
            (p.len() - 1 - mi, 1)
        }
        _ => (0, 0),
    };
    let mut bouts = [None, None];
    bouts[0] = raccourcir(r, k, false, &mut p, tol, arret0);
    // Au bout de la fin, ne pas dépasser la cassure du début (son nœud de file)
    let arret1 = if bouts[0].is_some() { arret1.max(1) } else { arret1 };
    bouts[1] = raccourcir(r, k, true, &mut p, tol, arret1);
    let w0 = r.aretes[k].largeur * 0.5;
    let mut w = vec![w0; p.len()];
    let m = p.len();
    // La largeur passe en biseau le long de la courbe, de la cassure au nœud
    for (i, b) in bouts.iter().enumerate() {
        let Some(b) = b else { continue };
        let n = b.points.min(m - 1);
        for s in 1..=n {
            let t = s as f32 / n as f32;
            let j = if i == 0 { n - s } else { m - 1 - n + s };
            w[j] = w0 + (b.demi - w0) * t;
        }
    }
    (p, w, bouts)
}

static RAYON_D: AtomicU32 = AtomicU32::new(0x4090_0000); // 4.5
static RAYON_G: AtomicU32 = AtomicU32::new(0x4090_0000); // 4.5

/// Règle les rayons des virages à droite et à gauche (m), réglés dans la surimpression.
pub fn regler_rayons(droite: f32, gauche: f32) {
    RAYON_D.store(droite.to_bits(), Ordering::Relaxed);
    RAYON_G.store(gauche.to_bits(), Ordering::Relaxed);
}

fn rayon(a_gauche: bool) -> f32 {
    f32::from_bits(if a_gauche { &RAYON_G } else { &RAYON_D }.load(Ordering::Relaxed))
}
const VIRAGE_LOIN: f32 = 30.0;
/// Un virage déjà assuré (`deja_assure`) : bifurcation cherchée à moins de tant de
/// mètres en amont, la voie de destination rejointe à moins de tant en aval.
const AMONT: f32 = 60.0;
const AVAL: f32 = 60.0;
/// Un axe prioritaire dont deux files qui se suivent sont décalées de plus de tant (m)
/// ne l'est plus.
const DECALE_MAX: f32 = 0.5;
/// Un élargissement regarde la bifurcation suivante jusqu'à tant de mètres.
const ELARGI: f32 = 80.0;
/// La longueur de la partie fondue : une base, plus autant de fois l'écart à
/// rattraper, bornée (m).
const FONTE_BASE: f32 = 12.0;
const FONTE_ECART: f32 = 8.0;
const FONTE_MAX: f32 = 90.0;
/// Bout à bout, dans une fourche : la file rejoint l'autre sur tant de fois l'écart (m),
/// entre un minimum et un maximum.
const BOUT_ECART: f32 = 2.0;
const BOUT_MIN: f32 = 2.0;
const BOUT_MAX: f32 = 8.0;
/// Les nœuds reliés par une voie plus courte (m) ne font qu'un carrefour ; au plus
/// tant de nœuds par carrefour.
const SERRE: f32 = 12.0;
const GROUPE_MAX: usize = 24;
/// Le pas des points de la partie fondue (m).
const FONTE_PAS: f32 = 1.0;
/// Une voie courte et presque droite : ses files vont en ligne droite d'un nœud de
/// files à l'autre (longueur au plus, écart de l'axe à sa corde au plus, m).
const DROIT_LONGUEUR: f32 = 60.0;
const DROIT_ECART: f32 = 3.0;
/// Le trait d'un nœud de files : de tant sous la file à tant au-dessus (m).
pub(crate) const MARQUE_BAS: f32 = 0.5;
pub(crate) const MARQUE_HAUT: f32 = 3.0;

/// Un fil : ses points ; s'il est la partie fondue qui prolonge une autre file, un
/// virage d'une voie à une autre, ou une file réservée au bus.
pub struct Fil {
    pub points: Vec<[f32; 3]>,
    pub raccord: bool,
    pub virage: bool,
    pub bus: bool,
    /// Un nœud de files : un trait vertical, là où une file arrive à un nœud.
    pub marque: bool,
    /// Une continuité dans un carrefour : de la ligne d'arrêt d'une voie à celle d'une
    /// autre, tout droit.
    pub traverse: bool,
    /// Le sens de circulation : 1 dans l'ordre des points, −1 à rebours, 0 les deux ou
    /// sans objet. Un cône au milieu du fil le montre.
    pub sens: i8,
}

/// Un bout d'arête à un nœud.
#[derive(Clone, Copy)]
struct Bout {
    arete: usize,
    /// Le bout est le début de l'arête (le tracé part du nœud).
    debut: bool,
}

/// Le bout d'une file qui en prolonge une autre : le point de départ (le bout de
/// l'autre) et la direction de l'autre en s'éloignant du nœud.
#[derive(Clone, Copy)]
struct Fonte {
    p: [f32; 3],
    o: [f32; 2],
    /// Bout à bout (une fourche, une jonction) : la file rejoint l'autre au plus court.
    court: bool,
}

/// (arête, file, début) → sa fonte.
type Fontes = HashMap<(usize, usize, bool), Fonte>;
/// (arête, début) → le recul de ses files au nœud.
/// (arête, début, file) → le recul de cette file (`usize::MAX` : toutes ses files).
type Reculs = HashMap<(usize, bool, usize), f32>;

fn longueur(p: &[[f32; 3]]) -> f32 {
    p.windows(2).map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1])).sum()
}

/// La file `i` de l'arête `k` est réservée au bus : marquée dans OSM, ou sur une voie
/// de bus (le TEOR, en site propre).
fn est_bus(r: &Reseau, k: usize, i: usize) -> bool {
    let a = &r.aretes[k];
    a.genre == sage_reel::BUS || a.files.est_bus(i)
}

fn carrossable(r: &Reseau, k: usize) -> bool {
    let a = &r.aretes[k];
    // Une amorce (le sommet d'une épingle vers une passerelle) n'est ni une voie des
    // fils ni un morceau de carrefour
    a.files.total > 0 && a.points.len() >= 2 && !est_amorce(r, k)
}

/// Les bouts carrossables du nœud `n`.
fn bouts(r: &Reseau, n: usize) -> Vec<Bout> {
    let g = groupe(r, n);
    let mut v = Vec::new();
    let mut vues = std::collections::HashSet::new();
    for &m in &g {
        for &k in &r.noeuds[m].aretes {
            if !carrossable(r, k) || courte(r, k) || !vues.insert(k) {
                continue;
            }
            let a = &r.aretes[k];
            // Ses deux bouts dans le groupe : une voie interne du carrefour, pas une entrée
            if a.a != a.b && g.contains(&a.a) && g.contains(&a.b) {
                continue;
            }
            if g.contains(&a.a) {
                v.push(Bout { arete: k, debut: true });
            }
            if g.contains(&a.b) {
                v.push(Bout { arete: k, debut: false });
            }
        }
    }
    v
}

/// Une voie carrossable si courte qu'elle n'est qu'un morceau de carrefour : ses deux
/// nœuds n'en font qu'un, elle n'a pas de files.
fn courte(r: &Reseau, k: usize) -> bool {
    carrossable(r, k) && !guidee(r, k) && !sens_unique(r, k) && longueur(&r.aretes[k].points) < SERRE
}

/// Une voie à sens unique (une chaussée séparée) : elle relie deux carrefours
/// distincts, même courte, au lieu de n'en faire qu'un.
fn sens_unique(r: &Reseau, k: usize) -> bool {
    let f = r.aretes[k].files;
    (f.arriere == 0 && f.avant == f.total) || (f.avant == 0 && f.arriere == f.total)
}

/// Une voie de bus en site propre (le TEOR) : comme un tramway, ses files suivent
/// toujours son tracé (ni ligne droite, ni partie fondue, ni regroupement).
fn guidee(r: &Reseau, k: usize) -> bool {
    r.aretes[k].genre == sage_reel::BUS
}

/// Le groupe du nœud `n` : les nœuds reliés à lui par des voies courtes (`courte`),
/// triés ; lui seul s'il n'y en a pas.
fn groupe(r: &Reseau, n: usize) -> Vec<usize> {
    let mut g = vec![n];
    let mut i = 0;
    while i < g.len() && g.len() < GROUPE_MAX {
        let m = g[i];
        for &k in &r.noeuds[m].aretes {
            if courte(r, k) {
                let a = &r.aretes[k];
                for o in [a.a, a.b] {
                    if !g.contains(&o) {
                        g.push(o);
                    }
                }
            }
        }
        i += 1;
    }
    g.sort_unstable();
    g
}

/// L'axe décalé de `y` à gauche du tracé (normale (dz, −dx), z vers le sud), avec
/// des onglets bornés (comme les lignes des files).
fn decaler(pts: &[[f32; 3]], y: f32) -> Vec<[f32; 3]> {
    crate::files::decaler(pts, y)
}

/// Le point de la polyligne `p` à `s` mètres de son début.
fn point_a(p: &[[f32; 3]], s: f32) -> [f32; 3] {
    let mut a = 0.0f32;
    for w in p.windows(2) {
        let l = (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]);
        if a + l >= s {
            let u = if l > 1e-6 { ((s - a) / l).clamp(0.0, 1.0) } else { 0.0 };
            return [w[0][0] + (w[1][0] - w[0][0]) * u, w[0][1] + (w[1][1] - w[0][1]) * u, w[0][2] + (w[1][2] - w[0][2]) * u];
        }
        a += l;
    }
    p[p.len() - 1]
}

/// La polyligne `p` sans ses `d0` premiers mètres ni ses `d1` derniers.
fn rogner(p: &[[f32; 3]], d0: f32, d1: f32) -> Vec<[f32; 3]> {
    let total = longueur(p);
    if total <= 1e-3 || d0 + d1 >= total {
        return Vec::new();
    }
    if d0 <= 0.0 && d1 <= 0.0 {
        return p.to_vec();
    }
    let (s0, s1) = (d0, total - d1);
    let mut v = vec![point_a(p, s0)];
    let mut s = 0.0f32;
    for w in p.windows(2) {
        s += (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]);
        if s > s0 && s < s1 {
            v.push(w[1]);
        }
    }
    v.push(point_a(p, s1));
    v.dedup_by(|a, b| (a[0] - b[0]).hypot(a[1] - b[1]) < 1e-4);
    if v.len() < 2 {
        return Vec::new();
    }
    v
}

/// Le décalage (à gauche du tracé) de la file `i` de l'arête `k` (0 : la plus à gauche).
fn decalage(r: &Reseau, k: usize, i: usize) -> f32 {
    crate::files::decalage(&r.aretes[k], i)
}

/// L'axe brut de la file `i` de l'arête `k`, dans le sens du tracé.
fn axe_file(r: &Reseau, k: usize, i: usize) -> Vec<[f32; 3]> {
    decaler(&trace(r, k), decalage(r, k, i))
}

/// Les files d'un bout : celles qui arrivent au nœud et celles qui en partent.
/// Les files `0..arriere` vont à contresens du tracé, les `total − avant..total`
/// dans son sens, celles du milieu dans les deux.
fn files_du_bout(r: &Reseau, b: Bout) -> (Vec<usize>, Vec<usize>) {
    let f = r.aretes[b.arete].files;
    let n = f.total as usize;
    let ar = (f.arriere as usize).min(n);
    let av = (f.avant as usize).min(n - ar);
    let contresens: Vec<usize> = (0..n - av).collect();
    let sens: Vec<usize> = (ar..n).collect();
    // Au début, le tracé part du nœud : ses files dans le sens du tracé en partent
    if b.debut {
        (contresens, sens)
    } else {
        (sens, contresens)
    }
}

/// Le bout de la polyligne au nœud : le point, et la direction en s'éloignant du nœud.
fn extremite(fil: &[[f32; 3]], debut: bool) -> Option<([f32; 3], [f32; 2])> {
    if fil.len() < 2 {
        return None;
    }
    let (p, q) = if debut { (fil[0], fil[1]) } else { (fil[fil.len() - 1], fil[fil.len() - 2]) };
    let (dx, dz) = (q[0] - p[0], q[1] - p[1]);
    let l = dx.hypot(dz);
    (l > 1e-6).then_some((p, [dx / l, dz / l]))
}

/// La direction d'un bout en s'éloignant du nœud, sur son axe (vers le point à
/// quelques largeurs, ou au milieu de l'arête).
fn direction(r: &Reseau, b: Bout) -> Option<[f32; 2]> {
    let a = &r.aretes[b.arete];
    let d = (2.0 * a.largeur).clamp(5.0, 30.0).min(0.5 * longueur(&a.points));
    let t = trace(r, b.arete);
    let p: Vec<[f32; 3]> = if b.debut { t } else { t.into_iter().rev().collect() };
    let q = point_a(&p, d);
    let (dx, dz) = (q[0] - p[0][0], q[1] - p[0][1]);
    let l = dx.hypot(dz);
    (l > 1e-6).then_some([dx / l, dz / l])
}

/// La gauche de la direction `d` (z vers le sud).
fn gauche(d: [f32; 2]) -> [f32; 2] {
    [d[1], -d[0]]
}

fn dot(a: [f32; 2], b: [f32; 2]) -> f32 {
    a[0] * b[0] + a[1] * b[1]
}

/// Une file à un nœud : laquelle, son bout, sa direction en s'éloignant du nœud.
struct Place {
    arete: usize,
    file: usize,
    debut: bool,
    p: [f32; 3],
    o: [f32; 2],
    bus: bool,
}

/// Les files arrivant (`arrivee`) ou partant d'un bout, de gauche à droite dans le
/// sens du déplacement.
fn places(r: &Reseau, b: Bout, arrivee: bool) -> Vec<Place> {
    let (arr, dep) = files_du_bout(r, b);
    let liste = if arrivee { arr } else { dep };
    let mut v: Vec<(f32, Place)> = liste
        .into_iter()
        .filter_map(|i| {
            let axe = axe_file(r, b.arete, i);
            let (p, o) = extremite(&axe, b.debut)?;
            // Le rang à gauche : par le décalage de la file, mesuré dans le sens du
            // déplacement (le tracé dans le sens du déplacement : décalage tel quel)
            let y = decalage(r, b.arete, i);
            let dans_le_sens = b.debut != arrivee;
            let rang = if dans_le_sens { y } else { -y };
            let bus = est_bus(r, b.arete, i);
            Some((rang, Place { arete: b.arete, file: i, debut: b.debut, p, o, bus }))
        })
        .collect();
    v.sort_by(|a, b| b.0.total_cmp(&a.0));
    v.into_iter().map(|x| x.1).collect()
}

/// Le nœud d'un bout.
fn noeud_du_bout(r: &Reseau, b: Bout) -> usize {
    let a = &r.aretes[b.arete];
    if b.debut { a.a } else { a.b }
}

/// Les voies qu'on peut prendre en partant du nœud `n` : (arête, autre nœud, longueur).
fn sorties(r: &Reseau, n: usize) -> Vec<(usize, usize, f32)> {
    r.noeuds[n]
        .aretes
        .iter()
        .filter_map(|&k| {
            let a = &r.aretes[k];
            if !carrossable(r, k) || a.a == a.b {
                return None;
            }
            let b = Bout { arete: k, debut: a.a == n };
            (!files_du_bout(r, b).1.is_empty()).then(|| (k, if a.a == n { a.b } else { a.a }, longueur(&a.points)))
        })
        .collect()
}

/// Les voies par lesquelles on arrive au nœud `n` : (arête, autre nœud, longueur).
fn entrees(r: &Reseau, n: usize) -> Vec<(usize, usize, f32)> {
    r.noeuds[n]
        .aretes
        .iter()
        .filter_map(|&k| {
            let a = &r.aretes[k];
            if !carrossable(r, k) || a.a == a.b {
                return None;
            }
            let b = Bout { arete: k, debut: a.a == n };
            (!files_du_bout(r, b).0.is_empty()).then(|| (k, if a.a == n { a.b } else { a.a }, longueur(&a.points)))
        })
        .collect()
}

/// Les nœuds atteints depuis `n` en roulant au plus `budget` mètres (sans `n` ni
/// `evite`), par l'arête `premiere` d'abord si elle est donnée.
fn en_aval(r: &Reseau, n: usize, premiere: Option<usize>, budget: f32, evite: usize) -> std::collections::HashSet<usize> {
    let mut vus = std::collections::HashSet::new();
    let mut pile: Vec<(usize, f32)> = Vec::new();
    for (k, m, l) in sorties(r, n) {
        if premiere.is_none_or(|p| p == k) && l <= budget {
            pile.push((m, budget - l));
        }
    }
    while let Some((m, reste)) = pile.pop() {
        if m == n || m == evite || !vus.insert(m) {
            continue;
        }
        for (_, o, l) in sorties(r, m) {
            if l <= reste {
                pile.push((o, reste - l));
            }
        }
    }
    vus
}

/// Le virage du bout `e` (où l'on arrive) vers le bout `d` (où l'on part) est déjà
/// assuré : en remontant `e` de moins de `AMONT` mètres, une bifurcation dont une autre
/// branche atteint, en moins de `AVAL` mètres, la voie de `d` en aval du nœud.
fn deja_assure(r: &Reseau, e: Bout, d: Bout) -> bool {
    let n = noeud_du_bout(r, e);
    // La voie de destination, courte : son autre bout `t`, et la voie qui la prolonge
    // au-delà (la plus alignée)
    let ad = &r.aretes[d.arete];
    if longueur(&ad.points) > AVAL {
        return false;
    }
    let t = if d.debut { ad.b } else { ad.a };
    if t == n {
        return false;
    }
    let Some(o) = direction(r, Bout { arete: d.arete, debut: !d.debut }) else { return false };
    let arrive = [-o[0], -o[1]];
    let suite = sorties(r, t)
        .into_iter()
        .filter(|x| x.0 != d.arete)
        .filter_map(|(k, _, _)| direction(r, Bout { arete: k, debut: r.aretes[k].a == t }).map(|ok| (k, dot(arrive, ok))))
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|x| x.0);
    // Remonter : (nœud, arête par laquelle on en repart vers n, distance faite)
    let mut pile: Vec<(usize, usize, f32)> = Vec::new();
    let a = &r.aretes[e.arete];
    let amont = if e.debut { a.b } else { a.a };
    pile.push((amont, e.arete, longueur(&a.points)));
    let mut vus = std::collections::HashSet::new();
    while let Some((v, venue, fait)) = pile.pop() {
        if fait > AMONT || !vus.insert(v) {
            continue;
        }
        for (k, _, _) in sorties(r, v) {
            if k == venue {
                continue;
            }
            // Une branche sœur qui rejoint `t` (sans repasser par le nœud du virage) par
            // une autre voie que la destination et que sa suite : elle s'insère dans la
            // destination ; arriver par la suite, c'est venir d'en face
            if arrivees_a(r, v, k, AVAL + fait, n, t).iter().any(|&j| j != d.arete && Some(j) != suite) {
                return true;
            }
        }
        for (k, o, l) in entrees(r, v) {
            pile.push((o, k, fait + l));
        }
    }
    false
}

/// Les voies par lesquelles on atteint `t` depuis `v` en partant par `k`, en roulant au
/// plus `budget` mètres, sans passer par `evite`.
fn arrivees_a(r: &Reseau, v: usize, k: usize, budget: f32, evite: usize, t: usize) -> Vec<usize> {
    let mut res = Vec::new();
    let a = &r.aretes[k];
    let l = longueur(&a.points);
    if l > budget {
        return res;
    }
    let m = if a.a == v { a.b } else { a.a };
    let mut pile = vec![(m, k, budget - l)];
    let mut vus = std::collections::HashSet::new();
    while let Some((x, par, reste)) = pile.pop() {
        if x == t {
            res.push(par);
            continue;
        }
        if x == evite || x == v || !vus.insert(x) {
            continue;
        }
        for (j, o, lj) in sorties(r, x) {
            if lj <= reste {
                pile.push((o, j, reste - lj));
            }
        }
    }
    res
}

/// Le côté où placer `x` files ajoutées sur la voie du bout `d` (où l'on part) : à
/// gauche (`Some(true)`) si, à son autre bout, à moins de `ELARGI` mètres, elle se
/// partage et que ses branches de gauche prennent au plus `x` files ; à droite
/// (`Some(false)`) pour celles de droite ; `None` sinon.
fn cote_ajout(r: &Reseau, d: Bout, x: usize) -> Option<bool> {
    let a = &r.aretes[d.arete];
    if longueur(&a.points) > ELARGI {
        return None;
    }
    let loin = Bout { arete: d.arete, debut: !d.debut };
    let o = direction(r, loin)?;
    // En arrivant au bout lointain : à rebours de sa direction depuis ce nœud
    let t = [-o[0], -o[1]];
    let n2 = noeud_du_bout(r, loin);
    // Les branches qui partent vers l'avant : (direction, files libres)
    let branches: Vec<([f32; 2], usize)> = bouts(r, n2)
        .into_iter()
        .filter(|b| b.arete != d.arete)
        .filter_map(|b| {
            let libres = places(r, b, false).iter().filter(|p| !p.bus).count();
            let ob = direction(r, b)?;
            (libres > 0 && dot(t, ob) >= 0.0).then_some((ob, libres))
        })
        .collect();
    if branches.len() < 2 {
        return None;
    }
    // La plus alignée continue ; les autres sont à sa gauche ou à sa droite
    let suite = branches.iter().max_by(|a, b| dot(t, a.0).total_cmp(&dot(t, b.0)))?.0;
    let (mut g, mut dr) = (0usize, 0usize);
    for &(ob, libres) in &branches {
        if ob == suite {
            continue;
        }
        if dot(ob, gauche(suite)) > 0.0 {
            g += libres;
        } else {
            dr += libres;
        }
    }
    if g > 0 && g <= x && dr == 0 {
        Some(true)
    } else if dr > 0 && dr <= x && g == 0 {
        Some(false)
    } else {
        None
    }
}

/// Le bout d'où vient une place.
fn bout_de(p: &Place) -> Bout {
    Bout { arete: p.arete, debut: p.debut }
}

/// La polyligne de la file `i` de l'arête du bout `b`, en s'éloignant du nœud.
fn depuis_noeud(r: &Reseau, b: Bout, i: usize) -> Vec<[f32; 3]> {
    let axe = axe_file(r, b.arete, i);
    if b.debut {
        axe
    } else {
        axe.into_iter().rev().collect()
    }
}

/// Le décalage d'une file à gauche de la direction qui s'éloigne du nœud.
fn decalage_depuis(r: &Reseau, b: Bout, i: usize) -> f32 {
    let y = decalage(r, b.arete, i);
    if b.debut {
        y
    } else {
        -y
    }
}

/// Le chemin de `u` à `w` par les voies internes (courtes) d'un carrefour regroupé,
/// décalé de `y` à gauche du sens de marche : sa polyligne, de `u` à `w`.
fn chemin_interne(r: &Reseau, u: usize, w: usize, y: f32) -> Option<Vec<[f32; 3]>> {
    let mut venu: HashMap<usize, (usize, usize)> = HashMap::new();
    let mut file = std::collections::VecDeque::from([u]);
    let mut vus = std::collections::HashSet::from([u]);
    while let Some(m) = file.pop_front() {
        if m == w {
            break;
        }
        if vus.len() > GROUPE_MAX {
            return None;
        }
        for &k in &r.noeuds[m].aretes {
            let a = &r.aretes[k];
            if !courte(r, k) || a.a == a.b {
                continue;
            }
            let o = if a.a == m { a.b } else { a.a };
            if vus.insert(o) {
                venu.insert(o, (m, k));
                file.push_back(o);
            }
        }
    }
    if !venu.contains_key(&w) {
        return None;
    }
    // Les voies, de `w` à `u`, puis remises dans l'ordre
    let mut etapes = Vec::new();
    let mut m = w;
    while m != u {
        let (p, k) = venu[&m];
        etapes.push((p, k));
        m = p;
    }
    etapes.reverse();
    let mut pts: Vec<[f32; 3]> = Vec::new();
    for (p, k) in etapes {
        let a = &r.aretes[k];
        let mut l = trace(r, k);
        if a.a != p {
            l.reverse();
        }
        let l = decaler(&l, y);
        if !pts.is_empty() {
            pts.pop();
        }
        pts.extend(l);
    }
    (pts.len() >= 2).then_some(pts)
}

/// La polyligne `c` suivie de `f` (le dernier point de `c` remplacé par le premier de `f`).
fn joindre(mut c: Vec<[f32; 3]>, f: Vec<[f32; 3]>) -> Vec<[f32; 3]> {
    c.pop();
    c.extend(f);
    c
}

/// Amène un bout de la polyligne en `cible`, le décalage s'effaçant sur `sur` mètres.
fn deplacer_bout(pts: &mut [[f32; 3]], debut: bool, cible: [f32; 3], sur: f32) {
    if pts.len() < 2 {
        return;
    }
    let n = pts.len();
    let b = if debut { 0 } else { n - 1 };
    let (dx, dz) = (cible[0] - pts[b][0], cible[1] - pts[b][1]);
    let mut l = 0.0;
    for j in 0..n {
        let k = if debut { j } else { n - 1 - j };
        if j > 0 {
            let q = if debut { k - 1 } else { k + 1 };
            l += (pts[k][0] - pts[q][0]).hypot(pts[k][1] - pts[q][1]);
        }
        let w = 1.0 - (l / sur).min(1.0);
        if w <= 0.0 {
            break;
        }
        let w = w * w * (3.0 - 2.0 * w);
        pts[k][0] += dx * w;
        pts[k][1] += dz * w;
    }
}

/// La voie au-delà de l'autre bout de la voie du bout `b` (la plus alignée) et sa file
/// qui prolonge la file `i` (au même décalage) : celle qui y arrive (`arrivee`, le
/// trafic vient de là) ou celle qui en part. Rend sa polyligne (en s'éloignant du nœud
/// lointain), son bout et sa file.
fn au_dela(r: &Reseau, b: Bout, i: usize, arrivee: bool) -> Option<(Vec<[f32; 3]>, Bout, usize)> {
    let loin = Bout { arete: b.arete, debut: !b.debut };
    let n2 = noeud_du_bout(r, loin);
    let o = direction(r, loin)?;
    let t = [-o[0], -o[1]];
    let y = decalage_depuis(r, b, i);
    let mut meilleur: Option<(Bout, usize, f32)> = None;
    for kb in bouts(r, n2) {
        if kb.arete == b.arete {
            continue;
        }
        let Some(ok) = direction(r, kb) else { continue };
        if dot(t, ok) < 0.7 {
            continue;
        }
        let (arr, dep) = files_du_bout(r, kb);
        for j in if arrivee { arr } else { dep } {
            let ecart = (decalage_depuis(r, kb, j) - y).abs();
            if meilleur.is_none_or(|m| ecart < m.2) {
                meilleur = Some((kb, j, ecart));
            }
        }
    }
    let (kb, j, _) = meilleur?;
    Some((depuis_noeud(r, kb, j), kb, j))
}

/// L'arc d'un virage de la file `fe` (en s'éloignant du nœud : on y arrive à
/// rebours) à la file `fd` (en s'éloignant du nœud), de rayon `rayon` : les deux files
/// prolongées en ligne droite depuis le nœud jusqu'à leur croisement X, l'arc tangent
/// aux deux. Rend ses points et ses deux points de raccordement ; rien si les files sont
/// presque parallèles ou si l'arc tomberait trop loin.
fn arc_virage(fe: &[[f32; 3]], fd: &[[f32; 3]], rayon: f32) -> Option<(Vec<[f32; 3]>, [f32; 3], [f32; 3])> {
    if fe.len() < 2 || fd.len() < 2 {
        return None;
    }
    // Les directions sur quelques mètres
    let dir = |p: &[[f32; 3]]| -> Option<[f32; 2]> {
        let q = point_a(p, (0.5 * longueur(p)).min(8.0));
        let (dx, dz) = (q[0] - p[0][0], q[1] - p[0][1]);
        let l = dx.hypot(dz);
        (l > 1e-3).then_some([dx / l, dz / l])
    };
    let (oe, od) = (dir(fe)?, dir(fd)?);
    let din = [-oe[0], -oe[1]];
    let (pa, pb) = (fe[0], fd[0]);
    // pa + s·din = pb + u·od
    let det = din[0] * (-od[1]) - din[1] * (-od[0]);
    if det.abs() < 0.05 {
        return None;
    }
    let (rx, rz) = (pb[0] - pa[0], pb[1] - pa[1]);
    let s = (rx * (-od[1]) - rz * (-od[0])) / det;
    let x = [pa[0] + din[0] * s, pa[1] + din[1] * s];
    let cos_phi = dot(din, od).clamp(-1.0, 1.0);
    let phi = cos_phi.acos();
    let t = rayon * (phi / 2.0).tan();
    let (a, b) = ([x[0] - din[0] * t, x[1] - din[1] * t], [x[0] + od[0] * t, x[1] + od[1] * t]);
    // Trop loin du nœud : non
    let loin = |q: [f32; 2], p: [f32; 3]| (q[0] - p[0]).hypot(q[1] - p[1]) > VIRAGE_LOIN;
    if loin(a, pa) || loin(b, pb) {
        return None;
    }
    let (ha, hb) = (point_a(fe, abscisse(fe, [a[0], a[1], 0.0]))[2], point_a(fd, abscisse(fd, [b[0], b[1], 0.0]))[2]);
    // L'arc en Bézier : des poignées de 4/3·tan(φ/4)·rayon
    let k = 4.0 / 3.0 * (phi / 4.0).tan() * rayon;
    let (p1, p2) = ([a[0] + din[0] * k, a[1] + din[1] * k], [b[0] - od[0] * k, b[1] - od[1] * k]);
    let n = 12;
    let pts = (0..=n)
        .map(|j| {
            let t = j as f32 / n as f32;
            let u = 1.0 - t;
            let (c0, c1, c2, c3) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            [c0 * a[0] + c1 * p1[0] + c2 * p2[0] + c3 * b[0], c0 * a[1] + c1 * p1[1] + c2 * p2[1] + c3 * b[1], ha + (hb - ha) * t]
        })
        .collect();
    Some((pts, [a[0], a[1], ha], [b[0], b[1], hb]))
}

/// L'abscisse (m depuis le début de `p`) du point de `p` le plus proche de `q`.
fn abscisse(p: &[[f32; 3]], q: [f32; 3]) -> f32 {
    let (mut meilleur, mut s_meilleur, mut s) = (f32::MAX, 0.0f32, 0.0f32);
    for w in p.windows(2) {
        let (dx, dz) = (w[1][0] - w[0][0], w[1][1] - w[0][1]);
        let l2 = dx * dx + dz * dz;
        let l = l2.sqrt();
        let u = if l2 > 1e-9 { (((q[0] - w[0][0]) * dx + (q[1] - w[0][1]) * dz) / l2).clamp(0.0, 1.0) } else { 0.0 };
        let (x, z) = (w[0][0] + dx * u, w[0][1] + dz * u);
        let e = (x - q[0]).hypot(z - q[1]);
        if e < meilleur {
            meilleur = e;
            s_meilleur = s + u * l;
        }
        s += l;
    }
    s_meilleur
}

/// Le point de `p` à `s` m de son début, et la direction de `p` là.
fn point_dir(p: &[[f32; 3]], s: f32) -> ([f32; 3], [f32; 2]) {
    let a = point_a(p, s);
    let b = point_a(p, s + 1.0);
    let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
    let l = dx.hypot(dz);
    if l > 1e-6 {
        return (a, [dx / l, dz / l]);
    }
    let c = point_a(p, (s - 1.0).max(0.0));
    let (dx, dz) = (a[0] - c[0], a[1] - c[1]);
    let l = dx.hypot(dz).max(1e-6);
    (a, [dx / l, dz / l])
}

/// La courbe de `p` (direction `dp`, celle du déplacement) à `q` (direction `dq`).
fn courbe(p: [f32; 3], dp: [f32; 2], q: [f32; 3], dq: [f32; 2]) -> Vec<[f32; 3]> {
    let c = (q[0] - p[0]).hypot(q[1] - p[1]) / 2.5;
    let p1 = [p[0] + dp[0] * c, p[1] + dp[1] * c];
    let p2 = [q[0] - dq[0] * c, q[1] - dq[1] * c];
    let n = 12;
    (0..=n)
        .map(|j| {
            let t = j as f32 / n as f32;
            let u = 1.0 - t;
            let (a, b, cc, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            [a * p[0] + b * p1[0] + cc * p2[0] + d * q[0], a * p[1] + b * p1[1] + cc * p2[1] + d * q[1], p[2] + (q[2] - p[2]) * t]
        })
        .collect()
}

/// Les files d'arrivée `aa` (rangs dans le bout `e`, de gauche à droite) vers les
/// départs `ss` (de gauche à droite) : la file i couvre [i/k, (i+1)/k) et va aux
/// départs dont le milieu y tombe ; à aucun, au plus proche.
fn apparier(e: usize, aa: &[usize], ss: &[(usize, usize)], paires: &mut Vec<((usize, usize), (usize, usize))>) {
    let (k, m) = (aa.len(), ss.len());
    if k == 0 || m == 0 {
        return;
    }
    for (i, &ia) in aa.iter().enumerate() {
        let mut faits = 0;
        for (j, &q) in ss.iter().enumerate() {
            let c = (j as f32 + 0.5) / m as f32;
            if c >= i as f32 / k as f32 && c < (i + 1) as f32 / k as f32 {
                paires.push(((e, ia), q));
                faits += 1;
            }
        }
        if faits == 0 {
            let j = (((i as f32 + 0.5) / k as f32 * m as f32) as usize).min(m - 1);
            paires.push(((e, ia), ss[j]));
        }
    }
}

/// L'analyse du nœud `n` : les paires (file qui arrive, file qui part) des bouts qui se
/// continuent (à moins de `TOUT_DROIT`) et les fontes qui en découlent ; le recul des
/// bouts qui ne font que tourner ; les virages (de la file de droite hors bus pour
/// tourner à droite, de celle de gauche pour tourner à gauche, vers la même de l'autre
/// voie).
fn analyser(r: &Reseau, n: usize, fontes: &mut Fontes, reculs: &mut Reculs, virages: &mut Vec<Fil>, mut journal: Option<&mut Vec<String>>) {
    let bouts_ = bouts(r, n);
    if bouts_.len() < 2 {
        return;
    }
    // Les faisceaux : les bouts presque parallèles et côte à côte (les chaussées d'une
    // même approche, tracées à part dans OSM) ne font qu'une entrée, leurs files rangées
    // ensemble de gauche à droite
    let membres: Vec<Vec<Bout>> = bouts_.iter().map(|&b| vec![b]).collect();
    if membres.len() < 2 {
        return;
    }
    let bs: Vec<Bout> = membres.iter().map(|m| m[0]).collect();
    let dirs: Vec<Option<[f32; 2]>> = membres
        .iter()
        .map(|m| {
            let (mut x, mut z) = (0.0f32, 0.0f32);
            for &b in m {
                let o = direction(r, b)?;
                x += o[0];
                z += o[1];
            }
            let l = x.hypot(z);
            (l > 1e-3).then_some([x / l, z / l])
        })
        .collect();
    let cos_max = TOUT_DROIT.to_radians().cos();
    // Les destinations de chaque bout d'arrivée
    let cibles: Vec<Vec<usize>> = (0..bs.len())
        .map(|e| {
            (0..bs.len())
                .filter(|&d| d != e)
                .filter(|&d| {
                    if bs.len() == 2 {
                        return true;
                    }
                    match (dirs[e], dirs[d]) {
                        (Some(oe), Some(od)) => dot([-oe[0], -oe[1]], od) > cos_max,
                        _ => false,
                    }
                })
                .collect()
        })
        .collect();
    // Les places d'un faisceau, de gauche à droite dans le sens du déplacement
    let ranger = |f: usize, arrivee: bool| -> Vec<Place> {
        let mut v: Vec<Place> = membres[f].iter().flat_map(|&b| places(r, b, arrivee)).collect();
        if membres[f].len() > 1 {
            if let Some(o) = dirs[f] {
                let sens = if arrivee { [-o[0], -o[1]] } else { o };
                let g = gauche(sens);
                v.sort_by(|a, b| (b.p[0] * g[0] + b.p[1] * g[1]).total_cmp(&(a.p[0] * g[0] + a.p[1] * g[1])));
            }
        }
        v
    };
    let arrivees: Vec<Vec<Place>> = (0..bs.len()).map(|f| ranger(f, true)).collect();
    let departs: Vec<Vec<Place>> = (0..bs.len()).map(|f| ranger(f, false)).collect();
    // Les files d'un bout de départ sont partagées entre les bouts qui y vont, de
    // gauche à droite, au prorata de leurs files d'arrivée
    let mut parts: Vec<Vec<(usize, std::ops::Range<usize>)>> = vec![Vec::new(); bs.len()];
    for d in 0..bs.len() {
        let m = departs[d].len();
        let Some(od) = dirs[d] else { continue };
        let mut sources: Vec<usize> = (0..bs.len()).filter(|&e| cibles[e].contains(&d) && !arrivees[e].is_empty()).collect();
        if m == 0 || sources.is_empty() {
            continue;
        }
        // Une source à gauche du départ (vu dans son sens) : son bout est à gauche
        sources.sort_by(|&a, &b| {
            let ga = dirs[a].map_or(0.0, |o| dot(o, gauche(od)));
            let gb = dirs[b].map_or(0.0, |o| dot(o, gauche(od)));
            gb.total_cmp(&ga)
        });
        let total: usize = sources.iter().map(|&e| arrivees[e].len()).sum();
        // Une jonction plus large que le départ : une arrivée de même largeur s'y
        // prolonge file à file ; une plus étroite rejoint les files du bord, de son côté
        if total > m && sources.len() >= 2 && sources.iter().any(|&e| arrivees[e].len() == m) {
            let premiere = sources.iter().position(|&e| arrivees[e].len() == m).unwrap_or(0);
            for (rang, &e) in sources.iter().enumerate() {
                let k = arrivees[e].len().min(m);
                let r = if arrivees[e].len() == m { 0..m } else if rang < premiere { 0..k } else { m - k..m };
                parts[d].push((e, r));
            }
            continue;
        }
        let mut cumul = 0;
        for &e in &sources {
            let k = arrivees[e].len();
            let a = (cumul * m) as f32 / total as f32;
            let b = ((cumul + k) * m) as f32 / total as f32;
            let mut debut = a.round() as usize;
            let mut fin = b.round() as usize;
            if fin <= debut {
                // Au moins une file : celle au milieu de la part
                debut = (((a + b) * 0.5) as usize).min(m - 1);
                fin = debut + 1;
            }
            parts[d].push((e, debut..fin.min(m)));
            cumul += k;
        }
    }
    // Les paires : (bout d'arrivée, rang), (bout de départ, rang)
    let mut paires: Vec<((usize, usize), (usize, usize))> = Vec::new();
    for e in 0..bs.len() {
        let k = arrivees[e].len();
        if k == 0 {
            continue;
        }
        let Some(oe) = dirs[e] else { continue };
        let t = [-oe[0], -oe[1]];
        let mut ds: Vec<usize> = cibles[e].clone();
        ds.sort_by(|&a, &b| {
            let ga = dirs[a].map_or(0.0, |o| dot(o, gauche(t)));
            let gb = dirs[b].map_or(0.0, |o| dot(o, gauche(t)));
            gb.total_cmp(&ga)
        });
        let mut suite: Vec<(usize, usize)> = Vec::new();
        for &d in &ds {
            for (s, rg) in &parts[d] {
                if *s == e {
                    suite.extend(rg.clone().map(|j| (d, j)));
                }
            }
        }
        let m = suite.len();
        if m == 0 {
            continue;
        }
        // Les files de bus vers les files de bus, les autres vers les autres : une file de
        // bus sans suite de bus s'arrête, une qui commence n'a pas de source
        let (ab, ag): (Vec<usize>, Vec<usize>) = (0..k).partition(|&i| arrivees[e][i].bus);
        let (sb, sg): (Vec<(usize, usize)>, Vec<(usize, usize)>) = suite.iter().copied().partition(|q| departs[q.0][q.1].bus);
        // Une file de bus sans suite de bus rejoint la file ordinaire la plus proche (de
        // son côté) ; une qui commence part de la plus proche
        let rang_a = |i: usize| (i as f32 + 0.5) / k as f32;
        let pos_s: Vec<f32> = (0..m).map(|j| (j as f32 + 0.5) / m as f32).collect();
        let proche_s = |x: f32| -> Option<(usize, usize)> {
            (0..m).filter(|&j| !departs[suite[j].0][suite[j].1].bus).min_by(|&a, &b| (pos_s[a] - x).abs().total_cmp(&(pos_s[b] - x).abs())).map(|j| suite[j])
        };
        if sb.is_empty() && !sg.is_empty() {
            for &i in &ab {
                if let Some(q) = proche_s(rang_a(i)) {
                    paires.push(((e, i), q));
                }
            }
        }
        if ab.is_empty() && !ag.is_empty() {
            for (j, q) in suite.iter().enumerate() {
                if departs[q.0][q.1].bus {
                    let i = ag.iter().copied().min_by(|&a, &b| (rang_a(a) - pos_s[j]).abs().total_cmp(&(rang_a(b) - pos_s[j]).abs()));
                    if let Some(i) = i {
                        paires.push(((e, i), *q));
                    }
                }
            }
        }
        for (aa, ss) in [(ag, sg), (ab, sb)] {
            apparier(e, &aa, &ss, &mut paires);
        }
    }
    // Un élargissement (une voie qui continue avec plus de files) : les files ajoutées
    // du côté de la branche qui les prend à la prochaine bifurcation, si elle est proche
    if bs.len() == 2 {
        for e in 0..2 {
            let d = 1 - e;
            let (k, m) = (arrivees[e].len(), departs[d].len());
            if k == 0 || m <= k || !paires.iter().all(|(a, q)| a.0 != e || q.0 == d) {
                continue;
            }
            if arrivees[e].iter().any(|p| p.bus) || departs[d].iter().any(|p| p.bus) {
                continue;
            }
            let x = m - k;
            if cote_ajout(r, bs[d], x) == Some(true) {
                paires.retain(|(a, _)| a.0 != e);
                for j in 0..=x {
                    paires.push(((e, 0), (d, j)));
                }
                for i in 1..k {
                    paires.push(((e, i), (d, i + x)));
                }
                if let Some(jl) = journal.as_mut() {
                    jl.push(format!("élargissement de a{} : {x} file(s) ajoutée(s) à gauche (bifurcation proche)", bs[d].arete));
                }
            }
        }
    }
    // Une branche qui a exactement les files de l'arrivée (bus compris) : le tronc s'y
    // prolonge file à file ; les autres branches partent de sa file voisine de leur côté
    for e in 0..bs.len() {
        let k = arrivees[e].len();
        let Some(oe) = dirs[e] else { continue };
        let t = [-oe[0], -oe[1]];
        let mut ds: Vec<usize> = paires.iter().filter(|(a, _)| a.0 == e).map(|(_, q)| q.0).collect();
        ds.sort();
        ds.dedup();
        if ds.len() < 2 {
            continue;
        }
        let pareille = |d: usize| departs[d].len() == k && (0..k).all(|i| arrivees[e][i].bus == departs[d][i].bus);
        let Some(&dx) = ds.iter().filter(|&&d| pareille(d)).max_by(|&&a, &&b| dirs[a].map_or(-2.0, |o| dot(t, o)).total_cmp(&dirs[b].map_or(-2.0, |o| dot(t, o)))) else { continue };
        let Some(ox) = dirs[dx] else { continue };
        paires.retain(|(a, _)| a.0 != e);
        for i in 0..k {
            paires.push(((e, i), (dx, i)));
        }
        let libres: Vec<usize> = (0..k).filter(|&i| !arrivees[e][i].bus).collect();
        for &d in ds.iter().filter(|&&d| d != dx) {
            let Some(od) = dirs[d] else { continue };
            // À gauche de la branche prolongée : la file libre la plus à gauche
            let a_gauche = dot(od, gauche(ox)) > 0.0;
            let Some(&i) = (if a_gauche { libres.first() } else { libres.last() }) else { continue };
            for j in 0..departs[d].len() {
                if !departs[d][j].bus {
                    paires.push(((e, i), (d, j)));
                }
            }
        }
    }
    // Un vrai carrefour : une voie y croise une autre (on peut y tourner, entre
    // `TOUT_DROIT` et `VIRAGE_MAX`). Sinon (une fourche, une jonction), les files glissent
    let cos_virage = VIRAGE_MAX.to_radians().cos();
    let tourne = |e: usize, d: usize| -> bool {
        if d == e || cibles[e].contains(&d) || arrivees[e].is_empty() || departs[d].is_empty() {
            return false;
        }
        match (dirs[e], dirs[d]) {
            (Some(oe), Some(od)) => {
                let c = dot([-oe[0], -oe[1]], od);
                c <= cos_max && c >= cos_virage
            }
            _ => false,
        }
    };
    let carrefour = bs.len() >= 3 && (0..bs.len()).any(|e| (0..bs.len()).any(|d| tourne(e, d)));
    // Une fourche : la file qui borde la branche la plus alignée peut aussi y continuer
    if !carrefour {
        for e in 0..bs.len() {
            let Some(oe) = dirs[e] else { continue };
            let t = [-oe[0], -oe[1]];
            let mut ds: Vec<usize> = paires.iter().filter(|(a, _)| a.0 == e).map(|(_, q)| q.0).collect();
            ds.sort();
            ds.dedup();
            if ds.len() < 2 {
                continue;
            }
            let Some(&dx) = ds.iter().max_by(|&&a, &&b| dirs[a].map_or(-2.0, |o| dot(t, o)).total_cmp(&dirs[b].map_or(-2.0, |o| dot(t, o)))) else { continue };
            let vers = |d: usize| -> Vec<(usize, usize)> { paires.iter().filter(|(a, q)| a.0 == e && q.0 == d).map(|(a, q)| (a.1, q.1)).collect() };
            let alignee = vers(dx);
            let moyenne = alignee.iter().map(|x| x.0 as f32).sum::<f32>() / alignee.len() as f32;
            let mut ajouts = Vec::new();
            for &d in ds.iter().filter(|&&d| d != dx) {
                // Seulement une file qui se partage déjà dans sa branche (une file qui y
                // continue seule ne bifurque pas)
                let vd = vers(d);
                let Some(&(i, _)) = vd.iter().filter(|x| !alignee.iter().any(|y| y.0 == x.0) && vd.iter().filter(|y| y.0 == x.0).count() >= 2).min_by(|a, b| (a.0 as f32 - moyenne).abs().total_cmp(&(b.0 as f32 - moyenne).abs())) else { continue };
                let js = alignee.iter().map(|x| x.1);
                let j = if (i as f32) < moyenne { js.min() } else { js.max() };
                let Some(j) = j else { continue };
                if arrivees[e][i].bus == departs[dx][j].bus {
                    ajouts.push(((e, i), (dx, j)));
                }
            }
            paires.extend(ajouts);
        }
    }
    // L'axe prioritaire : une voie dont toutes les files continuent, une à une, dans
    // toutes celles d'une autre voie (le boulevard qui traverse) ; elle n'est pas
    // coupée, ses files se prolongent, les autres voies s'y raccrochent
    let mut tronc = vec![false; bs.len()];
    if carrefour {
        for e in 0..bs.len() {
            let k = arrivees[e].len();
            let de_e: Vec<&((usize, usize), (usize, usize))> = paires.iter().filter(|(a, _)| a.0 == e).collect();
            let Some(d) = de_e.first().map(|x| x.1 .0) else { continue };
            let m = departs[d].len();
            let vers_d = paires.iter().filter(|(_, q)| q.0 == d).count();
            if k > 0 && k == m && de_e.len() == k && vers_d == m && de_e.iter().all(|x| x.1 .0 == d) {
                let mut ai: Vec<usize> = de_e.iter().map(|x| x.0 .1).collect();
                let mut qj: Vec<usize> = de_e.iter().map(|x| x.1 .1).collect();
                ai.sort();
                ai.dedup();
                qj.sort();
                qj.dedup();
                if ai.len() == k && qj.len() == m {
                    tronc[e] = true;
                    tronc[d] = true;
                }
            }
        }
        // Un axe dont les files ne se suivent pas (décalées : il faudrait une fonte en
        // travers du carrefour) n'est pas prioritaire : ses files s'arrêtent à la ligne
        // d'arrêt, où s'accrochent les virages, et une continuité les relie
        for &(a, q) in &paires {
            // (deux nœuds d'un carrefour regroupé : la file suit les voies internes)
            if tronc[a.0] && tronc[q.0] && noeud_du_bout(r, bs[a.0]) == noeud_du_bout(r, bs[q.0]) {
                let (pa, pq) = (&arrivees[a.0][a.1], &departs[q.0][q.1]);
                // (seulement quand les deux voies n'ont pas le même nombre de files : une
                // voie qui change seulement de largeur garde son axe)
                let autre = r.aretes[pa.arete].files.total != r.aretes[pq.arete].files.total;
                if autre && (pa.p[0] - pq.p[0]).hypot(pa.p[1] - pq.p[1]) > DECALE_MAX {
                    tronc[a.0] = false;
                    tronc[q.0] = false;
                }
            }
        }
    }
    // Qui glisse vers qui
    let mut nb: HashMap<(bool, usize, usize), usize> = HashMap::new();
    for &(a, q) in &paires {
        *nb.entry((true, a.0, a.1)).or_default() += 1;
        *nb.entry((false, q.0, q.1)).or_default() += 1;
    }
    for &(a, q) in &paires {
        let (pa, pq) = (&arrivees[a.0][a.1], &departs[q.0][q.1]);
        let se_partage = nb[&(true, a.0, a.1)] > 1;
        let se_rejoignent = nb[&(false, q.0, q.1)] > 1;
        let depart_glisse = if se_partage {
            true
        } else if se_rejoignent {
            false
        } else {
            r.aretes[pa.arete].files.total >= r.aretes[pq.arete].files.total
        };
        let (glisse, garde) = if depart_glisse { (pq, pa) } else { (pa, pq) };
        if let Some(j) = journal.as_mut() {
            j.push(format!("a{}·f{} vers a{}·f{} ; glisse a{}·f{}", pa.arete, pa.file, pq.arete, pq.file, glisse.arete, glisse.file));
        }
        if !carrefour || (tronc[a.0] && tronc[q.0]) {
            // Un carrefour regroupé, les deux voies à des nœuds différents : la file passe
            // par les voies internes (sans fonte en travers du carrefour)
            let (na, nq) = (noeud_du_bout(r, bs[a.0]), noeud_du_bout(r, bs[q.0]));
            if na != nq {
                // (décalée comme la file d'arrivée au départ, comme celle de départ à la fin :
                // elle change de place en route, sans passer par l'axe des voies internes)
                let (ya, yq) = (-decalage_depuis(r, bout_de(pa), pa.file), decalage_depuis(r, bout_de(pq), pq.file));
                if let Some((ca, cq)) = chemin_interne(r, na, nq, ya).zip(chemin_interne(r, na, nq, yq)).filter(|(a, q)| a.len() == q.len()) {
                    let lt = longueur(&cq).max(1e-3);
                    let mut l = 0.0;
                    let c: Vec<[f32; 3]> = (0..cq.len())
                        .map(|j| {
                            if j > 0 {
                                l += (cq[j][0] - cq[j - 1][0]).hypot(cq[j][1] - cq[j - 1][1]);
                            }
                            let t = (l / lt).clamp(0.0, 1.0);
                            let t = t * t * (3.0 - 2.0 * t);
                            [ca[j][0] + (cq[j][0] - ca[j][0]) * t, ca[j][1] + (cq[j][1] - ca[j][1]) * t, ca[j][2] + (cq[j][2] - ca[j][2]) * t]
                        })
                        .collect();
                    let mut pts = vec![pa.p];
                    pts.extend_from_slice(&c[1..c.len() - 1]);
                    pts.push(pq.p);
                    let bus = pa.bus || pq.bus;
                    virages.push(Fil { points: pts, raccord: false, virage: false, bus, marque: false, traverse: false, sens: 1 });
                    continue;
                }
            }
            fontes.insert((glisse.arete, glisse.file, glisse.debut), Fonte { p: garde.p, o: garde.o, court: false });
        }
    }
    if !carrefour {
        return;
    }
    // Un carrefour : chaque voie a une ligne d'arrêt, où toutes ses files s'arrêtent ;
    // virages (bleu) et continuités (rouge) relient ces lignes. La ligne est au bord des
    // voies qui la croisent, ou plus loin au point de raccordement le plus éloigné de
    // ses virages
    let mut continue_ = vec![false; bs.len()];
    for &(a, q) in &paires {
        continue_[a.0] = true;
        continue_[q.0] = true;
    }
    let mut bord = vec![0.0f32; bs.len()];
    for e in 0..bs.len() {
        let croisent = (0..bs.len()).filter(|&d| tourne(e, d) || tourne(d, e));
        let w = croisent.map(|d| membres[d].iter().map(|b| r.aretes[b.arete].largeur).sum::<f32>() * 0.5).fold(0.0f32, f32::max);
        if w > 0.0 && !tronc[e] {
            bord[e] = w + VIRAGE_MARGE;
        }
    }
    // Les virages : leurs arcs d'abord, qui repoussent les lignes d'arrêt
    struct Virage {
        e: usize,
        d: usize,
        fe: Vec<[f32; 3]>,
        fd: Vec<[f32; 3]>,
        arc: Option<(Vec<[f32; 3]>, f32, f32)>,
        /// La longueur des voies internes du carrefour ajoutées devant `fe` ou `fd`.
        le: f32,
        ld: f32,
        /// (arête, début, file) des deux files, pour leur ligne d'arrêt.
        ke: (usize, bool, usize),
        kd: (usize, bool, usize),
        /// Le virage prolongé sur une voie voisine : où il quitte (ou rejoint) sa file.
        force_e: Option<f32>,
        force_d: Option<f32>,
    }
    let mut vs: Vec<Virage> = Vec::new();
    for e in 0..bs.len() {
        let Some(oe) = dirs[e] else { continue };
        let t = [-oe[0], -oe[1]];
        if arrivees[e].is_empty() {
            continue;
        }
        // Au plus un virage de chaque côté : vers la voie qui part le plus près de
        // l'équerre (de celles qui ont des files au départ)
        let mut meilleur: [Option<(usize, f32)>; 2] = [None, None];
        for d in 0..bs.len() {
            if d == e || cibles[e].contains(&d) || departs[d].is_empty() {
                continue;
            }
            let Some(od) = dirs[d] else { continue };
            let c = dot(t, od);
            if c > cos_max || c < cos_virage {
                continue;
            }
            let cote = usize::from(dot(od, gauche(t)) > 0.0);
            let ecart = (c.clamp(-1.0, 1.0).acos() - std::f32::consts::FRAC_PI_2).abs();
            if meilleur[cote].is_none_or(|(_, m)| ecart < m) {
                meilleur[cote] = Some((d, ecart));
            }
        }
        // Une voie qui ne fait que tourner : toutes ses files tournent (partagées entre
        // les deux côtés s'il y en a deux), chacune vers sa file
        let seule = !paires.iter().any(|(a, _)| a.0 == e);
        let libres_e: Vec<usize> = (0..arrivees[e].len()).filter(|&i| !arrivees[e][i].bus).collect();
        let deux = meilleur.iter().all(|m| m.is_some());
        for (cote, choix) in meilleur.into_iter().enumerate() {
            let Some((d, _)) = choix else { continue };
            let a_gauche = cote == 1;
            // Un virage déjà assuré par une branche sœur, partie de la même bifurcation en
            // amont, qui rejoint la voie de destination un peu plus loin : pas de virage
            if deja_assure(r, bs[e], bs[d]) {
                if let Some(jl) = journal.as_mut() {
                    jl.push(format!("virage de a{} vers a{} : déjà assuré par une branche voisine", bs[e].arete, bs[d].arete));
                }
                continue;
            }
            let libres_d: Vec<usize> = (0..departs[d].len()).filter(|&j| !departs[d][j].bus).collect();
            let mut liste: Vec<(usize, usize)> = Vec::new();
            if seule && libres_e.len() > 1 && !libres_d.is_empty() {
                let n = libres_e.len();
                let moitie = n.div_ceil(2);
                let mes: Vec<usize> = if !deux { libres_e.clone() } else if a_gauche { libres_e[..moitie].to_vec() } else { libres_e[n - moitie..].to_vec() };
                let m = libres_d.len();
                // Alignées du côté où l'on tourne ; les files en trop rejoignent la
                // dernière de ce côté
                for (r, &i) in mes.iter().enumerate() {
                    let j = if a_gauche { libres_d[r.min(m - 1)] } else { libres_d[(m as isize - (mes.len() - r) as isize).max(0) as usize] };
                    liste.push((i, j));
                }
            } else {
                // Hors bus si les deux voies en ont ; sinon de bus à bus (le TEOR qui tourne)
                let choisir = |v: &[Place], bus: bool| -> Option<usize> {
                    let libres: Vec<usize> = (0..v.len()).filter(|&i| v[i].bus == bus).collect();
                    if a_gauche { libres.first().copied() } else { libres.last().copied() }
                };
                let paire = match (choisir(&arrivees[e], false), choisir(&departs[d], false)) {
                    (Some(i), Some(j)) => Some((i, j)),
                    _ => choisir(&arrivees[e], true).zip(choisir(&departs[d], true)),
                };
                liste.extend(paire);
            }
            for (i, j) in liste {
            let (pa, pq) = (&arrivees[e][i], &departs[d][j]);
            let (mut fe, mut fd) = (depuis_noeud(r, bout_de(pa), pa.file), depuis_noeud(r, bout_de(pq), pq.file));
            // Un carrefour regroupé : la voie qui n'est pas prioritaire est prolongée par
            // les voies internes jusqu'au nœud de l'autre (le virage les emprunte)
            let (mut le, mut ld) = (0.0f32, 0.0f32);
            let (ne, nd) = (noeud_du_bout(r, bs[e]), noeud_du_bout(r, bs[d]));
            // (seulement si ce chemin va dans le sens du virage : ni à rebours de la voie
            // de départ, ni en arrière de l'arrivée ; sinon le virage en boucle)
            let dans_le_sens = |de: [f32; 3], vers: [f32; 3]| {
                let (dx, dz) = (vers[0] - de[0], vers[1] - de[1]);
                let l = dx.hypot(dz);
                if l < 1e-3 {
                    return false;
                }
                let u = [dx / l, dz / l];
                let (Some(oe), Some(od)) = (dirs[e], dirs[d]) else { return false };
                dot(u, od) > 0.0 && dot(u, [-oe[0], -oe[1]]) > -0.3
            };
            if ne != nd {
                if !tronc[d] || tronc[e] {
                    if let Some(c) = chemin_interne(r, ne, nd, decalage_depuis(r, bout_de(pq), pq.file)).filter(|c| dans_le_sens(c[0], c[c.len() - 1])) {
                        ld = longueur(&c);
                        fd = joindre(c, fd);
                    }
                } else if let Some(c) = chemin_interne(r, nd, ne, decalage_depuis(r, bout_de(pa), pa.file)).filter(|c| dans_le_sens(c[c.len() - 1], c[0])) {
                    le = longueur(&c);
                    fe = joindre(c, fe);
                }
            }
            let mut arc = arc_virage(&fe, &fd, rayon(a_gauche)).map(|(pts, a, b)| (pts, abscisse(&fe, a), abscisse(&fd, b)));
            // Le rayon d'abord : une voie trop courte pour que l'arc s'y raccorde (au-delà
            // de sa ligne d'arrêt la plus lointaine) est prolongée par la voie qui la
            // précède ou la suit ; le virage commence (ou finit) sur celle-ci, la file de la
            // voie courte s'efface
            let (mut force_e, mut force_d) = (None, None);
            for cote_e in [true, false] {
                let Some((_, sa, sb)) = arc.clone() else { break };
                let (p, s, l_ajout, t) = if cote_e { (pa, sa, le, tronc[e]) } else { (pq, sb, ld, tronc[d]) };
                let l0 = longueur(&r.aretes[p.arete].points);
                if t || l_ajout > 0.0 || s <= PART_MAX * l0 + 0.01 {
                    continue;
                }
                let Some((ext, kb, j)) = au_dela(r, bout_de(p), p.file, cote_e) else { continue };
                let base = if cote_e { fe.clone() } else { fd.clone() };
                let lf = longueur(&base);
                let long = joindre(base, ext);
                let nouvel = if cote_e { arc_virage(&long, &fd, rayon(a_gauche)).map(|(pts, a, b)| (pts, abscisse(&long, a), abscisse(&fd, b))) } else { arc_virage(&fe, &long, rayon(a_gauche)).map(|(pts, a, b)| (pts, abscisse(&fe, a), abscisse(&long, b))) };
                let Some(nv) = nouvel else { continue };
                let s2 = if cote_e { nv.1 } else { nv.2 };
                if s2 <= lf {
                    continue;
                }
                let plus = |reculs: &mut Reculs, k: (usize, bool, usize), v: f32| {
                    reculs.entry(k).and_modify(|x| *x = x.max(v)).or_insert(v);
                };
                // La file de la voie d'à côté est coupée au départ du virage ; un bout de
                // file en reprend la fin jusqu'à la voie courte (on ne branche pas une
                // file de côté)
                plus(reculs, (kb.arete, kb.debut, j), s2 - lf);
                let morceau = troncon(&long, lf, s2);
                let morceau: Vec<[f32; 3]> = if cote_e { morceau.into_iter().rev().collect() } else { morceau };
                if morceau.len() >= 2 {
                    virages.push(Fil { points: morceau, raccord: false, virage: false, bus: p.bus, marque: false, traverse: false, sens: 1 });
                }
                if cote_e {
                    fe = long;
                    force_e = Some(s2);
                } else {
                    fd = long;
                    force_d = Some(s2);
                }
                arc = Some(nv);
                if let Some(jl) = journal.as_mut() {
                    jl.push(format!("virage prolongé sur a{}·f{} (a{} trop courte pour le rayon)", kb.arete, j, p.arete));
                }
            }
            if let Some((_, sa, sb)) = &arc {
                if !tronc[e] {
                    bord[e] = bord[e].max(*sa - le);
                }
                if !tronc[d] {
                    bord[d] = bord[d].max(*sb - ld);
                }
            }
            if let Some(jl) = journal.as_mut() {
                jl.push(format!("virage à {} : a{}·f{} vers a{}·f{}", if a_gauche { "gauche" } else { "droite" }, pa.arete, pa.file, pq.arete, pq.file));
            }
            vs.push(Virage { e, d, fe, fd, arc, le, ld, ke: (pa.arete, pa.debut, pa.file), kd: (pq.arete, pq.debut, pq.file), force_e, force_d });
            }
        }
    }
    for e in 0..bs.len() {
        let l = membres[e].iter().map(|b| longueur(&r.aretes[b.arete].points)).fold(f32::MAX, f32::min);
        bord[e] = bord[e].min(PART_MAX * l);
        for b in &membres[e] {
            reculs.insert((b.arete, b.debut, usize::MAX), bord[e]);
        }
        // La ligne d'arrêt parallèle à la voie traversée (la plus perpendiculaire) : chaque
        // file s'arrête un peu plus ou moins loin selon son décalage
        if !tronc[e] && bord[e] > 0.0 {
            if let Some(o) = dirs[e] {
                let n = gauche(o);
                let c = (0..bs.len()).filter(|&x| x != e).filter_map(|x| dirs[x]).min_by(|a, b| dot(*a, o).abs().total_cmp(&dot(*b, o).abs()));
                if let Some(c) = c {
                    let (ca, cb) = (dot(c, o), dot(c, n));
                    if cb.abs() > 0.3 {
                        let k = (ca / cb).clamp(-2.0, 2.0);
                        for b in &membres[e] {
                            let nf = r.aretes[b.arete].files.total as usize;
                            let ys: Vec<f32> = (0..nf).map(|i| decalage_depuis(r, *b, i) * k).collect();
                            let m = ys.iter().copied().fold(f32::MAX, f32::min);
                            let lmax = PART_MAX * longueur(&r.aretes[b.arete].points);
                            for (i, y) in ys.into_iter().enumerate() {
                                let v = (bord[e] + y - m).min(lmax);
                                reculs.entry((b.arete, b.debut, i)).and_modify(|x| *x = x.max(v)).or_insert(v);
                            }
                        }
                    }
                }
            }
        }
        if let Some(jl) = journal.as_mut() {
            let noms: Vec<String> = membres[e].iter().map(|b| format!("a{}", b.arete)).collect();
            jl.push(format!("ligne d'arrêt de {} à {:.1} m", noms.join(" + "), bord[e]));
        }
    }
    let marque = |n: [f32; 3]| Fil { points: vec![[n[0], n[1], n[2] - MARQUE_BAS], [n[0], n[1], n[2] + MARQUE_HAUT]], raccord: false, virage: false, bus: false, marque: true, traverse: false, sens: 0 };
    // Les virages, de ligne d'arrêt à ligne d'arrêt : le long de la file jusqu'à l'arc,
    // l'arc, puis le long de l'autre file
    for v in &vs {
        let arret = |k: (usize, bool, usize), e: usize| reculs.get(&k).copied().unwrap_or(bord[e]);
        let (mut se, mut sd) = (arret(v.ke, v.e).min(PART_MAX * (longueur(&v.fe) - v.le)) + v.le, arret(v.kd, v.d).min(PART_MAX * (longueur(&v.fd) - v.ld)) + v.ld);
        if let Some(x) = v.force_e {
            se = x;
        }
        if let Some(x) = v.force_d {
            sd = x;
        }
        // Sur l'axe prioritaire, le virage se raccroche à la file là où l'arc la touche
        if let Some((_, sa, sb)) = &v.arc {
            if tronc[v.e] {
                se = *sa;
                virages.push(marque(point_a(&v.fe, *sa)));
            }
            if tronc[v.d] {
                sd = *sb;
                virages.push(marque(point_a(&v.fd, *sb)));
            }
        }
        let pts = match &v.arc {
            Some((arc, sa, sb)) if *sa <= se + 0.01 && *sb <= sd + 0.01 => {
                let mut pts: Vec<[f32; 3]> = troncon(&v.fe, *sa, se).into_iter().rev().collect();
                pts.extend(arc.iter().copied());
                pts.extend(troncon(&v.fd, *sb, sd));
                pts
            }
            _ => {
                let (a, da) = point_dir(&v.fe, se);
                let (b, db) = point_dir(&v.fd, sd);
                courbe(a, [-da[0], -da[1]], b, db)
            }
        };
        // Une file de l'axe prioritaire qui glisse (fonte) commence ou finit sur l'autre
        // file, pas à sa place : le virage s'y accroche, en s'y fondant sur quelques mètres
        let mut pts = pts;
        for (debut, k, t, s) in [(true, v.ke, tronc[v.e], se), (false, v.kd, tronc[v.d], sd)] {
            if t && s <= 0.01 {
                if let Some(f) = fontes.get(&(k.0, k.2, k.1)) {
                    deplacer_bout(&mut pts, debut, f.p, 3.0);
                }
            }
        }
        virages.push(Fil { points: pts, raccord: false, virage: true, bus: false, marque: false, traverse: false, sens: 1 });
    }
    // Les continuités : d'une ligne d'arrêt à l'autre
    for &(a, q) in &paires {
        if tronc[a.0] && tronc[q.0] {
            continue;
        }
        let (pa, pq) = (&arrivees[a.0][a.1], &departs[q.0][q.1]);
        let (fe, fd) = (depuis_noeud(r, bout_de(pa), pa.file), depuis_noeud(r, bout_de(pq), pq.file));
        let arret = |p: &Place, e: usize| reculs.get(&(p.arete, p.debut, p.file)).copied().unwrap_or(bord[e]);
        let (pe, de) = point_dir(&fe, arret(pa, a.0).min(PART_MAX * longueur(&fe)));
        let (pd, dd) = point_dir(&fd, arret(pq, q.0).min(PART_MAX * longueur(&fd)));
        let bus = pa.bus || pq.bus;
        virages.push(Fil { points: courbe(pe, [-de[0], -de[1]], pd, dd), raccord: false, virage: false, bus, marque: false, traverse: true, sens: 1 });
    }
}

/// La partie de `p` entre les abscisses `s0` et `s1` (m depuis son début, `s0` ≤ `s1`).
fn troncon(p: &[[f32; 3]], s0: f32, s1: f32) -> Vec<[f32; 3]> {
    let l = longueur(p);
    if s1 - s0 < 0.05 {
        return Vec::new();
    }
    rogner(p, s0, (l - s1).max(0.0))
}

/// La partie fondue d'une file au départ de `q` (orientée en s'éloignant du nœud) :
/// une ligne droite du bout de l'autre file (`f.p`) au point de `q` à `l` mètres de son
/// début, sans l'axe de la voie. La hauteur suit `q` au même avancement. Rend la partie
/// fondue et le reste de `q`.
fn fondre(q: &[[f32; 3]], f: Fonte, lmax: f32) -> (Vec<[f32; 3]>, Vec<[f32; 3]>) {
    let ecart = (q[0][0] - f.p[0]).hypot(q[0][1] - f.p[1]);
    if ecart < 0.02 {
        return (Vec::new(), q.to_vec());
    }
    let l = if f.court { (BOUT_ECART * ecart).clamp(BOUT_MIN, BOUT_MAX) } else { (FONTE_BASE + FONTE_ECART * ecart).min(FONTE_MAX) }.min(lmax);
    if l < 0.5 {
        return (Vec::new(), q.to_vec());
    }
    let b = point_a(q, l);
    let pas = ((l / FONTE_PAS).ceil() as usize).max(2);
    let fondue: Vec<[f32; 3]> = (0..=pas)
        .map(|j| {
            let t = j as f32 / pas as f32;
            let h = point_a(q, l * t)[2];
            [f.p[0] + (b[0] - f.p[0]) * t, f.p[1] + (b[1] - f.p[1]) * t, h]
        })
        .collect();
    (fondue, rogner(q, l, 0.0))
}

/// Les morceaux du fil de la file `i` de l'arête `k` : reculé des grands nœuds, fondu
/// à ses bouts qui prolongent une autre file.
fn fil(r: &Reseau, k: usize, i: usize, fontes: &Fontes, reculs: &Reculs, sortie: &mut Vec<Fil>) {
    let axe = axe_file(r, k, i);
    let bus = est_bus(r, k, i);
    // Le sens de la file : dans le sens du tracé, à contresens, ou les deux
    let fl = r.aretes[k].files;
    let (n, ar) = (fl.total as usize, fl.arriere as usize);
    let av = (fl.avant as usize).min(n.saturating_sub(ar));
    let sens: i8 = if i >= ar.max(n - av) { 1 } else if i < ar.min(n - av) { -1 } else { 0 };
    let recul = |debut: bool| reculs.get(&(k, debut, i)).or_else(|| reculs.get(&(k, debut, usize::MAX))).copied().unwrap_or(0.0);
    let p = rogner(&axe, recul(true), recul(false));
    if p.len() < 2 {
        return;
    }
    // Une voie guidée suit son tracé, d'un bout à l'autre
    if guidee(r, k) {
        for n in [p[0], p[p.len() - 1]] {
            sortie.push(Fil { points: vec![[n[0], n[1], n[2] - MARQUE_BAS], [n[0], n[1], n[2] + MARQUE_HAUT]], raccord: false, virage: false, bus: false, marque: true, traverse: false, sens: 0 });
        }
        sortie.push(Fil { points: p, raccord: false, virage: false, bus, marque: false, traverse: false, sens });
        return;
    }
    // Les nœuds de files à ses bouts : le bout de l'autre file qu'elle prolonge, ou le sien
    // (un bout reculé, une ligne d'arrêt ou un virage prolongé, n'a pas de fonte)
    let fonte = |debut: bool| fontes.get(&(k, i, debut)).copied().filter(|_| recul(debut) <= 0.01);
    let n0 = fonte(true).map_or(p[0], |f| f.p);
    let n1 = fonte(false).map_or(p[p.len() - 1], |f| f.p);
    for n in [n0, n1] {
        sortie.push(Fil { points: vec![[n[0], n[1], n[2] - MARQUE_BAS], [n[0], n[1], n[2] + MARQUE_HAUT]], raccord: false, virage: false, bus: false, marque: true, traverse: false, sens: 0 });
    }
    // Une voie courte et presque droite : en ligne droite d'un nœud de files à l'autre
    // (une file sans partie qui glisse suit son tracé : rien à redresser)
    let glisse = fonte(true).is_some() || fonte(false).is_some();
    // (une rampe, entre murs : seulement si la droite reste sur sa chaussée ; ailleurs
    // toujours, les boulevards en dépendent)
    let t = trace(r, k);
    if glisse && droite(&t) && (!rampe(r, k) || sur_chaussee(&t, r.aretes[k].largeur * 0.5, n0, n1)) {
        let l = longueur(&p);
        let pas = ((l / 4.0).ceil() as usize).max(1);
        let pts = (0..=pas)
            .map(|j| {
                let t = j as f32 / pas as f32;
                [n0[0] + (n1[0] - n0[0]) * t, n0[1] + (n1[1] - n0[1]) * t, point_a(&p, l * t)[2]]
            })
            .collect();
        sortie.push(Fil { points: pts, raccord: false, virage: false, bus, marque: false, traverse: false, sens });
        return;
    }
    let lmax = PART_MAX * longueur(&p);
    let mut reste = p;
    if let Some(f) = fonte(true) {
        let (fondue, r2) = fondre(&reste, f, lmax);
        if fondue.len() >= 2 {
            sortie.push(Fil { points: fondue, raccord: true, virage: false, bus, marque: false, traverse: false, sens: 0 });
        }
        reste = r2;
    }
    if let Some(f) = fonte(false) {
        if reste.len() >= 2 {
            let envers: Vec<[f32; 3]> = reste.iter().rev().copied().collect();
            let (fondue, r2) = fondre(&envers, f, lmax);
            if fondue.len() >= 2 {
                sortie.push(Fil { points: fondue, raccord: true, virage: false, bus, marque: false, traverse: false, sens: 0 });
            }
            reste = r2.into_iter().rev().collect();
        }
    }
    if reste.len() >= 2 {
        sortie.push(Fil { points: reste, raccord: false, virage: false, bus, marque: false, traverse: false, sens });
    }
}

/// Une rampe ou un ouvrage : la voie est en tunnel ou sur un pont, ou elle touche à
/// l'un de ses bouts une voie qui l'est (une rampe d'accès, une tête de pont).
fn rampe(r: &Reseau, k: usize) -> bool {
    use sage_reel::reseau::Passage;
    let a = &r.aretes[k];
    a.passage != Passage::Sol || [a.a, a.b].iter().any(|&n| r.noeuds[n].aretes.iter().any(|&j| j != k && r.aretes[j].passage != Passage::Sol))
}

/// La corde de `n0` à `n1` reste sur la chaussée (à moins de `w` de l'axe `p`) sur
/// sa partie centrale (les bouts glissent vers une autre voie, ils peuvent en sortir).
fn sur_chaussee(p: &[[f32; 3]], w: f32, n0: [f32; 3], n1: [f32; 3]) -> bool {
    let loin = |q: [f32; 2]| {
        p.windows(2)
            .map(|s| {
                let (ax, az, bx, bz) = (s[0][0], s[0][1], s[1][0], s[1][1]);
                let (dx, dz) = (bx - ax, bz - az);
                let l2 = dx * dx + dz * dz;
                let t = if l2 > 1e-9 { (((q[0] - ax) * dx + (q[1] - az) * dz) / l2).clamp(0.0, 1.0) } else { 0.0 };
                (q[0] - ax - dx * t).hypot(q[1] - az - dz * t)
            })
            .fold(f32::MAX, f32::min)
    };
    (2..=8).all(|j| {
        let t = j as f32 / 10.0;
        loin([n0[0] + (n1[0] - n0[0]) * t, n0[1] + (n1[1] - n0[1]) * t]) <= w
    })
}

/// Une voie courte et presque droite : au plus `DROIT_LONGUEUR`, et son axe à moins de
/// `DROIT_ECART` de la corde entre ses bouts.
fn droite(p: &[[f32; 3]]) -> bool {
    if longueur(p) > DROIT_LONGUEUR {
        return false;
    }
    let (a, b) = (p[0], p[p.len() - 1]);
    let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
    let l = dx.hypot(dz);
    if l < 1e-3 {
        return false;
    }
    p.iter().all(|q| ((q[0] - a[0]) * dz - (q[1] - a[1]) * dx).abs() / l <= DROIT_ECART)
}

/// Les fils dans la boîte `b` (x0, z0, x1, z1) : ceux des arêtes qui la touchent.
pub fn dans(r: &Reseau, b: [f32; 4]) -> Vec<Fil> {
    let mut v = Vec::new();
    let mut fontes: Fontes = HashMap::new();
    let mut reculs: Reculs = HashMap::new();
    let mut virages = Vec::new();
    let mut vus: std::collections::HashSet<usize> = std::collections::HashSet::new();
    // Les voies qui touchent la boîte, puis leurs voisines (celles des mêmes nœuds) :
    // le lissage d'un fil dépend de ses voisins, la même voie se lisse alors de la même
    // façon dans chaque tuile (pas de rupture au bord)
    let touche = |a: &sage_reel::reseau::Arete| {
        let w = a.largeur * 0.5;
        let (x0, z0, x1, z1) = a.points.iter().fold((f32::MAX, f32::MAX, f32::MIN, f32::MIN), |m, p| (m.0.min(p[0]), m.1.min(p[1]), m.2.max(p[0]), m.3.max(p[1])));
        !(x1 + w < b[0] || x0 - w > b[2] || z1 + w < b[1] || z0 - w > b[3])
    };
    let premieres: Vec<usize> = (0..r.aretes.len()).filter(|&k| carrossable(r, k) && touche(&r.aretes[k])).collect();
    let mut prises: std::collections::BTreeSet<usize> = premieres.iter().copied().collect();
    if lisse() {
        for &k in &premieres {
            let a = &r.aretes[k];
            for n in [a.a, a.b] {
                for m in groupe(r, n) {
                    for &j in &r.noeuds[m].aretes {
                        if carrossable(r, j) {
                            prises.insert(j);
                        }
                    }
                }
            }
        }
    }
    for &k in &prises {
        let a = &r.aretes[k];
        for n in [a.a, a.b] {
            if !vus.contains(&n) {
                for m in groupe(r, n) {
                    vus.insert(m);
                }
                analyser(r, n, &mut fontes, &mut reculs, &mut virages, None);
            }
        }
    }
    // Les files, une fois tous les nœuds analysés (leurs reculs connus)
    for &k in &prises {
        let a = &r.aretes[k];
        if !vus.contains(&a.a) || !vus.contains(&a.b) || courte(r, k) {
            continue;
        }
        // Une voie interne d'un carrefour regroupé : ses liaisons la remplacent
        if a.a != a.b && groupe(r, a.a).contains(&a.b) {
            continue;
        }
        for i in 0..a.files.total as usize {
            fil(r, k, i, &fontes, &reculs, &mut v);
        }
    }
    v.extend(virages);
    if lisse() {
        lisser(&mut v);
    }
    v
}

fn nom_genre(g: u8) -> &'static str {
    match g {
        sage_reel::GRANDE => "grande",
        sage_reel::ROUTE => "route",
        sage_reel::SERVICE => "service",
        sage_reel::PIETONNE => "piétonne",
        sage_reel::CHEMIN => "chemin",
        sage_reel::SENTIER => "sentier",
        sage_reel::BUS => "bus",
        sage_reel::RUE_PIETONNE => "rue piétonne",
        _ => "?",
    }
}

/// Ce que l'on sait du nœud `n`, ligne à ligne : sa place, ses voies (bout, genre,
/// passage, largeur, files, cap en s'éloignant du nœud), les paires de files et qui glisse.
pub fn decrire(r: &Reseau, n: usize) -> Vec<String> {
    let Some(nd) = r.noeuds.get(n) else { return vec![format!("Nœud {n} : inconnu")] };
    let bs = bouts(r, n);
    let g = groupe(r, n);
    let mut v = vec![format!("Nœud {n} · x {:.0} z {:.0} · h {:.1} m · {} voies, {} bouts carrossables", nd.p[0], nd.p[1], nd.p[2], nd.aretes.len(), bs.len())];
    if g.len() > 1 {
        v.push(format!("Carrefour de {} nœuds (voies de moins de {SERRE:.0} m) : {}", g.len(), g.iter().map(|m| m.to_string()).collect::<Vec<_>>().join(" ")));
    }
    for b in &bs {
        let (k, debut) = (b.arete, b.debut);
        let a = &r.aretes[k];
        {
            let f = a.files;
            let cap = direction(r, Bout { arete: k, debut }).map_or("–".to_string(), |o| format!("{:.0}°", o[0].atan2(-o[1]).to_degrees().rem_euclid(360.0)));
            let sens = if debut { "part" } else { "arrive" };
            let bus: Vec<String> = (0..f.total as usize).filter(|&i| est_bus(r, k, i)).map(|i| format!("f{i}")).collect();
            let bus = if bus.is_empty() { String::new() } else { format!(" · bus {}", bus.join(" ")) };
            v.push(format!(
                "a{k} ({sens}) · {} · {} · {:.1} m · {} files ({} sens, {} contresens, {}){bus} · cap {cap} · {:.0} m",
                nom_genre(a.genre),
                a.passage.nom(),
                a.largeur,
                f.total,
                f.avant,
                f.arriere,
                if f.osm { "OSM" } else { "déduites" },
                longueur(&a.points)
            ));
        }
    }
    // Les raccords des branches de fourche : leurs nœuds de files
    for b in &bs {
        let (_, _, rc) = trace_raccords(r, b.arete);
        let Some(rc) = &rc[if b.debut { 0 } else { 1 }] else { continue };
        for nd in &rc.noeuds {
            v.push(format!(
                "a{}·f{} raccordée ({}) : nœud de file à la cassure ({:.1} m du nœud), vers a{}·f{}",
                b.arete, nd.rang, rc.regle, nd.distance, nd.tronc, nd.rang_tronc
            ));
        }
    }
    if bs.len() >= 2 {
        let (mut fontes, mut reculs, mut virages) = (HashMap::new(), HashMap::new(), Vec::new());
        let mut j = Vec::new();
        analyser(r, n, &mut fontes, &mut reculs, &mut virages, Some(&mut j));
        if j.is_empty() {
            v.push("Aucune paire de files".into());
        }
        v.extend(j);
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rogner_garde_le_milieu() {
        let p = [[0.0, 0.0, 0.0], [10.0, 0.0, 1.0]];
        let v = rogner(&p, 2.0, 3.0);
        assert_eq!(v.len(), 2);
        assert!((v[0][0] - 2.0).abs() < 1e-4 && (v[1][0] - 7.0).abs() < 1e-4);
        assert!(rogner(&p, 6.0, 5.0).is_empty());
    }
}

/// Au-delà de cet écart (°) entre deux voies bout à bout, leurs files tournent : les
/// intérieures se rejoignent où elles se coupent, les autres décrivent des arcs. En deçà,
/// leurs tracés sont presque parallèles.
const GIRATION: f32 = 3.0;

/// Une giration : deux voies bout à bout (seules de leur genre au nœud, ou qui se
/// prolongent : même disposition de files, alignées) qui font un angle. Les files
/// intérieures (du côté du virage) se rejoignent où elles se coupent ; les autres
/// suivent un arc tangent à leurs deux tracés, de rayon leur distance aux intérieures.
#[derive(Clone, Copy, Debug)]
pub struct Giration {
    /// Le nœud.
    o: [f32; 2],
    /// La direction de notre voie vers le nœud, et celle de l'autre en le quittant.
    u1: [f32; 2],
    u2: [f32; 2],
    /// La place en travers (à gauche de u1) des files intérieures, chez nous.
    t_in: f32,
    /// Le rapport des largeurs (l'autre voie sur la nôtre) : une place t chez nous est
    /// t × rapport chez elle.
    rapport: f32,
    /// Le virage tourne à gauche.
    gauche: bool,
    /// À travers un petit tronçon (plus court que la giration) : une polyligne tout
    /// entière dans le virage disparaît, et celle qui finit avant la coupe reste telle
    /// quelle (le tronçon voisin porte le coin ou l'arc).
    relais: bool,
    /// Un angle presque droit : pas d'arc, les bords et les files prolongés sans
    /// rétrécir jusqu'à se couper (un coin à l'intérieur comme à l'extérieur).
    droit: bool,
    /// Les places en travers des files de l'autre voie (au plus 8), et la plus grande
    /// des nôtres : à angle droit, une file (ou une ligne entre files) va jusqu'à la
    /// plus lointaine des files d'en face ; un bord, jusqu'au bord d'en face.
    cibles: [f32; 8],
    nc: u8,
    t_max: f32,
    /// La voie d'en face.
    pub voie: usize,
}

fn a_gauche(u: [f32; 2]) -> [f32; 2] {
    [-u[1], u[0]]
}

/// Les girations des deux bouts de l'arête `k`, d'axe `pts` (aucune à un bout raccordé).
pub fn girations(r: &Reseau, k: usize, pts: &[[f32; 3]], raccords: &[Option<Raccord>; 2]) -> [Option<Giration>; 2] {
    let mut out = [None, None];
    let a = &r.aretes[k];
    let m = pts.len();
    if m < 2 || a.files.total == 0 {
        return out;
    }
    let norm = |v: [f32; 2]| {
        let l = v[0].hypot(v[1]).max(1e-6);
        [v[0] / l, v[1] / l]
    };
    // Les files d'un bout qui entrent au nœud, et celles qui en sortent
    let sens = |b: &sage_reel::reseau::Arete, f: bool| {
        let (mut e, mut s_) = (0usize, 0usize);
        for i in 0..b.files.total as usize {
            match crate::files::sens(b, i) {
                crate::files::Sens::Avant => {
                    if f { e += 1 } else { s_ += 1 }
                }
                crate::files::Sens::Arriere => {
                    if f { s_ += 1 } else { e += 1 }
                }
                crate::files::Sens::Partagee => {}
            }
        }
        (e, s_)
    };
    for (idx, fin) in [(0usize, false), (1usize, true)] {
        if raccords[idx].is_some() {
            continue;
        }
        let n = if fin { a.b } else { a.a };
        let mut bouts: Vec<(usize, bool)> = Vec::new();
        for &j in &r.noeuds[n].aretes {
            let b = &r.aretes[j];
            // Seules comptent les voies de notre genre (une voie de service qui s'y
            // greffe ne l'empêche pas)
            if !sage_reel::carrossable(b.genre) || b.points.len() < 2 || b.files.total == 0 || b.genre != a.genre || (j != k && est_amorce(r, j)) {
                continue;
            }
            for f in [false, true] {
                if (if f { b.b } else { b.a }) == n {
                    bouts.push((j, f));
                }
            }
        }
        let (ie, ip) = if fin { (m - 1, m - 2) } else { (0, 1) };
        let u1 = norm([pts[ie][0] - pts[ip][0], pts[ie][1] - pts[ip][1]]);
        let depart = |j: usize, f: bool| -> Option<[f32; 2]> {
            let q = deporte(r, j);
            let mq = q.len();
            if mq < 2 {
                return None;
            }
            Some(if f { norm([q[mq - 2][0] - q[mq - 1][0], q[mq - 2][1] - q[mq - 1][1]]) } else { norm([q[1][0] - q[0][0], q[1][1] - q[0][1]]) })
        };
        let (ek, sk) = sens(a, fin);
        // Une boucle (ses deux bouts au même nœud, souvent une voie de service en
        // raquette) se raccorde à elle-même ; les autres voies n'y cherchent pas de suite
        let boucle = |j: usize| r.aretes[j].a == r.aretes[j].b;
        let autres: Vec<(usize, bool)> = bouts.iter().copied().filter(|&(j, f)| (j, f) != (k, fin) && !boucle(j)).collect();
        let partenaire = if boucle(k) {
            Some((k, !fin))
        } else if bouts.len() == 2 {
            // Bout à bout : même de largeurs différentes (le rapport des largeurs fait
            // correspondre les places en travers)
            autres.first().copied().filter(|&(j, _)| j != k)
        } else {
            // La voie qui nous prolonge : même disposition, alignée, et seule
            let v: Vec<(usize, bool)> = autres
                .iter()
                .copied()
                .filter(|&(j, f)| {
                    let b = &r.aretes[j];
                    let (ej, sj) = sens(b, f);
                    j != k && b.files.total == a.files.total && ej == sk && sj == ek && depart(j, f).is_some_and(|u2| (u1[0] * u2[0] + u1[1] * u2[1]) >= TOUT_DROIT.to_radians().cos())
                })
                .collect();
            if v.len() == 1 { Some(v[0]) } else { None }
        };
        // Les bouts carrossables de notre genre à un nœud
        let bouts_de = |n: usize| -> Vec<(usize, bool)> {
            let mut v = Vec::new();
            for &j in &r.noeuds[n].aretes {
                let b = &r.aretes[j];
                if !sage_reel::carrossable(b.genre) || b.points.len() < 2 || b.files.total == 0 || b.genre != a.genre || b.a == b.b {
                    continue;
                }
                for f in [false, true] {
                    if (if f { b.b } else { b.a }) == n {
                        v.push((j, f));
                    }
                }
            }
            v
        };
        let longueur = |j: usize| r.aretes[j].points.windows(2).map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1])).sum::<f32>();
        // Un petit tronçon `s` (pris à son bout `fs` au nœud `n`), seul entre deux
        // nœuds à deux bouts : la voie qui le prolonge à son autre nœud, alignée
        let au_dela = |s_: usize, fs: bool, u: [f32; 2]| -> Option<(usize, bool, usize)> {
            if longueur(s_) >= GIR_COURT {
                return None;
            }
            let n2 = if fs { r.aretes[s_].a } else { r.aretes[s_].b };
            let b2 = bouts_de(n2);
            if b2.len() != 2 {
                return None;
            }
            let (q, fq) = b2.into_iter().find(|&(q, _)| q != s_)?;
            let _ = u;
            Some((q, fq, n2))
        };
        // Le point à `d` m le long de la voie `j` depuis son bout `f` (déportée)
        let point_a = |j: usize, f: bool, d: f32| -> Option<[f32; 2]> {
            let q = deporte(r, j);
            if q.len() < 2 {
                return None;
            }
            let v: Vec<[f32; 2]> = if f { q.iter().rev().map(|p| [p[0], p[1]]).collect() } else { q.iter().map(|p| [p[0], p[1]]).collect() };
            let mut cumul = 0.0f32;
            for i in 1..v.len() {
                let sg = (v[i][0] - v[i - 1][0]).hypot(v[i][1] - v[i - 1][1]);
                if cumul + sg >= d && sg > 1e-6 {
                    let t = (d - cumul) / sg;
                    return Some([v[i - 1][0] + (v[i][0] - v[i - 1][0]) * t, v[i - 1][1] + (v[i][1] - v[i - 1][1]) * t]);
                }
                cumul += sg;
            }
            v.last().copied()
        };
        // Le relais : notre voie arrive à un petit tronçon aligné, au-delà duquel la
        // voie suivante tourne
        let mut relais = false;
        let mut o = [pts[ie][0], pts[ie][1]];
        let mut u1 = u1;
        // La direction de sortie, quand la voie d'en face n'est qu'un petit tronçon :
        // la corde jusqu'à la voie qui le prolonge (commune aux deux girations)
        let mut u2_chaine: Option<[f32; 2]> = None;
        let partenaire = match partenaire {
            Some((j, f)) => {
                // Nous sommes nous-mêmes le petit tronçon (la voie d'en face nous
                // prolonge à notre autre bout)
                let autre = if fin { a.a } else { a.b };
                relais = longueur(k) < GIR_COURT && bouts_de(autre).len() == 2;
                // Notre direction : la même corde que nos deux voisines (du nœud du
                // virage jusqu'à DIR_PORTEE sur la voie qui nous prolonge), pour que
                // les coins des trois tombent au même point
                if relais {
                    if let Some((q, fq)) = bouts_de(autre).into_iter().find(|&(q, _)| q != k) {
                        if let Some(pq) = point_a(q, fq, DIR_PORTEE) {
                            let d = [o[0] - pq[0], o[1] - pq[1]];
                            if d[0].hypot(d[1]) > 1.0 {
                                u1 = norm(d);
                            }
                        }
                    }
                }
                // La voie d'en face est un petit tronçon : la corde vers celle qui le
                // prolonge
                if bouts.len() == 2 {
                    if let Some((q, fq, _)) = au_dela(j, f, u1) {
                        if let Some(pq) = point_a(q, fq, DIR_PORTEE) {
                            let d = [pq[0] - o[0], pq[1] - o[1]];
                            if d[0].hypot(d[1]) > 1.0 {
                                u2_chaine = Some(norm(d));
                            }
                        }
                    }
                }
                Some((j, f))
            }
            None if bouts.len() == 2 => {
                let Some((s_, fs)) = autres.first().copied().filter(|&(j, _)| j != k) else { continue };
                match (depart(s_, fs), au_dela(s_, fs, u1)) {
                    (Some(us), Some((q, fq, n2))) if u1[0] * us[0] + u1[1] * us[1] >= TOUT_DROIT.to_radians().cos() => {
                        relais = true;
                        o = [r.noeuds[n2].p[0], r.noeuds[n2].p[1]];
                        // Notre direction d'arrivée : la corde depuis notre point à
                        // DIR_PORTEE jusqu'au nœud du virage (la même que la sienne)
                        if let Some(pk) = point_a(k, fin, DIR_PORTEE) {
                            let d = [o[0] - pk[0], o[1] - pk[1]];
                            if d[0].hypot(d[1]) > 1.0 {
                                u1 = norm(d);
                            }
                        }
                        Some((q, fq))
                    }
                    _ => None,
                }
            }
            None => None,
        };
        let Some((j, f)) = partenaire else { continue };
        let Some(u2) = u2_chaine.or_else(|| depart(j, f)) else { continue };
        let c = u1[0] * u2[0] + u1[1] * u2[1];
        if c > GIRATION.to_radians().cos() || c < -0.95 {
            continue;
        }
        let l1 = a_gauche(u1);
        let gauche = u1[0] * u2[1] - u1[1] * u2[0] > 0.0;
        // La place en travers des files (le décalage suit la normale du tracé)
        let g = crate::files::normales(pts);
        let sg = if g[ie][0] * l1[0] + g[ie][1] * l1[1] >= 0.0 { 1.0 } else { -1.0 };
        let ts = (0..a.files.total as usize).map(|i| crate::files::decalage(a, i) * sg);
        let t_in = if gauche { ts.fold(f32::MIN, f32::max) } else { ts.fold(f32::MAX, f32::min) };
        let rapport = r.aretes[j].largeur / a.largeur.max(0.1);
        let droit = c.abs() <= ANGLE_DROIT.to_radians().sin();
        let mut cibles = [0.0f32; 8];
        let bj = &r.aretes[j];
        let nc = (bj.files.total as usize).min(8);
        for (i, c_) in cibles.iter_mut().enumerate().take(nc) {
            *c_ = crate::files::decalage(bj, i);
        }
        let t_max = (0..a.files.total as usize).map(|i| crate::files::decalage(a, i).abs()).fold(0.0f32, f32::max);
        out[idx] = Some(Giration { o, u1, u2, t_in, rapport, gauche, relais, droit, cibles, nc: nc as u8, t_max, voie: j });
    }
    out
}

/// Fait tourner les bouts d'une polyligne de l'arête (une file, une ligne entre files,
/// un bord) à ses girations : le coin où elle coupe sa suite, ou son demi-arc.
pub fn girer(gs: &[Option<Giration>; 2], l: &mut Vec<[f32; 3]>) {
    for (idx, g) in gs.iter().enumerate() {
        let Some(g) = g else { continue };
        if l.len() < 2 {
            return;
        }
        if idx == 0 {
            l.reverse();
        }
        girer_fin(g, l);
        if idx == 0 {
            l.reverse();
        }
    }
}

fn girer_fin(g: &Giration, l: &mut Vec<[f32; 3]>) {
    let (l1, l2) = (a_gauche(g.u1), a_gauche(g.u2));
    let e = l[l.len() - 1];
    let z = e[2];
    let t1 = (e[0] - g.o[0]) * l1[0] + (e[1] - g.o[1]) * l1[1];
    let t2 = t1 * g.rapport;
    let sgn = if g.gauche { 1.0 } else { -1.0 };
    // Le rayon : la distance aux files intérieures, moyenne des deux voies
    let rho = (g.t_in - t1) * (1.0 + g.rapport) / 2.0 * sgn;
    let croix = |x: [f32; 2], y: [f32; 2]| x[0] * y[1] - x[1] * y[0];
    let det = croix(g.u1, g.u2);
    if det.abs() < 1e-4 {
        return;
    }
    // Le point où se coupent la parallèle à la place a chez nous et celle à la place b
    // chez l'autre
    let x = |a: f32, b: f32| {
        let p1 = [g.o[0] + a * l1[0], g.o[1] + a * l1[1]];
        let p2 = [g.o[0] + b * l2[0], g.o[1] + b * l2[1]];
        let s = croix([p2[0] - p1[0], p2[1] - p1[1]], g.u2) / det;
        [p1[0] + s * g.u1[0], p1[1] + s * g.u1[1]]
    };
    let proj = |p: [f32; 2]| (p[0] - g.o[0]) * g.u1[0] + (p[1] - g.o[1]) * g.u1[1];
    // On ne raccourcit que la fin de la polyligne (une boucle revient près du nœud par
    // son autre bout : ses points-là restent)
    // À travers un petit tronçon : la polyligne tout entière dans le virage disparaît ;
    // celle qui finit avant la coupe la laisse au tronçon voisin
    let p0 = proj([l[0][0], l[0][1]]);
    let pe = proj([l[l.len() - 1][0], l[l.len() - 1][1]]);
    let relais = |l: &mut Vec<[f32; 3]>, coupe: f32| -> bool {
        if !g.relais {
            return false;
        }
        if p0 >= coupe - 1e-3 {
            l.clear();
            return true;
        }
        pe < coupe - 1e-3
    };
    let garder = |l: &mut Vec<[f32; 3]>, coupe: f32| {
        while l.len() > 1 && proj([l[l.len() - 1][0], l[l.len() - 1][1]]) >= coupe - 1e-3 {
            l.pop();
        }
    };
    // Deux voies de largeurs différentes presque dans l'axe : leurs parallèles ne se
    // coupent que très loin (l'écart divisé par le sinus de l'angle). On les rejoint
    // alors au milieu de leurs deux bouts au nœud, en biseau.
    // À angle droit : prolongés sans rétrécir jusqu'à se couper ; un bord jusqu'au
    // bord d'en face, une file jusqu'à la plus lointaine des files d'en face
    if g.droit {
        let p = if t1.abs() > g.t_max + 0.2 || g.nc == 0 {
            x(t1, t2)
        } else {
            g.cibles[..g.nc as usize].iter().map(|&b| x(t1, b)).max_by(|a, b| proj(*a).total_cmp(&proj(*b))).unwrap_or_else(|| x(t1, t2))
        };
        if relais(l, proj(p)) {
            return;
        }
        garder(l, proj(p));
        l.push([p[0], p[1], z]);
        return;
    }
    let ecart = (t2 - t1).abs();
    if ecart > 0.05 && ecart / det.abs() > 1.0 {
        let m = [g.o[0] + (t1 * l1[0] + t2 * l2[0]) / 2.0, g.o[1] + (t1 * l1[1] + t2 * l2[1]) / 2.0];
        // Le bout au nœud est remplacé par le milieu (sinon un aller-retour en travers)
        if l.len() > 1 {
            l.pop();
        }
        garder(l, proj(m));
        l.push([m[0], m[1], z]);
        return;
    }
    if rho <= 0.01 || g.droit {
        let p = x(t1, t2);
        if relais(l, proj(p)) {
            return;
        }
        garder(l, proj(p));
        l.push([p[0], p[1], z]);
        return;
    }
    let cc = x(t1 + sgn * rho, t2 + sgn * rho);
    let t_1 = [cc[0] - sgn * rho * l1[0], cc[1] - sgn * rho * l1[1]];
    let b = {
        let v = [l1[0] + l2[0], l1[1] + l2[1]];
        let n = v[0].hypot(v[1]).max(1e-6);
        [v[0] / n, v[1] / n]
    };
    let mm = [cc[0] - sgn * rho * b[0], cc[1] - sgn * rho * b[1]];
    if relais(l, proj(t_1)) {
        return;
    }
    garder(l, proj(t_1));
    let a0 = (t_1[1] - cc[1]).atan2(t_1[0] - cc[0]);
    let a1 = (mm[1] - cc[1]).atan2(mm[0] - cc[0]);
    let mut d = a1 - a0;
    while d > std::f32::consts::PI {
        d -= std::f32::consts::TAU;
    }
    while d < -std::f32::consts::PI {
        d += std::f32::consts::TAU;
    }
    let pas = ((d.abs() / 8f32.to_radians()).ceil() as usize).max(2);
    for s in 0..=pas {
        let a = a0 + d * s as f32 / pas as f32;
        l.push([cc[0] + rho * a.cos(), cc[1] + rho * a.sin(), z]);
    }
}
