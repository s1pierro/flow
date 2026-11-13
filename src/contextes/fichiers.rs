//! Le contexte Fichiers : un explorateur. Chaque fenêtre a son dossier ouvert et son
//! choix d'afficher les fichiers cachés. Un dossier s'ouvre au toucher, un fichier
//! audio se lit dans la piste « Fichier » du mixeur, un fichier texte s'ouvre dans
//! l'éditeur, un SVG dans le contexte SVG : l'un ou l'autre prend la place de l'explorateur
//! dans la fenêtre.
//!
//! Les entrées sont des cartes de 3,5 pour 1 : à gauche l'empreinte d'un dossier (sa
//! place dans la synthèse du stockage) ou le badge du type MIME d'un fichier, à
//! droite le nom, une légende (taille, part du parent ou type) et la date.
//!
//! Sous le fil d'Ariane, une barre d'outils :
//! - **Rechercher** : ne garde que les noms qui contiennent le texte (sans casse ni
//!   accents) ;
//! - **Tri** : Nom, Date (récents d'abord), Taille (gros d'abord), Type ; les
//!   dossiers restent devant ;
//! - **Coller** : quand des fichiers ont été copiés ou coupés, ici ou dans une autre
//!   fenêtre ;
//! - **Nouveau** : un dossier ou un fichier, nommé dans une boîte de dialogue.
//!
//! Un appui long sur une carte la sélectionne : la barre devient celle de la
//! sélection (Tout, Copier, Couper, Renommer pour une seule, Supprimer après
//! confirmation, Fin), et toucher une carte l'ajoute ou la retire. Copier, déplacer
//! et supprimer tournent en arrière-plan, avec leur avancement et « Arrêter »
//! (`crate::operations`).

use super::{abreger, empreinte, hachage, TEINTES};
use crate::contexte::{Commun, Contexte, Demande, Etat, Genre};
use crate::disque;
use crate::draw::Rect;
use crate::clavier::{self, Edition, Frappe};
use crate::explorateur::{self, Dossier, Famille, Nature, Tri};
use crate::operations::{self, Sorte};
use crate::paquets;
use crate::radio;
use crate::surface::{FenetreId, Placee};
use crate::ui::{id, sub_id, Id, Sense};
use crate::widgets::{Cx, Icon};
use std::collections::BTreeSet;
use std::path::PathBuf;

pub const CLE: &str = "fichiers";
pub const GENRE: Genre = Genre { cle: CLE, nom: "Fichiers", legende: "Parcourir le stockage, ouvrir les documents", icone: Icon::Dossier, ouvrir, applet: Some(applet) };

const FICHIERS: Id = id("fichiers");
const RECHERCHE: Id = id("fichiers/recherche");
const RECHERCHE_CORPS: Id = id("fichiers/recherche-corps");
const OUTIL: Id = id("fichiers/outil");
const NOM: Id = id("fichiers/nom");
const NOM_CORPS: Id = id("fichiers/nom-corps");
const DIALOGUE: Id = id("fichiers/dialogue");
/// Le rapport largeur sur hauteur des cartes.
const RAPPORT: f32 = 3.5;
/// Largeur visée d'une carte (dp) : le nombre de colonnes en découle.
const CARTE_LARGE: f32 = 260.0;
/// Hauteur minimale d'une carte (dp), quand la fenêtre est étroite.
const CARTE_MIN: f32 = 56.0;

/// Le badge du type d'un fichier : un carré teinté selon sa famille, son extension
/// au centre.
fn badge(cx: &mut Cx, r: Rect, texte: &str, famille: Famille) {
    let teinte = match famille {
        Famille::Audio => TEINTES[4],
        Famille::Image => TEINTES[2],
        Famille::Video => TEINTES[1],
        Famille::Texte => TEINTES[0],
        Famille::Document => TEINTES[6],
        Famille::Archive => TEINTES[3],
        Famille::Application => TEINTES[5],
        Famille::Inconnue => [0x8A, 0x8F, 0x98, 255],
    };
    cx.dl.rrect(r, r.w * 0.22, teinte);
    // Le coin replié, en haut à droite
    let p = r.w * 0.28;
    cx.dl.rrect(Rect::new(r.right() - p, r.y, p, p), r.w * 0.06, [255, 255, 255, 70]);
    let taille = (r.h / cx.dp / cx.theme.texte * if texte.chars().count() > 3 { 0.24 } else { 0.3 }).clamp(8.0, 20.0);
    cx.centered(Rect::new(r.x, r.y + r.h * 0.12, r.w, r.h * 0.8), texte, taille, [255, 255, 255, 255]);
}

/// Une date de modification : l'heure si c'est aujourd'hui, sinon le jour.
fn date(t: u64, maintenant: u64) -> String {
    let (j, m, a) = crate::android::date_de(t);
    if crate::android::date_de(maintenant) == (j, m, a) {
        let (h, mi) = crate::android::heure_de(t);
        format!("Modifié aujourd'hui à {h:02}:{mi:02}")
    } else {
        format!("Modifié le {j:02}/{m:02}/{a}")
    }
}

/// Une boîte de dialogue ouverte sur la fenêtre.
#[derive(Clone, Debug, PartialEq)]
enum Dialogue {
    /// Nommer un nouveau dossier ou fichier.
    Creer,
    /// Renommer une entrée (son nom actuel).
    Renommer(String),
    /// Confirmer la suppression de ces entrées.
    Supprimer(Vec<String>),
    /// Un `.apk` touché : l'installer, le mettre à jour, ouvrir l'application.
    Application { chemin: PathBuf, apk: Result<paquets::Apk, String> },
}

/// Une commande de la barre d'outils.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Outil {
    Tri,
    Coller,
    Nouveau,
    Tout,
    Copier,
    Couper,
    Renommer,
    Supprimer,
    Fin,
    Arreter,
}

impl Outil {
    fn rang(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Default)]
pub struct Fichiers {
    /// Le dossier à ouvrir au premier dessin (relu du fichier de l'espace).
    chemin: Option<PathBuf>,
    dossier: Option<Dossier>,
    caches: bool,
    tri: Tri,
    /// Le texte cherché dans les noms.
    recherche: Edition,
    /// Les noms des entrées sélectionnées, dans le dossier ouvert.
    selection: BTreeSet<String>,
    dialogue: Option<Dialogue>,
    /// Le nom saisi dans la boîte de dialogue, et ce qui ne va pas.
    nom: Edition,
    erreur: Option<String>,
    /// Entrée a été tapée dans la boîte de dialogue : valider à la prochaine image.
    valider: bool,
    /// Un message pour la ligne d'état, et s'il signale une erreur.
    message: Option<(String, bool)>,
    /// La génération des opérations déjà vue (`Manutention::generation`).
    generation: u64,
}

fn ouvrir(e: &Etat) -> Box<dyn Contexte> {
    Box::new(Fichiers {
        chemin: e.texte("dossier").map(PathBuf::from),
        caches: e.oui("caches").unwrap_or(false),
        tri: e.texte("tri").and_then(Tri::de).unwrap_or_default(),
        ..Fichiers::default()
    })
}

/// Place des largeurs les unes après les autres, à la ligne quand la place manque :
/// pour chacune, son abscisse depuis le bord et son rang de ligne.
pub(super) fn ranger(largeurs: &[f32], place: f32, ecart: f32) -> Vec<(f32, usize)> {
    let (mut x, mut ligne) = (0.0, 0);
    largeurs
        .iter()
        .map(|&w| {
            if x > 0.0 && x + w > place {
                x = 0.0;
                ligne += 1;
            }
            let p = (x, ligne);
            x += w + ecart;
            p
        })
        .collect()
}

impl Fichiers {
    fn aller(&mut self, chemin: PathBuf) {
        if self.chemin.as_ref() != Some(&chemin) {
            self.selection.clear();
            self.recherche = Edition::default();
        }
        let mut d = Dossier::lire(&chemin, self.caches);
        explorateur::trier_par(&mut d.entrees, self.tri);
        // Les entrées disparues quittent la sélection
        self.selection.retain(|n| d.entrees.iter().any(|e| &e.nom == n));
        self.dossier = Some(d);
        self.chemin = Some(chemin);
    }

    fn relire(&mut self) {
        if let Some(c) = self.chemin.clone().filter(|_| self.dossier.is_some()) {
            self.aller(c);
        }
    }

    /// Les chemins des entrées sélectionnées.
    fn choisis(&self) -> Vec<PathBuf> {
        let Some(d) = &self.dossier else { return Vec::new() };
        self.selection.iter().map(|n| d.chemin.join(n)).collect()
    }

    /// Ouvre une boîte de dialogue (et le clavier pour un nom à saisir).
    fn dialoguer(&mut self, d: Dialogue, k: usize, c: &mut Commun) {
        self.nom = match &d {
            Dialogue::Renommer(n) => Edition::new(n),
            _ => Edition::default(),
        };
        self.erreur = None;
        if matches!(d, Dialogue::Creer | Dialogue::Renommer(_)) {
            c.clavier.ouvrir(sub_id(NOM, k));
        }
        self.dialogue = Some(d);
    }

    fn fermer_dialogue(&mut self, k: usize, c: &mut Commun) {
        self.dialogue = None;
        self.erreur = None;
        if c.clavier.cible == Some(sub_id(NOM, k)) {
            c.clavier.fermer();
        }
    }

    /// Valide la boîte de dialogue : `dossier` choisit entre dossier et fichier pour
    /// une création. Vrai si elle peut se fermer.
    fn conclure(&mut self, dossier: bool, c: &mut Commun) -> bool {
        let Some(ici) = self.dossier.as_ref().map(|d| d.chemin.clone()) else { return true };
        let r = match self.dialogue.clone() {
            Some(Dialogue::Creer) => {
                let f = if dossier { operations::creer_dossier } else { operations::creer_fichier };
                f(&ici, &self.nom.texte).map(|p| format!("Créé : {}", p.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned())))
            }
            Some(Dialogue::Renommer(ancien)) => operations::renommer(&ici.join(&ancien), &self.nom.texte).map(|p| {
                self.selection.clear();
                format!("Renommé : {}", p.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned()))
            }),
            Some(Dialogue::Supprimer(noms)) => {
                if c.fichiers.occupe() {
                    Err("Une opération est déjà en cours.".to_string())
                } else {
                    c.fichiers.travail = Some(operations::lancer(Sorte::Supprimer, noms.iter().map(|n| ici.join(n)).collect(), None));
                    self.selection.clear();
                    return true;
                }
            }
            Some(Dialogue::Application { .. }) | None => return true,
        };
        match r {
            Ok(m) => {
                c.fichiers.change();
                self.message = Some((m, false));
                true
            }
            Err(e) => {
                self.erreur = Some(e);
                false
            }
        }
    }

    /// Fait une commande de la barre d'outils.
    fn outil(&mut self, o: Outil, k: usize, c: &mut Commun, visibles: &[String]) {
        self.message = None;
        let n = self.selection.len();
        match o {
            Outil::Tri => {
                self.tri = self.tri.suivant();
                self.relire();
            }
            Outil::Nouveau => self.dialoguer(Dialogue::Creer, k, c),
            Outil::Coller => {
                let Some((chemins, couper)) = c.fichiers.presse.clone() else { return };
                let Some(ici) = self.dossier.as_ref().map(|d| d.chemin.clone()) else { return };
                if c.fichiers.occupe() {
                    self.message = Some(("Une opération est déjà en cours.".into(), true));
                    return;
                }
                c.fichiers.travail = Some(operations::lancer(if couper { Sorte::Deplacer } else { Sorte::Copier }, chemins, Some(ici)));
            }
            Outil::Tout => {
                self.selection.extend(visibles.iter().cloned());
            }
            Outil::Copier | Outil::Couper => {
                c.fichiers.presse = Some((self.choisis(), o == Outil::Couper));
                self.selection.clear();
                let verbe = if o == Outil::Couper { "à déplacer" } else { "à copier" };
                self.message = Some((format!("{n} élément{} {verbe} : ouvrez le dossier voulu, puis Coller", if n > 1 { "s" } else { "" }), false));
            }
            Outil::Renommer => {
                if let Some(nom) = self.selection.iter().next().cloned() {
                    self.dialoguer(Dialogue::Renommer(nom), k, c);
                }
            }
            Outil::Supprimer => {
                let noms: Vec<String> = self.selection.iter().cloned().collect();
                self.dialoguer(Dialogue::Supprimer(noms), k, c);
            }
            Outil::Fin => self.selection.clear(),
            Outil::Arreter => {
                if let Some(t) = &c.fichiers.travail {
                    t.annule.store(true, std::sync::atomic::Ordering::Relaxed);
                }
            }
        }
    }
}

impl Contexte for Fichiers {
    fn cle(&self) -> &'static str {
        CLE
    }

    fn dupliquer(&self) -> Box<dyn Contexte> {
        Box::new(self.clone())
    }

    fn ecrire(&self, e: &mut Etat) {
        if let Some(c) = &self.chemin {
            e.mettre("dossier", c.display());
        }
        e.mettre_oui("caches", self.caches);
        if self.tri != Tri::Nom {
            e.mettre("tri", self.tri.nom());
        }
    }

    fn retour(&mut self, _c: &Commun) {
        // Le dossier a pu changer entre-temps
        self.relire();
    }

    /// Redessiner pendant une opération, jusqu'à son bilan.
    fn anime(&self, c: &Commun) -> bool {
        // Une installation se suit sans attendre d'événement
        c.fichiers.travail.is_some() || matches!(self.dialogue, Some(Dialogue::Application { .. }))
    }

    fn frappe(&mut self, f: FenetreId, cible: Id, frappe: Frappe, c: &mut Commun) -> bool {
        let k = f as usize;
        if cible == sub_id(RECHERCHE, k) {
            if self.recherche.appliquer(frappe) {
                c.clavier.fermer();
            }
            true
        } else if cible == sub_id(NOM, k) {
            if self.nom.appliquer(frappe) {
                self.valider = true;
            }
            self.erreur = None;
            true
        } else {
            false
        }
    }

    fn dessiner(&mut self, cx: &mut Cx, c: &mut Commun, p: &Placee) -> String {
        let r = p.contenu;
        let (d, col) = (cx.dp, cx.couleurs());
        cx.dl.rrect(r, 0.0, col.surface);
        let k = p.id as usize;
        if self.dossier.is_none() {
            let depart = self.chemin.clone().or_else(|| c.racine.clone()).unwrap_or_else(|| PathBuf::from("/"));
            self.aller(depart);
            self.generation = c.fichiers.generation;
        }
        // Une opération finie : son bilan ; toutes les fenêtres relisent leur dossier
        if let Some(bilan) = c.fichiers.relever() {
            self.message = Some(bilan);
        }
        if self.generation != c.fichiers.generation {
            self.generation = c.fichiers.generation;
            self.relire();
        }
        if std::mem::take(&mut self.valider) && self.dialogue.is_some() {
            let dossier = !self.nom.texte.contains('.');
            if self.conclure(dossier, c) {
                self.fermer_dialogue(k, c);
            }
        }
        let dossier = self.dossier.clone().unwrap_or_default();
        let base = sub_id(FICHIERS, k);
        // Les empreintes viennent de la synthèse de l'analyse du stockage, enregistrée
        // sur le disque et refaite au plus une fois par jour
        let synthese = c.empreintes();
        let situation = synthese.as_ref().and_then(|s| {
            let rel = dossier.chemin.strip_prefix(&s.racine).ok()?;
            let noms: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
            disque::situer(&s.arbre, &noms)
        });
        // Un défilement par dossier : en revenant au parent, on retrouve sa position
        let scroll = sub_id(base, hachage(&dossier.chemin));
        let lecture = radio::lecteur_fichiers().adresse.clone();
        let ligne = (cx.theme.ligne * d).round();
        let mut aller: Option<PathBuf> = None;
        // Le fil d'Ariane, fixe en haut de la fenêtre : chaque étape s'ouvre au toucher
        // (c'est par lui qu'on remonte) ; trop long, on voit la fin
        let pad = 14.0 * d;
        let bande = Rect::new(r.x, r.y, r.w, (ligne + 8.0 * d).min(r.h));
        cx.ui.region_add(sub_id(base, 999_997), bande, crate::ui::Sense::Sink);
        let etapes = explorateur::ariane(&dossier.chemin, c.racine.as_deref());
        let sep = "  ›  ";
        let largeur: f32 = etapes.iter().map(|(n, _)| cx.measure(n, 15.0)).sum::<f32>() + cx.measure(sep, 15.0) * (etapes.len() as f32 - 1.0);
        let x0 = bande.x + pad;
        let place = (bande.right() - pad - x0).max(0.0);
        let mut x = x0 + (place - largeur).min(0.0);
        let cy = bande.y + 4.0 * d + ligne / 2.0;
        cx.clip(Some(Rect::new(x0, bande.y, place, bande.h)));
        for (i, (nom, chemin)) in etapes.iter().enumerate() {
            let w = cx.measure(nom, 15.0);
            let zone = Rect::new(x - 4.0 * d, bande.y, w + 8.0 * d, bande.h);
            let derniere = i + 1 == etapes.len();
            let k = sub_id(base, 1_000_000 + i);
            if cx.ui.click(k, zone, None) && !derniere {
                aller = Some(chemin.clone());
            }
            if cx.ui.pressed(k) && !derniere {
                cx.dl.rrect(zone.inset(4.0 * d), 6.0 * d, col.puce);
            }
            cx.text_at(x, cy, nom, 15.0, if derniere { col.texte } else { col.accent });
            x += w;
            if !derniere {
                cx.text_at(x, cy, sep, 15.0, col.pale);
                x += cx.measure(sep, 15.0);
            }
        }
        cx.clip(None);
        cx.dl.rrect(Rect::new(bande.x, bande.bottom() - d.max(1.0), bande.w, d.max(1.0)), 0.0, col.ligne);

        // La barre d'outils : la recherche et les commandes, ou celles de la sélection,
        // puis l'avancement d'une opération
        let visibles: Vec<String> = dossier.entrees.iter().filter(|e| explorateur::correspond(&e.nom, &self.recherche.texte)).map(|e| e.nom.clone()).collect();
        let (hc, gap) = (cx.commande(), cx.ecart());
        let occupe = c.fichiers.occupe();
        let mut outils: Vec<(Outil, String)> = Vec::new();
        let selection = !self.selection.is_empty();
        if selection {
            outils.push((Outil::Tout, "Tout".into()));
            outils.push((Outil::Copier, "Copier".into()));
            outils.push((Outil::Couper, "Couper".into()));
            if self.selection.len() == 1 {
                outils.push((Outil::Renommer, "Renommer".into()));
            }
            if !occupe {
                outils.push((Outil::Supprimer, "Supprimer".into()));
            }
            outils.push((Outil::Fin, "Fin".into()));
        } else {
            outils.push((Outil::Tri, format!("Tri : {}", self.tri.nom())));
            if let Some((l, couper)) = c.fichiers.presse.as_ref().filter(|_| !occupe) {
                outils.push((Outil::Coller, format!("{} ({})", if *couper { "Déplacer ici" } else { "Coller" }, l.len())));
            }
            outils.push((Outil::Nouveau, "Nouveau".into()));
        }
        let largeurs: Vec<f32> = outils.iter().map(|(_, n)| cx.pill_width(n)).collect();
        let place = (r.w - 2.0 * pad).max(0.0);
        let total = largeurs.iter().sum::<f32>() + gap * largeurs.len() as f32;
        // La recherche (ou le compte de la sélection) partage la première ligne si
        // elle garde 160 dp, sinon elle a la sienne
        let tete = if selection { cx.measure(&format!("{} sélectionné{}", self.selection.len(), if self.selection.len() > 1 { "s" } else { "" }), 15.0) + gap } else { 0.0 };
        let seule = !selection && place - total < 160.0 * d;
        let flex = if selection { tete } else if seule { 0.0 } else { place - total + gap };
        // Sur sa ligne, la recherche (ou le compte) d'abord, puis les commandes
        let mut largeurs_rang = if seule { Vec::new() } else { vec![flex - gap] };
        largeurs_rang.extend(largeurs.iter().copied());
        let rangs = ranger(&largeurs_rang, place, gap);
        let premier = !seule as usize;
        let lignes = rangs.iter().map(|r| r.1).max().unwrap_or(0) + 1 + seule as usize;
        let haut_outils = lignes as f32 * (hc + gap) + gap;
        let haut_travail = if c.fichiers.travail.is_some() { hc + gap } else { 0.0 };
        let barre = Rect::new(r.x, bande.bottom(), r.w, (haut_outils + haut_travail).min((r.h - bande.h).max(0.0)));
        cx.ui.region_add(sub_id(base, 999_996), barre, Sense::Sink);
        let y0 = barre.y + gap;
        let dec = if seule { hc + gap } else { 0.0 };
        if selection {
            let n = self.selection.len();
            cx.text_at(r.x + pad, y0 + hc / 2.0, &format!("{n} sélectionné{}", if n > 1 { "s" } else { "" }), 15.0, col.texte);
        } else {
            // Le champ de recherche, dans son propre petit corps
            let w = if seule { place } else { flex - gap };
            let marge = (cx.theme.marge * 0.4 * d).round();
            let zone = Rect::new(r.x + pad - 14.0 * d, y0 - 8.0 * d - marge, w + 28.0 * d, hc + 8.0 * d + marge + 4.0 * d);
            let corps = sub_id(RECHERCHE_CORPS, k);
            let mut bc = cx.body(corps, zone, &c.scrolls);
            clavier::champ(cx, &mut bc, sub_id(RECHERCHE, k), "Rechercher", &mut self.recherche, &mut c.clavier);
            cx.body_end(bc, &mut c.scrolls);
        }
        let mut choisi = None;
        for (i, (o, nom)) in outils.iter().enumerate() {
            let (x, l) = rangs[i + premier];
            if cx.pill(sub_id(OUTIL, k * 16 + o.rang()), r.x + pad + x, y0 + dec + l as f32 * (hc + gap), nom, *o == Outil::Coller) {
                choisi = Some(*o);
            }
        }
        // L'avancement d'une opération, et « Arrêter »
        if let Some(t) = c.fichiers.travail.clone() {
            let y = y0 + lignes as f32 * (hc + gap);
            let arret = "Arrêter";
            let wa = cx.pill_width(arret);
            let xa = r.right() - pad - wa;
            if !t.annule.load(std::sync::atomic::Ordering::Relaxed) && cx.pill(sub_id(OUTIL, k * 16 + Outil::Arreter.rang()), xa, y, arret, false) {
                choisi = Some(Outil::Arreter);
            }
            let w = (xa - gap - r.x - pad).max(0.0);
            cx.clip(Some(Rect::new(r.x + pad, y, w, hc)));
            cx.text_at(r.x + pad, y + hc * 0.35, &t.libelle(), 13.0, col.texte);
            cx.clip(None);
            let jauge = Rect::new(r.x + pad, y + hc * 0.68, w, 4.0 * d);
            cx.dl.rrect(jauge, 2.0 * d, col.ligne);
            cx.dl.rrect(Rect::new(jauge.x, jauge.y, jauge.w * t.part(), jauge.h), 2.0 * d, col.accent);
        }
        cx.dl.rrect(Rect::new(barre.x, barre.bottom() - d.max(1.0), barre.w, d.max(1.0)), 0.0, col.ligne);
        if let Some(o) = choisi {
            self.outil(o, k, c, &visibles);
        }

        // Sous la barre, les entrées défilent
        let corps = Rect::new(r.x, barre.bottom(), r.w, (r.bottom() - barre.bottom()).max(0.0));
        let mut b = cx.body(scroll, corps, &c.scrolls);
        b.y -= 8.0 * d;
        if let Some(e) = &dossier.erreur {
            let aide = if e == "Accès refusé" { format!("{e}. Pour parcourir tout le stockage, accordez à Flow l'accès à tous les fichiers dans les réglages d'Android.") } else { format!("{e}.") };
            cx.paragraph(&mut b, &aide);
        } else if dossier.entrees.is_empty() {
            cx.paragraph(&mut b, "Dossier vide.");
        } else if visibles.is_empty() {
            cx.paragraph(&mut b, &format!("Aucun nom ne contient « {} ».", self.recherche.texte.trim()));
        }
        // Les entrées, en cartes de 3,5 pour 1, sur autant de colonnes que la largeur
        // le permet
        let ecart = cx.ecart().max(8.0 * d);
        b.y += ecart;
        let colonnes = (((b.w + ecart) / (CARTE_LARGE * d + ecart)).floor() as usize).max(1);
        let large = (b.w - ecart * (colonnes - 1) as f32) / colonnes as f32;
        let haute = (large / RAPPORT).max(CARTE_MIN * d * cx.theme.texte).round();
        let vue = b.rect();
        let maintenant = disque::maintenant();
        let entrees: Vec<&explorateur::Entree> = dossier.entrees.iter().filter(|e| visibles.contains(&e.nom)).collect();
        let mut basculer: Option<String> = None;
        for (i, e) in entrees.iter().enumerate() {
            let carte = Rect::new(b.x + (i % colonnes) as f32 * (large + ecart), b.y + (i / colonnes) as f32 * (haute + ecart), large, haute);
            if carte.bottom() < vue.y || carte.y > vue.bottom() {
                continue;
            }
            let chemin = dossier.chemin.join(&e.nom);
            let joue = lecture.as_deref() == chemin.to_str();
            // Un identifiant par nom : une carte garde le sien quand le filtre change
            let kc = sub_id(base, hachage(&chemin));
            let (clic, long) = cx.ui.click_long(kc, carte, Some(b.id()));
            let appui = cx.ui.pressed(kc);
            let choisie = self.selection.contains(&e.nom);
            let rayon = cx.theme.rayon(carte.h).min(14.0 * d);
            let bord = if choisie { (2.0 * d).max(1.0) } else { d.max(1.0) };
            cx.dl.rrect(carte, rayon, if appui || joue || choisie { col.accent } else { col.ligne });
            cx.dl.rrect(carte.inset(bord), rayon, if appui || choisie { col.puce } else { col.fond });
            if choisie {
                // La coche, en haut à droite
                let rc = (9.0 * d).round();
                let (cxc, cyc) = (carte.right() - rc - 6.0 * d, carte.y + rc + 6.0 * d);
                cx.dl.rrect(Rect::new(cxc - rc, cyc - rc, 2.0 * rc, 2.0 * rc), rc, col.accent);
                let u = rc / 9.0;
                cx.dl.line([cxc - 4.0 * u, cyc], [cxc - 1.0 * u, cyc + 3.5 * u], 1.8 * u, col.sur_accent);
                cx.dl.line([cxc - 1.0 * u, cyc + 3.5 * u], [cxc + 4.5 * u, cyc - 3.5 * u], 1.8 * u, col.sur_accent);
            }
            // À gauche, un carré : l'empreinte du dossier, ou le badge du type du fichier
            let pad = (carte.h * 0.16).clamp(6.0 * d, 14.0 * d);
            let cote = carte.h - 2.0 * pad;
            let carre = Rect::new(carte.x + pad, carte.y + pad, cote, cote);
            let legende = if e.dossier() {
                let sous = situation.as_ref().and_then(|p| Some((p.enfant(&e.nom)?, p.noeud.enfants.iter().find(|x| x.nom == e.nom)?.taille, p.noeud.taille)));
                match sous {
                    Some(((arcs, teinte), t, parent)) => {
                        empreinte(cx, carre, &arcs, TEINTES[teinte % TEINTES.len()]);
                        let part = if parent > 0 { t as f64 / parent as f64 * 100.0 } else { 0.0 };
                        let part = if part >= 10.0 { format!("{part:.0} %") } else { format!("{part:.1} %").replace('.', ",") };
                        format!("Dossier · {} · {part} du parent", explorateur::taille(t))
                    }
                    None => {
                        cx.dl.rrect(carre, cote * 0.22, col.puce);
                        cx.icon(carre.inset(cote * 0.2), Icon::Dossier, col.accent);
                        // Trop petit pour la synthèse, ou pas encore analysé
                        if synthese.is_some() { "Dossier · presque vide".to_string() } else { "Dossier".to_string() }
                    }
                }
            } else {
                let (type_mime, famille) = explorateur::mime(&e.nom);
                badge(cx, carre, &explorateur::badge(&e.nom), famille);
                format!("{type_mime} · {}", explorateur::taille(e.taille))
            };
            let date = e.modifie.and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok()).map(|t| date(t.as_secs(), maintenant));
            // À droite : le titre, la légende, la date
            let x = carre.right() + pad;
            let w = (carte.right() - pad - x).max(0.0);
            cx.clip(Some(Rect::new(x, carte.y, w, carte.h).intersect(&vue)));
            let t = cx.theme.texte;
            let lignes = if date.is_some() && carte.h >= 64.0 * d * t { 3.0 } else { 2.0 };
            let y0 = carte.cy() - (20.0 + 16.0 * (lignes - 1.0)) * d * t / 2.0;
            let titre = abreger(cx, &e.nom, 15.0, w);
            cx.text_at(x, y0 + 10.0 * d * t, &titre, 15.0, if joue { col.accent } else { col.texte });
            let legende = abreger(cx, &legende, 12.0, w);
            cx.text_at(x, y0 + 28.0 * d * t, &legende, 12.0, col.pale);
            if let (Some(date), 3.0) = (&date, lignes) {
                cx.text_at(x, y0 + 44.0 * d * t, date, 12.0, col.pale);
            }
            cx.clip(Some(vue));
            if long || (clic && selection) {
                basculer = Some(e.nom.clone());
            } else if clic {
                self.message = None;
                match e.nature {
                    Nature::Dossier => aller = Some(chemin),
                    Nature::Audio if joue => radio::lecteur_fichiers().arreter(),
                    Nature::Audio => radio::lecteur_fichiers().jouer(&chemin.to_string_lossy()),
                    Nature::Texte => {
                        let mut etat = Etat::default();
                        etat.mettre("document", chemin.display());
                        c.demandes.push(Demande::Ouvrir { depuis: p.id, cle: super::texte::CLE, etat });
                    }
                    Nature::Image if chemin.extension().is_some_and(|x| x.eq_ignore_ascii_case("svg")) => {
                        let mut etat = Etat::default();
                        etat.mettre("document", chemin.display());
                        c.demandes.push(Demande::Ouvrir { depuis: p.id, cle: super::svg::CLE, etat });
                    }
                    _ if chemin.extension().is_some_and(|x| x.eq_ignore_ascii_case("apk")) => {
                        // Une application : ce qu'elle est, et l'installer ou l'ouvrir
                        paquets::oublier();
                        let apk = paquets::lire(&chemin);
                        self.dialoguer(Dialogue::Application { chemin, apk }, k, c);
                    }
                    _ if chemin.extension().is_some_and(|x| x.eq_ignore_ascii_case("glb")) => {
                        let mut etat = Etat::default();
                        etat.mettre("document", chemin.display());
                        c.demandes.push(Demande::Ouvrir { depuis: p.id, cle: super::mailles::CLE, etat });
                    }
                    _ => {}
                }
            }
        }
        b.y += entrees.len().div_ceil(colonnes) as f32 * (haute + ecart);
        b.y += cx.ecart();
        let mut caches = self.caches;
        cx.toggle(&mut b, sub_id(base, 999_998), "Fichiers cachés", &mut caches);
        cx.body_end(b, &mut c.scrolls);
        if caches != self.caches {
            self.caches = caches;
            aller = Some(dossier.chemin.clone());
        }
        if let Some(nom) = basculer {
            if !self.selection.remove(&nom) {
                self.selection.insert(nom);
            }
            self.message = None;
        }
        if let Some(ch) = aller {
            self.message = None;
            self.aller(ch);
        }

        // La boîte de dialogue, par-dessus la fenêtre
        if let Some(dl) = self.dialogue.clone() {
            self.dessiner_dialogue(cx, c, p, &dl);
        }

        if let Some((m, _)) = &self.message {
            return m.clone();
        }
        let (nd, nf) = dossier.compte();
        let pluriel = |n: usize, mot: &str| format!("{n} {mot}{}", if n > 1 { "s" } else { "" });
        let filtre = if visibles.len() < dossier.entrees.len() { format!(" · {} affichés", visibles.len()) } else { String::new() };
        format!("{} · {}{filtre}", pluriel(nd, "dossier"), pluriel(nf, "fichier"))
    }
}

impl Fichiers {
    /// Le contenu de la boîte d'une application : son nom, ses versions, où en est
    /// l'installation, et les boutons (0 fermer, 1 installer, 2 ouvrir, 3 autoriser).
    fn dialogue_application(&mut self, chemin: &std::path::Path, apk: &Result<paquets::Apk, String>) -> (String, Vec<String>, Vec<(&'static str, u8)>) {
        let a = match apk {
            Ok(a) => a,
            Err(e) => return ("Application".into(), vec![e.clone()], vec![("Fermer", 0)]),
        };
        let suivi = paquets::suivi();
        // Installée : on relit l'application, pour sa nouvelle version
        if suivi == paquets::Suivi::Reussie && a.installee.as_ref().map(|i| i.1) != Some(a.code) {
            if let Ok(nouveau) = paquets::lire(chemin) {
                self.dialogue = Some(Dialogue::Application { chemin: chemin.to_path_buf(), apk: Ok(nouveau) });
            }
        }
        let mut q = vec![a.paquet.clone(), format!("Ce fichier : version {} ({})", a.version, a.code)];
        q.push(match &a.installee {
            Some((v, c)) => format!("Installée : version {v} ({c})"),
            None => "Pas encore installée".into(),
        });
        let action = a.action();
        let autorisee = paquets::autorisee();
        match &suivi {
            paquets::Suivi::Copie(f) => q.push(format!("Copie vers l'installateur… {:.0} %", f * 100.0)),
            paquets::Suivi::Confirmation => q.push("Confirmez dans la fenêtre d'Android.".into()),
            paquets::Suivi::Reussie => q.push("Installation réussie.".into()),
            paquets::Suivi::Annulee => q.push("Installation annulée.".into()),
            paquets::Suivi::Echec(m) => self.erreur = Some(if m.is_empty() { "L'installation a échoué.".into() } else { format!("Échec : {m}") }),
            paquets::Suivi::Rien if action == paquets::Action::PlusAncienne => q.push("Plus ancienne que l'installée : Android la refusera.".into()),
            paquets::Suivi::Rien if !autorisee => q.push("Autorisez Flow à installer des applications.".into()),
            paquets::Suivi::Rien => {}
        }
        let mut b = vec![("Fermer", 0)];
        if a.installee.is_some() && !suivi.en_cours() {
            b.push(("Ouvrir", 2));
        }
        if !suivi.en_cours() && suivi != paquets::Suivi::Reussie {
            match action.bouton() {
                Some(_) if !autorisee => b.push(("Autoriser", 3)),
                Some(nom) => b.push((nom, 1)),
                None => {}
            }
        }
        (a.nom.clone(), q, b)
    }

    /// La boîte de dialogue : un voile sur la fenêtre, une carte en haut (le clavier
    /// prend le bas), le nom à saisir ou la question, et les boutons.
    fn dessiner_dialogue(&mut self, cx: &mut Cx, c: &mut Commun, p: &Placee, dl: &Dialogue) {
        let (d, col) = (cx.dp, cx.couleurs());
        let k = p.id as usize;
        let r = p.contenu;
        let base = sub_id(DIALOGUE, k);
        cx.dl.rrect(r, 0.0, [0, 0, 0, 110]);
        cx.ui.region_add(sub_id(base, 0), r, Sense::Sink);
        let (hc, gap, pad) = (cx.commande(), cx.ecart(), 18.0 * d);
        let w = (r.w - 2.0 * 12.0 * d).min(440.0 * d).max(0.0);
        let x = r.x + (r.w - w) / 2.0;
        let (titre, question, boutons): (String, Vec<String>, Vec<(&str, u8)>) = match dl {
            Dialogue::Application { chemin, apk } => self.dialogue_application(chemin, apk),
            Dialogue::Creer => ("Nouveau".into(), Vec::new(), vec![("Annuler", 0), ("Fichier", 1), ("Dossier", 2)]),
            Dialogue::Renommer(n) => (format!("Renommer « {} »", n), Vec::new(), vec![("Annuler", 0), ("Renommer", 1)]),
            Dialogue::Supprimer(noms) => {
                let n = noms.len();
                let dossiers = self.dossier.as_ref().map_or(0, |dd| dd.entrees.iter().filter(|e| e.dossier() && noms.contains(&e.nom)).count());
                let mut q = vec![if n == 1 { format!("« {} »", noms[0]) } else { format!("{n} éléments") }];
                if dossiers > 0 {
                    q.push(format!("dont {dossiers} dossier{} et tout son contenu", if dossiers > 1 { "s" } else { "" }).replace("tout son", if dossiers > 1 { "tout leur" } else { "tout son" }));
                }
                q.push("La suppression est définitive.".into());
                ("Supprimer ?".into(), q, vec![("Annuler", 0), ("Supprimer", 1)])
            }
        };
        let saisie = matches!(dl, Dialogue::Creer | Dialogue::Renommer(_));
        let lh = 20.0 * d * cx.theme.texte;
        let h = pad + 26.0 * d * cx.theme.texte + if saisie { hc + 2.0 * gap } else { 0.0 } + question.len() as f32 * lh + if self.erreur.is_some() { lh } else { 0.0 } + gap + hc + pad;
        let carte = Rect::new(x, r.y + (r.h * 0.12).min(60.0 * d), w, h);
        let rayon = cx.theme.rayon(hc).min(18.0 * d);
        cx.ombre(carte, rayon, 14.0);
        cx.dl.rrect(carte, rayon, col.surface);
        cx.ui.region_add(sub_id(base, 1), carte, Sense::Sink);
        let mut y = carte.y + pad;
        cx.clip(Some(carte));
        cx.text_at(carte.x + pad, y + 11.0 * d * cx.theme.texte, &titre, 18.0, col.texte);
        y += 26.0 * d * cx.theme.texte;
        if saisie {
            let marge = (cx.theme.marge * 0.4 * d).round();
            let zone = Rect::new(carte.x + pad - 14.0 * d, y + gap - 8.0 * d - marge, carte.w - 2.0 * pad + 28.0 * d, hc + 8.0 * d + marge + 4.0 * d);
            let corps = sub_id(NOM_CORPS, k);
            let mut b = cx.body(corps, zone, &c.scrolls);
            clavier::champ(cx, &mut b, sub_id(NOM, k), "Nom", &mut self.nom, &mut c.clavier);
            cx.body_end(b, &mut c.scrolls);
            cx.clip(Some(carte));
            y += hc + 2.0 * gap;
        }
        for q in &question {
            cx.text_at(carte.x + pad, y + lh / 2.0, q, 14.0, col.pale);
            y += lh;
        }
        if let Some(e) = &self.erreur {
            cx.text_at(carte.x + pad, y + lh / 2.0, e, 14.0, [0xE0, 0x3A, 0x3A, 255]);
            y += lh;
        }
        y += gap;
        // Les boutons, à droite ; le dernier est l'action
        let mut xb = carte.right() - pad;
        let mut choisi = None;
        for (i, (nom, code)) in boutons.iter().enumerate().rev() {
            let wb = cx.pill_width(nom);
            xb -= wb;
            if cx.pill(sub_id(base, 10 + i), xb, y, nom, i + 1 == boutons.len()) {
                choisi = Some(*code);
            }
            xb -= gap;
        }
        cx.clip(None);
        match (choisi, dl) {
            (Some(0), _) => self.fermer_dialogue(k, c),
            (Some(code), Dialogue::Application { chemin, apk }) => {
                self.erreur = None;
                match (code, apk) {
                    (1, _) => {
                        if let Err(e) = paquets::installer(chemin) {
                            self.erreur = Some(e);
                        }
                    }
                    (2, Ok(a)) => {
                        if paquets::lancer(&a.paquet) {
                            self.fermer_dialogue(k, c);
                        } else {
                            self.erreur = Some("Cette application n'a pas d'écran à ouvrir.".into());
                        }
                    }
                    (3, _) => paquets::autoriser(),
                    _ => {}
                }
            }
            (Some(code), _) => {
                // Créer : 1 fichier, 2 dossier ; sinon l'action
                if self.conclure(code == 2, c) {
                    self.fermer_dialogue(k, c);
                }
            }
            (None, _) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_boite_d_une_application_hors_d_android() {
        let mut f = Fichiers::default();
        let apk = paquets::lire(std::path::Path::new("/x/flow.apk"));
        let (titre, q, b) = f.dialogue_application(std::path::Path::new("/x/flow.apk"), &apk);
        assert_eq!((titre.as_str(), b), ("Application", vec![("Fermer", 0)]));
        assert_eq!(q.len(), 1, "le message d'erreur seul");
    }

    #[test]
    fn les_commandes_se_rangent_en_lignes() {
        assert_eq!(ranger(&[50.0, 50.0, 50.0], 120.0, 10.0), vec![(0.0, 0), (60.0, 0), (0.0, 1)]);
        // Une commande trop large reste seule sur sa ligne
        assert_eq!(ranger(&[200.0, 20.0], 100.0, 10.0), vec![(0.0, 0), (0.0, 1)]);
    }

    #[test]
    fn creer_renommer_selectionner_et_supprimer() {
        let d = std::env::temp_dir().join(format!("flow-fichiers-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("b.txt"), "bb").unwrap();
        let mut c = Commun::default();
        let mut f = Fichiers::default();
        f.aller(d.clone());
        // Créer un dossier
        f.dialoguer(Dialogue::Creer, 1, &mut c);
        f.nom = Edition::new("Album");
        assert!(f.conclure(true, &mut c));
        assert!(d.join("Album").is_dir());
        assert_eq!(c.fichiers.generation, 1);
        // Un nom pris : l'erreur reste dans la boîte
        f.relire();
        f.nom = Edition::new("Album");
        assert!(!f.conclure(false, &mut c));
        assert!(f.erreur.as_deref().unwrap().contains("existe déjà"));
        // Renommer
        f.dialoguer(Dialogue::Renommer("b.txt".into()), 1, &mut c);
        f.nom = Edition::new("c.txt");
        assert!(f.conclure(false, &mut c));
        assert!(d.join("c.txt").exists());
        // Sélectionner et copier, puis coller dans Album
        f.relire();
        f.selection.insert("c.txt".into());
        let tous: Vec<String> = f.dossier.as_ref().unwrap().entrees.iter().map(|e| e.nom.clone()).collect();
        f.outil(Outil::Copier, 1, &mut c, &tous);
        assert!(f.selection.is_empty());
        assert_eq!(c.fichiers.presse.as_ref().unwrap().0, vec![d.join("c.txt")]);
        f.aller(d.join("Album"));
        f.outil(Outil::Coller, 1, &mut c, &tous);
        while c.fichiers.occupe() {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(c.fichiers.relever().unwrap().0, "1 élément copié");
        assert_eq!(std::fs::read_to_string(d.join("Album/c.txt")).unwrap(), "bb");
        // Tout, puis supprimer
        f.aller(d.clone());
        f.outil(Outil::Tout, 1, &mut c, &tous);
        assert_eq!(f.selection.len(), 2);
        f.outil(Outil::Supprimer, 1, &mut c, &tous);
        assert!(matches!(&f.dialogue, Some(Dialogue::Supprimer(n)) if n.len() == 2));
        assert!(f.conclure(false, &mut c));
        while c.fichiers.occupe() {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        c.fichiers.relever();
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 0);
        // Le tri se retient dans l'espace
        f.outil(Outil::Tri, 1, &mut c, &tous);
        let mut e = Etat::default();
        f.ecrire(&mut e);
        assert_eq!(e.texte("tri"), Some("Date"));
        let _ = std::fs::remove_dir_all(d);
    }
}

// --- L'applet de la barre -------------------------------------------------------

/// L'applet Fichiers : il n'apparaît que pendant une copie, un déplacement ou une
/// suppression ; dans la barre, l'avancement en pourcentage ; dans le volet, le détail
/// et le bouton Arrêter.
struct AppletFichiers;

fn applet() -> Box<dyn crate::applet::Applet> {
    Box::new(AppletFichiers)
}

impl crate::applet::Applet for AppletFichiers {
    fn largeur(&mut self, cx: &mut Cx, c: &Commun, h: f32, compact: bool) -> f32 {
        if c.fichiers.occupe() {
            h * 0.8 + if compact { 0.0 } else { cx.measure("100 %", 13.0) + cx.pad() * 0.5 }
        } else {
            0.0
        }
    }

    fn barre(&mut self, cx: &mut Cx, c: &mut Commun, r: Rect, _id: Id, compact: bool) {
        let (d, col) = (cx.dp, cx.couleurs());
        let Some(t) = c.fichiers.travail.clone() else { return };
        let ic = (r.h * 0.5).round();
        cx.icon(Rect::new(r.x + (r.h * 0.8 - ic) / 2.0, r.cy() - ic / 2.0, ic, ic), Icon::Dossier, col.texte);
        if !compact {
            cx.text_at(r.x + r.h * 0.8, r.cy(), &format!("{:.0} %", t.part() * 100.0), 13.0, col.texte);
        }
        // L'avancement, en filet sous l'applet
        let e = 3.0 * d;
        let fond = Rect::new(r.x + 4.0 * d, r.bottom() - e - 4.0 * d, r.w - 8.0 * d, e);
        cx.dl.rrect(fond, e / 2.0, col.ligne);
        cx.dl.rrect(Rect::new(fond.x, fond.y, (fond.w * t.part()).max(e), e), e / 2.0, col.accent);
    }

    fn hauteur_volet(&mut self, cx: &mut Cx, _c: &Commun, _w: f32) -> f32 {
        let d = cx.dp;
        let section = (20.0 + 18.0 * cx.theme.texte) * d;
        8.0 * d + section + 22.0 * d * cx.theme.texte + 8.0 * d + 16.0 * d + 2.0 * (cx.commande() + 2.0 * (cx.theme.marge * d).round()) + 20.0 * d
    }

    fn volet(&mut self, cx: &mut Cx, c: &mut Commun, r: Rect, id: Id) -> bool {
        let Some(t) = c.fichiers.travail.clone() else { return true };
        let (d, col) = (cx.dp, cx.couleurs());
        let scroll = sub_id(id, 0);
        let mut b = cx.body(scroll, r, &c.scrolls);
        cx.section(&mut b, "OPÉRATION EN COURS");
        cx.paragraph(&mut b, &t.libelle());
        let e = 6.0 * d;
        let fond = Rect::new(b.x, b.y, b.w, e);
        cx.dl.rrect(fond, e / 2.0, col.ligne);
        cx.dl.rrect(Rect::new(fond.x, fond.y, (fond.w * t.part()).max(e), e), e / 2.0, col.accent);
        b.y += e + 10.0 * d;
        let annulee = t.annule.load(std::sync::atomic::Ordering::Relaxed);
        if cx.button(&mut b, sub_id(scroll, 1), if annulee { "Arrêt demandé…" } else { "Arrêter" }) {
            t.annule.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        let ouvrir = cx.button(&mut b, sub_id(scroll, 2), "Ouvrir les Fichiers");
        if ouvrir {
            c.demandes.push(crate::contexte::Demande::Montrer { cle: CLE });
        }
        cx.body_end(b, &mut c.scrolls);
        ouvrir
    }

    fn anime(&self, c: &Commun) -> bool {
        c.fichiers.occupe()
    }
}
