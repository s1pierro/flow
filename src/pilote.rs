//! Le pilotage de Flow depuis une autre application (Debian, par Termux), par le bus
//! d'Android : chaque commande arrive au récepteur `Pilote` (Java) en diffusion
//! explicite, avec un jeton :
//!
//! ```text
//! am broadcast -n fr.s1pierro.flow/.Pilote -a fr.s1pierro.flow.PILOTE \
//!     --es jeton "$(cat /sdcard/Flow/banc/jeton)" --es cmd "capture"
//! ```
//!
//! La réponse revient dans le même échange : sa première ligne est `ok` ou `erreur`,
//! la suite le détail. Le pilotage s'arme dans les Paramètres (Interface › Pilotage) ;
//! il repart désarmé à chaque lancement. En l'armant, Flow tire un jeton au hasard et
//! l'écrit dans `Flow/banc/jeton` : une commande sans ce jeton est refusée.
//!
//! Les commandes (coordonnées en pixels de l'écran) :
//! - `ping` : la version de Flow ;
//! - `etat` : l'écran, et chaque fenêtre (numéro, contexte, cadre, document) ;
//! - `contexte <clé> [document]` : ouvre un contexte dans la plus grande fenêtre ;
//! - `capture [nom]` : l'écran en PNG dans `Flow/banc/captures/` ; rend son chemin ;
//! - `toucher <x> <y> [durée ms]` : un doigt posé puis levé ;
//! - `glisser <x1> <y1> <x2> <y2> [durée ms]` : un doigt qui glisse.
//!
//! Le récepteur appelle `commande` sur un thread à part ; ce qui touche à l'interface
//! s'exécute sur le thread de rendu (`traiter`, `apres_dessin`), qui répond par un
//! canal. Une commande sans réponse au bout de `ATTENTE` est déclarée perdue (Flow en
//! arrière-plan : le rendu est suspendu).

use crate::app::App;
use crate::ui::Event;
use std::collections::VecDeque;
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// L'attente d'une réponse du thread de rendu.
const ATTENTE: Duration = Duration::from_secs(20);
/// Les doigts du pilote : leurs identifiants, loin de ceux de l'écran et de la
/// télécommande.
const DOIGT: i32 = 9_000;

static ARME: AtomicBool = AtomicBool::new(false);
static JETON: Mutex<String> = Mutex::new(String::new());
/// Le dossier du banc (`Flow/banc`), connu une fois le pilotage armé.
static DOSSIER: Mutex<Option<PathBuf>> = Mutex::new(None);
/// Les commandes en attente du thread de rendu.
static FILE: Mutex<VecDeque<(Vec<String>, Sender<String>)>> = Mutex::new(VecDeque::new());
/// Les événements des doigts du pilote, chacun à son heure.
static DOIGTS: Mutex<VecDeque<(Event, Instant)>> = Mutex::new(VecDeque::new());
/// Une capture demandée : son chemin, et à qui répondre.
static CAPTURE: Mutex<Option<(PathBuf, Sender<String>)>> = Mutex::new(None);

pub fn arme() -> bool {
    ARME.load(Ordering::Relaxed)
}

/// Arme (ou désarme) le pilotage. En l'armant, un jeton neuf est tiré et écrit dans
/// `<dossier>/banc/jeton` ; rend le chemin du jeton, ou l'erreur.
pub fn armer(oui: bool, dossier: Option<&Path>) -> Result<String, String> {
    if !oui {
        ARME.store(false, Ordering::Relaxed);
        if let Ok(mut j) = JETON.lock() {
            j.clear();
        }
        return Ok(String::new());
    }
    let banc = dossier.ok_or("le dossier de Flow est inconnu")?.join("banc");
    std::fs::create_dir_all(banc.join("captures")).map_err(|e| e.to_string())?;
    let jeton = tirer_jeton()?;
    let chemin = banc.join("jeton");
    std::fs::write(&chemin, format!("{jeton}\n")).map_err(|e| e.to_string())?;
    *JETON.lock().map_err(|e| e.to_string())? = jeton;
    *DOSSIER.lock().map_err(|e| e.to_string())? = Some(banc);
    ARME.store(true, Ordering::Relaxed);
    Ok(chemin.display().to_string())
}

/// 32 caractères hexadécimaux tirés de /dev/urandom.
fn tirer_jeton() -> Result<String, String> {
    use std::io::Read;
    let mut b = [0u8; 16];
    std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut b)).map_err(|e| e.to_string())?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}

/// Les mots d'une commande : séparés par des espaces, un mot entre guillemets peut en
/// contenir.
pub fn mots(cmd: &str) -> Vec<String> {
    let mut v = Vec::new();
    let mut m = String::new();
    let (mut dans, mut entame) = (false, false);
    for c in cmd.chars() {
        match c {
            '"' => {
                dans = !dans;
                entame = true;
            }
            c if c.is_whitespace() && !dans => {
                if entame {
                    v.push(std::mem::take(&mut m));
                    entame = false;
                }
            }
            c => {
                m.push(c);
                entame = true;
            }
        }
    }
    if entame {
        v.push(m);
    }
    v
}

/// Exécute une commande venue du bus (sur le thread du récepteur) ; rend la réponse.
pub fn commande(jeton: &str, cmd: &str) -> String {
    if !arme() {
        return "erreur\nLe pilotage n'est pas armé (Paramètres › Interface › Pilotage).".into();
    }
    let bon = JETON.lock().map(|j| !j.is_empty() && *j == jeton.trim()).unwrap_or(false);
    if !bon {
        return "erreur\nJeton refusé.".into();
    }
    let m = mots(cmd);
    let Some(nom) = m.first() else { return "erreur\nCommande vide.".into() };
    if nom == "ping" {
        return format!("ok\nFlow {}", env!("CARGO_PKG_VERSION"));
    }
    let (tx, rx) = channel();
    match FILE.lock() {
        Ok(mut f) => f.push_back((m, tx)),
        Err(_) => return "erreur\nFile bloquée.".into(),
    }
    crate::radio::reveiller();
    match rx.recv_timeout(ATTENTE) {
        Ok(r) => r,
        Err(_) => "erreur\nPas de réponse : Flow est-il au premier plan ?".into(),
    }
}

/// L'entrée du récepteur Java (`Pilote.commande`).
#[no_mangle]
pub extern "C" fn Java_fr_s1pierro_flow_Pilote_commande(env: *mut c_void, _classe: *mut c_void, jeton: *mut c_void, cmd: *mut c_void) -> *mut c_void {
    let (j, c) = unsafe { (crate::acces::lire_jstring(env, jeton).unwrap_or_default(), crate::acces::lire_jstring(env, cmd).unwrap_or_default()) };
    let r = commande(&j, &c);
    unsafe { crate::acces::jstring(env, &r) }
}

/// Des événements de doigts sont-ils encore à venir (le rendu doit continuer) ?
pub fn occupe() -> bool {
    DOIGTS.lock().map(|d| !d.is_empty()).unwrap_or(false) || CAPTURE.lock().map(|c| c.is_some()).unwrap_or(false)
}

/// Les événements des doigts du pilote dont l'heure est venue.
pub fn evenements() -> Vec<Event> {
    let Ok(mut d) = DOIGTS.lock() else { return Vec::new() };
    let t = Instant::now();
    let mut v = Vec::new();
    while d.front().is_some_and(|e| e.1 <= t) {
        v.push(d.pop_front().unwrap().0);
    }
    v
}

fn nombre(m: &[String], i: usize) -> Result<f32, String> {
    m.get(i).ok_or(format!("argument {i} manquant"))?.parse::<f32>().map_err(|_| format!("« {} » n'est pas un nombre", m[i]))
}

/// Exécute les commandes en attente, sur le thread de rendu, avant le dessin d'une
/// image de `w` × `h` pixels.
pub fn traiter(app: &mut App, w: f32, h: f32, dp: f32) {
    let file: Vec<_> = match FILE.lock() {
        Ok(mut f) => f.drain(..).collect(),
        Err(_) => return,
    };
    for (m, tx) in file {
        match executer(app, &m, w, h, dp, &tx) {
            Ok(Some(r)) => {
                let _ = tx.send(format!("ok\n{r}"));
            }
            // La réponse viendra plus tard (capture)
            Ok(None) => {}
            Err(e) => {
                let _ = tx.send(format!("erreur\n{e}"));
            }
        }
    }
}

fn executer(app: &mut App, m: &[String], w: f32, h: f32, dp: f32, tx: &Sender<String>) -> Result<Option<String>, String> {
    match m[0].as_str() {
        "etat" => {
            let mut r = format!("version {}\necran {w} {h} dp {dp}\n", env!("CARGO_PKG_VERSION"));
            r.push_str(&app.decrire_fenetres());
            Ok(Some(r))
        }
        "contexte" => {
            let cle = m.get(1).ok_or("contexte <clé> [document]")?;
            app.piloter_contexte(cle, m.get(2).map(|s| s.as_str()))?;
            Ok(Some(format!("{cle} demandé")))
        }
        "capture" => {
            let dossier = DOSSIER.lock().ok().and_then(|d| d.clone()).ok_or("dossier du banc inconnu")?;
            let nom = match m.get(1) {
                Some(n) => n.trim_end_matches(".png").replace('/', "_"),
                None => format!("capture-{}", crate::disque::maintenant()),
            };
            let chemin = dossier.join("captures").join(format!("{nom}.png"));
            *CAPTURE.lock().map_err(|e| e.to_string())? = Some((chemin, tx.clone()));
            Ok(None)
        }
        "toucher" => {
            let (x, y) = (nombre(m, 1)?, nombre(m, 2)?);
            let duree = if m.len() > 3 { nombre(m, 3)? } else { 80.0 };
            let t = Instant::now();
            let mut d = DOIGTS.lock().map_err(|e| e.to_string())?;
            d.push_back((Event::Down { id: DOIGT, x, y }, t));
            d.push_back((Event::Up { id: DOIGT }, t + Duration::from_secs_f32(duree.max(1.0) / 1000.0)));
            Ok(Some(format!("toucher {x} {y}")))
        }
        "glisser" => {
            let (x1, y1, x2, y2) = (nombre(m, 1)?, nombre(m, 2)?, nombre(m, 3)?, nombre(m, 4)?);
            let duree = if m.len() > 5 { nombre(m, 5)? } else { 300.0 }.max(16.0) / 1000.0;
            let pas = ((duree / 0.016).ceil() as usize).max(2);
            let t = Instant::now();
            let mut d = DOIGTS.lock().map_err(|e| e.to_string())?;
            d.push_back((Event::Down { id: DOIGT + 1, x: x1, y: y1 }, t));
            for i in 1..=pas {
                let f = i as f32 / pas as f32;
                d.push_back((Event::Move { id: DOIGT + 1, x: x1 + (x2 - x1) * f, y: y1 + (y2 - y1) * f }, t + Duration::from_secs_f32(duree * f)));
            }
            d.push_back((Event::Up { id: DOIGT + 1 }, t + Duration::from_secs_f32(duree + 0.02)));
            Ok(Some(format!("glisser {x1} {y1} -> {x2} {y2}")))
        }
        autre => Err(format!("commande inconnue : {autre} (ping, etat, contexte, capture, toucher, glisser)")),
    }
}

/// Après le dessin d'une image : la capture demandée, lue par `lire` (les pixels RGBA
/// de `w` × `h`, de bas en haut), écrite en PNG.
pub fn apres_dessin(w: usize, h: usize, lire: impl FnOnce() -> Vec<u8>) {
    let Some((chemin, tx)) = CAPTURE.lock().ok().and_then(|mut c| c.take()) else { return };
    let px = lire();
    let r = match ecrire_png(&chemin, w, h, &px) {
        Ok(()) => format!("ok\n{}\n{w} {h}", chemin.display()),
        Err(e) => format!("erreur\ncapture : {e}"),
    };
    let _ = tx.send(r);
}

/// Écrit des pixels RGBA (`w` × `h`, de bas en haut comme les rend OpenGL) en PNG RGB,
/// retourné à l'endroit. Sans dépendance : des blocs « stockés » de zlib, sans
/// compression.
pub fn ecrire_png(chemin: &Path, w: usize, h: usize, rgba: &[u8]) -> Result<(), String> {
    if rgba.len() < w * h * 4 {
        return Err("pixels incomplets".into());
    }
    let mut brut = Vec::with_capacity(h * (w * 3 + 1));
    for y in (0..h).rev() {
        brut.push(0);
        for x in 0..w {
            let i = (y * w + x) * 4;
            brut.extend_from_slice(&rgba[i..i + 3]);
        }
    }
    let mut z = vec![0x78, 0x01];
    let mut morceaux = brut.chunks(65_535).peekable();
    while let Some(c) = morceaux.next() {
        z.push(if morceaux.peek().is_none() { 1 } else { 0 });
        let n = c.len() as u16;
        z.extend_from_slice(&n.to_le_bytes());
        z.extend_from_slice(&(!n).to_le_bytes());
        z.extend_from_slice(c);
    }
    if brut.is_empty() {
        z.extend_from_slice(&[1, 0, 0, 0xff, 0xff]);
    }
    z.extend_from_slice(&adler32(&brut).to_be_bytes());
    let mut png = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&(w as u32).to_be_bytes());
    ihdr.extend_from_slice(&(h as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    morceau(&mut png, b"IHDR", &ihdr);
    morceau(&mut png, b"IDAT", &z);
    morceau(&mut png, b"IEND", &[]);
    if let Some(d) = chemin.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    std::fs::write(chemin, png).map_err(|e| e.to_string())
}

fn morceau(png: &mut Vec<u8>, genre: &[u8; 4], donnees: &[u8]) {
    png.extend_from_slice(&(donnees.len() as u32).to_be_bytes());
    let debut = png.len();
    png.extend_from_slice(genre);
    png.extend_from_slice(donnees);
    let c = crc32(&png[debut..]);
    png.extend_from_slice(&c.to_be_bytes());
}

fn crc32(b: &[u8]) -> u32 {
    let mut c = 0xffff_ffffu32;
    for &x in b {
        c ^= x as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
        }
    }
    !c
}

fn adler32(b: &[u8]) -> u32 {
    let (mut a, mut s) = (1u32, 0u32);
    for c in b.chunks(5552) {
        for &x in c {
            a += x as u32;
            s += a;
        }
        a %= 65_521;
        s %= 65_521;
    }
    (s << 16) | a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn les_mots_gardent_les_guillemets() {
        assert_eq!(mots(r#"contexte lieux "/sdcard/Flow/lieux/un lieu.reel""#), vec!["contexte", "lieux", "/sdcard/Flow/lieux/un lieu.reel"]);
        assert_eq!(mots("  toucher 10   20 "), vec!["toucher", "10", "20"]);
    }

    #[test]
    fn le_png_a_ses_morceaux_et_un_crc_juste() {
        assert_eq!(crc32(b"IEND"), 0xae42_6082);
        let d = std::env::temp_dir().join("flow-pilote-essai.png");
        ecrire_png(&d, 3, 2, &[200u8; 3 * 2 * 4]).unwrap();
        let b = std::fs::read(&d).unwrap();
        assert_eq!(&b[..8], &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
        assert_eq!(&b[b.len() - 8..b.len() - 4], b"IEND");
    }

    #[test]
    fn sans_armement_tout_est_refuse() {
        assert!(commande("x", "ping").starts_with("erreur"));
    }
}
