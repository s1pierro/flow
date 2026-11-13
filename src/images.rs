//! Les images des fichiers (JPEG, PNG, WebP, GIF, BMP, HEIC, AVIF), décodées par le
//! décodeur d'Android (`AImageDecoder` de `libjnigraphics`, Android 11 et plus),
//! chargé à l'exécution comme les autres bibliothèques du système. Il applique
//! l'orientation EXIF et rend du RVBA prémultiplié, à la taille demandée.
//!
//! Le décodage se fait dans deux threads : on *demande* une image (un chemin, la plus
//! grande taille voulue de son côté le plus long), et on la reçoit aux images
//! suivantes, quand elle est prête. Les demandes les plus récentes passent d'abord
//! (ce qui est à l'écran) ; les images servies restent en cache, dans une limite de
//! mémoire, les moins récemment vues partant les premières.

use crate::audio::bibliotheque;
use crate::draw::Image;
use std::collections::HashMap;
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
use std::time::SystemTime;

type Decodeur = c_void;
type Entete = c_void;

bibliotheque! { "libjnigraphics.so", Graphiques,
    AImageDecoder_createFromFd: fn(i32, *mut *mut Decodeur) -> i32;
    AImageDecoder_delete: fn(*mut Decodeur) -> ();
    AImageDecoder_getHeaderInfo: fn(*const Decodeur) -> *const Entete;
    AImageDecoderHeaderInfo_getWidth: fn(*const Entete) -> i32;
    AImageDecoderHeaderInfo_getHeight: fn(*const Entete) -> i32;
    AImageDecoder_setAndroidBitmapFormat: fn(*mut Decodeur, i32) -> i32;
    AImageDecoder_setTargetSize: fn(*mut Decodeur, i32, i32) -> i32;
    AImageDecoder_getMinimumStride: fn(*mut Decodeur) -> usize;
    AImageDecoder_decodeImage: fn(*mut Decodeur, *mut c_void, usize, usize) -> i32;
}

const RGBA_8888: i32 = 1;
const SUCCES: i32 = 0;

fn graphiques() -> Result<&'static Graphiques, String> {
    static G: OnceLock<Result<Graphiques, String>> = OnceLock::new();
    G.get_or_init(|| unsafe { Graphiques::charger() }).as_ref().map_err(|e| e.clone())
}

/// Une image décodée, et la taille de l'original.
#[derive(Clone, Debug)]
pub struct Decodee {
    pub largeur: u32,
    pub hauteur: u32,
    pub pixels: Vec<u8>,
    pub original: (u32, u32),
}

/// La taille de `(l, h)` réduite pour que son plus grand côté tienne dans `max`.
pub fn ajuster(l: u32, h: u32, max: u32) -> (u32, u32) {
    let grand = l.max(h).max(1);
    if grand <= max {
        return (l.max(1), h.max(1));
    }
    let k = max as f64 / grand as f64;
    (((l as f64 * k).round() as u32).max(1), ((h as f64 * k).round() as u32).max(1))
}

/// Décode le fichier, réduit pour que son plus grand côté tienne dans `max` pixels.
pub fn decoder(chemin: &Path, max: u32) -> Result<Decodee, String> {
    use std::os::fd::AsRawFd;
    let g = graphiques()?;
    let f = std::fs::File::open(chemin).map_err(|e| format!("illisible : {e}"))?;
    unsafe {
        let mut d: *mut Decodeur = std::ptr::null_mut();
        if (g.AImageDecoder_createFromFd)(f.as_raw_fd(), &mut d) != SUCCES || d.is_null() {
            return Err("format d'image inconnu".into());
        }
        // Libère le décodeur quoi qu'il arrive (le fichier, lui, se ferme après)
        struct Garde<'a>(&'a Graphiques, *mut Decodeur);
        impl Drop for Garde<'_> {
            fn drop(&mut self) {
                unsafe { (self.0.AImageDecoder_delete)(self.1) };
            }
        }
        let _garde = Garde(g, d);
        let e = (g.AImageDecoder_getHeaderInfo)(d);
        let (l, h) = ((g.AImageDecoderHeaderInfo_getWidth)(e).max(1) as u32, (g.AImageDecoderHeaderInfo_getHeight)(e).max(1) as u32);
        (g.AImageDecoder_setAndroidBitmapFormat)(d, RGBA_8888);
        let (mut tl, mut th) = ajuster(l, h, max);
        if (tl, th) != (l, h) && (g.AImageDecoder_setTargetSize)(d, tl as i32, th as i32) != SUCCES {
            (tl, th) = (l, h);
        }
        let pas = (g.AImageDecoder_getMinimumStride)(d).max(tl as usize * 4);
        let mut brut = vec![0u8; pas * th as usize];
        let r = (g.AImageDecoder_decodeImage)(d, brut.as_mut_ptr() as *mut c_void, pas, brut.len());
        // Une image incomplète (tronquée) est rendue telle quelle
        if r != SUCCES && r != -1 {
            return Err(format!("décodage impossible ({r})"));
        }
        let ligne = tl as usize * 4;
        let pixels = if pas == ligne {
            brut
        } else {
            let mut p = Vec::with_capacity(ligne * th as usize);
            for y in 0..th as usize {
                p.extend_from_slice(&brut[y * pas..y * pas + ligne]);
            }
            p
        };
        drop(f);
        Ok(Decodee { largeur: tl, hauteur: th, pixels, original: (l, h) })
    }
}

/// Une image en cache, ou en route.
enum Etat {
    Attente,
    Prete(Image, (u32, u32)),
    Erreur(String),
}

struct Entree {
    etat: Etat,
    date: Option<SystemTime>,
    /// Le tour où elle a été demandée pour la dernière fois.
    vue: u64,
}

#[derive(Default)]
struct Cache {
    entrees: HashMap<(PathBuf, u32), Entree>,
    /// Les demandes en attente, la plus récente à la fin.
    file: Vec<(PathBuf, u32)>,
    tour: u64,
    octets: usize,
}

/// La mémoire des images décodées gardées.
const BUDGET: usize = 160 << 20;
/// Au-delà, les plus vieilles demandes en attente sont oubliées.
const FILE_MAX: usize = 96;

fn cache() -> MutexGuard<'static, Cache> {
    cache_et_signal().0.lock().unwrap_or_else(|e| e.into_inner())
}

fn cache_et_signal() -> &'static (Mutex<Cache>, Condvar) {
    static C: OnceLock<(Mutex<Cache>, Condvar)> = OnceLock::new();
    C.get_or_init(|| {
        for _ in 0..2 {
            std::thread::spawn(travailler);
        }
        (Mutex::new(Cache::default()), Condvar::new())
    })
}

fn identifiant(chemin: &Path, max: u32) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    chemin.hash(&mut h);
    max.hash(&mut h);
    h.finish() | 1 << 63
}

fn travailler() {
    let (m, signal) = cache_et_signal();
    loop {
        let (chemin, max) = {
            let mut c = m.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                if let Some(x) = c.file.pop() {
                    break x;
                }
                c = signal.wait(c).unwrap_or_else(|e| e.into_inner());
            }
        };
        let r = decoder(&chemin, max);
        let mut c = m.lock().unwrap_or_else(|e| e.into_inner());
        let Some(e) = c.entrees.get_mut(&(chemin.clone(), max)) else { continue };
        let ajout = match r {
            Ok(d) => {
                let n = d.pixels.len();
                e.etat = Etat::Prete(Image { id: identifiant(&chemin, max), revision: e.date.map_or(0, |t| t.duration_since(SystemTime::UNIX_EPOCH).map_or(0, |x| x.as_nanos() as u64)), largeur: d.largeur, hauteur: d.hauteur, pixels: Arc::new(d.pixels) }, d.original);
                n
            }
            Err(x) => {
                e.etat = Etat::Erreur(x);
                0
            }
        };
        c.octets += ajout;
        liberer(&mut c);
        drop(c);
        crate::radio::reveiller();
    }
}

/// Oublie les images les moins récemment vues, au-delà du budget.
fn liberer(c: &mut Cache) {
    while c.octets > BUDGET {
        let Some(k) = c.entrees.iter().filter(|(_, e)| matches!(e.etat, Etat::Prete(..))).min_by_key(|(_, e)| e.vue).map(|(k, _)| k.clone()) else { break };
        if let Some(Entree { etat: Etat::Prete(i, _), .. }) = c.entrees.remove(&k) {
            c.octets = c.octets.saturating_sub(i.pixels.len());
        }
    }
}

/// Ce que donne une demande.
#[derive(Clone, Debug)]
pub enum Reponse {
    /// Pas encore décodée.
    Attente,
    /// L'image, et la taille de l'original.
    Prete(Image, (u32, u32)),
    Erreur(String),
}

/// Demande l'image du fichier (de date `date`, qui la fait redécoder si elle change),
/// réduite à `max` pixels sur son plus grand côté.
pub fn demander(chemin: &Path, date: Option<SystemTime>, max: u32) -> Reponse {
    let (_, signal) = cache_et_signal();
    let mut c = cache();
    c.tour += 1;
    let tour = c.tour;
    let k = (chemin.to_path_buf(), max);
    let a_jour = c.entrees.get(&k).is_some_and(|e| e.date == date);
    if !a_jour {
        if let Some(Entree { etat: Etat::Prete(i, _), .. }) = c.entrees.remove(&k) {
            c.octets = c.octets.saturating_sub(i.pixels.len());
        }
        c.entrees.insert(k.clone(), Entree { etat: Etat::Attente, date, vue: tour });
    }
    let e = c.entrees.get_mut(&k).expect("entrée posée");
    e.vue = tour;
    let r = match &e.etat {
        Etat::Prete(i, o) => Reponse::Prete(i.clone(), *o),
        Etat::Erreur(x) => Reponse::Erreur(x.clone()),
        Etat::Attente => Reponse::Attente,
    };
    if matches!(r, Reponse::Attente) {
        // En tête de file (la fin) : ce qui est demandé maintenant est à l'écran
        c.file.retain(|x| *x != k);
        c.file.push(k);
        if c.file.len() > FILE_MAX {
            let trop = c.file.len() - FILE_MAX;
            let oubliees: Vec<_> = c.file.drain(..trop).collect();
            for o in oubliees {
                c.entrees.remove(&o);
            }
        }
        signal.notify_one();
    }
    r
}

/// Vrai si le fichier est une image que le décodeur sait lire (le SVG a son contexte).
pub fn lisible(nom: &str) -> bool {
    let ext = nom.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "heic" | "heif" | "avif")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn les_tailles_s_ajustent_et_les_formats_se_reconnaissent() {
        assert_eq!(ajuster(4000, 3000, 2000), (2000, 1500));
        assert_eq!(ajuster(300, 4000, 400), (30, 400));
        assert_eq!(ajuster(100, 50, 400), (100, 50));
        assert!(lisible("Photo.JPG") && lisible("a.webp") && !lisible("dessin.svg") && !lisible("note.txt"));
    }

    /// Un BMP de 4 × 2 (rouge à gauche, bleu à droite) décodé par Android, réduit de
    /// moitié ; sans le décodeur (hors d'Android), rien à vérifier.
    #[test]
    fn android_decode_une_image() {
        if graphiques().is_err() {
            return;
        }
        let (l, h) = (4u32, 2u32);
        let ligne = (l * 3).div_ceil(4) * 4;
        let mut b = Vec::new();
        b.extend_from_slice(b"BM");
        b.extend_from_slice(&(54 + ligne * h).to_le_bytes());
        b.extend_from_slice(&[0, 0, 0, 0]);
        b.extend_from_slice(&54u32.to_le_bytes());
        b.extend_from_slice(&40u32.to_le_bytes());
        b.extend_from_slice(&(l as i32).to_le_bytes());
        b.extend_from_slice(&(h as i32).to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&24u16.to_le_bytes());
        b.extend_from_slice(&[0; 24]);
        for _ in 0..h {
            for x in 0..l {
                b.extend_from_slice(if x < 2 { &[0, 0, 255] } else { &[255, 0, 0] });
            }
            b.resize(b.len() + (ligne - l * 3) as usize, 0);
        }
        let f = std::env::temp_dir().join(format!("flow-image-{}.bmp", std::process::id()));
        std::fs::write(&f, &b).unwrap();
        let d = decoder(&f, 2).unwrap();
        let _ = std::fs::remove_file(&f);
        assert_eq!((d.largeur, d.hauteur, d.original), (2, 1, (4, 2)));
        assert_eq!(&d.pixels[0..4], &[255, 0, 0, 255]);
        assert_eq!(&d.pixels[4..8], &[0, 0, 255, 255]);
    }
}
