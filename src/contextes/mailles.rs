//! Le contexte Mailles : éditer la géométrie d'un `.glb`, sommet par sommet, arête
//! par arête, face par face.
//!
//! - **Des vues dans la fenêtre**, partagées comme l'espace de travail de Flow (le
//!   titre de chaque vue ouvre son menu : changer de vue, la partager, la fermer) :
//!   la **Vue 3D** (autant qu'on veut, chacune avec sa caméra), l'**Arborescence**
//!   (les objets du fichier) et les **Opérations** (le tableau de tout ce qu'on peut
//!   faire, et quand).
//! - **Vue 3D** : en haut, le mode (Sommets, Arêtes, Faces) et « Cadrer » ; toucher un
//!   élément l'ajoute à la sélection ou l'en retire, toucher le vide la vide, toucher
//!   un autre objet le rend actif. Un doigt qui glisse fait tourner la vue, deux
//!   doigts rapprochent et déplacent. En bas, la **barre rapide** : les opérations que
//!   la sélection permet (`maille::OPERATIONS`). Déplacer, Tourner et Échelle
//!   s'allument : glisser transforme alors la sélection au lieu de tourner la vue.
//! - **En haut** : annuler, rétablir, le nom, « Enregistrer ».
//!
//! Le fichier se réécrit en glTF binaire, un nœud par objet à sa place d'origine. Ce
//! que Mailles ne sait pas réécrire (armatures, animations, textures) n'est pas
//! perdu : le premier enregistrement va alors dans une copie (`nom-mailles.glb`).

use crate::clavier::{self, Edition, Frappe};
use crate::contexte::{Commun, Contexte, Etat, Genre};
use crate::draw::{Color, Maillage3d, Rect, Vue3d};
use crate::gestes::Geste;
use crate::glb;
use crate::maille::{self as m, Element, Mode, Objet, Selection, Transfo};
use crate::sculpture::{self as sc, Camera, V3};
use crate::surface::{Changement, FenetreId, Placee, Sens, Surface};
use crate::ui::{id, sub_id, Id, Sense};
use crate::widgets::{Cx, Icon};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

pub const CLE: &str = "mailles";
pub const GENRE: Genre = Genre { cle: CLE, nom: "Mailles", legende: "Éditer un .glb : sommets, arêtes, faces", icone: Icon::Maille, ouvrir, applet: None };

const MAILLES: Id = id("mailles");
const NOM: Id = id("mailles/nom");
const NOM_CORPS: Id = id("mailles/nom-corps");
const BOITE: Id = id("mailles/boite");
const BROUILLONS: &str = "brouillons";
const HISTORIQUE: usize = 40;

/// Les vues de la fenêtre.
const VUES: [(&str, Icon); 3] = [("Vue 3D", Icon::Cube), ("Arborescence", Icon::List), ("Opérations", Icon::Menu)];
const V3D: u32 = 0;
const ARBRE: u32 = 1;
const OPS: u32 = 2;

/// Au-delà, la vue 3D ne montre que les arêtes choisies (le dessin deviendrait lent).
const ARETES_MAX: usize = 20_000;

/// Des identifiants de maillage pour le moteur de rendu, loin de ceux d'Argile.
static COMPTEUR: AtomicU64 = AtomicU64::new(1 << 40);

/// Ce qu'on retient pour annuler.
#[derive(Clone)]
struct Instantane {
    objets: Vec<Objet>,
    actif: usize,
    sel: Selection,
    mode: Mode,
}

/// Un doigt qui travaille dans une vue 3D.
#[derive(Clone)]
enum Trait {
    Orbite { doigt: (f32, f32) },
    /// Transforme la sélection : le doigt de départ, les sommets et leur place de
    /// départ, le centre, et la forme d'avant.
    Transfo { depart: (f32, f32), base: Vec<(u32, V3)>, centre: V3, avant: Box<Instantane> },
}

pub struct Mailles {
    chemin: Option<PathBuf>,
    brouillon: Option<PathBuf>,
    modifie: bool,
    nom: Edition,
    charge: bool,
    message: Option<(String, bool)>,
    /// Ce que le fichier lu porte et qu'on ne réécrit pas.
    perdu: Vec<&'static str>,
    objets: Vec<Objet>,
    actif: usize,
    sel: Selection,
    mode: Mode,
    /// La transformation allumée, et son axe (après une extrusion).
    transfo: Option<(Transfo, Option<V3>)>,
    passe: Vec<Instantane>,
    futur: Vec<Instantane>,
    /// Les vues de la fenêtre, et la caméra et la zone de chaque vue 3D.
    vues: Surface,
    cameras: HashMap<FenetreId, Camera>,
    zones: HashMap<FenetreId, Rect>,
    /// La dernière vue 3D touchée : les opérations s'y rapportent (le regard, le centre).
    derniere: FenetreId,
    cadrer: bool,
    trait_: Option<(FenetreId, Trait)>,
    pincement: f32,
    id: u64,
    revision: u64,
    gpu: Option<(u64, Arc<Vec<f32>>, Arc<Vec<u32>>, Arc<Vec<(usize, usize)>>)>,
    surligne: Option<(u64, Arc<Vec<u32>>)>,
    dp: f32,
    /// La boîte du premier enregistrement est ouverte : le nom se saisit dans `nom` ;
    /// l'erreur de la dernière tentative ; Entrée a été frappée.
    boite: bool,
    erreur: Option<String>,
    valider: bool,
}

/// Les vues de départ : la 3D à gauche, l'arborescence et les opérations à droite.
fn vues_par_defaut() -> Surface {
    let mut s = Surface::new(V3D);
    if let Some(droite) = s.partager_apres(1, Sens::Colonnes, 0.68) {
        s.changer_vue(droite, ARBRE);
        if let Some(bas) = s.partager_apres(droite, Sens::Lignes, 0.45) {
            s.changer_vue(bas, OPS);
        }
    }
    s.changements();
    s.modifiee = false;
    s
}

impl Default for Mailles {
    fn default() -> Mailles {
        Mailles {
            chemin: None,
            brouillon: None,
            modifie: false,
            nom: Edition::default(),
            charge: false,
            message: None,
            perdu: Vec::new(),
            objets: vec![m::cube("Cube", [0.0; 3], 1.0)],
            actif: 0,
            sel: Selection::default(),
            mode: Mode::Sommets,
            transfo: None,
            passe: Vec::new(),
            futur: Vec::new(),
            vues: vues_par_defaut(),
            cameras: HashMap::new(),
            zones: HashMap::new(),
            derniere: 1,
            cadrer: true,
            trait_: None,
            pincement: 1.0,
            id: COMPTEUR.fetch_add(2, Ordering::Relaxed),
            revision: 0,
            gpu: None,
            surligne: None,
            dp: 1.0,
            boite: false,
            erreur: None,
            valider: false,
        }
    }
}

fn ouvrir(e: &Etat) -> Box<dyn Contexte> {
    let mut a = Mailles {
        chemin: e.texte("document").filter(|t| !t.is_empty()).map(PathBuf::from),
        brouillon: e.texte("brouillon").map(PathBuf::from),
        nom: Edition::new(e.texte("nom").unwrap_or("")),
        ..Mailles::default()
    };
    if let Some(md) = e.texte("mode").and_then(Mode::depuis) {
        a.mode = md;
    }
    if let Some(s) = e.texte("vues").and_then(|t| Surface::lire_avec(t, &|n| VUES.iter().position(|v| v.0 == n).map(|i| i as u32))) {
        a.vues = s;
    }
    for p in a.vues.disposer(Rect::default()) {
        if let [l, t, d, x, y, z] = e.nombres(&format!("camera{}", p.0))[..] {
            a.cameras.insert(p.0, Camera { lacet: l, tangage: t, distance: d.max(0.01), cible: [x, y, z] });
        }
    }
    a.cadrer = a.cameras.is_empty();
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

/// Un point de la scène sur l'écran, dans la zone `z` ; rien s'il est derrière l'œil.
fn projeter(mvp: &[f32; 16], z: Rect, p: V3) -> Option<[f32; 2]> {
    let x = mvp[0] * p[0] + mvp[4] * p[1] + mvp[8] * p[2] + mvp[12];
    let y = mvp[1] * p[0] + mvp[5] * p[1] + mvp[9] * p[2] + mvp[13];
    let w = mvp[3] * p[0] + mvp[7] * p[1] + mvp[11] * p[2] + mvp[15];
    (w > 1e-6).then(|| [z.x + (x / w * 0.5 + 0.5) * z.w, z.y + (0.5 - y / w * 0.5) * z.h])
}

/// La distance d'un point à un segment, à l'écran.
fn distance_segment(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let l = dx * dx + dy * dy;
    let t = if l > 0.0 { (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / l).clamp(0.0, 1.0) } else { 0.0 };
    ((p[0] - a[0] - t * dx).powi(2) + (p[1] - a[1] - t * dy).powi(2)).sqrt()
}

fn teinte(c: [f32; 3], a: u8) -> Color {
    [(c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8, a]
}

impl Mailles {
    fn retouche(&mut self) {
        self.revision += 1;
    }

    fn instantane(&self) -> Instantane {
        Instantane { objets: self.objets.clone(), actif: self.actif, sel: self.sel.clone(), mode: self.mode }
    }

    fn restaurer(&mut self, i: Instantane) {
        self.objets = i.objets;
        self.actif = i.actif.min(self.objets.len().saturating_sub(1));
        self.sel = i.sel;
        self.mode = i.mode;
        self.retouche();
    }

    /// Retient l'état d'avant une modification, pour l'annuler.
    fn noter(&mut self, avant: Instantane) {
        if avant.objets == self.objets {
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
        let ici = Instantane { objets: self.objets.clone(), actif: self.actif, sel: self.sel.clone(), mode: self.mode };
        vers.push(ici);
        self.restaurer(i);
        self.trait_ = None;
        self.modifie = true;
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
        match glb::lire_objets(&o) {
            Ok(s) if !s.objets.is_empty() => {
                self.objets = s
                    .objets
                    .into_iter()
                    .map(|ob| {
                        let monde: Vec<V3> = ob.positions.iter().map(|p| glb::appliquer(&ob.matrice, *p)).collect();
                        Objet::depuis_triangles(&ob.nom, ob.matrice, monde, &ob.triangles, ob.couleur.unwrap_or([0.72, 0.74, 0.78]))
                    })
                    .collect();
                if !brouillon {
                    self.perdu = s.perdu;
                }
                self.actif = 0;
                self.sel = Selection::default();
                self.modifie = brouillon;
                self.retouche();
                if brouillon {
                    self.message = Some(("Brouillon non enregistré retrouvé".into(), false));
                } else if !self.perdu.is_empty() {
                    self.message = Some((format!("Ce fichier porte des {} : Mailles les laisse de côté, l'enregistrement ira dans une copie.", self.perdu.join(", ")), false));
                }
            }
            Ok(_) => self.message = Some(("Ce glb ne contient aucun objet.".into(), true)),
            Err(e) => self.message = Some((e, true)),
        }
    }

    /// Le fichier où enregistrer.
    fn destination(&self, c: &Commun) -> Result<PathBuf, String> {
        if let Some(p) = &self.chemin {
            if self.perdu.is_empty() {
                return Ok(p.clone());
            }
            // Une copie, pour ne pas perdre ce que Mailles ne réécrit pas
            let stem = p.file_stem().map_or("maille".into(), |s| s.to_string_lossy().into_owned());
            let dossier = p.parent().map(Path::to_path_buf).unwrap_or_default();
            return (0..).map(|n| dossier.join(if n == 0 { format!("{stem}-mailles.glb") } else { format!("{stem}-mailles-{n}.glb") })).find(|q| !q.exists()).ok_or_else(|| "Aucun nom libre.".into());
        }
        let dossier = Mailles::dossier_neuf(c).ok_or("Aucun dossier où enregistrer.")?;
        let saisi = self.nom.texte.trim().trim_end_matches(".glb").to_string();
        if saisi.contains('/') || saisi.starts_with('.') {
            return Err("Nom de fichier invalide.".into());
        }
        if saisi.is_empty() {
            return (1..).map(|n| dossier.join(format!("maille-{n}.glb"))).find(|p| !p.exists()).ok_or_else(|| "Aucun nom libre.".into());
        }
        let p = dossier.join(format!("{saisi}.glb"));
        if p.exists() {
            return Err(format!("{saisi}.glb existe déjà : choisissez un autre nom."));
        }
        Ok(p)
    }

    /// Où vont les nouvelles mailles : `Documents/Mailles`, ou à défaut le dossier de
    /// Flow.
    fn dossier_neuf(c: &Commun) -> Option<PathBuf> {
        c.racine.as_ref().map(|r| r.join("Documents").join("Mailles")).or_else(|| c.dossier.clone())
    }

    /// La disquette : enregistre, ou pour une maille qui n'a jamais été enregistrée,
    /// ouvre la boîte où saisir son nom (proposé : le premier `maille-N` libre).
    fn demander_enregistrement(&mut self, k: usize, c: &mut Commun) {
        if self.chemin.is_some() {
            self.message = Some(match self.enregistrer(c) {
                Ok(()) => (format!("Enregistré : {}", self.chemin.as_deref().map_or(String::new(), |p| p.display().to_string())), false),
                Err(e) => (e, true),
            });
            return;
        }
        if self.nom.texte.trim().is_empty() {
            let libre = Mailles::dossier_neuf(c).and_then(|d| (1..10_000).map(|n| format!("maille-{n}")).find(|n| !d.join(format!("{n}.glb")).exists()));
            self.nom = Edition::new(&libre.unwrap_or_default());
        }
        self.erreur = None;
        self.boite = true;
        c.clavier.ouvrir(sub_id(NOM, k));
    }

    fn fermer_boite(&mut self, k: usize, c: &mut Commun) {
        self.boite = false;
        self.erreur = None;
        if c.clavier.cible == Some(sub_id(NOM, k)) {
            c.clavier.fermer();
        }
    }

    /// Enregistre sous le nom saisi ; la boîte se ferme si c'est fait.
    fn conclure_boite(&mut self, k: usize, c: &mut Commun) {
        if self.nom.texte.trim().is_empty() {
            self.erreur = Some("Donnez un nom à la maille.".into());
            return;
        }
        match self.enregistrer(c) {
            Ok(()) => {
                self.fermer_boite(k, c);
                self.message = Some((format!("Enregistré : {}", self.chemin.as_deref().map_or(String::new(), |p| p.display().to_string())), false));
            }
            Err(e) => self.erreur = Some(e),
        }
    }

    /// La boîte du premier enregistrement, par-dessus les vues : le nom, le dossier,
    /// Annuler et Enregistrer.
    fn dessiner_boite(&mut self, cx: &mut Cx, c: &mut Commun, p: &Placee) {
        let (d, col) = (cx.dp, cx.couleurs());
        let k = p.id as usize;
        let r = p.contenu;
        let base = sub_id(BOITE, k);
        cx.dl.rrect(r, 0.0, [0, 0, 0, 110]);
        cx.ui.region_add(sub_id(base, 0), r, Sense::Sink);
        let (hc, gap, pad) = (cx.commande(), cx.ecart(), 18.0 * d);
        let w = (r.w - 2.0 * 12.0 * d).min(440.0 * d).max(0.0);
        let lh = 20.0 * d * cx.theme.texte;
        // Le dossier, à partir de la racine du stockage quand il y est
        let dossier = Mailles::dossier_neuf(c).map_or(String::new(), |p| {
            let court = c.racine.as_ref().and_then(|r| p.strip_prefix(r).ok()).unwrap_or(&p);
            format!("Dans {}", court.display())
        });
        let h = pad + 26.0 * d * cx.theme.texte + hc + 2.0 * gap + lh + if self.erreur.is_some() { lh } else { 0.0 } + gap + hc + pad;
        let carte = Rect::new(r.x + (r.w - w) / 2.0, r.y + (r.h * 0.12).min(60.0 * d), w, h);
        let rayon = cx.theme.rayon(hc).min(18.0 * d);
        cx.ombre(carte, rayon, 14.0);
        cx.dl.rrect(carte, rayon, col.surface);
        cx.ui.region_add(sub_id(base, 1), carte, Sense::Sink);
        let mut y = carte.y + pad;
        cx.clip(Some(carte));
        cx.text_at(carte.x + pad, y + 11.0 * d * cx.theme.texte, "Enregistrer la maille", 18.0, col.texte);
        y += 26.0 * d * cx.theme.texte;
        let marge = (cx.theme.marge * 0.4 * d).round();
        let zone = Rect::new(carte.x + pad - 14.0 * d, y + gap - 8.0 * d - marge, carte.w - 2.0 * pad + 28.0 * d, hc + 8.0 * d + marge + 4.0 * d);
        let mut b = cx.body(sub_id(NOM_CORPS, k), zone, &c.scrolls);
        clavier::champ(cx, &mut b, sub_id(NOM, k), "Nom", &mut self.nom, &mut c.clavier);
        cx.body_end(b, &mut c.scrolls);
        cx.clip(Some(carte));
        y += hc + 2.0 * gap;
        cx.text_at(carte.x + pad, y + lh / 2.0, &format!("{dossier}, en .glb"), 14.0, col.pale);
        y += lh;
        if let Some(e) = &self.erreur {
            cx.text_at(carte.x + pad, y + lh / 2.0, e, 14.0, [0xE0, 0x3A, 0x3A, 255]);
            y += lh;
        }
        y += gap;
        let mut xb = carte.right() - pad;
        let mut choisi = None;
        for (i, nom) in ["Annuler", "Enregistrer"].into_iter().enumerate().rev() {
            xb -= cx.pill_width(nom);
            if cx.pill(sub_id(base, 10 + i), xb, y, nom, i == 1) {
                choisi = Some(i);
            }
            xb -= gap;
        }
        cx.clip(None);
        match choisi {
            Some(0) => self.fermer_boite(k, c),
            Some(_) => self.conclure_boite(k, c),
            None if std::mem::take(&mut self.valider) => self.conclure_boite(k, c),
            None => {}
        }
    }

    /// Les objets en glTF binaire : chacun dans le repère de son nœud, en triangles,
    /// avec des normales lissées.
    fn octets(&self) -> Vec<u8> {
        let prepares: Vec<(Vec<V3>, Vec<V3>, Vec<[u32; 3]>)> = self
            .objets
            .iter()
            .map(|o| {
                let inv = glb::inverse(&o.matrice);
                let local: Vec<V3> = o.sommets.iter().map(|p| glb::appliquer(&inv, *p)).collect();
                let tris: Vec<[u32; 3]> = o.triangles().into_iter().map(|t| t.0).collect();
                let mut normales = vec![[0.0f32; 3]; local.len()];
                for t in &tris {
                    let [a, b, c] = t.map(|i| local[i as usize]);
                    let n = sc::vectoriel(sc::oter(b, a), sc::oter(c, a));
                    for &i in t {
                        normales[i as usize] = sc::ajouter(normales[i as usize], n);
                    }
                }
                let normales = normales.into_iter().map(|n| if sc::norme(n) > 1e-12 { sc::unitaire(n) } else { [0.0, 1.0, 0.0] }).collect();
                (local, normales, tris)
            })
            .collect();
        let ecrits: Vec<glb::ObjetEcrit> = self
            .objets
            .iter()
            .zip(&prepares)
            .map(|(o, (p, n, t))| glb::ObjetEcrit { nom: &o.nom, matrice: o.matrice, positions: p, normales: n, triangles: t, couleur: o.couleur })
            .collect();
        glb::ecrire_objets(&ecrits, "Flow · Mailles")
    }

    fn enregistrer(&mut self, c: &Commun) -> Result<(), String> {
        let chemin = self.destination(c)?;
        ecrire_octets(&chemin, &self.octets())?;
        self.chemin = Some(chemin);
        self.perdu.clear();
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
            None => "Nouvelle maille".into(),
        }
    }

    /// La boîte de tous les objets visibles.
    fn boite(&self) -> (V3, V3) {
        self.objets.iter().filter(|o| o.visible).filter_map(Objet::boite).reduce(|(a, b), (c, d)| ([a[0].min(c[0]), a[1].min(c[1]), a[2].min(c[2])], [b[0].max(d[0]), b[1].max(d[1]), b[2].max(d[2])])).unwrap_or(([-1.0; 3], [1.0; 3]))
    }

    fn camera(&self, f: FenetreId) -> Camera {
        self.cameras.get(&f).copied().unwrap_or_else(|| {
            let mut c = Camera::default();
            self.cadrer_camera(&mut c, f);
            c
        })
    }

    /// Cadre tous les objets visibles ; une vue plus haute que large recule d'autant.
    fn cadrer_camera(&self, c: &mut Camera, f: FenetreId) {
        c.cadrer(self.boite());
        if let Some(z) = self.zones.get(&f).filter(|z| z.w > 1.0 && z.h > z.w) {
            c.distance *= (z.h / z.w).min(3.0);
        }
    }

    /// Ce qui part vers la carte graphique : les faces des objets visibles, à plat
    /// (trois sommets par triangle, avec la normale de sa face), et le surlignage des
    /// faces choisies.
    fn maillages3d(&mut self) -> (Maillage3d, Option<Maillage3d>) {
        if self.gpu.as_ref().is_none_or(|g| g.0 != self.revision) {
            let mut sommets = Vec::new();
            let mut origine = Vec::new();
            for (k, o) in self.objets.iter().enumerate().filter(|(_, o)| o.visible) {
                let normales: Vec<V3> = (0..o.faces.len()).map(|f| o.normale(f)).collect();
                for (t, f) in o.triangles() {
                    for i in t {
                        sommets.extend_from_slice(&o.sommets[i as usize]);
                        sommets.extend_from_slice(&normales[f]);
                    }
                    origine.push((k, f));
                }
            }
            let indices: Vec<u32> = (0..origine.len() as u32 * 3).collect();
            self.gpu = Some((self.revision, Arc::new(sommets), Arc::new(indices), Arc::new(origine)));
            self.surligne = None;
        }
        let (rev, sommets, indices, origine) = self.gpu.clone().unwrap();
        let principal = Maillage3d { id: self.id, revision: rev, sommets: sommets.clone(), indices };
        // Les faces choisies (ou couvertes par la sélection) de l'objet actif
        let choisies = self.objets.get(self.actif).map(|o| self.sel.faces_de(o, self.mode)).unwrap_or_default();
        if choisies.is_empty() {
            return (principal, None);
        }
        let cle = {
            use std::hash::{Hash, Hasher};
            let mut h = std::hash::DefaultHasher::new();
            (rev, self.actif, &choisies).hash(&mut h);
            h.finish()
        };
        if self.surligne.as_ref().is_none_or(|s| s.0 != cle) {
            let ind: Vec<u32> = origine.iter().enumerate().filter(|(_, (k, f))| *k == self.actif && choisies.contains(f)).flat_map(|(t, _)| [t as u32 * 3, t as u32 * 3 + 1, t as u32 * 3 + 2]).collect();
            self.surligne = Some((cle, Arc::new(ind)));
        }
        let s = self.surligne.as_ref().unwrap();
        (principal, Some(Maillage3d { id: self.id + 1, revision: s.0, sommets, indices: s.1.clone() }))
    }

    /// Le contexte des opérations : mode, sélection, objet actif.
    fn contexte_ops(&self) -> m::Contexte {
        m::Contexte::de(&self.objets, self.actif, &self.sel, self.mode)
    }

    /// Change de mode, en gardant ce qui était choisi.
    fn changer_mode(&mut self, mode: Mode) {
        if mode == self.mode {
            return;
        }
        if let Some(o) = self.objets.get(self.actif) {
            self.sel = self.sel.convertir(o, self.mode, mode);
        }
        self.mode = mode;
    }

    /// Fait une opération du tableau.
    fn executer(&mut self, cle: &str) {
        let Some(op) = m::operation(cle) else { return };
        if !(op.possible)(&self.contexte_ops()) {
            return;
        }
        let cam = self.camera(self.derniere);
        let transfo = match cle {
            "deplacer" => Some(Transfo::Deplacer),
            "tourner" => Some(Transfo::Tourner),
            "echelle" => Some(Transfo::Echelle),
            _ => None,
        };
        if let Some(t) = transfo {
            self.transfo = if self.transfo.is_some_and(|x| x.0 == t) { None } else { Some((t, None)) };
            return;
        }
        let avant = self.instantane();
        let mode = self.mode;
        match cle {
            "tout" | "inverser" | "rien" => {
                let Some(o) = self.objets.get(self.actif) else { return };
                let tout = match mode {
                    Mode::Sommets => Selection { sommets: (0..o.sommets.len() as u32).collect(), ..Selection::default() },
                    Mode::Aretes => Selection { aretes: o.aretes().into_iter().collect(), ..Selection::default() },
                    Mode::Faces => Selection { faces: (0..o.faces.len()).collect(), ..Selection::default() },
                };
                self.sel = match cle {
                    "tout" => tout,
                    "rien" => Selection::default(),
                    _ => Selection {
                        sommets: tout.sommets.difference(&self.sel.sommets).copied().collect(),
                        aretes: tout.aretes.difference(&self.sel.aretes).copied().collect(),
                        faces: tout.faces.difference(&self.sel.faces).copied().collect(),
                    },
                };
                if self.sel.vide() {
                    self.transfo = None;
                }
                return;
            }
            "cube" => {
                let demi = (cam.distance * 0.12).max(0.01);
                let n = self.objets.len() + 1;
                self.objets.push(m::cube(&format!("Cube {n}"), cam.cible, demi));
                self.actif = self.objets.len() - 1;
                self.sel = Selection::default();
            }
            _ => {
                let Some(o) = self.objets.get_mut(self.actif) else { return };
                let faces = self.sel.faces_de(o, mode);
                let sommets = self.sel.sommets_de(o, mode);
                let suite = match cle {
                    "extruder" => m::extruder(o, &faces),
                    "subdiviser" => m::subdiviser(o, &faces),
                    "fusionner" => m::fusionner(o, &sommets),
                    "face" => m::creer_face(o, &sommets, cam.oeil()),
                    "trianguler" => m::trianguler(o, &faces),
                    "retourner" => m::retourner(o, &faces),
                    "dupliquer" => m::dupliquer(o, &faces),
                    "supprimer" => m::supprimer(o, &self.sel, mode),
                    _ => return,
                };
                // La sélection qui suit, dans le mode où l'on était
                self.sel = suite.selection.convertir(o, suite.mode, mode);
                if self.sel.vide() && !suite.selection.vide() {
                    self.mode = suite.mode;
                    self.sel = suite.selection;
                }
                self.transfo = suite.transfo;
            }
        }
        self.retouche();
        self.noter(avant);
    }

    /// L'élément de l'objet actif touché en (x, y) dans la vue `f`, ou un autre objet.
    fn choisir(&mut self, f: FenetreId, x: f32, y: f32) {
        let Some(&z) = self.zones.get(&f) else { return };
        let cam = self.camera(f);
        let mvp = cam.mvp(z.w / z.h.max(1.0));
        let nx = (x - z.x) / z.w.max(1.0) * 2.0 - 1.0;
        let ny = 1.0 - (y - z.y) / z.h.max(1.0) * 2.0;
        let (orig, d) = cam.rayon(z.w / z.h.max(1.0), nx, ny);
        // L'objet touché le plus proche
        let touche = self.objets.iter().enumerate().filter(|(_, o)| o.visible).filter_map(|(k, o)| m::toucher(o, orig, d).map(|(t, fa)| (t, k, fa))).min_by(|a, b| a.0.total_cmp(&b.0));
        if let Some((_, k, _)) = touche {
            if k != self.actif {
                self.actif = k;
                self.sel = Selection::default();
                self.transfo = None;
                return;
            }
        }
        let Some(o) = self.objets.get(self.actif) else { return };
        let oeil = cam.oeil();
        let devant: Vec<bool> = (0..o.faces.len()).map(|fa| sc::scalaire(o.normale(fa), sc::oter(oeil, o.centre_face(fa))) > 0.0).collect();
        let rayon = 28.0 * self.dp;
        let element = match self.mode {
            Mode::Faces => touche.filter(|t| t.1 == self.actif).map(|t| Element::Face(t.2)),
            Mode::Sommets => {
                let mut vus = vec![false; o.sommets.len()];
                for (fa, p) in o.faces.iter().enumerate() {
                    if devant[fa] {
                        for &i in p {
                            vus[i as usize] = true;
                        }
                    }
                }
                o.sommets
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| vus[*i])
                    .filter_map(|(i, p)| projeter(&mvp, z, *p).map(|q| (((q[0] - x).powi(2) + (q[1] - y).powi(2)).sqrt(), i)))
                    .filter(|(dd, _)| *dd < rayon)
                    .min_by(|a, b| a.0.total_cmp(&b.0))
                    .map(|(_, i)| Element::Sommet(i as u32))
            }
            Mode::Aretes => {
                let par = o.faces_par_arete();
                par.iter()
                    .filter(|(_, fs)| fs.iter().any(|&fa| devant[fa]))
                    .filter_map(|(a, _)| {
                        let (p, q) = (projeter(&mvp, z, o.sommets[a.0 as usize])?, projeter(&mvp, z, o.sommets[a.1 as usize])?);
                        Some((distance_segment([x, y], p, q), *a))
                    })
                    .filter(|(dd, _)| *dd < rayon * 0.8)
                    .min_by(|a, b| a.0.total_cmp(&b.0))
                    .map(|(_, a)| Element::Arete(a))
            }
        };
        match element {
            Some(e) => self.sel.basculer(e),
            None => self.sel = Selection::default(),
        }
        if self.sel.vide() {
            self.transfo = None;
        }
    }

    /// Les gestes d'une vue 3D.
    fn recevoir(&mut self, f: FenetreId, g: Geste) {
        self.derniere = f;
        let Some(&z) = self.zones.get(&f) else { return };
        let mut cam = self.camera(f);
        match g {
            Geste::Tap { x, y, .. } => self.choisir(f, x, y),
            Geste::SaisieDebut { doigt, .. } => {
                let t = match (self.transfo, self.objets.get(self.actif)) {
                    (Some(_), Some(o)) if !self.sel.vide() => {
                        let s = self.sel.sommets_de(o, self.mode);
                        let base: Vec<(u32, V3)> = s.iter().map(|&i| (i, o.sommets[i as usize])).collect();
                        Trait::Transfo { depart: doigt, centre: m::centre_de(o, &s), base, avant: Box::new(self.instantane()) }
                    }
                    _ => Trait::Orbite { doigt },
                };
                self.trait_ = Some((f, t));
            }
            Geste::SaisieDeplace { doigt, .. } => match self.trait_.take() {
                Some((vf, Trait::Orbite { doigt: avant })) if vf == f => {
                    let k = 0.006 / self.dp.max(0.1);
                    cam.lacet -= (doigt.0 - avant.0) * k;
                    cam.tangage = (cam.tangage + (doigt.1 - avant.1) * k).clamp(-1.45, 1.45);
                    self.cameras.insert(f, cam);
                    self.trait_ = Some((f, Trait::Orbite { doigt }));
                }
                Some((vf, Trait::Transfo { depart, base, centre, avant })) if vf == f => {
                    let (dx, dy) = (doigt.0 - depart.0, doigt.1 - depart.1);
                    let (fwd, r, u) = cam.axes();
                    let (t, axe) = self.transfo.unwrap_or((Transfo::Deplacer, None));
                    if let Some(o) = self.objets.get_mut(self.actif) {
                        for &(i, p) in &base {
                            o.sommets[i as usize] = match t {
                                Transfo::Deplacer => {
                                    let k = cam.taille_a(centre, 1.0, z.h);
                                    let libre = sc::ajouter(sc::fois(r, dx * k), sc::fois(u, -dy * k));
                                    let d = match axe {
                                        // Le long de l'axe : la part du glissement qui suit son image à l'écran
                                        Some(a) => {
                                            let mvp = cam.mvp(z.w / z.h.max(1.0));
                                            match (projeter(&mvp, z, centre), projeter(&mvp, z, sc::ajouter(centre, a))) {
                                                (Some(c0), Some(c1)) => {
                                                    let (sx, sy) = (c1[0] - c0[0], c1[1] - c0[1]);
                                                    let l2 = sx * sx + sy * sy;
                                                    if l2 < (24.0 * self.dp).powi(2) {
                                                        // L'axe vu de face : le glissement vertical le suit
                                                        sc::fois(a, -dy * k)
                                                    } else {
                                                        sc::fois(a, (dx * sx + dy * sy) / l2)
                                                    }
                                                }
                                                _ => sc::fois(a, sc::scalaire(libre, a)),
                                            }
                                        }
                                        None => libre,
                                    };
                                    sc::ajouter(p, d)
                                }
                                Transfo::Tourner => m::tourner(p, centre, sc::fois(fwd, -1.0), dx * 0.01),
                                Transfo::Echelle => sc::ajouter(centre, sc::fois(sc::oter(p, centre), (dx * 0.005 / self.dp.max(0.1)).exp())),
                            };
                        }
                    }
                    self.retouche();
                    self.trait_ = Some((f, Trait::Transfo { depart, base, centre, avant }));
                }
                autre => self.trait_ = autre,
            },
            Geste::SaisieFin { .. } => {
                if let Some((_, Trait::Transfo { avant, .. })) = self.trait_.take() {
                    self.noter(*avant);
                }
            }
            Geste::SaisieAnnulee { .. } => {
                if let Some((_, Trait::Transfo { avant, .. })) = self.trait_.take() {
                    self.restaurer(*avant);
                }
            }
            Geste::PincementDebut { .. } => self.pincement = 1.0,
            Geste::Pincement { echelle, deplacement, .. } => {
                let k = echelle / self.pincement.max(1e-3);
                self.pincement = echelle;
                let (_, r, u) = cam.axes();
                let px = cam.taille_a(cam.cible, 1.0, z.h);
                cam.cible = sc::oter(cam.cible, sc::ajouter(sc::fois(r, deplacement.0 * px), sc::fois(u, -deplacement.1 * px)));
                cam.distance = (cam.distance / k.max(1e-3)).clamp(0.01, 5000.0);
                self.cameras.insert(f, cam);
            }
            _ => {}
        }
    }

    /// Une vue 3D : la forme, ses arêtes et sommets, le mode en haut, la barre rapide
    /// en bas.
    fn vue_3d(&mut self, cx: &mut Cx, p: &Placee, base: Id) {
        let (d, col) = (cx.dp, cx.couleurs());
        let r = p.contenu;
        let f = p.id;
        let (hc, gap) = (cx.commande(), cx.ecart());
        let kv = sub_id(base, f as usize * 1000);
        // En haut : le mode, et « Cadrer »
        let mut haut: Vec<(String, bool)> = Mode::TOUS.iter().map(|md| (md.nom().to_string(), *md == self.mode)).collect();
        haut.push(("Cadrer".into(), false));
        let place = (r.w - 2.0 * cx.coin()).max(0.0);
        let lh: Vec<f32> = haut.iter().map(|(n, _)| cx.pill_width(n)).collect();
        let rh = super::fichiers::ranger(&lh, place, gap * 0.6);
        let hh = (rh.iter().map(|x| x.1).max().unwrap_or(0) + 1) as f32 * (hc + gap * 0.6);
        // En bas : la barre rapide, ce que la sélection permet
        let ctx = self.contexte_ops();
        let rapides: Vec<&m::Operation> = m::OPERATIONS.iter().filter(|o| (o.possible)(&ctx) && o.cle != "cube").collect();
        let lb: Vec<f32> = rapides.iter().map(|o| cx.pill_width(o.nom)).collect();
        let rb = super::fichiers::ranger(&lb, place, gap * 0.6);
        let hbar = if rapides.is_empty() { 0.0 } else { (rb.iter().map(|x| x.1).max().unwrap_or(0) + 1) as f32 * (hc + gap * 0.6) + gap };
        let z = r;
        self.zones.insert(f, z);
        if !self.cameras.contains_key(&f) {
            let c = self.camera(f);
            self.cameras.insert(f, c);
        }
        for g in cx.ui.gestes(sub_id(kv, 30_000), z) {
            self.recevoir(f, g);
        }
        let cam = self.camera(f);
        cx.dl.rrect(z, 0.0, col.fond);
        if z.w < 2.0 || z.h < 2.0 {
            return;
        }
        let (principal, dessus) = self.maillages3d();
        let mvp = cam.mvp(z.w / z.h);
        let couleur = self.objets.get(self.actif).map_or([0.72, 0.74, 0.78], |o| o.couleur);
        let a = col.accent;
        let accent3 = [a[0] as f32 / 255.0, a[1] as f32 / 255.0, a[2] as f32 / 255.0];
        cx.clip(Some(z));
        cx.dl.vue3d(Vue3d { rect: z, mvp, vue: cam.vue(), maillage: principal, couleur, dessus: dessus.map(|m3| (m3, accent3)) });
        // Les arêtes et les sommets de l'objet actif, du côté de l'œil
        if let Some(o) = self.objets.get(self.actif).filter(|o| o.visible) {
            let oeil = cam.oeil();
            let devant: Vec<bool> = (0..o.faces.len()).map(|fa| sc::scalaire(o.normale(fa), sc::oter(oeil, o.centre_face(fa))) > 0.0).collect();
            let ecran: Vec<Option<[f32; 2]>> = o.sommets.iter().map(|q| projeter(&mvp, z, *q)).collect();
            let choisies = self.sel.aretes_de(o, self.mode);
            let par = o.faces_par_arete();
            let toutes = par.len() <= ARETES_MAX;
            let fil = [col.texte[0], col.texte[1], col.texte[2], 120];
            for (ar, fs) in &par {
                let choisie = choisies.contains(ar);
                if !(choisie || toutes && fs.iter().any(|&fa| devant[fa])) {
                    continue;
                }
                if let (Some(pa), Some(pb)) = (ecran[ar.0 as usize], ecran[ar.1 as usize]) {
                    if choisie {
                        cx.dl.line(pa, pb, 3.0 * d, a);
                    } else {
                        cx.dl.line(pa, pb, 1.2 * d, fil);
                    }
                }
            }
            if self.mode == Mode::Sommets {
                let mut vus = vec![false; o.sommets.len()];
                for (fa, poly) in o.faces.iter().enumerate() {
                    if devant[fa] {
                        for &i in poly {
                            vus[i as usize] = true;
                        }
                    }
                }
                for (i, q) in ecran.iter().enumerate() {
                    let choisi = self.sel.sommets.contains(&(i as u32));
                    if let (Some(q), true) = (q, vus[i] || choisi) {
                        let s = if choisi { 4.5 * d } else { 2.6 * d };
                        cx.dl.rrect(Rect::new(q[0] - s, q[1] - s, 2.0 * s, 2.0 * s), s, if choisi { a } else { col.texte });
                    }
                }
            }
            if self.mode == Mode::Faces {
                for fa in 0..o.faces.len() {
                    if devant[fa] {
                        if let Some(q) = projeter(&mvp, z, o.centre_face(fa)) {
                            let s = 2.0 * d;
                            cx.dl.rrect(Rect::new(q[0] - s, q[1] - s, 2.0 * s, 2.0 * s), s, if self.sel.faces.contains(&fa) { col.sur_accent } else { fil });
                        }
                    }
                }
            }
        }
        cx.clip(None);
        // Le mode et « Cadrer », sur la vue
        let (x0, y0) = (r.x + cx.coin(), r.y + cx.coin());
        let mut choix = None;
        for (i, (nom, actif)) in haut.iter().enumerate() {
            let (x, l) = rh[i];
            if cx.pill(sub_id(kv, 100 + i), x0 + x, y0 + l as f32 * (hc + gap * 0.6), nom, *actif) {
                choix = Some(i);
            }
        }
        match choix {
            Some(i) if i < 3 => self.changer_mode(Mode::TOUS[i]),
            Some(_) => {
                let mut c = cam;
                self.cadrer_camera(&mut c, f);
                self.cameras.insert(f, c);
            }
            None => {}
        }
        let _ = hh;
        // La barre rapide
        if !rapides.is_empty() {
            let yb = r.bottom() - hbar;
            let mut faite = None;
            for (i, op) in rapides.iter().enumerate() {
                let (x, l) = rb[i];
                let allume = self.transfo.is_some_and(|t| matches!((t.0, op.cle), (Transfo::Deplacer, "deplacer") | (Transfo::Tourner, "tourner") | (Transfo::Echelle, "echelle")));
                if cx.pill(sub_id(kv, 200 + i), x0 + x, yb + l as f32 * (hc + gap * 0.6), op.nom, allume) {
                    faite = Some(op.cle);
                }
            }
            if let Some(k) = faite {
                self.derniere = f;
                self.executer(k);
            }
        }
    }

    /// L'arborescence : les objets, leur taille, leur visibilité.
    fn vue_arbre(&mut self, cx: &mut Cx, c: &mut Commun, p: &Placee, base: Id) {
        let (d, col) = (cx.dp, cx.couleurs());
        let r = p.contenu;
        let kv = sub_id(base, p.id as usize * 1000);
        cx.dl.rrect(r, 0.0, col.surface);
        let (hc, gap) = (cx.commande(), cx.ecart());
        let bas = hc + 2.0 * gap;
        let liste = Rect::new(r.x, r.y, r.w, (r.h - bas).max(0.0));
        let corps = sub_id(kv, 1);
        let mut b = cx.body(corps, liste, &c.scrolls);
        let h = (cx.theme.ligne * d * 1.2).round();
        let mut actif = None;
        let mut voir = None;
        for (k, o) in self.objets.iter().enumerate() {
            let ligne = Rect::new(r.x, b.y, r.w, h);
            let oeil = Rect::new(ligne.right() - h, ligne.y, h, h);
            let lid = sub_id(kv, 100 + k);
            let vid = sub_id(kv, 10_000 + k);
            if cx.ui.click(vid, oeil, Some(corps)) {
                voir = Some(k);
            }
            if cx.ui.click(lid, Rect::new(ligne.x, ligne.y, ligne.w - h, h), Some(corps)) {
                actif = Some(k);
            }
            let est = k == self.actif;
            if est || cx.ui.pressed(lid) {
                cx.dl.rrect(ligne.inset(2.0 * d), cx.theme.rayon(h) * 0.4, col.puce);
            }
            let s = (22.0 * d).round();
            cx.dl.rrect(Rect::new(b.x, ligne.cy() - s / 2.0, s, s), 5.0 * d, teinte(o.couleur, if o.visible { 255 } else { 90 }));
            let x = b.x + s + 12.0 * d;
            cx.clip(Some(Rect::new(x, ligne.y, (oeil.x - x).max(0.0), h).intersect(&liste)));
            cx.text_at(x, ligne.y + h * 0.36, &o.nom, 15.0, if est { col.accent } else { col.texte });
            cx.text_at(x, ligne.y + h * 0.72, &format!("{} sommets · {} faces", o.sommets.len(), o.faces.len()), 12.0, col.pale);
            cx.clip(Some(liste));
            // L'œil : plein si l'objet se voit
            let e = oeil.inset(h * 0.3);
            let teinte_oeil = if o.visible { col.texte } else { col.ligne };
            cx.dl.secteur([e.cx(), e.cy()], e.w * 0.42, e.w * 0.5, 0.0, std::f32::consts::TAU, 0.0, teinte_oeil);
            if o.visible {
                cx.dl.rrect(Rect::new(e.cx() - e.w * 0.18, e.cy() - e.w * 0.18, e.w * 0.36, e.w * 0.36), e.w * 0.18, teinte_oeil);
            }
            b.y += h;
        }
        cx.body_end(b, &mut c.scrolls);
        if let Some(k) = actif {
            if k != self.actif {
                self.actif = k;
                self.sel = Selection::default();
                self.transfo = None;
            }
        }
        if let Some(k) = voir {
            let avant = self.instantane();
            self.objets[k].visible = !self.objets[k].visible;
            self.retouche();
            self.noter(avant);
        }
        // En bas : ajouter un cube, supprimer l'objet actif
        cx.dl.rrect(Rect::new(r.x, liste.bottom(), r.w, d.max(1.0)), 0.0, col.ligne);
        let y = liste.bottom() + gap;
        let mut x = r.x + cx.coin();
        if cx.pill(sub_id(kv, 2), x, y, "Ajouter un cube", false) {
            self.executer("cube");
        }
        x += cx.pill_width("Ajouter un cube") + gap;
        if self.objets.len() > 1 && cx.pill(sub_id(kv, 3), x, y, "Supprimer l'objet", false) {
            let avant = self.instantane();
            self.objets.remove(self.actif);
            self.actif = self.actif.min(self.objets.len() - 1);
            self.sel = Selection::default();
            self.transfo = None;
            self.retouche();
            self.noter(avant);
        }
    }

    /// Le tableau des opérations : par groupe, ce qu'elles font et si la sélection les
    /// permet ; toucher une opération possible la fait.
    fn vue_operations(&mut self, cx: &mut Cx, c: &mut Commun, p: &Placee, base: Id) {
        let (d, col) = (cx.dp, cx.couleurs());
        let r = p.contenu;
        let kv = sub_id(base, p.id as usize * 1000);
        cx.dl.rrect(r, 0.0, col.surface);
        let ctx = self.contexte_ops();
        let corps = sub_id(kv, 1);
        let mut b = cx.body(corps, r, &c.scrolls);
        let h = (cx.theme.ligne * d * 1.15).round();
        let mut faite = None;
        let mut groupe = "";
        for (i, op) in m::OPERATIONS.iter().enumerate() {
            if op.groupe != groupe {
                groupe = op.groupe;
                cx.section(&mut b, &groupe.to_uppercase());
            }
            let possible = (op.possible)(&ctx);
            let ligne = Rect::new(r.x, b.y, r.w, h);
            let lid = sub_id(kv, 100 + i);
            if cx.ui.click(lid, ligne, Some(corps)) && possible {
                faite = Some(op.cle);
            }
            let allume = self.transfo.is_some_and(|t| matches!((t.0, op.cle), (Transfo::Deplacer, "deplacer") | (Transfo::Tourner, "tourner") | (Transfo::Echelle, "echelle")));
            if allume || possible && cx.ui.pressed(lid) {
                cx.dl.rrect(ligne.inset(2.0 * d), cx.theme.rayon(h) * 0.4, col.puce);
            }
            cx.clip(Some(ligne.intersect(&r)));
            cx.text_at(b.x, ligne.y + h * 0.36, op.nom, 15.0, if allume { col.accent } else if possible { col.texte } else { col.ligne });
            cx.text_at(b.x, ligne.y + h * 0.72, op.aide, 12.0, if possible { col.pale } else { col.ligne });
            cx.clip(Some(r));
            b.y += h;
        }
        cx.body_end(b, &mut c.scrolls);
        if let Some(k) = faite {
            self.executer(k);
        }
    }

    /// La barre de la fenêtre, à droite du titre : l'état (mode, objet, sélection, ou
    /// le dernier message), puis le nom du fichier (une astérisque s'il est modifié), la
    /// disquette qui enregistre (à l'accent tant qu'il y a à enregistrer), annuler et
    /// rétablir, juste avant la poignée.
    fn barre(&mut self, cx: &mut Cx, c: &mut Commun, p: &Placee) {
        let (d, col) = (cx.dp, cx.couleurs());
        let e = p.etat;
        let k = p.id as usize;
        let h = e.h;
        let mut x = e.right();
        // Annuler, rétablir
        for (j, (icone, retablir)) in [(Icon::Retablir, true), (Icon::Annuler, false)].into_iter().enumerate() {
            x -= h;
            let case = Rect::new(x, e.y, h, h);
            let possible = !if retablir { &self.futur } else { &self.passe }.is_empty();
            let kb = sub_id(MAILLES, 10_000 + k * 2 + j);
            if cx.ui.click(kb, case, None) && possible {
                self.defaire(retablir);
            }
            if possible && cx.ui.pressed(kb) {
                cx.dl.rrect(case.inset(3.0 * d), cx.theme.rayon(case.h - 6.0 * d), col.puce);
            }
            cx.icon(case.inset(h * 0.24), icone, if possible { col.texte } else { col.ligne });
        }
        // La disquette
        x -= h;
        let case = Rect::new(x, e.y, h, h);
        let kd = sub_id(MAILLES, 20_001 + k);
        if cx.ui.click(kd, case, None) {
            self.demander_enregistrement(k, c);
        }
        if cx.ui.pressed(kd) {
            cx.dl.rrect(case.inset(3.0 * d), cx.theme.rayon(case.h - 6.0 * d), col.puce);
        }
        cx.icon(case.inset(h * 0.26), Icon::Disquette, if self.modifie { col.accent } else { col.pale });
        // Le nom, et l'astérisque des modifications
        let nom = format!("{}{}", self.nom_affiche(), if self.modifie { "*" } else { "" });
        let place = (x - e.x - 4.0 * d).max(0.0);
        let wn = cx.measure(&nom, 14.0).min(place);
        let xn = x - wn - 2.0 * d;
        cx.clip(Some(Rect::new(xn, e.y, wn, h)));
        cx.text_at(xn, e.cy(), &nom, 14.0, if self.modifie { col.accent } else { col.texte });
        cx.clip(None);
        // L'état, ou le dernier message, à gauche
        let texte = match &self.message {
            Some((m, _)) => m.clone(),
            None => {
                let n = self.sel.nombre(self.mode);
                let choisis = if n > 0 { format!(" · {n} choisi{}", if n > 1 { "s" } else { "" }) } else { String::new() };
                let objet = self.objets.get(self.actif).map_or(String::new(), |o| format!("{} · ", o.nom));
                format!("{objet}{}{choisis}", self.mode.nom())
            }
        };
        let erreur = self.message.as_ref().is_some_and(|m| m.1);
        cx.clip(Some(Rect::new(e.x, e.y, (xn - e.x - 12.0 * d).max(0.0), h)));
        cx.mono_at(e.x + 4.0 * d, e.cy(), &texte, 12.0, if erreur { col.accent } else { col.pale });
        cx.clip(None);
    }
}

impl Contexte for Mailles {
    fn cle(&self) -> &'static str {
        CLE
    }

    /// Une nouvelle maille (un cube), avec les mêmes vues.
    fn dupliquer(&self) -> Box<dyn Contexte> {
        let mut vues = Surface::lire(&self.vues.ecrire()).unwrap_or_else(vues_par_defaut);
        vues.changements();
        Box::new(Mailles { vues, mode: self.mode, ..Mailles::default() })
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
        e.mettre("mode", self.mode.cle());
        e.mettre("vues", self.vues.ecrire_avec(&|v| VUES.get(v as usize).map_or("?", |x| x.0).to_string()));
        for (f, c) in &self.cameras {
            e.mettre(&format!("camera{f}"), format!("{} {} {} {} {} {}", c.lacet, c.tangage, c.distance, c.cible[0], c.cible[1], c.cible[2]));
        }
    }

    fn retour(&mut self, _c: &Commun) {
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
            c.dossier.as_ref().map(|d| d.join(BROUILLONS).join(format!("mailles-{t}.glb")))
        };
        let Some(chemin) = self.brouillon.clone().or_else(nouveau) else { return };
        match ecrire_octets(&chemin, &self.octets()) {
            Ok(()) => self.brouillon = Some(chemin),
            Err(e) => crate::android::log(&format!("brouillon Mailles non écrit : {e}")),
        }
    }

    fn frappe(&mut self, f: FenetreId, cible: Id, frappe: Frappe, c: &mut Commun) -> bool {
        if cible != sub_id(NOM, f as usize) {
            return false;
        }
        // Entrée enregistre (à la prochaine image, dans `dessiner_boite`)
        if self.nom.appliquer(frappe) {
            self.valider = self.boite;
            if !self.boite {
                c.clavier.fermer();
            }
        }
        self.erreur = None;
        true
    }

    fn dessiner(&mut self, cx: &mut Cx, c: &mut Commun, p: &Placee) -> String {
        if !self.charge {
            self.charger();
        }
        let r = p.contenu;
        let k = p.id as usize;
        self.dp = cx.dp;
        let corps = r;
        if std::mem::take(&mut self.cadrer) {
            // Chaque vue 3D se recadre à son premier dessin, une fois sa taille connue
            self.cameras.clear();
        }
        // Les vues de la fenêtre, partagées comme l'espace de travail
        let base = sub_id(MAILLES, k);
        self.vues.base = Some(base);
        self.vues.accueil = V3D;
        let placees = self.vues.frame(cx, corps, &VUES);
        for ch in self.vues.changements() {
            if let Changement::Fermee(f) = ch {
                self.cameras.remove(&f);
                self.zones.remove(&f);
            }
        }
        for vp in &placees {
            match vp.vue {
                V3D => self.vue_3d(cx, vp, base),
                ARBRE => self.vue_arbre(cx, c, vp, base),
                OPS => self.vue_operations(cx, c, vp, base),
                _ => {}
            }
        }
        self.vues.dessus(cx, &VUES);
        self.barre(cx, c, p);
        if self.boite {
            self.dessiner_boite(cx, c, p);
        }
        // Tout est dessiné dans la barre : la ligne d'état de Flow reste vide
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pret() -> Mailles {
        let mut a = Mailles { charge: true, dp: 1.0, ..Mailles::default() };
        a.zones.insert(1, Rect::new(0.0, 0.0, 800.0, 600.0));
        a.cameras.insert(1, Camera { lacet: 0.0, tangage: 0.0, distance: 6.0, cible: [0.0; 3] });
        a
    }

    #[test]
    fn toucher_choisit_selon_le_mode() {
        let mut a = pret();
        // Face au cube (+z vers l'œil) : le centre de l'écran tombe sur la face avant
        a.mode = Mode::Faces;
        a.recevoir(1, Geste::Tap { x: 400.0, y: 300.0, intensite: 0.0, precision: 0.0 });
        assert_eq!(a.sel.faces, [1].into_iter().collect());
        // Le même toucher la retire ; le vide vide la sélection
        a.recevoir(1, Geste::Tap { x: 400.0, y: 300.0, intensite: 0.0, precision: 0.0 });
        assert!(a.sel.vide());
        // En sommets, le coin haut droit de la face avant
        a.mode = Mode::Sommets;
        let mvp = a.camera(1).mvp(800.0 / 600.0);
        let q = projeter(&mvp, Rect::new(0.0, 0.0, 800.0, 600.0), [1.0, 1.0, 1.0]).unwrap();
        a.recevoir(1, Geste::Tap { x: q[0] + 3.0, y: q[1] - 2.0, intensite: 0.0, precision: 0.0 });
        assert_eq!(a.sel.sommets, [7].into_iter().collect());
        a.recevoir(1, Geste::Tap { x: 5.0, y: 5.0, intensite: 0.0, precision: 0.0 });
        assert!(a.sel.vide());
    }

    #[test]
    fn extruder_puis_glisser_le_long_de_la_normale_et_annuler() {
        let mut a = pret();
        a.mode = Mode::Faces;
        a.sel.faces.insert(1);
        a.executer("extruder");
        assert_eq!(a.objets[0].faces.len(), 10);
        assert!(matches!(a.transfo, Some((Transfo::Deplacer, Some(_)))));
        // Glisser vers le haut de l'écran : l'axe +z est face à l'œil, il ne bouge
        // presque pas à l'écran ; un axe de côté, oui. On vérifie que seuls les sommets
        // choisis bougent, le long de l'axe
        a.recevoir(1, Geste::SaisieDebut { x: 400.0, y: 300.0, doigt: (400.0, 300.0) });
        a.recevoir(1, Geste::SaisieDeplace { x: 400.0, y: 250.0, doigt: (400.0, 250.0) });
        a.recevoir(1, Geste::SaisieFin { x: 400.0, y: 250.0, depart: (400.0, 300.0), vecteur: (0.0, -50.0) });
        let o = &a.objets[0];
        assert!((0..8).all(|i| o.sommets[i] == m::cube("c", [0.0; 3], 1.0).sommets[i]));
        assert!((8..12).all(|i| o.sommets[i][0].abs() == 1.0 && o.sommets[i][1].abs() == 1.0));
        assert!(a.modifie && a.passe.len() == 2);
        a.defaire(false);
        a.defaire(false);
        assert_eq!(a.objets[0].faces.len(), 6);
    }

    #[test]
    fn la_barre_rapide_suit_la_selection() {
        let mut a = pret();
        let possibles = |a: &Mailles| -> Vec<&'static str> {
            let c = a.contexte_ops();
            m::OPERATIONS.iter().filter(|o| (o.possible)(&c)).map(|o| o.cle).collect()
        };
        assert_eq!(possibles(&a), ["tout", "cube"]);
        a.executer("tout");
        assert!(possibles(&a).contains(&"subdiviser") && possibles(&a).contains(&"supprimer"));
        a.executer("deplacer");
        assert!(a.transfo.is_some());
        a.executer("rien");
        assert!(a.sel.vide() && a.transfo.is_none());
    }

    #[test]
    fn l_enregistrement_garde_les_objets_a_leur_place() {
        let dir = std::env::temp_dir().join(format!("flow-mailles-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut a = pret();
        a.executer("cube");
        a.objets[1].matrice[12] = 5.0;
        a.objets[1].sommets.iter_mut().for_each(|p| p[0] += 5.0);
        let chemin = dir.join("essai.glb");
        a.chemin = Some(chemin.clone());
        let c = Commun::default();
        a.enregistrer(&c).unwrap();
        let mut b = Mailles { chemin: Some(chemin), ..Mailles::default() };
        b.charger();
        assert_eq!(b.objets.len(), 2);
        assert_eq!(b.objets[1].matrice[12], 5.0);
        let (x0, x1) = b.objets[1].boite().map(|(p, q)| (p[0], q[0])).unwrap();
        assert!(x0 > 3.0 && x1 > x0, "{x0} {x1}");
        assert_eq!(b.objets[0].faces.len(), 6, "les triangles redeviennent des quadrilatères");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn le_premier_enregistrement_demande_un_nom() {
        let dir = std::env::temp_dir().join(format!("flow-mailles-nom-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut c = Commun { racine: Some(dir.clone()), ..Commun::default() };
        let mut a = pret();
        a.executer("cube");
        // La disquette ouvre la boîte, avec un nom libre proposé et le clavier
        a.demander_enregistrement(1, &mut c);
        assert!(a.boite && a.chemin.is_none());
        assert_eq!(a.nom.texte, "maille-1");
        assert_eq!(c.clavier.cible, Some(sub_id(NOM, 1)));
        // Un nom vide est refusé ; un nom saisi enregistre et ferme la boîte
        a.nom = Edition::default();
        a.conclure_boite(1, &mut c);
        assert!(a.boite && a.erreur.is_some());
        a.nom = Edition::new("chaise");
        a.conclure_boite(1, &mut c);
        let attendu = dir.join("Documents").join("Mailles").join("chaise.glb");
        assert!(!a.boite && attendu.exists() && a.chemin.as_deref() == Some(attendu.as_path()));
        // Ensuite, la disquette enregistre sans rien demander
        a.demander_enregistrement(1, &mut c);
        assert!(!a.boite);
        // Un nom déjà pris est refusé dans la boîte
        let mut b = pret();
        b.demander_enregistrement(1, &mut c);
        assert_eq!(b.nom.texte, "maille-1");
        b.nom = Edition::new("chaise");
        b.conclure_boite(1, &mut c);
        assert!(b.boite && b.erreur.as_deref().is_some_and(|e| e.contains("existe déjà")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
