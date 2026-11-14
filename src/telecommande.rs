//! La télécommande : un second appareil pilote Flow par le Wi-Fi local.
//!
//! - **Le transport** : UDP, sur le port `PORT`, un message par paquet, en une ligne de
//!   texte (`Message::texte`, `Message::lire`). Un paquet perdu ne retarde pas les
//!   suivants, ce qui compte plus ici que la fiabilité : un mouvement perdu est vite
//!   rattrapé par le suivant.
//! - **La découverte** : la télécommande diffuse « FLOW? » sur le réseau ; chaque Flow
//!   qui accepte d'être piloté répond « FLOW! ». On peut aussi taper l'adresse.
//! - **La liaison** : l'appareil piloté montre un code à quatre chiffres, que la
//!   télécommande renvoie (« LIE 1234 »). Ensuite seuls ses messages sont écoutés.
//! - **Côté piloté** (`ecouter`) : un thread reçoit, range les messages dans des files
//!   que la boucle de rendu vide à chaque image (`evenements`, `frappes`) et la
//!   réveille. Les doigts distants deviennent des événements comme ceux de l'écran
//!   (identifiants à partir de `ID0`) : tout Flow les comprend, gestes compris.
//! - **Côté télécommande** (`Client`) : une socket sans blocage, lue à chaque image ; un
//!   ping toutes les `PING` s mesure l'aller-retour (dernier, moyenne, pire, pertes).
//! - **Le Wi-Fi** : tant que la télécommande sert (d'un côté ou de l'autre), le Wi-Fi est
//!   gardé en faible latence et les diffusions passent (`acces::verrous_wifi`).

use crate::clavier::Frappe;
use crate::ui::Event;
use std::collections::VecDeque;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[cfg(not(test))]
pub const PORT: u16 = 47_811;
/// Les essais ont leur port : Flow peut écouter sur l'appareil qui les lance.
#[cfg(test)]
pub const PORT: u16 = 47_819;
/// Les identifiants des doigts distants (loin de ceux de l'écran).
pub const ID0: i32 = 1000;
/// L'intervalle des pings, en secondes.
pub const PING: f32 = 0.2;
/// Le doigt virtuel du défilement à deux doigts.
const ID_DEFILE: i32 = ID0 + 999;

/// L'état d'une manette de jeu : sticks de −1 à 1 (y vers le bas, comme Android),
/// gâchettes de 0 à 1, boutons en bits (`bouton`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Manette {
    pub gauche: [f32; 2],
    pub droit: [f32; 2],
    pub lt: f32,
    pub rt: f32,
    pub boutons: u32,
}

/// Les bits des boutons de `Manette`, dans l'ordre du protocole.
pub mod bouton {
    pub const A: u32 = 1 << 0;
    pub const B: u32 = 1 << 1;
    pub const X: u32 = 1 << 2;
    pub const Y: u32 = 1 << 3;
    pub const L1: u32 = 1 << 4;
    pub const R1: u32 = 1 << 5;
    pub const SELECT: u32 = 1 << 6;
    pub const START: u32 = 1 << 7;
    pub const HAUT: u32 = 1 << 8;
    pub const BAS: u32 = 1 << 9;
    pub const GAUCHE: u32 = 1 << 10;
    pub const DROITE: u32 = 1 << 11;
    pub const STICK_G: u32 = 1 << 12;
    pub const STICK_D: u32 = 1 << 13;
}

impl Manette {
    pub fn appuye(&self, b: u32) -> bool {
        self.boutons & b != 0
    }

    /// Au repos : rien d'enfoncé, sticks au centre.
    pub fn au_repos(&self) -> bool {
        self.boutons == 0 && self.lt == 0.0 && self.rt == 0.0 && self.gauche == [0.0; 2] && self.droit == [0.0; 2]
    }
}

/// Une manette muette depuis ce temps (s) est remise au repos : un paquet perdu ne
/// laisse pas une voiture accélérer.
pub const MANETTE_MUETTE: f32 = 1.0;

/// Un message de la télécommande ou de l'appareil piloté.
#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    /// « FLOW? » : qui accepte d'être piloté ?
    Qui,
    /// « FLOW! nom » : moi.
    Moi(String),
    /// « LIE code » : se lier avec ce code ; « LIE OK » / « LIE NON » : la réponse.
    Lier(u32),
    Lie(bool),
    /// « P n t » : un ping, renvoyé tel quel.
    Ping(u32, u64),
    /// Un doigt sur l'écran (de 0 à 1 dans les deux sens) : posé, glissé, levé.
    Pose(i32, f32, f32),
    Glisse(i32, f32, f32),
    Leve(i32),
    /// Le pavé : déplacer le curseur (en dp, l'accélération déjà appliquée par la
    /// télécommande), cliquer, tenir ou lâcher.
    Relatif(f32, f32),
    /// Le défilement à deux doigts (en dp) : un glissé sous le curseur, qui ne le
    /// déplace pas ; « S0 » le termine.
    Defile(f32, f32),
    FinDefile,
    /// « T w h » : la taille de l'écran piloté (en pixels), envoyée avec chaque ping,
    /// pour que le miroir en garde les proportions.
    Taille(f32, f32),
    Clic,
    Bouton(bool),
    /// Une frappe du clavier.
    Touche(Frappe),
    /// La télécommande sert de clavier (celui de l'écran piloté se cache), ou plus.
    Clavier(bool),
    /// « G lx ly rx ry lt rt boutons » : l'état de la manette, envoyé à chaque
    /// changement et toutes les 0,1 s.
    Manette(Manette),
    /// « Q » : la télécommande s'en va.
    Fin,
}

impl Message {
    pub fn texte(&self) -> String {
        match self {
            Message::Qui => "FLOW?".into(),
            Message::Moi(n) => format!("FLOW! {n}"),
            Message::Lier(c) => format!("LIE {c:04}"),
            Message::Lie(ok) => format!("LIE {}", if *ok { "OK" } else { "NON" }),
            Message::Ping(n, t) => format!("P {n} {t}"),
            Message::Pose(i, x, y) => format!("D {i} {x:.5} {y:.5}"),
            Message::Glisse(i, x, y) => format!("M {i} {x:.5} {y:.5}"),
            Message::Leve(i) => format!("U {i}"),
            Message::Relatif(x, y) => format!("R {x:.5} {y:.5}"),
            Message::Defile(x, y) => format!("S {x:.3} {y:.3}"),
            Message::FinDefile => "S0".into(),
            Message::Taille(w, h) => format!("T {w:.0} {h:.0}"),
            Message::Clic => "C".into(),
            Message::Bouton(b) => format!("B {}", *b as u8),
            Message::Touche(f) => match f {
                Frappe::Car(c) => format!("K c {c}"),
                Frappe::Effacer => "K e".into(),
                Frappe::Entree => "K n".into(),
                Frappe::Ligne => "K l".into(),
                Frappe::Gauche => "K g".into(),
                Frappe::Droite => "K d".into(),
            },
            Message::Clavier(b) => format!("KB {}", *b as u8),
            Message::Manette(m) => format!("G {:.3} {:.3} {:.3} {:.3} {:.3} {:.3} {}", m.gauche[0], m.gauche[1], m.droit[0], m.droit[1], m.lt, m.rt, m.boutons),
            Message::Fin => "Q".into(),
        }
    }

    pub fn lire(s: &str) -> Option<Message> {
        let mut m = s.splitn(2, ' ');
        let tete = m.next()?;
        let reste = m.next().unwrap_or("");
        let mots: Vec<&str> = reste.split(' ').collect();
        let f = |i: usize| mots.get(i).and_then(|v| v.parse::<f32>().ok()).filter(|v| v.is_finite());
        let n = |i: usize| mots.get(i).and_then(|v| v.parse::<i32>().ok());
        Some(match tete {
            "FLOW?" => Message::Qui,
            "FLOW!" => Message::Moi(reste.chars().take(40).collect()),
            "LIE" => match reste {
                "OK" => Message::Lie(true),
                "NON" => Message::Lie(false),
                c => Message::Lier(c.parse().ok()?),
            },
            "P" => Message::Ping(mots.first()?.parse().ok()?, mots.get(1)?.parse().ok()?),
            "D" => Message::Pose(n(0)?, f(1)?, f(2)?),
            "M" => Message::Glisse(n(0)?, f(1)?, f(2)?),
            "U" => Message::Leve(n(0)?),
            "R" => Message::Relatif(f(0)?, f(1)?),
            "S" => Message::Defile(f(0)?, f(1)?),
            "S0" => Message::FinDefile,
            "T" => Message::Taille(f(0)?.max(1.0), f(1)?.max(1.0)),
            "C" => Message::Clic,
            "B" => Message::Bouton(reste == "1"),
            "K" => Message::Touche(match (mots.first().copied()?, reste.get(2..)) {
                ("c", Some(c)) => Frappe::Car(c.chars().next()?),
                ("e", _) => Frappe::Effacer,
                ("n", _) => Frappe::Entree,
                ("l", _) => Frappe::Ligne,
                ("g", _) => Frappe::Gauche,
                ("d", _) => Frappe::Droite,
                _ => return None,
            }),
            "KB" => Message::Clavier(reste == "1"),
            "G" => {
                let a = |i: usize| f(i).map(|v| v.clamp(-1.0, 1.0));
                Message::Manette(Manette { gauche: [a(0)?, a(1)?], droit: [a(2)?, a(3)?], lt: a(4)?.max(0.0), rt: a(5)?.max(0.0), boutons: mots.get(6)?.parse().ok()? })
            }
            "Q" => Message::Fin,
            _ => return None,
        })
    }
}

/// L'adresse de cet appareil sur le Wi-Fi et la longueur du préfixe de son réseau
/// (demandées à Android ; ailleurs, l'adresse de sortie et /24).
pub fn reseau_local() -> Option<(Ipv4Addr, u8)> {
    if let Some(t) = crate::acces::wifi() {
        let (ip, p) = t.split_once('/')?;
        return Some((ip.parse().ok()?, p.parse().ok()?));
    }
    adresse_sortie().map(|a| (a, 24))
}

/// L'adresse de cet appareil sur le réseau local, s'il en a.
pub fn adresse_locale() -> Option<Ipv4Addr> {
    reseau_local().map(|r| r.0)
}

/// Les adresses à interroger pour trouver les Flow : la diffusion générale, celle du
/// réseau, puis (si les diffusions se perdent, ce qui arrive sur bien des Wi-Fi) chaque
/// adresse du réseau, une à une (au plus 1022).
pub fn adresses_a_sonder(local: Option<(Ipv4Addr, u8)>, une_a_une: bool) -> Vec<Ipv4Addr> {
    let mut v = vec![Ipv4Addr::BROADCAST];
    let Some((ip, p)) = local else { return v };
    let p = p.clamp(22, 30) as u32;
    let masque = u32::MAX << (32 - p);
    let reseau = u32::from(ip) & masque;
    let diffusion = reseau | !masque;
    v.push(Ipv4Addr::from(diffusion));
    if une_a_une {
        v.extend((reseau + 1..diffusion).filter(|&a| a != u32::from(ip)).map(Ipv4Addr::from));
    }
    v
}

/// L'adresse par laquelle cet appareil sort (sans rien envoyer).
fn adresse_sortie() -> Option<Ipv4Addr> {
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    // Aucun paquet ne part : la socket choisit seulement son interface
    s.connect("192.0.2.1:9").ok()?;
    match s.local_addr().ok()? {
        SocketAddr::V4(a) if !a.ip().is_unspecified() => Some(*a.ip()),
        _ => None,
    }
}

// ----- Côté piloté -----------------------------------------------------------------

/// L'état de l'appareil piloté, partagé entre son thread et la boucle de rendu.
#[derive(Default)]
pub struct Hote {
    pub code: u32,
    /// La télécommande liée, et quand elle a parlé pour la dernière fois.
    pub lie: Option<SocketAddr>,
    pub vu: Option<Instant>,
    pub recus: u64,
    pub erreur: Option<String>,
    doigts: VecDeque<Message>,
    touches: VecDeque<Frappe>,
    /// Le curseur du pavé (en pixels de l'écran), quand il a bougé, si le bouton est
    /// tenu ; la télécommande sert-elle de clavier ?
    curseur: Option<(f32, f32)>,
    curseur_vu: Option<Instant>,
    tenu: bool,
    clavier: bool,
    /// Le défilement en cours : où son doigt virtuel est rendu.
    defile: Option<(f32, f32)>,
    /// La manette de la télécommande, et quand elle a parlé.
    manette: Option<(Manette, Instant)>,
}

struct Ecoute {
    hote: Arc<Mutex<Hote>>,
    arret: Arc<AtomicBool>,
}

static ECOUTE: Mutex<Option<Ecoute>> = Mutex::new(None);
static TAILLE: Mutex<(f32, f32)> = Mutex::new((0.0, 0.0));

fn hote() -> Option<Arc<Mutex<Hote>>> {
    ECOUTE.lock().ok()?.as_ref().map(|e| e.hote.clone())
}

/// Accepter une télécommande (ou plus) ; l'erreur si le port est pris.
pub fn ecouter(oui: bool) -> Result<(), String> {
    let mut e = ECOUTE.lock().map_err(|e| e.to_string())?;
    if !oui {
        if let Some(x) = e.take() {
            x.arret.store(true, Ordering::Relaxed);
            let _ = crate::acces::verrous_wifi(false);
        }
        return Ok(());
    }
    if e.is_some() {
        return Ok(());
    }
    // Les verrous d'abord : ils lient aussi les sockets à venir au Wi-Fi
    let verrous = crate::acces::verrous_wifi(true).filter(|e| !e.is_empty());
    let socket = UdpSocket::bind(("0.0.0.0", PORT)).map_err(|e| format!("port {PORT} : {e}"))?;
    socket.set_read_timeout(Some(Duration::from_millis(250))).map_err(|e| e.to_string())?;
    let _ = socket.set_broadcast(true);
    let code = (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos()) / 1000) % 9000 + 1000;
    let hote = Arc::new(Mutex::new(Hote { code, erreur: verrous.map(|e| format!("Wi-Fi : {e}")), ..Default::default() }));
    let arret = Arc::new(AtomicBool::new(false));
    let (h, a) = (hote.clone(), arret.clone());
    std::thread::spawn(move || boucle_hote(socket, h, a));
    *e = Some(Ecoute { hote, arret });
    Ok(())
}

/// Flow accepte-t-il une télécommande ? Son code, la télécommande liée, ses messages.
pub fn etat_hote() -> Option<(u32, Option<SocketAddr>, Option<f32>, u64, Option<String>)> {
    let h = hote()?;
    let h = h.lock().ok()?;
    Some((h.code, h.lie, h.vu.map(|v| v.elapsed().as_secs_f32()), h.recus, h.erreur.clone()))
}

fn boucle_hote(socket: UdpSocket, h: Arc<Mutex<Hote>>, arret: Arc<AtomicBool>) {
    let mut b = [0u8; 512];
    while !arret.load(Ordering::Relaxed) {
        let (n, de) = match socket.recv_from(&mut b) {
            Ok(x) => x,
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => continue,
            Err(e) => {
                if let Ok(mut h) = h.lock() {
                    h.erreur = Some(e.to_string());
                }
                std::thread::sleep(Duration::from_millis(200));
                continue;
            }
        };
        let Some(m) = std::str::from_utf8(&b[..n]).ok().and_then(Message::lire) else { continue };
        let mut taille = None;
        let reponse = {
            let Ok(mut h) = h.lock() else { return };
            match m {
                Message::Qui => Some(Message::Moi("Flow".into())),
                // Un ping, même d'une télécommande pas encore liée : rien n'en dépend
                Message::Ping(..) => {
                    taille = TAILLE.lock().ok().map(|t| *t).filter(|t| t.0 > 0.0);
                    Some(m)
                }
                Message::Lier(c) => {
                    let ok = c == h.code;
                    if ok {
                        h.lie = Some(de);
                        h.vu = Some(Instant::now());
                        h.clavier = false;
                    }
                    Some(Message::Lie(ok))
                }
                _ if h.lie != Some(de) => None,
                Message::Fin => {
                    h.lie = None;
                    h.clavier = false;
                    h.manette = None;
                    // Un bouton tenu se lâche
                    if std::mem::take(&mut h.tenu) {
                        h.doigts.push_back(Message::Bouton(false));
                    }
                    None
                }
                Message::Touche(f) => {
                    h.touches.push_back(f);
                    None
                }
                Message::Clavier(oui) => {
                    h.clavier = oui;
                    None
                }
                Message::Manette(m) => {
                    h.manette = Some((m, Instant::now()));
                    None
                }
                m => {
                    h.doigts.push_back(m);
                    None
                }
            }
        };
        if let Ok(mut hh) = h.lock() {
            hh.recus += 1;
            if hh.lie == Some(de) {
                hh.vu = Some(Instant::now());
            }
        }
        for r in reponse.into_iter().chain(taille.map(|(w, h)| Message::Taille(w, h))) {
            let _ = socket.send_to(r.texte().as_bytes(), de);
        }
        crate::radio::reveiller();
    }
}

/// Les événements des doigts distants arrivés depuis la dernière image, pour un écran
/// de `w` × `h` pixels à `dp` pixels par dp : (événement, décalage en secondes, pour
/// qu'un clic dure).
pub fn evenements(w: f32, h: f32, dp: f32) -> Vec<(Event, f64)> {
    if let Ok(mut t) = TAILLE.lock() {
        *t = (w, h);
    }
    let Some(hote) = hote() else { return Vec::new() };
    let Ok(mut s) = hote.lock() else { return Vec::new() };
    let mut v = Vec::new();
    while let Some(m) = s.doigts.pop_front() {
        let c = s.curseur.unwrap_or((w / 2.0, h / 2.0));
        match m {
            Message::Pose(i, x, y) => v.push((Event::Down { id: ID0 + 1 + i, x: x * w, y: y * h }, 0.0)),
            Message::Glisse(i, x, y) => v.push((Event::Move { id: ID0 + 1 + i, x: x * w, y: y * h }, 0.0)),
            Message::Leve(i) => v.push((Event::Up { id: ID0 + 1 + i }, 0.0)),
            Message::Relatif(dx, dy) => {
                let n = ((c.0 + dx * dp).clamp(0.0, w - 1.0), (c.1 + dy * dp).clamp(0.0, h - 1.0));
                s.curseur = Some(n);
                s.curseur_vu = Some(Instant::now());
                if s.tenu {
                    v.push((Event::Move { id: ID0, x: n.0, y: n.1 }, 0.0));
                }
            }
            Message::Defile(dx, dy) => {
                s.curseur_vu = Some(Instant::now());
                let (x, y) = match s.defile {
                    Some(p) => p,
                    None => {
                        v.push((Event::Down { id: ID_DEFILE, x: c.0, y: c.1 }, 0.0));
                        c
                    }
                };
                // Le doigt virtuel peut sortir de l'écran : le glissé continue
                let n = (x + dx * dp, y + dy * dp);
                s.defile = Some(n);
                v.push((Event::Move { id: ID_DEFILE, x: n.0, y: n.1 }, 0.0));
            }
            Message::FinDefile => {
                if s.defile.take().is_some() {
                    v.push((Event::Up { id: ID_DEFILE }, 0.0));
                }
            }
            Message::Clic => {
                s.curseur_vu = Some(Instant::now());
                v.push((Event::Down { id: ID0, x: c.0, y: c.1 }, 0.0));
                v.push((Event::Up { id: ID0 }, 0.04));
            }
            Message::Bouton(b) => {
                s.curseur_vu = Some(Instant::now());
                if b != s.tenu {
                    s.tenu = b;
                    v.push((if b { Event::Down { id: ID0, x: c.0, y: c.1 } } else { Event::Up { id: ID0 } }, 0.0));
                }
            }
            _ => {}
        }
    }
    v
}

/// Les frappes de la télécommande arrivées depuis la dernière image.
pub fn frappes() -> Vec<Frappe> {
    hote().and_then(|h| h.lock().ok().map(|mut h| h.touches.drain(..).collect())).unwrap_or_default()
}

/// La télécommande liée sert-elle de clavier ? (Celui de l'écran se cache alors.)
pub fn clavier_distant() -> bool {
    hote().and_then(|h| h.lock().ok().map(|h| h.lie.is_some() && h.clavier)).unwrap_or(false)
}

/// Le curseur du pavé à dessiner : (x, y, bouton tenu), s'il a servi depuis peu.
pub fn curseur() -> Option<(f32, f32, bool)> {
    let h = hote()?;
    let h = h.lock().ok()?;
    let recent = h.curseur_vu.is_some_and(|t| t.elapsed() < Duration::from_secs(4)) || h.tenu;
    let (x, y) = h.curseur?;
    (h.lie.is_some() && recent).then_some((x, y, h.tenu))
}

/// La manette de la télécommande liée, si elle a parlé il y a moins de
/// `MANETTE_MUETTE` s.
pub fn manette() -> Option<Manette> {
    let h = hote()?;
    let h = h.lock().ok()?;
    let (m, t) = h.manette?;
    (h.lie.is_some() && t.elapsed().as_secs_f32() < MANETTE_MUETTE).then_some(m)
}

/// Le curseur est-il encore à montrer (pour redessiner quand il doit disparaître),
/// une manette tenue ?
pub fn anime() -> bool {
    curseur().is_some() || manette().is_some_and(|m| !m.au_repos())
}

// ----- Côté télécommande -----------------------------------------------------------

/// Les aller-retours mesurés, en millisecondes.
#[derive(Clone, Debug, Default)]
pub struct Latence {
    pub dernier: Option<f32>,
    /// Les derniers (au plus 50), pour la moyenne et le pire.
    pub recents: VecDeque<f32>,
    pub envoyes: u32,
    pub perdus: u32,
}

impl Latence {
    pub fn moyenne(&self) -> Option<f32> {
        (!self.recents.is_empty()).then(|| self.recents.iter().sum::<f32>() / self.recents.len() as f32)
    }

    pub fn pire(&self) -> Option<f32> {
        self.recents.iter().cloned().reduce(f32::max)
    }

    /// La médiane : ce qu'on sent le plus souvent.
    pub fn mediane(&self) -> Option<f32> {
        let mut v: Vec<f32> = self.recents.iter().cloned().collect();
        v.sort_by(f32::total_cmp);
        v.get(v.len() / 2).copied()
    }
}

/// La télécommande : sa socket, les Flow trouvés, la liaison et la latence.
pub struct Client {
    socket: UdpSocket,
    pub trouves: Vec<(SocketAddr, String)>,
    pub hote: Option<SocketAddr>,
    /// Lié (le code a été accepté) ; refusé (mauvais code).
    pub lie: bool,
    pub refuse: bool,
    pub latence: Latence,
    numero: u32,
    attente: VecDeque<(u32, Instant)>,
    dernier_ping: Option<Instant>,
    origine: Instant,
    pub derniere_reponse: Option<Instant>,
    /// La recherche : combien d'envois déjà, quand le dernier ; l'adresse de cet
    /// appareil ; la dernière erreur d'envoi.
    relances: u8,
    relance: Option<Instant>,
    pub local: Option<(Ipv4Addr, u8)>,
    pub erreur: Option<String>,
    /// La taille de l'écran piloté, en pixels ; son nom (« Flow », « WhiteRoom »…).
    pub taille: Option<(f32, f32)>,
    pub nom_hote: Option<String>,
}

/// Les envois d'une recherche (toutes les 0,3 s) ; au troisième, chaque adresse du
/// réseau est interrogée.
const RELANCES: u8 = 6;

impl Client {
    pub fn new() -> Result<Client, String> {
        let erreur = crate::acces::verrous_wifi(true).filter(|e| !e.is_empty()).map(|e| format!("Wi-Fi : {e}"));
        let socket = UdpSocket::bind("0.0.0.0:0").map_err(|e| e.to_string())?;
        socket.set_nonblocking(true).map_err(|e| e.to_string())?;
        let _ = socket.set_broadcast(true);
        Ok(Client { socket, trouves: Vec::new(), hote: None, lie: false, refuse: false, latence: Latence::default(), numero: 0, attente: VecDeque::new(), dernier_ping: None, origine: Instant::now(), derniere_reponse: None, relances: RELANCES, relance: None, local: reseau_local(), erreur, taille: None, nom_hote: None })
    }

    /// Diffuse « qui accepte d'être piloté ? » (à tout le réseau, et au /24 local).
    /// La recherche se répète `RELANCES` fois : une diffusion se perd facilement.
    pub fn chercher(&mut self) {
        self.trouves.clear();
        self.erreur = None;
        self.local = reseau_local();
        self.relances = 0;
        self.relancer();
    }

    /// La recherche est-elle en cours ?
    pub fn cherche(&self) -> bool {
        self.relances < RELANCES || self.relance.is_some_and(|t| t.elapsed().as_secs_f32() < 0.8)
    }

    fn relancer(&mut self) {
        let m = Message::Qui.texte();
        for a in adresses_a_sonder(self.local, self.relances == 2) {
            if let Err(e) = self.socket.send_to(m.as_bytes(), (a, PORT)) {
                self.erreur = Some(format!("envoi vers {a} : {e}"));
            }
        }
        self.relances += 1;
        self.relance = Some(Instant::now());
    }

    /// Se lie à `ip` avec `code`.
    pub fn lier(&mut self, ip: Ipv4Addr, code: u32) {
        let a = SocketAddr::V4(SocketAddrV4::new(ip, PORT));
        self.hote = Some(a);
        self.lie = false;
        self.refuse = false;
        self.taille = None;
        self.nom_hote = None;
        self.latence = Latence::default();
        self.attente.clear();
        let _ = self.socket.send_to(Message::Lier(code).texte().as_bytes(), a);
    }

    pub fn delier(&mut self) {
        self.envoyer(&Message::Fin);
        self.lie = false;
        self.hote = None;
    }

    /// Envoie à l'appareil piloté (s'il y en a un).
    pub fn envoyer(&self, m: &Message) {
        if let Some(a) = self.hote {
            let _ = self.socket.send_to(m.texte().as_bytes(), a);
        }
    }

    /// À chaque image : lit ce qui est arrivé, envoie le ping s'il est temps. Vrai si
    /// quelque chose a changé.
    pub fn avancer(&mut self) -> bool {
        let mut change = false;
        let mut b = [0u8; 512];
        while let Ok((n, de)) = self.socket.recv_from(&mut b) {
            let Some(m) = std::str::from_utf8(&b[..n]).ok().and_then(Message::lire) else { continue };
            change = true;
            match m {
                Message::Moi(nom) if Some(de) == self.hote => self.nom_hote = Some(nom),
                Message::Moi(nom) => {
                    if !self.trouves.iter().any(|(a, _)| *a == de) {
                        self.trouves.push((de, nom));
                    }
                }
                Message::Lie(ok) if Some(de) == self.hote => {
                    self.lie = ok;
                    self.refuse = !ok;
                    // Son nom dit ce qu'il sait faire
                    if ok {
                        self.envoyer(&Message::Qui);
                    }
                }
                Message::Taille(w, h) if Some(de) == self.hote => self.taille = Some((w, h)),
                Message::Ping(k, _) if Some(de) == self.hote => {
                    self.derniere_reponse = Some(Instant::now());
                    if let Some(i) = self.attente.iter().position(|(n, _)| *n == k) {
                        let ms = self.attente[i].1.elapsed().as_secs_f32() * 1000.0;
                        self.attente.remove(i);
                        let l = &mut self.latence;
                        l.dernier = Some(ms);
                        l.recents.push_back(ms);
                        if l.recents.len() > 50 {
                            l.recents.pop_front();
                        }
                    }
                }
                _ => {}
            }
        }
        if self.relances < RELANCES && self.relance.is_none_or(|t| t.elapsed().as_secs_f32() >= 0.3) {
            self.relancer();
            change = true;
        }
        // Les pings sans réponse depuis 1 s sont perdus
        while self.attente.front().is_some_and(|(_, t)| t.elapsed() > Duration::from_secs(1)) {
            self.attente.pop_front();
            self.latence.perdus += 1;
            change = true;
        }
        if self.lie && self.dernier_ping.is_none_or(|t| t.elapsed().as_secs_f32() >= PING) {
            self.numero += 1;
            self.dernier_ping = Some(Instant::now());
            self.attente.push_back((self.numero, Instant::now()));
            self.latence.envoyes += 1;
            self.envoyer(&Message::Ping(self.numero, self.origine.elapsed().as_millis() as u64));
        }
        change
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        if self.lie {
            self.envoyer(&Message::Fin);
        }
        // Les verrous restent pris si Flow est lui-même piloté
        if ECOUTE.lock().map_or(true, |e| e.is_none()) {
            let _ = crate::acces::verrous_wifi(false);
        }
    }
}

/// Les doigts suivis d'une image à l'autre, pour en tirer posés, glissés et levés.
#[derive(Default)]
pub struct Suivi {
    avant: Vec<(i32, f32, f32)>,
}

impl Suivi {
    /// Les messages qui mènent des doigts d'avant à ceux de maintenant (`(id, x, y)`
    /// de 0 à 1).
    pub fn messages(&mut self, maintenant: &[(i32, f32, f32)]) -> Vec<Message> {
        let mut v = Vec::new();
        for &(i, _, _) in &self.avant {
            if !maintenant.iter().any(|d| d.0 == i) {
                v.push(Message::Leve(i));
            }
        }
        for &(i, x, y) in maintenant {
            match self.avant.iter().find(|d| d.0 == i) {
                None => v.push(Message::Pose(i, x, y)),
                Some(&(_, ax, ay)) if (ax - x).abs() > 1e-5 || (ay - y).abs() > 1e-5 => v.push(Message::Glisse(i, x, y)),
                _ => {}
            }
        }
        self.avant = maintenant.to_vec();
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn les_messages_se_relisent() {
        for m in [
            Message::Qui,
            Message::Moi("Flow".into()),
            Message::Lier(42),
            Message::Lie(true),
            Message::Lie(false),
            Message::Ping(7, 123456),
            Message::Pose(1, 0.25, 0.5),
            Message::Glisse(1, 0.75, 1.0),
            Message::Leve(1),
            Message::Relatif(-0.01, 0.02),
            Message::Defile(3.5, -12.0),
            Message::FinDefile,
            Message::Taille(1920.0, 1200.0),
            Message::Manette(Manette { gauche: [0.5, -1.0], droit: [0.0, 0.25], lt: 0.0, rt: 1.0, boutons: bouton::A | bouton::DROITE }),
            Message::Clic,
            Message::Bouton(true),
            Message::Touche(Frappe::Car('é')),
            Message::Touche(Frappe::Car(' ')),
            Message::Touche(Frappe::Effacer),
            Message::Touche(Frappe::Entree),
            Message::Clavier(true),
            Message::Fin,
        ] {
            assert_eq!(Message::lire(&m.texte()), Some(m.clone()), "{}", m.texte());
        }
        assert_eq!(Message::lire("n'importe quoi"), None);
        assert_eq!(Message::lire("D 1 NaN 0"), None);
    }

    #[test]
    fn les_adresses_a_sonder_couvrent_le_reseau() {
        let v = adresses_a_sonder(Some((Ipv4Addr::new(192, 168, 1, 20), 24)), false);
        assert_eq!(v, vec![Ipv4Addr::BROADCAST, Ipv4Addr::new(192, 168, 1, 255)]);
        let v = adresses_a_sonder(Some((Ipv4Addr::new(192, 168, 1, 20), 24)), true);
        assert_eq!(v.len(), 2 + 253);
        assert!(v.contains(&Ipv4Addr::new(192, 168, 1, 1)) && v.contains(&Ipv4Addr::new(192, 168, 1, 254)));
        assert!(!v.contains(&Ipv4Addr::new(192, 168, 1, 20)));
        // Un grand réseau n'est sondé que sur 1022 adresses autour de soi
        let v = adresses_a_sonder(Some((Ipv4Addr::new(10, 1, 2, 3), 8)), true);
        assert_eq!(v[1], Ipv4Addr::new(10, 1, 3, 255));
        assert_eq!(v.len(), 2 + 1021);
    }

    #[test]
    fn le_suivi_dit_pose_glisse_leve() {
        let mut s = Suivi::default();
        assert_eq!(s.messages(&[(3, 0.1, 0.2)]), vec![Message::Pose(3, 0.1, 0.2)]);
        assert_eq!(s.messages(&[(3, 0.1, 0.2)]), vec![]);
        assert_eq!(s.messages(&[(3, 0.3, 0.2), (4, 0.5, 0.5)]), vec![Message::Glisse(3, 0.3, 0.2), Message::Pose(4, 0.5, 0.5)]);
        assert_eq!(s.messages(&[(4, 0.5, 0.5)]), vec![Message::Leve(3)]);
    }

    #[test]
    fn une_telecommande_se_lie_et_pilote_en_local() {
        // Les deux bouts sur cet appareil : le vrai réseau, en boucle locale
        // Flow lancé sur l'appareil qui teste peut déjà tenir le port
        if let Err(e) = ecouter(true) {
            eprintln!("essai sauté : {e}");
            return;
        }
        let (code, ..) = etat_hote().unwrap();
        let mut c = Client::new().unwrap();
        c.lier(Ipv4Addr::LOCALHOST, code + 1);
        let attendre = |c: &mut Client, f: &dyn Fn(&Client) -> bool| {
            for _ in 0..200 {
                c.avancer();
                if f(c) {
                    return true;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            false
        };
        assert!(attendre(&mut c, &|c| c.refuse), "un mauvais code est refusé");
        c.lier(Ipv4Addr::LOCALHOST, code);
        assert!(attendre(&mut c, &|c| c.lie), "le bon code lie");
        // Un ping revient, et sa latence se mesure
        assert!(attendre(&mut c, &|c| c.latence.dernier.is_some()));
        assert!(c.latence.dernier.unwrap() < 200.0);
        // Un doigt, un clic au curseur, une frappe
        c.envoyer(&Message::Pose(0, 0.5, 0.25));
        c.envoyer(&Message::Leve(0));
        c.envoyer(&Message::Relatif(50.0, 0.0));
        c.envoyer(&Message::Clic);
        c.envoyer(&Message::Touche(Frappe::Car('a')));
        c.envoyer(&Message::Clavier(true));
        let m = Manette { gauche: [0.0, -1.0], rt: 1.0, boutons: bouton::A, ..Manette::default() };
        c.envoyer(&Message::Manette(m));
        let mut ev = Vec::new();
        let mut fr = Vec::new();
        for _ in 0..200 {
            ev.extend(evenements(1000.0, 800.0, 2.0));
            fr.extend(frappes());
            if ev.len() >= 4 && !fr.is_empty() && clavier_distant() && manette().is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(ev[0].0, Event::Down { id: ID0 + 1, x: 500.0, y: 200.0 });
        assert_eq!(ev[1].0, Event::Up { id: ID0 + 1 });
        // Le curseur part du milieu, un dixième de largeur à droite
        assert_eq!(ev[2].0, Event::Down { id: ID0, x: 600.0, y: 400.0 });
        assert_eq!(ev[3].0, Event::Up { id: ID0 });
        assert!(ev[3].1 > 0.0);
        assert_eq!(fr, vec![Frappe::Car('a')]);
        assert!(clavier_distant());
        assert!(curseur().is_some());
        assert_eq!(manette(), Some(m));
        assert!(anime());
        // Le nom de l'appareil piloté, demandé à la liaison
        assert!(attendre(&mut c, &|c| c.nom_hote.as_deref() == Some("Flow")));
        // La télécommande s'en va : plus liée, plus de clavier distant
        c.delier();
        std::thread::sleep(Duration::from_millis(100));
        assert!(!clavier_distant());
        assert_eq!(manette(), None);
        assert!(etat_hote().unwrap().1.is_none());
        ecouter(false).unwrap();
        assert!(etat_hote().is_none());
    }
}
