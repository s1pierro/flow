//! Les modifieurs des objets de Mailles, repris de Rustcraft et enrichis : une pile par
//! objet, appliquée dans l'ordre à sa forme sans la changer.
//!
//! - **Booléen** : avec un ou plusieurs autres objets, les **cibles** (désignées par leur
//!   identifiant), l'une après l'autre : union, différence ou intersection.
//! - **Répétition** : des copies décalées (d'une longueur, ou d'une fraction de la taille
//!   de la forme), éventuellement unies.
//! - **Miroir** : la symétrie par les plans de l'objet (X, Y, Z, au choix et cumulés),
//!   les sommets posés sur le plan soudés.
//!
//! `evaluer_tout` calcule toutes les piles (une cible se calcule avec sa propre pile ; une
//! boucle de cibles est dite, pas suivie). `appliquer` fige un modifieur, `couper` coupe
//! un objet en deux par les cibles d'un booléen.

use crate::booleens::{self, Forme, Op};
use crate::glb;
use crate::maille::Objet;
use crate::sculpture::V3;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq)]
pub enum Genre {
    Booleen { op: Op, cibles: Vec<u64> },
    Repetition { n: u32, relatif: bool, decalage: V3, unir: bool },
    Miroir { axes: [bool; 3], souder: bool },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Modifieur {
    pub genre: Genre,
    pub actif: bool,
}

/// Les modifieurs qu'on peut ajouter : clé, nom, à quoi il sert.
pub const GENRES: [(&str, &str, &str); 3] = [
    ("booleen", "Booléen", "Unir, creuser ou garder le commun avec d'autres objets"),
    ("repetition", "Répétition", "Des copies décalées, unies ou non"),
    ("miroir", "Miroir", "La symétrie par les plans de l'objet"),
];

impl Modifieur {
    /// Un modifieur neuf, aux réglages par défaut (ceux de Rustcraft pour les deux
    /// premiers).
    pub fn neuf(cle: &str) -> Option<Modifieur> {
        let genre = match cle {
            "booleen" => Genre::Booleen { op: Op::Difference, cibles: Vec::new() },
            "repetition" => Genre::Repetition { n: 3, relatif: true, decalage: [1.2, 0.0, 0.0], unir: false },
            "miroir" => Genre::Miroir { axes: [true, false, false], souder: true },
            _ => return None,
        };
        Some(Modifieur { genre, actif: true })
    }

    pub fn nom(&self) -> &'static str {
        match self.genre {
            Genre::Booleen { .. } => "Booléen",
            Genre::Repetition { .. } => "Répétition",
            Genre::Miroir { .. } => "Miroir",
        }
    }
}

/// Le résultat d'un objet : sa forme modifiée, et un message par modifieur qui n'a pas
/// pu s'appliquer.
#[derive(Clone, Debug)]
pub struct Resultat {
    pub forme: Arc<Forme>,
    pub erreurs: Vec<Option<String>>,
}

/// Les résultats, par identifiant d'objet (seulement les objets qui ont des modifieurs).
pub type Resultats = HashMap<u64, Resultat>;

pub fn forme(o: &Objet) -> Forme {
    Forme { sommets: o.sommets.clone(), faces: o.faces.clone() }
}

fn indice(objets: &[Objet], id: u64) -> Option<usize> {
    objets.iter().position(|o| o.id == id)
}

/// Calcule toutes les piles.
pub fn evaluer_tout(objets: &[Objet]) -> Resultats {
    let mut memo = HashMap::new();
    let mut r = Resultats::new();
    for (k, o) in objets.iter().enumerate() {
        if !o.modifs.is_empty() {
            let (_, res, _) = evaluer(objets, k, &[], &mut memo);
            r.insert(o.id, res);
        }
    }
    r
}

/// La forme modifiée de l'objet `k` (`pile` : les objets en cours de calcul). Le
/// booléen dit si une boucle a été rencontrée : ce résultat-là ne se garde pas.
fn evaluer(objets: &[Objet], k: usize, pile: &[usize], memo: &mut HashMap<usize, Arc<Forme>>) -> (Arc<Forme>, Resultat, bool) {
    let o = &objets[k];
    if let Some(f) = memo.get(&k) {
        return (f.clone(), Resultat { forme: f.clone(), erreurs: vec![None; o.modifs.len()] }, false);
    }
    let mut f = Arc::new(forme(o));
    let (mut erreurs, mut boucle) = (Vec::new(), false);
    for m in &o.modifs {
        if !m.actif {
            erreurs.push(None);
            continue;
        }
        match modifier(objets, k, &f, m, pile, memo, &mut boucle) {
            Ok(x) => {
                f = Arc::new(x);
                erreurs.push(None);
            }
            Err(e) => erreurs.push(Some(e)),
        }
    }
    if !boucle {
        memo.insert(k, f.clone());
    }
    (f.clone(), Resultat { forme: f, erreurs }, boucle)
}

fn lisible(e: String) -> String {
    if e == booleens::NON_FERME {
        "l'objet ou une cible n'est pas un volume fermé".into()
    } else {
        e
    }
}

/// Les formes des cibles d'un booléen, chacune avec sa propre pile.
fn formes_cibles(objets: &[Objet], k: usize, cibles: &[u64], pile: &[usize], memo: &mut HashMap<usize, Arc<Forme>>, boucle: &mut bool) -> Result<Vec<Arc<Forme>>, String> {
    if cibles.is_empty() {
        return Err("choisissez une cible".into());
    }
    let pile2: Vec<usize> = pile.iter().copied().chain([k]).collect();
    let mut r = Vec::new();
    for &id in cibles {
        let t = indice(objets, id).ok_or("une cible n'existe plus")?;
        if pile2.contains(&t) {
            *boucle = true;
            return Err("dépendance circulaire".into());
        }
        let (f, _, b) = evaluer(objets, t, &pile2, memo);
        *boucle |= b;
        r.push(f);
    }
    Ok(r)
}

fn modifier(objets: &[Objet], k: usize, f: &Forme, m: &Modifieur, pile: &[usize], memo: &mut HashMap<usize, Arc<Forme>>, boucle: &mut bool) -> Result<Forme, String> {
    match &m.genre {
        Genre::Booleen { op, cibles } => {
            let formes = formes_cibles(objets, k, cibles, pile, memo, boucle)?;
            let mut r = f.clone();
            for c in formes {
                r = booleens::operer(&r, &c, *op).map_err(lisible)?;
            }
            Ok(r)
        }
        Genre::Repetition { n, relatif, decalage, unir } => repeter(f, (*n).clamp(1, 200), *relatif, *decalage, *unir),
        Genre::Miroir { axes, souder } => {
            let mut r = f.clone();
            for a in (0..3).filter(|&a| axes[a]) {
                r = miroir(&r, &objets[k].matrice, a, *souder);
            }
            Ok(r)
        }
    }
}

fn repeter(f: &Forme, n: u32, relatif: bool, d: V3, unir: bool) -> Result<Forme, String> {
    let taille = f.boite().map_or([0.0; 3], |(a, b)| [b[0] - a[0], b[1] - a[1], b[2] - a[2]]);
    let pas: V3 = [0, 1, 2].map(|c| if relatif { d[c] * taille[c] } else { d[c] });
    let copies: Vec<Forme> = (0..n)
        .map(|i| {
            let mut c = f.clone();
            c.sommets.iter_mut().for_each(|s| *s = [0, 1, 2].map(|a| s[a] + pas[a] * i as f32));
            c
        })
        .collect();
    if !unir || n < 2 {
        return Ok(Forme::assembler(&copies));
    }
    let mut l = copies.into_iter();
    let mut r = l.next().unwrap_or_default();
    for c in l {
        r = booleens::operer(&r, &c, Op::Union).map_err(lisible)?;
    }
    Ok(r)
}

/// La forme et son reflet par le plan `axe` = 0 du repère de l'objet (sa matrice
/// d'origine) : les faces du reflet retournées pour rester tournées vers dehors. Avec
/// `souder`, les sommets à un dix-millième de la taille du plan y sont posés et
/// partagés, et les faces posées sur le plan (qui se doubleraient) s'en vont.
pub fn miroir(f: &Forme, mat: &[f32; 16], axe: usize, souder: bool) -> Forme {
    let inv = glb::inverse(mat);
    let local: Vec<V3> = f.sommets.iter().map(|p| glb::appliquer(&inv, *p)).collect();
    let taille = local
        .iter()
        .fold(None, |b: Option<(V3, V3)>, q| Some(b.map_or((*q, *q), |(a, c)| ([a[0].min(q[0]), a[1].min(q[1]), a[2].min(q[2])], [c[0].max(q[0]), c[1].max(q[1]), c[2].max(q[2])]))))
        .map_or(1.0, |(a, b)| ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2) + (b[2] - a[2]).powi(2)).sqrt());
    let eps = (taille * 1e-4).max(1e-7);
    let plan = |i: usize| souder && local[i][axe].abs() <= eps;
    let mut sommets = f.sommets.clone();
    let mut reflet = vec![0u32; local.len()];
    for (i, q) in local.iter().enumerate() {
        let mut q = *q;
        if plan(i) {
            q[axe] = 0.0;
            sommets[i] = glb::appliquer(mat, q);
            reflet[i] = i as u32;
        } else {
            q[axe] = -q[axe];
            sommets.push(glb::appliquer(mat, q));
            reflet[i] = (sommets.len() - 1) as u32;
        }
    }
    let gardees: Vec<&Vec<u32>> = f.faces.iter().filter(|p| !p.iter().all(|&i| plan(i as usize))).collect();
    let mut faces: Vec<Vec<u32>> = gardees.iter().map(|p| (*p).clone()).collect();
    faces.extend(gardees.iter().map(|p| p.iter().rev().map(|&i| reflet[i as usize]).collect()));
    Forme { sommets, faces }
}

/// La forme de base de l'objet `k` avec son seul modifieur `i` (même éteint) : ce que
/// fige « Appliquer ».
pub fn appliquer(objets: &[Objet], k: usize, i: usize) -> Result<Forme, String> {
    let o = &objets[k];
    let m = o.modifs.get(i).ok_or("modifieur absent")?;
    let mut memo = HashMap::new();
    modifier(objets, k, &forme(o), m, &[], &mut memo, &mut false)
}

/// L'objet `k` coupé en deux par les cibles du booléen `i` (réunies) : la part hors des
/// cibles, et la part dedans.
pub fn couper(objets: &[Objet], k: usize, i: usize) -> Result<(Forme, Forme), String> {
    let o = &objets[k];
    let Some(Modifieur { genre: Genre::Booleen { cibles, .. }, .. }) = o.modifs.get(i) else { return Err("ce n'est pas un booléen".into()) };
    let mut memo = HashMap::new();
    let formes = formes_cibles(objets, k, cibles, &[], &mut memo, &mut false)?;
    let mut tout = Forme::default();
    for c in formes {
        tout = booleens::operer(&tout, &c, Op::Union).map_err(lisible)?;
    }
    let base = forme(o);
    let dehors = booleens::operer(&base, &tout, Op::Difference).map_err(lisible)?;
    let dedans = booleens::operer(&base, &tout, Op::Intersection).map_err(lisible)?;
    Ok((dehors, dedans))
}

/// Les objets qui servent de cible à un booléen actif d'un objet visible (montrés en
/// fil de fer, laissés de côté à l'enregistrement).
pub fn cibles(objets: &[Objet]) -> HashSet<usize> {
    let mut c = HashSet::new();
    for (k, o) in objets.iter().enumerate().filter(|(_, o)| o.visible) {
        for m in o.modifs.iter().filter(|m| m.actif) {
            if let Genre::Booleen { cibles, .. } = &m.genre {
                c.extend(cibles.iter().filter_map(|&id| indice(objets, id)).filter(|&t| t != k));
            }
        }
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::maille::cube;

    fn scene() -> Vec<Objet> {
        let a = cube("a", [0.0; 3], 0.5);
        let b = cube("b", [0.5, 0.0, 0.0], 0.5);
        let mut v = vec![a, b];
        let id = v[1].id;
        v[0].modifs.push(Modifieur { genre: Genre::Booleen { op: Op::Difference, cibles: vec![id] }, actif: true });
        v
    }

    fn res(o: &[Objet], k: usize) -> Resultat {
        evaluer_tout(o).remove(&o[k].id).unwrap()
    }

    #[test]
    fn un_booleen_garde_la_base_et_suit_la_cible() {
        let mut o = scene();
        assert!((res(&o, 0).forme.volume() - 0.5).abs() < 1e-5);
        assert_eq!(o[0].faces.len(), 6);
        o[1].sommets.iter_mut().for_each(|p| p[0] += 0.25);
        assert!((res(&o, 0).forme.volume() - 0.75).abs() < 1e-5);
    }

    #[test]
    fn plusieurs_cibles_s_appliquent_l_une_apres_l_autre() {
        let mut o = scene();
        o.push(cube("c", [-0.5, 0.0, 0.0], 0.5));
        let ids = vec![o[1].id, o[2].id];
        o[0].modifs[0].genre = Genre::Booleen { op: Op::Difference, cibles: ids.clone() };
        assert!(res(&o, 0).forme.vide(), "creusé des deux côtés, il ne reste rien");
        o[0].modifs[0].genre = Genre::Booleen { op: Op::Union, cibles: ids };
        assert!((res(&o, 0).forme.volume() - 2.0).abs() < 1e-5);
    }

    #[test]
    fn les_erreurs_sont_dites_sans_arreter_la_pile() {
        let mut o = scene();
        o[0].modifs[0].genre = Genre::Booleen { op: Op::Difference, cibles: vec![] };
        o[0].modifs.push(Modifieur::neuf("repetition").unwrap());
        let r = res(&o, 0);
        assert_eq!(r.erreurs[0].as_deref(), Some("choisissez une cible"));
        assert_eq!(r.forme.faces.len(), 18, "la répétition s'applique quand même");
        o[0].modifs[0].genre = Genre::Booleen { op: Op::Difference, cibles: vec![12345] };
        assert_eq!(res(&o, 0).erreurs[0].as_deref(), Some("une cible n'existe plus"));
        // Une boucle : a creuse b, b creuse a
        let (ia, ib) = (o[0].id, o[1].id);
        o[0].modifs.truncate(1);
        o[0].modifs[0].genre = Genre::Booleen { op: Op::Difference, cibles: vec![ib] };
        o[1].modifs.push(Modifieur { genre: Genre::Booleen { op: Op::Difference, cibles: vec![ia] }, actif: true });
        let t = evaluer_tout(&o);
        assert_eq!(t[&ia].erreurs[0], None, "b, calculé sous a, voit la boucle et reste entier");
        assert!((t[&ia].forme.volume() - 0.5).abs() < 1e-5);
        assert_eq!(t[&ib].erreurs[0], None);
    }

    #[test]
    fn un_volume_ouvert_est_explique() {
        let mut o = scene();
        o[1].faces.pop();
        assert_eq!(res(&o, 0).erreurs[0].as_deref(), Some("l'objet ou une cible n'est pas un volume fermé"));
    }

    #[test]
    fn la_repetition_relative_constante_et_unie() {
        let mut o = scene();
        o[0].modifs = vec![Modifieur::neuf("repetition").unwrap()];
        let (_, b) = res(&o, 0).forme.boite().unwrap();
        assert!((b[0] - (0.5 + 2.4)).abs() < 1e-5);
        o[0].modifs[0].genre = Genre::Repetition { n: 3, relatif: false, decalage: [0.5, 0.0, 0.0], unir: true };
        assert!((res(&o, 0).forme.volume() - 2.0).abs() < 1e-5);
    }

    #[test]
    fn le_miroir_reflete_par_le_plan_de_l_objet_et_soude() {
        // Un demi-cube de x = 0 à 1 : son reflet va de -1 à 0, soudé au plan
        let mut o = vec![cube("a", [0.5, 0.0, 0.0], 0.5)];
        o[0].modifs.push(Modifieur::neuf("miroir").unwrap());
        let f = res(&o, 0).forme;
        let (a, b) = f.boite().unwrap();
        assert!((a[0] + 1.0).abs() < 1e-6 && (b[0] - 1.0).abs() < 1e-6);
        assert_eq!(f.sommets.len(), 12, "les quatre sommets du plan partagés");
        assert_eq!(f.faces.len(), 10, "les deux faces du plan s'en vont");
        assert!((f.volume() - 2.0).abs() < 1e-5, "fermé et tourné vers dehors");
        // Le plan suit l'objet : décalé de 3 en x, le reflet aussi
        o[0].matrice[12] = 3.0;
        let (_, b) = res(&o, 0).forme.boite().unwrap();
        assert!((b[0] - 6.0).abs() < 1e-5, "{b:?}");
        // Sans souder : tout se double
        o[0].matrice[12] = 0.0;
        o[0].modifs[0].genre = Genre::Miroir { axes: [true, true, false], souder: false };
        let f = res(&o, 0).forme;
        assert_eq!((f.sommets.len(), f.faces.len()), (32, 24));
    }

    #[test]
    fn appliquer_et_couper() {
        let mut o = scene();
        o[0].modifs[0].actif = false;
        assert!((appliquer(&o, 0, 0).unwrap().volume() - 0.5).abs() < 1e-5);
        let (dehors, dedans) = couper(&o, 0, 0).unwrap();
        assert!((dehors.volume() - 0.5).abs() < 1e-5 && (dedans.volume() - 0.5).abs() < 1e-5);
    }

    #[test]
    fn seuls_les_booleens_actifs_d_objets_visibles_font_une_cible() {
        let mut o = scene();
        assert_eq!(cibles(&o), HashSet::from([1]));
        o[0].visible = false;
        assert!(cibles(&o).is_empty());
        o[0].visible = true;
        o[0].modifs[0].actif = false;
        assert!(cibles(&o).is_empty());
    }
}
