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
//! L'espace de travail retient le document, le curseur, le brouillon et le nom d'un
//! nouveau document.

use crate::clavier::{self, Edition, Frappe};
use crate::contexte::{Commun, Contexte, Etat, Genre};
use crate::draw::Rect;
use crate::surface::{FenetreId, Placee};
use crate::tampon::{couper, ligne_de, position_en, Segment, Tampon};
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
    /// Le découpage en lignes : pour quelle révision et combien de colonnes.
    coupe: Option<(u64, usize, Vec<Segment>)>,
    /// Faire défiler jusqu'au curseur à la prochaine image.
    suivre: bool,
}

fn ouvrir(e: &Etat) -> Box<dyn Contexte> {
    Box::new(Texte {
        chemin: e.texte("document").map(PathBuf::from),
        brouillon: e.texte("brouillon").map(PathBuf::from),
        curseur_lu: e.nombre("curseur").map_or(0, |n| n.max(0.0) as usize),
        nom: Edition::new(e.texte("nom").unwrap_or("")),
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
        let a_jour = self.coupe.as_ref().is_some_and(|(r, n, _)| *r == self.tampon.revision && *n == colonnes);
        if !a_jour {
            self.coupe = Some((self.tampon.revision, colonnes, couper(&self.tampon.car, colonnes)));
        }
        &self.coupe.as_ref().unwrap().2
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

        // Le texte
        let zone = Rect::new(r.x, r.y + haut, r.w, (r.h - haut).max(0.0));
        let champ = sub_id(CHAMP, k);
        let scroll = sub_id(TEXTE, k);
        let lh = (cx.px(TAILLE) * 1.5).round().max(1.0);
        let cw = cx.mesure_mono("0", TAILLE).max(1.0);
        let marge_haut = 8.0 * d;
        let colonnes = ((zone.w - 28.0 * d) / cw).floor().max(4.0) as usize;
        let curseur = self.tampon.curseur;
        let (ligne_curseur, n) = {
            let s = self.segments(colonnes);
            (ligne_de(s, curseur), s.len())
        };
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
        let focus = c.clavier.cible == Some(champ);
        cx.ui.region_add(champ, zone, Sense::Click { scroll: Some(b.id()) });
        if let Some((x, y)) = cx.ui.clic_en(champ) {
            let ligne = ((y - top) / lh).floor().max(0.0) as usize;
            let colonne = ((x - b.x) / cw).round().max(0.0) as usize;
            let s = self.segments(colonnes);
            self.tampon.curseur = position_en(s, ligne, colonne);
            if !self.lecture_seule {
                c.clavier.ouvrir(champ);
                self.suivre = true;
            }
        }
        if self.tampon.car.is_empty() && !focus {
            cx.text_at(b.x, top + lh / 2.0, if self.lecture_seule { "" } else { "Touchez pour écrire" }, TAILLE, col.pale);
        }
        let premiere = ((vue.y - top) / lh).floor().max(0.0) as usize;
        let derniere = (((vue.bottom() - top) / lh).ceil().max(0.0) as usize).min(n);
        let couleur = if self.lecture_seule { col.pale } else { col.texte };
        let curseur = self.tampon.curseur;
        let s = self.segments(colonnes).to_vec();
        for (i, &(a, e)) in s.iter().enumerate().take(derniere).skip(premiere) {
            let t: String = self.tampon.car[a..e].iter().collect();
            cx.mono_at(b.x, top + i as f32 * lh + lh / 2.0, &t, TAILLE, couleur);
        }
        if focus {
            let l = ligne_de(&s, curseur);
            let avant: String = self.tampon.car[s[l].0..curseur.max(s[l].0)].iter().collect();
            let x = b.x + cx.mesure_mono(&avant, TAILLE);
            let y = top + l as f32 * lh;
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
        format!("{} · {l}:{co} · {lignes} ligne{}{}", self.nom_affiche(), if lignes > 1 { "s" } else { "" }, if self.modifie { " · modifié" } else { "" })
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
