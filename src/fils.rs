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

use std::sync::atomic::{AtomicU32, Ordering};
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
/// La longueur de la partie fondue : une base, plus autant de fois l'écart à
/// rattraper, bornée (m).
const FONTE_BASE: f32 = 12.0;
const FONTE_ECART: f32 = 8.0;
const FONTE_MAX: f32 = 90.0;
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
            // Une boucle a ses deux bouts dans le groupe
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
    carrossable(r, k) && !guidee(r, k) && longueur(&r.aretes[k].points) < SERRE
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
    let bs = bouts(r, n);
    if bs.len() < 2 {
        return;
    }
    let dirs: Vec<Option<[f32; 2]>> = bs.iter().map(|&b| direction(r, b)).collect();
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
    let arrivees: Vec<Vec<Place>> = bs.iter().map(|&b| places(r, b, true)).collect();
    let departs: Vec<Vec<Place>> = bs.iter().map(|&b| places(r, b, false)).collect();
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
        fontes.insert((glisse.arete, glisse.file, glisse.debut), Fonte { p: garde.p, o: garde.o });
    }
    if bs.len() < 3 {
        return;
    }
    // Les bouts qui ne se continuent pas : leurs files s'arrêtent au bord des autres voies
    let mut continue_ = vec![false; bs.len()];
    for &(a, q) in &paires {
        continue_[a.0] = true;
        continue_[q.0] = true;
    }
    let mut recul = vec![0.0f32; bs.len()];
    for e in 0..bs.len() {
        if continue_[e] {
            continue;
        }
        let w = (0..bs.len()).filter(|&d| d != e).map(|d| r.aretes[bs[d].arete].largeur * 0.5).fold(0.0f32, f32::max);
        let l = longueur(&r.aretes[bs[e].arete].points);
        recul[e] = (w + VIRAGE_MARGE).min(PART_MAX * l);
        reculs.insert((bs[e].arete, bs[e].debut, usize::MAX), recul[e]);
    }
    // Les virages
    let cos_virage = VIRAGE_MAX.to_radians().cos();
    for e in 0..bs.len() {
        let Some(oe) = dirs[e] else { continue };
        let t = [-oe[0], -oe[1]];
        for d in 0..bs.len() {
            if d == e || cibles[e].contains(&d) {
                continue;
            }
            let Some(od) = dirs[d] else { continue };
            let c = dot(t, od);
            if c > cos_max || c < cos_virage {
                continue;
            }
            let a_gauche = dot(od, gauche(t)) > 0.0;
            // Hors bus si les deux voies en ont ; sinon de bus à bus (le TEOR qui tourne)
            let choisir = |v: &[Place], bus: bool| -> Option<usize> {
                let libres: Vec<usize> = (0..v.len()).filter(|&i| v[i].bus == bus).collect();
                if a_gauche { libres.first().copied() } else { libres.last().copied() }
            };
            let paire = match (choisir(&arrivees[e], false), choisir(&departs[d], false)) {
                (Some(i), Some(j)) => Some((i, j)),
                _ => choisir(&arrivees[e], true).zip(choisir(&departs[d], true)),
            };
            let Some((i, j)) = paire else { continue };
            let (pa, pq) = (&arrivees[e][i], &departs[d][j]);
            // Une voie qui ne fait que tourner : le virage part (ou arrive) au bout de sa
            // file ; sur une voie qui continue, juste après le point de la file en face de
            // l'autre bout (`VIRAGE_RAYON` plus loin). Deux voies qui continuent : au bord
            // de l'autre voie
            let (fe, fd) = (depuis_noeud(r, bs[e], pa.file), depuis_noeud(r, bs[d], pq.file));
            // Le raccord en arc : les deux files prolongées en ligne droite jusqu'à leur
            // croisement, raccordées par un arc de cercle tangent aux deux ; ses points de
            // raccordement, là où il les touche. Une file qui ne fait que tourner s'arrête là
            if let Some((pts, a, b)) = arc_virage(&fe, &fd, rayon(a_gauche)) {
                if !continue_[e] {
                    reculs.insert((bs[e].arete, bs[e].debut, pa.file), abscisse(&fe, a));
                }
                if !continue_[d] {
                    reculs.insert((bs[d].arete, bs[d].debut, pq.file), abscisse(&fd, b));
                }
                virages.push(Fil { points: pts, raccord: false, virage: true, bus: false, marque: false });
                for n in [a, b] {
                    virages.push(Fil { points: vec![[n[0], n[1], n[2] - MARQUE_BAS], [n[0], n[1], n[2] + MARQUE_HAUT]], raccord: false, virage: false, bus: false, marque: true });
                }
                if let Some(jl) = journal.as_mut() {
                    jl.push(format!("virage à {} : a{}·f{} vers a{}·f{}", if a_gauche { "gauche" } else { "droite" }, pa.arete, pa.file, pq.arete, pq.file));
                }
                continue;
            }
            let (se, sd) = match (continue_[e], continue_[d]) {
                (false, false) => (recul[e], recul[d]),
                (false, true) => (recul[e], abscisse(&fd, point_a(&fe, recul[e])) + VIRAGE_RAYON),
                (true, false) => (abscisse(&fe, point_a(&fd, recul[d])) + VIRAGE_RAYON, recul[d]),
                (true, true) => (r.aretes[bs[d].arete].largeur * 0.5 + VIRAGE_MARGE, r.aretes[bs[e].arete].largeur * 0.5 + VIRAGE_MARGE),
            };
            let (a, da) = point_dir(&fe, se.min(PART_MAX * longueur(&fe)));
            let (b, db) = point_dir(&fd, sd.min(PART_MAX * longueur(&fd)));
            virages.push(Fil { points: courbe(a, [-da[0], -da[1]], b, db), raccord: false, virage: true, bus: false, marque: false });
            // Ses points de raccordement : des nœuds de files
            for n in [a, b] {
                virages.push(Fil { points: vec![[n[0], n[1], n[2] - MARQUE_BAS], [n[0], n[1], n[2] + MARQUE_HAUT]], raccord: false, virage: false, bus: false, marque: true });
            }
            if let Some(jl) = journal.as_mut() {
                jl.push(format!("virage à {} : a{}·f{} vers a{}·f{}", if a_gauche { "gauche" } else { "droite" }, pa.arete, pa.file, pq.arete, pq.file));
            }
        }
    }
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
    let l = (FONTE_BASE + FONTE_ECART * ecart).min(FONTE_MAX).min(lmax);
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
    let recul = |debut: bool| reculs.get(&(k, debut, i)).or_else(|| reculs.get(&(k, debut, usize::MAX))).copied().unwrap_or(0.0);
    let p = rogner(&axe, recul(true), recul(false));
    if p.len() < 2 {
        return;
    }
    // Une voie guidée suit son tracé, d'un bout à l'autre
    if guidee(r, k) {
        for n in [p[0], p[p.len() - 1]] {
            sortie.push(Fil { points: vec![[n[0], n[1], n[2] - MARQUE_BAS], [n[0], n[1], n[2] + MARQUE_HAUT]], raccord: false, virage: false, bus: false, marque: true });
        }
        sortie.push(Fil { points: p, raccord: false, virage: false, bus, marque: false });
        return;
    }
    // Les nœuds de files à ses bouts : le bout de l'autre file qu'elle prolonge, ou le sien
    let n0 = fontes.get(&(k, i, true)).map_or(p[0], |f| f.p);
    let n1 = fontes.get(&(k, i, false)).map_or(p[p.len() - 1], |f| f.p);
    for n in [n0, n1] {
        sortie.push(Fil { points: vec![[n[0], n[1], n[2] - MARQUE_BAS], [n[0], n[1], n[2] + MARQUE_HAUT]], raccord: false, virage: false, bus: false, marque: true });
    }
    // Une voie courte et presque droite : en ligne droite d'un nœud de files à l'autre
    if droite(&r.aretes[k].points) {
        let l = longueur(&p);
        let pas = ((l / 4.0).ceil() as usize).max(1);
        let pts = (0..=pas)
            .map(|j| {
                let t = j as f32 / pas as f32;
                [n0[0] + (n1[0] - n0[0]) * t, n0[1] + (n1[1] - n0[1]) * t, point_a(&p, l * t)[2]]
            })
            .collect();
        sortie.push(Fil { points: pts, raccord: false, virage: false, bus, marque: false });
        return;
    }
    let lmax = PART_MAX * longueur(&p);
    let mut reste = p;
    if let Some(&f) = fontes.get(&(k, i, true)) {
        let (fondue, r2) = fondre(&reste, f, lmax);
        if fondue.len() >= 2 {
            sortie.push(Fil { points: fondue, raccord: true, virage: false, bus, marque: false });
        }
        reste = r2;
    }
    if let Some(&f) = fontes.get(&(k, i, false)) {
        if reste.len() >= 2 {
            let envers: Vec<[f32; 3]> = reste.iter().rev().copied().collect();
            let (fondue, r2) = fondre(&envers, f, lmax);
            if fondue.len() >= 2 {
                sortie.push(Fil { points: fondue, raccord: true, virage: false, bus, marque: false });
            }
            reste = r2.into_iter().rev().collect();
        }
    }
    if reste.len() >= 2 {
        sortie.push(Fil { points: reste, raccord: false, virage: false, bus, marque: false });
    }
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
