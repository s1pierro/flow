//! Les opérations sur les fichiers, sans interface : créer, renommer, supprimer,
//! copier et déplacer. Les opérations longues (copier, déplacer, supprimer) tournent
//! sur un fil à part et rendent compte de leur avancement (`Avancement`), qu'on peut
//! interrompre. Le presse-papier des fichiers et le travail en cours sont communs à
//! toutes les fenêtres (`Manutention`, dans `Commun::fichiers`).

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// Ce que font ensemble toutes les fenêtres Fichiers.
#[derive(Default)]
pub struct Manutention {
    /// Le presse-papier des fichiers : les chemins, et s'ils sont coupés (déplacés au
    /// collage) plutôt que copiés.
    pub presse: Option<(Vec<PathBuf>, bool)>,
    /// L'opération en cours ou finie, pas encore relevée.
    pub travail: Option<Arc<Avancement>>,
    /// Augmente après chaque opération : les fenêtres relisent leur dossier.
    pub generation: u64,
}

impl Manutention {
    /// Vrai pendant une opération.
    pub fn occupe(&self) -> bool {
        self.travail.as_ref().is_some_and(|t| !t.fini.load(Ordering::Relaxed))
    }

    /// Relève une opération finie : son bilan, et si c'est une erreur. Les fenêtres
    /// relisent alors leur dossier.
    pub fn relever(&mut self) -> Option<(String, bool)> {
        let t = self.travail.as_ref().filter(|t| t.fini.load(Ordering::Relaxed))?.clone();
        self.travail = None;
        self.generation += 1;
        if t.sorte == Sorte::Deplacer {
            // Des fichiers coupés ne se collent qu'une fois
            self.presse = None;
        }
        Some(t.bilan())
    }

    /// Une opération faite sur place (créer, renommer) : les fenêtres relisent.
    pub fn change(&mut self) {
        self.generation += 1;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sorte {
    Copier,
    Deplacer,
    Supprimer,
}

impl Sorte {
    pub fn nom(self) -> &'static str {
        match self {
            Sorte::Copier => "Copie",
            Sorte::Deplacer => "Déplacement",
            Sorte::Supprimer => "Suppression",
        }
    }
}

/// L'avancement d'une opération, lu par l'interface pendant qu'elle tourne.
pub struct Avancement {
    pub sorte: Sorte,
    /// Le total et ce qui est fait : en octets pour une copie, en éléments sinon.
    pub total: u64,
    pub fait: AtomicU64,
    pub octets: bool,
    /// Les éléments de premier niveau traités.
    pub elements: AtomicUsize,
    pub fini: AtomicBool,
    /// Demandé par l'interface : l'opération s'arrête au plus tôt.
    pub annule: AtomicBool,
    pub erreur: Mutex<Option<String>>,
}

impl Avancement {
    fn new(sorte: Sorte, total: u64, octets: bool) -> Avancement {
        Avancement { sorte, total, fait: AtomicU64::new(0), octets, elements: AtomicUsize::new(0), fini: AtomicBool::new(false), annule: AtomicBool::new(false), erreur: Mutex::new(None) }
    }

    /// La part faite, de 0 à 1.
    pub fn part(&self) -> f32 {
        if self.total == 0 { 0.0 } else { (self.fait.load(Ordering::Relaxed) as f64 / self.total as f64).min(1.0) as f32 }
    }

    /// Une ligne pour l'interface : « Copie : 12 Mo sur 40 Mo ».
    pub fn libelle(&self) -> String {
        let fait = self.fait.load(Ordering::Relaxed);
        if self.octets {
            format!("{} : {} sur {}", self.sorte.nom(), crate::explorateur::taille(fait), crate::explorateur::taille(self.total))
        } else {
            format!("{} : {fait} sur {}", self.sorte.nom(), self.total)
        }
    }

    /// Le bilan d'une opération finie, et si c'est une erreur.
    pub fn bilan(&self) -> (String, bool) {
        let n = self.elements.load(Ordering::Relaxed);
        let elements = format!("{n} élément{}", if n > 1 { "s" } else { "" });
        let fait = match self.sorte {
            Sorte::Copier => "copié",
            Sorte::Deplacer => "déplacé",
            Sorte::Supprimer => "supprimé",
        };
        let fait = format!("{fait}{}", if n > 1 { "s" } else { "" });
        if let Some(e) = self.erreur.lock().ok().and_then(|e| e.clone()) {
            return (format!("{} interrompue ({elements} {fait}) : {e}", self.sorte.nom()), true);
        }
        if self.annule.load(Ordering::Relaxed) {
            return (format!("{} arrêtée : {elements} {fait}", self.sorte.nom()), false);
        }
        (format!("{elements} {fait}"), false)
    }
}

/// Un nom de fichier acceptable : ni vide, ni « . » ou « .. », sans « / ».
pub fn nom_valide(nom: &str) -> Result<String, String> {
    let nom = nom.trim();
    if nom.is_empty() {
        return Err("Donnez un nom.".into());
    }
    if nom == "." || nom == ".." || nom.contains('/') || nom.contains('\0') {
        return Err(format!("« {nom} » n'est pas un nom de fichier valide."));
    }
    if nom.len() > 255 {
        return Err("Nom trop long.".into());
    }
    Ok(nom.to_string())
}

fn erreur(e: std::io::Error) -> String {
    match e.kind() {
        std::io::ErrorKind::PermissionDenied => "accès refusé".into(),
        std::io::ErrorKind::AlreadyExists => "existe déjà".into(),
        std::io::ErrorKind::NotFound => "introuvable".into(),
        _ => e.to_string(),
    }
}

fn existe(p: &Path) -> bool {
    std::fs::symlink_metadata(p).is_ok()
}

pub fn creer_dossier(dans: &Path, nom: &str) -> Result<PathBuf, String> {
    let nom = nom_valide(nom)?;
    let p = dans.join(&nom);
    if existe(&p) {
        return Err(format!("« {nom} » existe déjà."));
    }
    std::fs::create_dir(&p).map_err(|e| format!("Dossier non créé : {}.", erreur(e)))?;
    Ok(p)
}

pub fn creer_fichier(dans: &Path, nom: &str) -> Result<PathBuf, String> {
    let nom = nom_valide(nom)?;
    let p = dans.join(&nom);
    std::fs::OpenOptions::new().write(true).create_new(true).open(&p).map_err(|e| format!("Fichier non créé : « {nom} » {}.", erreur(e)))?;
    Ok(p)
}

/// Renomme sur place. Changer seulement la casse est permis.
pub fn renommer(chemin: &Path, nom: &str) -> Result<PathBuf, String> {
    let nom = nom_valide(nom)?;
    let dans = chemin.parent().ok_or("Impossible de renommer la racine.")?;
    let p = dans.join(&nom);
    if p == chemin {
        return Ok(p);
    }
    let meme = chemin.file_name().is_some_and(|n| n.to_string_lossy().to_lowercase() == nom.to_lowercase());
    if existe(&p) && !meme {
        return Err(format!("« {nom} » existe déjà."));
    }
    std::fs::rename(chemin, &p).map_err(|e| format!("Non renommé : {}.", erreur(e)))?;
    Ok(p)
}

/// Un chemin libre dans `dans` pour `nom` : `nom` lui-même, sinon « nom (2).ext »,
/// « nom (3).ext »…
pub fn nom_libre(dans: &Path, nom: &str) -> PathBuf {
    let p = dans.join(nom);
    if !existe(&p) {
        return p;
    }
    let (base, ext) = match nom.rsplit_once('.') {
        Some((b, e)) if !b.is_empty() => (b.to_string(), format!(".{e}")),
        _ => (nom.to_string(), String::new()),
    };
    (2..).map(|i| dans.join(format!("{base} ({i}){ext}"))).find(|p| !existe(p)).unwrap()
}

/// La taille de tout ce que contient `p` (sans suivre les liens), en octets.
fn taille_de(p: &Path) -> u64 {
    match std::fs::symlink_metadata(p) {
        Ok(m) if m.is_dir() => std::fs::read_dir(p).map(|it| it.flatten().map(|e| taille_de(&e.path())).sum()).unwrap_or(0),
        Ok(m) => m.len(),
        Err(_) => 0,
    }
}

/// Lance une opération sur un fil à part. `dest` : le dossier où copier ou déplacer.
pub fn lancer(sorte: Sorte, sources: Vec<PathBuf>, dest: Option<PathBuf>) -> Arc<Avancement> {
    let av = Arc::new(match sorte {
        Sorte::Copier => Avancement::new(sorte, sources.iter().map(|s| taille_de(s)).sum(), true),
        _ => Avancement::new(sorte, sources.len() as u64, false),
    });
    let a = av.clone();
    std::thread::spawn(move || {
        if let Err(e) = executer(sorte, &sources, dest.as_deref(), &a) {
            if let Ok(mut x) = a.erreur.lock() {
                *x = Some(e);
            }
        }
        a.fini.store(true, Ordering::Relaxed);
    });
    av
}

/// Fait l'opération ici même (le fil de `lancer`, ou un test).
pub fn executer(sorte: Sorte, sources: &[PathBuf], dest: Option<&Path>, av: &Avancement) -> Result<(), String> {
    for src in sources {
        if av.annule.load(Ordering::Relaxed) {
            return Ok(());
        }
        let nom = src.file_name().map(|n| n.to_string_lossy().into_owned()).ok_or("Chemin sans nom.")?;
        match sorte {
            Sorte::Supprimer => {
                let m = std::fs::symlink_metadata(src).map_err(|e| format!("{nom} : {}", erreur(e)))?;
                if m.is_dir() { std::fs::remove_dir_all(src) } else { std::fs::remove_file(src) }.map_err(|e| format!("{nom} : {}", erreur(e)))?;
                av.fait.fetch_add(1, Ordering::Relaxed);
            }
            Sorte::Copier | Sorte::Deplacer => {
                let dest = dest.ok_or("Aucun dossier de destination.")?;
                if dest.starts_with(src) {
                    return Err(format!("« {nom} » ne peut pas aller dans lui-même."));
                }
                if sorte == Sorte::Deplacer && src.parent() == Some(dest) {
                    // Déjà là
                    av.fait.fetch_add(1, Ordering::Relaxed);
                    av.elements.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
                let cible = nom_libre(dest, &nom);
                if sorte == Sorte::Copier {
                    copier_tout(src, &cible, av)?;
                } else {
                    // Sur le même volume, renommer suffit ; sinon copier puis effacer
                    if std::fs::rename(src, &cible).is_err() {
                        copier_tout(src, &cible, av)?;
                        if av.annule.load(Ordering::Relaxed) {
                            return Ok(());
                        }
                        let m = std::fs::symlink_metadata(src).map_err(|e| format!("{nom} : {}", erreur(e)))?;
                        if m.is_dir() { std::fs::remove_dir_all(src) } else { std::fs::remove_file(src) }.map_err(|e| format!("{nom} : {}", erreur(e)))?;
                    }
                    av.fait.store(av.elements.load(Ordering::Relaxed) as u64 + 1, Ordering::Relaxed);
                }
            }
        }
        av.elements.fetch_add(1, Ordering::Relaxed);
    }
    Ok(())
}

/// Copie `src` (fichier, lien ou dossier entier) en `dst`, qui n'existe pas. Une copie
/// interrompue n'en laisse pas de morceau.
fn copier_tout(src: &Path, dst: &Path, av: &Avancement) -> Result<(), String> {
    let nom = || src.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned());
    let m = std::fs::symlink_metadata(src).map_err(|e| format!("{} : {}", nom(), erreur(e)))?;
    if m.file_type().is_symlink() {
        let cible = std::fs::read_link(src).map_err(|e| format!("{} : {}", nom(), erreur(e)))?;
        return std::os::unix::fs::symlink(cible, dst).map_err(|e| format!("{} : {}", nom(), erreur(e)));
    }
    if m.is_dir() {
        std::fs::create_dir(dst).map_err(|e| format!("{} : {}", nom(), erreur(e)))?;
        let entrees = std::fs::read_dir(src).map_err(|e| format!("{} : {}", nom(), erreur(e)))?;
        for e in entrees.flatten() {
            if av.annule.load(Ordering::Relaxed) {
                return Ok(());
            }
            copier_tout(&e.path(), &dst.join(e.file_name()), av)?;
        }
        return Ok(());
    }
    let mut lu = std::fs::File::open(src).map_err(|e| format!("{} : {}", nom(), erreur(e)))?;
    let mut ecrit = std::fs::OpenOptions::new().write(true).create_new(true).open(dst).map_err(|e| format!("{} : {}", nom(), erreur(e)))?;
    let mut tampon = vec![0u8; 256 * 1024];
    let resultat = (|| -> std::io::Result<bool> {
        loop {
            if av.annule.load(Ordering::Relaxed) {
                return Ok(false);
            }
            let n = lu.read(&mut tampon)?;
            if n == 0 {
                return Ok(true);
            }
            ecrit.write_all(&tampon[..n])?;
            if av.octets {
                av.fait.fetch_add(n as u64, Ordering::Relaxed);
            }
        }
    })();
    match resultat {
        Ok(true) => {
            if let Ok(t) = m.modified() {
                let _ = ecrit.set_modified(t);
            }
            Ok(())
        }
        Ok(false) => {
            drop(ecrit);
            let _ = std::fs::remove_file(dst);
            Ok(())
        }
        Err(e) => {
            drop(ecrit);
            let _ = std::fs::remove_file(dst);
            Err(format!("{} : {}", nom(), erreur(e)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dossier(nom: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("flow-op-{nom}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn faire(sorte: Sorte, sources: &[PathBuf], dest: Option<&Path>) -> Result<(), String> {
        let total = if sorte == Sorte::Copier { sources.iter().map(|s| taille_de(s)).sum() } else { sources.len() as u64 };
        let av = Avancement::new(sorte, total, sorte == Sorte::Copier);
        let r = executer(sorte, sources, dest, &av);
        if r.is_ok() {
            assert_eq!(av.part(), if total == 0 { 0.0 } else { 1.0 });
        }
        r
    }

    #[test]
    fn creer_renommer_et_noms_libres() {
        let d = dossier("creer");
        let a = creer_dossier(&d, " Album ").unwrap();
        assert!(a.is_dir() && a.ends_with("Album"));
        assert!(creer_dossier(&d, "Album").unwrap_err().contains("existe déjà"));
        assert!(creer_dossier(&d, "a/b").is_err());
        assert!(creer_dossier(&d, "..").is_err());
        let f = creer_fichier(&d, "notes.txt").unwrap();
        assert!(creer_fichier(&d, "notes.txt").is_err());
        assert_eq!(nom_libre(&d, "notes.txt"), d.join("notes (2).txt"));
        assert_eq!(nom_libre(&d, "Album"), d.join("Album (2)"));
        assert_eq!(nom_libre(&d, "neuf"), d.join("neuf"));
        // Renommer ; vers un nom pris, non ; la casse seule, oui
        let g = renommer(&f, "lettre.txt").unwrap();
        assert!(g.exists() && !f.exists());
        assert!(renommer(&g, "Album").unwrap_err().contains("existe déjà"));
        let h = renommer(&g, "Lettre.txt").unwrap();
        assert!(h.exists());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn copier_deplacer_supprimer() {
        let d = dossier("copier");
        let src = d.join("src");
        std::fs::create_dir_all(src.join("sous")).unwrap();
        std::fs::write(src.join("a.txt"), "aaa").unwrap();
        std::fs::write(src.join("sous/b.txt"), "bbbbb").unwrap();
        let dest = d.join("dest");
        std::fs::create_dir(&dest).unwrap();
        // Copier un dossier entier, deux fois : la seconde copie prend un nom libre
        faire(Sorte::Copier, &[src.clone()], Some(&dest)).unwrap();
        faire(Sorte::Copier, &[src.clone()], Some(&dest)).unwrap();
        assert_eq!(std::fs::read_to_string(dest.join("src/sous/b.txt")).unwrap(), "bbbbb");
        assert!(dest.join("src (2)/a.txt").exists());
        // Pas dans lui-même
        assert!(faire(Sorte::Copier, &[src.clone()], Some(&src.join("sous"))).unwrap_err().contains("lui-même"));
        // Déplacer un fichier, puis le dossier copié
        faire(Sorte::Deplacer, &[src.join("a.txt")], Some(&dest)).unwrap();
        assert!(!src.join("a.txt").exists() && dest.join("a.txt").exists());
        faire(Sorte::Deplacer, &[dest.join("src (2)")], Some(&src)).unwrap();
        assert!(src.join("src (2)/sous/b.txt").exists());
        // Déplacer là où il est déjà ne change rien
        faire(Sorte::Deplacer, &[dest.join("a.txt")], Some(&dest)).unwrap();
        assert!(dest.join("a.txt").exists());
        // Supprimer
        faire(Sorte::Supprimer, &[dest.join("src"), dest.join("a.txt")], None).unwrap();
        assert_eq!(std::fs::read_dir(&dest).unwrap().count(), 0);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn une_operation_lancee_se_releve() {
        let d = dossier("lancer");
        std::fs::write(d.join("x"), vec![7u8; 600_000]).unwrap();
        std::fs::create_dir(d.join("y")).unwrap();
        let mut m = Manutention { presse: Some((vec![d.join("x")], true)), ..Manutention::default() };
        m.travail = Some(lancer(Sorte::Deplacer, vec![d.join("x")], Some(d.join("y"))));
        while m.occupe() {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let (bilan, erreur) = m.relever().unwrap();
        assert_eq!((bilan.as_str(), erreur), ("1 élément déplacé", false));
        assert_eq!(m.generation, 1);
        assert!(m.presse.is_none(), "des fichiers coupés ne se collent qu'une fois");
        assert_eq!(std::fs::metadata(d.join("y/x")).unwrap().len(), 600_000);
        let _ = std::fs::remove_dir_all(d);
    }
}
