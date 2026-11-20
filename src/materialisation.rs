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
//! À chaque carrefour, une dalle horizontale à la hauteur du nœud (`dalle`) bouche
//! les coins entre les rubans ; autour du nœud, sur `rayon_carrefour`, les routes
//! sont à plat à sa hauteur (`lisser`) : leurs volumes s'y recouvrent sans marche
//! (0.121, 0.123).
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

/// Les points (x, z, h) d'une voie, les hauteurs lissées le long d'elle (moyenne
/// pondérée en triangle sur ± `LISSAGE` mètres). Sur `ra` mètres depuis le début et
/// `rb` depuis la fin (les carrefours, `rayon_carrefour`), la voie est à plat à la
/// hauteur de son nœud : toutes les routes d'un carrefour s'y recouvrent à la même
/// hauteur (pas de marche) ; le lissage s'installe sur les `LISSAGE` mètres suivants.
/// Des points sont ajoutés aux limites des parties à plat.
fn lisser(p: &[[f32; 3]], ra: f32, rb: f32) -> Vec<[f32; 3]> {
    let abscisses = |p: &[[f32; 3]]| {
        let mut s = vec![0.0f32; p.len()];
        for i in 1..p.len() {
            s[i] = s[i - 1] + (p[i][0] - p[i - 1][0]).hypot(p[i][1] - p[i - 1][1]);
        }
        s
    };
    let s0 = abscisses(p);
    let long = s0[p.len() - 1];
    let (ha, hb) = (p[0][2], p[p.len() - 1][2]);
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
    // Les points, avec ceux des limites des parties à plat
    let mut q: Vec<([f32; 3], f32)> = Vec::with_capacity(p.len() + 2);
    for i in 0..p.len() {
        if i > 0 {
            for c in [ra, long - rb] {
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
            let h = if ra + rb >= long {
                // Trop courte : d'un nœud à l'autre en ligne droite
                ha + (hb - ha) * (s / long.max(1e-6))
            } else if s <= ra {
                ha
            } else if s >= long - rb {
                hb
            } else {
                let fa = ((s - ra) / LISSAGE).clamp(0.0, 1.0);
                let fb = ((long - rb - s) / LISSAGE).clamp(0.0, 1.0);
                // Du nœud vers le lissé, par le plus proche des deux bouts
                let (f, hn) = if s - ra < long - rb - s { (fa, ha) } else { (fb, hb) };
                hn * (1.0 - f) + f * x[2]
            };
            [x[0], x[1], h]
        })
        .collect()
}

/// Le rayon (m) de la partie à plat d'un carrefour : le nœud `n` s'il réunit au moins
/// deux routes, assez pour que leurs emprises (bouts prolongés compris) s'y recouvrent
/// toutes ; 0 sinon.
fn rayon_carrefour(r: &Reseau, n: usize) -> f32 {
    let routes: Vec<_> = r.noeuds[n].aretes.iter().map(|&k| &r.aretes[k]).filter(|a| a.genre != sage_reel::PIETONNE).collect();
    if routes.len() < 2 {
        return 0.0;
    }
    let w = routes.iter().map(|a| (a.largeur * 0.5).max(1.0)).fold(0.0, f32::max);
    1.5 * w + 1.0
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

/// Une dalle de carrefour : un octogone horizontal de rayon inscrit `w` centré en `c`
/// (x, z), de `bas(sommet)` à `haut`.
fn dalle(c: [f32; 2], w: f32, haut: f32, bas: impl Fn([f32; 2]) -> f32) -> Forme {
    const COTES: usize = 8;
    let r = w / (std::f32::consts::PI / COTES as f32).cos();
    let mut f = Forme::default();
    for k in 0..COTES {
        let t = k as f32 * std::f32::consts::TAU / COTES as f32;
        let q = [c[0] + r * t.cos(), c[1] + r * t.sin()];
        f.sommets.push([q[0], bas(q), q[1]]);
        f.sommets.push([q[0], haut, q[1]]);
    }
    let n = COTES as u32;
    for k in 0..n {
        let k1 = (k + 1) % n;
        f.faces.push(vec![2 * k, 2 * k1, 2 * k1 + 1]);
        f.faces.push(vec![2 * k, 2 * k1 + 1, 2 * k + 1]);
    }
    for k in 1..n - 1 {
        f.faces.push(vec![0, 2 * (k + 1), 2 * k]);
        f.faces.push(vec![1, 2 * k + 1, 2 * (k + 1) + 1]);
    }
    if f.volume() < 0.0 {
        f.faces.iter_mut().for_each(|p| p.reverse());
    }
    f
}

/// Les carrefours près de la boîte `b` : les nœuds où se rejoignent au moins deux
/// routes, avec la plus grande demi-largeur et s'il y arrive une route au sol.
fn carrefours(r: &Reseau, b: [f32; 4]) -> Vec<([f32; 3], f32, bool)> {
    let marge = 20.0;
    let mut v = Vec::new();
    for n in &r.noeuds {
        let p = n.p;
        if p[0] < b[0] - marge || p[0] > b[2] + marge || p[1] < b[1] - marge || p[1] > b[3] + marge {
            continue;
        }
        let routes: Vec<_> = n.aretes.iter().map(|&k| &r.aretes[k]).filter(|a| a.genre != sage_reel::PIETONNE).collect();
        if routes.len() < 2 {
            continue;
        }
        let w = routes.iter().map(|a| (a.largeur * 0.5).max(1.0)).fold(0.0, f32::max);
        v.push((p, w, routes.iter().any(|a| a.passage == Passage::Sol)));
    }
    v
}

/// Les morceaux du tracé lissé de chaque route de `passage` qui touchent la boîte `b`
/// (marge comprise), avec leur demi-largeur.
fn traces(r: &Reseau, b: [f32; 4], passage: Passage, memoire: Option<&Memoire>) -> Vec<(Vec<[f32; 3]>, f32)> {
    let marge = 10.0;
    let mut v = Vec::new();
    for (k, a) in r.aretes.iter().enumerate().filter(|(_, a)| a.passage == passage && a.genre != sage_reel::PIETONNE) {
        let w = (a.largeur * 0.5).max(1.0);
        let m = marge + 2.0 * w;
        let dedans = |p: [f32; 3], q: [f32; 3]| p[0].max(q[0]) >= b[0] - m && p[0].min(q[0]) <= b[2] + m && p[1].max(q[1]) >= b[1] - m && p[1].min(q[1]) <= b[3] + m;
        if !a.points.iter().any(|p| dedans(*p, *p)) && !a.points.windows(2).any(|s| dedans(s[0], s[1])) {
            continue;
        }
        let lisse = || lisser(&a.points, rayon_carrefour(r, a.a), rayon_carrefour(r, a.b));
        let pts: Arc<Vec<[f32; 3]>> = match memoire.and_then(|m| m.traces.lock().ok()) {
            Some(mut t) => t.entry(k).or_insert_with(|| Arc::new(lisse())).clone(),
            None => Arc::new(lisse()),
        };
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

/// Ce qu'une génération de carte (`generer_carte`) garde d'une tuile à l'autre : les
/// volumes fermés des bâtiments (un bâtiment à cheval sur plusieurs tuiles n'est monté
/// qu'une fois, puis oublié après la dernière tuile qui le touche) et les tracés lissés
/// des routes.
#[derive(Default)]
pub struct Memoire {
    /// Par bâtiment : son volume (rien s'il est refusé) et les tuiles qui le
    /// demanderont encore.
    batiments: Mutex<HashMap<u32, (Option<Arc<Forme>>, u32)>>,
    /// Les tuiles qui touchent chaque bâtiment, parmi celles de la génération.
    touches: HashMap<u32, u32>,
    /// Par arête du réseau : son tracé lissé.
    traces: Mutex<HashMap<usize, Arc<Vec<[f32; 3]>>>>,
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
        Memoire { touches, ..Default::default() }
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
    let e = b.entry(k).or_insert_with(|| (volume_batiment(d, k).map(Arc::new), m.touches.get(&k).copied().unwrap_or(1)));
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
    let (mut remblais, mut defonces) = (Vec::new(), Vec::new());
    for passage in [Passage::Sol, Passage::Pont, Passage::Tunnel] {
        for (t, w) in traces(&r, b, passage, memoire) {
            let rem = if passage == Passage::Sol {
                ruban(&t, w, |h| h, |g, c, d| terre(g).min(terre([c[0], c[1]])).min(terre(d)).min(c[2]) - SOUS)
            } else {
                ruban(&t, w, |h| h, |_, c, _| c[2] - OUVRAGE)
            };
            remblais.push(rem.en(mq::matiere::REMBLAI));
            defonces.push(chaussee(ruban(&t, w, |h| h + DEFONCE_HAUT, |_, c, _| c[2] - DEFONCE_SOUS)));
        }
    }
    // Les dalles des carrefours : elles bouchent les coins entre les rubans
    for (p, w, sol) in carrefours(&r, b) {
        let (c, h) = ([p[0], p[1]], p[2]);
        let rem = if sol { dalle(c, w, h, |q| terre(q).min(terre(c)).min(h) - SOUS) } else { dalle(c, w, h, |_| h - OUVRAGE) };
        remblais.push(rem.en(mq::matiere::REMBLAI));
        defonces.push(chaussee(dalle(c, w, h + DEFONCE_HAUT, |_| h - DEFONCE_SOUS)));
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
