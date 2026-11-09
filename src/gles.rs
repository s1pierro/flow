//! EGL et OpenGL ES 3 chargés à l'exécution (dlopen).
//!
//! Lier l'APK aux libEGL de Termux serait une erreur : leur nom de chargement
//! (libEGL.so.1, Mesa) n'existe pas sur Android, l'application planterait au démarrage.

#![allow(non_snake_case, clippy::too_many_arguments)]

use std::ffi::{c_char, c_void, CString};
use std::ptr::null_mut;

extern "C" {
    fn dlopen(name: *const c_char, flags: i32) -> *mut c_void;
    fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
}
const RTLD_NOW: i32 = 2;

macro_rules! api {
    ($name:ident { $($f:ident : fn($($a:ty),*) $(-> $r:ty)?;)* }) => {
        pub struct $name { $(pub $f: unsafe extern "C" fn($($a),*) $(-> $r)?,)* }
        impl $name {
            unsafe fn load(h: *mut c_void) -> Result<$name, String> {
                Ok($name { $($f: {
                    let p = dlsym(h, concat!(stringify!($f), "\0").as_ptr() as *const c_char);
                    if p.is_null() { return Err(concat!("symbole absent : ", stringify!($f)).to_string()); }
                    std::mem::transmute::<*mut c_void, unsafe extern "C" fn($($a),*) $(-> $r)?>(p)
                },)* })
            }
        }
    };
}

type P = *mut c_void;

api!(EglApi {
    eglGetDisplay: fn(P) -> P;
    eglInitialize: fn(P, *mut i32, *mut i32) -> u32;
    eglChooseConfig: fn(P, *const i32, *mut P, i32, *mut i32) -> u32;
    eglCreateContext: fn(P, P, P, *const i32) -> P;
    eglCreateWindowSurface: fn(P, P, P, *const i32) -> P;
    eglCreatePbufferSurface: fn(P, P, *const i32) -> P;
    eglDestroySurface: fn(P, P) -> u32;
    eglMakeCurrent: fn(P, P, P, P) -> u32;
    eglSwapBuffers: fn(P, P) -> u32;
    eglQuerySurface: fn(P, P, i32, *mut i32) -> u32;
    eglGetError: fn() -> i32;
});

api!(GlApi {
    glCreateShader: fn(u32) -> u32;
    glShaderSource: fn(u32, i32, *const *const c_char, *const i32);
    glCompileShader: fn(u32);
    glGetShaderiv: fn(u32, u32, *mut i32);
    glGetShaderInfoLog: fn(u32, i32, *mut i32, *mut c_char);
    glCreateProgram: fn() -> u32;
    glAttachShader: fn(u32, u32);
    glLinkProgram: fn(u32);
    glGetProgramiv: fn(u32, u32, *mut i32);
    glGetProgramInfoLog: fn(u32, i32, *mut i32, *mut c_char);
    glUseProgram: fn(u32);
    glGetUniformLocation: fn(u32, *const c_char) -> i32;
    glUniform1i: fn(i32, i32);
    glUniform1f: fn(i32, f32);
    glUniform2f: fn(i32, f32, f32);
    glUniform3f: fn(i32, f32, f32, f32);
    glUniformMatrix4fv: fn(i32, i32, u8, *const f32);
    glGenVertexArrays: fn(i32, *mut u32);
    glBindVertexArray: fn(u32);
    glGenBuffers: fn(i32, *mut u32);
    glBindBuffer: fn(u32, u32);
    glBufferData: fn(u32, isize, *const c_void, u32);
    glVertexAttribPointer: fn(u32, i32, u32, u8, i32, *const c_void);
    glEnableVertexAttribArray: fn(u32);
    glGenTextures: fn(i32, *mut u32);
    glBindTexture: fn(u32, u32);
    glActiveTexture: fn(u32);
    glTexParameteri: fn(u32, u32, i32);
    glTexImage2D: fn(u32, i32, i32, i32, i32, i32, u32, u32, *const c_void);
    glTexSubImage2D: fn(u32, i32, i32, i32, i32, i32, u32, u32, *const c_void);
    glPixelStorei: fn(u32, i32);
    glViewport: fn(i32, i32, i32, i32);
    glScissor: fn(i32, i32, i32, i32);
    glEnable: fn(u32);
    glDisable: fn(u32);
    glBlendFunc: fn(u32, u32);
    glClearColor: fn(f32, f32, f32, f32);
    glClear: fn(u32);
    glDrawArrays: fn(u32, i32, i32);
    glGetError: fn() -> u32;
    glReadPixels: fn(i32, i32, i32, i32, u32, u32, *mut c_void);
});

unsafe fn open(names: &[&str]) -> Result<P, String> {
    for n in names {
        let c = CString::new(*n).unwrap();
        let h = dlopen(c.as_ptr(), RTLD_NOW);
        if !h.is_null() {
            return Ok(h);
        }
    }
    Err(format!("bibliothèque introuvable : {}", names[0]))
}

pub struct Egl {
    pub api: EglApi,
    display: P,
    config: P,
    context: P,
    pub surface: P,
}

const NONE: i32 = 0x3038;
const WIDTH: i32 = 0x3057;
const HEIGHT: i32 = 0x3056;

impl Egl {
    /// `pbuffer` : rendu hors écran, pour l'outil d'aperçu. Celui-ci tourne dans Termux,
    /// où « libEGL.so » désigne Mesa : il vise donc la bibliothèque du système par son chemin.
    pub fn new(pbuffer: bool) -> Result<Egl, String> {
        unsafe {
            let names = if pbuffer { ["/system/lib64/libEGL.so", "libEGL.so"] } else { ["libEGL.so", "/system/lib64/libEGL.so"] };
            let api = EglApi::load(open(&names)?)?;
            let display = (api.eglGetDisplay)(null_mut());
            if (api.eglInitialize)(display, null_mut(), null_mut()) == 0 {
                return Err(format!("eglInitialize : {:#x}", (api.eglGetError)()));
            }
            let kind = if pbuffer { 1 } else { 4 };
            let mut config = null_mut();
            // Antialiasing 4× si possible : la 3D en a besoin, l'interface s'en passe.
            let chosen = [4, 0].iter().any(|&samples| {
                let at = [
                    0x3033, kind, 0x3040, 0x40, 0x3024, 8, 0x3023, 8, 0x3022, 8, 0x3025, 16, 0x3032,
                    (samples > 0) as i32, 0x3031, samples, NONE,
                ];
                let mut n = 0;
                (api.eglChooseConfig)(display, at.as_ptr(), &mut config, 1, &mut n) != 0 && n > 0
            });
            if !chosen {
                return Err("aucune configuration EGL OpenGL ES 3".to_string());
            }
            let context = (api.eglCreateContext)(display, config, null_mut(), [0x3098, 3, NONE].as_ptr());
            if context.is_null() {
                return Err(format!("eglCreateContext : {:#x}", (api.eglGetError)()));
            }
            Ok(Egl { api, display, config, context, surface: null_mut() })
        }
    }

    pub fn attach_window(&mut self, window: P) -> Result<(), String> {
        unsafe {
            self.surface = (self.api.eglCreateWindowSurface)(self.display, self.config, window, [NONE].as_ptr());
            self.current()
        }
    }

    pub fn attach_pbuffer(&mut self, w: i32, h: i32) -> Result<(), String> {
        unsafe {
            self.surface = (self.api.eglCreatePbufferSurface)(self.display, self.config, [WIDTH, w, HEIGHT, h, NONE].as_ptr());
            self.current()
        }
    }

    unsafe fn current(&self) -> Result<(), String> {
        if self.surface.is_null() || (self.api.eglMakeCurrent)(self.display, self.surface, self.surface, self.context) == 0 {
            return Err(format!("surface EGL : {:#x}", (self.api.eglGetError)()));
        }
        Ok(())
    }

    /// Lâche la surface ; le contexte, et donc les ressources OpenGL, restent valides.
    pub fn detach(&mut self) {
        unsafe {
            (self.api.eglMakeCurrent)(self.display, null_mut(), null_mut(), null_mut());
            (self.api.eglDestroySurface)(self.display, self.surface);
        }
        self.surface = null_mut();
    }

    pub fn size(&self) -> (f32, f32) {
        let (mut w, mut h) = (0, 0);
        unsafe {
            (self.api.eglQuerySurface)(self.display, self.surface, WIDTH, &mut w);
            (self.api.eglQuerySurface)(self.display, self.surface, HEIGHT, &mut h);
        }
        (w as f32, h as f32)
    }

    pub fn swap(&self) {
        unsafe {
            (self.api.eglSwapBuffers)(self.display, self.surface);
        }
    }
}

pub fn load_gl(pbuffer: bool) -> Result<GlApi, String> {
    let names = if pbuffer { ["/system/lib64/libGLESv3.so", "libGLESv3.so"] } else { ["libGLESv3.so", "/system/lib64/libGLESv3.so"] };
    unsafe { GlApi::load(open(&names)?) }
}
