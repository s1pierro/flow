//! Les opérations booléennes entre volumes fermés, par `boolmesh` (Rust pur), reprises de
//! Rustcraft. Chaque coin de triangle porte le numéro de sa face d'origine : les
//! triangles d'une même face, coupés par l'opération, se réunissent ensuite en un seul
//! polygone quand ils forment une boucle convexe (un quadrilatère entamé redevient un
//! polygone, pas un éventail de triangles).

use crate::sculpture::V3;
use boolmesh::prelude::*;
use std::collections::{HashMap, HashSet};
use std::ops::{Add, Mul};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Op {
    Union,
    #[default]
    Difference,
    Intersection,
}

impl Op {
    pub const TOUTES: [Op; 3] = [Op::Difference, Op::Union, Op::Intersection];

    pub fn nom(self) -> &'static str {
        match self {
            Op::Union => "Union",
            Op::Difference => "Différence",
            Op::Intersection => "Intersection",
        }
    }
}

/// Une forme : des sommets et des faces (polygones dans le sens direct vu de dehors).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Forme {
    pub sommets: Vec<V3>,
    pub faces: Vec<Vec<u32>>,
}

impl Forme {
    pub fn vide(&self) -> bool {
        self.faces.is_empty()
    }

    /// Plusieurs formes en une seule (sans les unir : elles peuvent se croiser).
    pub fn assembler<'a>(formes: impl IntoIterator<Item = &'a Forme>) -> Forme {
        let mut r = Forme::default();
        for f in formes {
            let d = r.sommets.len() as u32;
            r.sommets.extend_from_slice(&f.sommets);
            r.faces.extend(f.faces.iter().map(|p| p.iter().map(|i| i + d).collect()));
        }
        r
    }

    /// Le volume (positif pour une forme fermée bien orientée).
    pub fn volume(&self) -> f64 {
        let mut v = 0.0;
        for p in &self.faces {
            for i in 1..p.len().saturating_sub(1) {
                let [a, b, c] = [p[0], p[i], p[i + 1]].map(|k| self.sommets[k as usize].map(|x| x as f64));
                v += a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0]) + a[2] * (b[0] * c[1] - b[1] * c[0]);
            }
        }
        v / 6.0
    }

    /// La boîte (min, max), s'il y a des sommets.
    pub fn boite(&self) -> Option<(V3, V3)> {
        let mut it = self.sommets.iter();
        let p = *it.next()?;
        Some(it.fold((p, p), |(a, b), q| ([a[0].min(q[0]), a[1].min(q[1]), a[2].min(q[2])], [b[0].max(q[0]), b[1].max(q[1]), b[2].max(q[2])])))
    }
}

/// Ce que porte un coin : le numéro de sa face d'origine.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Coin {
    face: f64,
}

impl Add for Coin {
    type Output = Coin;
    fn add(self, o: Coin) -> Coin {
        Coin { face: self.face + o.face }
    }
}

impl Mul<f64> for Coin {
    type Output = Coin;
    fn mul(self, k: f64) -> Coin {
        Coin { face: self.face * k }
    }
}

pub const NON_FERME: &str = "maillage non fermé";

fn vers_manifold(m: &Forme, faces: &mut usize) -> Result<Manifold<Coin>, String> {
    if m.faces.is_empty() {
        return Err(NON_FERME.into());
    }
    let mut idx = Vec::with_capacity(m.faces.len() * 6);
    let mut coins = Vec::with_capacity(m.faces.len() * 6);
    let mut pos = Vec::with_capacity(m.sommets.len() * 3);
    let mut local: HashMap<u32, usize> = HashMap::with_capacity(m.sommets.len());
    for p in &m.faces {
        let id = *faces as f64;
        *faces += 1;
        for k in 1..p.len().saturating_sub(1) {
            for i in [p[0], p[k], p[k + 1]] {
                coins.push(Coin { face: id });
                idx.push(*local.entry(i).or_insert_with(|| {
                    let s = m.sommets[i as usize];
                    pos.extend_from_slice(&[s[0] as f64, s[1] as f64, s[2] as f64]);
                    pos.len() / 3 - 1
                }));
            }
        }
    }
    Manifold::new_with_values(&pos, &idx, &coins).map_err(|_| NON_FERME.to_string())
}

fn vers_forme(r: &Manifold<Coin>) -> Forme {
    let sommets: Vec<[f64; 3]> = r.ps.iter().map(|p| [p.x, p.y, p.z]).collect();
    // Les triangles de chaque face d'origine, dans l'ordre d'apparition
    let mut ordre: Vec<usize> = Vec::new();
    let mut groupes: HashMap<usize, Vec<[usize; 3]>> = HashMap::new();
    for t in 0..r.hs.len() / 3 {
        let h = &r.hs[3 * t..3 * t + 3];
        if h.iter().any(|x| x.tail >= sommets.len()) || r.vals.len() < 3 * t + 3 {
            continue;
        }
        let id = (r.vals[3 * t].face + 0.5).floor().max(0.0) as usize;
        groupes
            .entry(id)
            .or_insert_with(|| {
                ordre.push(id);
                Vec::new()
            })
            .push([h[0].tail, h[1].tail, h[2].tail]);
    }
    let mut faces = Vec::new();
    for id in ordre {
        let tris = &groupes[&id];
        match (tris.len() > 1).then(|| contour(&sommets, tris)).flatten() {
            Some(s) => faces.push(s.into_iter().map(|i| i as u32).collect()),
            None => faces.extend(tris.iter().map(|t| t.iter().map(|&i| i as u32).collect())),
        }
    }
    Forme { sommets: sommets.iter().map(|p| [p[0] as f32, p[1] as f32, p[2] as f32]).collect(), faces }
}

/// Le contour d'un groupe de triangles coplanaires : une boucle unique et convexe, sinon
/// rien.
fn contour(sommets: &[[f64; 3]], tris: &[[usize; 3]]) -> Option<Vec<usize>> {
    let aretes: HashSet<(usize, usize)> = tris.iter().flat_map(|&[a, b, c]| [(a, b), (b, c), (c, a)]).collect();
    let mut suivant: HashMap<usize, usize> = HashMap::new();
    let mut depart = None;
    for &[a, b, c] in tris {
        for (i, j) in [(a, b), (b, c), (c, a)] {
            if aretes.contains(&(j, i)) {
                continue; // une arête intérieure
            }
            if suivant.insert(i, j).is_some() {
                return None; // un contour qui se touche lui-même
            }
            depart.get_or_insert(i);
        }
    }
    let depart = depart?;
    let mut boucle = Vec::with_capacity(suivant.len());
    let mut i = depart;
    loop {
        boucle.push(i);
        i = *suivant.get(&i)?;
        if i == depart {
            break;
        }
        if boucle.len() > suivant.len() {
            return None;
        }
    }
    if boucle.len() != suivant.len() {
        return None; // plusieurs boucles : un trou
    }
    // Convexe : tous les virages du même côté que la normale (alignements tolérés)
    let p: Vec<[f64; 3]> = boucle.iter().map(|i| sommets[*i]).collect();
    let l = p.len();
    let mut n = [0.0; 3];
    for k in 0..l {
        let (a, b) = (p[k], p[(k + 1) % l]);
        n[0] += (a[1] - b[1]) * (a[2] + b[2]);
        n[1] += (a[2] - b[2]) * (a[0] + b[0]);
        n[2] += (a[0] - b[0]) * (a[1] + b[1]);
    }
    let virage = |k: usize| {
        let (a, b, c) = (p[k], p[(k + 1) % l], p[(k + 2) % l]);
        let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let w = [c[0] - b[0], c[1] - b[1], c[2] - b[2]];
        [u[1] * w[2] - u[2] * w[1], u[2] * w[0] - u[0] * w[2], u[0] * w[1] - u[1] * w[0]]
    };
    for k in 0..l {
        let x = virage(k);
        if x[0] * n[0] + x[1] * n[1] + x[2] * n[2] < -1e-12 {
            return None;
        }
    }
    // Commencer sur un vrai coin : l'éventail n'a pas de triangle plat au départ
    let coin = (0..l).find(|k| {
        let x = virage((k + l - 1) % l);
        (x[0] * x[0] + x[1] * x[1] + x[2] * x[2]).sqrt() > 1e-12
    });
    boucle.rotate_left(coin.unwrap_or(0));
    Some(boucle)
}

fn calculer(a: &Forme, b: &Forme, op: Op) -> Result<Forme, String> {
    let mut faces = 0;
    let (x, y) = (vers_manifold(a, &mut faces)?, vers_manifold(b, &mut faces)?);
    let t = match op {
        Op::Union => OpType::Add,
        Op::Difference => OpType::Subtract,
        Op::Intersection => OpType::Intersect,
    };
    match compute_boolean(&x, &y, t) {
        Ok(r) => Ok(vers_forme(&r)),
        // boolmesh le dit ainsi quand rien ne reste : un résultat, pas une panne
        Err(e) if e.contains("empty") => Ok(Forme::default()),
        Err(e) => Err(e),
    }
}

/// `a` op `b`, deux volumes fermés. Une panique de la bibliothèque devient une erreur :
/// une forme difficile ne doit pas fermer Flow.
pub fn operation(a: &Forme, b: &Forme, op: Op) -> Result<Forme, String> {
    std::panic::catch_unwind(|| calculer(a, b, op)).unwrap_or_else(|_| Err("calcul impossible sur cette forme".into()))
}

/// `operation`, un côté vide toléré (le résultat vide d'un modifieur peut suivre).
pub fn operer(a: &Forme, b: &Forme, op: Op) -> Result<Forme, String> {
    match (a.vide(), b.vide(), op) {
        (true, _, Op::Union) => Ok(b.clone()),
        (true, _, _) | (_, true, Op::Intersection) => Ok(Forme::default()),
        (_, true, _) => Ok(a.clone()),
        _ => operation(a, b, op),
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// Un cube de côté 1, décalé de `d` en x.
    pub fn cube(d: f32) -> Forme {
        let s: Vec<V3> = (0..8).map(|i| [d + if i & 1 == 0 { -0.5 } else { 0.5 }, if i & 2 == 0 { -0.5 } else { 0.5 }, if i & 4 == 0 { -0.5 } else { 0.5 }]).collect();
        Forme { sommets: s, faces: vec![vec![0, 2, 3, 1], vec![4, 5, 7, 6], vec![0, 1, 5, 4], vec![2, 6, 7, 3], vec![0, 4, 6, 2], vec![1, 3, 7, 5]] }
    }

    #[test]
    fn les_trois_operations_donnent_le_bon_volume() {
        let (a, b) = (cube(0.0), cube(0.5));
        assert!((a.volume() - 1.0).abs() < 1e-6);
        let v = |o| operation(&a, &b, o).unwrap().volume();
        assert!((v(Op::Union) - 1.5).abs() < 1e-5);
        assert!((v(Op::Difference) - 0.5).abs() < 1e-5);
        assert!((v(Op::Intersection) - 0.5).abs() < 1e-5);
    }

    #[test]
    fn une_face_coupee_redevient_un_polygone() {
        let r = operation(&cube(0.0), &cube(0.5), Op::Difference).unwrap();
        assert_eq!(r.faces.len(), 6, "{:?}", r.faces);
        assert!(r.faces.iter().all(|f| f.len() == 4));
    }

    #[test]
    fn rien_de_commun_est_un_resultat_vide() {
        assert!(operation(&cube(0.0), &cube(5.0), Op::Intersection).unwrap().vide());
    }

    #[test]
    fn un_maillage_ouvert_est_refuse_sans_panique() {
        let mut o = cube(0.0);
        o.faces.pop();
        assert_eq!(operation(&o, &cube(0.5), Op::Union).unwrap_err(), NON_FERME);
        assert!(operation(&Forme::default(), &cube(0.5), Op::Union).is_err());
        assert!(operer(&Forme::default(), &cube(0.5), Op::Union).is_ok());
    }
}
