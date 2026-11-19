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

// --- Lieux sur le banc --------------------------------------------------------------

/// Une tuile de la maquette des Lieux faite par un module : sommets de 9 flottants
/// (ceux de `relief3d::SOMMET` : position, normale, u, v, matière), triangles, et sa
/// texture (les octets RGBA de `maquette::texture`).
#[derive(Clone, Debug, Default)]
pub struct TuileFaite {
    pub sommets: Vec<f32>,
    pub indices: Vec<u32>,
    pub pixels: Vec<u8>,
}

/// La sortie de `banc_lieux_tuile` : ses trois tampons, ou l'erreur (texte).
#[repr(C)]
pub struct SortieTuile {
    pub sommets: Tampon,
    pub indices: Tampon,
    pub pixels: Tampon,
    pub erreur: Tampon,
}

/// Un module qui fabrique les tuiles de la maquette des Lieux (déclaré par
/// `lieux!(Type)`, en plus de `module!`). Flow lui donne le lieu (ses octets, au format
/// `.reel`) quand il change, puis lui demande des tuiles, dans le fil de la fabrique :
/// son état est à part de celui de `Module` (une tuile longue ne bloque pas les
/// commandes). Les tuiles peuvent être demandées par plusieurs fils à la fois (`&self`).
pub trait ModuleLieux: Send + Sync + 'static {
    fn nouveau() -> Self
    where
        Self: Sized;
    /// Le lieu, en octets `.reel`.
    fn lieu(&mut self, octets: &[u8]) -> Result<(), String>;
    /// La tuile `(z, i, j)` du dernier lieu reçu.
    fn tuile(&self, cle: (u8, i32, i32)) -> Result<TuileFaite, String>;
    /// La même tuile pour la vue Matérialisation de Lieux (par défaut : comme `tuile`).
    fn materialiser(&self, cle: (u8, i32, i32)) -> Result<TuileFaite, String> {
        self.tuile(cle)
    }
}

pub struct EtatLieux<M> {
    pub module: std::sync::RwLock<Option<M>>,
}

fn panique(e: Box<dyn std::any::Any + Send>) -> String {
    let t = e.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| e.downcast_ref::<String>().cloned()).unwrap_or_default();
    format!("le module a paniqué : {t}")
}

fn octets_f32(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

impl<M: ModuleLieux> EtatLieux<M> {
    pub const fn new() -> EtatLieux<M> {
        EtatLieux { module: std::sync::RwLock::new(None) }
    }

    /// # Safety
    /// `octets` valide sur `n` octets, `erreur` valide.
    pub unsafe fn lieu(&self, octets: *const u8, n: usize, erreur: *mut Tampon) -> i32 {
        let o = if octets.is_null() { &[][..] } else { std::slice::from_raw_parts(octets, n) };
        let mut g = self.module.write().unwrap_or_else(|e| e.into_inner());
        let m = g.get_or_insert_with(M::nouveau);
        match catch_unwind(AssertUnwindSafe(|| m.lieu(o))).map_err(panique).and_then(|r| r) {
            Ok(()) => 0,
            Err(e) => {
                if !erreur.is_null() {
                    *erreur = Tampon::de(e.into_bytes());
                }
                1
            }
        }
    }

    /// # Safety
    /// `sortie` valide.
    pub unsafe fn tuile(&self, z: u8, i: i32, j: i32, sortie: *mut SortieTuile) -> i32 {
        self.fabriquer(z, i, j, sortie, false)
    }

    /// # Safety
    /// `sortie` valide.
    pub unsafe fn materialiser(&self, z: u8, i: i32, j: i32, sortie: *mut SortieTuile) -> i32 {
        self.fabriquer(z, i, j, sortie, true)
    }

    unsafe fn fabriquer(&self, z: u8, i: i32, j: i32, sortie: *mut SortieTuile, materialiser: bool) -> i32 {
        if self.module.read().map(|g| g.is_none()).unwrap_or(false) {
            self.module.write().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(M::nouveau);
        }
        // En lecture : plusieurs fils à la fois
        let g = self.module.read().unwrap_or_else(|e| e.into_inner());
        let Some(m) = g.as_ref() else { return 1 };
        let r = catch_unwind(AssertUnwindSafe(|| if materialiser { m.materialiser((z, i, j)) } else { m.tuile((z, i, j)) })).map_err(panique).and_then(|r| r);
        let s = &mut *sortie;
        match r {
            Ok(t) => {
                s.sommets = Tampon::de(octets_f32(&t.sommets));
                s.indices = Tampon::de(t.indices.iter().flat_map(|x| x.to_le_bytes()).collect());
                s.pixels = Tampon::de(t.pixels);
                0
            }
            Err(e) => {
                s.erreur = Tampon::de(e.into_bytes());
                1
            }
        }
    }
}

impl<M: ModuleLieux> Default for EtatLieux<M> {
    fn default() -> Self {
        Self::new()
    }
}

/// Déclare le fabricant de tuiles `$t` (qui implémente `ModuleLieux`).
#[macro_export]
macro_rules! lieux {
    ($t:ty) => {
        static BANC_LIEUX: $crate::EtatLieux<$t> = $crate::EtatLieux::new();

        /// # Safety
        /// Appelée par Flow.
        #[no_mangle]
        pub unsafe extern "C" fn banc_lieux_lieu(octets: *const u8, n: usize, erreur: *mut $crate::Tampon) -> i32 {
            BANC_LIEUX.lieu(octets, n, erreur)
        }

        /// # Safety
        /// Appelée par Flow.
        #[no_mangle]
        pub unsafe extern "C" fn banc_lieux_tuile(z: u8, i: i32, j: i32, sortie: *mut $crate::SortieTuile) -> i32 {
            BANC_LIEUX.tuile(z, i, j, sortie)
        }

        /// # Safety
        /// Appelée par Flow (vue Matérialisation ; facultative pour Flow).
        #[no_mangle]
        pub unsafe extern "C" fn banc_lieux_materialiser(z: u8, i: i32, j: i32, sortie: *mut $crate::SortieTuile) -> i32 {
            BANC_LIEUX.materialiser(z, i, j, sortie)
        }
    };
}

pub type FnLieuxLieu = unsafe extern "C" fn(*const u8, usize, *mut Tampon) -> i32;
pub type FnLieuxTuile = unsafe extern "C" fn(u8, i32, i32, *mut SortieTuile) -> i32;

// --- Tracés en surimpression sur la maquette des Lieux ---------------------------------

/// Un groupe de traits d'une même couleur et d'une même épaisseur (pixels) : des
/// segments, les points (x, hauteur, z) deux par deux.
#[derive(Clone, Debug, Default)]
pub struct Traits {
    pub couleur: [f32; 3],
    pub epaisseur: f32,
    pub points: Vec<[f32; 3]>,
}

/// Les groupes en flottants : pour chacun r, g, b, épaisseur, nombre de points, puis
/// les points.
pub fn traits_en_octets(v: &[Traits]) -> Vec<u8> {
    let mut f: Vec<f32> = Vec::new();
    for t in v {
        f.extend_from_slice(&[t.couleur[0], t.couleur[1], t.couleur[2], t.epaisseur, t.points.len() as f32]);
        for p in &t.points {
            f.extend_from_slice(p);
        }
    }
    octets_f32(&f)
}

/// L'inverse de `traits_en_octets` (rien de ce qui déborde).
pub fn traits_depuis_octets(o: &[u8]) -> Vec<Traits> {
    let f: Vec<f32> = o.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
    let mut v = Vec::new();
    let mut k = 0;
    while k + 5 <= f.len() {
        let n = f[k + 4] as usize;
        if k + 5 + 3 * n > f.len() {
            break;
        }
        let points = f[k + 5..k + 5 + 3 * n].chunks_exact(3).map(|p| [p[0], p[1], p[2]]).collect();
        v.push(Traits { couleur: [f[k], f[k + 1], f[k + 2]], epaisseur: f[k + 3], points });
        k += 5 + 3 * n;
    }
    v
}

/// Ce que rend la fonction des tracés d'un module.
pub enum Traces {
    /// Les traits de la tuile.
    Faits(Vec<Traits>),
    /// Pas encore (un calcul en cours) : Flow redemande plus tard.
    PasPret,
}

/// Déclare la fonction des tracés `$f: fn((u8, i32, i32)) -> Result<Traces, String>` :
/// les traits en surimpression sur une tuile de la maquette, quand Lieux est sur le
/// banc. Appelée dans le fil de l'interface : elle doit être rapide (ou rendre
/// `PasPret` et calculer à part). Elle ne passe pas par l'état de `lieux!` (pris par
/// la fabrique pendant qu'elle fait une tuile).
#[macro_export]
macro_rules! traces {
    ($f:path) => {
        /// # Safety
        /// Appelée par Flow. 0 : faits ; 2 : pas prêt ; 1 : erreur (dans `sortie`).
        #[no_mangle]
        pub unsafe extern "C" fn banc_lieux_traces(z: u8, i: i32, j: i32, sortie: *mut $crate::Tampon) -> i32 {
            let r = std::panic::catch_unwind(|| $f((z, i, j)));
            let (code, o) = match r {
                Ok(Ok($crate::Traces::Faits(v))) => (0, $crate::traits_en_octets(&v)),
                Ok(Ok($crate::Traces::PasPret)) => (2, Vec::new()),
                Ok(Err(e)) => (1, e.into_bytes()),
                Err(_) => (1, b"le module a panique (traces)".to_vec()),
            };
            if !sortie.is_null() {
                *sortie = $crate::Tampon::de(o);
            }
            code
        }
    };
}

pub type FnLieuxTraces = unsafe extern "C" fn(u8, i32, i32, *mut Tampon) -> i32;

// --- Faces en surimpression sur la maquette des Lieux ---------------------------------

/// Ce que rend la fonction des faces d'un module : des triangles aux sommets de
/// `SOMMET` flottants (position, normale, couleur), comme la scène.
pub enum Faces {
    Faites(Scene),
    PasPret,
}

/// Déclare la fonction des faces `$f: fn((u8, i32, i32)) -> Result<Faces, String>` :
/// des volumes en surimpression sur une tuile de la maquette (jamais incorporés aux
/// tuiles), quand Lieux est sur le banc. Comme `traces!` : dans le fil de
/// l'interface, rapide ou `PasPret`.
#[macro_export]
macro_rules! faces {
    ($f:path) => {
        /// # Safety
        /// Appelée par Flow. 0 : faites ; 2 : pas prêt ; 1 : erreur (dans `sommets`).
        #[no_mangle]
        pub unsafe extern "C" fn banc_lieux_faces(z: u8, i: i32, j: i32, sommets: *mut $crate::Tampon, indices: *mut $crate::Tampon) -> i32 {
            let r = std::panic::catch_unwind(|| $f((z, i, j)));
            let (code, s, ind) = match r {
                Ok(Ok($crate::Faces::Faites(sc))) => (0, sc.sommets.iter().flat_map(|x| x.to_le_bytes()).collect(), sc.indices.iter().flat_map(|x| x.to_le_bytes()).collect()),
                Ok(Ok($crate::Faces::PasPret)) => (2, Vec::new(), Vec::new()),
                Ok(Err(e)) => (1, e.into_bytes(), Vec::new()),
                Err(_) => (1, b"le module a panique (faces)".to_vec(), Vec::new()),
            };
            if !sommets.is_null() {
                *sommets = $crate::Tampon::de(s);
            }
            if !indices.is_null() {
                *indices = $crate::Tampon::de(ind);
            }
            code
        }
    };
}

pub type FnLieuxFaces = unsafe extern "C" fn(u8, i32, i32, *mut Tampon, *mut Tampon) -> i32;
