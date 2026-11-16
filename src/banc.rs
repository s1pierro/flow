//! Le chargeur des modules du banc d'essai (l'interface : la crate `flow-banc`,
//! `banc/abi`). Un module est une bibliothèque `Flow/banc/modules/<nom>.so` ; le
//! stockage partagé ne permet pas d'exécuter : elle est copiée dans le dossier privé de
//! l'application, sous un nom neuf à chaque chargement (`dlopen` rendrait sinon
//! l'ancienne, déjà ouverte), puis ouverte. Les anciennes ne sont jamais refermées
//! (une bibliothèque Rust ne se décharge pas sans risque) : leurs copies sont effacées
//! au chargement suivant, leur mémoire reste jusqu'à la fin du processus.

use flow_banc::{FnAbi, FnCommande, FnLiberer, FnPas, FnScene, Tampon, ABI, SOMMET};
use std::ffi::{c_char, c_void, CStr, CString};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};

extern "C" {
    fn dlopen(name: *const c_char, flags: i32) -> *mut c_void;
    fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
    fn dlerror() -> *const c_char;
}
const RTLD_NOW: i32 = 2;

/// Un module chargé.
pub struct Charge {
    pub nom: String,
    /// Le numéro du chargement (le nom de la copie).
    pub numero: u64,
    commande: FnCommande,
    pas: FnPas,
    scene: FnScene,
    liberer: FnLiberer,
    /// La scène reçue : sommets (9 flottants), indices, et sa révision.
    pub sommets: std::sync::Arc<Vec<f32>>,
    pub indices: std::sync::Arc<Vec<u32>>,
    pub revision: u64,
}

#[derive(Default)]
pub struct Banc {
    pub module: Option<Charge>,
    numero: u64,
    /// Le journal des commandes et des chargements (le plus récent à la fin).
    pub journal: Vec<String>,
}

pub fn banc() -> MutexGuard<'static, Banc> {
    static B: OnceLock<Mutex<Banc>> = OnceLock::new();
    B.get_or_init(|| Mutex::new(Banc::default())).lock().unwrap_or_else(|e| e.into_inner())
}

/// Le dossier privé où les modules sont copiés (exécutable).
static PRIVE: OnceLock<PathBuf> = OnceLock::new();

/// Note le dossier interne de l'application (au lancement, sur Android).
pub fn noter_prive(p: PathBuf) {
    let _ = PRIVE.set(p);
}

fn prive() -> PathBuf {
    PRIVE.get().cloned().unwrap_or_else(std::env::temp_dir).join("banc")
}

/// Le dossier des modules à charger, dans le stockage partagé.
pub fn modules(flow: &Path) -> PathBuf {
    flow.join("banc").join("modules")
}

/// Les modules disponibles (les noms des `.so` du dossier), triés.
pub fn disponibles(flow: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(modules(flow))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().to_str().and_then(|n| n.strip_suffix(".so")).map(String::from))
        .collect();
    v.sort();
    v
}

unsafe fn symbole<T: Copy>(h: *mut c_void, nom: &str) -> Result<T, String> {
    let c = CString::new(nom).unwrap();
    let p = dlsym(h, c.as_ptr());
    if p.is_null() {
        return Err(format!("symbole absent : {nom}"));
    }
    Ok(std::mem::transmute_copy::<*mut c_void, T>(&p))
}

impl Banc {
    fn noter(&mut self, t: impl Into<String>) {
        self.journal.push(t.into());
        let n = self.journal.len();
        if n > 200 {
            self.journal.drain(..n - 200);
        }
    }

    /// Charge (ou recharge) le module `nom` du dossier `flow/banc/modules`.
    pub fn charger(&mut self, flow: &Path, nom: &str) -> Result<String, String> {
        let source = modules(flow).join(format!("{nom}.so"));
        let d = prive();
        std::fs::create_dir_all(&d).map_err(|e| format!("{}: {e}", d.display()))?;
        // Les copies des chargements passés : plus utiles (déjà ouvertes, ou pas)
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let _ = std::fs::remove_file(e.path());
        }
        self.numero += 1;
        let copie = d.join(format!("{nom}-{}.so", self.numero));
        std::fs::copy(&source, &copie).map_err(|e| format!("{} : {e}", source.display()))?;
        let r = unsafe {
            let c = CString::new(copie.to_string_lossy().as_bytes()).map_err(|e| e.to_string())?;
            let h = dlopen(c.as_ptr(), RTLD_NOW);
            if h.is_null() {
                let e = dlerror();
                let e = if e.is_null() { "?".to_string() } else { CStr::from_ptr(e).to_string_lossy().into_owned() };
                Err(format!("dlopen : {e}"))
            } else {
                let abi: FnAbi = symbole(h, "banc_abi")?;
                let v = abi();
                if v != ABI {
                    Err(format!("interface {v}, Flow attend {ABI} : reconstruire le module"))
                } else {
                    Ok(Charge {
                        nom: nom.to_string(),
                        numero: self.numero,
                        commande: symbole(h, "banc_commande")?,
                        pas: symbole(h, "banc_pas")?,
                        scene: symbole(h, "banc_scene")?,
                        liberer: symbole(h, "banc_liberer")?,
                        sommets: Default::default(),
                        indices: Default::default(),
                        revision: 0,
                    })
                }
            }
        };
        match r {
            Ok(mut m) => {
                m.lire_scene();
                let t = format!("chargé {nom} (n° {}, {} triangles)", m.numero, m.indices.len() / 3);
                self.module = Some(m);
                self.noter(&t);
                Ok(t)
            }
            Err(e) => {
                self.noter(format!("échec {nom} : {e}"));
                Err(e)
            }
        }
    }

    /// Une commande au module chargé.
    pub fn commande(&mut self, texte: &str) -> Result<String, String> {
        let Some(m) = &mut self.module else { return Err("aucun module chargé (banc charger <nom>)".into()) };
        let r = m.commande(texte);
        m.lire_scene();
        let ligne = match &r {
            Ok(t) => format!("› {texte}\n{t}"),
            Err(e) => format!("› {texte}\nerreur : {e}"),
        };
        self.noter(ligne);
        r
    }
}

impl Charge {
    fn commande(&self, texte: &str) -> Result<String, String> {
        let mut t = Tampon::VIDE;
        let code = unsafe { (self.commande)(texte.as_ptr(), texte.len(), &mut t) };
        let s = unsafe { String::from_utf8_lossy(t.octets()).into_owned() };
        unsafe { (self.liberer)(t) };
        if code == 0 {
            Ok(s)
        } else {
            Err(s)
        }
    }

    /// Le temps qui passe, puis la scène relue si elle a changé.
    pub fn pas(&mut self, dt: f32) {
        unsafe { (self.pas)(dt) };
        self.lire_scene();
    }

    /// Relit la scène du module si sa révision a changé.
    pub fn lire_scene(&mut self) {
        let (mut s, mut i) = (Tampon::VIDE, Tampon::VIDE);
        let rev = unsafe { (self.scene)(self.revision, &mut s, &mut i) };
        if rev != self.revision {
            let (so, io) = unsafe { (s.octets(), i.octets()) };
            let sommets: Vec<f32> = so.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
            let n = (sommets.len() / SOMMET) as u32;
            // Des indices hors des sommets : la scène est refusée
            let indices: Vec<u32> = io.chunks_exact(4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
            if indices.iter().all(|&k| k < n) && indices.len() % 3 == 0 {
                self.sommets = std::sync::Arc::new(sommets);
                self.indices = std::sync::Arc::new(indices);
            }
            self.revision = rev;
        }
        unsafe {
            (self.liberer)(s);
            (self.liberer)(i);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Le module d'exemple, s'il est construit (banc/construire.sh banc/exemples/cube) :
    /// chargé, commandé, rechargé.
    #[test]
    fn le_cube_se_charge_et_obeit() {
        let flow = Path::new("/sdcard/Flow");
        if !modules(flow).join("cube.so").exists() {
            return;
        }
        let mut b = Banc::default();
        b.charger(flow, "cube").unwrap();
        let n = b.module.as_ref().unwrap().indices.len();
        assert_eq!(n, 9 * 12 * 3);
        assert!(b.commande("grille 2").unwrap().contains("2×2"));
        assert_eq!(b.module.as_ref().unwrap().indices.len(), 4 * 12 * 3);
        assert!(b.commande("voler").is_err());
        b.commande("tourner 1").unwrap();
        let r = b.module.as_ref().unwrap().revision;
        b.module.as_mut().unwrap().pas(0.1);
        assert_ne!(b.module.as_ref().unwrap().revision, r);
        // Rechargé : un module neuf, son état de départ
        b.charger(flow, "cube").unwrap();
        assert_eq!(b.module.as_ref().unwrap().numero, 2);
        assert_eq!(b.module.as_ref().unwrap().indices.len(), 9 * 12 * 3);
    }
}
