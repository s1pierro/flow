//! Le contexte Toile : une page web (HTML, CSS, JavaScript), rendue par le moteur
//! d'Android (WebView, voir `java/fr/s1pierro/flow/Toile.java`).
//!
//! Flow dessine tout lui-même : la page est rendue hors de l'écran, dans une image que
//! la fenêtre affiche (`DrawList::image`) ; les touchers lui sont renvoyés. Elle se
//! superpose donc comme n'importe quelle fenêtre, en Free flow aussi. Une page locale
//! (`file://`) se recharge quand son fichier change : avec le bouton Source, le
//! fichier s'ouvre dans l'éditeur Texte à côté, et chaque enregistrement se voit.
//! La saisie dans les champs d'une page passe, pour l'instant, par le clavier
//! d'Android. Hors d'Android, la page reste vide.

use crate::acces;
use crate::clavier::{self, Edition, Frappe};
use crate::contexte::{Commun, Contexte, Demande, Etat, Genre};
use crate::draw::{Image, Rect};
use crate::surface::{FenetreId, Placee};
use crate::ui::{id, sub_id, Id};
use crate::widgets::{Cx, Icon};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Instant, SystemTime};

pub const CLE: &str = "toile";
pub const GENRE: Genre = Genre { cle: CLE, nom: "Toile", legende: "Pages web : HTML, CSS, JavaScript", icone: Icon::Toile, ouvrir, applet: None };

const CHAMP: Id = id("toile-adresse");
const BOUTONS: Id = id("toile-boutons");
const PAGE: Id = id("toile-page");

/// Les pages ouvertes : chacune a son numéro, qui la nomme côté Java.
static SUIVANTE: AtomicU64 = AtomicU64::new(1);

/// Ce que Java dit de la page.
#[derive(Clone, Debug, Default, PartialEq)]
struct EtatPage {
    version: u64,
    largeur: u32,
    hauteur: u32,
    url: String,
    titre: String,
    progres: u32,
    arriere: bool,
    avant: bool,
    console: String,
}

impl EtatPage {
    fn lire(t: &str) -> EtatPage {
        let l: Vec<&str> = t.split('\n').collect();
        let s = |i: usize| l.get(i).copied().unwrap_or("").to_string();
        let n = |i: usize| l.get(i).and_then(|x| x.trim().parse::<u64>().ok()).unwrap_or(0);
        EtatPage { version: n(0), largeur: n(1) as u32, hauteur: n(2) as u32, url: s(3), titre: s(4), progres: n(5) as u32, arriere: n(6) == 1, avant: n(7) == 1, console: s(8) }
    }
}

pub struct Toile {
    num: u64,
    adresse: Edition,
    /// L'adresse chargée (ou à charger à la première image).
    chargee: Option<String>,
    page: EtatPage,
    image: Option<Image>,
    pixels: Vec<u8>,
    taille: (u32, u32),
    /// Le doigt posé sur la page, et où il était.
    doigt: Option<(f32, f32)>,
    /// Le fichier local montré et sa date, pour le recharger quand il change.
    fichier: Option<(PathBuf, Option<SystemTime>)>,
    verifie: Instant,
}

/// Une adresse saisie : un chemin devient `file://`, un nom sans protocole `https://`.
pub fn normaliser(a: &str) -> String {
    let a = a.trim();
    if a.is_empty() || a.contains("://") || a.starts_with("about:") || a.starts_with("data:") || a.starts_with("javascript:") {
        a.to_string()
    } else if a.starts_with('/') {
        format!("file://{a}")
    } else {
        format!("https://{a}")
    }
}

/// Le fichier d'une adresse `file://`.
fn fichier_de(url: &str) -> Option<PathBuf> {
    let p = url.strip_prefix("file://")?;
    let p = p.split(['?', '#']).next().unwrap_or(p);
    Some(PathBuf::from(p))
}

fn date(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

fn ouvrir(e: &Etat) -> Box<dyn Contexte> {
    let a = e.texte("document").map(|d| normaliser(d)).or_else(|| e.texte("adresse").map(str::to_string)).unwrap_or_default();
    Box::new(Toile::nouvelle(&a))
}

impl Toile {
    fn nouvelle(adresse: &str) -> Toile {
        Toile {
            num: SUIVANTE.fetch_add(1, Ordering::Relaxed),
            adresse: Edition::new(adresse),
            chargee: (!adresse.is_empty()).then(|| adresse.to_string()),
            page: EtatPage::default(),
            image: None,
            pixels: Vec::new(),
            taille: (0, 0),
            doigt: None,
            fichier: None,
            verifie: Instant::now(),
        }
    }

    fn id(&self) -> String {
        format!("p{}", self.num)
    }

    /// Charge une adresse dans la page.
    fn aller(&mut self, a: &str) {
        let a = normaliser(a);
        if a.is_empty() {
            return;
        }
        self.adresse = Edition::new(&a);
        self.fichier = fichier_de(&a).map(|p| {
            let d = date(&p);
            (p, d)
        });
        acces::toile("ouvrir", &format!("{}\n{a}", self.id()));
        self.chargee = Some(a);
    }

    fn naviguer(&self, m: &str) {
        acces::toile("naviguer", &format!("{} {m}", self.id()));
    }

    fn toucher(&self, action: u8, x: f32, y: f32, page: Rect) {
        acces::toile("toucher", &format!("{} {action} {:.1} {:.1}", self.id(), x - page.x, y - page.y));
    }

    /// Les boutons de l'en-tête : précédent, suivant, recharger, puis l'adresse, puis
    /// Source pour un fichier local.
    fn dessiner_tete(&mut self, cx: &mut Cx, c: &mut Commun, tete: Rect, k: usize) {
        let col = cx.couleurs();
        cx.dl.rrect(tete, 0.0, col.surface);
        cx.dl.rrect(Rect::new(tete.x, tete.bottom() - 1.0, tete.w, 1.0), 0.0, col.ligne);
        cx.clip(Some(tete));
        let (h, m) = (cx.commande(), (cx.ecart() * 0.5).round());
        let y = tete.y + m;
        let mut x = tete.x + m;
        let base = sub_id(BOUTONS, k);
        let source = self.fichier.as_ref().map(|f| f.0.clone());
        for (i, (icone, actif)) in [(Icon::Precedent, self.page.arriere), (Icon::Suivant, self.page.avant), (Icon::Recharger, true)].into_iter().enumerate() {
            let r = Rect::new(x, y, h, h);
            if cx.pastille_plate(sub_id(base, i), r, icone, false) && actif {
                match i {
                    0 => self.naviguer("arriere"),
                    1 => self.naviguer("avant"),
                    _ => self.naviguer(if self.page.progres < 100 { "arreter" } else { "recharger" }),
                }
            }
            if !actif {
                // Pâle : rien en arrière (ou en avant)
                cx.dl.rrect(r, 0.0, [col.surface[0], col.surface[1], col.surface[2], 150]);
            }
            x += h;
        }
        let fin = tete.right() - m - if source.is_some() { h + m } else { 0.0 };
        let champ = Rect::new(x + m, y, (fin - x - m).max(0.0), h);
        clavier::champ_en(cx, champ, None, tete, sub_id(CHAMP, k), "Adresse ou chemin (https://…, /sdcard/…)", &mut self.adresse, &mut c.clavier);
        if let Some(f) = source {
            if cx.pastille_plate(sub_id(base, 9), Rect::new(fin + m, y, h, h), Icon::Fichier, false) {
                let mut e = Etat::default();
                e.mettre("document", f.display());
                c.demandes.push(Demande::OuvrirACote { depuis: k as FenetreId, cle: super::texte::CLE, etat: e });
            }
        }
        // L'avancement du chargement, en filet sous l'en-tête
        if self.page.progres > 0 && self.page.progres < 100 {
            cx.dl.rrect(Rect::new(tete.x, tete.bottom() - 3.0 * cx.dp, tete.w * self.page.progres as f32 / 100.0, 3.0 * cx.dp), 0.0, col.accent);
        }
        cx.clip(None);
    }
}

impl Clone for Toile {
    /// Une copie est une autre page, sur la même adresse.
    fn clone(&self) -> Toile {
        Toile::nouvelle(self.chargee.as_deref().unwrap_or(&self.adresse.texte))
    }
}

impl Drop for Toile {
    fn drop(&mut self) {
        acces::toile("fermer", &self.id());
    }
}

impl Contexte for Toile {
    fn cle(&self) -> &'static str {
        CLE
    }

    fn dupliquer(&self) -> Box<dyn Contexte> {
        Box::new(self.clone())
    }

    fn ecrire(&self, e: &mut Etat) {
        let a = if self.page.url.is_empty() { self.chargee.clone().unwrap_or_default() } else { self.page.url.clone() };
        if !a.is_empty() {
            e.mettre("adresse", a);
        }
    }

    /// La page change d'elle-même (animations, chargement) : on la suit.
    fn anime(&self, _c: &Commun) -> bool {
        cfg!(target_os = "android")
    }

    fn document(&self) -> Option<&Path> {
        self.fichier.as_ref().map(|f| f.0.as_path())
    }

    /// Entrée charge l'adresse saisie.
    fn frappe(&mut self, f: FenetreId, cible: Id, frappe: Frappe, c: &mut Commun) -> bool {
        if cible != sub_id(CHAMP, f as usize) {
            return false;
        }
        if self.adresse.appliquer(frappe) {
            c.clavier.fermer();
            let a = self.adresse.texte.clone();
            self.aller(&a);
        }
        true
    }

    fn dessiner(&mut self, cx: &mut Cx, c: &mut Commun, p: &Placee) -> String {
        let r = p.contenu;
        let k = p.id as usize;
        let col = cx.couleurs();
        let hauteur = cx.commande() + 2.0 * (cx.ecart() * 0.5).round();
        let tete = Rect::new(r.x, r.y, r.w, hauteur.min(r.h));
        let zone = Rect::new(r.x, tete.bottom(), r.w, (r.bottom() - tete.bottom()).max(0.0));
        // La page : créée à la première image, à la taille de la zone
        if let Some(a) = self.chargee.clone().filter(|_| self.page.version == 0 && self.taille == (0, 0)) {
            self.aller(&a);
        }
        let t = (zone.w.round().max(1.0) as u32, zone.h.round().max(1.0) as u32);
        if t != self.taille {
            self.taille = t;
            acces::toile("taille", &format!("{} {} {}", self.id(), t.0, t.1));
        }
        if let Some(e) = acces::toile("etat", &self.id()) {
            let e = EtatPage::lire(&e);
            let nouvelle = e.version != self.page.version;
            // L'adresse suit la navigation, sauf pendant la saisie
            if e.url != self.page.url && !e.url.is_empty() && c.clavier.cible != Some(sub_id(CHAMP, k)) {
                self.adresse = Edition::new(&e.url);
                if fichier_de(&e.url).as_deref() != self.fichier.as_ref().map(|f| f.0.as_path()) {
                    self.fichier = fichier_de(&e.url).map(|p| {
                        let d = date(&p);
                        (p, d)
                    });
                }
            }
            self.page = e;
            if nouvelle && acces::toile_pixels(&self.id(), &mut self.pixels) {
                self.image = Some(Image { id: 0x70_6167_6500_0000 + self.num, revision: self.page.version, largeur: self.page.largeur, hauteur: self.page.hauteur, pixels: Arc::new(std::mem::take(&mut self.pixels)) });
            }
        }
        // Un fichier local qui change se recharge
        if self.verifie.elapsed().as_secs_f32() > 0.8 {
            self.verifie = Instant::now();
            if let Some((f, d)) = &mut self.fichier {
                let n = date(f);
                if n != *d {
                    *d = n;
                    self.naviguer("recharger");
                }
            }
        }
        // La page, à sa taille (pendant un redimensionnement, l'image d'avant)
        cx.dl.rrect(zone, 0.0, [255, 255, 255, 255]);
        cx.clip(Some(zone));
        match &self.image {
            Some(img) => cx.dl.image(Rect::new(zone.x, zone.y, img.largeur as f32, img.hauteur as f32), img.clone()),
            None => {
                let texte = if cfg!(target_os = "android") {
                    if self.chargee.is_some() { "Chargement…" } else { "Saisissez une adresse, ou ouvrez un .html depuis Fichiers." }
                } else {
                    "La Toile ne s'affiche que sous Android."
                };
                cx.text_at(zone.x + cx.pad(), zone.y + cx.pad() + 10.0 * cx.dp, texte, 15.0, col.pale);
            }
        }
        cx.clip(None);
        // Les doigts vont à la page
        let pk = sub_id(PAGE, k);
        match cx.ui.drag(pk, zone) {
            Some(g) => {
                match self.doigt {
                    None => self.toucher(0, g.x, g.y, zone),
                    Some(a) if a != (g.x, g.y) => self.toucher(2, g.x, g.y, zone),
                    _ => {}
                }
                self.doigt = Some((g.x, g.y));
            }
            None => {
                if let Some((x, y)) = self.doigt.take() {
                    self.toucher(1, x, y, zone);
                } else if let Some((x, y)) = cx.ui.clic_en(pk) {
                    // Un tap plus bref qu'une image
                    self.toucher(0, x, y, zone);
                    self.toucher(1, x, y, zone);
                }
            }
        }
        self.dessiner_tete(cx, c, tete, k);
        // La ligne d'état : le dernier message de la console, sinon le titre
        if !self.page.console.is_empty() {
            self.page.console.clone()
        } else {
            self.page.titre.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn les_adresses_se_normalisent_et_l_etat_se_relit() {
        assert_eq!(normaliser("/sdcard/a.html"), "file:///sdcard/a.html");
        assert_eq!(normaliser("exemple.org"), "https://exemple.org");
        assert_eq!(normaliser(" http://x "), "http://x");
        assert_eq!(fichier_de("file:///sdcard/a.html?b#c"), Some(PathBuf::from("/sdcard/a.html")));
        let e = EtatPage::lire("7\n800\n600\nfile:///a.html\nTitre\n100\n1\n0\nbonjour (a.html:3)");
        assert_eq!((e.version, e.largeur, e.titre.as_str(), e.arriere, e.avant, e.console.as_str()), (7, 800, "Titre", true, false, "bonjour (a.html:3)"));
        // Hors d'Android, la page ne s'ouvre pas, mais rien ne casse
        let t = Toile::nouvelle("/x.html");
        assert_eq!(t.chargee.as_deref(), Some("/x.html"));
        assert!(acces::toile("etat", &t.id()).is_none());
    }
}
