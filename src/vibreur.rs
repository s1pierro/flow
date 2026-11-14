//! Le retour au toucher : des stries, les vibrations les plus courtes de l'appareil.
//!
//! `tic` ne bloque pas : la demande part à un thread qui appelle Java
//! (`fr.s1pierro.flow.Vibreur`), et les demandes qui s'accumulent pendant un appel n'en
//! font qu'une (le vibreur ne suivrait pas). `px_par_mm` donne la densité réelle de
//! l'écran, pour poser des stries au millimètre.

use std::sync::mpsc::{channel, Sender};
use std::sync::{Mutex, OnceLock};

static FILE: OnceLock<Mutex<Sender<u8>>> = OnceLock::new();
static DPI: OnceLock<Option<f32>> = OnceLock::new();
static ERREUR: Mutex<Option<String>> = Mutex::new(None);

/// Les niveaux de force d'une strie, du plus léger au plus fort.
pub const NIVEAUX: [&str; 4] = ["Légère", "Moyenne", "Forte", "Maximale"];

/// Une strie, de niveau `niveau` (0 à 3, voir `NIVEAUX`).
pub fn tic(niveau: u8) {
    let f = FILE.get_or_init(|| {
        let (tx, rx) = channel::<u8>();
        std::thread::spawn(move || {
            while let Ok(mut niveau) = rx.recv() {
                // Les demandes arrivées entre-temps se fondent en une
                while let Ok(n) = rx.try_recv() {
                    niveau = niveau.max(n);
                }
                let e = crate::acces::vibreur("tic", &niveau.to_string()).filter(|e| !e.is_empty());
                if let Ok(mut x) = ERREUR.lock() {
                    *x = e;
                }
            }
        });
        Mutex::new(tx)
    });
    if let Ok(t) = f.lock() {
        let _ = t.send(niveau);
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

/// Des cercles concentriques posés au premier contact, tous les `pas` pixels autour de
/// lui : rend vrai chaque fois que le doigt en franchit un. Seul l'éloignement du contact
/// se sent, pas le tour qu'on fait autour.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Stries {
    origine: Option<[f32; 2]>,
    cercle: i32,
}

impl Stries {
    /// Le doigt est en `p` (ou levé : `None`), les cercles tous les `pas` pixels (0 :
    /// aucun). Vrai si un cercle vient d'être franchi.
    pub fn suivre(&mut self, p: Option<[f32; 2]>, pas: f32) -> bool {
        let Some(p) = p else {
            self.origine = None;
            return false;
        };
        if pas <= 0.0 {
            return false;
        }
        let Some(o) = self.origine else {
            self.origine = Some(p);
            self.cercle = 0;
            return false;
        };
        let cercle = ((p[0] - o[0]).hypot(p[1] - o[1]) / pas).floor() as i32;
        let franchi = cercle != self.cercle;
        self.cercle = cercle;
        franchi
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn les_cercles_se_franchissent_en_s_eloignant() {
        let mut s = Stries::default();
        // Le contact pose les cercles : pas de strie
        assert!(!s.suivre(Some([100.0, 100.0]), 10.0));
        // En deçà du premier cercle : rien
        assert!(!s.suivre(Some([108.0, 100.0]), 10.0));
        // Le premier cercle, dans n'importe quelle direction
        assert!(s.suivre(Some([108.0, 108.0]), 10.0));
        assert!(!s.suivre(Some([92.0, 108.0]), 10.0));
        // Tourner autour à distance égale : rien
        assert!(!s.suivre(Some([88.0, 100.0]), 10.0));
        assert!(!s.suivre(Some([100.0, 112.0]), 10.0));
        // Le deuxième, puis en revenant
        assert!(s.suivre(Some([100.0, 121.0]), 10.0));
        assert!(s.suivre(Some([100.0, 119.0]), 10.0));
        // Levé puis reposé ailleurs : nouveaux cercles, pas de strie
        assert!(!s.suivre(None, 10.0));
        assert!(!s.suivre(Some([300.0, 300.0]), 10.0));
        assert!(!s.suivre(Some([306.0, 300.0]), 10.0));
        // Sans pas, jamais
        assert!(!s.suivre(Some([400.0, 300.0]), 0.0));
    }

    #[test]
    fn hors_d_android_le_millimetre_vient_du_dp() {
        assert!((px_par_mm(1.0) - 160.0 / 25.4).abs() < 1e-4);
    }
}
