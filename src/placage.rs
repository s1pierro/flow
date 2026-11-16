//! Le placage de la maquette : les textures des matières (contexte Matières, fichiers
//! `Flow/matieres/<nom>.txt`) posées sur les faces que les volumes laissent dans les
//! tuiles (`maquette::matiere`). Chaque matière de la maquette prend une matière de la
//! bibliothèque (`TABLE`) ; ses images, refaites d'après leurs réglages dans un fil,
//! s'empilent en un tableau de textures (une couche par matière de la maquette, dans
//! l'ordre de leurs numéros, à partir de 1). Les coordonnées de texture des faces
//! sont en mètres : la couche se répète tous les `cote` mètres de sa matière.
//!
//! Les réglages sont relus quand un fichier change (vu au plus toutes les deux
//! secondes) : une matière retouchée dans son contexte se voit sur la maquette.

use crate::draw::Placage3d;
use crate::maquette::matiere;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Instant, SystemTime};

/// Le côté des couches (les images des matières y sont ramenées).
pub const COTE: u32 = 512;

/// La matière de la bibliothèque de chaque matière de la maquette, de 1 à `HAUT - 1`.
pub const TABLE: [(u16, &str); 5] = [
    (matiere::LIT, "vase-lit"),
    (matiere::BERGE, "taille-quai"),
    (matiere::REMBLAI, "pierre-quai"),
    (matiere::QUAI, "paves-quai"),
    (matiere::QUAI_MUR, "pierre-quai"),
];

#[derive(Default)]
struct Etat {
    dossier: Option<PathBuf>,
    /// Les dates des fichiers lus (pour voir qu'ils changent).
    dates: Vec<Option<SystemTime>>,
    vu: Option<Instant>,
    en_cours: bool,
    pret: Option<Placage3d>,
    revision: u64,
}

fn etat() -> &'static Mutex<Etat> {
    static E: OnceLock<Mutex<Etat>> = OnceLock::new();
    E.get_or_init(|| Mutex::new(Etat::default()))
}

fn dates(d: &Path) -> Vec<Option<SystemTime>> {
    TABLE.iter().map(|(_, n)| std::fs::metadata(d.join(format!("{n}.txt"))).and_then(|m| m.modified()).ok()).collect()
}

/// Ramène une image carrée de `n` pixels à `COTE` (au plus proche).
fn ramener(rgba: &[u8], n: u32) -> Vec<u8> {
    if n == COTE {
        return rgba.to_vec();
    }
    let mut r = vec![0u8; (COTE * COTE * 4) as usize];
    for y in 0..COTE {
        for x in 0..COTE {
            let (sx, sy) = (x * n / COTE, y * n / COTE);
            let s = ((sy * n + sx) * 4) as usize;
            let d = ((y * COTE + x) * 4) as usize;
            r[d..d + 4].copy_from_slice(&rgba[s..s + 4]);
        }
    }
    r
}

/// Les couches et leurs côtés en mètres, d'après les réglages du dossier (une matière
/// absente : une couche grise, d'un mètre).
pub fn fabriquer(d: &Path) -> (Vec<u8>, Vec<f32>) {
    let mut pixels = Vec::with_capacity((COTE * COTE * 4) as usize * TABLE.len());
    let mut cotes = Vec::with_capacity(TABLE.len());
    for (_, nom) in TABLE {
        match crate::contextes::matieres::lire(d, nom) {
            Some(r) => {
                let img = sage_matieres::generer(&r);
                pixels.extend(ramener(&img.rgba, img.l.min(img.h)));
                cotes.push(r.cote.max(0.05));
            }
            None => {
                pixels.extend(std::iter::repeat_n([150u8, 150, 150, 255], (COTE * COTE) as usize).flatten());
                cotes.push(1.0);
            }
        }
    }
    (pixels, cotes)
}

/// Les textures des matières lues dans `dossier` : celles prêtes, s'il y en a (la
/// première fois, ou quand un fichier change, elles se refont dans un fil).
pub fn textures(dossier: &Path) -> Option<Placage3d> {
    let mut e = etat().lock().unwrap();
    let change = e.dossier.as_deref() != Some(dossier);
    let a_voir = change || e.vu.is_none_or(|t| t.elapsed().as_secs_f32() > 2.0);
    if a_voir && !e.en_cours {
        e.vu = Some(Instant::now());
        let dt = dates(dossier);
        if change || dt != e.dates {
            e.dossier = Some(dossier.to_path_buf());
            e.dates = dt;
            e.en_cours = true;
            let d = dossier.to_path_buf();
            std::thread::spawn(move || {
                let (pixels, cotes) = fabriquer(&d);
                let mut e = etat().lock().unwrap();
                e.en_cours = false;
                e.revision += 1;
                let mut c = [1.0f32; 8];
                c[..cotes.len()].copy_from_slice(&cotes);
                e.pret = Some(Placage3d {
                    image: crate::draw::Image { id: 0x504c_4143_4147_4500, revision: e.revision, largeur: COTE, hauteur: COTE * TABLE.len() as u32, pixels: Arc::new(pixels) },
                    couches: TABLE.len() as u32,
                    cotes: c,
                });
            });
        }
    }
    e.pret.clone()
}

/// Des textures se refont : la vue doit se redessiner quand elles seront prêtes.
pub fn en_cours() -> bool {
    etat().lock().unwrap().en_cours
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_table_suit_les_numeros() {
        for (k, (m, _)) in TABLE.iter().enumerate() {
            assert_eq!(*m as usize, k + 1);
        }
    }

    #[test]
    fn une_matiere_absente_est_grise() {
        let (px, cotes) = fabriquer(Path::new("/nulle-part"));
        assert_eq!(px.len(), (COTE * COTE * 4) as usize * TABLE.len());
        assert_eq!(cotes, vec![1.0; TABLE.len()]);
        assert_eq!(&px[..4], &[150, 150, 150, 255]);
    }
}
