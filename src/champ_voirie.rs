//! Les hauteurs de la voirie au sol (0.135, revu en 0.137) : la surface que lisent tous
//! les volumes de la Matérialisation (`materialisation::ruban`, `dalle`) au lieu de la
//! hauteur propre de chaque voie, pour que deux volumes qui se recouvrent lisent la
//! même hauteur au même endroit (plus de marche entre eux).
//!
//! En un point, chaque voie au sol dont l'emprise le touche donne son propre profil
//! (`materialisation::lisser` : plate en travers, pente en long lissée, posée sur les
//! plans des carrefours), au droit du point sur son axe. Son poids : plein sur sa
//! demi-largeur, puis décroissant jusqu'à zéro à `FRANGE` mètres au-delà de son bord,
//! fois son rang (la route principale l'emporte). La hauteur est leur moyenne.
//!
//! - Une voie seule garde exactement son profil : pas de dévers venu d'une voisine
//!   (0.135 mélangeait toutes les voies à 3 m au-delà de leur bord, puis floutait :
//!   deux voies proches à des hauteurs différentes penchaient l'une vers l'autre).
//! - Là où des emprises se recouvrent (carrefours, tronçons qui se suivent, couloirs
//!   de bus, bretelles), toutes les voies lisent la même moyenne.
//! - Le passage de l'un à l'autre est continu : une voisine perd son poids sur la
//!   frange.
//!
//! Les ponts et les tunnels n'y sont pas : ils gardent leur propre profil.
//!
//! Les voies sont des chaînes (0.138) : les tronçons d'une même rue enchaînés à
//! travers les nœuds qui ne relient qu'eux deux (`passage_simple` : une coupure
//! d'OSM, l'entrée d'un pont ou d'un tunnel), lissés d'un bout à l'autre, le passage
//! gardé point par point. Seuls leurs segments au sol comptent ici ; la
//! Matérialisation fait ses rubans de toute la chaîne (`chaines_dans`).

use crate::materialisation::{au_sol, code_passage, lisser, plan, rang, Pt};
use sage_reel::reseau::Reseau;
use std::collections::HashMap;

/// Au-delà du bord d'une voie, son poids décroît jusqu'à zéro sur cette distance (m).
pub const FRANGE: f32 = 1.0;
/// Une voie plus haute ou plus basse que la lectrice d'au moins `NIVEAU_PLEIN` mètres
/// perd son poids, qui est nul à `NIVEAU_NUL` : deux voies superposées (une route au
/// pied d'une autre, l'entrée d'un tunnel sous une route) ne se mélangent pas.
const NIVEAU_PLEIN: f32 = 1.0;
const NIVEAU_NUL: f32 = 2.0;
/// Le côté (m) des cases de l'index des segments.
const CASE: f32 = 16.0;

/// Une chaîne de voies : son tracé lissé (`Pt`, passage compris), sa demi-largeur, son
/// poids et sa boîte (x0, z0, x1, z1).
pub struct Chaine {
    pub points: Vec<Pt>,
    pub w: f32,
    poids: f32,
    boite: [f32; 4],
    /// Le tracé des rubans : `points` coupé aux bouches des carrefours simples
    /// (`carrefours`), et ces bouches (début, fin).
    pub trace: Vec<Pt>,
    pub bouches: [Option<crate::carrefours::Bouche>; 2],
    /// Les bouts (début, fin) qui ne sont pas prolongés : le tronc d'une fourche
    /// (`fourches`), que ses deux branches continuent bord à bord.
    pub nets: [bool; 2],
    /// Les files des tronçons à ses bouts (début, fin) : `sage_reel::Files`.
    pub files: [sage_reel::Files; 2],
    /// Ses arêtes du réseau, du début à la fin.
    pub aretes: Vec<usize>,
    /// Une voie de bus en site propre (le TEOR à Rouen) : chaussée en bitume bordeaux,
    /// unie à sa voie soeur de l'autre sens (`unir_bus`).
    pub bus: bool,
    /// Le revêtement le long de la chaîne : les points des tronçons (x, z) et celui de
    /// leur voie (`sage_reel::BITUME`…) ; un point de la chaussée prend celui du plus
    /// proche. Vide : tout en bitume.
    pub revetements: Vec<([f32; 2], u8)>,
}

/// L'en-tête du champ dans le cache (changé si son format change).
const CACHE_MAGIE: &[u8; 8] = b"FLCHAMP1";

pub struct Champ {
    voies: Vec<Chaine>,
    /// Les segments au sol (voie, premier point) dont la zone d'influence touche
    /// chaque case.
    index: HashMap<(i32, i32), Vec<(u32, u32)>>,
    /// Les chaînes dont la boîte touche chaque grande case (`CASE_CHAINES`).
    chaines: HashMap<(i32, i32), Vec<u32>>,
    /// Les carrefours simples, d'une seule pièce (`carrefours`).
    carrefours: Vec<crate::carrefours::Carrefour>,
    simples: std::collections::HashSet<usize>,
    /// Les nœuds des fourches (`fourches`) : sans dalle.
    fourches: std::collections::HashSet<usize>,
    deports: Vec<Deport>,
}

/// Une branche de fourche déportée : ses arêtes du réseau près du nœud `c`, décalées de
/// `decal` au nœud, en douceur jusqu'à rien à `l` mètres (le tracé en surimpression).
#[derive(Clone, Debug)]
pub struct Deport {
    pub aretes: Vec<usize>,
    pub c: [f32; 2],
    pub decal: [f32; 2],
    pub l: f32,
}

impl Deport {
    /// Le décalage d'un point (x, z) : selon sa distance au nœud.
    pub fn en(&self, p: [f32; 2]) -> [f32; 2] {
        let s = (p[0] - self.c[0]).hypot(p[1] - self.c[1]);
        if s >= self.l {
            return [0.0, 0.0];
        }
        let u = s / self.l;
        let k = 1.0 - u * u * (3.0 - 2.0 * u);
        [self.decal[0] * k, self.decal[1] * k]
    }
}

/// Le côté (m) des cases de l'index des chaînes.
const CASE_CHAINES: f32 = 64.0;
/// L'écart de largeur (m) en deçà duquel deux tronçons s'enchaînent toujours.
const LARGEUR_ECART: f32 = 0.6;
/// Au-delà, jusqu'à cet écart (m), ils s'enchaînent si leurs rangs sont voisins (une
/// même rue qui change de largeur) : la largeur passe de l'une à l'autre en biseau
/// (0.142 ; avant, deux rubans prolongés et une dalle : marche, éclats).
const BISEAU_ECART_MAX: f32 = 5.0;
/// La longueur du biseau (m) par mètre d'écart de largeur.
const BISEAU_PAR_METRE: f32 = 3.0;
/// Deux chaînes parallèles qui se recouvrent (les chaussées séparées d'un boulevard,
/// chacune large de toutes ses voies) : leurs demi-largeurs réduites pour laisser ce
/// jeu (m) entre leurs bords, d'au plus `SERRAGE_MAX`, en pente douce.
const JEU: f32 = 0.3;
const SERRAGE_MAX: f32 = 0.5;
/// Parallèles : à moins de tant de degrés (dans un sens ou l'autre).
const PARALLELES: f32 = 20.0;
/// Deux voies de bus parallèles (les deux sens d'un site propre) dont les bords sont à
/// moins de `UNIR_MAX` mètres : une seule chaussée, chacune élargie vers l'autre jusqu'au
/// milieu de l'écart, plus `JOINT` de recouvrement.
const UNIR_MAX: f32 = 2.5;
const JOINT: f32 = 0.15;
/// Le pas (m) des points des voies de bus, pour suivre l'écart à la voie soeur.
const PAS_BUS: f32 = 2.0;

/// Les routes (sans les piétonnes) d'un nœud.
fn routes(r: &Reseau, n: usize) -> Vec<usize> {
    r.noeuds[n].aretes.iter().copied().filter(|&k| r.aretes[k].genre != sage_reel::PIETONNE).collect()
}

/// Si le nœud `n` ne relie que deux tronçons d'une même rue (une coupure d'OSM,
/// l'entrée d'un pont ou d'un tunnel, un changement de largeur) : les deux, qui s'y
/// enchaînent.
pub fn passage_simple(r: &Reseau, n: usize) -> Option<(usize, usize)> {
    let v = routes(r, n);
    if v.len() != 2 || v[0] == v[1] {
        return None;
    }
    let (a, b) = (&r.aretes[v[0]], &r.aretes[v[1]]);
    // Une voie de bus ne s'enchaîne qu'à une voie de bus (sa chaussée est bordeaux)
    if (a.genre == sage_reel::BUS) != (b.genre == sage_reel::BUS) {
        return None;
    }
    let e = (a.largeur - b.largeur).abs();
    (e < LARGEUR_ECART || (e <= BISEAU_ECART_MAX && rang(a.genre).abs_diff(rang(b.genre)) <= 1)).then_some((v[0], v[1]))
}

/// Les chaînes du réseau : (arêtes dans l'ordre, chacune à l'endroit ou non, nœud de
/// départ, nœud d'arrivée).
fn enchainer(r: &Reseau) -> Vec<(Vec<(usize, bool)>, usize, usize)> {
    let mut vu = vec![false; r.aretes.len()];
    let mut out = Vec::new();
    // L'autre bout d'une arête
    let autre = |k: usize, n: usize| if r.aretes[k].a == n { r.aretes[k].b } else { r.aretes[k].a };
    for k0 in 0..r.aretes.len() {
        if vu[k0] || r.aretes[k0].genre == sage_reel::PIETONNE || r.aretes[k0].points.len() < 2 {
            continue;
        }
        vu[k0] = true;
        let mut suite: std::collections::VecDeque<(usize, bool)> = std::collections::VecDeque::from([(k0, true)]);
        // Vers l'arrière depuis a, puis vers l'avant depuis b
        let mut debut = r.aretes[k0].a;
        while let Some((x, y)) = passage_simple(r, debut) {
            let k = if x == suite[0].0 { y } else { x };
            if vu[k] || r.aretes[k].points.len() < 2 {
                break;
            }
            vu[k] = true;
            // À l'endroit si elle finit au nœud de début
            suite.push_front((k, r.aretes[k].b == debut));
            debut = autre(k, debut);
        }
        let mut fin = r.aretes[k0].b;
        while let Some((x, y)) = passage_simple(r, fin) {
            let k = if x == suite[suite.len() - 1].0 { y } else { x };
            if vu[k] || r.aretes[k].points.len() < 2 {
                break;
            }
            vu[k] = true;
            suite.push_back((k, r.aretes[k].a == fin));
            fin = autre(k, fin);
        }
        out.push((suite.into_iter().collect(), debut, fin));
    }
    out
}

/// Le poids d'une voie selon son rang : la principale l'emporte nettement.
fn poids(genre: u8) -> f32 {
    match rang(genre) {
        0 => 8.0,
        1 => 4.0,
        2 => 2.0,
        _ => 1.0,
    }
}

/// Les largeurs d'une chaîne en biseau à ses changements de largeur (`biseaux` : point
/// commun, demi-largeur avant, après) : sur `BISEAU_PAR_METRE` mètres par mètre
/// d'écart, centrés sur le point commun (au plus 45 % des tronçons voisins), des
/// points ajoutés aux deux bouts du biseau.
fn biseauter(pts: &mut Vec<[f32; 3]>, codes: &mut Vec<f32>, ws: &mut Vec<f32>, biseaux: &[(usize, f32, f32)]) {
    let abscisses = |p: &[[f32; 3]]| {
        let mut s = vec![0.0f32; p.len()];
        for i in 1..p.len() {
            s[i] = s[i - 1] + (p[i][0] - p[i - 1][0]).hypot(p[i][1] - p[i - 1][1]);
        }
        s
    };
    let s = abscisses(pts);
    let l = s[s.len() - 1];
    // Chaque biseau : (centre, demi-longueur, largeur avant, après), borné par les
    // biseaux voisins et les bouts
    let mut v: Vec<(f32, f32, f32, f32)> = Vec::new();
    for (j, &(i, wa, wb)) in biseaux.iter().enumerate() {
        let avant = if j > 0 { s[i] - s[biseaux[j - 1].0] } else { s[i] };
        let apres = if j + 1 < biseaux.len() { s[biseaux[j + 1].0] - s[i] } else { l - s[i] };
        let demi = (BISEAU_PAR_METRE * 2.0 * (wb - wa).abs() / 2.0).min(0.45 * avant).min(0.45 * apres);
        if demi > 0.05 {
            v.push((s[i], demi, wa, wb));
        }
    }
    // Les points ajoutés aux bouts des biseaux
    let mut coupes: Vec<f32> = v.iter().flat_map(|b| [b.0 - b.1, b.0 + b.1]).collect();
    coupes.sort_by(f32::total_cmp);
    let (mut p2, mut c2, mut w2) = (Vec::with_capacity(pts.len() + coupes.len()), Vec::with_capacity(pts.len() + coupes.len()), Vec::with_capacity(pts.len() + coupes.len()));
    let mut k = 0;
    for i in 0..pts.len() {
        if i > 0 {
            while k < coupes.len() && coupes[k] < s[i] {
                let c = coupes[k];
                k += 1;
                if c <= s[i - 1] + 0.05 || c >= s[i] - 0.05 {
                    continue;
                }
                let t = (c - s[i - 1]) / (s[i] - s[i - 1]);
                p2.push(std::array::from_fn(|q| pts[i - 1][q] + t * (pts[i][q] - pts[i - 1][q])));
                c2.push(codes[i - 1].max(codes[i]));
                w2.push(ws[i]);
            }
        }
        p2.push(pts[i]);
        c2.push(codes[i]);
        w2.push(ws[i]);
    }
    let s2 = abscisses(&p2);
    for (x, w) in s2.iter().zip(w2.iter_mut()) {
        for &(c, demi, wa, wb) in &v {
            if (x - c).abs() < demi {
                *w = wa + (wb - wa) * (x - (c - demi)) / (2.0 * demi);
            }
        }
    }
    *pts = p2;
    *codes = c2;
    *ws = w2;
}

/// Les chaussées parallèles qui se recouvrent, desserrées : chaque point au sol dont
/// l'emprise mord sur celle d'une chaîne parallèle au même niveau voit sa demi-largeur
/// réduite au prorata (`JEU` entre les bords), puis la réduction étalée en biseau. Près
/// d'un nœud commun aux deux chaînes (où elles se rejoignent), rien.
fn desserrer(voies: &mut [Chaine], bouts: &[(usize, usize)], fourches: &[(usize, [f32; 2], f32)]) {
    let cos_min = PARALLELES.to_radians().cos();
    let mut index: HashMap<(i32, i32), Vec<(u32, u32)>> = HashMap::new();
    for (k, v) in voies.iter().enumerate() {
        for (i, s) in v.points.windows(2).enumerate() {
            if !au_sol(&s[0]) || !au_sol(&s[1]) {
                continue;
            }
            let (a, b) = (case(s[0][0].min(s[1][0]), s[0][1].min(s[1][1])), case(s[0][0].max(s[1][0]), s[0][1].max(s[1][1])));
            for ci in a.0..=b.0 {
                for cj in a.1..=b.1 {
                    index.entry((ci, cj)).or_default().push((k as u32, i as u32));
                }
            }
        }
    }
    let mut nouvelles: Vec<(usize, Vec<f32>)> = Vec::new();
    for (k, v) in voies.iter().enumerate() {
        let p = &v.points;
        let n = p.len();
        if n < 2 {
            continue;
        }
        let mut ws: Vec<f32> = p.iter().map(|q| q[6]).collect();
        let mut touche = false;
        for i in 0..n {
            let q = p[i];
            if !au_sol(&q) {
                continue;
            }
            let (a, b) = (p[i.saturating_sub(1)], p[(i + 1).min(n - 1)]);
            let t = { let (x, z) = (b[0] - a[0], b[1] - a[1]); let l = x.hypot(z).max(1e-6); [x / l, z / l] };
            let c = case(q[0], q[1]);
            let mut f = 1.0f32;
            for ci in c.0 - 1..=c.0 + 1 {
                for cj in c.1 - 1..=c.1 + 1 {
                    let Some(l) = index.get(&(ci, cj)) else { continue };
                    for &(k2, j) in l {
                        let k2 = k2 as usize;
                        // Deux voies de bus se recouvrent : elles font une seule chaussée
                        if k2 == k || (v.bus && voies[k2].bus) {
                            continue;
                        }
                        let (u, w) = (voies[k2].points[j as usize], voies[k2].points[j as usize + 1]);
                        if (u[2] - q[2]).abs().min((w[2] - q[2]).abs()) > NIVEAU_NUL {
                            continue;
                        }
                        let (x, z) = (w[0] - u[0], w[1] - u[1]);
                        let l2 = x * x + z * z;
                        if l2 < 1e-6 || ((x * t[0] + z * t[1]) / l2.sqrt()).abs() < cos_min {
                            continue;
                        }
                        let s = (((q[0] - u[0]) * x + (q[1] - u[1]) * z) / l2).clamp(0.0, 1.0);
                        let d = (q[0] - u[0] - s * x).hypot(q[1] - u[1] - s * z);
                        let w2 = u[6].max(w[6]);
                        if d >= q[6] + w2 + JEU {
                            continue;
                        }
                        // Pas là où les deux chaînes se rejoignent
                        let (a1, b1) = bouts[k];
                        let (a2, b2) = bouts[k2];
                        let commun = [a1, b1].iter().filter(|n| **n == a2 || **n == b2).count() > 0;
                        if commun {
                            // Les deux branches d'une fourche, près de son nœud : bord à
                            // bord, elles font une seule chaussée
                            let branches = fourches.iter().any(|&(nf, c, l)| [a1, b1].contains(&nf) && [a2, b2].contains(&nf) && (q[0] - c[0]).hypot(q[1] - c[1]) < l + 5.0);
                            if branches {
                                continue;
                            }
                            let pres = [p[0], p[n - 1]].iter().any(|e| (e[0] - q[0]).hypot(e[1] - q[1]) < q[6] + w2);
                            if pres {
                                continue;
                            }
                        }
                        f = f.min(((d - JEU).max(0.0) / (q[6] + w2)).max(1.0 - SERRAGE_MAX));
                    }
                }
            }
            if f < 0.999 {
                ws[i] = q[6] * f;
                touche = true;
            }
        }
        if !touche {
            continue;
        }
        // La réduction étalée : une pente d'au plus 1 m de largeur par BISEAU_PAR_METRE
        let pente = 0.5 / BISEAU_PAR_METRE;
        for i in 1..n {
            let ds = (p[i][0] - p[i - 1][0]).hypot(p[i][1] - p[i - 1][1]);
            ws[i] = ws[i].min(ws[i - 1] + ds * pente);
        }
        for i in (0..n - 1).rev() {
            let ds = (p[i + 1][0] - p[i][0]).hypot(p[i + 1][1] - p[i][1]);
            ws[i] = ws[i].min(ws[i + 1] + ds * pente);
        }
        nouvelles.push((k, ws));
    }
    for (k, ws) in nouvelles {
        for (q, w) in voies[k].points.iter_mut().zip(ws) {
            q[6] = w;
        }
    }
}

/// Une fourche : un nœud au sol où finissent trois chaînes, une large (le tronc) et deux
/// pas plus larges que lui (l'une plus étroite) qui en partent presque dans le même sens
/// (une chaussée qui se partage, ou une voie qui continue et une bretelle qui s'en
/// détache), à moins de `FOURCHE_ANGLE`
/// degrés l'une de l'autre (une chaussée à deux files qui se partage en deux voies). Les
/// deux branches y partent bord à bord : chacune décalée vers son côté pour que son bord
/// extérieur prolonge celui du tronc (élargie s'il le faut pour que les deux le
/// couvrent), le décalage s'effaçant en douceur sur `FOURCHE_L_MIN` à `FOURCHE_L_MAX`
/// mètres ; le bout du tronc n'est plus prolongé (`Chaine::nets`). Avant, le tronc
/// finissait en bout carré et les branches partaient de son axe, rétrécies l'une contre
/// l'autre. Rend (nœud, centre, longueur du décalage) de chacune.
const FOURCHE_ANGLE: f32 = 45.0;
const FOURCHE_L_MIN: f32 = 15.0;
const FOURCHE_L_MAX: f32 = 40.0;

fn fourches(r: &Reseau, voies: &mut [Chaine], bouts: &[(usize, usize)], deports: &mut Vec<Deport>, plaque: &mut dyn FnMut(&[Chaine], usize, (usize, bool), (usize, bool), (usize, bool)) -> Option<f32>) -> Vec<(usize, [f32; 2], f32)> {
    let mut aux: HashMap<usize, Vec<(usize, bool)>> = HashMap::new();
    for (k, &(a, b)) in bouts.iter().enumerate() {
        aux.entry(a).or_default().push((k, true));
        aux.entry(b).or_default().push((k, false));
    }
    let longueur = |p: &[Pt]| p.windows(2).map(|s| (s[1][0] - s[0][0]).hypot(s[1][1] - s[0][1])).sum::<f32>();
    let mut v = Vec::new();
    let mut prises = std::collections::HashSet::new();
    // Les fourches : par les largeurs, par les files déduites, par les files lues
    let mut bilan = [0usize; 4];
    let mut noeuds: Vec<usize> = aux.keys().copied().collect();
    noeuds.sort_unstable();
    // FLOW_FOURCHE="x z" : pourquoi le nœud le plus proche est (ou n'est pas) une fourche
    let suivi: Option<[f32; 2]> = std::env::var("FLOW_FOURCHE").ok().and_then(|v| {
        let mut it = v.split_whitespace().filter_map(|x| x.parse().ok());
        Some([it.next()?, it.next()?])
    });
    let suivi = suivi.and_then(|p| noeuds.iter().copied().min_by(|&a, &b| {
        let d = |n: usize| (r.noeuds[n].p[0] - p[0]).hypot(r.noeuds[n].p[1] - p[1]);
        d(a).total_cmp(&d(b))
    }));
    for n in noeuds {
        let bs = &aux[&n];
        let rs = routes(r, n);
        if suivi == Some(n) {
            eprintln!("fourche ? nœud {n} en {:?} : {} bouts de chaînes {:?}, {} routes, passages {:?}", r.noeuds[n].p, bs.len(), bs, rs.len(), rs.iter().map(|&k| r.aretes[k].passage).collect::<Vec<_>>());
            for &(k, debut) in bs {
                let p = if debut { voies[k].points.first() } else { voies[k].points.last() };
                eprintln!("  chaîne {k} ({}) : bout {:?}, {} points, prise {}", if debut { "début" } else { "fin" }, p.map(|q| [q[0], q[1], q[5], q[6]]), voies[k].points.len(), prises.contains(&(k, debut)));
            }
        }
        if bs.len() != 3 || rs.len() != 3 || rs.iter().any(|&k| r.aretes[k].passage != sage_reel::reseau::Passage::Sol) || bs.iter().any(|b| prises.contains(b)) {
            continue;
        }
        // Les bouts au nœud : leurs points depuis le nœud, leur direction (vers 15 m),
        // leur demi-largeur
        let c = [r.noeuds[n].p[0], r.noeuds[n].p[1]];
        let mut bras = Vec::new();
        for &(k, debut) in bs {
            let p: Vec<Pt> = if debut { voies[k].points.clone() } else { voies[k].points.iter().rev().copied().collect() };
            if p.len() < 2 || (p[0][0] - c[0]).hypot(p[0][1] - c[1]) > 0.5 || !au_sol(&p[0]) {
                break;
            }
            let mut s = 0.0;
            let mut q = p[1];
            for i in 1..p.len() {
                s += (p[i][0] - p[i - 1][0]).hypot(p[i][1] - p[i - 1][1]);
                q = p[i];
                if s >= 15.0 {
                    break;
                }
            }
            let (x, z) = (q[0] - p[0][0], q[1] - p[0][1]);
            let l = x.hypot(z).max(1e-6);
            bras.push((k, debut, [x / l, z / l], p[0][6], longueur(&p)));
        }
        if suivi == Some(n) {
            eprintln!("  {} bras retenus", bras.len());
        }
        if bras.len() != 3 || bras[0].0 == bras[1].0 || bras[1].0 == bras[2].0 || bras[0].0 == bras[2].0 {
            continue;
        }
        let dot = |a: [f32; 2], b: [f32; 2]| a[0] * b[0] + a[1] * b[1];
        let cos_max = FOURCHE_ANGLE.to_radians().cos();
        let Some(t) = (0..3).find(|&t| {
            let (b1, b2) = (&bras[(t + 1) % 3], &bras[(t + 2) % 3]);
            dot(b1.2, b2.2) > cos_max && dot(bras[t].2, b1.2) < -0.7 && dot(bras[t].2, b2.2) < -0.7 && bras[t].3 + 0.3 >= b1.3.max(b2.3) && bras[t].3 + 0.3 >= b1.3.min(b2.3) + 0.5
        }) else {
            if suivi == Some(n) {
                eprintln!("  pas de tronc : bras {:?}", bras.iter().map(|b| (b.0, b.2, b.3)).collect::<Vec<_>>());
            }
            continue;
        };
        if suivi == Some(n) {
            eprintln!("  bras {:?}", bras.iter().map(|b| (b.0, b.2, b.3, b.4)).collect::<Vec<_>>());
        }
        let tronc = bras[t];
        let f = [-tronc.2[0], -tronc.2[1]];
        let g = [-f[1], f[0]];
        let (mut b1, mut b2) = (bras[(t + 1) % 3], bras[(t + 2) % 3]);
        let cote = |d: [f32; 2]| f[0] * d[1] - f[1] * d[0];
        if cote(b1.2) < cote(b2.2) {
            std::mem::swap(&mut b1, &mut b2);
        }
        // En plaque, si le nez se trouve (0.176) : rien n'est décalé
        if let Some(s_nez) = plaque(voies, n, (tronc.0, tronc.1), (b1.0, b1.1), (b2.0, b2.1)) {
            if suivi == Some(n) {
                eprintln!("  fourche en plaque : tronc {}, branches {} et {}, nez à {s_nez:.1} m", tronc.0, b1.0, b2.0);
            }
            for b in [tronc, b1, b2] {
                prises.insert((b.0, b.1));
            }
            bilan[3] += 1;
            v.push((n, c, s_nez));
            continue;
        }
        // b1 à gauche (vers +g), b2 à droite ; élargies pour couvrir le tronc
        let w = tronc.3;
        // Les files du tronc (lues ou déduites) : la branche qui dévie (le moins de
        // files ; à égalité, la plus écartée du tronc) part du centre des files du bord
        // de son côté, la branche qui continue du centre des autres. Sans files (moins
        // de 2 au tronc) : les bords extérieurs des branches prolongent ceux du tronc
        let fichier = |b: &(usize, bool, [f32; 2], f32, f32)| voies[b.0].files[if b.1 { 0 } else { 1 }];
        let (ft, f1, f2) = (fichier(&tronc), fichier(&b1), fichier(&b2));
        let t_files = ft.total as usize;
        let (o1, o2, w1, w2) = if t_files >= 2 {
            let lw = 2.0 * w / t_files as f32;
            let devie1 = match f1.total.cmp(&f2.total) {
                std::cmp::Ordering::Less => true,
                std::cmp::Ordering::Greater => false,
                std::cmp::Ordering::Equal => dot(f, b1.2) < dot(f, b2.2),
            };
            let fd = if devie1 { f1.total } else { f2.total } as usize;
            let kd = fd.clamp(1, t_files - 1);
            let kc = t_files - kd;
            let (k1, k2) = if devie1 { (kd, kc) } else { (kc, kd) };
            let demi = |k: usize| k as f32 * lw / 2.0;
            (w - demi(k1), w - demi(k2), demi(k1), demi(k2))
        } else {
            let e = (w - b1.3 - b2.3).max(0.0);
            let (w1, w2) = if e > 0.0 { (b1.3 + e / 2.0 + 0.05, b2.3 + e / 2.0 + 0.05) } else { (b1.3, b2.3) };
            (w - w1, w - w2, w1, w2)
        };
        bilan[if t_files < 2 { 0 } else if ft.osm { 2 } else { 1 }] += 1;
        let l = (5.0 * o1.max(o2)).clamp(FOURCHE_L_MIN, FOURCHE_L_MAX).min(0.45 * b1.4.min(b2.4));
        if suivi == Some(n) {
            eprintln!("  fourche : tronc {} (w {w}, {:?}), branches {} ({:?}) et {} ({:?}) → décalages {o1:.2} {o2:.2}, demi-largeurs {w1:.2} {w2:.2}, l {l:.1}", tronc.0, ft, b1.0, f1, b2.0, f2);
        }
        if l < 6.0 {
            continue;
        }
        for (b, sens, o, wn) in [(b1, 1.0f32, o1, w1), (b2, -1.0, o2, w2)] {
            let (k, debut) = (b.0, b.1);
            let mut p: Vec<Pt> = if debut { voies[k].points.clone() } else { voies[k].points.iter().rev().copied().collect() };
            // Des points tous les 2 m sur la longueur du décalage
            let mut q: Vec<Pt> = vec![p[0]];
            let mut s = 0.0f32;
            for i in 1..p.len() {
                let d = (p[i][0] - p[i - 1][0]).hypot(p[i][1] - p[i - 1][1]);
                if s < l {
                    let m = (d / 2.0).ceil().max(1.0) as usize;
                    for j in 1..m {
                        let u = j as f32 / m as f32;
                        q.push(std::array::from_fn(|c| p[i - 1][c] + u * (p[i][c] - p[i - 1][c])));
                    }
                }
                q.push(p[i]);
                s += d;
            }
            p = q;
            let mut s = 0.0f32;
            for i in 0..p.len() {
                if i > 0 {
                    s += (p[i][0] - p[i - 1][0]).hypot(p[i][1] - p[i - 1][1]);
                }
                if s >= l {
                    break;
                }
                let u = s / l;
                let k1 = 1.0 - u * u * (3.0 - 2.0 * u);
                p[i][0] += g[0] * sens * o * k1;
                p[i][1] += g[1] * sens * o * k1;
                p[i][6] += (wn - p[i][6]) * k1;
            }
            // Le déport, pour la surimpression : les arêtes du réseau sur `l` depuis le
            // nœud
            let ch = &voies[k].aretes;
            let mut aretes = Vec::new();
            let mut s = 0.0f32;
            let ordre: Vec<usize> = if debut { ch.clone() } else { ch.iter().rev().copied().collect() };
            for a in ordre {
                aretes.push(a);
                s += r.aretes[a].points.windows(2).map(|q| (q[1][0] - q[0][0]).hypot(q[1][1] - q[0][1])).sum::<f32>();
                if s >= l {
                    break;
                }
            }
            deports.push(Deport { aretes, c, decal: [g[0] * sens * o, g[1] * sens * o], l });
            if !debut {
                p.reverse();
            }
            let v = &mut voies[k];
            v.points = p;
            v.w = v.w.max(wn);
            for q in &v.points {
                v.boite = [v.boite[0].min(q[0]), v.boite[1].min(q[1]), v.boite[2].max(q[0]), v.boite[3].max(q[1])];
            }
            prises.insert((k, debut));
        }
        voies[tronc.0].nets[if tronc.1 { 0 } else { 1 }] = true;
        prises.insert((tronc.0, tronc.1));
        v.push((n, c, l));
    }
    if std::env::var("FLOW_FOURCHE").is_ok() {
        eprintln!("plaques refusées (bras, tronc court, place, plan, croisé, envers, maillage) : {:?}", crate::carrefours::ECHECS.lock().map(|e| *e).unwrap_or_default());
        eprintln!("fourches : {} faites ; en plaque {} ; décalées par les largeurs {}, par les files déduites {}, lues dans OSM {}", v.len(), bilan[3], bilan[0], bilan[1], bilan[2]);
    }
    v
}

/// Les points d'un tracé redécoupés au plus tous les `pas` mètres (le passage du plus
/// haut des deux bouts).
fn redecouper(p: &[Pt], pas: f32) -> Vec<Pt> {
    let mut v = Vec::with_capacity(p.len());
    for (i, &q) in p.iter().enumerate() {
        if i > 0 {
            let a = p[i - 1];
            let l = (q[0] - a[0]).hypot(q[1] - a[1]);
            let n = (l / pas).ceil() as usize;
            for j in 1..n {
                let t = j as f32 / n as f32;
                let mut m: Pt = std::array::from_fn(|c| a[c] + t * (q[c] - a[c]));
                m[5] = a[5].max(q[5]);
                v.push(m);
            }
        }
        v.push(q);
    }
    v
}

/// Les deux sens d'un site propre de bus en une seule chaussée : chaque point au sol
/// d'une voie de bus dont une voie de bus parallèle, au même niveau, a son bord à moins
/// de `UNIR_MAX` mètres du sien est déplacé vers elle et élargi d'autant (son bord
/// extérieur reste en place, l'intérieur va jusqu'au milieu de l'écart plus `JOINT`),
/// l'élargissement étalé en pente douce. Deux voies qui se recouvrent déjà restent
/// telles quelles : le champ leur donne la même hauteur.
fn unir_bus(voies: &mut [Chaine]) {
    for v in voies.iter_mut().filter(|v| v.bus) {
        v.points = redecouper(&v.points, PAS_BUS);
    }
    let cos_min = PARALLELES.to_radians().cos();
    let mut index: HashMap<(i32, i32), Vec<(u32, u32)>> = HashMap::new();
    for (k, v) in voies.iter().enumerate().filter(|(_, v)| v.bus) {
        for (i, s) in v.points.windows(2).enumerate() {
            if !au_sol(&s[0]) || !au_sol(&s[1]) {
                continue;
            }
            let (a, b) = (case(s[0][0].min(s[1][0]), s[0][1].min(s[1][1])), case(s[0][0].max(s[1][0]), s[0][1].max(s[1][1])));
            for ci in a.0..=b.0 {
                for cj in a.1..=b.1 {
                    index.entry((ci, cj)).or_default().push((k as u32, i as u32));
                }
            }
        }
    }
    let mut nouvelles: Vec<(usize, Vec<f32>, Vec<[f32; 2]>)> = Vec::new();
    for (k, v) in voies.iter().enumerate().filter(|(_, v)| v.bus) {
        let p = &v.points;
        let n = p.len();
        if n < 2 {
            continue;
        }
        // L'élargissement de chaque point et la direction de la voie soeur
        let mut es = vec![0.0f32; n];
        let mut dirs = vec![[0.0f32; 2]; n];
        for i in 0..n {
            let q = p[i];
            if !au_sol(&q) {
                continue;
            }
            let (a, b) = (p[i.saturating_sub(1)], p[(i + 1).min(n - 1)]);
            let t = { let (x, z) = (b[0] - a[0], b[1] - a[1]); let l = x.hypot(z).max(1e-6); [x / l, z / l] };
            let c = case(q[0], q[1]);
            // La voie soeur la plus proche : (écart des bords, distance, direction)
            let mut soeur: Option<(f32, f32, [f32; 2])> = None;
            for ci in c.0 - 1..=c.0 + 1 {
                for cj in c.1 - 1..=c.1 + 1 {
                    let Some(l) = index.get(&(ci, cj)) else { continue };
                    for &(k2, j) in l {
                        if k2 as usize == k {
                            continue;
                        }
                        let (u, w) = (voies[k2 as usize].points[j as usize], voies[k2 as usize].points[j as usize + 1]);
                        if (u[2] - q[2]).abs().min((w[2] - q[2]).abs()) > NIVEAU_NUL {
                            continue;
                        }
                        let (x, z) = (w[0] - u[0], w[1] - u[1]);
                        let l2 = x * x + z * z;
                        if l2 < 1e-6 || ((x * t[0] + z * t[1]) / l2.sqrt()).abs() < cos_min {
                            continue;
                        }
                        let s = (((q[0] - u[0]) * x + (q[1] - u[1]) * z) / l2).clamp(0.0, 1.0);
                        let (dx, dz) = (u[0] + s * x - q[0], u[1] + s * z - q[1]);
                        let d = dx.hypot(dz);
                        let ecart = d - q[6] - (u[6] + s * (w[6] - u[6]));
                        if d < 1e-3 || ecart > UNIR_MAX || soeur.is_some_and(|e| e.1 <= d) {
                            continue;
                        }
                        soeur = Some((ecart, d, [dx / d, dz / d]));
                    }
                }
            }
            if let Some((ecart, _, dir)) = soeur {
                if ecart > 0.0 {
                    es[i] = 0.5 * ecart + JOINT;
                    dirs[i] = dir;
                }
            }
        }
        if es.iter().all(|&e| e == 0.0) {
            continue;
        }
        // L'élargissement retombe en pente douce là où la voie soeur s'écarte
        let pente = 0.5 / BISEAU_PAR_METRE;
        for i in 1..n {
            let ds = (p[i][0] - p[i - 1][0]).hypot(p[i][1] - p[i - 1][1]);
            es[i] = es[i].min(es[i - 1] + ds * pente);
        }
        for i in (0..n - 1).rev() {
            let ds = (p[i + 1][0] - p[i][0]).hypot(p[i + 1][1] - p[i][1]);
            es[i] = es[i].min(es[i + 1] + ds * pente);
        }
        nouvelles.push((k, es, dirs));
    }
    for (k, es, dirs) in nouvelles {
        let v = &mut voies[k];
        for ((q, e), d) in v.points.iter_mut().zip(es).zip(dirs) {
            if e <= 0.0 {
                continue;
            }
            // L'axe avance de la moitié de l'élargissement vers la voie soeur, la
            // demi-largeur en prend autant : le bord extérieur ne bouge pas
            let o = [0.5 * e * d[0], 0.5 * e * d[1]];
            q[2] += q[3] * o[0] + q[4] * o[1];
            q[0] += o[0];
            q[1] += o[1];
            q[6] += 0.5 * e;
            v.w = v.w.max(q[6]);
        }
        let mut boite = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
        for p in &v.points {
            boite = [boite[0].min(p[0]), boite[1].min(p[1]), boite[2].max(p[0]), boite[3].max(p[1])];
        }
        v.boite = boite;
    }
}

fn case(x: f32, z: f32) -> (i32, i32) {
    ((x / CASE).floor() as i32, (z / CASE).floor() as i32)
}

impl Champ {
    /// Les chaînes de voies du réseau nivelé (sans les piétonnes), lissées.
    pub fn new(r: &Reseau) -> Champ {
        let mut t_chaines = crate::suivi::tache("chaînes (lissage)");
        let mut voies = Vec::new();
        let mut bouts = Vec::new();
        for (suite, debut, fin) in enchainer(r) {
            bouts.push((debut, fin));
            let (mut pts, mut codes, mut ws): (Vec<[f32; 3]>, Vec<f32>, Vec<f32>) = (Vec::new(), Vec::new(), Vec::new());
            let (mut w, mut pd) = (0.0f32, 0.0f32);
            let bus = suite.iter().all(|&(k, _)| r.aretes[k].genre == sage_reel::BUS);
            let revetements: Vec<([f32; 2], u8)> = if suite.iter().all(|&(k, _)| r.aretes[k].revetement == sage_reel::BITUME) {
                Vec::new()
            } else {
                suite.iter().flat_map(|&(k, _)| r.aretes[k].points.iter().map(move |p| ([p[0], p[1]], r.aretes[k].revetement))).collect()
            };
            // Les changements de largeur : (point commun, largeur avant, après)
            let mut biseaux: Vec<(usize, f32, f32)> = Vec::new();
            for &(k, endroit) in &suite {
                let a = &r.aretes[k];
                let c = code_passage(a.passage);
                let wa = (a.largeur * 0.5).max(1.0);
                let l: Vec<[f32; 3]> = if endroit { a.points.clone() } else { a.points.iter().rev().copied().collect() };
                // Le point commun : au sol si l'un des deux l'est (la fin d'une rampe)
                let saute = usize::from(!pts.is_empty());
                if saute == 1 {
                    let d = codes.len() - 1;
                    codes[d] = codes[d].min(c);
                    if (ws[d] - wa).abs() > 1e-3 {
                        biseaux.push((d, ws[d], wa));
                    }
                }
                pts.extend_from_slice(&l[saute..]);
                codes.extend(std::iter::repeat_n(c, l.len() - saute));
                ws.extend(std::iter::repeat_n(wa, l.len() - saute));
                w = w.max(wa);
                pd = pd.max(poids(a.genre));
            }
            if !biseaux.is_empty() {
                biseauter(&mut pts, &mut codes, &mut ws, &biseaux);
            }
            let points = lisser(&pts, &codes, &ws, plan(r, debut), plan(r, fin));
            let mut boite = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
            for p in &points {
                boite = [boite[0].min(p[0]), boite[1].min(p[1]), boite[2].max(p[0]), boite[3].max(p[1])];
            }
            voies.push(Chaine { points, w, poids: pd, boite, trace: Vec::new(), bouches: [None, None], nets: [false, false], files: [r.aretes[suite[0].0].files, r.aretes[suite[suite.len() - 1].0].files], aretes: suite.iter().map(|x| x.0).collect(), bus, revetements });
        }
        t_chaines.detail(format!("{} chaînes", voies.len()));
        drop(t_chaines);
        let t_fourches = crate::suivi::tache("fourches");
        let mut deports = Vec::new();
        // Les fourches en plaque d'abord ; les autres, branches décalées
        let mut plaques_fourches = Vec::new();
        let mut coupees: HashMap<(usize, bool), f32> = HashMap::new();
        let fourches = fourches(r, &mut voies, &bouts, &mut deports, &mut |voies, n, t, b1, b2| {
            let (car, coupes) = crate::carrefours::plaque_fourche(r, voies, n, t, b1, b2, &|k, debut| coupees.get(&(k, debut)).copied().unwrap_or(crate::carrefours::AUTRE_FOURCHE))?;
            for c in &coupes {
                coupees.insert((c.0, c.1), c.2);
            }
            let s = coupes.iter().map(|c| c.2).fold(0.0f32, f32::max);
            plaques_fourches.push((car, coupes));
            Some(s)
        });
        drop(t_fourches);
        let t_desserrer = crate::suivi::tache("desserrer, bus");
        desserrer(&mut voies, &bouts, &fourches);
        unir_bus(&mut voies);
        drop(t_desserrer);
        let t_carrefours = crate::suivi::tache("carrefours");
        // Les carrefours simples : les chaînes coupées à leurs bouches
        let deja: Vec<_> = plaques_fourches.iter().flat_map(|p| p.1.iter().copied()).collect();
        let (mut carrefours, mut coupes, absorbees) = crate::carrefours::construire(r, &voies, &bouts, &fourches.iter().map(|f| f.0).collect(), &deja);
        for (car, cs) in plaques_fourches {
            for (k, debut, s, bo) in cs {
                coupes[k][usize::from(!debut)] = Some((s, bo));
            }
            carrefours.push(car);
        }
        for ((v, c), &ab) in voies.iter_mut().zip(&coupes).zip(&absorbees) {
            // Une chaîne absorbée dans la plaque d'un groupe n'a plus de ruban
            v.trace = if ab { Vec::new() } else { crate::carrefours::couper(&v.points, c) };
            v.bouches = [c[0].map(|x| x.1), c[1].map(|x| x.1)];
        }
        drop(t_carrefours);
        let _t_index = crate::suivi::tache("index");
        let simples: std::collections::HashSet<usize> = carrefours.iter().flat_map(|c| c.noeuds.iter().copied()).collect();
        let fourches = fourches.iter().map(|f| f.0).filter(|n| !simples.contains(n)).collect();
        Champ::indexer(voies, carrefours, simples, fourches, deports)
    }

    /// Le champ, ses index faits.
    fn indexer(voies: Vec<Chaine>, carrefours: Vec<crate::carrefours::Carrefour>, simples: std::collections::HashSet<usize>, fourches: std::collections::HashSet<usize>, deports: Vec<Deport>) -> Champ {
        let mut index: HashMap<(i32, i32), Vec<(u32, u32)>> = HashMap::new();
        for (k, v) in voies.iter().enumerate() {
            // Les bouts comptent jusqu'à leur prolongement (une demi-largeur), frange
            // comprise
            let r = 2.0 * v.w + FRANGE;
            for (i, s) in v.points.windows(2).enumerate() {
                // Seuls les segments au sol font le champ
                if !au_sol(&s[0]) || !au_sol(&s[1]) {
                    continue;
                }
                let (a, b) = (case(s[0][0].min(s[1][0]) - r, s[0][1].min(s[1][1]) - r), case(s[0][0].max(s[1][0]) + r, s[0][1].max(s[1][1]) + r));
                for ci in a.0..=b.0 {
                    for cj in a.1..=b.1 {
                        index.entry((ci, cj)).or_default().push((k as u32, i as u32));
                    }
                }
            }
        }
        let mut chaines: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        for (k, v) in voies.iter().enumerate() {
            let r = 2.0 * v.w;
            let c = |x: f32, z: f32| ((x / CASE_CHAINES).floor() as i32, (z / CASE_CHAINES).floor() as i32);
            let (a, b) = (c(v.boite[0] - r, v.boite[1] - r), c(v.boite[2] + r, v.boite[3] + r));
            for ci in a.0..=b.0 {
                for cj in a.1..=b.1 {
                    chaines.entry((ci, cj)).or_default().push(k as u32);
                }
            }
        }
        Champ { voies, index, chaines, carrefours, simples, fourches, deports }
    }

    /// Le champ en octets, pour le cache (sans ses index, refaits à la lecture).
    pub fn ecrire(&self) -> Vec<u8> {
        use crate::cache_lieu::Ecrit;
        let mut e = Ecrit(CACHE_MAGIE.to_vec());
        fn pts(e: &mut Ecrit, v: &[Pt]) {
            e.u(v.len() as u32);
            for p in v {
                e.fs(p);
            }
        }
        fn bouche(e: &mut Ecrit, b: &crate::carrefours::Bouche) {
            e.fs(&b.q);
            e.fs(&b.g);
            e.fs(&b.m);
            e.fs(&b.d);
        }
        fn files(e: &mut Ecrit, f: sage_reel::Files) {
            for x in [f.total, f.avant, f.arriere, u8::from(f.osm)] {
                e.b(x);
            }
            e.u(u32::from(f.bus));
        }
        fn ensemble(e: &mut Ecrit, s: &std::collections::HashSet<usize>) {
            let mut v: Vec<usize> = s.iter().copied().collect();
            v.sort_unstable();
            e.u(v.len() as u32);
            for n in v {
                e.u(n as u32);
            }
        }
        e.u(self.voies.len() as u32);
        for v in &self.voies {
            pts(&mut e, &v.points);
            e.f(v.w);
            e.f(v.poids);
            e.fs(&v.boite);
            pts(&mut e, &v.trace);
            for b in &v.bouches {
                match b {
                    Some(b) => {
                        e.b(1);
                        bouche(&mut e, b);
                    }
                    None => e.b(0),
                }
            }
            e.b(u8::from(v.nets[0]));
            e.b(u8::from(v.nets[1]));
            files(&mut e, v.files[0]);
            files(&mut e, v.files[1]);
            e.u(v.aretes.len() as u32);
            for &k in &v.aretes {
                e.u(k as u32);
            }
            e.b(u8::from(v.bus));
            e.u(v.revetements.len() as u32);
            for (p, r) in &v.revetements {
                e.fs(p);
                e.b(*r);
            }
        }
        e.u(self.carrefours.len() as u32);
        for c in &self.carrefours {
            e.u(c.noeuds.len() as u32);
            for &n in &c.noeuds {
                e.u(n as u32);
            }
            e.fs(&c.c);
            e.u(c.plans.len() as u32);
            for p in &c.plans {
                e.fs(&p.c);
                e.fs(&p.g);
                e.f(p.rayon);
            }
            e.u(c.contour.len() as u32);
            for p in &c.contour {
                e.fs(p);
            }
            e.u(c.trous.len() as u32);
            for t in &c.trous {
                e.u(t.len() as u32);
                for p in t {
                    e.fs(p);
                }
            }
            e.u(c.bouches.len() as u32);
            for (k, b) in &c.bouches {
                e.u(*k as u32);
                bouche(&mut e, b);
            }
            e.fs(&c.boite);
            e.b(u8::from(c.aplanir));
        }
        ensemble(&mut e, &self.simples);
        ensemble(&mut e, &self.fourches);
        e.u(self.deports.len() as u32);
        for d in &self.deports {
            e.u(d.aretes.len() as u32);
            for &k in &d.aretes {
                e.u(k as u32);
            }
            e.fs(&d.c);
            e.fs(&d.decal);
            e.f(d.l);
        }
        e.0
    }

    /// Le champ relu du cache (`ecrire`), ses index refaits.
    pub fn lire(o: &[u8]) -> Option<Champ> {
        use crate::cache_lieu::Lu;
        if o.get(..8)? != CACHE_MAGIE {
            return None;
        }
        let mut l = Lu(o, 8);
        fn pts(l: &mut Lu) -> Option<Vec<Pt>> {
            let n = l.u()? as usize;
            let mut v = Vec::with_capacity(n.min(1 << 24));
            for _ in 0..n {
                v.push(l.fs::<7>()?);
            }
            Some(v)
        }
        fn bouche(l: &mut Lu) -> Option<crate::carrefours::Bouche> {
            Some(crate::carrefours::Bouche { q: l.fs::<7>()?, g: l.fs::<2>()?, m: l.fs::<2>()?, d: l.fs::<2>()? })
        }
        fn files(l: &mut Lu) -> Option<sage_reel::Files> {
            let (total, avant, arriere, osm) = (l.b()?, l.b()?, l.b()?, l.b()? != 0);
            Some(sage_reel::Files { total, avant, arriere, osm, bus: l.u()? as u16 })
        }
        fn usizes(l: &mut Lu) -> Option<Vec<usize>> {
            let n = l.u()? as usize;
            let mut v = Vec::with_capacity(n.min(1 << 24));
            for _ in 0..n {
                v.push(l.u()? as usize);
            }
            Some(v)
        }
        let nv = l.u()? as usize;
        let mut voies = Vec::with_capacity(nv.min(1 << 24));
        for _ in 0..nv {
            let points = pts(&mut l)?;
            let (w, poids, boite) = (l.f()?, l.f()?, l.fs::<4>()?);
            let trace = pts(&mut l)?;
            let mut bouches = [None, None];
            for b in &mut bouches {
                if l.b()? == 1 {
                    *b = Some(bouche(&mut l)?);
                }
            }
            let nets = [l.b()? != 0, l.b()? != 0];
            let files = [files(&mut l)?, files(&mut l)?];
            let aretes = usizes(&mut l)?;
            let bus = l.b()? != 0;
            let nr = l.u()? as usize;
            let mut revetements = Vec::with_capacity(nr.min(1 << 24));
            for _ in 0..nr {
                revetements.push((l.fs::<2>()?, l.b()?));
            }
            voies.push(Chaine { points, w, poids, boite, trace, bouches, nets, files, aretes, bus, revetements });
        }
        let nc = l.u()? as usize;
        let mut carrefours = Vec::with_capacity(nc.min(1 << 24));
        for _ in 0..nc {
            let noeuds = usizes(&mut l)?;
            let c = l.fs::<2>()?;
            let np = l.u()? as usize;
            let mut plans = Vec::with_capacity(np.min(1 << 20));
            for _ in 0..np {
                plans.push(crate::materialisation::Plan { c: l.fs::<3>()?, g: l.fs::<2>()?, rayon: l.f()? });
            }
            let n = l.u()? as usize;
            let mut contour = Vec::with_capacity(n.min(1 << 24));
            for _ in 0..n {
                contour.push(l.fs::<2>()?);
            }
            let nt = l.u()? as usize;
            let mut trous = Vec::with_capacity(nt.min(1 << 20));
            for _ in 0..nt {
                let n = l.u()? as usize;
                let mut t = Vec::with_capacity(n.min(1 << 24));
                for _ in 0..n {
                    t.push(l.fs::<2>()?);
                }
                trous.push(t);
            }
            let nb = l.u()? as usize;
            let mut bouches = Vec::with_capacity(nb.min(1 << 20));
            for _ in 0..nb {
                bouches.push((l.u()? as usize, bouche(&mut l)?));
            }
            let boite = l.fs::<4>()?;
            let aplanir = l.b()? != 0;
            carrefours.push(crate::carrefours::Carrefour { noeuds, c, plans, contour, trous, bouches, boite, aplanir });
        }
        let simples = usizes(&mut l)?.into_iter().collect();
        let fourches = usizes(&mut l)?.into_iter().collect();
        let nd = l.u()? as usize;
        let mut deports = Vec::with_capacity(nd.min(1 << 20));
        for _ in 0..nd {
            let aretes = usizes(&mut l)?;
            deports.push(Deport { aretes, c: l.fs::<2>()?, decal: l.fs::<2>()?, l: l.f()? });
        }
        if !l.fini() {
            return None;
        }
        Some(Champ::indexer(voies, carrefours, simples, fourches, deports))
    }

    /// Le nœud `n` est-il une fourche (`fourches`) ?
    /// Les branches de fourches déportées (pour la surimpression).
    pub fn deports(&self) -> &[Deport] {
        &self.deports
    }

    pub fn fourche(&self, n: usize) -> bool {
        self.fourches.contains(&n)
    }

    /// Le nœud `n` est-il un carrefour simple ?
    pub fn simple(&self, n: usize) -> bool {
        self.simples.contains(&n)
    }

    /// Les carrefours simples dont le contour (élargi de `marge`) touche `b`.
    pub fn carrefours_dans(&self, b: [f32; 4], marge: f32) -> Vec<&crate::carrefours::Carrefour> {
        self.carrefours.iter().filter(|c| c.boite[2] >= b[0] - marge && c.boite[0] <= b[2] + marge && c.boite[3] >= b[1] - marge && c.boite[1] <= b[3] + marge).collect()
    }

    /// Les chaînes dont la boîte (élargie de `marge` et de leur largeur) touche `b`.
    pub fn chaines_dans(&self, b: [f32; 4], marge: f32) -> Vec<&Chaine> {
        let c = |x: f32, z: f32| ((x / CASE_CHAINES).floor() as i32, (z / CASE_CHAINES).floor() as i32);
        let (a, z) = (c(b[0] - marge, b[1] - marge), c(b[2] + marge, b[3] + marge));
        let mut ids: Vec<u32> = Vec::new();
        for ci in a.0..=z.0 {
            for cj in a.1..=z.1 {
                if let Some(l) = self.chaines.get(&(ci, cj)) {
                    ids.extend_from_slice(l);
                }
            }
        }
        ids.sort_unstable();
        ids.dedup();
        ids.into_iter()
            .map(|k| &self.voies[k as usize])
            .filter(|v| {
                let m = marge + 2.0 * v.w;
                v.boite[2] >= b[0] - m && v.boite[0] <= b[2] + m && v.boite[3] >= b[1] - m && v.boite[1] <= b[3] + m
            })
            .collect()
    }

    /// La hauteur de la voirie en (x, z), lue par une voie à la hauteur `niveau` : la
    /// moyenne des profils des voies dont l'emprise (frange comprise) touche le point et
    /// qui sont à son niveau (`NIVEAU_PLEIN`, `NIVEAU_NUL`) ; rien s'il n'y en a pas.
    pub fn h(&self, x: f32, z: f32, niveau: f32) -> Option<f32> {
        let l = self.index.get(&case(x, z))?;
        // Pour chaque voie, son segment le plus proche : (distance, hauteur)
        let mut proches: Vec<(u32, f32, f32, f32)> = Vec::with_capacity(4);
        for &(k, i) in l {
            let v = &self.voies[k as usize];
            let n = v.points.len();
            let (a, b) = (&v.points[i as usize], &v.points[i as usize + 1]);
            let (ux, uz) = (b[0] - a[0], b[1] - a[1]);
            let l2 = ux * ux + uz * uz;
            let t = if l2 > 1e-12 { (((x - a[0]) * ux + (z - a[1]) * uz) / l2).clamp(0.0, 1.0) } else { 0.0 };
            let p: Pt = std::array::from_fn(|c| a[c] + t * (b[c] - a[c]));
            let mut d = (x - p[0]).hypot(z - p[1]);
            // La demi-largeur en ce point (en biseau là où la rue change de largeur)
            let w = p[6];
            // Au-delà d'un bout, le prolongement du ruban (une demi-largeur) compte
            // comme l'emprise : seule la distance en travers compte
            let bout = (i == 0 && t == 0.0) || (i as usize + 2 == n && t == 1.0);
            if bout && l2 > 1e-12 {
                let ln = l2.sqrt();
                let long = (((x - p[0]) * ux + (z - p[1]) * uz) / ln).abs();
                if long <= w {
                    d = (((x - a[0]) * uz - (z - a[1]) * ux) / ln).abs();
                }
            }
            if d >= w + FRANGE {
                continue;
            }
            let h = p[2] + p[3] * (x - p[0]) + p[4] * (z - p[1]);
            match proches.iter_mut().find(|e| e.0 == k) {
                Some(e) if d - w < e.1 - e.3 => *e = (k, d, h, w),
                Some(_) => {}
                None => proches.push((k, d, h, w)),
            }
        }
        let (mut s, mut m) = (0.0f32, 0.0f32);
        for (k, d, h, w) in proches {
            let v = &self.voies[k as usize];
            let f = if d <= w { 1.0 } else { (1.0 - (d - w) / FRANGE).powi(2) };
            let e = (h - niveau).abs();
            let f = f * if e <= NIVEAU_PLEIN { 1.0 } else { (1.0 - (e - NIVEAU_PLEIN) / (NIVEAU_NUL - NIVEAU_PLEIN)).max(0.0).powi(2) };
            s += f * v.poids * h;
            m += f * v.poids;
        }
        (m > 1e-6).then(|| s / m)
    }
}
