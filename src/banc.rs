//! Le chargeur des modules du banc d'essai (l'interface : la crate `flow-banc`,
//! `banc/abi`). Un module est une bibliothèque `Flow/banc/modules/<nom>.so` ; le
//! stockage partagé ne permet pas d'exécuter : elle est copiée dans le dossier privé de
//! l'application, sous un nom neuf à chaque chargement (`dlopen` rendrait sinon
//! l'ancienne, déjà ouverte), puis ouverte. Les anciennes ne sont jamais refermées
//! (une bibliothèque Rust ne se décharge pas sans risque) : leurs copies sont effacées
//! au chargement suivant, leur mémoire reste jusqu'à la fin du processus.

use flow_banc::{FnAbi, FnCommande, FnLiberer, FnLieuxFaces, FnLieuxLieu, FnLieuxTraces, FnLieuxTuile, FnPas, FnScene, SortieTuile, Tampon, Traits, ABI, SOMMET};
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
    /// Ses fonctions de Lieux, s'il en a (`lieux!`) : il fabrique alors les tuiles de la
    /// maquette, quand Lieux est sur le banc.
    lieux: Option<(FnLieuxLieu, FnLieuxTuile)>,
    /// Sa fonction des tracés en surimpression sur la maquette (`traces!`), s'il en a.
    traces: Option<FnLieuxTraces>,
    /// Sa fonction des faces en surimpression (`faces!`), s'il en a.
    faces: Option<FnLieuxFaces>,
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
    /// Lieux sur le banc : ses tuiles de maquette faites par le module.
    pub lieux: bool,
    /// Change à chaque chargement et à chaque bascule : Lieux refait alors ses tuiles.
    pub version: u64,
    /// La version du lieu déjà donnée au module (chargement, `faite` de la carte).
    lieu_donne: Option<(u64, u64)>,
    /// Le dossier de Flow du dernier chargement : l'état du banc y est retenu
    /// (`banc/etat.txt`) pour être rétabli au prochain lancement.
    flow: Option<PathBuf>,
}

/// Le fichier où l'état du banc est retenu, dans le dossier de Flow.
fn fichier_etat(flow: &Path) -> PathBuf {
    flow.join("banc").join("etat.txt")
}

impl Banc {
    /// Retient le module chargé et si Lieux est sur le banc.
    fn memoriser(&self) {
        let (Some(f), Some(m)) = (&self.flow, &self.module) else { return };
        let _ = std::fs::write(fichier_etat(f), format!("module = {}\nlieux = {}\n", m.nom, if self.lieux { "oui" } else { "non" }));
    }
}

/// Au lancement (une fois) : recharge le module retenu et remet Lieux sur le banc s'il
/// y était.
pub fn retablir(flow: &Path) {
    static FAIT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if FAIT.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    let Ok(t) = std::fs::read_to_string(fichier_etat(flow)) else { return };
    let val = |k: &str| t.lines().filter_map(|l| l.split_once('=')).find(|(a, _)| a.trim() == k).map(|(_, v)| v.trim().to_string());
    let Some(nom) = val("module").filter(|n| !n.is_empty()) else { return };
    let mut b = banc();
    if b.charger(flow, &nom).is_ok() && val("lieux").as_deref() == Some("oui") {
        b.lieux = true;
        b.version += 1;
        b.noter("Lieux remis sur le banc (état du dernier lancement)");
    }
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
                        lieux: symbole::<FnLieuxLieu>(h, "banc_lieux_lieu").ok().zip(symbole::<FnLieuxTuile>(h, "banc_lieux_tuile").ok()),
                        traces: symbole::<FnLieuxTraces>(h, "banc_lieux_traces").ok(),
                        faces: symbole::<FnLieuxFaces>(h, "banc_lieux_faces").ok(),
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
                let mut l = String::new();
                for (oui, quoi) in [(m.lieux.is_some(), "tuiles"), (m.traces.is_some(), "tracés"), (m.faces.is_some(), "faces")] {
                    if oui {
                        l += if l.is_empty() { ", Lieux : " } else { ", " };
                        l += quoi;
                    }
                }
                let t = format!("chargé {nom} (n° {}, {} triangles{l})", m.numero, m.indices.len() / 3);
                self.module = Some(m);
                self.version += 1;
                self.lieu_donne = None;
                self.flow = Some(flow.to_path_buf());
                self.memoriser();
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

/// Une tuile de la maquette faite par le module, si Lieux est sur le banc et que le
/// module en fabrique : ses sommets, triangles et texture, ou l'erreur. `faite` est la
/// version des données de la carte : le lieu est redonné au module quand elle change.
/// Appelé dans le fil de la fabrique ; le verrou du banc n'est pas tenu pendant que le
/// module travaille.
pub fn tuile_lieux(d: &crate::carte::Donnees, faite: u64, cle: (u8, i32, i32)) -> Option<Result<(Vec<f32>, Vec<u32>, Vec<u8>), String>> {
    let (f_lieu, f_tuile, liberer, numero, donne) = {
        let b = banc();
        let m = b.module.as_ref().filter(|_| b.lieux)?;
        let (l, t) = m.lieux?;
        (l, t, m.liberer, m.numero, b.lieu_donne)
    };
    if donne != Some((numero, faite)) {
        let o = d.lieu.octets();
        let mut e = Tampon::VIDE;
        let code = unsafe { f_lieu(o.as_ptr(), o.len(), &mut e) };
        let texte = unsafe { String::from_utf8_lossy(e.octets()).into_owned() };
        unsafe { liberer(e) };
        if code != 0 {
            return Some(Err(format!("lieu refusé : {texte}")));
        }
        banc().lieu_donne = Some((numero, faite));
    }
    let mut s = SortieTuile { sommets: Tampon::VIDE, indices: Tampon::VIDE, pixels: Tampon::VIDE, erreur: Tampon::VIDE };
    let code = unsafe { f_tuile(cle.0, cle.1, cle.2, &mut s) };
    let r = unsafe {
        if code == 0 {
            let sommets: Vec<f32> = s.sommets.octets().chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
            let indices: Vec<u32> = s.indices.octets().chunks_exact(4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
            let n = (sommets.len() / crate::relief3d::SOMMET) as u32;
            if indices.iter().all(|&k| k < n) {
                Ok((sommets, indices, s.pixels.octets().to_vec()))
            } else {
                Err("indices hors des sommets".to_string())
            }
        } else {
            Err(String::from_utf8_lossy(s.erreur.octets()).into_owned())
        }
    };
    unsafe {
        liberer(s.sommets);
        liberer(s.indices);
        liberer(s.pixels);
        liberer(s.erreur);
    }
    if let Err(e) = &r {
        banc().noter(format!("tuile {cle:?} : {e}"));
    }
    Some(r)
}

/// Lieux est-il sur le banc, avec un module qui fait les tracés ?
pub fn traces_actives() -> bool {
    let b = banc();
    b.lieux && b.module.as_ref().is_some_and(|m| m.traces.is_some())
}

/// Lieux est-il sur le banc, avec un module qui fait des faces en surimpression ?
pub fn faces_actives() -> bool {
    let b = banc();
    b.lieux && b.module.as_ref().is_some_and(|m| m.faces.is_some())
}

/// Les faces du module sur la tuile `cle` (sommets de 9 flottants : position, normale,
/// couleur ; triangles) : comme `traces_lieux`.
pub fn faces_lieux(faite: u64, cle: (u8, i32, i32)) -> Option<Result<Option<(Vec<f32>, Vec<u32>)>, String>> {
    let (f, liberer, pret) = {
        let b = banc();
        let m = b.module.as_ref().filter(|_| b.lieux)?;
        let f = m.faces?;
        (f, m.liberer, b.lieu_donne == Some((m.numero, faite)))
    };
    if !pret {
        return Some(Ok(None));
    }
    let (mut s, mut i) = (Tampon::VIDE, Tampon::VIDE);
    let code = unsafe { f(cle.0, cle.1, cle.2, &mut s, &mut i) };
    let r = unsafe {
        match code {
            0 => {
                let sommets: Vec<f32> = s.octets().chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
                let indices: Vec<u32> = i.octets().chunks_exact(4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
                let n = (sommets.len() / SOMMET) as u32;
                if indices.iter().all(|&k| k < n) {
                    Ok(Some((sommets, indices)))
                } else {
                    Err("indices hors des sommets".to_string())
                }
            }
            2 => Ok(None),
            _ => Err(String::from_utf8_lossy(s.octets()).into_owned()),
        }
    };
    unsafe {
        liberer(s);
        liberer(i);
    }
    if let Err(e) = &r {
        banc().noter(format!("faces {cle:?} : {e}"));
    }
    Some(r)
}

/// Les tracés du module sur la tuile `cle`, si Lieux est sur le banc et que le module
/// en fait : `Some(Ok(Some(traits)))`, `Some(Ok(None))` s'ils ne sont pas prêts (le
/// lieu pas encore donné par la fabrique, ou un calcul en cours : redemander), l'erreur
/// sinon ; `None` hors du banc (Flow fait les siens).
pub fn traces_lieux(faite: u64, cle: (u8, i32, i32)) -> Option<Result<Option<Vec<Traits>>, String>> {
    let (f, liberer, pret) = {
        let b = banc();
        let m = b.module.as_ref().filter(|_| b.lieux)?;
        let f = m.traces?;
        (f, m.liberer, b.lieu_donne == Some((m.numero, faite)))
    };
    if !pret {
        return Some(Ok(None));
    }
    let mut t = Tampon::VIDE;
    let code = unsafe { f(cle.0, cle.1, cle.2, &mut t) };
    let r = unsafe {
        match code {
            0 => Ok(Some(flow_banc::traits_depuis_octets(t.octets()))),
            2 => Ok(None),
            _ => Err(String::from_utf8_lossy(t.octets()).into_owned()),
        }
    };
    unsafe { liberer(t) };
    if let Err(e) = &r {
        banc().noter(format!("tracés {cle:?} : {e}"));
    }
    Some(r)
}

/// La version du banc pour Lieux : elle change quand ses tuiles sont à refaire.
pub fn version_lieux() -> u64 {
    banc().version
}

/// Les commandes du banc qui ne demandent pas sa fenêtre (`banc vue` et `banc cadrer`
/// vont au contexte Banc) : rien si ce n'en est pas une.
pub fn piloter(flow: Option<&Path>, m: &[String]) -> Option<Result<String, String>> {
    if m[0] != "banc" {
        return None;
    }
    let mut b = banc();
    Some(match m.get(1).map(|s| s.as_str()) {
        None | Some("etat") => Ok(match &b.module {
            Some(x) => format!("module {} n° {} · {} triangles · révision {} · lieux {}", x.nom, x.numero, x.indices.len() / 3, x.revision, if b.lieux && x.lieux.is_some() { "sur le banc" } else if x.lieux.is_some() { "non (banc lieux oui)" } else { "—" }),
            None => "aucun module".into(),
        }),
        Some("modules") => Ok(flow.map(disponibles).unwrap_or_default().join("\n")),
        Some("charger") => match (m.get(2), flow) {
            (Some(n), Some(f)) => b.charger(f, n),
            _ => Err("banc charger <nom>".into()),
        },
        Some("recharger") => {
            let nom = b.module.as_ref().map(|x| x.nom.clone());
            match (nom, flow) {
                (Some(n), Some(f)) => b.charger(f, &n),
                _ => Err("aucun module chargé".into()),
            }
        }
        Some("lieux") => {
            let v = match m.get(2).map(|s| s.as_str()) {
                Some("oui" | "1" | "on") => true,
                Some("non" | "0" | "off") => false,
                _ => return Some(Err("banc lieux oui|non".into())),
            };
            if b.lieux != v {
                b.lieux = v;
                b.version += 1;
                b.memoriser();
            }
            let fabrique = b.module.as_ref().is_some_and(|x| x.lieux.is_some());
            Ok(match (v, fabrique) {
                (true, true) => "Lieux sur le banc : ses tuiles sont refaites par le module".into(),
                (true, false) => "Lieux sur le banc, mais le module chargé ne fabrique pas de tuiles".into(),
                _ => "Lieux hors du banc".into(),
            })
        }
        Some("vue" | "cadrer") => return None,
        Some(_) => b.commande(&m[1..].join(" ")),
    })
}

impl Charge {
    /// Fabrique-t-il les tuiles des Lieux ?
    pub fn fabrique_lieux(&self) -> bool {
        self.lieux.is_some()
    }

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
