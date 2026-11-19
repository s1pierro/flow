//! La vue 3D d'un lieu : son relief en tuiles à niveaux de détail, chacune drapée de la
//! tuile de la carte 2D de même clé (déjà dessinée : sol, eau, routes, bâtiments,
//! ombrage).
//!
//! - **Les tuiles** suivent la pyramide de la carte (`carte::boite_tuile`) : la tuile
//!   (z, i, j) a toujours `COTES` × `COTES` cases, quelle que soit sa taille. Près de
//!   l'œil, on descend jusqu'au niveau où une case vaut à peu près un pas de la grille
//!   du lieu ; au loin, de grandes tuiles grossières.
//! - **Le maillage en passes** (`Decoupe`) : chaque triangle de la grille est coupé là
//!   où un champ passe par 0, les points de coupe partagés (la couture est étanche).
//!   D'abord **l'eau** : la terre descend jusqu'à la ligne d'eau, et une surface plane
//!   au niveau de l'eau remplace le lit. Puis **les voies**, près de l'œil (cases de
//!   `VOIRIE_CASE` mètres au plus) : le sol terrassé le long de leur tracé, coupé au
//!   bord des rubans de la voirie, qui s'y cousent (`voirie3d`).
//! - **Les jupes** : chaque tuile a un rebord qui descend sous elle, ce qui cache les
//!   fentes entre deux niveaux voisins.
//! - **Les maillages** se font au fil des images (quelques millisecondes par image),
//!   les grossiers d'abord ; en attendant, l'ancêtre le plus proche déjà fait. Une
//!   retouche du lieu (`Carte::retoucher`) les périme : ils se refont, l'ancien restant
//!   visible jusque-là. Ceux qui ne servent plus s'oublient.
//! - **Les textures** sont les tuiles de la carte (`Carte::textures`), demandées aux
//!   mêmes threads ; en attendant, la partie d'un ancêtre.
//! - **Les bâtiments** se montent en volume (`toits::monter` : murs et pans du toit)
//!   près de l'œil, et avec eux les tabliers des ponts (`tabliers::monter` : une dalle
//!   un peu plus large que la chaussée, une seule pour des ponts parallèles voisins),
//!   à moins de `BATI_RAYON`, par tuiles d'environ 300 m : chacun dans celle qui
//!   contient son centre. Ils se font dans le même temps que le relief, après
//!   lui, et se périment de même.

use crate::carte::{boite_tuile, Carte, Cle, Donnees, NIVEAU_MAX};
use crate::draw::{Maillage3d, Morceau3d, Rect, Terrain3d};
use crate::sculpture::{produit, Camera, CHAMP};
use crate::widgets::Cx;
use sage_reel::{teintes_bati, toits};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Cases par côté d'une tuile.
pub const COTES: usize = 32;
/// Les flottants d'un sommet du relief : position, normale, texture (u, v), matière.
pub const SOMMET: usize = 9;
/// Les matières du relief : la terre, l'eau, la chaussée, ses bords (le rendu blanc les teinte).
pub const TERRE: f32 = 0.0;
pub const EAU: f32 = 1.0;
pub const CHAUSSEE: f32 = 2.0;
/// Les trottoirs et accotements, de part et d'autre de la chaussée.
pub const BORD: f32 = 3.0;
/// Une tuile se divise quand l'œil est à moins de `PRES` fois son côté.
const PRES: f32 = 2.4;
/// Le temps donné aux maillages à chaque image, en secondes.
const BUDGET: f64 = 0.008;
/// Les maillages gardés au plus.
const GARDES: usize = 600;
/// Les bâtiments se montent jusqu'à cette distance de l'œil (m).
pub const BATI_RAYON: f32 = 2000.0;
/// Le côté visé des tuiles de bâtiments (m).
const BATI_TUILE: f32 = 300.0;
/// Le ciel, et la brume au loin.
pub const CIEL: [f32; 3] = [0.80, 0.86, 0.92];

static INSTANCES: AtomicU64 = AtomicU64::new(1);

struct Morceau {
    maillage: Maillage3d,
    /// La retouche jusqu'à laquelle il est à jour, et la dernière image où il a servi.
    faite: u64,
    vu: u64,
    /// La version de la voirie avec laquelle il a été fait.
    voirie: u64,
    /// Ses arêtes, pour le rendu blanc (faites à la première demande).
    aretes: Option<Maillage3d>,
}

impl Morceau {
    fn aretes(&mut self, flottants: usize, pli: bool) -> Maillage3d {
        let m = &self.maillage;
        self.aretes
            .get_or_insert_with(|| Maillage3d { id: m.id ^ (1 << 63), revision: m.revision, sommets: m.sommets.clone(), indices: Arc::new(aretes(&m.sommets, flottants, &m.indices, pli)) })
            .clone()
    }
}

/// La vue 3D d'un lieu : sa caméra, ses maillages.
pub struct Relief3d {
    pub camera: Camera,
    morceaux: HashMap<Cle, Morceau>,
    batis: HashMap<Cle, Morceau>,
    image: u64,
    num: u64,
    revision: u64,
    /// Les tuiles dessinées à la dernière image, et les tuiles de bâtiments (pour les
    /// essais et la barre).
    pub dessinees: usize,
    pub bati_dessinees: usize,
    /// Des maillages restent à faire.
    pub en_attente: bool,
    /// La génération en cours : (tuiles faites, tuiles du lot), pour l'indicateur ;
    /// `None` quand tout est fait. Le lot court depuis la dernière fois que tout
    /// était fait, et grandit si la vue en demande d'autres en route.
    pub generation: Option<(usize, usize)>,
    /// Le rendu blanc : faces blanches, arêtes tracées.
    pub blanc: bool,
    /// En maquette : les volumes de défonce des lits, montrés en fil de fer et en
    /// surface semi-transparente.
    pub defonce: bool,
    /// Le rendu Maquette (`maquette`) : des tuiles en volume, fabriquées à part.
    pub maquette: bool,
    fabrique: Option<crate::maquette::Fabrique>,
    /// La fabrique et les tuiles faites sont celles de la Matérialisation.
    fabrique_materialise: bool,
    tuiles_maquette: HashMap<Cle, Faite>,
    /// Les tuiles sélectionnées de la maquette (surlignées de `accent`), et s'il ne
    /// faut montrer qu'elles.
    pub selection: std::collections::BTreeSet<Cle>,
    pub seules: bool,
    /// Les textures des matières de la maquette.
    pub placage: Option<crate::draw::Placage3d>,
    /// La version du banc (`banc::version_lieux`) des tuiles faites : quand elle change
    /// (un module rechargé, Lieux mis sur le banc ou retiré), elles sont refaites.
    banc: u64,
    pub accent: [f32; 3],
    /// Les tuiles de la maquette montrées à la dernière image (pour le toucher).
    montrees: Vec<Cle>,
    /// Le réseau des voies en surimpression, par tuile (`traces_reseau`), pour le réseau
    /// d'adresse donnée ; et si son calcul a été lancé.
    traces: HashMap<Cle, Vec<(Maillage3d, [f32; 3], f32)>>,
    traces_de: usize,
    reseau_lance: bool,
    /// Les réglages des tracés (commande pilote `traces`) : la distance de la caméra à
    /// sa cible en deçà de laquelle ils paraissent, et le nombre de tuiles sélectionnées
    /// au plus ; et ce que la dernière image a tracé (tuiles, segments, d'où).
    pub traces_tuiles: usize,
    pub traces_bilan: String,
    /// Les réglages de la surimpression (tracés et remblais), donnés par Lieux.
    pub sur: Surimpression,
    /// Les faces en surimpression du module du banc, par tuile (vide : rien).
    faces: HashMap<Cle, Option<Maillage3d>>,
    /// Les hauteurs basse et haute du relief de chaque tuile (`bornes`), à la retouche `bornes_de`.
    bornes: std::cell::RefCell<HashMap<Cle, (f32, f32)>>,
    bornes_de: u64,
    /// La zone de la vue Matérialisation (`ZoneFine`), si on y est : seules ses tuiles, toutes au
    /// niveau le plus fin, montrées une fois toutes faites.
    pub zone: Option<ZoneFine>,
    /// Ses tuiles, pour la zone et la retouche de la carte données.
    zone_cles: Option<(ZoneFine, u64, Vec<Cle>)>,
    /// Son avancement tant qu'elle n'est pas prête, pour l'attente.
    pub zone_etat: Option<EtatZone>,
    zone_debut: Option<(ZoneFine, std::time::Instant, usize)>,
}

/// La zone de la vue Matérialisation : un disque de `rayon` mètres autour de `centre` (x, z), où seules les
/// tuiles du niveau le plus fin sont montrées, toutes ensemble une fois faites ; la
/// caméra ne s'en éloigne pas plus qu'il ne faut pour la voir entière.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ZoneFine {
    pub centre: [f32; 2],
    pub rayon: f32,
}

/// Les tuiles d'une zone fine au plus (au-delà, on ne la matérialise pas : la mémoire).
pub const ZONE_MAX: usize = 400;

/// Où en est la zone fine qui se prépare.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EtatZone {
    pub faites: usize,
    pub total: usize,
    /// Les secondes depuis le début, et l'estimation de ce qui reste (après la
    /// première tuile faite).
    pub ecoule: f32,
    pub reste: Option<f32>,
    /// Trop de tuiles : rien n'est lancé.
    pub trop: bool,
}

/// Le niveau le plus fin des tuiles du lieu.
pub fn niveau_fin(d: &Donnees) -> u8 {
    let (_, _, c) = d.monde();
    ((c / (COTES as f32 * d.lieu.pas as f32)).log2().floor() as i32).clamp(0, NIVEAU_MAX as i32) as u8
}

/// Les tuiles du niveau le plus fin qui touchent la zone `z` (et le lieu), de la plus
/// proche du centre à la plus lointaine.
pub fn tuiles_zone(d: &Donnees, z: ZoneFine) -> Vec<Cle> {
    let n = niveau_fin(d);
    let (x0, z0, c) = d.monde();
    let t = c / (1u32 << n) as f32;
    let e = d.lieu.etendue();
    let max = (1i32 << n) - 1;
    let [cx, cz] = z.centre;
    let r = z.rayon.max(1.0);
    let (i0, i1) = ((((cx - r - x0) / t).floor() as i32).max(0), (((cx + r - x0) / t).floor() as i32).min(max));
    let (j0, j1) = ((((cz - r - z0) / t).floor() as i32).max(0), (((cz + r - z0) / t).floor() as i32).min(max));
    let mut v = Vec::new();
    for j in j0..=j1 {
        for i in i0..=i1 {
            let cle = (n, i, j);
            let b = boite_tuile(d, cle);
            if (b[0] as f64) > e[2] || (b[2] as f64) < e[0] || (b[1] as f64) > e[3] || (b[3] as f64) < e[1] {
                continue;
            }
            let dx = (b[0] - cx).max(0.0).max(cx - b[2]);
            let dz = (b[1] - cz).max(0.0).max(cz - b[3]);
            let dist = dx.hypot(dz);
            if dist <= r {
                v.push((dist, cle));
            }
        }
    }
    v.sort_by(|a, b| a.0.total_cmp(&b.0));
    v.into_iter().map(|x| x.1).collect()
}

/// Une tuile de la maquette reçue de la fabrique.
struct Faite {
    morceau: Morceau,
    image: crate::draw::Image,
    contour: Maillage3d,
    /// Sa surface d'eau, s'il y en a.
    eau: Option<Maillage3d>,
    /// Le volume de défonce de ses lits, s'il y en a : faces et arêtes.
    defonce: Option<(Maillage3d, Maillage3d)>,
    mesure: crate::maquette::Mesure,
}

impl Relief3d {
    /// Vue de biais, depuis le sud, sur le centre du lieu.
    pub fn new(d: &Donnees) -> Relief3d {
        let e = d.lieu.etendue();
        let (cx, cz) = (((e[0] + e[2]) / 2.0) as f32, ((e[1] + e[3]) / 2.0) as f32);
        let cote = (e[2] - e[0]).max(e[3] - e[1]) as f32;
        let camera = Camera { lacet: 0.0, tangage: 0.55, distance: cote * 0.7, cible: [cx, d.lieu.relief(cx as f64, cz as f64) as f32, cz] };
        Relief3d { camera, morceaux: HashMap::new(), batis: HashMap::new(), image: 0, num: INSTANCES.fetch_add(1, Ordering::Relaxed), revision: 0, dessinees: 0, bati_dessinees: 0, en_attente: false, generation: None, blanc: false, defonce: false, maquette: false, fabrique: None, tuiles_maquette: HashMap::new(), selection: Default::default(), seules: false, placage: None, banc: 0, accent: [0.2, 0.4, 0.9], montrees: Vec::new(), traces: HashMap::new(), traces_de: 0, reseau_lance: false, traces_tuiles: TRACES_TUILES, traces_bilan: String::new(), faces: HashMap::new(), sur: Surimpression::default(), bornes: Default::default(), bornes_de: 0, fabrique_materialise: false, zone: None, zone_cles: None, zone_etat: None, zone_debut: None }
    }

    /// La projection : proche et lointain selon la distance, assez loin pour l'horizon.
    pub fn mvp(&self, aspect: f32) -> [f32; 16] {
        let fy = 1.0 / (CHAMP / 2.0).tan();
        let d = self.camera.distance;
        let (pres, loin) = ((0.01 * d).max(0.5), (80.0 * d).max(60_000.0));
        let mut p = [0.0; 16];
        p[0] = fy / aspect;
        p[5] = fy;
        p[10] = (loin + pres) / (pres - loin);
        p[11] = -1.0;
        p[14] = 2.0 * loin * pres / (pres - loin);
        produit(&p, &self.camera.vue())
    }

    /// Pose la cible sur le sol et garde l'œil au-dessus.
    pub fn poser(&mut self, d: &Donnees) {
        let l = &d.lieu;
        let e = l.etendue();
        let c = &mut self.camera;
        c.cible[0] = c.cible[0].clamp(e[0] as f32, e[2] as f32);
        c.cible[2] = c.cible[2].clamp(e[1] as f32, e[3] as f32);
        c.cible[1] = l.relief(c.cible[0] as f64, c.cible[2] as f64) as f32;
        let cote = (e[2] - e[0]).max(e[3] - e[1]) as f32;
        c.distance = c.distance.clamp(20.0, cote * 3.0);
        c.tangage = c.tangage.clamp(0.02, 1.5);
        for _ in 0..40 {
            let o = c.oeil();
            if o[1] > l.relief(o[0] as f64, o[2] as f64) as f32 + 3.0 || c.tangage >= 1.5 {
                break;
            }
            c.tangage += 0.02;
        }
    }

    /// Un doigt glisse de (dx, dy) pixels : tourner autour.
    pub fn orbiter(&mut self, dx: f32, dy: f32, dp: f32) {
        let k = 0.006 / dp.max(0.1);
        self.camera.lacet -= dx * k;
        self.camera.tangage = (self.camera.tangage + dy * k).clamp(0.02, 1.5);
    }

    /// Une manette (celle d'une télécommande) pendant `dt` s : le stick gauche avance
    /// et décale au sol (d'autant plus vite que la vue est haute), le droit tourne
    /// autour de la cible et la penche, les gâchettes rapprochent (RT) et éloignent
    /// (LT), la croix fait de même par crans plus doux.
    pub fn piloter(&mut self, m: &crate::telecommande::Manette, dt: f32) {
        use crate::telecommande::bouton;
        let morte = |v: f32| if v.abs() < 0.12 { 0.0 } else { (v - 0.12 * v.signum()) / 0.88 };
        let c = &mut self.camera;
        let (gx, gy) = (morte(m.gauche[0]), morte(m.gauche[1]));
        let vitesse = c.distance * 0.9 * if m.appuye(bouton::A) { 3.0 } else { 1.0 };
        let (sl, cl) = c.lacet.sin_cos();
        // Droite (cl, 0, -sl) ; devant au sol (-sl, 0, -cl) ; pousser vers le haut avance
        let (avant, droite) = (-gy * vitesse * dt, gx * vitesse * dt);
        c.cible[0] += droite * cl - avant * sl;
        c.cible[2] += -droite * sl - avant * cl;
        c.lacet -= morte(m.droit[0]) * 1.8 * dt;
        c.tangage = (c.tangage - morte(m.droit[1]) * 1.1 * dt).clamp(0.02, 1.5);
        let mut zoom = m.rt - m.lt;
        if m.appuye(bouton::HAUT) {
            zoom += 0.4;
        }
        if m.appuye(bouton::BAS) {
            zoom -= 0.4;
        }
        c.distance = (c.distance * (-zoom * 1.6 * dt).exp()).clamp(5.0, 200_000.0);
    }

    /// Deux doigts : rapprocher de `k` (écart relatif), glisser de (dx, dy) pixels.
    pub fn pincer(&mut self, k: f32, dx: f32, dy: f32, h: f32) {
        // Le déplacement dans le plan du sol, à l'échelle de la cible
        let px = self.camera.taille_a(self.camera.cible, 1.0, h);
        let (sl, cl) = self.camera.lacet.sin_cos();
        // Droite (cl, 0, -sl) ; devant au sol (-sl, 0, -cl)
        let (avant, droite) = (dy * px / self.camera.tangage.sin().max(0.3), -dx * px);
        self.camera.cible[0] += droite * cl - avant * sl;
        self.camera.cible[2] += -droite * sl - avant * cl;
        self.camera.distance /= k.max(1e-3);
    }

    /// Les tuiles à montrer depuis l'œil, de la plus proche à la plus lointaine.
    fn choisir(&self, d: &Donnees, mvp: &[f32; 16]) -> Vec<Cle> {
        let l = &d.lieu;
        let (_, _, c) = d.monde();
        let e = l.etendue();
        let zmax = ((c / (COTES as f32 * l.pas as f32)).log2().floor() as i32).clamp(0, NIVEAU_MAX as i32) as u8;
        let o = self.camera.oeil();
        let mut bornes = self.bornes.borrow_mut();
        let mut v = Vec::new();
        let mut pile = vec![(0u8, 0i32, 0i32)];
        while let Some(cle) = pile.pop() {
            let b = boite_tuile(d, cle);
            if (b[0] as f64) > e[2] || (b[2] as f64) < e[0] || (b[1] as f64) > e[3] || (b[3] as f64) < e[1] {
                continue;
            }
            // Les hauteurs de la tuile elle-même (pas celles de tout le lieu), avec de
            // quoi couvrir les lits défoncés, les remblais et les ponts
            let (hmin, hmax) = bornes_tuile(d, cle, &mut bornes);
            let (hmin, hmax) = (hmin - BORNE_BAS, hmax + BORNE_HAUT);
            if dehors(mvp, b, hmin, hmax) {
                continue;
            }
            let cote = b[2] - b[0];
            let dx = (b[0] - o[0]).max(0.0).max(o[0] - b[2]);
            let dz = (b[1] - o[2]).max(0.0).max(o[2] - b[3]);
            let dy = (o[1] - hmax).max(0.0).max(hmin - o[1]);
            let dist = (dx * dx + dy * dy + dz * dz).sqrt();
            if cle.0 < zmax && dist < PRES * cote {
                let (z, i, j) = cle;
                for (a, b) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    pile.push((z + 1, i * 2 + a, j * 2 + b));
                }
            } else {
                v.push((dist, cle));
            }
        }
        v.sort_by(|a, b| a.0.total_cmp(&b.0));
        v.into_iter().map(|(_, c)| c).collect()
    }

    /// Les tuiles de bâtiments à montrer (à moins de `BATI_RAYON` de l'œil, dans le
    /// champ), de la plus proche à la plus lointaine.
    fn choisir_bati(&self, d: &Donnees, mvp: &[f32; 16]) -> Vec<Cle> {
        let l = &d.lieu;
        let (x0, z0, c) = d.monde();
        let e = l.etendue();
        let z = niveau_bati(c);
        let t = c / (1u32 << z) as f32;
        let (hmin, hmax) = (l.base, l.base + 65535.0 * l.echelle + 80.0);
        let o = self.camera.oeil();
        let n = (1i32 << z) - 1;
        let (i0, i1) = (((o[0] - BATI_RAYON - x0) / t).floor().max(0.0) as i32, (((o[0] + BATI_RAYON - x0) / t).floor() as i32).min(n));
        let (j0, j1) = (((o[2] - BATI_RAYON - z0) / t).floor().max(0.0) as i32, (((o[2] + BATI_RAYON - z0) / t).floor() as i32).min(n));
        let mut v = Vec::new();
        for j in j0..=j1 {
            for i in i0..=i1 {
                let cle = (z, i, j);
                let b = boite_tuile(d, cle);
                if (b[0] as f64) > e[2] || (b[2] as f64) < e[0] || (b[1] as f64) > e[3] || (b[3] as f64) < e[1] || dehors(mvp, b, hmin, hmax) {
                    continue;
                }
                let dx = (b[0] - o[0]).max(0.0).max(o[0] - b[2]);
                let dz = (b[1] - o[2]).max(0.0).max(o[2] - b[3]);
                let dy = (o[1] - hmax).max(0.0);
                let dist = (dx * dx + dy * dy + dz * dz).sqrt();
                if dist < BATI_RAYON {
                    v.push((dist, cle));
                }
            }
        }
        v.sort_by(|a, b| a.0.total_cmp(&b.0));
        v.into_iter().map(|(_, c)| c).collect()
    }

    /// Tient le compte de la génération : `finies` tuiles faites à cette image, `restantes`
    /// encore à faire ; le lot repart de zéro quand il ne reste rien.
    fn compter(&mut self, restantes: usize, finies: usize) {
        if restantes == 0 {
            self.generation = None;
            return;
        }
        let (faites, total) = self.generation.unwrap_or((0, 0));
        let faites = faites + finies;
        self.generation = Some((faites, total.max(faites + restantes)));
    }

    /// Dessine le lieu de `carte` dans `zone` en Maquette : les tuiles demandées à la
    /// fabrique (la plus proche de la cible d'abord, chacune après ses ancêtres absents), quelques-unes
    /// prises par image ; en attendant, l'ancêtre le plus proche déjà fait.
    fn dessiner_maquette(&mut self, cx: &mut Cx, zone: Rect, carte: &mut Carte) {
        let _suivi = crate::suivi::tache_min("image maquette", 25.0);
        let ciel = CIEL.map(|c| (c * 255.0) as u8);
        cx.dl.rrect(zone, 0.0, [ciel[0], ciel[1], ciel[2], 255]);
        let donnees = carte.donnees.clone();
        let synchrone = cfg!(test) || std::env::var_os("FLOW_MAQUETTE_SYNC").is_some();
        // La Matérialisation (une zone) a ses propres tuiles : en changeant de vue, la
        // fabrique et les tuiles faites sont refaites
        let materialiser = self.zone.is_some();
        if materialiser != self.fabrique_materialise {
            self.fabrique_materialise = materialiser;
            self.fabrique = None;
            self.tuiles_maquette.clear();
        }
        if self.fabrique.is_none() && !synchrone {
            crate::suivi::repere("maquette", false);
            self.fabrique = Some(crate::maquette::Fabrique::pour(donnees.clone(), materialiser));
        }
        if self.bornes_de != carte.compteur {
            self.bornes_de = carte.compteur;
            self.bornes.borrow_mut().clear();
        }
        let v = crate::banc::version_lieux();
        if v != self.banc {
            self.banc = v;
            self.tuiles_maquette.clear();
            self.traces.clear();
            self.faces.clear();
        }
        let attente = crate::suivi::tache_min("attente des données (image)", 5.0);
        let Ok(d) = donnees.read() else { return };
        drop(attente);
        // La zone fine : la cible reste dans le disque, la caméra assez près pour le
        // voir entier (le champ le plus étroit, en largeur en portrait)
        let aspect = zone.w / zone.h.max(1.0);
        if let Some(z) = self.zone {
            let c = &mut self.camera;
            let (dx, dz) = (c.cible[0] - z.centre[0], c.cible[2] - z.centre[1]);
            let l = dx.hypot(dz);
            if l > z.rayon {
                c.cible[0] = z.centre[0] + dx / l * z.rayon;
                c.cible[2] = z.centre[1] + dz / l * z.rayon;
            }
            let demi = (CHAMP / 2.0).tan() * aspect.min(1.0);
            c.distance = c.distance.min(z.rayon * 1.15 / demi.max(0.05)).max(20.0);
        }
        self.poser(&d);
        let mvp = self.mvp(aspect);
        let zone_cles: Option<Vec<Cle>> = self.zone.map(|z| {
            if self.zone_cles.as_ref().is_none_or(|(zz, n, _)| *zz != z || *n != carte.compteur) {
                self.zone_cles = Some((z, carte.compteur, tuiles_zone(&d, z)));
            }
            self.zone_cles.as_ref().map(|x| x.2.clone()).unwrap_or_default()
        });
        let trop = zone_cles.as_ref().is_some_and(|v| v.len() > ZONE_MAX);
        // Les seules tuiles sélectionnées, à leur niveau ; sinon, celles de la vue ; en
        // zone fine, les siennes (aucune s'il y en a trop)
        let seules = (self.seules && !self.selection.is_empty()) || zone_cles.is_some();
        let voulues: Vec<Cle> = if let Some(v) = &zone_cles {
            if trop { Vec::new() } else { v.clone() }
        } else if seules {
            let mut v: Vec<Cle> = self.selection.iter().copied().collect();
            v.sort();
            v
        } else {
            // De la plus proche du point visé (la cible, au sol) à la plus lointaine
            let c = self.camera.cible;
            let mut v: Vec<(f32, Cle)> = self.choisir(&d, &mvp).into_iter().map(|cle| {
                let b = boite_tuile(&d, cle);
                let dx = (b[0] - c[0]).max(0.0).max(c[0] - b[2]);
                let dz = (b[1] - c[2]).max(0.0).max(c[2] - b[3]);
                (dx * dx + dz * dz, cle)
            }).collect();
            v.sort_by(|a, b| a.0.total_cmp(&b.0));
            v.into_iter().map(|x| x.1).collect()
        };
        let perime = |t: &HashMap<Cle, Faite>, cle: Cle| t.get(&cle).is_none_or(|m| !carte.a_jour(m.morceau.faite, boite_tuile(&d, cle)));
        // Dans l'ordre des voulues : chacune précédée de ses ancêtres absents, du plus
        // grossier au plus fin (0/0/0, le fond, vient donc en premier)
        let mut a_faire: Vec<Cle> = Vec::new();
        for &cle in &voulues {
            if perime(&self.tuiles_maquette, cle) {
                let (z, i, j) = cle;
                for k in (1..=z).rev().filter(|_| !seules) {
                    let a = (z - k, i >> k, j >> k);
                    if !self.tuiles_maquette.contains_key(&a) {
                        a_faire.push(a);
                    }
                }
                a_faire.push(cle);
            }
        }
        // Les sélectionnées hors de la vue, après (pour leur contour et leur mesure)
        for &cle in &self.selection {
            if perime(&self.tuiles_maquette, cle) {
                a_faire.push(cle);
            }
        }
        let mut vues = HashSet::new();
        let a_faire: Vec<(Cle, u64)> = a_faire.into_iter().filter(|&x| vues.insert(x)).map(|x| (x, carte.compteur)).collect();
        // Les tuiles finies : quelques-unes par image (l'interface d'abord)
        let finies = match &self.fabrique {
            Some(f) => {
                f.demander(a_faire.clone());
                let finies = f.prendre(3);
                self.en_attente = f.occupee();
                finies
            }
            None => {
                // Essais et aperçus : tout de suite
                let v: Vec<crate::maquette::Tuile> = a_faire.iter().map(|&(cle, faite)| crate::maquette::tuile_pour(&d, cle, faite, materialiser)).collect();
                self.en_attente = false;
                v
            }
        };
        drop(d);
        self.compter(a_faire.len().saturating_sub(finies.len()), finies.len());
        for t in finies {
            self.revision += 1;
            let id = 0x6d61_7100_0000_0000u64 ^ (self.num << 40) ^ ((t.cle.0 as u64) << 32) ^ ((t.cle.1 as u64 & 0xFFFF) << 16) ^ (t.cle.2 as u64 & 0xFFFF);
            let maillage = Maillage3d { id, revision: self.revision, sommets: Arc::new(t.sommets), indices: Arc::new(t.indices) };
            let aretes = Some(Maillage3d { id: id ^ (1 << 63), revision: self.revision, sommets: maillage.sommets.clone(), indices: Arc::new(t.aretes) });
            let contour = Maillage3d { id: id ^ (1 << 62), revision: self.revision, sommets: maillage.sommets.clone(), indices: Arc::new(t.contour) };
            let image = crate::draw::Image { id, revision: self.revision, largeur: crate::maquette::TEX as u32, hauteur: (crate::maquette::TEX + crate::maquette::BANDE) as u32, pixels: Arc::new(t.pixels) };
            let eau = (!t.eau_indices.is_empty()).then(|| Maillage3d { id: id ^ (1 << 61), revision: self.revision, sommets: Arc::new(t.eau_sommets), indices: Arc::new(t.eau_indices) });
            let defonce = (!t.defonce_indices.is_empty()).then(|| {
                let sommets = Arc::new(t.defonce_sommets);
                (Maillage3d { id: id ^ (1 << 60), revision: self.revision, sommets: sommets.clone(), indices: Arc::new(t.defonce_indices) }, Maillage3d { id: id ^ (1 << 59), revision: self.revision, sommets, indices: Arc::new(t.defonce_aretes) })
            });
            self.tuiles_maquette.insert(t.cle, Faite { morceau: Morceau { maillage, faite: t.faite, vu: self.image, voirie: 0, aretes }, image, contour, eau, defonce, mesure: t.mesure });
        }
        // La zone fine : rien tant que toutes ses tuiles ne sont pas faites (à jour)
        if let Some(v) = &zone_cles {
            let z = self.zone.unwrap();
            let faites = {
                let Ok(d) = donnees.read() else { return };
                let t = &self.tuiles_maquette;
                if trop { 0 } else { v.iter().filter(|&&k| t.get(&k).is_some_and(|m| carte.a_jour(m.morceau.faite, boite_tuile(&d, k)))).count() }
            };
            if self.zone_debut.as_ref().is_none_or(|(zz, _, _)| *zz != z) {
                self.zone_debut = Some((z, std::time::Instant::now(), faites));
            }
            let (_, debut, deja) = self.zone_debut.unwrap();
            let ecoule = debut.elapsed().as_secs_f32();
            let neuves = faites.saturating_sub(deja);
            let reste = (neuves > 0).then(|| ecoule / neuves as f32 * (v.len() - faites) as f32);
            let etat = EtatZone { faites, total: v.len(), ecoule, reste, trop };
            // Gardées en mémoire tant que la zone est là
            for &k in v {
                if let Some(f) = self.tuiles_maquette.get_mut(&k) {
                    f.morceau.vu = self.image;
                }
            }
            if trop || faites < v.len() {
                self.zone_etat = Some(etat);
                self.montrees.clear();
                self.dessinees = 0;
                self.oublier(Some(v));
                return;
            }
        }
        self.zone_etat = None;
        if zone_cles.is_none() {
            self.zone_debut = None;
        }
        let mut montrees: Vec<Cle> = Vec::new();
        let mut vues = HashSet::new();
        for &cle in &voulues {
            let (z, i, j) = cle;
            // En attendant une tuile, l'ancêtre le plus proche déjà fait (pas pour les
            // seules sélectionnées : rien d'autre qu'elles)
            // En dézoomant : ses quatre filles, si elles sont déjà faites, plutôt qu'un
            // ancêtre plus grossier qu'elle
            let filles = [(0, 0), (1, 0), (0, 1), (1, 1)].map(|(a, b)| (z + 1, i * 2 + a, j * 2 + b));
            if !seules && !self.tuiles_maquette.contains_key(&cle) && filles.iter().all(|f| self.tuiles_maquette.contains_key(f)) {
                for f in filles {
                    if vues.insert(f) {
                        montrees.push(f);
                    }
                }
                continue;
            }
            let a = if seules { Some(cle).filter(|c| self.tuiles_maquette.contains_key(c)) } else { (0..=z).map(|k| (z - k, i >> k, j >> k)).find(|a| self.tuiles_maquette.contains_key(a)) };
            if let Some(a) = a {
                if vues.insert(a) {
                    montrees.push(a);
                }
            }
        }
        let (mut morceaux, mut aretes_sol, mut choix, mut eaux) = (Vec::with_capacity(montrees.len()), Vec::new(), Vec::new(), Vec::new());
        let (mut defonces, mut defonces_aretes) = (Vec::new(), Vec::new());
        for cle in &montrees {
            let Some(f) = self.tuiles_maquette.get_mut(cle) else { continue };
            f.morceau.vu = self.image;
            if self.blanc {
                if let Some(a) = &f.morceau.aretes {
                    aretes_sol.push(a.clone());
                }
            }
            morceaux.push(Morceau3d { maillage: f.morceau.maillage.clone(), image: Some(f.image.clone()), uv: [0.0, 0.0, 1.0, 1.0], couleur: [0.72, 0.76, 0.62] });
            if let Some(e) = &f.eau {
                eaux.push(e.clone());
            }
            if let (true, Some((faces, traits))) = (self.defonce, &f.defonce) {
                defonces.push(faces.clone());
                defonces_aretes.push(traits.clone());
            }
        }
        // Le contour des sélectionnées faites, montrées ou non
        for cle in &self.selection {
            if let Some(f) = self.tuiles_maquette.get_mut(cle) {
                f.morceau.vu = self.image;
                choix.push(f.contour.clone());
            }
        }
        // Les tuiles des tracés et celles des faces du banc : sur la sélection seule
        // (`traces_tuiles` au plus) ; sinon, la caméra à moins de sa distance réglée de
        // la cible, les tuiles voulues à moins de `rayon` × la distance de la caméra
        let (pres_traces, pres_faces) = {
            let c = self.camera.cible;
            let r = self.camera.distance;
            let Ok(d) = donnees.read() else { return };
            // En Matérialisation, comme hors sélection : selon la distance et le rayon
            let selection_seule = seules && zone_cles.is_none();
            let choisir = |distance: f32, rayon: f32| -> Vec<Cle> {
                if selection_seule {
                    if voulues.len() <= self.traces_tuiles { voulues.clone() } else { Vec::new() }
                } else if r <= distance {
                    voulues.iter().copied().filter(|&cle| {
                        let b = boite_tuile(&d, cle);
                        let dx = (b[0] - c[0]).max(0.0).max(c[0] - b[2]);
                        let dz = (b[1] - c[2]).max(0.0).max(c[2] - b[3]);
                        dx.hypot(dz) <= r * rayon
                    }).collect()
                } else {
                    Vec::new()
                }
            };
            (if self.sur.traces { choisir(self.sur.traces_distance, self.sur.traces_rayon) } else { Vec::new() }, if self.sur.remblai { choisir(self.sur.remblai_distance, self.sur.remblai_rayon) } else { Vec::new() })
        };
        let faces = self.faces_montrees(&donnees, &pres_faces, carte.compteur);
        let traces = self.traces_montrees(&donnees, &pres_traces, carte.compteur);
        self.montrees = montrees;
        self.dessinees = morceaux.len();
        self.bati_dessinees = 0;
        let dist = self.camera.distance;
        cx.dl.terrain(Terrain3d { rect: zone, mvp, oeil: self.camera.oeil(), ciel: CIEL, brume: [dist * 3.0 + 2000.0, dist * 12.0 + 12_000.0], morceaux, bati: faces, blanc: self.blanc, aretes: aretes_sol, aretes_bati: Vec::new(), choix, accent: self.accent, eaux, defonces, defonces_aretes, placage: self.placage.clone(), traces });
        let zone = zone_cles;
        self.oublier(zone.as_ref());
    }

    /// Au-delà de `GARDES` tuiles de maquette faites, oublie les moins récemment vues
    /// (jamais le fond, la sélection ni la zone fine).
    fn oublier(&mut self, zone: Option<&Vec<Cle>>) {
        let garde = GARDES.max(zone.map_or(0, |z| z.len() + 50));
        if self.tuiles_maquette.len() > garde {
            let sel = &self.selection;
            let dans_zone: HashSet<Cle> = zone.map(|z| z.iter().copied().collect()).unwrap_or_default();
            let mut v: Vec<(u64, Cle)> = self.tuiles_maquette.iter().filter(|(k, _)| k.0 > 0 && !sel.contains(k) && !dans_zone.contains(k)).map(|(k, m)| (m.morceau.vu, *k)).collect();
            v.sort();
            let n = self.tuiles_maquette.len() - garde;
            for (_, k) in v.iter().take(n) {
                self.tuiles_maquette.remove(k);
            }
        }
    }

    /// La tuile de la maquette montrée sous le point (x, y) de `zone`, s'il touche le
    /// terrain.
    pub fn tuile_touchee(&self, d: &Donnees, zone: Rect, x: f32, y: f32) -> Option<Cle> {
        let (nx, ny) = ((x - zone.x) / zone.w.max(1.0) * 2.0 - 1.0, 1.0 - (y - zone.y) / zone.h.max(1.0) * 2.0);
        let p = crate::selection::pointer(d, &self.camera, zone.w / zone.h.max(1.0), nx, ny)?;
        self.montrees.iter().copied().filter(|&c| {
            let b = boite_tuile(d, c);
            p[0] >= b[0] && p[0] <= b[2] && p[2] >= b[1] && p[2] <= b[3]
        }).max_by_key(|c| c.0)
    }

    /// La mesure d'une tuile de la maquette, si elle est faite.
    pub fn mesure(&self, cle: Cle) -> Option<crate::maquette::Mesure> {
        self.tuiles_maquette.get(&cle).map(|f| f.mesure)
    }

    /// Dessine le lieu de `carte` dans `zone`.
    pub fn dessiner(&mut self, cx: &mut Cx, zone: Rect, carte: &mut Carte) {
        self.image += 1;
        if self.maquette {
            self.dessiner_maquette(cx, zone, carte);
            return;
        }
        // Hors de la maquette, sa fabrique s'arrête (ses tuiles restent pour le retour)
        self.fabrique = None;
        let ciel = CIEL.map(|c| (c * 255.0) as u8);
        cx.dl.rrect(zone, 0.0, [ciel[0], ciel[1], ciel[2], 255]);
        let donnees = carte.donnees.clone();
        let Ok(d) = donnees.read() else { return };
        // La voirie refaite à part : posée dès qu'elle est prête
        d.prendre_voirie();
        let voirie = d.voirie_version();
        self.poser(&d);
        let mvp = self.mvp(zone.w / zone.h.max(1.0));
        if self.bornes_de != carte.compteur {
            self.bornes_de = carte.compteur;
            self.bornes.borrow_mut().clear();
        }
        let voulues = self.choisir(&d, &mvp);
        // Les maillages à faire (absents ou périmés), les plus grossiers d'abord
        let mut a_faire: Vec<(u8, usize, Cle)> = Vec::new();
        for (rang, &cle) in voulues.iter().enumerate() {
            let perime = match self.morceaux.get(&cle) {
                Some(m) => {
                    let b = boite_tuile(&d, cle);
                    !carte.a_jour(m.faite, b) || (m.voirie != voirie && (b[2] - b[0]) / COTES as f32 <= VOIRIE_CASE)
                }
                None => true,
            };
            if perime {
                a_faire.push((cle.0, rang, cle));
                // Et ses ancêtres absents : de quoi montrer vite
                let (z, i, j) = cle;
                for k in 1..=z {
                    let a = (z - k, i >> k, j >> k);
                    if !self.morceaux.contains_key(&a) {
                        a_faire.push((a.0, rang, a));
                    }
                }
            }
        }
        a_faire.sort();
        a_faire.dedup_by_key(|x| x.2);
        let debut = std::time::Instant::now();
        let mut faits = HashSet::new();
        for &(_, _, cle) in &a_faire {
            if debut.elapsed().as_secs_f64() > BUDGET {
                break;
            }
            if !faits.insert(cle) {
                continue;
            }
            let (sommets, indices) = fabriquer(&d, cle);
            self.revision += 1;
            let id = 0x7233_6400_0000_0000u64 ^ (self.num << 40) ^ ((cle.0 as u64) << 32) ^ ((cle.1 as u64 & 0xFFFF) << 16) ^ (cle.2 as u64 & 0xFFFF);
            let maillage = Maillage3d { id, revision: self.revision, sommets: Arc::new(sommets), indices: Arc::new(indices) };
            self.morceaux.insert(cle, Morceau { maillage, faite: carte.compteur, vu: self.image, voirie, aretes: None });
        }
        self.en_attente = faits.len() < a_faire.iter().map(|x| x.2).collect::<HashSet<_>>().len() || d.voirie_en_cours();
        self.compter(a_faire.len() - faits.len(), faits.len());
        // Les bâtiments, dans le temps qui reste
        let voulus = self.choisir_bati(&d, &mvp);
        for &cle in &voulus {
            let perime = match self.batis.get(&cle) {
                Some(m) => !carte.a_jour(m.faite, boite_tuile(&d, cle)),
                None => true,
            };
            if !perime {
                continue;
            }
            if debut.elapsed().as_secs_f64() > BUDGET {
                self.en_attente = true;
                break;
            }
            let (sommets, indices) = fabriquer_bati(&d, cle);
            self.revision += 1;
            let id = 0x6261_7469_0000_0000u64 ^ (self.num << 40) ^ ((cle.0 as u64) << 32) ^ ((cle.1 as u64 & 0xFFFF) << 16) ^ (cle.2 as u64 & 0xFFFF);
            let maillage = Maillage3d { id, revision: self.revision, sommets: Arc::new(sommets), indices: Arc::new(indices) };
            self.batis.insert(cle, Morceau { maillage, faite: carte.compteur, vu: self.image, voirie: 0, aretes: None });
        }
        let (mut bati, mut aretes_bati) = (Vec::new(), Vec::new());
        for cle in &voulus {
            if let Some(m) = self.batis.get_mut(cle) {
                m.vu = self.image;
                if !m.maillage.indices.is_empty() {
                    bati.push(m.maillage.clone());
                    if self.blanc {
                        aretes_bati.push(m.aretes(9, true));
                    }
                }
            }
        }
        self.bati_dessinees = bati.len();
        drop(d);
        // Ce qui se dessine : la tuile, ou son ancêtre le plus proche déjà fait
        let mut montrees: Vec<Cle> = Vec::new();
        let mut vues = HashSet::new();
        for &cle in &voulues {
            let (z, i, j) = cle;
            if let Some(a) = (0..=z).map(|k| (z - k, i >> k, j >> k)).find(|a| self.morceaux.contains_key(a)) {
                if vues.insert(a) {
                    montrees.push(a);
                }
            }
        }
        let textures = carte.textures(&montrees);
        let (mut morceaux, mut aretes_sol) = (Vec::with_capacity(montrees.len()), Vec::new());
        for (cle, t) in montrees.iter().zip(textures) {
            let Some(m) = self.morceaux.get_mut(cle) else { continue };
            m.vu = self.image;
            if self.blanc {
                aretes_sol.push(m.aretes(SOMMET, false));
            }
            let (image, uv) = match t {
                Some((i, uv)) => (Some(i), uv),
                None => (None, [0.0, 0.0, 1.0, 1.0]),
            };
            morceaux.push(Morceau3d { maillage: m.maillage.clone(), image, uv, couleur: [0.72, 0.76, 0.62] });
        }
        self.dessinees = morceaux.len();
        let d = self.camera.distance;
        cx.dl.terrain(Terrain3d { rect: zone, mvp, oeil: self.camera.oeil(), ciel: CIEL, brume: [d * 3.0 + 2000.0, d * 12.0 + 12_000.0], morceaux, bati, blanc: self.blanc, aretes: aretes_sol, aretes_bati, choix: Vec::new(), accent: self.accent, eaux: Vec::new(), defonces: Vec::new(), defonces_aretes: Vec::new(), placage: None, traces: Vec::new() });
        // Les maillages qui n'ont pas servi depuis longtemps
        if self.batis.len() > GARDES / 2 {
            let mut v: Vec<(u64, Cle)> = self.batis.iter().map(|(k, m)| (m.vu, *k)).collect();
            v.sort();
            for (_, k) in v.iter().take(self.batis.len() - GARDES / 2) {
                self.batis.remove(k);
            }
        }
        if self.morceaux.len() > GARDES {
            let mut v: Vec<(u64, Cle)> = self.morceaux.iter().filter(|(k, _)| k.0 > 0).map(|(k, m)| (m.vu, *k)).collect();
            v.sort();
            for (_, k) in v.iter().take(self.morceaux.len() - GARDES) {
                self.morceaux.remove(k);
            }
        }
    }
}

/// Le niveau des tuiles de bâtiments : des tuiles d'au moins `BATI_TUILE` mètres.
fn niveau_bati(c: f32) -> u8 {
    ((c / BATI_TUILE).log2().floor().max(0.0) as u8).min(NIVEAU_MAX)
}

/// Les bâtiments et les tabliers des ponts de la tuile `cle` (ceux dont le centre y
/// tombe), en volume : sommets de 9 flottants (position, normale, couleur) et triangles.
/// Les tabliers ont le gris des quartiers habités, plus sombre dessous.
pub fn fabriquer_bati(d: &Donnees, cle: Cle) -> (Vec<f32>, Vec<u32>) {
    let b = boite_tuile(d, cle);
    let l = &d.lieu;
    let mut v = Vec::new();
    for k in d.batiments_dans(b) {
        let bt = &l.batiments[k as usize];
        let c = bt.centre();
        if c[0] < b[0] || c[0] >= b[2] || c[1] < b[1] || c[1] >= b[3] || bt.contour.len() < 3 {
            continue;
        }
        let (murs, toit) = teintes_bati(bt.genre);
        toits::monter(bt, murs, toit, &mut |p, n, coul| {
            for q in p {
                v.extend_from_slice(&[q[0], q[1], q[2], n[0], n[1], n[2], coul[0], coul[1], coul[2]]);
            }
        });
    }
    let dessus = crate::carte::couleur_sol(sage_reel::HABITE);
    let dessous = dessus.map(|c| c * 0.72);
    for t in &d.tabliers {
        let c = t.centre();
        if c[0] < b[0] || c[0] >= b[2] || c[1] < b[1] || c[1] >= b[3] {
            continue;
        }
        sage_reel::tabliers::monter(t, dessus, dessous, &mut |p, n, coul| {
            for q in p {
                v.extend_from_slice(&[q[0], q[1], q[2], n[0], n[1], n[2], coul[0], coul[1], coul[2]]);
            }
        });
    }
    let ind = (0..(v.len() / 9) as u32).collect();
    (v, ind)
}

/// La boîte (en x, z ; de `hmin` à `hmax` en hauteur) est-elle hors du champ ?
/// Ce qu'on ajoute sous et sur le relief d'une tuile pour la boîte de ses volumes (m).
const BORNE_BAS: f32 = 25.0;
const BORNE_HAUT: f32 = 40.0;
/// Une tuile d'au plus ce nombre de pas de côté se lit point par point ; au-dessus, ses
/// quatre filles.
const BORNES_FEUILLE: f32 = 32.0;

/// Les hauteurs basse et haute du relief sous la tuile `cle` (bord compris), d'après la
/// grille du lieu ; gardées dans `cache` (chaque point n'est lu qu'une fois).
fn bornes_tuile(d: &Donnees, cle: Cle, cache: &mut HashMap<Cle, (f32, f32)>) -> (f32, f32) {
    if let Some(&b) = cache.get(&cle) {
        return b;
    }
    let l = &d.lieu;
    let b = boite_tuile(d, cle);
    let pas = l.pas as f32;
    let r = if b[2] - b[0] <= BORNES_FEUILLE * pas || cle.0 >= 20 {
        let o = sage_reel::origine() as f32;
        let g = l.grille;
        let a = |x: f32| ((x - o) / pas).floor() as i64;
        let (a0, a1) = (a(b[0]).max(g.a0 as i64), (a(b[2]) + 1).min((g.a0 + g.nx) as i64 - 1));
        let (b0, b1) = (a(b[1]).max(g.b0 as i64), (a(b[3]) + 1).min((g.b0 + g.nz) as i64 - 1));
        let mut m = (f32::MAX, f32::MIN);
        for bb in b0..=b1 {
            for aa in a0..=a1 {
                let h = l.h_point(aa, bb);
                m = (m.0.min(h), m.1.max(h));
            }
        }
        if m.0 > m.1 {
            // Hors de la grille : tout le relief possible
            (l.base, l.base + 65535.0 * l.echelle)
        } else {
            m
        }
    } else {
        let (z, i, j) = cle;
        [(0, 0), (1, 0), (0, 1), (1, 1)].iter().fold((f32::MAX, f32::MIN), |m, &(a, c)| {
            let f = bornes_tuile(d, (z + 1, i * 2 + a, j * 2 + c), cache);
            (m.0.min(f.0), m.1.max(f.1))
        })
    };
    cache.insert(cle, r);
    r
}

fn dehors(mvp: &[f32; 16], b: [f32; 4], hmin: f32, hmax: f32) -> bool {
    let mut coins = [[0f32; 4]; 8];
    for (k, c) in coins.iter_mut().enumerate() {
        let p = [if k & 1 == 0 { b[0] } else { b[2] }, if k & 2 == 0 { hmin } else { hmax }, if k & 4 == 0 { b[1] } else { b[3] }];
        for (r, v) in c.iter_mut().enumerate() {
            *v = mvp[r] * p[0] + mvp[4 + r] * p[1] + mvp[8 + r] * p[2] + mvp[12 + r];
        }
    }
    // Tous du même côté d'un des plans du champ
    (0..3).any(|a| coins.iter().all(|c| c[a] < -c[3]) || coins.iter().all(|c| c[a] > c[3]))
}

/// Le maillage de la tuile `cle` : sommets de `SOMMET` flottants (position, normale,
/// texture, matière)
/// et triangles, avec ses jupes.
pub fn fabriquer(d: &Donnees, cle: Cle) -> (Vec<f32>, Vec<u32>) {
    let l = &d.lieu;
    let b = boite_tuile(d, cle);
    let n = COTES;
    let s = (b[2] - b[0]) / n as f32;
    // Près de l'œil, la voirie en volume : le sol terrassé le long des voies
    let (_, vo) = d.voirie();
    let terrasser = s <= VOIRIE_CASE && vo.pres([b[0] - s, b[1] - s, b[2] + s, b[3] + s]);
    // Les hauteurs (terrassées), avec une case de plus tout autour pour les normales
    let m = n + 3;
    let mut h = vec![0f32; m * m];
    for k in 0..m {
        let z = b[1] + (k as f32 - 1.0) * s;
        for a in 0..m {
            let x = b[0] + (a as f32 - 1.0) * s;
            let r = l.relief(x as f64, z as f64) as f32;
            h[k * m + a] = if terrasser { vo.sol(x, z, r, l.eau(x as f64, z as f64) > 0.5, s.min(crate::voirie3d::MARGE_MAX)) } else { r };
        }
    }
    let hh = |a: usize, k: usize| h[(k + 1) * m + a + 1];
    let mut v = Vec::with_capacity(((n + 1) * (n + 1) + 4 * (n + 1)) * SOMMET);
    let sommet = |a: usize, k: usize, bas: f32| -> [f32; SOMMET] {
        let y = hh(a, k);
        let (gx, gz) = ((h[(k + 1) * m + a + 2] - h[(k + 1) * m + a]) / (2.0 * s), (h[(k + 2) * m + a + 1] - h[k * m + a + 1]) / (2.0 * s));
        let nn = (gx * gx + 1.0 + gz * gz).sqrt();
        [b[0] + a as f32 * s, y - bas, b[1] + k as f32 * s, -gx / nn, 1.0 / nn, -gz / nn, a as f32 / n as f32, k as f32 / n as f32, TERRE]
    };
    for k in 0..=n {
        for a in 0..=n {
            v.extend_from_slice(&sommet(a, k, 0.0));
        }
    }
    let mut ind = Vec::with_capacity(n * n * 6 + 4 * n * 6);
    let w = (n + 1) as u32;
    // L'eau aux sommets (de 0 à 1, interpolée entre les points du lieu)
    let mut e = vec![0f32; (n + 1) * (n + 1)];
    for k in 0..=n {
        for a in 0..=n {
            e[k * (n + 1) + a] = l.eau((b[0] + a as f32 * s) as f64, (b[1] + k as f32 * s) as f64) as f32;
        }
    }
    // 1. L'eau : chaque triangle coupé à la rive ; la part d'eau, plane au niveau de
    // l'eau, se dessine ; la part de terre passe à la suite
    let niveau = l.niveau;
    let champ_eau: Vec<f32> = e.iter().map(|x| 0.5 - x).collect();
    let mut coupe = Decoupe::default();
    let mut a_plat = HashMap::new();
    let mut terre: Vec<[u32; 3]> = Vec::with_capacity(n * n * 2);
    for k in 0..n as u32 {
        for a in 0..n as u32 {
            let p = k * w + a;
            for t in [[p, p + w, p + 1], [p + 1, p + w, p + w + 1]] {
                let (dehors, dedans) = coupe.couper(&mut v, &champ_eau, t, |m| m[1] = niveau);
                eventail(&dehors, |t| terre.push(t));
                let dedans: Vec<u32> = dedans.iter().map(|&i| copie(&mut v, &mut a_plat, i, |m| *m = sommet_plat(m, niveau, EAU))).collect();
                eventail(&dedans, |t| ind.extend_from_slice(&t));
            }
        }
    }
    // 2. Les voies, près de l'œil : la terre coupée exactement au bord des rubans et des
    // places (le point cherché sur l'arête), son bord à la hauteur de la voie ; ce
    // qu'ils couvrent s'en va, et les rubans s'ajoutent
    if terrasser {
        let champ_voie: Vec<f32> = v.chunks(SOMMET).map(|m| if vo.sur(m[0], m[2]).is_some() { -1.0 } else { 1.0 }).collect();
        let bord = |a: &[f32; SOMMET], c: &[f32; SOMMET], ca: f32, _: f32| {
            // a dedans ou dehors, c de l'autre côté : la limite, par dichotomie
            let dans = |t: f32| vo.sur(a[0] + (c[0] - a[0]) * t, a[2] + (c[2] - a[2]) * t).is_some();
            let da = ca <= 0.0;
            let (mut lo, mut hi) = (0.0f32, 1.0f32);
            for _ in 0..14 {
                let mi = (lo + hi) / 2.0;
                if dans(mi) == da { lo = mi } else { hi = mi }
            }
            (lo + hi) / 2.0
        };
        let mut coupe = Decoupe::default();
        let mut fond = HashMap::new();
        for t in terre {
            let (dehors, dedans) = coupe.couper_par(&mut v, &champ_voie, t, &bord, |m| {
                if let Some(y) = vo.hauteur(m[0], m[2]) {
                    m[1] = y - crate::voirie3d::ENFOUI;
                }
            });
            eventail(&dehors, |t| ind.extend_from_slice(&t));
            // Sous les rubans, un fond un peu plus bas : il bouche les angles rentrants,
            // où le bord de la coupe (droit) passe à côté des rubans
            // (les points de coupe restent au bord, à la hauteur de la voie)
            let dedans: Vec<u32> = dedans.iter().map(|&i| if (i as usize) < champ_voie.len() { copie(&mut v, &mut fond, i, |m| *m = sommet_plat(m, vo.hauteur(m[0], m[2]).unwrap_or(m[1]) - FOND, TERRE)) } else { i }).collect();
            eventail(&dedans, |t| ind.extend_from_slice(&t));
        }
        vo.monter(b, &mut v, &mut ind);
    } else {
        for t in terre {
            ind.extend_from_slice(&t);
        }
    }
    // Les jupes : chaque bord, doublé plus bas
    let jupe = s * 1.5 + 4.0;
    let bords: [Box<dyn Fn(usize) -> (usize, usize)>; 4] = [Box::new(|t| (t, 0)), Box::new(|t| (t, n)), Box::new(|t| (0, t)), Box::new(|t| (n, t))];
    for bord in &bords {
        let depart = (v.len() / SOMMET) as u32;
        for t in 0..=n {
            let (a, k) = bord(t);
            v.extend_from_slice(&sommet(a, k, jupe));
        }
        for t in 0..n as u32 {
            let (a0, k0) = bord(t as usize);
            let (a1, k1) = bord(t as usize + 1);
            let (p0, p1) = (k0 as u32 * w + a0 as u32, k1 as u32 * w + a1 as u32);
            ind.extend_from_slice(&[p0, depart + t, p1, p1, depart + t, depart + t + 1]);
        }
    }
    (v, ind)
}

/// Les arêtes à tracer d'un maillage (sommets de `flottants` flottants, la position
/// d'abord), en indices deux par deux. Chaque arête une fois, repérée par la position
/// de ses bouts (au centimètre) : les sommets doublés (la rive, les murs d'un bâtiment)
/// n'en font qu'une. Sans `pli` (le relief), toutes, sauf celles des faces debout (les
/// jupes) ; avec (les volumes), les bords et les plis de plus de 20° seulement : pas
/// les diagonales d'une face plane. Les rubans de la voirie se tracent comme les volumes
/// (en plus, la limite de la chaussée et de ses bords).
pub fn aretes(v: &[f32], flottants: usize, ind: &[u32], pli: bool) -> Vec<u32> {
    type Point = (i64, i64, i64);
    let pos = |i: u32| {
        let k = i as usize * flottants;
        [v[k], v[k + 1], v[k + 2]]
    };
    let cle = |p: [f32; 3]| -> Point { ((p[0] * 100.0).round() as i64, (p[1] * 100.0).round() as i64, (p[2] * 100.0).round() as i64) };
    // Chaque arête : ses sommets, la normale de sa première face, pliée, ses faces
    // Le relief : la matière d'une face de la voirie (chaussée, bords), sinon -1
    let voirie = |t: &[u32]| -> f32 {
        if pli || flottants < SOMMET {
            return -1.0;
        }
        let m = t.iter().map(|&i| v[i as usize * flottants + 8]).fold(f32::MAX, f32::min);
        // (la maquette, matière 4 et plus, n'est pas de la voirie)
        if m >= CHAUSSEE - 0.5 && m < crate::maquette::MAQUETTE - 0.5 && t.iter().all(|&i| v[i as usize * flottants + 8] == m) { m } else { -1.0 }
    };
    let mut vues: HashMap<(Point, Point), (u32, u32, [f32; 3], bool, u32, f32)> = HashMap::new();
    let mut ordre = Vec::new();
    let pli_min = 20f32.to_radians().cos();
    for t in ind.chunks_exact(3) {
        let (a, b, c) = (pos(t[0]), pos(t[1]), pos(t[2]));
        let (u, w) = ([b[0] - a[0], b[1] - a[1], b[2] - a[2]], [c[0] - a[0], c[1] - a[1], c[2] - a[2]]);
        let n = [u[1] * w[2] - u[2] * w[1], u[2] * w[0] - u[0] * w[2], u[0] * w[1] - u[1] * w[0]];
        let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        if l < 1e-9 {
            continue;
        }
        let n = n.map(|x| x / l);
        if !pli && n[1].abs() < 0.05 {
            continue;
        }
        let mat = voirie(t);
        for (i, j) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            let (ki, kj) = (cle(pos(i)), cle(pos(j)));
            if ki == kj {
                continue;
            }
            let k = if ki < kj { (ki, kj) } else { (kj, ki) };
            match vues.get_mut(&k) {
                Some(e) => {
                    e.3 |= e.2[0] * n[0] + e.2[1] * n[1] + e.2[2] * n[2] < pli_min || e.5 != mat;
                    e.4 += 1;
                }
                None => {
                    vues.insert(k, (i, j, n, false, 1, mat));
                    ordre.push(k);
                }
            }
        }
    }
    let mut r = Vec::with_capacity(ordre.len() * 2);
    for k in ordre {
        let e = vues[&k];
        // La voirie, comme les volumes : ses bords, ses plis, les limites des bandes
        if (!pli && e.5 < 0.0) || e.3 || e.4 == 1 {
            r.extend_from_slice(&[e.0, e.1]);
        }
    }
    r
}

/// Le côté des cases au-delà duquel les voies ne se découpent pas dans le relief (m) :
/// de loin, elles restent peintes sur la carte.
pub const VOIRIE_CASE: f32 = 6.0;
/// Le fond sous les rubans, sous la voie (m).
const FOND: f32 = 0.3;

/// Un sommet posé à plat : à la hauteur `y`, la normale vers le haut, de la matière `mat`.
fn sommet_plat(m: &[f32; SOMMET], y: f32, mat: f32) -> [f32; SOMMET] {
    [m[0], y, m[2], 0.0, 1.0, 0.0, m[6], m[7], mat]
}

fn lire_sommet(v: &[f32], i: u32) -> [f32; SOMMET] {
    let i = i as usize * SOMMET;
    v[i..i + SOMMET].try_into().unwrap_or([0.0; SOMMET])
}

fn pousser(v: &mut Vec<f32>, s: [f32; SOMMET]) -> u32 {
    v.extend_from_slice(&s);
    (v.len() / SOMMET - 1) as u32
}

/// Le double du sommet `i`, retouché par `f` (fait une fois : `faits`).
fn copie(v: &mut Vec<f32>, faits: &mut HashMap<u32, u32>, i: u32, f: impl Fn(&mut [f32; SOMMET])) -> u32 {
    if let Some(&c) = faits.get(&i) {
        return c;
    }
    let mut m = lire_sommet(v, i);
    f(&mut m);
    let c = pousser(v, m);
    faits.insert(i, c);
    c
}

/// Un polygone convexe (dans son sens) en triangles, en éventail.
fn eventail(p: &[u32], mut f: impl FnMut([u32; 3])) {
    for k in 1..p.len().saturating_sub(1) {
        f([p[0], p[k], p[k + 1]]);
    }
}

/// Une découpe du relief le long de la ligne où un champ, donné à chaque sommet, passe
/// par 0 (dedans : 0 ou moins). Les points de coupe se font une fois par arête : les deux
/// triangles de part et d'autre (et la tuile voisine, qui calcule pareil) les partagent,
/// et la couture est étanche. Les passes se suivent : l'eau, puis les voies sur la terre.
#[derive(Default)]
struct Decoupe {
    coupes: HashMap<(u32, u32), u32>,
}

impl Decoupe {
    /// Le triangle `t`, en deux parts (dehors, dedans), chacune un polygone dans le sens
    /// du triangle. Un point de coupe est interpolé entre les deux bouts de l'arête, puis
    /// retouché par `poser` (sa hauteur).
    fn couper(&mut self, v: &mut Vec<f32>, champ: &[f32], t: [u32; 3], poser: impl Fn(&mut [f32; SOMMET])) -> (Vec<u32>, Vec<u32>) {
        let lineaire = |_: &[f32; SOMMET], _: &[f32; SOMMET], ca: f32, cb: f32| if (ca - cb).abs() > 1e-9 { (ca / (ca - cb)).clamp(0.0, 1.0) } else { 0.5 };
        self.couper_par(v, champ, t, &lineaire, poser)
    }

    /// `couper`, le point de coupe sur l'arête (a, b) à la fraction `fraction(a, b,
    /// champ en a, champ en b)`.
    fn couper_par(&mut self, v: &mut Vec<f32>, champ: &[f32], t: [u32; 3], fraction: &dyn Fn(&[f32; SOMMET], &[f32; SOMMET], f32, f32) -> f32, poser: impl Fn(&mut [f32; SOMMET])) -> (Vec<u32>, Vec<u32>) {
        let c = t.map(|i| champ[i as usize]);
        let d = c.map(|x| x <= 0.0);
        if d.iter().all(|x| !x) {
            return (t.to_vec(), Vec::new());
        }
        if d.iter().all(|x| *x) {
            return (Vec::new(), t.to_vec());
        }
        let (mut dehors, mut dedans) = (Vec::with_capacity(4), Vec::with_capacity(4));
        for k in 0..3 {
            let (i, j) = (t[k], t[(k + 1) % 3]);
            if d[k] { dedans.push(i) } else { dehors.push(i) }
            if d[k] != d[(k + 1) % 3] {
                let cle = (i.min(j), i.max(j));
                let p = match self.coupes.get(&cle) {
                    Some(&p) => p,
                    None => {
                        let (a, b) = (lire_sommet(v, cle.0), lire_sommet(v, cle.1));
                        let (ca, cb) = (champ[cle.0 as usize], champ[cle.1 as usize]);
                        let f = fraction(&a, &b, ca, cb);
                        let mut m = [0f32; SOMMET];
                        for k in 0..SOMMET {
                            m[k] = a[k] + (b[k] - a[k]) * f;
                        }
                        poser(&mut m);
                        let p = pousser(v, m);
                        self.coupes.insert(cle, p);
                        p
                    }
                };
                dehors.push(p);
                dedans.push(p);
            }
        }
        (dehors, dedans)
    }
}

/// Le réseau des voies est tracé sur la maquette quand la caméra est à moins de
/// `TRACES_DISTANCE` (m) de sa cible ; ou sur la sélection seule, d'au plus
/// `TRACES_TUILES` tuiles.
pub const TRACES_DISTANCE: f32 = 300.0;
pub const TRACES_TUILES: usize = 4;
/// Les réglages de la surimpression sur la maquette : les tracés du réseau et les
/// faces du banc (les remblais), chacun montré ou non, avec la distance de la caméra
/// à sa cible en deçà de laquelle il paraît (m) et son rayon autour de la cible (en
/// fois cette distance).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Surimpression {
    pub traces: bool,
    pub traces_distance: f32,
    pub traces_rayon: f32,
    pub remblai: bool,
    pub remblai_distance: f32,
    pub remblai_rayon: f32,
}

impl Default for Surimpression {
    fn default() -> Self {
        Surimpression { traces: true, traces_distance: TRACES_DISTANCE, traces_rayon: 1.0, remblai: true, remblai_distance: 2.0 * TRACES_DISTANCE, remblai_rayon: 2.0 }
    }
}

impl Surimpression {
    /// « 1 300 1 1 600 2 » : tracés (0/1), distance, rayon ; remblais, de même.
    pub fn lire(v: &[f32]) -> Option<Surimpression> {
        let [t, td, tr, r, rd, rr] = v[..] else { return None };
        Some(Surimpression { traces: t != 0.0, traces_distance: td, traces_rayon: tr, remblai: r != 0.0, remblai_distance: rd, remblai_rayon: rr })
    }

    pub fn ecrire(&self) -> String {
        format!("{} {} {} {} {} {}", u8::from(self.traces), self.traces_distance, self.traces_rayon, u8::from(self.remblai), self.remblai_distance, self.remblai_rayon)
    }
}

/// Les tracés passent d'autant au-dessus de la chaussée (m).
const TRACES_DESSUS: f32 = 1.0;

/// Les tuiles de remblais des voies faites au plus par image.
const FACES_PAR_IMAGE: usize = 1;
/// La demi-largeur des croix des nœuds (m).
const TRACES_BRAS: f32 = 1.5;

impl Relief3d {
    /// Le réseau nivelé en surimpression sur les tuiles voulues (pas sur les ancêtres
    /// montrés à leur place en attendant), s'il y en a au plus
    /// `TRACES_TUILES` : ses arêtes coupées à chaque tuile et ses nœuds, à
    /// `TRACES_DESSUS` au-dessus de la chaussée. Rien n'entre dans les tuiles faites.
    /// Le réseau est calculé à part la première fois (dans un fil) : en attendant, rien.
    /// Oublie les tracés faits : ils seront redemandés.
    pub fn refaire_traces(&mut self) {
        self.traces.clear();
        self.faces.clear();
    }

    /// Les remblais des voies en surimpression sur les tuiles `pres` : ceux de Flow
    /// (`remblais_voies`), ou ceux du module du banc s'il en fait (pour les essais).
    /// Le réseau nivelé est calculé à part la première fois : en attendant, rien.
    fn faces_montrees(&mut self, donnees: &Arc<std::sync::RwLock<Donnees>>, pres: &[Cle], faite: u64) -> Vec<Maillage3d> {
        let banc = crate::banc::faces_actives();
        let reseau = if banc {
            None
        } else {
            let r = donnees.read().ok().and_then(|d| d.reseau_pret());
            if r.is_none() && !pres.is_empty() && !self.reseau_lance {
                self.reseau_lance = true;
                let donnees = donnees.clone();
                let _ = std::thread::Builder::new().name("reseau".into()).spawn(move || {
                    if let Ok(d) = donnees.read() {
                        d.reseau();
                    }
                });
            }
            match r {
                Some(r) => Some(r),
                None => return Vec::new(),
            }
        };
        let mut v = Vec::new();
        // Au plus quelques tuiles neuves par image : l'image ne s'arrête pas
        let mut neuves = 0;
        for &cle in pres {
            if !self.faces.contains_key(&cle) && neuves < FACES_PAR_IMAGE {
                let faites = match &reseau {
                    Some(r) => {
                        neuves += 1;
                        match donnees.read() {
                            Ok(d) => Ok(Some(crate::remblais_voies::tuile(&d, r, cle))),
                            Err(_) => Ok(None),
                        }
                    }
                    None => crate::banc::faces_lieux(faite, cle).unwrap_or(Ok(None)),
                };
                match faites {
                    Ok(Some((s, i))) => {
                        self.revision += 1;
                        let id = 0x6661_0000_0000_0000u64 ^ (self.num << 40) ^ ((cle.0 as u64) << 32) ^ ((cle.1 as u64 & 0xFFFF) << 16) ^ (cle.2 as u64 & 0xFFFF);
                        let m = (!i.is_empty()).then(|| Maillage3d { id, revision: self.revision, sommets: Arc::new(s), indices: Arc::new(i) });
                        self.faces.insert(cle, m);
                    }
                    Ok(None) => {}
                    Err(_) => {
                        self.faces.insert(cle, None);
                    }
                }
            }
            if let Some(Some(m)) = self.faces.get(&cle) {
                v.push(m.clone());
            }
        }
        if self.faces.len() > 256 {
            self.faces.retain(|k, _| pres.contains(k));
        }
        v
    }

    fn traces_montrees(&mut self, donnees: &Arc<std::sync::RwLock<Donnees>>, montrees: &[Cle], faite: u64) -> Vec<(Maillage3d, [f32; 3], f32)> {
        if montrees.is_empty() {
            self.traces_bilan = "aucune tuile".into();
            return Vec::new();
        }
        // Sur le banc, les tracés du module (s'il en fait)
        if crate::banc::traces_actives() {
            let mut v = Vec::new();
            let (mut faites, mut attendues, mut segments) = (0, 0, 0);
            for &cle in montrees {
                if !self.traces.contains_key(&cle) {
                    let r = crate::banc::traces_lieux(faite, cle).unwrap_or(Ok(None));
                    match r {
                        Ok(Some(traits)) => {
                            self.revision += 1;
                            let t = maillages_traits(&traits, cle, self.num, self.revision);
                            self.traces.insert(cle, t);
                        }
                        Ok(None) => attendues += 1,
                        Err(_) => {
                            self.traces.insert(cle, Vec::new());
                        }
                    }
                }
                if let Some(t) = self.traces.get(&cle) {
                    faites += 1;
                    segments += t.iter().map(|m| m.0.indices.len() / 2).sum::<usize>();
                    v.extend(t.iter().cloned());
                }
            }
            self.traces_bilan = format!("banc : {faites} tuiles, {segments} segments, {attendues} en attente");
            return v;
        }
        let Ok(d) = donnees.read() else { return Vec::new() };
        let Some(r) = d.reseau_pret() else {
            if !self.reseau_lance {
                self.reseau_lance = true;
                let donnees = donnees.clone();
                let _ = std::thread::Builder::new().name("reseau".into()).spawn(move || {
                    if let Ok(d) = donnees.read() {
                        d.reseau();
                    }
                });
            }
            return Vec::new();
        };
        let adresse = Arc::as_ptr(&r) as usize;
        if adresse != self.traces_de {
            self.traces.clear();
            self.traces_de = adresse;
        }
        let mut v = Vec::new();
        for &cle in montrees {
            if !self.traces.contains_key(&cle) {
                self.revision += 1;
                let t = traces_reseau(&r, boite_tuile(&d, cle), cle, self.num, self.revision);
                self.traces.insert(cle, t);
            }
            v.extend(self.traces[&cle].iter().cloned());
        }
        if self.traces.len() > 256 {
            self.traces.retain(|k, _| montrees.contains(k));
        }
        let segments: usize = v.iter().map(|m| m.0.indices.len() / 2).sum();
        self.traces_bilan = format!("flow : {} tuiles, {segments} segments", montrees.len());
        v
    }
}

/// Les traits d'un module du banc en maillages (sommets de 9 flottants, normale vers le
/// haut), un par groupe.
fn maillages_traits(traits: &[flow_banc::Traits], cle: Cle, num: u64, revision: u64) -> Vec<(Maillage3d, [f32; 3], f32)> {
    let base = 0x7462_0000_0000_0000u64 ^ (num << 40) ^ ((cle.0 as u64) << 32) ^ ((cle.1 as u64 & 0xFFFF) << 16) ^ (cle.2 as u64 & 0xFFFF);
    traits
        .iter()
        .enumerate()
        .filter(|(_, t)| t.points.len() >= 2)
        .map(|(k, t)| {
            let n = t.points.len() / 2 * 2;
            let s: Vec<f32> = t.points[..n].iter().flat_map(|p| [p[0], p[1], p[2], 0.0, 1.0, 0.0, 0.0, 0.0, 0.0]).collect();
            let i: Vec<u32> = (0..n as u32).collect();
            (Maillage3d { id: base ^ ((k as u64 & 0xFF) << 48), revision, sommets: Arc::new(s), indices: Arc::new(i) }, t.couleur, t.epaisseur)
        })
        .collect()
}

/// Les couleurs des tracés : au sol, en pont, en tunnel ; et des nœuds.
const TRACE_SOL: [f32; 3] = [1.0, 0.78, 0.05];
const TRACE_PONT: [f32; 3] = [0.92, 0.2, 0.85];
const TRACE_TUNNEL: [f32; 3] = [0.15, 0.75, 0.95];
const TRACE_NOEUD: [f32; 3] = [0.9, 0.1, 0.1];

/// Le segment p → q (x, z, h) coupé à la boîte b (Liang–Barsky), ou rien.
fn couper(p: [f32; 3], q: [f32; 3], b: [f32; 4]) -> Option<([f32; 3], [f32; 3])> {
    let (dx, dz) = (q[0] - p[0], q[1] - p[1]);
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    for (pk, qk) in [(-dx, p[0] - b[0]), (dx, b[2] - p[0]), (-dz, p[1] - b[1]), (dz, b[3] - p[1])] {
        if pk == 0.0 {
            if qk < 0.0 {
                return None;
            }
        } else {
            let t = qk / pk;
            if pk < 0.0 {
                t0 = t0.max(t);
            } else {
                t1 = t1.min(t);
            }
        }
    }
    if t0 > t1 {
        return None;
    }
    let at = |t: f32| [p[0] + t * dx, p[1] + t * dz, p[2] + t * (q[2] - p[2])];
    Some((at(t0), at(t1)))
}

/// Les tracés du réseau `r` dans la boîte `b` : un maillage de traits par passage, et
/// les nœuds en croix.
fn traces_reseau(r: &sage_reel::reseau::Reseau, b: [f32; 4], cle: Cle, num: u64, revision: u64) -> Vec<(Maillage3d, [f32; 3], f32)> {
    use sage_reel::reseau::Passage;
    // Un sommet de 9 flottants : position (x, h, z), normale vers le haut, le reste nul
    let poser = |s: &mut Vec<f32>, i: &mut Vec<u32>, a: [f32; 3], c: [f32; 3]| {
        let n = (s.len() / 9) as u32;
        for p in [a, c] {
            s.extend_from_slice(&[p[0], p[2] + TRACES_DESSUS, p[1], 0.0, 1.0, 0.0, 0.0, 0.0, 0.0]);
        }
        i.extend_from_slice(&[n, n + 1]);
    };
    let mut listes: [(Vec<f32>, Vec<u32>); 4] = Default::default();
    for a in &r.aretes {
        let k = match a.passage {
            Passage::Sol => 0,
            Passage::Pont => 1,
            Passage::Tunnel => 2,
        };
        for w in a.points.windows(2) {
            if let Some((p, q)) = couper(w[0], w[1], b) {
                let (s, i) = &mut listes[k];
                poser(s, i, p, q);
            }
        }
    }
    // Les nœuds : une croix couchée
    let bras = TRACES_BRAS;
    for n in &r.noeuds {
        let p = n.p;
        if p[0] < b[0] || p[0] > b[2] || p[1] < b[1] || p[1] > b[3] {
            continue;
        }
        let (s, i) = &mut listes[3];
        poser(s, i, [p[0] - bras, p[1], p[2]], [p[0] + bras, p[1], p[2]]);
        poser(s, i, [p[0], p[1] - bras, p[2]], [p[0], p[1] + bras, p[2]]);
    }
    let base = 0x7472_0000_0000_0000u64 ^ (num << 40) ^ ((cle.0 as u64) << 32) ^ ((cle.1 as u64 & 0xFFFF) << 16) ^ (cle.2 as u64 & 0xFFFF);
    let reglages = [(TRACE_SOL, 2.0), (TRACE_PONT, 3.0), (TRACE_TUNNEL, 2.0), (TRACE_NOEUD, 3.0)];
    listes
        .into_iter()
        .zip(reglages)
        .enumerate()
        .filter(|(_, ((_, i), _))| !i.is_empty())
        .map(|(k, ((s, i), (c, e)))| (Maillage3d { id: base ^ ((k as u64 + 1) << 56), revision, sommets: Arc::new(s), indices: Arc::new(i) }, c, e))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sage_reel::{Grille, Lieu};

    fn donnees() -> Donnees {
        // Une pente vers l'est, sur 3 × 2 km
        let pas = 2.0;
        let g = Grille::emprise(pas, 3000.0, 2000.0);
        let h: Vec<f32> = (0..g.points()).map(|i| g.point(i).0 as f32 * 0.5).collect();
        let eau = vec![false; g.points()];
        Donnees::new(Lieu::sur("Pente", 49.0, 1.0, 0.0, 0.0, pas, g, -10.0, &h, &eau, Vec::new(), Vec::new(), Vec::new()))
    }

    /// Banc : FLOW_LIEU ; le nombre de tuiles d'une zone fine au centre selon le rayon.
    #[test]
    #[ignore]
    fn banc_zone_fine() {
        let l = sage_reel::Lieu::lire(std::path::Path::new(&std::env::var("FLOW_LIEU").unwrap())).unwrap();
        let d = Donnees::new(l);
        let e = d.lieu.etendue();
        let centre = [((e[0] + e[2]) / 2.0) as f32, ((e[1] + e[3]) / 2.0) as f32];
        let (_, _, c) = d.monde();
        println!("niveau fin {} : tuiles de {:.0} m", niveau_fin(&d), c / (1u32 << niveau_fin(&d)) as f32);
        for r in [100.0, 250.0, 500.0, 750.0, 1000.0, 1500.0] {
            println!("rayon {r} m : {} tuiles", tuiles_zone(&d, ZoneFine { centre, rayon: r }).len());
        }
    }

    #[test]
    fn la_zone_fine_prend_les_tuiles_les_plus_fines_du_disque() {
        let d = donnees();
        let n = niveau_fin(&d);
        let z = ZoneFine { centre: [0.0, 0.0], rayon: 300.0 };
        let v = tuiles_zone(&d, z);
        assert!(!v.is_empty() && v.iter().all(|c| c.0 == n));
        // Toutes touchent le disque ; la plus proche contient le centre
        for &c in &v {
            let b = boite_tuile(&d, c);
            let dx = (b[0] - 0.0f32).max(0.0).max(0.0 - b[2]);
            let dz = (b[1] - 0.0f32).max(0.0).max(0.0 - b[3]);
            assert!(dx.hypot(dz) <= 300.0);
        }
        let b = boite_tuile(&d, v[0]);
        assert!(b[0] <= 0.0 && b[2] >= 0.0 && b[1] <= 0.0 && b[3] >= 0.0);
        // Un rayon double : environ quatre fois plus de tuiles
        let w = tuiles_zone(&d, ZoneFine { rayon: 600.0, ..z });
        assert!(w.len() > 3 * v.len() && w.len() < 5 * v.len(), "{} {}", v.len(), w.len());
    }

    #[test]
    fn une_tuile_suit_le_relief_et_a_ses_jupes() {
        let d = donnees();
        let (v, ind) = fabriquer(&d, (0, 0, 0));
        let n = COTES + 1;
        assert_eq!(v.len(), (n * n + 4 * n) * SOMMET);
        assert_eq!(ind.len(), COTES * COTES * 6 + 4 * COTES * 6);
        assert!(ind.iter().all(|&i| (i as usize) < v.len() / SOMMET));
        // Un sommet du milieu : à la hauteur du relief, la normale penchée vers l'ouest
        let k = (n / 2 * n + n / 2) * SOMMET;
        let (x, y, z) = (v[k], v[k + 1], v[k + 2]);
        assert!((y - d.lieu.relief(x as f64, z as f64) as f32).abs() < 0.05);
        assert!(v[k + 3] < -0.01 && v[k + 4] > 0.9);
        // Les jupes sous le bord
        let j = n * n * SOMMET;
        assert!(v[j + 1] < v[1] - 1.0);
    }

    #[test]
    fn les_batiments_se_montent_une_fois_dans_leur_tuile() {
        use sage_reel::{Batiment, MAISON};
        let pas = 4.0;
        let g = Grille::emprise(pas, 2000.0, 2000.0);
        let h = vec![10.0; g.points()];
        let carre = |x: f32, z: f32| vec![[x, z], [x + 10.0, z], [x + 10.0, z + 8.0], [x, z + 8.0]];
        let b = vec![Batiment::plat(MAISON, 9.0, 16.0, carre(0.0, 0.0)), Batiment::plat(MAISON, 9.0, 16.0, carre(500.0, 300.0))];
        let d = Donnees::new(Lieu::sur("B", 49.0, 1.0, 0.0, 0.0, pas, g, 0.0, &h, &vec![false; g.points()], Vec::new(), b, Vec::new()));
        let (_, _, c) = d.monde();
        let z = niveau_bati(c);
        let t = c / (1u32 << z) as f32;
        assert!(t >= BATI_TUILE && t < BATI_TUILE * 2.0, "{t}");
        // Chaque bâtiment dans une seule tuile : 2 en tout sur toutes les tuiles
        let n = 1i32 << z;
        let mut murs = 0;
        let mut hauts = Vec::new();
        for j in 0..n {
            for i in 0..n {
                let (v, ind) = fabriquer_bati(&d, (z, i, j));
                assert_eq!(ind.len(), v.len() / 9);
                // Un toit plat : deux triangles vers le haut par bâtiment
                murs += v.chunks(27).filter(|t| t[4] > 0.9).count();
                hauts.extend(v.chunks(9).map(|s| s[1]));
            }
        }
        assert_eq!(murs, 4);
        assert!(hauts.iter().all(|&y| (y - 9.0).abs() < 1e-3 || (y - 16.0).abs() < 1e-3));
    }

    #[test]
    fn les_ponts_ont_leur_tablier_avec_les_batiments() {
        use sage_reel::{Route, GRANDE};
        let pas = 4.0;
        let g = Grille::emprise(pas, 2000.0, 2000.0);
        let h = vec![10.0; g.points()];
        // Deux chaussées parallèles : un seul tablier, dans la tuile de son milieu
        let ponts = vec![
            Route { genre: GRANDE, largeur: 8.0, points: vec![[0.0, 0.0, 20.0], [100.0, 0.0, 20.0]] },
            Route { genre: GRANDE, largeur: 8.0, points: vec![[100.0, 11.0, 20.0], [0.0, 11.0, 20.0]] },
        ];
        let d = Donnees::new(Lieu::sur("P", 49.0, 1.0, 0.0, 0.0, pas, g, 0.0, &h, &vec![false; g.points()], Vec::new(), Vec::new(), ponts));
        assert_eq!(d.tabliers.len(), 1);
        let (_, _, c) = d.monde();
        let z = niveau_bati(c);
        let n = 1i32 << z;
        let mut dessus = Vec::new();
        for j in 0..n {
            for i in 0..n {
                let (v, _) = fabriquer_bati(&d, (z, i, j));
                dessus.extend(v.chunks(9).filter(|s| s[4] > 0.9).map(|s| (s[1], s[2])));
            }
        }
        // Le dessus de 20 m (aux bouts) à 20 m plus le bombement, de z = −4,75 à 15,75 (les
        // deux chaussées et le débord)
        let fleche = sage_reel::tabliers::FLECHE_MAX;
        assert!(!dessus.is_empty() && dessus.iter().all(|&(y, _)| (20.0 - 1e-3..=20.0 + fleche).contains(&y)));
        assert!(dessus.iter().any(|&(y, _)| (y - 20.0).abs() < 1e-3) && dessus.iter().any(|&(y, _)| y > 20.5));
        let (zmin, zmax) = dessus.iter().fold((f32::MAX, f32::MIN), |a, &(_, z)| (a.0.min(z), a.1.max(z)));
        assert!((zmin + 4.75).abs() < 0.05 && (zmax - 15.75).abs() < 0.05, "{zmin} {zmax}");
    }

    #[test]
    fn l_eau_est_plane_et_cousue_a_la_rive() {
        // Un plateau à 10 m, une rivière en biais (lit à 2 m), l'eau à 5 m
        let pas = 4.0;
        let g = Grille::emprise(pas, 2000.0, 2000.0);
        let (a0, b0) = g.point(0);
        let dans = |i: usize| {
            let (a, b) = g.point(i);
            ((a - a0) as f32 - (b - b0) as f32 * 0.3 - 225.0).abs() < 30.0
        };
        let h: Vec<f32> = (0..g.points()).map(|i| if dans(i) { 2.0 } else { 10.0 }).collect();
        let eau: Vec<bool> = (0..g.points()).map(dans).collect();
        let d = Donnees::new(Lieu::sur("R", 49.0, 1.0, 0.0, 0.0, pas, g, 5.0, &h, &eau, Vec::new(), Vec::new(), Vec::new()));
        for cle in [(0, 0, 0), (2, 1, 1), (3, 3, 2)] {
            let (v, ind) = fabriquer(&d, cle);
            assert!(ind.iter().all(|&i| (i as usize) < v.len() / SOMMET));
            let p = |i: u32| [v[i as usize * SOMMET], v[i as usize * SOMMET + 1], v[i as usize * SOMMET + 2]];
            let b = boite_tuile(&d, cle);
            let (mut plans, mut aretes) = (0, HashMap::new());
            for t in ind.chunks(3) {
                let (a, bb, c) = (p(t[0]), p(t[1]), p(t[2]));
                let (u, w) = ([bb[0] - a[0], bb[2] - a[2]], [c[0] - a[0], c[2] - a[2]]);
                // Les jupes sont debout : on ne regarde que le dessus
                if (u[0] * w[1] - u[1] * w[0]).abs() < 1e-6 {
                    continue;
                }
                // Rien ne descend sous l'eau : le lit est remplacé par la surface
                assert!([a, bb, c].iter().all(|q| q[1] >= 5.0 - 1e-3), "{cle:?} {a:?} {bb:?} {c:?}");
                if [a, bb, c].iter().all(|q| (q[1] - 5.0).abs() < 1e-3) && t.iter().all(|&i| v[i as usize * SOMMET + 4] > 0.999 && v[i as usize * SOMMET + 8] == EAU) {
                    plans += 1;
                }
                // Chaque arête intérieure est partagée par deux triangles (par position)
                for (q, r) in [(a, bb), (bb, c), (c, a)] {
                    let k = |q: [f32; 3]| ((q[0] * 100.0).round() as i64, (q[2] * 100.0).round() as i64);
                    let e = if k(q) < k(r) { (k(q), k(r)) } else { (k(r), k(q)) };
                    *aretes.entry(e).or_insert(0) += 1;
                }
            }
            assert!(plans > 0, "{cle:?}");
            let bord = |x: i64, z: i64| {
                let (x, z) = (x as f32 / 100.0, z as f32 / 100.0);
                (x - b[0]).abs() < 0.02 || (x - b[2]).abs() < 0.02 || (z - b[1]).abs() < 0.02 || (z - b[3]).abs() < 0.02
            };
            for (e, n) in &aretes {
                if !(bord(e.0 .0, e.0 .1) && bord(e.1 .0, e.1 .1)) {
                    assert_eq!(*n, 2, "{cle:?} {e:?}");
                }
            }
        }
    }

    #[test]
    fn les_aretes_d_un_cube_sont_ses_douze_bords() {
        // Un cube en soupe de triangles (deux par face, sommets doublés), 9 flottants
        let c = |x: f32, y: f32, z: f32| [x, y, z, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0];
        let p = [c(0., 0., 0.), c(1., 0., 0.), c(1., 1., 0.), c(0., 1., 0.), c(0., 0., 1.), c(1., 0., 1.), c(1., 1., 1.), c(0., 1., 1.)];
        let faces = [[0, 3, 2, 1], [4, 5, 6, 7], [0, 1, 5, 4], [3, 7, 6, 2], [0, 4, 7, 3], [1, 2, 6, 5]];
        let mut v = Vec::new();
        for f in faces {
            for k in [0, 1, 2, 0, 2, 3] {
                v.extend_from_slice(&p[f[k]]);
            }
        }
        let ind: Vec<u32> = (0..36).collect();
        assert_eq!(aretes(&v, 9, &ind, true).len(), 24);
        // Sans pli (le relief) : ni les faces debout, ni les arêtes qu'elles seules
        // portent ; le dessus et le dessous, avec leur diagonale (2 × 5 arêtes)
        assert_eq!(aretes(&v, 9, &ind, false).len(), 2 * 10);
    }

    #[test]
    fn les_voies_sont_decoupees_a_leur_hauteur_et_cousues() {
        use sage_reel::{Route, GRANDE};
        // Un plateau à 10 m, une route de 8 m posée à 11 m en travers, de biais
        let pas = 4.0;
        let g = Grille::emprise(pas, 2000.0, 2000.0);
        let h = vec![10.0; g.points()];
        let d0 = Donnees::new(Lieu::sur("V", 49.0, 1.0, 0.0, 0.0, pas, g, 0.0, &h, &vec![false; g.points()], Vec::new(), Vec::new(), Vec::new()));
        let (x0, z0, c) = d0.monde();
        let route = Route { genre: GRANDE, largeur: 8.0, points: vec![[x0, z0 + c * 0.53, 11.0], [x0 + c, z0 + c * 0.55, 11.0]] };
        let d = Donnees::new(Lieu::sur("V", 49.0, 1.0, 0.0, 0.0, pas, g, 0.0, &h, &vec![false; g.points()], vec![route], Vec::new(), Vec::new()));
        // Une tuile fine sur la route (cases de moins de VOIRIE_CASE m), et une grossière
        let z = (0..=NIVEAU_MAX).find(|&z| c / (1u32 << z) as f32 / COTES as f32 <= VOIRIE_CASE).unwrap();
        let n = 1i32 << z;
        let cle = (z, n / 2, ((0.54 * n as f32) as i32));
        let (v, ind) = fabriquer(&d, cle);
        let b = boite_tuile(&d, cle);
        let p = |i: u32| [v[i as usize * SOMMET], v[i as usize * SOMMET + 1], v[i as usize * SOMMET + 2], v[i as usize * SOMMET + 8]];
        // La distance à l'axe de la route
        let axe = |x: f32, z: f32| {
            let (a, bb) = (d.lieu.routes[0].points[0], d.lieu.routes[0].points[1]);
            let (ex, ez) = (bb[0] - a[0], bb[1] - a[1]);
            ((x - a[0]) * ez - (z - a[1]) * ex).abs() / ex.hypot(ez)
        };
        let mut dessus = 0;
        for t in ind.chunks(3) {
            let q = [p(t[0]), p(t[1]), p(t[2])];
            let (u, w) = ([q[1][0] - q[0][0], q[1][2] - q[0][2]], [q[2][0] - q[0][0], q[2][2] - q[0][2]]);
            if (u[0] * w[1] - u[1] * w[0]).abs() < 1e-6 {
                continue;
            }
            // Rien plus haut que la route ni plus bas que le plateau (moins les jupes)
            assert!(q.iter().all(|m| m[1] <= 11.0 + crate::voirie3d::TROTTOIR + 1e-3 && m[1] >= 10.0 - crate::voirie3d::JUPE - 1e-3), "{q:?}");
            let (cx, cz) = ((q[0][0] + q[1][0] + q[2][0]) / 3.0, (q[0][2] + q[1][2] + q[2][2]) / 3.0);
            // (hors le fond sous les rubans)
            if q.iter().all(|m| m[3] == TERRE) && !q.iter().all(|m| (m[1] - 11.0 + crate::voirie3d::ENFOUI).abs() < 1e-3 || (m[1] - 11.0 + FOND).abs() < 1e-3) {
                // Pas de terre sous la route, et la terre entre le plateau et la route
                assert!(axe(cx, cz) > 4.0 + 2.0 - 0.5, "{q:?}");
                assert!(q.iter().all(|m| m[1] >= 10.0 - 1e-3), "{q:?}");
            }
            if q.iter().all(|m| m[3] == CHAUSSEE && (m[1] - 11.0).abs() < 1e-3) {
                assert!(axe(cx, cz) < 4.0 + 0.1);
                dessus += 1;
            }
        }
        assert!(dessus >= 1, "{dessus}");
        // La terre au bord de la plateforme à la hauteur de la route ; loin, le plateau
        let terre: Vec<[f32; 4]> = v.chunks(SOMMET).map(|m| [m[0], m[1], m[2], m[8]]).filter(|m| m[3] == TERRE && m[1] > 5.0).collect();
        assert!(terre.iter().filter(|m| (axe(m[0], m[2]) - 6.0).abs() < 0.05).all(|m| (m[1] - 11.0 + crate::voirie3d::ENFOUI).abs() < 0.05));
        for m in terre.iter().filter(|m| axe(m[0], m[2]) > 6.0 + crate::voirie3d::MARGE_MAX + crate::voirie3d::PORTEE) {
            assert!((m[1] - 10.0).abs() < 1e-3, "{m:?} {} {b:?}", axe(m[0], m[2]));
        }
        // De loin, la route n'est pas découpée : tout le relief à 10 m
        let (v, _) = fabriquer(&d, (0, 0, 0));
        assert!(v.chunks(SOMMET).take((COTES + 1) * (COTES + 1)).all(|m| (m[1] - 10.0).abs() < 1e-3));
    }

    #[test]
    fn de_pres_les_tuiles_sont_fines_et_de_loin_grossieres() {
        let d = donnees();
        let mut r = Relief3d::new(&d);
        r.camera.distance = 8000.0;
        r.camera.tangage = 1.3;
        r.poser(&d);
        let mvp = r.mvp(1.5);
        let loin = r.choisir(&d, &mvp);
        assert!(!loin.is_empty());
        r.camera.distance = 60.0;
        r.camera.tangage = 0.3;
        r.poser(&d);
        let mvp = r.mvp(1.5);
        let pres = r.choisir(&d, &mvp);
        let zmax = |v: &[Cle]| v.iter().map(|c| c.0).max().unwrap();
        assert!(zmax(&pres) > zmax(&loin), "{} {}", zmax(&pres), zmax(&loin));
        // La plus proche en premier, et l'œil au-dessus du sol
        assert_eq!(pres[0].0, zmax(&pres));
        let o = r.camera.oeil();
        assert!(o[1] > d.lieu.relief(o[0] as f64, o[2] as f64) as f32);
    }
}
