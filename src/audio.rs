//! AAudio, chargée à l'exécution (dlopen) : rien à lier à la compilation ; et l'anneau
//! sans verrou par lequel un producteur (décodeur, générateur) passe ses échantillons
//! au rappel audio. La sortie elle-même et le mélange des voies sont dans
//! [`crate::mixage`].

use std::cell::UnsafeCell;
use std::ffi::{c_char, c_void, CStr};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;

pub(crate) type Builder = c_void;
pub(crate) type Stream = c_void;
pub(crate) type DataCb = unsafe extern "C" fn(*mut Stream, *mut c_void, *mut c_void, i32) -> i32;
pub(crate) type ErrorCb = unsafe extern "C" fn(*mut Stream, *mut c_void, i32);

pub(crate) const AAUDIO_OK: i32 = 0;
pub(crate) const AAUDIO_FORMAT_PCM_FLOAT: i32 = 2;
pub(crate) const AAUDIO_SHARING_MODE_SHARED: i32 = 1;
pub(crate) const AAUDIO_PERFORMANCE_MODE_POWER_SAVING: i32 = 11;
pub(crate) const AAUDIO_CALLBACK_RESULT_CONTINUE: i32 = 0;
pub(crate) const AAUDIO_USAGE_MEDIA: i32 = 1;
pub(crate) const AAUDIO_CONTENT_TYPE_MUSIC: i32 = 2;

extern "C" {
    pub(crate) fn dlopen(name: *const c_char, flags: i32) -> *mut c_void;
    pub(crate) fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
}

pub(crate) const RTLD_NOW: i32 = 2;

/// Une bibliothèque du système, chargée à l'exécution : une structure de pointeurs de
/// fonctions, remplie par `dlsym`.
macro_rules! bibliotheque {
    ($lib:literal, $nom:ident, $($f:ident : fn($($arg:ty),*) -> $ret:ty;)*) => {
        #[allow(non_snake_case)]
        pub(crate) struct $nom { $(pub(crate) $f: unsafe extern "C" fn($($arg),*) -> $ret,)* }

        impl $nom {
            unsafe fn charger() -> Result<$nom, String> {
                let lib = $crate::audio::dlopen(concat!($lib, "\0").as_ptr() as *const std::ffi::c_char, $crate::audio::RTLD_NOW);
                if lib.is_null() {
                    return Err(format!("{} introuvable", $lib));
                }
                Ok($nom { $($f: {
                    let p = $crate::audio::dlsym(lib, concat!(stringify!($f), "\0").as_ptr() as *const std::ffi::c_char);
                    if p.is_null() {
                        return Err(format!("symbole absent : {}", stringify!($f)));
                    }
                    std::mem::transmute::<*mut std::ffi::c_void, unsafe extern "C" fn($($arg),*) -> $ret>(p)
                },)* })
            }
        }
    };
}
pub(crate) use bibliotheque;

bibliotheque! { "libaaudio.so", Api,
    AAudio_createStreamBuilder: fn(*mut *mut Builder) -> i32;
    AAudio_convertResultToText: fn(i32) -> *const c_char;
    AAudioStreamBuilder_setPerformanceMode: fn(*mut Builder, i32) -> ();
    AAudioStreamBuilder_setSharingMode: fn(*mut Builder, i32) -> ();
    AAudioStreamBuilder_setFormat: fn(*mut Builder, i32) -> ();
    AAudioStreamBuilder_setChannelCount: fn(*mut Builder, i32) -> ();
    AAudioStreamBuilder_setUsage: fn(*mut Builder, i32) -> ();
    AAudioStreamBuilder_setContentType: fn(*mut Builder, i32) -> ();
    AAudioStreamBuilder_setDataCallback: fn(*mut Builder, Option<DataCb>, *mut c_void) -> ();
    AAudioStreamBuilder_setErrorCallback: fn(*mut Builder, Option<ErrorCb>, *mut c_void) -> ();
    AAudioStreamBuilder_openStream: fn(*mut Builder, *mut *mut Stream) -> i32;
    AAudioStreamBuilder_delete: fn(*mut Builder) -> i32;
    AAudioStream_requestStart: fn(*mut Stream) -> i32;
    AAudioStream_close: fn(*mut Stream) -> i32;
    AAudioStream_getSampleRate: fn(*mut Stream) -> i32;
}

pub(crate) fn aaudio() -> Result<&'static Api, String> {
    static API: OnceLock<Result<Api, String>> = OnceLock::new();
    API.get_or_init(|| unsafe { Api::charger() }).as_ref().map_err(|e| e.clone())
}

pub(crate) fn texte(api: &Api, code: i32) -> String {
    unsafe {
        let p = (api.AAudio_convertResultToText)(code);
        if p.is_null() {
            format!("code {code}")
        } else {
            CStr::from_ptr(p).to_string_lossy().into_owned()
        }
    }
}

/// Anneau d'échantillons à un producteur et un consommateur, sans verrou.
pub struct Anneau {
    tampon: Box<[UnsafeCell<f32>]>,
    /// Positions absolues (jamais ramenées) : lu ≤ écrit ≤ lu + capacité.
    ecrit: AtomicUsize,
    lu: AtomicUsize,
}

// Un seul thread écrit, un seul lit : chaque case n'est touchée que par l'un à la fois.
unsafe impl Sync for Anneau {}

impl Anneau {
    pub fn new(capacite: usize) -> Anneau {
        Anneau { tampon: (0..capacite.max(2)).map(|_| UnsafeCell::new(0.0)).collect(), ecrit: AtomicUsize::new(0), lu: AtomicUsize::new(0) }
    }

    pub fn capacite(&self) -> usize {
        self.tampon.len()
    }

    /// Échantillons en attente.
    pub fn rempli(&self) -> usize {
        self.ecrit.load(Ordering::Acquire) - self.lu.load(Ordering::Acquire)
    }

    /// Écrit ce qui tient ; renvoie le nombre d'échantillons écrits. Producteur seul.
    pub fn ecrire(&self, s: &[f32]) -> usize {
        let (e, l) = (self.ecrit.load(Ordering::Relaxed), self.lu.load(Ordering::Acquire));
        let n = s.len().min(self.capacite() - (e - l));
        for (i, x) in s[..n].iter().enumerate() {
            unsafe { *self.tampon[(e + i) % self.capacite()].get() = *x };
        }
        self.ecrit.store(e + n, Ordering::Release);
        n
    }

    /// Place libre, en échantillons.
    pub fn libre(&self) -> usize {
        self.capacite() - self.rempli()
    }

    /// Oublie le contenu. Seulement quand ni producteur ni consommateur n'y touchent.
    pub fn vider(&self) {
        self.lu.store(self.ecrit.load(Ordering::Acquire), Ordering::Release);
    }

    /// Une trame stéréo, s'il y en a une entière. Consommateur seul.
    pub fn lire_trame(&self) -> Option<[f32; 2]> {
        let (l, e) = (self.lu.load(Ordering::Relaxed), self.ecrit.load(Ordering::Acquire));
        if e - l < 2 {
            return None;
        }
        let c = self.capacite();
        let t = unsafe { [*self.tampon[l % c].get(), *self.tampon[(l + 1) % c].get()] };
        self.lu.store(l + 2, Ordering::Release);
        Some(t)
    }

    /// Remplit `out` ; le manque est complété par du silence. Renvoie ce qui a été lu.
    /// Consommateur seul.
    pub fn lire(&self, out: &mut [f32], gain: f32) -> usize {
        let (l, e) = (self.lu.load(Ordering::Relaxed), self.ecrit.load(Ordering::Acquire));
        let n = out.len().min(e - l);
        for (i, o) in out[..n].iter_mut().enumerate() {
            *o = unsafe { *self.tampon[(l + i) % self.capacite()].get() } * gain;
        }
        out[n..].fill(0.0);
        self.lu.store(l + n, Ordering::Release);
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn l_anneau_rend_ce_qu_on_y_met_puis_du_silence() {
        let a = Anneau::new(8);
        assert_eq!(a.ecrire(&[1., 2., 3., 4., 5., 6., 7., 8., 9.]), 8, "plein à 8");
        let mut out = [0.0; 5];
        assert_eq!(a.lire(&mut out, 1.0), 5);
        assert_eq!(out, [1., 2., 3., 4., 5.]);
        assert_eq!(a.ecrire(&[10., 11.]), 2);
        let mut out = [9.0; 6];
        assert_eq!(a.lire(&mut out, 0.5), 5);
        assert_eq!(out, [3., 3.5, 4., 5., 5.5, 0.]);
        assert_eq!(a.rempli(), 0);
        a.ecrire(&[1., 2., 3.]);
        assert_eq!(a.lire_trame(), Some([1., 2.]));
        assert_eq!(a.lire_trame(), None, "une demi-trame ne se lit pas");
        a.vider();
        assert_eq!((a.rempli(), a.libre()), (0, 8));
    }
}
