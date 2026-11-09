//! Le mixeur : une seule sortie AAudio pour toute l'application, où se mélangent des
//! voies (une radio, un son généré, plus tard un fichier ou une vidéo).
//!
//! - Chaque voie a son anneau, à la fréquence de sa source : le rappel la convertit à
//!   celle de l'appareil (interpolation linéaire).
//! - Les réglages vivent par **piste**, c'est-à-dire par nom (« Radio », « Essai ») :
//!   gain, balance, muet, solo, prioritaire. Ils s'appliquent à toute voie de ce nom et
//!   restent connus quand aucune ne joue.
//! - Une voie prioritaire qui sonne baisse les autres.
//! - Tout changement de gain glisse sur ~10 ms : ni clic à l'ouverture, ni à la
//!   fermeture, ni en bougeant un curseur.
//! - Un limiteur doux évite la saturation quand les voies se cumulent.
//!
//! - Le mixeur tient le service de lecture (`Lecture.java`) : tant qu'une voie joue ou
//!   qu'une source s'annonce, l'application reste vivante hors de l'écran, avec une
//!   notification. Elle montre l'annonce la plus récente (titre, texte, état). Pause
//!   y met tout le mixeur en pause : la sortie se tait sans rien consommer, si bien
//!   qu'un fichier reprend où il en était ; les sources en direct sont prévenues et
//!   coupent leur flux. Fermer prévient toutes les sources.
//!
//! Le rappel audio n'alloue rien, ne prend aucun verrou et ne fait aucun appel système.
//! Les voies sont des cases fixes, réservées par compare-and-swap. Un gardien (un
//! thread) rouvre la sortie si le système la coupe, et la ferme quand plus rien ne joue.

use crate::audio::*;
use std::cell::UnsafeCell;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

/// Nombre de voies simultanées.
pub const VOIES: usize = 8;
/// Capacité d'un anneau : 2 s de stéréo à 48 kHz.
const CAPACITE: usize = 48_000 * 2 * 2;
/// Durée du glissement des gains, en secondes.
const GLISSE: f32 = 0.010;
/// Gain des autres voies quand une prioritaire sonne.
const BAISSE: f32 = 0.25;

const LIBRE: u8 = 0;
const RESERVEE: u8 = 1;
const ACTIVE: u8 = 2;
const FIN: u8 = 3;

/// Une valeur f32 partagée sans verrou.
struct F(AtomicU32);

impl F {
    const fn new(v: f32) -> F {
        F(AtomicU32::new(v.to_bits()))
    }
    fn get(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }
    fn set(&self, v: f32) {
        self.0.store(v.to_bits(), Ordering::Relaxed)
    }
}

/// Les réglages d'une piste.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reglage {
    /// 0 à 1, suivant une courbe (l'oreille entend en logarithme).
    pub gain: f32,
    /// -1 (gauche) à 1 (droite).
    pub balance: f32,
    pub muet: bool,
    pub solo: bool,
    /// Quand elle sonne, les autres baissent.
    pub prioritaire: bool,
}

impl Default for Reglage {
    fn default() -> Self {
        Reglage { gain: 0.8, balance: 0.0, muet: false, solo: false, prioritaire: false }
    }
}

/// Ce que seul le rappel audio touche.
#[derive(Default)]
struct Rt {
    neuve: bool,
    frac: f32,
    avant: [f32; 2],
    apres: [f32; 2],
    g: [f32; 2],
}

struct Case {
    etat: AtomicU8,
    generation: AtomicU32,
    anneau: OnceLock<Anneau>,
    frequence: AtomicU32,
    gain: F,
    balance: F,
    muet: AtomicBool,
    solo: AtomicBool,
    prioritaire: AtomicBool,
    crete: [F; 2],
    rt: UnsafeCell<Rt>,
    /// Le nom de la piste : jamais lu par le rappel.
    nom: Mutex<String>,
}

impl Case {
    fn new() -> Case {
        Case {
            etat: AtomicU8::new(LIBRE),
            generation: AtomicU32::new(0),
            anneau: OnceLock::new(),
            frequence: AtomicU32::new(48_000),
            gain: F::new(1.0),
            balance: F::new(0.0),
            muet: AtomicBool::new(false),
            solo: AtomicBool::new(false),
            prioritaire: AtomicBool::new(false),
            crete: [F::new(0.0), F::new(0.0)],
            rt: UnsafeCell::new(Rt::default()),
            nom: Mutex::new(String::new()),
        }
    }

    fn anneau(&self) -> &Anneau {
        self.anneau.get_or_init(|| Anneau::new(CAPACITE))
    }

    fn appliquer(&self, r: &Reglage) {
        self.gain.set(r.gain);
        self.balance.set(r.balance);
        self.muet.store(r.muet, Ordering::Relaxed);
        self.solo.store(r.solo, Ordering::Relaxed);
        self.prioritaire.store(r.prioritaire, Ordering::Relaxed);
    }
}

/// La sortie ouverte.
struct Flux(*mut Stream);
unsafe impl Send for Flux {}

pub struct Mixeur {
    cases: [Case; VOIES],
    /// Réglages par piste, dans l'ordre d'apparition.
    pistes: Mutex<Vec<(String, Reglage)>>,
    sortie: Mutex<Option<Flux>>,
    frequence: AtomicU32,
    coupe: AtomicBool,
    pub general: F2,
    crete: [F; 2],
    baisse: F,
    /// Une erreur d'ouverture, à montrer.
    erreur: Mutex<Option<String>>,
    /// En pause (depuis la notification) : la sortie se tait et ne consomme rien.
    pause: AtomicBool,
    /// Ce que les sources annoncent, par piste, de la plus ancienne à la plus récente.
    annonces: Mutex<Vec<(String, Annonce)>>,
    /// Les sources qui veulent être prévenues des commandes de la notification.
    ecoutes: Mutex<Vec<(String, fn(i32))>>,
    /// Le dernier état envoyé au service, pour n'envoyer que les changements.
    publie: Mutex<Option<Annonce>>,
}

/// Les états du service de lecture (`Lecture.java`).
pub mod service {
    pub const ARRET: i32 = 0;
    pub const CONNEXION: i32 = 1;
    pub const LECTURE: i32 = 2;
    pub const PAUSE: i32 = 3;
    pub const ERREUR: i32 = 4;
}

/// Les commandes de la notification, transmises aux sources.
pub mod commande {
    pub const JOUER: i32 = 1;
    pub const SUSPENDRE: i32 = 2;
    pub const FERMER: i32 = 3;
}

/// Ce qu'une source dit d'elle pour la notification.
#[derive(Clone, Debug, PartialEq)]
pub struct Annonce {
    pub etat: i32,
    pub titre: String,
    pub texte: String,
}

/// Le gain général (exposé à l'interface).
pub struct F2(F);

impl F2 {
    pub fn get(&self) -> f32 {
        self.0.get()
    }
    pub fn set(&self, v: f32) {
        self.0.set(v.clamp(0.0, 1.0))
    }
}

unsafe impl Sync for Mixeur {}

pub fn mixeur() -> &'static Mixeur {
    static M: OnceLock<Mixeur> = OnceLock::new();
    M.get_or_init(|| Mixeur {
        cases: std::array::from_fn(|_| Case::new()),
        pistes: Mutex::new(Vec::new()),
        sortie: Mutex::new(None),
        frequence: AtomicU32::new(48_000),
        coupe: AtomicBool::new(false),
        general: F2(F::new(1.0)),
        crete: [F::new(0.0), F::new(0.0)],
        baisse: F::new(1.0),
        erreur: Mutex::new(None),
        pause: AtomicBool::new(false),
        annonces: Mutex::new(Vec::new()),
        ecoutes: Mutex::new(Vec::new()),
        publie: Mutex::new(None),
    })
}

/// Les commandes de la notification, appelées par `Lecture.commande` sur le thread
/// principal.
#[no_mangle]
pub extern "C" fn Java_fr_s1pierro_flow_Lecture_commande(_env: *mut c_void, _classe: *mut c_void, c: i32) {
    mixeur().commander(c);
    crate::radio::reveiller();
}

/// Ce que l'interface montre d'une piste.
#[derive(Clone, Debug)]
pub struct Piste {
    pub nom: String,
    pub reglage: Reglage,
    /// Crêtes gauche et droite, si une voie de ce nom joue.
    pub niveau: Option<[f32; 2]>,
    /// Remplissage de son anneau, de 0 à 1.
    pub tampon: f32,
}

impl Mixeur {
    /// Ouvre une voie pour la piste `nom`, à la fréquence de la source. La sortie
    /// s'ouvre au besoin. La voie se referme (en fondu) quand elle est lâchée.
    pub fn ouvrir(&'static self, nom: &str, frequence: u32) -> Result<Voie, String> {
        let reglage = self.reglage(nom);
        let i = (0..VOIES)
            .find(|&i| self.cases[i].etat.compare_exchange(LIBRE, RESERVEE, Ordering::AcqRel, Ordering::Relaxed).is_ok())
            .ok_or("plus de voie libre")?;
        let c = &self.cases[i];
        // Réservée : le rappel n'y touche pas, on peut tout remettre à zéro
        c.anneau().vider();
        unsafe { *c.rt.get() = Rt { neuve: true, ..Rt::default() } };
        c.frequence.store(frequence.max(1), Ordering::Relaxed);
        c.appliquer(&reglage);
        c.crete[0].set(0.0);
        c.crete[1].set(0.0);
        *c.nom.lock().unwrap() = nom.to_string();
        let generation = c.generation.fetch_add(1, Ordering::AcqRel) + 1;
        c.etat.store(ACTIVE, Ordering::Release);
        // Une source qui démarre sort le mixeur de la pause
        self.pause.store(false, Ordering::Release);
        if let Err(e) = self.demarrer() {
            c.etat.store(LIBRE, Ordering::Release);
            return Err(e);
        }
        self.publier();
        Ok(Voie { m: self, i, generation })
    }

    /// Une source dit ce qu'elle joue (ou se retire avec `None`) : la notification suit.
    /// Une source qui se connecte ou joue sort le mixeur de la pause.
    pub fn annoncer(&'static self, piste: &str, a: Option<Annonce>) {
        {
            let mut l = self.annonces.lock().unwrap();
            l.retain(|(p, _)| p != piste);
            if let Some(a) = a {
                if a.etat == service::CONNEXION || a.etat == service::LECTURE {
                    self.reprendre_sortie();
                }
                l.push((piste.to_string(), a));
            }
        }
        self.publier();
    }

    /// Une source demande à recevoir les commandes de la notification.
    pub fn ecouter(&self, piste: &str, f: fn(i32)) {
        let mut l = self.ecoutes.lock().unwrap();
        l.retain(|(p, _)| p != piste);
        l.push((piste.to_string(), f));
    }

    pub fn en_pause(&self) -> bool {
        self.pause.load(Ordering::Acquire)
    }

    fn reprendre_sortie(&'static self) {
        self.pause.store(false, Ordering::Release);
        if self.actif() {
            let _ = self.demarrer();
        }
    }

    /// Lecture, pause ou fermer, depuis la notification.
    pub fn commander(&'static self, c: i32) {
        match c {
            commande::SUSPENDRE => self.pause.store(true, Ordering::Release),
            commande::JOUER => self.reprendre_sortie(),
            // Sortie rouverte au besoin : les voies lâchées en pause finissent leur fondu
            commande::FERMER => self.reprendre_sortie(),
            _ => return,
        }
        // Les sources sont prévenues hors du verrou : elles peuvent annoncer en retour
        let l: Vec<fn(i32)> = self.ecoutes.lock().unwrap().iter().map(|(_, f)| *f).collect();
        for f in l {
            f(c);
        }
        if c == commande::FERMER {
            self.annonces.lock().unwrap().clear();
        }
        self.publier();
    }

    /// Envoie au service ce qu'il doit montrer, si cela a changé.
    pub fn publier(&self) {
        let derniere = self.annonces.lock().unwrap().last().map(|(_, a)| a.clone());
        let voies = self.cases.iter().filter(|c| c.etat.load(Ordering::Acquire) == ACTIVE).count();
        let a = if self.en_pause() && (derniere.is_some() || voies > 0) {
            Annonce { etat: service::PAUSE, titre: derniere.map_or("Flow".into(), |a| a.titre), texte: "En pause".into() }
        } else if let Some(a) = derniere {
            a
        } else if voies > 0 {
            Annonce { etat: service::LECTURE, titre: "Flow".into(), texte: if voies == 1 { "1 voie".into() } else { format!("{voies} voies") } }
        } else {
            Annonce { etat: service::ARRET, titre: String::new(), texte: String::new() }
        };
        let mut p = self.publie.lock().unwrap();
        // Rien à arrêter si rien n'a jamais été montré
        if p.as_ref() == Some(&a) || (p.is_none() && a.etat == service::ARRET) {
            return;
        }
        crate::acces::service(a.etat, &a.titre, &a.texte);
        *p = (a.etat != service::ARRET).then_some(a);
    }

    /// Les réglages connus d'une piste (déclarée au besoin).
    pub fn reglage(&self, nom: &str) -> Reglage {
        let mut p = self.pistes.lock().unwrap();
        if let Some((_, r)) = p.iter().find(|(n, _)| n == nom) {
            return *r;
        }
        p.push((nom.to_string(), Reglage::default()));
        Reglage::default()
    }

    /// Change les réglages d'une piste, et de ses voies qui jouent.
    pub fn regler(&self, nom: &str, r: Reglage) {
        {
            let mut p = self.pistes.lock().unwrap();
            match p.iter_mut().find(|(n, _)| n == nom) {
                Some(e) => e.1 = r,
                None => p.push((nom.to_string(), r)),
            }
        }
        for c in &self.cases {
            if c.etat.load(Ordering::Acquire) == ACTIVE && *c.nom.lock().unwrap() == nom {
                c.appliquer(&r);
            }
        }
    }

    pub fn pistes(&self) -> Vec<Piste> {
        let p = self.pistes.lock().unwrap().clone();
        p.into_iter()
            .map(|(nom, reglage)| {
                let c = self.cases.iter().find(|c| c.etat.load(Ordering::Acquire) == ACTIVE && *c.nom.lock().unwrap() == nom);
                Piste {
                    niveau: c.map(|c| [c.crete[0].get(), c.crete[1].get()]),
                    tampon: c.map_or(0.0, |c| c.anneau().rempli() as f32 / CAPACITE as f32),
                    nom,
                    reglage,
                }
            })
            .collect()
    }

    /// Crêtes de la sortie, si elle est ouverte.
    pub fn niveau(&self) -> Option<[f32; 2]> {
        self.sortie.lock().unwrap().is_some().then(|| [self.crete[0].get(), self.crete[1].get()])
    }

    pub fn frequence(&self) -> u32 {
        self.frequence.load(Ordering::Relaxed)
    }

    pub fn erreur(&self) -> Option<String> {
        self.erreur.lock().unwrap().clone()
    }

    /// Vrai si une voie joue : l'interface anime alors ses vumètres.
    pub fn actif(&self) -> bool {
        self.cases.iter().any(|c| c.etat.load(Ordering::Acquire) != LIBRE)
    }

    /// Ouvre la sortie si elle ne l'est pas, avec son gardien.
    fn demarrer(&'static self) -> Result<(), String> {
        let mut s = self.sortie.lock().unwrap();
        if s.is_some() {
            return Ok(());
        }
        let r = unsafe { self.ouvrir_flux() };
        *self.erreur.lock().unwrap() = r.as_ref().err().cloned();
        *s = Some(r?);
        std::thread::spawn(move || self.garder());
        Ok(())
    }

    unsafe fn ouvrir_flux(&'static self) -> Result<Flux, String> {
        let api = aaudio()?;
        let mut b: *mut Builder = std::ptr::null_mut();
        let r = (api.AAudio_createStreamBuilder)(&mut b);
        if r != AAUDIO_OK {
            return Err(format!("AAudio : {}", texte(api, r)));
        }
        let user = self as *const Mixeur as *mut c_void;
        (api.AAudioStreamBuilder_setPerformanceMode)(b, AAUDIO_PERFORMANCE_MODE_POWER_SAVING);
        (api.AAudioStreamBuilder_setSharingMode)(b, AAUDIO_SHARING_MODE_SHARED);
        (api.AAudioStreamBuilder_setFormat)(b, AAUDIO_FORMAT_PCM_FLOAT);
        (api.AAudioStreamBuilder_setChannelCount)(b, 2);
        // Pas de fréquence imposée : celle de l'appareil, sans conversion par Android
        (api.AAudioStreamBuilder_setUsage)(b, AAUDIO_USAGE_MEDIA);
        (api.AAudioStreamBuilder_setContentType)(b, AAUDIO_CONTENT_TYPE_MUSIC);
        (api.AAudioStreamBuilder_setDataCallback)(b, Some(rappel), user);
        (api.AAudioStreamBuilder_setErrorCallback)(b, Some(erreur), user);
        let mut flux: *mut Stream = std::ptr::null_mut();
        let r = (api.AAudioStreamBuilder_openStream)(b, &mut flux);
        (api.AAudioStreamBuilder_delete)(b);
        if r != AAUDIO_OK {
            return Err(format!("AAudio : {}", texte(api, r)));
        }
        self.frequence.store((api.AAudioStream_getSampleRate)(flux).max(8_000) as u32, Ordering::Relaxed);
        let r = (api.AAudioStream_requestStart)(flux);
        if r != AAUDIO_OK {
            (api.AAudioStream_close)(flux);
            return Err(format!("AAudio : {}", texte(api, r)));
        }
        Ok(Flux(flux))
    }

    /// Le gardien : rouvre la sortie coupée par le système (casque débranché…), la
    /// ferme après quelques secondes de silence complet.
    fn garder(&'static self) {
        let mut repos = 0;
        loop {
            std::thread::sleep(Duration::from_millis(250));
            // Les voies libérées par le rappel : la notification suit
            self.publier();
            let mut s = self.sortie.lock().unwrap();
            let Some(flux) = s.as_ref() else { return };
            let api = match aaudio() {
                Ok(a) => a,
                Err(_) => return,
            };
            if self.coupe.swap(false, Ordering::AcqRel) {
                unsafe { (api.AAudioStream_close)(flux.0) };
                *s = None;
                let r = unsafe { self.ouvrir_flux() };
                *self.erreur.lock().unwrap() = r.as_ref().err().cloned();
                match r {
                    Ok(f) => *s = Some(f),
                    Err(_) => return,
                }
            }
            // Rien ne joue, ou tout est en pause : la sortie se ferme après 4 s
            repos = if self.actif() && !self.en_pause() { 0 } else { repos + 1 };
            if repos >= 16 {
                // close() attend la fin du rappel
                unsafe { (api.AAudioStream_close)(s.take().unwrap().0) };
                self.crete[0].set(0.0);
                self.crete[1].set(0.0);
                drop(s);
                self.publier();
                return;
            }
        }
    }

    /// Le mélange, appelé par AAudio.
    fn melanger(&self, out: &mut [f32]) {
        out.fill(0.0);
        let frames = out.len() / 2;
        let fs = self.frequence.load(Ordering::Relaxed) as f32;
        let k = 1.0 - (-1.0 / (GLISSE * fs)).exp();
        // Décroissance des crêtes : environ 20 dB par seconde
        let declin = 0.1f32.powf(frames as f32 / fs);
        if self.pause.load(Ordering::Acquire) {
            // En pause : silence, et les anneaux restent où ils en sont
            for c in &self.cases {
                for ch in 0..2 {
                    c.crete[ch].set(c.crete[ch].get() * declin);
                }
            }
            for ch in 0..2 {
                self.crete[ch].set(self.crete[ch].get() * declin);
            }
            return;
        }
        let actives = |c: &&Case| c.etat.load(Ordering::Acquire) >= ACTIVE;
        let solo = self.cases.iter().filter(actives).any(|c| c.solo.load(Ordering::Relaxed));
        let prio = self.cases.iter().filter(actives).any(|c| c.prioritaire.load(Ordering::Relaxed) && c.crete[0].get().max(c.crete[1].get()) > 0.01);
        let baisse = if prio { BAISSE } else { 1.0 };
        self.baisse.set(baisse);
        for c in self.cases.iter().filter(actives) {
            let fin = c.etat.load(Ordering::Acquire) == FIN;
            let rt = unsafe { &mut *c.rt.get() };
            let a = c.anneau();
            let p = c.prioritaire.load(Ordering::Relaxed);
            let audible = !fin && !c.muet.load(Ordering::Relaxed) && (!solo || c.solo.load(Ordering::Relaxed));
            let g = if audible { c.gain.get().powi(2) * if p { 1.0 } else { baisse } } else { 0.0 };
            let b = c.balance.get().clamp(-1.0, 1.0);
            let cible = [g * (1.0 - b).min(1.0), g * (1.0 + b).min(1.0)];
            if rt.neuve {
                rt.neuve = false;
                rt.frac = 1.0;
            }
            let pas = c.frequence.load(Ordering::Relaxed) as f32 / fs;
            let mut crete = [0.0f32; 2];
            for o in out.chunks_exact_mut(2) {
                rt.frac += pas;
                while rt.frac >= 1.0 {
                    rt.avant = rt.apres;
                    rt.apres = a.lire_trame().unwrap_or([0.0, 0.0]);
                    rt.frac -= 1.0;
                }
                for ch in 0..2 {
                    rt.g[ch] += (cible[ch] - rt.g[ch]) * k;
                    let x = rt.avant[ch] + (rt.apres[ch] - rt.avant[ch]) * rt.frac;
                    let y = x * rt.g[ch];
                    o[ch] += y;
                    crete[ch] = crete[ch].max(y.abs());
                }
            }
            for ch in 0..2 {
                c.crete[ch].set(crete[ch].max(c.crete[ch].get() * declin));
            }
            if fin && rt.g[0] < 1e-4 && rt.g[1] < 1e-4 {
                c.etat.store(LIBRE, Ordering::Release);
            }
        }
        let g = self.general.get().powi(2);
        let mut crete = [0.0f32; 2];
        for o in out.chunks_exact_mut(2) {
            for ch in 0..2 {
                o[ch] = limiter(o[ch] * g);
                crete[ch] = crete[ch].max(o[ch].abs());
            }
        }
        for ch in 0..2 {
            self.crete[ch].set(crete[ch].max(self.crete[ch].get() * declin));
        }
    }
}

/// Limiteur doux : transparent jusqu'à 0,8, puis une courbe qui tend vers 1.
pub fn limiter(x: f32) -> f32 {
    const SEUIL: f32 = 0.8;
    let a = x.abs();
    if a <= SEUIL {
        x
    } else {
        let r = 1.0 - SEUIL;
        x.signum() * (SEUIL + r * ((a - SEUIL) / r).tanh())
    }
}

unsafe extern "C" fn rappel(_: *mut Stream, user: *mut c_void, audio: *mut c_void, frames: i32) -> i32 {
    let m = &*(user as *const Mixeur);
    m.melanger(std::slice::from_raw_parts_mut(audio as *mut f32, frames as usize * 2));
    AAUDIO_CALLBACK_RESULT_CONTINUE
}

unsafe extern "C" fn erreur(_: *mut Stream, user: *mut c_void, _: i32) {
    // Interdit de fermer le flux ici : le gardien s'en charge
    (*(user as *const Mixeur)).coupe.store(true, Ordering::Release);
}

/// Une voie ouverte, tenue par son producteur. Lâchée, elle s'éteint en fondu.
pub struct Voie {
    m: &'static Mixeur,
    i: usize,
    generation: u32,
}

impl Voie {
    fn case(&self) -> &Case {
        &self.m.cases[self.i]
    }

    /// Écrit des échantillons stéréo entrelacés ; renvoie ce qui a tenu.
    pub fn ecrire(&self, s: &[f32]) -> usize {
        self.case().anneau().ecrire(s)
    }

    /// Écrit tout, en attendant la place ; s'interrompt si `stop` devient vrai.
    pub fn pousser(&self, s: &[f32], stop: &AtomicBool) {
        let mut fait = 0;
        while fait < s.len() && !stop.load(Ordering::Relaxed) {
            fait += self.ecrire(&s[fait..]);
            if fait < s.len() {
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }

    pub fn rempli(&self) -> usize {
        self.case().anneau().rempli()
    }

    pub fn regler_frequence(&self, f: u32) {
        self.case().frequence.store(f.max(1), Ordering::Relaxed);
    }

    /// La fréquence de la sortie, que la source n'a pas besoin d'atteindre.
    pub fn frequence_sortie(&self) -> u32 {
        self.m.frequence()
    }
}

impl Drop for Voie {
    fn drop(&mut self) {
        let c = self.case();
        if c.generation.load(Ordering::Acquire) == self.generation {
            let _ = c.etat.compare_exchange(ACTIVE, FIN, Ordering::AcqRel, Ordering::Relaxed);
        }
    }
}

/// La voie d'essai : un arpège doux en sinus, généré dans un thread, sur la piste
/// « Essai ». Une seule à la fois ; Fermer dans la notification l'arrête.
struct Essai {
    stop: std::sync::Arc<AtomicBool>,
}

pub const ESSAI: &str = "Essai";

fn essai_global() -> std::sync::MutexGuard<'static, Option<Essai>> {
    static E: Mutex<Option<Essai>> = Mutex::new(None);
    E.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn essai_actif() -> bool {
    essai_global().is_some()
}

/// Lance ou arrête la voie d'essai.
pub fn essai(oui: bool) -> Result<(), String> {
    let m = mixeur();
    if !oui {
        if let Some(e) = essai_global().take() {
            e.stop.store(true, Ordering::Relaxed);
            m.annoncer(ESSAI, None);
        }
        return Ok(());
    }
    if essai_actif() {
        return Ok(());
    }
    const F: u32 = 48_000;
    m.ecouter(ESSAI, |c| {
        if c == commande::FERMER {
            let _ = essai(false);
        }
    });
    let voie = m.ouvrir(ESSAI, F)?;
    let stop = std::sync::Arc::new(AtomicBool::new(false));
    let s = stop.clone();
    std::thread::spawn(move || {
        // La, do, mi, la, mi, do : une note par quart de seconde
        const NOTES: [f32; 6] = [220.0, 261.63, 329.63, 440.0, 329.63, 261.63];
        let duree = F as usize / 4;
        let mut bloc = vec![0.0f32; 1024 * 2];
        let mut t = 0usize;
        while !s.load(Ordering::Relaxed) {
            for (k, o) in bloc.chunks_exact_mut(2).enumerate() {
                let n = (t + k) / duree;
                let dans = ((t + k) % duree) as f32 / F as f32;
                let f = NOTES[n % NOTES.len()];
                let env = (dans * 200.0).min(1.0) * (-dans * 9.0).exp();
                let x = (std::f32::consts::TAU * f * dans).sin() * env * 0.5;
                // Les notes alternent un peu à gauche, un peu à droite
                let d = if n % 2 == 0 { 0.35 } else { -0.35 };
                o[0] = x * (1.0 - d);
                o[1] = x * (1.0 + d);
            }
            t += 1024;
            voie.pousser(&bloc, &s);
        }
    });
    *essai_global() = Some(Essai { stop });
    m.annoncer(ESSAI, Some(Annonce { etat: service::LECTURE, titre: "Voie d'essai".into(), texte: "Arpège la, do, mi".into() }));
    Ok(())
}

/// Une crête en position de vumètre, de 0 (−48 dB et moins) à 1 (0 dB).
pub fn vumetre(crete: f32) -> f32 {
    if crete <= 0.0 {
        return 0.0;
    }
    ((20.0 * crete.log10() + 48.0) / 48.0).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mixeur_seul() -> &'static Mixeur {
        Box::leak(Box::new(Mixeur {
            cases: std::array::from_fn(|_| Case::new()),
            pistes: Mutex::new(Vec::new()),
            sortie: Mutex::new(None),
            frequence: AtomicU32::new(48_000),
            coupe: AtomicBool::new(false),
            general: F2(F::new(1.0)),
            crete: [F::new(0.0), F::new(0.0)],
            baisse: F::new(1.0),
            erreur: Mutex::new(None),
            pause: AtomicBool::new(false),
            annonces: Mutex::new(Vec::new()),
            ecoutes: Mutex::new(Vec::new()),
            publie: Mutex::new(None),
        }))
    }

    /// Une voie active sans ouvrir AAudio.
    fn voie(m: &'static Mixeur, nom: &str, f: u32) -> Voie {
        let r = m.reglage(nom);
        let i = (0..VOIES).find(|&i| m.cases[i].etat.compare_exchange(LIBRE, RESERVEE, Ordering::AcqRel, Ordering::Relaxed).is_ok()).unwrap();
        let c = &m.cases[i];
        unsafe { *c.rt.get() = Rt { neuve: true, ..Rt::default() } };
        c.frequence.store(f, Ordering::Relaxed);
        c.appliquer(&r);
        *c.nom.lock().unwrap() = nom.into();
        let generation = c.generation.fetch_add(1, Ordering::AcqRel) + 1;
        c.etat.store(ACTIVE, Ordering::Release);
        Voie { m, i, generation }
    }

    #[test]
    fn deux_voies_s_additionnent_et_le_gain_glisse() {
        let m = mixeur_seul();
        m.regler("A", Reglage { gain: 1.0, ..Reglage::default() });
        m.regler("B", Reglage { gain: 1.0, ..Reglage::default() });
        let (a, b) = (voie(m, "A", 48_000), voie(m, "B", 48_000));
        a.ecrire(&[0.25; 9600]);
        b.ecrire(&[0.25; 9600]);
        let mut out = vec![0.0; 4000];
        m.melanger(&mut out);
        assert!(out[0].abs() < 0.05, "pas de clic à l'ouverture : {}", out[0]);
        assert!((out[3998] - 0.5).abs() < 0.01, "{}", out[3998]);
        assert_eq!(m.pistes().len(), 2);
    }

    #[test]
    fn muet_solo_et_balance() {
        let m = mixeur_seul();
        m.regler("A", Reglage { gain: 1.0, balance: -1.0, ..Reglage::default() });
        m.regler("B", Reglage { gain: 1.0, ..Reglage::default() });
        let (a, b) = (voie(m, "A", 48_000), voie(m, "B", 48_000));
        a.ecrire(&[0.25; 20000]);
        b.ecrire(&[0.25; 20000]);
        let mut out = vec![0.0; 4000];
        m.melanger(&mut out);
        assert!((out[3998] - 0.5).abs() < 0.01 && (out[3999] - 0.25).abs() < 0.01, "A toute à gauche");
        m.regler("B", Reglage { gain: 1.0, solo: true, ..Reglage::default() });
        m.melanger(&mut out);
        assert!((out[3998] - 0.25).abs() < 0.01, "B en solo : A se tait");
        m.regler("B", Reglage { gain: 1.0, muet: true, ..Reglage::default() });
        m.melanger(&mut out);
        assert!((out[3998] - 0.25).abs() < 0.01 && out[3999].abs() < 0.01, "B muette : reste A à gauche");
    }

    #[test]
    fn une_voie_lachee_s_eteint_en_fondu_puis_libere_sa_case() {
        let m = mixeur_seul();
        m.regler("A", Reglage { gain: 1.0, ..Reglage::default() });
        let a = voie(m, "A", 48_000);
        a.ecrire(&[0.5; 40000]);
        let mut out = vec![0.0; 4000];
        m.melanger(&mut out);
        let i = a.i;
        drop(a);
        m.melanger(&mut out);
        assert!(out[0] > 0.3 && out[3998].abs() < 0.01, "fondu : {} → {}", out[0], out[3998]);
        for _ in 0..3 {
            m.melanger(&mut out);
        }
        assert_eq!(m.cases[i].etat.load(Ordering::Acquire), LIBRE);
        assert!(!m.actif());
    }

    #[test]
    fn en_pause_la_sortie_se_tait_sans_rien_consommer() {
        let m = mixeur_seul();
        let a = voie(m, "A", 48_000);
        a.ecrire(&[0.5; 2000]);
        m.pause.store(true, Ordering::Release);
        let mut out = vec![1.0; 400];
        m.melanger(&mut out);
        assert!(out.iter().all(|x| *x == 0.0));
        assert_eq!(a.rempli(), 2000);
    }

    #[test]
    fn la_frequence_de_la_source_est_convertie() {
        let m = mixeur_seul();
        m.regler("A", Reglage { gain: 1.0, ..Reglage::default() });
        let a = voie(m, "A", 24_000);
        a.ecrire(&[0.1; 2000]);
        let mut out = vec![0.0; 1000];
        m.melanger(&mut out);
        // 500 trames de sortie à 48 kHz consomment 250 trames à 24 kHz, plus celle
        // d'avance que garde l'interpolation
        assert_eq!(a.rempli(), 2000 - 2 * 251);
    }

    #[test]
    fn la_voie_prioritaire_baisse_les_autres_et_le_limiteur_borne() {
        let m = mixeur_seul();
        m.regler("A", Reglage { gain: 1.0, ..Reglage::default() });
        m.regler("P", Reglage { gain: 1.0, prioritaire: true, ..Reglage::default() });
        let (a, p) = (voie(m, "A", 48_000), voie(m, "P", 48_000));
        a.ecrire(&[0.4; 40000]);
        p.ecrire(&[0.2; 40000]);
        let mut out = vec![0.0; 4000];
        m.melanger(&mut out);
        m.melanger(&mut out);
        assert!((out[3998] - (0.4 * BAISSE + 0.2)).abs() < 0.01, "{}", out[3998]);
        assert!(limiter(3.0) < 1.0 && limiter(0.5) == 0.5 && limiter(-3.0) > -1.0);
        assert_eq!(vumetre(1.0), 1.0);
        assert_eq!(vumetre(0.0), 0.0);
    }
}
