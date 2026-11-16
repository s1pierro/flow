//! Le diagnostic du réseau des voies : ce que les données d'un lieu disent de la
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
    fn nom(self) -> &'static str {
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

/// Un nœud du réseau : un carrefour, un raccord (changement de passage) ou un bout.
#[derive(Clone, Debug)]
pub struct Noeud {
    /// x, z, hauteur (comme les points des routes).
    pub p: [f32; 3],
    pub aretes: Vec<usize>,
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
}

#[derive(Clone, Debug, Default)]
pub struct Reseau {
    pub noeuds: Vec<Noeud>,
    pub aretes: Vec<Arete>,
    /// Ce que la construction a fait : raccords, nœuds ajoutés aux croisements à niveau.
    pub raccords: usize,
    pub a_niveau: usize,
}

/// Un tracé en cours de construction.
struct Brut {
    passage: Passage,
    genre: u8,
    largeur: f32,
    points: Vec<[f32; 3]>,
}

fn projeter(p: [f32; 3], a: [f32; 3], b: [f32; 3]) -> (f32, f32) {
    let (ux, uz) = (b[0] - a[0], b[1] - a[1]);
    let l2 = (ux * ux + uz * uz).max(1e-12);
    let u = (((p[0] - a[0]) * ux + (p[1] - a[1]) * uz) / l2).clamp(0.0, 1.0);
    (u, (p[0] - a[0] - ux * u).hypot(p[1] - a[1] - uz * u))
}

fn entre(a: [f32; 3], b: [f32; 3], u: f32) -> [f32; 3] {
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
    let mut t: Vec<Brut> = traces(l).into_iter().filter(|x| x.route.points.len() >= 2).map(|x| Brut { passage: x.passage, genre: x.route.genre, largeur: x.route.largeur, points: x.route.points.clone() }).collect();
    let mut r = Reseau::default();
    // Les sommets de tous les tracés, au centimètre
    let compter = |t: &[Brut]| {
        let mut s: HashMap<(i64, i64), usize> = HashMap::new();
        for x in t {
            for p in &x.points {
                *s.entry(cle(*p)).or_default() += 1;
            }
        }
        s
    };
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
                if sommets[&cle(p)] > 1 {
                    continue;
                }
                let mut best: Option<(usize, usize, f32, f32)> = None;
                for &(kk, s) in index.autour(p) {
                    if kk == k {
                        continue;
                    }
                    let (u, d) = projeter(p, t[kk].points[s], t[kk].points[s + 1]);
                    if d < RACCORD && best.is_none_or(|b| d < b.3) {
                        best = Some((kk, s, u, d));
                    }
                }
                let Some((kk, s, u, _)) = best else { continue };
                let (a, b) = (t[kk].points[s], t[kk].points[s + 1]);
                let lg = (b[0] - a[0]).hypot(b[1] - a[1]);
                // Sur un sommet existant s'il est tout près ; sinon un sommet nouveau
                let cible = if u * lg < SUR_SOMMET {
                    a
                } else if (1.0 - u) * lg < SUR_SOMMET {
                    b
                } else {
                    let q = entre(a, b, u);
                    inserer.push((kk, s, u, q));
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
                    if k1 == k2 || t[k1].passage != Passage::Sol || t[k2].passage != Passage::Sol {
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
    let mut numero: HashMap<(i64, i64), usize> = HashMap::new();
    let mut noeud = |p: [f32; 3], r: &mut Reseau| -> usize {
        *numero.entry(cle(p)).or_insert_with(|| {
            r.noeuds.push(Noeud { p, aretes: Vec::new() });
            r.noeuds.len() - 1
        })
    };
    for x in &t {
        let n = x.points.len();
        let mut debut = 0;
        for i in 1..n {
            if i == n - 1 || sommets[&cle(x.points[i])] > 1 {
                let (a, b) = (noeud(x.points[debut], &mut r), noeud(x.points[i], &mut r));
                let k = r.aretes.len();
                r.aretes.push(Arete { a, b, points: x.points[debut..=i].to_vec(), passage: x.passage, genre: x.genre, largeur: x.largeur });
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

    /// Le réseau autour de (x, z), à moins de `rayon` mètres, en rubans colorés par
    /// passage (le sol gris, les ponts orange, les tunnels violets) ; les bouts libres
    /// marqués en rouge, les nœuds où le passage change en jaune.
    pub fn scene(&self, x: f32, z: f32, rayon: f32) -> flow_banc::Scene {
        let mut s = flow_banc::Scene::default();
        let dedans = |p: [f32; 3]| (p[0] - x).hypot(p[1] - z) < rayon;
        for a in &self.aretes {
            let c = match a.passage {
                Passage::Sol => [0.62, 0.62, 0.66],
                Passage::Pont => [0.95, 0.6, 0.2],
                Passage::Tunnel => [0.6, 0.35, 0.85],
            };
            let w = (a.largeur * 0.5).max(0.8);
            for seg in a.points.windows(2) {
                let (p, q) = (seg[0], seg[1]);
                if !dedans(p) && !dedans(q) {
                    continue;
                }
                let (dx, dz) = (q[0] - p[0], q[1] - p[1]);
                let l = dx.hypot(dz).max(1e-6);
                let (nx, nz) = (-dz / l * w, dx / l * w);
                let v = |m: [f32; 3], sx: f32| [m[0] + nx * sx, m[2], m[1] + nz * sx];
                s.polygone(&[v(p, -1.0), v(p, 1.0), v(q, 1.0), v(q, -1.0)], c);
                s.polygone(&[v(p, -1.0), v(q, -1.0), v(q, 1.0), v(p, 1.0)], c);
            }
        }
        for n in self.noeuds.iter().filter(|n| dedans(n.p)) {
            let mut p: Vec<Passage> = n.aretes.iter().map(|&k| self.aretes[k].passage).collect();
            p.sort_by_key(|x| *x as u8);
            p.dedup();
            let c = if n.aretes.len() == 1 {
                [0.9, 0.15, 0.15]
            } else if p.len() > 1 {
                [0.95, 0.85, 0.1]
            } else {
                continue;
            };
            let (h, y) = (2.0, n.p[2] + 0.3);
            s.polygone(&[[n.p[0] - h, y, n.p[1] - h], [n.p[0] - h, y, n.p[1] + h], [n.p[0] + h, y, n.p[1] + h], [n.p[0] + h, y, n.p[1] - h]], c);
            s.polygone(&[[n.p[0] - h, y, n.p[1] - h], [n.p[0] + h, y, n.p[1] - h], [n.p[0] + h, y, n.p[1] + h], [n.p[0] - h, y, n.p[1] + h]], c);
        }
        s
    }
}
