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
}

pub struct Champ {
    voies: Vec<Chaine>,
    /// Les segments au sol (voie, premier point) dont la zone d'influence touche
    /// chaque case.
    index: HashMap<(i32, i32), Vec<(u32, u32)>>,
    /// Les chaînes dont la boîte touche chaque grande case (`CASE_CHAINES`).
    chaines: HashMap<(i32, i32), Vec<u32>>,
}

/// Le côté (m) des cases de l'index des chaînes.
const CASE_CHAINES: f32 = 64.0;
/// L'écart de largeur (m) au-delà duquel deux tronçons ne sont pas enchaînés.
const LARGEUR_ECART: f32 = 0.6;

/// Les routes (sans les piétonnes) d'un nœud.
fn routes(r: &Reseau, n: usize) -> Vec<usize> {
    r.noeuds[n].aretes.iter().copied().filter(|&k| r.aretes[k].genre != sage_reel::PIETONNE).collect()
}

/// Si le nœud `n` ne relie que deux tronçons de même largeur (une coupure d'OSM,
/// l'entrée d'un pont ou d'un tunnel) : les deux, qui s'y enchaînent.
pub fn passage_simple(r: &Reseau, n: usize) -> Option<(usize, usize)> {
    let v = routes(r, n);
    if v.len() != 2 || v[0] == v[1] {
        return None;
    }
    let (a, b) = (&r.aretes[v[0]], &r.aretes[v[1]]);
    ((a.largeur - b.largeur).abs() < LARGEUR_ECART).then_some((v[0], v[1]))
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

fn case(x: f32, z: f32) -> (i32, i32) {
    ((x / CASE).floor() as i32, (z / CASE).floor() as i32)
}

impl Champ {
    /// Les chaînes de voies du réseau nivelé (sans les piétonnes), lissées.
    pub fn new(r: &Reseau) -> Champ {
        let mut voies = Vec::new();
        for (suite, debut, fin) in enchainer(r) {
            let (mut pts, mut codes): (Vec<[f32; 3]>, Vec<f32>) = (Vec::new(), Vec::new());
            let (mut w, mut pd) = (0.0f32, 0.0f32);
            for &(k, endroit) in &suite {
                let a = &r.aretes[k];
                let c = code_passage(a.passage);
                let l: Vec<[f32; 3]> = if endroit { a.points.clone() } else { a.points.iter().rev().copied().collect() };
                // Le point commun : au sol si l'un des deux l'est (la fin d'une rampe)
                let saute = usize::from(!pts.is_empty());
                if saute == 1 {
                    let d = codes.len() - 1;
                    codes[d] = codes[d].min(c);
                }
                pts.extend_from_slice(&l[saute..]);
                codes.extend(std::iter::repeat_n(c, l.len() - saute));
                w = w.max((a.largeur * 0.5).max(1.0));
                pd = pd.max(poids(a.genre));
            }
            let points = lisser(&pts, &codes, plan(r, debut), plan(r, fin));
            let mut boite = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
            for p in &points {
                boite = [boite[0].min(p[0]), boite[1].min(p[1]), boite[2].max(p[0]), boite[3].max(p[1])];
            }
            voies.push(Chaine { points, w, poids: pd, boite });
        }
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
        Champ { voies, index, chaines }
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
        let mut proches: Vec<(u32, f32, f32)> = Vec::with_capacity(4);
        for &(k, i) in l {
            let v = &self.voies[k as usize];
            let n = v.points.len();
            let (a, b) = (&v.points[i as usize], &v.points[i as usize + 1]);
            let (ux, uz) = (b[0] - a[0], b[1] - a[1]);
            let l2 = ux * ux + uz * uz;
            let t = if l2 > 1e-12 { (((x - a[0]) * ux + (z - a[1]) * uz) / l2).clamp(0.0, 1.0) } else { 0.0 };
            let p: Pt = std::array::from_fn(|c| a[c] + t * (b[c] - a[c]));
            let mut d = (x - p[0]).hypot(z - p[1]);
            // Au-delà d'un bout, le prolongement du ruban (une demi-largeur) compte
            // comme l'emprise : seule la distance en travers compte
            let bout = (i == 0 && t == 0.0) || (i as usize + 2 == n && t == 1.0);
            if bout && l2 > 1e-12 {
                let ln = l2.sqrt();
                let long = (((x - p[0]) * ux + (z - p[1]) * uz) / ln).abs();
                if long <= v.w {
                    d = (((x - a[0]) * uz - (z - a[1]) * ux) / ln).abs();
                }
            }
            if d >= v.w + FRANGE {
                continue;
            }
            let h = p[2] + p[3] * (x - p[0]) + p[4] * (z - p[1]);
            match proches.iter_mut().find(|e| e.0 == k) {
                Some(e) if d < e.1 => *e = (k, d, h),
                Some(_) => {}
                None => proches.push((k, d, h)),
            }
        }
        let (mut s, mut m) = (0.0f32, 0.0f32);
        for (k, d, h) in proches {
            let v = &self.voies[k as usize];
            let f = if d <= v.w { 1.0 } else { (1.0 - (d - v.w) / FRANGE).powi(2) };
            let e = (h - niveau).abs();
            let f = f * if e <= NIVEAU_PLEIN { 1.0 } else { (1.0 - (e - NIVEAU_PLEIN) / (NIVEAU_NUL - NIVEAU_PLEIN)).max(0.0).powi(2) };
            s += f * v.poids * h;
            m += f * v.poids;
        }
        (m > 1e-6).then(|| s / m)
    }
}
