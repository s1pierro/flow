//! La dissolution limitée d'un volume fermé (comme celle de Blender) : les triangles
//! voisins d'un même plan se réunissent en régions ; un sommet au milieu d'une région,
//! ou au milieu d'un bord droit entre deux régions, s'en va ; chaque région se
//! retriangule par oreilles sans eux. Les opérations booléennes laissent des plans
//! hachés en éventails (le dessus d'un remblai, le fond plat d'un lit) : ils redeviennent
//! quelques triangles. Le volume reste fermé, sans fente : un sommet ne part que si
//! toutes les régions qui le touchent se retriangulent. Une région trouée, ou que les
//! oreilles ne savent pas défaire, garde ses triangles et ses sommets.

use crate::booleens::Forme;
use std::collections::{HashMap, HashSet};

/// L'écart d'angle sous lequel deux triangles sont d'un même plan (en cosinus), et
/// l'écart de leurs sommets au plan (m).
pub const COS_PLAN: f64 = 0.99995;
pub const ECART_PLAN: f64 = 0.01;

type P = [f64; 3];

fn sous(a: P, b: P) -> P {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn vect(a: P, b: P) -> P {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn scal(a: P, b: P) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Les triangles du volume (ses faces en éventails), et la matière de chacun.
fn triangles(f: &Forme) -> (Vec<[u32; 3]>, Vec<u16>) {
    let mut t = Vec::with_capacity(f.faces.len() * 2);
    let mut m = Vec::with_capacity(f.faces.len() * 2);
    for (k0, p) in f.faces.iter().enumerate() {
        for k in 1..p.len().saturating_sub(1) {
            t.push([p[0], p[k], p[k + 1]]);
            m.push(f.matiere(k0));
        }
    }
    (t, m)
}

pub fn dissoudre(f: &Forme) -> Forme {
    let pos: Vec<P> = f.sommets.iter().map(|p| p.map(|x| x as f64)).collect();
    // Une région ne réunit que des triangles d'une même matière
    let (tri, mat) = triangles(f);
    let n = tri.len();
    // La normale de chaque triangle (nulle s'il est dégénéré)
    let normales: Vec<P> = tri.iter().map(|t| {
        let c = vect(sous(pos[t[1] as usize], pos[t[0] as usize]), sous(pos[t[2] as usize], pos[t[0] as usize]));
        let l = scal(c, c).sqrt();
        if l > 1e-12 { c.map(|x| x / l) } else { [0.0; 3] }
    }).collect();
    // Les voisins par les arêtes (orientées : l'arête a→b d'un triangle est b→a du voisin)
    let mut par_arete: HashMap<(u32, u32), usize> = HashMap::with_capacity(n * 3);
    for (k, t) in tri.iter().enumerate() {
        for j in 0..3 {
            par_arete.insert((t[j], t[(j + 1) % 3]), k);
        }
    }
    // Les régions : depuis un triangle, ceux de son plan, de proche en proche
    let mut region = vec![usize::MAX; n];
    let mut regions: Vec<Vec<usize>> = Vec::new();
    for s in 0..n {
        if region[s] != usize::MAX {
            continue;
        }
        let r = regions.len();
        region[s] = r;
        let mut liste = vec![s];
        let (ns, o) = (normales[s], pos[tri[s][0] as usize]);
        let plan = ns != [0.0; 3];
        let mut k = 0;
        while k < liste.len() && plan {
            let t = tri[liste[k]];
            k += 1;
            for j in 0..3 {
                let Some(&v) = par_arete.get(&(t[(j + 1) % 3], t[j])) else { continue };
                if region[v] != usize::MAX || mat[v] != mat[s] || scal(normales[v], ns) < COS_PLAN {
                    continue;
                }
                if tri[v].iter().all(|&i| scal(sous(pos[i as usize], o), ns).abs() < ECART_PLAN) {
                    region[v] = r;
                    liste.push(v);
                }
            }
        }
        regions.push(liste);
    }
    // Le bord de chaque région : ses arêtes orientées sans voisin dans la région, en
    // boucles (une seule : sinon la région est trouée, ou sa boucle se touche)
    let boucle = |r: usize| -> Option<Vec<u32>> {
        let mut suivant: HashMap<u32, u32> = HashMap::new();
        for &k in &regions[r] {
            let t = tri[k];
            for j in 0..3 {
                let (a, b) = (t[j], t[(j + 1) % 3]);
                let dehors = par_arete.get(&(b, a)).is_none_or(|&v| region[v] != r);
                if dehors && suivant.insert(a, b).is_some() {
                    return None;
                }
            }
        }
        let &debut = suivant.keys().next()?;
        let mut l = vec![debut];
        let mut c = suivant[&debut];
        while c != debut {
            l.push(c);
            c = *suivant.get(&c)?;
            if l.len() > suivant.len() {
                return None;
            }
        }
        (l.len() == suivant.len()).then_some(l)
    };
    let mut bords: Vec<Option<Vec<u32>>> = (0..regions.len()).map(boucle).collect();
    // Les régions de chaque sommet
    let mut autour: Vec<Vec<usize>> = vec![Vec::new(); pos.len()];
    for (k, t) in tri.iter().enumerate() {
        for &i in t {
            if !autour[i as usize].contains(&region[k]) {
                autour[i as usize].push(region[k]);
            }
        }
    }
    // Un sommet part s'il est dans une seule région (au milieu d'elle), ou entre deux
    // régions sur un bord droit ; et que toutes ses régions se refont. Une région qui ne
    // se retriangule pas renonce, et ses sommets restent : on recommence.
    let mut refaites: HashMap<usize, Vec<[u32; 3]>> = HashMap::new();
    loop {
        let partants: HashSet<u32> = (0..pos.len() as u32).filter(|&i| {
            let rs = &autour[i as usize];
            if rs.is_empty() || !rs.iter().all(|&r| bords[r].is_some()) {
                return false;
            }
            match rs.len() {
                1 => !bords[rs[0]].as_ref().unwrap().contains(&i),
                2 => {
                    // Ses deux voisins sur le bord de la première région : alignés avec lui
                    let l = bords[rs[0]].as_ref().unwrap();
                    let Some(k) = l.iter().position(|&x| x == i) else { return false };
                    let (a, b) = (pos[l[(k + l.len() - 1) % l.len()] as usize], pos[l[(k + 1) % l.len()] as usize]);
                    let p = pos[i as usize];
                    let (u, v) = (sous(p, a), sous(b, p));
                    let c = vect(u, v);
                    scal(c, c).sqrt() < 1e-6 * (scal(u, u).sqrt() * scal(v, v).sqrt()).max(1e-12) && scal(u, v) > 0.0
                }
                _ => false,
            }
        }).collect();
        refaites.clear();
        let mut renonce = false;
        for r in 0..regions.len() {
            let Some(l) = &bords[r] else { continue };
            let gardes: Vec<u32> = l.iter().copied().filter(|i| !partants.contains(i)).collect();
            // Rien ne part de cette région : elle reste telle quelle
            let interieur = regions[r].iter().any(|&k| tri[k].iter().any(|i| partants.contains(i) && !l.contains(i)));
            if gardes.len() == l.len() && !interieur {
                continue;
            }
            match oreilles(&gardes, &pos, normales[regions[r][0]]) {
                Some(t) => {
                    refaites.insert(r, t);
                }
                None => {
                    bords[r] = None;
                    renonce = true;
                }
            }
        }
        if !renonce {
            break;
        }
    }
    // Le volume refait : les régions retriangulées, les autres telles quelles
    let mut r = Forme::default();
    let mut nouveau: HashMap<u32, u32> = HashMap::new();
    let mut num = |i: u32, r: &mut Forme| *nouveau.entry(i).or_insert_with(|| {
        r.sommets.push(f.sommets[i as usize]);
        (r.sommets.len() - 1) as u32
    });
    for (k, t) in tri.iter().enumerate() {
        if refaites.contains_key(&region[k]) {
            continue;
        }
        let f = t.map(|i| num(i, &mut r));
        r.faces.push(f.to_vec());
        r.matieres.push(mat[k]);
    }
    let mut cles: Vec<&usize> = refaites.keys().collect();
    cles.sort();
    for c in cles {
        let m = mat[regions[*c][0]];
        for t in &refaites[c] {
            let f = t.map(|i| num(i, &mut r));
            r.faces.push(f.to_vec());
            r.matieres.push(m);
        }
    }
    if r.matieres.iter().all(|&m| m == 0) {
        r.matieres.clear();
    }
    // Par sûreté : un résultat qui ne serait plus fermé, on n'en veut pas
    if ferme(f) && !ferme(&r) {
        return f.clone();
    }
    r
}

/// Chaque arête parcourue une fois dans chaque sens.
pub fn ferme(f: &Forme) -> bool {
    let mut a: HashMap<(u32, u32), i32> = HashMap::new();
    for p in &f.faces {
        for k in 0..p.len() {
            *a.entry((p[k], p[(k + 1) % p.len()])).or_default() += 1;
        }
    }
    a.iter().all(|(&(x, y), &c)| c == 1 && a.get(&(y, x)) == Some(&1))
}

/// Le polygone `l` (une boucle tournant dans le sens direct autour de `n`) en triangles,
/// par oreilles, dans le plan le plus proche de lui ; rien s'il n'y parvient pas.
fn oreilles(l: &[u32], pos: &[P], n: P) -> Option<Vec<[u32; 3]>> {
    if l.len() < 3 {
        return None;
    }
    // Le plan de projection : sans l'axe le plus fort de la normale
    let ax = (0..3).max_by(|&a, &b| n[a].abs().total_cmp(&n[b].abs())).unwrap();
    let (u, v) = match ax {
        0 => (1, 2),
        1 => (2, 0),
        _ => (0, 1),
    };
    let signe = if n[ax] >= 0.0 { 1.0 } else { -1.0 };
    let p2 = |i: u32| [pos[i as usize][u], pos[i as usize][v]];
    let croix = |a: [f64; 2], b: [f64; 2], c: [f64; 2]| ((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])) * signe;
    let mut reste: Vec<u32> = l.to_vec();
    let mut t = Vec::with_capacity(l.len());
    let mut garde = 0;
    while reste.len() > 3 {
        let m = reste.len();
        let mut trouve = false;
        for k in 0..m {
            let (ia, ib, ic) = (reste[(k + m - 1) % m], reste[k], reste[(k + 1) % m]);
            let (a, b, c) = (p2(ia), p2(ib), p2(ic));
            let aire = croix(a, b, c);
            if aire <= 1e-9 {
                continue;
            }
            // Aucun autre sommet dans l'oreille (ni sur ses bords)
            let dedans = reste.iter().any(|&j| {
                if j == ia || j == ib || j == ic {
                    return false;
                }
                let p = p2(j);
                croix(a, b, p) >= -1e-12 && croix(b, c, p) >= -1e-12 && croix(c, a, p) >= -1e-12
            });
            if dedans {
                continue;
            }
            t.push([ia, ib, ic]);
            reste.remove(k);
            trouve = true;
            break;
        }
        garde += 1;
        if !trouve || garde > l.len() * 2 {
            return None;
        }
    }
    let (a, b, c) = (p2(reste[0]), p2(reste[1]), p2(reste[2]));
    if croix(a, b, c) <= 1e-9 {
        return None;
    }
    t.push([reste[0], reste[1], reste[2]]);
    Some(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Un pavé dont chaque face est une grille de `n` × `n` carrés en deux triangles.
    fn pave_hache(n: usize) -> Forme {
        let mut f = Forme::default();
        let mut index: HashMap<[i64; 3], u32> = HashMap::new();
        let mut id = |p: [i64; 3], f: &mut Forme| *index.entry(p).or_insert_with(|| {
            f.sommets.push(p.map(|x| x as f32 / n as f32));
            (f.sommets.len() - 1) as u32
        });
        let m = n as i64;
        // Chaque face : (axe fixe, valeur, sens)
        for (ax, val, dehors) in [(0, 0, -1), (0, m, 1), (1, 0, -1), (1, m, 1), (2, 0, -1), (2, m, 1)] {
            let (u, v) = ((ax + 1) % 3, (ax + 2) % 3);
            for a in 0..m {
                for b in 0..m {
                    let p = |da: i64, db: i64| {
                        let mut q = [0i64; 3];
                        q[ax] = val;
                        q[u] = a + da;
                        q[v] = b + db;
                        q
                    };
                    let c = [p(0, 0), p(1, 0), p(1, 1), p(0, 1)].map(|q| id(q, &mut f));
                    let quad = if dehors > 0 { vec![c[0], c[1], c[2], c[3]] } else { vec![c[3], c[2], c[1], c[0]] };
                    f.faces.push(vec![quad[0], quad[1], quad[2]]);
                    f.faces.push(vec![quad[0], quad[2], quad[3]]);
                }
            }
        }
        f
    }


    #[test]
    fn un_pave_hache_redevient_douze_triangles() {
        let f = pave_hache(5);
        assert!(ferme(&f) && (f.volume() - 1.0).abs() < 1e-4);
        let r = dissoudre(&f);
        assert!(ferme(&r), "{:?}", r.faces);
        assert_eq!((r.sommets.len(), r.faces.len()), (8, 12));
        assert!((r.volume() - 1.0).abs() < 1e-4);
    }

    #[test]
    fn un_volume_que_rien_ne_simplifie_reste_tel_quel() {
        let f = crate::booleens::tests::cube(0.0);
        let r = dissoudre(&f);
        assert!(ferme(&r));
        assert!((r.volume() - f.volume()).abs() < 1e-5);
        assert!(r.faces.len() <= 12);
    }
}
