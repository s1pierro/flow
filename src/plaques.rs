//! Les plaques de carrefour du réseau nivelé : après le nivellement des voies, chaque
//! carrefour (un nœud d'au moins trois bouts carrossables, ou un groupe de tels nœuds
//! reliés par des tronçons de moins de `LIEN` mètres) devient une plaque plane, ajustée
//! sur le profil de toutes ses branches (la principale compte le plus), de pente au
//! plus `PENTE_MAX`. Chaque branche est posée sur ce plan jusqu'à sa bouche (le rayon
//! de la plaque), puis rejoint son profil par un raccord lissé qui garde la pente du
//! plan à la bouche et retrouve celle de la voie au bout : ni marche entre les
//! branches, ni cassure à l'entrée du carrefour. Les tronçons intérieurs d'un groupe
//! sont entièrement sur le plan. Les ponts et les tunnels ne contraignent pas les
//! plaques (seulement les voies entre elles).
//!
//! Hiérarchie : à un nœud où une route arrive, seules les routes font le carrefour ;
//! les voies de service, chemins et piétonnes s'y greffent sans la modifier (`niveau`).

use crate::reseau::{entre, Reseau};
use std::collections::HashMap;

/// Un tronçon de moins de tant (m) entre deux nœuds de carrefour les met dans la même
/// plaque.
const LIEN: f32 = 12.0;
/// Les anneaux (les petits giratoires) : une boucle de rues entre nœuds de carrefour,
/// chacune d'au plus `ANNEAU_RUE` mètres (à travers les nœuds de deux bouts), de
/// périmètre au plus `ANNEAU_PERIMETRE`, met tous ses carrefours dans la même plaque, et
/// ses rues entières sur le plan.
const ANNEAU_RUE: f32 = 30.0;
const ANNEAU_PERIMETRE: f32 = 60.0;
/// La bouche d'une branche : à `BOUCHE_MIN` mètres du nœud au moins, `BOUCHE_MAX` au
/// plus, `BOUCHE_MARGE` au-delà de la bordure ; la direction d'une branche est prise à
/// `DIRECTION` mètres.
const BOUCHE_MIN: f32 = 2.0;
const BOUCHE_MAX: f32 = 20.0;
const BOUCHE_MARGE: f32 = 0.5;
const DIRECTION: f32 = 6.0;
/// Deux branches voisines à moins de tant (degrés) forment un Y : jusqu'à leur
/// séparation, elles prennent le profil moyen des deux (ni marche entre elles, ni long
/// retard sur leur pente).
const Y_ANGLE: f32 = 45.0;
/// … et seulement si leurs profils d'origine s'écartent d'au moins tant (mètres) là où
/// leurs bords se croisent : en dessous, la marche serait invisible.
const Y_ECART: f32 = 0.3;
/// … et si chacune peut ensuite retrouver son profil sans dépasser sa pente de plus de
/// tant.
const Y_PENTE: f32 = 0.08;
/// La plus forte pente d'une plaque.
const PENTE_MAX: f32 = 0.08;
/// Une plaque d'un seul nœud dont les routes ne sont que deux bouts (bout à bout, quel
/// que soit l'angle) : la rue continue, une voie secondaire s'y greffe ; le plan
/// reprend la pente de chacun des deux bouts, sans plafond, pour que le profil de la
/// rue ne bouge presque pas. Deux bouts presque alignés (sinus de leur angle sous
/// tant) : la pente de l'un à l'autre, plate en travers.
const ALIGNES: f32 = 0.3;
/// La portée (m), au-delà de la bouche, du profil d'une branche pris pour ajuster le
/// plan.
const PORTEE: f32 = 12.0;
/// Le raccord : au moins tant de mètres, assez pour n'ajouter que `RACCORD_PENTE` à
/// la pente de la voie (un raccord lissé atteint 1,5 fois sa pente moyenne), au plus
/// `RACCORD_MAX`.
const RACCORD_MIN: f32 = 8.0;
const RACCORD_PENTE: f32 = 0.05;
const RACCORD_MAX: f32 = 40.0;
/// Le pas (m) des points ajoutés sur la plaque et le raccord.
const PAS: f32 = 2.0;
/// Le raccord des voies non carrossables à un nœud de plaque (m).
const RACCORD_PIETON: f32 = 5.0;

/// Un plan : h = c + g · (x, z) − (cx, cz).
#[derive(Clone, Copy, Debug)]
pub struct Plan {
    pub c: [f32; 3],
    pub g: [f32; 2],
}

impl Plan {
    pub fn h(&self, x: f32, z: f32) -> f32 {
        self.c[2] + self.g[0] * (x - self.c[0]) + self.g[1] * (z - self.c[1])
    }
}

/// Le bilan : le nombre de plaques, de nœuds groupés, de branches posées.
#[derive(Default, Debug)]
pub struct Bilan {
    pub plaques: usize,
    pub groupes: usize,
    pub branches: usize,
    pub greffes: usize,
    /// Les rues des anneaux (petits giratoires) posées entières sur leur plaque.
    pub anneaux: usize,
}

fn carrossable(r: &Reseau, k: usize) -> bool {
    let a = &r.aretes[k];
    sage_reel::carrossable(a.genre) && a.points.len() >= 2
}

/// Une route (grande route, route, bus).
fn principale(r: &Reseau, k: usize) -> bool {
    carrossable(r, k) && crate::materialisation::rang(r.aretes[k].genre) <= 1
}

/// Le niveau d'une voie dans la hiérarchie : 1 les routes, 0 les autres (service,
/// chemins, piétonnes). À un nœud, seules les voies carrossables du plus haut niveau
/// présent comptent (`compte`) : une voie de service ne fait pas de carrefour avec une
/// route, elle s'y greffe (`greffer`).
fn niveau(r: &Reseau, k: usize) -> u8 {
    u8::from(principale(r, k))
}

fn longueur(q: &[[f32; 3]]) -> f32 {
    q.windows(2).map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1])).sum()
}

/// Le rang d'une voie (le plus petit passe en premier) et son poids dans l'ajustement.
fn poids(genre: u8, largeur: f32) -> f32 {
    let rang = crate::materialisation::rang(genre) as i32;
    2f32.powi(4 - rang.min(4)) * largeur.max(2.0)
}

/// La hauteur d'une polyligne à l'abscisse `s` (interpolée).
fn hauteur(q: &[[f32; 3]], abs: &[f32], s: f32) -> f32 {
    let i = abs.partition_point(|&x| x < s);
    if i == 0 {
        return q[0][2];
    }
    if i >= q.len() {
        return q[q.len() - 1][2];
    }
    let t = ((s - abs[i - 1]) / (abs[i] - abs[i - 1]).max(1e-6)).clamp(0.0, 1.0);
    q[i - 1][2] + t * (q[i][2] - q[i - 1][2])
}

fn abscisses(q: &[[f32; 3]]) -> Vec<f32> {
    let mut s = vec![0.0f32; q.len()];
    for i in 1..q.len() {
        s[i] = s[i - 1] + (q[i][0] - q[i - 1][0]).hypot(q[i][1] - q[i - 1][1]);
    }
    s
}

fn lisse(u: f32) -> f32 {
    let u = u.clamp(0.0, 1.0);
    u * u * (3.0 - 2.0 * u)
}

/// Pose les plaques de carrefour sur le réseau nivelé.
pub fn poser(r: &mut Reseau) -> Bilan {
    poser_avec(r, &[])
}

/// `poser`, les arêtes `rapides` (les voies rapides posées) au-dessus des routes dans
/// la hiérarchie : à leurs nœuds, elles seules comptent ; une bretelle s'y greffe.
pub fn poser_avec(r: &mut Reseau, rapides: &[bool]) -> Bilan {
    let niv = |r: &Reseau, k: usize| -> u8 { if rapides.get(k).copied().unwrap_or(false) { 2 } else { niveau(r, k) } };
    let mut bilan = Bilan::default();
    let nn = r.noeuds.len();
    // Le plus haut niveau carrossable de chaque nœud, et les voies qui y comptent
    let haut: Vec<u8> = (0..nn).map(|n| r.noeuds[n].aretes.iter().filter(|&&k| carrossable(r, k)).map(|&k| niv(r, k)).max().unwrap_or(0)).collect();
    let compte = |r: &Reseau, n: usize, k: usize| carrossable(r, k) && niv(r, k) == haut[n];
    // 1. Les nœuds de carrefour : au moins trois bouts carrossables (du plus haut niveau)
    let bouts = |n: usize| -> usize { r.noeuds[n].aretes.iter().filter(|&&k| compte(r, n, k)).map(|&k| usize::from(r.aretes[k].a == n) + usize::from(r.aretes[k].b == n)).sum() };
    // Les voies rapides posées ne font pas de plaque : elles ne bougent plus
    let carrefour: Vec<bool> = (0..nn).map(|n| bouts(n) >= 3 && haut[n] < 2).collect();
    // 2. Les groupes : des nœuds de carrefour reliés par des tronçons courts
    let mut parent: Vec<usize> = (0..nn).collect();
    fn racine(p: &mut [usize], mut i: usize) -> usize {
        while p[i] != i {
            p[i] = p[p[i]];
            i = p[i];
        }
        i
    }
    for (k, a) in r.aretes.iter().enumerate() {
        if compte(r, a.a, k) && compte(r, a.b, k) && a.a != a.b && carrefour[a.a] && carrefour[a.b] && longueur(&a.points) < LIEN {
            let (x, y) = (racine(&mut parent, a.a), racine(&mut parent, a.b));
            if x != y {
                parent[x] = y;
            }
        }
    }
    // Les anneaux : les rues courtes entre carrefours, puis pour chacune le plus court
    // retour par les autres ; une boucle assez courte réunit ses carrefours
    let mut anneau: std::collections::HashSet<usize> = std::collections::HashSet::new();
    {
        let degre0: Vec<usize> = (0..nn).map(bouts).collect();
        let na = r.aretes.len();
        let mut vue_r = vec![false; na];
        // (début, fin, longueur, voies)
        let mut rues: Vec<(usize, usize, f32, Vec<usize>)> = Vec::new();
        for n0 in (0..nn).filter(|&n| carrefour[n]) {
            for &k0 in &r.noeuds[n0].aretes {
                if !compte(r, n0, k0) || vue_r[k0] {
                    continue;
                }
                let (mut cur, mut k, mut total, mut voies) = (n0, k0, 0.0f32, Vec::new());
                loop {
                    vue_r[k] = true;
                    voies.push(k);
                    let a = &r.aretes[k];
                    total += longueur(&a.points);
                    cur = if a.a == cur { a.b } else { a.a };
                    if a.a == a.b || carrefour[cur] || degre0[cur] != 2 || total > ANNEAU_RUE {
                        break;
                    }
                    let Some(&k2) = r.noeuds[cur].aretes.iter().find(|&&j| j != k && compte(r, cur, j)) else { break };
                    if vue_r[k2] {
                        break;
                    }
                    k = k2;
                }
                if carrefour[cur] && cur != n0 && total <= ANNEAU_RUE {
                    rues.push((n0, cur, total, voies));
                }
            }
        }
        let mut voisins: HashMap<usize, Vec<(usize, f32, usize)>> = HashMap::new();
        for (i, x) in rues.iter().enumerate() {
            voisins.entry(x.0).or_default().push((x.1, x.2, i));
            voisins.entry(x.1).or_default().push((x.0, x.2, i));
        }
        for (i, (u, v, l, voies)) in rues.iter().enumerate() {
            // Le plus court chemin de v à u sans cette rue (Dijkstra borné)
            let borne = ANNEAU_PERIMETRE - l;
            let mut dist: HashMap<usize, f32> = HashMap::new();
            let mut tas = std::collections::BinaryHeap::new();
            dist.insert(*v, 0.0);
            tas.push(std::cmp::Reverse(((0.0f32 * 1000.0) as i64, *v)));
            let mut trouve = false;
            while let Some(std::cmp::Reverse((d, n))) = tas.pop() {
                let d = d as f32 / 1000.0;
                if n == *u {
                    trouve = true;
                    break;
                }
                if d > dist.get(&n).copied().unwrap_or(f32::MAX) + 1e-3 {
                    continue;
                }
                for &(m, lm, j) in voisins.get(&n).map_or(&[][..], |x| &x[..]) {
                    let dm = d + lm;
                    if j == i || dm > borne || dm >= dist.get(&m).copied().unwrap_or(f32::MAX) {
                        continue;
                    }
                    dist.insert(m, dm);
                    tas.push(std::cmp::Reverse(((dm * 1000.0) as i64, m)));
                }
            }
            if trouve {
                let (x, y) = (racine(&mut parent, *u), racine(&mut parent, *v));
                if x != y {
                    parent[x] = y;
                }
                anneau.extend(voies.iter().copied());
                bilan.anneaux += 1;
            }
        }
    }
    let mut groupes: HashMap<usize, Vec<usize>> = HashMap::new();
    for n in (0..nn).filter(|&n| carrefour[n]) {
        let g = racine(&mut parent, n);
        groupes.entry(g).or_default().push(n);
    }
    let groupe_de: Vec<Option<usize>> = (0..nn).map(|n| carrefour[n].then(|| racine(&mut parent, n))).collect();
    // 3. La bouche de chaque branche (nœud, voie) : comme celle des plaques de la
    //    Matérialisation (`carrefours`), là où son bord croise celui de la branche
    //    voisine, plus la bordure arrondie et une marge ; un Y serré l'éloigne, un T ou
    //    une croix la garde près du nœud
    let mut bouche: HashMap<(usize, usize), f32> = HashMap::new();
    // Les branches d'un Y (deux branches voisines à moins de Y_ANGLE) : jusqu'à leur
    // séparation, et leur voisine
    let mut y_de: HashMap<(usize, usize), (f32, usize, f32)> = HashMap::new();
    for n in (0..nn).filter(|&n| carrefour[n]) {
        let c = [r.noeuds[n].p[0], r.noeuds[n].p[1]];
        // (voie, direction, demi-largeur, angle)
        let mut bras: Vec<(usize, [f32; 2], f32, f32)> = Vec::new();
        for &k in &r.noeuds[n].aretes {
            if !compte(r, n, k) {
                continue;
            }
            let a = &r.aretes[k];
            for fin in [false, true] {
                if (if fin { a.b } else { a.a }) != n {
                    continue;
                }
                let q: Vec<[f32; 3]> = if fin { a.points.iter().rev().copied().collect() } else { a.points.clone() };
                let abs = abscisses(&q);
                let x = DIRECTION.min(abs[abs.len() - 1]);
                let i = abs.partition_point(|&y| y < x).clamp(1, q.len() - 1);
                let t = ((x - abs[i - 1]) / (abs[i] - abs[i - 1]).max(1e-6)).clamp(0.0, 1.0);
                let pt = entre(q[i - 1], q[i], t);
                let d = [pt[0] - c[0], pt[1] - c[1]];
                let l = d[0].hypot(d[1]).max(1e-6);
                let u = [d[0] / l, d[1] / l];
                bras.push((k, u, a.largeur * 0.5, u[1].atan2(u[0])));
            }
        }
        bras.sort_by(|a, b| a.3.total_cmp(&b.3));
        let m = bras.len();
        let mut sv = vec![BOUCHE_MIN; m];
        for i in 0..m {
            let j = (i + 1) % m;
            let theta = (bras[j].3 - bras[i].3).rem_euclid(std::f32::consts::TAU);
            if theta >= std::f32::consts::PI - 0.05 || theta < 1e-3 {
                continue;
            }
            let (ui, uj) = (bras[i].1, bras[j].1);
            let pi = [c[0] - ui[1] * bras[i].2, c[1] + ui[0] * bras[i].2];
            let pj = [c[0] + uj[1] * bras[j].2, c[1] - uj[0] * bras[j].2];
            let det = uj[0] * ui[1] - ui[0] * uj[1];
            if det.abs() < 1e-6 {
                continue;
            }
            let rr = [pj[0] - pi[0], pj[1] - pi[1]];
            let (a, b) = ((uj[0] * rr[1] - rr[0] * uj[1]) / det, (ui[0] * rr[1] - ui[1] * rr[0]) / det);
            let tau = crate::carrefours::BORDURE / (theta / 2.0).tan();
            if a > 0.0 {
                sv[i] = sv[i].max(a + tau + BOUCHE_MARGE);
            }
            if b > 0.0 {
                sv[j] = sv[j].max(b + tau + BOUCHE_MARGE);
            }
            if theta < Y_ANGLE.to_radians() && a > 0.0 && b > 0.0 && bras[i].0 != bras[j].0 {
                for (x, autre, d) in [(i, bras[j].0, a), (j, bras[i].0, b)] {
                    // Là où leurs bords se croisent : au-delà, entre elles, le coin
                    // arrondi n'est plus de la chaussée
                    let sep = (d + BOUCHE_MARGE).min(BOUCHE_MAX);
                    let e = y_de.entry((n, bras[x].0)).or_insert((sep, autre, theta));
                    if theta < e.2 {
                        *e = (sep, autre, theta);
                    }
                }
            }
        }
        for (i, b) in bras.iter().enumerate() {
            bouche.insert((n, b.0), sv[i].clamp(BOUCHE_MIN, BOUCHE_MAX));
        }
    }
    let _ = &bouche;
    // Le cœur plan de la plaque autour de chaque nœud : une fois et demie la plus
    // grande demi-largeur, plus un mètre (comme `materialisation::plan`)
    let coeur: Vec<f32> = (0..nn)
        .map(|n| {
            if !carrefour[n] {
                return 0.0;
            }
            let w = r.noeuds[n].aretes.iter().filter(|&&k| compte(r, n, k)).map(|&k| r.aretes[k].largeur * 0.5).fold(1.0f32, f32::max);
            1.5 * w + 1.0
        })
        .collect();
    let rayon = |n: usize, _k: usize| -> f32 { coeur[n] };
    // 4. Le plan de chaque plaque : moindres carrés pondérés sur les branches (des
    //    nœuds jusqu'à la bouche plus PORTEE), la pente plafonnée
    let mut plans: HashMap<usize, Plan> = HashMap::new();
    for (&g, noeuds) in &groupes {
        let (mut sw, mut sx, mut sz, mut sh) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
        let mut echantillons: Vec<(f32, f32, f32, f32)> = Vec::new();
        // Les bouts de route (direction, hauteur au loin, distance) pour la continuité
        let mut bouts_route: Vec<([f32; 2], f32, f32)> = Vec::new();
        for &n in noeuds {
            let p = r.noeuds[n].p;
            echantillons.push((p[0], p[1], p[2], 4.0 * poids(sage_reel::ROUTE, 6.0)));
            for &k in &r.noeuds[n].aretes {
                if !compte(r, n, k) {
                    continue;
                }
                let a = &r.aretes[k];
                let w = poids(a.genre, a.largeur);
                for fin in [false, true] {
                    if (if fin { a.b } else { a.a }) != n {
                        continue;
                    }
                    let q: Vec<[f32; 3]> = if fin { a.points.iter().rev().copied().collect() } else { a.points.clone() };
                    let abs = abscisses(&q);
                    let jusqu = (rayon(n, k) + PORTEE).min(abs[abs.len() - 1]);
                    if noeuds.len() == 1 && principale(r, k) && jusqu > 0.5 {
                        let i = abs.partition_point(|&y| y < jusqu).clamp(1, q.len() - 1);
                        let t = ((jusqu - abs[i - 1]) / (abs[i] - abs[i - 1]).max(1e-6)).clamp(0.0, 1.0);
                        let pt = entre(q[i - 1], q[i], t);
                        let d = [pt[0] - p[0], pt[1] - p[1]];
                        let l = d[0].hypot(d[1]).max(1e-6);
                        bouts_route.push(([d[0] / l, d[1] / l], hauteur(&q, &abs, jusqu), jusqu));
                    }
                    let mut s = PAS;
                    while s <= jusqu {
                        let h = hauteur(&q, &abs, s);
                        let i = abs.partition_point(|&x| x < s).clamp(1, q.len() - 1);
                        let t = ((s - abs[i - 1]) / (abs[i] - abs[i - 1]).max(1e-6)).clamp(0.0, 1.0);
                        let pt = entre(q[i - 1], q[i], t);
                        echantillons.push((pt[0], pt[1], h, w));
                        s += PAS;
                    }
                }
            }
        }
        for &(x, z, h, w) in &echantillons {
            let w = w as f64;
            sw += w;
            sx += w * x as f64;
            sz += w * z as f64;
            sh += w * h as f64;
        }
        if sw <= 0.0 {
            continue;
        }
        let (cx, cz, ch) = ((sx / sw) as f32, (sz / sw) as f32, (sh / sw) as f32);
        // h − ch = gx (x − cx) + gz (z − cz)
        let (mut a11, mut a12, mut a22, mut b1, mut b2) = (0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64);
        for &(x, z, h, w) in &echantillons {
            let (dx, dz, dh, w) = ((x - cx) as f64, (z - cz) as f64, (h - ch) as f64, w as f64);
            a11 += w * dx * dx;
            a12 += w * dx * dz;
            a22 += w * dz * dz;
            b1 += w * dx * dh;
            b2 += w * dz * dh;
        }
        let det = a11 * a22 - a12 * a12;
        let mut gr = if det.abs() > 1e-6 { [((b1 * a22 - b2 * a12) / det) as f32, ((a11 * b2 - a12 * b1) / det) as f32] } else { [0.0, 0.0] };
        let c = if bouts_route.len() == 2 {
            let ((u, h1, s1), (v, h2, s2)) = (bouts_route[0], bouts_route[1]);
            let pn = r.noeuds[noeuds[0]].p;
            let det = u[0] * v[1] - u[1] * v[0];
            if det.abs() >= ALIGNES {
                // La pente de chaque bout depuis le nœud : g·u = pu, g·v = pv
                let (pu, pv) = ((h1 - pn[2]) / s1.max(1.0), (h2 - pn[2]) / s2.max(1.0));
                gr = [(pu * v[1] - pv * u[1]) / det, (u[0] * pv - v[0] * pu) / det];
            } else {
                // Presque alignés : la pente de l'un à l'autre, plate en travers
                let t = [v[0] - u[0], v[1] - u[1]];
                let l = t[0].hypot(t[1]).max(1e-6);
                let pente = (h2 - h1) / (s1 + s2).max(1.0);
                gr = [t[0] / l * pente, t[1] / l * pente];
            }
            pn
        } else {
            let m = gr[0].hypot(gr[1]);
            if m > PENTE_MAX {
                gr = [gr[0] * PENTE_MAX / m, gr[1] * PENTE_MAX / m];
            }
            [cx, cz, ch]
        };
        plans.insert(g, Plan { c, g: gr });
        bilan.plaques += 1;
        if noeuds.len() > 1 {
            bilan.groupes += 1;
        }
    }
    // 5. Les rues : d'une plaque à l'autre (ou à une impasse), à travers les nœuds de
    //    deux voies, chacune posée sur la plaque de chaque bout jusqu'à la bouche puis
    //    raccordée par un raccord lissé ; les tronçons intérieurs courts d'une plaque,
    //    entièrement sur le plan
    let anciens: Vec<f32> = r.noeuds.iter().map(|n| n.p[2]).collect();
    let degre: Vec<usize> = (0..nn).map(bouts).collect();
    // Les profils moyens des Y : à même distance du nœud, la moyenne des hauteurs
    // d'origine des deux branches (un point par mètre jusqu'à leur séparation)
    // Les points d'une rue depuis n0 par k0, et le nœud où elle s'arrête
    let rue_points = |n0: usize, k0: usize| -> (Vec<[f32; 3]>, usize) {
        let mut q: Vec<[f32; 3]> = Vec::new();
        let (mut cur, mut k) = (n0, k0);
        let mut vus = 0;
        loop {
            let a = &r.aretes[k];
            let endroit = a.a == cur;
            let pts: Vec<[f32; 3]> = if endroit { a.points.clone() } else { a.points.iter().rev().copied().collect() };
            if q.is_empty() {
                q.extend_from_slice(&pts);
            } else {
                q.extend_from_slice(&pts[1..]);
            }
            cur = if endroit { a.b } else { a.a };
            vus += 1;
            if a.a == a.b || carrefour[cur] || degre[cur] != 2 || !compte(r, cur, k) || vus > 200 {
                break;
            }
            let Some(&k2) = r.noeuds[cur].aretes.iter().find(|&&j| j != k && compte(r, cur, j)) else { break };
            k = k2;
        }
        (q, cur)
    };
    let a_plan = |n: usize| groupe_de[n].is_some_and(|g| plans.contains_key(&g));
    let mut y_moyen: HashMap<(usize, usize), (f32, Vec<f32>)> = HashMap::new();
    for (&(n, k), &(sep, autre, _)) in &y_de {
        let ((q1, f1), (q2, f2)) = (rue_points(n, k), rue_points(n, autre));
        let (a1, a2) = (abscisses(&q1), abscisses(&q2));
        let sep = sep.min(a1[a1.len() - 1]).min(a2[a2.len() - 1]);
        let ecart = (hauteur(&q1, &a1, sep) - hauteur(&q2, &a2, sep)).abs();
        if ecart < Y_ECART {
            continue;
        }
        // La pente raisonnable : chaque branche doit pouvoir retrouver son profil (la
        // moitié de l'écart) dans la place qui lui reste avant l'autre bout de sa rue,
        // sans dépasser sa pente de plus de Y_PENTE ; sinon la marche reste (un mur)
        let place = |abs: &[f32], fin: usize| {
            let total = abs[abs.len() - 1];
            let part = if a_plan(fin) { 0.5 * total } else { total };
            part - sep
        };
        let raisonnable = |pl: f32| pl > 1.0 && 1.5 * (0.5 * ecart) / pl <= Y_PENTE;
        if !raisonnable(place(&a1, f1)) || !raisonnable(place(&a2, f2)) {
            continue;
        }
        let v: Vec<f32> = (0..=sep.ceil() as usize).map(|c| 0.5 * (hauteur(&q1, &a1, c as f32) + hauteur(&q2, &a2, c as f32))).collect();
        y_moyen.insert((n, k), (sep, v));
    }
    let lire_y = |v: &[f32], c: f32| -> f32 {
        let i = (c.floor().max(0.0) as usize).min(v.len() - 1);
        let j = (i + 1).min(v.len() - 1);
        v[i] + (v[j] - v[i]) * (c - i as f32).clamp(0.0, 1.0)
    };
    let mut vue = vec![false; r.aretes.len()];
    let mut departs: Vec<(usize, usize)> = Vec::new();
    for n in (0..nn).filter(|&n| groupe_de[n].is_some_and(|g| plans.contains_key(&g))) {
        for &k in &r.noeuds[n].aretes {
            if compte(r, n, k) {
                departs.push((n, k));
            }
        }
    }
    let mut hauteurs_noeuds: Vec<(usize, f32)> = Vec::new();
    for (n0, k0) in departs {
        if vue[k0] {
            continue;
        }
        // La rue : ses voies (voie, à l'endroit), ses points depuis n0, l'abscisse de
        // début de chaque voie, ses nœuds intérieurs
        let mut membres: Vec<(usize, bool, f32)> = Vec::new();
        let mut q: Vec<[f32; 3]> = Vec::new();
        let mut interieurs: Vec<(usize, f32)> = Vec::new();
        let (mut cur, mut k) = (n0, k0);
        loop {
            vue[k] = true;
            let a = &r.aretes[k];
            let endroit = a.a == cur;
            let pts: Vec<[f32; 3]> = if endroit { a.points.clone() } else { a.points.iter().rev().copied().collect() };
            let debut = longueur(&q);
            if q.is_empty() {
                q.extend_from_slice(&pts);
            } else {
                q.extend_from_slice(&pts[1..]);
            }
            membres.push((k, endroit, debut));
            let suivant = if endroit { a.b } else { a.a };
            cur = suivant;
            if a.a == a.b || carrefour[cur] || degre[cur] != 2 || !compte(r, cur, k) {
                break;
            }
            let Some(&k2) = r.noeuds[cur].aretes.iter().find(|&&j| j != k && compte(r, cur, j)) else { break };
            if vue[k2] {
                break;
            }
            interieurs.push((cur, longueur(&q)));
            k = k2;
        }
        let abs = abscisses(&q);
        let total = abs[abs.len() - 1];
        let pa = groupe_de[n0].and_then(|g| plans.get(&g).map(|p| (g, *p)));
        let pb = groupe_de[cur].and_then(|g| plans.get(&g).map(|p| (g, *p)));
        // Le tronçon intérieur court d'une plaque : sur le plan
        let interieur = matches!((pa, pb), (Some((ga, _)), Some((gb, _))) if ga == gb && (total < LIEN || anneau.contains(&k0)));
        let pts: Vec<[f32; 4]> = if interieur {
            let p = pa.unwrap().1;
            densifier(&q, &abs, &[(0.0, total)]).into_iter().map(|x| [x[0], x[1], p.h(x[0], x[1]), x[3]]).collect()
        } else {
            // La part de chaque bout : toute la rue, ou la moitié si les deux bouts ont
            // une plaque
            let part = if pa.is_some() && pb.is_some() { 0.5 * total } else { total };
            let point = |x: f32| {
                let i = abs.partition_point(|&y| y < x).clamp(1, q.len() - 1);
                let t = ((x - abs[i - 1]) / (abs[i] - abs[i - 1]).max(1e-6)).clamp(0.0, 1.0);
                entre(q[i - 1], q[i], t)
            };
            // Un bout : son plan, son cœur plan, la part d'un Y (séparation, profil
            // moyen), le début et la longueur du raccord
            struct Bout {
                p: Plan,
                s: f32,
                y: Option<(f32, Vec<f32>)>,
                sr: f32,
                l: f32,
            }
            let bout = |n: usize, k: usize, p: Plan, depuis_b: bool| -> Bout {
                let s = rayon(n, k).min(0.6 * part);
                let y = y_moyen.get(&(n, k)).map(|(sep, v)| (sep.min(0.6 * part), v.clone())).filter(|(sep, _)| *sep > s + 0.5);
                let sr = y.as_ref().map_or(s, |(sep, _)| *sep);
                let x = if depuis_b { total - sr } else { sr };
                let pt = point(x);
                let cible = match &y {
                    Some((_, v)) => lire_y(v, sr),
                    None => p.h(pt[0], pt[1]),
                };
                let ecart = (hauteur(&q, &abs, x) - cible).abs();
                let l = RACCORD_MIN.max(1.5 * ecart / RACCORD_PENTE).min(RACCORD_MAX).min(part - sr).max(0.0);
                Bout { p, s, y, sr, l }
            };
            // Ce qu'un bout demande en un point à `c` mètres de lui : (poids, hauteur)
            let demande = |b: &Bout, c: f32, x: f32, z: f32| -> (f32, f32) {
                let plan = b.p.h(x, z);
                if c <= b.s {
                    return (1.0, plan);
                }
                if let Some((sep, v)) = &b.y {
                    if c <= *sep {
                        let hy = lire_y(v, c);
                        let t = (c - b.s) / (sep - b.s).max(1e-3);
                        return (1.0, hy + (plan - hy) * (1.0 - lisse(t)));
                    }
                }
                let cible = match &b.y {
                    Some((sep, v)) => {
                        let h1 = lire_y(v, *sep);
                        let pente = h1 - lire_y(v, (sep - 1.0).max(0.0));
                        h1 + pente * (c - sep)
                    }
                    None => plan,
                };
                let w = if b.l > 1e-3 { 1.0 - lisse((c - b.sr) / b.l) } else { 0.0 };
                (w, cible)
            };
            let za = pa.map(|(_, p)| bout(n0, k0, p, false));
            let k1 = membres[membres.len() - 1].0;
            let zb = pb.map(|(_, p)| bout(cur, k1, p, true));
            let mut coupes: Vec<(f32, f32)> = Vec::new();
            if let Some(b) = &za {
                coupes.push((0.0, b.sr + b.l));
            }
            if let Some(b) = &zb {
                coupes.push((total - b.sr - b.l, total));
            }
            bilan.branches += usize::from(za.is_some()) + usize::from(zb.is_some());
            densifier(&q, &abs, &coupes)
                .into_iter()
                .map(|x| {
                    let (c, h0) = (x[3], x[2]);
                    let mut h = h0;
                    if let Some(b) = &za {
                        let (w, t) = demande(b, c, x[0], x[1]);
                        h += w * (t - h0);
                    }
                    if let Some(b) = &zb {
                        let (w, t) = demande(b, total - c, x[0], x[1]);
                        h += w * (t - h0);
                    }
                    [x[0], x[1], h, c]
                })
                .collect()
        };
        // Rendue à ses voies (coupée aux abscisses de leurs bouts) et à ses nœuds
        // intérieurs
        let bornes: Vec<f32> = membres.iter().map(|m| m.2).chain(std::iter::once(total)).collect();
        for (i, &(k, endroit, _)) in membres.iter().enumerate() {
            let (s0, s1) = (bornes[i], bornes[i + 1]);
            let mut v: Vec<[f32; 3]> = pts.iter().filter(|x| x[3] >= s0 - 1e-3 && x[3] <= s1 + 1e-3).map(|x| [x[0], x[1], x[2]]).collect();
            if v.len() < 2 {
                continue;
            }
            if !endroit {
                v.reverse();
            }
            r.aretes[k].points = v;
        }
        for (n, c) in interieurs {
            let i = pts.partition_point(|x| x[3] < c - 1e-3).min(pts.len() - 1);
            hauteurs_noeuds.push((n, pts[i][2]));
        }
    }
    for (n, h) in hauteurs_noeuds {
        r.noeuds[n].p[2] = h;
    }
    // Les nœuds de plaque : à la hauteur du plan
    for n in 0..nn {
        if let Some(p) = groupe_de[n].and_then(|g| plans.get(&g)) {
            let q = r.noeuds[n].p;
            r.noeuds[n].p[2] = p.h(q[0], q[1]);
        }
    }
    // Les voies non carrossables qui arrivent à un nœud de plaque : décalées de ce
    // qu'il a bougé, en s'effaçant sur RACCORD_PIETON mètres
    for k in 0..r.aretes.len() {
        if carrossable(r, k) {
            continue;
        }
        let (a, b) = (r.aretes[k].a, r.aretes[k].b);
        let greffe = |n: usize| niv(r, k) < haut[n];
        let (da, db) = (if greffe(a) { 0.0 } else { r.noeuds[a].p[2] - anciens[a] }, if greffe(b) { 0.0 } else { r.noeuds[b].p[2] - anciens[b] });
        if da.abs() < 1e-4 && db.abs() < 1e-4 {
            continue;
        }
        let q = r.aretes[k].points.clone();
        let abs = abscisses(&q);
        let total = abs[abs.len() - 1];
        for (i, p) in r.aretes[k].points.iter_mut().enumerate() {
            let fa = 1.0 - lisse(abs[i] / RACCORD_PIETON);
            let fb = 1.0 - lisse((total - abs[i]) / RACCORD_PIETON);
            p[2] += da * fa + db * fb;
        }
    }
    // 6. Les greffes : une voie d'un niveau inférieur (service, chemin, piétonne) qui
    //    arrive à un nœud de route ne la modifie pas ; elle prend la hauteur de la
    //    route jusqu'à son bord (plus une marge), puis rejoint son profil par un raccord
    //    lissé
    let mut greffes: Vec<(usize, Vec<[f32; 3]>)> = Vec::new();
    for k in 0..r.aretes.len() {
        let a = &r.aretes[k];
        if a.points.len() < 2 || a.a == a.b {
            continue;
        }
        let mut pts = a.points.clone();
        let mut change = false;
        for fin in [false, true] {
            let n = if fin { a.b } else { a.a };
            if niv(r, k) >= haut[n] {
                continue;
            }
            let routes: Vec<usize> = r.noeuds[n].aretes.iter().copied().filter(|&j| j != k && compte(r, n, j)).collect();
            if routes.is_empty() {
                continue;
            }
            let w = routes.iter().map(|&j| r.aretes[j].largeur * 0.5).fold(0.0f32, f32::max) + BOUCHE_MARGE;
            // La hauteur de la route au droit d'un point : son axe le plus proche
            let route_h = |x: f32, z: f32| -> f32 {
                let mut best = (f32::MAX, r.noeuds[n].p[2]);
                for &j in &routes {
                    for s in r.aretes[j].points.windows(2) {
                        let (ux, uz) = (s[1][0] - s[0][0], s[1][1] - s[0][1]);
                        let l2 = ux * ux + uz * uz;
                        let t = if l2 > 1e-9 { (((x - s[0][0]) * ux + (z - s[0][1]) * uz) / l2).clamp(0.0, 1.0) } else { 0.0 };
                        let p = entre(s[0], s[1], t);
                        let d = (x - p[0]).hypot(z - p[1]);
                        if d < best.0 {
                            best = (d, p[2]);
                        }
                    }
                }
                best.1
            };
            let q: Vec<[f32; 3]> = if fin { pts.iter().rev().copied().collect() } else { pts.clone() };
            let abs = abscisses(&q);
            let total = abs[abs.len() - 1];
            let sb = w.min(0.4 * total);
            let hb = hauteur(&q, &abs, sb);
            let pb = {
                let i = abs.partition_point(|&y| y < sb).clamp(1, q.len() - 1);
                let t = ((sb - abs[i - 1]) / (abs[i] - abs[i - 1]).max(1e-6)).clamp(0.0, 1.0);
                entre(q[i - 1], q[i], t)
            };
            let delta = route_h(pb[0], pb[1]) - hb;
            let l = RACCORD_MIN.max(1.5 * delta.abs() / RACCORD_PENTE).min(RACCORD_MAX).min(0.5 * total - sb).max(0.0);
            let mut v: Vec<[f32; 3]> = densifier(&q, &abs, &[(0.0, sb + l)])
                .into_iter()
                .map(|x| {
                    let c = x[3];
                    let h = if c <= sb {
                        route_h(x[0], x[1])
                    } else if l > 1e-3 && c < sb + l {
                        x[2] + delta * (1.0 - lisse((c - sb) / l))
                    } else {
                        x[2]
                    };
                    [x[0], x[1], h]
                })
                .collect();
            v[0][2] = r.noeuds[n].p[2];
            if fin {
                v.reverse();
            }
            pts = v;
            change = true;
        }
        if change {
            greffes.push((k, pts));
        }
    }
    bilan.greffes = greffes.len();
    for (k, pts) in greffes {
        r.aretes[k].points = pts;
    }
    bilan
}

/// Les points d'une polyligne (x, z, h, abscisse), avec des points tous les `PAS`
/// mètres dans les intervalles d'abscisses donnés.
fn densifier(q: &[[f32; 3]], abs: &[f32], zones: &[(f32, f32)]) -> Vec<[f32; 4]> {
    let dans = |s: f32| zones.iter().any(|&(a, b)| s >= a - 1e-3 && s <= b + 1e-3);
    let mut v: Vec<[f32; 4]> = Vec::with_capacity(q.len());
    for i in 0..q.len() {
        if i > 0 {
            let (s0, s1) = (abs[i - 1], abs[i]);
            let sg = s1 - s0;
            // Les bornes des zones dans ce segment, et des points au pas
            let mut cibles: Vec<f32> = Vec::new();
            for &(a, b) in zones {
                for x in [a, b] {
                    if x > s0 + 0.05 && x < s1 - 0.05 {
                        cibles.push(x);
                    }
                }
            }
            let n = (sg / PAS).ceil() as usize;
            for t in 1..n {
                let x = s0 + sg * t as f32 / n as f32;
                if dans(x) {
                    cibles.push(x);
                }
            }
            cibles.sort_by(f32::total_cmp);
            cibles.dedup_by(|a, b| (*a - *b).abs() < 0.3);
            for x in cibles {
                let p = entre(q[i - 1], q[i], (x - s0) / sg.max(1e-6));
                v.push([p[0], p[1], p[2], x]);
            }
        }
        v.push([q[i][0], q[i][1], q[i][2], abs[i]]);
    }
    v
}
