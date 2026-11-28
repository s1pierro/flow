//! Le réseau des voies d'un lieu (tous les tracés, au sol, en pont et en tunnel, en
//! un graphe), ses contraintes de hauteur (croisements dénivelés, cours d'eau) et son
//! profil nivelé (`nivele`). Venu de sage-reel, où il ne servait qu'à Flow. Aussi : le diagnostic du réseau des voies : ce que les données d'un lieu disent de la
//! continuité des tracés (bouts libres, ponts et tunnels raccordés ou non), des
//! croisements sans nœud commun (entre voies, et avec les cours d'eau) et des pentes.

use sage_reel::{Lieu, Route};
use std::collections::HashMap;

/// Où passe un tracé.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Passage {
    Sol,
    Pont,
    Tunnel,
}

impl Passage {
    pub fn nom(self) -> &'static str {
        match self {
            Passage::Sol => "sol",
            Passage::Pont => "pont",
            Passage::Tunnel => "tunnel",
        }
    }
}

pub struct Trace<'a> {
    pub passage: Passage,
    pub route: &'a Route,
}

/// Le genre des voies ferrées dans le réseau (au-delà des genres de voies) : des
/// trains au sol ou sur un pont, lissées et fixées avec les voies rapides, pour que les
/// routes passent dessus ou dessous ; retirées du réseau une fois nivelé.
pub const RAIL: u8 = 200;
/// Le gabarit sous une voie ferrée (le fil de contact), sans l'épaisseur du tablier.
pub const GABARIT_RAIL: f32 = 5.0;

pub fn traces(l: &Lieu) -> Vec<Trace<'_>> {
    let mut v: Vec<Trace> = l.routes.iter().map(|r| Trace { passage: Passage::Sol, route: r }).collect();
    v.extend(l.ponts.iter().map(|r| Trace { passage: Passage::Pont, route: r }));
    v.extend(l.transports.tunnels.iter().map(|r| Trace { passage: Passage::Tunnel, route: r }));
    v
}

fn cle(p: [f32; 3]) -> (i64, i64) {
    ((p[0] as f64 * 100.0).round() as i64, (p[1] as f64 * 100.0).round() as i64)
}

const CASE: f32 = 32.0;

fn case(x: f32, z: f32) -> (i32, i32) {
    ((x / CASE).floor() as i32, (z / CASE).floor() as i32)
}

/// L'intersection propre des segments ab et cd (hors bouts), en xz.
fn croise(a: [f32; 3], b: [f32; 3], c: [f32; 3], d: [f32; 3]) -> Option<(f32, f32)> {
    let (r, s) = ([b[0] - a[0], b[1] - a[1]], [d[0] - c[0], d[1] - c[1]]);
    let den = r[0] * s[1] - r[1] * s[0];
    if den.abs() < 1e-9 {
        return None;
    }
    let (qx, qz) = (c[0] - a[0], c[1] - a[1]);
    let t = (qx * s[1] - qz * s[0]) / den;
    let u = (qx * r[1] - qz * r[0]) / den;
    let e = 1e-4;
    (t > e && t < 1.0 - e && u > e && u < 1.0 - e).then_some((t, u))
}

pub fn diagnostic(l: &Lieu) -> String {
    let t = traces(l);
    let mut out = Vec::new();
    // Les tracés et leur longueur
    let mut par: HashMap<Passage, (usize, f32)> = HashMap::new();
    for x in &t {
        let lg: f32 = x.route.points.windows(2).map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1])).sum();
        let e = par.entry(x.passage).or_default();
        e.0 += 1;
        e.1 += lg;
    }
    for p in [Passage::Sol, Passage::Pont, Passage::Tunnel] {
        let (n, lg) = par.get(&p).copied().unwrap_or_default();
        out.push(format!("{} : {n} tracés, {:.1} km", p.nom(), lg / 1000.0));
    }
    // Les sommets de chaque tracé, indexés au centimètre
    let mut sommets: HashMap<(i64, i64), Vec<(usize, usize)>> = HashMap::new();
    for (k, x) in t.iter().enumerate() {
        for (i, p) in x.route.points.iter().enumerate() {
            sommets.entry(cle(*p)).or_default().push((k, i));
        }
    }
    // Les segments, indexés par case
    let mut cases: HashMap<(i32, i32), Vec<(usize, usize)>> = HashMap::new();
    for (k, x) in t.iter().enumerate() {
        for i in 0..x.route.points.len().saturating_sub(1) {
            let (a, b) = (x.route.points[i], x.route.points[i + 1]);
            let (c0, c1) = (case(a[0].min(b[0]), a[1].min(b[1])), case(a[0].max(b[0]), a[1].max(b[1])));
            for ci in c0.0..=c1.0 {
                for cj in c0.1..=c1.1 {
                    cases.entry((ci, cj)).or_default().push((k, i));
                }
            }
        }
    }
    let proche = |p: [f32; 3], sauf: usize, r: f32| -> Option<(usize, f32)> {
        let (ci, cj) = case(p[0], p[1]);
        let mut best: Option<(usize, f32)> = None;
        for i in ci - 1..=ci + 1 {
            for j in cj - 1..=cj + 1 {
                for &(k, s) in cases.get(&(i, j)).into_iter().flatten() {
                    if k == sauf {
                        continue;
                    }
                    let (a, b) = (t[k].route.points[s], t[k].route.points[s + 1]);
                    let (ux, uz) = (b[0] - a[0], b[1] - a[1]);
                    let l2 = (ux * ux + uz * uz).max(1e-12);
                    let u = (((p[0] - a[0]) * ux + (p[1] - a[1]) * uz) / l2).clamp(0.0, 1.0);
                    let d = (p[0] - a[0] - ux * u).hypot(p[1] - a[1] - uz * u);
                    if d < r && best.is_none_or(|b| d < b.1) {
                        best = Some((k, d));
                    }
                }
            }
        }
        best
    };
    // Les bouts : raccordés (un autre tracé y a un sommet), presque (un autre tracé à
    // moins de 3 m), libres
    let mut bouts: HashMap<(Passage, &str, Option<Passage>), usize> = HashMap::new();
    for (k, x) in t.iter().enumerate() {
        let n = x.route.points.len();
        for i in [0, n - 1] {
            let p = x.route.points[i];
            let autres: Vec<&(usize, usize)> = sommets[&cle(p)].iter().filter(|(kk, _)| *kk != k).collect();
            let etat = if let Some(&&(kk, _)) = autres.first() {
                ("raccordé", Some(t[kk].passage))
            } else if let Some((kk, _)) = proche(p, k, 3.0) {
                ("à moins de 3 m d'un", Some(t[kk].passage))
            } else {
                ("libre", None)
            };
            *bouts.entry((x.passage, etat.0, etat.1)).or_default() += 1;
        }
    }
    out.push("bouts des tracés :".into());
    let mut b: Vec<_> = bouts.into_iter().collect();
    b.sort_by_key(|(k, _)| (k.0 as u8, k.1, k.2.map(|p| p as u8)));
    for ((p, e, a), n) in b {
        out.push(format!("  {} {e}{} : {n}", p.nom(), a.map(|a| format!(" {}", a.nom())).unwrap_or_default()));
    }
    // Les croisements sans nœud commun, par paire de passages
    let mut crois: HashMap<(Passage, Passage), usize> = HashMap::new();
    let mut vus = std::collections::HashSet::new();
    let mut ecarts_sol = Vec::new();
    for v in cases.values() {
        for (x, &(k1, s1)) in v.iter().enumerate() {
            for &(k2, s2) in &v[x + 1..] {
                if k1 == k2 {
                    continue;
                }
                let (a, b) = (t[k1].route.points[s1], t[k1].route.points[s1 + 1]);
                let (c, d) = (t[k2].route.points[s2], t[k2].route.points[s2 + 1]);
                if let Some((tt, u)) = croise(a, b, c, d) {
                    let key = if (k1, s1) < (k2, s2) { (k1, s1, k2, s2) } else { (k2, s2, k1, s1) };
                    if !vus.insert(key) {
                        continue;
                    }
                    let (p1, p2) = (t[k1].passage, t[k2].passage);
                    let pr = if (p1 as u8) <= (p2 as u8) { (p1, p2) } else { (p2, p1) };
                    *crois.entry(pr).or_default() += 1;
                    if pr == (Passage::Sol, Passage::Sol) {
                        let y1 = a[2] + (b[2] - a[2]) * tt;
                        let y2 = c[2] + (d[2] - c[2]) * u;
                        ecarts_sol.push((y1 - y2).abs());
                    }
                }
            }
        }
    }
    out.push("croisements sans nœud commun :".into());
    let mut c: Vec<_> = crois.into_iter().collect();
    c.sort_by_key(|(k, _)| (k.0 as u8, k.1 as u8));
    for ((a, b), n) in c {
        out.push(format!("  {} × {} : {n}", a.nom(), b.nom()));
    }
    if !ecarts_sol.is_empty() {
        ecarts_sol.sort_by(f32::total_cmp);
        out.push(format!("  sol × sol, écart de hauteur : médian {:.1} m, max {:.1} m", ecarts_sol[ecarts_sol.len() / 2], ecarts_sol[ecarts_sol.len() - 1]));
    }
    // Avec les cours d'eau (à ciel ouvert)
    let mut eau: HashMap<Passage, usize> = HashMap::new();
    for ce in l.cours_eau.iter().filter(|c| !c.souterrain) {
        for w in ce.points.windows(2) {
            let (c3, d3) = ([w[0][0], w[0][1], 0.0], [w[1][0], w[1][1], 0.0]);
            let (c0, c1) = (case(c3[0].min(d3[0]), c3[1].min(d3[1])), case(c3[0].max(d3[0]), c3[1].max(d3[1])));
            let mut vu = std::collections::HashSet::new();
            for ci in c0.0..=c1.0 {
                for cj in c0.1..=c1.1 {
                    for &(k, s) in cases.get(&(ci, cj)).into_iter().flatten() {
                        if vu.insert((k, s)) && croise(t[k].route.points[s], t[k].route.points[s + 1], c3, d3).is_some() {
                            *eau.entry(t[k].passage).or_default() += 1;
                        }
                    }
                }
            }
        }
    }
    out.push(format!("croisements avec les cours d'eau ({} tracés) :", l.cours_eau.iter().filter(|c| !c.souterrain).count()));
    for p in [Passage::Sol, Passage::Pont, Passage::Tunnel] {
        out.push(format!("  {} : {}", p.nom(), eau.get(&p).copied().unwrap_or(0)));
    }
    // Les pentes au sol, et les sauts aux raccords pont/tunnel ↔ sol
    let mut pentes = Vec::new();
    for x in t.iter().filter(|x| x.passage == Passage::Sol) {
        for w in x.route.points.windows(2) {
            let l = (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]);
            if l > 2.0 {
                pentes.push((w[1][2] - w[0][2]).abs() / l);
            }
        }
    }
    pentes.sort_by(f32::total_cmp);
    if !pentes.is_empty() {
        let q = |f: f32| pentes[((pentes.len() - 1) as f32 * f) as usize] * 100.0;
        out.push(format!("pentes au sol : médiane {:.1} %, 99e centile {:.1} %, max {:.1} % ; > 10 % : {}", q(0.5), q(0.99), q(1.0), pentes.iter().filter(|&&p| p > 0.10).count()));
    }
    let mut sauts = Vec::new();
    for (k, x) in t.iter().enumerate().filter(|(_, x)| x.passage != Passage::Sol) {
        let n = x.route.points.len();
        for i in [0, n - 1] {
            let p = x.route.points[i];
            for &(kk, ii) in &sommets[&cle(p)] {
                if kk != k && t[kk].passage == Passage::Sol {
                    sauts.push((p[2] - t[kk].route.points[ii][2]).abs());
                }
            }
        }
    }
    if !sauts.is_empty() {
        sauts.sort_by(f32::total_cmp);
        out.push(format!("sauts de hauteur aux raccords pont/tunnel ↔ sol : {} raccords, médian {:.2} m, max {:.2} m", sauts.len(), sauts[sauts.len() / 2], sauts[sauts.len() - 1]));
    }
    out.join("\n")
}

// --- Étape 1 : le réseau ---------------------------------------------------------

/// Un bout libre à moins de `RACCORD` mètres d'un autre tracé y est raccordé.
pub const RACCORD: f32 = 3.0;
/// Deux voies au sol qui se croisent sans nœud commun, à moins de `A_NIVEAU` mètres de
/// hauteur l'une de l'autre, se croisent à niveau : un nœud est ajouté.
pub const A_NIVEAU: f32 = 1.0;
/// Un raccord tombe sur un sommet existant s'il en est à moins de `SUR_SOMMET` mètres.
const SUR_SOMMET: f32 = 0.5;

/// D'où vient un nœud du réseau : un point des tracés du lieu (les données), ou un
/// point que la construction a ajouté ou sur lequel elle a raccordé un bout.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Origine {
    /// Un point des tracés du lieu (OSM, coupé aux ponts et aux tunnels ; ou retouché).
    #[default]
    Donnees,
    /// Le bout d'un tracé coupé au bord de la carte.
    Bord,
    /// Un bout libre raccordé à un autre tracé, sur un point ajouté (`RACCORD`).
    Raccord,
    /// Un bout libre raccordé sur un sommet existant d'un autre tracé (`SUR_SOMMET`).
    RaccordSommet,
    /// Deux voies au sol qui se croisent sans point commun : un point ajouté (`A_NIVEAU`).
    Croisement,
}

/// Un nœud du réseau : un carrefour, un raccord (changement de passage) ou un bout.
#[derive(Clone, Debug)]
pub struct Noeud {
    /// x, z, hauteur (comme les points des routes).
    pub p: [f32; 3],
    pub aretes: Vec<usize>,
    /// D'où il vient (tracé pendant la construction).
    pub origine: Origine,
}

/// Une arête : d'un nœud à l'autre, un même tracé et un même passage.
#[derive(Clone, Debug)]
pub struct Arete {
    pub a: usize,
    pub b: usize,
    pub points: Vec<[f32; 3]>,
    pub passage: Passage,
    pub genre: u8,
    pub largeur: f32,
    /// Le revêtement de la voie (`sage_reel::BITUME`…).
    pub revetement: u8,
    /// Ses files, dans le sens de ses points (donnée annexe : le réseau n'en dépend
    /// pas).
    pub files: sage_reel::Files,
    /// Le niveau d'OSM (`sage_reel::Route::niveau`).
    pub niveau: i8,
}

#[derive(Clone, Debug, Default)]
pub struct Reseau {
    pub noeuds: Vec<Noeud>,
    pub aretes: Vec<Arete>,
    /// Ce que la construction a fait : raccords, nœuds ajoutés aux croisements à niveau.
    pub raccords: usize,
    pub a_niveau: usize,
    /// Les arêtes posées sur un tablier au-dessus d'un souterrain (`poser_tabliers`).
    pub tabliers: Vec<u32>,
    /// Les arêtes fixes : les voies rapides posées (`lisser_rapides`).
    pub fixes: Vec<u32>,
}

/// Un tracé en cours de construction.
struct Brut {
    passage: Passage,
    niveau: i8,
    files: sage_reel::Files,
    genre: u8,
    revetement: u8,
    largeur: f32,
    points: Vec<[f32; 3]>,
}

/// La relecture des voies au sol : un point tous les `RELIEF_PAS` mètres au plus, la
/// hauteur prise au relief moyenné sur une fenêtre d'environ `RELIEF_FENETRE` mètres
/// (des points de la grille, pondérés en pyramide) : un relief lissé en surface,
/// le même pour toutes les voies.
pub const RELIEF_FENETRE: f64 = 6.0;
pub const RELIEF_PAS: f32 = 4.0;

/// La hauteur du relief adouci en (x, z).
pub fn relief_adouci(l: &Lieu, x: f64, z: f64) -> f32 {
    let n = ((RELIEF_FENETRE * 0.5 / l.pas).round() as i64).max(1);
    let (mut somme, mut poids) = (0.0f64, 0.0f64);
    for i in -n..=n {
        for j in -n..=n {
            let w = ((n + 1 - i.abs()) * (n + 1 - j.abs())) as f64;
            somme += w * l.relief(x + i as f64 * l.pas, z + j as f64 * l.pas);
            poids += w;
        }
    }
    (somme / poids) as f32
}

fn relire(l: &Lieu, p: &[[f32; 3]]) -> Vec<[f32; 3]> {
    let mut v: Vec<[f32; 3]> = Vec::with_capacity(p.len() * 2);
    let h = |x: f32, z: f32| relief_adouci(l, x as f64, z as f64);
    v.push([p[0][0], p[0][1], h(p[0][0], p[0][1])]);
    for w in p.windows(2) {
        let (a, b) = (w[0], w[1]);
        let d = (b[0] - a[0]).hypot(b[1] - a[1]);
        let n = (d / RELIEF_PAS).ceil().max(1.0) as usize;
        for i in 1..=n {
            let t = i as f32 / n as f32;
            let (x, z) = (a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t);
            v.push([x, z, h(x, z)]);
        }
    }
    v
}

fn projeter(p: [f32; 3], a: [f32; 3], b: [f32; 3]) -> (f32, f32) {
    let (ux, uz) = (b[0] - a[0], b[1] - a[1]);
    let l2 = (ux * ux + uz * uz).max(1e-12);
    let u = (((p[0] - a[0]) * ux + (p[1] - a[1]) * uz) / l2).clamp(0.0, 1.0);
    (u, (p[0] - a[0] - ux * u).hypot(p[1] - a[1] - uz * u))
}

pub fn entre(a: [f32; 3], b: [f32; 3], u: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * u, a[1] + (b[1] - a[1]) * u, a[2] + (b[2] - a[2]) * u]
}

struct Index {
    cases: HashMap<(i32, i32), Vec<(usize, usize)>>,
}

impl Index {
    fn new(t: &[Brut]) -> Index {
        let mut cases: HashMap<(i32, i32), Vec<(usize, usize)>> = HashMap::new();
        for (k, x) in t.iter().enumerate() {
            for i in 0..x.points.len().saturating_sub(1) {
                let (a, b) = (x.points[i], x.points[i + 1]);
                let (c0, c1) = (case(a[0].min(b[0]), a[1].min(b[1])), case(a[0].max(b[0]), a[1].max(b[1])));
                for ci in c0.0..=c1.0 {
                    for cj in c0.1..=c1.1 {
                        cases.entry((ci, cj)).or_default().push((k, i));
                    }
                }
            }
        }
        Index { cases }
    }

    fn autour(&self, p: [f32; 3]) -> impl Iterator<Item = &(usize, usize)> {
        let (ci, cj) = case(p[0], p[1]);
        (ci - 1..=ci + 1).flat_map(move |i| (cj - 1..=cj + 1).map(move |j| (i, j))).flat_map(|c| self.cases.get(&c).into_iter().flatten())
    }
}

/// Le réseau des voies d'un lieu : tous les tracés (sol, ponts, tunnels) en un graphe.
pub fn reseau(l: &Lieu) -> Reseau {
    let mut t: Vec<Brut> = traces(l).into_iter().filter(|x| x.route.points.len() >= 2).map(|x| Brut { passage: x.passage, niveau: x.route.niveau, files: x.route.files, genre: x.route.genre, revetement: x.route.revetement, largeur: x.route.largeur, points: x.route.points.clone() }).collect();
    // Les voies ferrées des trains, au sol et en pont, après les voies : leurs arêtes et
    // leurs nœuds viennent en dernier (`retirer_rails`)
    {
        use sage_reel::transports as tr;
        for x in l.transports.rails.iter().filter(|x| x.mode == tr::TRAIN && x.ou != tr::TUNNEL && x.points.len() >= 2) {
            let passage = if x.ou == tr::PONT { Passage::Pont } else { Passage::Sol };
            t.push(Brut { passage, niveau: x.niveau, files: Default::default(), genre: RAIL, revetement: 0, largeur: tr::largeur(x.mode), points: x.points.clone() });
        }
    }
    // Les voies au sol relues sur le relief adouci (au lieu du profil du lieu, lissé
    // voie par voie le long de son axe) : toutes lisent la même surface, et se
    // rejoignent sans écart
    if !l.h.is_empty() {
        for b in t.iter_mut().filter(|b| b.passage == Passage::Sol) {
            b.points = relire(l, &b.points);
        }
    }
    let mut r = Reseau::default();
    // Les sommets de tous les tracés, au centimètre
    // (au centimètre, voie ferrée) : un rail et une route ne partagent pas de nœud
    let compter = |t: &[Brut]| {
        let mut s: HashMap<((i64, i64), bool), usize> = HashMap::new();
        for x in t {
            for p in &x.points {
                *s.entry((cle(*p), x.genre == RAIL)).or_default() += 1;
            }
        }
        s
    };
    // Les points ajoutés ou visés par la construction, et pourquoi
    let mut marques: HashMap<(i64, i64), Origine> = HashMap::new();
    // 1. Les bouts libres raccordés au tracé le plus proche (moins de RACCORD mètres)
    {
        let sommets = compter(&t);
        let index = Index::new(&t);
        // (tracé, segment, u, point) à insérer ; (tracé, au début, point) à prolonger
        let mut inserer: Vec<(usize, usize, f32, [f32; 3])> = Vec::new();
        let mut prolonger: Vec<(usize, bool, [f32; 3])> = Vec::new();
        for (k, x) in t.iter().enumerate() {
            let n = x.points.len();
            for (debut, p) in [(true, x.points[0]), (false, x.points[n - 1])] {
                if sommets[&(cle(p), x.genre == RAIL)] > 1 {
                    continue;
                }
                let mut best: Option<(usize, usize, f32, f32)> = None;
                for &(kk, s) in index.autour(p) {
                    if kk == k || (t[kk].genre == RAIL) != (x.genre == RAIL) {
                        continue;
                    }
                    let (u, d) = projeter(p, t[kk].points[s], t[kk].points[s + 1]);
                    // D'un autre passage (une voie au sol au-dessus d'un tunnel, sous un
                    // pont) : seulement près d'un bout de ce tracé (l'entrée d'un pont ou
                    // d'un tunnel qu'OSM n'a pas tout à fait rejointe)
                    if t[kk].passage != x.passage {
                        let q = &t[kk].points;
                        let lg = |i: usize| (q[i + 1][0] - q[i][0]).hypot(q[i + 1][1] - q[i][1]);
                        let avant: f32 = (0..s).map(lg).sum::<f32>() + u * lg(s);
                        let apres: f32 = (1.0 - u) * lg(s) + (s + 1..q.len() - 1).map(lg).sum::<f32>();
                        if avant.min(apres) > RACCORD {
                            continue;
                        }
                    }
                    if d < RACCORD && best.is_none_or(|b| d < b.3) {
                        best = Some((kk, s, u, d));
                    }
                }
                let Some((kk, s, u, _)) = best else { continue };
                let (a, b) = (t[kk].points[s], t[kk].points[s + 1]);
                let lg = (b[0] - a[0]).hypot(b[1] - a[1]);
                // Sur un sommet existant s'il est tout près ; sinon un sommet nouveau
                let cible = if u * lg < SUR_SOMMET {
                    marques.entry(cle(a)).or_insert(Origine::RaccordSommet);
                    a
                } else if (1.0 - u) * lg < SUR_SOMMET {
                    marques.entry(cle(b)).or_insert(Origine::RaccordSommet);
                    b
                } else {
                    let q = entre(a, b, u);
                    inserer.push((kk, s, u, q));
                    marques.insert(cle(q), Origine::Raccord);
                    q
                };
                prolonger.push((k, debut, cible));
                r.raccords += 1;
            }
        }
        for (k, debut, q) in prolonger {
            let pts = &mut t[k].points;
            let bout = if debut { pts[0] } else { pts[pts.len() - 1] };
            let tout_pres = (bout[0] - q[0]).hypot(bout[1] - q[1]) < 0.02;
            match (debut, tout_pres) {
                (true, true) => pts[0] = q,
                (true, false) => pts.insert(0, q),
                (false, true) => *pts.last_mut().unwrap() = q,
                (false, false) => pts.push(q),
            }
        }
        appliquer(&mut t, inserer);
    }
    // 2. Les croisements à niveau de deux voies au sol, sans nœud commun : un nœud
    {
        let index = Index::new(&t);
        let mut inserer = Vec::new();
        let mut vus = std::collections::HashSet::new();
        for v in index.cases.values() {
            for (x, &(k1, s1)) in v.iter().enumerate() {
                for &(k2, s2) in &v[x + 1..] {
                    if k1 == k2 || t[k1].passage != Passage::Sol || t[k2].passage != Passage::Sol || t[k1].genre == RAIL || t[k2].genre == RAIL {
                        continue;
                    }
                    let (a, b, c, d) = (t[k1].points[s1], t[k1].points[s1 + 1], t[k2].points[s2], t[k2].points[s2 + 1]);
                    let Some((u1, u2)) = croise(a, b, c, d) else { continue };
                    if !vus.insert((k1.min(k2), k1.max(k2), if k1 < k2 { (s1, s2) } else { (s2, s1) })) {
                        continue;
                    }
                    let (p1, p2) = (entre(a, b, u1), entre(c, d, u2));
                    if (p1[2] - p2[2]).abs() >= A_NIVEAU {
                        continue;
                    }
                    let q = [p1[0], p1[1], (p1[2] + p2[2]) / 2.0];
                    marques.insert(cle(q), Origine::Croisement);
                    inserer.push((k1, s1, u1, q));
                    inserer.push((k2, s2, u2, q));
                    r.a_niveau += 1;
                }
            }
        }
        appliquer(&mut t, inserer);
    }
    // 3. Les nœuds : les bouts, et les sommets que partagent plusieurs tracés (ou un
    //    tracé qui repasse par lui-même) ; les tracés coupés en arêtes à leurs nœuds
    let sommets = compter(&t);
    let mut numero: HashMap<((i64, i64), bool), usize> = HashMap::new();
    let [x0, z0, x1, z1] = l.etendue();
    let au_bord = |p: [f32; 3]| (p[0] as f64 - x0).min(x1 - p[0] as f64).min(p[1] as f64 - z0).min(z1 - p[1] as f64) < 1.0;
    let mut noeud = |p: [f32; 3], rail: bool, r: &mut Reseau| -> usize {
        *numero.entry((cle(p), rail)).or_insert_with(|| {
            let origine = marques.get(&cle(p)).copied().unwrap_or(if au_bord(p) { Origine::Bord } else { Origine::Donnees });
            r.noeuds.push(Noeud { p, aretes: Vec::new(), origine });
            r.noeuds.len() - 1
        })
    };
    for x in &t {
        let n = x.points.len();
        let mut debut = 0;
        for i in 1..n {
            let rail = x.genre == RAIL;
            if i == n - 1 || sommets[&(cle(x.points[i]), rail)] > 1 {
                let (a, b) = (noeud(x.points[debut], rail, &mut r), noeud(x.points[i], rail, &mut r));
                let k = r.aretes.len();
                r.aretes.push(Arete { a, b, points: x.points[debut..=i].to_vec(), passage: x.passage, genre: x.genre, largeur: x.largeur, revetement: x.revetement, files: x.files, niveau: x.niveau });
                r.noeuds[a].aretes.push(k);
                if b != a {
                    r.noeuds[b].aretes.push(k);
                }
                debut = i;
            }
        }
    }
    r
}

/// Insère les sommets (tracé, segment, u, point), du plus loin au plus près dans chaque
/// tracé pour que les numéros des segments restent justes.
fn appliquer(t: &mut [Brut], mut inserer: Vec<(usize, usize, f32, [f32; 3])>) {
    inserer.sort_by(|a, b| (b.0, b.1).cmp(&(a.0, a.1)).then(b.2.total_cmp(&a.2)));
    inserer.dedup_by(|a, b| a.0 == b.0 && a.1 == b.1 && (a.2 - b.2).abs() < 1e-6);
    for (k, s, _, q) in inserer {
        t[k].points.insert(s + 1, q);
    }
}

impl Reseau {
    /// Le résumé : nœuds par degré, composantes, bouts libres, ce qui a été raccordé.
    pub fn resume(&self) -> String {
        let mut degres: HashMap<usize, usize> = HashMap::new();
        for n in &self.noeuds {
            *degres.entry(n.aretes.len().min(5)).or_default() += 1;
        }
        let mut d: Vec<_> = degres.into_iter().collect();
        d.sort();
        // Les composantes connexes (union-find sur les nœuds)
        let mut pere: Vec<usize> = (0..self.noeuds.len()).collect();
        fn racine(p: &mut [usize], mut i: usize) -> usize {
            while p[i] != i {
                p[i] = p[p[i]];
                i = p[i];
            }
            i
        }
        for a in &self.aretes {
            let (x, y) = (racine(&mut pere, a.a), racine(&mut pere, a.b));
            pere[x] = y;
        }
        let mut tailles: HashMap<usize, f32> = HashMap::new();
        let mut total = 0.0;
        for a in &self.aretes {
            let lg: f32 = a.points.windows(2).map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1])).sum();
            *tailles.entry(racine(&mut pere, a.a)).or_default() += lg;
            total += lg;
        }
        let mut t: Vec<f32> = tailles.values().copied().collect();
        t.sort_by(|a, b| b.total_cmp(a));
        let mut par: HashMap<Passage, usize> = HashMap::new();
        for a in &self.aretes {
            *par.entry(a.passage).or_default() += 1;
        }
        let mut s = vec![
            format!("réseau : {} nœuds, {} arêtes (sol {}, pont {}, tunnel {}), {:.1} km", self.noeuds.len(), self.aretes.len(), par.get(&Passage::Sol).unwrap_or(&0), par.get(&Passage::Pont).unwrap_or(&0), par.get(&Passage::Tunnel).unwrap_or(&0), total / 1000.0),
            format!("construction : {} bouts raccordés, {} croisements à niveau reçoivent un nœud", self.raccords, self.a_niveau),
            format!("nœuds par degré : {}", d.iter().map(|(k, n)| format!("{}{k} : {n}", if *k == 5 { "≥" } else { "" })).collect::<Vec<_>>().join(", ")),
            format!("composantes : {} ; la plus grande {:.1} km ({:.1} %), puis {}", t.len(), t[0] / 1000.0, t[0] / total * 100.0, t.iter().skip(1).take(4).map(|x| format!("{:.2} km", x / 1000.0)).collect::<Vec<_>>().join(", ")),
        ];
        // Les changements de passage aux nœuds (pont ↔ sol…)
        let mut changes = 0;
        for n in &self.noeuds {
            let mut p: Vec<Passage> = n.aretes.iter().map(|&k| self.aretes[k].passage).collect();
            p.sort_by_key(|x| *x as u8);
            p.dedup();
            if p.len() > 1 {
                changes += 1;
            }
        }
        s.push(format!("nœuds où le passage change (raccords de ponts et tunnels) : {changes}"));
        s.join("\n")
    }

}

// --- Étape 2 : les contraintes de hauteur -------------------------------------------

/// Le gabarit sous un ouvrage (m), et l'épaisseur d'un tablier ou d'une dalle de
/// tunnel : l'écart de hauteur minimal entre deux voies qui se croisent dénivelées.
pub const GABARIT: f32 = 4.5;
/// Quand la hauteur limitée de la voie du dessous est connue (`maxheight` d'OSM, les
/// gabarits du lieu) : l'écart est cette hauteur plus la dalle et sa marge, plus
/// épaisse sous une voie au sol ou un pont (chaussée, remblai) qu'entre deux
/// souterrains (une dalle armée mince).
pub const DALLE_SOUS_SOL: f32 = 2.0;
pub const DALLE_ENTRE_SOUTERRAINS: f32 = 1.0;
/// Un gabarit s'applique à une voie s'il la longe à moins de tant (m), à moins de
/// `GABARIT_ANGLE` degrés de sa direction.
const GABARIT_PORTEE: f32 = 3.0;
const GABARIT_ANGLE: f32 = 30.0;
pub const EPAISSEUR: f32 = 1.0;
/// Sous une voie, un passage piéton (un souterrain) : sa hauteur libre et sa dalle
/// (0.18 ; avant, 5,5 m comme une route : un souterrain piéton coincé entre la surface
/// et un tunnel routier ne tenait nulle part).
pub const GABARIT_PIETON: f32 = 2.5;
pub const EPAISSEUR_PIETON: f32 = 0.5;
/// Le tirant d'air d'un pont sur l'eau (m), sous son tablier.
pub const TIRANT_D_AIR: f32 = 4.0;

/// Un point d'une arête : (arête, segment, avancée sur le segment de 0 à 1).
pub type Ou = (usize, usize, f32);

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Genre {
    /// Un pont (ou un pont implicite) sur une voie.
    PontSurVoie,
    /// Un tunnel sous une voie.
    TunnelSousVoie,
    /// Un pont sur l'eau : un croisement avec un cours d'eau, ou un point de pont au-dessus
    /// d'une surface d'eau.
    PontSurEau,
}

/// Une contrainte : `haut` doit passer au moins `ecart` au-dessus de `bas` (une voie) ou
/// de `plancher` (l'eau : `bas` est alors vide).
#[derive(Clone, Debug)]
pub struct Contrainte {
    pub genre: Genre,
    pub haut: Ou,
    pub bas: Option<Ou>,
    pub plancher: f32,
    pub ecart: f32,
    /// Le haut est une voie au sol : il lui faudra un pont implicite.
    pub implicite: bool,
    /// La hauteur limitée de la voie du dessous, si un gabarit du lieu la donne.
    pub gabarit: Option<f32>,
}

/// Les gabarits du lieu (hauteurs limitées), leurs segments par case.
pub struct Gabarits<'a> {
    l: &'a Lieu,
    cases: HashMap<(i32, i32), Vec<(usize, usize)>>,
}

impl<'a> Gabarits<'a> {
    pub fn new(l: &'a Lieu) -> Self {
        let mut cases: HashMap<(i32, i32), Vec<(usize, usize)>> = HashMap::new();
        for (g, gb) in l.gabarits.iter().enumerate() {
            for i in 0..gb.points.len().saturating_sub(1) {
                let (p, q) = (gb.points[i], gb.points[i + 1]);
                let (c0, c1) = (case(p[0].min(q[0]) - GABARIT_PORTEE, p[1].min(q[1]) - GABARIT_PORTEE), case(p[0].max(q[0]) + GABARIT_PORTEE, p[1].max(q[1]) + GABARIT_PORTEE));
                for ci in c0.0..=c1.0 {
                    for cj in c0.1..=c1.1 {
                        cases.entry((ci, cj)).or_default().push((g, i));
                    }
                }
            }
        }
        Gabarits { l, cases }
    }

    /// La hauteur limitée au point `u` du segment pq d'une voie : le plus bas des
    /// gabarits qui le longent.
    pub fn hauteur(&self, p: [f32; 3], q: [f32; 3], u: f32) -> Option<f32> {
        let c = [p[0] + (q[0] - p[0]) * u, p[1] + (q[1] - p[1]) * u];
        let (dx, dz) = (q[0] - p[0], q[1] - p[1]);
        let ld = dx.hypot(dz).max(1e-6);
        let mut h: Option<f32> = None;
        for &(g, i) in self.cases.get(&case(c[0], c[1]))? {
            let gb = &self.l.gabarits[g];
            let (e0, e1) = (gb.points[i], gb.points[i + 1]);
            let (ex, ez) = (e1[0] - e0[0], e1[1] - e0[1]);
            let le = ex.hypot(ez).max(1e-6);
            let cos = ((dx * ex + dz * ez) / (ld * le)).abs();
            if cos < GABARIT_ANGLE.to_radians().cos() {
                continue;
            }
            let t = (((c[0] - e0[0]) * ex + (c[1] - e0[1]) * ez) / (le * le)).clamp(0.0, 1.0);
            let dist = (c[0] - e0[0] - ex * t).hypot(c[1] - e0[1] - ez * t);
            if dist <= GABARIT_PORTEE {
                h = Some(h.map_or(gb.hauteur, |x: f32| x.min(gb.hauteur)));
            }
        }
        h
    }
}

/// Les classes du lieu (voies rapides, bretelles : `sage_reel::circulation::Classe`),
/// leurs segments par case.
pub struct Classes<'a> {
    l: &'a Lieu,
    cases: HashMap<(i32, i32), Vec<(usize, usize)>>,
}

/// Une voie prend la classe d'un tracé qui longe (à moins de `CLASSE_PORTEE` m, à moins
/// de `GABARIT_ANGLE` près) au moins `CLASSE_PART` de ses points.
const CLASSE_PORTEE: f32 = 4.0;
const CLASSE_PART: f32 = 0.6;

impl<'a> Classes<'a> {
    pub fn new(l: &'a Lieu) -> Self {
        let mut cases: HashMap<(i32, i32), Vec<(usize, usize)>> = HashMap::new();
        for (g, gb) in l.classes.iter().enumerate() {
            for i in 0..gb.points.len().saturating_sub(1) {
                let (p, q) = (gb.points[i], gb.points[i + 1]);
                let (c0, c1) = (case(p[0].min(q[0]) - CLASSE_PORTEE, p[1].min(q[1]) - CLASSE_PORTEE), case(p[0].max(q[0]) + CLASSE_PORTEE, p[1].max(q[1]) + CLASSE_PORTEE));
                for ci in c0.0..=c1.0 {
                    for cj in c0.1..=c1.1 {
                        cases.entry((ci, cj)).or_default().push((g, i));
                    }
                }
            }
        }
        Classes { l, cases }
    }

    /// La classe au milieu du segment pq (0 : aucune).
    fn au_segment(&self, p: [f32; 3], q: [f32; 3]) -> u8 {
        let c = [(p[0] + q[0]) * 0.5, (p[1] + q[1]) * 0.5];
        let (dx, dz) = (q[0] - p[0], q[1] - p[1]);
        let ld = dx.hypot(dz).max(1e-6);
        let mut best = (f32::MAX, 0u8);
        for &(g, i) in self.cases.get(&case(c[0], c[1])).map(|v| v.as_slice()).unwrap_or(&[]) {
            let gb = &self.l.classes[g];
            let (e0, e1) = (gb.points[i], gb.points[i + 1]);
            let (ex, ez) = (e1[0] - e0[0], e1[1] - e0[1]);
            let le = ex.hypot(ez).max(1e-6);
            if ((dx * ex + dz * ez) / (ld * le)).abs() < GABARIT_ANGLE.to_radians().cos() {
                continue;
            }
            let t = (((c[0] - e0[0]) * ex + (c[1] - e0[1]) * ez) / (le * le)).clamp(0.0, 1.0);
            let dist = (c[0] - e0[0] - ex * t).hypot(c[1] - e0[1] - ez * t);
            if dist <= CLASSE_PORTEE && dist < best.0 {
                best = (dist, gb.classe);
            }
        }
        best.1
    }

    /// La classe d'une voie : celle qui longe au moins `CLASSE_PART` de sa longueur.
    pub fn classe(&self, a: &Arete) -> u8 {
        // Seules les grandes routes (le générateur y met motorway, trunk et leurs
        // bretelles) : une voie de service qui longe une voie rapide (au-dessus de son
        // tunnel) n'en prend pas la classe
        if a.genre != sage_reel::GRANDE {
            return 0;
        }
        let mut long = [0.0f32; 3];
        let mut tot = 0.0f32;
        for w in a.points.windows(2) {
            let l = (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]);
            tot += l;
            let c = self.au_segment(w[0], w[1]) as usize;
            if c < 3 {
                long[c] += l;
            }
        }
        (1..3).find(|&c| long[c] >= CLASSE_PART * tot.max(1e-3)).unwrap_or(0) as u8
    }
}

/// Le lissage des voies rapides : la demi-fenêtre (m) du relief lissé, la pente des
/// rampes vers un plancher, le dépassement (m) d'un plancher de part et d'autre de la
/// voie enjambée, la demi-fenêtre de l'arrondi final et son nombre de passes.
const RAPIDE_FENETRE: f32 = 60.0;
const RAPIDE_PENTE: f32 = 0.04;
const RAPIDE_MARGE: f32 = 2.0;
const RAPIDE_ARRONDI: f32 = 25.0;
const RAPIDE_PASSES: usize = 3;
/// Deux chaussées de voie rapide jumelles : à moins de tant (m) d'axe à axe, de tracés
/// parallèles (cosinus au moins), sur au moins tant de points.
const JUMELLE_DISTANCE: f32 = 15.0;
const JUMELLE_PARALLELE: f32 = 0.9;
const JUMELLE_POINTS: usize = 8;
/// Aux abords d'une entrée de tunnel (tant de mètres après un point au sol), pas de
/// plafond : le tunnel suit sa rampe d'accès.
const ENTREE_SANS_PLAFOND: f32 = 10.0;
/// La hauteur commune de deux jumelles s'impose entièrement à tant (m) du bord de leur
/// zone jumelée, progressivement avant.
const JUMELLE_RACCORD: f32 = 25.0;

/// Le bilan du lissage des voies rapides.
#[derive(Default, Debug)]
pub struct Rapides {
    /// Par arête : une voie rapide posée (fixe ensuite).
    pub fixes: Vec<bool>,
    pub chaines: usize,
    pub planchers: usize,
    /// Les paires de chaînes jumelles (deux chaussées d'une même voie), lissées ensemble.
    pub jumelles: usize,
}

/// Les voies rapides (classe `CLASSE_RAPIDE`, au sol ou en pont) posées avant toutes les
/// autres, chaîne par chaîne (à travers les nœuds où deux bouts de voie rapide se
/// suivent) : le relief (de leurs points au sol) lissé sur ±`RAPIDE_FENETRE`, relevé
/// au-dessus de chaque plancher (ce qu'elles enjambent : son relief, ou sa hauteur si
/// c'est un pont, ou l'eau ; plus l'écart) par des rampes à `RAPIDE_PENTE`, puis
/// arrondi sans repasser sous un plancher. Elles ne bougent plus ensuite : les autres
/// voies s'adaptent à elles (`niveler_fixe`, `plaques::poser_avec`).
pub fn lisser_rapides(r: &mut Reseau, l: &Lieu) -> Rapides {
    let na = r.aretes.len();
    let mut rapide = vec![false; na];
    {
        let classes = Classes::new(l);
        for (k, a) in r.aretes.iter().enumerate() {
            rapide[k] = a.points.len() >= 2 && (a.genre == RAIL || (sage_reel::carrossable(a.genre) && classes.classe(a) == sage_reel::circulation::CLASSE_RAPIDE));
        }
    }
    let mut bilan = Rapides { fixes: rapide.clone(), ..Default::default() };
    if !rapide.iter().any(|&x| x) {
        return bilan;
    }
    // Des points tous les RELIEF_PAS mètres au moins
    for k in 0..na {
        if rapide[k] {
            let d = densifier_xz(&r.aretes[k].points);
            r.aretes[k].points = d;
        }
    }
    let nn = r.noeuds.len();
    let mut bouts: Vec<Vec<usize>> = vec![Vec::new(); nn];
    for k in (0..na).filter(|&k| rapide[k]) {
        let a = &r.aretes[k];
        if a.a != a.b {
            bouts[a.a].push(k);
            bouts[a.b].push(k);
        }
    }
    // Les planchers : les contraintes dont le haut est une voie rapide et le bas une
    // autre voie rapide (lissée avant elle) ou l'eau ; sous un viaduc, aussi les voies au
    // sol, à leur relief. Ailleurs, une route ordinaire ou une bretelle dessous ne relève
    // pas la voie rapide : c'est elle qui s'adapte ensuite
    // (arête, segment, u) → (bas : voie rapide (arête, segment, u) ou niveau de l'eau,
    // écart, dépassement)
    let cs = contraintes(r, l);
    let mut planchers: HashMap<usize, Vec<(usize, f32, Result<Ou, f32>, f32, f32)>> = HashMap::new();
    for c in &cs {
        let (k, sg, u) = c.haut;
        if !rapide[k] {
            continue;
        }
        let (bas, demi) = match c.bas {
            Some(o) if rapide[o.0] => (Ok(o), r.aretes[o.0].largeur * 0.5 + RAPIDE_MARGE),
            // Un viaduc enjambe les voies au sol posées sur leur terrain : leur relief au
            // croisement (pas leur hauteur du moment, qu'elles adapteront) plus l'écart
            Some(o) if r.aretes[k].passage == Passage::Pont && r.aretes[o.0].passage == Passage::Sol => {
                let q = entre(r.aretes[o.0].points[o.1], r.aretes[o.0].points[o.1 + 1], o.2);
                (Err(l.relief(q[0] as f64, q[1] as f64) as f32), r.aretes[o.0].largeur * 0.5 + RAPIDE_MARGE)
            }
            Some(_) => continue,
            None => (Err(c.plancher), RAPIDE_MARGE * 2.0),
        };
        planchers.entry(k).or_default().push((sg, u, bas, c.ecart, demi));
    }
    // Les chaînes
    let mut chaines: Vec<Vec<(usize, bool)>> = Vec::new();
    let mut chaine_de = vec![usize::MAX; na];
    let mut vue = vec![false; na];
    for k0 in 0..na {
        if !rapide[k0] || vue[k0] {
            continue;
        }
        vue[k0] = true;
        // Vers l'arrière depuis k0.a, puis vers l'avant depuis k0.b
        let suivre = |depart: usize, k: usize, vue: &mut Vec<bool>| -> Vec<(usize, bool)> {
            let mut v = Vec::new();
            let (mut n, mut k) = (depart, k);
            for _ in 0..10_000 {
                if bouts[n].len() != 2 {
                    break;
                }
                let k2 = if bouts[n][0] == k { bouts[n][1] } else { bouts[n][0] };
                if vue[k2] || k2 == k {
                    break;
                }
                vue[k2] = true;
                let a = &r.aretes[k2];
                let endroit = a.a == n;
                v.push((k2, endroit));
                n = if endroit { a.b } else { a.a };
                k = k2;
            }
            v
        };
        let arriere = suivre(r.aretes[k0].a, k0, &mut vue);
        let avant = suivre(r.aretes[k0].b, k0, &mut vue);
        let mut chaine: Vec<(usize, bool)> = arriere.into_iter().rev().map(|(k, e)| (k, !e)).collect();
        chaine.push((k0, true));
        chaine.extend(avant);
        for &(k, _) in &chaine {
            chaine_de[k] = chaines.len();
        }
        chaines.push(chaine);
    }
    // Les points de chaque chaîne : (arête, indice, position, abscisse, au sol) ; les
    // joints (le premier point d'une voie après la première, le même que le dernier de
    // la précédente) en reçoivent la hauteur ; par voie, son sens et ses abscisses
    struct Etat {
        pts: Vec<(usize, usize, [f32; 3], f32, bool)>,
        joints: Vec<(usize, usize, usize)>,
        debut: HashMap<usize, (bool, Vec<f32>)>,
        h: Vec<f32>,
        strict: Vec<f32>,
        pl: Vec<(f32, f32, f32, f32)>,
    }
    let nc = chaines.len();
    let mut etats: Vec<Etat> = Vec::with_capacity(nc);
    for chaine in &chaines {
        let mut pts: Vec<(usize, usize, [f32; 3], f32, bool)> = Vec::new();
        let mut joints: Vec<(usize, usize, usize)> = Vec::new();
        let mut debut: HashMap<usize, (bool, Vec<f32>)> = HashMap::new();
        let mut s_ = 0.0f32;
        for &(k, endroit) in chaine {
            let a = &r.aretes[k];
            let m = a.points.len();
            let mut abs = vec![0.0f32; m];
            for x in 0..m {
                let i = if endroit { x } else { m - 1 - x };
                if x > 0 {
                    let j = if endroit { i - 1 } else { i + 1 };
                    s_ += (a.points[i][0] - a.points[j][0]).hypot(a.points[i][1] - a.points[j][1]);
                }
                abs[i] = s_;
                if x > 0 || pts.is_empty() {
                    pts.push((k, i, a.points[i], s_, a.passage == Passage::Sol));
                } else {
                    joints.push((k, i, pts.len() - 1));
                }
            }
            debut.insert(k, (endroit, abs));
        }
        etats.push(Etat { pts, joints, debut, h: Vec::new(), strict: Vec::new(), pl: Vec::new() });
    }
    // Les jumelles : pour chaque point, le point le plus proche d'une autre chaîne à
    // moins de JUMELLE_DISTANCE, de tracé parallèle (dans un sens ou l'autre)
    let dir = |e: &Etat, i: usize| -> [f32; 2] {
        let (a, b) = (i.saturating_sub(1), (i + 1).min(e.pts.len() - 1));
        let (p, q) = (e.pts[a].2, e.pts[b].2);
        let (dx, dz) = (q[0] - p[0], q[1] - p[1]);
        let l = dx.hypot(dz).max(1e-6);
        [dx / l, dz / l]
    };
    let mut grille: HashMap<(i32, i32), Vec<(usize, usize)>> = HashMap::new();
    let case = |p: [f32; 3]| ((p[0] / JUMELLE_DISTANCE).floor() as i32, (p[1] / JUMELLE_DISTANCE).floor() as i32);
    for (c, e) in etats.iter().enumerate() {
        for (i, p) in e.pts.iter().enumerate() {
            grille.entry(case(p.2)).or_default().push((c, i));
        }
    }
    let mut jumelle: Vec<Vec<Option<(usize, usize)>>> = etats.iter().map(|e| vec![None; e.pts.len()]).collect();
    for (c, e) in etats.iter().enumerate() {
        for i in 0..e.pts.len() {
            let p = e.pts[i].2;
            let d = dir(e, i);
            let (cx, cz) = case(p);
            let mut meilleur: Option<(f32, usize, usize)> = None;
            for dx in -1..=1 {
                for dz in -1..=1 {
                    for &(c2, j) in grille.get(&(cx + dx, cz + dz)).map_or(&[][..], |v| &v[..]) {
                        // Une voie ferrée ne se jumelle qu'avec une voie ferrée
                        if c2 == c || (r.aretes[e.pts[i].0].genre == RAIL) != (r.aretes[etats[c2].pts[j].0].genre == RAIL) {
                            continue;
                        }
                        let q = etats[c2].pts[j].2;
                        let dd = (q[0] - p[0]).hypot(q[1] - p[1]);
                        if dd > JUMELLE_DISTANCE || meilleur.is_some_and(|m| m.0 <= dd) {
                            continue;
                        }
                        let d2 = dir(&etats[c2], j);
                        if (d[0] * d2[0] + d[1] * d2[1]).abs() < JUMELLE_PARALLELE {
                            continue;
                        }
                        meilleur = Some((dd, c2, j));
                    }
                }
            }
            jumelle[c][i] = meilleur.map(|m| (m.1, m.2));
        }
    }
    // Les paires : deux chaînes jumelées sur assez de points sont lissées ensemble
    let mut groupe: Vec<usize> = (0..nc).collect();
    fn racine_g(g: &mut [usize], mut i: usize) -> usize {
        while g[i] != i {
            g[i] = g[g[i]];
            i = g[i];
        }
        i
    }
    let mut compte: HashMap<(usize, usize), usize> = HashMap::new();
    for c in 0..nc {
        for &(c2, _) in jumelle[c].iter().flatten() {
            *compte.entry((c.min(c2), c.max(c2))).or_default() += 1;
        }
    }
    for (&(a, b), &n) in &compte {
        if n >= JUMELLE_POINTS {
            let (ra, rb) = (racine_g(&mut groupe, a), racine_g(&mut groupe, b));
            groupe[ra] = rb;
            bilan.jumelles += 1;
        }
    }
    let g_de: Vec<usize> = (0..nc).map(|c| racine_g(&mut groupe, c)).collect();
    // Une jumelle n'est retenue qu'entre chaînes d'un même groupe
    for c in 0..nc {
        for j in jumelle[c].iter_mut() {
            if j.is_some_and(|(c2, _)| g_de[c2] != g_de[c]) {
                *j = None;
            }
        }
    }
    // L'ordre : du plus bas au plus haut ; un groupe passe après tous ceux qu'il enjambe
    // (un cycle, s'il y en a, est rompu dans l'ordre des groupes)
    let mut dessous: HashMap<usize, std::collections::HashSet<usize>> = HashMap::new();
    for (&k, v) in &planchers {
        for (_, _, bas, _, _) in v {
            if let Ok(o) = bas {
                let (ch, cb) = (chaine_de[k], chaine_de[o.0]);
                if ch != usize::MAX && cb != usize::MAX && g_de[ch] != g_de[cb] {
                    dessous.entry(g_de[ch]).or_default().insert(g_de[cb]);
                }
            }
        }
    }
    let mut groupes: Vec<usize> = g_de.clone();
    groupes.sort();
    groupes.dedup();
    let mut ordre: Vec<usize> = Vec::with_capacity(groupes.len());
    let mut faite: std::collections::HashSet<usize> = Default::default();
    while ordre.len() < groupes.len() {
        let avant = ordre.len();
        for &g in &groupes {
            if !faite.contains(&g) && dessous.get(&g).map_or(true, |d| d.iter().all(|x| faite.contains(x))) {
                faite.insert(g);
                ordre.push(g);
            }
        }
        if ordre.len() == avant {
            let g = *groupes.iter().find(|g| !faite.contains(g)).unwrap();
            faite.insert(g);
            ordre.push(g);
        }
    }
    // Le lissage en triangle sur ±demi des points (ceux que `garder` retient)
    fn lisser_tri(s: &[f32], h: &[f32], garder: &dyn Fn(usize) -> bool, demi: f32) -> Vec<f32> {
        let n = s.len();
        let mut out = h.to_vec();
        let mut j0 = 0usize;
        for i in 0..n {
            while j0 < n && s[j0] < s[i] - demi {
                j0 += 1;
            }
            let (mut sw, mut sh) = (0.0f32, 0.0f32);
            let mut j = j0;
            while j < n && s[j] <= s[i] + demi {
                if garder(j) {
                    let w = 1.0 - (s[j] - s[i]).abs() / demi;
                    sw += w;
                    sh += w * h[j];
                }
                j += 1;
            }
            if sw > 1e-6 {
                out[i] = sh / sw;
            }
        }
        out
    }
    let gab = Gabarits::new(l);
    let passage_de = |r: &Reseau, e: &Etat, i: usize| r.aretes[e.pts[i].0].passage;
    for g in ordre {
        let membres: Vec<usize> = (0..nc).filter(|&c| g_de[c] == g).collect();
        // 1. La base : le relief lissé (triangle sur ±RAPIDE_FENETRE) des points au sol ;
        // loin de tout point au sol (un long tunnel, un long pont), interpolée entre les
        // plus proches qui en ont
        let mut rels: HashMap<usize, Vec<f32>> = HashMap::new();
        for &c in &membres {
            bilan.chaines += 1;
            let e = &mut etats[c];
            let n = e.pts.len();
            let s: Vec<f32> = e.pts.iter().map(|p| p.3).collect();
            let rel: Vec<f32> = e.pts.iter().map(|p| l.relief(p.2[0] as f64, p.2[1] as f64) as f32).collect();
            let sol: Vec<bool> = e.pts.iter().map(|p| p.4).collect();
            let mut base = lisser_tri(&s, &rel, &|j| sol[j], RAPIDE_FENETRE);
            let connu: Vec<bool> = (0..n)
                .map(|i| {
                    let (a, b) = (s[i] - RAPIDE_FENETRE, s[i] + RAPIDE_FENETRE);
                    (0..n).any(|j| sol[j] && s[j] >= a && s[j] <= b)
                })
                .collect();
            if connu.iter().any(|&c| c) && connu.iter().any(|&c| !c) {
                let mut i = 0;
                while i < n {
                    if connu[i] {
                        i += 1;
                        continue;
                    }
                    let j0 = i;
                    while i < n && !connu[i] {
                        i += 1;
                    }
                    let (ga, gb) = (j0.checked_sub(1), (i < n).then_some(i));
                    for j in j0..i {
                        base[j] = match (ga, gb) {
                            (Some(a), Some(b)) => base[a] + (base[b] - base[a]) * (s[j] - s[a]) / (s[b] - s[a]).max(1e-3),
                            (Some(a), None) => base[a],
                            (None, Some(b)) => base[b],
                            (None, None) => e.pts[j].2[2],
                        };
                    }
                }
            }
            e.h = base;
            rels.insert(c, rel);
        }
        // Le poids de la hauteur commune en chaque point jumelé (en pont ou en tunnel des
        // deux côtés) : nul au bord de la zone jumelée, entier à JUMELLE_RACCORD à
        // l'intérieur ; entre les deux, un raccord sans marche vers la hauteur propre
        let mut poids: HashMap<usize, Vec<f32>> = HashMap::new();
        for &c in &membres {
            let e = &etats[c];
            let n = e.pts.len();
            let lie: Vec<bool> = (0..n)
                .map(|i| match jumelle[c][i] {
                    Some((c2, j)) => {
                        let (pa, pb) = (passage_de(r, e, i), passage_de(r, &etats[c2], j));
                        pa != Passage::Sol && pa == pb
                    }
                    None => false,
                })
                .collect();
            let mut d = vec![f32::MAX; n];
            for i in 0..n {
                if !lie[i] {
                    d[i] = 0.0;
                } else if i > 0 {
                    d[i] = d[i - 1] + (e.pts[i].3 - e.pts[i - 1].3);
                }
            }
            for i in (0..n.saturating_sub(1)).rev() {
                d[i] = d[i].min(d[i + 1] + (e.pts[i + 1].3 - e.pts[i].3));
            }
            poids.insert(c, d.iter().map(|&x| if x == f32::MAX { 1.0 } else { (x / JUMELLE_RACCORD).min(1.0) }).collect());
        }
        // La base commune des jumelles : en pont la plus haute des deux (le tablier passe
        // au-dessus de ce que chacune enjambe), en tunnel la moyenne
        let partager = |etats: &mut Vec<Etat>, r: &Reseau, f: &dyn Fn(Passage, f32, f32) -> f32| {
            let copie: Vec<Vec<f32>> = membres.iter().map(|&c| etats[c].h.clone()).collect();
            let rang = |c: usize| membres.iter().position(|&m| m == c).unwrap();
            for &c in &membres {
                for i in 0..etats[c].pts.len() {
                    let Some((c2, j)) = jumelle[c][i] else { continue };
                    let (pa, pb) = (passage_de(r, &etats[c], i), passage_de(r, &etats[c2], j));
                    if pa == Passage::Sol || pa != pb {
                        continue;
                    }
                    let (propre, commun) = (copie[rang(c)][i], f(pa, copie[rang(c)][i], copie[rang(c2)][j]));
                    etats[c].h[i] = propre + poids[&c][i] * (commun - propre);
                }
            }
        };
        partager(&mut etats, r, &|p, a, b| if p == Passage::Pont { a.max(b) } else { (a + b) * 0.5 });
        // Les plafonds : sous le relief, un tunnel garde sa hauteur nécessaire (la hauteur
        // limitée et la dalle, sinon le gabarit et l'épaisseur) ; on y descend par des
        // rampes à RAPIDE_PENTE (balayage dans les deux sens)
        for &c in &membres {
            let rel = &rels[&c];
            let e = &mut etats[c];
            let n = e.pts.len();
            let mut plafond: Vec<f32> = vec![f32::MAX; n];
            // La distance (le long de la chaîne) au point au sol le plus proche : aux
            // abords d'une entrée, le tunnel suit sa rampe, la dalle n'existe qu'au-delà
            let mut pres = vec![f32::MAX; n];
            for i in 0..n {
                if e.pts[i].4 {
                    pres[i] = 0.0;
                } else if i > 0 {
                    pres[i] = pres[i - 1] + (e.pts[i].3 - e.pts[i - 1].3);
                }
            }
            for i in (0..n.saturating_sub(1)).rev() {
                pres[i] = pres[i].min(pres[i + 1] + (e.pts[i + 1].3 - e.pts[i].3));
            }
            for i in 0..n {
                let (k, idx) = (e.pts[i].0, e.pts[i].1);
                let a = &r.aretes[k];
                if a.passage != Passage::Tunnel || pres[i] < ENTREE_SANS_PLAFOND {
                    continue;
                }
                let j = idx.min(a.points.len() - 2);
                let requis = gab.hauteur(a.points[j], a.points[j + 1], if idx == j { 0.0 } else { 1.0 }).map_or(GABARIT + EPAISSEUR, |g| g + DALLE_SOUS_SOL);
                plafond[i] = rel[i] - requis;
            }
            e.strict = plafond.clone();
            for i in 1..n {
                plafond[i] = plafond[i].min(plafond[i - 1] + RAPIDE_PENTE * (e.pts[i].3 - e.pts[i - 1].3));
            }
            for i in (0..n.saturating_sub(1)).rev() {
                plafond[i] = plafond[i].min(plafond[i + 1] + RAPIDE_PENTE * (e.pts[i + 1].3 - e.pts[i].3));
            }
            for i in 0..n {
                e.h[i] = e.h[i].min(plafond[i]);
            }
        }
        // 2. Les planchers de chaque chaîne, à leur abscisse : (abscisse, hauteur,
        // dépassement, position) ; ceux d'une jumelle valent aussi pour l'autre (le
        // tablier est commun)
        let mut propres: Vec<(usize, f32, f32, f32, [f32; 2])> = Vec::new();
        for &c in &membres {
            for &(k, _) in &chaines[c] {
                let Some(v) = planchers.get(&k) else { continue };
                let (_, abs) = &etats[c].debut[&k];
                for &(sg, u, bas, ecart, demi) in v {
                    let hb = match bas {
                        Ok((kb, sb, ub)) => entre(r.aretes[kb].points[sb], r.aretes[kb].points[sb + 1], ub)[2],
                        Err(x) => x,
                    };
                    let q = entre(r.aretes[k].points[sg], r.aretes[k].points[sg + 1], u);
                    propres.push((c, abs[sg] + (abs[sg + 1] - abs[sg]) * u, hb + ecart, demi, [q[0], q[1]]));
                }
            }
        }
        bilan.planchers += propres.len();
        for &c in &membres {
            let e = &mut etats[c];
            let base0 = e.h.clone();
            let s: Vec<f32> = e.pts.iter().map(|p| p.3).collect();
            let mut pl: Vec<(f32, f32, f32, f32)> = Vec::new();
            for &(c2, sp, hp, demi, q) in &propres {
                let x = if c2 == c {
                    Some(sp)
                } else {
                    // Sur la jumelle : le point le plus proche de ce plancher
                    e.pts.iter().map(|p| ((p.2[0] - q[0]).hypot(p.2[1] - q[1]), p.3)).filter(|d| d.0 <= JUMELLE_DISTANCE + demi).min_by(|a, b| a.0.total_cmp(&b.0)).map(|d| d.1)
                };
                let Some(x) = x else { continue };
                // L'excédent du plancher sur la base, qui s'éteint à RAPIDE_PENTE : la
                // rampe suit le terrain, elle ne porte pas une hauteur au loin
                let i = s.partition_point(|&y| y < x).min(s.len() - 1);
                pl.push((x, hp, demi, hp - base0[i]));
            }
            for i in 0..e.pts.len() {
                let si = e.pts[i].3;
                let cone = pl
                    .iter()
                    .filter(|p| p.3 > 0.0)
                    .map(|p| {
                        let reste = p.3 - RAPIDE_PENTE * ((si - p.0).abs() - p.2).max(0.0);
                        if reste > 0.0 { base0[i] + reste } else { f32::MIN }
                    })
                    .fold(f32::MIN, f32::max);
                e.h[i] = e.h[i].max(cone);
            }
            e.pl = pl;
        }
        // Les jumelles en pont prennent la plus haute des deux, en tunnel la plus basse
        let haut_ou_bas = |r: &Reseau, etats: &mut Vec<Etat>| {
            let copie: Vec<Vec<f32>> = membres.iter().map(|&c| etats[c].h.clone()).collect();
            let rang = |c: usize| membres.iter().position(|&m| m == c).unwrap();
            for &c in &membres {
                for i in 0..etats[c].pts.len() {
                    let Some((c2, j)) = jumelle[c][i] else { continue };
                    let (pa, pb) = (passage_de(r, &etats[c], i), passage_de(r, &etats[c2], j));
                    if pa == Passage::Sol || pa != pb {
                        continue;
                    }
                    let (x, y) = (copie[rang(c)][i], copie[rang(c2)][j]);
                    let commun = if pa == Passage::Pont { x.max(y) } else { x.min(y) };
                    etats[c].h[i] = x + poids[&c][i] * (commun - x);
                }
            }
        };
        haut_ou_bas(r, &mut etats);
        // 3. L'arrondi, sans repasser sous un plancher ni au-dessus d'un plafond ; puis
        // les jumelles en pont ou en tunnel à la même hauteur (la moyenne)
        let borner = |e: &mut Etat| {
            for i in 0..e.pts.len() {
                let si = e.pts[i].3;
                let plateau = e.pl.iter().filter(|p| (si - p.0).abs() <= p.2).map(|p| p.1).fold(f32::MIN, f32::max);
                e.h[i] = e.h[i].min(e.strict[i]).max(plateau);
            }
        };
        for _ in 0..RAPIDE_PASSES {
            for &c in &membres {
                let e = &mut etats[c];
                let s: Vec<f32> = e.pts.iter().map(|p| p.3).collect();
                e.h = lisser_tri(&s, &e.h, &|_| true, RAPIDE_ARRONDI);
                borner(e);
            }
        }
        if membres.len() > 1 {
            partager(&mut etats, r, &|_, a, b| (a + b) * 0.5);
            for &c in &membres {
                borner(&mut etats[c]);
            }
        }
        // Rendues aux arêtes et aux nœuds
        for &c in &membres {
            let e = &etats[c];
            for (x, p) in e.pts.iter().enumerate() {
                r.aretes[p.0].points[p.1][2] = e.h[x];
            }
            for &(k, i, x) in &e.joints {
                r.aretes[k].points[i][2] = e.h[x];
            }
            for &(k, _) in &chaines[c] {
                let a = &r.aretes[k];
                let (na_, nb_) = (a.a, a.b);
                let (ha, hb) = (a.points[0][2], a.points[a.points.len() - 1][2]);
                r.noeuds[na_].p[2] = ha;
                r.noeuds[nb_].p[2] = hb;
            }
        }
    }
    // Les autres arêtes qui touchent un nœud de voie rapide y prennent sa hauteur
    for k in 0..na {
        if rapide[k] {
            continue;
        }
        let (a, b) = (r.aretes[k].a, r.aretes[k].b);
        let m = r.aretes[k].points.len();
        if !bouts[a].is_empty() {
            r.aretes[k].points[0][2] = r.noeuds[a].p[2];
        }
        if !bouts[b].is_empty() {
            r.aretes[k].points[m - 1][2] = r.noeuds[b].p[2];
        }
    }
    bilan
}

/// Le nom d'une classe.
pub fn nom_classe(c: u8) -> &'static str {
    match c {
        sage_reel::circulation::CLASSE_RAPIDE => "voie rapide",
        sage_reel::circulation::CLASSE_BRETELLE => "bretelle",
        _ => "",
    }
}

/// La pente la plus forte (degrés) d'une rampe d'entrée de souterrain.
pub const RAMPE_SOUTERRAIN: f32 = 15.0;
/// Un souterrain (avec ceux qui le prolongent) plus court que tant (m) est un passage
/// sous un bâtiment ou un auvent : il reste au sol.
pub const SOUTERRAIN_COURT: f32 = 20.0;

/// Les souterrains avant le nivellement : on se fie au relief de surface. Les très
/// courts sont remis au sol ; les autres sont enfoncés sous le relief adouci de la
/// hauteur limitée connue plus la dalle sous le sol (sinon `GABARIT + EPAISSEUR`) sur
/// toute leur longueur ; les voies au sol qui y mènent descendent en tranchée, à la
/// pente de la rampe au plus, jusqu'au premier carrefour (s'il manque de la place, la
/// rampe finit dans le souterrain). Une voie au sol de niveau négatif devient un
/// souterrain ; un souterrain très court sans voie au-dessus est remis au sol ; les
/// voies qui traversent des souterrains forment des tabliers.
pub fn preparer_souterrains(r: &mut Reseau, l: &Lieu) -> Souterrains {
    if l.h.is_empty() {
        return Souterrains::default();
    }
    let n = r.aretes.len();
    // Une voie au sol d'un niveau négatif (`layer` d'OSM sans `tunnel`) est un souterrain
    let mut negatifs = 0;
    for a in r.aretes.iter_mut().filter(|a| a.passage == Passage::Sol && a.niveau < 0 && a.genre != RAIL) {
        a.passage = Passage::Tunnel;
        negatifs += 1;
    }
    let tunnel = |r: &Reseau, k: usize| r.aretes[k].passage == Passage::Tunnel;
    // Les groupes de souterrains qui se prolongent
    let mut chef: Vec<usize> = (0..n).collect();
    fn racine(c: &mut [usize], mut i: usize) -> usize {
        while c[i] != i {
            c[i] = c[c[i]];
            i = c[i];
        }
        i
    }
    for no in &r.noeuds {
        let t: Vec<usize> = no.aretes.iter().copied().filter(|&k| tunnel(r, k)).collect();
        for w in t.windows(2) {
            let (a, b) = (racine(&mut chef, w[0]), racine(&mut chef, w[1]));
            chef[a] = b;
        }
    }
    let longueur = |a: &Arete| a.points.windows(2).map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1])).sum::<f32>();
    let mut total: HashMap<usize, f32> = HashMap::new();
    for k in (0..n).filter(|&k| tunnel(r, k)) {
        *total.entry(racine(&mut chef, k)).or_default() += longueur(&r.aretes[k]);
    }
    // Les groupes que traverse une voie au-dessus (une voie qui n'est pas un souterrain,
    // ou un souterrain d'un niveau plus haut)
    let mut couverts: std::collections::HashSet<usize> = std::collections::HashSet::new();
    for x in croisements(r) {
        let (a, b) = (&r.aretes[x.0], &r.aretes[x.3]);
        for (t, o) in [(x.0, b), (x.3, a)] {
            let at = &r.aretes[t];
            if at.passage == Passage::Tunnel && (o.passage != Passage::Tunnel || o.niveau > at.niveau) {
                couverts.insert(racine(&mut chef, t));
            }
        }
    }
    // 1. Les très courts sans voie au-dessus (passages sous un bâtiment, un auvent) : au
    //    sol, relus sur le relief
    let mut courts = 0;
    for k in 0..n {
        let g = racine(&mut chef, k);
        if tunnel(r, k) && total.get(&g).is_some_and(|&t| t < SOUTERRAIN_COURT) && !couverts.contains(&g) {
            r.aretes[k].passage = Passage::Sol;
            r.aretes[k].points = relire(l, &r.aretes[k].points);
            courts += 1;
        }
    }
    // 2. Le fond : à pleine profondeur sous le relief adouci (la hauteur limitée plus la
    //    dalle, sinon le gabarit d'une route ou d'un piéton)
    let gabarits = Gabarits::new(l);
    let mut enfonces = 0;
    let souterrains: Vec<usize> = (0..n).filter(|&k| tunnel(r, k)).collect();
    for &k in &souterrains {
        let pts = densifier_xz(&r.aretes[k].points);
        let route = sage_reel::carrossable(r.aretes[k].genre);
        let v: Vec<[f32; 3]> = (0..pts.len())
            .map(|i| {
                let p = pts[i];
                let (p0, p1, u) = if i + 1 < pts.len() { (pts[i], pts[i + 1], 0.0) } else { (pts[i - 1], pts[i], 1.0) };
                let requis = gabarits.hauteur(p0, p1, u).map_or(if route { GABARIT + EPAISSEUR } else { GABARIT_PIETON + EPAISSEUR_PIETON }, |g| g + DALLE_SOUS_SOL);
                [p[0], p[1], relief_adouci(l, p[0] as f64, p[1] as f64) - requis]
            })
            .collect();
        r.aretes[k].points = v;
        enfonces += 1;
    }
    let nn = r.noeuds.len();
    let bout = |a: &Arete, i: usize| if a.a == i { a.points[0][2] } else { a.points[a.points.len() - 1][2] };
    let sol = |r: &Reseau, k: usize| r.aretes[k].passage == Passage::Sol;
    // Le degré d'un nœud pour une sorte de voies (carrossables ou non)
    let degre = |r: &Reseau, i: usize, route: bool| r.noeuds[i].aretes.iter().filter(|&&k| sage_reel::carrossable(r.aretes[k].genre) == route).count();
    // 3. Les entrées : la hauteur du souterrain à l'entrée, relevée s'il n'y a pas la
    //    place de descendre en tranchée avant le premier carrefour (la fin de la rampe se
    //    fait alors dans le souterrain)
    let pente = RAMPE_SOUTERRAIN.to_radians().tan();
    // (hauteur, carrossable, tranchée adoucie au pied : le souterrain est au fond dès
    // l'entrée)
    let mut entree: Vec<Option<(f32, bool, bool)>> = vec![None; nn];
    for i in 0..nn {
        let no = &r.noeuds[i];
        let t: Vec<usize> = no.aretes.iter().copied().filter(|&k| tunnel(r, k)).collect();
        if t.is_empty() || no.aretes.iter().all(|&k| tunnel(r, k)) {
            continue;
        }
        let route = t.iter().any(|&k| sage_reel::carrossable(r.aretes[k].genre));
        let fond = t.iter().map(|&k| bout(&r.aretes[k], i)).fold(f32::MAX, f32::min);
        let mut h = fond;
        // Ce que demande chaque tranchée : adoucie au pied, ou droite
        let (mut h_doux, mut h_droit) = (fond, fond);
        if degre(r, i, route) > 2 {
            // L'entrée est un carrefour : elle reste au sol
            let s: Vec<f32> = no.aretes.iter().filter(|&&k| sol(r, k)).map(|&k| bout(&r.aretes[k], i)).collect();
            if !s.is_empty() {
                h = h.max(s.iter().sum::<f32>() / s.len() as f32);
            }
        } else {
            for &k0 in no.aretes.iter().filter(|&&k| sol(r, k) && sage_reel::carrossable(r.aretes[k].genre) == route) {
                // La tranchée suit la voie à travers les nœuds de deux voies jusqu'au
                // premier carrefour, où elle doit avoir rejoint le sol
                let (mut cur, mut k, mut long) = (i, k0, 0.0f32);
                for _ in 0..200 {
                    let a = &r.aretes[k];
                    long += longueur(a);
                    let j = if a.a == cur { a.b } else { a.a };
                    let suite: Vec<usize> = r.noeuds[j].aretes.iter().copied().filter(|&x| x != k && sage_reel::carrossable(r.aretes[x].genre) == route).collect();
                    if suite.len() == 1 && sol(r, suite[0]) && degre(r, j, route) == 2 {
                        cur = j;
                        k = suite[0];
                        continue;
                    }
                    // Une autre entrée au bout : une tranchée ouverte entre deux parties
                    // couvertes, qui ne remonte pas
                    let autre_entree = suite.iter().any(|&x| tunnel(r, x)) && long <= TRANCHEE_OUVERTE;
                    if !suite.is_empty() && !autre_entree {
                        h_doux = h_doux.max(bout(a, j) - pente * adoucie(long));
                        h_droit = h_droit.max(bout(a, j) - pente * long);
                    }
                    break;
                }
            }
        }
        // Assez de place pour une tranchée adoucie : le souterrain reste au fond ; sinon
        // la rampe, droite, finit dans le souterrain
        let doux = h <= fond + 1e-3 && h_doux <= fond + 1e-3;
        if !doux {
            h = h.max(h_droit);
        }
        entree[i] = Some((h, route, doux));
    }
    // La rampe dans les souterrains : depuis les entrées, au plus raide de la rampe
    let mut borne = vec![f32::MIN; nn];
    let mut file = std::collections::BinaryHeap::new();
    for i in 0..nn {
        if let Some((h, _, _)) = entree[i] {
            borne[i] = h;
            file.push((ordre(h), i));
        }
    }
    while let Some((b, i)) = file.pop() {
        if b != ordre(borne[i]) {
            continue;
        }
        for &k in &r.noeuds[i].aretes {
            if !tunnel(r, k) {
                continue;
            }
            let a = &r.aretes[k];
            let j = if a.a == i { a.b } else { a.a };
            let v = borne[i] - pente * longueur(a);
            if v > borne[j] {
                borne[j] = v;
                file.push((ordre(v), j));
            }
        }
    }
    for &k in &souterrains {
        let (na, nb) = (r.aretes[k].a, r.aretes[k].b);
        let tot = longueur(&r.aretes[k]);
        let a = &mut r.aretes[k];
        let mut s = 0.0f32;
        for i in 0..a.points.len() {
            if i > 0 {
                s += (a.points[i][0] - a.points[i - 1][0]).hypot(a.points[i][1] - a.points[i - 1][1]);
            }
            let rampe = (borne[na] - pente * s).max(borne[nb] - pente * (tot - s));
            // Le pied de la rampe arrondi
            if rampe > a.points[i][2] - 2.0 * RAMPE_ARRONDI {
                a.points[i][2] = smax(a.points[i][2], rampe, RAMPE_ARRONDI);
            }
        }
    }
    // Les nœuds tout en souterrain : la moyenne de leurs bouts
    for i in 0..nn {
        let no = &r.noeuds[i];
        if no.aretes.is_empty() || !no.aretes.iter().all(|&k| tunnel(r, k)) {
            continue;
        }
        let hs: Vec<f32> = no.aretes.iter().map(|&k| bout(&r.aretes[k], i)).collect();
        r.noeuds[i].p[2] = hs.iter().sum::<f32>() / hs.len() as f32;
    }
    // 4. Les tranchées : depuis chaque entrée, les voies au sol de la même sorte
    //    descendent au plus à la pente de la rampe, à travers les nœuds de deux voies
    //    seulement (pas au-delà d'un carrefour)
    // L'état de chaque nœud d'une tranchée : (plafond, carrossable, hauteur de
    // l'entrée, adoucie, distance à l'entrée)
    let mut plafond: Vec<(f32, bool, f32, bool, f32)> = vec![(f32::MAX, false, 0.0, false, 0.0); nn];
    let haut = |h: f32, doux: bool, d: f32| h + pente * if doux { adoucie(d) } else { d };
    let mut file = std::collections::BinaryHeap::new();
    for i in 0..nn {
        if let Some((h, route, doux)) = entree[i] {
            plafond[i] = (h, route, h, doux, 0.0);
            file.push((std::cmp::Reverse(ordre(h)), i));
        }
    }
    while let Some((std::cmp::Reverse(b), i)) = file.pop() {
        if b != ordre(plafond[i].0) {
            continue;
        }
        let (_, route, he, doux, d0) = plafond[i];
        if entree[i].is_none() && degre(r, i, route) != 2 {
            continue;
        }
        for &k in &r.noeuds[i].aretes {
            let a = &r.aretes[k];
            if !sol(r, k) || sage_reel::carrossable(a.genre) != route {
                continue;
            }
            let j = if a.a == i { a.b } else { a.a };
            if degre(r, j, route) != 2 || entree[j].is_some() {
                continue;
            }
            let d = d0 + longueur(a);
            let v = haut(he, doux, d);
            if v < plafond[j].0 && v < bout(a, j) + 0.01 {
                plafond[j] = (v, route, he, doux, d);
                file.push((std::cmp::Reverse(ordre(v)), j));
            }
        }
    }
    let mut tranchees = 0;
    for k in 0..n {
        if !sol(r, k) {
            continue;
        }
        let (na, nb) = (r.aretes[k].a, r.aretes[k].b);
        let route = sage_reel::carrossable(r.aretes[k].genre);
        let etat = |x: usize| Some(plafond[x]).filter(|p| p.0 < f32::MAX && p.1 == route);
        let (ea, eb) = (etat(na), etat(nb));
        if ea.is_none() && eb.is_none() {
            continue;
        }
        let tot = longueur(&r.aretes[k]);
        let a = &mut r.aretes[k];
        let mut s = 0.0f32;
        let mut touche = false;
        for i in 0..a.points.len() {
            if i > 0 {
                s += (a.points[i][0] - a.points[i - 1][0]).hypot(a.points[i][1] - a.points[i - 1][1]);
            }
            let ca = ea.map_or(f32::MAX, |e| haut(e.2, e.3, e.4 + s));
            let cb = eb.map_or(f32::MAX, |e| haut(e.2, e.3, e.4 + tot - s));
            let c = ca.min(cb);
            let h0 = a.points[i][2];
            if c > h0 + 2.0 * RAMPE_ARRONDI {
                continue;
            }
            // La tête de la tranchée arrondie (pas près d'un bout sans tranchée : un
            // carrefour, qui reste au sol)
            let libre = if ea.is_none() { tot - s } else if eb.is_none() { s } else { f32::MAX };
            let k_ = RAMPE_ARRONDI * (libre / 4.0).clamp(0.0, 1.0);
            let h = if k_ > 1e-3 { smin(h0, c, k_) } else { h0.min(c) };
            if h < h0 - 1e-3 {
                a.points[i][2] = h;
                touche = true;
            }
        }
        if touche {
            tranchees += 1;
        }
    }
    for i in 0..nn {
        if plafond[i].0 < r.noeuds[i].p[2] {
            r.noeuds[i].p[2] = plafond[i].0;
        }
    }
    // 4 bis. Les tranchées ouvertes : une suite de voies au sol (par des nœuds de deux
    //    voies) qui relie deux entrées à moins de TRANCHEE_OUVERTE mètres reste au fond,
    //    en ligne droite d'une entrée à l'autre (jamais au-dessus du sol)
    let mut vues = std::collections::HashSet::new();
    for i in 0..nn {
        let Some((hi, route, _)) = entree[i] else { continue };
        let departs: Vec<usize> = r.noeuds[i].aretes.iter().copied().filter(|&k| sol(r, k) && sage_reel::carrossable(r.aretes[k].genre) == route).collect();
        for k0 in departs {
            if vues.contains(&k0) {
                continue;
            }
            let (mut cur, mut k, mut long) = (i, k0, 0.0f32);
            let mut chaine: Vec<(usize, bool, f32)> = Vec::new();
            let mut fin: Option<usize> = None;
            for _ in 0..200 {
                let a = &r.aretes[k];
                let endroit = a.a == cur;
                chaine.push((k, endroit, long));
                long += longueur(a);
                let j = if endroit { a.b } else { a.a };
                if entree[j].is_some_and(|e| e.1 == route) {
                    fin = Some(j);
                    break;
                }
                let suite: Vec<usize> = r.noeuds[j].aretes.iter().copied().filter(|&x| x != k && sage_reel::carrossable(r.aretes[x].genre) == route).collect();
                if suite.len() == 1 && sol(r, suite[0]) && degre(r, j, route) == 2 && long <= TRANCHEE_OUVERTE {
                    cur = j;
                    k = suite[0];
                    continue;
                }
                break;
            }
            let Some(j) = fin else { continue };
            if j == i || long > TRANCHEE_OUVERTE || long < 1e-3 {
                continue;
            }
            let hj = entree[j].unwrap().0;
            for &(k, endroit, debut) in &chaine {
                vues.insert(k);
                let a = &mut r.aretes[k];
                let tot: f32 = a.points.windows(2).map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1])).sum();
                let mut s_ = 0.0f32;
                let m = a.points.len();
                for x in 0..m {
                    let idx = if endroit { x } else { m - 1 - x };
                    if x > 0 {
                        let jdx = if endroit { idx - 1 } else { idx + 1 };
                        s_ += (a.points[idx][0] - a.points[jdx][0]).hypot(a.points[idx][1] - a.points[jdx][1]);
                    }
                    let t = ((debut + s_.min(tot)) / long).clamp(0.0, 1.0);
                    let ligne = hi + (hj - hi) * t;
                    if ligne < a.points[idx][2] {
                        a.points[idx][2] = ligne;
                    }
                }
                let (na, nb) = (a.a, a.b);
                let (ha, hb) = (a.points[0][2], a.points[m - 1][2]);
                r.noeuds[na].p[2] = r.noeuds[na].p[2].min(ha);
                r.noeuds[nb].p[2] = r.noeuds[nb].p[2].min(hb);
            }
        }
    }
    // Les tabliers au-dessus des souterrains : après les voies rapides, quand la
    // profondeur des souterrains est connue (`poser_tabliers`)
    Souterrains { negatifs, courts, enfonces, tranchees, tabliers: 0 }
}

/// Les croisements propres (en xz) entre arêtes qui ne se touchent pas :
/// (arête, segment, avancée, autre arête, segment, avancée).
fn croisements(r: &Reseau) -> Vec<(usize, usize, f32, usize, usize, f32)> {
    let mut cases: HashMap<(i32, i32), Vec<(usize, usize)>> = HashMap::new();
    for (k, a) in r.aretes.iter().enumerate() {
        for i in 0..a.points.len().saturating_sub(1) {
            let (p, q) = (a.points[i], a.points[i + 1]);
            let (c0, c1) = (case(p[0].min(q[0]), p[1].min(q[1])), case(p[0].max(q[0]), p[1].max(q[1])));
            for ci in c0.0..=c1.0 {
                for cj in c0.1..=c1.1 {
                    cases.entry((ci, cj)).or_default().push((k, i));
                }
            }
        }
    }
    let mut vus = std::collections::HashSet::new();
    let mut v = Vec::new();
    for liste in cases.values() {
        for (x, &(k1, s1)) in liste.iter().enumerate() {
            for &(k2, s2) in &liste[x + 1..] {
                if k1 == k2 {
                    continue;
                }
                let (a1, a2) = (&r.aretes[k1], &r.aretes[k2]);
                let Some((u1, u2)) = croise(a1.points[s1], a1.points[s1 + 1], a2.points[s2], a2.points[s2 + 1]) else { continue };
                if vus.insert(((k1, s1).min((k2, s2)), (k1, s1).max((k2, s2)))) {
                    v.push((k1, s1, u1, k2, s2, u2));
                }
            }
        }
    }
    v
}

/// Les voies au sol qui traversent des souterrains à moins de `TABLIER_LIEN` mètres
/// les unes des autres forment un tablier (un ouvrage, à peu près plan) : un plan
/// ajusté sur leurs hauteurs de part et d'autre (pente plafonnée à `TABLIER_PENTE`),
/// chacune posée dessus au-dessus des souterrains et raccordée au-delà. Rend le
/// nombre de tabliers.
/// Les tabliers au-dessus des souterrains proches de la surface : les voies au sol qui
/// les traversent (pas les voies fixes), à moins de `TABLIER_PROCHE` mètres au-delà de
/// l'écart nécessaire, posées sur un plan commun. Un souterrain plus profond n'en a pas
/// besoin : les voies au-dessus suivent le relief.
pub fn poser_tabliers(r: &mut Reseau) -> usize {
    // Les traversées : (voie au sol, abscisse sur elle, x, z, demi-largeur du souterrain)
    let abscisses = |a: &Arete| -> Vec<f32> {
        let mut v = vec![0.0f32];
        for w in a.points.windows(2) {
            let l = v[v.len() - 1];
            v.push(l + (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]));
        }
        v
    };
    let mut tr: Vec<(usize, f32, f32, f32, f32)> = Vec::new();
    for x in croisements(r) {
        for (s, ss, su, t) in [(x.0, x.1, x.2, x.3), (x.3, x.4, x.5, x.0)] {
            let (a, b) = (&r.aretes[s], &r.aretes[t]);
            if a.passage != Passage::Sol || b.passage != Passage::Tunnel || r.fixes.contains(&(s as u32)) {
                continue;
            }
            let ab = abscisses(a);
            let c = ab[ss] + (ab[ss + 1] - ab[ss]) * su;
            let p = entre(a.points[ss], a.points[ss + 1], su);
            // Le souterrain est-il proche ? L'écart à lui, au-delà de l'écart nécessaire
            let (tt, tu) = if t == x.3 { (x.4, x.5) } else { (x.1, x.2) };
            let q = entre(b.points[tt], b.points[tt + 1], tu);
            let requis = if b.genre == sage_reel::PIETONNE { GABARIT_PIETON + EPAISSEUR_PIETON } else { GABARIT + EPAISSEUR };
            if p[2] - q[2] > requis + TABLIER_PROCHE {
                continue;
            }
            tr.push((s, c, p[0], p[1], b.largeur * 0.5));
        }
    }
    if tr.is_empty() {
        return 0;
    }
    // Les tabliers : les traversées proches réunies
    let m = tr.len();
    let mut chef: Vec<usize> = (0..m).collect();
    fn racine(c: &mut [usize], mut i: usize) -> usize {
        while c[i] != i {
            c[i] = c[c[i]];
            i = c[i];
        }
        i
    }
    let mut grille: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
    for (i, t) in tr.iter().enumerate() {
        grille.entry(((t.2 / TABLIER_LIEN).floor() as i32, (t.3 / TABLIER_LIEN).floor() as i32)).or_default().push(i);
    }
    for i in 0..m {
        let c = ((tr[i].2 / TABLIER_LIEN).floor() as i32, (tr[i].3 / TABLIER_LIEN).floor() as i32);
        for dx in -1..=1 {
            for dz in -1..=1 {
                for &j in grille.get(&(c.0 + dx, c.1 + dz)).map_or(&[][..], |v| &v[..]) {
                    if j > i && (tr[i].2 - tr[j].2).hypot(tr[i].3 - tr[j].3) < TABLIER_LIEN {
                        let (a, b) = (racine(&mut chef, i), racine(&mut chef, j));
                        chef[a] = b;
                    }
                }
            }
        }
    }
    let mut groupes: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..m {
        groupes.entry(racine(&mut chef, i)).or_default().push(i);
    }
    let mut n = 0;
    for (_, g) in groupes {
        // La zone de chaque voie : de la première à la dernière traversée, plus la
        // demi-largeur du souterrain et une marge
        let mut zones: HashMap<usize, (f32, f32)> = HashMap::new();
        for &i in &g {
            let (k, c, _, _, w) = tr[i];
            let e = zones.entry(k).or_insert((f32::MAX, f32::MIN));
            e.0 = e.0.min(c - w - TABLIER_MARGE);
            e.1 = e.1.max(c + w + TABLIER_MARGE);
        }
        // Le plan : sur les hauteurs des voies aux bords de leur zone
        let mut ech: Vec<(f32, f32, f32)> = Vec::new();
        for (&k, &(s0, s1)) in &zones {
            let a = &r.aretes[k];
            let ab = abscisses(a);
            let tot = ab[ab.len() - 1];
            for x in [s0.max(0.0), s1.min(tot)] {
                let i = ab.partition_point(|&y| y < x).clamp(1, a.points.len() - 1);
                let t = ((x - ab[i - 1]) / (ab[i] - ab[i - 1]).max(1e-6)).clamp(0.0, 1.0);
                let p = entre(a.points[i - 1], a.points[i], t);
                ech.push((p[0], p[1], p[2]));
            }
        }
        let w = ech.len() as f32;
        let (cx, cz, ch) = (ech.iter().map(|e| e.0).sum::<f32>() / w, ech.iter().map(|e| e.1).sum::<f32>() / w, ech.iter().map(|e| e.2).sum::<f32>() / w);
        let (mut a11, mut a12, mut a22, mut b1, mut b2) = (0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64);
        for &(x, z, h) in &ech {
            let (dx, dz, dh) = ((x - cx) as f64, (z - cz) as f64, (h - ch) as f64);
            a11 += dx * dx;
            a12 += dx * dz;
            a22 += dz * dz;
            b1 += dx * dh;
            b2 += dz * dh;
        }
        let det = a11 * a22 - a12 * a12;
        let mut gr = if det.abs() > 1e-3 { [((b1 * a22 - b2 * a12) / det) as f32, ((a11 * b2 - a12 * b1) / det) as f32] } else { [0.0, 0.0] };
        let mg = gr[0].hypot(gr[1]);
        if mg > TABLIER_PENTE {
            gr = [gr[0] * TABLIER_PENTE / mg, gr[1] * TABLIER_PENTE / mg];
        }
        let plan = |x: f32, z: f32| ch + gr[0] * (x - cx) + gr[1] * (z - cz);
        // Chaque voie sur le plan dans sa zone, raccordée au-delà
        for &k in zones.keys() {
            r.tabliers.push(k as u32);
        }
        for (&k, &(s0, s1)) in &zones {
            let a = &mut r.aretes[k];
            let ab = abscisses(a);
            for i in 0..a.points.len() {
                let c = ab[i];
                let d = if c < s0 { s0 - c } else if c > s1 { c - s1 } else { 0.0 };
                if d >= TABLIER_RACCORD {
                    continue;
                }
                let t = d / TABLIER_RACCORD;
                let poids = 1.0 - t * t * (3.0 - 2.0 * t);
                let p = a.points[i];
                a.points[i][2] = p[2] + poids * (plan(p[0], p[1]) - p[2]);
            }
            let (na, nb) = (a.a, a.b);
            let (ha, hb) = (a.points[0][2], a.points[a.points.len() - 1][2]);
            if ab[0] + TABLIER_RACCORD > s0 {
                r.noeuds[na].p[2] = ha;
            }
            if ab[ab.len() - 1] - TABLIER_RACCORD < s1 {
                r.noeuds[nb].p[2] = hb;
            }
        }
        n += 1;
    }
    n
}

/// Les tabliers au-dessus des souterrains : traversées réunies à moins de tant (m), la
/// marge de chaque côté du souterrain (m), la pente plafonnée, la longueur du raccord.
const TABLIER_LIEN: f32 = 12.0;
const TABLIER_MARGE: f32 = 2.0;
const TABLIER_PENTE: f32 = 0.08;
const TABLIER_RACCORD: f32 = 10.0;
/// Un souterrain plus profond que l'écart nécessaire plus tant (m) n'a pas de tablier.
const TABLIER_PROCHE: f32 = 2.0;

/// Ce que `preparer_souterrains` a fait.
#[derive(Clone, Copy, Debug, Default)]
pub struct Souterrains {
    /// Voies au sol de niveau négatif devenues souterrains.
    pub negatifs: usize,
    /// Souterrains très courts remis au sol.
    pub courts: usize,
    pub enfonces: usize,
    /// Voies au sol descendues en tranchée vers une entrée.
    pub tranchees: usize,
    /// Tabliers posés au-dessus des souterrains.
    pub tabliers: usize,
}

/// La transition au pied d'une rampe (m) : la pente passe de 0 à celle de la rampe
/// (une parabole) ; et l'arrondi (m) de la tête d'une tranchée et du pied d'une rampe
/// dans un souterrain.
const RAMPE_TRANSITION: f32 = 10.0;
/// Une suite de voies au sol entre deux entrées de souterrain plus courte que tant (m)
/// est une tranchée ouverte entre deux parties couvertes : elle reste au fond.
const TRANCHEE_OUVERTE: f32 = 150.0;
const RAMPE_ARRONDI: f32 = 1.2;

/// La distance « adoucie » d'une rampe qui part à plat : une parabole sur
/// `RAMPE_TRANSITION`, puis droite.
fn adoucie(d: f32) -> f32 {
    if d < RAMPE_TRANSITION { d * d / (2.0 * RAMPE_TRANSITION) } else { d - RAMPE_TRANSITION / 2.0 }
}

/// Minimum et maximum arrondis (de `k` mètres au plus).
/// (polynôme : exacts dès que les deux valeurs s'écartent de plus de `k`).
fn smin(a: f32, b: f32, k: f32) -> f32 {
    let h = (k - (a - b).abs()).max(0.0);
    a.min(b) - h * h / (4.0 * k)
}

fn smax(a: f32, b: f32, k: f32) -> f32 {
    let h = (k - (a - b).abs()).max(0.0);
    a.max(b) + h * h / (4.0 * k)
}

/// Un f32 rangeable (pour le tas).
fn ordre(x: f32) -> i64 {
    (x as f64 * 1000.0).round() as i64
}

/// Les points d'un tracé, un au moins tous les `RELIEF_PAS` mètres (hauteurs
/// interpolées).
fn densifier_xz(p: &[[f32; 3]]) -> Vec<[f32; 3]> {
    let mut v: Vec<[f32; 3]> = Vec::with_capacity(p.len() * 2);
    v.push(p[0]);
    for w in p.windows(2) {
        let (a, b) = (w[0], w[1]);
        let d = (b[0] - a[0]).hypot(b[1] - a[1]);
        let n = (d / RELIEF_PAS).ceil().max(1.0) as usize;
        for i in 1..=n {
            let t = i as f32 / n as f32;
            v.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]);
        }
    }
    v
}

/// La contrainte du croisement de deux voies (o1, o2) : qui passe dessus (le pont,
/// puis le sol, puis le tunnel ; à passage égal le niveau d'OSM, puis le plus haut
/// aujourd'hui) et l'écart (la hauteur limitée du dessous plus la dalle, sinon le
/// gabarit).
fn contrainte_croisement(r: &Reseau, gabarits: &Gabarits, o1: Ou, o2: Ou) -> Contrainte {
    use Passage::*;
    let (a1, a2) = (&r.aretes[o1.0], &r.aretes[o2.0]);
    let (h1, h2) = (r.hauteur(o1), r.hauteur(o2));
    let rang = |p: Passage| match p {
        Pont => 2,
        Sol => 1,
        Tunnel => 0,
    };
    let premier_dessus = match rang(a1.passage).cmp(&rang(a2.passage)) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Less => false,
        std::cmp::Ordering::Equal if a1.niveau != a2.niveau => a1.niveau > a2.niveau,
        std::cmp::Ordering::Equal => h1 >= h2,
    };
    let ((haut, ph), (bas, pb)) = if premier_dessus { ((o1, a1.passage), (o2, a2.passage)) } else { ((o2, a2.passage), (o1, a1.passage)) };
    let genre = if pb == Tunnel && ph != Pont { Genre::TunnelSousVoie } else { Genre::PontSurVoie };
    let ab = &r.aretes[bas.0];
    let gabarit = gabarits.hauteur(ab.points[bas.1], ab.points[bas.1 + 1], bas.2);
    // Une voie ferrée au-dessus passe sur un tablier, même posée au sol (un pont-rail)
    let rail_dessus = r.aretes[haut.0].genre == RAIL;
    let ecart = match gabarit {
        Some(g) => g + if ph == Tunnel { DALLE_ENTRE_SOUTERRAINS } else if ph == Pont || rail_dessus { EPAISSEUR } else { DALLE_SOUS_SOL },
        None if ab.genre == RAIL => GABARIT_RAIL + EPAISSEUR,
        None if ab.genre == sage_reel::PIETONNE => GABARIT_PIETON + EPAISSEUR_PIETON,
        None => GABARIT + EPAISSEUR,
    };
    Contrainte { genre, haut, bas: Some(bas), plancher: 0.0, ecart, implicite: ph == Sol && genre == Genre::PontSurVoie, gabarit }
}

/// La part du haut dans la correction d'une contrainte (1 : seul le haut bouge, 0 :
/// seul le bas).
pub fn part_du_haut(r: &Reseau, c: &Contrainte) -> f32 {
    let ph = r.aretes[c.haut.0].passage;
    let pb = c.bas.map(|b| r.aretes[b.0].passage);
    // Entre une route et une piétonne, seule la piétonne bouge
    let pieton = |k: usize| !sage_reel::carrossable(r.aretes[k].genre);
    // Sauf un souterrain piéton au-dessus d'un tunnel routier (niveaux d'OSM) : le
    // tunnel routier, plus profond, s'enfonce ; le souterrain reste tenu sous la surface
    let sous_terre = |k: usize| r.aretes[k].passage == Passage::Tunnel;
    // On se fie au relief de surface : sur un souterrain, seul le souterrain bouge
    if pb == Some(Passage::Tunnel) && ph != Passage::Tunnel {
        return 0.0;
    }
    match c.bas.map(|b| (pieton(c.haut.0), pieton(b.0))) {
        Some((true, false)) if sous_terre(c.haut.0) && c.bas.is_some_and(|b| sous_terre(b.0) && r.aretes[b.0].niveau < r.aretes[c.haut.0].niveau) => 0.0,
        Some((true, false)) => 1.0,
        Some((false, true)) => 0.0,
        _ => match (ph, pb) {
            (_, None) => 1.0,
            (Passage::Pont, Some(Passage::Sol)) => 0.85,
            (Passage::Sol, Some(Passage::Tunnel)) => 0.15,
            _ => 0.5,
        },
    }
}

/// La hauteur d'une arête au plus près de (x, z) (projection sur son tracé), et la
/// distance.
fn hauteur_pres(a: &Arete, x: f32, z: f32) -> (f32, f32) {
    let mut m = (f32::MAX, 0.0f32);
    for w in a.points.windows(2) {
        let (dx, dz) = (w[1][0] - w[0][0], w[1][1] - w[0][1]);
        let l2 = dx * dx + dz * dz;
        let t = if l2 > 1e-9 { (((x - w[0][0]) * dx + (z - w[0][1]) * dz) / l2).clamp(0.0, 1.0) } else { 0.0 };
        let d = (x - w[0][0] - dx * t).hypot(z - w[0][1] - dz * t);
        if d < m.0 {
            m = (d, w[0][2] + (w[1][2] - w[0][2]) * t);
        }
    }
    (m.1, m.0)
}

/// La fiche d'une voie (commande « traces voie k ») : son passage, son profil (hauteur
/// d'origine, nivelée, relief), ses croisements (l'autre voie, l'écart demandé, la
/// hauteur limitée, qui bouge, l'écart obtenu).
pub fn decrire_voie(fin: &Reseau, l: &Lieu, k: usize) -> String {
    let Some(a1) = fin.aretes.get(k) else { return format!("pas de voie a{k}") };
    let a0 = a1;
    // Ses croisements, cherchés sur le réseau nivelé seul (sans le refaire) : les
    // arêtes dont la boîte touche la sienne
    let boite = |a: &Arete| a.points.iter().fold([f32::MAX, f32::MAX, f32::MIN, f32::MIN], |m, p| [m[0].min(p[0]), m[1].min(p[1]), m[2].max(p[0]), m[3].max(p[1])]);
    let bk = boite(a1);
    let gabarits = Gabarits::new(l);
    let mut cs: Vec<Contrainte> = Vec::new();
    for (j, b) in fin.aretes.iter().enumerate() {
        if j == k {
            continue;
        }
        let bj = boite(b);
        if bj[0] > bk[2] || bj[2] < bk[0] || bj[1] > bk[3] || bj[3] < bk[1] {
            continue;
        }
        for s1 in 0..a1.points.len() - 1 {
            for s2 in 0..b.points.len() - 1 {
                if let Some((u1, u2)) = croise(a1.points[s1], a1.points[s1 + 1], b.points[s2], b.points[s2 + 1]) {
                    cs.push(contrainte_croisement(fin, &gabarits, (k, s1, u1), (j, s2, u2)));
                }
            }
        }
    }
    let brut = fin;
    let classes = Classes::new(l);
    let nom = |a: &Arete| {
        let c = nom_classe(classes.classe(a));
        format!("{} genre {} niveau {} largeur {:.1}{}", a.passage.nom(), a.genre, a.niveau, a.largeur, if c.is_empty() { String::new() } else { format!(" · {c}") })
    };
    let mut o = format!("a{k} · {} · nœuds {} → {}", nom(a0), a0.a, a0.b);
    let couv = a1.points.iter().map(|p| l.relief(p[0] as f64, p[1] as f64) as f32 - p[2]).fold(f32::MIN, f32::max);
    o += &format!(" · couverture max {couv:.2} m{}{}\nprofil (abscisse : nivelée · relief)\n", if fin.fixes.contains(&(k as u32)) { " · fixe" } else { "" }, if fin.tabliers.contains(&(k as u32)) { " · sur un tablier" } else { "" });
    let mut s = 0.0f32;
    for (i, p) in a0.points.iter().enumerate() {
        if i > 0 {
            s += (p[0] - a0.points[i - 1][0]).hypot(p[1] - a0.points[i - 1][1]);
        }
        o += &format!("  {s:5.1} m ({:.1};{:.1}) : {:.2} · relief {:.2}\n", p[0], p[1], p[2], l.relief(p[0] as f64, p[1] as f64));
    }
    let mut n = 0;
    for c in &cs {
        let dessus = c.haut.0 == k;
        if !dessus && c.bas.is_none_or(|b| b.0 != k) {
            continue;
        }
        n += 1;
        let p = brut.point(c.haut);
        // Une voie rapide posée est fixe : l'autre prend toute la correction
        let part = if fin.fixes.contains(&(c.haut.0 as u32)) {
            0.0
        } else if c.bas.is_some_and(|b| fin.fixes.contains(&(b.0 as u32))) {
            1.0
        } else {
            part_du_haut(brut, c)
        };
        let bouge = if part >= 0.99 { "le dessus bouge" } else if part <= 0.01 { "le dessous bouge" } else { "les deux bougent" };
        let hh = hauteur_pres(&fin.aretes[c.haut.0], p[0], p[1]).0;
        let (autre, hb) = match c.bas {
            Some(b) => (format!("a{} {}", if dessus { b.0 } else { c.haut.0 }, nom(&fin.aretes[if dessus { b.0 } else { c.haut.0 }])), hauteur_pres(&fin.aretes[b.0], p[0], p[1]).0),
            None => ("l'eau".into(), c.plancher),
        };
        let obtenu = hh - hb;
        o += &format!(
            "{} {} · ({:.1};{:.1}) · écart demandé {:.2}{} · {} (part du dessus {:.2}) · dessus {:.2}, dessous {:.2} : écart obtenu {:.2}{}\n",
            if dessus { "sous elle :" } else { "au-dessus :" },
            autre,
            p[0],
            p[1],
            c.ecart,
            c.gabarit.map_or(" (hauteur limitée inconnue)".into(), |g| format!(" (hauteur limitée {g:.2})")),
            bouge,
            part,
            hh,
            hb,
            obtenu,
            if obtenu + 0.05 < c.ecart { format!(" · manque {:.2}", c.ecart - obtenu) } else { String::new() }
        );
    }
    if n == 0 {
        o += "aucun croisement\n";
    }
    o
}

impl Reseau {
    pub fn hauteur(&self, (a, s, u): Ou) -> f32 {
        let p = &self.aretes[a].points;
        p[s][2] + (p[s + 1][2] - p[s][2]) * u
    }

    pub fn point(&self, (a, s, u): Ou) -> [f32; 3] {
        let p = &self.aretes[a].points;
        entre(p[s], p[s + 1], u)
    }

    /// Ce qu'il manque à une contrainte pour être tenue (0 si elle l'est).
    pub fn manque(&self, c: &Contrainte) -> f32 {
        let bas = c.bas.map_or(c.plancher, |b| self.hauteur(b));
        (bas + c.ecart - self.hauteur(c.haut)).max(0.0)
    }
}

/// Les contraintes de hauteur du réseau : chaque croisement dénivelé (sans nœud commun),
/// chaque croisement avec un cours d'eau à ciel ouvert, chaque point de pont au-dessus
/// de l'eau.
pub fn contraintes(r: &Reseau, l: &Lieu) -> Vec<Contrainte> {
    let mut v = Vec::new();
    let gabarits = Gabarits::new(l);
    // Les segments des arêtes, par case
    let mut cases: HashMap<(i32, i32), Vec<(usize, usize)>> = HashMap::new();
    for (k, a) in r.aretes.iter().enumerate() {
        for i in 0..a.points.len() - 1 {
            let (p, q) = (a.points[i], a.points[i + 1]);
            let (c0, c1) = (case(p[0].min(q[0]), p[1].min(q[1])), case(p[0].max(q[0]), p[1].max(q[1])));
            for ci in c0.0..=c1.0 {
                for cj in c0.1..=c1.1 {
                    cases.entry((ci, cj)).or_default().push((k, i));
                }
            }
        }
    }
    // 1. Les croisements entre voies
    let mut vus = std::collections::HashSet::new();
    for liste in cases.values() {
        for (x, &(k1, s1)) in liste.iter().enumerate() {
            for &(k2, s2) in &liste[x + 1..] {
                if k1 == k2 {
                    continue;
                }
                let (a1, a2) = (&r.aretes[k1], &r.aretes[k2]);
                // Une voie ferrée et une voie au sol au même niveau : un passage à niveau
                if (a1.genre == RAIL || a2.genre == RAIL) && a1.passage == Passage::Sol && a2.passage == Passage::Sol && a1.niveau == a2.niveau {
                    continue;
                }
                // Deux arêtes qui partagent un nœud et s'y touchent ne se croisent pas
                let Some((u1, u2)) = croise(a1.points[s1], a1.points[s1 + 1], a2.points[s2], a2.points[s2 + 1]) else { continue };
                if !vus.insert(((k1, s1).min((k2, s2)), (k1, s1).max((k2, s2)))) {
                    continue;
                }
                v.push(contrainte_croisement(r, &gabarits, (k1, s1, u1), (k2, s2, u2)));
            }
        }
    }
    // 2. Les cours d'eau à ciel ouvert : la voie passe au-dessus de l'eau, tirant d'air
    //    et tablier ; l'eau est au niveau du lieu dans la surface d'eau, ailleurs au plus
    //    bas du relief à 10 m autour
    let niveau_eau = |x: f32, z: f32| -> f32 {
        if l.eau(x as f64, z as f64) > 0.5 {
            return l.niveau;
        }
        let mut m = f32::MAX;
        for dx in [-10.0f32, 0.0, 10.0] {
            for dz in [-10.0f32, 0.0, 10.0] {
                m = m.min(l.relief((x + dx) as f64, (z + dz) as f64) as f32);
            }
        }
        m
    };
    for ce in l.cours_eau.iter().filter(|c| !c.souterrain) {
        for w in ce.points.windows(2) {
            let (c3, d3) = ([w[0][0], w[0][1], 0.0], [w[1][0], w[1][1], 0.0]);
            let (c0, c1) = (case(c3[0].min(d3[0]), c3[1].min(d3[1])), case(c3[0].max(d3[0]), c3[1].max(d3[1])));
            let mut vu = std::collections::HashSet::new();
            for ci in c0.0..=c1.0 {
                for cj in c0.1..=c1.1 {
                    for &(k, s) in cases.get(&(ci, cj)).into_iter().flatten() {
                        let a = &r.aretes[k];
                        if a.passage == Passage::Tunnel || !vu.insert((k, s)) {
                            continue;
                        }
                        let Some((u, _)) = croise(a.points[s], a.points[s + 1], c3, d3) else { continue };
                        let p = entre(a.points[s], a.points[s + 1], u);
                        v.push(Contrainte { genre: Genre::PontSurEau, haut: (k, s, u), bas: None, plancher: niveau_eau(p[0], p[1]), ecart: TIRANT_D_AIR + EPAISSEUR, implicite: a.passage == Passage::Sol, gabarit: None });
                    }
                }
            }
        }
    }
    // 3. Les points des ponts au-dessus de la surface d'eau (la Seine, sur toute la
    //    traversée)
    for (k, a) in r.aretes.iter().enumerate().filter(|(_, a)| a.passage == Passage::Pont) {
        for i in 0..a.points.len() - 1 {
            let p = a.points[i];
            if l.eau(p[0] as f64, p[1] as f64) > 0.5 {
                v.push(Contrainte { genre: Genre::PontSurEau, haut: (k, i, 0.0), bas: None, plancher: l.niveau, ecart: TIRANT_D_AIR + EPAISSEUR, implicite: false, gabarit: None });
            }
        }
    }
    v
}

/// Le résumé des contraintes : combien de chaque genre, combien ne sont pas tenues par
/// les hauteurs d'aujourd'hui, et de combien.
pub fn resume_contraintes(r: &Reseau, v: &[Contrainte]) -> String {
    let mut s = vec![format!("{} contraintes de hauteur", v.len())];
    for (g, nom) in [(Genre::PontSurVoie, "pont sur une voie"), (Genre::TunnelSousVoie, "tunnel sous une voie"), (Genre::PontSurEau, "pont sur l'eau")] {
        let x: Vec<&Contrainte> = v.iter().filter(|c| c.genre == g).collect();
        let mut m: Vec<f32> = x.iter().map(|c| r.manque(c)).filter(|&m| m > 0.01).collect();
        m.sort_by(f32::total_cmp);
        let imp = x.iter().filter(|c| c.implicite).count();
        let manque = if m.is_empty() { "toutes tenues".to_string() } else { format!("{} non tenues (manque médian {:.1} m, max {:.1} m)", m.len(), m[m.len() / 2], m[m.len() - 1]) };
        s.push(format!("  {nom} : {} ; {manque}{}", x.len(), if imp > 0 { format!(" ; {imp} ponts implicites (voie au sol dessus)") } else { String::new() }));
    }
    s.join("\n")
}


// --- Étape 3 : le profil lissé ------------------------------------------------------

/// La pente des rampes d'un ouvrage (la surélévation d'un pont, l'abaissement d'un
/// tunnel décroît d'autant par mètre, en s'ajoutant au relief) : voie carrossable, et
/// chemins et sentiers.
pub const PENTE_ROUTE: f32 = 0.08;
pub const PENTE_CHEMIN: f32 = 0.15;
/// Au sol, aucune pente maximale : la voie suit le relief (Rouen a des rues à 15–19 %).
/// Au-delà de celle-ci, plus raide que les rues les plus raides, la pente est une
/// anomalie du relief ou des données, et elle est rabotée.
pub const PENTE_ANOMALIE: f32 = 0.25;
/// Une voie au sol à moins de tant de mètres d'un bout de pont est tenue par lui : sous
/// un autre pont, elle ne descend pas, et la rampe du pont la relève.
const TENUE: f32 = 30.0;
/// Le poids qui retient un point au sol à sa hauteur d'origine (le relief lissé), et
/// celui, faible, d'un point de pont ou de tunnel (ses rampes suivent le reste).
const RETENUE_SOL: f32 = 0.3;
const RETENUE_OUVRAGE: f32 = 0.02;
/// Une contrainte ne monte (ou ne descend) pas une voie de plus de tant (m) : au-delà,
/// les données se contredisent, elle reste non tenue.
const MONTEE_MAX: f32 = 12.0;

/// L'avancement du nivellement en cours (pour mille), pour la barre d'état.
pub static AVANCEMENT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

fn avancer(x: f32) {
    AVANCEMENT.store((x.clamp(0.0, 1.0) * 1000.0) as u32, std::sync::atomic::Ordering::Relaxed);
}

fn pente_max(genre: u8) -> f32 {
    if sage_reel::carrossable(genre) {
        PENTE_ROUTE
    } else {
        PENTE_CHEMIN
    }
}

/// Le bilan du nivellement.
pub struct Bilan {
    pub iterations: usize,
    /// Les contraintes écartées : le haut et le bas se rejoignent par le réseau en moins
    /// de `ecart / PENTE_ANOMALIE` mètres (il faudrait une pente impossible).
    pub impossibles: usize,
    pub duree: f32,
}

/// Nivelle le réseau : une hauteur par nœud (les arêtes qui s'y rejoignent n'ont plus
/// de saut), un profil lissé qui suit le relief au sol, tenant les contraintes de
/// hauteur et, autant que le relief le permet, la pente maximale. Relaxation : à chaque
/// tour, chaque point se rapproche de la moyenne de ses voisins et de sa hauteur
/// d'origine, puis les contraintes et les pentes sont imposées (projections).
pub fn niveler(r: &mut Reseau, v: &[Contrainte], iterations: usize) -> Bilan {
    niveler_fixe(r, v, iterations, &[])
}

/// `niveler`, les arêtes `fixes` (les voies rapides posées) ne bougeant pas : les
/// autres s'adaptent à elles.
pub fn niveler_fixe(r: &mut Reseau, v: &[Contrainte], iterations: usize, fixes: &[bool]) -> Bilan {
    let t0 = std::time::Instant::now();
    avancer(0.0);
    let nn = r.noeuds.len();
    // Les variables : les nœuds, puis les points intérieurs des arêtes
    let mut ids: Vec<Vec<usize>> = Vec::with_capacity(r.aretes.len());
    let mut y: Vec<f32> = vec![0.0; nn];
    let mut cible: Vec<f32> = vec![0.0; nn];
    let mut retenue: Vec<f32> = vec![0.0; nn];
    // Les nœuds : la moyenne des bouts au sol qui y arrivent (sinon de tous)
    // Les routes passent d'abord : un nœud touché par une route ne prend la hauteur
    // que des routes, et les voies non carrossables (piétonnes, chemins, sentiers) ne
    // le tirent pas
    let mut route = vec![false; nn];
    for a in r.aretes.iter().filter(|a| sage_reel::carrossable(a.genre)) {
        route[a.a] = true;
        route[a.b] = true;
    }
    // La hiérarchie (comme les bords, la surface et les plaques) : 0 les voies non
    // carrossables, 1 les voies de service (et chemins carrossables), 2 les routes
    // (grandes routes, routes, bus). Un point ne regarde que ses voisins de son niveau
    // ou au-dessus ; une voie inférieure ne tire pas une route
    let niveau_voie = |a: &Arete| -> u8 {
        if !sage_reel::carrossable(a.genre) {
            0
        } else if crate::materialisation::rang(a.genre) >= 2 {
            1
        } else {
            2
        }
    };
    let mut niv: Vec<u8> = vec![0; nn];
    for a in &r.aretes {
        let x = niveau_voie(a);
        niv[a.a] = niv[a.a].max(x);
        niv[a.b] = niv[a.b].max(x);
    }
    let mut somme = vec![(0.0f32, 0usize, 0.0f32, 0usize); nn];
    for a in &r.aretes {
        for (n, p) in [(a.a, a.points[0]), (a.b, *a.points.last().unwrap())] {
            if niveau_voie(a) < niv[n] {
                continue;
            }
            let s = &mut somme[n];
            if a.passage == Passage::Sol {
                s.0 += p[2];
                s.1 += 1;
            }
            s.2 += p[2];
            s.3 += 1;
        }
    }
    for i in 0..nn {
        let s = somme[i];
        let (c, w) = if s.1 > 0 { (s.0 / s.1 as f32, RETENUE_SOL) } else { (s.2 / s.3.max(1) as f32, RETENUE_OUVRAGE) };
        cible[i] = c;
        y[i] = c;
        retenue[i] = w;
    }
    for a in &r.aretes {
        let n = a.points.len();
        let mut v = Vec::with_capacity(n);
        v.push(a.a);
        for p in &a.points[1..n - 1] {
            y.push(p[2]);
            cible.push(p[2]);
            retenue.push(if a.passage == Passage::Sol { RETENUE_SOL } else { RETENUE_OUVRAGE });
            route.push(sage_reel::carrossable(a.genre));
            niv.push(niveau_voie(a));
            v.push(y.len() - 1);
        }
        v.push(a.b);
        ids.push(v);
    }
    let nv = y.len();
    // Les points fixes : ceux des arêtes fixes (et leurs nœuds)
    let mut fixe = vec![false; nv];
    for (k, f) in fixes.iter().enumerate() {
        if *f {
            for &v in &ids[k] {
                fixe[v] = true;
            }
        }
    }
    // Les liens entre points voisins : (autre, longueur, pente max)
    let mut liens: Vec<(usize, usize, f32, f32)> = Vec::new();
    let mut voisins: Vec<Vec<usize>> = vec![Vec::new(); nv];
    for (k, a) in r.aretes.iter().enumerate() {
        let g = pente_max(a.genre);
        for i in 0..a.points.len() - 1 {
            let (p, q) = (a.points[i], a.points[i + 1]);
            let d = (q[0] - p[0]).hypot(q[1] - p[1]).max(0.01);
            let (x, z) = (ids[k][i], ids[k][i + 1]);
            if x == z {
                continue;
            }
            liens.push((x, z, d, g));
            // Un point ne regarde que ses voisins de son niveau ou au-dessus
            if niv[z] >= niv[x] {
                voisins[x].push(z);
            }
            if niv[x] >= niv[z] {
                voisins[z].push(x);
            }
        }
    }
    // Les voies tenues : les points au sol d'une voie carrossable à moins de TENUE
    // mètres (par le réseau au sol) d'un bout de pont. Sous un croisement, elles ne
    // descendent pas (seul le dessus monte), et la rampe du pont peut les relever
    let mut tenu = vec![false; nv];
    {
        let mut sol_var = vec![false; nv];
        let mut pont_noeud = vec![false; nn];
        let mut sol_noeud = vec![false; nn];
        for (k, a) in r.aretes.iter().enumerate() {
            if !sage_reel::carrossable(a.genre) {
                continue;
            }
            match a.passage {
                Passage::Sol => {
                    for &v in &ids[k] {
                        sol_var[v] = true;
                    }
                    sol_noeud[a.a] = true;
                    sol_noeud[a.b] = true;
                }
                Passage::Pont => {
                    pont_noeud[a.a] = true;
                    pont_noeud[a.b] = true;
                }
                _ => {}
            }
        }
        let mut long_sol: Vec<Vec<(usize, f32)>> = vec![Vec::new(); nv];
        for (k, a) in r.aretes.iter().enumerate() {
            if a.passage != Passage::Sol || !sage_reel::carrossable(a.genre) {
                continue;
            }
            for i in 0..a.points.len() - 1 {
                let (p, q) = (a.points[i], a.points[i + 1]);
                let d = (q[0] - p[0]).hypot(q[1] - p[1]);
                let (x, z) = (ids[k][i], ids[k][i + 1]);
                long_sol[x].push((z, d));
                long_sol[z].push((x, d));
            }
        }
        let mut dist = vec![f32::MAX; nv];
        let mut file: Vec<usize> = Vec::new();
        for n in 0..nn {
            if pont_noeud[n] && sol_noeud[n] {
                dist[n] = 0.0;
                file.push(n);
            }
        }
        while let Some(i) = file.pop() {
            for &(j, d) in &long_sol[i] {
                let dj = dist[i] + d;
                if dj < dist[j] && dj <= TENUE {
                    dist[j] = dj;
                    file.push(j);
                }
            }
        }
        for i in 0..nv {
            tenu[i] = sol_var[i] && dist[i] <= TENUE;
        }
    }
    // Les contraintes en variables : (haut : v0, v1, u ; bas ; plancher ; écart ; part
    // du haut dans la correction)
    struct C {
        h: (usize, usize, f32),
        b: Option<(usize, usize, f32)>,
        plancher: f32,
        ecart: f32,
        part: f32,
        // Le haut, le bas sur une piétonne : ses bouts qui sont des routes ne bougent pas
        pieton_h: bool,
        pieton_b: bool,
    }
    let var = |(a, s, u): Ou| (ids[a][s], ids[a][s + 1], u);
    let cs: Vec<C> = v
        .iter()
        .map(|c| {
            let pieton = |k: usize| !sage_reel::carrossable(r.aretes[k].genre);
            let mut part = part_du_haut(r, c);
            // Un pont au-dessus d'une voie tenue : seul le pont monte
            if r.aretes[c.haut.0].passage == Passage::Pont && c.bas.is_some_and(|b| {
                let (v0, v1, _) = var(b);
                tenu[v0] || tenu[v1]
            }) {
                part = 1.0;
            }
            // Une voie fixe ne bouge pas : l'autre prend toute la correction
            let (vh, vb) = (var(c.haut), c.bas.map(var));
            if fixe[vh.0] || fixe[vh.1] {
                part = 0.0;
            } else if vb.is_some_and(|b| fixe[b.0] || fixe[b.1]) {
                part = 1.0;
            }
            C { h: var(c.haut), b: c.bas.map(var), plancher: c.plancher, ecart: c.ecart, part, pieton_h: pieton(c.haut.0), pieton_b: c.bas.is_some_and(|b| pieton(b.0)) }
        })
        .collect();
    let lire = |y: &[f32], (a, b, u): (usize, usize, f32)| y[a] * (1.0 - u) + y[b] * u;
    let pousser = |y: &mut [f32], (a, b, u): (usize, usize, f32), dh: f32| {
        let n = ((1.0 - u) * (1.0 - u) + u * u).max(1e-6);
        y[a] += dh * (1.0 - u) / n;
        if b != a {
            y[b] += dh * u / n;
        }
    };
    let imposer = |y: &mut [f32]| {
        for c in &cs {
            let bas = c.b.map_or(c.plancher, |b| lire(y, b));
            let m = bas + c.ecart - lire(y, c.h);
            if m > 0.0 {
                pousser(y, c.h, m * c.part);
                if let Some(b) = c.b {
                    pousser(y, b, -m * (1.0 - c.part));
                }
            }
        }
    };
    // 1. Le profil lissé, sans les contraintes : chaque point vers la moyenne de ses
    //    voisins et sa hauteur d'origine (Jacobi)
    let mut suivant = y.clone();
    for it in 0..iterations {
        avancer(0.1 + 0.6 * it as f32 / iterations.max(1) as f32);
        for i in 0..nv {
            let vs = &voisins[i];
            if vs.is_empty() || fixe[i] {
                suivant[i] = y[i];
                continue;
            }
            let m: f32 = vs.iter().map(|&j| y[j]).sum::<f32>() / vs.len() as f32;
            suivant[i] = (m + retenue[i] * cible[i]) / (1.0 + retenue[i]);
        }
        std::mem::swap(&mut y, &mut suivant);
    }
    // Les anomalies : les pentes au-delà de PENTE_ANOMALIE rabotées (projections, les
    // deux bouts se rapprochent)
    for _ in 0..40 {
        let mut reste = false;
        for &(a, b, d, _) in &liens {
            let e = (y[a] - y[b]).abs() - PENTE_ANOMALIE * d;
            if e > 1e-3 {
                reste = true;
                // Entre une route et une piétonne, seule la piétonne bouge
                let (ka, kb) = match (fixe[a], fixe[b], route[a], route[b]) {
                    (true, true, _, _) => continue,
                    (true, false, _, _) => (0.0, 1.0),
                    (false, true, _, _) => (1.0, 0.0),
                    // La voie inférieure bouge seule
                    _ if niv[a] > niv[b] => (0.0, 1.0),
                    _ if niv[a] < niv[b] => (1.0, 0.0),
                    _ => (0.5, 0.5),
                };
                let s = if y[a] > y[b] { e } else { -e };
                y[a] -= s * ka;
                y[b] += s * kb;
            }
        }
        if !reste {
            break;
        }
    }
    // Les liens de chaque point, pour les rampes
    let mut adj: Vec<Vec<(usize, f32)>> = vec![Vec::new(); nv];
    let mut long: Vec<Vec<(usize, f32)>> = vec![Vec::new(); nv];
    for &(a, b, d, g) in &liens {
        adj[a].push((b, g * d));
        adj[b].push((a, g * d));
        long[a].push((b, d));
        long[b].push((a, d));
    }
    // Les contraintes impossibles : le bas à moins de `ecart / PENTE_ANOMALIE` mètres du
    // haut par le réseau (un croisement mal tracé : en vrai un carrefour, ou une bretelle)
    let mut possible = vec![true; cs.len()];
    for (k, c) in cs.iter().enumerate() {
        let Some(b) = c.b else { continue };
        let rayon = c.ecart / PENTE_ANOMALIE;
        let mut dist: HashMap<usize, f32> = HashMap::new();
        let mut file = vec![(c.h.0, 0.0f32), (c.h.1, 0.0)];
        while let Some((i, di)) = file.pop() {
            if dist.get(&i).is_some_and(|&x| x <= di) {
                continue;
            }
            dist.insert(i, di);
            if i == b.0 || i == b.1 {
                possible[k] = false;
                break;
            }
            for &(j, d) in &long[i] {
                if di + d < rayon {
                    file.push((j, di + d));
                }
            }
        }
    }
    avancer(0.8);
    let impossibles = possible.iter().filter(|x| !**x).count();
    let cs: Vec<&C> = cs.iter().zip(&possible).filter(|(_, p)| **p).map(|(c, _)| c).collect();
    // Les surélévations (et les abaissements) au-dessus du profil lissé `base` : une
    // contrainte monte ses points, la surélévation décroît de `g·d` d'un point à son
    // voisin (la rampe s'ajoute au terrain, quelle que soit sa pente)
    let base = y.clone();
    let mut monte = vec![0.0f32; nv];
    let mut descend = vec![0.0f32; nv];
    // Les bas des contraintes ne montent pas avec les rampes, leurs hauts ne descendent
    // pas avec les descentes
    let mut fixe_bas = vec![false; nv];
    let mut fixe_haut = vec![false; nv];
    for c in cs.iter() {
        fixe_haut[c.h.0] = true;
        fixe_haut[c.h.1] = true;
        // Une voie tenue reste libre de monter avec la rampe de son pont
        if let Some(b) = c.b {
            fixe_bas[b.0] = !tenu[b.0];
            fixe_bas[b.1] = !tenu[b.1];
        }
    }
    for i in 0..nv {
        if fixe[i] {
            fixe_bas[i] = true;
            fixe_haut[i] = true;
        }
    }
    let propager = |d: &mut [f32], sources: &[usize], fixes: &[bool]| {
        use std::cmp::Ordering;
        #[derive(PartialEq)]
        struct E(f32, usize);
        impl Eq for E {}
        impl PartialOrd for E {
            fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
                Some(self.cmp(o))
            }
        }
        impl Ord for E {
            fn cmp(&self, o: &Self) -> Ordering {
                self.0.total_cmp(&o.0)
            }
        }
        let mut tas: std::collections::BinaryHeap<E> = sources.iter().map(|&i| E(d[i], i)).collect();
        while let Some(E(h, i)) = tas.pop() {
            if h < d[i] - 1e-6 {
                continue;
            }
            for &(j, dh) in &adj[i] {
                // Une rampe ne passe jamais d'une voie inférieure à une voie supérieure
                if niv[j] > niv[i] {
                    continue;
                }
                let voulu = h - dh;
                if voulu > d[j] + 1e-4 && !fixes[j] {
                    d[j] = voulu;
                    tas.push(E(voulu, j));
                }
            }
        }
    };
    // 2. Les contraintes : le haut monte et le bas descend (selon leur part), puis les
    //    rampes. Ce que demande chaque contrainte est mesuré sur le profil lissé, sans
    //    cumul ; quelques tours reprennent celles qu'une rampe voisine a défaites
    let mut tours = 0;
    for _ in 0..5 {
        avancer(0.85 + 0.03 * tours as f32);
        tours += 1;
        for i in 0..nv {
            y[i] = base[i] + monte[i] - descend[i];
        }
        let (mut hauts, mut bas_) = (Vec::new(), Vec::new());
        // Ce que chaque point doit monter (descendre) à ce tour : la plus forte des
        // demandes, pas leur somme (trois voies sur un même tunnel ne l'enfoncent pas
        // trois fois)
        let (mut a_monter, mut a_descendre) = (vec![0.0f32; nv], vec![0.0f32; nv]);
        for c in cs.iter() {
            let bas = c.b.map_or(c.plancher, |b| lire(&y, b));
            let m = bas + c.ecart - lire(&y, c.h);
            if m <= 0.02 {
                continue;
            }
            let dh = (m * c.part + 0.01).min(MONTEE_MAX);
            for v in [c.h.0, c.h.1].into_iter().filter(|&v| c.part > 0.0 && !fixe[v] && !(c.pieton_h && route[v])) {
                a_monter[v] = a_monter[v].max(dh);
                hauts.push(v);
            }
            if let Some(b) = c.b.filter(|_| c.part < 1.0) {
                let db = (m * (1.0 - c.part) + 0.01).min(MONTEE_MAX);
                for v in [b.0, b.1].into_iter().filter(|&v| !fixe[v] && !(c.pieton_b && route[v])) {
                    a_descendre[v] = a_descendre[v].max(db);
                    bas_.push(v);
                }
            }
        }
        if hauts.is_empty() && bas_.is_empty() {
            break;
        }
        for i in 0..nv {
            monte[i] = (monte[i] + a_monter[i]).min(MONTEE_MAX);
            descend[i] = (descend[i] + a_descendre[i]).min(MONTEE_MAX);
        }
        hauts.sort_unstable();
        hauts.dedup();
        bas_.sort_unstable();
        bas_.dedup();
        propager(&mut monte, &hauts, &fixe_bas);
        propager(&mut descend, &bas_, &fixe_haut);
    }
    for i in 0..nv {
        y[i] = base[i] + monte[i] - descend[i];
    }
    let _ = (pousser, imposer, tours);
    // Les hauteurs rendues aux arêtes et aux nœuds
    for (k, a) in r.aretes.iter_mut().enumerate() {
        for (i, p) in a.points.iter_mut().enumerate() {
            p[2] = y[ids[k][i]];
        }
    }
    for (i, n) in r.noeuds.iter_mut().enumerate() {
        n.p[2] = y[i];
    }
    avancer(1.0);
    Bilan { iterations, impossibles, duree: t0.elapsed().as_secs_f32() }
}

/// Le bilan d'un profil : contraintes non tenues, pentes au-delà du maximum, écart au
/// relief des points au sol (remblai au-dessus, déblai au-dessous), sauts aux nœuds.
pub fn bilan_profil(r: &Reseau, v: &[Contrainte], l: &Lieu) -> String {
    let mut s = Vec::new();
    let mut m: Vec<f32> = v.iter().map(|c| r.manque(c)).filter(|&m| m > 0.05).collect();
    m.sort_by(f32::total_cmp);
    s.push(if m.is_empty() { format!("contraintes : les {} tenues", v.len()) } else { format!("contraintes : {} non tenues sur {} (manque max {:.2} m)", m.len(), v.len(), m[m.len() - 1]) });
    let (mut au_dela, mut total, mut pmax) = (0usize, 0usize, 0.0f32);
    for a in &r.aretes {
        let g = PENTE_ANOMALIE;
        for w in a.points.windows(2) {
            let d = (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]);
            if d < 2.0 {
                continue;
            }
            let p = (w[1][2] - w[0][2]).abs() / d;
            total += 1;
            pmax = pmax.max(p);
            if p > g + 0.01 {
                au_dela += 1;
            }
        }
    }
    s.push(format!("pentes : {au_dela} segments au-delà de 25 % sur {total} ({:.2} %), pente max {:.1} %", au_dela as f32 / total.max(1) as f32 * 100.0, pmax * 100.0));
    let mut ecarts: Vec<f32> = Vec::new();
    for a in r.aretes.iter().filter(|a| a.passage == Passage::Sol) {
        for p in &a.points {
            ecarts.push(p[2] - l.relief(p[0] as f64, p[1] as f64) as f32);
        }
    }
    ecarts.sort_by(f32::total_cmp);
    if !ecarts.is_empty() {
        let q = |f: f32| ecarts[((ecarts.len() - 1) as f32 * f) as usize];
        s.push(format!("au sol, écart au relief : 1er centile {:.2} m, médiane {:.2} m, 99e centile {:.2} m ; remblai > 1 m : {} points, déblai > 1 m : {} points (sur {})", q(0.01), q(0.5), q(0.99), ecarts.iter().filter(|&&e| e > 1.0).count(), ecarts.iter().filter(|&&e| e < -1.0).count(), ecarts.len()));
    }
    // Les sauts aux nœuds (entre le bout de chaque arête et son nœud)
    let mut saut = 0.0f32;
    for a in &r.aretes {
        saut = saut.max((a.points[0][2] - r.noeuds[a.a].p[2]).abs()).max((a.points.last().unwrap()[2] - r.noeuds[a.b].p[2]).abs());
    }
    s.push(format!("sauts aux nœuds : max {saut:.2} m"));
    s.join("\n")
}

/// Les pires pentes : (pente, x, z, passage, genre, longueur du segment, distance à la
/// contrainte la plus proche), et la répartition des pentes au-delà du maximum.
pub fn pires_pentes(r: &Reseau, v: &[Contrainte], n: usize) -> String {
    let mut seg: Vec<(f32, [f32; 3], Passage, u8, f32)> = Vec::new();
    let mut paliers = [0usize; 4];
    for a in &r.aretes {
        let g = 0.15;
        for w in a.points.windows(2) {
            let d = (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]);
            if d < 2.0 {
                continue;
            }
            let p = (w[1][2] - w[0][2]).abs() / d;
            if p > g + 0.01 {
                let k = if p < 0.15 { 0 } else if p < 0.3 { 1 } else if p < 1.0 { 2 } else { 3 };
                paliers[k] += 1;
                seg.push((p, entre(w[0], w[1], 0.5), a.passage, a.genre, d));
            }
        }
    }
    seg.sort_by(|a, b| b.0.total_cmp(&a.0));
    let pts: Vec<[f32; 3]> = v.iter().map(|c| r.point(c.haut)).collect();
    let mut s = vec![format!("pentes au-delà de 15 % : < 15 % : {}, 15–30 % : {}, 30–100 % : {}, ≥ 100 % : {}", paliers[0], paliers[1], paliers[2], paliers[3])];
    for (p, m, pa, g, d) in seg.iter().take(n) {
        let dc = pts.iter().map(|q| (q[0] - m[0]).hypot(q[1] - m[1])).fold(f32::MAX, f32::min);
        s.push(format!("  {:.0} % en ({:.0}, {:.0}) h {:.1} · {} genre {g} · segment {:.1} m · contrainte à {:.0} m", p * 100.0, m[0], m[1], m[2], pa.nom(), d, dc));
    }
    s.join("\n")
}

/// Le réseau d'un lieu, nivelé : construit, contraint, son profil lissé (50 tours).
pub fn nivele(l: &Lieu) -> Reseau {
    let mut r = reseau(l);
    preparer_souterrains(&mut r, l);
    let rapides = lisser_rapides(&mut r, l);
    r.fixes = (0..r.aretes.len() as u32).filter(|&k| rapides.fixes[k as usize]).collect();
    poser_tabliers(&mut r);
    adoucir_lacets(&mut r, &rapides.fixes);
    combler_creux(&mut r, &rapides.fixes);
    let c = contraintes(&r, l);
    let mut fixes = rapides.fixes;
    niveler_fixe(&mut r, &c, 50, &fixes);
    retirer_rails(&mut r, &mut fixes);
    r
}

/// Retire les voies ferrées du réseau nivelé (elles n'ont servi qu'à le contraindre) :
/// leurs arêtes et leurs nœuds sont les derniers (`reseau`) ; rend leur nombre.
pub fn retirer_rails(r: &mut Reseau, fixes: &mut Vec<bool>) -> usize {
    let Some(na) = r.aretes.iter().position(|a| a.genre == RAIL) else { return 0 };
    let n = r.aretes.len() - na;
    debug_assert!(r.aretes[na..].iter().all(|a| a.genre == RAIL));
    let nn = r.aretes[na..].iter().map(|a| a.a.min(a.b)).min().unwrap_or(r.noeuds.len());
    r.aretes.truncate(na);
    r.noeuds.truncate(nn);
    fixes.truncate(na);
    r.fixes.retain(|&k| (k as usize) < na);
    r.tabliers.retain(|&k| (k as usize) < na);
    n
}

/// Les dépressions étroites sous les routes : une route au sol dont le relief plonge
/// sur moins de `CREUX_LARGEUR` mètres, d'au moins `CREUX_PROFONDEUR`, et remonte de
/// l'autre côté (les deux bords à moins de `PENTE_ROUTE` l'un de l'autre) les franchit
/// en ligne droite : un pont qu'OSM ne connaît pas (une voie ferrée abandonnée, un
/// chemin creux, un ruisseau busé). Le long des chaînes de voies au sol carrossables
/// (à travers les nœuds de deux voies), hors voies fixes ; rend le nombre de creux.
pub const CREUX_LARGEUR: f32 = 40.0;
pub const CREUX_PROFONDEUR: f32 = 3.0;
/// Ce dont un point du creux peut dépasser la ligne des bords.
const CREUX_TOLERANCE: f32 = 0.2;

pub fn combler_creux(r: &mut Reseau, fixes: &[bool]) -> usize {
    profils_sol(r, fixes, false)
}

/// Les lacets : là où une route au sol tourne de plus de `LACET_ANGLE` degrés entre
/// `LACET_BRAS` mètres avant et après un point (un virage en épingle), son profil ne
/// suit plus le relief (le tracé y coupe le flanc) : il passe en pente régulière de
/// l'entrée à la sortie du virage. Rend le nombre de lacets.
pub const LACET_BRAS: f32 = 15.0;
pub const LACET_ANGLE: f32 = 120.0;

pub fn adoucir_lacets(r: &mut Reseau, fixes: &[bool]) -> usize {
    profils_sol(r, fixes, true)
}

/// Les chaînes de voies au sol carrossables (hors voies fixes) et leur profil : les
/// creux franchis (deux passes), ou les lacets adoucis.
fn profils_sol(r: &mut Reseau, fixes: &[bool], lacets: bool) -> usize {
    let na = r.aretes.len();
    let ok: Vec<bool> = (0..na)
        .map(|k| {
            let a = &r.aretes[k];
            a.passage == Passage::Sol && sage_reel::carrossable(a.genre) && a.genre != RAIL && a.a != a.b && a.points.len() >= 2 && !fixes.get(k).copied().unwrap_or(false)
        })
        .collect();
    let nn = r.noeuds.len();
    let mut bouts: Vec<Vec<usize>> = vec![Vec::new(); nn];
    for k in (0..na).filter(|&k| ok[k]) {
        bouts[r.aretes[k].a].push(k);
        bouts[r.aretes[k].b].push(k);
    }
    // La direction de la voie k en partant du nœud n (vers son point à 5 m)
    let dir = |r: &Reseau, k: usize, n: usize| -> [f32; 2] {
        let a = &r.aretes[k];
        let q: Vec<[f32; 3]> = if a.a == n { a.points.clone() } else { a.points.iter().rev().copied().collect() };
        let s = q.iter().find(|p| (p[0] - q[0][0]).hypot(p[1] - q[0][1]) >= 5.0).unwrap_or(&q[q.len() - 1]);
        let (dx, dz) = (s[0] - q[0][0], s[1] - q[0][1]);
        let l = dx.hypot(dz).max(1e-6);
        [dx / l, dz / l]
    };
    // Deux passes : la seconde voit les carrefours relevés par la première (une voie qui
    // part d'un carrefour au bord d'un creux y trouve son bord)
    let mut creux = 0;
    for _passe in 0..if lacets { 1 } else { 2 } {
        creux += combler_creux_passe(r, &ok, &bouts, &dir, lacets);
    }
    creux
}

/// Une passe de `combler_creux` : les chaînes traversent un nœud de deux voies, ou un
/// carrefour par la voie qui prolonge le mieux (à moins de 30°).
fn combler_creux_passe(r: &mut Reseau, ok: &[bool], bouts: &[Vec<usize>], dir: &dyn Fn(&Reseau, usize, usize) -> [f32; 2], lacets: bool) -> usize {
    let na = r.aretes.len();
    let mut vue = vec![false; na];
    let mut creux = 0;
    let droit = 30f32.to_radians().cos();
    for k0 in 0..na {
        if !ok[k0] || vue[k0] {
            continue;
        }
        vue[k0] = true;
        let suivre = |r: &Reseau, depart: usize, k: usize, vue: &mut Vec<bool>| -> Vec<(usize, bool)> {
            let mut v = Vec::new();
            let (mut n, mut k) = (depart, k);
            for _ in 0..10_000 {
                // La voie qui prolonge k au-delà de n : la seule autre d'un nœud de deux
                // voies, sinon la plus droite à moins de 30°
                let d = dir(r, k, n);
                let k2 = if bouts[n].len() == 2 && r.noeuds[n].aretes.len() == 2 {
                    Some(if bouts[n][0] == k { bouts[n][1] } else { bouts[n][0] })
                } else {
                    bouts[n].iter().copied().filter(|&c| c != k).map(|c| (c, { let e = dir(r, c, n); d[0] * e[0] + d[1] * e[1] })).filter(|x| x.1 <= -droit).min_by(|a, b| a.1.total_cmp(&b.1)).map(|x| x.0)
                };
                let Some(k2) = k2 else { break };
                if vue[k2] || k2 == k {
                    break;
                }
                vue[k2] = true;
                let a = &r.aretes[k2];
                let endroit = a.a == n;
                v.push((k2, endroit));
                n = if endroit { a.b } else { a.a };
                k = k2;
            }
            v
        };
        let arriere = suivre(r, r.aretes[k0].a, k0, &mut vue);
        let avant = suivre(r, r.aretes[k0].b, k0, &mut vue);
        let mut chaine: Vec<(usize, bool)> = arriere.into_iter().rev().map(|(k, e)| (k, !e)).collect();
        chaine.push((k0, true));
        chaine.extend(avant);
        // Les points de la chaîne : (abscisse, hauteur, où l'écrire : arêtes et points,
        // et le nœud d'un joint)
        let mut s_ = 0.0f32;
        let mut pts: Vec<(f32, f32, Vec<(usize, usize)>, Option<usize>)> = Vec::new();
        for &(k, endroit) in &chaine {
            let a = &r.aretes[k];
            let m = a.points.len();
            for x in 0..m {
                let i = if endroit { x } else { m - 1 - x };
                let p = a.points[i];
                if x == 0 {
                    let noeud = if endroit { a.a } else { a.b };
                    if let Some(der) = pts.last_mut() {
                        der.2.push((k, i));
                        der.3 = Some(noeud);
                        continue;
                    }
                    pts.push((0.0, p[2], vec![(k, i)], None));
                    continue;
                }
                let q = pts.last().map(|d| (d.0, d.2[0])).unwrap();
                let pq = r.aretes[q.1 .0].points[q.1 .1];
                s_ += (p[0] - pq[0]).hypot(p[1] - pq[1]);
                pts.push((s_, p[2], vec![(k, i)], None));
            }
        }
        let n = pts.len();
        let mut h: Vec<f32> = pts.iter().map(|p| p.1).collect();
        let mut change = false;
        let mut j = 0;
        if lacets {
            let xz: Vec<[f32; 2]> = pts.iter().map(|p| { let q = r.aretes[p.2[0].0].points[p.2[0].1]; [q[0], q[1]] }).collect();
            let limite = LACET_ANGLE.to_radians().cos();
            // Pour chaque point, ses bras (le dernier point à LACET_BRAS avant, le premier
            // à LACET_BRAS après) s'il tourne de plus de LACET_ANGLE entre eux
            let serre = |i: usize| -> Option<(usize, usize)> {
                let a = (0..i).rev().find(|&x| pts[i].0 - pts[x].0 >= LACET_BRAS)?;
                let b = (i + 1..n).find(|&x| pts[x].0 - pts[i].0 >= LACET_BRAS)?;
                let (u, v) = ([xz[i][0] - xz[a][0], xz[i][1] - xz[a][1]], [xz[b][0] - xz[i][0], xz[b][1] - xz[i][1]]);
                let c = (u[0] * v[0] + u[1] * v[1]) / (u[0].hypot(u[1]) * v[0].hypot(v[1])).max(1e-6);
                (c < limite).then_some((a, b))
            };
            let mut i = 0;
            while i < n {
                let Some((a, mut b)) = serre(i) else {
                    i += 1;
                    continue;
                };
                // Le virage entier : tant que les points suivants tournent aussi
                let mut x = i + 1;
                while x < n && x <= b {
                    if let Some((_, b2)) = serre(x) {
                        b = b.max(b2);
                    }
                    x += 1;
                }
                let (sa, sb, ha, hb) = (pts[a].0, pts[b].0, h[a], h[b]);
                for y in a + 1..b {
                    h[y] = ha + (hb - ha) * (pts[y].0 - sa) / (sb - sa).max(1e-3);
                }
                creux += 1;
                change = true;
                i = b + 1;
            }
            j = n;
        }
        while j + 2 < n {
            // Le bord le plus loin au-delà duquel tout le creux reste sous la ligne
            let mut meilleur: Option<usize> = None;
            let mut k = j + 2;
            while k < n && pts[k].0 - pts[j].0 <= CREUX_LARGEUR {
                let (sj, sk) = (pts[j].0, pts[k].0);
                let ligne = |x: usize| h[j] + (h[k] - h[j]) * (pts[x].0 - sj) / (sk - sj).max(1e-3);
                let pente = ((h[k] - h[j]) / (sk - sj).max(1e-3)).abs();
                let dessous = (j + 1..k).all(|x| h[x] <= ligne(x) + CREUX_TOLERANCE);
                let fond = (j + 1..k).map(|x| ligne(x) - h[x]).fold(f32::MIN, f32::max);
                if pente <= PENTE_ROUTE && dessous && fond >= CREUX_PROFONDEUR {
                    meilleur = Some(k);
                }
                k += 1;
            }
            if let Some(k) = meilleur {
                let (sj, sk, hj, hk) = (pts[j].0, pts[k].0, h[j], h[k]);
                for x in j + 1..k {
                    h[x] = h[x].max(hj + (hk - hj) * (pts[x].0 - sj) / (sk - sj).max(1e-3));
                }
                creux += 1;
                change = true;
                j = k;
            } else {
                j += 1;
            }
        }
        if change {
            for (x, p) in pts.iter().enumerate() {
                for &(k, i) in &p.2 {
                    r.aretes[k].points[i][2] = h[x];
                }
                // Un nœud relevé : toutes ses voies y partent de sa nouvelle hauteur
                if let Some(nd) = p.3 {
                    r.noeuds[nd].p[2] = h[x];
                    for k in r.noeuds[nd].aretes.clone() {
                        let a = &mut r.aretes[k];
                        if a.a == nd {
                            a.points[0][2] = h[x];
                        }
                        if a.b == nd {
                            let m = a.points.len() - 1;
                            a.points[m][2] = h[x];
                        }
                    }
                }
            }
        }
    }
    creux
}
