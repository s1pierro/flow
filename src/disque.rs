//! L'occupation du disque, sans interface : le parcours d'un dossier dans un thread
//! (taille occupée sur le disque, comme `du`), l'arbre des tailles, la place libre du
//! système de fichiers, et la géométrie du graphique en anneaux : le dossier au
//! centre, ses enfants sur le premier anneau, leurs enfants sur le suivant…

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Un fichier ou un dossier, avec sa taille sur le disque. Les enfants sont rangés du
/// plus gros au plus petit.
#[derive(Clone, Debug, Default)]
pub struct Noeud {
    pub nom: String,
    pub taille: u64,
    pub dossier: bool,
    /// Fichiers contenus, à toutes les profondeurs (1 pour un fichier).
    pub fichiers: u64,
    pub enfants: Vec<Noeud>,
}

impl Noeud {
    /// Le nœud au bout d'un chemin d'indices, s'il existe encore.
    pub fn suivre(&self, chemin: &[usize]) -> Option<&Noeud> {
        chemin.iter().try_fold(self, |n, &i| n.enfants.get(i))
    }

    fn ranger(&mut self) {
        self.enfants.sort_by(|a, b| b.taille.cmp(&a.taille).then_with(|| a.nom.cmp(&b.nom)));
    }
}

/// Ce que le parcours a déjà compté, lisible pendant qu'il travaille.
#[derive(Default)]
pub struct Progres {
    pub fichiers: AtomicU64,
    pub octets: AtomicU64,
}

/// Une analyse : lancée dans un thread, lisible à tout moment, abandonnée quand on la
/// lâche.
pub struct Analyse {
    pub racine: PathBuf,
    pub progres: Arc<Progres>,
    resultat: Arc<Mutex<Option<(Arc<Noeud>, f32)>>>,
    stop: Arc<AtomicBool>,
}

impl Analyse {
    /// Parcourt `racine`. `fin` est appelé à la fin (pour réveiller l'affichage).
    pub fn lancer(racine: PathBuf, fin: fn()) -> Analyse {
        let (progres, resultat, stop) = (Arc::new(Progres::default()), Arc::new(Mutex::new(None)), Arc::new(AtomicBool::new(false)));
        let (p, r, s, chemin) = (progres.clone(), resultat.clone(), stop.clone(), racine.clone());
        std::thread::spawn(move || {
            let debut = Instant::now();
            let nom = chemin.file_name().map_or("/".to_string(), |n| n.to_string_lossy().into_owned());
            let n = parcourir(&chemin, nom, &s, &p);
            if !s.load(Ordering::Relaxed) {
                *r.lock().unwrap() = Some((Arc::new(n), debut.elapsed().as_secs_f32()));
                fin();
            }
        });
        Analyse { racine, progres, resultat, stop }
    }

    /// L'arbre et la durée du parcours, une fois fini.
    pub fn resultat(&self) -> Option<(Arc<Noeud>, f32)> {
        self.resultat.lock().unwrap().clone()
    }

    pub fn en_cours(&self) -> bool {
        self.resultat.lock().unwrap().is_none()
    }
}

impl Drop for Analyse {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Taille occupée sur le disque (en blocs), à défaut la taille apparente.
fn occupe(m: &std::fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    let b = m.blocks() * 512;
    if b > 0 { b } else { m.len() }
}

/// Parcours récursif. Les liens symboliques sont ignorés (ni boucle, ni double
/// compte) ; un dossier illisible compte pour vide.
pub fn parcourir(p: &Path, nom: String, stop: &AtomicBool, progres: &Progres) -> Noeud {
    let mut n = Noeud { nom, dossier: true, ..Noeud::default() };
    let Ok(it) = std::fs::read_dir(p) else { return n };
    for e in it.flatten() {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let Ok(m) = e.metadata() else { continue };
        let nom = e.file_name().to_string_lossy().into_owned();
        let enfant = if m.file_type().is_symlink() {
            continue;
        } else if m.is_dir() {
            parcourir(&e.path(), nom, stop, progres)
        } else {
            let t = occupe(&m);
            progres.fichiers.fetch_add(1, Ordering::Relaxed);
            progres.octets.fetch_add(t, Ordering::Relaxed);
            Noeud { nom, taille: t, dossier: false, fichiers: 1, enfants: Vec::new() }
        };
        n.taille += enfant.taille;
        n.fichiers += enfant.fichiers;
        n.enfants.push(enfant);
    }
    n.ranger();
    n
}

/// L'espace du système de fichiers qui contient `p` : (total, libre), en octets.
pub fn espace(p: &Path) -> Option<(u64, u64)> {
    // struct statvfs de Bionic (et de la glibc) sur 64 bits : onze champs de 8 octets,
    // puis de la réserve
    #[repr(C)]
    struct Statvfs {
        bsize: u64,
        frsize: u64,
        blocks: u64,
        bfree: u64,
        bavail: u64,
        files: u64,
        ffree: u64,
        favail: u64,
        fsid: u64,
        flag: u64,
        namemax: u64,
        reserve: [u64; 8],
    }
    extern "C" {
        fn statvfs(path: *const std::ffi::c_char, buf: *mut Statvfs) -> i32;
    }
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(p.as_os_str().as_bytes()).ok()?;
    let mut s: Statvfs = unsafe { std::mem::zeroed() };
    if unsafe { statvfs(c.as_ptr(), &mut s) } != 0 {
        return None;
    }
    let f = if s.frsize > 0 { s.frsize } else { s.bsize };
    Some((s.blocks * f, s.bavail * f))
}

/// Un secteur du graphique : un nœud, son anneau (1 pour les enfants du centre), ses
/// angles (en radians, de 0 à 2π, à partir du haut, dans le sens horaire) et la
/// teinte de son ancêtre du premier anneau.
#[derive(Clone, Debug, PartialEq)]
pub struct Secteur {
    pub chemin: Vec<usize>,
    pub anneau: usize,
    pub a0: f32,
    pub a1: f32,
    pub teinte: usize,
    pub dossier: bool,
}

/// Les secteurs de `n` sur `anneaux` anneaux. Ceux plus étroits que `min` (radians)
/// sont omis, comme leurs descendants : ils gardent leur part d'angle, laissée vide.
pub fn secteurs(n: &Noeud, anneaux: usize, min: f32) -> Vec<Secteur> {
    let mut l = Vec::new();
    fn aller(n: &Noeud, chemin: &mut Vec<usize>, anneau: usize, anneaux: usize, a0: f32, a1: f32, teinte: Option<usize>, min: f32, l: &mut Vec<Secteur>) {
        if anneau > anneaux || n.taille == 0 {
            return;
        }
        let mut a = a0;
        for (i, e) in n.enfants.iter().enumerate() {
            let da = (a1 - a0) * e.taille as f32 / n.taille as f32;
            if da >= min {
                chemin.push(i);
                let t = teinte.unwrap_or(i);
                l.push(Secteur { chemin: chemin.clone(), anneau, a0: a, a1: a + da, teinte: t, dossier: e.dossier });
                aller(e, chemin, anneau + 1, anneaux, a, a + da, Some(t), min, l);
                chemin.pop();
            }
            a += da;
        }
    }
    aller(n, &mut Vec::new(), 1, anneaux, 0.0, std::f32::consts::TAU, None, min, &mut l);
    l
}

/// Le secteur sous un point, donné par son anneau et son angle (même repère que
/// `Secteur`).
pub fn toucher(s: &[Secteur], anneau: usize, angle: f32) -> Option<&Secteur> {
    s.iter().find(|s| s.anneau == anneau && angle >= s.a0 && angle < s.a1)
}

/// L'emplacement d'un dossier dans le graphique vu depuis la racine : son nœud, son
/// angle à chaque niveau (du premier anneau jusqu'à lui) et la teinte de sa branche
/// (l'indice de son ancêtre du premier anneau).
pub struct Place<'a> {
    pub noeud: &'a Noeud,
    pub arcs: Vec<(f32, f32)>,
    pub teinte: Option<usize>,
}

/// L'angle du `i`-ième enfant de `parent`, dans l'angle `span` du parent.
pub fn arc_enfant(parent: &Noeud, span: (f32, f32), i: usize) -> (f32, f32) {
    if parent.taille == 0 {
        return (span.0, span.0);
    }
    let k = (span.1 - span.0) / parent.taille as f32;
    let avant: u64 = parent.enfants[..i].iter().map(|e| e.taille).sum();
    let a0 = span.0 + k * avant as f32;
    (a0, a0 + k * parent.enfants[i].taille as f32)
}

/// Suit des noms depuis la racine (même disposition que le graphique centré sur
/// elle). `None` si un nom n'est pas dans l'arbre (créé après l'analyse…).
pub fn placer<'a, S: AsRef<str>>(racine: &'a Noeud, noms: &[S]) -> Option<Place<'a>> {
    let mut p = Place { noeud: racine, arcs: Vec::new(), teinte: None };
    let mut span = (0.0, std::f32::consts::TAU);
    for nom in noms {
        let i = p.noeud.enfants.iter().position(|e| e.nom == nom.as_ref())?;
        span = arc_enfant(p.noeud, span, i);
        p.arcs.push(span);
        p.teinte.get_or_insert(i);
        p.noeud = &p.noeud.enfants[i];
    }
    Some(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(nom: &str, t: u64) -> Noeud {
        Noeud { nom: nom.into(), taille: t, dossier: false, fichiers: 1, enfants: vec![] }
    }

    fn d(nom: &str, mut e: Vec<Noeud>) -> Noeud {
        let mut n = Noeud { nom: nom.into(), dossier: true, taille: e.iter().map(|e| e.taille).sum(), fichiers: e.iter().map(|e| e.fichiers).sum(), enfants: vec![] };
        n.enfants.append(&mut e);
        n.ranger();
        n
    }

    #[test]
    fn les_secteurs_partagent_le_tour_selon_les_tailles() {
        let arbre = d("racine", vec![f("petit", 25), d("Music", vec![f("a", 50), f("b", 25)])]);
        assert_eq!(arbre.enfants[0].nom, "Music", "le plus gros d'abord");
        let s = secteurs(&arbre, 3, 0.0);
        let tau = std::f32::consts::TAU;
        let proche = |a: f32, b: f32| (a - b).abs() < 1e-5;
        assert_eq!(s.len(), 4);
        assert!(s[0].anneau == 1 && proche(s[0].a0, 0.0) && proche(s[0].a1, 0.75 * tau));
        // Les enfants de Music restent dans son angle, avec sa teinte
        assert!(s[1].anneau == 2 && proche(s[1].a1, 0.5 * tau) && s[1].teinte == 0);
        assert!(proche(s[2].a0, 0.5 * tau) && proche(s[2].a1, 0.75 * tau));
        assert!(s[3].teinte == 1 && proche(s[3].a0, 0.75 * tau));
        assert_eq!(toucher(&s, 2, 1.0).map(|s| s.chemin.clone()), Some(vec![0, 0]));
        assert_eq!(arbre.suivre(&[0, 1]).map(|n| n.nom.as_str()), Some("b"));
        // La place d'un dossier suit la même disposition que les secteurs
        let p = placer(&arbre, &["Music", "b"]).unwrap();
        assert_eq!((p.noeud.nom.as_str(), p.teinte, p.arcs.len()), ("b", Some(0), 2));
        assert!(proche(p.arcs[1].0, s[2].a0) && proche(p.arcs[1].1, s[2].a1));
        assert!(placer(&arbre, &["absent"]).is_none());
        // Un seuil écarte les secteurs trop étroits sans décaler les suivants
        let s = secteurs(&arbre, 1, 0.3 * tau);
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn le_parcours_compte_fichiers_et_dossiers() {
        let dir = std::env::temp_dir().join(format!("flow-disque-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sous")).unwrap();
        std::fs::write(dir.join("a.bin"), vec![0u8; 10_000]).unwrap();
        std::fs::write(dir.join("sous/b.bin"), vec![1u8; 30_000]).unwrap();
        let p = Progres::default();
        let n = parcourir(&dir, "essai".into(), &AtomicBool::new(false), &p);
        assert_eq!((n.fichiers, p.fichiers.load(Ordering::Relaxed)), (2, 2));
        assert!(n.taille >= 40_000, "{}", n.taille);
        assert_eq!(n.enfants[0].nom, "sous");
        assert!(espace(&dir).is_some_and(|(t, l)| t > 0 && l <= t));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
