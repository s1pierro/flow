//! Le magnétophone de la Boombox : tout ce que décode le lecteur de la radio passe
//! par ici. Les cinq dernières minutes du flux restent en mémoire (le tampon, en PCM
//! 16 bits stéréo), si bien qu'un enregistrement peut commencer dans le passé : la
//! part voulue du tampon est écrite d'abord, puis le direct à mesure qu'il arrive.
//!
//! Les enregistrements sont des `.wav` (PCM 16 bits stéréo, à la fréquence du flux).
//! L'en-tête est remis à jour chaque seconde : un fichier interrompu (Flow tué) reste
//! lisible jusqu'à sa dernière seconde.

use std::collections::VecDeque;
use std::fs::File;
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};

/// La durée du tampon, en secondes.
pub const TAMPON_S: u32 = 300;

/// Un enregistrement en cours.
pub struct Prise {
    pub chemin: PathBuf,
    ecrivain: BufWriter<File>,
    /// Les trames écrites (une trame : un échantillon gauche et droit).
    trames: u64,
    /// Les trames à la dernière mise à jour de l'en-tête.
    entete_a: u64,
    frequence: u32,
}

impl Prise {
    pub fn duree_s(&self) -> f32 {
        self.trames as f32 / self.frequence.max(1) as f32
    }

    /// Réécrit les tailles de l'en-tête, puis revient en fin de fichier.
    fn mettre_a_jour(&mut self) -> std::io::Result<()> {
        let donnees = (self.trames * 4).min(u32::MAX as u64 - 36) as u32;
        self.ecrivain.seek(SeekFrom::Start(0))?;
        self.ecrivain.write_all(&entete(self.frequence, donnees))?;
        self.ecrivain.seek(SeekFrom::End(0))?;
        self.ecrivain.flush()?;
        self.entete_a = self.trames;
        Ok(())
    }
}

/// Le magnétophone : le tampon du flux en cours, et l'enregistrement s'il y en a un.
#[derive(Default)]
pub struct Magneto {
    /// Le flux dont le tampon garde la fin.
    pub url: Option<String>,
    pub frequence: u32,
    tampon: VecDeque<[i16; 2]>,
    pub prise: Option<Prise>,
    /// La dernière erreur d'écriture : l'enregistrement s'est alors arrêté.
    pub erreur: Option<String>,
}

pub fn magneto() -> MutexGuard<'static, Magneto> {
    static M: OnceLock<Mutex<Magneto>> = OnceLock::new();
    M.get_or_init(|| Mutex::new(Magneto::default())).lock().unwrap_or_else(|e| e.into_inner())
}

fn echantillon(x: f32) -> i16 {
    (x.clamp(-1.0, 1.0) * 32767.0).round() as i16
}

impl Magneto {
    /// Ce que le décodeur vient de rendre : du f32 stéréo entrelacé, du flux `url`. Un
    /// autre flux (ou une autre fréquence) vide le tampon et clôt l'enregistrement.
    pub fn recevoir(&mut self, url: &str, frequence: u32, stereo: &[f32]) {
        if self.url.as_deref() != Some(url) || self.frequence != frequence {
            self.arreter();
            self.url = Some(url.to_string());
            self.frequence = frequence;
            self.tampon = VecDeque::with_capacity((TAMPON_S * frequence) as usize);
        }
        let max = (TAMPON_S * frequence) as usize;
        let mut ecrit: Vec<u8> = Vec::new();
        for t in stereo.chunks_exact(2) {
            let tr = [echantillon(t[0]), echantillon(t[1])];
            if self.tampon.len() >= max {
                self.tampon.pop_front();
            }
            self.tampon.push_back(tr);
            if self.prise.is_some() {
                ecrit.extend_from_slice(&tr[0].to_le_bytes());
                ecrit.extend_from_slice(&tr[1].to_le_bytes());
            }
        }
        if let Some(p) = &mut self.prise {
            let r = p.ecrivain.write_all(&ecrit).and_then(|_| {
                p.trames += (ecrit.len() / 4) as u64;
                if p.trames - p.entete_a >= p.frequence as u64 {
                    p.mettre_a_jour()
                } else {
                    Ok(())
                }
            });
            if let Err(e) = r {
                self.erreur = Some(format!("enregistrement interrompu : {e}"));
                self.prise = None;
            }
        }
    }

    /// Les secondes du flux `url` que garde le tampon (0 pour un autre flux).
    pub fn passe_s(&self, url: &str) -> f32 {
        if self.url.as_deref() == Some(url) && self.frequence > 0 {
            self.tampon.len() as f32 / self.frequence as f32
        } else {
            0.0
        }
    }

    /// Commence un enregistrement dans `dossier`, sous un nom tiré de `nom` (sans
    /// extension ; « -2 », « -3 »… s'il est pris), avec les `passe_s` dernières
    /// secondes du tampon.
    pub fn commencer(&mut self, dossier: &Path, nom: &str, passe_s: f32) -> Result<PathBuf, String> {
        if self.frequence == 0 {
            return Err("aucun flux à enregistrer".into());
        }
        std::fs::create_dir_all(dossier).map_err(|e| format!("dossier impossible à créer : {e}"))?;
        let chemin = (1..)
            .map(|i| dossier.join(if i == 1 { format!("{nom}.wav") } else { format!("{nom}-{i}.wav") }))
            .find(|c| !c.exists())
            .unwrap_or_else(|| dossier.join(format!("{nom}.wav")));
        let f = File::create(&chemin).map_err(|e| format!("fichier impossible à créer : {e}"))?;
        let mut p = Prise { chemin: chemin.clone(), ecrivain: BufWriter::new(f), trames: 0, entete_a: 0, frequence: self.frequence };
        let n = ((passe_s.max(0.0) * self.frequence as f32) as usize).min(self.tampon.len());
        let mut octets = Vec::with_capacity(44 + n * 4);
        octets.extend_from_slice(&entete(self.frequence, 0));
        for tr in self.tampon.iter().skip(self.tampon.len() - n) {
            octets.extend_from_slice(&tr[0].to_le_bytes());
            octets.extend_from_slice(&tr[1].to_le_bytes());
        }
        p.ecrivain.write_all(&octets).map_err(|e| format!("écriture impossible : {e}"))?;
        p.trames = n as u64;
        p.mettre_a_jour().map_err(|e| format!("écriture impossible : {e}"))?;
        self.erreur = None;
        self.prise = Some(p);
        Ok(chemin)
    }

    /// Clôt l'enregistrement en cours : le fichier et sa durée.
    pub fn arreter(&mut self) -> Option<(PathBuf, f32)> {
        let mut p = self.prise.take()?;
        if let Err(e) = p.mettre_a_jour() {
            self.erreur = Some(format!("fin d'enregistrement : {e}"));
        }
        Some((p.chemin.clone(), p.duree_s()))
    }
}

/// L'en-tête d'un `.wav` PCM 16 bits stéréo.
fn entete(frequence: u32, donnees: u32) -> [u8; 44] {
    let mut h = [0u8; 44];
    let mut put = |i: usize, b: &[u8]| h[i..i + b.len()].copy_from_slice(b);
    put(0, b"RIFF");
    put(4, &(36 + donnees).to_le_bytes());
    put(8, b"WAVEfmt ");
    put(16, &16u32.to_le_bytes());
    put(20, &1u16.to_le_bytes());
    put(22, &2u16.to_le_bytes());
    put(24, &frequence.to_le_bytes());
    put(28, &(frequence * 4).to_le_bytes());
    put(32, &4u16.to_le_bytes());
    put(34, &16u16.to_le_bytes());
    put(36, b"data");
    put(40, &donnees.to_le_bytes());
    h
}

/// Un enregistrement du dossier.
#[derive(Clone, Debug, PartialEq)]
pub struct Enregistrement {
    pub chemin: PathBuf,
    pub nom: String,
    pub duree_s: f32,
}

/// Les `.wav` du dossier, du plus récent au plus ancien.
pub fn lister(dossier: &Path) -> Vec<Enregistrement> {
    let Ok(rd) = std::fs::read_dir(dossier) else { return Vec::new() };
    let mut l: Vec<(std::time::SystemTime, Enregistrement)> = rd
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("wav")))
        .map(|e| {
            let chemin = e.path();
            let m = e.metadata().ok();
            let taille = m.as_ref().map_or(0, |m| m.len());
            let quand = m.and_then(|m| m.modified().ok()).unwrap_or(std::time::UNIX_EPOCH);
            // La durée d'après la taille et l'octet par seconde de l'en-tête
            let mut h = [0u8; 44];
            let par_s = File::open(&chemin).and_then(|mut f| f.read_exact(&mut h)).ok().map(|_| u32::from_le_bytes([h[28], h[29], h[30], h[31]])).filter(|&x| x > 0);
            let duree_s = par_s.map_or(0.0, |b| taille.saturating_sub(44) as f32 / b as f32);
            let nom = chemin.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned());
            (quand, Enregistrement { chemin, nom, duree_s })
        })
        .collect();
    l.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.nom.cmp(&a.1.nom)));
    l.into_iter().map(|x| x.1).collect()
}

/// Une durée en clair : « 45 s », « 2:05 », « 1:02:03 ».
pub fn duree(s: f32) -> String {
    let s = s.max(0.0) as u64;
    match s {
        0..=59 => format!("{s} s"),
        60..=3599 => format!("{}:{:02}", s / 60, s % 60),
        _ => format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60),
    }
}

/// Un nom de fichier tiré d'une adresse de flux : son dernier morceau, sans
/// extension, réduit aux lettres, chiffres, tirets.
pub fn nom_de(url: &str) -> String {
    let c = crate::radio::courte(url);
    let dernier = c.rsplit('/').find(|s| !s.is_empty()).unwrap_or(c);
    let sans = dernier.rsplit_once('.').map_or(dernier, |(a, _)| a);
    let n: String = sans.chars().map(|ch| if ch.is_alphanumeric() || ch == '-' || ch == '_' { ch } else { '-' }).collect();
    let n = n.trim_matches('-').to_string();
    if n.is_empty() {
        "flux".into()
    } else {
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_tampon_garde_cinq_minutes_et_l_enregistrement_part_du_passe() {
        let mut m = Magneto::default();
        // 10 Hz pour garder le test léger : 6 minutes reçues, 5 gardées
        let une_s: Vec<f32> = (0..20).map(|i| if i % 2 == 0 { 0.5 } else { -0.5 }).collect();
        for _ in 0..360 {
            m.recevoir("http://a/fip.mp3", 10, &une_s);
        }
        assert_eq!(m.passe_s("http://a/fip.mp3"), 300.0);
        assert_eq!(m.passe_s("http://b"), 0.0);
        let d = std::env::temp_dir().join(format!("flow-magneto-{}", std::process::id()));
        let c = m.commencer(&d, "fip", 60.0).unwrap();
        for _ in 0..3 {
            m.recevoir("http://a/fip.mp3", 10, &une_s);
        }
        let (c2, s) = m.arreter().unwrap();
        assert_eq!((c2, s), (c.clone(), 63.0));
        let o = std::fs::read(&c).unwrap();
        assert_eq!(&o[0..4], b"RIFF");
        assert_eq!(u32::from_le_bytes([o[40], o[41], o[42], o[43]]) as usize, o.len() - 44);
        assert_eq!(o.len(), 44 + 630 * 4);
        assert_eq!(i16::from_le_bytes([o[44], o[45]]), 16384);
        // Un deuxième sous le même nom ; la liste les retrouve, avec leur durée
        m.commencer(&d, "fip", 0.0).unwrap();
        m.arreter();
        let l = lister(&d);
        assert_eq!(l.len(), 2);
        assert!(l.iter().any(|e| e.nom == "fip-2.wav" && e.duree_s == 0.0) && l.iter().any(|e| e.nom == "fip.wav" && e.duree_s == 63.0));
        // Un autre flux vide le tampon
        m.recevoir("http://b", 10, &une_s);
        assert_eq!(m.passe_s("http://b"), 1.0);
        let _ = std::fs::remove_dir_all(&d);
        assert_eq!((duree(45.0), duree(125.0), duree(3723.0)), ("45 s".to_string(), "2:05".to_string(), "1:02:03".to_string()));
        assert_eq!((nom_de("https://icecast.radiofrance.fr/fip-midfi.mp3"), nom_de("https://x.org/"), nom_de("https://s.com/aac 128")), ("fip-midfi".to_string(), "x".to_string(), "aac-128".to_string()));
    }
}
