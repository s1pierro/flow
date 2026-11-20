//! Les bâtiments en volumes fermés, pour la Matérialisation : ils seront unis au terrain
//! (avant les routes : la défonce d'un tunnel recoupe alors leurs sous-sols). Pour
//! l'instant, montrés en surimpression dans la vue Matérialisation, ceux que les
//! booléens refusent d'une autre couleur (0.124). Un bâtiment dont le toit ne ferme pas
//! (forme tourmentée sous un toit unique) est aplani (`volume_accepte`, 0.125).
//!
//! Un volume : le contour (augmenté des points où le pan du dessus change, les hauts
//! des pignons), un fond à plat au pied des murs, les murs du pied jusqu'au toit, et
//! chaque pan du toit (le morceau du contour où son plan est le plus bas, comme
//! `sage_reel::toits::monter`). Les sommets sont soudés (à `SOUDURE` près) : chaque
//! arête est partagée par deux faces.

use crate::booleens::Forme;
use crate::carte::{boite_tuile, Cle, Donnees};
use sage_reel::{aire, toits, trianguler, Batiment};
use std::collections::HashMap;

/// Deux sommets plus proches que ça (m) n'en font qu'un.
const SOUDURE: f32 = 0.002;
/// La couleur des volumes acceptés, et celle des refusés.
pub const ACCEPTE: [f32; 3] = [0.56, 0.64, 0.78];
pub const REFUSE: [f32; 3] = [0.88, 0.22, 0.18];

fn sur(p: &[f32; 3], x: f32, z: f32) -> f32 {
    p[0] + p[1] * x + p[2] * z
}

/// Le morceau d'un polygone où `g` ≤ 0 (`g` affine) : Sutherland-Hodgman.
fn couper(poly: &[[f32; 2]], g: impl Fn([f32; 2]) -> f32) -> Vec<[f32; 2]> {
    let mut o = Vec::with_capacity(poly.len() + 2);
    for k in 0..poly.len() {
        let (p, q) = (poly[k], poly[(k + 1) % poly.len()]);
        let (gp, gq) = (g(p), g(q));
        if gp <= 0.0 {
            o.push(p);
        }
        if (gp <= 0.0) != (gq <= 0.0) {
            let t = gp / (gp - gq);
            o.push([p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t]);
        }
    }
    o
}

/// Les sommets soudés d'une forme en construction.
struct Soudeur {
    sommets: Vec<[f32; 3]>,
    cases: HashMap<(i64, i64, i64), Vec<u32>>,
}

impl Soudeur {
    fn cle(p: [f32; 3]) -> (i64, i64, i64) {
        let c = |v: f32| (v / (4.0 * SOUDURE)).floor() as i64;
        (c(p[0]), c(p[1]), c(p[2]))
    }

    fn sommet(&mut self, p: [f32; 3]) -> u32 {
        let (a, b, c) = Soudeur::cle(p);
        for i in a - 1..=a + 1 {
            for j in b - 1..=b + 1 {
                for k in c - 1..=c + 1 {
                    for &s in self.cases.get(&(i, j, k)).map(|v| v.as_slice()).unwrap_or(&[]) {
                        let q = self.sommets[s as usize];
                        if (q[0] - p[0]).abs() <= SOUDURE && (q[1] - p[1]).abs() <= SOUDURE && (q[2] - p[2]).abs() <= SOUDURE {
                            return s;
                        }
                    }
                }
            }
        }
        let s = self.sommets.len() as u32;
        self.sommets.push(p);
        self.cases.entry((a, b, c)).or_default().push(s);
        s
    }
}

/// Le contour sans points confondus ni trop proches, dans le sens direct ; rien s'il
/// n'en reste pas trois.
fn contour_propre(c: &[[f32; 2]]) -> Option<Vec<[f32; 2]>> {
    let mut v: Vec<[f32; 2]> = Vec::with_capacity(c.len());
    for &p in c {
        if v.last().is_none_or(|q: &[f32; 2]| (p[0] - q[0]).hypot(p[1] - q[1]) > 0.01) {
            v.push(p);
        }
    }
    while v.len() > 3 && (v[0][0] - v[v.len() - 1][0]).hypot(v[0][1] - v[v.len() - 1][1]) <= 0.01 {
        v.pop();
    }
    if v.len() < 3 || aire(&v).abs() < 0.5 {
        return None;
    }
    if aire(&v) < 0.0 {
        v.reverse();
    }
    Some(v)
}

/// Le volume fermé d'un bâtiment ; rien si son contour est dégénéré.
pub fn volume(b: &Batiment) -> Option<Forme> {
    let c = contour_propre(&b.contour)?;
    let pl = toits::plans(b);
    let haut = |x: f32, z: f32| toits::hauteur(&pl, x, z);
    // Le contour augmenté des hauts des pignons (là où le pan du dessus change)
    let mut p: Vec<[f32; 2]> = Vec::with_capacity(c.len() * 2);
    for s in 0..c.len() {
        let (a, d) = (c[s], c[(s + 1) % c.len()]);
        let (dx, dz) = (d[0] - a[0], d[1] - a[1]);
        let mut ts = Vec::new();
        for i in 0..pl.len() {
            for j in i + 1..pl.len() {
                let (ia, id, ja, jd) = (sur(&pl[i], a[0], a[1]), sur(&pl[i], d[0], d[1]), sur(&pl[j], a[0], a[1]), sur(&pl[j], d[0], d[1]));
                let den = (id - ia) - (jd - ja);
                if den.abs() < 1e-6 {
                    continue;
                }
                let t = (ja - ia) / den;
                if t > 1e-3 && t < 1.0 - 1e-3 {
                    let (x, z) = (a[0] + dx * t, a[1] + dz * t);
                    if sur(&pl[i], x, z) <= haut(x, z) + 1e-3 {
                        ts.push(t);
                    }
                }
            }
        }
        ts.sort_by(|x, y| x.total_cmp(y));
        ts.dedup_by(|x, y| (*x - *y).abs() < 1e-3);
        p.push(a);
        p.extend(ts.iter().map(|&t| [a[0] + dx * t, a[1] + dz * t]));
    }
    let n = p.len();
    let mut s = Soudeur { sommets: Vec::new(), cases: HashMap::new() };
    let bas: Vec<u32> = p.iter().map(|q| s.sommet([q[0], b.pied, q[1]])).collect();
    let dessus: Vec<u32> = p.iter().map(|q| s.sommet([q[0], haut(q[0], q[1]), q[1]])).collect();
    let mut faces: Vec<Vec<u32>> = Vec::new();
    let tri = |f: &mut Vec<Vec<u32>>, a: u32, b: u32, c: u32| {
        if a != b && b != c && c != a {
            f.push(vec![a, b, c]);
        }
    };
    // Le fond, tourné vers le bas (le contour est dans le sens direct)
    for [i, j, k] in trianguler(&p) {
        tri(&mut faces, bas[i], bas[k], bas[j]);
    }
    // Les murs
    for i in 0..n {
        let j = (i + 1) % n;
        tri(&mut faces, bas[i], bas[j], dessus[j]);
        tri(&mut faces, bas[i], dessus[j], dessus[i]);
    }
    // Les pans
    for (i, q) in pl.iter().enumerate() {
        let mut morceau = p.clone();
        for (j, r) in pl.iter().enumerate() {
            if i != j && morceau.len() > 2 {
                morceau = couper(&morceau, |x| sur(q, x[0], x[1]) - sur(r, x[0], x[1]));
            }
        }
        if morceau.len() < 3 || aire(&morceau).abs() < 0.01 {
            continue;
        }
        let ids: Vec<u32> = morceau.iter().map(|m| s.sommet([m[0], sur(q, m[0], m[1]), m[1]])).collect();
        let sens = aire(&morceau) > 0.0;
        for [a, b2, c2] in trianguler(&morceau) {
            if sens {
                tri(&mut faces, ids[a], ids[b2], ids[c2]);
            } else {
                tri(&mut faces, ids[a], ids[c2], ids[b2]);
            }
        }
    }
    let mut f = Forme { sommets: s.sommets, faces, matieres: Vec::new() };
    if f.volume() < 0.0 {
        f.faces.iter_mut().for_each(|x| x.reverse());
    }
    Some(f)
}

/// Le volume fermé d'un bâtiment accepté par les booléens : avec son toit si possible,
/// sinon (un toit unique sur une forme tourmentée : ses pans se recoupent) à toit plat
/// à mi-hauteur entre l'égout et le faîtage, par simple extrusion de la base. Rend aussi
/// s'il a été aplani ; rien si même à plat il est refusé.
pub fn volume_accepte(b: &Batiment) -> Option<(Forme, bool)> {
    if let Some(f) = volume(b).filter(crate::booleens::ferme) {
        return Some((f, false));
    }
    let h = (b.egout + b.toit) / 2.0;
    let plat = Batiment::plat(b.genre, b.pied, h, b.contour.clone());
    volume(&plat).filter(crate::booleens::ferme).map(|f| (f, true))
}

/// Les bâtiments de la tuile `cle` (ceux dont le centre y tombe) en volumes fermés,
/// pour la surimpression : sommets de 9 flottants (position, normale, couleur selon
/// qu'il est accepté ou refusé par les booléens), triangles ; et le nombre d'acceptés
/// (dont aplanis, `volume_accepte`) et de refusés.
pub fn tuile(d: &Donnees, cle: Cle) -> (Vec<f32>, Vec<u32>, usize, usize, usize) {
    let _t = crate::suivi::tache("bâtiments fermés (tuile)");
    let bx = boite_tuile(d, cle);
    let (mut v, mut ind) = (Vec::new(), Vec::new());
    let (mut ok, mut aplanis, mut refuses) = (0, 0, 0);
    for k in d.batiments_dans(bx) {
        let bt = &d.lieu.batiments[k as usize];
        let c = bt.centre();
        if c[0] < bx[0] || c[0] >= bx[2] || c[1] < bx[1] || c[1] >= bx[3] || bt.contour.len() < 3 {
            continue;
        }
        let (f, bon) = match volume_accepte(bt) {
            Some((f, plat)) => {
                ok += 1;
                aplanis += usize::from(plat);
                (f, true)
            }
            None => match volume(bt) {
                Some(f) => {
                    refuses += 1;
                    (f, false)
                }
                None => {
                    refuses += 1;
                    continue;
                }
            },
        };
        let couleur = if bon { ACCEPTE } else { REFUSE };
        for p in &f.faces {
            let (a, b, c) = (f.sommets[p[0] as usize], f.sommets[p[1] as usize], f.sommets[p[2] as usize]);
            let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let w = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            let nn = [u[1] * w[2] - u[2] * w[1], u[2] * w[0] - u[0] * w[2], u[0] * w[1] - u[1] * w[0]];
            let l = (nn[0] * nn[0] + nn[1] * nn[1] + nn[2] * nn[2]).sqrt().max(1e-12);
            let nn = nn.map(|x| x / l);
            // Les toits un peu plus clairs que les murs
            let teinte = if nn[1] > 0.3 { couleur.map(|x| (x * 1.15).min(1.0)) } else { couleur };
            let i = (v.len() / crate::relief3d::SOMMET) as u32;
            for q in [a, b, c] {
                v.extend_from_slice(&[q[0], q[1], q[2], nn[0], nn[1], nn[2], teinte[0], teinte[1], teinte[2]]);
            }
            ind.extend_from_slice(&[i, i + 1, i + 2]);
        }
    }
    (v, ind, ok, aplanis, refuses)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Les causes d'un refus : contour qui se recoupe, point touché deux fois, pointe
    /// aller-retour, arêtes du volume non partagées par exactement deux faces.
    fn causes(b: &Batiment) -> Vec<String> {
        let mut v = Vec::new();
        let Some(c) = contour_propre(&b.contour) else { return vec!["contour dégénéré".into()] };
        let n = c.len();
        let croise = |a: [f32; 2], b: [f32; 2], p: [f32; 2], q: [f32; 2]| {
            let o = |a: [f32; 2], b: [f32; 2], c: [f32; 2]| (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
            o(a, b, p) * o(a, b, q) < 0.0 && o(p, q, a) * o(p, q, b) < 0.0
        };
        let mut x = 0;
        for i in 0..n {
            for j in i + 2..n {
                if i == 0 && j == n - 1 {
                    continue;
                }
                if croise(c[i], c[(i + 1) % n], c[j], c[(j + 1) % n]) {
                    x += 1;
                }
            }
        }
        if x > 0 {
            v.push(format!("contour recoupé ({x})"));
        }
        let mut deux = 0;
        for i in 0..n {
            for j in i + 1..n {
                if (c[i][0] - c[j][0]).hypot(c[i][1] - c[j][1]) < 0.01 {
                    deux += 1;
                }
            }
        }
        if deux > 0 {
            v.push(format!("point touché deux fois ({deux})"));
        }
        let mut pointes = 0;
        for i in 0..n {
            let (a, p, q) = (c[(i + n - 1) % n], c[i], c[(i + 1) % n]);
            let (u, w) = ([p[0] - a[0], p[1] - a[1]], [q[0] - p[0], q[1] - p[1]]);
            let cr = u[0] * w[1] - u[1] * w[0];
            let d = u[0] * w[0] + u[1] * w[1];
            if d < 0.0 && cr.abs() < 1e-3 * (u[0].hypot(u[1]) * w[0].hypot(w[1])).max(1e-9) * 50.0 {
                pointes += 1;
            }
        }
        if pointes > 0 {
            v.push(format!("pointe aller-retour ({pointes})"));
        }
        if let Some(f) = volume(b) {
            let mut aretes: HashMap<(u32, u32), i32> = HashMap::new();
            for p in &f.faces {
                for k in 0..p.len() {
                    *aretes.entry((p[k], p[(k + 1) % p.len()])).or_insert(0) += 1;
                }
            }
            let mauvaises = aretes.iter().filter(|(&(a, b), &m)| m != 1 || aretes.get(&(b, a)).copied().unwrap_or(0) != 1).count();
            if mauvaises > 0 {
                v.push(format!("{mauvaises} arêtes mal partagées"));
            }
        }
        if v.is_empty() {
            v.push("cause inconnue".into());
        }
        v
    }

    /// Banc : FLOW_LIEU, FLOW_ZONE « x z rayon » ; les bâtiments refusés et leurs causes.
    #[test]
    #[ignore]
    fn banc_batiments_refuses() {
        let l = sage_reel::Lieu::lire(std::path::Path::new(&std::env::var("FLOW_LIEU").unwrap())).unwrap();
        let z: Vec<f32> = std::env::var("FLOW_ZONE").unwrap().split_whitespace().map(|x| x.parse().unwrap()).collect();
        let (mut ok, mut refuses) = (0, Vec::new());
        for (k, b) in l.batiments.iter().enumerate() {
            let c = b.centre();
            if (c[0] - z[0]).hypot(c[1] - z[1]) > z[2] || b.contour.len() < 3 {
                continue;
            }
            match volume(b) {
                Some(f) if crate::booleens::ferme(&f) => ok += 1,
                _ => {
                    if volume_accepte(b).is_none() {
                        println!("  n° {k} refusé même à plat");
                    }
                    refuses.push(k)
                }
            }
        }
        println!("{ok} acceptés, {} refusés", refuses.len());
        let mut compte: HashMap<String, usize> = HashMap::new();
        for &k in &refuses {
            let b = &l.batiments[k];
            let cs = causes(b);
            for c in &cs {
                *compte.entry(c.split(" (").next().unwrap().split(' ').skip_while(|w| w.parse::<usize>().is_ok()).collect::<Vec<_>>().join(" ")).or_insert(0) += 1;
            }
            let c = b.centre();
            println!("  n° {k} en ({:.0}, {:.0}) : {} points, forme {}, aire {:.0} m² — {}", c[0], c[1], b.contour.len(), b.forme, aire(&b.contour).abs(), cs.join(", "));
        }
        println!("causes : {compte:?}");
        // Le détail du premier : contour, plans, arêtes mal partagées
        if let Some(&k) = refuses.first() {
            let b = &l.batiments[k];
            println!("détail n° {k} : pied {} égout {} toit {} axe {:.3} ; contour {:?}", b.pied, b.egout, b.toit, b.axe, b.contour);
            println!("plans {:?}", toits::plans(b));
            let f = volume(b).unwrap();
            let mut aretes: HashMap<(u32, u32), i32> = HashMap::new();
            for p in &f.faces {
                for k in 0..p.len() {
                    *aretes.entry((p[k], p[(k + 1) % p.len()])).or_insert(0) += 1;
                }
            }
            for (&(a, c), &m) in &aretes {
                let r = aretes.get(&(c, a)).copied().unwrap_or(0);
                if m != 1 || r != 1 {
                    println!("  arête {a}→{c} ×{m}, retour ×{r} : {:?} → {:?}", f.sommets[a as usize], f.sommets[c as usize]);
                }
            }
        }
    }

    fn maison(forme: u8, contour: Vec<[f32; 2]>) -> Batiment {
        Batiment { genre: 0, pied: -1.0, toit: 9.0, egout: 5.0, forme, axe: 0.0, contour }
    }

    #[test]
    fn les_maisons_sont_fermees() {
        let rect = vec![[0.0, 0.0], [12.0, 0.0], [12.0, 8.0], [0.0, 8.0]];
        let l = vec![[0.0, 0.0], [12.0, 0.0], [12.0, 4.0], [6.0, 4.0], [6.0, 10.0], [0.0, 10.0]];
        for forme in [toits::PLAT, toits::DEUX_PANS, toits::CROUPE, toits::APPENTIS] {
            for c in [rect.clone(), l.clone()] {
                let f = volume(&maison(forme, c.clone())).expect("volume");
                assert!(crate::booleens::ferme(&f), "forme {forme}, {} sommets", c.len());
                assert!(f.volume() > 0.0);
            }
        }
    }
}
