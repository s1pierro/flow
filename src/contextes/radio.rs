//! Le contexte Radio : une adresse de flux à saisir, lecture et arrêt, le mixeur, et
//! les derniers flux écoutés. Le lecteur (`radio::lecteur()`) et l'historique sont
//! partagés ; l'adresse saisie est propre à chaque fenêtre.

use crate::clavier::{self, Edition, Frappe};
use crate::contexte::{Commun, Contexte, Etat, Genre};
use crate::draw::Rect;
use crate::mixage::{self, mixeur, Piste, Reglage};
use crate::radio::{self, Etat as Lecture};
use crate::surface::{FenetreId, Placee};
use crate::ui::{id, sub_id, Id};
use crate::widgets::{Body, Cx, Icon};

pub const CLE: &str = "radio";
pub const GENRE: Genre = Genre { cle: CLE, nom: "Radio", icone: Icon::Radio, ouvrir };

const RADIO: Id = id("radio");
const RADIO_CHAMP: Id = id("radio-adresse");

/// Deux flux pour essayer, tant que l'historique est vide.
const SUGGESTIONS: [&str; 2] = ["https://icecast.radiofrance.fr/fip-midfi.mp3", "https://stream.radioparadise.com/aac-128"];

#[derive(Clone, Default)]
pub struct Radio {
    adresse: Edition,
}

fn ouvrir(e: &Etat) -> Box<dyn Contexte> {
    Box::new(Radio { adresse: Edition::new(e.texte("adresse").unwrap_or("")) })
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

impl Contexte for Radio {
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
    }

    /// Les vumètres s'animent tant qu'une voie joue.
    fn anime(&self, _c: &Commun) -> bool {
        mixeur().actif() && !mixeur().en_pause()
    }

    /// Entrée lance le flux saisi.
    fn frappe(&mut self, f: FenetreId, cible: Id, frappe: Frappe, c: &mut Commun) -> bool {
        if cible != sub_id(RADIO_CHAMP, f as usize) {
            return false;
        }
        if self.adresse.appliquer(frappe) {
            c.clavier.fermer();
            let a = self.adresse.texte.clone();
            jouer(c, &a);
        }
        true
    }

    fn dessiner(&mut self, cx: &mut Cx, c: &mut Commun, p: &Placee) -> String {
        let r = p.contenu;
        let (col, h) = (cx.couleurs(), cx.commande());
        cx.dl.rrect(r, 0.0, col.surface);
        let (scroll, champ) = (sub_id(RADIO, p.id as usize), sub_id(RADIO_CHAMP, p.id as usize));
        let (etat, actif, en_cours) = {
            let l = radio::lecteur();
            (l.etat(), l.actif(), l.adresse.clone())
        };
        let mut b = cx.body(scroll, r, &c.scrolls);
        cx.section(&mut b, "FLUX");
        clavier::champ(cx, &mut b, champ, "Adresse du flux (http…)", &mut self.adresse, &mut c.clavier);
        // Lecture ou arrêt, et l'état à côté
        let y = b.y + cx.ecart();
        if cx.pastille(sub_id(scroll, 1), b.x, y, if actif { Icon::Stop } else { Icon::Lecture }, actif) {
            if actif {
                radio::lecteur().arreter();
            } else {
                let a = if self.adresse.texte.trim().is_empty() { c.historique.liste.first().cloned().unwrap_or_default() } else { self.adresse.texte.clone() };
                jouer(c, &a);
            }
        }
        let x = b.x + h + cx.pad();
        let (titre, detail) = match &etat {
            Lecture::Arret => ("Arrêt".to_string(), String::new()),
            Lecture::Connexion => ("Connexion…".to_string(), en_cours.as_deref().map(radio::courte).unwrap_or("").to_string()),
            Lecture::Lecture { .. } => ("Lecture".to_string(), etat.detail()),
            Lecture::Erreur(e) => ("Erreur".to_string(), e.clone()),
        };
        cx.clip(Some(Rect::new(x, y, (b.x + b.w - x).max(0.0), h).intersect(&b.rect())));
        cx.text_at(x, y + h * 0.32, &titre, 16.0, if matches!(etat, Lecture::Erreur(_)) { [0xE0, 0x3A, 0x3A, 255] } else { col.texte });
        cx.text_at(x, y + h * 0.75, &detail, 13.0, col.pale);
        cx.clip(Some(b.rect()));
        b.y = y + h + cx.ecart();
        mixage_ui(cx, &mut b, scroll);
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
            jouer(c, &a);
        }
        if let Some(f) = &c.historique.chemin {
            cx.paragraph(&mut b, &format!("Historique enregistré dans {}.", f.display()));
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

/// Le tableau du mixeur : la voie d'essai à lancer, chaque piste connue avec son
/// vumètre, son volume, sa balance et ses bascules, puis le volume général.
fn mixage_ui(cx: &mut Cx, b: &mut Body, scroll: Id) {
    let m = mixeur();
    cx.section(b, "MIXAGE");
    let mut essai = mixage::essai_actif();
    cx.toggle(b, sub_id(scroll, 3), "Voie d'essai (arpège)", &mut essai);
    if essai != mixage::essai_actif() {
        if let Err(e) = mixage::essai(essai) {
            crate::android::log(&format!("voie d'essai : {e}"));
        }
    }
    for (k, p) in m.pistes().into_iter().enumerate() {
        piste(cx, b, sub_id(scroll, 200 + 10 * k), &p);
    }
    // Le volume général, avec le vumètre de la sortie
    let ligne = (cx.theme.ligne * cx.dp).round();
    let info = match (m.niveau(), m.erreur()) {
        (_, Some(e)) => e,
        (Some(_), None) if m.en_pause() => "en pause".into(),
        (Some(_), None) => format!("sortie {:.1} kHz", m.frequence() as f32 / 1000.0),
        (None, None) => "sortie fermée".into(),
    };
    entete(cx, b, "Général", &info, m.niveau(), ligne);
    let mut g = m.general.get();
    let avant = g;
    let shown = format!("{:.0} %", g * 100.0);
    cx.slider(b, sub_id(scroll, 4), "Volume général", &shown, &mut g, 0.0, 1.0);
    if g != avant {
        m.general.set((g * 20.0).round() / 20.0);
    }
    if let Some(e) = m.erreur() {
        cx.paragraph(b, &e);
    }
}

/// Le nom d'une piste, un texte pâle et son vumètre (deux barres, gauche et droite).
fn entete(cx: &mut Cx, b: &mut Body, nom: &str, info: &str, niveau: Option<[f32; 2]>, ligne: f32) {
    let (d, col) = (cx.dp, cx.couleurs());
    let y = b.y;
    let w = cx.measure(nom, 16.0);
    cx.text_at(b.x, y + ligne / 2.0, nom, 16.0, col.texte);
    cx.text_at(b.x + w + 8.0 * d, y + ligne / 2.0, info, 12.0, col.pale);
    let x0 = b.x + (b.w * 0.5).max(w + 8.0 * d + cx.measure(info, 12.0) + 12.0 * d).min(b.w * 0.7);
    let large = b.x + b.w - x0;
    let (e, ep) = (5.0 * d, 3.0 * d);
    for ch in 0..2 {
        let r = Rect::new(x0, y + ligne / 2.0 - e - ep / 2.0 + ch as f32 * (e + ep), large, e);
        cx.dl.rrect(r, e / 2.0, col.ligne);
        if let Some(n) = niveau {
            let v = mixage::vumetre(n[ch]);
            // Vert, puis l'accent près du maximum, rouge à la saturation
            let teinte = if n[ch] >= 0.99 { [0xE0, 0x3A, 0x3A, 255] } else if v > 0.85 { col.accent } else { [0x22, 0xC5, 0x5E, 255] };
            if v > 0.0 {
                cx.dl.rrect(Rect::new(x0, r.y, (large * v).max(e), e), e / 2.0, teinte);
            }
        }
    }
    b.y += ligne;
}

fn piste(cx: &mut Cx, b: &mut Body, base: Id, p: &Piste) {
    let m = mixeur();
    let ligne = (cx.theme.ligne * cx.dp).round();
    let info = if p.niveau.is_some() { "joue" } else { "silencieuse" };
    entete(cx, b, &p.nom, info, p.niveau, ligne);
    let mut r: Reglage = p.reglage;
    let shown = format!("{:.0} %", r.gain * 100.0);
    cx.slider(b, sub_id(base, 0), "Volume", &shown, &mut r.gain, 0.0, 1.0);
    r.gain = (r.gain * 20.0).round() / 20.0;
    let shown = match (r.balance * 100.0).round() as i32 {
        0 => "centre".to_string(),
        v if v < 0 => format!("G {}", -v),
        v => format!("D {v}"),
    };
    cx.slider(b, sub_id(base, 1), "Balance", &shown, &mut r.balance, -1.0, 1.0);
    r.balance = (r.balance * 20.0).round() / 20.0;
    // Les bascules, en pilules, sous les curseurs
    let (gap, h) = (cx.ecart().max(4.0 * cx.dp), cx.commande());
    let mut x = b.x;
    let y = b.y + cx.ecart() * 0.5;
    for (i, (label, v)) in [("Muet", &mut r.muet), ("Solo", &mut r.solo), ("Prioritaire", &mut r.prioritaire)].into_iter().enumerate() {
        if cx.pill(sub_id(base, 2 + i), x, y, label, *v) {
            *v = !*v;
        }
        x += cx.pill_width(label) + gap;
    }
    b.y = y + h + cx.ecart();
    if r != p.reglage {
        m.regler(&p.nom, r);
    }
}
