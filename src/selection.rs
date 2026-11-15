//! La sélection de tuiles complètes de la Maquette : un ensemble de clés (niveau, i, j),
//! qu'un toucher bascule, et ce qu'on en dit (surface, éléments du lieu qu'elles
//! touchent). Les tuiles d'une sélection peuvent être de niveaux différents : une
//! tuile et l'une de ses ancêtres se recouvrent, la surface ne compte leur part
//! commune qu'une fois, et chaque élément du lieu n'est compté qu'une fois.
//! Retenue dans l'espace : « selection = niveau/i/j … », « selection_seules = oui ».

use crate::carte::{boite_tuile, Cle, Donnees};
use crate::sculpture::Camera;
use std::collections::{BTreeSet, HashSet};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Selection {
    pub cles: BTreeSet<Cle>,
    /// Ne montrer que les tuiles sélectionnées.
    pub seules: bool,
}

impl Selection {
    /// Ajoute la tuile, ou la retire si elle y est.
    pub fn basculer(&mut self, cle: Cle) {
        if !self.cles.remove(&cle) {
            self.cles.insert(cle);
        }
    }

    pub fn est_vide(&self) -> bool {
        self.cles.is_empty()
    }

    /// « 5/13/15 5/13/16 »
    pub fn ecrire(&self) -> String {
        self.cles.iter().map(|c| nom(*c)).collect::<Vec<_>>().join(" ")
    }

    pub fn lire(t: &str, seules: bool) -> Selection {
        let cles = t
            .split_whitespace()
            .filter_map(|m| {
                let v: Vec<i64> = m.split('/').filter_map(|x| x.parse().ok()).collect();
                match v[..] {
                    [z, i, j] if (0..=30).contains(&z) => Some((z as u8, i as i32, j as i32)),
                    _ => None,
                }
            })
            .collect();
        Selection { cles, seules }
    }
}

/// L'identifiant d'une tuile : « niveau/i/j ».
pub fn nom((z, i, j): Cle) -> String {
    format!("{z}/{i}/{j}")
}

/// La boîte d'une tuile bornée au lieu (vide si elle en sort).
pub fn boite_dans(d: &Donnees, cle: Cle) -> Option<[f32; 4]> {
    let b = boite_tuile(d, cle);
    let e = d.lieu.etendue();
    let r = [b[0].max(e[0] as f32), b[1].max(e[1] as f32), b[2].min(e[2] as f32), b[3].min(e[3] as f32)];
    (r[2] > r[0] && r[3] > r[1]).then_some(r)
}

/// Ce que touche une sélection.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Resume {
    pub tuiles: usize,
    /// En m² (les recouvrements comptés une fois).
    pub surface: f64,
    /// Les bâtiments par genre (Maison, Immeuble, Industriel, Édifice, Annexe).
    pub batiments: [usize; 5],
    pub routes: usize,
    pub ponts: usize,
    pub tunnels: usize,
    pub rails: usize,
    pub arrets: usize,
}

impl Resume {
    pub fn batiments_total(&self) -> usize {
        self.batiments.iter().sum()
    }
}

fn croise(a: [f32; 4], b: [f32; 4]) -> bool {
    a[0] <= b[2] && b[0] <= a[2] && a[1] <= b[3] && b[1] <= a[3]
}

/// La boîte d'une suite de points (x, z en tête).
fn boite_de<'a>(pts: impl Iterator<Item = (f32, f32)> + 'a) -> [f32; 4] {
    pts.fold([f32::MAX, f32::MAX, f32::MIN, f32::MIN], |b, (x, z)| [b[0].min(x), b[1].min(z), b[2].max(x), b[3].max(z)])
}

pub fn resumer(d: &Donnees, cles: &BTreeSet<Cle>) -> Resume {
    let l = &d.lieu;
    let mut r = Resume { tuiles: cles.len(), ..Resume::default() };
    let (mut bati, mut routes, mut ponts, mut tunnels, mut rails, mut arrets) = (HashSet::new(), HashSet::new(), HashSet::new(), HashSet::new(), HashSet::new(), HashSet::new());
    for &cle in cles {
        let Some(b) = boite_dans(d, cle) else { continue };
        // Une ancêtre sélectionnée la couvre déjà
        let (z, i, j) = cle;
        if !(1..=z).any(|k| cles.contains(&(z - k, i >> k, j >> k))) {
            r.surface += ((b[2] - b[0]) as f64) * ((b[3] - b[1]) as f64);
        }
        for k in d.batiments_dans(b) {
            if croise(boite_de(l.batiments[k as usize].contour.iter().map(|p| (p[0], p[1]))), b) {
                bati.insert(k);
            }
        }
        for (ponts_, ens) in [(false, &mut routes), (true, &mut ponts)] {
            let liste = if ponts_ { &l.ponts } else { &l.routes };
            for k in d.routes_dans(b, ponts_) {
                if croise(boite_de(liste[k as usize].points.iter().map(|p| (p[0], p[1]))), b) {
                    ens.insert(k);
                }
            }
        }
        let t = &l.transports;
        for k in d.tunnels_dans(b) {
            if croise(boite_de(t.tunnels[k as usize].points.iter().map(|p| (p[0], p[1]))), b) {
                tunnels.insert(k);
            }
        }
        for k in d.rails_dans(b) {
            if croise(boite_de(t.rails[k as usize].points.iter().map(|p| (p[0], p[1]))), b) {
                rails.insert(k);
            }
        }
        for k in d.arrets_dans(b) {
            let a = &t.arrets[k as usize];
            if croise([a.x, a.z, a.x, a.z], b) {
                arrets.insert(k);
            }
        }
    }
    for k in bati {
        let g = (l.batiments[k as usize].genre as usize).min(4);
        r.batiments[g] += 1;
    }
    (r.routes, r.ponts, r.tunnels, r.rails, r.arrets) = (routes.len(), ponts.len(), tunnels.len(), rails.len(), arrets.len());
    r
}

/// Le point du terrain sous le point (nx, ny) de la vue (de -1 à 1, y vers le haut),
/// par une marche le long du rayon puis une dichotomie. Le terrain : le relief, l'eau
/// à plat à son niveau.
pub fn pointer(d: &Donnees, camera: &Camera, aspect: f32, nx: f32, ny: f32) -> Option<[f32; 3]> {
    let l = &d.lieu;
    let e = l.etendue();
    let (o, dir) = camera.rayon(aspect, nx, ny);
    let haut = |x: f32, z: f32| {
        let r = l.relief(x as f64, z as f64) as f32;
        if l.eau(x as f64, z as f64) >= 0.5 { r.max(l.niveau) } else { r }
    };
    let dessous = |t: f32| {
        let p = [o[0] + dir[0] * t, o[1] + dir[1] * t, o[2] + dir[2] * t];
        p[1] <= haut(p[0], p[2])
    };
    let cote = (e[2] - e[0]).max(e[3] - e[1]) as f32;
    let fin = camera.distance * 20.0 + cote * 2.0;
    let mut t = 0.0;
    let mut avant = 0.0;
    while t < fin {
        let p = [o[0] + dir[0] * t, o[2] + dir[2] * t];
        let dedans = p[0] as f64 >= e[0] && p[0] as f64 <= e[2] && p[1] as f64 >= e[1] && p[1] as f64 <= e[3];
        if dedans && dessous(t) {
            let (mut a, mut b) = (avant, t);
            for _ in 0..30 {
                let m = (a + b) / 2.0;
                if dessous(m) {
                    b = m;
                } else {
                    a = m;
                }
            }
            return Some([o[0] + dir[0] * b, o[1] + dir[1] * b, o[2] + dir[2] * b]);
        }
        avant = t;
        t += (t * 0.004).max(0.5);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use sage_reel::{cote, Batiment, Lieu};

    fn donnees() -> Donnees {
        let pas = 64.0;
        let n = cote(pas);
        let h: Vec<f32> = vec![50.0; n * n];
        let eau = vec![false; n * n];
        let mut l = Lieu::new("Essai", 49.0, 1.0, 0.0, 0.0, pas, 40.0, &h, &eau, Vec::new(), Vec::new(), Vec::new());
        let carre = |x: f32, z: f32| Batiment { genre: sage_reel::MAISON, pied: 50.0, toit: 56.0, egout: 54.0, forme: 0, axe: 0.0, contour: vec![[x, z], [x + 8.0, z], [x + 8.0, z + 8.0], [x, z + 8.0]] };
        l.batiments = vec![carre(10.0, 10.0), carre(500.0, 500.0)];
        Donnees::new(l)
    }

    #[test]
    fn la_selection_bascule_et_se_relit() {
        let mut s = Selection::default();
        s.basculer((5, 13, 15));
        s.basculer((3, 1, 2));
        s.basculer((5, 13, 15));
        s.basculer((4, 0, 7));
        assert_eq!(s.ecrire(), "3/1/2 4/0/7");
        assert_eq!(Selection::lire(&s.ecrire(), true), Selection { cles: s.cles.clone(), seules: true });
        assert!(Selection::lire("x 1/2 3/4/5", false).cles.len() == 1);
    }

    #[test]
    fn la_surface_compte_les_recouvrements_une_fois_et_les_elements_aussi() {
        let d = donnees();
        let (x0, z0, c) = d.monde();
        let n = 6u8;
        let t = c / (1u32 << n) as f32;
        let cle = (n, ((10.0 - x0) / t).floor() as i32, ((10.0 - z0) / t).floor() as i32);
        let parent = (n - 1, cle.1 >> 1, cle.2 >> 1);
        let mut cles = BTreeSet::new();
        cles.insert(cle);
        let r = resumer(&d, &cles);
        assert_eq!(r.batiments[0], 1);
        assert!((r.surface - (t * t) as f64).abs() < 1.0);
        // Avec sa parente : la surface est celle de la parente, le bâtiment compté une fois
        cles.insert(parent);
        let r = resumer(&d, &cles);
        assert_eq!((r.tuiles, r.batiments_total()), (2, 1));
        assert!((r.surface - (4.0 * t * t) as f64).abs() < 1.0);
    }

    #[test]
    fn le_toucher_trouve_le_terrain() {
        let d = donnees();
        let camera = Camera { lacet: 0.3, tangage: 0.6, distance: 800.0, cible: [100.0, 50.0, -200.0] };
        // Au centre de la vue : la cible
        let p = pointer(&d, &camera, 1.5, 0.0, 0.0).expect("le sol");
        assert!((p[0] - 100.0).abs() < 1.0 && (p[1] - 50.0).abs() < 0.1 && (p[2] + 200.0).abs() < 1.0, "{p:?}");
        // Vers le ciel : rien
        let rasante = Camera { tangage: 0.05, ..camera };
        assert!(pointer(&d, &rasante, 1.5, 0.0, 1.0).is_none());
    }
}
