//! Le contexte Lieux : générer, explorer et éditer les lieux réels de S.A.G.E. (`.reel`).
//!
//! - **Sans lieu ouvert** : la liste des lieux (`Flow/lieux/`), et la génération d'un
//!   nouveau : un nom, une position (« latitude, longitude »), le pas de la grille, l'emprise
//!   (largeur × longueur en km, jusqu'à la carte entière de 20 km), les
//!   toits mesurés au LiDAR ou non. « Choisir sur la carte » ouvre un plan minimaliste
//!   (`crate::plan`, tuiles d'OpenFreeMap) avec le cadre de l'emprise au centre : on y
//!   glisse la carte, on cherche un nom (Nominatim), et le centre devient la position. Flow télécharge lui-même (le relief et le LiDAR de
//!   l'IGN, OpenStreetMap par Overpass) et suit l'avancement ; on peut arrêter, et
//!   relancer reprend ce qui est déjà téléchargé (`<lieu>.reel.cache/`). Un lieu
//!   vierge (plat, sans rien) se crée aussi.
//! - **Un lieu ouvert** : la carte (`carte`), en tuiles dessinées à la demande ; deux
//!   doigts zooment et déplacent ; à côté (en paysage) ou dessous (en portrait), le
//!   panneau des outils :
//!   - **Voir** : toucher la carte dit ce qu'il y a là (position, altitude, sol, voie,
//!     tunnel, voie ferrée, arrêt et ses lignes, bâtiment) ; un doigt déplace la carte.
//!     Le panneau liste les lignes (bus, tram, métro, train) : en choisir une la
//!     surligne sur la carte, avec ses arrêts.
//!   - **Bâti** : toucher un bâtiment le choisit, le glisser le déplace ; son genre, ses
//!     hauteurs, son toit se règlent dans le panneau ; « Nouveau bâtiment » se trace
//!     point par point.
//!   - **Voies** : routes et ponts ; toucher en choisit une, ses points se tirent ; genre,
//!     largeur, pont ou non ; « Nouvelle voie » se trace point par point.
//!   - **Relief** : un pinceau qui monte, creuse, lisse ou aplanit.
//!   - **Eau** : un pinceau qui met de l'eau (en creusant le lit sous le niveau) ou en
//!     retire ; le niveau de l'eau.
//!   - **Sol** : un pinceau d'occupation du sol (bois, prés, champs, habité, zones).
//! - **La 3D** (`relief3d`) : le cube de la barre fait passer de la carte au relief en
//!   3D, drapé des mêmes tuiles, les bâtiments en volume près de l'œil, et retour, sur
//!   le même point ; un doigt tourne autour,
//!   deux doigts rapprochent et déplacent. Un outil d'édition ramène à la carte. Deux
//!   pilules, en haut à gauche, choisissent le rendu : drapé de la carte ou Maquette ;
//!   Blanc (faces blanches, arêtes tracées) s'ajoute à l'un comme à l'autre (retenu dans
//!   l'espace).
//! - **Dans la barre de la fenêtre** : la liste des lieux, la 3D, le nom, enregistrer,
//!   annuler et rétablir.
//!
//! Un lieu couvre un rectangle de la carte de S.A.G.E. (`Grille`, format 7) : la carte et
//! la 3D ne montrent que lui.
//!
//! Un lieu généré est dérivé d'OpenStreetMap (ODbL) : il ne se publie pas.

use crate::carte::{boite_bati, boite_route, boite_tuile, Carte, Donnees, Vue};
use crate::clavier::{self, Edition, Frappe};
use crate::contexte::{Commun, Contexte, Etat, Genre};
use crate::draw::{Color, Rect};
use crate::gestes::Geste;
use crate::surface::{FenetreId, Placee};
use crate::ui::{id, sub_id, Id};
use crate::relief3d::Relief3d;
use crate::sculpture::Camera;
use crate::widgets::{Cx, Icon};
use sage_reel::generer::{self, Demande, Reseau, Suivi};
use sage_reel::{origine, toits, Batiment, Lieu, Route};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread::JoinHandle;

pub const CLE: &str = "lieux";
pub const GENRE: Genre = Genre { cle: CLE, nom: "Lieux", legende: "Générer, explorer et éditer les lieux réels de S.A.G.E.", icone: Icon::Carte, ouvrir, applet: None };

const LIEUX: Id = id("lieux");
const NOM: Id = id("lieux/nom");
const POSITION: Id = id("lieux/position");
const RECHERCHE: Id = id("lieux/recherche");
/// Le dossier des lieux, dans celui de Flow.
pub const DOSSIER: &str = "lieux";
const HISTORIQUE: usize = 60;

const OUTILS: [&str; 7] = ["Voir", "Bâti", "Voies", "Relief", "Eau", "Sol", "Sélection"];
const VOIR: usize = 0;
const BATI: usize = 1;
const VOIES: usize = 2;
const RELIEF: usize = 3;
const EAU: usize = 4;
const SOL: usize = 5;
/// La sélection de tuiles de la maquette (en 3D, comme Voir).
const SELECTION: usize = 6;

const GENRES_BATI: [&str; 5] = ["Maison", "Immeuble", "Industriel", "Édifice", "Annexe"];
const FORMES: [&str; 4] = ["Plat", "Deux pans", "Croupe", "Appentis"];
const GENRES_VOIE: [&str; sage_reel::GENRES as usize] = sage_reel::NOMS_GENRES;
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
    /// La dernière image où une manette a mené la 3D.
    manette_vu: Option<std::time::Instant>,
    passe: Vec<Retouche>,
    futur: Vec<Retouche>,
    /// Une retouche du panneau en cours (un curseur tiré) : elle ne s'empile qu'une fois.
    ouverte: bool,
    info: Option<Vec<String>>,
    /// Le dernier doigt sur la carte (le cercle du pinceau).
    doigt: Option<(f32, f32)>,
    zone: Rect,
    dp: f32,
    /// La vue 3D, quand elle remplace la carte ; sa caméra relue de l'espace, en
    /// attendant le lieu ; le doigt qui la fait tourner.
    trois_d: Option<Relief3d>,
    camera3d: Option<Camera>,
    /// Le rendu de la 3D : drapé de la carte, ou blanc aux arêtes tracées.
    blanc3d: bool,
    /// En maquette : les volumes de défonce des lits montrés.
    defonce3d: bool,
    /// Le rendu Maquette de la 3D (`maquette`).
    maquette3d: bool,
    /// En maquette : le suivi des tâches (`suivi`) par-dessus la vue.
    suivi3d: bool,
    /// En maquette : les tracés du réseau et les remblais des voies en surimpression.
    sur: crate::relief3d::Surimpression,
    /// La vue Matérialisation (une maquette à part : le niveau le plus fin seul, dans
    /// un rayon, montré quand tout est fait) : on y est ou non, son rayon (m), son
    /// centre (x, z), posé au point visé quand on y entre.
    materialisation3d: bool,
    zone_rayon: f32,
    /// En Matérialisation : les bâtiments en volumes fermés en surimpression.
    bati_mat: bool,
    zone_centre: Option<[f32; 2]>,
    /// Les tuiles sélectionnées de la maquette, et leur résumé (refait quand elles ou
    /// la carte changent).
    selection: crate::selection::Selection,
    resume: Option<(std::collections::BTreeSet<crate::carte::Cle>, u64, crate::selection::Resume)>,
    orbite: Option<(f32, f32)>,
    /// Le plan où choisir la zone à générer, quand il est ouvert ; la recherche d'un
    /// nom (Nominatim), en cours ou faite.
    plan: Option<crate::plan::Plan>,
    recherche: Edition,
    cherche: Option<JoinHandle<Result<Vec<(String, f64, f64)>, String>>>,
    resultats: Vec<(String, f64, f64)>,
    /// Le plan à rouvrir (relu de l'espace) au premier dessin.
    plan_a_ouvrir: bool,
    /// L'ordre de dessin des couches de la carte, et la couche choisie pour le changer.
    ordre: Vec<crate::carte::Couche>,
    couche: Option<crate::carte::Couche>,
    /// La ligne surlignée : (mode, numéro).
    ligne: Option<(u8, String)>,
}

/// Cherche un lieu par son nom (Nominatim, en France : le relief vient de l'IGN) :
/// (nom complet, latitude, longitude).
fn chercher(texte: &str, cache: &Path) -> Result<Vec<(String, f64, f64)>, String> {
    let url = format!("https://nominatim.openstreetmap.org/search?format=jsonv2&limit=6&countrycodes=fr&accept-language=fr&q={}", generer::encoder(texte));
    let f = cache.join("recherche.json");
    let _ = std::fs::create_dir_all(cache);
    ReseauFlow.telecharger(&url, None, &f)?;
    let t = std::fs::read_to_string(&f).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&f);
    let j = sage_reel::json::lire(&t).ok_or("réponse illisible")?;
    let num = |v: Option<&sage_reel::json::J>| v.and_then(|v| v.txt().and_then(|t| t.parse::<f64>().ok()).or_else(|| v.num()));
    Ok(j.liste().unwrap_or(&[]).iter().filter_map(|e| Some((e.get("display_name")?.txt()?.to_string(), num(e.get("lat"))?, num(e.get("lon"))?))).collect())
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
            manette_vu: None,
            passe: Vec::new(),
            futur: Vec::new(),
            ouverte: false,
            info: None,
            doigt: None,
            zone: Rect::default(),
            dp: 1.0,
            trois_d: None,
            camera3d: None,
            blanc3d: false,
            defonce3d: false,
            suivi3d: false,
            sur: Default::default(),
            materialisation3d: false,
            zone_rayon: 500.0,
            bati_mat: false,
            zone_centre: None,
            maquette3d: false,
            selection: Default::default(),
            resume: None,
            orbite: None,
            plan: None,
            recherche: Edition::default(),
            cherche: None,
            resultats: Vec::new(),
            plan_a_ouvrir: false,
            ordre: crate::carte::ordre_par_defaut(),
            couche: None,
            ligne: None,
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
        if let [la, t, d, x, y, z] = e.nombres("vue3d")[..] {
            l.camera3d = Some(Camera { lacet: la, tangage: t, distance: d, cible: [x, y, z] });
        }
    }
    l.outil = (e.nombre("outil").unwrap_or(0.0) as usize).min(OUTILS.len() - 1);
    // La position d'un lieu à générer ; le plan ouvert dessus
    if let Some(p) = e.texte("position") {
        l.position = Edition::new(p);
    }
    l.plan_a_ouvrir = e.texte("plan") == Some("oui");
    // « rendu3d = maquette blanc » : les deux, dans n'importe quel ordre
    let rendu = e.texte("rendu3d").unwrap_or("");
    l.blanc3d = rendu.split_whitespace().any(|m| m == "blanc");
    l.maquette3d = rendu.split_whitespace().any(|m| m == "maquette");
    l.defonce3d = rendu.split_whitespace().any(|m| m == "defonce");
    l.suivi3d = rendu.split_whitespace().any(|m| m == "suivi");
    l.materialisation3d = l.maquette3d && rendu.split_whitespace().any(|m| m == "materialisation");
    if let Some(s) = crate::relief3d::Surimpression::lire(&e.nombres("surimpression")) {
        l.sur = s;
    }
    l.bati_mat = e.texte("materialisation_batiments") == Some("oui");
    // « zone = rayon x z » : la zone de la Matérialisation
    if let [r, x, z] = e.nombres("zone")[..] {
        l.zone_rayon = r;
        l.zone_centre = Some([x, z]);
    } else if let Some(r) = e.nombre("zone_rayon") {
        l.zone_rayon = r;
    }
    // Les tuiles sélectionnées : « selection = niveau/i/j … »
    l.selection = crate::selection::Selection::lire(e.texte("selection").unwrap_or(""), e.texte("selection_seules") == Some("oui"));
    if let Some(o) = e.texte("ordre") {
        l.ordre = crate::carte::ordre_lu(o);
    }
    // La ligne surlignée : « mode numéro »
    if let Some((m, n)) = e.texte("ligne").and_then(|t| t.split_once(' ')) {
        l.ligne = m.parse().ok().map(|m| (m, n.to_string()));
    }
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

    /// La grille du lieu à générer (ou du lieu vierge), au pas `pas`.
    fn grille_au(&self, pas: f64) -> sage_reel::Grille {
        let (x, z) = self.emprise_m();
        sage_reel::Grille::emprise(pas, x, z)
    }

    fn grille(&self) -> sage_reel::Grille {
        self.grille_au(self.pas as f64)
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
        let fil = std::thread::Builder::new().name("chargement".into()).spawn(move || {
            let mut t = crate::suivi::tache("lecture du lieu");
            t.detail(f.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned()));
            Lieu::lire(&f)
        });
        let Ok(fil) = fil else { return };
        self.chargement = Some((chemin, fil));
        self.message = None;
    }

    /// Ouvre un lieu tout prêt (lu, généré, vierge).
    fn ouvrir_lieu(&mut self, lieu: Lieu, chemin: Option<PathBuf>, modifie: bool) {
        // Un autre ordre de dessin : les tuiles déjà parties se refont
        let mut carte = {
            let _t = crate::suivi::tache("ouverture du lieu (index)");
            Carte::new(lieu)
        };
        if self.ordre != crate::carte::ordre_par_defaut() {
            if let Ok(mut d) = carte.donnees.write() {
                d.ordre = self.ordre.clone();
            }
            carte.tout_retoucher();
        }
        self.carte = Some(carte);
        self.trois_d = None;
        if self.chemin != chemin {
            self.vue = None;
            self.camera3d = None;
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
        self.trois_d = None;
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
        let pas = self.pas.max(4.0) as f64;
        self.ouvrir_lieu(Lieu::vierge_sur(&nom, lat, lon, pas, self.grille_au(pas), 100.0), chemin, true);
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
                // Les indices sont ceux des tableaux du lieu (son rectangle de la carte)
                let g = l.grille;
                let (mx, mz) = g.sol();
                let (o, p) = (origine() as f32, l.pas as f32);
                let mut z = vide;
                let mut etendre = |(a, b): (i64, i64), pas: f32| {
                    let (a, b) = (a as f32 * pas + o, b as f32 * pas + o);
                    z = union(z, [a - pas, b - pas, a + pas, b + pas]);
                };
                let mut r2 = Vec::with_capacity(relief.len());
                for &(i, _) in &relief {
                    let (a, b) = g.point(i as usize);
                    r2.push((i, l.h_point(a, b)));
                    etendre((a, b), p);
                }
                l.poser_hauteurs(&relief.iter().map(|&(i, v)| (i as usize, v)).collect::<Vec<_>>());
                let mut e2 = Vec::with_capacity(eau.len());
                for &(i, v) in &eau {
                    let (a, b) = g.point(i as usize);
                    e2.push((i, l.est_eau(a, b)));
                    l.poser_eau(a, b, v);
                    etendre((a, b), p);
                }
                let mut s2 = Vec::with_capacity(sol.len());
                if !sol.is_empty() && l.sol.len() != mx * mz {
                    l.sol = vec![0; mx * mz];
                }
                for &(i, v) in &sol {
                    if let Some(x) = l.sol.get_mut(i as usize) {
                        s2.push((i, *x));
                        *x = v;
                        let (a, b) = ((g.a0 / 2 + i as usize % mx) as i64, (g.b0 / 2 + i as usize / mx) as i64);
                        etendre((a, b), p * 2.0);
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
        // Le lieu entier, au centre
        let e = self.carte.as_ref().and_then(|c| c.donnees.read().ok().map(|d| d.lieu.etendue())).unwrap_or([-10000.0, -10000.0, 10000.0, 10000.0]);
        let (w, h) = ((e[2] - e[0]) as f32, (e[3] - e[1]) as f32);
        Vue { cx: (e[0] + e[2]) / 2.0, cz: (e[1] + e[3]) / 2.0, echelle: (zone.w / w).min(zone.h / h) }
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
        // Le cours d'eau le plus proche : dans l'eau, celui dont c'est la surface (à
        // 300 m de son axe au plus) ; ailleurs, un ruisseau tout près
        let portee = if l.eau(x as f64, z as f64) >= 0.5 { 300.0 } else { 8.0 };
        let proche = d
            .cours_eau_dans([x - portee, z - portee, x + portee, z + portee])
            .into_iter()
            .map(|k| (l.cours_eau[k as usize].points.windows(2).map(|s| distance_segment((x, z), s[0], s[1])).fold(f32::MAX, f32::min), k))
            .filter(|(e, _)| *e < portee)
            .min_by(|a, b| a.0.total_cmp(&b.0));
        if let Some((e, k)) = proche {
            let c = &l.cours_eau[k as usize];
            let mut t = sage_reel::cours_eau::NOMS[(c.genre as usize).min(3)].to_string();
            if !c.nom.is_empty() {
                t = format!("{t} · {}", c.nom);
            }
            if c.largeur > 0.0 {
                t = format!("{t} · {:.1} m de large (OSM)", c.largeur).replace('.', ",");
            }
            if c.souterrain {
                t.push_str(" · sous terre");
            }
            v.push(format!("{t} · axe à {e:.0} m"));
        }
        if let Some(k) = Lieux::bati_sous(d, x, z) {
            let b = &l.batiments[k];
            v.push(format!("{} · {} · faîtage à {:.1} m", GENRES_BATI[(b.genre as usize).min(4)], FORMES[(b.forme as usize).min(3)], b.toit - b.pied - 1.0).replace('.', ","));
        }
        if let Some((pont, k)) = Lieux::voie_pres(d, x, z, 6.0) {
            let r = if pont { &l.ponts[k] } else { &l.routes[k] };
            v.push(format!("{}{} · {:.1} m de large", GENRES_VOIE[(r.genre as usize).min(GENRES_VOIE.len() - 1)], if pont { " (pont)" } else { "" }, r.largeur).replace('.', ","));
        }
        v.extend(Lieux::decrire_transports(d, x, z));
        v
    }

    /// Les tunnels, voies ferrées et arrêts près de (x, z).
    fn decrire_transports(d: &Donnees, x: f32, z: f32) -> Vec<String> {
        use sage_reel::transports as tr;
        let (l, t) = (&d.lieu, &d.lieu.transports);
        let mut v = Vec::new();
        let pres = |pts: &mut dyn Iterator<Item = [f32; 2]>| {
            let pts: Vec<[f32; 2]> = pts.collect();
            pts.windows(2).map(|s| distance_segment((x, z), s[0], s[1])).fold(f32::MAX, f32::min)
        };
        let marge = 8.0;
        let boite = [x - marge, z - marge, x + marge, z + marge];
        let sol = l.relief(x as f64, z as f64) as f32;
        if let Some((_, k)) = d.tunnels_dans(boite).into_iter().map(|k| (pres(&mut t.tunnels[k as usize].points.iter().map(|p| [p[0], p[1]])) - t.tunnels[k as usize].largeur / 2.0, k)).filter(|(e, _)| *e < 4.0).min_by(|a, b| a.0.total_cmp(&b.0)) {
            let r = &t.tunnels[k as usize];
            let y = r.points.iter().min_by(|a, b| (a[0] - x).hypot(a[1] - z).total_cmp(&(b[0] - x).hypot(b[1] - z))).map_or(sol, |p| p[2]);
            v.push(format!("tunnel · {} · {:.1} m sous le sol", GENRES_VOIE[(r.genre as usize).min(GENRES_VOIE.len() - 1)], (sol - y).max(0.0)).replace('.', ","));
        }
        if let Some((_, k)) = d.rails_dans(boite).into_iter().map(|k| (pres(&mut t.rails[k as usize].points.iter().map(|p| [p[0], p[1]])), k)).filter(|(e, _)| *e < 5.0).min_by(|a, b| a.0.total_cmp(&b.0)) {
            let r = &t.rails[k as usize];
            let ou = match r.ou {
                tr::PONT => " (pont)",
                tr::TUNNEL => " (tunnel)",
                _ => "",
            };
            let niveau = if r.niveau != 0 { format!(" · niveau {}", r.niveau) } else { String::new() };
            v.push(format!("voie ferrée · {}{ou}{niveau}", tr::NOMS_MODES[r.mode.min(tr::MODES - 1) as usize]));
        }
        let arret = d.arrets_dans([x - 25.0, z - 25.0, x + 25.0, z + 25.0]).into_iter().map(|k| ((t.arrets[k as usize].x - x).hypot(t.arrets[k as usize].z - z), k as usize)).filter(|(e, _)| *e < 25.0).min_by(|a, b| a.0.total_cmp(&b.0));
        if let Some((_, k)) = arret {
            let a = &t.arrets[k];
            let modes: Vec<&str> = (0..tr::MODES).filter(|m| a.modes & (1 << m) != 0).map(|m| tr::NOMS_MODES[m as usize]).collect();
            let nom = if a.nom.is_empty() { "sans nom" } else { a.nom.as_str() };
            let sous = if a.niveau < 0 { format!(" · sous terre (niveau {})", a.niveau) } else { String::new() };
            v.push(format!("arrêt {nom} · {}{}{sous}", modes.join(", "), if a.abri { " · abri" } else { "" }));
            let mut lignes: Vec<String> = t.lignes.iter().filter(|li| li.arrets.contains(&(k as u32))).map(|li| li.titre()).collect();
            lignes.sort();
            lignes.dedup();
            if !lignes.is_empty() {
                v.push(format!("lignes : {}", lignes.join(", ")));
            }
        }
        v
    }

    /// Les lignes du lieu, une par mode et numéro (l'aller et le retour ensemble),
    /// rangées par mode puis par numéro : (mode, numéro, indices des relations).
    fn groupes_lignes(t: &sage_reel::transports::Transports) -> Vec<(u8, String, Vec<usize>)> {
        let mut g: Vec<(u8, String, Vec<usize>)> = Vec::new();
        for (k, l) in t.lignes.iter().enumerate() {
            match g.iter_mut().find(|e| e.0 == l.mode && e.1 == l.numero) {
                Some(e) => e.2.push(k),
                None => g.push((l.mode, l.numero.clone(), vec![k])),
            }
        }
        // Les numéros dans l'ordre naturel (2 avant 10)
        let cle = |n: &str| {
            let chiffres: String = n.chars().take_while(|c| c.is_ascii_digit()).collect();
            (chiffres.is_empty(), chiffres.parse::<u32>().unwrap_or(0), n.to_string())
        };
        g.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| cle(&a.1).cmp(&cle(&b.1))));
        g
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
        let g = l.grille;
        let zone = [x - rayon, z - rayon, x + rayon, z + rayon];
        match outil {
            RELIEF | EAU => {
                let (a0, a1) = (((x - rayon - o) / p).floor() as i64, ((x + rayon - o) / p).ceil() as i64);
                let (b0, b1) = (((z - rayon - o) / p).floor() as i64, ((z + rayon - o) / p).ceil() as i64);
                let mut changes = Vec::new();
                // Seulement dans le lieu
                for b in b0.max(g.b0 as i64)..=b1.min((g.b0 + g.nz) as i64 - 1) {
                    for a in a0.max(g.a0 as i64)..=a1.min((g.a0 + g.nx) as i64 - 1) {
                        let (px, pz) = (a as f32 * p + o, b as f32 * p + o);
                        let dd = (px - x).hypot(pz - z);
                        if dd > rayon {
                            continue;
                        }
                        let w = lisse(1.0 - dd / rayon);
                        let Some(i) = g.indice(a, b) else { continue };
                        let i = i as u32;
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
                let (mx, mz) = g.sol();
                if l.sol.len() != mx * mz {
                    l.sol = vec![0; mx * mz];
                }
                // La grille du sol de la carte, au double pas ; le lieu commence en (sa, sb)
                let (sa, sb) = ((g.a0 / 2) as i64, (g.b0 / 2) as i64);
                let q = 2.0 * p;
                let (a0, a1) = (((x - rayon - o) / q).floor() as i64, ((x + rayon - o) / q).ceil() as i64);
                let (b0, b1) = (((z - rayon - o) / q).floor() as i64, ((z + rayon - o) / q).ceil() as i64);
                for b in b0.max(sb)..=b1.min(sb + mz as i64 - 1) {
                    for a in a0.max(sa)..=a1.min(sa + mx as i64 - 1) {
                        if (a as f32 * q + o - x).hypot(b as f32 * q + o - z) > rayon {
                            continue;
                        }
                        let i = (b - sb) as usize * mx + (a - sa) as usize;
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
            let c = self.carte.as_ref().and_then(|c| c.donnees.read().ok().map(|d| d.monde().2)).unwrap_or(20000.0);
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

    /// Passe de la carte à la 3D (sur le même point), ou l'inverse.
    fn basculer_3d(&mut self) {
        if let Some(r) = self.trois_d.take() {
            if let Some(v) = &mut self.vue {
                (v.cx, v.cz) = (r.camera.cible[0] as f64, r.camera.cible[2] as f64);
            }
            return;
        }
        let Some(carte) = &self.carte else { return };
        let Ok(d) = carte.donnees.read() else { return };
        let mut r = Relief3d::new(&d);
        if let Some(v) = self.vue {
            r.camera.cible = [v.cx as f32, 0.0, v.cz as f32];
            r.camera.distance = self.zone.w.min(self.zone.h) / v.echelle * 1.1;
        }
        r.poser(&d);
        drop(d);
        self.trois_d = Some(r);
        if self.outil != SELECTION {
            self.outil = VOIR;
        }
        self.choix = None;
    }

    /// La vue 3D : un doigt tourne autour, deux doigts rapprochent et déplacent.
    fn dessiner_3d(&mut self, cx: &mut Cx, zone: Rect, base: Id, placage: Option<crate::draw::Placage3d>) {
        if self.trois_d.is_none() {
            let Some(carte) = &self.carte else { return };
            let Ok(d) = carte.donnees.read() else { return };
            let mut r = Relief3d::new(&d);
            if let Some(c) = self.camera3d.take() {
                r.camera = c;
            }
            drop(d);
            self.trois_d = Some(r);
        }
        let dp = cx.dp;
        // La manette d'une télécommande fait voler la caméra
        let maintenant = std::time::Instant::now();
        let manette = crate::telecommande::manette().map(|m| (m, self.manette_vu.map_or(0.0, |t| maintenant.duration_since(t).as_secs_f32().min(0.1))));
        self.manette_vu = manette.map(|_| maintenant);
        let Some(r) = &mut self.trois_d else { return };
        for g in cx.ui.gestes(sub_id(base, 2), zone) {
            match g {
                Geste::SaisieDebut { doigt, .. } => self.orbite = Some(doigt),
                Geste::SaisieDeplace { doigt, .. } => {
                    if let Some(a) = self.orbite {
                        r.orbiter(doigt.0 - a.0, doigt.1 - a.1, dp);
                    }
                    self.orbite = Some(doigt);
                }
                Geste::SaisieFin { .. } | Geste::SaisieAnnulee { .. } => self.orbite = None,
                Geste::PincementDebut { .. } => {
                    self.pincement = 1.0;
                    self.orbite = None;
                }
                Geste::Pincement { echelle, deplacement, .. } => {
                    let k = echelle / self.pincement.max(1e-3);
                    self.pincement = echelle;
                    r.pincer(k, deplacement.0, deplacement.1, zone.h);
                }
                // En Maquette, un toucher ajoute la tuile sous le doigt à la sélection, ou
                // l'en retire
                Geste::Tap { x, y, .. } if self.maquette3d => {
                    if let Some(carte) = &self.carte {
                        if let Ok(d) = carte.donnees.read() {
                            if let Some(c) = r.tuile_touchee(&d, zone, x, y) {
                                self.selection.basculer(c);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        if let Some((m, dt)) = manette {
            r.piloter(&m, dt);
        }
        r.blanc = self.blanc3d;
        r.maquette = self.maquette3d;
        r.defonce = self.maquette3d && self.defonce3d;
        if r.selection != self.selection.cles {
            r.selection = self.selection.cles.clone();
        }
        r.seules = self.selection.seules;
        r.sur = self.sur;
        r.bati_mat = self.bati_mat;
        if self.materialisation3d && self.maquette3d && self.zone_centre.is_none() {
            self.zone_centre = Some([r.camera.cible[0], r.camera.cible[2]]);
        }
        r.zone = match (self.materialisation3d && self.maquette3d, self.zone_centre) {
            (true, Some(centre)) => Some(crate::relief3d::ZoneFine { centre, rayon: self.zone_rayon }),
            _ => None,
        };
        r.placage = placage;
        let a = cx.couleurs().accent;
        r.accent = [a[0] as f32 / 255.0, a[1] as f32 / 255.0, a[2] as f32 / 255.0];
        let Some(carte) = &mut self.carte else { return };
        cx.clip(Some(zone));
        r.dessiner(cx, zone, carte);
        // Le rendu, en haut à gauche, par-dessus la vue (et ses gestes)
        let m = cx.ecart();
        let mut x = zone.x + m;
        // Carte, Maquette ou Matérialisation ; Blanc, en plus, sur chacune ; en maquette
        // (et en matérialisation), la défonce des lits et le suivi
        let n = if self.maquette3d { 6 } else { 4 };
        let mat = self.maquette3d && self.materialisation3d;
        // Trop étroit : les pilules passent à la ligne
        let mut y = zone.y + m;
        for (i, nom) in ["Carte", "Maquette", "Matérialisation", "Blanc", "Défonce", "Suivi"].into_iter().enumerate().take(n) {
            let choisi = match i {
                0 => !self.maquette3d,
                1 => self.maquette3d && !mat,
                2 => mat,
                3 => self.blanc3d,
                4 => self.defonce3d,
                _ => self.suivi3d,
            };
            if x > zone.x + m && x + cx.pill_width(nom) > zone.right() - m {
                x = zone.x + m;
                y += cx.commande() + m * 0.5;
            }
            if cx.pill(sub_id(base, 30 + i), x, y, nom, choisi) {
                match i {
                    0 => self.maquette3d = false,
                    1 => {
                        self.maquette3d = true;
                        self.materialisation3d = false;
                    }
                    2 => {
                        // La zone se centre au point visé en entrant
                        if !mat {
                            self.zone_centre = Some([r.camera.cible[0], r.camera.cible[2]]);
                        }
                        self.maquette3d = true;
                        self.materialisation3d = true;
                    }
                    3 => self.blanc3d = !self.blanc3d,
                    4 => self.defonce3d = !self.defonce3d,
                    _ => self.suivi3d = !self.suivi3d,
                }
            }
            // Un peu plus d'écart avant l'interrupteur Blanc
            x += cx.pill_width(nom) + m * if i == 2 { 1.5 } else { 0.5 };
        }
        // La zone fine qui se prépare : son avancement au milieu de la vue
        if let Some(e) = r.zone_etat {
            dessiner_attente_zone(cx, zone, &e, self.zone_rayon);
        }
        // La génération en cours, en bas à gauche : tuiles faites sur tuiles du lot, et
        // une barre d'avancement au pied de la pastille
        if let Some((faites, total)) = r.generation {
            let texte = format!("Génération · {faites} / {}", pluriel(total, "tuile", "tuiles"));
            let c = cx.couleurs();
            let h = cx.commande();
            let w = cx.measure(&texte, 13.0) + 2.0 * cx.pad();
            let p = Rect::new(zone.x + m, zone.y + zone.h - m - h, w, h);
            let rayon = cx.theme.rayon(h);
            cx.ombre(p, rayon, 6.0);
            cx.dl.rrect(p, rayon, c.surface);
            cx.text_at(p.x + cx.pad(), p.y + h * 0.45, &texte, 13.0, c.texte);
            let (bx, bw, bh) = (p.x + rayon * 0.6, p.w - rayon * 1.2, (2.5 * cx.dp).round());
            let by = p.y + h - bh - (4.0 * cx.dp).round();
            cx.dl.rrect(Rect::new(bx, by, bw, bh), bh / 2.0, c.puce);
            let f = faites as f32 / total.max(1) as f32;
            cx.dl.rrect(Rect::new(bx, by, (bw * f).max(bh), bh), bh / 2.0, c.accent);
        }
        if self.maquette3d && self.suivi3d {
            let haut = y + cx.commande() + m;
            let bas = zone.y + zone.h - m - if r.generation.is_some() { cx.commande() + m } else { 0.0 };
            dessiner_suivi(cx, Rect::new(zone.x + m, haut, zone.w - 2.0 * m, (bas - haut).max(0.0)), zone);
        }
        cx.clip(None);
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
        // Le cercle de la zone des cartes (et de la Matérialisation)
        if self.outil == VOIR {
            if let Some(c) = self.zone_centre {
                let n = 96;
                let p = |k: usize| {
                    let a = k as f32 / n as f32 * std::f32::consts::TAU;
                    vue.ecran(zone, c[0] + self.zone_rayon * a.cos(), c[1] + self.zone_rayon * a.sin())
                };
                for k in 0..n {
                    cx.dl.line(p(k), p(k + 1), 2.0 * d, accent);
                }
            }
        }
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
            // La ligne surlignée : son tracé, ses arrêts, leurs noms de près
            if let Some((mode, numero)) = &self.ligne {
                let t = &dn.lieu.transports;
                let choisies: Vec<&sage_reel::transports::Ligne> = t.lignes.iter().filter(|l| l.mode == *mode && &l.numero == numero).collect();
                if let Some(l0) = choisies.first() {
                    let c = l0.teinte().map(|v| (v * 255.0) as u8);
                    let large = (4.0 * vue.echelle).clamp(3.0 * d, 8.0 * d);
                    // Sous terre : un trait creux (le blanc au milieu) ; dessous d'abord,
                    // pour que l'aller et le retour superposés restent lisibles
                    for sous in [true, false] {
                        for m in choisies.iter().flat_map(|l| l.trace.iter()).filter(|m| (m.ou == sage_reel::transports::TUNNEL) == sous) {
                            for s in m.points.windows(2) {
                                let (a, b) = (vue.ecran(zone, s[0][0], s[0][1]), vue.ecran(zone, s[1][0], s[1][1]));
                                cx.dl.line(a, b, large + 3.0 * d, [255, 255, 255, 220]);
                                cx.dl.line(a, b, large, [c[0], c[1], c[2], 255]);
                                if sous {
                                    cx.dl.line(a, b, large * 0.4, [255, 255, 255, 255]);
                                }
                            }
                        }
                    }
                    let mut vus = HashSet::new();
                    let mut noms: Vec<(&str, f32, f32)> = Vec::new();
                    for &k in choisies.iter().flat_map(|l| l.arrets.iter()) {
                        if !vus.insert(k) {
                            continue;
                        }
                        let a = &t.arrets[k as usize];
                        let p = vue.ecran(zone, a.x, a.z);
                        // Une station souterraine : un carré
                        let rr = 6.0 * d;
                        let coin = if a.niveau < 0 { 1.5 * d } else { rr };
                        cx.dl.rrect(Rect::new(p[0] - rr, p[1] - rr, 2.0 * rr, 2.0 * rr), coin, [255, 255, 255, 255]);
                        cx.dl.rrect(Rect::new(p[0] - rr + 2.0 * d, p[1] - rr + 2.0 * d, 2.0 * rr - 4.0 * d, 2.0 * rr - 4.0 * d), coin, [c[0], c[1], c[2], 255]);
                        // Un nom une fois (les quais des deux sens sont voisins)
                        if vue.echelle > 0.15 && !a.nom.is_empty() && !noms.iter().any(|n| n.0 == a.nom && (n.1 - a.x).hypot(n.2 - a.z) < 80.0) {
                            noms.push((&a.nom, a.x, a.z));
                            cx.text_at(p[0] + rr + 3.0 * d, p[1] + 4.0 * d, &a.nom, 12.0, [0x20, 0x20, 0x28, 255]);
                        }
                    }
                }
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

    pub(crate) fn puces(cx: &mut Cx, b: &mut crate::widgets::Body, base: Id, labels: &[&str], choisi: usize) -> Option<usize> {
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
                    if self.trois_d.is_some() {
                        cx.paragraph(&mut b, "En 3D : un doigt tourne autour, deux doigts rapprochent et déplacent. Le cube de la barre revient à la carte ; un outil d'édition aussi.");
                    } else {
                        cx.paragraph(&mut b, "Touchez la carte pour savoir ce qu'il y a là. Deux doigts zooment. Le cube de la barre passe en 3D.");
                    }
                }
                if let Ok(dn) = carte.donnees.read() {
                    let l = &dn.lieu;
                    cx.section(&mut b, "Le lieu");
                    cx.paragraph(&mut b, &format!("{} · centre {:.4}, {:.4}", l.nom, l.lat, l.lon));
                    cx.paragraph(&mut b, &format!("{} × {} points de {} m · eau à {:.1} m", l.grille.nx, l.grille.nz, l.pas, l.niveau).replace('.', ","));
                    cx.paragraph(&mut b, &format!("{} bâtiments, {} routes, {} ponts", l.batiments.len(), l.routes.len(), l.ponts.len()));
                    // Les cours d'eau, par nom : genre, longueur, part sous terre
                    if !l.cours_eau.is_empty() {
                        cx.section(&mut b, "Cours d'eau");
                        let mut noms: Vec<(String, u8, f32, f32)> = Vec::new();
                        let mut sans_nom = 0;
                        for c in &l.cours_eau {
                            let long: f32 = c.points.windows(2).map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1])).sum();
                            if c.nom.is_empty() {
                                sans_nom += 1;
                                continue;
                            }
                            match noms.iter_mut().find(|n| n.0 == c.nom) {
                                Some(n) => {
                                    n.2 += long;
                                    n.3 += if c.souterrain { long } else { 0.0 };
                                }
                                None => noms.push((c.nom.clone(), c.genre, long, if c.souterrain { long } else { 0.0 })),
                            }
                        }
                        noms.sort_by(|a, b| b.2.total_cmp(&a.2));
                        for (nom, genre, long, sous) in noms {
                            let mut t = format!("{nom} · {} · {:.1} km", sage_reel::cours_eau::NOMS[(genre as usize).min(3)].to_lowercase(), long / 1000.0);
                            if sous > 0.0 {
                                t = format!("{t}, dont {:.1} sous terre", sous / 1000.0);
                            }
                            cx.paragraph(&mut b, &t.replace('.', ","));
                        }
                        if sans_nom > 0 {
                            cx.paragraph(&mut b, &format!("et {sans_nom} tronçons sans nom"));
                        }
                    }
                    // Les surfaces d'eau d'OSM : leurs côtés sont les berges
                    for (k, sf) in l.surfaces_eau.iter().enumerate() {
                        if k == 0 {
                            cx.section(&mut b, "Surfaces d'eau (berges)");
                        }
                        let peri: f32 = sf.anneaux.iter().map(|r| (0..r.len()).map(|i| (r[(i + 1) % r.len()][0] - r[i][0]).hypot(r[(i + 1) % r.len()][1] - r[i][1])).sum::<f32>()).sum();
                        let points: usize = sf.anneaux.iter().map(|r| r.len()).sum();
                        let iles = sf.anneaux.len().saturating_sub(1);
                        cx.paragraph(&mut b, &format!("{} · {:.1} km de berges · {} points{}", sage_reel::cours_eau::NOMS_SURFACES[(sf.nature as usize).min(2)], peri / 1000.0, points, if iles > 0 { format!(" · {iles} anneaux intérieurs") } else { String::new() }).replace('.', ","));
                    }
                    if !l.surfaces_eau.is_empty() {
                        let (q, n) = crate::berges::longueurs(&dn);
                        cx.paragraph(&mut b, &format!("Berges : {:.1} km de quais, {:.1} km naturelles (sur la carte : quais gris, berges naturelles vertes)", q / 1000.0, n / 1000.0).replace('.', ","));
                    }
                    let t = &l.transports;
                    if t.est_vide() {
                        cx.paragraph(&mut b, "Pas de transports : un lieu généré avant Flow 0.68 ; le générer à nouveau les ajoute (le cache garde le reste).");
                    } else {
                        cx.paragraph(&mut b, &format!("{} tunnels, {} voies ferrées, {} arrêts, {} lignes", t.tunnels.len(), t.rails.len(), t.arrets.len(), t.lignes.len()));
                        cx.section(&mut b, "Lignes");
                        cx.paragraph(&mut b, "Touchez une ligne pour la voir sur la carte, avec ses arrêts.");
                        for (i, (mode, numero, ks)) in Lieux::groupes_lignes(t).into_iter().enumerate() {
                            let li = &t.lignes[ks[0]];
                            let c = li.teinte().map(|v| (v * 255.0) as u8);
                            let choisie = self.ligne.as_ref() == Some(&(mode, numero.clone()));
                            if cx.row(&mut b, sub_id(base, 10_000 + i), &li.titre(), Some([c[0], c[1], c[2], 255]), choisie) {
                                self.ligne = if choisie { None } else { Some((mode, numero.clone())) };
                            }
                            if choisie {
                                // Le trajet le plus long, ses arrêts dans l'ordre
                                if let Some(l) = ks.iter().map(|&k| &t.lignes[k]).max_by_key(|l| l.arrets.len()) {
                                    if !l.nom.is_empty() {
                                        cx.paragraph(&mut b, &l.nom);
                                    }
                                    let noms: Vec<String> = l.arrets.iter().map(|&k| &t.arrets[k as usize]).filter(|a| !a.nom.is_empty()).map(|a| if a.niveau < 0 { format!("{} (sous terre)", a.nom) } else { a.nom.clone() }).collect();
                                    if !noms.is_empty() {
                                        cx.paragraph(&mut b, &format!("{} arrêts : {}", noms.len(), noms.join(" · ")));
                                    }
                                    let long = |m: &sage_reel::transports::Morceau| m.points.windows(2).map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1])).sum::<f32>();
                                    let tout: f32 = l.trace.iter().map(long).sum();
                                    let sous: f32 = l.trace.iter().filter(|m| m.ou == sage_reel::transports::TUNNEL).map(long).sum();
                                    if sous > 0.0 {
                                        cx.paragraph(&mut b, &format!("Sous terre : {:.1} km sur {:.1} km (trait creux sur la carte)", sous / 1000.0, tout / 1000.0).replace('.', ","));
                                    }
                                }
                            }
                        }
                    }
                }
                // La surimpression sur la maquette : tracés du réseau, remblais des voies
                if self.trois_d.is_some() && self.maquette3d {
                    cx.section(&mut b, "Surimpression");
                    let s = &mut self.sur;
                    cx.toggle(&mut b, sub_id(base, 660), "Tracés des voies", &mut s.traces);
                    if s.traces {
                        cx.slider(&mut b, sub_id(base, 661), "Distance", &format!("{:.0} m", s.traces_distance), &mut s.traces_distance, 50.0, 3000.0);
                        cx.slider(&mut b, sub_id(base, 662), "Rayon", &format!("× {:.1}", s.traces_rayon).replace('.', ","), &mut s.traces_rayon, 0.5, 4.0);
                    }
                    cx.toggle(&mut b, sub_id(base, 663), "Remblais des voies", &mut s.remblai);
                    if s.remblai {
                        cx.slider(&mut b, sub_id(base, 664), "Distance", &format!("{:.0} m", s.remblai_distance), &mut s.remblai_distance, 50.0, 3000.0);
                        cx.slider(&mut b, sub_id(base, 665), "Rayon", &format!("× {:.1}", s.remblai_rayon).replace('.', ","), &mut s.remblai_rayon, 0.5, 4.0);
                    }
                    cx.paragraph(&mut b, "Ils paraissent quand la caméra est à moins de la distance de son point visé, sur les tuiles à moins du rayon × cette distance.");
                    if cx.button(&mut b, sub_id(base, 666), "Refaire") {
                        if let Some(r) = self.trois_d.as_mut() {
                            r.refaire_traces();
                        }
                    }
                }
                // La Matérialisation : le niveau le plus fin seul, dans un rayon, montré
                // quand tout est fait
                if self.trois_d.is_some() && self.maquette3d && self.materialisation3d {
                    cx.section(&mut b, "Matérialisation");
                    cx.slider(&mut b, sub_id(base, 671), "Rayon", &format!("{:.0} m", self.zone_rayon), &mut self.zone_rayon, 50.0, 1500.0);
                    self.zone_rayon = (self.zone_rayon / 25.0).round() * 25.0;
                    {
                        if let Ok(d) = carte.donnees.read() {
                            let centre = self.zone_centre.or(self.trois_d.as_ref().map(|r| [r.camera.cible[0], r.camera.cible[2]])).unwrap_or([0.0; 2]);
                            let n = crate::relief3d::tuiles_zone(&d, crate::relief3d::ZoneFine { centre, rayon: self.zone_rayon }).len();
                            let (_, _, c) = d.monde();
                            let cote = c / (1u32 << crate::relief3d::niveau_fin(&d)) as f32;
                            let trop = if n > crate::relief3d::ZONE_MAX { format!(" : trop (au plus {})", crate::relief3d::ZONE_MAX) } else { String::new() };
                            cx.paragraph(&mut b, &format!("{} de {:.0} m{trop}. Elles sont toutes matérialisées avant que la vue paraisse ; la caméra reste assez près pour voir la zone entière.", pluriel(n, "tuile", "tuiles"), cote));
                        }
                    }
                    if cx.button(&mut b, sub_id(base, 672), "Recentrer sur le point visé") {
                        self.zone_centre = self.trois_d.as_ref().map(|r| [r.camera.cible[0], r.camera.cible[2]]);
                    }
                    // Les bâtiments en volumes fermés, avant de les unir au terrain
                    cx.toggle(&mut b, sub_id(base, 673), "Bâtiments (volumes fermés)", &mut self.bati_mat);
                    if self.bati_mat {
                        if let Some(r) = &self.trois_d {
                            let (faites, total, ok, aplanis, refuses) = r.bati_mat_bilan;
                            let attente = if faites < total { format!(" ({faites} tuiles sur {total})") } else { String::new() };
                            cx.paragraph(&mut b, &format!("{} fermés, en bleu, dont {} à toit plat (toit d'origine qui ne fermait pas) ; {} en rouge{attente}.", pluriel(ok, "bâtiment", "bâtiments"), aplanis, pluriel(refuses, "refusé", "refusés")));
                        }
                    }
                }
                // L'ordre de dessin, du dessus au dessous ; la 3D, drapée des mêmes
                // tuiles, le suit
                cx.section(&mut b, "Ordre de dessin");
                cx.paragraph(&mut b, "Du dessus au dessous. Touchez une couche, puis montez-la ou descendez-la.");
                let avant = self.ordre.clone();
                for (i, &k) in avant.iter().enumerate().rev() {
                    let c = crate::carte::couleur_couche(k).map(|v| (v * 255.0) as u8);
                    if cx.row(&mut b, sub_id(base, 600 + i), crate::carte::nom_couche(k), Some([c[0], c[1], c[2], 255]), self.couche == Some(k)) {
                        self.couche = if self.couche == Some(k) { None } else { Some(k) };
                    }
                }
                let pos = self.couche.and_then(|k| self.ordre.iter().position(|&c| c == k));
                if let Some(p) = pos {
                    if p + 1 < self.ordre.len() && cx.button(&mut b, sub_id(base, 640), "Monter") {
                        self.ordre.swap(p, p + 1);
                    }
                    if p > 0 && cx.button(&mut b, sub_id(base, 641), "Descendre") {
                        self.ordre.swap(p, p - 1);
                    }
                }
                if self.ordre != crate::carte::ordre_par_defaut() && cx.button(&mut b, sub_id(base, 642), "Ordre par défaut") {
                    self.ordre = crate::carte::ordre_par_defaut();
                }
                if self.ordre != avant {
                    if let Ok(mut dn) = carte.donnees.write() {
                        dn.ordre = self.ordre.clone();
                    }
                    carte.tout_retoucher();
                }
                // Les cartes pour WhiteRoom et S.A.G.E. (`sage-carte`) : la zone ou le lieu
                // entier matérialisés tuile par tuile dans un fil et écrits dans
                // Flow/cartes, sans passer par la vue Matérialisation
                self.panneau_cartes(cx, &mut b, base, c.dossier.clone());
                cx.section(&mut b, "Sources");
                cx.paragraph(&mut b, "Relief © IGN · eau, routes, transports, bâtiments, sol © les contributeurs OpenStreetMap (ODbL). Un lieu généré en est dérivé : ne le publiez pas.");
            }
            SELECTION => self.panneau_selection(cx, &mut b, base),
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

    /// La sélection de tuiles de la maquette : ne voir qu'elle, son résumé (surface,
    /// éléments du lieu touchés, maillages), la liste de ses tuiles.
    /// Le point visé : la cible de la caméra en 3D, le milieu de la carte en 2D.
    fn point_vise(&self) -> Option<[f32; 2]> {
        match (&self.trois_d, &self.vue) {
            (Some(r), _) => Some([r.camera.cible[0], r.camera.cible[2]]),
            (None, Some(v)) => Some([v.cx as f32, v.cz as f32]),
            _ => None,
        }
    }

    /// La section « Cartes » : générer sur le disque la carte de la zone (disque de la
    /// Matérialisation) ou du lieu entier.
    fn panneau_cartes(&mut self, cx: &mut Cx, b: &mut crate::widgets::Body, base: Id, flow: Option<std::path::PathBuf>) {
        let Some(carte) = &self.carte else { return };
        let donnees = carte.donnees.clone();
        cx.section(b, "Cartes (WhiteRoom, S.A.G.E.)");
        // Rayon et centre sont ceux de la Matérialisation ; réglés ici quand elle n'est
        // pas ouverte (le cercle de la zone est tracé sur la carte)
        let materialisation = self.trois_d.is_some() && self.maquette3d && self.materialisation3d;
        if !materialisation {
            cx.slider(b, sub_id(base, 676), "Rayon", &format!("{:.0} m", self.zone_rayon), &mut self.zone_rayon, 50.0, 1500.0);
            self.zone_rayon = (self.zone_rayon / 25.0).round() * 25.0;
            if cx.button(b, sub_id(base, 677), "Centrer la zone sur le point visé") {
                self.zone_centre = self.point_vise();
            }
        }
        // En 2D, la zone se pose au milieu de la carte (son cercle paraît)
        if self.zone_centre.is_none() && self.trois_d.is_none() {
            self.zone_centre = self.point_vise();
        }
        let centre = self.zone_centre.or_else(|| self.point_vise());
        let lieu = self.chemin.as_ref().and_then(|p| p.file_stem()).map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "lieu".into());
        let (zone, entiere, n_zone, n_tout, cote) = {
            let Ok(d) = donnees.read() else { return };
            let zone = centre.map(|centre| crate::relief3d::ZoneFine { centre, rayon: self.zone_rayon });
            let entiere = crate::generer_carte::zone_entiere(&d);
            let n_zone = zone.map_or(0, |z| crate::relief3d::tuiles_zone(&d, z).len());
            let n_tout = crate::relief3d::tuiles_zone(&d, entiere).len();
            let (_, _, c) = d.monde();
            (zone, entiere, n_zone, n_tout, c / (1u32 << crate::relief3d::niveau_fin(&d)) as f32)
        };
        let etat = crate::generer_carte::etat();
        if etat.as_ref().is_some_and(|e| !e.fini) {
            if cx.button(b, sub_id(base, 675), "Arrêter la génération") {
                crate::generer_carte::arreter();
            }
        } else if let Some(flow) = flow {
            cx.paragraph(b, &format!("Tuiles de {cote:.0} m : {} dans la zone ({:.0} m autour de {}), {} pour toute la carte. Une génération arrêtée reprend où elle en était.", n_zone, self.zone_rayon, centre.map_or("—".into(), |c| format!("{:.0} ; {:.0}", c[0], c[1])), n_tout));
            if let Some(z) = zone {
                if cx.button(b, sub_id(base, 674), "Générer la zone") {
                    let nom = crate::generer_carte::nom_par_defaut(&lieu, z);
                    crate::generer_carte::lancer(donnees.clone(), z, &flow, lieu.clone(), nom);
                }
            }
            if cx.button(b, sub_id(base, 678), "Générer toute la carte") {
                let nom = crate::generer_carte::nom_entiere(&lieu);
                crate::generer_carte::lancer(donnees.clone(), entiere, &flow, lieu.clone(), nom);
            }
        }
        if let Some(e) = etat {
            let lieu_carte = format!("Flow/{}/{}", crate::generer_carte::DOSSIER, e.nom);
            let texte = match (&e.erreur, e.fini) {
                (Some(m), _) => format!("Génération {m} : {} tuiles sur {} dans {lieu_carte} (relancer reprend où elle en était).", e.faites, e.total),
                (None, true) => format!("Carte écrite dans {lieu_carte} : {} tuiles, {} triangles, en {:.0} s{}.", e.total, e.triangles, e.duree, if e.reprises > 0 { format!(" ({} reprises)", e.reprises) } else { String::new() }),
                (None, false) => format!("Génération de {lieu_carte} : {} tuiles sur {}, {} triangles…", e.faites, e.total, e.triangles),
            };
            cx.paragraph(b, &texte);
        }
    }

    fn panneau_selection(&mut self, cx: &mut Cx, b: &mut crate::widgets::Body, base: Id) {
        let en_maquette = self.trois_d.is_some() && self.maquette3d;
        if !en_maquette {
            cx.paragraph(b, "Les tuiles se sélectionnent dans la 3D, en rendu Maquette.");
            if cx.button(b, sub_id(base, 700), "Passer en Maquette") {
                self.maquette3d = true;
                if self.trois_d.is_none() {
                    self.basculer_3d();
                }
            }
        } else {
            cx.paragraph(b, "Touchez une tuile pour la sélectionner ; touchez-la encore pour la retirer.");
        }
        cx.toggle(b, sub_id(base, 701), "Ne voir que la sélection", &mut self.selection.seules);
        if self.selection.est_vide() {
            cx.paragraph(b, "Aucune tuile sélectionnée.");
            return;
        }
        let Some(carte) = &self.carte else { return };
        // Le résumé, refait quand la sélection ou la carte changent
        let a_jour = self.resume.as_ref().is_some_and(|(c, n, _)| *c == self.selection.cles && *n == carte.compteur);
        if !a_jour {
            if let Ok(d) = carte.donnees.read() {
                self.resume = Some((self.selection.cles.clone(), carte.compteur, crate::selection::resumer(&d, &self.selection.cles)));
            }
        }
        let Some((_, _, r)) = &self.resume else { return };
        cx.section(b, "En résumé");
        cx.paragraph(b, &format!("{} · {}", pluriel(r.tuiles, "tuile", "tuiles"), surface(r.surface)));
        let genres = ["maison", "immeuble", "bâtiment industriel", "édifice", "annexe"];
        let pluriels = ["maisons", "immeubles", "bâtiments industriels", "édifices", "annexes"];
        let detail: Vec<String> = (0..5).filter(|&g| r.batiments[g] > 0).map(|g| pluriel(r.batiments[g], genres[g], pluriels[g])).collect();
        cx.paragraph(b, &if detail.is_empty() { "Aucun bâtiment".to_string() } else { format!("{} : {}", pluriel(r.batiments_total(), "bâtiment", "bâtiments"), detail.join(", ")) });
        let mut voies = vec![pluriel(r.routes, "route", "routes")];
        voies.extend([(r.ponts, "pont", "ponts"), (r.tunnels, "tunnel", "tunnels"), (r.rails, "voie ferrée", "voies ferrées"), (r.arrets, "arrêt", "arrêts")].into_iter().filter(|x| x.0 > 0).map(|(n, a, p)| pluriel(n, a, p)));
        cx.paragraph(b, &voies.join(", "));
        // Les maillages de la maquette : ceux des tuiles déjà faites
        let mesures: Vec<crate::maquette::Mesure> = self.selection.cles.iter().filter_map(|&c| self.trois_d.as_ref().and_then(|t| t.mesure(c))).collect();
        let (s, a, t, o) = mesures.iter().fold((0, 0, 0, 0), |m, x| (m.0 + x.sommets, m.1 + x.aretes, m.2 + x.triangles, m.3 + x.octets));
        let manque = self.selection.cles.len() - mesures.len();
        cx.paragraph(b, &format!("{} sommets · {} arêtes · {} triangles", milliers(s), milliers(a), milliers(t)));
        let attente = if manque > 0 { format!(" ({} en cours)", pluriel(manque, "tuile", "tuiles")) } else { String::new() };
        cx.paragraph(b, &format!("Mémoire : {}{attente}", octets(o)));
        cx.section(b, "Tuiles");
        cx.paragraph(b, "Niveau/colonne/rangée. Touchez une tuile pour y aller.");
        let mut aller = None;
        for (k, &c) in self.selection.cles.iter().enumerate() {
            let cote = carte.donnees.read().map(|d| boite_tuile(&d, c)).ok();
            let texte = match cote {
                Some(bt) => format!("{} · {:.0} m", crate::selection::nom(c), bt[2] - bt[0]),
                None => crate::selection::nom(c),
            };
            if cx.row(b, sub_id(base, 20_000 + k), &texte, None, false) {
                aller = cote;
            }
        }
        if let (Some(bt), Some(r3)) = (aller, &mut self.trois_d) {
            r3.camera.cible[0] = (bt[0] + bt[2]) / 2.0;
            r3.camera.cible[2] = (bt[1] + bt[3]) / 2.0;
            r3.camera.distance = r3.camera.distance.min((bt[2] - bt[0]) * 3.0).max(50.0);
        }
        if cx.button(b, sub_id(base, 702), "Tout retirer") {
            self.selection.cles.clear();
        }
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
            if cx.button(&mut b, sub_id(base, 716), "Choisir sur la carte") {
                c.clavier.fermer();
                self.ouvrir_plan(c);
            }
            let mut pas = self.pas;
            cx.slider(&mut b, sub_id(base, 710), "Pas de la grille", &format!("{pas:.0} m"), &mut pas, 2.0, 16.0);
            self.pas = pas.round();
            let (mut lx, mut lz) = (self.largeur, self.longueur);
            cx.slider(&mut b, sub_id(base, 714), "Largeur (est-ouest)", &format!("{lx:.1} km").replace('.', ","), &mut lx, 1.0, 20.0);
            cx.slider(&mut b, sub_id(base, 715), "Longueur (nord-sud)", &format!("{lz:.1} km").replace('.', ","), &mut lz, 1.0, 20.0);
            (self.largeur, self.longueur) = ((lx * 2.0).round() / 2.0, (lz * 2.0).round() / 2.0);
            cx.toggle(&mut b, sub_id(base, 711), "Toits mesurés au LiDAR", &mut self.toits);
            let g = self.grille();
            let zone = if g.est_entiere(self.pas as f64) { "La carte entière, 20 km de côté".to_string() } else { format!("{} × {} km autour du point", self.largeur, self.longueur).replace('.', ",") };
            cx.paragraph(&mut b, &format!("{zone} : {} × {} points (le lieu vierge aussi). Le relief et le LiDAR viennent de l'IGN, le reste d'OpenStreetMap ; les toits demandent ~0,4 Go de téléchargements dans une ville. Relancer reprend ce qui est déjà téléchargé.", g.nx, g.nz));
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

    // ----- Le plan où choisir la zone

    fn cache_plan(c: &Commun) -> PathBuf {
        Lieux::dossier(c).unwrap_or_else(std::env::temp_dir).join(".plan")
    }

    /// Ouvre le plan sur la position donnée (la zone tout autour), sinon sur la France.
    fn ouvrir_plan(&mut self, c: &Commun) {
        let (lat, lon, large) = match lire_position(&self.position.texte) {
            Some((lat, lon)) => (lat, lon, self.largeur.max(self.longueur) as f64 * 2500.0),
            None => (46.6, 2.4, 1.1e6),
        };
        self.plan = Some(crate::plan::Plan::new(lat, lon, large, Lieux::cache_plan(c), ReseauFlow));
        self.resultats.clear();
    }

    fn lancer_recherche(&mut self, c: &Commun) {
        let t = self.recherche.texte.trim().to_string();
        if t.is_empty() || self.cherche.is_some() {
            return;
        }
        let cache = Lieux::cache_plan(c);
        self.message = None;
        self.cherche = Some(std::thread::spawn(move || chercher(&t, &cache)));
    }

    /// Le plan, et à côté (en paysage) ou dessous (en portrait) de quoi chercher, régler
    /// l'emprise et choisir.
    fn dessiner_plan(&mut self, cx: &mut Cx, c: &mut Commun, r: Rect, base: Id, k: usize) {
        let d = cx.dp;
        let col = cx.couleurs();
        // La recherche finie : un seul résultat y va tout de suite
        if self.cherche.as_ref().is_some_and(|h| h.is_finished()) {
            match self.cherche.take().map(|h| h.join()) {
                Some(Ok(Ok(v))) => {
                    if v.is_empty() {
                        self.message = Some(("Aucun lieu de ce nom en France.".into(), true));
                    }
                    if let (Some((_, lat, lon)), true) = (v.first(), v.len() == 1) {
                        if let Some(p) = &mut self.plan {
                            p.aller(*lat, *lon, self.largeur.max(self.longueur) as f64 * 2500.0);
                        }
                    }
                    self.resultats = v;
                }
                Some(Ok(Err(e))) => self.message = Some((format!("Recherche impossible : {e}"), true)),
                _ => self.message = Some(("Recherche interrompue".into(), true)),
            }
        }
        let (zone, panneau) = if r.w > r.h * 1.1 {
            let w = (340.0 * d).min(r.w * 0.42);
            (Rect::new(r.x, r.y, r.w - w, r.h), Rect::new(r.right() - w, r.y, w, r.h))
        } else {
            let h = (r.h * 0.42).max(200.0 * d).min(r.h * 0.6);
            (Rect::new(r.x, r.y, r.w, r.h - h), Rect::new(r.x, r.bottom() - h, r.w, h))
        };
        let emprise = if self.grille().est_entiere(self.pas as f64) { (20.0, 20.0) } else { (self.largeur, self.longueur) };
        if let Some(p) = &mut self.plan {
            for g in cx.ui.gestes(sub_id(base, 2), zone) {
                p.recevoir(g, zone);
            }
            p.dessiner(cx, zone, emprise);
        }
        cx.dl.rrect(panneau, 0.0, col.fond);
        let mut b = cx.body(sub_id(base, 5), panneau, &c.scrolls);
        cx.section(&mut b, "Choisir la zone");
        if let Some((m, _)) = &self.message {
            let m = m.clone();
            cx.paragraph(&mut b, &m);
        }
        clavier::champ(cx, &mut b, sub_id(RECHERCHE, k), "Chercher une ville, une adresse", &mut self.recherche, &mut c.clavier);
        let attente = self.cherche.is_some();
        if cx.button(&mut b, sub_id(base, 720), if attente { "Recherche…" } else { "Chercher" }) && !attente {
            c.clavier.fermer();
            self.lancer_recherche(c);
        }
        let mut aller = None;
        for (i, (nom, lat, lon)) in self.resultats.iter().enumerate().filter(|_| self.resultats.len() > 1) {
            if cx.button(&mut b, sub_id(base, 730 + i), nom) {
                aller = Some((*lat, *lon));
            }
        }
        if let (Some((lat, lon)), Some(p)) = (aller, &mut self.plan) {
            p.aller(lat, lon, self.largeur.max(self.longueur) as f64 * 2500.0);
            self.resultats.clear();
        }
        let (mut lx, mut lz) = (self.largeur, self.longueur);
        cx.slider(&mut b, sub_id(base, 724), "Largeur (est-ouest)", &format!("{lx:.1} km").replace('.', ","), &mut lx, 1.0, 20.0);
        cx.slider(&mut b, sub_id(base, 725), "Longueur (nord-sud)", &format!("{lz:.1} km").replace('.', ","), &mut lz, 1.0, 20.0);
        (self.largeur, self.longueur) = ((lx * 2.0).round() / 2.0, (lz * 2.0).round() / 2.0);
        let (lat, lon) = self.plan.as_ref().map_or((0.0, 0.0), |p| p.position());
        let centre = format!("{lat:.5} ; {lon:.5}").replace('.', ",");
        cx.paragraph(&mut b, &format!("Centre : {centre}. Glissez la carte sous le cadre ; deux doigts zooment, un toucher y centre."));
        if cx.button(&mut b, sub_id(base, 721), "Choisir cette zone") {
            self.position = Edition::new(&format!("{lat:.5}, {lon:.5}"));
            self.plan = None;
            self.message = None;
        }
        if cx.button(&mut b, sub_id(base, 722), "Annuler") {
            self.plan = None;
            self.message = None;
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
            if bouton(cx, x, 4, Icon::Cube, true, if self.trois_d.is_some() { col.accent } else { col.texte }) {
                self.basculer_3d();
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
            (_, Some(carte)) if self.trois_d.is_some() => {
                let r = self.trois_d.as_ref().unwrap();
                let attente = if r.en_attente || carte.en_attente() { " · dessin…" } else { "" };
                format!("3D · {} tuiles · {} de bâti{attente}", r.dessinees, r.bati_dessinees)
            }
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

    fn piloter(&mut self, m: &[String], _c: &mut Commun) -> Option<Result<String, String>> {
        let nombres = || m[1..].iter().map(|s| s.parse::<f32>().map_err(|_| format!("« {s} » n'est pas un nombre"))).collect::<Result<Vec<f32>, String>>();
        let decrire = |c: &Camera| format!("{} {} {} {} {} {}", c.lacet, c.tangage, c.distance, c.cible[0], c.cible[1], c.cible[2]);
        Some(match m[0].as_str() {
            // La caméra 3D : la lire, ou la poser (et passer en 3D)
            "vue3d" => {
                if m.len() == 1 {
                    return Some(self.trois_d.as_ref().map(|r| decrire(&r.camera)).or(self.camera3d.as_ref().map(decrire)).ok_or("pas de vue 3D".to_string()));
                }
                let v = match nombres() {
                    Ok(v) if v.len() == 6 => v,
                    Ok(_) => return Some(Err("vue3d lacet tangage distance x y z".into())),
                    Err(e) => return Some(Err(e)),
                };
                if self.carte.is_none() {
                    return Some(Err("aucun lieu chargé".into()));
                }
                let c = Camera { lacet: v[0], tangage: v[1], distance: v[2], cible: [v[3], v[4], v[5]] };
                if self.trois_d.is_none() {
                    self.basculer_3d();
                }
                match &mut self.trois_d {
                    Some(r) => r.camera = c,
                    None => self.camera3d = Some(c),
                }
                Ok(format!("vue3d {}", decrire(&c)))
            }
            // La carte 2D : en revenir, ou poser sa vue
            "carte" | "vue2d" => {
                if self.trois_d.is_some() {
                    self.basculer_3d();
                }
                if m[0] == "vue2d" {
                    match nombres() {
                        Ok(v) if v.len() == 3 && v[2] > 0.0 => self.vue = Some(Vue { cx: v[0] as f64, cz: v[1] as f64, echelle: v[2] }),
                        Ok(_) => return Some(Err("vue2d x z échelle".into())),
                        Err(e) => return Some(Err(e)),
                    }
                }
                Ok(self.vue.map(|v| format!("vue2d {} {} {}", v.cx, v.cz, v.echelle)).unwrap_or_default())
            }
            // Le rendu de la 3D : les mots donnés sont mis, les autres ôtés
            "rendu" => {
                let a = |n: &str| m[1..].iter().any(|w| w == n);
                self.maquette3d = a("maquette");
                self.blanc3d = a("blanc");
                self.defonce3d = a("defonce");
                self.suivi3d = a("suivi");
                if a("materialisation") {
                    if !self.materialisation3d {
                        self.zone_centre = self.trois_d.as_ref().map(|t| [t.camera.cible[0], t.camera.cible[2]]);
                    }
                    self.maquette3d = true;
                    self.materialisation3d = true;
                } else {
                    self.materialisation3d = false;
                }
                Ok(format!("rendu maquette={} materialisation={} blanc={} defonce={} suivi={}", self.maquette3d, self.materialisation3d, self.blanc3d, self.defonce3d, self.suivi3d))
            }
            "selection" => {
                if m.len() > 1 {
                    self.selection = crate::selection::Selection::lire(&m[1..].join(" "), self.selection.seules);
                }
                Ok(format!("selection {}", self.selection.ecrire()))
            }
            // Les tracés du réseau en surimpression : « traces » (bilan), « traces
            // distance <m> », « traces tuiles <n> », « traces refaire »
            "traces" => {
                let Some(r) = self.trois_d.as_mut() else { return Some(Err("pas de 3D".into())) };
                let n = m.get(2).and_then(|s| s.parse::<f32>().ok());
                match (m.get(1).map(|s| s.as_str()), n) {
                    (Some("distance"), Some(x)) => self.sur.traces_distance = x,
                    (Some("tuiles"), Some(x)) => r.traces_tuiles = x as usize,
                    (Some("rayon"), Some(x)) => self.sur.traces_rayon = x,
                    (Some("oui"), _) => self.sur.traces = true,
                    (Some("non"), _) => self.sur.traces = false,
                    (Some("refaire"), _) => r.refaire_traces(),
                    (None, _) => {}
                    _ => return Some(Err("traces [oui | non | distance <m> | rayon <k> | tuiles <n> | refaire]".into())),
                }
                Ok(format!("traces distance {} rayon {} tuiles {} · remblai distance {} rayon {} · {}", self.sur.traces_distance, self.sur.traces_rayon, r.traces_tuiles, self.sur.remblai_distance, self.sur.remblai_rayon, r.traces_bilan))
            }
            // Les remblais des voies : « remblai distance <m> », « remblai rayon <k> »
            "remblai" => {
                let n = m.get(2).and_then(|s| s.parse::<f32>().ok());
                match (m.get(1).map(|s| s.as_str()), n) {
                    (Some("distance"), Some(x)) => self.sur.remblai_distance = x,
                    (Some("rayon"), Some(x)) => self.sur.remblai_rayon = x,
                    (Some("oui"), _) => self.sur.remblai = true,
                    (Some("non"), _) => self.sur.remblai = false,
                    (None, _) => {}
                    _ => return Some(Err("remblai [oui | non | distance <m> | rayon <k>]".into())),
                }
                Ok(format!("remblai {} distance {} rayon {}", self.sur.remblai, self.sur.remblai_distance, self.sur.remblai_rayon))
            }
            // La zone fine : « zone non », « zone <rayon> » (au point visé), « zone »
            "zone" => {
                match m.get(1).map(|s| s.as_str()) {
                    Some("non") => self.materialisation3d = false,
                    Some(x) => match x.parse::<f32>() {
                        Ok(r) => {
                            self.zone_rayon = r.clamp(50.0, 1500.0);
                            self.materialisation3d = true;
                            self.maquette3d = true;
                            self.zone_centre = self.trois_d.as_ref().map(|t| [t.camera.cible[0], t.camera.cible[2]]);
                        }
                        Err(_) => return Some(Err("zone [non | <rayon>]".into())),
                    },
                    None => {}
                }
                let etat = self.trois_d.as_ref().and_then(|t| t.zone_etat).map_or("prête ou éteinte".to_string(), |e| format!("{} / {} tuiles, {:.0} s{}{}", e.faites, e.total, e.ecoule, e.reste.map_or(String::new(), |r| format!(", reste ~{r:.0} s")), if e.trop { ", trop de tuiles" } else { "" }));
                Ok(format!("zone {} rayon {} centre {:?} · {etat}", self.materialisation3d, self.zone_rayon, self.zone_centre))
            }
            "seules" => {
                self.selection.seules = matches!(m.get(1).map(|s| s.as_str()), Some("oui" | "1" | "vrai"));
                Ok(format!("seules {}", self.selection.seules))
            }
            "outil" => match m.get(1).and_then(|s| s.parse::<usize>().ok()) {
                Some(o) if o < OUTILS.len() => {
                    self.outil = o;
                    Ok(format!("outil {o}"))
                }
                _ => Err(format!("outil 0..{}", OUTILS.len() - 1)),
            },
            // La génération de la maquette : « faites total », ou « finie »
            "generation" => Ok(match self.trois_d.as_ref().and_then(|r| r.generation) {
                Some((f, t)) => format!("generation {f} {t}"),
                None => "generation finie".into(),
            }),
            _ => return None,
        })
    }

    fn ecrire(&self, e: &mut Etat) {
        if self.carte.is_some() || self.chargement.is_some() {
            if let Some(p) = &self.chemin {
                e.mettre("document", p.display());
            }
            if let Some(v) = self.vue {
                e.mettre("vue", format!("{} {} {}", v.cx, v.cz, v.echelle));
            }
            if let Some(c) = self.trois_d.as_ref().map(|r| r.camera).or(self.camera3d) {
                e.mettre("vue3d", format!("{} {} {} {} {} {}", c.lacet, c.tangage, c.distance, c.cible[0], c.cible[1], c.cible[2]));
            }
        }
        e.mettre("outil", self.outil);
        if !self.position.texte.trim().is_empty() {
            e.mettre("position", self.position.texte.trim());
        }
        if self.plan.is_some() {
            e.mettre("plan", "oui");
        }
        let rendu: Vec<&str> = [(self.maquette3d, "maquette"), (self.maquette3d && self.materialisation3d, "materialisation"), (self.blanc3d, "blanc"), (self.defonce3d, "defonce"), (self.suivi3d, "suivi")].into_iter().filter(|r| r.0).map(|r| r.1).collect();
        if !rendu.is_empty() {
            e.mettre("rendu3d", rendu.join(" "));
        }
        if !self.selection.est_vide() {
            e.mettre("selection", self.selection.ecrire());
        }
        if self.selection.seules {
            e.mettre("selection_seules", "oui");
        }
        if self.bati_mat {
            e.mettre("materialisation_batiments", "oui");
        }
        if self.sur != crate::relief3d::Surimpression::default() {
            e.mettre("surimpression", self.sur.ecrire());
        }
        match self.zone_centre {
            Some(c) => e.mettre("zone", format!("{} {} {}", self.zone_rayon, c[0], c[1])),
            _ if self.zone_rayon != 500.0 => e.mettre("zone_rayon", self.zone_rayon),
            _ => {}
        }
        if let Some((m, n)) = &self.ligne {
            e.mettre("ligne", format!("{m} {n}"));
        }
        if self.ordre != crate::carte::ordre_par_defaut() {
            e.mettre("ordre", self.ordre.iter().map(|c| c.to_string()).collect::<Vec<_>>().join(" "));
        }
    }

    fn anime(&self, _c: &Commun) -> bool {
        crate::generer_carte::en_cours() || (self.maquette3d && self.suivi3d && self.trois_d.is_some() && crate::suivi::occupe()) || (self.maquette3d && self.trois_d.is_some() && crate::placage::en_cours()) || self.carte.as_ref().is_some_and(|c| c.en_attente()) || self.trois_d.as_ref().is_some_and(|r| r.en_attente) || self.generation.is_some() || self.chargement.is_some() || self.enregistrement.is_some() || self.cherche.is_some() || self.plan.as_ref().is_some_and(|p| p.en_attente())
    }

    fn frappe(&mut self, f: FenetreId, cible: Id, frappe: Frappe, c: &mut Commun) -> bool {
        let k = f as usize;
        let ed = if cible == sub_id(NOM, k) {
            &mut self.nom
        } else if cible == sub_id(POSITION, k) {
            &mut self.position
        } else if cible == sub_id(RECHERCHE, k) {
            &mut self.recherche
        } else {
            return false;
        };
        if ed.appliquer(frappe) {
            c.clavier.fermer();
            if cible == sub_id(RECHERCHE, k) {
                self.lancer_recherche(c);
            }
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
        if std::mem::take(&mut self.plan_a_ouvrir) && self.carte.is_none() && self.chemin.is_none() {
            self.ouvrir_plan(c);
        }
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
            // Un outil d'édition : la carte
            if self.trois_d.is_some() && self.outil != VOIR && self.outil != SELECTION {
                self.basculer_3d();
            }
            if self.trois_d.is_some() || self.camera3d.is_some() {
                let placage = if self.maquette3d { crate::contextes::matieres::dossier(c).and_then(|d| crate::placage::textures(&d)) } else { None };
                self.dessiner_3d(cx, zone, base, placage);
            } else {
                self.dessiner_carte(cx, zone, base);
            }
            self.panneau(cx, c, panneau, base);
        } else if self.plan.is_some() {
            self.dessiner_plan(cx, c, r, base, k);
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

    #[test]
    fn les_arrets_et_les_lignes_se_decrivent() {
        use sage_reel::transports::*;
        let pas = 64.0;
        let n = cote(pas);
        let mut lieu = Lieu::new("Essai", 49.0, 1.0, 0.0, 0.0, pas, 0.0, &vec![10.0; n * n], &vec![false; n * n], Vec::new(), Vec::new(), Vec::new());
        let ligne = |numero: &str, arrets: Vec<u32>| Ligne { mode: BUS, numero: numero.into(), nom: String::new(), couleur: None, arrets, trace: vec![Morceau { ou: SOL, points: vec![[0.0, 0.0], [100.0, 0.0]] }] };
        lieu.transports = Transports {
            tunnels: vec![Route { genre: sage_reel::ROUTE, largeur: 6.0, points: vec![[0.0, 200.0, 4.0], [100.0, 200.0, 4.0]] }],
            rails: vec![Rail { mode: METRO, ou: TUNNEL, niveau: -2, points: vec![[0.0, 400.0, 2.0], [100.0, 400.0, 2.0]] }],
            arrets: vec![Arret { modes: 1 << BUS, x: 50.0, z: 0.0, nom: "Mairie".into(), abri: true, niveau: 0 }, Arret { modes: 1 << METRO, x: 50.0, z: 410.0, nom: "Théâtre".into(), abri: false, niveau: -1 }],
            lignes: vec![ligne("10", vec![0]), ligne("2", vec![0]), ligne("2", vec![0]), Ligne { mode: METRO, ..ligne("M", vec![]) }],
        };
        let d = Donnees::new(lieu);
        let v = Lieux::decrire(&d, 55.0, 3.0);
        assert!(v.iter().any(|l| l == "arrêt Mairie · Bus · abri"), "{v:?}");
        assert!(v.iter().any(|l| l == "lignes : Bus 10, Bus 2"), "{v:?}");
        let v = Lieux::decrire(&d, 50.0, 201.0);
        assert!(v.iter().any(|l| l.starts_with("tunnel · ") && l.ends_with("6,0 m sous le sol")), "{v:?}");
        let v = Lieux::decrire(&d, 50.0, 401.0);
        assert!(v.iter().any(|l| l == "voie ferrée · Métro (tunnel) · niveau -2"), "{v:?}");
        assert!(v.iter().any(|l| l == "arrêt Théâtre · Métro · sous terre (niveau -1)"), "{v:?}");
        // Une ligne par numéro, l'aller et le retour ensemble, dans l'ordre naturel
        let g = Lieux::groupes_lignes(&d.lieu.transports);
        assert_eq!(g.iter().map(|e| (e.0, e.1.as_str(), e.2.len())).collect::<Vec<_>>(), vec![(BUS, "2", 2), (BUS, "10", 1), (METRO, "M", 1)]);
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

/// « 1 tuile », « 3 tuiles ».
fn pluriel(n: usize, un: &str, plusieurs: &str) -> String {
    format!("{} {}", milliers(n), if n > 1 { plusieurs } else { un })
}

/// « 12 345 » (espace fine insécable entre les milliers).
fn milliers(n: usize) -> String {
    let t = n.to_string();
    let mut r = String::new();
    for (i, c) in t.chars().enumerate() {
        if i > 0 && (t.len() - i) % 3 == 0 {
            r.push('\u{202f}');
        }
        r.push(c);
    }
    r
}

/// Une surface en m², ha ou km².
fn surface(m2: f64) -> String {
    if m2 < 10_000.0 {
        format!("{} m²", milliers(m2.round() as usize))
    } else if m2 < 1_000_000.0 {
        format!("{:.1} ha", m2 / 10_000.0).replace('.', ",")
    } else {
        format!("{:.2} km²", m2 / 1_000_000.0).replace('.', ",")
    }
}

/// Une taille en octets : Ko, Mo.
fn octets(n: usize) -> String {
    if n < 1024 * 1024 {
        format!("{:.0} Ko", n as f64 / 1024.0)
    } else {
        format!("{:.1} Mo", n as f64 / (1024.0 * 1024.0)).replace('.', ",")
    }
}

/// Le suivi des tâches (`suivi`) dans `p`, par-dessus la vue 3D (`zone`, la découpe à
/// rendre) : le repère, les étapes en cours avec leur durée qui court, les plus
/// coûteuses (cumuls) et les dernières finies, imbriquées.
fn dessiner_suivi(cx: &mut Cx, p: Rect, zone: Rect) {
    use crate::suivi;
    let d = cx.dp;
    let (taille, pas) = (11.0, 15.0 * d);
    let w = p.w.min(460.0 * d);
    if p.h < 4.0 * pas || w < 120.0 * d {
        return;
    }
    let p = Rect::new(p.x + p.w - w, p.y, w, p.h);
    let c = cx.couleurs();
    let rayon = 8.0 * d;
    cx.ombre(p, rayon, 6.0);
    cx.dl.rrect(p, rayon, [c.surface[0], c.surface[1], c.surface[2], 235]);
    let interieur = Rect::new(p.x + 8.0 * d, p.y + 6.0 * d, p.w - 16.0 * d, p.h - 12.0 * d);
    cx.clip(Some(interieur));
    let mut y = interieur.y + pas * 0.5;
    let x = interieur.x;
    let max_y = interieur.y + interieur.h - pas * 0.5;
    let ligne = |cx: &mut Cx, y: &mut f32, t: &str, couleur: crate::draw::Color| -> bool {
        if *y > max_y + 0.5 {
            return false;
        }
        cx.mono_at(x, *y, t, taille, couleur);
        *y += pas;
        true
    };
    let (nom, depuis) = suivi::depuis_repere();
    ligne(cx, &mut y, &format!("Suivi · « {nom} » il y a {}", suivi::duree(depuis as f32)), c.accent);
    let en_cours = suivi::en_cours();
    if en_cours.is_empty() {
        ligne(cx, &mut y, "en cours : rien", c.pale);
    }
    for (e, t) in en_cours.iter().take(8) {
        let det = if e.detail.is_empty() { String::new() } else { format!(" ({})", e.detail) };
        ligne(cx, &mut y, &format!("▶ {} {}… [{}]{det}", e.chemin, suivi::duree(*t), e.fil), c.texte);
    }
    // Les plus coûteuses, depuis le repère
    let cumuls = suivi::cumuls();
    if !cumuls.is_empty() {
        ligne(cx, &mut y, "cumuls : total · n · max · 1re", c.accent);
        for (k, s) in cumuls.iter().take(6) {
            ligne(cx, &mut y, &format!("{} · {}× · {} · {}  {k}", suivi::duree(s.total as f32), s.n, suivi::duree(s.max), suivi::duree(s.premiere)), c.texte);
        }
    }
    // Les dernières finies, autant que la place en laisse
    let reste = ((max_y + 0.5 - y) / pas).floor().max(0.0) as usize;
    if reste > 1 {
        ligne(cx, &mut y, "finies :", c.accent);
        for f in suivi::dernieres(reste - 1) {
            let nom = f.chemin.rsplit(" › ").next().unwrap_or(&f.chemin);
            let det = if f.detail.is_empty() { String::new() } else { format!(" ({})", f.detail) };
            let fil = if f.profondeur == 0 { format!(" [{}]", f.fil) } else { String::new() };
            let t = format!("{:+8.2} {:>9} {}{nom}{det}{fil}", f.debut, suivi::duree(f.duree), "  ".repeat(f.profondeur));
            let couleur = if f.duree >= 1.0 { c.accent } else if f.profondeur == 0 { c.texte } else { c.pale };
            if !ligne(cx, &mut y, &t, couleur) {
                break;
            }
        }
    }
    cx.clip(Some(zone));
}

/// L'attente de la zone fine, au milieu de la vue : les tuiles matérialisées sur
/// celles de la zone, une barre, le temps passé et ce qui reste à peu près.
fn dessiner_attente_zone(cx: &mut Cx, zone: Rect, e: &crate::relief3d::EtatZone, rayon: f32) {
    let c = cx.couleurs();
    let pad = cx.pad();
    let titre = "Matérialisation en préparation";
    let ligne = if e.trop {
        format!("{} tuiles : trop pour la mémoire (au plus {}). Réduisez le rayon.", e.total, crate::relief3d::ZONE_MAX)
    } else {
        format!("{} / {} tuiles matérialisées · rayon {:.0} m", e.faites, e.total, rayon)
    };
    let temps = if e.trop {
        String::new()
    } else {
        let duree = |s: f32| if s >= 90.0 { format!("{:.0} min {:02.0} s", (s / 60.0).floor(), s % 60.0) } else { format!("{s:.0} s") };
        match e.reste {
            Some(r) => format!("{} écoulées · encore ~{}", duree(e.ecoule), duree(r)),
            None => format!("{} écoulées", duree(e.ecoule)),
        }
    };
    let w = (cx.measure(&ligne, 14.0).max(cx.measure(titre, 16.0)).max(cx.measure(&temps, 13.0)) + 4.0 * pad).min(zone.w - 2.0 * pad);
    let h = 3.0 * cx.commande();
    let p = Rect::new(zone.cx() - w / 2.0, zone.cy() - h / 2.0, w, h);
    let rayon_coin = cx.theme.rayon(cx.commande());
    cx.ombre(p, rayon_coin, 8.0);
    cx.dl.rrect(p, rayon_coin, c.surface);
    let x = p.x + 2.0 * pad;
    cx.text_at(x, p.y + h * 0.20, titre, 16.0, c.texte);
    cx.text_at(x, p.y + h * 0.45, &ligne, 14.0, if e.trop { c.accent } else { c.texte });
    cx.text_at(x, p.y + h * 0.66, &temps, 13.0, c.pale);
    if !e.trop {
        let (bx, bw, bh) = (x, p.w - 4.0 * pad, (4.0 * cx.dp).round());
        let by = p.y + h * 0.84;
        cx.dl.rrect(Rect::new(bx, by, bw, bh), bh / 2.0, c.puce);
        let f = e.faites as f32 / e.total.max(1) as f32;
        cx.dl.rrect(Rect::new(bx, by, (bw * f).max(bh), bh), bh / 2.0, c.accent);
    }
}

