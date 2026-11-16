//! Le suivi des tâches : chaque étape chronométrée (`tache`) se range, à sa fin, dans
//! un journal (son fil, son chemin d'imbrication « tuile › chaussée › booléens », sa
//! durée, son détail) et dans des cumuls par chemin ; celles en cours se voient avec leur
//! durée qui court. Le temps est compté depuis le dernier repère (`repere`, posé à
//! l'ouverture de la maquette) : on y lit ce que coûtent les premières générations.
//! Les étapes longues (≥ FLOW_SUIVI_SEUIL ms, 20 par défaut) vont aussi dans logcat.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

/// Le nombre d'étapes finies gardées.
const GARDEES: usize = 600;

/// Une étape finie.
#[derive(Clone, Debug)]
pub struct Finie {
    /// Son début, en secondes depuis le repère (négatif : avant lui).
    pub debut: f64,
    pub duree: f32,
    pub fil: String,
    pub chemin: String,
    pub profondeur: usize,
    pub detail: String,
}

/// Une étape en cours.
#[derive(Clone, Debug)]
pub struct EnCours {
    id: u64,
    pub debut: Instant,
    pub fil: String,
    pub chemin: String,
    pub detail: String,
}

/// Les cumuls d'un chemin : nombre, total, plus longue (s), première (s).
#[derive(Clone, Copy, Debug, Default)]
pub struct Cumul {
    pub n: u32,
    pub total: f64,
    pub max: f32,
    pub premiere: f32,
}

struct Etat {
    repere: Instant,
    nom_repere: String,
    en_cours: Vec<EnCours>,
    finies: VecDeque<Finie>,
    cumuls: HashMap<String, Cumul>,
    /// Incrémenté à chaque changement (pour redessiner).
    revision: u64,
}

fn etat() -> &'static Mutex<Etat> {
    static E: OnceLock<Mutex<Etat>> = OnceLock::new();
    E.get_or_init(|| Mutex::new(Etat { repere: Instant::now(), nom_repere: "lancement".into(), en_cours: Vec::new(), finies: VecDeque::new(), cumuls: HashMap::new(), revision: 0 }))
}

fn seuil_log() -> f32 {
    static S: OnceLock<f32> = OnceLock::new();
    *S.get_or_init(|| std::env::var("FLOW_SUIVI_SEUIL").ok().and_then(|s| s.parse::<f32>().ok()).unwrap_or(20.0) / 1000.0)
}

thread_local! {
    /// Les noms des étapes ouvertes de ce fil, de la plus externe à la plus interne.
    static PILE: RefCell<Vec<(&'static str, u64)>> = const { RefCell::new(Vec::new()) };
}

static IDS: AtomicU64 = AtomicU64::new(1);

fn nom_du_fil() -> String {
    let t = std::thread::current();
    t.name().map(str::to_string).unwrap_or_else(|| format!("{:?}", t.id()).replace("ThreadId", "fil"))
}

/// Une étape chronométrée : finie (journal, cumuls) quand elle tombe.
pub struct Tache {
    id: u64,
    debut: Instant,
    chemin: String,
    profondeur: usize,
    detail: String,
    /// En deçà (s), elle n'entre pas au journal (les cumuls la comptent quand même).
    min: f32,
}

/// Ouvre l'étape `nom` (imbriquée dans celles déjà ouvertes par ce fil).
pub fn tache(nom: &'static str) -> Tache {
    tache_min(nom, 0.0)
}

/// `tache`, mais n'entre au journal que si elle dure au moins `min_ms` (les étapes de
/// chaque image : seulement quand elles traînent).
pub fn tache_min(nom: &'static str, min_ms: f32) -> Tache {
    let id = IDS.fetch_add(1, Ordering::Relaxed);
    let (chemin, profondeur) = PILE.with(|p| {
        let mut p = p.borrow_mut();
        p.push((nom, id));
        (p.iter().map(|x| x.0).collect::<Vec<_>>().join(" › "), p.len() - 1)
    });
    let debut = Instant::now();
    if min_ms <= 0.0 {
        if let Ok(mut e) = etat().lock() {
            e.en_cours.push(EnCours { id, debut, fil: nom_du_fil(), chemin: chemin.clone(), detail: String::new() });
            e.revision += 1;
        }
    }
    Tache { id, debut, chemin, profondeur, detail: String::new(), min: min_ms / 1000.0 }
}

impl Tache {
    /// Le détail de l'étape (montré aussi tant qu'elle court).
    pub fn detail(&mut self, d: impl Into<String>) {
        self.detail = d.into();
        if self.min <= 0.0 {
            if let Ok(mut e) = etat().lock() {
                if let Some(c) = e.en_cours.iter_mut().find(|c| c.id == self.id) {
                    c.detail = self.detail.clone();
                }
            }
        }
    }
}

impl Drop for Tache {
    fn drop(&mut self) {
        PILE.with(|p| {
            p.borrow_mut().pop();
        });
        let duree = self.debut.elapsed().as_secs_f32();
        let Ok(mut e) = etat().lock() else { return };
        e.en_cours.retain(|c| c.id != self.id);
        let premiere = !e.cumuls.contains_key(&self.chemin);
        let c = e.cumuls.entry(self.chemin.clone()).or_default();
        c.n += 1;
        c.total += duree as f64;
        c.max = c.max.max(duree);
        if premiere {
            c.premiere = duree;
        }
        if duree < self.min {
            return;
        }
        let debut = if self.debut >= e.repere { self.debut.duration_since(e.repere).as_secs_f64() } else { -(e.repere.duration_since(self.debut).as_secs_f64()) };
        let f = Finie { debut, duree, fil: nom_du_fil(), chemin: std::mem::take(&mut self.chemin), profondeur: self.profondeur, detail: std::mem::take(&mut self.detail) };
        if duree >= seuil_log() {
            crate::android::log(&format!("suivi {}", ligne(&f)));
        }
        e.finies.push_back(f);
        while e.finies.len() > GARDEES {
            e.finies.pop_front();
        }
        e.revision += 1;
    }
}

/// L'avancement de l'étape la plus interne ouverte par ce fil (son détail, tant qu'elle
/// court) : « 1200 / 62389 », par exemple.
pub fn avancement(texte: impl Into<String>) {
    let Some(id) = PILE.with(|p| p.borrow().last().map(|x| x.1)) else { return };
    if let Ok(mut e) = etat().lock() {
        if let Some(c) = e.en_cours.iter_mut().find(|c| c.id == id) {
            c.detail = texte.into();
            e.revision += 1;
        }
    }
}

/// Un événement ponctuel au journal.
pub fn noter(texte: impl Into<String>) {
    let mut t = tache("·");
    t.detail(texte);
}

/// Pose le repère (le temps des étapes compte depuis lui) ; `vider` : oublie aussi le
/// journal et les cumuls (les étapes en cours restent).
pub fn repere(nom: impl Into<String>, vider: bool) {
    let nom = nom.into();
    if let Ok(mut e) = etat().lock() {
        e.repere = Instant::now();
        e.nom_repere = nom.clone();
        if vider {
            e.finies.clear();
            e.cumuls.clear();
        }
        e.revision += 1;
    }
    noter(format!("repère « {nom} »"));
}

pub fn revision() -> u64 {
    etat().lock().map_or(0, |e| e.revision)
}

/// Des étapes courent-elles ?
pub fn occupe() -> bool {
    etat().lock().is_ok_and(|e| !e.en_cours.is_empty())
}

/// Le repère : son nom et les secondes écoulées depuis.
pub fn depuis_repere() -> (String, f64) {
    etat().lock().map(|e| (e.nom_repere.clone(), e.repere.elapsed().as_secs_f64())).unwrap_or_default()
}

/// Les étapes en cours, avec leur durée jusqu'ici (s), les plus anciennes d'abord.
pub fn en_cours() -> Vec<(EnCours, f32)> {
    etat().lock().map(|e| e.en_cours.iter().map(|c| (c.clone(), c.debut.elapsed().as_secs_f32())).collect()).unwrap_or_default()
}

/// Les `n` dernières étapes finies, dans l'ordre de leur fin.
pub fn dernieres(n: usize) -> Vec<Finie> {
    etat().lock().map(|e| e.finies.iter().rev().take(n).rev().cloned().collect()).unwrap_or_default()
}

/// Les cumuls, du plus gros total au plus petit.
pub fn cumuls() -> Vec<(String, Cumul)> {
    let mut v: Vec<(String, Cumul)> = etat().lock().map(|e| e.cumuls.iter().map(|(k, c)| (k.clone(), *c)).collect()).unwrap_or_default();
    v.sort_by(|a, b| b.1.total.total_cmp(&a.1.total));
    v
}

/// Une durée lisible : « 850 µs », « 12,3 ms », « 4,56 s ».
pub fn duree(s: f32) -> String {
    if s < 0.001 {
        format!("{:.0} µs", s * 1e6)
    } else if s < 1.0 {
        format!("{:.1} ms", s * 1e3).replace('.', ",")
    } else {
        format!("{:.2} s", s).replace('.', ",")
    }
}

/// Une étape finie en une ligne : « +12,345 s [maquette] tuile › chaussée 1,2 s (7/61/71) ».
pub fn ligne(f: &Finie) -> String {
    let d = if f.detail.is_empty() { String::new() } else { format!(" ({})", f.detail) };
    format!("{:+.3} s [{}] {} {}{d}", f.debut, f.fil, f.chemin, duree(f.duree))
}

/// Le texte du suivi pour le pilote : `suivi` (en cours et 60 dernières), `suivi
/// cumuls`, `suivi tout`, `suivi repere`.
pub fn piloter(m: &[String]) -> Option<Result<String, String>> {
    if m.first().map(String::as_str) != Some("suivi") {
        return None;
    }
    let (nom, t) = depuis_repere();
    let mut s = format!("repère « {nom} » il y a {}\n", duree(t as f32));
    match m.get(1).map(String::as_str) {
        Some("repere") => {
            repere(m.get(2).cloned().unwrap_or_else(|| "pilote".into()), true);
            return Some(Ok("repère posé, journal vidé".into()));
        }
        Some("cumuls") => {
            s += "chemin : n, total, max, première\n";
            for (k, c) in cumuls() {
                s += &format!("{k} : {} × , {}, {}, {}\n", c.n, duree(c.total as f32), duree(c.max), duree(c.premiere));
            }
        }
        Some("tout") | None => {
            let n = if m.get(1).is_some() { GARDEES } else { 60 };
            s += "en cours :\n";
            for (c, d) in en_cours() {
                let det = if c.detail.is_empty() { String::new() } else { format!(" ({})", c.detail) };
                s += &format!("  [{}] {} {}…{det}\n", c.fil, c.chemin, duree(d));
            }
            s += "finies :\n";
            for f in dernieres(n) {
                s += &format!("  {}{}\n", "  ".repeat(f.profondeur), ligne(&f));
            }
        }
        Some(a) => return Some(Err(format!("suivi : {a} ? (suivi, suivi cumuls, suivi tout, suivi repere [nom])"))),
    }
    Some(Ok(s))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn les_etapes_s_imbriquent_et_se_cumulent() {
        {
            let _a = tache("essai-a");
            for _ in 0..2 {
                let mut b = tache("b");
                b.detail("x");
                assert!(en_cours().iter().any(|(c, _)| c.chemin == "essai-a › b"));
            }
        }
        let c = cumuls();
        let b = c.iter().find(|(k, _)| k == "essai-a › b").unwrap().1;
        assert_eq!(b.n, 2);
        assert!(dernieres(GARDEES).iter().any(|f| f.chemin == "essai-a" && f.profondeur == 0));
        assert!(!en_cours().iter().any(|(c, _)| c.chemin.starts_with("essai-a")));
    }
}
