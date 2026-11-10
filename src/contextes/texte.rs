//! Le contexte Texte : un éditeur de texte, un document par fenêtre.
//!
//! - Il s'ouvre vide depuis le menu des fenêtres, ou avec un fichier quand on touche
//!   un fichier texte dans l'explorateur : il prend alors sa place dans la fenêtre
//!   (`Demande::Ouvrir`).
//! - Le texte se coupe à la largeur de la fenêtre (au mot si possible), en chasse
//!   fixe ; toucher le texte y place le curseur et ouvre le clavier ; Entrée passe à
//!   la ligne, « Coller » garde les retours à la ligne.
//! - « Enregistrer » écrit le fichier (un nouveau document va dans `Documents/`, sous
//!   le nom saisi en haut). Les fins de ligne Windows (`\r\n`) sont conservées.
//! - Ce qui n'est pas enregistré est mis à l'abri dans un brouillon
//!   (`Flow/brouillons/`) quand l'application passe en arrière-plan ou que la fenêtre
//!   se ferme : on le retrouve au lancement suivant.
//!
//! - À gauche, la gouttière : le numéro de chaque ligne du texte (pas des lignes
//!   coupées à l'écran) et, devant une ligne qui ouvre un bloc entre accolades sur
//!   plusieurs lignes, un chevron. Toucher la gouttière plie ou déplie le bloc : ses
//!   lignes intérieures disparaissent, l'accolade fermante reste, et une pastille
//!   « ··· N » (N lignes cachées) le déplie aussi. Le curseur ne reste jamais dans un
//!   bloc plié : il le déplie.
//!
//! - La coloration syntaxique suit l'extension du document (ou du nom saisi) :
//!   voir `crate::syntaxe` et les dictionnaires de `Flow/syntaxes/`.
//!
//! L'espace de travail retient le document, le curseur, le brouillon, le nom d'un
//! nouveau document et les blocs pliés (`plis`, numéros de ligne à partir de 0).

use crate::clavier::{self, Edition, Frappe};
use crate::contexte::{Commun, Contexte, Etat, Genre};
use crate::draw::{Color, Rect};
use crate::syntaxe::{Bibliotheque, Classe, Plage};
use std::sync::Arc;
use crate::surface::{FenetreId, Placee};
use crate::tampon::{blocs, couper, ligne_de, logiques, position_en, Segment, Tampon};
use crate::ui::{id, sub_id, Id, Sense};
use crate::widgets::{Cx, Icon};
use std::path::{Path, PathBuf};

pub const CLE: &str = "texte";
pub const GENRE: Genre = Genre { cle: CLE, nom: "Texte", legende: "Éditer un texte brut, avec brouillons", icone: Icon::Fichier, ouvrir };

const TEXTE: Id = id("texte");
const CHAMP: Id = id("texte/champ");
const NOM: Id = id("texte/nom");
const NOM_CORPS: Id = id("texte/nom-corps");
const ENREGISTRER: Id = id("texte/enregistrer");
const GOUTTIERE: Id = id("texte/gouttiere");
const PLI: Id = id("texte/pli");
/// La taille des numéros de ligne.
const TAILLE_NUMEROS: f32 = 12.0;
/// La taille du texte, et la taille de fichier au-delà de laquelle il n'est pas ouvert.
const TAILLE: f32 = 15.0;
const TAILLE_MAX: u64 = 4 * 1024 * 1024;
/// Le dossier des nouveaux documents, dans le stockage, et celui des brouillons, dans
/// le dossier de Flow.
const DOCUMENTS: &str = "Documents";
const BROUILLONS: &str = "brouillons";
const NOM_PAR_DEFAUT: &str = "sans-titre.txt";

#[derive(Default)]
pub struct Texte {
    /// Le fichier édité ; rien pour un nouveau document.
    chemin: Option<PathBuf>,
    /// Le brouillon où les modifications non enregistrées ont été mises à l'abri.
    brouillon: Option<PathBuf>,
    tampon: Tampon,
    modifie: bool,
    /// Le fichier avait des fins de ligne Windows : elles sont réécrites à l'identique.
    crlf: bool,
    /// Le fichier est lu au premier dessin.
    charge: bool,
    /// Le curseur relu de l'espace, placé une fois le fichier lu.
    curseur_lu: usize,
    /// Un fichier qu'on ne peut pas éditer (trop gros, pas du texte).
    lecture_seule: bool,
    /// Le nom d'un nouveau document.
    nom: Edition,
    /// Un message pour la ligne d'état, et s'il signale une erreur.
    message: Option<(String, bool)>,
    /// Le découpage en lignes : pour quelle révision et combien de colonnes, les
    /// lignes à l'écran et la ligne du texte de chacune.
    coupe: Option<(u64, usize, Vec<Segment>, Vec<usize>)>,
    /// Les blocs entre accolades (lignes de début et de fin), pour quelle révision.
    blocs: Option<(u64, Vec<(usize, usize)>)>,
    /// Les lignes de début des blocs repliés, croissantes.
    plis: Vec<usize>,
    /// La coloration : pour quelle révision et quels dictionnaires (leur adresse), le
    /// nom du langage, et les plages colorées.
    couleurs: Option<(u64, usize, Option<String>, Vec<Plage>)>,
    /// Les lignes à l'écran visibles (hors des blocs repliés), et pour quel état.
    vue: Option<(u64, usize, Vec<usize>, Vec<usize>)>,
    /// Faire défiler jusqu'au curseur à la prochaine image.
    suivre: bool,
}

fn ouvrir(e: &Etat) -> Box<dyn Contexte> {
    Box::new(Texte {
        chemin: e.texte("document").map(PathBuf::from),
        brouillon: e.texte("brouillon").map(PathBuf::from),
        curseur_lu: e.nombre("curseur").map_or(0, |n| n.max(0.0) as usize),
        nom: Edition::new(e.texte("nom").unwrap_or("")),
        plis: {
            let mut v: Vec<usize> = e.nombres("plis").into_iter().map(|n| n.max(0.0) as usize).collect();
            v.sort_unstable();
            v.dedup();
            v
        },
        ..Texte::default()
    })
}

/// Un nom de fichier unique pour un brouillon.
fn nom_unique() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COMPTE: AtomicU64 = AtomicU64::new(0);
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
    format!("brouillon-{t}-{}.txt", COMPTE.fetch_add(1, Ordering::Relaxed))
}

/// Écrit à côté puis renomme : un fichier n'est jamais à moitié écrit.
pub(super) fn ecrire_fichier(chemin: &Path, texte: &str) -> Result<(), String> {
    if let Some(d) = chemin.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let nom = chemin.file_name().map_or("texte".into(), |n| n.to_string_lossy().into_owned());
    let tmp = chemin.with_file_name(format!(".{nom}.flow~"));
    std::fs::write(&tmp, texte).and_then(|_| std::fs::rename(&tmp, chemin)).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })
}

/// Lit un fichier texte : son contenu (fins de ligne ramenées à `\n`) et s'il avait
/// des fins de ligne Windows.
fn lire_fichier(chemin: &Path) -> Result<(String, bool), String> {
    let taille = std::fs::metadata(chemin).map_err(|e| e.to_string())?.len();
    if taille > TAILLE_MAX {
        return Err(format!("Fichier trop gros pour l'éditeur ({}).", crate::explorateur::taille(taille)));
    }
    let octets = std::fs::read(chemin).map_err(|e| e.to_string())?;
    let texte = String::from_utf8(octets).map_err(|_| "Ce fichier n'est pas du texte (UTF-8).".to_string())?;
    let crlf = texte.contains("\r\n");
    Ok((if crlf { texte.replace("\r\n", "\n") } else { texte }, crlf))
}

/// La couleur d'une classe de la coloration, en thème clair ou sombre.
fn teinte(k: Classe, sombre: bool) -> Color {
    let (clair, fonce): ([u8; 3], [u8; 3]) = match k {
        Classe::MotCle => ([0xA6, 0x26, 0xA4], [0xD1, 0x8C, 0xF0]),
        Classe::Type => ([0x0E, 0x7C, 0x86], [0x4E, 0xC9, 0xB0]),
        Classe::Constante | Classe::Nombre => ([0xB3, 0x55, 0x00], [0xF0, 0xA8, 0x60]),
        Classe::Chaine => ([0x2E, 0x7D, 0x32], [0x98, 0xC3, 0x79]),
        Classe::Commentaire => ([0x8A, 0x8F, 0x98], [0x7F, 0x84, 0x8E]),
        Classe::Fonction => ([0x1D, 0x5F, 0xD0], [0x61, 0xAF, 0xEF]),
        Classe::Macro => ([0xC2, 0x18, 0x5B], [0xE0, 0x6C, 0x9F]),
        Classe::Balise => ([0xC6, 0x28, 0x28], [0xE0, 0x6C, 0x75]),
        Classe::Attribut => ([0x9A, 0x67, 0x00], [0xE5, 0xC0, 0x7B]),
        Classe::Preprocesseur => ([0x6A, 0x4F, 0xC8], [0xB4, 0x9C, 0xF0]),
    };
    let c = if sombre { fonce } else { clair };
    [c[0], c[1], c[2], 255]
}

/// Les lignes à l'écran dont la ligne du texte n'est pas cachée (`cachees` : des
/// intervalles inclus, croissants).
fn visibles(logique: &[usize], cachees: &[(usize, usize)]) -> Vec<usize> {
    let mut out = Vec::with_capacity(logique.len());
    let mut k = 0;
    for (i, &l) in logique.iter().enumerate() {
        while k < cachees.len() && cachees[k].1 < l {
            k += 1;
        }
        // Les intervalles peuvent s'emboîter : le suivant peut commencer plus tôt
        let cache = cachees[k..].iter().take_while(|c| c.0 <= l).any(|c| l <= c.1);
        if !cache {
            out.push(i);
        }
    }
    out
}

impl Texte {
    /// Lit le brouillon s'il y en a un, sinon le fichier.
    fn charger(&mut self) {
        self.charge = true;
        self.lecture_seule = false;
        if let Some(b) = &self.brouillon {
            if let Ok((t, _)) = lire_fichier(b) {
                self.crlf = self.chemin.as_deref().and_then(|c| lire_fichier(c).ok()).is_some_and(|(_, crlf)| crlf);
                self.tampon = Tampon::new(&t);
                self.modifie = true;
                self.message = Some(("Brouillon non enregistré retrouvé".into(), false));
                self.placer_curseur();
                return;
            }
            self.brouillon = None;
        }
        if let Some(c) = &self.chemin {
            match lire_fichier(c) {
                Ok((t, crlf)) => {
                    self.tampon = Tampon::new(&t);
                    self.crlf = crlf;
                }
                Err(e) if !c.exists() => self.message = Some((format!("Fichier introuvable : il sera créé à l'enregistrement ({e})."), true)),
                Err(e) => {
                    self.lecture_seule = true;
                    self.message = Some((e, true));
                }
            }
        }
        self.placer_curseur();
    }

    fn placer_curseur(&mut self) {
        self.tampon.curseur = self.curseur_lu.min(self.tampon.car.len());
        self.suivre = true;
    }

    /// Le fichier où enregistrer : le document, ou pour un nouveau document le nom
    /// saisi, dans `Documents/`.
    fn destination(&self, c: &Commun) -> Result<PathBuf, String> {
        if let Some(p) = &self.chemin {
            return Ok(p.clone());
        }
        let mut nom = self.nom.texte.trim().to_string();
        if nom.is_empty() {
            nom = NOM_PAR_DEFAUT.into();
        }
        if nom.contains('/') || nom.starts_with('.') {
            return Err("Nom de fichier invalide.".into());
        }
        if !nom.contains('.') {
            nom.push_str(".txt");
        }
        let dossier = c.racine.as_ref().map(|r| r.join(DOCUMENTS)).or_else(|| c.dossier.clone()).ok_or("Aucun dossier où enregistrer.")?;
        let p = dossier.join(&nom);
        if p.exists() {
            return Err(format!("{nom} existe déjà dans {} : choisissez un autre nom.", dossier.display()));
        }
        Ok(p)
    }

    fn enregistrer(&mut self, c: &Commun) -> Result<(), String> {
        let chemin = self.destination(c)?;
        let mut texte = self.tampon.texte();
        if self.crlf {
            texte = texte.replace('\n', "\r\n");
        }
        ecrire_fichier(&chemin, &texte)?;
        self.chemin = Some(chemin);
        self.modifie = false;
        if let Some(b) = self.brouillon.take() {
            let _ = std::fs::remove_file(b);
        }
        Ok(())
    }

    fn nom_affiche(&self) -> String {
        match &self.chemin {
            Some(p) => p.file_name().map_or(p.display().to_string(), |n| n.to_string_lossy().into_owned()),
            None if self.nom.texte.trim().is_empty() => "Nouveau document".into(),
            None => self.nom.texte.trim().to_string(),
        }
    }

    /// Le découpage en lignes, refait si le texte ou la largeur ont changé.
    fn segments(&mut self, colonnes: usize) -> &[Segment] {
        self.couper(colonnes);
        &self.coupe.as_ref().unwrap().2
    }

    fn couper(&mut self, colonnes: usize) {
        let a_jour = self.coupe.as_ref().is_some_and(|(r, n, _, _)| *r == self.tampon.revision && *n == colonnes);
        if !a_jour {
            let s = couper(&self.tampon.car, colonnes);
            let l = logiques(&self.tampon.car, &s);
            self.coupe = Some((self.tampon.revision, colonnes, s, l));
        }
    }

    /// Les blocs entre accolades, refaits si le texte a changé ; les plis qui ne
    /// commencent plus un bloc sont oubliés.
    fn blocs(&mut self) -> &[(usize, usize)] {
        if self.blocs.as_ref().is_none_or(|(r, _)| *r != self.tampon.revision) {
            let b = blocs(&self.tampon.car);
            self.plis.retain(|l| b.iter().any(|x| x.0 == *l));
            self.blocs = Some((self.tampon.revision, b));
        }
        &self.blocs.as_ref().unwrap().1
    }

    /// Les lignes du texte cachées par les plis : des intervalles inclus, croissants.
    fn cachees(&mut self) -> Vec<(usize, usize)> {
        let plis = self.plis.clone();
        let mut v: Vec<(usize, usize)> = self.blocs().iter().filter(|b| plis.contains(&b.0) && b.1 > b.0 + 1).map(|b| (b.0 + 1, b.1 - 1)).collect();
        v.sort_unstable();
        v
    }

    /// Les lignes à l'écran visibles, refaites si le texte, la largeur ou les plis ont
    /// changé.
    fn visibles(&mut self, colonnes: usize) -> Vec<usize> {
        self.couper(colonnes);
        self.blocs();
        let a_jour = self.vue.as_ref().is_some_and(|(r, n, p, _)| *r == self.tampon.revision && *n == colonnes && *p == self.plis);
        if !a_jour {
            let cachees = self.cachees();
            let logique = &self.coupe.as_ref().unwrap().3;
            let v = visibles(logique, &cachees);
            self.vue = Some((self.tampon.revision, colonnes, self.plis.clone(), v));
        }
        self.vue.as_ref().unwrap().3.clone()
    }

    /// Le nom de fichier qui décide du langage : celui du document, ou le nom saisi.
    fn nom_fichier(&self) -> String {
        match &self.chemin {
            Some(p) => p.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned()),
            None => self.nom.texte.trim().to_string(),
        }
    }

    /// Refait la coloration si le texte, le nom ou les dictionnaires ont changé.
    fn colorer(&mut self, bib: &Arc<Bibliotheque>) {
        let adresse = Arc::as_ptr(bib) as usize;
        let langue = bib.pour(&self.nom_fichier());
        let nom = langue.map(|l| l.nom.clone());
        let a_jour = self.couleurs.as_ref().is_some_and(|(r, a, n, _)| *r == self.tampon.revision && *a == adresse && *n == nom);
        if !a_jour {
            let plages = langue.map_or(Vec::new(), |l| bib.colorer(&self.tampon.car, l));
            self.couleurs = Some((self.tampon.revision, adresse, nom, plages));
        }
    }

    /// Plie ou déplie le bloc qui commence à la ligne `l`.
    fn basculer(&mut self, l: usize) {
        if !self.blocs().iter().any(|b| b.0 == l) {
            return;
        }
        match self.plis.binary_search(&l) {
            Ok(i) => {
                self.plis.remove(i);
            }
            Err(i) => self.plis.insert(i, l),
        }
    }

    /// Déplie les blocs qui cachent la ligne `l` du texte.
    fn montrer(&mut self, l: usize) {
        let blocs = self.blocs().to_vec();
        self.plis.retain(|p| !blocs.iter().any(|b| b.0 == *p && *p < l && l < b.1));
    }
}

impl Contexte for Texte {
    fn cle(&self) -> &'static str {
        CLE
    }

    /// La fenêtre née d'un partage reçoit un nouveau document : un même fichier
    /// n'est pas édité deux fois.
    fn dupliquer(&self) -> Box<dyn Contexte> {
        Box::new(Texte::default())
    }

    fn document(&self) -> Option<&Path> {
        self.chemin.as_deref()
    }

    fn ecrire(&self, e: &mut Etat) {
        if let Some(p) = &self.chemin {
            e.mettre("document", p.display());
        } else if !self.nom.texte.trim().is_empty() {
            e.mettre("nom", self.nom.texte.trim());
        }
        if let Some(b) = &self.brouillon {
            e.mettre("brouillon", b.display());
        }
        let curseur = if self.charge { self.tampon.curseur } else { self.curseur_lu };
        if curseur > 0 {
            e.mettre("curseur", curseur);
        }
        if !self.plis.is_empty() {
            e.mettre("plis", self.plis.iter().map(|l| l.to_string()).collect::<Vec<_>>().join(" "));
        }
    }

    fn mettre_a_l_abri(&mut self, c: &Commun) {
        if !self.charge {
            return;
        }
        if !self.modifie {
            // Plus rien à garder
            if let Some(b) = self.brouillon.take() {
                let _ = std::fs::remove_file(b);
            }
            return;
        }
        let Some(chemin) = self.brouillon.clone().or_else(|| c.dossier.as_ref().map(|d| d.join(BROUILLONS).join(nom_unique()))) else { return };
        match ecrire_fichier(&chemin, &self.tampon.texte()) {
            Ok(()) => self.brouillon = Some(chemin),
            Err(e) => crate::android::log(&format!("brouillon non écrit : {e}")),
        }
    }

    fn retour(&mut self, _c: &Commun) {
        // Le fichier a pu changer entre-temps : relu s'il n'a pas de modifications ici
        if self.charge && !self.modifie && self.chemin.is_some() {
            self.curseur_lu = self.tampon.curseur;
            self.charge = false;
        }
    }

    fn frappe(&mut self, f: FenetreId, cible: Id, frappe: Frappe, c: &mut Commun) -> bool {
        let k = f as usize;
        if cible == sub_id(CHAMP, k) {
            if !self.lecture_seule && self.tampon.appliquer(frappe) {
                self.modifie = true;
                self.message = None;
            }
            self.suivre = true;
            true
        } else if cible == sub_id(NOM, k) {
            if self.nom.appliquer(frappe) {
                c.clavier.fermer();
            }
            self.message = None;
            true
        } else {
            false
        }
    }

    fn dessiner(&mut self, cx: &mut Cx, c: &mut Commun, p: &Placee) -> String {
        if !self.charge {
            self.charger();
        }
        let (d, col) = (cx.dp, cx.couleurs());
        let r = p.contenu;
        let k = p.id as usize;
        cx.dl.rrect(r, 0.0, col.surface);
        let (h, m, gap) = (cx.commande(), cx.coin(), cx.ecart());
        let haut = (h + 2.0 * m).min(r.h);

        // En haut : le nom du document (ou sa saisie), puis « Enregistrer »
        let label = "Enregistrer";
        let wp = cx.pill_width(label);
        let xp = r.right() - wp - m;
        let x0 = p.suite.0;
        if xp > x0 + gap && cx.pill(sub_id(ENREGISTRER, k), xp, r.y + m, label, self.modifie) && !self.lecture_seule {
            self.message = Some(match self.enregistrer(c) {
                Ok(()) => (format!("Enregistré : {}", self.chemin.as_deref().map_or(String::new(), |p| p.display().to_string())), false),
                Err(e) => (e, true),
            });
        }
        let x1 = (xp - gap).max(x0);
        if let Some(chemin) = &self.chemin {
            let dossier = chemin.parent().map_or(String::new(), |p| p.display().to_string());
            cx.clip(Some(Rect::new(x0, r.y, x1 - x0, haut)));
            let pastille = if self.modifie { " •" } else { "" };
            cx.text_at(x0, r.y + m + h * 0.32, &format!("{}{pastille}", self.nom_affiche()), 16.0, col.texte);
            cx.text_at(x0, r.y + m + h * 0.78, &dossier, 12.0, col.pale);
            cx.clip(None);
        } else {
            // Un nouveau document : son nom se saisit ici
            let pad = 14.0 * d;
            let zone = Rect::new(x0 - pad, r.y + m - 8.0 * d - (cx.theme.marge * 0.4 * d).round(), x1 - x0 + 2.0 * pad, haut);
            let corps = sub_id(NOM_CORPS, k);
            let mut b = cx.body(corps, zone, &c.scrolls);
            clavier::champ(cx, &mut b, sub_id(NOM, k), "Nom du nouveau document", &mut self.nom, &mut c.clavier);
            cx.body_end(b, &mut c.scrolls);
        }
        cx.dl.rrect(Rect::new(r.x, r.y + haut - d, r.w, d.max(1.0)), 0.0, col.ligne);

        // Le texte, avec à gauche la gouttière : les numéros de ligne et les plis
        let zone = Rect::new(r.x, r.y + haut, r.w, (r.h - haut).max(0.0));
        let champ = sub_id(CHAMP, k);
        let scroll = sub_id(TEXTE, k);
        let lh = (cx.px(TAILLE) * 1.5).round().max(1.0);
        let cw = cx.mesure_mono("0", TAILLE).max(1.0);
        let cn = cx.mesure_mono("0", TAILLE_NUMEROS).max(1.0);
        let marge_haut = 8.0 * d;
        let chiffres = self.tampon.lignes().to_string().len().max(2);
        let fleche = (lh * 0.8).round();
        let gouttiere = (chiffres as f32 * cn + fleche + 10.0 * d).round();
        let colonnes = ((zone.w - 28.0 * d - gouttiere) / cw).floor().max(4.0) as usize;
        // Le curseur ne reste pas dans un bloc replié
        let curseur = self.tampon.curseur;
        let seg_curseur = ligne_de(self.segments(colonnes), curseur);
        let l_curseur = self.coupe.as_ref().unwrap().3[seg_curseur];
        self.montrer(l_curseur);
        let vis = self.visibles(colonnes);
        let rang = |seg: usize| vis.partition_point(|&v| v < seg);
        let ligne_curseur = rang(seg_curseur);
        let n = vis.len();
        // Le curseur reste visible après une frappe
        if std::mem::take(&mut self.suivre) {
            let (off, contenu) = c.scrolls.get(&scroll).copied().unwrap_or((0.0, 0.0));
            let y = marge_haut + ligne_curseur as f32 * lh;
            let mut off = off;
            if y < off {
                off = y - marge_haut;
            } else if y + lh > off + zone.h {
                off = y + lh - zone.h + marge_haut;
            }
            let contenu = contenu.max(marge_haut + n as f32 * lh + 12.0 * d);
            c.scrolls.insert(scroll, (off.max(0.0), contenu));
        }
        let mut b = cx.body(scroll, zone, &c.scrolls);
        let top = b.y;
        let vue = b.rect();
        let xt = b.x + gouttiere;
        let focus = c.clavier.cible == Some(champ);
        let (s, logique) = {
            let co = self.coupe.as_ref().unwrap();
            (co.2.clone(), co.3.clone())
        };
        let blocs_ = self.blocs().to_vec();
        let premiere = ((vue.y - top) / lh).floor().max(0.0) as usize;
        let derniere = (((vue.bottom() - top) / lh).ceil().max(0.0) as usize).min(n);
        // Le fond de la gouttière et son filet
        let g = Rect::new(vue.x, vue.y, b.x - vue.x + gouttiere - 4.0 * d, vue.h);
        cx.dl.rrect(g, 0.0, col.fond);
        cx.dl.rrect(Rect::new(g.right(), vue.y, d.max(1.0), vue.h), 0.0, col.ligne);
        cx.ui.region_add(champ, Rect::new(g.right(), zone.y, zone.right() - g.right(), zone.h), Sense::Click { scroll: Some(b.id()) });
        // Toucher la gouttière plie ou déplie le bloc de la ligne
        let kg = sub_id(GOUTTIERE, k);
        cx.ui.region_add(kg, g, Sense::Click { scroll: Some(b.id()) });
        let mut basculer = None;
        if let Some((_, y)) = cx.ui.clic_en(kg) {
            let row = ((y - top) / lh).floor().max(0.0) as usize;
            if let Some(&seg) = vis.get(row) {
                basculer = Some(logique[seg]);
            }
        }
        // La coloration syntaxique, d'après l'extension du document
        let bib = c.syntaxes();
        self.colorer(&bib);
        let plages = self.couleurs.as_ref().map_or(Vec::new(), |c| c.3.clone());
        // Les pastilles « ··· » des blocs repliés : les toucher déplie
        let mut pastilles = Vec::new();
        let couleur = if self.lecture_seule { col.pale } else { col.texte };
        let l_courante = l_curseur;
        for row in premiere..derniere {
            let si = vis[row];
            let (a, e) = s[si];
            let y = top + row as f32 * lh;
            let cy = y + lh / 2.0;
            let t: String = self.tampon.car[a..e].iter().collect();
            if plages.is_empty() || self.lecture_seule {
                cx.mono_at(xt, cy, &t, TAILLE, couleur);
            } else {
                // Par morceaux : le texte ordinaire et les plages colorées
                let mut pos = a;
                let mut x = xt;
                let morceau = |cx: &mut Cx, de: usize, a_: usize, teinte: Color, x: &mut f32| {
                    if a_ > de {
                        let m: String = self.tampon.car[de..a_].iter().collect();
                        cx.mono_at(*x, cy, &m, TAILLE, teinte);
                        *x += cx.mesure_mono(&m, TAILLE);
                    }
                };
                let mut k = plages.partition_point(|p| p.1 <= a);
                while k < plages.len() && plages[k].0 < e {
                    let (pa, pe, classe) = plages[k];
                    let (pa, pe) = (pa.max(a), pe.min(e));
                    morceau(cx, pos, pa, couleur, &mut x);
                    morceau(cx, pa, pe, teinte(classe, cx.theme.sombre), &mut x);
                    pos = pe;
                    k += 1;
                }
                morceau(cx, pos, e, couleur, &mut x);
            }
            let l = logique[si];
            let premiere_du_texte = si == 0 || logique[si - 1] != l;
            if premiere_du_texte {
                let num = (l + 1).to_string();
                let wn = cx.mesure_mono(&num, TAILLE_NUMEROS);
                let xn = b.x + chiffres as f32 * cn - wn;
                cx.mono_at(xn, cy, &num, TAILLE_NUMEROS, if l == l_courante && focus { col.texte } else { col.pale });
                if let Some(bl) = blocs_.iter().find(|x| x.0 == l) {
                    let plie = self.plis.binary_search(&l).is_ok() && bl.1 > bl.0 + 1;
                    let f = Rect::new(b.x + chiffres as f32 * cn + 4.0 * d, y + (lh - fleche) / 2.0, fleche, fleche);
                    let (fx, fy, u) = (f.cx(), f.cy(), fleche / 10.0);
                    let tf = (1.4 * d).max(1.0);
                    let tc = if plie { col.accent } else { col.pale };
                    if plie {
                        cx.dl.line([fx - 1.5 * u, fy - 3.0 * u], [fx + 1.5 * u, fy], tf, tc);
                        cx.dl.line([fx + 1.5 * u, fy], [fx - 1.5 * u, fy + 3.0 * u], tf, tc);
                    } else {
                        cx.dl.line([fx - 3.0 * u, fy - 1.5 * u], [fx, fy + 1.5 * u], tf, tc);
                        cx.dl.line([fx, fy + 1.5 * u], [fx + 3.0 * u, fy - 1.5 * u], tf, tc);
                    }
                }
            }
            // Au bout de la dernière ligne à l'écran d'un bloc replié : « ··· N »
            let derniere_du_texte = s.get(si + 1).is_none_or(|_| logique[si + 1] != l);
            if derniere_du_texte && self.plis.binary_search(&l).is_ok() {
                if let Some(bl) = blocs_.iter().find(|x| x.0 == l && x.1 > x.0 + 1) {
                    let texte = format!("··· {}", bl.1 - bl.0 - 1);
                    let wp = cx.mesure_mono(&texte, TAILLE_NUMEROS) + 12.0 * d;
                    let pr = Rect::new(xt + cx.mesure_mono(&t, TAILLE) + 8.0 * d, y + lh * 0.15, wp, lh * 0.7);
                    let kp = sub_id(PLI, k * 1_000_003 + l);
                    pastilles.push((kp, pr, l));
                    cx.dl.rrect(pr, pr.h / 2.0, col.puce);
                    cx.mono_at(pr.x + 6.0 * d, pr.cy(), &texte, TAILLE_NUMEROS, col.accent);
                }
            }
        }
        for (kp, pr, l) in pastilles {
            if cx.ui.click(kp, pr, Some(b.id())) {
                basculer = Some(l);
            }
        }
        if let Some((x, y)) = cx.ui.clic_en(champ) {
            let row = ((y - top) / lh).floor().max(0.0) as usize;
            let seg = vis.get(row.min(n.saturating_sub(1))).copied().unwrap_or(0);
            let colonne = ((x - xt) / cw).round().max(0.0) as usize;
            self.tampon.curseur = position_en(&s, seg, colonne);
            if !self.lecture_seule {
                c.clavier.ouvrir(champ);
                self.suivre = true;
            }
        }
        if let Some(l) = basculer {
            self.basculer(l);
        }
        if self.tampon.car.is_empty() && !focus {
            cx.text_at(xt, top + lh / 2.0, if self.lecture_seule { "" } else { "Touchez pour écrire" }, TAILLE, col.pale);
        }
        if focus {
            let curseur = self.tampon.curseur;
            let l = ligne_de(&s, curseur);
            let avant: String = self.tampon.car[s[l].0..curseur.max(s[l].0)].iter().collect();
            let x = xt + cx.mesure_mono(&avant, TAILLE);
            let y = top + rang(l) as f32 * lh;
            cx.dl.rrect(Rect::new(x - d, y + lh * 0.15, 2.0 * d, lh * 0.7), d, col.accent);
        }
        b.y = top + n as f32 * lh;
        cx.body_end(b, &mut c.scrolls);

        // La ligne d'état
        if let Some((msg, _)) = &self.message {
            return msg.clone();
        }
        let (l, co) = self.tampon.position();
        let lignes = self.tampon.lignes();
        let langue = self.couleurs.as_ref().and_then(|c| c.2.clone()).map_or(String::new(), |n| format!(" · {n}"));
        format!("{}{langue} · {l}:{co} · {lignes} ligne{}{}", self.nom_affiche(), if lignes > 1 { "s" } else { "" }, if self.modifie { " · modifié" } else { "" })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dossier(nom: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("flow-texte-{nom}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn commun(d: &Path) -> Commun {
        Commun { racine: Some(d.to_path_buf()), dossier: Some(d.join("Flow")), ..Commun::default() }
    }

    fn taper(t: &mut Texte, s: &str) {
        let mut c = Commun::default();
        for ch in s.chars() {
            t.frappe(1, sub_id(CHAMP, 1), if ch == '\n' { Frappe::Entree } else { Frappe::Car(ch) }, &mut c);
        }
    }

    #[test]
    fn un_fichier_s_edite_et_s_enregistre_avec_ses_fins_de_ligne() {
        let d = dossier("crlf");
        let f = d.join("a.txt");
        std::fs::write(&f, "un\r\ndeux").unwrap();
        let mut e = Etat::default();
        e.mettre("document", f.display());
        e.mettre("curseur", 2);
        let mut t = Texte { chemin: Some(f.clone()), curseur_lu: 2, ..Texte::default() };
        t.charger();
        assert_eq!(t.tampon.texte(), "un\ndeux");
        taper(&mut t, "!");
        assert!(t.modifie);
        t.enregistrer(&commun(&d)).unwrap();
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "un!\r\ndeux");
        assert!(!t.modifie);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn un_nouveau_document_va_dans_documents_sans_ecraser() {
        let d = dossier("nouveau");
        let c = commun(&d);
        let mut t = Texte::default();
        t.charger();
        t.nom = Edition::new("notes");
        taper(&mut t, "bonjour\nmonde");
        t.enregistrer(&c).unwrap();
        let f = d.join("Documents/notes.txt");
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "bonjour\nmonde");
        assert_eq!(t.document(), Some(f.as_path()));
        // Un autre nouveau document du même nom n'écrase pas le premier
        let mut u = Texte { nom: Edition::new("notes.txt"), ..Texte::default() };
        u.charger();
        assert!(u.enregistrer(&c).unwrap_err().contains("existe déjà"));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn les_modifications_sont_mises_a_l_abri_et_retrouvees() {
        let d = dossier("brouillon");
        let c = commun(&d);
        let f = d.join("b.txt");
        std::fs::write(&f, "original").unwrap();
        let mut t = Texte { chemin: Some(f.clone()), ..Texte::default() };
        t.charger();
        taper(&mut t, "modifié ");
        t.mettre_a_l_abri(&c);
        let mut e = Etat::default();
        t.ecrire(&mut e);
        let brouillon = PathBuf::from(e.texte("brouillon").unwrap());
        assert!(brouillon.starts_with(d.join("Flow/brouillons")));
        // Relu depuis l'espace : le brouillon prime sur le fichier, intact
        let mut u = Texte { chemin: Some(f.clone()), brouillon: Some(brouillon.clone()), ..Texte::default() };
        u.charger();
        assert_eq!(u.tampon.texte(), "modifié original");
        assert!(u.modifie);
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "original");
        // Enregistré, le brouillon disparaît
        u.enregistrer(&c).unwrap();
        assert!(!brouillon.exists());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn les_blocs_se_replient_et_se_deplient() {
        let mut t = Texte::default();
        t.charger();
        taper(&mut t, "fn a() {\n    b {\n        c\n    }\n    d\n}\nfin");
        // Sept lignes, deux blocs : 0 à 5, et 1 à 3
        assert_eq!(t.visibles(80).len(), 7);
        t.basculer(1);
        assert_eq!(t.visibles(80), vec![0, 1, 3, 4, 5, 6]);
        t.basculer(0);
        assert_eq!(t.visibles(80), vec![0, 5, 6]);
        // Une ligne sans bloc ne se plie pas ; le pli se retient dans l'espace
        t.basculer(4);
        let mut e = Etat::default();
        t.ecrire(&mut e);
        assert_eq!(e.texte("plis"), Some("0 1"));
        // Le curseur dans un bloc replié déplie tout ce qui le cache
        t.montrer(2);
        assert!(t.plis.is_empty());
        assert_eq!(t.visibles(80).len(), 7);
        // Un pli qui ne commence plus un bloc après une modification est oublié
        t.basculer(1);
        t.tampon.curseur = t.tampon.car.iter().position(|c| *c == 'b').unwrap() + 2;
        taper(&mut t, "x");
        t.tampon.curseur = 0;
        t.tampon.car.retain(|c| *c != '{');
        t.tampon.revision += 1;
        assert!(t.blocs().is_empty());
        assert!(t.plis.is_empty());
    }

    #[test]
    fn un_fichier_binaire_reste_en_lecture_seule() {
        let d = dossier("binaire");
        let f = d.join("x.txt");
        std::fs::write(&f, [0xFF, 0xFE, 0x00]).unwrap();
        let mut t = Texte { chemin: Some(f), ..Texte::default() };
        t.charger();
        assert!(t.lecture_seule);
        taper(&mut t, "a");
        assert!(!t.modifie);
        let _ = std::fs::remove_dir_all(d);
    }
}
