//! Intégration Android : NativeActivity et thread de rendu.
//!
//! Le thread principal ne fait que transmettre les événements ; toute l'interface
//! vit sur le thread de rendu, qui dort tant qu'il n'y a rien à redessiner.

#![allow(non_camel_case_types)]

use crate::app::App;
use crate::draw::{DrawList, Rect};
use crate::gles::{load_gl, Egl};
use crate::reglages::{Fichier, NOM};
use crate::renderer::Renderer;
use crate::text::Text;
use crate::ui::{Event, Ui};
use crate::widgets::Cx;
use std::ffi::{c_void, CStr, CString};
use std::path::PathBuf;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex, Once, OnceLock};
use std::time::{Duration, Instant};

type ANativeWindow = c_void;
type AInputQueue = c_void;
type AInputEvent = c_void;
type ALooper = c_void;

#[repr(C)]
pub struct ANativeActivity {
    callbacks: *mut ANativeActivityCallbacks,
    vm: *mut c_void,
    env: *mut c_void,
    clazz: *mut c_void,
    internal_data_path: *const i8,
    external_data_path: *const i8,
    sdk_version: i32,
    instance: *mut c_void,
    asset_manager: *mut c_void,
    obb_path: *const i8,
}

type Cb = Option<unsafe extern "C" fn(*mut ANativeActivity)>;
type CbWindow = Option<unsafe extern "C" fn(*mut ANativeActivity, *mut ANativeWindow)>;
type CbQueue = Option<unsafe extern "C" fn(*mut ANativeActivity, *mut AInputQueue)>;

#[repr(C)]
pub struct ANativeActivityCallbacks {
    on_start: Cb,
    on_resume: Cb,
    on_save_instance_state: Option<unsafe extern "C" fn(*mut ANativeActivity, *mut usize) -> *mut c_void>,
    on_pause: Cb,
    on_stop: Cb,
    on_destroy: Cb,
    on_window_focus_changed: Option<unsafe extern "C" fn(*mut ANativeActivity, i32)>,
    on_native_window_created: CbWindow,
    on_native_window_resized: CbWindow,
    on_native_window_redraw_needed: CbWindow,
    on_native_window_destroyed: CbWindow,
    on_input_queue_created: CbQueue,
    on_input_queue_destroyed: CbQueue,
    on_content_rect_changed: Option<unsafe extern "C" fn(*mut ANativeActivity, *const c_void)>,
    on_configuration_changed: Cb,
    on_low_memory: Cb,
}

const AINPUT_EVENT_TYPE_MOTION: i32 = 2;
const ACTION_DOWN: i32 = 0;
const ACTION_UP: i32 = 1;
const ACTION_MOVE: i32 = 2;
const ACTION_CANCEL: i32 = 3;
const ACTION_POINTER_DOWN: i32 = 5;
const ACTION_POINTER_UP: i32 = 6;
const ACTION_MASK: i32 = 0xff;
const ACTION_POINTER_INDEX_MASK: i32 = 0xff00;
const ACTION_POINTER_INDEX_SHIFT: i32 = 8;

type ALooperCallback = unsafe extern "C" fn(fd: i32, events: i32, data: *mut c_void) -> i32;

#[link(name = "android")]
extern "C" {
    fn ALooper_forThread() -> *mut ALooper;
    fn AInputQueue_attachLooper(q: *mut AInputQueue, l: *mut ALooper, ident: i32, cb: Option<ALooperCallback>, data: *mut c_void);
    fn AInputQueue_detachLooper(q: *mut AInputQueue);
    fn AInputQueue_getEvent(q: *mut AInputQueue, out: *mut *mut AInputEvent) -> i32;
    fn AInputQueue_preDispatchEvent(q: *mut AInputQueue, e: *mut AInputEvent) -> i32;
    fn AInputQueue_finishEvent(q: *mut AInputQueue, e: *mut AInputEvent, handled: i32);
    fn AInputEvent_getType(e: *const AInputEvent) -> i32;
    fn AMotionEvent_getAction(e: *const AInputEvent) -> i32;
    fn AMotionEvent_getPointerCount(e: *const AInputEvent) -> usize;
    fn AMotionEvent_getPointerId(e: *const AInputEvent, pointer_index: usize) -> i32;
    fn AMotionEvent_getX(e: *const AInputEvent, pointer_index: usize) -> f32;
    fn AMotionEvent_getY(e: *const AInputEvent, pointer_index: usize) -> f32;
    fn AConfiguration_new() -> *mut c_void;
    fn AConfiguration_fromAssetManager(c: *mut c_void, am: *mut c_void);
    fn AConfiguration_getDensity(c: *const c_void) -> i32;
    fn AConfiguration_delete(c: *mut c_void);
}

#[link(name = "log")]
extern "C" {
    fn __android_log_write(prio: i32, tag: *const i8, text: *const i8) -> i32;
}

pub fn log(s: &str) {
    let text = CString::new(s).unwrap_or_default();
    unsafe {
        __android_log_write(6, c"Flow".as_ptr() as *const _, text.as_ptr() as *const _);
    }
}

// ---------------------------------------------------------------------------
// État partagé entre le thread principal et le thread de rendu
// ---------------------------------------------------------------------------

struct Shared {
    window: *mut ANativeWindow,
    window_changed: bool,
    /// Le rendu a lâché l'ancienne fenêtre : Android peut la détruire.
    released: bool,
    redraw: bool,
    events: Vec<(Event, f64)>,
    dp: f32,
    /// Le dossier de l'application : les réglages de l'interface y vivent tant que
    /// l'accès à tous les fichiers n'est pas accordé.
    dossier: Option<PathBuf>,
    /// L'accès à tous les fichiers est accordé, et la racine du stockage partagé.
    acces: bool,
    racine: Option<PathBuf>,
    /// Relire les réglages (retour dans l'application, accès changé).
    relire: bool,
    /// La machine virtuelle et l'activité, pour interroger la batterie depuis le
    /// thread de rendu.
    vm: usize,
    activite: usize,
}
unsafe impl Send for Shared {}

static SHARED: Mutex<Shared> =
    Mutex::new(Shared { window: null_mut(), window_changed: false, released: true, redraw: false, events: Vec::new(), dp: 1.0, dossier: None, acces: false, racine: None, relire: true, vm: 0, activite: 0 });
static WAKE: Condvar = Condvar::new();
static START: OnceLock<Instant> = OnceLock::new();

fn now() -> f64 {
    START.get_or_init(Instant::now).elapsed().as_secs_f64()
}

/// Redessine dès que possible : appelé par les threads de travail (la radio).
fn reveiller() {
    if let Ok(mut s) = SHARED.lock() {
        s.redraw = true;
    }
    WAKE.notify_all();
}

fn push(events: &[Event]) {
    let t = now();
    let mut s = SHARED.lock().unwrap();
    s.events.extend(events.iter().map(|e| (*e, t)));
    WAKE.notify_all();
}

// ---------------------------------------------------------------------------
// Thread de rendu
// ---------------------------------------------------------------------------

fn render_loop() -> Result<(), String> {
    let mut egl = Egl::new(false)?;
    let mut gl = Some(load_gl(false)?);
    let mut renderer: Option<Renderer> = None;
    let mut app: Option<(App, Ui)> = None;
    let mut text = Text::new();
    let mut dl = DrawList::default();
    let mut last = Instant::now();
    let mut again = false; // la dernière image demande à être suivie d'une autre
    let mut fichier: Option<Fichier> = None;
    // L'espace de travail, à côté des réglages ; enregistré au plus toutes les quelques
    // secondes pendant une animation
    let mut espace: Option<crate::espace::Fichier> = None;
    let mut sauve = Instant::now();
    // L'heure et la batterie de la barre : relues à chaque minute, et l'écran redessiné
    let (vm, activite) = {
        let s = SHARED.lock().unwrap();
        (s.vm as *mut c_void, s.activite as *mut c_void)
    };
    let env = unsafe { crate::acces::attacher(vm) };
    let mut minute = None;
    let mut relire = true;

    loop {
        let (window, changed, events, dp, chemin, racine) = {
            let mut s = SHARED.lock().unwrap();
            loop {
                let drawing = !egl.surface.is_null();
                let busy = drawing && (again || app.as_ref().is_some_and(|(a, _)| a.animating()));
                if s.window_changed || s.redraw || busy || (drawing && !s.events.is_empty()) || (drawing && minute != Some(heure().1)) {
                    break;
                }
                // Réveil à la minute suivante, pour l'heure de la barre
                let reste = 60 - heure().2 as u64;
                s = WAKE.wait_timeout(s, Duration::from_secs(reste.max(1))).unwrap().0;
            }
            s.redraw = false;
            let changed = std::mem::take(&mut s.window_changed);
            relire |= std::mem::take(&mut s.relire);
            // Les réglages : dans le stockage partagé si l'accès est accordé, sinon dans
            // le dossier de l'application
            let chemin = match (&s.racine, s.acces) {
                (Some(r), true) => Some(r.join("Flow").join(NOM)),
                _ => s.dossier.as_ref().map(|d| d.join(NOM)),
            };
            // Sans l'accès, l'explorateur y montre comment l'accorder
            let racine = s.racine.clone();
            (s.window, changed, std::mem::take(&mut s.events), s.dp, chemin, racine)
        };
        if fichier.as_ref().map(|f| &f.chemin) != chemin.as_ref() {
            // Le nouveau fichier part des réglages actuels s'il n'existe pas encore
            espace = chemin.as_ref().and_then(|c| c.parent()).map(|d| crate::espace::Fichier::new(d.join(crate::espace::NOM)));
            fichier = chemin.map(Fichier::new);
            relire = true;
        }
        again = false;

        if changed {
            if !egl.surface.is_null() {
                egl.detach();
            }
            if window.is_null() {
                // L'application passe en arrière-plan : l'espace est enregistré tel quel
                if let (Some((a, _)), Some(f)) = (app.as_ref(), espace.as_mut()) {
                    if let Err(e) = f.suivre(&a.espace()) {
                        log(&format!("espace non enregistré : {e}"));
                    }
                }
                SHARED.lock().unwrap().released = true;
                WAKE.notify_all();
                continue;
            }
            egl.attach_window(window)?;
            if let Some(api) = gl.take() {
                renderer = Some(Renderer::new(api)?);
            }
            last = Instant::now();
            // Au retour dans l'application, les réglages ont pu être modifiés à la main
            relire = true;
        }
        if egl.surface.is_null() {
            continue;
        }
        let (Some(render), (app, ui)) = (renderer.as_ref(), app.get_or_insert_with(|| (App::new(dp), Ui::new(dp)))) else { continue };
        let h = heure();
        if minute != Some(h.1) {
            minute = Some(h.1);
            app.heure = Some((h.0, h.1));
            if let Some(env) = env {
                app.batterie = unsafe { crate::acces::batterie(env, activite) };
            }
        }
        if std::mem::take(&mut relire) {
            if let Some(r) = fichier.as_mut().and_then(|f| f.relire()) {
                app.appliquer(r);
            }
            app.commun.fichier_reglages = fichier.as_ref().map(|f| f.chemin.display().to_string());
            app.commun.fichier_espace = espace.as_ref().map(|f| f.chemin.display().to_string());
            // L'espace de travail, s'il a changé sur le disque (au premier lancement, ou
            // modifié à la main) ; sinon les fenêtres relisent ce qui a pu changer
            if let Some(f) = espace.as_mut() {
                if let Some(t) = f.relire() {
                    app.ouvrir_espace(&t);
                    f.connaitre(app.espace());
                }
            }
            app.racine_en(racine.clone());
            // L'historique des radios vit à côté des réglages de l'interface
            if let Some(d) = fichier.as_ref().and_then(|f| f.chemin.parent()) {
                app.historique_en(d.join(crate::radio::HISTORIQUE));
            }
        }

        for (e, t) in events {
            ui.event(e, t);
        }
        let dt = last.elapsed().as_secs_f32().min(0.05);
        last = Instant::now();

        let (w, h) = egl.size();
        dl.clear();
        let mut cx = Cx { ui, text: &mut text, dl: &mut dl, dp, theme: Default::default() };
        let scene = app.frame(&mut cx, Rect::new(0.0, 0.0, w, h), dt);
        ui.end_frame();
        // Les réglages changés s'enregistrent une fois le doigt levé, pas à chaque pas
        // d'un curseur
        if ui.doigts() == 0 {
            if let Some(Err(e)) = fichier.as_mut().map(|f| f.suivre(&app.reglages())) {
                log(&format!("réglages non enregistrés : {e}"));
                fichier = None;
            }
            // L'espace aussi, mais au plus toutes les 5 s tant qu'une fenêtre s'anime
            // (une animation peut changer l'état à chaque image)
            if !app.animating() || sauve.elapsed() > Duration::from_secs(5) {
                sauve = Instant::now();
                if let Some(Err(e)) = espace.as_mut().map(|f| f.suivre(&app.espace())) {
                    log(&format!("espace non enregistré : {e}"));
                    espace = None;
                }
            }
        }
        render.draw(w, h, scene.fond, &dl, &mut text);
        egl.swap();
        // Après une rotation, la nouvelle taille n'est connue qu'ici ; et un appui change
        // l'état après le dessin du widget, qu'il faut redessiner à jour.
        again = egl.size() != (w, h) || ui.a_redessiner();
    }
}

#[repr(C)]
struct Tm {
    sec: i32,
    min: i32,
    hour: i32,
    mday: i32,
    mon: i32,
    year: i32,
    wday: i32,
    yday: i32,
    isdst: i32,
    gmtoff: i64,
    zone: *const i8,
}

extern "C" {
    fn time(t: *mut i64) -> i64;
    fn localtime_r(t: *const i64, tm: *mut Tm) -> *mut Tm;
}

/// L'heure locale : heures, minutes, secondes.
fn heure() -> (u8, u8, u8) {
    unsafe {
        let t = time(null_mut());
        let mut tm: Tm = std::mem::zeroed();
        if localtime_r(&t, &mut tm).is_null() {
            return (0, 0, 0);
        }
        (tm.hour as u8, tm.min as u8, tm.sec.clamp(0, 59) as u8)
    }
}

fn spawn_render_thread() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        std::thread::spawn(|| {
            if let Err(e) = render_loop() {
                log(&e);
                // Personne ne doit rester bloqué sur une fenêtre à lâcher.
                SHARED.lock().unwrap().released = true;
                WAKE.notify_all();
            }
        });
    });
}

// ---------------------------------------------------------------------------
// Callbacks de l'activité
// ---------------------------------------------------------------------------

unsafe extern "C" fn on_window_created(_: *mut ANativeActivity, window: *mut ANativeWindow) {
    let mut s = SHARED.lock().unwrap();
    s.window = window;
    s.window_changed = true;
    s.released = false;
    WAKE.notify_all();
}

unsafe extern "C" fn on_window_changed(_: *mut ANativeActivity, _: *mut ANativeWindow) {
    SHARED.lock().unwrap().redraw = true;
    WAKE.notify_all();
}

/// Android détruit la fenêtre au retour : le rendu doit l'avoir lâchée avant.
unsafe extern "C" fn on_window_destroyed(_: *mut ANativeActivity, _: *mut ANativeWindow) {
    let mut s = SHARED.lock().unwrap();
    s.window = null_mut();
    s.window_changed = true;
    WAKE.notify_all();
    let (_guard, _) = WAKE.wait_timeout_while(s, Duration::from_secs(2), |s| !s.released).unwrap();
}

unsafe extern "C" fn on_input(_fd: i32, _events: i32, data: *mut c_void) -> i32 {
    let queue = data as *mut AInputQueue;
    let mut event: *mut AInputEvent = null_mut();
    while AInputQueue_getEvent(queue, &mut event) >= 0 {
        if AInputQueue_preDispatchEvent(queue, event) != 0 {
            continue;
        }
        let mut handled = 0;
        if AInputEvent_getType(event) == AINPUT_EVENT_TYPE_MOTION {
            handled = 1;
            let action = AMotionEvent_getAction(event);
            let idx = ((action & ACTION_POINTER_INDEX_MASK) >> ACTION_POINTER_INDEX_SHIFT) as usize;
            let id = |i: usize| AMotionEvent_getPointerId(event, i);
            match action & ACTION_MASK {
                ACTION_DOWN | ACTION_POINTER_DOWN => {
                    push(&[Event::Down { id: id(idx), x: AMotionEvent_getX(event, idx), y: AMotionEvent_getY(event, idx) }])
                }
                ACTION_MOVE => {
                    let moves: Vec<Event> = (0..AMotionEvent_getPointerCount(event))
                        .map(|i| Event::Move { id: id(i), x: AMotionEvent_getX(event, i), y: AMotionEvent_getY(event, i) })
                        .collect();
                    push(&moves);
                }
                ACTION_UP | ACTION_POINTER_UP => push(&[Event::Up { id: id(idx) }]),
                ACTION_CANCEL => push(&[Event::Cancel]),
                _ => handled = 0,
            }
        }
        AInputQueue_finishEvent(queue, event, handled);
    }
    1
}

/// Au premier plan : l'accès à tous les fichiers est-il accordé ? Sinon, il est demandé
/// une fois par lancement. La réponse arrive au retour suivant.
unsafe extern "C" fn on_resume(activity: *mut ANativeActivity) {
    static DEMANDE: AtomicBool = AtomicBool::new(false);
    let a = &*activity;
    let acces = crate::acces::accorde(a.env, a.clazz, a.sdk_version);
    if !acces && !DEMANDE.swap(true, Ordering::Relaxed) && !crate::acces::demander(a.env, a.clazz, a.sdk_version) {
        log("accès à tous les fichiers : demande impossible");
    }
    let racine = crate::acces::racine(a.env).map(PathBuf::from);
    let mut s = SHARED.lock().unwrap();
    s.acces = acces;
    s.racine = racine.or_else(|| Some(PathBuf::from("/storage/emulated/0")));
    s.relire = true;
    WAKE.notify_all();
}

unsafe extern "C" fn on_queue_created(_: *mut ANativeActivity, queue: *mut AInputQueue) {
    AInputQueue_attachLooper(queue, ALooper_forThread(), 1, Some(on_input), queue);
}

unsafe extern "C" fn on_queue_destroyed(_: *mut ANativeActivity, queue: *mut AInputQueue) {
    AInputQueue_detachLooper(queue);
}

/// Un pixel indépendant de la densité (dp), en pixels réels.
unsafe fn density(activity: *mut ANativeActivity) -> f32 {
    let c = AConfiguration_new();
    AConfiguration_fromAssetManager(c, (*activity).asset_manager);
    let d = AConfiguration_getDensity(c);
    AConfiguration_delete(c);
    // 0 : non précisée, 0xFFFF / 0xFFFE : valeurs spéciales
    if (1..=1000).contains(&d) {
        d as f32 / 160.0
    } else {
        2.0
    }
}

/// Point d'entrée appelé par `android.app.NativeActivity`.
///
/// # Safety
/// Appelé uniquement par Android, avec une activité valide.
#[no_mangle]
pub unsafe extern "C" fn ANativeActivity_onCreate(activity: *mut ANativeActivity, _: *mut c_void, _: usize) {
    let mut s = SHARED.lock().unwrap();
    s.dp = density(activity);
    s.vm = (*activity).vm as usize;
    crate::acces::noter_vm((*activity).vm as *mut c_void);
    crate::radio::au_changement(reveiller);
    s.activite = (*activity).clazz as usize;
    // Le dossier externe de l'application, qu'on peut ouvrir depuis un gestionnaire de
    // fichiers ; à défaut, le dossier interne
    let dossier = |p: *const i8| (!p.is_null()).then(|| PathBuf::from(CStr::from_ptr(p as *const _).to_string_lossy().into_owned()));
    s.dossier = dossier((*activity).external_data_path).or_else(|| dossier((*activity).internal_data_path));
    drop(s);
    crate::acces::noter_contexte((*activity).env, (*activity).clazz);
    let cb = &mut *(*activity).callbacks;
    cb.on_resume = Some(on_resume);
    cb.on_native_window_created = Some(on_window_created);
    cb.on_native_window_redraw_needed = Some(on_window_changed);
    cb.on_native_window_resized = Some(on_window_changed);
    cb.on_native_window_destroyed = Some(on_window_destroyed);
    cb.on_input_queue_created = Some(on_queue_created);
    cb.on_input_queue_destroyed = Some(on_queue_destroyed);
    spawn_render_thread();
}
