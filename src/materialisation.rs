//! La Matérialisation des Lieux (vue Matérialisation de la 3D) : les tuiles les plus
//! fines, fabriquées comme celles de la Maquette, puis, route par route (les piétonnes
//! ne sont ni remblayées ni défoncées) :
//! 1. l'union de son remblai : de la chaussée jusqu'à `SOUS` sous le terrain ; sur un
//!    pont (bretelle comprise) ou dans un tunnel, une dalle de `OUVRAGE` mètres ;
//! 2. sa défonce par le dessus : de la chaussée jusqu'à `DEFONCE_HAUT` au-dessus ; son
//!    fond est la chaussée (bitume), ses côtés et son dessus du béton clair.
//!
//! Avant les routes, les bâtiments qui touchent la tuile sont unis au terrain, en
//! volumes fermés enfoncés de `SOUBASSEMENT` sous le terrain (0.126) : la défonce d'un
//! tunnel recoupe leurs sous-sols. Ils sont en béton clair, en attendant leur matière.
//!
//! Chaque volume est un ruban le long du tracé de la route (`ruban`) : une section
//! horizontale en travers à chaque point du tracé, les sections reliées. Aucun triangle
//! ne relie deux routes : la chaussée est plate en travers par construction (0.120 ;
//! avant, le plan des voies fusionné puis triangulé donnait stries et dévers).
//! À chaque carrefour, une dalle (`dalle`) bouche les coins entre les rubans ; autour
//! du nœud, les routes sont posées sur le plan du carrefour (`plan`, `lisser`),
//! incliné comme la route principale : leurs volumes s'y recouvrent sans marche, et
//! une rue en pente coupée en arêtes garde sa pente (0.134 ; avant, 0.121 à 0.123,
//! des carrefours à plat : des paliers en escalier le long des rues en pente).
//! Les volumes de défonce, coupés à la tuile, sont joints en matière
//! `MAQUETTE_DEFONCE` : la 3D les montre avec la défonce des lits.

use crate::booleens::{Forme, Op};
use crate::carte::{Cle, Donnees};
use crate::maquette as mq;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
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
/// L'enfoncement des bâtiments dans la tuile (m) : leur soubassement descend d'autant
/// sous le point le plus bas du terrain le long de leur contour.
const SOUBASSEMENT: f32 = 3.0;

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

/// Un point d'un tracé prêt pour un ruban : x, z, la hauteur de son axe et la pente
/// en travers (gx, gz) : un bord à `o` de l'axe est à `h + g·o`.
pub(crate) type Pt = [f32; 5];

/// La hauteur d'un point à `o` (x, z) de l'axe d'un point de tracé.
fn hauteur_a(p: &Pt, o: [f32; 2]) -> f32 {
    p[2] + p[3] * (o[0] - p[0]) + p[4] * (o[1] - p[1])
}

/// Le plan d'un carrefour : il passe par le nœud `c` avec la pente `g` (celle de la
/// route principale, nulle en travers d'elle) ; les routes y sont posées sur `rayon`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Plan {
    c: [f32; 3],
    g: [f32; 2],
    rayon: f32,
}

impl Plan {
    fn h(&self, x: f32, z: f32) -> f32 {
        self.c[2] + self.g[0] * (x - self.c[0]) + self.g[1] * (z - self.c[1])
    }
}

/// La distance (m) à laquelle la pente d'une route est prise depuis un carrefour.
const PENTE_SUR: f32 = 12.0;
/// La plus forte pente d'un plan de carrefour.
const PENTE_PLAN_MAX: f32 = 0.15;

/// Le rang d'une route (le plus petit passe en premier) : grandes routes, routes et
/// bus, service, chemins, sentiers.
pub(crate) fn rang(genre: u8) -> u8 {
    match genre {
        sage_reel::GRANDE => 0,
        sage_reel::ROUTE | sage_reel::BUS => 1,
        sage_reel::SERVICE => 2,
        sage_reel::CHEMIN => 3,
        _ => 4,
    }
}

/// D'un nœud le long d'une arête : la direction (x, z) vers le point à `PENTE_SUR`
/// mètres (ou le bout), sa hauteur et la distance parcourue.
fn depuis_noeud(a: &sage_reel::reseau::Arete, n: usize) -> Option<([f32; 2], f32, f32)> {
    let pts: Vec<[f32; 3]> = if a.a == n { a.points.clone() } else { a.points.iter().rev().copied().collect() };
    let o = *pts.first()?;
    let mut s = 0.0;
    let mut q = o;
    for w in pts.windows(2) {
        let l = (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]);
        if s + l >= PENTE_SUR && l > 1e-6 {
            let t = (PENTE_SUR - s) / l;
            q = sage_reel::reseau::entre(w[0], w[1], t);
            s = PENTE_SUR;
            break;
        }
        s += l;
        q = w[1];
    }
    let l = (q[0] - o[0]).hypot(q[1] - o[1]);
    (l > 0.5).then(|| ([(q[0] - o[0]) / l, (q[1] - o[1]) / l], q[2], s))
}

/// Le plan du carrefour du nœud `n`, s'il réunit au moins deux routes : incliné comme
/// la route principale (rang, puis largeur), prise de part et d'autre du nœud si une
/// autre route la prolonge (la plus en face), sinon d'un seul côté ; plat en travers
/// d'elle. Son rayon : assez pour que les emprises (bouts prolongés compris) s'y
/// recouvrent toutes. Une rue qui se poursuit d'une arête à l'autre garde ainsi sa
/// pente, et les rues qui la croisent épousent son dévers (avant, 0.123, le carrefour
/// était à plat : des paliers, en escalier, le long des rues en pente).
pub(crate) fn plan(r: &Reseau, n: usize) -> Option<Plan> {
    let noeud = &r.noeuds[n];
    let routes: Vec<(&sage_reel::reseau::Arete, Option<([f32; 2], f32, f32)>)> = noeud.aretes.iter().map(|&k| &r.aretes[k]).filter(|a| a.genre != sage_reel::PIETONNE).map(|a| (a, depuis_noeud(a, n))).collect();
    if routes.len() < 2 {
        return None;
    }
    let w = routes.iter().map(|(a, _)| (a.largeur * 0.5).max(1.0)).fold(0.0, f32::max);
    let rayon = 1.5 * w + 1.0;
    let hn = noeud.p[2];
    let mut ordre: Vec<usize> = (0..routes.len()).filter(|&i| routes[i].1.is_some()).collect();
    ordre.sort_by(|&i, &j| rang(routes[i].0.genre).cmp(&rang(routes[j].0.genre)).then(routes[j].0.largeur.total_cmp(&routes[i].0.largeur)));
    let mut g = [0.0f32; 2];
    if let Some(&p) = ordre.first() {
        let (da, ha, la) = routes[p].1.unwrap();
        // Le prolongement : la route la plus en face, d'abord parmi celles de même rang
        let en_face = ordre[1..].iter().copied().filter(|&i| {
            let d = routes[i].1.unwrap().0;
            da[0] * d[0] + da[1] * d[1] < -0.5
        });
        let meilleur = en_face.min_by(|&i, &j| {
            let (di, dj) = (routes[i].1.unwrap().0, routes[j].1.unwrap().0);
            rang(routes[i].0.genre).cmp(&rang(routes[j].0.genre)).then((da[0] * di[0] + da[1] * di[1]).total_cmp(&(da[0] * dj[0] + da[1] * dj[1])))
        });
        let (t, pente) = match meilleur {
            Some(i) => {
                let (db, hb, lb) = routes[i].1.unwrap();
                let t = [db[0] - da[0], db[1] - da[1]];
                let l = t[0].hypot(t[1]).max(1e-6);
                ([t[0] / l, t[1] / l], (hb - ha) / (la + lb).max(1.0))
            }
            None => (da, (ha - hn) / la.max(1.0)),
        };
        let pente = pente.clamp(-PENTE_PLAN_MAX, PENTE_PLAN_MAX);
        g = [t[0] * pente, t[1] * pente];
    }
    Some(Plan { c: noeud.p, g, rayon })
}

/// Les points d'une voie (x, z, h et pente en travers), les hauteurs lissées le long
/// d'elle (moyenne pondérée en triangle sur ± `LISSAGE` mètres). Près d'un carrefour
/// (`pa` au début, `pb` à la fin, sur leur rayon), la voie est posée sur son plan :
/// toutes les routes d'un carrefour s'y recouvrent sans marche, en long comme en
/// travers ; elle rejoint son profil lissé sur les `LISSAGE` mètres suivants. Des
/// points sont ajoutés aux limites des parties posées.
pub(crate) fn lisser(p: &[[f32; 3]], pa: Option<Plan>, pb: Option<Plan>) -> Vec<Pt> {
    let abscisses = |p: &[[f32; 3]]| {
        let mut s = vec![0.0f32; p.len()];
        for i in 1..p.len() {
            s[i] = s[i - 1] + (p[i][0] - p[i - 1][0]).hypot(p[i][1] - p[i - 1][1]);
        }
        s
    };
    let s0 = abscisses(p);
    let long = s0[p.len() - 1];
    // Un bout sans carrefour : un plan plat à sa hauteur, de rayon nul
    let bout = |q: [f32; 3]| Plan { c: q, g: [0.0; 2], rayon: 0.0 };
    let pa = pa.unwrap_or(bout(p[0]));
    let pb = pb.unwrap_or(bout(p[p.len() - 1]));
    let (ra, rb) = (pa.rayon, pb.rayon);
    // Les hauteurs lissées aux points d'origine
    let lisse: Vec<f32> = (0..p.len())
        .map(|i| {
            let (mut somme, mut poids) = (0.0, 0.0);
            for j in 0..p.len() {
                let w = (1.0 - (s0[j] - s0[i]).abs() / LISSAGE).max(0.0);
                somme += w * p[j][2];
                poids += w;
            }
            somme / poids
        })
        .collect();
    // Les points, avec ceux des limites des parties posées
    let mut q: Vec<([f32; 3], f32)> = Vec::with_capacity(p.len() + 4);
    for i in 0..p.len() {
        if i > 0 {
            let mut coupes = [ra, ra + LISSAGE, long - rb - LISSAGE, long - rb];
            coupes.sort_by(f32::total_cmp);
            for c in coupes {
                if c > s0[i - 1] + 0.05 && c < s0[i] - 0.05 {
                    let t = (c - s0[i - 1]) / (s0[i] - s0[i - 1]);
                    let h = lisse[i - 1] + t * (lisse[i] - lisse[i - 1]);
                    q.push(([p[i - 1][0] + t * (p[i][0] - p[i - 1][0]), p[i - 1][1] + t * (p[i][1] - p[i - 1][1]), h], c));
                }
            }
        }
        q.push(([p[i][0], p[i][1], lisse[i]], s0[i]));
    }
    q.into_iter()
        .map(|(x, s)| {
            let (ha, hb) = (pa.h(x[0], x[1]), pb.h(x[0], x[1]));
            let (h, g) = if ra + rb >= long {
                // Trop courte : d'un plan à l'autre
                let f = s / long.max(1e-6);
                (ha + (hb - ha) * f, [pa.g[0] + (pb.g[0] - pa.g[0]) * f, pa.g[1] + (pb.g[1] - pa.g[1]) * f])
            } else if s <= ra {
                (ha, pa.g)
            } else if s >= long - rb {
                (hb, pb.g)
            } else {
                let fa = ((s - ra) / LISSAGE).clamp(0.0, 1.0);
                let fb = ((long - rb - s) / LISSAGE).clamp(0.0, 1.0);
                // Du plan vers le lissé, par le plus proche des deux bouts
                let (f, hn, gn) = if s - ra < long - rb - s { (fa, ha, pa.g) } else { (fb, hb, pb.g) };
                (hn * (1.0 - f) + f * x[2], [gn[0] * (1.0 - f), gn[1] * (1.0 - f)])
            };
            [x[0], x[1], h, g[0], g[1]]
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
fn morceaux(p: &[Pt]) -> Vec<Vec<Pt>> {
    let mut v: Vec<Pt> = Vec::with_capacity(p.len());
    for &q in p {
        if v.last().is_none_or(|l: &Pt| (q[0] - l[0]).hypot(q[1] - l[1]) > CONFONDUS) {
            v.push(q);
        }
    }
    let dir = |a: Pt, b: Pt| {
        let l = (b[0] - a[0]).hypot(b[1] - a[1]).max(1e-6);
        [(b[0] - a[0]) / l, (b[1] - a[1]) / l]
    };
    let mut out = Vec::new();
    let mut cur: Vec<Pt> = Vec::new();
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

/// Un tracé rééchantillonné : un point au plus tous les `pas` mètres.
fn reechantillonner(p: &[Pt], pas: f32) -> Vec<Pt> {
    let mut v = Vec::with_capacity(p.len() * 2);
    v.push(p[0]);
    for s in p.windows(2) {
        let d = (s[1][0] - s[0][0]).hypot(s[1][1] - s[0][1]);
        let n = (d / pas).ceil().max(1.0) as usize;
        for k in 1..=n {
            let t = k as f32 / n as f32;
            v.push(std::array::from_fn(|c| s[0][c] + t * (s[1][c] - s[0][c])));
        }
    }
    v
}

/// L'écart (m) entre deux sections d'un ruban posé sur le champ de la voirie.
const PAS_RUBAN: f32 = 2.0;

/// Le ruban d'un morceau de tracé de demi-largeur `w` : à chaque point une section en
/// travers (bords en onglet) à trois points, bord gauche, axe et bord droit, en haut
/// comme en bas. Leurs hauteurs : le champ de la voirie (`champ`) s'il y en a,
/// sinon la pente du point (`Pt`). La section va de `bas(gauche, droite, axe,
/// [h gauche, h axe, h droite])` à `haut(h)` de chacun ; les deux bouts sont
/// prolongés de `w` (ils couvrent le carrefour).
fn ruban(p: &[Pt], w: f32, champ: Option<&crate::champ_voirie::Champ>, haut: impl Fn(f32) -> f32, bas: impl Fn([f32; 2], [f32; 2], [f32; 2], [f32; 3]) -> [f32; 3]) -> Forme {
    let n = p.len();
    let dir = |a: Pt, b: Pt| {
        let l = (b[0] - a[0]).hypot(b[1] - a[1]).max(1e-6);
        [(b[0] - a[0]) / l, (b[1] - a[1]) / l]
    };
    let hauteur = |q: &Pt, o: [f32; 2]| champ.and_then(|c| c.h(o[0], o[1])).unwrap_or_else(|| hauteur_a(q, o));
    // Les points, bouts prolongés (sur la même pente)
    let (d0, d1) = (dir(p[0], p[1]), dir(p[n - 2], p[n - 1]));
    let prolonge = |a: &Pt, d: [f32; 2]| {
        let o = [a[0] + d[0], a[1] + d[1]];
        [o[0], o[1], hauteur_a(a, o), a[3], a[4]]
    };
    let mut q: Vec<Pt> = Vec::with_capacity(n + 2);
    q.push(prolonge(&p[0], [-d0[0] * w, -d0[1] * w]));
    q.extend_from_slice(p);
    q.push(prolonge(&p[n - 1], [d1[0] * w, d1[1] * w]));
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
        let (g, m, d) = ([q[i][0] + nx, q[i][1] + nz], [q[i][0], q[i][1]], [q[i][0] - nx, q[i][1] - nz]);
        let hs = [hauteur(&q[i], g), hauteur(&q[i], m), hauteur(&q[i], d)];
        let bs = bas(g, d, m, hs);
        // Bas : gauche, axe, droite ; haut : droite, axe, gauche
        f.sommets.push([g[0], bs[0], g[1]]);
        f.sommets.push([m[0], bs[1], m[1]]);
        f.sommets.push([d[0], bs[2], d[1]]);
        f.sommets.push([d[0], haut(hs[2]), d[1]]);
        f.sommets.push([m[0], haut(hs[1]), m[1]]);
        f.sommets.push([g[0], haut(hs[0]), g[1]]);
    }
    const S: u32 = 6;
    let m = q.len() as u32;
    for i in 0..m - 1 {
        let (a, b) = (S * i, S * (i + 1));
        for k in 0..S {
            let k1 = (k + 1) % S;
            f.faces.push(vec![a + k, a + k1, b + k1]);
            f.faces.push(vec![a + k, b + k1, b + k]);
        }
    }
    // Les bouts : en éventail depuis l'axe du bas (jamais trois points alignés)
    let z = S * (m - 1);
    for [u, v, x] in [[1, 2, 3], [1, 3, 4], [1, 4, 5], [1, 5, 0]] {
        f.faces.push(vec![u, x, v]);
        f.faces.push(vec![z + u, z + v, z + x]);
    }
    if f.volume() < 0.0 {
        f.faces.iter_mut().for_each(|p| p.reverse());
    }
    f
}

/// Une dalle de carrefour : un octogone de rayon inscrit `w` centré en `c` (x, z), de
/// `bas(point)` à `haut(point)` (le champ de la voirie ou le plan du carrefour), aux
/// sommets et au centre.
fn dalle(c: [f32; 2], w: f32, haut: impl Fn([f32; 2]) -> f32, bas: impl Fn([f32; 2]) -> f32) -> Forme {
    const COTES: usize = 8;
    let r = w / (std::f32::consts::PI / COTES as f32).cos();
    let mut f = Forme::default();
    for k in 0..COTES {
        let t = k as f32 * std::f32::consts::TAU / COTES as f32;
        let q = [c[0] + r * t.cos(), c[1] + r * t.sin()];
        f.sommets.push([q[0], bas(q), q[1]]);
        f.sommets.push([q[0], haut(q), q[1]]);
    }
    let n = COTES as u32;
    let (cb, ch) = (2 * n, 2 * n + 1);
    f.sommets.push([c[0], bas(c), c[1]]);
    f.sommets.push([c[0], haut(c), c[1]]);
    for k in 0..n {
        let k1 = (k + 1) % n;
        f.faces.push(vec![2 * k, 2 * k1, 2 * k1 + 1]);
        f.faces.push(vec![2 * k, 2 * k1 + 1, 2 * k + 1]);
        f.faces.push(vec![cb, 2 * k1, 2 * k]);
        f.faces.push(vec![ch, 2 * k + 1, 2 * k1 + 1]);
    }
    if f.volume() < 0.0 {
        f.faces.iter_mut().for_each(|p| p.reverse());
    }
    f
}

/// Les carrefours près de la boîte `b` : les nœuds où se rejoignent au moins deux
/// routes, avec leur plan, la plus grande demi-largeur et s'il y arrive une route au
/// sol.
fn carrefours(r: &Reseau, b: [f32; 4]) -> Vec<(Plan, f32, bool)> {
    let marge = 20.0;
    let mut v = Vec::new();
    for (k, n) in r.noeuds.iter().enumerate() {
        let p = n.p;
        if p[0] < b[0] - marge || p[0] > b[2] + marge || p[1] < b[1] - marge || p[1] > b[3] + marge {
            continue;
        }
        let Some(pl) = plan(r, k) else { continue };
        let routes: Vec<_> = n.aretes.iter().map(|&k| &r.aretes[k]).filter(|a| a.genre != sage_reel::PIETONNE).collect();
        let w = routes.iter().map(|a| (a.largeur * 0.5).max(1.0)).fold(0.0, f32::max);
        v.push((pl, w, routes.iter().any(|a| a.passage == Passage::Sol)));
    }
    v
}

/// Les morceaux du tracé lissé de chaque route de `passage` qui touchent la boîte `b`
/// (marge comprise), avec leur demi-largeur.
fn traces(r: &Reseau, b: [f32; 4], passage: Passage, memoire: Option<&Memoire>) -> Vec<(Vec<Pt>, f32)> {
    let marge = 10.0;
    let mut v = Vec::new();
    for (k, a) in r.aretes.iter().enumerate().filter(|(_, a)| a.passage == passage && a.genre != sage_reel::PIETONNE) {
        let w = (a.largeur * 0.5).max(1.0);
        let m = marge + 2.0 * w;
        let dedans = |p: &[f32], q: &[f32]| p[0].max(q[0]) >= b[0] - m && p[0].min(q[0]) <= b[2] + m && p[1].max(q[1]) >= b[1] - m && p[1].min(q[1]) <= b[3] + m;
        if !a.points.iter().any(|p| dedans(p, p)) && !a.points.windows(2).any(|s| dedans(&s[0], &s[1])) {
            continue;
        }
        let lisse = || lisser(&a.points, plan(r, a.a), plan(r, a.b));
        let pts: Arc<Vec<Pt>> = match memoire.and_then(|m| m.traces.lock().ok()) {
            Some(mut t) => t.entry(k).or_insert_with(|| Arc::new(lisse())).clone(),
            None => Arc::new(lisse()),
        };
        // Les suites de segments qui touchent la boîte
        let mut cur: Vec<Pt> = Vec::new();
        for s in pts.windows(2) {
            if dedans(&s[0], &s[1]) {
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

/// Ce qu'une génération de carte (`generer_carte`) garde d'une tuile à l'autre : les
/// volumes fermés des bâtiments (un bâtiment à cheval sur plusieurs tuiles n'est monté
/// qu'une fois, puis oublié après la dernière tuile qui le touche) et les tracés lissés
/// des routes.
#[derive(Default)]
pub struct Memoire {
    /// Par bâtiment : son volume (rien s'il est refusé) et les tuiles qui le
    /// demanderont encore.
    batiments: Mutex<HashMap<u32, (Option<Arc<Forme>>, u32)>>,
    /// Les tuiles qui touchent chaque bâtiment, parmi celles de la génération (moins
    /// celles faites ailleurs, par le renfort).
    touches: Mutex<HashMap<u32, u32>>,
    /// Par arête du réseau : son tracé lissé.
    traces: Mutex<HashMap<usize, Arc<Vec<Pt>>>>,
    /// Le temps passé par étape.
    mesures: Mutex<Mesures>,
}

/// Le temps passé par étape d'une génération : nom, secondes, nombre de fois (dans
/// l'ordre de première apparition) ; les tuiles faites, dont les simples (sans route,
/// bâtiment, quai ni lit : aucun booléen utile).
#[derive(Clone, Debug, Default)]
pub struct Mesures {
    pub etapes: Vec<(&'static str, f64, u32)>,
    pub tuiles: u32,
    pub simples: u32,
}

impl Mesures {
    pub fn ajouter(&mut self, nom: &'static str, s: f64) {
        match self.etapes.iter_mut().find(|e| e.0 == nom) {
            Some(e) => {
                e.1 += s;
                e.2 += 1;
            }
            None => self.etapes.push((nom, s, 1)),
        }
    }

    pub fn total(&self) -> f64 {
        self.etapes.iter().map(|e| e.1).sum()
    }

    /// Le bilan en texte, une étape par ligne, la plus longue d'abord.
    pub fn texte(&self) -> String {
        let total = self.total().max(1e-9);
        let mut v = self.etapes.clone();
        v.sort_by(|a, b| b.1.total_cmp(&a.1));
        let mut t = format!("{} tuiles, dont {} simples ; {:.1} s mesurées\n", self.tuiles, self.simples, total);
        for (nom, s, n) in v {
            t += &format!("{nom} : {s:.1} s ({:.0} %), {n} fois, {:.1} ms en moyenne\n", 100.0 * s / total, 1000.0 * s / n.max(1) as f64);
        }
        t
    }
}

impl Memoire {
    /// La mémoire d'une génération de ces tuiles.
    pub fn pour(d: &Donnees, cles: &[Cle]) -> Memoire {
        let mut touches: HashMap<u32, u32> = HashMap::new();
        for &cle in cles {
            if crate::chaussee::pour(d, cle) {
                for k in d.batiments_dans(crate::carte::boite_tuile(d, cle)) {
                    *touches.entry(k).or_default() += 1;
                }
            }
        }
        Memoire { touches: Mutex::new(touches), ..Default::default() }
    }

    /// La tuile `cle` a été faite ailleurs (par le renfort) : ses bâtiments ne
    /// l'attendent plus ici.
    pub fn faite_ailleurs(&self, d: &Donnees, cle: Cle) {
        let (Ok(mut b), Ok(mut t)) = (self.batiments.lock(), self.touches.lock()) else { return };
        for k in d.batiments_dans(crate::carte::boite_tuile(d, cle)) {
            if let Some(n) = t.get_mut(&k) {
                *n = n.saturating_sub(1);
            }
            if let Some(e) = b.get_mut(&k) {
                e.1 = e.1.saturating_sub(1);
                if e.1 == 0 {
                    b.remove(&k);
                }
            }
        }
    }

    /// Note `s` secondes passées à l'étape `nom`.
    pub fn noter(&self, nom: &'static str, s: f64) {
        if let Ok(mut m) = self.mesures.lock() {
            m.ajouter(nom, s);
        }
    }

    /// Le temps passé par étape jusqu'ici.
    pub fn mesures(&self) -> Mesures {
        self.mesures.lock().map(|m| m.clone()).unwrap_or_default()
    }
}

/// Le volume fermé du bâtiment `k`, enfoncé de `SOUBASSEMENT` sous le point le plus bas
/// du terrain sous son contour.
fn volume_batiment(d: &Donnees, k: u32) -> Option<Forme> {
    let bt = &d.lieu.batiments[k as usize];
    if bt.contour.len() < 3 {
        return None;
    }
    let bas = bt.contour.iter().map(|&p| d.lieu.relief(p[0] as f64, p[1] as f64) as f32).fold(f32::MAX, f32::min);
    let mut bt = bt.clone();
    bt.pied = bas - SOUBASSEMENT;
    crate::bati_ferme::volume_accepte(&bt).map(|(f, _)| f.en(mq::matiere::BETON))
}

/// Le volume du bâtiment `k`, pris dans la mémoire s'il y en a une.
fn batiment(d: &Donnees, k: u32, memoire: Option<&Memoire>) -> Option<Arc<Forme>> {
    let Some(m) = memoire else { return volume_batiment(d, k).map(Arc::new) };
    let Ok(mut b) = m.batiments.lock() else { return volume_batiment(d, k).map(Arc::new) };
    let reste = m.touches.lock().ok().and_then(|t| t.get(&k).copied()).unwrap_or(1).max(1);
    let e = b.entry(k).or_insert_with(|| (volume_batiment(d, k).map(Arc::new), reste));
    let f = e.0.clone();
    e.1 = e.1.saturating_sub(1);
    if e.1 == 0 {
        b.remove(&k);
    }
    f
}

/// La tuile `cle` matérialisée (sommets, indices, texture) ; hors des tuiles assez
/// fines pour la chaussée (`chaussee::pour`), celle de la Maquette.
pub fn tuile(d: &Donnees, cle: Cle) -> (Vec<f32>, Vec<u32>, Vec<u8>) {
    faire(d, cle, true, None)
}

/// La tuile `cle` pour une carte (`generer_carte`) : sans les volumes de défonce
/// montrés à part (Flow seul les dessine), avec la mémoire de la génération.
pub fn tuile_carte(d: &Donnees, cle: Cle, memoire: &Memoire) -> (Vec<f32>, Vec<u32>, Vec<u8>) {
    faire(d, cle, false, Some(memoire))
}

fn faire(d: &Donnees, cle: Cle, defonces_a_part: bool, memoire: Option<&Memoire>) -> (Vec<f32>, Vec<u32>, Vec<u8>) {
    // Le chronomètre des étapes, pour une génération (`Memoire::mesures`)
    let t0 = std::cell::Cell::new(std::time::Instant::now());
    let noter = |nom: &'static str| {
        if let Some(m) = memoire {
            m.noter(nom, t0.get().elapsed().as_secs_f64());
        }
        t0.set(std::time::Instant::now());
    };
    if !crate::chaussee::pour(d, cle) {
        let (s, i, p, _) = mq::fabriquer_avec_defonce(d, cle);
        noter("tuile de la Maquette (pas de chaussée)");
        return (s, i, p);
    }
    let _t = crate::suivi::tache("matérialisation (tuile)");
    let r = d.reseau();
    noter("réseau nivelé (une fois)");
    let b = crate::carte::boite_tuile(d, cle);
    let def = mq::defonce(d, cle);
    let rem = mq::remblai_morceaux(d, cle);
    let epa = mq::epaulement_morceaux(d, cle);
    noter("volumes des quais et des lits");
    let terre = |p: [f32; 2]| d.lieu.relief(p[0] as f64, p[1] as f64) as f32;
    let champ = d.champ_voirie();
    noter("champ de hauteur de la voirie (une fois)");
    let (mut remblais, mut defonces) = (Vec::new(), Vec::new());
    for passage in [Passage::Sol, Passage::Pont, Passage::Tunnel] {
        // Au sol, les rubans sont posés sur le champ de la voirie, une section tous
        // les PAS_RUBAN mètres ; les ponts et les tunnels gardent leur profil
        let sol = passage == Passage::Sol;
        let ch = if sol { Some(&*champ) } else { None };
        for (t, w) in traces(&r, b, passage, memoire) {
            let t = if sol { reechantillonner(&t, PAS_RUBAN) } else { t };
            let rem = if sol {
                ruban(&t, w, ch, |h| h, |g, d, c, hs| {
                    let m = terre(g).min(terre(c)).min(terre(d)).min(hs[0]).min(hs[1]).min(hs[2]) - SOUS;
                    [m; 3]
                })
            } else {
                ruban(&t, w, ch, |h| h, |_, _, _, hs| hs.map(|h| h - OUVRAGE))
            };
            remblais.push(rem.en(mq::matiere::REMBLAI));
            defonces.push(chaussee(ruban(&t, w, ch, |h| h + DEFONCE_HAUT, |_, _, _, hs| hs.map(|h| h - DEFONCE_SOUS))));
        }
    }
    // Les dalles des carrefours : elles bouchent les coins entre les rubans
    for (pl, w, sol) in carrefours(&r, b) {
        let c = [pl.c[0], pl.c[1]];
        let h = |q: [f32; 2]| if sol { champ.h(q[0], q[1]) } else { None }.unwrap_or_else(|| pl.h(q[0], q[1]));
        let rem = if sol { dalle(c, w, h, |q| terre(q).min(terre(c)).min(h(q)) - SOUS) } else { dalle(c, w, h, |q| h(q) - OUVRAGE) };
        remblais.push(rem.en(mq::matiere::REMBLAI));
        defonces.push(chaussee(dalle(c, w, |q| h(q) + DEFONCE_HAUT, |q| h(q) - DEFONCE_SOUS)));
    }
    noter("rubans et dalles des routes");
    // Les bâtiments qui touchent la tuile, en volumes fermés (`bati_ferme`), enfoncés de
    // `SOUBASSEMENT` : unis avant les routes, dont les défonces (un tunnel) recoupent
    // alors leurs sous-sols
    let batiments: Vec<Arc<Forme>> = {
        let _t = crate::suivi::tache("bâtiments");
        d.batiments_dans(b).into_iter().filter_map(|k| batiment(d, k, memoire)).collect()
    };
    noter("volumes des bâtiments");
    let socle = mq::socle(d);
    let boite = pave([b[0], socle - 50.0, b[1]], [b[2], 2000.0, b[3]]);
    // Comme la Maquette (remblai des quais, épaulement, défonce des lits), puis les
    // remblais des voies unis, puis leurs défonces ôtées
    // Les quais une à une (unions et différences mêlées) ; bâtiments, remblais et
    // défonces des routes unis d'abord entre eux en arbre, puis appliqués d'un coup
    // (`booleens::enchainer_familles`)
    use crate::booleens::{Famille, Temps};
    let mut familles: Vec<Famille> = rem.iter().map(|m| Famille { op: Op::Union, formes: vec![m], arbre: false }).collect();
    familles.extend(epa.iter().map(|m| Famille { op: Op::Difference, formes: vec![m], arbre: false }));
    familles.push(Famille { op: Op::Difference, formes: vec![&def], arbre: false });
    let n_quais = familles.len();
    familles.push(Famille { op: Op::Union, formes: batiments.iter().map(|f| &**f).collect(), arbre: true });
    familles.push(Famille { op: Op::Union, formes: remblais.iter().filter(|f| !f.vide()).collect(), arbre: true });
    familles.push(Famille { op: Op::Difference, formes: defonces.iter().filter(|f| !f.vide()).collect(), arbre: true });
    familles.push(Famille { op: Op::Intersection, formes: vec![&boite], arbre: false });
    let simple = batiments.is_empty() && remblais.iter().all(|f| f.vide()) && rem.iter().all(|f| f.vide()) && epa.iter().all(|f| f.vide()) && def.vide();
    if let Some(m) = memoire {
        if let Ok(mut m) = m.mesures.lock() {
            m.tuiles += 1;
            m.simples += simple as u32;
        }
    }
    let solide = mq::solide(d, cle);
    noter("terrain (solide de la tuile)");
    let f = match crate::booleens::enchainer_familles(&solide, &familles, |t, s| {
        if let Some(m) = memoire {
            let famille = |k: usize| match k.checked_sub(n_quais) {
                None => 0,
                Some(j) => j + 1,
            };
            let nom = match t {
                Temps::Conversions => "booléens : conversions",
                Temps::Regroupement(k) => ["booléens : quais et lits", "booléens : bâtiments, unis entre eux", "booléens : remblais des routes, unis entre eux", "booléens : défonces des routes, unies entre elles", "booléens : découpe à la tuile"][famille(k).min(4)],
                Temps::Application(k) => ["booléens : quais et lits", "booléens : bâtiments, union à la tuile", "booléens : remblais des routes, union à la tuile", "booléens : défonces des routes, différence à la tuile", "booléens : découpe à la tuile"][famille(k).min(4)],
            };
            m.noter(nom, s);
        }
    }) {
        Ok((f, _)) => f,
        Err(_) => solide,
    };
    t0.set(std::time::Instant::now());
    let (mut v, mut ind) = mq::habiller(d, &f, b);
    noter("habillage (maillage de la forme)");
    for g in defonces.iter().filter(|_| defonces_a_part) {
        let Ok(g) = crate::booleens::operation(g, &boite, Op::Intersection) else { continue };
        let (dv, di, _) = mq::maillage_defonce(&g);
        let base = (v.len() / crate::relief3d::SOMMET) as u32;
        v.extend_from_slice(&dv);
        ind.extend(di.iter().map(|k| k + base));
    }
    let tex = mq::texture(d, b);
    noter("texture du sol");
    (v, ind, tex)
}
