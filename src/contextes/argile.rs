//! Le contexte Argile : sculpter une forme en 3D, comme de la terre.
//!
//! - **Un doigt sur la forme** sculpte au bout du curseur déporté, avec le pinceau
//!   choisi : Gonfler, Creuser, Lisser, Aplatir, Pincer, Tirer (qui saisit la
//!   matière et l'emmène). Un toucher bref donne un seul coup.
//! - **Un doigt à côté de la forme** la fait tourner ; **deux doigts** rapprochent
//!   (pincer) et déplacent la vue. « Cadrer » remet la forme au centre.
//! - **Le pinceau** : sa taille à l'écran (Petit, Moyen, Grand), sa force (Léger,
//!   Appuyé, Fort), et la symétrie gauche-droite (le coup se répète de l'autre
//!   côté).
//! - **La matière** : quatre couleurs de terre ; « Subdiviser » rend la forme
//!   quatre fois plus fine (et plus douce) pour les détails ; « Nouvelle boule »
//!   repart de zéro.
//! - **En haut** : annuler, rétablir (par coup de pinceau, l'historique dure la
//!   session), le nom, « Enregistrer ».
//!
//! La forme s'enregistre en glTF binaire (`Documents/Argile/<nom>.glb`, ou
//! `sculpture-N.glb`), lisible par Rustcraft, S.A.G.E. et la plupart des outils 3D ;
//! Fichiers ouvre les `.glb` ici. Ce qui n'est pas enregistré est mis à l'abri dans
//! un brouillon (`Flow/brouillons/`).

use crate::clavier::{self, Edition, Frappe};
use crate::contexte::{Commun, Contexte, Etat, Genre};
use crate::draw::{Color, Maillage3d, Rect, Vue3d};
use crate::gestes::Geste;
use crate::glb;
use crate::sculpture::{self as sc, Camera, Instantane, Maillage, Pinceau, Saisie, V3};
use crate::surface::{FenetreId, Placee};
use crate::ui::{id, sub_id, Id};
use crate::widgets::{Cx, Icon};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

pub const CLE: &str = "argile";
pub const GENRE: Genre = Genre { cle: CLE, nom: "Argile", legende: "Sculpture 3D : gonfler, creuser, lisser, tirer", icone: Icon::Argile, ouvrir, applet: None };

const ARGILE: Id = id("argile");
const NOM: Id = id("argile/nom");
const BROUILLONS: &str = "brouillons";
const HISTORIQUE: usize = 30;
/// La finesse d'une nouvelle boule (20 480 triangles).
const NIVEAU: u32 = 5;

/// Les tailles du pinceau à l'écran, en dp (rayon).
const TAILLES: [(&str, f32); 3] = [("Petit", 18.0), ("Moyen", 36.0), ("Grand", 72.0)];
const FORCES: [(&str, f32); 3] = [("Léger", 0.3), ("Appuyé", 0.6), ("Fort", 1.0)];
/// Les terres proposées.
const MATIERES: [[f32; 3]; 4] = [[0.80, 0.47, 0.34], [0.64, 0.62, 0.59], [0.93, 0.91, 0.87], [0.46, 0.58, 0.44]];

/// Un identifiant de maillage par instance, pour le moteur de rendu.
static COMPTEUR: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, PartialEq)]
enum Commande {
    Pinceau(Pinceau),
    Taille(usize),
    Force(usize),
    Symetrie,
    Matiere(usize),
    Subdiviser,
    Cadrer,
    Boule,
}

/// Un doigt qui travaille.
#[derive(Clone, Debug)]
enum Trait {
    /// Sculpte : le dernier point touché, le rayon du coup dans le modèle, et la
    /// forme d'avant.
    Pinceau { dernier: V3, rayon: f32, avant: Instantane },
    /// Emmène la matière saisie en `point`.
    Tirer { saisie: Saisie, point: V3, avant: Instantane },
    /// Fait tourner la vue (le doigt, en pixels).
    Orbite { doigt: (f32, f32) },
}

pub struct Argile {
    chemin: Option<PathBuf>,
    brouillon: Option<PathBuf>,
    modifie: bool,
    nom: Edition,
    charge: bool,
    message: Option<(String, bool)>,
    maillage: Maillage,
    id: u64,
    revision: u64,
    /// Ce qui part vers la carte graphique, refait après une retouche.
    gpu: Option<(u64, Arc<Vec<f32>>, Arc<Vec<[u32; 3]>>, Arc<Vec<u32>>)>,
    passe: Vec<Instantane>,
    futur: Vec<Instantane>,
    camera: Camera,
    cadrer: bool,
    zone: Rect,
    dp: f32,
    pinceau: Pinceau,
    taille: usize,
    force: usize,
    symetrie: bool,
    couleur: [f32; 3],
    trait_: Option<Trait>,
    /// Le curseur pendant un trait, pour montrer le pinceau.
    curseur: Option<(f32, f32)>,
    pincement: f32,
}

impl Default for Argile {
    fn default() -> Argile {
        Argile {
            chemin: None,
            brouillon: None,
            modifie: false,
            nom: Edition::default(),
            charge: false,
            message: None,
            maillage: Maillage::boule(NIVEAU, 1.0),
            id: COMPTEUR.fetch_add(1, Ordering::Relaxed),
            revision: 0,
            gpu: None,
            passe: Vec::new(),
            futur: Vec::new(),
            camera: Camera::default(),
            cadrer: false,
            zone: Rect::new(0.0, 0.0, 1.0, 1.0),
            dp: 1.0,
            pinceau: Pinceau::Gonfler,
            taille: 1,
            force: 1,
            symetrie: true,
            couleur: MATIERES[0],
            trait_: None,
            curseur: None,
            pincement: 1.0,
        }
    }
}

fn ouvrir(e: &Etat) -> Box<dyn Contexte> {
    let mut a = Argile {
        chemin: e.texte("document").filter(|t| !t.is_empty()).map(PathBuf::from),
        brouillon: e.texte("brouillon").map(PathBuf::from),
        nom: Edition::new(e.texte("nom").unwrap_or("")),
        ..Argile::default()
    };
    if let Some(p) = e.texte("pinceau").and_then(Pinceau::depuis) {
        a.pinceau = p;
    }
    if let Some(v) = e.oui("symetrie") {
        a.symetrie = v;
    }
    a.taille = e.nombre("taille").map_or(1, |t| (t as usize).min(TAILLES.len() - 1));
    a.force = e.nombre("force").map_or(1, |t| (t as usize).min(FORCES.len() - 1));
    match e.nombres("vue")[..] {
        [l, t, d, x, y, z] => a.camera = Camera { lacet: l, tangage: t, distance: d.max(0.05), cible: [x, y, z] },
        _ => a.cadrer = a.chemin.is_some() || a.brouillon.is_some(),
    }
    Box::new(a)
}

/// Écrit des octets d'un coup (par un fichier temporaire renommé).
fn ecrire_octets(chemin: &Path, octets: &[u8]) -> Result<(), String> {
    if let Some(d) = chemin.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let tmp = chemin.with_extension("glb~");
    std::fs::write(&tmp, octets).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, chemin).map_err(|e| e.to_string())
}

fn teinte(c: [f32; 3]) -> Color {
    [(c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8, 255]
}

impl Argile {
    /// Le maillage a changé.
    fn retouche(&mut self) {
        self.revision += 1;
    }

    /// Lit le brouillon s'il y en a un, sinon le fichier.
    fn charger(&mut self) {
        self.charge = true;
        let (octets, brouillon) = match self.brouillon.as_ref().and_then(|b| std::fs::read(b).ok()) {
            Some(o) => (Some(o), true),
            None => {
                self.brouillon = None;
                match &self.chemin {
                    Some(c) => match std::fs::read(c) {
                        Ok(o) => (Some(o), false),
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                            self.message = Some(("Fichier introuvable : il sera créé à l'enregistrement.".into(), true));
                            (None, false)
                        }
                        Err(e) => {
                            self.message = Some((e.to_string(), true));
                            (None, false)
                        }
                    },
                    None => (None, false),
                }
            }
        };
        let Some(o) = octets else { return };
        match glb::lire(&o) {
            Ok(l) => {
                self.maillage = Maillage::nouveau(l.positions, l.triangles);
                if let Some(c) = l.couleur {
                    self.couleur = c;
                }
                self.modifie = brouillon;
                self.retouche();
                if brouillon {
                    self.message = Some(("Brouillon non enregistré retrouvé".into(), false));
                }
            }
            Err(e) => self.message = Some((e, true)),
        }
    }

    /// Ce qui part vers la carte graphique (refait seulement après une retouche).
    fn maillage3d(&mut self) -> Maillage3d {
        let a_jour = self.gpu.as_ref().is_some_and(|g| g.0 == self.revision);
        if !a_jour {
            let sommets = Arc::new(self.maillage.plat());
            // Les indices ne changent qu'avec la topologie
            let indices = match &self.gpu {
                Some((_, _, t, i)) if Arc::ptr_eq(t, &self.maillage.triangles) => i.clone(),
                _ => Arc::new(self.maillage.indices()),
            };
            self.gpu = Some((self.revision, sommets, self.maillage.triangles.clone(), indices));
        }
        let g = self.gpu.as_ref().unwrap();
        Maillage3d { id: self.id, revision: self.revision, sommets: g.1.clone(), indices: g.3.clone() }
    }

    /// Retient la forme d'avant une retouche, pour l'annuler.
    fn noter(&mut self, avant: Instantane) {
        if avant.positions == self.maillage.positions && Arc::ptr_eq(&avant.triangles, &self.maillage.triangles) {
            return;
        }
        self.passe.push(avant);
        if self.passe.len() > HISTORIQUE {
            self.passe.remove(0);
        }
        self.futur.clear();
        self.modifie = true;
        self.message = None;
    }

    fn defaire(&mut self, retablir: bool) {
        let (de, vers) = if retablir { (&mut self.futur, &mut self.passe) } else { (&mut self.passe, &mut self.futur) };
        let Some(i) = de.pop() else { return };
        vers.push(self.maillage.instantane());
        self.maillage.restaurer(i);
        self.trait_ = None;
        self.modifie = true;
        self.retouche();
    }

    /// Le rayon qui passe par le point (x, y) de l'écran.
    fn rayon(&self, x: f32, y: f32) -> (V3, V3) {
        let z = self.zone;
        let nx = (x - z.x) / z.w.max(1.0) * 2.0 - 1.0;
        let ny = 1.0 - (y - z.y) / z.h.max(1.0) * 2.0;
        self.camera.rayon(z.w / z.h.max(1.0), nx, ny)
    }

    /// Le rayon du pinceau dans le modèle, au point `p`.
    fn rayon_pinceau(&self, p: V3) -> f32 {
        self.camera.taille_a(p, TAILLES[self.taille].1 * self.dp, self.zone.h)
    }

    fn coup(&mut self, centre: V3, rayon: f32) {
        sc::appliquer(&mut self.maillage, self.pinceau, centre, rayon, FORCES[self.force].1, self.symetrie);
        self.retouche();
    }

    fn recevoir(&mut self, g: Geste) {
        match g {
            Geste::Tap { x, y, .. } if self.pinceau != Pinceau::Tirer => {
                let (o, d) = self.rayon(x, y);
                if let Some((_, p)) = self.maillage.toucher(o, d) {
                    let avant = self.maillage.instantane();
                    let r = self.rayon_pinceau(p);
                    self.coup(p, r);
                    self.noter(avant);
                }
            }
            Geste::SaisieDebut { x, y, doigt } => {
                let (o, d) = self.rayon(x, y);
                self.curseur = Some((x, y));
                self.trait_ = Some(match self.maillage.toucher(o, d) {
                    Some((_, p)) => {
                        let avant = self.maillage.instantane();
                        let rayon = self.rayon_pinceau(p);
                        if self.pinceau == Pinceau::Tirer {
                            Trait::Tirer { saisie: Saisie::prendre(&self.maillage, p, rayon, self.symetrie), point: p, avant }
                        } else {
                            self.coup(p, rayon);
                            Trait::Pinceau { dernier: p, rayon, avant }
                        }
                    }
                    None => {
                        self.curseur = None;
                        Trait::Orbite { doigt }
                    }
                });
            }
            Geste::SaisieDeplace { x, y, doigt } => {
                let (o, d) = self.rayon(x, y);
                match self.trait_.take() {
                    Some(Trait::Pinceau { dernier, rayon, avant }) => {
                        self.curseur = Some((x, y));
                        let mut dernier = dernier;
                        if let Some((_, p)) = self.maillage.toucher(o, d) {
                            // Des coups réguliers le long du chemin
                            let pas = rayon * 0.25;
                            let l = sc::norme(sc::oter(p, dernier));
                            if l >= pas {
                                let n = ((l / pas).ceil() as usize).min(24);
                                for j in 1..=n {
                                    let c = sc::ajouter(dernier, sc::fois(sc::oter(p, dernier), j as f32 / n as f32));
                                    sc::coup(&mut self.maillage, self.pinceau, c, rayon, FORCES[self.force].1, self.symetrie);
                                }
                                self.maillage.normaliser();
                                self.retouche();
                                dernier = p;
                            }
                        }
                        self.trait_ = Some(Trait::Pinceau { dernier, rayon, avant });
                    }
                    Some(Trait::Tirer { saisie, point, avant }) => {
                        self.curseur = Some((x, y));
                        let q = self.camera.sur_plan(point, o, d);
                        saisie.deplacer(&mut self.maillage, sc::oter(q, point));
                        self.retouche();
                        self.trait_ = Some(Trait::Tirer { saisie, point, avant });
                    }
                    Some(Trait::Orbite { doigt: avant }) => {
                        let k = 0.006 / self.dp.max(0.1);
                        self.camera.lacet -= (doigt.0 - avant.0) * k;
                        self.camera.tangage = (self.camera.tangage + (doigt.1 - avant.1) * k).clamp(-1.45, 1.45);
                        self.trait_ = Some(Trait::Orbite { doigt });
                    }
                    None => {}
                }
            }
            Geste::SaisieFin { .. } => {
                self.curseur = None;
                match self.trait_.take() {
                    Some(Trait::Pinceau { avant, .. } | Trait::Tirer { avant, .. }) => self.noter(avant),
                    _ => {}
                }
            }
            Geste::SaisieAnnulee { .. } => {
                self.curseur = None;
                if let Some(Trait::Pinceau { avant, .. } | Trait::Tirer { avant, .. }) = self.trait_.take() {
                    self.maillage.restaurer(avant);
                    self.retouche();
                }
            }
            Geste::PincementDebut { .. } => self.pincement = 1.0,
            Geste::Pincement { echelle, deplacement, .. } => {
                let f = echelle / self.pincement.max(1e-3);
                self.pincement = echelle;
                // Glisser deux doigts déplace la vue avec eux
                let (_, r, u) = self.camera.axes();
                let k = self.camera.taille_a(self.camera.cible, 1.0, self.zone.h);
                self.camera.cible = sc::oter(self.camera.cible, sc::ajouter(sc::fois(r, deplacement.0 * k), sc::fois(u, -deplacement.1 * k)));
                self.camera.distance = (self.camera.distance / f.max(1e-3)).clamp(0.05, 500.0);
            }
            _ => {}
        }
    }

    fn commander(&mut self, c: Commande) {
        match c {
            Commande::Pinceau(p) => self.pinceau = p,
            Commande::Taille(t) => self.taille = t,
            Commande::Force(f) => self.force = f,
            Commande::Symetrie => self.symetrie = !self.symetrie,
            Commande::Matiere(i) => {
                if self.couleur != MATIERES[i] {
                    self.couleur = MATIERES[i];
                    self.modifie = true;
                }
            }
            Commande::Subdiviser => {
                let avant = self.maillage.instantane();
                if self.maillage.subdiviser() {
                    self.noter(avant);
                    self.retouche();
                    self.message = Some((format!("{} triangles", self.maillage.triangles.len()), false));
                } else {
                    self.message = Some((format!("Déjà assez fin : au-delà de {} triangles, le pinceau deviendrait lent.", sc::TRIANGLES_MAX), true));
                }
            }
            Commande::Cadrer => self.cadrer = true,
            Commande::Boule => {
                let avant = self.maillage.instantane();
                self.maillage = Maillage::boule(NIVEAU, 1.0);
                self.noter(avant);
                self.retouche();
                self.cadrer = true;
            }
        }
    }

    fn destination(&self, c: &Commun) -> Result<PathBuf, String> {
        if let Some(p) = &self.chemin {
            return Ok(p.clone());
        }
        let dossier = c.racine.as_ref().map(|r| r.join("Documents").join("Argile")).or_else(|| c.dossier.clone()).ok_or("Aucun dossier où enregistrer.")?;
        let saisi = self.nom.texte.trim().trim_end_matches(".glb").to_string();
        if saisi.contains('/') || saisi.starts_with('.') {
            return Err("Nom de fichier invalide.".into());
        }
        if saisi.is_empty() {
            return (1..).map(|n| dossier.join(format!("sculpture-{n}.glb"))).find(|p| !p.exists()).ok_or_else(|| "Aucun nom libre.".into());
        }
        let p = dossier.join(format!("{saisi}.glb"));
        if p.exists() {
            return Err(format!("{saisi}.glb existe déjà : choisissez un autre nom."));
        }
        Ok(p)
    }

    fn octets(&self, nom: &str) -> Vec<u8> {
        let m = &self.maillage;
        glb::ecrire(&m.positions, &m.normales, &m.triangles, self.couleur, nom)
    }

    fn enregistrer(&mut self, c: &Commun) -> Result<(), String> {
        let chemin = self.destination(c)?;
        let nom = chemin.file_stem().map_or("argile".into(), |n| n.to_string_lossy().into_owned());
        ecrire_octets(&chemin, &self.octets(&nom))?;
        self.chemin = Some(chemin);
        self.modifie = false;
        self.nom = Edition::default();
        if let Some(b) = self.brouillon.take() {
            let _ = std::fs::remove_file(b);
        }
        Ok(())
    }

    fn nom_affiche(&self) -> String {
        match &self.chemin {
            Some(p) => p.file_name().map_or(p.display().to_string(), |n| n.to_string_lossy().into_owned()),
            None => "Nouvelle sculpture".into(),
        }
    }

    /// Les commandes de la barre du bas, par groupes (chacun sur sa ligne).
    fn commandes(&self) -> Vec<Vec<(Commande, String, bool)>> {
        let mut g = vec![Pinceau::TOUS.iter().map(|p| (Commande::Pinceau(*p), p.nom().to_string(), *p == self.pinceau)).collect::<Vec<_>>()];
        let mut reglages: Vec<(Commande, String, bool)> = TAILLES.iter().enumerate().map(|(i, t)| (Commande::Taille(i), t.0.to_string(), i == self.taille)).collect();
        reglages.extend(FORCES.iter().enumerate().map(|(i, f)| (Commande::Force(i), f.0.to_string(), i == self.force)));
        reglages.push((Commande::Symetrie, "Symétrie".into(), self.symetrie));
        g.push(reglages);
        let mut matiere: Vec<(Commande, String, bool)> = (0..MATIERES.len()).map(|i| (Commande::Matiere(i), String::new(), self.couleur == MATIERES[i])).collect();
        matiere.extend([(Commande::Subdiviser, "Subdiviser".into(), false), (Commande::Cadrer, "Cadrer".into(), false), (Commande::Boule, "Nouvelle boule".into(), false)]);
        g.push(matiere);
        g
    }

    /// La barre du bas : sa hauteur, et (si `dessiner`) la commande touchée.
    fn barre(&self, cx: &mut Cx, r: Rect, k: usize, dessiner: bool) -> (f32, Option<Commande>) {
        let (d, col) = (cx.dp, cx.couleurs());
        let (hc, gap) = (cx.commande(), cx.ecart());
        let pad = 10.0 * d;
        let place = (r.w - 2.0 * pad).max(0.0);
        let mut y = r.y + gap;
        let mut choisie = None;
        if dessiner {
            cx.dl.rrect(r, 0.0, col.surface);
            cx.dl.rrect(Rect::new(r.x, r.y, r.w, d.max(1.0)), 0.0, col.ligne);
        }
        let mut n = 0;
        for groupe in self.commandes() {
            let largeurs: Vec<f32> = groupe.iter().map(|(c, nom, _)| if matches!(c, Commande::Matiere(_)) { hc * 0.8 } else { cx.pill_width(nom) }).collect();
            let rangs = super::fichiers::ranger(&largeurs, place, gap * 0.6);
            let lignes = rangs.iter().map(|x| x.1).max().unwrap_or(0) + 1;
            if dessiner {
                for (i, (c, nom, actif)) in groupe.iter().enumerate() {
                    let (x, l) = rangs[i];
                    let (x, yy) = (r.x + pad + x, y + l as f32 * (hc + gap * 0.6));
                    n += 1;
                    let kc = sub_id(ARGILE, k * 64 + n);
                    if let Commande::Matiere(m) = c {
                        let case = Rect::new(x, yy, largeurs[i], hc);
                        if cx.ui.click(kc, case, None) {
                            choisie = Some(*c);
                        }
                        let rayon = case.w * 0.36;
                        let (ccx, ccy) = (case.cx(), case.cy());
                        if *actif {
                            cx.dl.secteur([ccx, ccy], rayon + 3.0 * d, rayon + 5.0 * d, 0.0, std::f32::consts::TAU, 0.0, col.texte);
                        }
                        cx.dl.rrect(Rect::new(ccx - rayon, ccy - rayon, 2.0 * rayon, 2.0 * rayon), rayon, teinte(MATIERES[*m]));
                        cx.dl.secteur([ccx, ccy], rayon - d, rayon, 0.0, std::f32::consts::TAU, 0.0, [col.pale[0], col.pale[1], col.pale[2], 140]);
                    } else if cx.pill(kc, x, yy, nom, *actif) {
                        choisie = Some(*c);
                    }
                }
            }
            y += lignes as f32 * (hc + gap * 0.6);
        }
        (y - r.y + gap * 0.4, choisie)
    }

    /// En haut : annuler, rétablir, le nom, « Enregistrer ».
    fn entete(&mut self, cx: &mut Cx, c: &mut Commun, p: &Placee, r: Rect) {
        let (d, col) = (cx.dp, cx.couleurs());
        let (h, m, gap) = (cx.commande(), cx.coin(), cx.ecart());
        let k = p.id as usize;
        cx.dl.rrect(r, 0.0, col.surface);
        let mut x0 = p.suite.0;
        for (j, (icone, retablir)) in [(Icon::Annuler, false), (Icon::Retablir, true)].into_iter().enumerate() {
            let case = Rect::new(x0, r.y + m, h, h);
            let possible = !if retablir { &self.futur } else { &self.passe }.is_empty();
            let kb = sub_id(ARGILE, 10_000 + k * 2 + j);
            if cx.ui.click(kb, case, None) && possible {
                self.defaire(retablir);
            }
            if possible && cx.ui.pressed(kb) {
                cx.dl.rrect(case.inset(2.0 * d), cx.theme.rayon(case.h - 4.0 * d), col.puce);
            }
            cx.icon(case.inset(cx.pad() * 0.6), icone, if possible { col.texte } else { col.ligne });
            x0 += h;
        }
        x0 += gap;
        let enregistrer = "Enregistrer";
        let we = cx.pill_width(enregistrer);
        let xe = r.right() - we - m;
        if xe > x0 + gap && cx.pill(sub_id(ARGILE, 20_001 + k), xe, r.y + m, enregistrer, self.modifie) {
            self.message = Some(match self.enregistrer(c) {
                Ok(()) => (format!("Enregistré : {}", self.chemin.as_deref().map_or(String::new(), |p| p.display().to_string())), false),
                Err(e) => (e, true),
            });
        }
        let x1 = (xe - gap).max(x0);
        if let Some(chemin) = &self.chemin {
            let dossier = chemin.parent().map_or(String::new(), |p| p.display().to_string());
            cx.clip(Some(Rect::new(x0, r.y, x1 - x0, r.h)));
            let pastille = if self.modifie { " •" } else { "" };
            cx.text_at(x0, r.y + m + h * 0.32, &format!("{}{pastille}", self.nom_affiche()), 16.0, col.texte);
            cx.text_at(x0, r.y + m + h * 0.78, &dossier, 12.0, col.pale);
            cx.clip(None);
        } else {
            let pad = 14.0 * d;
            let zone = Rect::new(x0 - pad, r.y + m - 8.0 * d - (cx.theme.marge * 0.4 * d).round(), x1 - x0 + 2.0 * pad, r.h);
            // Posé à sa place, hors d'une colonne : rien ne le fait défiler
            let ch = Rect::new(zone.x + 14.0 * d, zone.y + 8.0 * d + (cx.theme.marge * 0.4 * d).round(), (zone.w - 28.0 * d).max(0.0), cx.commande());
            clavier::champ_en(cx, ch, None, zone, sub_id(NOM, k), "Nom de la sculpture", &mut self.nom, &mut c.clavier);
            cx.clip(None);
        }
        cx.dl.rrect(Rect::new(r.x, r.bottom() - d, r.w, d.max(1.0)), 0.0, col.ligne);
    }
}

impl Contexte for Argile {
    fn cle(&self) -> &'static str {
        CLE
    }

    /// Une nouvelle boule, avec le même pinceau.
    fn dupliquer(&self) -> Box<dyn Contexte> {
        Box::new(Argile { pinceau: self.pinceau, taille: self.taille, force: self.force, symetrie: self.symetrie, couleur: self.couleur, ..Argile::default() })
    }

    fn document(&self) -> Option<&Path> {
        self.chemin.as_deref()
    }

    fn ecrire(&self, e: &mut Etat) {
        if let Some(c) = &self.chemin {
            e.mettre("document", c.display());
        } else if !self.nom.texte.trim().is_empty() {
            e.mettre("nom", self.nom.texte.trim());
        }
        if let Some(b) = &self.brouillon {
            e.mettre("brouillon", b.display());
        }
        let c = &self.camera;
        e.mettre("vue", format!("{} {} {} {} {} {}", c.lacet, c.tangage, c.distance, c.cible[0], c.cible[1], c.cible[2]));
        e.mettre("pinceau", self.pinceau.nom());
        e.mettre("taille", self.taille);
        e.mettre("force", self.force);
        e.mettre_oui("symetrie", self.symetrie);
    }

    fn retour(&mut self, _c: &Commun) {
        // Le fichier a pu changer : relu s'il n'y a rien de neuf ici
        if self.charge && !self.modifie && self.chemin.is_some() && self.trait_.is_none() {
            self.charger();
        }
    }

    fn mettre_a_l_abri(&mut self, c: &Commun) {
        if !self.charge {
            return;
        }
        if !self.modifie {
            if let Some(b) = self.brouillon.take() {
                let _ = std::fs::remove_file(b);
            }
            return;
        }
        let nouveau = || {
            let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
            c.dossier.as_ref().map(|d| d.join(BROUILLONS).join(format!("argile-{t}.glb")))
        };
        let Some(chemin) = self.brouillon.clone().or_else(nouveau) else { return };
        match ecrire_octets(&chemin, &self.octets("brouillon")) {
            Ok(()) => self.brouillon = Some(chemin),
            Err(e) => crate::android::log(&format!("brouillon Argile non écrit : {e}")),
        }
    }

    fn frappe(&mut self, f: FenetreId, cible: Id, frappe: Frappe, c: &mut Commun) -> bool {
        if cible == sub_id(NOM, f as usize) {
            if self.nom.appliquer(frappe) {
                c.clavier.fermer();
            }
            true
        } else {
            false
        }
    }

    fn dessiner(&mut self, cx: &mut Cx, c: &mut Commun, p: &Placee) -> String {
        if !self.charge {
            self.charger();
        }
        let r = p.contenu;
        let k = p.id as usize;
        self.dp = cx.dp;
        let col = cx.couleurs();
        let haut = (cx.commande() + 2.0 * cx.coin()).min(r.h / 3.0);
        let corps = Rect::new(r.x, r.y + haut, r.w, (r.h - haut).max(0.0));
        let (hb, _) = self.barre(cx, Rect::new(r.x, 0.0, r.w, 0.0), k, false);
        let hb = hb.min(corps.h * 0.6);
        let z = Rect::new(r.x, corps.y, r.w, (corps.h - hb).max(0.0));
        self.zone = z;
        if std::mem::take(&mut self.cadrer) {
            self.camera.cadrer(self.maillage.boite());
        }
        for g in cx.ui.gestes(sub_id(ARGILE, 30_000 + k), z) {
            self.recevoir(g);
        }

        // Le fond, la forme par-dessus, puis le pinceau
        cx.dl.rrect(z, 0.0, col.fond);
        if z.w >= 2.0 && z.h >= 2.0 {
            let m = self.maillage3d();
            cx.clip(Some(z));
            cx.dl.vue3d(Vue3d { rect: z, mvp: self.camera.mvp(z.w / z.h), vue: self.camera.vue(), maillage: m, couleur: self.couleur, dessus: None });
            if let Some((x, y)) = self.curseur {
                let rp = TAILLES[self.taille].1 * self.dp;
                let a = col.accent;
                cx.dl.secteur([x, y], rp - 1.0 * self.dp, rp + 1.0 * self.dp, 0.0, std::f32::consts::TAU, 0.0, [a[0], a[1], a[2], 200]);
            }
            cx.clip(None);
        }
        if let (_, Some(cmd)) = self.barre(cx, Rect::new(r.x, z.bottom(), r.w, hb), k, true) {
            self.commander(cmd);
        }
        self.entete(cx, c, p, Rect::new(r.x, r.y, r.w, haut));

        if let Some((m, _)) = &self.message {
            return m.clone();
        }
        let sym = if self.symetrie { " · symétrie" } else { "" };
        let etat = if self.modifie { " · non enregistré" } else { "" };
        format!("{} · {} triangles · {}{sym}{etat}", self.nom_affiche(), self.maillage.triangles.len(), self.pinceau.nom())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pret() -> Argile {
        let mut a = Argile { zone: Rect::new(0.0, 0.0, 800.0, 600.0), dp: 1.0, charge: true, ..Argile::default() };
        a.maillage = Maillage::boule(3, 1.0);
        a.camera = Camera { lacet: 0.0, tangage: 0.0, distance: 4.0, cible: [0.0; 3] };
        a.symetrie = false;
        a
    }

    fn glisser(a: &mut Argile, de: (f32, f32), vers: (f32, f32)) {
        a.recevoir(Geste::SaisieDebut { x: de.0, y: de.1, doigt: de });
        for k in 1..=4 {
            let t = k as f32 / 4.0;
            let p = (de.0 + (vers.0 - de.0) * t, de.1 + (vers.1 - de.1) * t);
            a.recevoir(Geste::SaisieDeplace { x: p.0, y: p.1, doigt: p });
        }
        a.recevoir(Geste::SaisieFin { x: vers.0, y: vers.1, depart: de, vecteur: (vers.0 - de.0, vers.1 - de.1) });
    }

    #[test]
    fn sculpter_tourner_annuler() {
        let mut a = pret();
        let avant = a.maillage.positions.clone();
        // Un trait sur la face avant la gonfle
        glisser(&mut a, (380.0, 300.0), (420.0, 300.0));
        assert_eq!(a.passe.len(), 1);
        assert!(a.modifie);
        let z_max = |m: &Maillage| m.positions.iter().map(|p| p[2]).fold(f32::MIN, f32::max);
        assert!(z_max(&a.maillage) > 1.01);
        // À côté de la forme, le doigt tourne la vue sans rien changer
        let lacet = a.camera.lacet;
        glisser(&mut a, (20.0, 20.0), (120.0, 20.0));
        assert_ne!(a.camera.lacet, lacet);
        assert_eq!(a.passe.len(), 1);
        // Annuler, rétablir
        a.defaire(false);
        assert_eq!(a.maillage.positions, avant);
        a.defaire(true);
        assert!(z_max(&a.maillage) > 1.01);
    }

    #[test]
    fn tirer_toucher_subdiviser() {
        let mut a = pret();
        a.commander(Commande::Pinceau(Pinceau::Tirer));
        // Tirer le centre vers la droite : la matière suit
        glisser(&mut a, (400.0, 300.0), (500.0, 300.0));
        assert!(a.maillage.positions.iter().any(|p| p[0] > 0.3 && p[2] > 0.8));
        // Un toucher bref avec Creuser donne un coup
        a.commander(Commande::Pinceau(Pinceau::Creuser));
        let n = a.passe.len();
        a.recevoir(Geste::Tap { x: 400.0, y: 250.0, intensite: 0.0, precision: 0.0 });
        assert_eq!(a.passe.len(), n + 1);
        // Subdiviser, puis annuler revient à l'ancienne topologie
        let t = a.maillage.triangles.len();
        a.commander(Commande::Subdiviser);
        assert_eq!(a.maillage.triangles.len(), t * 4);
        a.defaire(false);
        assert_eq!(a.maillage.triangles.len(), t);
    }

    #[test]
    fn enregistrer_et_relire() {
        let d = std::env::temp_dir().join(format!("flow-argile-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let c = Commun { racine: Some(d.clone()), dossier: Some(d.join("Flow")), ..Commun::default() };
        let mut a = pret();
        glisser(&mut a, (380.0, 300.0), (420.0, 300.0));
        a.commander(Commande::Matiere(2));
        a.nom = Edition::new("tete");
        a.enregistrer(&c).unwrap();
        let f = d.join("Documents/Argile/tete.glb");
        assert!(f.exists());
        assert!(!a.modifie);
        // Une autre fenêtre le relit à l'identique
        let mut e = Etat::default();
        a.ecrire(&mut e);
        let mut b = Argile { chemin: Some(f), ..Argile::default() };
        b.charger();
        assert_eq!(b.maillage.positions, a.maillage.positions);
        assert_eq!(b.couleur, MATIERES[2]);
        // Le brouillon garde ce qui n'est pas enregistré
        a.recevoir(Geste::Tap { x: 400.0, y: 300.0, intensite: 0.0, precision: 0.0 });
        a.mettre_a_l_abri(&c);
        let br = a.brouillon.clone().unwrap();
        assert!(br.exists());
        let mut r = Argile { brouillon: Some(br), ..Argile::default() };
        r.charger();
        assert!(r.modifie);
        assert_eq!(r.maillage.positions, a.maillage.positions);
        let _ = std::fs::remove_dir_all(d);
    }
}
