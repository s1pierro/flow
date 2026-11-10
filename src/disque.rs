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

/// La synthèse d'une analyse, pour les empreintes des dossiers dans Fichiers : les
/// dossiers seuls, avec ce qu'il faut pour retrouver leur angle dans le graphique sans
/// l'arbre complet. Elle s'enregistre dans `Flow/empreintes.txt`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Empreinte {
    pub nom: String,
    pub taille: u64,
    /// Sa place parmi les entrées du parent, fichiers compris (la teinte du graphique).
    pub rang: usize,
    /// Les octets des entrées qui le précèdent dans le parent (son angle de départ).
    pub avant: u64,
    pub enfants: Vec<Empreinte>,
}

/// Le nom du fichier de la synthèse, dans le dossier de Flow.
pub const EMPREINTES: &str = "empreintes.txt";
/// Un dossier plus petit que cette part du tout n'a pas d'empreinte visible : il n'est
/// pas gardé.
const SEUIL_EMPREINTE: f64 = 1e-5;
/// L'âge au-delà duquel la synthèse est refaite, en secondes.
pub const UN_JOUR: u64 = 24 * 3600;

impl Empreinte {
    pub fn depuis(n: &Noeud) -> Empreinte {
        fn aller(n: &Noeud, rang: usize, avant: u64, min: u64) -> Empreinte {
            let mut e = Empreinte { nom: n.nom.clone(), taille: n.taille, rang, avant, enfants: Vec::new() };
            let mut acc = 0;
            for (i, c) in n.enfants.iter().enumerate() {
                if c.dossier && c.taille > 0 && c.taille >= min && !c.nom.contains(['\t', '\n', '\r']) {
                    e.enfants.push(aller(c, i, acc, min));
                }
                acc += c.taille;
            }
            e
        }
        aller(n, 0, 0, (n.taille as f64 * SEUIL_EMPREINTE).ceil() as u64)
    }

    /// L'angle d'un enfant, dans l'angle `span` de ce dossier.
    fn arc(&self, enfant: &Empreinte, span: (f32, f32)) -> (f32, f32) {
        if self.taille == 0 {
            return (span.0, span.0);
        }
        let k = (span.1 - span.0) / self.taille as f32;
        let a0 = span.0 + k * enfant.avant as f32;
        (a0, a0 + k * enfant.taille as f32)
    }
}

/// Un dossier situé dans la synthèse : son nœud, ses angles depuis la racine, la
/// teinte de sa branche.
pub struct Situation<'a> {
    pub noeud: &'a Empreinte,
    pub arcs: Vec<(f32, f32)>,
    pub teinte: Option<usize>,
}

impl Situation<'_> {
    /// L'empreinte d'un sous-dossier : ses angles et sa teinte.
    pub fn enfant(&self, nom: &str) -> Option<(Vec<(f32, f32)>, usize)> {
        let e = self.noeud.enfants.iter().find(|e| e.nom == nom)?;
        let span = self.arcs.last().copied().unwrap_or((0.0, std::f32::consts::TAU));
        let mut arcs = self.arcs.clone();
        arcs.push(self.noeud.arc(e, span));
        Some((arcs, self.teinte.unwrap_or(e.rang)))
    }
}

/// Suit des noms depuis la racine de la synthèse (comme `placer` dans l'arbre complet).
pub fn situer<'a, S: AsRef<str>>(racine: &'a Empreinte, noms: &[S]) -> Option<Situation<'a>> {
    let mut p = Situation { noeud: racine, arcs: Vec::new(), teinte: None };
    let mut span = (0.0, std::f32::consts::TAU);
    for nom in noms {
        let e = p.noeud.enfants.iter().find(|e| e.nom == nom.as_ref())?;
        span = p.noeud.arc(e, span);
        p.arcs.push(span);
        p.teinte.get_or_insert(e.rang);
        p.noeud = e;
    }
    Some(p)
}

/// La synthèse et ce qu'elle décrit : le dossier analysé, et quand (secondes Unix).
#[derive(Clone, Debug, PartialEq)]
pub struct Synthese {
    pub racine: PathBuf,
    pub date: u64,
    pub arbre: Empreinte,
}

/// L'heure, en secondes Unix.
pub fn maintenant() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

impl Synthese {
    pub fn ecrire(&self) -> String {
        let mut s = String::new();
        s += "# Synthèse de l'analyse du stockage : les empreintes des dossiers dans Fichiers.\n";
        s += "# Refaite par Flow au plus une fois par jour, ou à chaque analyse demandée.\n";
        s += "# Une ligne par dossier : profondeur, rang parmi les entrées du parent (fichiers\n";
        s += "# compris), octets avant lui, taille en octets, nom ; séparés par des tabulations.\n";
        s += &format!("racine = {}\ndate = {}\n", self.racine.display(), self.date);
        fn aller(e: &Empreinte, prof: usize, s: &mut String) {
            *s += &format!("{prof}\t{}\t{}\t{}\t{}\n", e.rang, e.avant, e.taille, e.nom);
            for c in &e.enfants {
                aller(c, prof + 1, s);
            }
        }
        aller(&self.arbre, 0, &mut s);
        s
    }

    /// Relit une synthèse ; `None` si elle est illisible.
    pub fn lire(t: &str) -> Option<Synthese> {
        let (mut racine, mut date) = (None, None);
        // La pile des dossiers ouverts, du plus haut au plus profond
        let mut pile: Vec<Empreinte> = Vec::new();
        let fermer = |pile: &mut Vec<Empreinte>, jusqua: usize| {
            while pile.len() > jusqua.max(1) {
                let e = pile.pop().unwrap();
                pile.last_mut().unwrap().enfants.push(e);
            }
        };
        for l in t.lines() {
            if l.starts_with('#') || l.trim().is_empty() {
                continue;
            }
            if let Some(v) = l.strip_prefix("racine = ") {
                racine = Some(PathBuf::from(v));
                continue;
            }
            if let Some(v) = l.strip_prefix("date = ") {
                date = v.trim().parse().ok();
                continue;
            }
            let mut c = l.splitn(5, '\t');
            let mut n = || c.next();
            let (prof, rang, avant, taille, nom) = (n()?.parse::<usize>().ok()?, n()?.parse().ok()?, n()?.parse().ok()?, n()?.parse().ok()?, n()?.to_string());
            if prof == 0 {
                if !pile.is_empty() {
                    return None;
                }
            } else if prof > pile.len() {
                return None;
            } else {
                fermer(&mut pile, prof);
            }
            pile.push(Empreinte { nom, taille, rang, avant, enfants: Vec::new() });
        }
        fermer(&mut pile, 1);
        Some(Synthese { racine: racine?, date: date?, arbre: pile.pop()? })
    }
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
    fn la_synthese_place_les_dossiers_comme_le_graphique() {
        let arbre = d("racine", vec![f("gros", 500), d("photos", vec![f("a", 100), d("vacances", vec![f("b", 60)]), f("c", 10)]), f("petit", 5), d("vide", vec![])]);
        let s = Synthese { racine: "/x".into(), date: 42, arbre: Empreinte::depuis(&arbre) };
        // Les dossiers vides et les fichiers ne sont pas gardés
        assert_eq!(s.arbre.enfants.len(), 1);
        assert_eq!(Synthese::lire(&s.ecrire()), Some(s.clone()));
        let complet = placer(&arbre, &["photos", "vacances"]).unwrap();
        let resume = situer(&s.arbre, &["photos", "vacances"]).unwrap();
        assert_eq!(complet.arcs, resume.arcs);
        assert_eq!(complet.teinte, resume.teinte);
        // L'empreinte d'un sous-dossier, comme Fichiers la calcule depuis l'arbre complet
        let parent = situer(&s.arbre, &["photos"]).unwrap();
        let (arcs, teinte) = parent.enfant("vacances").unwrap();
        assert_eq!((arcs, Some(teinte)), (complet.arcs.clone(), complet.teinte));
        assert!(Synthese::lire("racine = /x\ndate = 1\n2\t0\t0\t1\tperdu\n").is_none());
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
