//! Le contexte SVG : un éditeur de dessin vectoriel. Il montre tout SVG (voir
//! `crate::svg` pour ce qu'il comprend), et le retouche.
//!
//! - **Deux doigts** naviguent : pincer zoome autour des doigts, les glisser déplace
//!   la vue. À l'ouverture, le dessin est cadré dans la fenêtre ; « Cadrer » y revient.
//! - **Les outils**, en bas : Choisir, Rectangle, Ellipse, Ligne, Polygone, Texte.
//!   - Choisir : toucher une forme la sélectionne (l'enfant de la racine qui la
//!     contient : un groupe se prend en entier), glisser depuis une forme la déplace.
//!   - Sommets : toucher une forme (même dans un groupe) montre ses sommets (carrés)
//!     et les contrôles de ses courbes (ronds). Toucher un sommet le choisit ;
//!     glisser depuis un sommet ou un contrôle l'emmène au curseur déporté ; toucher
//!     le contour ajoute un sommet sans changer la forme. Pour le sommet choisi :
//!     Retirer (les courbes voisines se fondent), Courber ou Redresser le segment qui
//!     y arrive, Ouvrir ou Fermer le tracé. Un rectangle, une ellipse… retouchés
//!     deviennent un `path` ; un polygone le reste tant qu'il est droit.
//!   - Rectangle, Ellipse, Ligne : glisser trace la forme, du départ du curseur
//!     déporté jusqu'à lui. Toucher sélectionne, comme Choisir.
//!   - Polygone : chaque toucher pose un sommet ; toucher le premier, ou « Fermer »,
//!     termine.
//!   - Texte : toucher ouvre la saisie du texte, posé là.
//! - **Le style** : Fond ou Trait, puis une couleur (ou Aucun) et une épaisseur. Il
//!   s'applique à la sélection, et aux formes qu'on trace ensuite.
//! - **Une sélection** montre sa boîte, quatre poignées d'angle pour l'agrandir
//!   (l'angle opposé reste en place) et une poignée ronde au-dessus pour la tourner ;
//!   et ses actions : Dupliquer, Simplifier, Supprimer, Monter, Descendre, Fin. Les
//!   retouches s'écrivent dans l'attribut `transform`.
//! - **Simplifier** (une sélection, ou la forme de l'outil Sommets) retire les sommets
//!   inutiles des chemins, polygones et polylignes qu'elle contient
//!   (`svg::simplifier`), en aperçu : « Sans perte » d'abord, puis un écart permis
//!   (en unités du dessin) et ce que deviennent les suites de droites : Droites,
//!   Courbes rapides (Schneider), Courbes optimales (le moins de sommets possible) ou
//!   B-spline (nœuds placés seuls, courbure continue). Les sommets restants sont montrés, et leur compte ;
//!   « Appliquer » garde le résultat, « Annuler » (ou toute autre action) y renonce.
//!   Le dernier réglage est retenu.
//! - **En haut** : annuler, rétablir (l'historique dure la session), le nom, « Source »
//!   (le code du dessin dans l'éditeur de texte, coloré ; « Dessin » y revient en le
//!   relisant, ou dit l'erreur), « Enregistrer ».
//!
//! Comme Texte : un nouveau dessin va dans `Documents/SVG/` sous le nom saisi
//! (`dessin-N` sinon), et ce qui n'est pas enregistré est mis à l'abri dans un
//! brouillon (`Flow/brouillons/`). Un fichier illisible s'ouvre sur sa source, pour
//! qu'on la répare.

use super::texte::Texte;
use crate::clavier::{self, Edition, Frappe};
use crate::contexte::{Commun, Contexte, Etat, Genre};
use crate::draw::{Color, Rect};
use crate::gestes::Geste;
use crate::surface::{FenetreId, Placee};
use crate::svg::{self, Ajustement, Document, Forme, Matrice, Morceau, Repere, Simplification, P};
use crate::ui::{id, sub_id, Id, Sense};
use crate::widgets::{Cx, Icon};
use std::path::{Path, PathBuf};

pub const CLE: &str = "svg";
pub const GENRE: Genre = Genre { cle: CLE, nom: "SVG", legende: "Dessin vectoriel : formes, sélection, source", icone: Icon::Vecteur, ouvrir, applet: None };

const SVG: Id = id("svg");
const NOM: Id = id("svg/nom");
const NOM_CORPS: Id = id("svg/nom-corps");
const SAISIE: Id = id("svg/saisie");
const SAISIE_CORPS: Id = id("svg/saisie-corps");
const POIGNEE: Id = id("svg/poignee");
const BROUILLONS: &str = "brouillons";
const ZOOM: (f32, f32) = (0.02, 50.0);
const HISTORIQUE: usize = 200;
const PAPIER: Color = [0xFF, 0xFF, 0xFF, 255];
/// Les écarts proposés pour « Simplifier », en unités du dessin (0 : sans perte).
const TOLERANCES: [f32; 7] = [0.0, 0.1, 0.25, 0.5, 1.0, 2.0, 5.0];

/// Une simplification en aperçu : sur quoi, le document d'origine (et écrit, pour
/// l'historique), et les sommets avant et après.
#[derive(Clone, Debug)]
struct Apercu {
    noeud: usize,
    origine: Document,
    avant: String,
    compte: (usize, usize),
}

/// Les couleurs proposées (la dernière case est « Aucun »).
const COULEURS: [Color; 10] = [
    [0x1E, 0x1E, 0x25, 255],
    [0xFF, 0xFF, 0xFF, 255],
    [0x6B, 0x72, 0x80, 255],
    [0xDC, 0x26, 0x26, 255],
    [0xEA, 0x8A, 0x0C, 255],
    [0xFA, 0xCC, 0x15, 255],
    [0x16, 0xA3, 0x4A, 255],
    [0x25, 0x63, 0xEB, 255],
    [0x93, 0x33, 0xEA, 255],
    [0xDB, 0x27, 0x77, 255],
];
/// Les épaisseurs de trait proposées, en unités du dessin.
const EPAISSEURS: [f32; 5] = [0.5, 1.0, 2.0, 4.0, 8.0];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outil {
    Choisir,
    Sommets,
    Rectangle,
    Ellipse,
    Ligne,
    Polygone,
    Texte,
}

impl Outil {
    const TOUS: [Outil; 7] = [Outil::Choisir, Outil::Sommets, Outil::Rectangle, Outil::Ellipse, Outil::Ligne, Outil::Polygone, Outil::Texte];

    fn nom(self) -> &'static str {
        match self {
            Outil::Choisir => "Choisir",
            Outil::Sommets => "Sommets",
            Outil::Rectangle => "Rectangle",
            Outil::Ellipse => "Ellipse",
            Outil::Ligne => "Ligne",
            Outil::Polygone => "Polygone",
            Outil::Texte => "Texte",
        }
    }
}

/// Un commandement de la barre du bas.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Commande {
    Outil(Outil),
    Cible(bool),
    Couleur(Option<Color>),
    Epaisseur(f32),
    Dupliquer,
    Supprimer,
    Monter,
    Descendre,
    Fin,
    Fermer,
    Abandonner,
    Cadrer,
    RetirerSommet,
    Basculer,
    OuvrirFermer,
    Simplifier,
    /// L'écart permis : un indice de `TOLERANCES`.
    Tolerance(usize),
    Ajuster(Ajustement),
    Appliquer,
    Renoncer,
}

/// Un doigt qui travaille : trace une forme, ou déplace la sélection.
#[derive(Clone, Debug, PartialEq)]
enum Travail {
    Trace { depart: P, fin: P },
    Deplace { noeud: usize, avant: String, depart: P, instantane: String },
    /// Un sommet ou un contrôle du tracé choisi, emmené par le curseur.
    Repere { repere: Repere, instantane: String },
}

/// Une poignée tenue : laquelle (0 à 3 les angles, 4 la rotation), la
/// transformation d'avant, le point fixe (ou le centre), où le doigt l'a prise, et le
/// document d'avant.
#[derive(Clone, Debug, PartialEq)]
struct Prise {
    poignee: usize,
    avant: String,
    fixe: P,
    depart: P,
    instantane: String,
}

/// La vue : le point du dessin au coin haut gauche de la zone, et le zoom (1 : une
/// unité du dessin vaut un dp).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Vue {
    x: f32,
    y: f32,
    zoom: f32,
}

pub struct Svg {
    chemin: Option<PathBuf>,
    brouillon: Option<PathBuf>,
    modifie: bool,
    nom: Edition,
    charge: bool,
    message: Option<(String, bool)>,
    doc: Document,
    /// Augmente à chaque retouche : les formes sont refaites.
    revision: u64,
    formes: Option<(u64, Vec<Forme>)>,
    /// L'historique : le document (écrit) d'avant chaque retouche, puis ce qu'on peut
    /// rétablir.
    passe: Vec<String>,
    futur: Vec<String>,
    vue: Vue,
    /// La vue reste à cadrer (à l'ouverture, ou demandé).
    cadrer: bool,
    zone: Rect,
    dp: f32,
    outil: Outil,
    /// Le style : la couleur choisie s'applique au fond (vrai) ou au trait.
    cible_fond: bool,
    fond: Option<Color>,
    trait_: Option<Color>,
    epaisseur: f32,
    selection: Option<usize>,
    /// Le sommet choisi du tracé (outil Sommets).
    pointe: Option<usize>,
    travail: Option<Travail>,
    prise: Option<Prise>,
    /// Les sommets d'un polygone en cours.
    sommets: Vec<P>,
    /// Un texte à poser là, et sa saisie.
    texte_en: Option<P>,
    saisie: Edition,
    pincement: f32,
    /// La source, ouverte dans l'éditeur de texte, et le texte qu'elle avait au départ.
    source: Option<(Texte, String)>,
    /// La simplification en aperçu, et son réglage (retenu).
    apercu: Option<Apercu>,
    tolerance: usize,
    ajustement: Ajustement,
}

impl Default for Svg {
    fn default() -> Svg {
        Svg {
            chemin: None,
            brouillon: None,
            modifie: false,
            nom: Edition::default(),
            charge: false,
            message: None,
            doc: Document::nouveau(800.0, 600.0),
            revision: 0,
            formes: None,
            passe: Vec::new(),
            futur: Vec::new(),
            vue: Vue { x: 0.0, y: 0.0, zoom: 1.0 },
            cadrer: true,
            zone: Rect::new(0.0, 0.0, 1.0, 1.0),
            dp: 1.0,
            outil: Outil::Choisir,
            cible_fond: true,
            fond: Some(COULEURS[7]),
            trait_: Some(COULEURS[0]),
            epaisseur: 2.0,
            selection: None,
            pointe: None,
            travail: None,
            prise: None,
            sommets: Vec::new(),
            texte_en: None,
            saisie: Edition::default(),
            pincement: 1.0,
            source: None,
            apercu: None,
            tolerance: 0,
            ajustement: Ajustement::Droites,
        }
    }
}

fn ouvrir(e: &Etat) -> Box<dyn Contexte> {
    let v = e.nombres("vue");
    let mut s = Svg {
        chemin: e.texte("document").filter(|t| !t.is_empty()).map(PathBuf::from),
        brouillon: e.texte("brouillon").map(PathBuf::from),
        nom: Edition::new(e.texte("nom").unwrap_or("")),
        ..Svg::default()
    };
    if let [t, c] = e.nombres("simplifier")[..] {
        s.tolerance = (t.max(0.0) as usize).min(TOLERANCES.len() - 1);
        s.ajustement = Ajustement::TOUS[(c.max(0.0) as usize).min(Ajustement::TOUS.len() - 1)];
    }
    if let [x, y, z] = v[..] {
        s.vue = Vue { x, y, zoom: z.clamp(ZOOM.0, ZOOM.1) };
        s.cadrer = false;
    }
    Box::new(s)
}

/// Une couleur et son opacité, pour un attribut : « #rrggbb », et l'opacité si elle
/// n'est pas pleine.
fn ecrire_couleur(c: Option<Color>) -> (String, Option<String>) {
    match c {
        None => ("none".into(), None),
        Some(c) => (svg::hex(c), (c[3] < 255).then(|| svg::nombre(c[3] as f32 / 255.0))),
    }
}

impl Svg {
    fn vers_dessin(&self, x: f32, y: f32) -> P {
        let k = self.vue.zoom * self.dp;
        (self.vue.x + (x - self.zone.x) / k, self.vue.y + (y - self.zone.y) / k)
    }

    fn vers_ecran(&self, p: P) -> [f32; 2] {
        let k = self.vue.zoom * self.dp;
        [self.zone.x + (p.0 - self.vue.x) * k, self.zone.y + (p.1 - self.vue.y) * k]
    }

    /// Lit le brouillon s'il y en a un, sinon le fichier. Un texte illisible ouvre la
    /// source.
    fn charger(&mut self) {
        self.charge = true;
        let (texte, brouillon) = match self.brouillon.as_ref().and_then(|b| std::fs::read_to_string(b).ok()) {
            Some(t) => (Some(t), true),
            None => {
                self.brouillon = None;
                match &self.chemin {
                    Some(c) => match std::fs::read_to_string(c) {
                        Ok(t) => (Some(t), false),
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
        let Some(t) = texte else { return };
        self.modifie = brouillon;
        match Document::lire(&t) {
            Ok(d) => {
                self.doc = d;
                self.retouche();
                if brouillon {
                    self.message = Some(("Brouillon non enregistré retrouvé".into(), false));
                }
            }
            Err(e) => {
                self.message = Some((format!("SVG illisible : {e}"), true));
                let mut ed = Texte::integre("source.svg", &t);
                ed.signaler(Some((format!("SVG illisible : {e}"), true)));
                self.source = Some((ed, String::new()));
            }
        }
    }

    /// Les formes du document, refaites après une retouche.
    fn formes(&mut self) -> &[Forme] {
        if self.formes.as_ref().is_none_or(|f| f.0 != self.revision) {
            self.formes = Some((self.revision, svg::rendu(&self.doc)));
        }
        &self.formes.as_ref().unwrap().1
    }

    /// Le document a changé (les formes sont à refaire).
    fn retouche(&mut self) {
        self.revision += 1;
    }

    /// Retient le document d'avant une retouche, pour l'annuler.
    fn noter(&mut self, avant: String) {
        self.passe.push(avant);
        if self.passe.len() > HISTORIQUE {
            self.passe.remove(0);
        }
        self.futur.clear();
        self.modifie = true;
        self.message = None;
    }

    fn defaire(&mut self, retablir: bool) {
        // Pendant un aperçu, annuler y renonce seulement
        if self.apercu.is_some() {
            self.conclure_simplification(false);
            return;
        }
        let (de, vers) = if retablir { (&mut self.futur, &mut self.passe) } else { (&mut self.passe, &mut self.futur) };
        let Some(t) = de.pop() else { return };
        vers.push(self.doc.ecrire());
        if let Ok(d) = Document::lire(&t) {
            self.doc = d;
        }
        self.selection = None;
        self.pointe = None;
        self.prise = None;
        self.travail = None;
        self.modifie = true;
        self.retouche();
    }

    /// Le tracé de la forme choisie, s'il s'édite, et sa transformation vers le
    /// document.
    fn trace_choisi(&self) -> Option<(usize, Vec<Morceau>, Matrice)> {
        let k = self.selection?;
        Some((k, svg::trace_de(&self.doc.noeuds[k])?, self.doc.ctm(k)))
    }

    /// Le repère du tracé choisi le plus proche de `p` (dans le dessin), à portée de
    /// doigt.
    fn repere_proche(&self, p: P) -> Option<Repere> {
        let (_, t, m) = self.trace_choisi()?;
        let portee = 18.0 / self.vue.zoom;
        svg::reperes(&t)
            .into_iter()
            .map(|(r, q, _)| {
                let q = m.appliquer(q);
                (r, (q.0 - p.0).hypot(q.1 - p.1))
            })
            .filter(|x| x.1 < portee)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|x| x.0)
    }

    /// Retouche le tracé choisi ; `f` dit si elle a eu lieu. Retenu pour annuler.
    fn modifier_trace(&mut self, f: impl FnOnce(&mut Vec<Morceau>) -> bool) -> bool {
        let Some((k, mut t, _)) = self.trace_choisi() else { return false };
        let avant_t = t.clone();
        if !f(&mut t) || t == avant_t {
            return false;
        }
        let avant = self.doc.ecrire();
        self.doc.poser_trace(k, &t);
        self.noter(avant);
        self.retouche();
        true
    }

    /// Un toucher avec l'outil Sommets : choisit un sommet, coupe le contour touché
    /// (un sommet de plus), ou choisit la forme touchée, même dans un groupe.
    fn toucher_sommets(&mut self, p: P) {
        if let Some(r) = self.repere_proche(p) {
            if let Repere::Sommet(i) = r {
                self.pointe = Some(i);
            }
            return;
        }
        let portee = 18.0 / self.vue.zoom;
        if let Some((_, t, m)) = self.trace_choisi() {
            if let Some((i, u)) = svg::segment_proche(&t, &m, p, portee * 0.7) {
                let mut nouveau = None;
                self.modifier_trace(|t| {
                    nouveau = svg::couper(t, i, u);
                    nouveau.is_some()
                });
                self.pointe = nouveau;
                return;
            }
        }
        let doc = self.doc.clone();
        let formes = self.formes().to_vec();
        self.pointe = None;
        self.selection = formes.iter().rev().find(|f| f.touche(p, portee * 0.5) && svg::trace_de(&doc.noeuds[f.noeud]).is_some()).map(|f| f.noeud);
        if self.selection.is_none() {
            self.message = Some(("Sommets : touchez un chemin ou une forme simple".into(), false));
        }
    }

    /// Cadre le dessin dans la zone : sa vue (`viewBox`), sinon ce qu'il dessine.
    fn cadrer(&mut self) {
        let cadre = self.doc.cadre().or_else(|| {
            let doc = self.doc.clone();
            let f = self.formes().to_vec();
            svg::boite_de(&doc, &f, doc.racine).map(|b| [b[0], b[1], b[2] - b[0], b[3] - b[1]])
        });
        let [x, y, w, h] = cadre.unwrap_or([0.0, 0.0, 800.0, 600.0]);
        let (w, h) = (w.max(1.0), h.max(1.0));
        let zw = self.zone.w / self.dp;
        let zh = self.zone.h / self.dp;
        let zoom = (zw / w).min(zh / h) * 0.92;
        self.vue.zoom = zoom.clamp(ZOOM.0, ZOOM.1);
        self.vue.x = x + w / 2.0 - zw / self.vue.zoom / 2.0;
        self.vue.y = y + h / 2.0 - zh / self.vue.zoom / 2.0;
    }

    fn destination(&self, c: &Commun) -> Result<PathBuf, String> {
        if let Some(p) = &self.chemin {
            return Ok(p.clone());
        }
        let dossier = c.racine.as_ref().map(|r| r.join("Documents").join("SVG")).or_else(|| c.dossier.clone()).ok_or("Aucun dossier où enregistrer.")?;
        let saisi = self.nom.texte.trim().trim_end_matches(".svg").to_string();
        if saisi.contains('/') || saisi.starts_with('.') {
            return Err("Nom de fichier invalide.".into());
        }
        if saisi.is_empty() {
            return (1..).map(|n| dossier.join(format!("dessin-{n}.svg"))).find(|p| !p.exists()).ok_or_else(|| "Aucun nom libre.".into());
        }
        let p = dossier.join(format!("{saisi}.svg"));
        if p.exists() {
            return Err(format!("{saisi}.svg existe déjà : choisissez un autre nom."));
        }
        Ok(p)
    }

    /// Le texte à écrire : la source si elle est ouverte, sinon le document.
    fn texte(&self) -> String {
        match &self.source {
            Some((ed, _)) => ed.contenu(),
            None => self.doc.ecrire(),
        }
    }

    fn enregistrer(&mut self, c: &Commun) -> Result<(), String> {
        // Ce qu'on voit est ce qu'on enregistre
        self.conclure_simplification(true);
        let chemin = self.destination(c)?;
        super::texte::ecrire_fichier(&chemin, &self.texte())?;
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
            None => "Nouveau dessin".into(),
        }
    }

    /// Ouvre la source dans l'éditeur de texte.
    fn ouvrir_source(&mut self) {
        self.conclure_simplification(true);
        let t = self.doc.ecrire();
        self.source = Some((Texte::integre("source.svg", &t), t));
        self.travail = None;
        self.prise = None;
    }

    /// Revient au dessin en relisant la source. Vrai si elle se lit.
    fn fermer_source(&mut self) -> bool {
        let Some((ed, depart)) = &mut self.source else { return true };
        let t = ed.contenu();
        if t == *depart {
            self.source = None;
            return true;
        }
        match Document::lire(&t) {
            Ok(d) => {
                let avant = self.doc.ecrire();
                self.doc = d;
                self.selection = None;
                self.source = None;
                self.noter(avant);
                self.retouche();
                true
            }
            Err(e) => {
                ed.signaler(Some((format!("Le dessin ne se lit pas : {e}"), true)));
                false
            }
        }
    }

    /// Ajoute une forme tracée, avec le style courant, et la sélectionne.
    fn creer(&mut self, nom: &str, mut attrs: Vec<(String, String)>) {
        let avant = self.doc.ecrire();
        let ligne = nom == "line";
        let (f, fo) = ecrire_couleur(if ligne { None } else { self.fond });
        let (t, to) = ecrire_couleur(if ligne { self.trait_.or(Some(COULEURS[0])) } else { self.trait_ });
        attrs.push(("fill".into(), f));
        if let Some(o) = fo {
            attrs.push(("fill-opacity".into(), o));
        }
        attrs.push(("stroke".into(), t));
        if let Some(o) = to {
            attrs.push(("stroke-opacity".into(), o));
        }
        attrs.push(("stroke-width".into(), svg::nombre(self.epaisseur)));
        let racine = self.doc.racine;
        let k = self.doc.ajouter(racine, nom, attrs);
        self.selection = Some(k);
        self.noter(avant);
        self.retouche();
    }

    fn creer_trace(&mut self, a: P, b: P) {
        let n = |v: f32| svg::nombre(v);
        let (x0, y0, x1, y1) = (a.0.min(b.0), a.1.min(b.1), a.0.max(b.0), a.1.max(b.1));
        // Trop petit pour être voulu
        let seuil = 4.0 / self.vue.zoom;
        if (x1 - x0).max(y1 - y0) < seuil {
            return;
        }
        match self.outil {
            Outil::Rectangle => self.creer("rect", vec![("x".into(), n(x0)), ("y".into(), n(y0)), ("width".into(), n(x1 - x0)), ("height".into(), n(y1 - y0))]),
            Outil::Ellipse => self.creer("ellipse", vec![("cx".into(), n((x0 + x1) / 2.0)), ("cy".into(), n((y0 + y1) / 2.0)), ("rx".into(), n((x1 - x0) / 2.0)), ("ry".into(), n((y1 - y0) / 2.0))]),
            Outil::Ligne => self.creer("line", vec![("x1".into(), n(a.0)), ("y1".into(), n(a.1)), ("x2".into(), n(b.0)), ("y2".into(), n(b.1))]),
            _ => {}
        }
    }

    fn fermer_polygone(&mut self) {
        let s = std::mem::take(&mut self.sommets);
        if s.len() >= 3 {
            let points = s.iter().map(|p| format!("{},{}", svg::nombre(p.0), svg::nombre(p.1))).collect::<Vec<_>>().join(" ");
            self.creer("polygon", vec![("points".into(), points)]);
        }
    }

    fn poser_texte(&mut self) {
        let Some(p) = self.texte_en.take() else { return };
        let t = self.saisie.texte.trim().to_string();
        self.saisie = Edition::default();
        if t.is_empty() {
            return;
        }
        let avant = self.doc.ecrire();
        let taille = (20.0 / self.vue.zoom).max(1.0).round();
        let (f, fo) = ecrire_couleur(self.fond.or(Some(COULEURS[0])));
        let mut attrs = vec![("x".into(), svg::nombre(p.0)), ("y".into(), svg::nombre(p.1)), ("font-size".into(), svg::nombre(taille)), ("fill".into(), f)];
        if let Some(o) = fo {
            attrs.push(("fill-opacity".into(), o));
        }
        let racine = self.doc.racine;
        let k = self.doc.ajouter(racine, "text", attrs);
        self.doc.ajouter_texte(k, &t);
        self.selection = Some(k);
        self.noter(avant);
        self.retouche();
    }

    /// La couleur ou l'épaisseur choisies : le style courant, et la sélection.
    fn styler(&mut self, c: Commande) {
        let Some(k) = self.selection else {
            match c {
                Commande::Couleur(x) if self.cible_fond => self.fond = x,
                Commande::Couleur(x) => self.trait_ = x,
                Commande::Epaisseur(e) => self.epaisseur = e,
                _ => {}
            }
            return;
        };
        let avant = self.doc.ecrire();
        match c {
            Commande::Couleur(x) => {
                let (v, o) = ecrire_couleur(x);
                let (p, po) = if self.cible_fond { ("fill", "fill-opacity") } else { ("stroke", "stroke-opacity") };
                if self.cible_fond {
                    self.fond = x;
                } else {
                    self.trait_ = x;
                }
                self.doc.definir(k, p, Some(&v));
                self.doc.definir(k, po, o.as_deref());
                // Un trait choisi sans épaisseur se voit
                if !self.cible_fond && x.is_some() && self.doc.noeuds[k].attr("stroke-width").is_none() {
                    self.doc.definir(k, "stroke-width", Some(&svg::nombre(self.epaisseur)));
                }
            }
            Commande::Epaisseur(e) => {
                self.epaisseur = e;
                self.doc.definir(k, "stroke-width", Some(&svg::nombre(e)));
            }
            _ => return,
        }
        // Dans un groupe, les enfants qui ont leur propre style le gardent
        self.noter(avant);
        self.retouche();
    }

    fn commander(&mut self, c: Commande) {
        // Toute autre action renonce à l'aperçu en cours
        if self.apercu.is_some() && !matches!(c, Commande::Tolerance(_) | Commande::Ajuster(_) | Commande::Appliquer | Commande::Renoncer | Commande::Cadrer) {
            self.conclure_simplification(false);
        }
        match c {
            Commande::Simplifier => self.ouvrir_simplification(),
            Commande::Tolerance(i) => {
                self.tolerance = i.min(TOLERANCES.len() - 1);
                self.simplifier_apercu();
            }
            Commande::Ajuster(a) => {
                self.ajustement = a;
                self.simplifier_apercu();
            }
            Commande::Appliquer => self.conclure_simplification(true),
            Commande::Renoncer => self.conclure_simplification(false),
            Commande::Outil(o) => {
                self.outil = o;
                self.sommets.clear();
                self.pointe = None;
                if o == Outil::Sommets && self.trace_choisi().is_none() {
                    self.selection = None;
                }
            }
            Commande::RetirerSommet => {
                if let Some(i) = self.pointe.take() {
                    if !self.modifier_trace(|t| svg::retirer_sommet(t, i)) {
                        self.message = Some(("Un tracé garde au moins deux sommets : « Supprimer » (outil Choisir) retire la forme.".into(), true));
                    }
                }
            }
            Commande::Basculer => {
                if let Some(i) = self.pointe {
                    // Un départ courbé gagne un sommet jumeau : le sommet choisi reste
                    self.modifier_trace(|t| svg::basculer(t, i));
                }
            }
            Commande::OuvrirFermer => {
                if let Some(i) = self.pointe {
                    self.modifier_trace(|t| {
                        svg::ouvrir_fermer(t, i);
                        true
                    });
                }
            }
            Commande::Cible(f) => self.cible_fond = f,
            Commande::Couleur(_) | Commande::Epaisseur(_) => self.styler(c),
            Commande::Dupliquer => {
                if let Some(k) = self.selection {
                    let avant = self.doc.ecrire();
                    if let Some(n) = self.doc.dupliquer(k) {
                        // Un peu décalée, pour qu'on la voie
                        let t = self.doc.noeuds[n].attr("transform").unwrap_or("").to_string();
                        let d = 10.0 / self.vue.zoom;
                        self.doc.transformer(n, &Matrice::translation(d, d), &t);
                        self.selection = Some(n);
                        self.noter(avant);
                        self.retouche();
                    }
                }
            }
            Commande::Supprimer => {
                if let Some(k) = self.selection.take() {
                    let avant = self.doc.ecrire();
                    self.doc.retirer(k);
                    self.noter(avant);
                    self.retouche();
                }
            }
            Commande::Monter | Commande::Descendre => {
                if let Some(k) = self.selection {
                    let avant = self.doc.ecrire();
                    if self.doc.changer_plan(k, c == Commande::Monter) {
                        self.noter(avant);
                        self.retouche();
                    }
                }
            }
            Commande::Fin => {
                self.selection = None;
                self.pointe = None;
            }
            Commande::Fermer => self.fermer_polygone(),
            Commande::Abandonner => self.sommets.clear(),
            Commande::Cadrer => self.cadrer = true,
        }
    }

    /// Vrai si la sélection contient de quoi simplifier.
    fn simplifiable(&self) -> bool {
        let Some(k) = self.selection else { return false };
        (0..self.doc.noeuds.len()).any(|n| matches!(self.doc.noeuds[n].nom.as_str(), "path" | "polygon" | "polyline") && self.doc.sous(n, k))
    }

    /// Le réglage courant de « Simplifier ».
    fn simplification(&self) -> Simplification {
        match TOLERANCES[self.tolerance] {
            0.0 => Simplification::SANS_PERTE,
            e => Simplification { tolerance: e, ajustement: self.ajustement },
        }
    }

    /// Ouvre l'aperçu de la simplification de la sélection.
    fn ouvrir_simplification(&mut self) {
        let Some(k) = self.selection else { return };
        self.apercu = Some(Apercu { noeud: k, origine: self.doc.clone(), avant: self.doc.ecrire(), compte: (0, 0) });
        self.travail = None;
        self.prise = None;
        self.simplifier_apercu();
    }

    /// Refait l'aperçu depuis l'original, au réglage courant.
    fn simplifier_apercu(&mut self) {
        let s = self.simplification();
        let Some(a) = &mut self.apercu else { return };
        let mut d = a.origine.clone();
        a.compte = d.simplifier(a.noeud, s);
        self.doc = d;
        self.retouche();
    }

    /// Ferme l'aperçu : garde la simplification (retenue pour annuler) ou y renonce.
    fn conclure_simplification(&mut self, garder: bool) {
        let Some(a) = self.apercu.take() else { return };
        if garder && self.doc != a.origine {
            self.noter(a.avant);
            self.pointe = None;
            self.message = Some((format!("Simplifié : de {} à {} sommets", a.compte.0, a.compte.1), false));
        } else {
            self.doc = a.origine;
            self.retouche();
        }
    }

    /// Emmène un repère du tracé choisi en `p` (dans le dessin), sans le retenir
    /// pour annuler (c'est fait à la fin de la saisie).
    fn emmener(&mut self, r: Repere, p: P) {
        let Some((k, mut t, m)) = self.trace_choisi() else { return };
        svg::deplacer(&mut t, r, m.inverse().appliquer(p));
        self.doc.poser_trace(k, &t);
        self.retouche();
    }

    /// Un geste sur le dessin.
    fn recevoir(&mut self, g: Geste, c: &mut Commun, k: usize) {
        let tolerance = 10.0 / self.vue.zoom;
        // Toucher le dessin pendant un aperçu y renonce (naviguer à deux doigts non)
        if self.apercu.is_some() && matches!(g, Geste::Tap { .. } | Geste::Appui { .. } | Geste::AppuiLong { .. } | Geste::SaisieDebut { .. }) {
            self.conclure_simplification(false);
            return;
        }
        match g {
            Geste::Tap { x, y, .. } | Geste::Appui { x, y, .. } => {
                let p = self.vers_dessin(x, y);
                self.message = None;
                match self.outil {
                    Outil::Polygone => {
                        // Toucher le premier sommet ferme le polygone
                        let premier = self.sommets.first().map(|s| self.vers_ecran(*s));
                        if self.sommets.len() >= 3 && premier.is_some_and(|q| (q[0] - x).hypot(q[1] - y) < 16.0 * self.dp) {
                            self.fermer_polygone();
                        } else {
                            self.sommets.push(p);
                        }
                    }
                    Outil::Sommets => self.toucher_sommets(p),
                    Outil::Texte => {
                        self.texte_en = Some(p);
                        self.saisie = Edition::default();
                        c.clavier.ouvrir(sub_id(SAISIE, k));
                    }
                    _ => {
                        let doc = self.doc.clone();
                        let f = self.formes().to_vec();
                        self.selection = svg::sous(&doc, &f, p, tolerance);
                    }
                }
            }
            Geste::SaisieDebut { x, y, .. } => {
                let p = self.vers_dessin(x, y);
                match self.outil {
                    Outil::Rectangle | Outil::Ellipse | Outil::Ligne => self.travail = Some(Travail::Trace { depart: p, fin: p }),
                    Outil::Sommets => {
                        if let Some(r) = self.repere_proche(p) {
                            if let Repere::Sommet(i) = r {
                                self.pointe = Some(i);
                            }
                            self.travail = Some(Travail::Repere { repere: r, instantane: self.doc.ecrire() });
                        }
                    }
                    Outil::Choisir => {
                        let doc = self.doc.clone();
                        let f = self.formes().to_vec();
                        if let Some(n) = svg::sous(&doc, &f, p, tolerance) {
                            self.selection = Some(n);
                            let avant = self.doc.noeuds[n].attr("transform").unwrap_or("").to_string();
                            self.travail = Some(Travail::Deplace { noeud: n, avant, depart: p, instantane: self.doc.ecrire() });
                        }
                    }
                    _ => {}
                }
            }
            Geste::SaisieDeplace { x, y, .. } => {
                let p = self.vers_dessin(x, y);
                match &mut self.travail {
                    Some(Travail::Trace { fin, .. }) => *fin = p,
                    Some(Travail::Deplace { noeud, avant, depart, .. }) => {
                        let (n, a, d) = (*noeud, avant.clone(), *depart);
                        self.doc.transformer(n, &Matrice::translation(p.0 - d.0, p.1 - d.1), &a);
                        self.retouche();
                    }
                    Some(Travail::Repere { repere, .. }) => {
                        let r = *repere;
                        self.emmener(r, p);
                    }
                    None => {}
                }
            }
            Geste::SaisieFin { x, y, .. } => {
                let p = self.vers_dessin(x, y);
                match self.travail.take() {
                    Some(Travail::Trace { depart, .. }) => self.creer_trace(depart, p),
                    Some(Travail::Deplace { noeud, avant, depart, instantane }) => {
                        self.doc.transformer(noeud, &Matrice::translation(p.0 - depart.0, p.1 - depart.1), &avant);
                        if self.doc.ecrire() != instantane {
                            self.noter(instantane);
                        }
                        self.retouche();
                    }
                    Some(Travail::Repere { repere, instantane }) => {
                        self.emmener(repere, p);
                        if self.doc.ecrire() != instantane {
                            self.noter(instantane);
                        }
                    }
                    None => {}
                }
            }
            Geste::SaisieAnnulee { .. } => match self.travail.take() {
                Some(Travail::Deplace { noeud, avant, .. }) => {
                    self.doc.transformer(noeud, &Matrice::IDENTITE, &avant);
                    self.retouche();
                }
                Some(Travail::Repere { instantane, .. }) => {
                    if let Ok(d) = Document::lire(&instantane) {
                        self.doc = d;
                        self.retouche();
                    }
                }
                _ => {}
            },
            Geste::PincementDebut { .. } => self.pincement = 1.0,
            Geste::Pincement { echelle, centre, deplacement } => {
                let k = self.vue.zoom * self.dp;
                self.vue.x -= deplacement.0 / k;
                self.vue.y -= deplacement.1 / k;
                let sous = self.vers_dessin(centre.0, centre.1);
                let f = echelle / self.pincement.max(1e-3);
                self.pincement = echelle;
                self.vue.zoom = (self.vue.zoom * f).clamp(ZOOM.0, ZOOM.1);
                let k = self.vue.zoom * self.dp;
                self.vue.x = sous.0 - (centre.0 - self.zone.x) / k;
                self.vue.y = sous.1 - (centre.1 - self.zone.y) / k;
            }
            _ => {}
        }
    }

    /// Dessine les formes dans la zone.
    fn peindre(&mut self, cx: &mut Cx, z: Rect) {
        let (d, col) = (cx.dp, cx.couleurs());
        cx.dl.rrect(z, 0.0, col.fond);
        cx.clip(Some(z));
        // La page : la vue du document
        if let Some([x, y, w, h]) = self.doc.cadre() {
            let a = self.vers_ecran((x, y));
            let b = self.vers_ecran((x + w, y + h));
            let page = Rect::new(a[0], a[1], b[0] - a[0], b[1] - a[1]);
            cx.dl.shadow(page, 0.0, 8.0 * d, [0, 0, 0, 50]);
            cx.dl.rrect(page, 0.0, PAPIER);
        }
        let formes = self.formes().to_vec();
        let k = self.vue.zoom * self.dp;
        for f in &formes {
            let a = self.vers_ecran((f.boite[0], f.boite[1]));
            let b = self.vers_ecran((f.boite[2], f.boite[3]));
            let e = f.epaisseur * k;
            if b[0] + e < z.x || a[0] - e > z.right() || b[1] + e < z.y || a[1] - e > z.bottom() {
                continue;
            }
            if let Some(t) = &f.texte {
                let Some(c) = f.fond else { continue };
                let p = self.vers_ecran((t.x, t.y));
                let px = t.taille * k;
                if px < 2.0 {
                    continue;
                }
                let taille = px / (d * cx.theme.texte);
                let w = cx.measure(&t.chaine, taille);
                let x = match t.ancre {
                    0 => p[0] - w / 2.0,
                    1 => p[0] - w,
                    _ => p[0],
                };
                cx.text_at(x, p[1] - px * 0.33, &t.chaine, taille, c);
                continue;
            }
            let ecran: Vec<Vec<P>> = f.contours.iter().map(|(v, _)| v.iter().map(|p| self.vers_ecran(*p)).map(|q| (q[0], q[1])).collect()).collect();
            if let Some(c) = f.fond {
                let (x0, x1) = (z.x - 2.0, z.right() + 2.0);
                let dl = &mut *cx.dl;
                svg::balayer(&ecran, f.pair_impair, z.y, z.bottom(), |y, a, b| {
                    let (a, b) = (a.max(x0), b.min(x1));
                    if b > a {
                        dl.bande(y, a, b, c);
                    }
                });
            }
            if let Some(c) = f.trait_ {
                let e = e.max(1.0);
                for (i, v) in ecran.iter().enumerate() {
                    let n = v.len();
                    let ferme = f.contours[i].1;
                    let segs = if ferme { n } else { n.saturating_sub(1) };
                    for j in 0..segs {
                        let (a, b) = (v[j], v[(j + 1) % n]);
                        cx.dl.line([a.0, a.1], [b.0, b.1], e, c);
                    }
                }
            }
        }
        cx.clip(None);
    }

    /// Ce qui se dessine par-dessus : la forme en cours, les sommets du polygone, la
    /// sélection et ses poignées. Renvoie les poignées (rectangles à saisir).
    fn superposer(&mut self, cx: &mut Cx, z: Rect, k: usize) {
        let (d, col) = (cx.dp, cx.couleurs());
        cx.clip(Some(z));
        let accent = col.accent;
        let voile = [accent[0], accent[1], accent[2], 60];
        match &self.travail {
            Some(Travail::Trace { depart, fin }) => {
                let (a, b) = (self.vers_ecran(*depart), self.vers_ecran(*fin));
                if self.outil == Outil::Ligne {
                    cx.dl.line(a, b, 2.0 * d, accent);
                } else {
                    let r = Rect::new(a[0].min(b[0]), a[1].min(b[1]), (a[0] - b[0]).abs(), (a[1] - b[1]).abs());
                    let rayon = if self.outil == Outil::Ellipse { r.w.min(r.h) / 2.0 } else { 0.0 };
                    cx.dl.rrect(r, rayon, voile);
                }
            }
            Some(Travail::Deplace { .. } | Travail::Repere { .. }) | None => {}
        }
        if !self.sommets.is_empty() {
            let pts: Vec<[f32; 2]> = self.sommets.iter().map(|p| self.vers_ecran(*p)).collect();
            for w in pts.windows(2) {
                cx.dl.line(w[0], w[1], 2.0 * d, accent);
            }
            for (i, p) in pts.iter().enumerate() {
                let r = if i == 0 { 7.0 } else { 4.5 } * d;
                cx.dl.rrect(Rect::new(p[0] - r, p[1] - r, 2.0 * r, 2.0 * r), r, accent);
            }
        }
        // La sélection : sa boîte, ses poignées
        let sel = self.selection.filter(|s| self.doc.noeuds[*s].parent.is_some());
        if sel.is_none() {
            self.selection = None;
        }
        if self.apercu.is_some() {
            self.prise = None;
            self.montrer_apercu(cx);
        } else if self.outil == Outil::Sommets {
            self.prise = None;
            if sel.is_some() {
                self.montrer_sommets(cx);
            }
        } else if let Some(s) = sel {
            let doc = self.doc.clone();
            let f = self.formes().to_vec();
            if let Some(b) = svg::boite_de(&doc, &f, s) {
                let a = self.vers_ecran((b[0], b[1]));
                let c = self.vers_ecran((b[2], b[3]));
                let r = Rect::new(a[0], a[1], (c[0] - a[0]).max(1.0), (c[1] - a[1]).max(1.0));
                let t = d.max(1.0);
                for (p, q) in [((r.x, r.y), (r.right(), r.y)), ((r.right(), r.y), (r.right(), r.bottom())), ((r.right(), r.bottom()), (r.x, r.bottom())), ((r.x, r.bottom()), (r.x, r.y))] {
                    cx.dl.line([p.0, p.1], [q.0, q.1], t, accent);
                }
                // Les angles, dans l'ordre : haut gauche, haut droit, bas droit, bas gauche
                let angles = [(r.x, r.y), (r.right(), r.y), (r.right(), r.bottom()), (r.x, r.bottom())];
                let opposes = [(b[2], b[3]), (b[0], b[3]), (b[0], b[1]), (b[2], b[1])];
                let rot = (r.cx(), r.y - 28.0 * d);
                cx.dl.line([r.cx(), r.y], [rot.0, rot.1], t, accent);
                let centre = ((b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0);
                let mut tenue = None;
                for i in 0..5 {
                    let (x, y) = if i < 4 { angles[i] } else { rot };
                    let tenue_ici = self.prise.as_ref().is_some_and(|p| p.poignee == i);
                    let rc = if tenue_ici { 8.0 } else { 6.0 } * d;
                    if i < 4 {
                        cx.dl.rrect(Rect::new(x - rc, y - rc, 2.0 * rc, 2.0 * rc), 2.0 * d, PAPIER);
                        cx.dl.rrect(Rect::new(x - rc + t, y - rc + t, 2.0 * rc - 2.0 * t, 2.0 * rc - 2.0 * t), 1.5 * d, accent);
                    } else {
                        cx.dl.rrect(Rect::new(x - rc, y - rc, 2.0 * rc, 2.0 * rc), rc, accent);
                        cx.dl.rrect(Rect::new(x - rc * 0.45, y - rc * 0.45, rc * 0.9, rc * 0.9), rc, PAPIER);
                    }
                    let prise = Rect::new(x - 22.0 * d, y - 22.0 * d, 44.0 * d, 44.0 * d);
                    if let Some(g) = cx.ui.drag(sub_id(POIGNEE, k * 8 + i), prise) {
                        tenue = Some((i, self.vers_dessin(g.x, g.y)));
                    }
                }
                match tenue {
                    Some((i, p)) => {
                        if self.prise.as_ref().is_none_or(|x| x.poignee != i) {
                            let avant = self.doc.noeuds[s].attr("transform").unwrap_or("").to_string();
                            self.prise = Some(Prise { poignee: i, avant, fixe: if i < 4 { opposes[i] } else { centre }, depart: p, instantane: self.doc.ecrire() });
                        }
                        let pr = self.prise.clone().unwrap();
                        let m = if i < 4 {
                            // Agrandir : l'angle opposé reste en place
                            let e = |a: f32, b: f32| {
                                let v = if b.abs() < 1e-6 { 1.0 } else { a / b };
                                if v.abs() < 0.02 { 0.02f32.copysign(v) } else { v }
                            };
                            let (sx, sy) = (e(p.0 - pr.fixe.0, pr.depart.0 - pr.fixe.0), e(p.1 - pr.fixe.1, pr.depart.1 - pr.fixe.1));
                            Matrice::translation(pr.fixe.0, pr.fixe.1).fois(&Matrice::echelle(sx, sy)).fois(&Matrice::translation(-pr.fixe.0, -pr.fixe.1))
                        } else {
                            // Tourner autour du centre, par crans de 15° à l'approche
                            let a0 = (pr.depart.1 - pr.fixe.1).atan2(pr.depart.0 - pr.fixe.0);
                            let a1 = (p.1 - pr.fixe.1).atan2(p.0 - pr.fixe.0);
                            let mut da = (a1 - a0).to_degrees();
                            let cran = (da / 15.0).round() * 15.0;
                            if (da - cran).abs() < 3.0 {
                                da = cran;
                            }
                            Matrice::rotation(da.to_radians(), pr.fixe.0, pr.fixe.1)
                        };
                        self.doc.transformer(s, &m, &pr.avant);
                        self.retouche();
                    }
                    None => {
                        if let Some(pr) = self.prise.take() {
                            if self.doc.ecrire() != pr.instantane {
                                self.noter(pr.instantane);
                            }
                        }
                    }
                }
            }
        } else {
            self.prise = None;
        }
        cx.clip(None);
    }

    /// Le tracé choisi (outil Sommets) : son contour, ses sommets, ses contrôles et
    /// leurs tiges.
    /// Pendant un aperçu de simplification : les sommets qui restent.
    fn montrer_apercu(&mut self, cx: &mut Cx) {
        let Some(k) = self.apercu.as_ref().map(|a| a.noeud) else { return };
        let (d, col) = (cx.dp, cx.couleurs());
        let rc = 3.5 * d;
        for n in 0..self.doc.noeuds.len() {
            if !matches!(self.doc.noeuds[n].nom.as_str(), "path" | "polygon" | "polyline") || !self.doc.sous(n, k) {
                continue;
            }
            let Some(t) = svg::trace_de(&self.doc.noeuds[n]) else { continue };
            let m = self.doc.ctm(n);
            for p in t.iter().filter_map(Morceau::fin) {
                let q = self.vers_ecran(m.appliquer(p));
                cx.dl.rrect(Rect::new(q[0] - rc, q[1] - rc, 2.0 * rc, 2.0 * rc), d, col.accent);
                cx.dl.rrect(Rect::new(q[0] - rc + d, q[1] - rc + d, 2.0 * (rc - d), 2.0 * (rc - d)), d * 0.5, PAPIER);
            }
        }
    }

    fn montrer_sommets(&mut self, cx: &mut Cx) {
        let Some((k, t, m)) = self.trace_choisi() else { return };
        let (d, col) = (cx.dp, cx.couleurs());
        let accent = col.accent;
        let tige = [accent[0], accent[1], accent[2], 150];
        let formes = self.formes().to_vec();
        for f in formes.iter().filter(|f| f.noeud == k) {
            for (v, ferme) in &f.contours {
                let e: Vec<[f32; 2]> = v.iter().map(|p| self.vers_ecran(*p)).collect();
                let n = if *ferme { e.len() } else { e.len().saturating_sub(1) };
                for j in 0..n {
                    cx.dl.line(e[j], e[(j + 1) % e.len()], 1.5 * d, accent);
                }
            }
        }
        let tenu = match &self.travail {
            Some(Travail::Repere { repere, .. }) => Some(*repere),
            _ => None,
        };
        let reperes = svg::reperes(&t);
        // Les tiges d'abord, puis les contrôles, puis les sommets par-dessus
        for (_, p, liens) in &reperes {
            let q = self.vers_ecran(m.appliquer(*p));
            for l in liens {
                cx.dl.line(q, self.vers_ecran(m.appliquer(*l)), d.max(1.0), tige);
            }
        }
        for passe in 0..2 {
            for (r, p, _) in &reperes {
                let q = self.vers_ecran(m.appliquer(*p));
                let grand = if tenu == Some(*r) { 1.5 } else { 1.0 };
                match (r, passe) {
                    (Repere::Controle(..), 0) => {
                        let rc = 4.5 * d * grand;
                        cx.dl.rrect(Rect::new(q[0] - rc, q[1] - rc, 2.0 * rc, 2.0 * rc), rc, accent);
                        cx.dl.rrect(Rect::new(q[0] - rc * 0.5, q[1] - rc * 0.5, rc, rc), rc, PAPIER);
                    }
                    (Repere::Sommet(i), 1) => {
                        let rc = 5.5 * d * grand;
                        let choisi = self.pointe == Some(*i);
                        cx.dl.rrect(Rect::new(q[0] - rc, q[1] - rc, 2.0 * rc, 2.0 * rc), 1.5 * d, accent);
                        let b = 1.5 * d;
                        cx.dl.rrect(Rect::new(q[0] - rc + b, q[1] - rc + b, 2.0 * (rc - b), 2.0 * (rc - b)), d, if choisi { accent } else { PAPIER });
                    }
                    _ => {}
                }
            }
        }
    }

    /// Les commandes de la barre du bas, par groupes (chacun sur sa ligne).
    fn commandes(&self) -> Vec<Vec<(Commande, String, bool)>> {
        let mut g = Vec::new();
        let mut outils: Vec<(Commande, String, bool)> = Outil::TOUS.iter().map(|o| (Commande::Outil(*o), o.nom().to_string(), *o == self.outil)).collect();
        outils.push((Commande::Cadrer, "Cadrer".into(), false));
        g.push(outils);
        if self.outil == Outil::Polygone && !self.sommets.is_empty() {
            let mut v = Vec::new();
            if self.sommets.len() >= 3 {
                v.push((Commande::Fermer, "Fermer le polygone".into(), true));
            }
            v.push((Commande::Abandonner, "Abandonner".into(), false));
            g.push(v);
        }
        // Le style : la cible, les couleurs (« Aucun » en dernier), les épaisseurs
        let actuelle = if self.cible_fond { self.fond } else { self.trait_ };
        let mut style = vec![(Commande::Cible(true), "Fond".to_string(), self.cible_fond), (Commande::Cible(false), "Trait".to_string(), !self.cible_fond)];
        for c in COULEURS {
            style.push((Commande::Couleur(Some(c)), String::new(), actuelle == Some(c)));
        }
        style.push((Commande::Couleur(None), "Aucun".into(), actuelle.is_none()));
        if !self.cible_fond {
            for e in EPAISSEURS {
                style.push((Commande::Epaisseur(e), svg::nombre(e), (self.epaisseur - e).abs() < 1e-4));
            }
        }
        g.push(style);
        if let Some(a) = &self.apercu {
            let mut v: Vec<(Commande, String, bool)> = TOLERANCES
                .iter()
                .enumerate()
                .map(|(i, e)| (Commande::Tolerance(i), if *e == 0.0 { "Sans perte".to_string() } else { format!("± {}", svg::nombre(*e).replace('.', ",")) }, i == self.tolerance))
                .collect();
            g.push(v);
            if self.tolerance > 0 {
                g.push(
                    Ajustement::TOUS
                        .iter()
                        .map(|a| {
                            let nom = match a {
                                Ajustement::Droites => "Droites",
                                Ajustement::Rapide => "Courbes rapides",
                                Ajustement::Optimal => "Courbes optimales",
                                Ajustement::Spline => "B-spline",
                            };
                            (Commande::Ajuster(*a), nom.to_string(), *a == self.ajustement)
                        })
                        .collect(),
                );
            }
            g.push(vec![
                (Commande::Appliquer, format!("Appliquer · de {} à {} sommets", a.compte.0, a.compte.1), true),
                (Commande::Renoncer, "Annuler".into(), false),
            ]);
        } else if self.outil == Outil::Sommets {
            if let Some((_, t, _)) = self.trace_choisi() {
                let mut v = Vec::new();
                if let Some(i) = self.pointe.filter(|i| t.get(*i).is_some_and(|m| m.fin().is_some())) {
                    v.push((Commande::RetirerSommet, "Retirer le sommet".to_string(), false));
                    if let Some(droit) = svg::droit(&t, i) {
                        v.push((Commande::Basculer, if droit { "Courber" } else { "Redresser" }.into(), false));
                    }
                    v.push((Commande::OuvrirFermer, if svg::est_ferme(&t, i) { "Ouvrir" } else { "Fermer" }.into(), false));
                }
                if self.simplifiable() {
                    v.push((Commande::Simplifier, "Simplifier".into(), false));
                }
                v.push((Commande::Fin, "Fin".into(), false));
                g.push(v);
            }
        } else if self.selection.is_some() {
            let mut v = vec![(Commande::Dupliquer, "Dupliquer".to_string(), false)];
            if self.simplifiable() {
                v.push((Commande::Simplifier, "Simplifier".into(), false));
            }
            v.extend([
                (Commande::Supprimer, "Supprimer".into(), false),
                (Commande::Monter, "Monter".into(), false),
                (Commande::Descendre, "Descendre".into(), false),
                (Commande::Fin, "Fin".into(), false),
            ]);
            g.push(v);
        }
        g
    }

    /// La barre du bas : sa hauteur pour une largeur, et (si `dessiner`) les
    /// commandes, dont celle touchée.
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
            let largeurs: Vec<f32> = groupe.iter().map(|(c, nom, _)| if matches!(c, Commande::Couleur(Some(_))) { hc * 0.8 } else { cx.pill_width(nom) }).collect();
            let rangs = super::fichiers::ranger(&largeurs, place, gap * 0.6);
            let lignes = rangs.iter().map(|x| x.1).max().unwrap_or(0) + 1;
            if dessiner {
                for (i, (c, nom, actif)) in groupe.iter().enumerate() {
                    let (x, l) = rangs[i];
                    let (x, yy) = (r.x + pad + x, y + l as f32 * (hc + gap * 0.6));
                    n += 1;
                    let kc = sub_id(SVG, k * 64 + n);
                    if let Commande::Couleur(Some(couleur)) = c {
                        let case = Rect::new(x, yy, largeurs[i], hc);
                        if cx.ui.click(kc, case, None) {
                            choisie = Some(*c);
                        }
                        let rayon = case.w * 0.36;
                        let (ccx, ccy) = (case.cx(), case.cy());
                        if *actif {
                            cx.dl.secteur([ccx, ccy], rayon + 3.0 * d, rayon + 5.0 * d, 0.0, std::f32::consts::TAU, 0.0, col.texte);
                        }
                        cx.dl.rrect(Rect::new(ccx - rayon, ccy - rayon, 2.0 * rayon, 2.0 * rayon), rayon, *couleur);
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

    /// En haut : annuler, rétablir, le nom, « Source » ou « Dessin », « Enregistrer ».
    fn entete(&mut self, cx: &mut Cx, c: &mut Commun, p: &Placee, r: Rect) {
        let (d, col) = (cx.dp, cx.couleurs());
        let (h, m, gap) = (cx.commande(), cx.coin(), cx.ecart());
        let k = p.id as usize;
        cx.dl.rrect(r, 0.0, col.surface);
        let mut x0 = p.suite.0;
        for (j, (icone, retablir)) in [(Icon::Annuler, false), (Icon::Retablir, true)].into_iter().enumerate() {
            let case = Rect::new(x0, r.y + m, h, h);
            let possible = match &self.source {
                Some((ed, _)) => if retablir { ed.peut_retablir() } else { ed.peut_annuler() },
                None => !if retablir { &self.futur } else { &self.passe }.is_empty(),
            };
            let kb = sub_id(SVG, 10_000 + k * 2 + j);
            if cx.ui.click(kb, case, None) && possible {
                match &mut self.source {
                    Some((ed, _)) => ed.defaire(retablir),
                    None => self.defaire(retablir),
                }
            }
            if possible && cx.ui.pressed(kb) {
                cx.dl.rrect(case.inset(2.0 * d), cx.theme.rayon(case.h - 4.0 * d), col.puce);
            }
            cx.icon(case.inset(cx.pad() * 0.6), icone, if possible { col.texte } else { col.ligne });
            x0 += h;
        }
        x0 += gap;
        let enregistrer = "Enregistrer";
        let bascule = if self.source.is_some() { "Dessin" } else { "Source" };
        let we = cx.pill_width(enregistrer);
        let wb = cx.pill_width(bascule);
        let xe = r.right() - we - m;
        let xb = xe - gap - wb;
        if xb > x0 + gap {
            if cx.pill(sub_id(SVG, 20_000 + k), xb, r.y + m, bascule, self.source.is_some()) {
                if self.source.is_some() {
                    if self.fermer_source() {
                        self.message = None;
                    }
                } else {
                    self.ouvrir_source();
                }
            }
            if cx.pill(sub_id(SVG, 20_001 + k), xe, r.y + m, enregistrer, self.modifie) {
                self.message = Some(match self.enregistrer(c) {
                    Ok(()) => (format!("Enregistré : {}", self.chemin.as_deref().map_or(String::new(), |p| p.display().to_string())), false),
                    Err(e) => (e, true),
                });
            }
        }
        let x1 = (xb - gap).max(x0);
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
            let mut b = cx.body(sub_id(NOM_CORPS, k), zone, &c.scrolls);
            clavier::champ(cx, &mut b, sub_id(NOM, k), "Nom du dessin", &mut self.nom, &mut c.clavier);
            cx.body_end(b, &mut c.scrolls);
        }
        cx.dl.rrect(Rect::new(r.x, r.bottom() - d, r.w, d.max(1.0)), 0.0, col.ligne);
    }

    /// La saisie d'un texte à poser : une carte en haut du dessin.
    fn saisir_texte(&mut self, cx: &mut Cx, c: &mut Commun, z: Rect, k: usize) {
        let (d, col) = (cx.dp, cx.couleurs());
        let (hc, gap) = (cx.commande(), cx.ecart());
        let pad = 12.0 * d;
        let w = (z.w - 24.0 * d).min(440.0 * d).max(0.0);
        let carte = Rect::new(z.x + (z.w - w) / 2.0, z.y + 12.0 * d, w, hc + 2.0 * pad + hc + gap);
        cx.ombre(carte, 14.0 * d, 12.0);
        cx.dl.rrect(carte, 14.0 * d, col.surface);
        cx.ui.region_add(sub_id(SAISIE, 1_000 + k), carte, Sense::Sink);
        let marge = (cx.theme.marge * 0.4 * d).round();
        let zone = Rect::new(carte.x + pad - 14.0 * d, carte.y + pad - 8.0 * d - marge, carte.w - 2.0 * pad + 28.0 * d, hc + 8.0 * d + marge + 4.0 * d);
        let mut b = cx.body(sub_id(SAISIE_CORPS, k), zone, &c.scrolls);
        clavier::champ(cx, &mut b, sub_id(SAISIE, k), "Texte à poser", &mut self.saisie, &mut c.clavier);
        cx.body_end(b, &mut c.scrolls);
        let y = carte.y + pad + hc + gap;
        let mut x = carte.right() - pad;
        for (i, (nom, poser)) in [("Poser", true), ("Annuler", false)].into_iter().enumerate() {
            let wb = cx.pill_width(nom);
            x -= wb;
            if cx.pill(sub_id(SAISIE, 2_000 + k * 2 + i), x, y, nom, poser) {
                if poser {
                    self.poser_texte();
                } else {
                    self.texte_en = None;
                }
                c.clavier.fermer();
            }
            x -= gap;
        }
    }
}

impl Contexte for Svg {
    fn cle(&self) -> &'static str {
        CLE
    }

    /// Un nouveau dessin, avec le même style.
    fn dupliquer(&self) -> Box<dyn Contexte> {
        Box::new(Svg { fond: self.fond, trait_: self.trait_, epaisseur: self.epaisseur, ..Svg::default() })
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
        if self.charge {
            e.mettre("vue", format!("{} {} {}", svg::nombre(self.vue.x), svg::nombre(self.vue.y), self.vue.zoom));
        }
        let a = Ajustement::TOUS.iter().position(|a| *a == self.ajustement).unwrap_or(0);
        e.mettre("simplifier", format!("{} {a}", self.tolerance));
    }

    fn retour(&mut self, _c: &Commun) {
        // Le fichier a pu changer : relu s'il n'y a rien de neuf ici
        if self.charge && !self.modifie && self.chemin.is_some() && self.source.is_none() && self.travail.is_none() {
            let vue = self.vue;
            self.charger();
            self.vue = vue;
        }
    }

    fn mettre_a_l_abri(&mut self, c: &Commun) {
        if !self.charge {
            return;
        }
        if !self.modifie && self.source.as_ref().is_none_or(|(ed, d)| ed.contenu() == *d) {
            if let Some(b) = self.brouillon.take() {
                let _ = std::fs::remove_file(b);
            }
            return;
        }
        let nouveau = || {
            let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
            c.dossier.as_ref().map(|d| d.join(BROUILLONS).join(format!("svg-{t}.svg")))
        };
        let Some(chemin) = self.brouillon.clone().or_else(nouveau) else { return };
        match super::texte::ecrire_fichier(&chemin, &self.texte()) {
            Ok(()) => self.brouillon = Some(chemin),
            Err(e) => crate::android::log(&format!("brouillon SVG non écrit : {e}")),
        }
    }

    fn frappe(&mut self, f: FenetreId, cible: Id, frappe: Frappe, c: &mut Commun) -> bool {
        let k = f as usize;
        if cible == sub_id(NOM, k) {
            if self.nom.appliquer(frappe) {
                c.clavier.fermer();
            }
            true
        } else if cible == sub_id(SAISIE, k) {
            if self.saisie.appliquer(frappe) {
                self.poser_texte();
                c.clavier.fermer();
            }
            true
        } else if let Some((ed, _)) = &mut self.source {
            if ed.frappe(f, cible, frappe, c) {
                self.modifie = true;
                return true;
            }
            false
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
        let haut = (cx.commande() + 2.0 * cx.coin()).min(r.h / 3.0);
        let corps = Rect::new(r.x, r.y + haut, r.w, (r.h - haut).max(0.0));

        // La source : l'éditeur de texte, sous l'en-tête
        if let Some((ed, _)) = &mut self.source {
            let mut q = *p;
            q.contenu = corps;
            let etat = ed.dessiner(cx, c, &q);
            self.entete(cx, c, p, Rect::new(r.x, r.y, r.w, haut));
            return format!("Source · {etat}");
        }

        // L'aperçu suit sa sélection
        if self.apercu.as_ref().is_some_and(|a| Some(a.noeud) != self.selection) {
            self.conclure_simplification(false);
        }
        // La barre du bas, puis le dessin au-dessus
        let (hb, _) = self.barre(cx, Rect::new(r.x, 0.0, r.w, 0.0), k, false);
        let hb = hb.min(corps.h * 0.6);
        let z = Rect::new(r.x, corps.y, r.w, (corps.h - hb).max(0.0));
        self.zone = z;
        if std::mem::take(&mut self.cadrer) || (self.vue.zoom <= 0.0) {
            self.cadrer();
        }
        for g in cx.ui.gestes(sub_id(SVG, 30_000 + k), z) {
            self.recevoir(g, c, k);
        }
        self.peindre(cx, z);
        self.superposer(cx, z, k);
        if self.texte_en.is_some() {
            self.saisir_texte(cx, c, z, k);
        }
        if let (_, Some(cmd)) = self.barre(cx, Rect::new(r.x, z.bottom(), r.w, hb), k, true) {
            self.commander(cmd);
        }
        self.entete(cx, c, p, Rect::new(r.x, r.y, r.w, haut));

        if let Some((m, _)) = &self.message {
            return m.clone();
        }
        let n = self.doc.noeuds[self.doc.racine].enfants.iter().filter(|e| self.doc.noeuds[**e].element()).count();
        let sel = match (self.selection, self.outil) {
            (None, Outil::Sommets) => " · touchez une forme pour voir ses sommets".to_string(),
            (Some(_), Outil::Sommets) if self.pointe.is_none() => " · touchez un sommet, ou le contour pour en ajouter".to_string(),
            (Some(s), _) => format!(" · <{}> choisi", self.doc.noeuds[s].nom),
            (None, _) => String::new(),
        };
        let etat = if self.modifie { " · non enregistré" } else { "" };
        format!("{} · {n} élément{} · {:.0} %{sel}{etat}", self.nom_affiche(), if n > 1 { "s" } else { "" }, self.vue.zoom * 100.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dossier(nom: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("flow-svg-{nom}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn pret() -> Svg {
        let mut s = Svg { zone: Rect::new(0.0, 0.0, 800.0, 600.0), dp: 1.0, charge: true, ..Svg::default() };
        s.vue = Vue { x: 0.0, y: 0.0, zoom: 1.0 };
        s
    }

    fn glisser(s: &mut Svg, a: P, b: P) {
        let mut c = Commun::default();
        s.recevoir(Geste::SaisieDebut { x: a.0, y: a.1, doigt: a }, &mut c, 1);
        s.recevoir(Geste::SaisieDeplace { x: (a.0 + b.0) / 2.0, y: (a.1 + b.1) / 2.0, doigt: b }, &mut c, 1);
        s.recevoir(Geste::SaisieFin { x: b.0, y: b.1, depart: a, vecteur: (b.0 - a.0, b.1 - a.1) }, &mut c, 1);
    }

    fn toucher(s: &mut Svg, p: P) {
        let mut c = Commun::default();
        s.recevoir(Geste::Tap { x: p.0, y: p.1, intensite: 0.0, precision: 0.0 }, &mut c, 1);
    }

    #[test]
    fn tracer_choisir_deplacer_annuler() {
        let mut s = pret();
        s.commander(Commande::Outil(Outil::Rectangle));
        glisser(&mut s, (10.0, 20.0), (110.0, 70.0));
        let r = s.selection.unwrap();
        assert_eq!(s.doc.noeuds[r].nom, "rect");
        assert_eq!(s.doc.noeuds[r].attr("width"), Some("100"));
        assert_eq!(s.doc.noeuds[r].attr("fill"), Some("#2563eb"));
        // Choisir : toucher ailleurs ne choisit rien, toucher le rectangle le choisit
        s.commander(Commande::Outil(Outil::Choisir));
        toucher(&mut s, (500.0, 500.0));
        assert_eq!(s.selection, None);
        toucher(&mut s, (50.0, 40.0));
        assert_eq!(s.selection, Some(r));
        // Le déplacer de (30, 5)
        glisser(&mut s, (50.0, 40.0), (80.0, 45.0));
        assert_eq!(s.doc.noeuds[r].attr("transform"), Some("translate(30 5)"));
        assert_eq!(s.passe.len(), 2);
        // Annuler, rétablir
        s.defaire(false);
        assert!(!s.doc.ecrire().contains("translate"));
        s.defaire(true);
        assert!(s.doc.ecrire().contains("translate(30 5)"));
        // Une ellipse et une ligne
        s.commander(Commande::Outil(Outil::Ellipse));
        glisser(&mut s, (200.0, 200.0), (300.0, 260.0));
        let e = s.selection.unwrap();
        assert_eq!((s.doc.noeuds[e].attr("rx"), s.doc.noeuds[e].attr("cy")), (Some("50"), Some("230")));
        s.commander(Commande::Outil(Outil::Ligne));
        glisser(&mut s, (0.0, 0.0), (40.0, 30.0));
        let l = s.selection.unwrap();
        assert_eq!(s.doc.noeuds[l].attr("fill"), Some("none"));
        // Un glissé minuscule ne trace rien
        let n = s.doc.noeuds[s.doc.racine].enfants.len();
        glisser(&mut s, (0.0, 0.0), (1.0, 1.0));
        assert_eq!(s.doc.noeuds[s.doc.racine].enfants.len(), n);
    }

    #[test]
    fn polygone_texte_style_et_actions() {
        let mut s = pret();
        s.commander(Commande::Outil(Outil::Polygone));
        for p in [(0.0, 0.0), (100.0, 0.0), (50.0, 80.0)] {
            toucher(&mut s, p);
        }
        // Toucher le premier sommet ferme
        toucher(&mut s, (3.0, 2.0));
        let g = s.selection.unwrap();
        assert_eq!(s.doc.noeuds[g].attr("points"), Some("0,0 100,0 50,80"));
        assert!(s.sommets.is_empty());
        // Le style s'applique à la sélection
        s.commander(Commande::Cible(false));
        s.commander(Commande::Couleur(Some(COULEURS[3])));
        s.commander(Commande::Epaisseur(8.0));
        assert_eq!(s.doc.noeuds[g].attr("stroke"), Some("#dc2626"));
        assert_eq!(s.doc.noeuds[g].attr("stroke-width"), Some("8"));
        s.commander(Commande::Cible(true));
        s.commander(Commande::Couleur(None));
        assert_eq!(s.doc.noeuds[g].attr("fill"), Some("none"));
        // Dupliquer, descendre, supprimer
        s.commander(Commande::Dupliquer);
        let d = s.selection.unwrap();
        assert_ne!(d, g);
        assert_eq!(s.doc.noeuds[d].attr("transform"), Some("translate(10 10)"));
        s.commander(Commande::Descendre);
        assert_eq!(s.doc.noeuds[s.doc.racine].enfants, vec![d, g]);
        s.commander(Commande::Supprimer);
        assert_eq!(s.doc.noeuds[s.doc.racine].enfants, vec![g]);
        // Un texte
        s.commander(Commande::Outil(Outil::Texte));
        toucher(&mut s, (10.0, 300.0));
        s.saisie = Edition::new("Salut");
        s.poser_texte();
        let t = s.selection.unwrap();
        assert!(s.doc.ecrire().contains(">Salut</text>"));
        assert_eq!(s.doc.noeuds[t].attr("font-size"), Some("20"));
    }

    #[test]
    fn la_source_se_relit_ou_signale_l_erreur() {
        let mut s = pret();
        s.ouvrir_source();
        let t = s.source.as_ref().unwrap().0.contenu();
        assert!(t.starts_with("<svg"));
        // Une source modifiée se relit
        let neuf = t.replace("</svg>", "  <circle cx=\"5\" cy=\"5\" r=\"4\"/>\n</svg>").replace("/>\n</svg>", "/>\n</svg>");
        let neuf = if neuf.contains("<circle") { neuf } else { t.replace("viewBox=\"0 0 800 600\"/>", "viewBox=\"0 0 800 600\"><circle cx=\"5\" cy=\"5\" r=\"4\"/></svg>") };
        s.source.as_mut().unwrap().0.remplacer_texte(&neuf);
        assert!(s.fermer_source());
        assert!(s.source.is_none());
        assert!(s.doc.ecrire().contains("<circle"));
        assert_eq!(s.passe.len(), 1);
        // Une source cassée reste ouverte
        s.ouvrir_source();
        s.source.as_mut().unwrap().0.remplacer_texte("<svg><g></svg");
        assert!(!s.fermer_source());
        assert!(s.source.is_some());
    }

    #[test]
    fn enregistrer_et_relire() {
        let d = dossier("enregistrer");
        let c = Commun { racine: Some(d.clone()), dossier: Some(d.join("Flow")), ..Commun::default() };
        let mut s = pret();
        s.commander(Commande::Outil(Outil::Rectangle));
        glisser(&mut s, (0.0, 0.0), (50.0, 50.0));
        s.nom = Edition::new("plan");
        s.enregistrer(&c).unwrap();
        let f = d.join("Documents/SVG/plan.svg");
        assert!(std::fs::read_to_string(&f).unwrap().contains("<rect"));
        let mut e = Etat::default();
        s.ecrire(&mut e);
        let mut u = Svg { chemin: Some(f.clone()), ..Svg::default() };
        u.charger();
        assert_eq!(u.doc.ecrire(), s.doc.ecrire());
        assert!(!u.modifie);
        // Un fichier illisible s'ouvre sur sa source
        std::fs::write(d.join("casse.svg"), "<svg><rect></svg").unwrap();
        let mut v = Svg { chemin: Some(d.join("casse.svg")), ..Svg::default() };
        v.charger();
        assert!(v.source.is_some());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn simplifier_en_apercu() {
        let mut s = pret();
        s.doc = Document::lire(r##"<svg viewBox="0 0 800 600"><g><path d="M0 0 L50 0 L100 0 L100 0.3 L100 100" fill="none" stroke="#000"/><polyline points="0 200 50 200.05 100 200" fill="none" stroke="#000"/></g><rect width="10" height="10"/></svg>"##).unwrap();
        s.retouche();
        toucher(&mut s, (50.0, 0.0));
        let g = s.selection.unwrap();
        assert_eq!(s.doc.noeuds[g].nom, "g");
        assert!(s.commandes().iter().flatten().any(|c| c.0 == Commande::Simplifier));
        // Sans perte d'abord : seuls les sommets alignés s'en vont
        s.commander(Commande::Simplifier);
        assert_eq!(s.apercu.as_ref().unwrap().compte, (8, 6));
        assert!(s.commandes().iter().flatten().any(|c| c.1 == "Appliquer · de 8 à 6 sommets"));
        // Un écart de 0,5 : le petit crochet de la polyligne s'en va aussi
        s.commander(Commande::Tolerance(3));
        assert_eq!(s.apercu.as_ref().unwrap().compte, (8, 5));
        // Annuler rend l'original, sans rien noter
        s.commander(Commande::Renoncer);
        assert!(s.apercu.is_none() && s.passe.is_empty());
        assert!(s.doc.ecrire().contains("L50 0"));
        // Le réglage est retenu ; appliquer se note et s'annule
        s.commander(Commande::Simplifier);
        assert_eq!(s.apercu.as_ref().unwrap().compte, (8, 5));
        s.commander(Commande::Appliquer);
        assert_eq!(s.passe.len(), 1);
        let e = s.doc.ecrire();
        assert!(e.contains("d=\"M0 0 L100 0 L100 100\"") && e.contains("points=\"0,200 100,200\""), "{e}");
        let mut et = Etat::default();
        s.ecrire(&mut et);
        assert_eq!(et.nombres("simplifier"), vec![3.0, 0.0]);
        s.defaire(false);
        assert!(s.doc.ecrire().contains("L50 0"));
        // Une autre action pendant l'aperçu y renonce
        s.selection = Some(g);
        s.commander(Commande::Simplifier);
        s.commander(Commande::Monter);
        assert!(s.apercu.is_none() && s.doc.ecrire().contains("L50 0"));
    }

    #[test]
    fn editer_les_sommets() {
        let mut s = pret();
        s.doc = Document::lire(r#"<svg viewBox="0 0 800 600"><g transform="translate(100 0)"><rect x="0" y="0" width="100" height="50" fill="red"/></g></svg>"#).unwrap();
        s.retouche();
        s.commander(Commande::Outil(Outil::Sommets));
        // Toucher la forme, dans son groupe, la choisit
        toucher(&mut s, (150.0, 25.0));
        let r = s.selection.unwrap();
        assert_eq!(s.doc.noeuds[r].nom, "rect");
        // Toucher un angle le pointe ; le glisser le déplace (le groupe est décalé de 100)
        toucher(&mut s, (201.0, 49.0));
        assert_eq!(s.pointe, Some(2));
        glisser(&mut s, (200.0, 50.0), (220.0, 80.0));
        assert_eq!(s.doc.noeuds[r].nom, "path");
        assert_eq!(s.doc.noeuds[r].attr("d"), Some("M0 0 L100 0 L120 80 L0 50 Z"));
        assert_eq!(s.doc.noeuds[r].attr("fill"), Some("red"));
        assert_eq!(s.passe.len(), 1);
        // Toucher le contour ajoute un sommet, pointé
        toucher(&mut s, (150.0, 0.0));
        assert_eq!(s.pointe, Some(1));
        assert_eq!(s.doc.noeuds[r].attr("d"), Some("M0 0 L50 0 L100 0 L120 80 L0 50 Z"));
        // Courber le segment qui y arrive, puis retirer ce sommet
        assert!(s.commandes().iter().flatten().any(|c| c.1 == "Courber"));
        s.commander(Commande::Basculer);
        assert!(s.doc.noeuds[r].attr("d").unwrap().starts_with("M0 0 C"));
        s.commander(Commande::RetirerSommet);
        assert_eq!(s.pointe, None);
        assert_eq!(svg::trace_de(&s.doc.noeuds[r]).unwrap().len(), 5);
        // Ouvrir le tracé
        s.pointe = Some(0);
        s.commander(Commande::OuvrirFermer);
        assert!(!s.doc.noeuds[r].attr("d").unwrap().ends_with('Z'));
        // Tout s'annule pas à pas, jusqu'au rectangle
        while !s.passe.is_empty() {
            s.defaire(false);
        }
        let r = s.doc.noeuds.iter().position(|n| n.nom == "rect").unwrap();
        assert_eq!(s.doc.noeuds[r].attr("width"), Some("100"));
    }

}
