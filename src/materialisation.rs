//! La Matérialisation des Lieux (vue Matérialisation de la 3D) : les tuiles les plus
//! fines, fabriquées comme celles de la Maquette, puis, route par route (les piétonnes
//! ne sont ni remblayées ni défoncées) :
//! 1. l'union de son remblai : de la chaussée jusqu'à `SOUS` sous le terrain ; sur un
//!    pont (bretelle comprise) ou dans un tunnel, une dalle de `OUVRAGE` mètres ;
//! 2. sa défonce par le dessus : de la chaussée jusqu'à `DEFONCE_HAUT` au-dessus ; son
//!    fond est la chaussée (bitume), ses côtés et son dessus du béton clair.
//!
//! Chaque volume est un ruban le long du tracé de la route (`ruban`) : une section
//! horizontale en travers à chaque point du tracé, les sections reliées. Aucun triangle
//! ne relie deux routes : la chaussée est plate en travers par construction (0.120 ;
//! avant, le plan des voies fusionné puis triangulé donnait stries et dévers).
//! Les volumes de défonce, coupés à la tuile, sont joints en matière
//! `MAQUETTE_DEFONCE` : la 3D les montre avec la défonce des lits.

use crate::booleens::{Forme, Op};
use crate::carte::{Cle, Donnees};
use crate::maquette as mq;
use sage_reel::reseau::{Passage, Reseau};

/// Des volumes disjoints en une seule forme (plusieurs coques).
#[allow(dead_code)]
fn reunir_formes(v: Vec<Forme>) -> Forme {
    let mut f = Forme::default();
    for g in v {
        let n = f.sommets.len() as u32;
        f.sommets.extend(g.sommets);
        f.faces.extend(g.faces.into_iter().map(|p| p.into_iter().map(|i| i + n).collect()));
        f.matieres.extend(g.matieres);
    }
    f
}

fn pave(a: [f32; 3], b: [f32; 3]) -> Forme {
    let p = |i: usize| [if i & 1 == 0 { a[0] } else { b[0] }, if i & 2 == 0 { a[1] } else { b[1] }, if i & 4 == 0 { a[2] } else { b[2] }];
    let mut f = Forme { sommets: (0..8).map(p).collect(), faces: vec![vec![0, 4, 6, 2], vec![1, 3, 7, 5], vec![0, 1, 5, 4], vec![2, 6, 7, 3], vec![0, 2, 3, 1], vec![4, 5, 7, 6]], matieres: Vec::new() };
    if f.volume() < 0.0 {
        f.faces.iter_mut().for_each(|p| p.reverse());
    }
    f
}


/// Ce que le remblai descend sous le terrain (m), comme les remblais en surimpression.
const SOUS: f32 = 0.3;
/// La défonce des voies par le dessus : de la voie (un peu dessous, pour ne pas
/// laisser de face confondue avec le dessus du remblai) jusqu'à `DEFONCE_HAUT` au-dessus.
const DEFONCE_HAUT: f32 = 3.2;
const DEFONCE_SOUS: f32 = 0.05;
/// Le remblai d'un pont (bretelle en l'air comprise) ou d'un tunnel : une dalle de
/// `OUVRAGE` mètres sous la chaussée, comme en surimpression.
const OUVRAGE: f32 = 1.0;

/// La matière d'un volume de défonce des voies : son dessous (tourné vers le bas, il
/// devient le fond, la chaussée) en bitume ; ses côtés et son dessus en béton clair.
fn chaussee(f: Forme) -> Forme {
    let mut f = f.en(mq::matiere::BETON);
    for k in 0..f.faces.len() {
        let p = &f.faces[k];
        let mut ny = 0.0f32;
        let mut l2 = [0f32; 3];
        for i in 0..p.len() {
            let (a, b) = (f.sommets[p[i] as usize], f.sommets[p[(i + 1) % p.len()] as usize]);
            l2[0] += (a[1] - b[1]) * (a[2] + b[2]);
            ny += (a[2] - b[2]) * (a[0] + b[0]);
            l2[2] += (a[0] - b[0]) * (a[1] + b[1]);
        }
        let l = (l2[0] * l2[0] + ny * ny + l2[2] * l2[2]).sqrt();
        if l > 1e-9 && ny / l < -0.6 {
            f.matieres[k] = mq::matiere::CHAUSSEE;
        }
    }
    f
}

/// La demi-longueur (m) du lissage des hauteurs le long d'une voie : le réseau nivelé
/// a des écarts de quelques dizaines de centimètres d'un point à l'autre.
const LISSAGE: f32 = 10.0;

/// Les points (x, z, h) d'une voie, les hauteurs lissées le long d'elle (moyenne
/// pondérée en triangle sur ± `LISSAGE` mètres).
fn lisser(p: &[[f32; 3]]) -> Vec<[f32; 3]> {
    let mut s = vec![0.0f32; p.len()];
    for i in 1..p.len() {
        s[i] = s[i - 1] + (p[i][0] - p[i - 1][0]).hypot(p[i][1] - p[i - 1][1]);
    }
    (0..p.len())
        .map(|i| {
            let (mut somme, mut poids) = (0.0, 0.0);
            for j in 0..p.len() {
                let w = (1.0 - (s[j] - s[i]).abs() / LISSAGE).max(0.0);
                somme += w * p[j][2];
                poids += w;
            }
            [p[i][0], p[i][1], somme / poids]
        })
        .collect()
}

/// Les points (x, z, h) plus proches que ça (m) sont confondus.
const CONFONDUS: f32 = 0.05;
/// Au-delà de ce virage (cosinus entre deux segments), le ruban est coupé en deux.
const VIRAGE_MAX: f32 = 0.0;

/// Les morceaux d'un tracé (x, z, h) bons pour un ruban : les points confondus ôtés,
/// coupé aux virages plus serrés que `VIRAGE_MAX` (le morceau suivant repart du point
/// du virage).
fn morceaux(p: &[[f32; 3]]) -> Vec<Vec<[f32; 3]>> {
    let mut v: Vec<[f32; 3]> = Vec::with_capacity(p.len());
    for &q in p {
        if v.last().is_none_or(|l: &[f32; 3]| (q[0] - l[0]).hypot(q[1] - l[1]) > CONFONDUS) {
            v.push(q);
        }
    }
    let dir = |a: [f32; 3], b: [f32; 3]| {
        let l = (b[0] - a[0]).hypot(b[1] - a[1]).max(1e-6);
        [(b[0] - a[0]) / l, (b[1] - a[1]) / l]
    };
    let mut out = Vec::new();
    let mut cur: Vec<[f32; 3]> = Vec::new();
    for i in 0..v.len() {
        cur.push(v[i]);
        if i > 0 && i + 1 < v.len() {
            let (a, b) = (dir(v[i - 1], v[i]), dir(v[i], v[i + 1]));
            if a[0] * b[0] + a[1] * b[1] < VIRAGE_MAX {
                out.push(std::mem::take(&mut cur));
                cur.push(v[i]);
            }
        }
    }
    out.push(cur);
    out.retain(|m| m.len() >= 2);
    out
}

/// Le ruban d'un morceau de tracé (x, z, h) de demi-largeur `w` : à chaque point une
/// section horizontale en travers (bords en onglet), de `bas(gauche, centre, droite)`
/// à `haut(h)` ; les deux bouts prolongés de `w` (ils couvrent le carrefour).
fn ruban(p: &[[f32; 3]], w: f32, haut: impl Fn(f32) -> f32, bas: impl Fn([f32; 2], [f32; 3], [f32; 2]) -> f32) -> Forme {
    let n = p.len();
    let dir = |a: [f32; 3], b: [f32; 3]| {
        let l = (b[0] - a[0]).hypot(b[1] - a[1]).max(1e-6);
        [(b[0] - a[0]) / l, (b[1] - a[1]) / l]
    };
    // Les points, bouts prolongés
    let (d0, d1) = (dir(p[0], p[1]), dir(p[n - 2], p[n - 1]));
    let mut q: Vec<[f32; 3]> = Vec::with_capacity(n + 2);
    q.push([p[0][0] - d0[0] * w, p[0][1] - d0[1] * w, p[0][2]]);
    q.extend_from_slice(p);
    q.push([p[n - 1][0] + d1[0] * w, p[n - 1][1] + d1[1] * w, p[n - 1][2]]);
    let mut f = Forme::default();
    for i in 0..q.len() {
        let a = if i > 0 { dir(q[i - 1], q[i]) } else { dir(q[0], q[1]) };
        let b = if i + 1 < q.len() { dir(q[i], q[i + 1]) } else { a };
        let mut t = [a[0] + b[0], a[1] + b[1]];
        let l = t[0].hypot(t[1]);
        if l < 1e-6 {
            t = b;
        } else {
            t = [t[0] / l, t[1] / l];
        }
        // L'onglet : la demi-largeur tenue de part et d'autre du virage
        let c = (t[0] * b[0] + t[1] * b[1]).max(0.5);
        let (nx, nz) = (-t[1] * w / c, t[0] * w / c);
        let (g, d) = ([q[i][0] + nx, q[i][1] + nz], [q[i][0] - nx, q[i][1] - nz]);
        let (h, y0) = (haut(q[i][2]), bas(g, q[i], d));
        // Gauche bas, droite bas, droite haut, gauche haut
        f.sommets.push([g[0], y0, g[1]]);
        f.sommets.push([d[0], y0, d[1]]);
        f.sommets.push([d[0], h, d[1]]);
        f.sommets.push([g[0], h, g[1]]);
    }
    let m = q.len() as u32;
    for i in 0..m - 1 {
        let (a, b) = (4 * i, 4 * (i + 1));
        for k in 0..4 {
            let k1 = (k + 1) % 4;
            f.faces.push(vec![a + k, a + k1, b + k1]);
            f.faces.push(vec![a + k, b + k1, b + k]);
        }
    }
    let z = 4 * (m - 1);
    f.faces.push(vec![3, 2, 1]);
    f.faces.push(vec![3, 1, 0]);
    f.faces.push(vec![z, z + 1, z + 2]);
    f.faces.push(vec![z, z + 2, z + 3]);
    if f.volume() < 0.0 {
        f.faces.iter_mut().for_each(|p| p.reverse());
    }
    f
}

/// Les morceaux du tracé lissé de chaque route de `passage` qui touchent la boîte `b`
/// (marge comprise), avec leur demi-largeur.
fn traces(r: &Reseau, b: [f32; 4], passage: Passage) -> Vec<(Vec<[f32; 3]>, f32)> {
    let marge = 10.0;
    let mut v = Vec::new();
    for a in r.aretes.iter().filter(|a| a.passage == passage && a.genre != sage_reel::PIETONNE) {
        let w = (a.largeur * 0.5).max(1.0);
        let m = marge + 2.0 * w;
        let dedans = |p: [f32; 3], q: [f32; 3]| p[0].max(q[0]) >= b[0] - m && p[0].min(q[0]) <= b[2] + m && p[1].max(q[1]) >= b[1] - m && p[1].min(q[1]) <= b[3] + m;
        if !a.points.iter().any(|p| dedans(*p, *p)) && !a.points.windows(2).any(|s| dedans(s[0], s[1])) {
            continue;
        }
        let pts = lisser(&a.points);
        // Les suites de segments qui touchent la boîte
        let mut cur: Vec<[f32; 3]> = Vec::new();
        for s in pts.windows(2) {
            if dedans(s[0], s[1]) {
                if cur.is_empty() {
                    cur.push(s[0]);
                }
                cur.push(s[1]);
            } else if !cur.is_empty() {
                v.extend(morceaux(&std::mem::take(&mut cur)).into_iter().map(|c| (c, w)));
            }
        }
        if !cur.is_empty() {
            v.extend(morceaux(&cur).into_iter().map(|c| (c, w)));
        }
    }
    v
}

/// La tuile `cle` matérialisée (sommets, indices, texture) ; hors des tuiles assez
/// fines pour la chaussée (`chaussee::pour`), celle de la Maquette.
pub fn tuile(d: &Donnees, cle: Cle) -> (Vec<f32>, Vec<u32>, Vec<u8>) {
    if !crate::chaussee::pour(d, cle) {
        let (s, i, p, _) = mq::fabriquer_avec_defonce(d, cle);
        return (s, i, p);
    }
    let _t = crate::suivi::tache("matérialisation (tuile)");
    let r = d.reseau();
    let b = crate::carte::boite_tuile(d, cle);
    let def = mq::defonce(d, cle);
    let rem = mq::remblai_morceaux(d, cle);
    let epa = mq::epaulement_morceaux(d, cle);
    let terre = |p: [f32; 2]| d.lieu.relief(p[0] as f64, p[1] as f64) as f32;
    let (mut remblais, mut defonces) = (Vec::new(), Vec::new());
    for passage in [Passage::Sol, Passage::Pont, Passage::Tunnel] {
        for (t, w) in traces(&r, b, passage) {
            let rem = if passage == Passage::Sol {
                ruban(&t, w, |h| h, |g, c, d| terre(g).min(terre([c[0], c[1]])).min(terre(d)).min(c[2]) - SOUS)
            } else {
                ruban(&t, w, |h| h, |_, c, _| c[2] - OUVRAGE)
            };
            remblais.push(rem.en(mq::matiere::REMBLAI));
            defonces.push(chaussee(ruban(&t, w, |h| h + DEFONCE_HAUT, |_, c, _| c[2] - DEFONCE_SOUS)));
        }
    }
    let socle = mq::socle(d);
    let boite = pave([b[0], socle - 50.0, b[1]], [b[2], 2000.0, b[3]]);
    // Comme la Maquette (remblai des quais, épaulement, défonce des lits), puis les
    // remblais des voies unis, puis leurs défonces ôtées
    let mut etapes: Vec<(Op, &Forme)> = rem.iter().map(|m| (Op::Union, m)).collect();
    etapes.extend(epa.iter().map(|m| (Op::Difference, m)));
    etapes.push((Op::Difference, &def));
    etapes.extend(remblais.iter().filter(|f| !f.vide()).map(|f| (Op::Union, f)));
    etapes.extend(defonces.iter().filter(|f| !f.vide()).map(|f| (Op::Difference, f)));
    etapes.push((Op::Intersection, &boite));
    let solide = mq::solide(d, cle);
    let f = match crate::booleens::enchainer(&solide, &etapes) {
        Ok((f, _)) => f,
        Err(_) => solide,
    };
    let (mut v, mut ind) = mq::habiller(d, &f, b);
    for g in &defonces {
        let Ok(g) = crate::booleens::operation(g, &boite, Op::Intersection) else { continue };
        let (dv, di, _) = mq::maillage_defonce(&g);
        let base = (v.len() / crate::relief3d::SOMMET) as u32;
        v.extend_from_slice(&dv);
        ind.extend(di.iter().map(|k| k + base));
    }
    (v, ind, mq::texture(d, b))
}
