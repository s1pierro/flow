//! La charge de l'appareil pendant la génération des cartes (et quand il sert de
//! renfort) : ce qu'on peut en mesurer sans droits particuliers, et le régulateur du
//! nombre de fils de calcul.
//!
//! - **Les mesures** (`Mesure::prendre`) : les cœurs, la mémoire disponible
//!   (`/proc/meminfo`), la mémoire de Flow (`/proc/self/status`), sa charge processeur
//!   (`/proc/self/stat`, rapportée à tous les cœurs), la température la plus haute des
//!   zones thermiques lisibles (`/sys/class/thermal`). La charge de tout l'appareil
//!   (`/proc/stat`) est fermée aux applications depuis Android 8.
//! - **Le régulateur** (`Regulateur`) : `FILS` fils de calcul (0 : automatique). En
//!   automatique, il part de cœurs − 1, en retire un quand la mémoire disponible
//!   passe sous `MEMOIRE_BASSE` ou que la température dépasse `CHAUD`, et en rend un
//!   quand tout va bien depuis `CALME`.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Le nombre de fils de calcul voulu (0 : automatique), pour la génération et le
/// renfort de cet appareil.
pub static FILS: AtomicUsize = AtomicUsize::new(0);

/// En automatique : la mémoire disponible sous laquelle on retire un fil (Mo), celle
/// au-dessus de laquelle on peut en rendre un.
const MEMOIRE_BASSE: u64 = 700;
const MEMOIRE_HAUTE: u64 = 1400;
/// La température (°C) au-dessus de laquelle on retire un fil, sous laquelle on peut
/// en rendre un.
const CHAUD: f32 = 80.0;
const TIEDE: f32 = 70.0;
/// Le temps sans alerte avant de rendre un fil.
const CALME: Duration = Duration::from_secs(10);
/// Entre deux ajustements.
const PAS: Duration = Duration::from_secs(2);

/// Le nombre de cœurs.
pub fn coeurs() -> usize {
    std::thread::available_parallelism().map_or(2, |n| n.get())
}

/// Le plus de fils de calcul : le réglage, ou cœurs − 1 en automatique (au moins 1).
pub fn fils_max() -> usize {
    match FILS.load(Ordering::Relaxed) {
        0 => coeurs().saturating_sub(1).max(1),
        n => n.min(coeurs()).max(1),
    }
}

/// Ce qu'on sait de la charge de l'appareil.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Mesure {
    /// Les fils qui calculent, et le plus permis.
    pub actifs: u8,
    pub max: u8,
    pub coeurs: u8,
    /// La charge processeur de Flow, en % de tous les cœurs.
    pub cpu: f32,
    /// La mémoire disponible de l'appareil et celle de Flow (Mo).
    pub libre: u32,
    pub flow: u32,
    /// La température la plus haute lue (°C ; NaN si aucune).
    pub temperature: f32,
}

impl Mesure {
    /// Les octets de la mesure (pour le renfort) : 3 octets, 4 flottants.
    pub fn octets(&self) -> Vec<u8> {
        let mut o = vec![self.actifs, self.max, self.coeurs];
        for x in [self.cpu, self.libre as f32, self.flow as f32, self.temperature] {
            o.extend_from_slice(&x.to_le_bytes());
        }
        o
    }

    pub const TAILLE: usize = 19;

    pub fn lire(o: &[u8]) -> Option<Mesure> {
        if o.len() < Self::TAILLE {
            return None;
        }
        let f = |k: usize| f32::from_le_bytes([o[k], o[k + 1], o[k + 2], o[k + 3]]);
        Some(Mesure { actifs: o[0], max: o[1], coeurs: o[2], cpu: f(3), libre: f(7) as u32, flow: f(11) as u32, temperature: f(15) })
    }

    /// Une ligne lisible : « 3/5 fils · CPU 62 % · libre 1,8 Go · Flow 640 Mo · 41 °C ».
    pub fn texte(&self) -> String {
        let go = |mo: u32| if mo >= 1000 { format!("{:.1} Go", mo as f32 / 1000.0).replace('.', ",") } else { format!("{mo} Mo") };
        let t = if self.temperature.is_nan() { String::new() } else { format!(" · {:.0} °C", self.temperature) };
        format!("{}/{} fils ({} cœurs) · CPU {:.0} % · libre {} · Flow {}{t}", self.actifs, self.max, self.coeurs, self.cpu, go(self.libre), go(self.flow))
    }
}

/// La mémoire disponible de l'appareil (Mo).
pub fn memoire_libre() -> Option<u64> {
    let t = std::fs::read_to_string("/proc/meminfo").ok()?;
    t.lines().find(|l| l.starts_with("MemAvailable:"))?.split_whitespace().nth(1)?.parse::<u64>().ok().map(|k| k / 1024)
}

/// La mémoire occupée par Flow (Mo).
pub fn memoire_flow() -> Option<u64> {
    let t = std::fs::read_to_string("/proc/self/status").ok()?;
    t.lines().find(|l| l.starts_with("VmRSS:"))?.split_whitespace().nth(1)?.parse::<u64>().ok().map(|k| k / 1024)
}

/// La température la plus haute des zones thermiques lisibles (°C).
pub fn temperature() -> Option<f32> {
    let mut max: Option<f32> = None;
    for e in std::fs::read_dir("/sys/class/thermal").ok()?.flatten() {
        if !e.file_name().to_string_lossy().starts_with("thermal_zone") {
            continue;
        }
        if let Some(v) = std::fs::read_to_string(e.path().join("temp")).ok().and_then(|t| t.trim().parse::<f32>().ok()) {
            // En millidegrés le plus souvent ; ignorer les valeurs absurdes
            let c = if v.abs() > 1000.0 { v / 1000.0 } else { v };
            if (5.0..130.0).contains(&c) {
                max = Some(max.map_or(c, |m: f32| m.max(c)));
            }
        }
    }
    max
}

/// Le temps processeur de Flow (s, tous fils compris).
fn temps_cpu() -> Option<f64> {
    let t = std::fs::read_to_string("/proc/self/stat").ok()?;
    // Après le nom (entre parenthèses) : utime et stime sont les champs 14 et 15
    let apres = &t[t.rfind(')')? + 2..];
    let v: Vec<&str> = apres.split_whitespace().collect();
    let (u, s) = (v.get(11)?.parse::<f64>().ok()?, v.get(12)?.parse::<f64>().ok()?);
    Some((u + s) / 100.0)
}

/// Le régulateur des fils de calcul d'un travail (génération ou renfort).
pub struct Regulateur {
    /// Les fils permis à cet instant.
    permis: AtomicUsize,
    /// Les fils qui calculent une tuile.
    actifs: AtomicUsize,
    interne: Mutex<Interne>,
}

struct Interne {
    dernier: Instant,
    alerte: Instant,
    cpu: Option<(Instant, f64)>,
    mesure: Mesure,
}

impl Default for Regulateur {
    fn default() -> Self {
        Self::new()
    }
}

impl Regulateur {
    pub fn new() -> Regulateur {
        let t = Instant::now();
        Regulateur { permis: AtomicUsize::new(fils_max()), actifs: AtomicUsize::new(0), interne: Mutex::new(Interne { dernier: t - PAS, alerte: t, cpu: None, mesure: Mesure::default() }) }
    }

    /// Le fil `i` (de 0) peut-il prendre une tuile ?
    pub fn permis(&self, i: usize) -> bool {
        self.ajuster();
        i < self.permis.load(Ordering::Relaxed)
    }

    /// Une tuile commence (`garde` la compte tant qu'elle vit).
    pub fn commencer(&self) -> Garde<'_> {
        self.actifs.fetch_add(1, Ordering::Relaxed);
        Garde(self)
    }

    /// La dernière mesure.
    pub fn mesure(&self) -> Mesure {
        self.ajuster();
        self.interne.lock().map(|i| i.mesure).unwrap_or_default()
    }

    /// Mesure et ajuste les fils permis (au plus tous les `PAS`).
    fn ajuster(&self) {
        let Ok(mut i) = self.interne.try_lock() else { return };
        let t = Instant::now();
        if t.duration_since(i.dernier) < PAS {
            return;
        }
        i.dernier = t;
        let max = fils_max();
        let libre = memoire_libre();
        let temp = temperature();
        let cpu = temps_cpu().map(|c| {
            let pct = i.cpu.map_or(0.0, |(t0, c0)| 100.0 * (c - c0) / t.duration_since(t0).as_secs_f64().max(1e-3) / coeurs() as f64);
            i.cpu = Some((t, c));
            pct as f32
        });
        let mut permis = self.permis.load(Ordering::Relaxed).min(max).max(1);
        if FILS.load(Ordering::Relaxed) != 0 {
            // Un nombre réglé à la main : tenu tel quel
            permis = max;
        } else {
            let alerte = libre.is_some_and(|l| l < MEMOIRE_BASSE) || temp.is_some_and(|c| c > CHAUD);
            let calme = libre.map_or(true, |l| l > MEMOIRE_HAUTE) && temp.map_or(true, |c| c < TIEDE);
            if alerte {
                permis = permis.saturating_sub(1).max(1);
                i.alerte = t;
            } else if calme && t.duration_since(i.alerte) >= CALME && permis < max {
                permis += 1;
                i.alerte = t;
            }
        }
        self.permis.store(permis, Ordering::Relaxed);
        i.mesure = Mesure {
            actifs: self.actifs.load(Ordering::Relaxed).min(255) as u8,
            max: permis.min(255) as u8,
            coeurs: coeurs().min(255) as u8,
            cpu: cpu.unwrap_or(i.mesure.cpu),
            libre: libre.unwrap_or(0) as u32,
            flow: memoire_flow().unwrap_or(0) as u32,
            temperature: temp.unwrap_or(f32::NAN),
        };
    }
}

/// Une tuile en cours de calcul (décomptée en finissant).
pub struct Garde<'a>(&'a Regulateur);

impl Drop for Garde<'_> {
    fn drop(&mut self) {
        self.0.actifs.fetch_sub(1, Ordering::Relaxed);
    }
}
