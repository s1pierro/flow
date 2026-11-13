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

pub const PORT: u16 = 47_811;
/// Les identifiants des doigts distants (loin de ceux de l'écran).
pub const ID0: i32 = 1000;
/// L'intervalle des pings, en secondes.
pub const PING: f32 = 0.2;
/// Le gain du pavé : un glissement sur toute sa largeur traverse l'écran piloté `GAIN`
/// fois.
const GAIN: f32 = 1.0;

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
    /// Le pavé : déplacer le curseur (en largeurs du pavé), cliquer, tenir ou lâcher.
    Relatif(f32, f32),
    Clic,
    Bouton(bool),
    /// Une frappe du clavier.
    Touche(Frappe),
    /// La télécommande sert de clavier (celui de l'écran piloté se cache), ou plus.
    Clavier(bool),
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
            "Q" => Message::Fin,
            _ => return None,
        })
    }
}

/// L'adresse de cet appareil sur le réseau local (celle qui sert à sortir), s'il en a.
pub fn adresse_locale() -> Option<Ipv4Addr> {
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
}

struct Ecoute {
    hote: Arc<Mutex<Hote>>,
    arret: Arc<AtomicBool>,
}

static ECOUTE: Mutex<Option<Ecoute>> = Mutex::new(None);
static TAILLE: Mutex<(f32, f32)> = Mutex::new((1.0, 1.0));

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
    let socket = UdpSocket::bind(("0.0.0.0", PORT)).map_err(|e| format!("port {PORT} : {e}"))?;
    socket.set_read_timeout(Some(Duration::from_millis(250))).map_err(|e| e.to_string())?;
    let _ = socket.set_broadcast(true);
    let code = (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos()) / 1000) % 9000 + 1000;
    let hote = Arc::new(Mutex::new(Hote { code, ..Default::default() }));
    let arret = Arc::new(AtomicBool::new(false));
    let (h, a) = (hote.clone(), arret.clone());
    std::thread::spawn(move || boucle_hote(socket, h, a));
    let _ = crate::acces::verrous_wifi(true);
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
        let reponse = {
            let Ok(mut h) = h.lock() else { return };
            match m {
                Message::Qui => Some(Message::Moi("Flow".into())),
                // Un ping, même d'une télécommande pas encore liée : rien n'en dépend
                Message::Ping(..) => Some(m),
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
        if let Some(r) = reponse {
            let _ = socket.send_to(r.texte().as_bytes(), de);
        }
        crate::radio::reveiller();
    }
}

/// Les événements des doigts distants arrivés depuis la dernière image, pour un écran
/// de `w` × `h` pixels : (événement, décalage en secondes, pour qu'un clic dure).
pub fn evenements(w: f32, h: f32) -> Vec<(Event, f64)> {
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
                let n = ((c.0 + dx * w * GAIN).clamp(0.0, w - 1.0), (c.1 + dy * w * GAIN).clamp(0.0, h - 1.0));
                s.curseur = Some(n);
                s.curseur_vu = Some(Instant::now());
                if s.tenu {
                    v.push((Event::Move { id: ID0, x: n.0, y: n.1 }, 0.0));
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

/// Le curseur est-il encore à montrer (pour redessiner quand il doit disparaître) ?
pub fn anime() -> bool {
    curseur().is_some()
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
}

impl Client {
    pub fn new() -> Result<Client, String> {
        let socket = UdpSocket::bind("0.0.0.0:0").map_err(|e| e.to_string())?;
        socket.set_nonblocking(true).map_err(|e| e.to_string())?;
        let _ = socket.set_broadcast(true);
        let _ = crate::acces::verrous_wifi(true);
        Ok(Client { socket, trouves: Vec::new(), hote: None, lie: false, refuse: false, latence: Latence::default(), numero: 0, attente: VecDeque::new(), dernier_ping: None, origine: Instant::now(), derniere_reponse: None })
    }

    /// Diffuse « qui accepte d'être piloté ? » (à tout le réseau, et au /24 local).
    pub fn chercher(&mut self) {
        self.trouves.clear();
        let m = Message::Qui.texte();
        let _ = self.socket.send_to(m.as_bytes(), (Ipv4Addr::BROADCAST, PORT));
        if let Some(a) = adresse_locale() {
            let o = a.octets();
            let _ = self.socket.send_to(m.as_bytes(), (Ipv4Addr::new(o[0], o[1], o[2], 255), PORT));
        }
    }

    /// Se lie à `ip` avec `code`.
    pub fn lier(&mut self, ip: Ipv4Addr, code: u32) {
        let a = SocketAddr::V4(SocketAddrV4::new(ip, PORT));
        self.hote = Some(a);
        self.lie = false;
        self.refuse = false;
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
                Message::Moi(nom) => {
                    if !self.trouves.iter().any(|(a, _)| *a == de) {
                        self.trouves.push((de, nom));
                    }
                }
                Message::Lie(ok) if Some(de) == self.hote => {
                    self.lie = ok;
                    self.refuse = !ok;
                }
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
        ecouter(true).unwrap();
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
        c.envoyer(&Message::Relatif(0.1, 0.0));
        c.envoyer(&Message::Clic);
        c.envoyer(&Message::Touche(Frappe::Car('a')));
        c.envoyer(&Message::Clavier(true));
        let mut ev = Vec::new();
        let mut fr = Vec::new();
        for _ in 0..200 {
            ev.extend(evenements(1000.0, 800.0));
            fr.extend(frappes());
            if ev.len() >= 4 && !fr.is_empty() && clavier_distant() {
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
        // La télécommande s'en va : plus liée, plus de clavier distant
        c.delier();
        std::thread::sleep(Duration::from_millis(100));
        assert!(!clavier_distant());
        assert!(etat_hote().unwrap().1.is_none());
        ecouter(false).unwrap();
        assert!(etat_hote().is_none());
    }
}
