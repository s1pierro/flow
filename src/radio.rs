//! Lecture d'un flux réseau (web radio) : `libmediandk` ouvre l'adresse (HTTP, HTTPS,
//! démultiplexage MP3, AAC, Ogg…), le décodeur rend du PCM 16 bits, converti en f32
//! stéréo et poussé dans l'anneau de [`crate::audio`].
//!
//! Tout se passe dans un thread : l'ouverture d'un flux bloque plusieurs secondes.

use crate::audio::{aaudio, bibliotheque};
use crate::mixage::{commande, mixeur, service, Annonce, Voie};
use std::ffi::{c_char, c_void, CString};
use std::sync::atomic::{AtomicBool, Ordering};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

type Extracteur = c_void;
type Codec = c_void;
type Format = c_void;

#[repr(C)]
pub(crate) struct InfoTampon {
    offset: i32,
    size: i32,
    presentation_time_us: i64,
    flags: u32,
}

const AMEDIA_OK: i32 = 0;
const FIN_DE_FLUX: u32 = 4;
const SORTIE_FORMAT_CHANGE: isize = -2;

bibliotheque! { "libmediandk.so", Media,
    AMediaExtractor_new: fn() -> *mut Extracteur;
    AMediaExtractor_delete: fn(*mut Extracteur) -> i32;
    AMediaExtractor_setDataSource: fn(*mut Extracteur, *const c_char) -> i32;
    AMediaExtractor_setDataSourceFd: fn(*mut Extracteur, i32, i64, i64) -> i32;
    AMediaExtractor_getTrackCount: fn(*mut Extracteur) -> usize;
    AMediaExtractor_getTrackFormat: fn(*mut Extracteur, usize) -> *mut Format;
    AMediaExtractor_selectTrack: fn(*mut Extracteur, usize) -> i32;
    AMediaExtractor_readSampleData: fn(*mut Extracteur, *mut u8, usize) -> isize;
    AMediaExtractor_getSampleTime: fn(*mut Extracteur) -> i64;
    AMediaExtractor_advance: fn(*mut Extracteur) -> bool;
    AMediaFormat_delete: fn(*mut Format) -> i32;
    AMediaFormat_getString: fn(*mut Format, *const c_char, *mut *const c_char) -> bool;
    AMediaFormat_getInt32: fn(*mut Format, *const c_char, *mut i32) -> bool;
    AMediaCodec_createDecoderByType: fn(*const c_char) -> *mut Codec;
    AMediaCodec_configure: fn(*mut Codec, *const Format, *mut c_void, *mut c_void, u32) -> i32;
    AMediaCodec_start: fn(*mut Codec) -> i32;
    AMediaCodec_stop: fn(*mut Codec) -> i32;
    AMediaCodec_delete: fn(*mut Codec) -> i32;
    AMediaCodec_dequeueInputBuffer: fn(*mut Codec, i64) -> isize;
    AMediaCodec_getInputBuffer: fn(*mut Codec, usize, *mut usize) -> *mut u8;
    AMediaCodec_queueInputBuffer: fn(*mut Codec, usize, isize, usize, u64, u32) -> i32;
    AMediaCodec_dequeueOutputBuffer: fn(*mut Codec, *mut InfoTampon, i64) -> isize;
    AMediaCodec_getOutputBuffer: fn(*mut Codec, usize, *mut usize) -> *mut u8;
    AMediaCodec_getOutputFormat: fn(*mut Codec) -> *mut Format;
    AMediaCodec_releaseOutputBuffer: fn(*mut Codec, usize, bool) -> i32;
}

fn media() -> Result<&'static Media, String> {
    static M: OnceLock<Result<Media, String>> = OnceLock::new();
    M.get_or_init(|| unsafe { Media::charger() }).as_ref().map_err(|e| e.clone())
}

/// Appelé à chaque changement d'état, depuis le thread de lecture : l'application y
/// réveille son affichage.
static REVEIL: OnceLock<fn()> = OnceLock::new();

pub fn au_changement(f: fn()) {
    let _ = REVEIL.set(f);
}

pub fn reveiller() {
    if let Some(f) = REVEIL.get() {
        f();
    }
}

/// Change l'état d'une session, et l'annonce à la notification tant que la session
/// n'a pas été arrêtée (un thread lâché ne doit plus rien annoncer).
fn noter(etat: &Mutex<Etat>, e: Etat, url: &str, piste: &str, stop: &AtomicBool) {
    if !stop.load(Ordering::Relaxed) {
        let a = match &e {
            Etat::Lecture { .. } => Some(Some((service::LECTURE, e.detail()))),
            Etat::Erreur(m) => Some(Some((service::ERREUR, m.clone()))),
            Etat::Arret => Some(None),
            _ => None,
        };
        if let Some(a) = a {
            mixeur().annoncer(piste, a.map(|(etat, texte)| Annonce { etat, titre: titre(url).to_string(), texte }));
        }
    }
    if let Ok(mut g) = etat.lock() {
        *g = e;
    }
    reveiller();
}

/// La piste du mixeur où joue la radio.
pub const PISTE: &str = "Radio";
/// Celle où jouent les fichiers audio.
pub const PISTE_FICHIER: &str = "Fichier";

fn global(l: &'static OnceLock<Mutex<Lecteur>>, piste: &'static str, direct: bool, f: fn(i32)) -> MutexGuard<'static, Lecteur> {
    let m = l.get_or_init(|| {
        mixeur().ecouter(piste, f);
        Mutex::new(Lecteur { piste, direct, session: None, adresse: None, dernier: None, suspendu: false })
    });
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Le lecteur de la radio : un seul, partagé entre l'interface et les commandes de
/// la notification.
pub fn lecteur() -> MutexGuard<'static, Lecteur> {
    static L: OnceLock<Mutex<Lecteur>> = OnceLock::new();
    global(&L, PISTE, true, |c| lecteur().commander(c))
}

/// Le lecteur des fichiers audio.
pub fn lecteur_fichiers() -> MutexGuard<'static, Lecteur> {
    static L: OnceLock<Mutex<Lecteur>> = OnceLock::new();
    global(&L, PISTE_FICHIER, false, |c| lecteur_fichiers().commander(c))
}

/// Un chemin du système de fichiers, plutôt qu'une adresse réseau.
pub fn local(adresse: &str) -> bool {
    let a = adresse.trim();
    a.starts_with('/')
}

/// Ce qu'on montre d'une source : le nom du fichier, ou l'adresse raccourcie.
pub fn titre(adresse: &str) -> &str {
    if local(adresse) {
        adresse.trim().rsplit('/').next().unwrap_or(adresse)
    } else {
        courte(adresse)
    }
}

/// Où en est la lecture, pour l'affichage.
#[derive(Clone, Debug, PartialEq)]
pub enum Etat {
    Arret,
    Connexion,
    Lecture { codec: String, frequence: i32, canaux: i32 },
    Erreur(String),
}

impl Etat {
    /// Le format lu, en clair : « mpeg · 44.1 kHz · stéréo ».
    pub fn detail(&self) -> String {
        match self {
            Etat::Lecture { codec, frequence, canaux } => {
                let canaux = match canaux {
                    1 => "mono".to_string(),
                    2 => "stéréo".to_string(),
                    n => format!("{n} canaux"),
                };
                format!("{codec} · {:.1} kHz · {canaux}", *frequence as f32 / 1000.0)
            }
            Etat::Erreur(e) => e.clone(),
            _ => String::new(),
        }
    }
}

/// Une lecture en cours : son anneau, son état et son signal d'arrêt lui sont
/// propres, si bien qu'un thread lâché (bloqué dans l'ouverture du flux) ne gêne
/// jamais la lecture suivante.
struct Session {
    etat: Arc<Mutex<Etat>>,
    stop: Arc<AtomicBool>,
}

/// Un lecteur : au plus un flux à la fois, sur sa piste du mixeur.
pub struct Lecteur {
    piste: &'static str,
    /// Une source en direct (la radio) : la pause coupe le flux, la reprise le
    /// relance. Un fichier, lui, attend simplement que le mixeur reprenne.
    direct: bool,
    session: Option<Session>,
    pub adresse: Option<String>,
    /// Le dernier flux lancé, que la commande Lecture de la notification reprend.
    pub dernier: Option<String>,
    /// Coupé par la pause de la notification : la reprise le relance.
    suspendu: bool,
}

impl Lecteur {
    pub fn etat(&self) -> Etat {
        self.session.as_ref().and_then(|s| s.etat.lock().ok().map(|e| e.clone())).unwrap_or(Etat::Arret)
    }

    pub fn actif(&self) -> bool {
        matches!(self.etat(), Etat::Connexion | Etat::Lecture { .. })
    }

    /// Les commandes de la notification, reçues du mixeur.
    fn commander(&mut self, c: i32) {
        match c {
            commande::SUSPENDRE if self.direct && self.actif() => {
                self.couper();
                self.suspendu = true;
            }
            commande::JOUER if self.suspendu => self.reprendre(),
            commande::FERMER => self.arreter(),
            _ => {}
        }
    }

    pub fn jouer(&mut self, adresse: &str) {
        self.couper();
        let s = Session { etat: Arc::new(Mutex::new(Etat::Connexion)), stop: Arc::new(AtomicBool::new(false)) };
        let (etat, stop) = (s.etat.clone(), s.stop.clone());
        let url = adresse.trim().to_string();
        let (piste, direct) = (self.piste, self.direct);
        std::thread::spawn(move || {
            // setDataSource sur une adresse réseau exige un thread connu de Java
            let vm = crate::acces::vm();
            let attache = unsafe { crate::acces::attacher(vm) }.is_some();
            let fin = match unsafe { lire(&url, piste, &etat, &stop) } {
                Ok(()) if stop.load(Ordering::Relaxed) || !direct => Etat::Arret,
                // Un direct ne devrait jamais finir
                Ok(()) => Etat::Erreur("fin du flux".into()),
                Err(e) => Etat::Erreur(e),
            };
            noter(&etat, fin, &url, piste, &stop);
            if attache {
                unsafe { crate::acces::detacher(vm) };
            }
        });
        self.session = Some(s);
        let a = adresse.trim().to_string();
        self.suspendu = false;
        let texte = if self.direct { "Connexion…" } else { "Ouverture…" };
        mixeur().annoncer(self.piste, Some(Annonce { etat: service::CONNEXION, titre: titre(&a).to_string(), texte: texte.into() }));
        self.adresse = Some(a.clone());
        self.dernier = Some(a);
    }

    /// Coupe le flux. Le thread n'est pas attendu (il peut être bloqué dans
    /// l'ouverture du flux) : il se termine seul en voyant le signal.
    fn couper(&mut self) {
        if let Some(s) = self.session.take() {
            s.stop.store(true, Ordering::Relaxed);
        }
        self.adresse = None;
    }

    /// Arrête la lecture et retire son annonce.
    pub fn arreter(&mut self) {
        self.couper();
        self.suspendu = false;
        mixeur().annoncer(self.piste, None);
    }

    pub fn reprendre(&mut self) {
        if let Some(d) = self.dernier.clone() {
            self.jouer(&d);
        }
    }
}

impl Drop for Lecteur {
    fn drop(&mut self) {
        self.couper();
    }
}

fn cle(s: &str) -> CString {
    CString::new(s).unwrap()
}

unsafe fn entier(m: &Media, f: *mut Format, k: &str) -> Option<i32> {
    let mut v = 0;
    (m.AMediaFormat_getInt32)(f, cle(k).as_ptr(), &mut v).then_some(v)
}

unsafe fn chaine(m: &Media, f: *mut Format, k: &str) -> Option<String> {
    let mut p: *const c_char = std::ptr::null();
    ((m.AMediaFormat_getString)(f, cle(k).as_ptr(), &mut p) && !p.is_null()).then(|| std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned())
}

/// Libère les objets NDK quoi qu'il arrive.
struct Garde<'a> {
    m: &'a Media,
    ex: *mut Extracteur,
    codec: *mut Codec,
    format: *mut Format,
}

impl Drop for Garde<'_> {
    fn drop(&mut self) {
        unsafe {
            if !self.codec.is_null() {
                (self.m.AMediaCodec_stop)(self.codec);
                (self.m.AMediaCodec_delete)(self.codec);
            }
            if !self.format.is_null() {
                (self.m.AMediaFormat_delete)(self.format);
            }
            (self.m.AMediaExtractor_delete)(self.ex);
        }
    }
}

unsafe fn lire(url: &str, piste_mix: &str, etat: &Mutex<Etat>, stop: &AtomicBool) -> Result<(), String> {
    let m = media()?;
    aaudio()?;
    let ex = (m.AMediaExtractor_new)();
    if ex.is_null() {
        return Err("extracteur indisponible".into());
    }
    let mut g = Garde { m, ex, codec: std::ptr::null_mut(), format: std::ptr::null_mut() };
    // Un chemin local s'ouvre ici et passe par son descripteur ; une adresse réseau,
    // Android l'ouvre lui-même
    let _fichier;
    if local(url) {
        use std::os::fd::AsRawFd;
        let f = std::fs::File::open(url.trim()).map_err(|e| format!("fichier illisible : {e}"))?;
        let taille = f.metadata().map(|m| m.len()).unwrap_or(0) as i64;
        if (m.AMediaExtractor_setDataSourceFd)(ex, f.as_raw_fd(), 0, taille) != AMEDIA_OK {
            return Err("format inconnu".into());
        }
        _fichier = f;
    } else {
        let curl = CString::new(url.trim()).map_err(|_| "adresse invalide".to_string())?;
        if (m.AMediaExtractor_setDataSource)(ex, curl.as_ptr()) != AMEDIA_OK {
            return Err("flux injoignable ou format inconnu".into());
        }
    }
    if stop.load(Ordering::Relaxed) {
        return Ok(());
    }

    // Première piste audio
    let mut piste = None;
    for i in 0..(m.AMediaExtractor_getTrackCount)(ex) {
        let f = (m.AMediaExtractor_getTrackFormat)(ex, i);
        match chaine(m, f, "mime") {
            Some(mime) if mime.starts_with("audio/") => {
                piste = Some((i, mime));
                g.format = f;
                break;
            }
            _ => {
                (m.AMediaFormat_delete)(f);
            }
        }
    }
    let (i, mime) = piste.ok_or("aucune piste audio")?;
    (m.AMediaExtractor_selectTrack)(ex, i);
    let mut frequence = entier(m, g.format, "sample-rate").unwrap_or(44_100);
    let mut canaux = entier(m, g.format, "channel-count").unwrap_or(2);

    g.codec = (m.AMediaCodec_createDecoderByType)(cle(&mime).as_ptr());
    if g.codec.is_null() {
        return Err(format!("pas de décodeur pour {mime}"));
    }
    if (m.AMediaCodec_configure)(g.codec, g.format, std::ptr::null_mut(), std::ptr::null_mut(), 0) != AMEDIA_OK || (m.AMediaCodec_start)(g.codec) != AMEDIA_OK {
        return Err(format!("décodeur {mime} refusé"));
    }

    let codec = mime.trim_start_matches("audio/").to_string();
    // Une voie de la piste du lecteur, à la fréquence du flux
    let voie: Voie = mixeur().ouvrir(piste_mix, frequence as u32)?;
    let mut f32s: Vec<f32> = Vec::new();
    let mut entree_finie = false;
    let mut annonce = false;

    while !stop.load(Ordering::Relaxed) {
        // Entrée : un échantillon compressé du réseau vers le décodeur
        if !entree_finie {
            let idx = (m.AMediaCodec_dequeueInputBuffer)(g.codec, 10_000);
            if idx >= 0 {
                let mut taille = 0usize;
                let buf = (m.AMediaCodec_getInputBuffer)(g.codec, idx as usize, &mut taille);
                let n = (m.AMediaExtractor_readSampleData)(ex, buf, taille);
                if n < 0 {
                    (m.AMediaCodec_queueInputBuffer)(g.codec, idx as usize, 0, 0, 0, FIN_DE_FLUX);
                    entree_finie = true;
                } else {
                    let t = (m.AMediaExtractor_getSampleTime)(ex).max(0) as u64;
                    (m.AMediaCodec_queueInputBuffer)(g.codec, idx as usize, 0, n as usize, t, 0);
                    (m.AMediaExtractor_advance)(ex);
                }
            }
        }

        // Sortie : du PCM 16 bits vers l'anneau
        let mut info = InfoTampon { offset: 0, size: 0, presentation_time_us: 0, flags: 0 };
        let idx = (m.AMediaCodec_dequeueOutputBuffer)(g.codec, &mut info, 10_000);
        if idx == SORTIE_FORMAT_CHANGE {
            let f = (m.AMediaCodec_getOutputFormat)(g.codec);
            let (fr, ca) = (entier(m, f, "sample-rate").unwrap_or(frequence), entier(m, f, "channel-count").unwrap_or(canaux));
            (m.AMediaFormat_delete)(f);
            canaux = ca;
            frequence = fr;
            voie.regler_frequence(fr as u32);
        } else if idx >= 0 {
            let mut taille = 0usize;
            let buf = (m.AMediaCodec_getOutputBuffer)(g.codec, idx as usize, &mut taille);
            f32s.clear();
            if !buf.is_null() && info.size > 0 {
                let pcm = std::slice::from_raw_parts(buf.add(info.offset as usize) as *const i16, info.size as usize / 2);
                vers_stereo(pcm, canaux.max(1) as usize, &mut f32s);
            }
            (m.AMediaCodec_releaseOutputBuffer)(g.codec, idx as usize, false);
            // Le premier son décodé : la lecture commence vraiment
            if !annonce && !f32s.is_empty() {
                annonce = true;
                noter(etat, Etat::Lecture { codec: codec.clone(), frequence, canaux }, url, piste_mix, stop);
            }
            // Pousse tout, en attendant que le rappel fasse de la place
            voie.pousser(&f32s, stop);
            if info.flags & FIN_DE_FLUX != 0 {
                // Laisse finir ce qui reste dans l'anneau
                while voie.rempli() > 0 && !stop.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(50));
                }
                return Ok(());
            }
        }
    }
    Ok(())
}

/// Les derniers flux écoutés, du plus récent au plus ancien, dans un fichier texte à
/// une adresse par ligne (`#` pour commenter), qu'on peut modifier à la main.
#[derive(Default)]
pub struct Historique {
    pub chemin: Option<PathBuf>,
    pub liste: Vec<String>,
}

pub const HISTORIQUE: &str = "radios.txt";
const GARDES: usize = 20;

impl Historique {
    pub fn ouvrir(chemin: PathBuf) -> Historique {
        let liste = std::fs::read_to_string(&chemin).map(|t| Historique::lire(&t)).unwrap_or_default();
        Historique { chemin: Some(chemin), liste }
    }

    pub fn lire(t: &str) -> Vec<String> {
        let mut l: Vec<String> = Vec::new();
        for a in t.lines().map(str::trim).filter(|a| !a.is_empty() && !a.starts_with('#')) {
            if !l.iter().any(|b| b == a) && l.len() < GARDES {
                l.push(a.to_string());
            }
        }
        l
    }

    pub fn ecrire(&self) -> String {
        let mut t = String::from("# Flow : les derniers flux radio écoutés, du plus récent au plus ancien.\n# Une adresse par ligne ; les lignes commençant par # sont ignorées.\n");
        for a in &self.liste {
            t.push_str(a);
            t.push('\n');
        }
        t
    }

    /// Place l'adresse en tête et enregistre.
    pub fn ajouter(&mut self, adresse: &str) -> std::io::Result<()> {
        let a = adresse.trim();
        if a.is_empty() {
            return Ok(());
        }
        self.liste.retain(|b| b != a);
        self.liste.insert(0, a.to_string());
        self.liste.truncate(GARDES);
        self.enregistrer()
    }

    pub fn retirer(&mut self, i: usize) -> std::io::Result<()> {
        if i < self.liste.len() {
            self.liste.remove(i);
        }
        self.enregistrer()
    }

    fn enregistrer(&self) -> std::io::Result<()> {
        let Some(c) = &self.chemin else { return Ok(()) };
        if let Some(d) = c.parent() {
            std::fs::create_dir_all(d)?;
        }
        let tmp = c.with_extension("txt~");
        std::fs::write(&tmp, self.ecrire())?;
        std::fs::rename(tmp, c)
    }
}

/// Une adresse raccourcie pour l'affichage : sans le protocole ni la barre finale.
pub fn courte(adresse: &str) -> &str {
    let a = adresse.split_once("://").map_or(adresse, |(_, r)| r);
    a.trim_end_matches('/')
}

/// PCM 16 bits entrelacé, n canaux → f32 stéréo.
fn vers_stereo(pcm: &[i16], canaux: usize, out: &mut Vec<f32>) {
    out.clear();
    const K: f32 = 1.0 / 32768.0;
    for trame in pcm.chunks_exact(canaux) {
        let g = trame[0] as f32 * K;
        let d = if canaux > 1 { trame[1] as f32 * K } else { g };
        out.push(g);
        out.push(d);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn l_historique_garde_l_ordre_sans_doublon() {
        let mut h = Historique { chemin: None, liste: Historique::lire("# commentaire\nhttp://a\n\n  http://b  \nhttp://a\n") };
        assert_eq!(h.liste, ["http://a", "http://b"]);
        h.ajouter("http://b").unwrap();
        h.ajouter(" http://c ").unwrap();
        assert_eq!(h.liste, ["http://c", "http://b", "http://a"]);
        assert_eq!(Historique::lire(&h.ecrire()), h.liste);
        assert_eq!(courte("https://stream.exemple.fr/radio/"), "stream.exemple.fr/radio");
    }

    #[test]
    fn le_mono_est_doublé_et_le_surplus_de_canaux_ignoré() {
        let mut out = Vec::new();
        vers_stereo(&[16384, -32768], 1, &mut out);
        assert_eq!(out, [0.5, 0.5, -1.0, -1.0]);
        vers_stereo(&[16384, 0, 99, -16384, 0, 99], 3, &mut out);
        assert_eq!(out, [0.5, 0.0, -0.5, 0.0]);
    }
}
