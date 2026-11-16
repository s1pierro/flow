//! L'interface des modules du banc d'essai de Flow : des bibliothèques `.so` (crates
//! `cdylib`) que le contexte Banc charge à chaud, sans réinstaller Flow.
//!
//! Un module implémente `Module` et se déclare par `module!(Type)`. Flow le trouve
//! dans `Flow/banc/modules/<nom>.so`, le copie dans son dossier privé (sous un nom neuf
//! à chaque chargement : un module rechargé est bien le nouveau) et l'ouvre par
//! `dlopen`. Il lui parle par cinq fonctions C (`ABI`) :
//!
//! - `banc_abi() -> u32` : la version de l'interface, `ABI` ;
//! - `banc_commande(texte, longueur, sortie) -> i32` : une commande en mots ; la
//!   réponse (texte) dans `sortie`, 0 si c'est un succès, 1 une erreur ;
//! - `banc_pas(dt)` : le temps qui passe (à chaque image où le Banc est montré) ;
//! - `banc_scene(sommets, indices) -> u64` : ce qu'il faut dessiner, des triangles aux
//!   sommets de 9 flottants (position, normale, couleur de 0 à 1), et sa révision (la
//!   même : rien n'a changé, les tampons restent vides) ;
//! - `banc_liberer(tampon)` : rend un tampon donné par le module.
//!
//! Une panique dans le module est rattrapée : elle devient une erreur.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Mutex;

/// La version de l'interface : Flow refuse un module d'une autre version.
pub const ABI: u32 = 1;

/// Les flottants d'un sommet de la scène : position, normale, couleur.
pub const SOMMET: usize = 9;

/// Un tampon d'octets donné par le module ; Flow le rend par `banc_liberer`.
#[repr(C)]
pub struct Tampon {
    pub ptr: *mut u8,
    pub len: usize,
    pub cap: usize,
}

impl Tampon {
    pub const VIDE: Tampon = Tampon { ptr: std::ptr::null_mut(), len: 0, cap: 0 };

    pub fn de(v: Vec<u8>) -> Tampon {
        let mut v = std::mem::ManuallyDrop::new(v);
        Tampon { ptr: v.as_mut_ptr(), len: v.len(), cap: v.capacity() }
    }

    /// # Safety
    /// Un tampon fait par `de`, rendu une seule fois.
    pub unsafe fn rendre(self) {
        if !self.ptr.is_null() {
            drop(Vec::from_raw_parts(self.ptr, self.len, self.cap));
        }
    }

    /// Les octets, sans les prendre.
    ///
    /// # Safety
    /// Un tampon valide.
    pub unsafe fn octets(&self) -> &[u8] {
        if self.ptr.is_null() {
            &[]
        } else {
            std::slice::from_raw_parts(self.ptr, self.len)
        }
    }
}

/// Ce qu'un module montre : des triangles aux sommets de `SOMMET` flottants.
#[derive(Clone, Debug, Default)]
pub struct Scene {
    pub sommets: Vec<f32>,
    pub indices: Vec<u32>,
}

impl Scene {
    /// Ajoute un triangle à plat, de couleur `c` (normale de sa face).
    pub fn triangle(&mut self, a: [f32; 3], b: [f32; 3], d: [f32; 3], c: [f32; 3]) {
        let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let v = [d[0] - a[0], d[1] - a[1], d[2] - a[2]];
        let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
        let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-12);
        let n = n.map(|x| x / l);
        let i = (self.sommets.len() / SOMMET) as u32;
        for p in [a, b, d] {
            self.sommets.extend_from_slice(&[p[0], p[1], p[2], n[0], n[1], n[2], c[0], c[1], c[2]]);
        }
        self.indices.extend_from_slice(&[i, i + 1, i + 2]);
    }

    /// Ajoute un polygone plan (en éventail), de couleur `c`.
    pub fn polygone(&mut self, p: &[[f32; 3]], c: [f32; 3]) {
        for k in 1..p.len().saturating_sub(1) {
            self.triangle(p[0], p[k], p[k + 1], c);
        }
    }
}

/// Un module du banc.
pub trait Module: Send + 'static {
    /// Le module neuf, au chargement.
    fn nouveau() -> Self
    where
        Self: Sized;

    /// Une commande, en mots (`m[0]` est son nom) : la réponse, ou l'erreur.
    fn commande(&mut self, m: &[&str]) -> Result<String, String>;

    /// Le temps qui passe (s).
    fn pas(&mut self, _dt: f32) {}

    /// La révision de la scène : elle change quand la scène change.
    fn revision(&self) -> u64;

    /// La scène à dessiner.
    fn scene(&self) -> Scene;
}

/// L'état d'un module : l'instance, et la révision déjà envoyée.
pub struct Etat<M> {
    pub module: Mutex<Option<M>>,
}

impl<M: Module> Etat<M> {
    pub const fn new() -> Etat<M> {
        Etat { module: Mutex::new(None) }
    }

    fn avec<R>(&self, f: impl FnOnce(&mut M) -> R) -> Result<R, String> {
        let mut g = self.module.lock().unwrap_or_else(|e| e.into_inner());
        let m = g.get_or_insert_with(M::nouveau);
        catch_unwind(AssertUnwindSafe(|| f(m))).map_err(|e| {
            let t = e.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| e.downcast_ref::<String>().cloned()).unwrap_or_default();
            format!("le module a paniqué : {t}")
        })
    }

    /// # Safety
    /// `texte` valide sur `longueur` octets, `sortie` valide.
    pub unsafe fn commande(&self, texte: *const u8, longueur: usize, sortie: *mut Tampon) -> i32 {
        let t = if texte.is_null() { String::new() } else { String::from_utf8_lossy(std::slice::from_raw_parts(texte, longueur)).into_owned() };
        let m: Vec<&str> = t.split_whitespace().collect();
        let r = if m.is_empty() { Err("commande vide".to_string()) } else { self.avec(|x| x.commande(&m)).and_then(|r| r) };
        let (code, texte) = match r {
            Ok(s) => (0, s),
            Err(e) => (1, e),
        };
        if !sortie.is_null() {
            *sortie = Tampon::de(texte.into_bytes());
        }
        code
    }

    pub fn pas(&self, dt: f32) {
        let _ = self.avec(|m| m.pas(dt));
    }

    /// # Safety
    /// `sommets`, `indices` valides.
    pub unsafe fn scene(&self, connue: u64, sommets: *mut Tampon, indices: *mut Tampon) -> u64 {
        let r = self.avec(|m| {
            let rev = m.revision();
            (rev, (rev != connue).then(|| m.scene()))
        });
        let Ok((rev, scene)) = r else { return connue };
        if let Some(s) = scene {
            let octets = |v: Vec<u8>| Tampon::de(v);
            *sommets = octets(s.sommets.iter().flat_map(|x| x.to_le_bytes()).collect());
            *indices = octets(s.indices.iter().flat_map(|x| x.to_le_bytes()).collect());
        }
        rev
    }
}

impl<M: Module> Default for Etat<M> {
    fn default() -> Self {
        Self::new()
    }
}

/// Déclare le module `$t` (qui implémente `Module`) : ses fonctions C.
#[macro_export]
macro_rules! module {
    ($t:ty) => {
        static BANC_ETAT: $crate::Etat<$t> = $crate::Etat::new();

        #[no_mangle]
        pub extern "C" fn banc_abi() -> u32 {
            $crate::ABI
        }

        /// # Safety
        /// Appelée par Flow.
        #[no_mangle]
        pub unsafe extern "C" fn banc_commande(texte: *const u8, longueur: usize, sortie: *mut $crate::Tampon) -> i32 {
            BANC_ETAT.commande(texte, longueur, sortie)
        }

        #[no_mangle]
        pub extern "C" fn banc_pas(dt: f32) {
            BANC_ETAT.pas(dt)
        }

        /// # Safety
        /// Appelée par Flow.
        #[no_mangle]
        pub unsafe extern "C" fn banc_scene(connue: u64, sommets: *mut $crate::Tampon, indices: *mut $crate::Tampon) -> u64 {
            BANC_ETAT.scene(connue, sommets, indices)
        }

        /// # Safety
        /// Un tampon donné par ce module.
        #[no_mangle]
        pub unsafe extern "C" fn banc_liberer(t: $crate::Tampon) {
            t.rendre()
        }
    };
}

/// Les signatures des fonctions C, pour qui les charge.
pub type FnAbi = unsafe extern "C" fn() -> u32;
pub type FnCommande = unsafe extern "C" fn(*const u8, usize, *mut Tampon) -> i32;
pub type FnPas = unsafe extern "C" fn(f32);
pub type FnScene = unsafe extern "C" fn(u64, *mut Tampon, *mut Tampon) -> u64;
pub type FnLiberer = unsafe extern "C" fn(Tampon);
