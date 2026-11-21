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
}

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
fn desserrer(voies: &mut [Chaine], bouts: &[(usize, usize)]) {
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
                        if k2 == k {
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
                            let pres = [p[0], p[n - 1]].iter().any(|e| (e[0] - q[0]).hypot(e[1] - q[1]) < 3.0 * (q[6] + w2));
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

fn case(x: f32, z: f32) -> (i32, i32) {
    ((x / CASE).floor() as i32, (z / CASE).floor() as i32)
}

impl Champ {
    /// Les chaînes de voies du réseau nivelé (sans les piétonnes), lissées.
    pub fn new(r: &Reseau) -> Champ {
        let mut voies = Vec::new();
        let mut bouts = Vec::new();
        for (suite, debut, fin) in enchainer(r) {
            bouts.push((debut, fin));
            let (mut pts, mut codes, mut ws): (Vec<[f32; 3]>, Vec<f32>, Vec<f32>) = (Vec::new(), Vec::new(), Vec::new());
            let (mut w, mut pd) = (0.0f32, 0.0f32);
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
            voies.push(Chaine { points, w, poids: pd, boite, trace: Vec::new(), bouches: [None, None] });
        }
        desserrer(&mut voies, &bouts);
        // Les carrefours simples : les chaînes coupées à leurs bouches
        let (carrefours, coupes, absorbees) = crate::carrefours::construire(r, &voies, &bouts);
        for ((v, c), &ab) in voies.iter_mut().zip(&coupes).zip(&absorbees) {
            // Une chaîne absorbée dans la plaque d'un groupe n'a plus de ruban
            v.trace = if ab { Vec::new() } else { crate::carrefours::couper(&v.points, c) };
            v.bouches = [c[0].map(|x| x.1), c[1].map(|x| x.1)];
        }
        let simples = carrefours.iter().flat_map(|c| c.noeuds.iter().copied()).collect();
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
        Champ { voies, index, chaines, carrefours, simples }
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
