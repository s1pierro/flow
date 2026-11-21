//! Le renfort de génération des cartes : un autre appareil du même réseau (le
//! téléphone) calcule des tuiles pour la génération en cours sur celui-ci
//! (`generer_carte`), qui les lui distribue une à une : chacun avance à sa vitesse.
//!
//! - **Le renfort** (« Servir de renfort », dans la liste des Lieux) répond aux
//!   recherches sur UDP (`PORT`) et attend la génération sur TCP (même port). Il reçoit
//!   le lieu (ses octets `sage_reel` et l'ordre des couches, qui fait l'image du sol)
//!   une fois, repéré par une empreinte : il le garde en mémoire et dans
//!   `Flow/renfort/<empreinte>.reel`. Puis, pour chaque tuile demandée, il rend son GLB.
//! - **La génération** cherche un renfort au départ, puis toutes les `RECHERCHE` s'il
//!   n'y en a pas ; un renfort d'une autre version de Flow est refusé (les tuiles ne se
//!   raccorderaient pas). Une tuile perdue (déconnexion) est rendue à la file.
//!
//! Les messages TCP : longueur (u32, petit-boutiste), puis un octet de genre et le
//! contenu.

use crate::carte::{Cle, Couche, Donnees};
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

/// Le port du renfort (UDP pour la recherche, TCP pour le travail).
pub const PORT: u16 = 47_812;
/// Entre deux recherches d'un renfort pendant une génération.
pub const RECHERCHE: Duration = Duration::from_secs(15);
/// Le temps laissé au renfort pour une tuile avant de la reprendre.
const ATTENTE_TUILE: Duration = Duration::from_secs(300);
const DEMANDE: &str = "FLOW-RENFORT?";
const REPONSE: &str = "FLOW-RENFORT ";
/// Le sous-dossier où le renfort garde les lieux reçus.
pub const DOSSIER: &str = "renfort";

/// La version de Flow : la même des deux côtés.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

// ----- L'éveil ----------------------------------------------------------------------

static EVEILS: AtomicUsize = AtomicUsize::new(0);

/// Garde l'appareil éveillé (processeur et Wi-Fi) tant qu'au moins un travail le
/// demande : la génération, le renfort.
pub fn eveil(pris: bool) {
    if pris {
        if EVEILS.fetch_add(1, Ordering::SeqCst) == 0 {
            let _ = crate::acces::eveil(true);
        }
    } else if EVEILS.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1)) == Ok(1) {
        let _ = crate::acces::eveil(false);
    }
}

// ----- Les messages -----------------------------------------------------------------

fn envoyer(s: &mut TcpStream, genre: u8, contenu: &[u8]) -> std::io::Result<()> {
    let mut o = Vec::with_capacity(5 + contenu.len());
    o.extend_from_slice(&(contenu.len() as u32 + 1).to_le_bytes());
    o.push(genre);
    o.extend_from_slice(contenu);
    s.write_all(&o)
}

fn recevoir(s: &mut TcpStream) -> std::io::Result<(u8, Vec<u8>)> {
    let mut l = [0u8; 4];
    s.read_exact(&mut l)?;
    let n = u32::from_le_bytes(l) as usize;
    if n == 0 || n > 1 << 30 {
        return Err(std::io::Error::other("message illisible"));
    }
    let mut o = vec![0u8; n];
    s.read_exact(&mut o)?;
    let genre = o.remove(0);
    Ok((genre, o))
}

/// L'empreinte (FNV-1a, 64 bits) d'octets.
fn empreinte(parts: &[&[u8]]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for p in parts {
        for &b in p.iter() {
            h ^= b as u64;
            h = h.wrapping_mul(0x100_0000_01b3);
        }
    }
    h
}

fn cle_octets(c: Cle) -> Vec<u8> {
    let mut o = vec![c.0];
    o.extend_from_slice(&c.1.to_le_bytes());
    o.extend_from_slice(&c.2.to_le_bytes());
    o
}

fn cle_de(o: &[u8]) -> Option<Cle> {
    Some((*o.first()?, i32::from_le_bytes(o.get(1..5)?.try_into().ok()?), i32::from_le_bytes(o.get(5..9)?.try_into().ok()?)))
}

// ----- Le renfort -------------------------------------------------------------------

/// Où en est le renfort de cet appareil.
#[derive(Clone, Debug, Default)]
pub struct EtatRenfort {
    pub actif: bool,
    /// La génération servie, et depuis quand.
    pub servie: Option<SocketAddr>,
    pub lieu: Option<String>,
    pub faites: usize,
    pub message: Option<String>,
    /// La charge de cet appareil (fils, processeur, mémoire, température).
    pub charge: Option<crate::charge::Mesure>,
}

/// Le lieu servi : son empreinte, ses données, sa mémoire de génération.
type LieuServi = (u64, Arc<RwLock<Donnees>>, Arc<crate::materialisation::Memoire>);

struct Serveur {
    etat: Arc<Mutex<EtatRenfort>>,
    arret: Arc<AtomicBool>,
}

static SERVEUR: Mutex<Option<Serveur>> = Mutex::new(None);

/// Ce que fait le renfort de cet appareil (rien s'il n'est pas actif).
pub fn etat_renfort() -> Option<EtatRenfort> {
    SERVEUR.lock().ok()?.as_ref().and_then(|s| s.etat.lock().ok().map(|e| e.clone()))
}

/// Sert de renfort (ou plus) ; `flow` : le dossier de Flow (pour garder les lieux reçus).
pub fn servir(oui: bool, flow: Option<std::path::PathBuf>) -> Result<(), String> {
    let mut s = SERVEUR.lock().map_err(|_| "renfort inaccessible")?;
    if !oui {
        if let Some(v) = s.take() {
            v.arret.store(true, Ordering::Relaxed);
            eveil(false);
        }
        return Ok(());
    }
    if s.is_some() {
        return Ok(());
    }
    let udp = UdpSocket::bind(("0.0.0.0", PORT)).map_err(|e| format!("port UDP {PORT} : {e}"))?;
    let tcp = TcpListener::bind(("0.0.0.0", PORT)).map_err(|e| format!("port TCP {PORT} : {e}"))?;
    let _ = udp.set_read_timeout(Some(Duration::from_millis(500)));
    let _ = tcp.set_nonblocking(true);
    let etat = Arc::new(Mutex::new(EtatRenfort { actif: true, ..Default::default() }));
    let arret = Arc::new(AtomicBool::new(false));
    // La recherche : répondre avec la version
    {
        let arret = arret.clone();
        let _ = std::thread::Builder::new().name("renfort-udp".into()).spawn(move || {
            let mut b = [0u8; 256];
            while !arret.load(Ordering::Relaxed) {
                if let Ok((n, de)) = udp.recv_from(&mut b) {
                    if &b[..n] == DEMANDE.as_bytes() {
                        let _ = udp.send_to(format!("{REPONSE}{}", version()).as_bytes(), de);
                    }
                }
            }
        });
    }
    // Le travail : une connexion par fil de calcul de la génération servie (chacune
    // fait une tuile à la fois ; le régulateur dit combien calculent ensemble)
    {
        let (etat, arret) = (etat.clone(), arret.clone());
        let _ = std::thread::Builder::new().name("renfort".into()).spawn(move || {
            let lieu: Arc<Mutex<Option<LieuServi>>> = Arc::new(Mutex::new(None));
            let regul = Arc::new(crate::charge::Regulateur::new());
            let places: Arc<Mutex<Vec<bool>>> = Arc::new(Mutex::new(Vec::new()));
            while !arret.load(Ordering::Relaxed) {
                maj(&etat, |e| e.charge = Some(regul.mesure()));
                match tcp.accept() {
                    Ok((s, de)) => {
                        let _ = s.set_nonblocking(false);
                        maj(&etat, |e| {
                            e.servie = Some(de);
                            e.message = None;
                        });
                        // La place de cette connexion : la première libre
                        let place = places.lock().map_or(0, |mut p| match p.iter().position(|&x| !x) {
                            Some(i) => {
                                p[i] = true;
                                i
                            }
                            None => {
                                p.push(true);
                                p.len() - 1
                            }
                        });
                        let (etat, arret, lieu, regul, places, flow) = (etat.clone(), arret.clone(), lieu.clone(), regul.clone(), places.clone(), flow.clone());
                        let _ = std::thread::Builder::new().name("renfort-fil".into()).spawn(move || {
                            let r = servir_generation(s, &etat, &arret, &lieu, &regul, place, flow.as_deref());
                            let reste = places.lock().map_or(0, |mut p| {
                                if let Some(x) = p.get_mut(place) {
                                    *x = false;
                                }
                                p.iter().filter(|&&x| x).count()
                            });
                            maj(&etat, |e| {
                                if reste == 0 {
                                    e.servie = None;
                                }
                                if let Err(m) = r {
                                    e.message = Some(m);
                                }
                            });
                        });
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(300)),
                }
            }
        });
    }
    eveil(true);
    *s = Some(Serveur { etat, arret });
    Ok(())
}

fn maj(e: &Mutex<EtatRenfort>, f: impl FnOnce(&mut EtatRenfort)) {
    if let Ok(mut e) = e.lock() {
        f(&mut e);
    }
}

/// Sert une génération jusqu'à ce qu'elle se termine ou se coupe.
fn servir_generation(mut s: TcpStream, etat: &Mutex<EtatRenfort>, arret: &AtomicBool, lieu: &Mutex<Option<LieuServi>>, regul: &crate::charge::Regulateur, place: usize, flow: Option<&std::path::Path>) -> Result<(), String> {
    let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
    let err = |e: std::io::Error| e.to_string();
    // Bonjour : la même version, sinon refus
    let (g, v) = recevoir(&mut s).map_err(err)?;
    let v = String::from_utf8_lossy(&v).into_owned();
    if g != b'B' {
        return Err("message inattendu".into());
    }
    if v != version() {
        let _ = envoyer(&mut s, b'R', version().as_bytes());
        return Err(format!("génération refusée : Flow {v} de l'autre côté, {} ici", version()));
    }
    // D'accord : la version, puis le nombre de fils de calcul (autant de connexions)
    envoyer(&mut s, b'O', format!("{}\n{}", version(), crate::charge::fils_max()).as_bytes()).map_err(err)?;
    let _ = s.set_read_timeout(None);
    loop {
        if arret.load(Ordering::Relaxed) {
            return Ok(());
        }
        let (g, o) = match recevoir(&mut s) {
            Ok(m) => m,
            // La génération est finie (ou coupée)
            Err(_) => return Ok(()),
        };
        match g {
            // Le lieu par son empreinte et son nom : déjà là, ou à envoyer
            b'L' => {
                let h = u64::from_le_bytes(o.get(..8).ok_or("empreinte")?.try_into().unwrap());
                let nom = String::from_utf8_lossy(&o[8..]).into_owned();
                let fichier = flow.map(|f| f.join(DOSSIER).join(format!("{h:016x}.reel")));
                let mut lieu = lieu.lock().map_err(|_| "lieu inaccessible")?;
                let deja = lieu.as_ref().is_some_and(|l| l.0 == h);
                let sur_disque = !deja && fichier.as_ref().is_some_and(|f| f.exists());
                if deja || sur_disque {
                    if sur_disque {
                        maj(etat, |e| e.message = Some(format!("lecture de {nom}…")));
                        let o = std::fs::read(fichier.as_ref().unwrap()).map_err(|e| e.to_string())?;
                        poser_lieu(&mut lieu, h, &o).ok_or("lieu reçu illisible")?;
                    }
                    maj(etat, |e| {
                        e.lieu = Some(nom);
                        e.message = None;
                    });
                    envoyer(&mut s, b'A', &[]).map_err(err)?;
                } else {
                    maj(etat, |e| e.message = Some(format!("réception de {nom}…")));
                    envoyer(&mut s, b'D', &[]).map_err(err)?;
                }
            }
            // Le lieu lui-même : l'ordre des couches puis les octets du lieu
            b'C' => {
                let h = u64::from_le_bytes(o.get(..8).ok_or("empreinte")?.try_into().unwrap());
                if let Some(f) = flow {
                    let d = f.join(DOSSIER);
                    let _ = std::fs::create_dir_all(&d);
                    // Un seul lieu gardé : les autres sont effacés
                    if let Ok(l) = std::fs::read_dir(&d) {
                        for e in l.flatten() {
                            let _ = std::fs::remove_file(e.path());
                        }
                    }
                    let _ = std::fs::write(d.join(format!("{h:016x}.reel")), &o[8..]);
                }
                poser_lieu(&mut *lieu.lock().map_err(|_| "lieu inaccessible")?, h, &o[8..]).ok_or("lieu reçu illisible")?;
                maj(etat, |e| e.message = None);
                envoyer(&mut s, b'A', &[]).map_err(err)?;
            }
            // Une tuile : la faire et la rendre
            b'T' => {
                let cle = cle_de(&o).ok_or("tuile illisible")?;
                let (_, d, memoire) = lieu.lock().map_err(|_| "lieu inaccessible")?.clone().ok_or("tuile demandée avant le lieu")?;
                // Attendre son tour (le régulateur ménage la mémoire et la chaleur)
                while !regul.permis(place) {
                    if arret.load(Ordering::Relaxed) {
                        return Ok(());
                    }
                    std::thread::sleep(Duration::from_millis(300));
                }
                let t = {
                    let _g = regul.commencer();
                    let d = d.read().map_err(|_| "données inaccessibles")?;
                    crate::generer_carte::tuile(&d, cle, &memoire)
                };
                // La tuile, précédée de la charge de cet appareil
                let mesure = regul.mesure();
                let mut r = cle_octets(cle);
                r.extend_from_slice(&mesure.octets());
                if let Some(t) = t {
                    let en = crate::generer_carte::entree(&t, [0.0; 4]);
                    r.extend_from_slice(&en.hmin.to_le_bytes());
                    r.extend_from_slice(&en.hmax.to_le_bytes());
                    r.extend_from_slice(&en.triangles.to_le_bytes());
                    r.extend_from_slice(&sage_carte::glb::ecrire(&t));
                }
                envoyer(&mut s, b'F', &r).map_err(err)?;
                maj(etat, |e| {
                    e.faites += 1;
                    e.charge = Some(mesure);
                });
            }
            _ => return Err("message inattendu".into()),
        }
    }
}

/// Le lieu reçu (ordre des couches puis octets du lieu), prêt à générer.
fn poser_lieu(lieu: &mut Option<LieuServi>, h: u64, o: &[u8]) -> Option<()> {
    let n = *o.first()? as usize;
    let ordre: Vec<Couche> = o.get(1..1 + n)?.to_vec();
    let l = sage_reel::Lieu::depuis_octets(o.get(1 + n..)?)?;
    let mut d = Donnees::new(l);
    d.ordre = ordre;
    let memoire = crate::materialisation::Memoire::pour(&d, &[]);
    *lieu = Some((h, Arc::new(RwLock::new(d)), Arc::new(memoire)));
    Some(())
}

// ----- Côté génération --------------------------------------------------------------

/// Cherche un renfort sur le réseau : son adresse et sa version.
pub fn chercher() -> Option<(SocketAddr, String)> {
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    let _ = s.set_broadcast(true);
    let _ = s.set_read_timeout(Some(Duration::from_millis(200)));
    let local = crate::telecommande::reseau_local();
    let mut moi: Vec<Ipv4Addr> = local.map(|l| vec![l.0]).unwrap_or_default();
    moi.push(Ipv4Addr::LOCALHOST);
    let attendre = |s: &UdpSocket, jusqua: Instant| -> Option<(SocketAddr, String)> {
        let mut b = [0u8; 256];
        while Instant::now() < jusqua {
            if let Ok((n, de)) = s.recv_from(&mut b) {
                let t = String::from_utf8_lossy(&b[..n]).into_owned();
                let mien = matches!(de, SocketAddr::V4(a) if moi.contains(a.ip()));
                if let (Some(v), false) = (t.strip_prefix(REPONSE), mien) {
                    return Some((SocketAddr::new(de.ip(), PORT), v.to_string()));
                }
            }
        }
        None
    };
    // Les diffusions, puis chaque adresse du réseau une à une
    for une_a_une in [false, true] {
        for a in crate::telecommande::adresses_a_sonder(local, une_a_une) {
            let _ = s.send_to(DEMANDE.as_bytes(), (a, PORT));
        }
        if let Some(r) = attendre(&s, Instant::now() + Duration::from_millis(if une_a_une { 1500 } else { 800 })) {
            return Some(r);
        }
    }
    None
}

/// Une génération servie par un renfort : connexion, version, lieu.
pub struct Lien {
    s: TcpStream,
    pub adresse: SocketAddr,
    /// Les fils de calcul du renfort (autant de liens à ouvrir).
    pub fils: usize,
}

impl Lien {
    /// Se lie au renfort `a` et lui donne le lieu de `d` (s'il ne l'a pas déjà) ;
    /// `etat` dit où on en est.
    pub fn ouvrir(a: SocketAddr, d: &Donnees, nom: &str, etat: impl Fn(&str)) -> Result<Lien, String> {
        let err = |e: std::io::Error| e.to_string();
        let mut s = TcpStream::connect_timeout(&a, Duration::from_secs(5)).map_err(err)?;
        let _ = s.set_nodelay(true);
        let _ = s.set_read_timeout(Some(Duration::from_secs(10)));
        envoyer(&mut s, b'B', version().as_bytes()).map_err(err)?;
        let (g, v) = recevoir(&mut s).map_err(err)?;
        if g != b'O' {
            return Err(format!("renfort en Flow {}, il faut la {} des deux côtés", String::from_utf8_lossy(&v), version()));
        }
        let fils = String::from_utf8_lossy(&v).split('\n').nth(1).and_then(|n| n.trim().parse::<usize>().ok()).unwrap_or(1).clamp(1, 32);
        // Le lieu : son empreinte d'abord
        let octets = d.lieu.octets();
        let mut contenu = vec![d.ordre.len() as u8];
        contenu.extend_from_slice(&d.ordre);
        let h = empreinte(&[&contenu, &octets]);
        let mut m = h.to_le_bytes().to_vec();
        m.extend_from_slice(nom.as_bytes());
        envoyer(&mut s, b'L', &m).map_err(err)?;
        let _ = s.set_read_timeout(Some(Duration::from_secs(120)));
        let (g, _) = recevoir(&mut s).map_err(err)?;
        if g == b'D' {
            etat(&format!("envoi du lieu au renfort ({:.0} Mo)…", octets.len() as f64 / 1e6));
            let mut m = h.to_le_bytes().to_vec();
            m.extend_from_slice(&contenu);
            m.extend_from_slice(&octets);
            drop(octets);
            envoyer(&mut s, b'C', &m).map_err(err)?;
            let (g, _) = recevoir(&mut s).map_err(err)?;
            if g != b'A' {
                return Err("le renfort n'a pas pu lire le lieu".into());
            }
        } else if g != b'A' {
            return Err("réponse inattendue du renfort".into());
        }
        let _ = s.set_read_timeout(Some(ATTENTE_TUILE));
        Ok(Lien { s, adresse: a, fils })
    }

    /// Fait faire la tuile `cle` : la charge du renfort, et rien si la tuile est vide,
    /// sinon (hmin, hmax, triangles, GLB).
    #[allow(clippy::type_complexity)]
    pub fn tuile(&mut self, cle: Cle) -> Result<(Option<crate::charge::Mesure>, Option<(f32, f32, u32, Vec<u8>)>), String> {
        let err = |e: std::io::Error| e.to_string();
        envoyer(&mut self.s, b'T', &cle_octets(cle)).map_err(err)?;
        let (g, o) = recevoir(&mut self.s).map_err(err)?;
        if g != b'F' || cle_de(&o) != Some(cle) {
            return Err("réponse inattendue du renfort".into());
        }
        let mesure = o.get(9..).and_then(crate::charge::Mesure::lire);
        let k = 9 + crate::charge::Mesure::TAILLE;
        if o.len() <= k {
            return Ok((mesure, None));
        }
        let f = |i: usize| o.get(k + i..k + i + 4).map(|b| [b[0], b[1], b[2], b[3]]).ok_or("tuile tronquée");
        Ok((mesure, Some((f32::from_le_bytes(f(0)?), f32::from_le_bytes(f(4)?), u32::from_le_bytes(f(8)?), o[k + 12..].to_vec()))))
    }
}
