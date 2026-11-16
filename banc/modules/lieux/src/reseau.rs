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

// --- Étape 2 : les contraintes de hauteur -------------------------------------------

/// Le gabarit sous un ouvrage (m), et l'épaisseur d'un tablier ou d'une dalle de
/// tunnel : l'écart de hauteur minimal entre deux voies qui se croisent dénivelées.
pub const GABARIT: f32 = 4.5;
pub const EPAISSEUR: f32 = 1.0;
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
    let ecart = GABARIT + EPAISSEUR;
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
                // Deux arêtes qui partagent un nœud et s'y touchent ne se croisent pas
                let Some((u1, u2)) = croise(a1.points[s1], a1.points[s1 + 1], a2.points[s2], a2.points[s2 + 1]) else { continue };
                if !vus.insert(((k1, s1).min((k2, s2)), (k1, s1).max((k2, s2)))) {
                    continue;
                }
                let (o1, o2) = ((k1, s1, u1), (k2, s2, u2));
                let (h1, h2) = (r.hauteur(o1), r.hauteur(o2));
                use Passage::*;
                // Qui passe dessus : le pont, puis le sol, puis le tunnel ; à passage égal,
                // le plus haut aujourd'hui
                let rang = |p: Passage| match p {
                    Pont => 2,
                    Sol => 1,
                    Tunnel => 0,
                };
                let premier_dessus = match rang(a1.passage).cmp(&rang(a2.passage)) {
                    std::cmp::Ordering::Greater => true,
                    std::cmp::Ordering::Less => false,
                    std::cmp::Ordering::Equal => h1 >= h2,
                };
                let ((haut, ph), (bas, pb)) = if premier_dessus { ((o1, a1.passage), (o2, a2.passage)) } else { ((o2, a2.passage), (o1, a1.passage)) };
                let genre = if pb == Tunnel && ph != Pont { Genre::TunnelSousVoie } else { Genre::PontSurVoie };
                v.push(Contrainte { genre, haut, bas: Some(bas), plancher: 0.0, ecart, implicite: ph == Sol && genre == Genre::PontSurVoie });
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
                        v.push(Contrainte { genre: Genre::PontSurEau, haut: (k, s, u), bas: None, plancher: niveau_eau(p[0], p[1]), ecart: TIRANT_D_AIR + EPAISSEUR, implicite: a.passage == Passage::Sol });
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
                v.push(Contrainte { genre: Genre::PontSurEau, haut: (k, i, 0.0), bas: None, plancher: l.niveau, ecart: TIRANT_D_AIR + EPAISSEUR, implicite: false });
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

/// Les contraintes autour de (x, z) en barres debout : du bas (la voie dessous ou l'eau)
/// à la hauteur voulue, vertes si elles sont tenues, rouges sinon, une marque blanche à
/// la hauteur d'aujourd'hui.
pub fn scene_contraintes(r: &Reseau, v: &[Contrainte], x: f32, z: f32, rayon: f32, s: &mut flow_banc::Scene) {
    for c in v {
        let p = r.point(c.haut);
        if (p[0] - x).hypot(p[1] - z) > rayon {
            continue;
        }
        let bas = c.bas.map_or(c.plancher, |b| r.hauteur(b));
        let voulu = bas + c.ecart;
        let col = if r.manque(c) > 0.01 { [0.95, 0.2, 0.2] } else { [0.25, 0.85, 0.35] };
        let e = 0.5;
        let barre = |s: &mut flow_banc::Scene, y0: f32, y1: f32, col: [f32; 3], e: f32| {
            let q = |dx: f32, y: f32, dz: f32| [p[0] + dx, y, p[1] + dz];
            for (a, b) in [((-e, -e), (e, -e)), ((e, -e), (e, e)), ((e, e), (-e, e)), ((-e, e), (-e, -e))] {
                s.polygone(&[q(a.0, y0, a.1), q(b.0, y0, b.1), q(b.0, y1, b.1), q(a.0, y1, a.1)], col);
            }
        };
        barre(s, bas, voulu, col, e);
        let h = r.hauteur(c.haut);
        barre(s, h - 0.15, h + 0.15, [1.0, 1.0, 1.0], 1.2);
    }
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
/// Le poids qui retient un point au sol à sa hauteur d'origine (le relief lissé), et
/// celui, faible, d'un point de pont ou de tunnel (ses rampes suivent le reste).
const RETENUE_SOL: f32 = 0.3;
const RETENUE_OUVRAGE: f32 = 0.02;
/// Une contrainte ne monte (ou ne descend) pas une voie de plus de tant (m) : au-delà,
/// les données se contredisent, elle reste non tenue.
const MONTEE_MAX: f32 = 12.0;

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
    let t0 = std::time::Instant::now();
    let nn = r.noeuds.len();
    // Les variables : les nœuds, puis les points intérieurs des arêtes
    let mut ids: Vec<Vec<usize>> = Vec::with_capacity(r.aretes.len());
    let mut y: Vec<f32> = vec![0.0; nn];
    let mut cible: Vec<f32> = vec![0.0; nn];
    let mut retenue: Vec<f32> = vec![0.0; nn];
    // Les nœuds : la moyenne des bouts au sol qui y arrivent (sinon de tous)
    let mut somme = vec![(0.0f32, 0usize, 0.0f32, 0usize); nn];
    for a in &r.aretes {
        for (n, p) in [(a.a, a.points[0]), (a.b, *a.points.last().unwrap())] {
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
            v.push(y.len() - 1);
        }
        v.push(a.b);
        ids.push(v);
    }
    let nv = y.len();
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
            voisins[x].push(z);
            voisins[z].push(x);
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
    }
    let var = |(a, s, u): Ou| (ids[a][s], ids[a][s + 1], u);
    let cs: Vec<C> = v
        .iter()
        .map(|c| {
            let ph = r.aretes[c.haut.0].passage;
            let pb = c.bas.map(|b| r.aretes[b.0].passage);
            let part = match (ph, pb) {
                (_, None) => 1.0,
                (Passage::Pont, Some(Passage::Sol)) => 0.85,
                (Passage::Sol, Some(Passage::Tunnel)) => 0.15,
                _ => 0.5,
            };
            C { h: var(c.haut), b: c.bas.map(var), plancher: c.plancher, ecart: c.ecart, part }
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
    for _ in 0..iterations {
        for i in 0..nv {
            let vs = &voisins[i];
            if vs.is_empty() {
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
                let s = if y[a] > y[b] { 0.5 * e } else { -0.5 * e };
                y[a] -= s;
                y[b] += s;
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
        if let Some(b) = c.b {
            fixe_bas[b.0] = true;
            fixe_bas[b.1] = true;
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
        tours += 1;
        for i in 0..nv {
            y[i] = base[i] + monte[i] - descend[i];
        }
        let (mut hauts, mut bas_) = (Vec::new(), Vec::new());
        for c in cs.iter() {
            let bas = c.b.map_or(c.plancher, |b| lire(&y, b));
            let m = bas + c.ecart - lire(&y, c.h);
            if m <= 0.02 {
                continue;
            }
            let dh = (m * c.part + 0.01).min(MONTEE_MAX);
            for v in [c.h.0, c.h.1] {
                monte[v] = (monte[v] + dh).min(MONTEE_MAX);
                hauts.push(v);
            }
            if let Some(b) = c.b {
                let db = (m * (1.0 - c.part) + 0.01).min(MONTEE_MAX);
                for v in [b.0, b.1] {
                    descend[v] = (descend[v] + db).min(MONTEE_MAX);
                    bas_.push(v);
                }
            }
        }
        if hauts.is_empty() && bas_.is_empty() {
            break;
        }
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
