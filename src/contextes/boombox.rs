//! Le contexte Boombox (la Radio jusqu'à la 0.37) : un en-tête fixe, aux couleurs
//! inversées (l'accent en fond), porte les modes (Flux, Magnéto, Bibliothèque), la
//! lecture, l'enregistrement et l'adresse du flux ; dessous, ce que montre le mode (en
//! Flux, les derniers flux écoutés, qui défilent). Le volume de la piste Radio se règle
//! dans le contexte Mixeur. Le lecteur (`radio::lecteur()`) et l'historique sont
//! partagés ; l'adresse saisie et le mode sont propres à chaque fenêtre.

use crate::clavier::{self, Edition, Frappe};
use crate::contexte::{Commun, Contexte, Etat, Genre};
use crate::draw::Rect;
use crate::mixage::mixeur;
use crate::radio::{self, Etat as Lecture};
use crate::surface::{FenetreId, Placee};
use crate::ui::{id, sub_id, Id};
use crate::widgets::{Cx, Icon};

pub const CLE: &str = "boombox";
pub const GENRE: Genre = Genre { cle: CLE, nom: "Boombox", legende: "Écouter un flux en ligne", icone: Icon::Radio, ouvrir, applet: Some(applet) };

const BOOMBOX: Id = id("boombox");
const ONGLETS: Id = id("boombox-onglets");
const LIRE: Id = id("boombox-lire");
const ENREGISTRER: Id = id("boombox-enregistrer");
const CHAMP: Id = id("radio-adresse");

const ROUGE: [u8; 4] = [0xE0, 0x3A, 0x3A, 255];

/// Deux flux pour essayer, tant que l'historique est vide.
const SUGGESTIONS: [&str; 2] = ["https://icecast.radiofrance.fr/fip-midfi.mp3", "https://stream.radioparadise.com/aac-128"];

/// Ce que montre le corps, sous l'en-tête.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Mode {
    #[default]
    Flux,
    Magneto,
    Bibliotheque,
}

const MODES: [(Mode, &str, &str); 3] = [(Mode::Flux, "Flux", "flux"), (Mode::Magneto, "Magnéto", "magneto"), (Mode::Bibliotheque, "Bibliothèque", "bibliotheque")];

#[derive(Clone, Default)]
pub struct Boombox {
    adresse: Edition,
    mode: Mode,
    /// Un mot passager, montré à la place de l'état de la lecture.
    note: Option<String>,
}

fn ouvrir(e: &Etat) -> Box<dyn Contexte> {
    let mode = e.texte("mode").and_then(|m| MODES.iter().find(|x| x.2 == m)).map_or(Mode::Flux, |x| x.0);
    Box::new(Boombox { adresse: Edition::new(e.texte("adresse").unwrap_or("")), mode, note: None })
}

/// Joue un flux et le place en tête de l'historique.
fn jouer(c: &mut Commun, adresse: &str) {
    let adresse = adresse.trim().to_string();
    if adresse.is_empty() {
        return;
    }
    radio::lecteur().jouer(&adresse);
    if let Err(e) = c.historique.ajouter(&adresse) {
        crate::android::log(&format!("historique des radios non enregistré : {e}"));
    }
}

impl Boombox {
    /// La hauteur de l'en-tête : les onglets, la ligne lecture et adresse, l'état.
    fn hauteur_tete(cx: &Cx) -> f32 {
        let m = cx.ecart();
        3.0 * m + 2.0 * cx.commande() + (22.0 * cx.dp * cx.theme.texte).round() + (m * 0.5).round()
    }

    /// L'en-tête fixe, dans le jeu de couleurs inversé.
    fn dessiner_tete(&mut self, cx: &mut Cx, c: &mut Commun, tete: Rect, k: usize) {
        let inv = cx.couleurs().inverse();
        cx.palette = Some(inv);
        cx.dl.rrect(tete, 0.0, inv.fond);
        cx.clip(Some(tete));
        let (h, m) = (cx.commande(), cx.ecart());
        let (x0, mut y) = (tete.x + m, tete.y + m);
        // Les modes
        let noms: Vec<&str> = MODES.iter().map(|x| x.1).collect();
        let i = MODES.iter().position(|x| x.0 == self.mode).unwrap_or(0);
        let j = cx.tabs(sub_id(ONGLETS, k), x0, y, &noms, i);
        if j != i {
            self.mode = MODES[j].0;
        }
        y += h + m;
        // Lecture ou arrêt, enregistrement, puis l'adresse
        let (etat, actif, en_cours) = {
            let l = radio::lecteur();
            (l.etat(), l.actif(), l.adresse.clone())
        };
        if cx.pastille(sub_id(LIRE, k), x0, y, if actif { Icon::Stop } else { Icon::Lecture }, actif) {
            self.note = None;
            if actif {
                radio::lecteur().arreter();
            } else {
                let a = if self.adresse.texte.trim().is_empty() { c.historique.liste.first().cloned().unwrap_or_default() } else { self.adresse.texte.clone() };
                jouer(c, &a);
            }
        }
        let g = (m * 0.6).round();
        let xr = x0 + h + g;
        if cx.pastille(sub_id(ENREGISTRER, k), xr, y, Icon::Enregistrer, false) {
            self.note = Some("L'enregistrement n'est pas encore branché.".into());
        }
        let ic = Rect::new(xr, y, h, h);
        cx.icon(ic.inset((cx.pad() * 0.6).min(h * 0.35)), Icon::Enregistrer, ROUGE);
        let xc = xr + h + g;
        let champ = Rect::new(xc, y, (tete.right() - m - xc).max(0.0), h);
        clavier::champ_en(cx, champ, None, tete, sub_id(CHAMP, k), "Adresse du flux (http…)", &mut self.adresse, &mut c.clavier);
        y += h + m;
        // L'état de la lecture, ou la note
        let lh = (22.0 * cx.dp * cx.theme.texte).round();
        let (titre, detail, erreur) = match (&self.note, &etat) {
            (Some(n), _) => (n.clone(), String::new(), false),
            (None, Lecture::Arret) => ("Arrêt".to_string(), String::new(), false),
            (None, Lecture::Connexion) => ("Connexion…".to_string(), en_cours.as_deref().map(radio::courte).unwrap_or("").to_string(), false),
            (None, Lecture::Lecture { .. }) => (en_cours.as_deref().map(radio::titre).unwrap_or("Lecture").to_string(), etat.detail(), false),
            (None, Lecture::Erreur(e)) => ("Erreur".to_string(), e.clone(), true),
        };
        let cy = y + lh / 2.0;
        cx.text_at(x0, cy, &titre, 15.0, inv.texte);
        if erreur {
            // Un point rouge dit l'erreur : le rouge ne se lit pas sur tous les accents
            cx.dl.rrect(Rect::new(x0 - 0.6 * m, cy - 3.0 * cx.dp, 6.0 * cx.dp, 6.0 * cx.dp), 3.0 * cx.dp, ROUGE);
        }
        if !detail.is_empty() {
            let x = x0 + cx.measure(&titre, 15.0) + cx.pad() * 0.6;
            cx.text_at(x, cy, &detail, 13.0, inv.pale);
        }
        cx.clip(None);
        cx.palette = None;
    }
}

impl Contexte for Boombox {
    fn cle(&self) -> &'static str {
        CLE
    }

    fn dupliquer(&self) -> Box<dyn Contexte> {
        Box::new(self.clone())
    }

    fn ecrire(&self, e: &mut Etat) {
        if !self.adresse.texte.is_empty() {
            e.mettre("adresse", &self.adresse.texte);
        }
        if self.mode != Mode::Flux {
            e.mettre("mode", MODES.iter().find(|x| x.0 == self.mode).map_or("flux", |x| x.2));
        }
    }

    /// La ligne d'état (le tampon) suit la lecture.
    fn anime(&self, _c: &Commun) -> bool {
        mixeur().actif() && !mixeur().en_pause()
    }

    /// Entrée lance le flux saisi.
    fn frappe(&mut self, f: FenetreId, cible: Id, frappe: Frappe, c: &mut Commun) -> bool {
        if cible != sub_id(CHAMP, f as usize) {
            return false;
        }
        if self.adresse.appliquer(frappe) {
            c.clavier.fermer();
            self.note = None;
            let a = self.adresse.texte.clone();
            jouer(c, &a);
        }
        true
    }

    fn dessiner(&mut self, cx: &mut Cx, c: &mut Commun, p: &Placee) -> String {
        let r = p.contenu;
        let k = p.id as usize;
        cx.dl.rrect(r, 0.0, cx.couleurs().surface);
        let tete = Rect::new(r.x, r.y, r.w, Boombox::hauteur_tete(cx).min(r.h));
        self.dessiner_tete(cx, c, tete, k);
        // Le corps défile sous l'en-tête, un défilement par mode
        let corps = Rect::new(r.x, tete.bottom(), r.w, (r.bottom() - tete.bottom()).max(0.0));
        let scroll = sub_id(sub_id(BOOMBOX, k), self.mode as usize);
        let (etat, actif, en_cours) = {
            let l = radio::lecteur();
            (l.etat(), l.actif(), l.adresse.clone())
        };
        let mut b = cx.body(scroll, corps, &c.scrolls);
        match self.mode {
            Mode::Flux => {
                // Les derniers flux : toucher pour écouter
                let (titre, liste) = if c.historique.liste.is_empty() { ("SUGGESTIONS", SUGGESTIONS.iter().map(|s| s.to_string()).collect()) } else { ("RÉCENTS", c.historique.liste.clone()) };
                cx.section(&mut b, titre);
                let mut choisi = None;
                for (i, a) in liste.iter().enumerate() {
                    let joue = actif && en_cours.as_deref() == Some(a.as_str());
                    if cx.row(&mut b, sub_id(scroll, 100 + i), radio::courte(a), None, joue) {
                        choisi = Some(a.clone());
                    }
                }
                if let Some(a) = choisi {
                    self.adresse = Edition::new(&a);
                    self.note = None;
                    jouer(c, &a);
                }
                if let Some(f) = &c.historique.chemin {
                    cx.paragraph(&mut b, &format!("Historique enregistré dans {}.", f.display()));
                }
            }
            Mode::Magneto => {
                cx.section(&mut b, "MAGNÉTO");
                cx.paragraph(&mut b, "Les enregistrements des flux viendront ici : le bouton rouge de l'en-tête enregistrera ce qui joue.");
            }
            Mode::Bibliotheque => {
                cx.section(&mut b, "BIBLIOTHÈQUE");
                cx.paragraph(&mut b, "Les morceaux et les enregistrements gardés viendront ici.");
            }
        }
        cx.body_end(b, &mut c.scrolls);
        match etat {
            Lecture::Lecture { .. } => {
                let t = mixeur().pistes().into_iter().find(|p| p.nom == radio::PISTE).map_or(0.0, |p| p.tampon);
                format!("tampon {:.0} %", t * 100.0)
            }
            Lecture::Connexion => "connexion".into(),
            Lecture::Erreur(_) => "erreur".into(),
            Lecture::Arret => "arrêt".into(),
        }
    }
}

// --- L'applet de la barre -------------------------------------------------------

/// L'applet de la Boombox : dans la barre, le poste, le flux en cours et un bouton lecture ou
/// arrêt ; dans le volet, l'état, le volume de la piste Radio et les derniers flux.
struct AppletRadio;

fn applet() -> Box<dyn crate::applet::Applet> {
    Box::new(AppletRadio)
}

/// Ce que le bouton lecture relancerait : le dernier flux, ou le premier de l'historique.
fn a_relancer(c: &Commun) -> Option<String> {
    radio::lecteur().dernier.clone().or_else(|| c.historique.liste.first().cloned())
}

/// Le titre court montré dans la barre, s'il y a une lecture.
fn titre_barre() -> Option<(String, bool)> {
    let l = radio::lecteur();
    let a = l.adresse.clone()?;
    let erreur = matches!(l.etat(), Lecture::Erreur(_));
    Some((radio::titre(&a).to_string(), erreur))
}

impl crate::applet::Applet for AppletRadio {
    fn largeur(&mut self, cx: &mut Cx, c: &Commun, h: f32, compact: bool) -> f32 {
        let texte = titre_barre().filter(|_| !compact).map_or(0.0, |(t, _)| cx.measure(&t, 13.0).min(150.0 * cx.dp) + 4.0 * cx.dp);
        // En compact, le poste seul : le volet garde la lecture et l'arrêt
        let bouton = if !compact && (radio::lecteur().actif() || a_relancer(c).is_some()) { h } else { 0.0 };
        h * 0.9 + texte + bouton
    }

    fn barre(&mut self, cx: &mut Cx, c: &mut Commun, r: Rect, id: Id, compact: bool) {
        let (d, col) = (cx.dp, cx.couleurs());
        let ic = (r.h * 0.5).round();
        let actif = radio::lecteur().actif();
        // Le poste prend l'accent pendant la lecture
        cx.icon(Rect::new(r.x + (r.h * 0.9 - ic) / 2.0, r.cy() - ic / 2.0, ic, ic), Icon::Radio, if actif { col.accent } else { col.texte });
        let mut x = r.x + r.h * 0.9;
        let bouton = !compact && (actif || a_relancer(c).is_some());
        if let Some((t, erreur)) = titre_barre().filter(|_| !compact) {
            let fin = r.right() - if bouton { r.h } else { 0.0 };
            cx.clip(Some(Rect::new(x, r.y, (fin - x).max(0.0), r.h)));
            let t = crate::contextes::abreger(cx, &t, 13.0, (fin - x - 4.0 * d).max(0.0));
            cx.text_at(x, r.cy(), &t, 13.0, if erreur { ROUGE } else { col.texte });
            cx.clip(None);
            x = fin;
        }
        if bouton && cx.pastille_plate(sub_id(id, 1), Rect::new(x, r.y, r.h, r.h), if actif { Icon::Stop } else { Icon::Lecture }, false) {
            if actif {
                radio::lecteur().arreter();
            } else if let Some(a) = a_relancer(c) {
                jouer(c, &a);
            }
        }
    }

    fn hauteur_volet(&mut self, cx: &mut Cx, c: &Commun, _w: f32) -> f32 {
        let d = cx.dp;
        let ligne = (cx.theme.ligne * d).round();
        let n = c.historique.liste.len().clamp(1, 6) as f32;
        let section = (20.0 + 18.0 * cx.theme.texte) * d;
        8.0 * d + 2.0 * section + cx.commande() + 2.0 * cx.ecart() + (64.0 * d * cx.theme.texte.max(1.0)).round() + n * ligne + cx.commande() + 2.0 * (cx.theme.marge * d).round() + 20.0 * d
    }

    fn volet(&mut self, cx: &mut Cx, c: &mut Commun, r: Rect, id: Id) -> bool {
        let (col, h) = (cx.couleurs(), cx.commande());
        let scroll = sub_id(id, 0);
        let (etat, actif, en_cours) = {
            let l = radio::lecteur();
            (l.etat(), l.actif(), l.adresse.clone())
        };
        let mut b = cx.body(scroll, r, &c.scrolls);
        cx.section(&mut b, "BOOMBOX");
        let y = b.y + cx.ecart();
        let relance = a_relancer(c);
        if cx.pastille(sub_id(scroll, 1), b.x, y, if actif { Icon::Stop } else { Icon::Lecture }, actif) {
            if actif {
                radio::lecteur().arreter();
            } else if let Some(a) = &relance {
                jouer(c, a);
            }
        }
        let x = b.x + h + cx.pad();
        let (titre, detail) = match &etat {
            Lecture::Arret => ("Arrêt".to_string(), relance.as_deref().map(radio::courte).unwrap_or("aucun flux écouté").to_string()),
            Lecture::Connexion => ("Connexion…".to_string(), en_cours.as_deref().map(radio::courte).unwrap_or("").to_string()),
            Lecture::Lecture { .. } => (en_cours.as_deref().map(radio::titre).unwrap_or("Lecture").to_string(), etat.detail()),
            Lecture::Erreur(e) => ("Erreur".to_string(), e.clone()),
        };
        cx.clip(Some(Rect::new(x, y, (b.x + b.w - x).max(0.0), h).intersect(&b.rect())));
        cx.text_at(x, y + h * 0.32, &titre, 16.0, if matches!(etat, Lecture::Erreur(_)) { ROUGE } else { col.texte });
        cx.text_at(x, y + h * 0.75, &detail, 13.0, col.pale);
        cx.clip(Some(b.rect()));
        b.y = y + h + cx.ecart();
        // Le volume de la piste Radio, comme dans le Mixeur
        let mut reglage = mixeur().reglage(radio::PISTE);
        let avant = reglage.gain;
        let shown = format!("{:.0} %", reglage.gain * 100.0);
        cx.slider(&mut b, sub_id(scroll, 2), "Volume", &shown, &mut reglage.gain, 0.0, 1.0);
        reglage.gain = (reglage.gain * 20.0).round() / 20.0;
        if reglage.gain != avant {
            mixeur().regler(radio::PISTE, reglage);
        }
        let (titre, liste) = if c.historique.liste.is_empty() { ("SUGGESTIONS", SUGGESTIONS.iter().map(|s| s.to_string()).collect::<Vec<_>>()) } else { ("RÉCENTS", c.historique.liste.iter().take(6).cloned().collect()) };
        cx.section(&mut b, titre);
        let mut choisi = None;
        for (i, a) in liste.iter().enumerate() {
            let joue = actif && en_cours.as_deref() == Some(a.as_str());
            if cx.row(&mut b, sub_id(scroll, 100 + i), radio::courte(a), None, joue) {
                choisi = Some(a.clone());
            }
        }
        if let Some(a) = choisi {
            jouer(c, &a);
        }
        let ouvrir = cx.button(&mut b, sub_id(scroll, 3), "Ouvrir la Boombox");
        if ouvrir {
            c.demandes.push(crate::contexte::Demande::Montrer { cle: CLE });
        }
        cx.body_end(b, &mut c.scrolls);
        ouvrir
    }
}
