//! Le contexte Lieux : générer, explorer et éditer les lieux réels de S.A.G.E. (`.reel`).
//!
//! - **Sans lieu ouvert** : la liste des lieux (`Flow/lieux/`), et la génération d'un
//!   nouveau : un nom, une position (« latitude, longitude »), le pas de la grille, l'emprise
//!   (largeur × longueur en km, jusqu'à la carte entière de 20 km), les
//!   toits mesurés au LiDAR ou non. Flow télécharge lui-même (le relief et le LiDAR de
//!   l'IGN, OpenStreetMap par Overpass) et suit l'avancement ; on peut arrêter, et
//!   relancer reprend ce qui est déjà téléchargé (`<lieu>.reel.cache/`). Un lieu
//!   vierge (plat, sans rien) se crée aussi.
//! - **Un lieu ouvert** : la carte (`carte`), en tuiles dessinées à la demande ; deux
//!   doigts zooment et déplacent ; à côté (en paysage) ou dessous (en portrait), le
//!   panneau des outils :
//!   - **Voir** : toucher la carte dit ce qu'il y a là (position, altitude, sol, voie,
//!     bâtiment) ; un doigt déplace la carte.
//!   - **Bâti** : toucher un bâtiment le choisit, le glisser le déplace ; son genre, ses
//!     hauteurs, son toit se règlent dans le panneau ; « Nouveau bâtiment » se trace
//!     point par point.
//!   - **Voies** : routes et ponts ; toucher en choisit une, ses points se tirent ; genre,
//!     largeur, pont ou non ; « Nouvelle voie » se trace point par point.
//!   - **Relief** : un pinceau qui monte, creuse, lisse ou aplanit.
//!   - **Eau** : un pinceau qui met de l'eau (en creusant le lit sous le niveau) ou en
//!     retire ; le niveau de l'eau.
//!   - **Sol** : un pinceau d'occupation du sol (bois, prés, champs, habité, zones).
//! - **Dans la barre de la fenêtre** : la liste des lieux, le nom, enregistrer, annuler
//!   et rétablir.
//!
//! Un lieu généré est dérivé d'OpenStreetMap (ODbL) : il ne se publie pas.

use crate::carte::{boite_bati, boite_route, Carte, Donnees, Vue};
use crate::clavier::{self, Edition, Frappe};
use crate::contexte::{Commun, Contexte, Etat, Genre};
use crate::draw::{Color, Rect};
use crate::gestes::Geste;
use crate::surface::{FenetreId, Placee};
use crate::ui::{id, sub_id, Id};
use crate::widgets::{Cx, Icon};
use sage_reel::generer::{self, Demande, Reseau, Suivi};
use sage_reel::{cote_sol, origine, toits, Batiment, Lieu, Route};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread::JoinHandle;

pub const CLE: &str = "lieux";
pub const GENRE: Genre = Genre { cle: CLE, nom: "Lieux", legende: "Générer, explorer et éditer les lieux réels de S.A.G.E.", icone: Icon::Carte, ouvrir, applet: None };

const LIEUX: Id = id("lieux");
const NOM: Id = id("lieux/nom");
const POSITION: Id = id("lieux/position");
/// Le dossier des lieux, dans celui de Flow.
pub const DOSSIER: &str = "lieux";
const HISTORIQUE: usize = 60;

const OUTILS: [&str; 6] = ["Voir", "Bâti", "Voies", "Relief", "Eau", "Sol"];
const VOIR: usize = 0;
const BATI: usize = 1;
const VOIES: usize = 2;
const RELIEF: usize = 3;
const EAU: usize = 4;
const SOL: usize = 5;

const GENRES_BATI: [&str; 5] = ["Maison", "Immeuble", "Industriel", "Édifice", "Annexe"];
const FORMES: [&str; 4] = ["Plat", "Deux pans", "Croupe", "Appentis"];
const GENRES_VOIE: [&str; 6] = ["Grande route", "Route", "Service", "Piétonne", "Chemin", "Sentier"];
const CLASSES: [&str; 6] = ["Rien", "Bois", "Prés", "Champs", "Habité", "Zones"];
const PINCEAUX: [&str; 4] = ["Monter", "Creuser", "Lisser", "Aplanir"];

/// Le réseau de Flow : Java (`Reseau.java`) sur Android, curl ailleurs (les essais).
struct ReseauFlow;

impl Reseau for ReseauFlow {
    fn telecharger(&self, url: &str, post: Option<&str>, sortie: &Path) -> Result<(), String> {
        let arg = match post {
            Some(p) => format!("{url}\n{}\n{p}", sortie.display()),
            None => format!("{url}\n{}", sortie.display()),
        };
        match crate::acces::reseau(&arg) {
            Some(e) if e.is_empty() => Ok(()),
            Some(e) => Err(e),
            None => generer::Curl.telecharger(url, post, sortie),
        }
    }
}

/// Une liste du lieu : les bâtiments, les routes, les ponts.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Liste {
    Bati,
    Routes,
    Ponts,
}

#[derive(Clone, Debug, PartialEq)]
enum Elem {
    B(Batiment),
    R(Route),
}

/// Ce qu'il faut faire pour défaire une retouche ; l'appliquer rend de quoi la refaire.
#[derive(Clone, Debug, PartialEq)]
enum Retouche {
    Pose(Liste, usize, Elem),
    Insere(Liste, usize, Elem),
    Retire(Liste, usize),
    /// Des points des grilles : (indice, valeur d'avant).
    Grille { relief: Vec<(u32, f32)>, eau: Vec<(u32, bool)>, sol: Vec<(u32, u8)> },
    Niveau(f32),
    Groupe(Vec<Retouche>),
}

/// Ce qui est choisi.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Choix {
    Bati(usize),
    Voie(bool, usize),
}

/// Un doigt qui travaille sur la carte.
#[derive(Clone, Debug)]
enum Trait {
    Glisse((f32, f32)),
    /// Un coup de pinceau : le dernier point peint (en mètres), la hauteur visée
    /// (aplanir), les valeurs d'avant (premier passage seulement).
    Peint { dernier: (f32, f32), cible: f32, vus: HashSet<u32>, retouche: Retouche },
    /// Un bâtiment qu'on déplace : le dernier point (en mètres).
    Bati { k: usize, dernier: (f32, f32), avant: Batiment },
    /// Le point `i` d'une voie qu'on tire.
    Point { pont: bool, k: usize, i: usize, avant: Route },
}

/// Une génération en cours.
struct Generation {
    suivi: Arc<Suivi>,
    fil: JoinHandle<Result<Lieu, String>>,
    sortie: PathBuf,
    journal: Vec<String>,
}

pub struct Lieux {
    chemin: Option<PathBuf>,
    carte: Option<Carte>,
    vue: Option<Vue>,
    outil: usize,
    modifie: bool,
    message: Option<(String, bool)>,
    // L'accueil
    nom: Edition,
    position: Edition,
    pas: f32,
    toits: bool,
    /// L'emprise à générer, en km : est-ouest, nord-sud (20 : la carte entière).
    largeur: f32,
    longueur: f32,
    generation: Option<Generation>,
    chargement: Option<(PathBuf, JoinHandle<Result<Lieu, String>>)>,
    enregistrement: Option<(PathBuf, JoinHandle<Result<(), String>>)>,
    liste: Option<Vec<(PathBuf, u64)>>,
    // L'édition
    choix: Option<Choix>,
    trace: Option<Vec<[f32; 2]>>,
    pinceau: usize,
    rayon: f32,
    force: f32,
    ajouter_eau: bool,
    classe: usize,
    trait_: Option<Trait>,
    pincement: f32,
    passe: Vec<Retouche>,
    futur: Vec<Retouche>,
    /// Une retouche du panneau en cours (un curseur tiré) : elle ne s'empile qu'une fois.
    ouverte: bool,
    info: Option<Vec<String>>,
    /// Le dernier doigt sur la carte (le cercle du pinceau).
    doigt: Option<(f32, f32)>,
    zone: Rect,
    dp: f32,
}

impl Default for Lieux {
    fn default() -> Lieux {
        Lieux {
            chemin: None,
            carte: None,
            vue: None,
            outil: VOIR,
            modifie: false,
            message: None,
            nom: Edition::default(),
            position: Edition::default(),
            pas: 4.0,
            toits: true,
            largeur: 20.0,
            longueur: 20.0,
            generation: None,
            chargement: None,
            enregistrement: None,
            liste: None,
            choix: None,
            trace: None,
            pinceau: 0,
            rayon: 40.0,
            force: 1.0,
            ajouter_eau: true,
            classe: 1,
            trait_: None,
            pincement: 1.0,
            passe: Vec::new(),
            futur: Vec::new(),
            ouverte: false,
            info: None,
            doigt: None,
            zone: Rect::default(),
            dp: 1.0,
        }
    }
}

fn ouvrir(e: &Etat) -> Box<dyn Contexte> {
    let mut l = Lieux::default();
    if let Some(d) = e.texte("document").filter(|d| !d.is_empty()) {
        l.chemin = Some(PathBuf::from(d));
        let v = e.nombres("vue");
        if v.len() == 3 && v[2] > 0.0 {
            l.vue = Some(Vue { cx: v[0] as f64, cz: v[1] as f64, echelle: v[2] });
        }
    }
    l.outil = (e.nombre("outil").unwrap_or(0.0) as usize).min(OUTILS.len() - 1);
    Box::new(l)
}

/// « 49.24, 1.41 », « 49,24 1,41 », « 49.24;1.41 », « 49.24 1.41 » : (latitude,
/// longitude).
pub fn lire_position(t: &str) -> Option<(f64, f64)> {
    let t = t.trim();
    let virgules = t.matches(',').count();
    // Une seule virgule sépare les deux nombres ; deux sont des virgules décimales
    let parties: Vec<&str> = if t.contains(';') {
        t.split(';').collect()
    } else if virgules == 1 {
        t.split(',').collect()
    } else {
        t.split_whitespace().collect()
    };
    let nombres: Vec<f64> = parties.iter().map(|s| s.trim().replace(',', ".")).filter(|s| !s.is_empty()).filter_map(|s| s.parse().ok()).collect();
    if nombres.len() != 2 || parties.iter().filter(|s| !s.trim().is_empty()).count() != 2 {
        return None;
    }
    let (la, lo) = (nombres[0], nombres[1]);
    ((-90.0..=90.0).contains(&la) && (-180.0..=180.0).contains(&lo)).then_some((la, lo))
}

/// Un nom de fichier tiré d'un nom de lieu : minuscules, tirets.
pub fn nom_de_fichier(nom: &str) -> String {
    let mut s = String::new();
    for c in nom.trim().chars() {
        let c = match c {
            'à' | 'â' | 'ä' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'î' | 'ï' => 'i',
            'ô' | 'ö' => 'o',
            'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            c => c,
        };
        if c.is_alphanumeric() {
            s.extend(c.to_lowercase());
        } else if !s.ends_with('-') && !s.is_empty() {
            s.push('-');
        }
    }
    let s = s.trim_end_matches('-').to_string();
    if s.is_empty() { "lieu".into() } else { s }
}

fn lisse(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Le point (x, z) est-il dans le contour ?
fn dedans(c: &[[f32; 2]], x: f32, z: f32) -> bool {
    let mut d = false;
    for e in 0..c.len() {
        let (a, b) = (c[e], c[(e + 1) % c.len()]);
        if (a[1] > z) != (b[1] > z) && x < a[0] + (z - a[1]) / (b[1] - a[1]) * (b[0] - a[0]) {
            d = !d;
        }
    }
    d
}

fn distance_segment(p: (f32, f32), a: [f32; 2], b: [f32; 2]) -> f32 {
    let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
    let l2 = dx * dx + dz * dz;
    let t = if l2 > 0.0 { (((p.0 - a[0]) * dx + (p.1 - a[1]) * dz) / l2).clamp(0.0, 1.0) } else { 0.0 };
    (p.0 - a[0] - dx * t).hypot(p.1 - a[1] - dz * t)
}

fn union(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]
}


/// La taille d'un fichier, lisible.
fn taille(o: u64) -> String {
    crate::explorateur::taille(o)
}

/// Le bâtiment par défaut sur un contour tracé : une maison à deux pans de 6 m.
fn bati_neuf(l: &Lieu, contour: Vec<[f32; 2]>) -> Batiment {
    let sols: Vec<f32> = contour.iter().map(|p| l.relief(p[0] as f64, p[1] as f64) as f32).collect();
    let bas = sols.iter().cloned().fold(f32::MAX, f32::min);
    let moyen = sols.iter().sum::<f32>() / sols.len().max(1) as f32;
    let axe = toits::axe_long(&contour);
    let k = toits::Cadre::new(&contour, axe);
    let egout = moyen + 6.0;
    let toit = egout + (40f32.to_radians().tan() * k.b).min(4.0);
    Batiment { genre: sage_reel::MAISON, pied: bas - 1.0, toit, egout, forme: toits::DEUX_PANS, axe, contour }
}

/// Une voie tracée : un point tous les 8 m, la hauteur du relief lissée (route) ou
/// tirée droite d'un bout à l'autre au-dessus du sol et de l'eau (pont).
fn voie_neuve(l: &Lieu, trace: &[[f32; 2]], pont: bool, genre: u8, largeur: f32) -> Route {
    let mut pts = vec![trace[0]];
    for s in trace.windows(2) {
        let d = (s[1][0] - s[0][0]).hypot(s[1][1] - s[0][1]);
        let k = (d / 8.0).ceil().max(1.0) as usize;
        for i in 1..=k {
            let t = i as f32 / k as f32;
            pts.push([s[0][0] + (s[1][0] - s[0][0]) * t, s[0][1] + (s[1][1] - s[0][1]) * t]);
        }
    }
    let sol: Vec<f32> = pts.iter().map(|p| l.relief(p[0] as f64, p[1] as f64) as f32).collect();
    let hs: Vec<f32> = if pont {
        let (a, b) = (sol[0].max(l.niveau + 0.5), sol[sol.len() - 1].max(l.niveau + 0.5));
        (0..pts.len()).map(|i| a + (b - a) * i as f32 / (pts.len() - 1).max(1) as f32).collect()
    } else {
        (0..sol.len()).map(|i| {
            let (a, b) = (i.saturating_sub(2), (i + 3).min(sol.len()));
            sol[a..b].iter().sum::<f32>() / (b - a) as f32
        }).collect()
    };
    Route { genre, largeur, points: pts.iter().zip(&hs).map(|(p, h)| [p[0], p[1], *h]).collect() }
}

impl Lieux {
    fn dossier(c: &Commun) -> Option<PathBuf> {
        c.dossier.as_ref().map(|d| d.join(DOSSIER))
    }

    /// L'emprise en mètres pour la génération : 20 km (le bout du curseur), la carte
    /// entière (0), qui fait un peu plus.
    fn emprise_m(&self) -> (f64, f64) {
        let m = |km: f32| if km >= 20.0 { 0.0 } else { km as f64 * 1000.0 };
        (m(self.largeur), m(self.longueur))
    }

    fn lister(&mut self, c: &Commun) {
        let mut v = Vec::new();
        if let Some(d) = Lieux::dossier(c) {
            if let Ok(rd) = std::fs::read_dir(&d) {
                for e in rd.flatten() {
                    let p = e.path();
                    if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("reel")) {
                        v.push((p, e.metadata().map_or(0, |m| m.len())));
                    }
                }
            }
        }
        v.sort();
        self.liste = Some(v);
    }

    fn charger(&mut self, chemin: PathBuf) {
        let f = chemin.clone();
        self.chargement = Some((chemin, std::thread::spawn(move || Lieu::lire(&f))));
        self.message = None;
    }

    /// Ouvre un lieu tout prêt (lu, généré, vierge).
    fn ouvrir_lieu(&mut self, lieu: Lieu, chemin: Option<PathBuf>, modifie: bool) {
        self.carte = Some(Carte::new(lieu));
        if self.chemin != chemin {
            self.vue = None;
        }
        self.chemin = chemin;
        self.modifie = modifie;
        self.passe.clear();
        self.futur.clear();
        self.choix = None;
        self.trace = None;
        self.trait_ = None;
        self.info = None;
    }

    fn fermer_lieu(&mut self) {
        self.carte = None;
        self.chemin = None;
        self.vue = None;
        self.modifie = false;
        self.passe.clear();
        self.futur.clear();
        self.choix = None;
        self.trace = None;
        self.liste = None;
    }

    fn generer(&mut self, c: &Commun) {
        let nom = self.nom.texte.trim().to_string();
        if nom.is_empty() {
            self.message = Some(("donnez un nom au lieu".into(), true));
            return;
        }
        let Some((lat, lon)) = lire_position(&self.position.texte) else {
            self.message = Some(("position illisible : « 49.2464, 1.4119 »".into(), true));
            return;
        };
        let Some(dossier) = Lieux::dossier(c) else {
            self.message = Some(("pas de dossier pour Flow : l'accès au stockage est-il accordé ?".into(), true));
            return;
        };
        let sortie = dossier.join(format!("{}.reel", nom_de_fichier(&nom)));
        let d = Demande { nom, lat, lon, pas: self.pas as f64, sortie: sortie.clone(), toits: self.toits, largeur: self.emprise_m().0, longueur: self.emprise_m().1 };
        let suivi = Arc::new(Suivi::default());
        let s = suivi.clone();
        let fil = std::thread::spawn(move || {
            let r = generer::generer(&d, &ReseauFlow, &s);
            crate::radio::reveiller();
            r
        });
        self.generation = Some(Generation { suivi, fil, sortie, journal: Vec::new() });
        self.message = None;
    }

    fn creer_vierge(&mut self, c: &Commun) {
        let nom = self.nom.texte.trim().to_string();
        let nom = if nom.is_empty() { "Lieu vierge".to_string() } else { nom };
        let (lat, lon) = lire_position(&self.position.texte).unwrap_or((46.5, 3.0));
        let chemin = Lieux::dossier(c).map(|d| d.join(format!("{}.reel", nom_de_fichier(&nom))));
        if chemin.as_ref().is_some_and(|p| p.exists()) {
            self.message = Some(("un lieu de ce nom existe déjà".into(), true));
            return;
        }
        // Un pas lâche : un lieu vierge est léger
        self.ouvrir_lieu(Lieu::vierge(&nom, lat, lon, self.pas.max(4.0) as f64, 100.0), chemin, true);
    }

    fn enregistrer(&mut self) {
        if self.enregistrement.is_some() {
            return;
        }
        let (Some(carte), Some(chemin)) = (&self.carte, self.chemin.clone()) else {
            self.message = Some(("pas de fichier où enregistrer".into(), true));
            return;
        };
        let Ok(d) = carte.donnees.read() else { return };
        let lieu = d.lieu.clone();
        drop(d);
        let f = chemin.clone();
        self.enregistrement = Some((chemin, std::thread::spawn(move || {
            let r = lieu.ecrire(&f).map_err(|e| e.to_string());
            crate::radio::reveiller();
            r
        })));
        self.modifie = false;
        self.message = Some(("enregistrement…".into(), false));
    }

    /// Les fils qui ont fini : chargement, génération, enregistrement.
    fn suivre(&mut self) {
        if self.chargement.as_ref().is_some_and(|(_, f)| f.is_finished()) {
            if let Some((p, f)) = self.chargement.take() {
                match f.join() {
                    Ok(Ok(l)) => self.ouvrir_lieu(l, Some(p), false),
                    Ok(Err(e)) => self.message = Some((format!("{} : {e}", p.display()), true)),
                    Err(_) => self.message = Some(("lecture interrompue".into(), true)),
                }
            }
        }
        if let Some(g) = &mut self.generation {
            let n = g.journal.len();
            g.journal.extend(g.suivi.journal(n));
            if g.fil.is_finished() {
                if let Some(g) = self.generation.take() {
                    match g.fil.join() {
                        Ok(Ok(l)) => {
                            self.ouvrir_lieu(l, Some(g.sortie), false);
                            self.message = Some(("lieu généré".into(), false));
                        }
                        Ok(Err(e)) => self.message = Some((format!("génération : {e}"), true)),
                        Err(_) => self.message = Some(("génération interrompue".into(), true)),
                    }
                    self.liste = None;
                }
            }
        }
        if self.enregistrement.as_ref().is_some_and(|(_, f)| f.is_finished()) {
            if let Some((p, f)) = self.enregistrement.take() {
                match f.join() {
                    Ok(Ok(())) => self.message = Some((format!("enregistré : {}", p.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned())), false)),
                    Ok(Err(e)) => {
                        self.modifie = true;
                        self.message = Some((format!("non enregistré : {e}"), true));
                    }
                    Err(_) => self.modifie = true,
                }
                self.liste = None;
            }
        }
    }

    // ----- Les retouches

    /// Applique une retouche ; rend celle qui la défait, et la zone qui a changé.
    fn appliquer(d: &mut Donnees, r: Retouche) -> (Retouche, [f32; 4], bool) {
        let l = &mut d.lieu;
        let zone_elem = |e: &Elem| match e {
            Elem::B(b) => boite_bati(b),
            Elem::R(r) => boite_route(r),
        };
        let vide = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
        match r {
            Retouche::Pose(li, k, e) => {
                let avant = match (li, &e) {
                    (Liste::Bati, Elem::B(b)) => Elem::B(std::mem::replace(&mut l.batiments[k], b.clone())),
                    (Liste::Routes, Elem::R(r)) => Elem::R(std::mem::replace(&mut l.routes[k], r.clone())),
                    (Liste::Ponts, Elem::R(r)) => Elem::R(std::mem::replace(&mut l.ponts[k], r.clone())),
                    _ => return (Retouche::Groupe(Vec::new()), vide, false),
                };
                let z = union(zone_elem(&avant), zone_elem(&e));
                (Retouche::Pose(li, k, avant), z, true)
            }
            Retouche::Insere(li, k, e) => {
                let z = zone_elem(&e);
                match (li, e) {
                    (Liste::Bati, Elem::B(b)) => l.batiments.insert(k.min(l.batiments.len()), b),
                    (Liste::Routes, Elem::R(r)) => l.routes.insert(k.min(l.routes.len()), r),
                    (Liste::Ponts, Elem::R(r)) => l.ponts.insert(k.min(l.ponts.len()), r),
                    _ => {}
                }
                (Retouche::Retire(li, k), z, true)
            }
            Retouche::Retire(li, k) => {
                let e = match li {
                    Liste::Bati => Elem::B(l.batiments.remove(k)),
                    Liste::Routes => Elem::R(l.routes.remove(k)),
                    Liste::Ponts => Elem::R(l.ponts.remove(k)),
                };
                let z = zone_elem(&e);
                (Retouche::Insere(li, k, e), z, true)
            }
            Retouche::Grille { relief, eau, sol } => {
                let (n, m) = (l.cote, cote_sol(l.cote));
                let (o, p) = (origine() as f32, l.pas as f32);
                let mut z = vide;
                let mut etendre = |i: u32, cote: usize, pas: f32| {
                    let (a, b) = ((i as usize % cote) as f32 * pas + o, (i as usize / cote) as f32 * pas + o);
                    z = union(z, [a - pas, b - pas, a + pas, b + pas]);
                };
                let mut r2 = Vec::with_capacity(relief.len());
                for &(i, _) in &relief {
                    r2.push((i, l.h_point((i as usize % n) as i64, (i as usize / n) as i64)));
                    etendre(i, n, p);
                }
                l.poser_hauteurs(&relief.iter().map(|&(i, v)| (i as usize, v)).collect::<Vec<_>>());
                let mut e2 = Vec::with_capacity(eau.len());
                for &(i, v) in &eau {
                    let (a, b) = ((i as usize % n) as i64, (i as usize / n) as i64);
                    e2.push((i, l.est_eau(a, b)));
                    l.poser_eau(a, b, v);
                    etendre(i, n, p);
                }
                let mut s2 = Vec::with_capacity(sol.len());
                if !sol.is_empty() && l.sol.len() != m * m {
                    l.sol = vec![0; m * m];
                }
                for &(i, v) in &sol {
                    if let Some(x) = l.sol.get_mut(i as usize) {
                        s2.push((i, *x));
                        *x = v;
                        etendre(i, m, p * 2.0);
                    }
                }
                (Retouche::Grille { relief: r2, eau: e2, sol: s2 }, z, false)
            }
            Retouche::Niveau(v) => {
                let avant = std::mem::replace(&mut l.niveau, v);
                (Retouche::Niveau(avant), [f32::MIN, f32::MIN, f32::MAX, f32::MAX], false)
            }
            Retouche::Groupe(v) => {
                let (mut inv, mut z, mut idx) = (Vec::new(), vide, false);
                for r in v {
                    let (i, zz, x) = Lieux::appliquer(d, r);
                    inv.push(i);
                    z = union(z, zz);
                    idx |= x;
                }
                inv.reverse();
                (Retouche::Groupe(inv), z, idx)
            }
        }
    }

    /// Fait une retouche (elle s'empile pour être défaite).
    fn faire(&mut self, r: Retouche) {
        let Some(carte) = &mut self.carte else { return };
        let (inv, z, idx) = match carte.donnees.write() {
            Ok(mut d) => {
                let x = Lieux::appliquer(&mut d, r);
                if x.2 {
                    d.reindexer();
                }
                x
            }
            Err(_) => return,
        };
        carte.retoucher(z);
        let _ = idx;
        self.empiler(inv);
    }

    fn empiler(&mut self, inv: Retouche) {
        self.passe.push(inv);
        if self.passe.len() > HISTORIQUE {
            self.passe.remove(0);
        }
        self.futur.clear();
        self.modifie = true;
    }

    fn defaire(&mut self, retablir: bool) {
        let pile = if retablir { &mut self.futur } else { &mut self.passe };
        let Some(r) = pile.pop() else { return };
        let Some(carte) = &mut self.carte else { return };
        let (inv, z) = match carte.donnees.write() {
            Ok(mut d) => {
                let (i, z, idx) = Lieux::appliquer(&mut d, r);
                if idx {
                    d.reindexer();
                }
                (i, z)
            }
            Err(_) => return,
        };
        carte.retoucher(z);
        if retablir { &mut self.passe } else { &mut self.futur }.push(inv);
        self.modifie = true;
        // Ce qui était choisi a pu disparaître
        self.choix = None;
        self.trait_ = None;
    }

    // ----- La carte

    fn vue_initiale(&self, zone: Rect) -> Vue {
        let c = self.carte.as_ref().and_then(|c| c.donnees.read().ok().map(|d| d.monde())).unwrap_or((-10000.0, 20000.0));
        Vue { cx: (c.0 + c.1 / 2.0) as f64, cz: (c.0 + c.1 / 2.0) as f64, echelle: zone.w.min(zone.h) / c.1 }
    }

    /// Ce qu'il y a au point (x, z), en quelques lignes.
    fn decrire(d: &Donnees, x: f32, z: f32) -> Vec<String> {
        let l = &d.lieu;
        let (la, lo) = l.vers_degres(x as f64, z as f64);
        let mut v = vec![format!("{la:.5}, {lo:.5}"), format!("altitude {:.1} m", l.relief(x as f64, z as f64)).replace('.', ",")];
        if l.eau(x as f64, z as f64) >= 0.5 {
            v.push(format!("eau (surface à {:.1} m)", l.niveau).replace('.', ","));
        } else {
            v.push(format!("sol : {}", CLASSES[(l.classe(x as f64, z as f64) as usize).min(5)]));
        }
        if let Some(k) = Lieux::bati_sous(d, x, z) {
            let b = &l.batiments[k];
            v.push(format!("{} · {} · faîtage à {:.1} m", GENRES_BATI[(b.genre as usize).min(4)], FORMES[(b.forme as usize).min(3)], b.toit - b.pied - 1.0).replace('.', ","));
        }
        if let Some((pont, k)) = Lieux::voie_pres(d, x, z, 6.0) {
            let r = if pont { &l.ponts[k] } else { &l.routes[k] };
            v.push(format!("{}{} · {:.1} m de large", GENRES_VOIE[(r.genre as usize).min(5)], if pont { " (pont)" } else { "" }, r.largeur).replace('.', ","));
        }
        v
    }

    /// Le plus petit bâtiment qui contient (x, z).
    fn bati_sous(d: &Donnees, x: f32, z: f32) -> Option<usize> {
        let l = &d.lieu;
        d.batiments_dans([x - 1.0, z - 1.0, x + 1.0, z + 1.0])
            .into_iter()
            .map(|k| k as usize)
            .filter(|&k| dedans(&l.batiments[k].contour, x, z))
            .min_by(|&a, &b| sage_reel::aire(&l.batiments[a].contour).abs().total_cmp(&sage_reel::aire(&l.batiments[b].contour).abs()))
    }

    /// La voie la plus proche de (x, z), à `marge` mètres de son bord au plus ; les ponts
    /// d'abord.
    fn voie_pres(d: &Donnees, x: f32, z: f32, marge: f32) -> Option<(bool, usize)> {
        let mut mieux: Option<(f32, bool, usize)> = None;
        for pont in [true, false] {
            let liste = if pont { &d.lieu.ponts } else { &d.lieu.routes };
            for k in d.routes_dans([x - marge, z - marge, x + marge, z + marge], pont) {
                let r = &liste[k as usize];
                for s in r.points.windows(2) {
                    let e = distance_segment((x, z), [s[0][0], s[0][1]], [s[1][0], s[1][1]]) - r.largeur / 2.0 - if pont { 1.0 } else { 0.0 };
                    if e < marge && mieux.is_none_or(|m| e < m.0) {
                        mieux = Some((e, pont, k as usize));
                    }
                }
            }
        }
        mieux.map(|m| (m.1, m.2))
    }

    /// Un coup de pinceau en (x, z).
    fn peindre(&mut self, x: f32, z: f32) {
        let (outil, pinceau, rayon, force, ajouter, classe) = (self.outil, self.pinceau, self.rayon, self.force, self.ajouter_eau, self.classe as u8);
        let Some(Trait::Peint { cible, vus, retouche, .. }) = &mut self.trait_ else { return };
        let Some(carte) = &mut self.carte else { return };
        let Ok(mut d) = carte.donnees.write() else { return };
        let l = &mut d.lieu;
        let Retouche::Grille { relief, eau, sol } = retouche else { return };
        let (o, p) = (origine() as f32, l.pas as f32);
        let n = l.cote as i64;
        let zone = [x - rayon, z - rayon, x + rayon, z + rayon];
        match outil {
            RELIEF | EAU => {
                let (a0, a1) = (((x - rayon - o) / p).floor() as i64, ((x + rayon - o) / p).ceil() as i64);
                let (b0, b1) = (((z - rayon - o) / p).floor() as i64, ((z + rayon - o) / p).ceil() as i64);
                let mut changes = Vec::new();
                for b in b0.max(0)..=b1.min(n - 1) {
                    for a in a0.max(0)..=a1.min(n - 1) {
                        let (px, pz) = (a as f32 * p + o, b as f32 * p + o);
                        let dd = (px - x).hypot(pz - z);
                        if dd > rayon {
                            continue;
                        }
                        let w = lisse(1.0 - dd / rayon);
                        let i = (b * n + a) as u32;
                        let h = l.h_point(a, b);
                        let neuf = if outil == RELIEF {
                            match pinceau {
                                0 => h + force * w,
                                1 => h - force * w,
                                2 => {
                                    let m = (l.h_point(a - 1, b) + l.h_point(a + 1, b) + l.h_point(a, b - 1) + l.h_point(a, b + 1)) / 4.0;
                                    h + (m - h) * (0.5 * w * force.min(4.0) / 4.0 + 0.2 * w).min(1.0)
                                }
                                _ => h + (*cible - h) * (0.5 * w).min(1.0),
                            }
                        } else {
                            // L'eau : posée ou retirée au-delà de la moitié du rayon ; le lit
                            // creusé sous le niveau, la berge relevée au-dessus
                            if w > 0.15 && l.est_eau(a, b) != ajouter {
                                if vus.insert(i | 0x8000_0000) {
                                    eau.push((i, l.est_eau(a, b)));
                                }
                                l.poser_eau(a, b, ajouter);
                            }
                            if ajouter && w > 0.15 {
                                h.min(l.niveau - 0.8 - 1.5 * w)
                            } else if !ajouter && w > 0.15 {
                                h.max(l.niveau + 0.3)
                            } else {
                                h
                            }
                        };
                        if (neuf - h).abs() > 1e-4 {
                            if vus.insert(i) {
                                relief.push((i, h));
                            }
                            changes.push((i as usize, neuf));
                        }
                    }
                }
                l.poser_hauteurs(&changes);
            }
            _ => {
                let m = cote_sol(l.cote) as i64;
                if l.sol.len() != (m * m) as usize {
                    l.sol = vec![0; (m * m) as usize];
                }
                let q = 2.0 * p;
                let (a0, a1) = (((x - rayon - o) / q).floor() as i64, ((x + rayon - o) / q).ceil() as i64);
                let (b0, b1) = (((z - rayon - o) / q).floor() as i64, ((z + rayon - o) / q).ceil() as i64);
                for b in b0.max(0)..=b1.min(m - 1) {
                    for a in a0.max(0)..=a1.min(m - 1) {
                        if (a as f32 * q + o - x).hypot(b as f32 * q + o - z) > rayon {
                            continue;
                        }
                        let i = (b * m + a) as usize;
                        if l.sol[i] != classe {
                            if vus.insert(i as u32) {
                                sol.push((i as u32, l.sol[i]));
                            }
                            l.sol[i] = classe;
                        }
                    }
                }
            }
        }
        drop(d);
        carte.retoucher([zone[0] - p, zone[1] - p, zone[2] + p, zone[3] + p]);
        self.modifie = true;
    }

    /// Les gestes sur la carte.
    fn recevoir(&mut self, g: Geste, zone: Rect) {
        let Some(mut vue) = self.vue else { return };
        let (min, max) = {
            let c = self.carte.as_ref().and_then(|c| c.donnees.read().ok().map(|d| d.monde().1)).unwrap_or(20000.0);
            (zone.w.min(zone.h) / c * 0.5, 8.0)
        };
        let peint = matches!(self.outil, RELIEF | EAU | SOL);
        match g {
            Geste::Tap { x, y, .. } => {
                let (wx, wz) = vue.monde(zone, x, y);
                self.doigt = Some((x, y));
                if let Some(t) = &mut self.trace {
                    t.push([wx, wz]);
                } else if peint {
                    self.trait_ = Some(Trait::Peint { dernier: (wx, wz), cible: 0.0, vus: HashSet::new(), retouche: Retouche::Grille { relief: Vec::new(), eau: Vec::new(), sol: Vec::new() } });
                    self.cible_aplanir(wx, wz);
                    self.peindre(wx, wz);
                    self.finir_trait();
                } else if let Some(carte) = &self.carte {
                    if let Ok(d) = carte.donnees.read() {
                        match self.outil {
                            VOIR => self.info = Some(Lieux::decrire(&d, wx, wz)),
                            BATI => self.choix = Lieux::bati_sous(&d, wx, wz).map(Choix::Bati),
                            VOIES => self.choix = Lieux::voie_pres(&d, wx, wz, 14.0 / vue.echelle).map(|(p, k)| Choix::Voie(p, k)),
                            _ => {}
                        }
                    }
                }
            }
            Geste::SaisieDebut { doigt, .. } => {
                let (wx, wz) = vue.monde(zone, doigt.0, doigt.1);
                self.doigt = Some(doigt);
                self.trait_ = Some(Trait::Glisse(doigt));
                if peint && self.trace.is_none() {
                    self.trait_ = Some(Trait::Peint { dernier: (wx, wz), cible: 0.0, vus: HashSet::new(), retouche: Retouche::Grille { relief: Vec::new(), eau: Vec::new(), sol: Vec::new() } });
                    self.cible_aplanir(wx, wz);
                    self.peindre(wx, wz);
                } else if let (Some(carte), Some(ch)) = (&self.carte, self.choix) {
                    if let Ok(d) = carte.donnees.read() {
                        match ch {
                            Choix::Bati(k) if k < d.lieu.batiments.len() && dedans(&d.lieu.batiments[k].contour, wx, wz) => {
                                self.trait_ = Some(Trait::Bati { k, dernier: (wx, wz), avant: d.lieu.batiments[k].clone() });
                            }
                            Choix::Voie(pont, k) => {
                                let r = if pont { d.lieu.ponts.get(k) } else { d.lieu.routes.get(k) };
                                if let Some(r) = r {
                                    let proche = r.points.iter().enumerate().map(|(i, q)| {
                                        let s = vue.ecran(zone, q[0], q[1]);
                                        (i, (s[0] - doigt.0).hypot(s[1] - doigt.1))
                                    }).min_by(|a, b| a.1.total_cmp(&b.1));
                                    if let Some((i, _)) = proche.filter(|p| p.1 < 24.0 * self.dp) {
                                        self.trait_ = Some(Trait::Point { pont, k, i, avant: r.clone() });
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            Geste::SaisieDeplace { doigt, .. } => {
                self.doigt = Some(doigt);
                let (wx, wz) = vue.monde(zone, doigt.0, doigt.1);
                match self.trait_.clone() {
                    Some(Trait::Glisse(avant)) => {
                        vue.deplacer(doigt.0 - avant.0, doigt.1 - avant.1);
                        self.trait_ = Some(Trait::Glisse(doigt));
                    }
                    Some(Trait::Peint { dernier, .. }) => {
                        // Des touches régulières le long du chemin
                        let pas = (self.rayon / 3.0).max(1.0);
                        let d = (wx - dernier.0).hypot(wz - dernier.1);
                        let k = (d / pas).floor() as usize;
                        for s in 1..=k {
                            let t = s as f32 * pas / d;
                            self.peindre(dernier.0 + (wx - dernier.0) * t, dernier.1 + (wz - dernier.1) * t);
                        }
                        if k > 0 {
                            if let Some(Trait::Peint { dernier: dd, .. }) = &mut self.trait_ {
                                let t = k as f32 * pas / d;
                                *dd = (dernier.0 + (wx - dernier.0) * t, dernier.1 + (wz - dernier.1) * t);
                            }
                        }
                    }
                    Some(Trait::Bati { k, dernier, avant }) => {
                        if let Some(carte) = &mut self.carte {
                            let z0 = boite_bati(&avant);
                            let zone = carte.donnees.write().ok().and_then(|mut d| {
                                let b = d.lieu.batiments.get_mut(k)?;
                                let z1 = boite_bati(b);
                                for p in &mut b.contour {
                                    p[0] += wx - dernier.0;
                                    p[1] += wz - dernier.1;
                                }
                                Some(union(z1, boite_bati(b)))
                            });
                            if let Some(z) = zone {
                                carte.retoucher(union(z0, z));
                            }
                            self.trait_ = Some(Trait::Bati { k, dernier: (wx, wz), avant });
                            self.modifie = true;
                        }
                    }
                    Some(Trait::Point { pont, k, i, avant }) => {
                        if let Some(carte) = &mut self.carte {
                            let zone = carte.donnees.write().ok().and_then(|mut d| {
                                let h = d.lieu.relief(wx as f64, wz as f64) as f32;
                                let r = if pont { d.lieu.ponts.get_mut(k) } else { d.lieu.routes.get_mut(k) }?;
                                let z1 = boite_route(r);
                                if let Some(q) = r.points.get_mut(i) {
                                    *q = [wx, wz, if pont { q[2] } else { h }];
                                }
                                Some(union(z1, boite_route(r)))
                            });
                            if let Some(z) = zone {
                                carte.retoucher(union(z, boite_route(&avant)));
                            }
                            self.modifie = true;
                        }
                    }
                    None => {}
                }
            }
            Geste::SaisieFin { .. } | Geste::SaisieAnnulee { .. } => self.finir_trait(),
            Geste::PincementDebut { .. } => {
                self.pincement = 1.0;
                // Deux doigts : la carte, pas le pinceau
                if matches!(self.trait_, Some(Trait::Glisse(_))) {
                    self.trait_ = None;
                }
            }
            Geste::Pincement { echelle, centre, deplacement } => {
                let k = echelle / self.pincement.max(1e-3);
                self.pincement = echelle;
                vue.deplacer(deplacement.0, deplacement.1);
                vue.zoomer(zone, k, centre.0, centre.1, min, max);
            }
            _ => {}
        }
        self.vue = Some(vue);
    }

    fn cible_aplanir(&mut self, x: f32, z: f32) {
        let h = self.carte.as_ref().and_then(|c| c.donnees.read().ok().map(|d| d.lieu.relief(x as f64, z as f64) as f32)).unwrap_or(0.0);
        if let Some(Trait::Peint { cible, .. }) = &mut self.trait_ {
            *cible = h;
        }
    }

    /// Le doigt se lève : un coup de pinceau s'empile, un bâtiment déplacé suit le sol,
    /// un point tiré aussi.
    fn finir_trait(&mut self) {
        match self.trait_.take() {
            Some(Trait::Peint { retouche, .. }) => {
                if let Retouche::Grille { relief, eau, sol } = &retouche {
                    if !(relief.is_empty() && eau.is_empty() && sol.is_empty()) {
                        self.empiler(retouche);
                    }
                }
            }
            Some(Trait::Bati { k, avant, .. }) => {
                let Some(carte) = &mut self.carte else { return };
                if let Ok(mut d) = carte.donnees.write() {
                    let sol = |l: &Lieu, b: &Batiment| b.contour.iter().map(|p| l.relief(p[0] as f64, p[1] as f64) as f32).fold(f32::MAX, f32::min);
                    let s0 = sol(&d.lieu, &avant);
                    let s1 = sol(&d.lieu, &d.lieu.batiments[k]);
                    let b = &mut d.lieu.batiments[k];
                    b.pied += s1 - s0;
                    b.egout += s1 - s0;
                    b.toit += s1 - s0;
                    d.reindexer();
                }
                carte.retoucher(boite_bati(&avant));
                self.empiler(Retouche::Pose(Liste::Bati, k, Elem::B(avant)));
            }
            Some(Trait::Point { pont, k, avant, .. }) => {
                if let Some(carte) = &mut self.carte {
                    if let Ok(mut d) = carte.donnees.write() {
                        d.reindexer();
                    }
                }
                self.empiler(Retouche::Pose(if pont { Liste::Ponts } else { Liste::Routes }, k, Elem::R(avant)));
            }
            _ => {}
        }
    }

    /// La carte et ce qui se dessine dessus.
    fn dessiner_carte(&mut self, cx: &mut Cx, zone: Rect, base: Id) {
        let d = cx.dp;
        if self.vue.is_none() {
            self.vue = Some(self.vue_initiale(zone));
        }
        for g in cx.ui.gestes(sub_id(base, 1), zone) {
            self.recevoir(g, zone);
        }
        let Some(vue) = self.vue else { return };
        let Some(carte) = &mut self.carte else { return };
        carte.dessiner(cx, zone, &vue);
        cx.clip(Some(zone));
        let accent = cx.couleurs().accent;
        // Ce qui est choisi
        if let Ok(dn) = carte.donnees.read() {
            match self.choix {
                Some(Choix::Bati(k)) if k < dn.lieu.batiments.len() => {
                    let c = &dn.lieu.batiments[k].contour;
                    for e in 0..c.len() {
                        let (a, b) = (c[e], c[(e + 1) % c.len()]);
                        cx.dl.line(vue.ecran(zone, a[0], a[1]), vue.ecran(zone, b[0], b[1]), 3.0 * d, accent);
                    }
                }
                Some(Choix::Voie(pont, k)) => {
                    let r = if pont { dn.lieu.ponts.get(k) } else { dn.lieu.routes.get(k) };
                    if let Some(r) = r {
                        let large = (r.largeur * vue.echelle).max(3.0 * d);
                        for s in r.points.windows(2) {
                            cx.dl.line(vue.ecran(zone, s[0][0], s[0][1]), vue.ecran(zone, s[1][0], s[1][1]), large + 4.0 * d, [accent[0], accent[1], accent[2], 110]);
                        }
                        // Les points, quand ils ne se chevauchent pas
                        let ecart = r.points.windows(2).map(|s| (s[1][0] - s[0][0]).hypot(s[1][1] - s[0][1])).fold(f32::MAX, f32::min) * vue.echelle;
                        if ecart > 14.0 * d && r.points.len() < 2000 {
                            for q in &r.points {
                                let s = vue.ecran(zone, q[0], q[1]);
                                let rr = 6.0 * d;
                                cx.dl.rrect(Rect::new(s[0] - rr, s[1] - rr, 2.0 * rr, 2.0 * rr), rr, [255, 255, 255, 255]);
                                cx.dl.rrect(Rect::new(s[0] - rr + 2.0 * d, s[1] - rr + 2.0 * d, 2.0 * rr - 4.0 * d, 2.0 * rr - 4.0 * d), rr, accent);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        // Le tracé en cours
        if let Some(t) = &self.trace {
            for (i, p) in t.iter().enumerate() {
                let s = vue.ecran(zone, p[0], p[1]);
                if i > 0 {
                    let q = t[i - 1];
                    cx.dl.line(vue.ecran(zone, q[0], q[1]), s, 3.0 * d, accent);
                }
                let rr = 5.0 * d;
                cx.dl.rrect(Rect::new(s[0] - rr, s[1] - rr, 2.0 * rr, 2.0 * rr), rr, accent);
            }
            if self.outil == BATI && t.len() > 2 {
                let (a, b) = (t[t.len() - 1], t[0]);
                cx.dl.line(vue.ecran(zone, a[0], a[1]), vue.ecran(zone, b[0], b[1]), 1.5 * d, accent);
            }
        }
        // Le pinceau
        if matches!(self.outil, RELIEF | EAU | SOL) && self.trace.is_none() {
            if let Some(p) = self.doigt.filter(|p| zone.contains(p.0, p.1)) {
                let r = self.rayon * vue.echelle;
                cx.dl.secteur([p.0, p.1], (r - 1.5 * d).max(0.0), r + 1.5 * d, 0.0, std::f32::consts::TAU, 0.0, [255, 255, 255, 200]);
                cx.dl.secteur([p.0, p.1], (r - 0.5 * d).max(0.0), r + 0.5 * d, 0.0, std::f32::consts::TAU, 0.0, [0x20, 0x20, 0x28, 220]);
            }
        }
        // L'échelle, en bas à gauche
        let metres = 120.0 * d / vue.echelle;
        let rond = [1.0f32, 2.0, 5.0].iter().flat_map(|m| (0..7).map(move |e| m * 10f32.powi(e))).filter(|v| *v <= metres).fold(1.0, f32::max);
        let w = rond * vue.echelle;
        let (x0, y0) = (zone.x + 12.0 * d, zone.bottom() - 14.0 * d);
        cx.dl.rrect(Rect::new(x0 - 4.0 * d, y0 - 20.0 * d, w + 8.0 * d + 60.0 * d, 26.0 * d), 4.0 * d, [255, 255, 255, 190]);
        cx.dl.line([x0, y0], [x0 + w, y0], 2.0 * d, [0x20, 0x20, 0x28, 255]);
        let texte = if rond >= 1000.0 { format!("{} km", rond / 1000.0) } else { format!("{rond} m") };
        cx.text_at(x0 + w + 6.0 * d, y0 - 6.0 * d, &texte, 12.0, [0x20, 0x20, 0x28, 255]);
        cx.clip(None);
    }

    // ----- Le panneau

    fn puces(cx: &mut Cx, b: &mut crate::widgets::Body, base: Id, labels: &[&str], choisi: usize) -> Option<usize> {
        let (d, col) = (cx.dp, cx.couleurs());
        let h = cx.commande() * 0.8;
        let ecart = (cx.theme.marge * 0.6 * d).round();
        let (mut x, mut y) = (b.x, b.y);
        let mut rendu = None;
        for (i, l) in labels.iter().enumerate() {
            let w = cx.measure(l, 14.0) + 1.4 * cx.pad();
            if x + w > b.x + b.w && x > b.x {
                x = b.x;
                y += h + ecart;
            }
            let r = Rect::new(x, y, w, h);
            if cx.ui.click(sub_id(base, i), r, Some(b.id())) {
                rendu = Some(i);
            }
            let on = i == choisi;
            cx.dl.rrect(r, cx.theme.rayon(h), if on { col.accent } else if cx.ui.pressed(sub_id(base, i)) { col.ligne } else { col.puce });
            cx.centered(r, l, 14.0, if on { col.sur_accent } else { col.texte });
            x += w + ecart;
        }
        b.y = y + h + ecart * 2.0;
        rendu
    }

    fn panneau(&mut self, cx: &mut Cx, c: &mut Commun, r: Rect, base: Id) {
        let (d, col) = (cx.dp, cx.couleurs());
        cx.dl.rrect(r, 0.0, col.surface);
        // Les outils, en haut, fixes
        let h = cx.commande();
        let tete = Rect::new(r.x, r.y, r.w, h + 2.0 * cx.ecart());
        let mut tb = crate::widgets::Body::fixe(sub_id(base, 2), tete, r.x + 10.0 * d, r.w - 20.0 * d, r.y + cx.ecart());
        let avant = self.outil;
        if let Some(o) = Lieux::puces(cx, &mut tb, sub_id(base, 100), &OUTILS, self.outil) {
            self.outil = o;
        }
        let tete = Rect::new(r.x, r.y, r.w, tb.y - r.y);
        cx.dl.rrect(Rect::new(r.x, tete.bottom() - 1.0, r.w, 1.0), 0.0, col.ligne);
        if self.outil != avant {
            self.trace = None;
            self.trait_ = None;
            if !matches!((avant, self.outil), (BATI, BATI) | (VOIES, VOIES)) {
                self.choix = None;
            }
        }
        let corps = Rect::new(r.x, tete.bottom(), r.w, (r.bottom() - tete.bottom()).max(0.0));
        let mut b = cx.body(sub_id(base, 3), corps, &c.scrolls);
        let Some(carte) = &mut self.carte else {
            cx.body_end(b, &mut c.scrolls);
            return;
        };
        match self.outil {
            VOIR => {
                if let Some(info) = &self.info {
                    cx.section(&mut b, "Ici");
                    for l in info.clone() {
                        cx.paragraph(&mut b, &l);
                    }
                } else {
                    cx.paragraph(&mut b, "Touchez la carte pour savoir ce qu'il y a là. Deux doigts zooment.");
                }
                if let Ok(dn) = carte.donnees.read() {
                    let l = &dn.lieu;
                    cx.section(&mut b, "Le lieu");
                    cx.paragraph(&mut b, &format!("{} · centre {:.4}, {:.4}", l.nom, l.lat, l.lon));
                    cx.paragraph(&mut b, &format!("{} × {} points de {} m · eau à {:.1} m", l.cote, l.cote, l.pas, l.niveau).replace('.', ","));
                    cx.paragraph(&mut b, &format!("{} bâtiments, {} routes, {} ponts", l.batiments.len(), l.routes.len(), l.ponts.len()));
                }
                cx.section(&mut b, "Sources");
                cx.paragraph(&mut b, "Relief © IGN · eau, routes, bâtiments, sol © les contributeurs OpenStreetMap (ODbL). Un lieu généré en est dérivé : ne le publiez pas.");
            }
            BATI => self.panneau_bati(cx, &mut b, base),
            VOIES => self.panneau_voies(cx, &mut b, base),
            RELIEF => {
                cx.section(&mut b, "Pinceau");
                if let Some(i) = Lieux::puces(cx, &mut b, sub_id(base, 300), &PINCEAUX, self.pinceau) {
                    self.pinceau = i;
                }
                cx.slider(&mut b, sub_id(base, 310), "Rayon", &format!("{:.0} m", self.rayon), &mut self.rayon, 4.0, 400.0);
                cx.slider(&mut b, sub_id(base, 311), "Force", &format!("{:.1} m", self.force).replace('.', ","), &mut self.force, 0.1, 8.0);
                cx.paragraph(&mut b, "Un doigt peint, deux doigts déplacent la carte.");
            }
            EAU => {
                cx.section(&mut b, "Pinceau");
                let i = if self.ajouter_eau { 0 } else { 1 };
                if let Some(i) = Lieux::puces(cx, &mut b, sub_id(base, 400), &["Mettre de l'eau", "Retirer"], i) {
                    self.ajouter_eau = i == 0;
                }
                cx.slider(&mut b, sub_id(base, 410), "Rayon", &format!("{:.0} m", self.rayon), &mut self.rayon, 4.0, 400.0);
                cx.section(&mut b, "Niveau de l'eau");
                let (mut niveau, lo, hi) = match carte.donnees.read() {
                    Ok(dn) => (dn.lieu.niveau, dn.lieu.base - 10.0, dn.lieu.base + 65535.0 * dn.lieu.echelle),
                    Err(_) => (0.0, 0.0, 1.0),
                };
                let avant = niveau;
                cx.slider(&mut b, sub_id(base, 411), "Surface", &format!("{niveau:.1} m").replace('.', ","), &mut niveau, lo, hi.max(lo + 1.0));
                if (niveau - avant).abs() > 1e-4 {
                    if let Ok(mut dn) = carte.donnees.write() {
                        dn.lieu.niveau = niveau;
                    }
                    carte.tout_retoucher();
                    if !self.ouverte {
                        self.ouverte = true;
                        self.empiler(Retouche::Niveau(avant));
                    }
                    self.modifie = true;
                }
                cx.paragraph(&mut b, "Mettre de l'eau creuse le lit sous le niveau ; la retirer relève la berge au-dessus.");
            }
            _ => {
                cx.section(&mut b, "Occupation du sol");
                if let Some(i) = Lieux::puces(cx, &mut b, sub_id(base, 500), &CLASSES, self.classe) {
                    self.classe = i;
                }
                cx.slider(&mut b, sub_id(base, 510), "Rayon", &format!("{:.0} m", self.rayon), &mut self.rayon, 4.0, 400.0);
            }
        }
        cx.body_end(b, &mut c.scrolls);
    }

    /// Le tracé : terminer (s'il y a assez de points) ou abandonner.
    fn panneau_trace(&mut self, cx: &mut Cx, b: &mut crate::widgets::Body, base: Id, min: usize) -> Option<Vec<[f32; 2]>> {
        let n = self.trace.as_ref().map_or(0, |t| t.len());
        cx.paragraph(b, &format!("Touchez la carte pour poser les points ({n}). Deux doigts déplacent la carte."));
        let mut fini = None;
        if n >= min && cx.button(b, sub_id(base, 600), "Terminer") {
            fini = self.trace.take();
        }
        if n > 0 && cx.button(b, sub_id(base, 601), "Retirer le dernier point") {
            if let Some(t) = &mut self.trace {
                t.pop();
            }
        }
        if cx.button(b, sub_id(base, 602), "Abandonner") {
            self.trace = None;
        }
        fini
    }

    fn panneau_bati(&mut self, cx: &mut Cx, b: &mut crate::widgets::Body, base: Id) {
        if self.trace.is_some() {
            cx.section(b, "Nouveau bâtiment");
            if let Some(t) = self.panneau_trace(cx, b, base, 3) {
                let neuf = self.carte.as_ref().and_then(|c| c.donnees.read().ok().map(|d| (bati_neuf(&d.lieu, t), d.lieu.batiments.len())));
                if let Some((nb, k)) = neuf {
                    self.faire(Retouche::Insere(Liste::Bati, k, Elem::B(nb)));
                    self.choix = Some(Choix::Bati(k));
                }
            }
            return;
        }
        let Some(carte) = &self.carte else { return };
        let k = match self.choix {
            Some(Choix::Bati(k)) => k,
            _ => {
                cx.paragraph(b, "Touchez un bâtiment pour le choisir ; glissez-le pour le déplacer.");
                if cx.button(b, sub_id(base, 200), "Nouveau bâtiment") {
                    self.trace = Some(Vec::new());
                    self.choix = None;
                }
                return;
            }
        };
        let Some((avant, sol)) = carte.donnees.read().ok().and_then(|d| {
            let bt = d.lieu.batiments.get(k)?.clone();
            let s = bt.contour.iter().map(|p| d.lieu.relief(p[0] as f64, p[1] as f64) as f32).sum::<f32>() / bt.contour.len().max(1) as f32;
            Some((bt, s))
        }) else {
            self.choix = None;
            return;
        };
        let mut bt = avant.clone();
        cx.section(b, "Genre");
        if let Some(i) = Lieux::puces(cx, b, sub_id(base, 210), &GENRES_BATI, bt.genre as usize) {
            bt.genre = i as u8;
        }
        cx.section(b, "Hauteurs");
        let mut murs = bt.egout - sol;
        cx.slider(b, sub_id(base, 220), "Murs", &format!("{murs:.1} m").replace('.', ","), &mut murs, 2.0, 80.0);
        let mut toit = bt.toit - bt.egout;
        cx.slider(b, sub_id(base, 221), "Toit", &format!("{toit:.1} m").replace('.', ","), &mut toit, 0.0, 20.0);
        bt.egout = sol + murs;
        bt.toit = bt.egout + toit;
        cx.section(b, "Toit");
        if let Some(i) = Lieux::puces(cx, b, sub_id(base, 230), &FORMES, bt.forme as usize) {
            bt.forme = i as u8;
            if bt.forme == toits::PLAT {
                bt.toit = bt.egout;
            } else if bt.toit - bt.egout < 0.5 {
                bt.toit = bt.egout + 3.0;
            }
        }
        if cx.button(b, sub_id(base, 240), "Tourner le toit d'un quart") {
            bt.axe += std::f32::consts::FRAC_PI_2;
        }
        let supprimer = cx.button(b, sub_id(base, 250), "Supprimer le bâtiment");
        if cx.button(b, sub_id(base, 251), "Nouveau bâtiment") {
            self.trace = Some(Vec::new());
            self.choix = None;
        }
        if supprimer {
            self.choix = None;
            self.faire(Retouche::Retire(Liste::Bati, k));
            return;
        }
        if bt != avant {
            self.poser(Liste::Bati, k, Elem::B(bt), Elem::B(avant));
        }
    }

    /// Une retouche du panneau : appliquée tout de suite ; empilée une fois par geste.
    fn poser(&mut self, li: Liste, k: usize, neuf: Elem, avant: Elem) {
        let Some(carte) = &mut self.carte else { return };
        let z = if let Ok(mut d) = carte.donnees.write() {
            let (_, z, idx) = Lieux::appliquer(&mut d, Retouche::Pose(li, k, neuf));
            if idx {
                d.reindexer();
            }
            z
        } else {
            return;
        };
        carte.retoucher(z);
        if !self.ouverte {
            self.ouverte = true;
            self.empiler(Retouche::Pose(li, k, avant));
        }
        self.modifie = true;
    }

    fn panneau_voies(&mut self, cx: &mut Cx, b: &mut crate::widgets::Body, base: Id) {
        if self.trace.is_some() {
            cx.section(b, "Nouvelle voie");
            if let Some(t) = self.panneau_trace(cx, b, base, 2) {
                let neuf = self.carte.as_ref().and_then(|c| c.donnees.read().ok().map(|d| (voie_neuve(&d.lieu, &t, false, sage_reel::ROUTE, 6.0), d.lieu.routes.len())));
                if let Some((r, k)) = neuf {
                    self.faire(Retouche::Insere(Liste::Routes, k, Elem::R(r)));
                    self.choix = Some(Choix::Voie(false, k));
                }
            }
            return;
        }
        let Some(carte) = &self.carte else { return };
        let (pont, k) = match self.choix {
            Some(Choix::Voie(p, k)) => (p, k),
            _ => {
                cx.paragraph(b, "Touchez une route ou un pont pour le choisir ; tirez ses points pour le modifier.");
                if cx.button(b, sub_id(base, 260), "Nouvelle voie") {
                    self.trace = Some(Vec::new());
                    self.choix = None;
                }
                return;
            }
        };
        let li = if pont { Liste::Ponts } else { Liste::Routes };
        let Some(avant) = carte.donnees.read().ok().and_then(|d| if pont { d.lieu.ponts.get(k).cloned() } else { d.lieu.routes.get(k).cloned() }) else {
            self.choix = None;
            return;
        };
        let mut r = avant.clone();
        cx.section(b, "Genre");
        if let Some(i) = Lieux::puces(cx, b, sub_id(base, 270), &GENRES_VOIE, r.genre as usize) {
            r.genre = i as u8;
        }
        cx.slider(b, sub_id(base, 271), "Largeur", &format!("{:.1} m", r.largeur).replace('.', ","), &mut r.largeur, 1.0, 40.0);
        let mut est_pont = pont;
        cx.toggle(b, sub_id(base, 272), "Pont", &mut est_pont);
        let supprimer = cx.button(b, sub_id(base, 273), if pont { "Supprimer le pont" } else { "Supprimer la route" });
        if cx.button(b, sub_id(base, 274), "Nouvelle voie") {
            self.trace = Some(Vec::new());
            self.choix = None;
        }
        if supprimer {
            self.choix = None;
            self.faire(Retouche::Retire(li, k));
            return;
        }
        if est_pont != pont {
            // Passer d'une liste à l'autre : les hauteurs refaites (tablier droit ou chaussée
            // au sol)
            let Some((neuf, n)) = carte.donnees.read().ok().map(|d| {
                let t: Vec<[f32; 2]> = r.points.iter().map(|p| [p[0], p[1]]).collect();
                let mut v = voie_neuve(&d.lieu, &t, est_pont, r.genre, r.largeur);
                v.points = r.points.iter().zip(&v.points).map(|(a, b)| [a[0], a[1], b[2]]).collect();
                (v, if est_pont { d.lieu.ponts.len() } else { d.lieu.routes.len() })
            }) else {
                return;
            };
            let vers = if est_pont { Liste::Ponts } else { Liste::Routes };
            self.faire(Retouche::Groupe(vec![Retouche::Retire(li, k), Retouche::Insere(vers, n, Elem::R(neuf))]));
            self.choix = Some(Choix::Voie(est_pont, n));
            return;
        }
        if r != avant {
            self.poser(li, k, Elem::R(r), Elem::R(avant));
        }
    }

    // ----- L'accueil

    fn accueil(&mut self, cx: &mut Cx, c: &mut Commun, r: Rect, base: Id, k: usize) {
        let col = cx.couleurs();
        cx.dl.rrect(r, 0.0, col.fond);
        if self.liste.is_none() {
            self.lister(c);
        }
        let mut b = cx.body(sub_id(base, 4), r, &c.scrolls);
        if let Some((m, erreur)) = &self.message {
            let (m, e) = (m.clone(), *erreur);
            cx.section(&mut b, if e { "Erreur" } else { "Message" });
            cx.paragraph(&mut b, &m);
        }
        if let Some((p, _)) = &self.chargement {
            cx.section(&mut b, "Chargement");
            cx.paragraph(&mut b, &format!("{}…", p.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned())));
        }
        if let Some(g) = &self.generation {
            let (etape, part) = g.suivi.etat();
            cx.section(&mut b, "Génération");
            cx.paragraph(&mut b, &format!("{etape} — {:.0} %", part * 100.0));
            let barre = Rect::new(b.x, b.y, b.w, 8.0 * cx.dp);
            cx.dl.rrect(barre, 4.0 * cx.dp, col.ligne);
            cx.dl.rrect(Rect::new(barre.x, barre.y, barre.w * part, barre.h), 4.0 * cx.dp, col.accent);
            b.y += barre.h + 12.0 * cx.dp;
            for l in g.journal.iter().rev().take(6).rev() {
                let l = l.clone();
                cx.paragraph(&mut b, &l);
            }
            let arret = g.suivi.arrete();
            if !arret && cx.button(&mut b, sub_id(base, 700), "Arrêter") {
                g.suivi.arreter();
            }
            if arret {
                cx.paragraph(&mut b, "arrêt demandé : les téléchargements en cours finissent…");
            }
        }
        cx.section(&mut b, "Lieux");
        let liste = self.liste.clone().unwrap_or_default();
        if liste.is_empty() {
            let ou = Lieux::dossier(c).map_or("Flow/lieux".to_string(), |d| d.display().to_string());
            cx.paragraph(&mut b, &format!("Aucun lieu dans {ou}. Générez-en un ci-dessous, ou ouvrez un .reel depuis Fichiers."));
        }
        for (i, (p, t)) in liste.iter().enumerate() {
            let nom = p.file_stem().map_or(String::new(), |n| n.to_string_lossy().into_owned());
            if super::ligne_fichier(cx, &mut b, sub_id(base, 800 + i), super::Puce::Icone(Icon::Carte), &nom, &taille(*t), false) && self.chargement.is_none() {
                self.charger(p.clone());
            }
        }
        if self.generation.is_none() {
            cx.section(&mut b, "Nouveau lieu");
            clavier::champ(cx, &mut b, sub_id(NOM, k), "Nom du lieu", &mut self.nom, &mut c.clavier);
            clavier::champ(cx, &mut b, sub_id(POSITION, k), "Position : latitude, longitude", &mut self.position, &mut c.clavier);
            let mut pas = self.pas;
            cx.slider(&mut b, sub_id(base, 710), "Pas de la grille", &format!("{pas:.0} m"), &mut pas, 2.0, 16.0);
            self.pas = pas.round();
            let (mut lx, mut lz) = (self.largeur, self.longueur);
            cx.slider(&mut b, sub_id(base, 712), "Largeur (est-ouest)", &format!("{lx:.1} km").replace('.', ","), &mut lx, 1.0, 20.0);
            cx.slider(&mut b, sub_id(base, 713), "Longueur (nord-sud)", &format!("{lz:.1} km").replace('.', ","), &mut lz, 1.0, 20.0);
            (self.largeur, self.longueur) = ((lx * 2.0).round() / 2.0, (lz * 2.0).round() / 2.0);
            cx.toggle(&mut b, sub_id(base, 711), "Toits mesurés au LiDAR", &mut self.toits);
            let n = sage_reel::cote(self.pas as f64);
            let [a0, a1, b0, b1] = sage_reel::generer::emprise(self.pas as f64, self.emprise_m().0, self.emprise_m().1);
            let (na, nb) = (a1 - a0 + 1, b1 - b0 + 1);
            let zone = if (na, nb) == (n, n) { "La carte entière, 20 km de côté".to_string() } else { format!("Une zone de {} × {} km au centre de la carte de 20 km (autour, le relief se prolonge à plat, sans rien dessus)", self.largeur, self.longueur).replace('.', ",") };
            cx.paragraph(&mut b, &format!("{zone} : {na} × {nb} points téléchargés. Le relief et le LiDAR viennent de l'IGN, le reste d'OpenStreetMap ; les toits demandent ~0,4 Go de téléchargements dans une ville. Relancer reprend ce qui est déjà téléchargé."));
            if cx.button(&mut b, sub_id(base, 712), "Générer") {
                c.clavier.fermer();
                self.generer(c);
            }
            if cx.button(&mut b, sub_id(base, 713), "Créer un lieu vierge") {
                c.clavier.fermer();
                self.creer_vierge(c);
            }
        }
        cx.body_end(b, &mut c.scrolls);
    }

    // ----- La barre de la fenêtre

    fn barre(&mut self, cx: &mut Cx, c: &mut Commun, p: &Placee) {
        let (d, col) = (cx.dp, cx.couleurs());
        let e = p.etat;
        let k = p.id as usize;
        let h = e.h;
        let mut x = e.right();
        let bouton = |cx: &mut Cx, x: f32, n: usize, icone: Icon, actif: bool, teinte: Color| -> bool {
            let case = Rect::new(x, e.y, h, h);
            let kb = sub_id(LIEUX, 10_000 + k * 8 + n);
            let clic = cx.ui.click(kb, case, None);
            if actif && cx.ui.pressed(kb) {
                cx.dl.rrect(case.inset(3.0 * d), cx.theme.rayon(case.h - 6.0 * d), col.puce);
            }
            cx.icon(case.inset(h * 0.24), icone, if actif { teinte } else { col.ligne });
            clic && actif
        };
        if self.carte.is_some() {
            x -= h;
            if bouton(cx, x, 0, Icon::Retablir, !self.futur.is_empty(), col.texte) {
                self.defaire(true);
            }
            x -= h;
            if bouton(cx, x, 1, Icon::Annuler, !self.passe.is_empty(), col.texte) {
                self.defaire(false);
            }
            x -= h;
            if bouton(cx, x, 2, Icon::Disquette, self.chemin.is_some() && self.enregistrement.is_none(), if self.modifie { col.accent } else { col.pale }) {
                self.enregistrer();
            }
            x -= h;
            if bouton(cx, x, 3, Icon::List, true, col.texte) {
                if self.modifie {
                    self.message = Some(("des retouches ne sont pas enregistrées : enregistrez, ou touchez encore la liste".into(), true));
                    self.modifie = false;
                } else {
                    self.fermer_lieu();
                    c.clavier.fermer();
                }
            }
        }
        // Le nom
        let nom = match (&self.carte, &self.chemin) {
            (Some(ca), _) => ca.donnees.read().map(|d| d.lieu.nom.clone()).unwrap_or_default(),
            _ => "Lieux".into(),
        };
        let nom = format!("{nom}{}", if self.modifie { "*" } else { "" });
        let place = (x - e.x - 4.0 * d).max(0.0);
        let wn = cx.measure(&nom, 14.0).min(place);
        let xn = x - wn - 2.0 * d;
        cx.clip(Some(Rect::new(xn, e.y, wn, h)));
        cx.text_at(xn, e.cy(), &nom, 14.0, if self.modifie { col.accent } else { col.texte });
        cx.clip(None);
        // Le message, ou l'état
        let texte = match (&self.message, &self.carte) {
            (Some((m, _)), Some(_)) => m.clone(),
            (_, Some(carte)) => {
                let z = self.vue.map_or(0, |v| carte.niveau(v.echelle));
                let attente = if carte.en_attente() { " · dessin…" } else { "" };
                format!("{} · niveau {z}{attente}", OUTILS[self.outil])
            }
            _ => String::new(),
        };
        let erreur = self.message.as_ref().is_some_and(|m| m.1);
        cx.clip(Some(Rect::new(e.x, e.y, (xn - e.x - 12.0 * d).max(0.0), h)));
        cx.mono_at(e.x + 4.0 * d, e.cy(), &texte, 12.0, if erreur { col.accent } else { col.pale });
        cx.clip(None);
    }
}

impl Contexte for Lieux {
    fn cle(&self) -> &'static str {
        CLE
    }

    fn dupliquer(&self) -> Box<dyn Contexte> {
        // Un lieu pèse lourd : la copie s'ouvre sur la liste
        Box::new(Lieux::default())
    }

    fn document(&self) -> Option<&Path> {
        self.chemin.as_deref()
    }

    fn ecrire(&self, e: &mut Etat) {
        if self.carte.is_some() || self.chargement.is_some() {
            if let Some(p) = &self.chemin {
                e.mettre("document", p.display());
            }
            if let Some(v) = self.vue {
                e.mettre("vue", format!("{} {} {}", v.cx, v.cz, v.echelle));
            }
        }
        e.mettre("outil", self.outil);
    }

    fn anime(&self, _c: &Commun) -> bool {
        self.carte.as_ref().is_some_and(|c| c.en_attente()) || self.generation.is_some() || self.chargement.is_some() || self.enregistrement.is_some()
    }

    fn frappe(&mut self, f: FenetreId, cible: Id, frappe: Frappe, c: &mut Commun) -> bool {
        let k = f as usize;
        let ed = if cible == sub_id(NOM, k) {
            &mut self.nom
        } else if cible == sub_id(POSITION, k) {
            &mut self.position
        } else {
            return false;
        };
        if ed.appliquer(frappe) {
            c.clavier.fermer();
        }
        self.message = None;
        true
    }

    fn dessiner(&mut self, cx: &mut Cx, c: &mut Commun, p: &Placee) -> String {
        let k = p.id as usize;
        let base = sub_id(LIEUX, k);
        self.dp = cx.dp;
        // Un document à ouvrir (relu de l'espace, ou venu de Fichiers)
        if self.carte.is_none() && self.chargement.is_none() {
            if let Some(ch) = self.chemin.clone() {
                let vue = self.vue;
                self.charger(ch);
                self.vue = vue;
            }
        }
        self.suivre();
        if cx.ui.doigts() == 0 {
            self.ouverte = false;
        }
        let r = p.contenu;
        if self.carte.is_some() {
            // Le panneau à droite en paysage, dessous en portrait
            let d = cx.dp;
            let (zone, panneau) = if r.w > r.h * 1.1 {
                let w = (340.0 * d).min(r.w * 0.42);
                (Rect::new(r.x, r.y, r.w - w, r.h), Rect::new(r.right() - w, r.y, w, r.h))
            } else {
                let h = (r.h * 0.40).max(160.0 * d).min(r.h * 0.6);
                (Rect::new(r.x, r.y, r.w, r.h - h), Rect::new(r.x, r.bottom() - h, r.w, h))
            };
            self.zone = zone;
            self.dessiner_carte(cx, zone, base);
            self.panneau(cx, c, panneau, base);
        } else {
            self.accueil(cx, c, r, base, k);
        }
        self.barre(cx, c, p);
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sage_reel::cote;

    #[test]
    fn la_position_se_lit_de_plusieurs_facons() {
        assert_eq!(lire_position("49.2464, 1.4119"), Some((49.2464, 1.4119)));
        assert_eq!(lire_position("49,2464 1,4119"), Some((49.2464, 1.4119)));
        assert_eq!(lire_position("49.2464;1.4119"), Some((49.2464, 1.4119)));
        assert_eq!(lire_position("49.2464 1.4119"), Some((49.2464, 1.4119)));
        assert_eq!(lire_position("-12.5,-77.1"), Some((-12.5, -77.1)));
        assert_eq!(lire_position("bonjour"), None);
        assert_eq!(lire_position("95, 2"), None);
        assert_eq!(nom_de_fichier("Les Andelys (Eure)"), "les-andelys-eure");
        assert_eq!(nom_de_fichier("  "), "lieu");
    }

    fn pret() -> Lieux {
        let mut l = Lieux::default();
        let pas = 64.0;
        let n = cote(pas);
        let lieu = Lieu::new("Essai", 49.0, 1.0, 0.0, 0.0, pas, 0.0, &vec![10.0; n * n], &vec![false; n * n], Vec::new(), Vec::new(), Vec::new());
        l.ouvrir_lieu(lieu, None, false);
        l
    }

    fn lieu(l: &Lieux) -> Lieu {
        l.carte.as_ref().unwrap().donnees.read().unwrap().lieu.clone()
    }

    #[test]
    fn un_batiment_se_trace_se_retouche_et_s_annule() {
        let mut l = pret();
        let contour = vec![[0.0, 0.0], [20.0, 0.0], [20.0, 12.0], [0.0, 12.0]];
        let nb = bati_neuf(&lieu(&l), contour);
        assert!((nb.egout - 16.0).abs() < 0.01 && nb.toit > nb.egout && nb.pied < 10.0);
        l.faire(Retouche::Insere(Liste::Bati, 0, Elem::B(nb.clone())));
        assert_eq!(lieu(&l).batiments.len(), 1);
        {
            let c = l.carte.as_ref().unwrap();
            let d = c.donnees.read().unwrap();
            assert_eq!(Lieux::bati_sous(&d, 10.0, 6.0), Some(0));
            assert_eq!(Lieux::bati_sous(&d, 30.0, 6.0), None);
        }
        // Une retouche du panneau : une seule entrée pour tout le geste
        let mut b2 = nb.clone();
        b2.genre = sage_reel::IMMEUBLE;
        l.poser(Liste::Bati, 0, Elem::B(b2.clone()), Elem::B(nb.clone()));
        let mut b3 = b2.clone();
        b3.egout += 3.0;
        l.poser(Liste::Bati, 0, Elem::B(b3.clone()), Elem::B(b2));
        assert_eq!(l.passe.len(), 2);
        assert_eq!(lieu(&l).batiments[0], b3);
        l.defaire(false);
        assert_eq!(lieu(&l).batiments[0], nb);
        l.defaire(false);
        assert!(lieu(&l).batiments.is_empty());
        l.defaire(true);
        l.defaire(true);
        assert_eq!(lieu(&l).batiments[0], b3);
    }

    #[test]
    fn le_pinceau_monte_le_relief_et_s_annule_d_un_coup() {
        let mut l = pret();
        l.outil = RELIEF;
        l.rayon = 200.0;
        l.force = 2.0;
        l.trait_ = Some(Trait::Peint { dernier: (0.0, 0.0), cible: 0.0, vus: HashSet::new(), retouche: Retouche::Grille { relief: Vec::new(), eau: Vec::new(), sol: Vec::new() } });
        l.peindre(0.0, 0.0);
        l.peindre(30.0, 0.0);
        l.finir_trait();
        let h = lieu(&l).relief(10.0, 0.0);
        assert!(h > 12.0, "{h}");
        assert_eq!(l.passe.len(), 1);
        l.defaire(false);
        assert!((lieu(&l).relief(10.0, 0.0) - 10.0).abs() < 0.02);
        // L'eau : posée, le lit creusé sous le niveau
        l.outil = EAU;
        l.rayon = 100.0;
        l.trait_ = Some(Trait::Peint { dernier: (0.0, 0.0), cible: 0.0, vus: HashSet::new(), retouche: Retouche::Grille { relief: Vec::new(), eau: Vec::new(), sol: Vec::new() } });
        l.peindre(0.0, 0.0);
        l.finir_trait();
        let li = lieu(&l);
        assert!(li.eau(0.0, 0.0) > 0.5 && li.relief(0.0, 0.0) < 0.0);
        l.defaire(false);
        assert!(lieu(&l).eau(0.0, 0.0) < 0.5);
    }

    #[test]
    fn une_voie_tracee_suit_le_sol_et_devient_un_pont() {
        let mut l = pret();
        let r = voie_neuve(&lieu(&l), &[[0.0, 0.0], [100.0, 0.0]], false, sage_reel::ROUTE, 6.0);
        assert!(r.points.len() >= 13 && r.points.iter().all(|p| (p[2] - 10.0).abs() < 0.01));
        l.faire(Retouche::Insere(Liste::Routes, 0, Elem::R(r.clone())));
        {
            let c = l.carte.as_ref().unwrap();
            let d = c.donnees.read().unwrap();
            assert_eq!(Lieux::voie_pres(&d, 50.0, 4.0, 5.0), Some((false, 0)));
        }
        l.faire(Retouche::Groupe(vec![Retouche::Retire(Liste::Routes, 0), Retouche::Insere(Liste::Ponts, 0, Elem::R(r.clone()))]));
        let li = lieu(&l);
        assert!(li.routes.is_empty() && li.ponts.len() == 1);
        l.defaire(false);
        let li = lieu(&l);
        assert_eq!((li.routes.len(), li.ponts.len()), (1, 0));
    }
}
