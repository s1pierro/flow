//! Le plan : une carte 2D minimaliste du monde, pour choisir où générer un lieu (le
//! contexte Lieux). Inspirée de la carte de Cadence : les tuiles vectorielles
//! d'OpenFreeMap (schéma OpenMapTiles, format MVT), simplifiées à chaque niveau, dont on
//! ne garde que l'essentiel.
//!
//! - Les surfaces (eau, bois, zones bâties) sont peintes une fois par tuile, dans une
//!   image de 256 pixels (remplissage pair-impair : les îles restent des îles).
//! - Les routes, les rivières et les noms des lieux habités se tracent en vecteurs, à
//!   épaisseur fixe à l'écran.
//!
//! Le niveau des tuiles suit le zoom (jusqu'à 14, au-delà on agrandit) ; en attendant
//! une tuile, on montre son ancêtre le plus proche. Trois fils téléchargent, les tuiles
//! demandées en dernier d'abord, et les gardent dans le dossier du cache
//! (`Flow/lieux/.plan/`) : données d'OpenStreetMap (ODbL), elles ne se publient pas.

use crate::draw::{Color, Image, Rect};
use crate::widgets::Cx;
use sage_reel::generer::Reseau;
use std::collections::{HashMap, HashSet};
use std::f64::consts::PI;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// Le rayon de la sphère de Mercator (m).
const R: f64 = 6378137.0;
/// Le tour du monde en mètres de Mercator.
const MONDE: f64 = 2.0 * PI * R;
/// Le niveau le plus fin des tuiles d'OpenFreeMap.
const NIVEAU_MAX: u8 = 14;
/// Le côté des images des surfaces.
const COTE: usize = 256;
/// Les tuiles gardées en mémoire au plus.
const GARDEES: usize = 120;
/// Le délai avant de redemander une tuile en échec.
const REESSAI: Duration = Duration::from_secs(20);
/// Les fils qui téléchargent.
const FILS: usize = 3;
/// La description des tuiles (leur adresse change à chaque mise à jour).
const TUILES: &str = "https://tiles.openfreemap.org/planet";

// Les genres de traits
const AUTOROUTE: u8 = 0;
const RAPIDE: u8 = 1;
const PRINCIPALE: u8 = 2;
const SECONDAIRE: u8 = 3;
const TERTIAIRE: u8 = 4;
const RUE: u8 = 5;
const RIVIERE: u8 = 6;
// Les lieux habités
const VILLE: u8 = 0;
const BOURG: u8 = 1;
const VILLAGE: u8 = 2;

/// Le fond, sous les surfaces.
const FOND: Color = [0x1E, 0x22, 0x27, 255];

/// (latitude, longitude) en degrés → mètres de Mercator (x vers l'est, y vers le nord).
pub fn mercator(lat: f64, lon: f64) -> [f64; 2] {
    let lat = lat.clamp(-85.0, 85.0);
    [R * lon.to_radians(), R * (PI / 4.0 + lat.to_radians() / 2.0).tan().ln()]
}

/// Mètres de Mercator → (latitude, longitude) en degrés.
pub fn geo(m: [f64; 2]) -> (f64, f64) {
    ((m[1] / R).sinh().atan().to_degrees(), (m[0] / R).to_degrees())
}

/// Une tuile : (niveau, colonne, ligne), la ligne 0 au nord.
pub type Cle = (u8, i32, i32);

/// La boîte d'une tuile en mètres de Mercator : x0, y0 (au sud), x1, y1.
fn boite(cle: Cle) -> [f64; 4] {
    let t = MONDE / (1u64 << cle.0) as f64;
    let x0 = -MONDE / 2.0 + cle.1 as f64 * t;
    let y1 = MONDE / 2.0 - cle.2 as f64 * t;
    [x0, y1 - t, x0 + t, y1]
}

// ----- Le format MVT (protobuf)

/// Un lecteur de protobuf.
struct Pb<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Pb<'a> {
    fn new(b: &'a [u8]) -> Pb<'a> {
        Pb { b, i: 0 }
    }

    fn varint(&mut self) -> Option<u64> {
        let mut v = 0u64;
        for k in 0..10 {
            let o = *self.b.get(self.i)?;
            self.i += 1;
            v |= ((o & 0x7F) as u64) << (7 * k);
            if o < 0x80 {
                return Some(v);
            }
        }
        None
    }

    /// Le champ suivant : (numéro, type).
    fn champ(&mut self) -> Option<(u64, u8)> {
        if self.i >= self.b.len() {
            return None;
        }
        let c = self.varint()?;
        Some((c >> 3, (c & 7) as u8))
    }

    fn octets(&mut self) -> Option<&'a [u8]> {
        let n = self.varint()? as usize;
        let s = self.b.get(self.i..self.i.checked_add(n)?)?;
        self.i += n;
        Some(s)
    }

    fn sauter(&mut self, t: u8) -> Option<()> {
        match t {
            0 => {
                self.varint()?;
            }
            1 => self.i += 8,
            2 => {
                self.octets()?;
            }
            5 => self.i += 4,
            _ => return None,
        }
        (self.i <= self.b.len()).then_some(())
    }

    /// Des entiers serrés (un champ « packed »).
    fn serres(&mut self) -> Option<Vec<u32>> {
        let mut p = Pb::new(self.octets()?);
        let mut v = Vec::new();
        while p.i < p.b.len() {
            v.push(p.varint()? as u32);
        }
        Some(v)
    }
}

/// Une valeur d'étiquette.
#[derive(Clone, Debug, PartialEq)]
enum Valeur {
    Txt(String),
    Num(f64),
}

fn valeur(b: &[u8]) -> Option<Valeur> {
    let mut p = Pb::new(b);
    let mut v = None;
    while let Some((n, t)) = p.champ() {
        v = match (n, t) {
            (1, 2) => Some(Valeur::Txt(String::from_utf8_lossy(p.octets()?).into_owned())),
            (2, 5) => {
                let o = p.b.get(p.i..p.i + 4)?;
                p.i += 4;
                Some(Valeur::Num(f32::from_le_bytes([o[0], o[1], o[2], o[3]]) as f64))
            }
            (3, 1) => {
                let o = p.b.get(p.i..p.i + 8)?;
                p.i += 8;
                Some(Valeur::Num(f64::from_le_bytes(o.try_into().ok()?)))
            }
            (4, 0) | (5, 0) => Some(Valeur::Num(p.varint()? as i64 as f64)),
            (6, 0) => {
                let z = p.varint()?;
                Some(Valeur::Num(((z >> 1) as i64 ^ -((z & 1) as i64)) as f64))
            }
            (7, 0) => Some(Valeur::Num(p.varint()? as f64)),
            _ => {
                p.sauter(t)?;
                v
            }
        };
    }
    v
}

/// Un objet d'une couche : ses étiquettes, son type (1 point, 2 ligne, 3 polygone), sa
/// géométrie en parties (une ligne, un anneau), en coordonnées de la tuile (y vers le
/// bas, de 0 à `etendue`).
struct Objet {
    tags: HashMap<String, Valeur>,
    genre: u8,
    parties: Vec<Vec<[f32; 2]>>,
}

impl Objet {
    fn txt(&self, k: &str) -> Option<&str> {
        match self.tags.get(k)? {
            Valeur::Txt(t) => Some(t),
            _ => None,
        }
    }

    fn num(&self, k: &str) -> Option<f64> {
        match self.tags.get(k)? {
            Valeur::Num(n) => Some(*n),
            _ => None,
        }
    }
}

/// Les commandes de géométrie d'un objet → ses parties.
fn geometrie(g: &[u32], genre: u8) -> Vec<Vec<[f32; 2]>> {
    let zz = |n: u32| ((n >> 1) as i32) ^ -((n & 1) as i32);
    let (mut x, mut y) = (0i32, 0i32);
    let mut parties: Vec<Vec<[f32; 2]>> = Vec::new();
    let mut i = 0;
    while i < g.len() {
        let (cmd, n) = (g[i] & 7, (g[i] >> 3) as usize);
        i += 1;
        match cmd {
            1 | 2 => {
                for _ in 0..n {
                    let (Some(&dx), Some(&dy)) = (g.get(i), g.get(i + 1)) else { break };
                    i += 2;
                    x += zz(dx);
                    y += zz(dy);
                    if cmd == 1 {
                        parties.push(Vec::new());
                    }
                    if let Some(p) = parties.last_mut() {
                        p.push([x as f32, y as f32]);
                    }
                }
            }
            7 => {}
            _ => break,
        }
    }
    // Un point, une ligne de deux points, un anneau de trois
    parties.retain(|p| p.len() >= genre.clamp(1, 3) as usize);
    parties
}

/// Les couches voulues d'une tuile MVT : (nom, étendue, objets).
fn lire_mvt(b: &[u8], voulues: &[&str]) -> Option<Vec<(String, f32, Vec<Objet>)>> {
    let mut couches = Vec::new();
    let mut p = Pb::new(b);
    while let Some((n, t)) = p.champ() {
        if (n, t) != (3, 2) {
            p.sauter(t)?;
            continue;
        }
        let mut c = Pb::new(p.octets()?);
        let (mut nom, mut etendue) = (String::new(), 4096.0f32);
        let (mut cles, mut valeurs, mut bruts) = (Vec::new(), Vec::new(), Vec::new());
        while let Some((n, t)) = c.champ() {
            match (n, t) {
                (1, 2) => nom = String::from_utf8_lossy(c.octets()?).into_owned(),
                (2, 2) => bruts.push(c.octets()?),
                (3, 2) => cles.push(String::from_utf8_lossy(c.octets()?).into_owned()),
                (4, 2) => valeurs.push(valeur(c.octets()?)),
                (5, 0) => etendue = c.varint()? as f32,
                _ => c.sauter(t)?,
            }
        }
        if !voulues.contains(&nom.as_str()) {
            continue;
        }
        let mut objets = Vec::new();
        for o in bruts {
            let mut f = Pb::new(o);
            let (mut tags, mut genre, mut geo) = (Vec::new(), 0u8, Vec::new());
            while let Some((n, t)) = f.champ() {
                match (n, t) {
                    (2, 2) => tags = f.serres()?,
                    (3, 0) => genre = f.varint()? as u8,
                    (4, 2) => geo = f.serres()?,
                    _ => f.sauter(t)?,
                }
            }
            let tags = tags.chunks(2).filter(|k| k.len() == 2).filter_map(|k| Some((cles.get(k[0] as usize)?.clone(), valeurs.get(k[1] as usize)?.clone()?))).collect();
            objets.push(Objet { tags, genre, parties: geometrie(&geo, genre) });
        }
        couches.push((nom, etendue, objets));
    }
    Some(couches)
}

// ----- Une tuile du plan

/// Un trait (route, rivière) : ses points relatifs au coin sud-ouest de la tuile.
struct Trait {
    genre: u8,
    boite: [f32; 4],
    pts: Vec<[f32; 2]>,
}

/// Un lieu habité nommé.
struct Nom {
    genre: u8,
    rang: f32,
    pos: [f32; 2],
    texte: String,
}

/// Ce qu'une tuile montre : l'image de ses surfaces, ses traits et ses noms.
pub struct Tuile {
    cle: Cle,
    origine: [f64; 2],
    image: Option<Image>,
    traits: Vec<Trait>,
    noms: Vec<Nom>,
}

fn boite_de(pts: &[[f32; 2]]) -> [f32; 4] {
    pts.iter().fold([f32::MAX, f32::MAX, f32::MIN, f32::MIN], |b, p| [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])])
}

/// Peint un polygone (ses anneaux, pair-impair) dans l'image, en coordonnées de pixels,
/// avec quatre lignes d'échantillons par pixel et les bouts de segments au prorata.
fn peindre(px: &mut [u8], anneaux: &[Vec<[f32; 2]>], c: [u8; 3], couv: &mut [f32]) {
    let n = COTE as f32;
    let b = anneaux.iter().fold([f32::MAX, f32::MAX, f32::MIN, f32::MIN], |b, a| {
        let k = boite_de(a);
        [b[0].min(k[0]), b[1].min(k[1]), b[2].max(k[2]), b[3].max(k[3])]
    });
    if b[2] < 0.0 || b[3] < 0.0 || b[0] > n || b[1] > n {
        return;
    }
    let (y0, y1) = (b[1].max(0.0).floor() as usize, (b[3].ceil() as usize).min(COTE));
    for v in &mut couv[y0 * COTE..y1 * COTE] {
        *v = 0.0;
    }
    let mut x = Vec::new();
    for row in y0..y1 {
        for s in 0..4 {
            let y = row as f32 + (s as f32 + 0.5) / 4.0;
            x.clear();
            for a in anneaux {
                for k in 0..a.len() {
                    let (p, q) = (a[k], a[(k + 1) % a.len()]);
                    if (p[1] <= y) != (q[1] <= y) {
                        x.push(p[0] + (y - p[1]) / (q[1] - p[1]) * (q[0] - p[0]));
                    }
                }
            }
            x.sort_by(|a, b| a.total_cmp(b));
            for e in x.chunks(2).filter(|e| e.len() == 2) {
                let (xa, xb) = (e[0].clamp(0.0, n), e[1].clamp(0.0, n));
                if xb <= xa {
                    continue;
                }
                let (ia, ib) = (xa.floor() as usize, (xb.ceil() as usize).min(COTE));
                for i in ia..ib {
                    let l = (xb.min(i as f32 + 1.0) - xa.max(i as f32)).max(0.0);
                    couv[row * COTE + i] += 0.25 * l;
                }
            }
        }
    }
    for i in y0 * COTE..y1 * COTE {
        let a = couv[i].min(1.0);
        if a > 0.0 {
            for k in 0..3 {
                let o = &mut px[i * 4 + k];
                *o = (*o as f32 * (1.0 - a) + c[k] as f32 * a) as u8;
            }
            px[i * 4 + 3] = 255;
        }
    }
}

/// Lit une tuile MVT d'OpenFreeMap.
fn lire_tuile(b: &[u8], cle: Cle) -> Option<Tuile> {
    let bt = boite(cle);
    let t = (bt[2] - bt[0]) as f32;
    let couches = lire_mvt(b, &["water", "landcover", "landuse", "waterway", "transportation", "place"])?;
    let mut tu = Tuile { cle, origine: [bt[0], bt[1]], image: None, traits: Vec::new(), noms: Vec::new() };
    // Les surfaces, des zones bâties à l'eau
    let mut px = vec![0u8; COTE * COTE * 4];
    for p in px.chunks_mut(4) {
        p.copy_from_slice(&FOND);
    }
    let mut couv = vec![0f32; COTE * COTE];
    let mut peint = false;
    for (couche, classes, c) in [
        ("landuse", &["residential", "suburb", "neighbourhood", "commercial", "industrial", "retail"][..], [0x2C, 0x2F, 0x36]),
        ("landcover", &["wood", "forest"][..], [0x24, 0x3D, 0x2C]),
        ("water", &[][..], [0x2E, 0x52, 0x80]),
    ] {
        for (nom, etendue, objets) in &couches {
            if nom != couche {
                continue;
            }
            let k = COTE as f32 / etendue;
            for o in objets.iter().filter(|o| o.genre == 3 && (classes.is_empty() || o.txt("class").is_some_and(|c| classes.contains(&c)))) {
                let anneaux: Vec<Vec<[f32; 2]>> = o.parties.iter().map(|a| a.iter().map(|p| [p[0] * k, p[1] * k]).collect()).collect();
                peindre(&mut px, &anneaux, c, &mut couv);
                peint = true;
            }
        }
    }
    if peint {
        let id = 0x706c_616e_0000_0000u64 ^ ((cle.0 as u64) << 52) ^ ((cle.1 as u64 & 0x3FFFFFF) << 26) ^ (cle.2 as u64 & 0x3FFFFFF);
        tu.image = Some(Image { id, revision: 1, largeur: COTE as u32, hauteur: COTE as u32, pixels: Arc::new(px) });
    }
    // Les traits et les noms, relatifs au coin sud-ouest de la tuile
    for (nom, etendue, objets) in &couches {
        let k = t / etendue;
        let rel = |p: &[f32; 2]| [p[0] * k, t - p[1] * k];
        for o in objets {
            let classe = o.txt("class").unwrap_or("");
            match nom.as_str() {
                "transportation" | "waterway" if o.genre == 2 => {
                    let genre = match (nom.as_str(), classe) {
                        ("waterway", "river" | "canal") => RIVIERE,
                        ("waterway", "stream") if cle.0 >= 12 => RIVIERE,
                        ("transportation", "motorway") => AUTOROUTE,
                        ("transportation", "trunk") => RAPIDE,
                        ("transportation", "primary") => PRINCIPALE,
                        ("transportation", "secondary") => SECONDAIRE,
                        ("transportation", "tertiary") => TERTIAIRE,
                        ("transportation", "minor") => RUE,
                        _ => continue,
                    };
                    for p in &o.parties {
                        let pts: Vec<[f32; 2]> = p.iter().map(rel).collect();
                        tu.traits.push(Trait { genre, boite: boite_de(&pts), pts });
                    }
                }
                "place" if o.genre == 1 => {
                    let genre = match classe {
                        "city" => VILLE,
                        "town" => BOURG,
                        "village" => VILLAGE,
                        _ => continue,
                    };
                    let (Some(texte), Some(p)) = (o.txt("name"), o.parties.first().and_then(|p| p.first())) else { continue };
                    // Les noms du tampon autour de la tuile appartiennent aux voisines
                    if p[0] < 0.0 || p[1] < 0.0 || p[0] >= *etendue || p[1] >= *etendue {
                        continue;
                    }
                    tu.noms.push(Nom { genre, rang: o.num("rank").unwrap_or(99.0) as f32, pos: rel(p), texte: texte.to_string() });
                }
                _ => {}
            }
        }
    }
    Some(tu)
}

// ----- Le téléchargement

enum Etat {
    Prete(Arc<Tuile>),
    Echec(Instant),
}

#[derive(Default)]
struct Partage {
    tuiles: HashMap<Cle, Etat>,
    /// Les tuiles à télécharger, la plus pressée à la fin ; celles en cours.
    file: Vec<Cle>,
    en_cours: HashSet<Cle>,
    /// L'adresse des tuiles, `{z}/{x}/{y}` à remplacer.
    modele: Option<String>,
    erreur: Option<String>,
    fin: bool,
}

/// Le fil qui télécharge : la dernière tuile de la file d'abord.
fn telecharger(p: &(Mutex<Partage>, Condvar), cache: &Path, reseau: &dyn Reseau) {
    let _ = std::fs::create_dir_all(cache);
    loop {
        let (cle, modele) = {
            let Ok(mut g) = p.0.lock() else { return };
            loop {
                if g.fin {
                    return;
                }
                if let Some(c) = g.file.pop() {
                    g.en_cours.insert(c);
                    break (c, g.modele.clone());
                }
                let Ok(n) = p.1.wait(g) else { return };
                g = n;
            }
        };
        let rendu = charger(cache, cle, modele, reseau);
        let Ok(mut g) = p.0.lock() else { return };
        g.en_cours.remove(&cle);
        match rendu {
            Ok((t, m)) => {
                g.tuiles.insert(cle, Etat::Prete(Arc::new(t)));
                g.modele = m.or(g.modele.take());
                g.erreur = None;
            }
            Err((e, oublier)) => {
                g.tuiles.insert(cle, Etat::Echec(Instant::now()));
                if oublier {
                    g.modele = None;
                }
                g.erreur = Some(e);
            }
        }
    }
}

/// L'adresse des tuiles, d'après leur description.
fn modele(cache: &Path, reseau: &dyn Reseau) -> Result<String, String> {
    let f = cache.join("planet.json");
    reseau.telecharger(TUILES, None, &f)?;
    let t = std::fs::read_to_string(&f).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&f);
    let j = sage_reel::json::lire(&t).ok_or("description illisible")?;
    j.get("tiles").and_then(|t| t.liste()).and_then(|l| l.first()).and_then(|u| u.txt()).map(String::from).ok_or_else(|| "description sans tuiles".into())
}

/// Une tuile : du cache, sinon d'OpenFreeMap (gardée dans le cache). Rend aussi
/// l'adresse des tuiles si elle a fallu la chercher ; l'erreur dit s'il faut l'oublier.
#[allow(clippy::type_complexity)]
fn charger(cache: &Path, cle: Cle, m: Option<String>, reseau: &dyn Reseau) -> Result<(Tuile, Option<String>), (String, bool)> {
    let f = cache.join(format!("{}-{}-{}.pbf", cle.0, cle.1, cle.2));
    if let Some(t) = std::fs::read(&f).ok().and_then(|b| lire_tuile(&b, cle)) {
        return Ok((t, None));
    }
    let (m, neuf) = match m {
        Some(m) => (m, false),
        None => (modele(cache, reseau).map_err(|e| (e, true))?, true),
    };
    let url = m.replace("{z}", &cle.0.to_string()).replace("{x}", &cle.1.to_string()).replace("{y}", &cle.2.to_string());
    if let Err(e) = reseau.telecharger(&url, None, &f) {
        // Une adresse périmée (une mise à jour des tuiles) : on la redemandera
        let _ = std::fs::remove_file(&f);
        return Err((e.clone(), e.contains("404")));
    }
    let b = std::fs::read(&f).map_err(|e| (e.to_string(), false))?;
    match lire_tuile(&b, cle) {
        Some(t) => Ok((t, neuf.then_some(m))),
        None => {
            let _ = std::fs::remove_file(&f);
            Err(("tuile illisible".into(), false))
        }
    }
}

// ----- Le plan

/// Le plan : la vue (centre et échelle) et les tuiles, téléchargées à part.
pub struct Plan {
    /// Le centre de la vue, en mètres de Mercator.
    pub centre: [f64; 2],
    /// Les pixels par mètre de Mercator ; 0 : à régler au premier dessin, pour que la
    /// vue couvre `largeur_initiale` mètres (vrais).
    pub echelle: f64,
    largeur_initiale: f64,
    partage: Arc<(Mutex<Partage>, Condvar)>,
    glisse: Option<(f32, f32)>,
    pincement: f32,
}

impl Drop for Plan {
    fn drop(&mut self) {
        if let Ok(mut p) = self.partage.0.lock() {
            p.fin = true;
        }
        self.partage.1.notify_all();
    }
}

impl Plan {
    /// Un plan centré sur (lat, lon), large de `largeur` mètres ; ses tuiles téléchargées
    /// par `reseau` et gardées dans `cache`.
    pub fn new<Rs: Reseau + Send + 'static>(lat: f64, lon: f64, largeur: f64, cache: PathBuf, reseau: Rs) -> Plan {
        let partage: Arc<(Mutex<Partage>, Condvar)> = Arc::default();
        let reseau = Arc::new(reseau);
        for _ in 0..FILS {
            let (p, c, r) = (partage.clone(), cache.clone(), reseau.clone());
            std::thread::spawn(move || telecharger(&p, &c, &*r));
        }
        Plan { centre: mercator(lat, lon), echelle: 0.0, largeur_initiale: largeur, partage, glisse: None, pincement: 1.0 }
    }

    /// Le centre de la vue : (latitude, longitude).
    pub fn position(&self) -> (f64, f64) {
        geo(self.centre)
    }

    /// Va en (lat, lon), la vue large de `largeur` mètres.
    pub fn aller(&mut self, lat: f64, lon: f64, largeur: f64) {
        self.centre = mercator(lat, lon);
        self.largeur_initiale = largeur;
        self.echelle = 0.0;
    }

    /// Le rapport des mètres de Mercator aux mètres vrais, au centre.
    fn etirement(&self) -> f64 {
        1.0 / self.position().0.to_radians().cos()
    }

    /// Des tuiles attendent-elles leur téléchargement ?
    pub fn en_attente(&self) -> bool {
        self.partage.0.lock().is_ok_and(|p| !p.en_cours.is_empty() || !p.file.is_empty())
    }

    fn ecran(&self, zone: Rect, m: [f64; 2]) -> [f32; 2] {
        [zone.cx() + ((m[0] - self.centre[0]) * self.echelle) as f32, zone.cy() - ((m[1] - self.centre[1]) * self.echelle) as f32]
    }

    fn monde(&self, zone: Rect, sx: f32, sy: f32) -> [f64; 2] {
        [self.centre[0] + (sx - zone.cx()) as f64 / self.echelle, self.centre[1] - (sy - zone.cy()) as f64 / self.echelle]
    }

    fn bornes(&self, zone: Rect) -> (f64, f64) {
        // Du monde entier à un demi-mètre par pixel
        ((zone.w.max(1.0) as f64) / MONDE, 2.0)
    }

    fn zoomer(&mut self, zone: Rect, k: f64, sx: f32, sy: f32) {
        let m = self.monde(zone, sx, sy);
        let (min, max) = self.bornes(zone);
        self.echelle = (self.echelle * k).clamp(min, max);
        self.centre = [m[0] - (sx - zone.cx()) as f64 / self.echelle, m[1] + (sy - zone.cy()) as f64 / self.echelle];
    }

    /// Un geste sur la carte : un doigt la glisse, deux la zooment, un toucher y centre.
    pub fn recevoir(&mut self, g: crate::gestes::Geste, zone: Rect) {
        use crate::gestes::Geste;
        if self.echelle <= 0.0 {
            return;
        }
        match g {
            Geste::Tap { x, y, .. } => self.centre = self.monde(zone, x, y),
            Geste::SaisieDebut { doigt, .. } => self.glisse = Some(doigt),
            Geste::SaisieDeplace { doigt, .. } => {
                if let Some(a) = self.glisse {
                    self.centre[0] -= (doigt.0 - a.0) as f64 / self.echelle;
                    self.centre[1] += (doigt.1 - a.1) as f64 / self.echelle;
                    self.glisse = Some(doigt);
                }
            }
            Geste::SaisieFin { .. } | Geste::SaisieAnnulee { .. } => self.glisse = None,
            Geste::PincementDebut { .. } => {
                self.pincement = 1.0;
                self.glisse = None;
            }
            Geste::Pincement { echelle, centre, deplacement } => {
                let k = echelle / self.pincement.max(1e-3);
                self.pincement = echelle;
                self.centre[0] -= deplacement.0 as f64 / self.echelle;
                self.centre[1] += deplacement.1 as f64 / self.echelle;
                self.zoomer(zone, k as f64, centre.0, centre.1);
            }
            _ => {}
        }
        // On ne sort pas du monde
        self.centre[0] = self.centre[0].clamp(-MONDE / 2.0, MONDE / 2.0);
        self.centre[1] = self.centre[1].clamp(-MONDE / 2.0 * 0.95, MONDE / 2.0 * 0.95);
    }

    /// Le niveau des tuiles pour l'échelle : une tuile fait de 180 à 360 dp à l'écran.
    fn niveau(&self, dp: f32) -> u8 {
        let z = (self.echelle * MONDE / (180.0 * dp as f64)).log2().floor();
        z.clamp(0.0, NIVEAU_MAX as f64) as u8
    }

    /// Les tuiles qui couvrent la vue, au niveau `z`.
    fn visibles(&self, zone: Rect, z: u8) -> Vec<Cle> {
        let a = self.monde(zone, zone.x, zone.bottom());
        let b = self.monde(zone, zone.right(), zone.y);
        let t = MONDE / (1u64 << z) as f64;
        let n = 1i32 << z;
        let col = |x: f64| (((x + MONDE / 2.0) / t).floor() as i32).clamp(0, n - 1);
        let lig = |y: f64| (((MONDE / 2.0 - y) / t).floor() as i32).clamp(0, n - 1);
        let mut v = Vec::new();
        for l in lig(b[1])..=lig(a[1]) {
            for c in col(a[0])..=col(b[0]) {
                v.push((z, c, l));
            }
        }
        // Du centre vers les bords : le milieu d'abord
        let m = self.monde(zone, zone.cx(), zone.cy());
        let (mc, ml) = ((m[0] + MONDE / 2.0) / t, (MONDE / 2.0 - m[1]) / t);
        v.sort_by(|p, q| {
            let d = |k: &Cle| (k.1 as f64 + 0.5 - mc).hypot(k.2 as f64 + 0.5 - ml);
            d(p).total_cmp(&d(q))
        });
        v
    }

    /// Demande les tuiles qui manquent ; rend, pour chacune, elle ou son ancêtre le plus
    /// proche s'il est là (sans doublon), et l'erreur du dernier téléchargement manqué.
    fn demander(&self, voulues: &[Cle]) -> (Vec<Arc<Tuile>>, Option<String>) {
        let Ok(mut p) = self.partage.0.lock() else { return (Vec::new(), None) };
        let mut montrees: Vec<Arc<Tuile>> = Vec::new();
        let mut vues = HashSet::new();
        let mut file = Vec::new();
        for &c in voulues {
            match p.tuiles.get(&c) {
                Some(Etat::Prete(_)) => {}
                Some(Etat::Echec(quand)) if quand.elapsed() < REESSAI => {}
                _ if p.en_cours.contains(&c) => {}
                _ => file.push(c),
            }
            for k in 0..=c.0.min(6) {
                if let Some(Etat::Prete(t)) = p.tuiles.get(&(c.0 - k, c.1 >> k, c.2 >> k)) {
                    if vues.insert(t.cle) {
                        montrees.push(t.clone());
                    }
                    break;
                }
            }
        }
        // La file se vide par la fin : le milieu de la vue en premier
        file.reverse();
        p.file = file;
        // La mémoire : on oublie ce qui ne se voit plus
        if p.tuiles.len() > GARDEES {
            let garder: HashSet<Cle> = voulues.iter().copied().chain(vues.iter().copied()).collect();
            p.tuiles.retain(|c, _| garder.contains(c));
        }
        let erreur = p.erreur.clone();
        drop(p);
        self.partage.1.notify_all();
        // Les ancêtres (plus grossiers) dessous
        montrees.sort_by_key(|t| t.cle.0);
        (montrees, erreur)
    }

    /// Dessine le plan dans `zone`, et le cadre de l'emprise (en km, est-ouest et
    /// nord-sud) en son centre.
    pub fn dessiner(&mut self, cx: &mut Cx, zone: Rect, emprise: (f32, f32)) {
        let d = cx.dp;
        if self.echelle <= 0.0 {
            self.echelle = zone.w.max(1.0) as f64 / (self.largeur_initiale * self.etirement());
            let (min, max) = self.bornes(zone);
            self.echelle = self.echelle.clamp(min, max);
        }
        cx.clip(Some(zone));
        cx.dl.rrect(zone, 0.0, FOND);
        let z = self.niveau(d);
        let (tuiles, erreur) = self.demander(&self.visibles(zone, z));
        let e = self.echelle;
        let vue = [self.monde(zone, zone.x, zone.bottom()), self.monde(zone, zone.right(), zone.y)];
        let dans = |t: &Tuile, b: [f32; 4]| {
            let (x0, y0) = (vue[0][0] - t.origine[0], vue[0][1] - t.origine[1]);
            let (x1, y1) = (vue[1][0] - t.origine[0], vue[1][1] - t.origine[1]);
            (b[2] as f64) >= x0 && (b[0] as f64) <= x1 && (b[3] as f64) >= y0 && (b[1] as f64) <= y1
        };
        let point = |t: &Tuile, p: [f32; 2]| self.ecran(zone, [t.origine[0] + p[0] as f64, t.origine[1] + p[1] as f64]);
        // Les surfaces
        for t in &tuiles {
            if let Some(img) = &t.image {
                let b = boite(t.cle);
                let (p0, p1) = (self.ecran(zone, [b[0], b[3]]), self.ecran(zone, [b[2], b[1]]));
                cx.dl.image(Rect::new(p0[0], p0[1], p1[0] - p0[0], p1[1] - p0[1]), img.clone());
            }
        }
        // Les traits, des moins importants aux plus importants ; ceux d'un ancêtre ne
        // se tracent que là où sa descendante manque (on ne double pas les routes)
        let fines: HashSet<(i32, i32)> = tuiles.iter().filter(|t| t.cle.0 == z).map(|t| (t.cle.1, t.cle.2)).collect();
        for genre in [RIVIERE, RUE, TERTIAIRE, SECONDAIRE, PRINCIPALE, RAPIDE, AUTOROUTE] {
            let (large, c): (f32, Color) = match genre {
                AUTOROUTE => (3.0, [0xF0, 0x8C, 0x3C, 255]),
                RAPIDE => (2.6, [0xF0, 0xC0, 0x48, 255]),
                PRINCIPALE => (2.0, [0xEE, 0xEE, 0xEE, 255]),
                SECONDAIRE => (1.6, [0xC8, 0xC8, 0xC8, 255]),
                TERTIAIRE => (1.2, [0xA0, 0xA0, 0xA0, 240]),
                RUE => (0.8, [0x84, 0x84, 0x84, 200]),
                _ => (1.6, [0x4A, 0x80, 0xC4, 255]),
            };
            let large = large * d;
            for t in &tuiles {
                let ancetre = t.cle.0 < z;
                for tr in t.traits.iter().filter(|tr| tr.genre == genre) {
                    if !dans(t, tr.boite) {
                        continue;
                    }
                    // Un point tous les deux pixels au moins
                    let mut a = point(t, tr.pts[0]);
                    for (i, p) in tr.pts.iter().enumerate().skip(1) {
                        let b = point(t, *p);
                        if (b[0] - a[0]).hypot(b[1] - a[1]) >= 2.0 || i + 1 == tr.pts.len() {
                            if ancetre {
                                // Le milieu du segment est-il sur une tuile fine déjà là ?
                                let m = self.monde(zone, (a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0);
                                let tz = MONDE / (1u64 << z) as f64;
                                let k = (((m[0] + MONDE / 2.0) / tz).floor() as i32, ((MONDE / 2.0 - m[1]) / tz).floor() as i32);
                                if fines.contains(&k) {
                                    a = b;
                                    continue;
                                }
                            }
                            cx.dl.line(a, b, large, c);
                            a = b;
                        }
                    }
                }
            }
        }
        // Les noms, des plus importants aux moins, sans se chevaucher ni se répéter
        let mut noms: Vec<(&Tuile, &Nom)> = tuiles.iter().flat_map(|t| t.noms.iter().map(move |n| (&**t, n))).collect();
        noms.sort_by(|a, b| (a.1.genre, a.1.rang).partial_cmp(&(b.1.genre, b.1.rang)).unwrap_or(std::cmp::Ordering::Equal));
        let mut poses: Vec<Rect> = Vec::new();
        let mut dits = HashSet::new();
        for (t, n) in noms {
            let s = point(t, n.pos);
            if !zone.contains(s[0], s[1]) || dits.contains(&n.texte) {
                continue;
            }
            let taille = match n.genre {
                VILLE => 15.0,
                BOURG => 13.0,
                _ => 11.5,
            };
            let w = cx.measure(&n.texte, taille);
            let h = taille * 1.3 * d;
            let r = Rect::new(s[0] + 6.0 * d, s[1] - h / 2.0, w, h);
            if poses.iter().any(|q| {
                let i = q.intersect(&r);
                i.w > 0.0 && i.h > 0.0
            }) {
                continue;
            }
            poses.push(Rect::new(s[0] - 4.0 * d, r.y, r.right() - s[0] + 8.0 * d, h).inset(-2.0 * d));
            dits.insert(n.texte.clone());
            let pr = if n.genre == VILLE { 3.5 } else { 2.5 } * d;
            cx.dl.rrect(Rect::new(s[0] - pr, s[1] - pr, 2.0 * pr, 2.0 * pr), pr, [0xF4, 0xF4, 0xF4, 255]);
            cx.text_at(r.x + 1.0 * d, s[1] + 1.0 * d, &n.texte, taille, [0, 0, 0, 220]);
            cx.text_at(r.x, s[1], &n.texte, taille, [0xF4, 0xF4, 0xF4, 255]);
        }
        // Le cadre de l'emprise, le dehors assombri
        let k = self.etirement() * e;
        let (w, h) = ((emprise.0 as f64 * 1000.0 * k) as f32, (emprise.1 as f64 * 1000.0 * k) as f32);
        let cadre = Rect::new(zone.cx() - w / 2.0, zone.cy() - h / 2.0, w, h);
        let ombre = [0, 0, 0, 110];
        cx.dl.rrect(Rect::new(zone.x, zone.y, zone.w, (cadre.y - zone.y).max(0.0)), 0.0, ombre);
        cx.dl.rrect(Rect::new(zone.x, cadre.bottom(), zone.w, (zone.bottom() - cadre.bottom()).max(0.0)), 0.0, ombre);
        let (hy, hh) = (cadre.y.max(zone.y), cadre.bottom().min(zone.bottom()) - cadre.y.max(zone.y));
        cx.dl.rrect(Rect::new(zone.x, hy, (cadre.x - zone.x).max(0.0), hh.max(0.0)), 0.0, ombre);
        cx.dl.rrect(Rect::new(cadre.right(), hy, (zone.right() - cadre.right()).max(0.0), hh.max(0.0)), 0.0, ombre);
        let accent = cx.couleurs().accent;
        let coins = [[cadre.x, cadre.y], [cadre.right(), cadre.y], [cadre.right(), cadre.bottom()], [cadre.x, cadre.bottom()]];
        for i in 0..4 {
            cx.dl.line(coins[i], coins[(i + 1) % 4], 2.0 * d, accent);
        }
        let (c0, br) = ([zone.cx(), zone.cy()], 8.0 * d);
        cx.dl.line([c0[0] - br, c0[1]], [c0[0] + br, c0[1]], 2.0 * d, accent);
        cx.dl.line([c0[0], c0[1] - br], [c0[0], c0[1] + br], 2.0 * d, accent);
        // L'échelle, en bas à gauche (en mètres vrais)
        let vrai = e * self.etirement();
        let metres = 110.0 * d as f64 / vrai;
        let rond = [1.0f64, 2.0, 5.0].iter().flat_map(|m| (0..8).map(move |x| m * 10f64.powi(x))).filter(|v| *v <= metres).fold(1.0, f64::max);
        let lw = (rond * vrai) as f32;
        let (x0, y0) = (zone.x + 12.0 * d, zone.bottom() - 14.0 * d);
        let texte = if rond >= 1000.0 { format!("{} km", rond / 1000.0) } else { format!("{rond} m") };
        cx.dl.rrect(Rect::new(x0 - 4.0 * d, y0 - 20.0 * d, lw + 8.0 * d + 60.0 * d, 26.0 * d), 4.0 * d, [255, 255, 255, 190]);
        cx.dl.line([x0, y0], [x0 + lw, y0], 2.0 * d, [0x20, 0x20, 0x28, 255]);
        cx.text_at(x0 + lw + 6.0 * d, y0 - 6.0 * d, &texte, 12.0, [0x20, 0x20, 0x28, 255]);
        // L'attribution, en haut à droite ; l'état du chargement, en haut à gauche
        let attr = "© OpenFreeMap © OpenMapTiles © OpenStreetMap";
        let aw = cx.measure(attr, 10.0);
        cx.dl.rrect(Rect::new(zone.right() - aw - 10.0 * d, zone.y, aw + 10.0 * d, 18.0 * d), 0.0, [255, 255, 255, 170]);
        cx.text_at(zone.right() - aw - 5.0 * d, zone.y + 9.0 * d, attr, 10.0, [0x20, 0x20, 0x28, 255]);
        let etat = if tuiles.is_empty() {
            Some(match (self.en_attente(), erreur) {
                (true, _) => "Chargement de la carte…".to_string(),
                (false, Some(e)) => format!("Carte indisponible : {e}"),
                (false, None) => "Carte vide".to_string(),
            })
        } else {
            None
        };
        if let Some(t) = etat {
            let tw = cx.measure(&t, 12.0);
            cx.dl.rrect(Rect::new(zone.x + 8.0 * d, zone.y + 26.0 * d, tw + 16.0 * d, 24.0 * d), 12.0 * d, [0, 0, 0, 160]);
            cx.text_at(zone.x + 16.0 * d, zone.y + 38.0 * d, &t, 12.0, [0xF4, 0xF4, 0xF4, 255]);
        }
        cx.clip(None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// De quoi écrire du protobuf.
    fn varint(o: &mut Vec<u8>, mut v: u64) {
        loop {
            let b = (v & 0x7F) as u8;
            v >>= 7;
            if v == 0 {
                o.push(b);
                return;
            }
            o.push(b | 0x80);
        }
    }
    fn champ(o: &mut Vec<u8>, n: u64, b: &[u8]) {
        varint(o, n << 3 | 2);
        varint(o, b.len() as u64);
        o.extend_from_slice(b);
    }
    fn serres(v: &[u32]) -> Vec<u8> {
        let mut o = Vec::new();
        for &x in v {
            varint(&mut o, x as u64);
        }
        o
    }
    fn zz(n: i32) -> u32 {
        ((n << 1) ^ (n >> 31)) as u32
    }
    /// Une couche : nom, clés, valeurs texte, objets (étiquettes, type, géométrie).
    fn couche(nom: &str, cles: &[&str], valeurs: &[&str], objets: &[(&[u32], u64, Vec<u32>)]) -> Vec<u8> {
        let mut c = Vec::new();
        champ(&mut c, 1, nom.as_bytes());
        for (tags, genre, geo) in objets {
            let mut f = Vec::new();
            champ(&mut f, 2, &serres(tags));
            varint(&mut f, 3 << 3);
            varint(&mut f, *genre);
            champ(&mut f, 4, &serres(geo));
            champ(&mut c, 2, &f);
        }
        for k in cles {
            champ(&mut c, 3, k.as_bytes());
        }
        for v in valeurs {
            let mut b = Vec::new();
            champ(&mut b, 1, v.as_bytes());
            champ(&mut c, 4, &b);
        }
        varint(&mut c, 5 << 3);
        varint(&mut c, 4096);
        c
    }

    #[test]
    fn mercator_et_retour() {
        let (lat, lon) = geo(mercator(49.4431, 1.0993));
        assert!((lat - 49.4431).abs() < 1e-9 && (lon - 1.0993).abs() < 1e-9);
        // La tuile 7 qui contient Rouen
        let m = mercator(49.4431, 1.0993);
        let b = boite((7, 64, 43));
        assert!(m[0] >= b[0] && m[0] <= b[2] && m[1] >= b[1] && m[1] <= b[3], "{b:?} {m:?}");
    }

    #[test]
    fn une_tuile_mvt_se_lit() {
        let cle = (10, 515, 349);
        let mut t = Vec::new();
        // Une route principale en diagonale, de (0, 4096) à (4096, 0)
        let route = couche("transportation", &["class"], &["primary"], &[(&[0, 0], 2, vec![1 << 3 | 1, zz(0), zz(4096), 1 << 3 | 2, zz(4096), zz(-4096)])]);
        champ(&mut t, 3, &route);
        // Un lac carré sur le quart nord-ouest, percé d'une île
        // Les déplacements sont relatifs au dernier point : l'île part de (0, 2048)
        let anneau = |dx: i32, dy: i32, c: i32| vec![1 << 3 | 1, zz(dx), zz(dy), 3 << 3 | 2, zz(c), zz(0), zz(0), zz(c), zz(-c), zz(0), 1 << 3 | 7];
        let mut lac = anneau(0, 0, 2048);
        lac.extend(anneau(512, 512 - 2048, 1024));
        assert_eq!(geometrie(&lac, 3).len(), 2);
        champ(&mut t, 3, &couche("water", &["class"], &["lake"], &[(&[0, 0], 3, lac)]));
        // Un bourg au milieu, et un autre dans le tampon (pour la voisine)
        champ(&mut t, 3, &couche("place", &["class", "name"], &["town", "Bourg", "Ailleurs"], &[(&[0, 0, 1, 1], 1, vec![1 << 3 | 1, zz(2048), zz(2048)]), (&[0, 0, 1, 2], 1, vec![1 << 3 | 1, zz(-50), zz(10)])]));
        let tu = lire_tuile(&t, cle).expect("tuile");
        assert_eq!(tu.traits.len(), 1);
        assert_eq!(tu.traits[0].genre, PRINCIPALE);
        let cote = (boite(cle)[2] - boite(cle)[0]) as f32;
        // Le premier point est au sud-ouest, le second au nord-est
        assert!(tu.traits[0].pts[0][0].abs() < 1.0 && tu.traits[0].pts[0][1].abs() < 1.0);
        assert!((tu.traits[0].pts[1][0] - cote).abs() < 1.0 && (tu.traits[0].pts[1][1] - cote).abs() < 1.0);
        assert_eq!(tu.noms.len(), 1);
        assert_eq!(tu.noms[0].texte, "Bourg");
        // L'eau au nord-ouest, pas l'île, pas le sud-est
        let img = tu.image.expect("image");
        let px = |x: usize, y: usize| img.pixels[(y * COTE + x) * 4..(y * COTE + x) * 4 + 3].to_vec();
        assert_eq!(px(10, 10), vec![0x2E, 0x52, 0x80]);
        assert_eq!(px(64, 64), FOND[..3].to_vec());
        assert_eq!(px(200, 200), FOND[..3].to_vec());
    }

    #[test]
    fn l_ile_reste_une_ile() {
        let mut px = vec![0u8; COTE * COTE * 4];
        let mut couv = vec![0f32; COTE * COTE];
        let carre = |x: f32, y: f32, c: f32| vec![[x, y], [x + c, y], [x + c, y + c], [x, y + c]];
        peindre(&mut px, &[carre(0.0, 0.0, 128.0), carre(32.0, 32.0, 64.0)], [255, 0, 0], &mut couv);
        let r = |x: usize, y: usize| px[(y * COTE + x) * 4];
        assert_eq!(r(10, 10), 255);
        assert_eq!(r(64, 64), 0);
        assert_eq!(r(200, 10), 0);
        // Un bord à mi-pixel : à moitié couvert
        let mut px = vec![0u8; COTE * COTE * 4];
        peindre(&mut px, &[carre(0.0, 0.0, 10.5)], [200, 0, 0], &mut couv);
        assert!((95..=105).contains(&px[(5 * COTE + 10) * 4]));
    }

    #[test]
    fn le_niveau_suit_le_zoom_et_le_glisser_deplace() {
        struct Rien;
        impl Reseau for Rien {
            fn telecharger(&self, _: &str, _: Option<&str>, _: &Path) -> Result<(), String> {
                Err("hors ligne".into())
            }
        }
        let mut p = Plan::new(49.44, 1.10, 1.0e6, std::env::temp_dir().join("flow-plan-essai"), Rien);
        let zone = Rect::new(0.0, 0.0, 1000.0, 800.0);
        // La France : un niveau bas, peu de tuiles
        p.echelle = 1000.0 / 1.5e6;
        let z = p.niveau(1.0);
        assert_eq!(z, 7);
        assert!(p.visibles(zone, z).len() <= 42);
        // Sur un écran dense, des tuiles plus grandes : un niveau plus bas
        assert!(p.niveau(2.6) < z);
        // Vingt kilomètres : un niveau fin
        p.echelle = 1000.0 / 3.0e4;
        assert_eq!(p.niveau(1.0), 12);
        // Plus près que tout : le niveau 14
        p.echelle = 2.0;
        assert_eq!(p.niveau(1.0), NIVEAU_MAX);
        // Glisser déplace le centre d'autant de mètres que de pixels / échelle
        p.echelle = 0.1;
        let c = p.centre;
        use crate::gestes::Geste;
        p.recevoir(Geste::SaisieDebut { x: 0.0, y: 0.0, doigt: (500.0, 400.0) }, zone);
        p.recevoir(Geste::SaisieDeplace { x: 0.0, y: 0.0, doigt: (600.0, 400.0) }, zone);
        assert!((p.centre[0] - (c[0] - 1000.0)).abs() < 1e-6);
    }
}
