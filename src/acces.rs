//! Accès à tous les fichiers du stockage partagé, demandé à Android par JNI.
//!
//! - Android 11 et plus : l'autorisation « accès à tous les fichiers »
//!   (`MANAGE_EXTERNAL_STORAGE`) s'accorde dans les réglages du système. On vérifie
//!   `Environment.isExternalStorageManager()`, et sinon on ouvre la page de réglage de
//!   l'application.
//! - Android 8 à 10 : l'autorisation d'écrire sur le stockage partagé se demande par la
//!   fenêtre habituelle (`requestPermissions`).
//!
//! Tout se fait sur le thread principal, depuis les rappels de l'activité, avec le
//! `JNIEnv` qu'elle fournit. Pas de dépendance : les fonctions JNI sont prises dans
//! leur table, à leur rang fixé par `jni.h`. Les variantes `…A` reçoivent leurs
//! arguments en tableau de `jvalue` (8 octets ; l'APK ne vise que l'ARM 64 bits).

use std::ffi::{c_char, c_void, CStr, CString};
use std::ptr::null;

type Obj = *mut c_void;
type Val = u64;

// Rangs dans la table de JNINativeInterface (jni.h)
const FIND_CLASS: usize = 6;
const PUSH_LOCAL_FRAME: usize = 19;
const POP_LOCAL_FRAME: usize = 20;
const EXCEPTION_CLEAR: usize = 17;
const NEW_OBJECT_A: usize = 30;
const GET_OBJECT_CLASS: usize = 31;
const GET_METHOD_ID: usize = 33;
const CALL_OBJECT_METHOD_A: usize = 36;
const CALL_INT_METHOD_A: usize = 51;
const CALL_VOID_METHOD_A: usize = 63;
const GET_STATIC_METHOD_ID: usize = 113;
const CALL_STATIC_OBJECT_METHOD_A: usize = 116;
const CALL_STATIC_BOOLEAN_METHOD_A: usize = 119;
const GET_STRING_LENGTH: usize = 164;
const GET_STRING_CHARS: usize = 165;
const RELEASE_STRING_CHARS: usize = 166;
const NEW_STRING_UTF: usize = 167;
const GET_STRING_UTF_CHARS: usize = 169;
const RELEASE_STRING_UTF_CHARS: usize = 170;
const NEW_OBJECT_ARRAY: usize = 172;
const SET_OBJECT_ARRAY_ELEMENT: usize = 174;
const EXCEPTION_CHECK: usize = 228;

/// Android 11 : l'API où apparaît l'accès à tous les fichiers.
const R: i32 = 30;

struct Jni {
    env: *mut c_void,
}

impl Jni {
    /// La fonction de rang `i` de la table, vue avec le type `F`.
    unsafe fn f<F: Copy>(&self, i: usize) -> F {
        let table = *(self.env as *const *const *const c_void);
        let p = *table.add(i);
        std::mem::transmute_copy(&p)
    }

    /// Vrai si une exception Java est levée ; elle est alors effacée.
    unsafe fn exception(&self) -> bool {
        let check: unsafe extern "C" fn(*mut c_void) -> u8 = self.f(EXCEPTION_CHECK);
        let oui = check(self.env) != 0;
        if oui {
            let clear: unsafe extern "C" fn(*mut c_void) = self.f(EXCEPTION_CLEAR);
            clear(self.env);
        }
        oui
    }

    unsafe fn classe(&self, nom: &str) -> Option<Obj> {
        let c = CString::new(nom).ok()?;
        let find: unsafe extern "C" fn(*mut c_void, *const c_char) -> Obj = self.f(FIND_CLASS);
        let k = find(self.env, c.as_ptr());
        (!self.exception() && !k.is_null()).then_some(k)
    }

    unsafe fn classe_de(&self, o: Obj) -> Obj {
        let g: unsafe extern "C" fn(*mut c_void, Obj) -> Obj = self.f(GET_OBJECT_CLASS);
        g(self.env, o)
    }

    unsafe fn methode(&self, k: Obj, nom: &str, sig: &str, statique: bool) -> Option<Obj> {
        let (n, s) = (CString::new(nom).ok()?, CString::new(sig).ok()?);
        let g: unsafe extern "C" fn(*mut c_void, Obj, *const c_char, *const c_char) -> Obj = self.f(if statique { GET_STATIC_METHOD_ID } else { GET_METHOD_ID });
        let m = g(self.env, k, n.as_ptr(), s.as_ptr());
        (!self.exception() && !m.is_null()).then_some(m)
    }

    unsafe fn texte(&self, s: &str) -> Option<Obj> {
        let c = CString::new(s).ok()?;
        let g: unsafe extern "C" fn(*mut c_void, *const c_char) -> Obj = self.f(NEW_STRING_UTF);
        let o = g(self.env, c.as_ptr());
        (!self.exception() && !o.is_null()).then_some(o)
    }

    unsafe fn lire_texte(&self, o: Obj) -> Option<String> {
        if o.is_null() {
            return None;
        }
        let get: unsafe extern "C" fn(*mut c_void, Obj, *mut u8) -> *const c_char = self.f(GET_STRING_UTF_CHARS);
        let rel: unsafe extern "C" fn(*mut c_void, Obj, *const c_char) = self.f(RELEASE_STRING_UTF_CHARS);
        let p = get(self.env, o, std::ptr::null_mut());
        if p.is_null() {
            return None;
        }
        let s = CStr::from_ptr(p).to_string_lossy().into_owned();
        rel(self.env, o, p);
        Some(s)
    }

    /// Un texte Java lu en UTF-16 : contrairement à `lire_texte` (UTF-8 modifié), les
    /// caractères hors du plan de base (émojis) passent intacts.
    unsafe fn lire_texte16(&self, o: Obj) -> Option<String> {
        if o.is_null() {
            return None;
        }
        let len: unsafe extern "C" fn(*mut c_void, Obj) -> i32 = self.f(GET_STRING_LENGTH);
        let get: unsafe extern "C" fn(*mut c_void, Obj, *mut u8) -> *const u16 = self.f(GET_STRING_CHARS);
        let rel: unsafe extern "C" fn(*mut c_void, Obj, *const u16) = self.f(RELEASE_STRING_CHARS);
        let n = len(self.env, o).max(0) as usize;
        let p = get(self.env, o, std::ptr::null_mut());
        if p.is_null() {
            self.exception();
            return None;
        }
        let s = String::from_utf16_lossy(std::slice::from_raw_parts(p, n));
        rel(self.env, o, p);
        Some(s)
    }

    unsafe fn objet(&self, o: Obj, m: Obj, args: &[Val]) -> Option<Obj> {
        let c: unsafe extern "C" fn(*mut c_void, Obj, Obj, *const Val) -> Obj = self.f(CALL_OBJECT_METHOD_A);
        let r = c(self.env, o, m, args.as_ptr());
        (!self.exception() && !r.is_null()).then_some(r)
    }

    unsafe fn entier(&self, o: Obj, m: Obj, args: &[Val]) -> Option<i32> {
        let c: unsafe extern "C" fn(*mut c_void, Obj, Obj, *const Val) -> i32 = self.f(CALL_INT_METHOD_A);
        let r = c(self.env, o, m, args.as_ptr());
        (!self.exception()).then_some(r)
    }

    /// Vrai si l'appel s'est passé sans exception.
    unsafe fn rien(&self, o: Obj, m: Obj, args: &[Val]) -> bool {
        let c: unsafe extern "C" fn(*mut c_void, Obj, Obj, *const Val) = self.f(CALL_VOID_METHOD_A);
        c(self.env, o, m, args.as_ptr());
        !self.exception()
    }

    unsafe fn objet_statique(&self, k: Obj, m: Obj, args: &[Val]) -> Option<Obj> {
        let c: unsafe extern "C" fn(*mut c_void, Obj, Obj, *const Val) -> Obj = self.f(CALL_STATIC_OBJECT_METHOD_A);
        let r = c(self.env, k, m, args.as_ptr());
        (!self.exception() && !r.is_null()).then_some(r)
    }

    unsafe fn booleen_statique(&self, k: Obj, m: Obj) -> Option<bool> {
        let c: unsafe extern "C" fn(*mut c_void, Obj, Obj, *const Val) -> u8 = self.f(CALL_STATIC_BOOLEAN_METHOD_A);
        let r = c(self.env, k, m, null());
        (!self.exception()).then_some(r != 0)
    }

    unsafe fn nouveau(&self, k: Obj, sig: &str, args: &[Val]) -> Option<Obj> {
        let m = self.methode(k, "<init>", sig, false)?;
        let c: unsafe extern "C" fn(*mut c_void, Obj, Obj, *const Val) -> Obj = self.f(NEW_OBJECT_A);
        let r = c(self.env, k, m, args.as_ptr());
        (!self.exception() && !r.is_null()).then_some(r)
    }
}

const ECRIRE: &str = "android.permission.WRITE_EXTERNAL_STORAGE";

/// Vrai si l'application peut lire et écrire tout le stockage partagé.
///
/// # Safety
/// Sur le thread principal, avec le `JNIEnv` et l'activité de `ANativeActivity`.
pub unsafe fn accorde(env: *mut c_void, activite: Obj, sdk: i32) -> bool {
    let j = Jni { env };
    if sdk >= R {
        (|| {
            let k = j.classe("android/os/Environment")?;
            let m = j.methode(k, "isExternalStorageManager", "()Z", true)?;
            j.booleen_statique(k, m)
        })()
        .unwrap_or(false)
    } else {
        (|| {
            let m = j.methode(j.classe_de(activite), "checkSelfPermission", "(Ljava/lang/String;)I", false)?;
            let p = j.texte(ECRIRE)?;
            j.entier(activite, m, &[p as Val])
        })()
            == Some(0)
    }
}

/// Demande l'accès : la page de réglage d'Android 11 et plus, la fenêtre d'autorisation
/// avant. L'activité revient au premier plan ensuite : `accorde` dit alors la réponse.
///
/// # Safety
/// Comme `accorde`.
pub unsafe fn demander(env: *mut c_void, activite: Obj, sdk: i32) -> bool {
    let j = Jni { env };
    let ka = j.classe_de(activite);
    if sdk >= R {
        let ouvrir = |action: &str, pour_moi: bool| -> Option<()> {
            let ki = j.classe("android/content/Intent")?;
            let a = j.texte(action)?;
            let intent = if pour_moi {
                let nom = j.objet(activite, j.methode(ka, "getPackageName", "()Ljava/lang/String;", false)?, &[])?;
                let nom = j.lire_texte(nom)?;
                let ku = j.classe("android/net/Uri")?;
                let parse = j.methode(ku, "parse", "(Ljava/lang/String;)Landroid/net/Uri;", true)?;
                let uri = j.objet_statique(ku, parse, &[j.texte(&format!("package:{nom}"))? as Val])?;
                j.nouveau(ki, "(Ljava/lang/String;Landroid/net/Uri;)V", &[a as Val, uri as Val])?
            } else {
                j.nouveau(ki, "(Ljava/lang/String;)V", &[a as Val])?
            };
            let start = j.methode(ka, "startActivity", "(Landroid/content/Intent;)V", false)?;
            j.rien(activite, start, &[intent as Val]).then_some(())
        };
        // La page de l'application, ou à défaut la liste de toutes les applications
        ouvrir("android.settings.MANAGE_APP_ALL_FILES_ACCESS_PERMISSION", true).or_else(|| ouvrir("android.settings.MANAGE_ALL_FILES_ACCESS_PERMISSION", false)).is_some()
    } else {
        (|| {
            let ks = j.classe("java/lang/String")?;
            let nouveau: unsafe extern "C" fn(*mut c_void, i32, Obj, Obj) -> Obj = j.f(NEW_OBJECT_ARRAY);
            let tableau = nouveau(env, 1, ks, std::ptr::null_mut());
            if j.exception() || tableau.is_null() {
                return None;
            }
            let poser: unsafe extern "C" fn(*mut c_void, Obj, i32, Obj) = j.f(SET_OBJECT_ARRAY_ELEMENT);
            poser(env, tableau, 0, j.texte(ECRIRE)?);
            let m = j.methode(ka, "requestPermissions", "([Ljava/lang/String;I)V", false)?;
            j.rien(activite, m, &[tableau as Val, 1]).then_some(())
        })()
        .is_some()
    }
}

/// Le `JNIEnv` du thread courant, qu'on attache à la machine virtuelle s'il ne l'est
/// pas encore (le thread de rendu, par exemple).
///
/// # Safety
/// `vm` : le `JavaVM` de `ANativeActivity`.
/// La machine virtuelle Java du processus, notée au lancement : un thread de travail
/// s'y attache pour les API qui l'exigent (ouvrir un flux réseau par `libmediandk`).
static VM: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

pub fn noter_vm(vm: *mut c_void) {
    VM.store(vm as usize, std::sync::atomic::Ordering::Relaxed);
}

pub fn vm() -> *mut c_void {
    VM.load(std::sync::atomic::Ordering::Relaxed) as *mut c_void
}

/// Détache le thread courant de la machine virtuelle : obligatoire avant qu'un thread
/// attaché ne se termine, sinon Android arrête l'application.
pub unsafe fn detacher(vm: *mut c_void) {
    if vm.is_null() {
        return;
    }
    // JNIInvokeInterface : DetachCurrentThread est au rang 5
    let table = *(vm as *const *const *const c_void);
    let detach: unsafe extern "C" fn(*mut c_void) -> i32 = std::mem::transmute_copy(&*table.add(5));
    detach(vm);
}

pub unsafe fn attacher(vm: *mut c_void) -> Option<*mut c_void> {
    if vm.is_null() {
        return None;
    }
    // JNIInvokeInterface : AttachCurrentThread est au rang 4
    let table = *(vm as *const *const *const c_void);
    let attach: unsafe extern "C" fn(*mut c_void, *mut *mut c_void, *mut c_void) -> i32 = std::mem::transmute_copy(&*table.add(4));
    let mut env = std::ptr::null_mut();
    (attach(vm, &mut env, std::ptr::null_mut()) == 0 && !env.is_null()).then_some(env)
}

/// La batterie : son niveau en % et si elle charge, lus dans l'annonce collante
/// `ACTION_BATTERY_CHANGED` (sans s'y abonner).
///
/// # Safety
/// `env` attaché au thread courant ; `activite` : une référence globale (celle de
/// `ANativeActivity` l'est).
pub unsafe fn batterie(env: *mut c_void, activite: Obj) -> Option<(u8, bool)> {
    let j = Jni { env };
    // Les références locales sont libérées d'un coup : ce thread ne revient jamais à Java
    let push: unsafe extern "C" fn(*mut c_void, i32) -> i32 = j.f(PUSH_LOCAL_FRAME);
    if push(env, 16) != 0 {
        j.exception();
        return None;
    }
    let r = (|| {
        let kf = j.classe("android/content/IntentFilter")?;
        let filtre = j.nouveau(kf, "(Ljava/lang/String;)V", &[j.texte("android.intent.action.BATTERY_CHANGED")? as Val])?;
        let m = j.methode(j.classe_de(activite), "registerReceiver", "(Landroid/content/BroadcastReceiver;Landroid/content/IntentFilter;)Landroid/content/Intent;", false)?;
        let intent = j.objet(activite, m, &[0, filtre as Val])?;
        let get = j.methode(j.classe_de(intent), "getIntExtra", "(Ljava/lang/String;I)I", false)?;
        let lire = |cle: &str| -> Option<i32> { j.entier(intent, get, &[j.texte(cle)? as Val, (-1i32) as u32 as Val]) };
        let (niveau, echelle, etat) = (lire("level")?, lire("scale")?, lire("status")?);
        (niveau >= 0 && echelle > 0).then(|| (((niveau * 100) / echelle).clamp(0, 100) as u8, etat == 2 || etat == 5))
    })();
    let pop: unsafe extern "C" fn(*mut c_void, Obj) -> Obj = j.f(POP_LOCAL_FRAME);
    pop(env, std::ptr::null_mut());
    r
}

/// La racine du stockage partagé, par exemple `/storage/emulated/0`.
///
/// # Safety
/// Comme `accorde`.
pub unsafe fn racine(env: *mut c_void) -> Option<String> {
    let j = Jni { env };
    let k = j.classe("android/os/Environment")?;
    let m = j.methode(k, "getExternalStorageDirectory", "()Ljava/io/File;", true)?;
    let f = j.objet_statique(k, m, &[])?;
    let kf = j.classe("java/io/File")?;
    let chemin = j.objet(f, j.methode(kf, "getAbsolutePath", "()Ljava/lang/String;", false)?, &[])?;
    j.lire_texte(chemin)
}

const NEW_GLOBAL_REF: usize = 21;

/// Le contexte de l'application (référence globale), qui survit à l'activité : c'est
/// lui qui pilote le service de lecture.
static CONTEXTE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Note le contexte de l'application.
///
/// # Safety
/// Sur le thread principal, avec le `JNIEnv` et l'activité de `ANativeActivity`.
pub unsafe fn noter_contexte(env: *mut c_void, activite: Obj) {
    let j = Jni { env };
    let c = (|| j.objet(activite, j.methode(j.classe_de(activite), "getApplicationContext", "()Landroid/content/Context;", false)?, &[]))();
    if let Some(c) = c {
        let global: unsafe extern "C" fn(*mut c_void, Obj) -> Obj = j.f(NEW_GLOBAL_REF);
        CONTEXTE.store(global(env, c) as usize, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Envoie au service de lecture (`fr.s1pierro.flow.Lecture`) son état, un titre et
/// un texte : il se met au premier plan, montre la notification et ses commandes, ou
/// s'arrête pour l'état 0. Depuis n'importe quel thread connu de Java.
pub fn service(etat: i32, titre: &str, texte: &str) {
    let contexte = CONTEXTE.load(std::sync::atomic::Ordering::Relaxed) as Obj;
    if contexte.is_null() {
        return;
    }
    unsafe {
        // Un thread de passage (le gardien du mixeur) est attaché le temps de l'appel,
        // puis détaché : un thread qui se termine attaché arrête l'application
        let vm = vm();
        let deja = deja_attache(vm);
        let Some(env) = attacher(vm) else { return };
        let j = Jni { env };
        let push: unsafe extern "C" fn(*mut c_void, i32) -> i32 = j.f(PUSH_LOCAL_FRAME);
        if push(env, 16) != 0 {
            j.exception();
            return;
        }
        let ok = (|| {
            let kc = j.classe_de(contexte);
            let paquet = j.objet(contexte, j.methode(kc, "getPackageName", "()Ljava/lang/String;", false)?, &[])?;
            let ki = j.classe("android/content/Intent")?;
            let intent = j.nouveau(ki, "()V", &[])?;
            let classe = j.methode(ki, "setClassName", "(Ljava/lang/String;Ljava/lang/String;)Landroid/content/Intent;", false)?;
            j.objet(intent, classe, &[paquet as Val, j.texte("fr.s1pierro.flow.Lecture")? as Val])?;
            let entier = j.methode(ki, "putExtra", "(Ljava/lang/String;I)Landroid/content/Intent;", false)?;
            j.objet(intent, entier, &[j.texte("etat")? as Val, etat as u32 as Val])?;
            let chaine = j.methode(ki, "putExtra", "(Ljava/lang/String;Ljava/lang/String;)Landroid/content/Intent;", false)?;
            j.objet(intent, chaine, &[j.texte("titre")? as Val, j.texte(titre)? as Val])?;
            j.objet(intent, chaine, &[j.texte("texte")? as Val, j.texte(texte)? as Val])?;
            let lancer = j.methode(kc, "startForegroundService", "(Landroid/content/Intent;)Landroid/content/ComponentName;", false)?;
            j.objet(contexte, lancer, &[intent as Val])
        })();
        if ok.is_none() {
            crate::android::log("service de lecture injoignable");
        }
        let pop: unsafe extern "C" fn(*mut c_void, Obj) -> Obj = j.f(POP_LOCAL_FRAME);
        pop(env, std::ptr::null_mut());
        if !deja {
            detacher(vm);
        }
    }
}

/// Le texte du presse-papier d'Android (son premier élément, converti en texte), ou
/// rien s'il est vide, illisible, ou hors d'Android. Android 10 et plus ne le donnent
/// qu'à l'application au premier plan : c'est le cas quand on touche le clavier.
pub fn presse_papier() -> Option<String> {
    let contexte = CONTEXTE.load(std::sync::atomic::Ordering::Relaxed) as Obj;
    if contexte.is_null() {
        return None;
    }
    unsafe {
        let vm = vm();
        let deja = deja_attache(vm);
        let env = attacher(vm)?;
        let j = Jni { env };
        let push: unsafe extern "C" fn(*mut c_void, i32) -> i32 = j.f(PUSH_LOCAL_FRAME);
        if push(env, 16) != 0 {
            j.exception();
            return None;
        }
        let texte = (|| {
            let kc = j.classe("android/content/Context")?;
            let service = j.methode(kc, "getSystemService", "(Ljava/lang/String;)Ljava/lang/Object;", false)?;
            let presse = j.objet(contexte, service, &[j.texte("clipboard")? as Val])?;
            let km = j.classe("android/content/ClipboardManager")?;
            let clip = j.objet(presse, j.methode(km, "getPrimaryClip", "()Landroid/content/ClipData;", false)?, &[])?;
            let kd = j.classe("android/content/ClipData")?;
            if j.entier(clip, j.methode(kd, "getItemCount", "()I", false)?, &[])? < 1 {
                return None;
            }
            let element = j.objet(clip, j.methode(kd, "getItemAt", "(I)Landroid/content/ClipData$Item;", false)?, &[0])?;
            let ke = j.classe("android/content/ClipData$Item")?;
            let suite = j.objet(element, j.methode(ke, "coerceToText", "(Landroid/content/Context;)Ljava/lang/CharSequence;", false)?, &[contexte as Val])?;
            let ko = j.classe("java/lang/Object")?;
            let chaine = j.objet(suite, j.methode(ko, "toString", "()Ljava/lang/String;", false)?, &[])?;
            j.lire_texte16(chaine)
        })();
        let pop: unsafe extern "C" fn(*mut c_void, Obj) -> Obj = j.f(POP_LOCAL_FRAME);
        pop(env, std::ptr::null_mut());
        if !deja {
            detacher(vm);
        }
        texte.filter(|t| !t.is_empty())
    }
}

/// Vrai si le thread courant est déjà connu de la machine virtuelle.
unsafe fn deja_attache(vm: *mut c_void) -> bool {
    if vm.is_null() {
        return false;
    }
    // JNIInvokeInterface : GetEnv est au rang 6 ; JNI_VERSION_1_6
    let table = *(vm as *const *const *const c_void);
    let get: unsafe extern "C" fn(*mut c_void, *mut *mut c_void, i32) -> i32 = std::mem::transmute_copy(&*table.add(6));
    let mut env = std::ptr::null_mut();
    get(vm, &mut env, 0x0001_0006) == 0 && !env.is_null()
}
