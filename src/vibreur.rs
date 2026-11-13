//! Le retour au toucher : des stries, les vibrations les plus courtes de l'appareil.
//!
//! `tic` ne bloque pas : la demande part à un thread qui appelle Java
//! (`fr.s1pierro.flow.Vibreur`), et les demandes qui s'accumulent pendant un appel n'en
//! font qu'une (le vibreur ne suivrait pas). `px_par_mm` donne la densité réelle de
//! l'écran, pour poser des stries au millimètre.

use std::sync::mpsc::{channel, Sender};
use std::sync::{Mutex, OnceLock};

static FILE: OnceLock<Mutex<Sender<f32>>> = OnceLock::new();
static DPI: OnceLock<Option<f32>> = OnceLock::new();
static ERREUR: Mutex<Option<String>> = Mutex::new(None);

/// Une strie, de force `force` (0 à 1).
pub fn tic(force: f32) {
    let f = FILE.get_or_init(|| {
        let (tx, rx) = channel::<f32>();
        std::thread::spawn(move || {
            while let Ok(mut force) = rx.recv() {
                // Les demandes arrivées entre-temps se fondent en une
                while let Ok(f) = rx.try_recv() {
                    force = force.max(f);
                }
                let e = crate::acces::vibreur("tic", &format!("{force:.2}")).filter(|e| !e.is_empty());
                if let Ok(mut x) = ERREUR.lock() {
                    *x = e;
                }
            }
        });
        Mutex::new(tx)
    });
    if let Ok(t) = f.lock() {
        let _ = t.send(force);
    }
}

/// La dernière erreur du vibreur (pas de vibreur…), s'il y en a une.
pub fn erreur() -> Option<String> {
    ERREUR.lock().ok()?.clone()
}

/// Les pixels par millimètre de l'écran : d'après sa densité réelle sous Android, sinon
/// d'après `dp` (160 px par pouce pour 1 dp).
pub fn px_par_mm(dp: f32) -> f32 {
    let dpi = DPI.get_or_init(|| {
        let t = crate::acces::vibreur("dpi", "")?;
        let mut v = t.split(' ').filter_map(|x| x.parse::<f32>().ok());
        let (x, y) = (v.next()?, v.next()?);
        // Certains appareils déclarent n'importe quoi : on garde ce qui est plausible
        let m = (x + y) / 2.0;
        (80.0..1000.0).contains(&m).then_some(m)
    });
    dpi.unwrap_or(160.0 * dp) / 25.4
}

/// Une grille de stries posée au premier contact : rend vrai chaque fois que le doigt
/// franchit une de ses lignes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Stries {
    origine: Option<[f32; 2]>,
    case: (i32, i32),
}

impl Stries {
    /// Le doigt est en `p` (ou levé : `None`), les lignes tous les `pas` pixels (0 :
    /// aucune). Vrai si une ligne vient d'être franchie.
    pub fn suivre(&mut self, p: Option<[f32; 2]>, pas: f32) -> bool {
        let Some(p) = p else {
            self.origine = None;
            return false;
        };
        if pas <= 0.0 {
            return false;
        }
        // La grille est centrée sur le contact : la première ligne est à un demi-pas
        let Some(o) = self.origine else {
            self.origine = Some(p);
            self.case = (0, 0);
            return false;
        };
        let case = (((p[0] - o[0]) / pas + 0.5).floor() as i32, ((p[1] - o[1]) / pas + 0.5).floor() as i32);
        let franchie = case != self.case;
        self.case = case;
        franchie
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn les_stries_se_franchissent_au_pas() {
        let mut s = Stries::default();
        // Le contact pose la grille : pas de strie
        assert!(!s.suivre(Some([100.0, 100.0]), 10.0));
        // Moins d'un demi-pas : rien
        assert!(!s.suivre(Some([104.0, 100.0]), 10.0));
        // Une ligne franchie
        assert!(s.suivre(Some([106.0, 100.0]), 10.0));
        assert!(!s.suivre(Some([114.0, 100.0]), 10.0));
        assert!(s.suivre(Some([116.0, 100.0]), 10.0));
        // Vers le bas aussi, et en revenant
        assert!(s.suivre(Some([116.0, 106.0]), 10.0));
        assert!(s.suivre(Some([116.0, 104.0]), 10.0));
        // Levé puis reposé ailleurs : nouvelle grille, pas de strie
        assert!(!s.suivre(None, 10.0));
        assert!(!s.suivre(Some([300.0, 300.0]), 10.0));
        assert!(!s.suivre(Some([303.0, 300.0]), 10.0));
        // Sans pas, jamais
        assert!(!s.suivre(Some([400.0, 300.0]), 0.0));
    }

    #[test]
    fn hors_d_android_le_millimetre_vient_du_dp() {
        assert!((px_par_mm(1.0) - 160.0 / 25.4).abs() < 1e-4);
    }
}
