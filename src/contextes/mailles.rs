//! Le contexte Mailles : éditer la géométrie d'un `.glb`, sommet par sommet, arête
//! par arête, face par face.
//!
//! - **Des vues dans la fenêtre**, partagées comme l'espace de travail de Flow (le
//!   titre de chaque vue ouvre son menu : changer de vue, la partager, la fermer) :
//!   la **Vue 3D** (autant qu'on veut, chacune avec sa caméra), l'**Arborescence**
//!   (les objets du fichier), les **Opérations** (le tableau de tout ce qu'on peut
//!   faire, et quand) et les **Modifieurs** de l'objet actif.
//! - **Vue 3D** : en haut, le mode (Sommets, Arêtes, Faces) et « Cadrer » ; toucher un
//!   élément l'ajoute à la sélection ou l'en retire, toucher le vide la vide, toucher
//!   un autre objet le rend actif. Un doigt qui glisse fait tourner la vue, deux
//!   doigts rapprochent et déplacent. En bas, la **barre rapide** : les opérations que
//!   la sélection permet (`maille::OPERATIONS`). Déplacer, Tourner et Échelle
//!   s'allument : glisser transforme alors la sélection au lieu de tourner la vue.
//! - **En haut** : annuler, rétablir, le nom, « Enregistrer ».
//!
//! - **Modifieurs** (`crate::modifieurs`) : une pile par objet (Booléen avec une ou
//!   plusieurs cibles, Répétition, Miroir), calculée dans un fil sans toucher à la forme
//!   de base ; la vue 3D montre le résultat (la base reste en arêtes et sommets pour
//!   l'éditer), les cibles en fil de fer. « Appliquer » fige un modifieur, « Couper en
//!   deux » sépare l'objet par les cibles d'un booléen.
//!
//! Le fichier se réécrit en glTF binaire, un nœud par objet à sa place d'origine. Ce
//! que Mailles ne sait pas réécrire (armatures, animations, textures) n'est pas
//! perdu : le premier enregistrement va alors dans une copie (`nom-mailles.glb`).
//! L'enregistrement fige les modifieurs : chaque objet y va modifié, les cibles des
//! booléens non. Le brouillon (mis à l'abri), lui, garde les formes de base et leurs
//! piles (dans `brouillon.modifs`, à côté).

use crate::clavier::{self, Edition, Frappe};
use crate::contexte::{Commun, Contexte, Etat, Genre};
use crate::draw::{Color, Maillage3d, Rect, Vue3d};
use crate::gestes::Geste;
use crate::glb;
use crate::booleens::{Forme, Op};
use crate::maille::{self as m, Element, Mode, Objet, Selection, Transfo};
use crate::modifieurs::{self as mo, Genre as Gm, Modifieur, Resultats};
use crate::widgets::Body;
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
const BOITE: Id = id("mailles/boite");
const BROUILLONS: &str = "brouillons";
const HISTORIQUE: usize = 40;

/// Les vues de la fenêtre.
const VUES: [(&str, Icon); 4] = [("Vue 3D", Icon::Cube), ("Arborescence", Icon::List), ("Opérations", Icon::Menu), ("Modifieurs", Icon::Sliders)];
/// Les clés des vues dans l'état (le lecteur de dispositions ne prend que lettres,
/// chiffres, - et _ : pas les noms affichés).
const CLES_VUES: [&str; 4] = ["vue3d", "arbre", "operations", "modifieurs"];
const V3D: u32 = 0;
const ARBRE: u32 = 1;
const OPS: u32 = 2;
const MODIFS: u32 = 3;

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
    gpu: Option<(u64, Arc<Vec<f32>>, Arc<Vec<u32>>)>,
    surligne: Option<(u64, Arc<Vec<f32>>, Arc<Vec<u32>>)>,
    dp: f32,
    /// La boîte du premier enregistrement est ouverte : le nom se saisit dans `nom` ;
    /// l'erreur de la dernière tentative ; Entrée a été frappée.
    boite: bool,
    erreur: Option<String>,
    valider: bool,
    /// Les résultats des modifieurs (calculés pour la révision `.0`), le calcul en
    /// cours (pour sa révision), et un numéro qui change à chaque arrivée.
    resultats: (u64, Arc<Resultats>),
    calcul: Option<(u64, std::sync::mpsc::Receiver<Resultats>)>,
    res_num: u64,
    /// Le curseur qu'on glisse : ses retouches ne font qu'un pas d'annulation.
    fusion: Option<Id>,
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
            resultats: (u64::MAX, Arc::new(Resultats::new())),
            calcul: None,
            res_num: 0,
            fusion: None,
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
    if let Some(s) = e.texte("vues").and_then(|t| Surface::lire_avec(t, &|n| CLES_VUES.iter().position(|v| *v == n).map(|i| i as u32))) {
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

/// Des pilules posées dans une colonne défilante (à la ligne quand la place manque) ;
/// renvoie celle touchée.
fn pilules(cx: &mut Cx, b: &mut Body, base: Id, items: &[(&str, bool)]) -> Option<usize> {
    let (hc, gap) = (cx.commande(), cx.ecart() * 0.6);
    let l: Vec<f32> = items.iter().map(|(n, _)| cx.pill_width(n)).collect();
    let rang = super::fichiers::ranger(&l, b.w, gap);
    let col = cx.couleurs();
    let mut touchee = None;
    for (i, (nom, on)) in items.iter().enumerate() {
        let (x, ligne) = rang[i];
        let rect = Rect::new(b.x + x, b.y + ligne as f32 * (hc + gap), l[i], hc);
        let id = sub_id(base, i);
        if cx.ui.click(id, rect, Some(b.id())) {
            touchee = Some(i);
        }
        let ray = cx.theme.rayon(hc);
        cx.dl.rrect(rect, ray, if *on { col.accent } else if cx.ui.pressed(id) { col.ligne } else { col.puce });
        cx.centered(rect, nom, 15.0, if *on { col.sur_accent } else { col.texte });
    }
    let lignes = rang.iter().map(|x| x.1).max().map_or(0, |m| m + 1);
    b.y += lignes as f32 * (hc + gap) + gap;
    touchee
}

/// La normale d'un polygone (Newell).
fn normale(s: &[V3], p: &[u32]) -> V3 {
    let mut n = [0.0f32; 3];
    for i in 0..p.len() {
        let (a, b) = (s[p[i] as usize], s[p[(i + 1) % p.len()] as usize]);
        n[0] += (a[1] - b[1]) * (a[2] + b[2]);
        n[1] += (a[2] - b[2]) * (a[0] + b[0]);
        n[2] += (a[0] - b[0]) * (a[1] + b[1]);
    }
    if sc::norme(n) < 1e-12 {
        [0.0, 0.0, 1.0]
    } else {
        sc::unitaire(n)
    }
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
                } else if let Some(t) = self.brouillon.as_ref().and_then(|b| std::fs::read_to_string(b.with_extension("modifs")).ok()) {
                    self.lire_modifs(&t);
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
        // Posé à sa place, hors d'une colonne : rien ne le fait défiler
        let ch = Rect::new(zone.x + 14.0 * d, zone.y + 8.0 * d + (cx.theme.marge * 0.4 * d).round(), (zone.w - 28.0 * d).max(0.0), cx.commande());
        clavier::champ_en(cx, ch, None, zone, sub_id(NOM, k), "Nom", &mut self.nom, &mut c.clavier);
        cx.clip(None);
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
    /// avec des normales lissées. `figer` : chaque objet modifié par sa pile, les cibles
    /// des booléens laissées de côté (l'enregistrement) ; sinon les formes de base (le
    /// brouillon, qui garde les piles à côté).
    fn octets(&mut self, figer: bool) -> Vec<u8> {
        let (res, cibles) = if figer && self.objets.iter().any(|o| !o.modifs.is_empty()) {
            self.attendre_calculs();
            (self.resultats.1.clone(), mo::cibles(&self.objets))
        } else {
            (Arc::new(Resultats::new()), Default::default())
        };
        let formes: Vec<(usize, Forme)> = self
            .objets
            .iter()
            .enumerate()
            .filter(|(k, _)| !cibles.contains(k))
            .map(|(k, o)| (k, res.get(&o.id).map_or_else(|| mo::forme(o), |r| (*r.forme).clone())))
            .filter(|(_, f)| !(figer && f.vide()))
            .collect();
        let prepares: Vec<(usize, Vec<V3>, Vec<V3>, Vec<[u32; 3]>)> = formes
            .iter()
            .map(|(k, f)| {
                let inv = glb::inverse(&self.objets[*k].matrice);
                let local: Vec<V3> = f.sommets.iter().map(|p| glb::appliquer(&inv, *p)).collect();
                let tris: Vec<[u32; 3]> = f.faces.iter().flat_map(|p| (1..p.len().saturating_sub(1)).map(move |i| [p[0], p[i], p[i + 1]])).collect();
                let mut normales = vec![[0.0f32; 3]; local.len()];
                for t in &tris {
                    let [a, b, c] = t.map(|i| local[i as usize]);
                    let n = sc::vectoriel(sc::oter(b, a), sc::oter(c, a));
                    for &i in t {
                        normales[i as usize] = sc::ajouter(normales[i as usize], n);
                    }
                }
                let normales = normales.into_iter().map(|n| if sc::norme(n) > 1e-12 { sc::unitaire(n) } else { [0.0, 1.0, 0.0] }).collect();
                (*k, local, normales, tris)
            })
            .collect();
        let ecrits: Vec<glb::ObjetEcrit> = prepares
            .iter()
            .map(|(k, p, n, t)| {
                let o = &self.objets[*k];
                glb::ObjetEcrit { nom: &o.nom, matrice: o.matrice, positions: p, normales: n, triangles: t, couleur: o.couleur }
            })
            .collect();
        glb::ecrire_objets(&ecrits, "Flow · Mailles")
    }

    /// Les piles de modifieurs, en texte (le brouillon) : `objet k`, puis une ligne par
    /// modifieur ; les cibles par leur rang dans la liste des objets.
    fn ecrire_modifs(&self) -> String {
        let rang = |id: u64| self.objets.iter().position(|o| o.id == id);
        let b = |x: bool| if x { "1" } else { "0" };
        let mut t = String::new();
        for (k, o) in self.objets.iter().enumerate().filter(|(_, o)| !o.modifs.is_empty()) {
            t += &format!("objet {k}\n");
            for m in &o.modifs {
                let a = b(m.actif);
                match &m.genre {
                    Gm::Booleen { op, cibles } => {
                        let op = Op::TOUTES.iter().position(|x| x == op).unwrap_or(0);
                        let c: Vec<String> = cibles.iter().filter_map(|&id| rang(id)).map(|r| r.to_string()).collect();
                        t += &format!("booleen {a} {op} {}\n", c.join(" "));
                    }
                    Gm::Repetition { n, relatif, decalage: d, unir } => t += &format!("repetition {a} {n} {} {} {} {} {}\n", b(*relatif), d[0], d[1], d[2], b(*unir)),
                    Gm::Miroir { axes, souder } => t += &format!("miroir {a} {} {} {} {}\n", b(axes[0]), b(axes[1]), b(axes[2]), b(*souder)),
                }
            }
        }
        t
    }

    /// Relit les piles écrites par `ecrire_modifs` (ce qui ne se lit pas est ignoré).
    fn lire_modifs(&mut self, texte: &str) {
        let ids: Vec<u64> = self.objets.iter().map(|o| o.id).collect();
        let mut k: Option<usize> = None;
        for l in texte.lines() {
            let mots: Vec<&str> = l.split_whitespace().collect();
            let n = |i: usize| mots.get(i).and_then(|x| x.parse::<f32>().ok());
            let oui = |i: usize| mots.get(i) == Some(&"1");
            let genre = match mots.first().copied() {
                Some("objet") => {
                    k = n(1).map(|x| x as usize).filter(|&x| x < self.objets.len());
                    continue;
                }
                Some("booleen") => Gm::Booleen { op: Op::TOUTES[(n(2).unwrap_or(0.0) as usize).min(2)], cibles: mots.iter().skip(3).filter_map(|x| x.parse::<usize>().ok()).filter_map(|r| ids.get(r).copied()).collect() },
                Some("repetition") => Gm::Repetition { n: n(2).unwrap_or(1.0).max(1.0) as u32, relatif: oui(3), decalage: [n(4).unwrap_or(0.0), n(5).unwrap_or(0.0), n(6).unwrap_or(0.0)], unir: oui(7) },
                Some("miroir") => Gm::Miroir { axes: [oui(2), oui(3), oui(4)], souder: oui(5) },
                _ => continue,
            };
            if let Some(k) = k {
                self.objets[k].modifs.push(Modifieur { genre, actif: oui(1) });
            }
        }
    }

    fn enregistrer(&mut self, c: &Commun) -> Result<(), String> {
        let chemin = self.destination(c)?;
        let octets = self.octets(true);
        ecrire_octets(&chemin, &octets)?;
        self.chemin = Some(chemin);
        self.perdu.clear();
        self.modifie = false;
        self.nom = Edition::default();
        if let Some(b) = self.brouillon.take() {
            let _ = std::fs::remove_file(b.with_extension("modifs"));
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

    /// Suit le calcul des modifieurs : prend le résultat arrivé, et en lance un nouveau
    /// si la forme a changé depuis (un seul à la fois ; en attendant, le dernier résultat
    /// reste affiché).
    fn suivre_calculs(&mut self) {
        if let Some((rev, rx)) = &self.calcul {
            match rx.try_recv() {
                Ok(r) => {
                    self.resultats = (*rev, Arc::new(r));
                    self.calcul = None;
                    self.res_num += 1;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => return,
                Err(_) => {
                    // Le fil est tombé : ne pas recommencer sur la même forme
                    self.resultats = (*rev, Arc::new(Resultats::new()));
                    self.calcul = None;
                    self.res_num += 1;
                    self.message = Some(("Calcul des modifieurs impossible sur cette forme".into(), true));
                }
            }
        }
        if self.resultats.0 == self.revision {
            return;
        }
        if !self.objets.iter().any(|o| !o.modifs.is_empty()) {
            if !self.resultats.1.is_empty() {
                self.res_num += 1;
            }
            self.resultats = (self.revision, Arc::new(Resultats::new()));
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let objets = self.objets.clone();
        std::thread::spawn(move || {
            let _ = tx.send(mo::evaluer_tout(&objets));
        });
        self.calcul = Some((self.revision, rx));
    }

    /// Attend le calcul des modifieurs (essais, enregistrement).
    fn attendre_calculs(&mut self) {
        self.suivre_calculs();
        if let Some((rev, rx)) = self.calcul.take() {
            self.resultats = (rev, Arc::new(rx.recv().unwrap_or_default()));
            self.res_num += 1;
        }
    }

    /// La forme montrée d'un objet : son résultat s'il a des modifieurs (le dernier
    /// calculé), sinon sa forme.
    fn forme_vue(&self, o: &Objet) -> Option<Arc<Forme>> {
        self.resultats.1.get(&o.id).map(|r| r.forme.clone())
    }

    /// Ce qui part vers la carte graphique : les faces des objets visibles (modifiés,
    /// sans les cibles des booléens), à plat (trois sommets par triangle, avec la
    /// normale de sa face), et le surlignage des faces choisies de l'objet actif.
    fn maillages3d(&mut self) -> (Maillage3d, Option<Maillage3d>) {
        let cle_gpu = self.revision.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ self.res_num;
        if self.gpu.as_ref().is_none_or(|g| g.0 != cle_gpu) {
            let mut sommets = Vec::new();
            let cibles = mo::cibles(&self.objets);
            for (k, o) in self.objets.iter().enumerate().filter(|(_, o)| o.visible) {
                if cibles.contains(&k) {
                    continue;
                }
                let vue = self.forme_vue(o);
                let (s, faces) = match &vue {
                    Some(f) => (&f.sommets, &f.faces),
                    None => (&o.sommets, &o.faces),
                };
                for p in faces {
                    let n = normale(s, p);
                    for i in 1..p.len().saturating_sub(1) {
                        for v in [p[0], p[i], p[i + 1]] {
                            sommets.extend_from_slice(&s[v as usize]);
                            sommets.extend_from_slice(&n);
                        }
                    }
                }
            }
            let indices: Vec<u32> = (0..sommets.len() as u32 / 6).collect();
            self.gpu = Some((cle_gpu, Arc::new(sommets), Arc::new(indices)));
            self.surligne = None;
        }
        let (rev, sommets, indices) = self.gpu.clone().unwrap();
        let principal = Maillage3d { id: self.id, revision: rev, sommets, indices };
        // Les faces choisies (ou couvertes par la sélection) de l'objet actif, sur sa
        // forme de base
        let Some(o) = self.objets.get(self.actif).filter(|o| o.visible) else { return (principal, None) };
        let choisies = self.sel.faces_de(o, self.mode);
        if choisies.is_empty() {
            return (principal, None);
        }
        let cle = {
            use std::hash::{Hash, Hasher};
            let mut h = std::hash::DefaultHasher::new();
            (self.revision, self.actif, &choisies).hash(&mut h);
            h.finish()
        };
        if self.surligne.as_ref().is_none_or(|s| s.0 != cle) {
            let mut v = Vec::new();
            for &f in &choisies {
                let p = &o.faces[f];
                let n = normale(&o.sommets, p);
                for i in 1..p.len().saturating_sub(1) {
                    for k in [p[0], p[i], p[i + 1]] {
                        v.extend_from_slice(&o.sommets[k as usize]);
                        v.extend_from_slice(&n);
                    }
                }
            }
            let ind: Vec<u32> = (0..v.len() as u32 / 6).collect();
            self.surligne = Some((cle, Arc::new(v), Arc::new(ind)));
        }
        let s = self.surligne.as_ref().unwrap();
        (principal, Some(Maillage3d { id: self.id + 1, revision: s.0, sommets: s.1.clone(), indices: s.2.clone() }))
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
        // Les cibles des booléens, en fil de fer (celle de l'objet actif l'est déjà)
        let fil_cible = [col.pale[0], col.pale[1], col.pale[2], 150];
        for k in mo::cibles(&self.objets) {
            let o = &self.objets[k];
            if !o.visible || k == self.actif {
                continue;
            }
            let ecran: Vec<Option<[f32; 2]>> = o.sommets.iter().map(|q| projeter(&mvp, z, *q)).collect();
            for ar in o.aretes().into_iter().take(ARETES_MAX) {
                if let (Some(pa), Some(pb)) = (ecran[ar.0 as usize], ecran[ar.1 as usize]) {
                    cx.dl.line(pa, pb, 1.0 * d, fil_cible);
                }
            }
        }
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
        let cibles = mo::cibles(&self.objets);
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
            let mut info = format!("{} sommets · {} faces", o.sommets.len(), o.faces.len());
            if !o.modifs.is_empty() {
                info += &format!(" · {} modifieur{}", o.modifs.len(), if o.modifs.len() > 1 { "s" } else { "" });
            }
            if cibles.contains(&k) {
                info += " · cible";
            }
            cx.text_at(x, ligne.y + h * 0.72, &info, 12.0, col.pale);
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

    /// Les modifieurs de l'objet actif : chacun avec ses réglages, son interrupteur,
    /// son erreur s'il n'a pas pu s'appliquer, et ses actions (monter, descendre,
    /// appliquer, retirer ; couper en deux pour un booléen) ; en bas, de quoi en ajouter.
    fn vue_modifieurs(&mut self, cx: &mut Cx, c: &mut Commun, p: &Placee, base: Id) {
        let col = cx.couleurs();
        let r = p.contenu;
        let kv = sub_id(base, p.id as usize * 1000);
        cx.dl.rrect(r, 0.0, col.surface);
        let corps = sub_id(kv, 1);
        let mut b = cx.body(corps, r, &c.scrolls);
        let k = self.actif;
        let Some(o) = self.objets.get(k) else {
            cx.body_end(b, &mut c.scrolls);
            return;
        };
        let avant = self.instantane();
        let mut modifs = o.modifs.clone();
        let autres: Vec<(u64, String, [f32; 3])> = self.objets.iter().filter(|x| x.id != o.id).map(|x| (x.id, x.nom.clone(), x.couleur)).collect();
        let erreurs: Vec<Option<String>> = self.resultats.1.get(&o.id).map(|r| r.erreurs.clone()).unwrap_or_default();
        let en_cours = self.calcul.is_some();
        cx.section(&mut b, &o.nom.to_uppercase());
        if modifs.is_empty() {
            cx.paragraph(&mut b, "Aucun modifieur. Ils changent la forme montrée et enregistrée sans toucher à la forme de base, qui reste à éditer.");
        } else if en_cours {
            cx.paragraph(&mut b, "Calcul en cours…");
        }
        enum Action {
            Monter(usize),
            Descendre(usize),
            Appliquer(usize),
            Retirer(usize),
            Couper(usize),
            Ajouter(&'static str),
        }
        let mut action = None;
        let mut glisse = None;
        let n = modifs.len();
        for (i, m) in modifs.iter_mut().enumerate() {
            let mid = sub_id(kv, 100 + i * 100);
            cx.section(&mut b, &format!("{} · {}", i + 1, m.nom()));
            if let Some(Some(e)) = erreurs.get(i) {
                let y = b.y + 10.0 * cx.dp;
                cx.text_at(b.x, y, e, 14.0, col.accent);
                b.y += 24.0 * cx.dp;
            }
            cx.toggle(&mut b, sub_id(mid, 1), "Actif", &mut m.actif);
            match &mut m.genre {
                Gm::Booleen { op, cibles } => {
                    let ops: Vec<(&str, bool)> = Op::TOUTES.iter().map(|x| (x.nom(), x == op)).collect();
                    if let Some(j) = pilules(cx, &mut b, sub_id(mid, 10), &ops) {
                        *op = Op::TOUTES[j];
                    }
                    cx.section(&mut b, "CIBLES");
                    if autres.is_empty() {
                        cx.paragraph(&mut b, "Ajoutez un autre objet (Arborescence) pour qu'il serve de cible.");
                    }
                    for (j, (id, nom, couleur)) in autres.iter().enumerate() {
                        let choisie = cibles.contains(id);
                        if cx.row(&mut b, sub_id(mid, 20 + j), nom, Some(teinte(*couleur, 255)), choisie) {
                            if choisie {
                                cibles.retain(|x| x != id);
                            } else {
                                cibles.push(*id);
                            }
                        }
                    }
                }
                Gm::Repetition { n, relatif, decalage, unir } => {
                    let mut x = *n as f32;
                    let id = sub_id(mid, 10);
                    cx.slider(&mut b, id, "Copies", &n.to_string(), &mut x, 1.0, 20.0);
                    if cx.ui.pressed(id) {
                        glisse = Some(id);
                    }
                    *n = (x.round() as u32).clamp(1, 20);
                    cx.toggle(&mut b, sub_id(mid, 11), "Décalage relatif à la taille", relatif);
                    let lim = if *relatif { 3.0 } else { 10.0 };
                    for (a, nom) in ["X", "Y", "Z"].iter().enumerate() {
                        let id = sub_id(mid, 12 + a);
                        let mut v = decalage[a];
                        let montre = if *relatif { format!("{:.2} × taille", v) } else { format!("{:.2} m", v) };
                        cx.slider(&mut b, id, &format!("Décalage {nom}"), &montre, &mut v, -lim, lim);
                        if cx.ui.pressed(id) {
                            glisse = Some(id);
                            // Des pas ronds : un centième (relatif) ou un centimètre
                            decalage[a] = (v * 100.0).round() / 100.0;
                        }
                    }
                    cx.toggle(&mut b, sub_id(mid, 16), "Unir les copies", unir);
                }
                Gm::Miroir { axes, souder } => {
                    let l: Vec<(&str, bool)> = ["X", "Y", "Z"].iter().enumerate().map(|(a, nom)| (*nom, axes[a])).collect();
                    if let Some(j) = pilules(cx, &mut b, sub_id(mid, 10), &l) {
                        axes[j] = !axes[j];
                    }
                    cx.toggle(&mut b, sub_id(mid, 14), "Souder sur le plan", souder);
                }
            }
            let mut l: Vec<(&str, bool)> = Vec::new();
            let mut quoi = Vec::new();
            if i > 0 {
                l.push(("Monter", false));
                quoi.push(Action::Monter(i));
            }
            if i + 1 < n {
                l.push(("Descendre", false));
                quoi.push(Action::Descendre(i));
            }
            l.push(("Appliquer", false));
            quoi.push(Action::Appliquer(i));
            if matches!(m.genre, Gm::Booleen { .. }) {
                l.push(("Couper en deux", false));
                quoi.push(Action::Couper(i));
            }
            l.push(("Retirer", false));
            quoi.push(Action::Retirer(i));
            if let Some(j) = pilules(cx, &mut b, sub_id(mid, 50), &l) {
                action = quoi.into_iter().nth(j);
            }
        }
        cx.section(&mut b, "AJOUTER");
        let l: Vec<(&str, bool)> = mo::GENRES.iter().map(|g| (g.1, false)).collect();
        if let Some(j) = pilules(cx, &mut b, sub_id(kv, 90), &l) {
            action = Some(Action::Ajouter(mo::GENRES[j].0));
        }
        for g in mo::GENRES {
            let y = b.y + 8.0 * cx.dp;
            cx.text_at(b.x, y, &format!("{} : {}", g.1, g.2), 12.5, col.pale);
            b.y += 20.0 * cx.dp;
        }
        cx.body_end(b, &mut c.scrolls);
        // Les réglages changés : un pas d'annulation (un seul pour tout un glissé)
        if modifs != self.objets[k].modifs {
            self.objets[k].modifs = modifs;
            self.retouche();
            if glisse.is_some() && self.fusion == glisse {
                self.modifie = true;
                self.futur.clear();
            } else {
                self.noter(avant.clone());
            }
        }
        self.fusion = glisse;
        let Some(a) = action else { return };
        let avant = self.instantane();
        match a {
            Action::Monter(i) => self.objets[k].modifs.swap(i, i - 1),
            Action::Descendre(i) => self.objets[k].modifs.swap(i, i + 1),
            Action::Retirer(i) => {
                self.objets[k].modifs.remove(i);
            }
            Action::Ajouter(cle) => {
                if let Some(m) = Modifieur::neuf(cle) {
                    self.objets[k].modifs.push(m);
                }
            }
            Action::Appliquer(i) => match mo::appliquer(&self.objets, k, i) {
                Ok(f) if !f.vide() => {
                    let o = &mut self.objets[k];
                    o.sommets = f.sommets;
                    o.faces = f.faces;
                    o.modifs.remove(i);
                    self.sel = Selection::default();
                    self.transfo = None;
                }
                Ok(_) => self.message = Some(("Le résultat est vide : rien n'est appliqué.".into(), true)),
                Err(e) => self.message = Some((e, true)),
            },
            Action::Couper(i) => match mo::couper(&self.objets, k, i) {
                Ok((dehors, dedans)) if !dehors.vide() || !dedans.vide() => {
                    let cibles = match &self.objets[k].modifs[i].genre {
                        Gm::Booleen { cibles, .. } => cibles.clone(),
                        _ => Vec::new(),
                    };
                    let mut morceau = None;
                    {
                        let o = &mut self.objets[k];
                        o.modifs.remove(i);
                        let neuf = |f: Forme, nom: String, o: &Objet| Objet { id: m::nouvel_id(), nom, sommets: f.sommets, faces: f.faces, modifs: Vec::new(), ..o.clone() };
                        match (dehors.vide(), dedans.vide()) {
                            (false, false) => {
                                morceau = Some(neuf(dedans, format!("{} (dedans)", o.nom), o));
                                o.sommets = dehors.sommets;
                                o.faces = dehors.faces;
                            }
                            (false, true) => {
                                o.sommets = dehors.sommets;
                                o.faces = dehors.faces;
                            }
                            _ => {
                                o.sommets = dedans.sommets;
                                o.faces = dedans.faces;
                            }
                        }
                    }
                    // Les cibles ont fait leur office : cachées
                    for x in self.objets.iter_mut().filter(|x| cibles.contains(&x.id)) {
                        x.visible = false;
                    }
                    if let Some(mc) = morceau {
                        self.objets.insert(k + 1, mc);
                    }
                    self.sel = Selection::default();
                    self.transfo = None;
                }
                Ok(_) => self.message = Some(("Rien à couper : le résultat est vide.".into(), true)),
                Err(e) => self.message = Some((e, true)),
            },
        }
        self.retouche();
        self.noter(avant);
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
        e.mettre("vues", self.vues.ecrire_avec(&|v| CLES_VUES.get(v as usize).map_or("?", |x| x).to_string()));
        for (f, c) in &self.cameras {
            e.mettre(&format!("camera{f}"), format!("{} {} {} {} {} {}", c.lacet, c.tangage, c.distance, c.cible[0], c.cible[1], c.cible[2]));
        }
    }

    /// Tant que les modifieurs se calculent, l'image se redessine (pour voir le résultat
    /// arriver).
    fn anime(&self, _c: &Commun) -> bool {
        self.calcul.is_some()
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
                let _ = std::fs::remove_file(b.with_extension("modifs"));
                let _ = std::fs::remove_file(b);
            }
            return;
        }
        let nouveau = || {
            let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
            c.dossier.as_ref().map(|d| d.join(BROUILLONS).join(format!("mailles-{t}.glb")))
        };
        let Some(chemin) = self.brouillon.clone().or_else(nouveau) else { return };
        let piles = self.ecrire_modifs();
        let modifs = chemin.with_extension("modifs");
        let ecrit = if piles.is_empty() {
            let _ = std::fs::remove_file(&modifs);
            Ok(())
        } else {
            std::fs::write(&modifs, piles).map_err(|e| e.to_string())
        };
        match ecrit.and_then(|_| ecrire_octets(&chemin, &self.octets(false))) {
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
        self.suivre_calculs();
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
                MODIFS => self.vue_modifieurs(cx, c, vp, base),
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

    /// Un cube creusé par un second, décalé d'une demi-largeur.
    fn creuse() -> Mailles {
        let mut a = pret();
        a.objets = vec![m::cube("a", [0.0; 3], 0.5), m::cube("b", [0.5, 0.0, 0.0], 0.5)];
        let id = a.objets[1].id;
        a.objets[0].modifs.push(Modifieur { genre: Gm::Booleen { op: Op::Difference, cibles: vec![id] }, actif: true });
        a.retouche();
        a
    }

    #[test]
    fn le_resultat_se_calcule_a_part_et_se_montre_sans_la_cible() {
        let mut a = creuse();
        a.attendre_calculs();
        let r = &a.resultats.1[&a.objets[0].id];
        assert!((r.forme.volume() - 0.5).abs() < 1e-5);
        assert_eq!(a.objets[0].faces.len(), 6, "la forme de base ne change pas");
        // La vue : le seul résultat (la cible en fil de fer), 6 faces en 12 triangles
        let (p, _) = a.maillages3d();
        assert_eq!(p.sommets.len(), 12 * 3 * 6);
        // Une retouche relance le calcul
        a.objets[1].visible = false;
        a.retouche();
        a.attendre_calculs();
        let (p, _) = a.maillages3d();
        assert_eq!(p.sommets.len(), 12 * 3 * 6, "une cible cachée creuse toujours");
    }

    #[test]
    fn l_enregistrement_fige_et_le_brouillon_garde_les_piles() {
        let mut a = creuse();
        let lu = glb::lire_objets(&a.octets(true)).unwrap();
        assert_eq!(lu.objets.len(), 1, "la cible n'est pas enregistrée");
        let o = &lu.objets[0];
        let f = Forme { sommets: o.positions.clone(), faces: o.triangles.iter().map(|t| t.to_vec()).collect() };
        assert!((f.volume() - 0.5).abs() < 1e-5);
        // Le brouillon : les deux formes de base, et la pile à côté
        assert_eq!(glb::lire_objets(&a.octets(false)).unwrap().objets.len(), 2);
        let t = a.ecrire_modifs();
        let mut b = pret();
        b.objets = vec![m::cube("a", [0.0; 3], 0.5), m::cube("b", [0.5, 0.0, 0.0], 0.5)];
        b.lire_modifs(&t);
        assert_eq!(b.objets[0].modifs, vec![Modifieur { genre: Gm::Booleen { op: Op::Difference, cibles: vec![b.objets[1].id] }, actif: true }]);
        a.objets[0].modifs.push(Modifieur { genre: Gm::Repetition { n: 4, relatif: false, decalage: [0.5, -1.25, 0.0], unir: true }, actif: false });
        a.objets[1].modifs.push(Modifieur::neuf("miroir").unwrap());
        let mut c = pret();
        c.objets = vec![m::cube("a", [0.0; 3], 0.5), m::cube("b", [0.5, 0.0, 0.0], 0.5)];
        c.lire_modifs(&a.ecrire_modifs());
        assert_eq!(c.objets[0].modifs[1], a.objets[0].modifs[1]);
        assert_eq!(c.objets[1].modifs, a.objets[1].modifs);
    }

}
