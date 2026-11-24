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
use sage_reel::reseau::Reseau;
use std::collections::HashMap;

/// Au plus cette part de la longueur d'une arête reculée ou fondue à chaque bout.
const PART_MAX: f32 = 0.45;
/// Une file continue (fourche, jonction, passage) vers les bouts à moins de cet écart
/// de sa direction (degrés) ; au-delà, jusqu'à `VIRAGE_MAX`, c'est un virage.
const TOUT_DROIT: f32 = 40.0;
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
pub const TOLERANCE_DEFAUT: f32 = 0.3;
static LISSE: AtomicBool = AtomicBool::new(true);
static TOLERANCE: AtomicU32 = AtomicU32::new(0x3e99_999a); // 0.3

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
            let mut garde = vec![false; dur.len()];
            for w in bornes.windows(2) {
                garde[w[0]] = true;
                garde[w[1]] = true;
                simplifier(&dur, w[0], w[1], tol, &mut garde);
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
const MARQUE_BAS: f32 = 0.5;
const MARQUE_HAUT: f32 = 3.0;

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
    a.files.total > 0 && a.points.len() >= 2
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
            let g = [g[0] / (c * c), g[1] / (c * c)];
            [pts[i][0] + g[0] * y, pts[i][1] + g[1] * y, pts[i][2]]
        })
        .collect()
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
    let a = &r.aretes[k];
    let n = a.files.total.max(1) as f32;
    let lw = a.largeur / n;
    a.largeur * 0.5 - (i as f32 + 0.5) * lw
}

/// L'axe brut de la file `i` de l'arête `k`, dans le sens du tracé.
fn axe_file(r: &Reseau, k: usize, i: usize) -> Vec<[f32; 3]> {
    decaler(&r.aretes[k].points, decalage(r, k, i))
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
    let p: Vec<[f32; 3]> = if b.debut { a.points.clone() } else { a.points.iter().rev().copied().collect() };
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
    let cible = en_aval(r, n, Some(d.arete), AVAL, n);
    if cible.is_empty() {
        return false;
    }
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
        for (k, m, _) in sorties(r, v) {
            if k == venue {
                continue;
            }
            // Sans repasser par le nœud du virage
            if en_aval(r, v, Some(k), AVAL + fait, n).iter().any(|x| cible.contains(x)) {
                return true;
            }
            let _ = m;
        }
        for (k, o, l) in entrees(r, v) {
            pile.push((o, k, fait + l));
        }
    }
    false
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
            let (fe, fd) = (depuis_noeud(r, bout_de(pa), pa.file), depuis_noeud(r, bout_de(pq), pq.file));
            let arc = arc_virage(&fe, &fd, rayon(a_gauche)).map(|(pts, a, b)| (pts, abscisse(&fe, a), abscisse(&fd, b)));
            if let Some((_, sa, sb)) = &arc {
                if !tronc[e] {
                    bord[e] = bord[e].max(*sa);
                }
                if !tronc[d] {
                    bord[d] = bord[d].max(*sb);
                }
            }
            if let Some(jl) = journal.as_mut() {
                jl.push(format!("virage à {} : a{}·f{} vers a{}·f{}", if a_gauche { "gauche" } else { "droite" }, pa.arete, pa.file, pq.arete, pq.file));
            }
            vs.push(Virage { e, d, fe, fd, arc });
            }
        }
    }
    for e in 0..bs.len() {
        let l = membres[e].iter().map(|b| longueur(&r.aretes[b.arete].points)).fold(f32::MAX, f32::min);
        bord[e] = bord[e].min(PART_MAX * l);
        for b in &membres[e] {
            reculs.insert((b.arete, b.debut, usize::MAX), bord[e]);
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
        let (mut se, mut sd) = (bord[v.e].min(PART_MAX * longueur(&v.fe)), bord[v.d].min(PART_MAX * longueur(&v.fd)));
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
        virages.push(Fil { points: pts, raccord: false, virage: true, bus: false, marque: false, traverse: false, sens: 1 });
    }
    // Les continuités : d'une ligne d'arrêt à l'autre
    for &(a, q) in &paires {
        if tronc[a.0] && tronc[q.0] {
            continue;
        }
        let (pa, pq) = (&arrivees[a.0][a.1], &departs[q.0][q.1]);
        let (fe, fd) = (depuis_noeud(r, bout_de(pa), pa.file), depuis_noeud(r, bout_de(pq), pq.file));
        let (pe, de) = point_dir(&fe, bord[a.0].min(PART_MAX * longueur(&fe)));
        let (pd, dd) = point_dir(&fd, bord[q.0].min(PART_MAX * longueur(&fd)));
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
    let n0 = fontes.get(&(k, i, true)).map_or(p[0], |f| f.p);
    let n1 = fontes.get(&(k, i, false)).map_or(p[p.len() - 1], |f| f.p);
    for n in [n0, n1] {
        sortie.push(Fil { points: vec![[n[0], n[1], n[2] - MARQUE_BAS], [n[0], n[1], n[2] + MARQUE_HAUT]], raccord: false, virage: false, bus: false, marque: true, traverse: false, sens: 0 });
    }
    // Une voie courte et presque droite : en ligne droite d'un nœud de files à l'autre
    // (une file sans partie qui glisse suit son tracé : rien à redresser)
    let glisse = fontes.contains_key(&(k, i, true)) || fontes.contains_key(&(k, i, false));
    // (une rampe, entre murs : seulement si la droite reste sur sa chaussée ; ailleurs
    // toujours, les boulevards en dépendent)
    if glisse && droite(&r.aretes[k].points) && (!rampe(r, k) || sur_chaussee(&r.aretes[k].points, r.aretes[k].largeur * 0.5, n0, n1)) {
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
    if let Some(&f) = fontes.get(&(k, i, true)) {
        let (fondue, r2) = fondre(&reste, f, lmax);
        if fondue.len() >= 2 {
            sortie.push(Fil { points: fondue, raccord: true, virage: false, bus, marque: false, traverse: false, sens: 0 });
        }
        reste = r2;
    }
    if let Some(&f) = fontes.get(&(k, i, false)) {
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
    for (k, a) in r.aretes.iter().enumerate() {
        if !carrossable(r, k) {
            continue;
        }
        let w = a.largeur * 0.5;
        let (x0, z0, x1, z1) = a.points.iter().fold((f32::MAX, f32::MAX, f32::MIN, f32::MIN), |m, p| (m.0.min(p[0]), m.1.min(p[1]), m.2.max(p[0]), m.3.max(p[1])));
        if x1 + w < b[0] || x0 - w > b[2] || z1 + w < b[1] || z0 - w > b[3] {
            continue;
        }
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
    for (k, a) in r.aretes.iter().enumerate() {
        if !vus.contains(&a.a) || !vus.contains(&a.b) || !carrossable(r, k) || courte(r, k) {
            continue;
        }
        // Une voie interne d'un carrefour regroupé : ses liaisons la remplacent
        if a.a != a.b && groupe(r, a.a).contains(&a.b) {
            continue;
        }
        let w = a.largeur * 0.5;
        let (x0, z0, x1, z1) = a.points.iter().fold((f32::MAX, f32::MAX, f32::MIN, f32::MIN), |m, p| (m.0.min(p[0]), m.1.min(p[1]), m.2.max(p[0]), m.3.max(p[1])));
        if x1 + w < b[0] || x0 - w > b[2] || z1 + w < b[1] || z0 - w > b[3] {
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
