//! Le contexte Mixeur : la console du mixeur audio (`crate::mixage`), partagé par
//! toute l'application. Chaque piste connue (Radio, Fichier, Essai…) a sa tranche :
//! nom, vumètre stéréo, volume au curseur vertical, balance, et les bascules Muet,
//! Solo, Prioritaire. La tranche Général, à droite, règle la sortie.
//!
//! Quand la fenêtre est trop étroite ou trop basse pour des tranches, la console se
//! range en liste (curseurs horizontaux), qui défile.
//!
//! Les réglages s'enregistrent dans `Flow/mixeur.txt` (voir `mixage`).
//!
//! Chaque tranche (et la sortie) montre la courbe de son égaliseur en vignette ; la
//! toucher ouvre l'éditeur à la place de la console : la courbe, cinq poignées à
//! tirer (fréquence et gain), et pour la bande choisie sa fréquence, son gain et sa
//! largeur.

use super::hachage;
use crate::contexte::{Commun, Contexte, Etat, Genre};
use crate::egaliseur::{self, Egaliseur, Forme, BANDES, FORMES};
use crate::draw::Rect;
use crate::mixage::{self, mixeur, Piste, Reglage};
use crate::surface::Placee;
use crate::ui::{id, sub_id, Id};
use crate::widgets::{Body, Cx, Icon};

pub const CLE: &str = "mixeur";
pub const GENRE: Genre = Genre { cle: CLE, nom: "Mixeur", legende: "Le volume de chaque piste audio", icone: Icon::Mixeur, ouvrir, applet: Some(applet) };

const MIXEUR: Id = id("mixeur");
const ROUGE: [u8; 4] = [0xE0, 0x3A, 0x3A, 255];
const VERT: [u8; 4] = [0x22, 0xC5, 0x5E, 255];

#[derive(Clone, Default)]
pub struct Mixeur {
    /// L'égaliseur ouvert à la place de la console, et sa bande choisie.
    eq: Option<Cible>,
    bande: usize,
    /// La poignée tirée sur la courbe.
    tiree: Option<usize>,
}

/// L'égaliseur qu'on règle : celui d'une piste, ou celui de la sortie.
#[derive(Clone, Debug, PartialEq)]
enum Cible {
    Piste(String),
    Sortie,
}

impl Cible {
    fn lire(&self) -> Egaliseur {
        match self {
            Cible::Piste(n) => mixeur().reglage(n).eq,
            Cible::Sortie => mixeur().egaliseur(),
        }
    }

    fn ecrire(&self, e: Egaliseur) {
        match self {
            Cible::Piste(n) => {
                let r = mixeur().reglage(n);
                if r.eq != e {
                    mixeur().regler(n, Reglage { eq: e, ..r });
                }
            }
            Cible::Sortie => mixeur().regler_egaliseur(&e),
        }
    }

    fn nom(&self) -> &str {
        match self {
            Cible::Piste(n) => n,
            Cible::Sortie => "Sortie",
        }
    }
}

/// L'éditeur ouvert se retrouve : `eq = Radio` (une piste) ou `eq_sortie = oui`, et
/// `bande = 2`.
fn ouvrir(e: &Etat) -> Box<dyn Contexte> {
    let eq = if e.texte("eq_sortie") == Some("oui") { Some(Cible::Sortie) } else { e.texte("eq").map(|n| Cible::Piste(n.to_string())) };
    let bande = e.texte("bande").and_then(|b| b.parse().ok()).unwrap_or(0usize).min(BANDES - 1);
    Box::new(Mixeur { eq, bande, tiree: None })
}

/// L'état de la sortie, en quelques mots.
fn sortie() -> String {
    let m = mixeur();
    match (m.niveau(), m.erreur()) {
        (_, Some(e)) => e,
        (Some(_), None) if m.en_pause() => "en pause".into(),
        (Some(_), None) => format!("sortie {:.1} kHz", m.frequence() as f32 / 1000.0),
        (None, None) => "sortie fermée".into(),
    }
}

fn balance_texte(b: f32) -> String {
    match (b * 100.0).round() as i32 {
        0 => "centre".to_string(),
        v if v < 0 => format!("G {}", -v),
        v => format!("D {v}"),
    }
}

/// La couleur d'un vumètre : vert, l'accent près du maximum, rouge à la saturation.
fn teinte(cx: &Cx, crete: f32, v: f32) -> [u8; 4] {
    if crete >= 0.99 {
        ROUGE
    } else if v > 0.85 {
        cx.couleurs().accent
    } else {
        VERT
    }
}

impl Contexte for Mixeur {
    fn cle(&self) -> &'static str {
        CLE
    }

    fn dupliquer(&self) -> Box<dyn Contexte> {
        Box::new(self.clone())
    }

    fn ecrire(&self, e: &mut Etat) {
        match &self.eq {
            Some(Cible::Sortie) => e.mettre("eq_sortie", "oui"),
            Some(Cible::Piste(n)) => e.mettre("eq", n),
            None => return,
        }
        e.mettre("bande", self.bande);
    }

    /// Les vumètres s'animent tant qu'une voie joue.
    fn anime(&self, _c: &Commun) -> bool {
        mixeur().actif() && !mixeur().en_pause()
    }

    fn dessiner(&mut self, cx: &mut Cx, c: &mut Commun, p: &Placee) -> String {
        let r = p.contenu;
        let (d, col) = (cx.dp, cx.couleurs());
        cx.dl.rrect(r, 0.0, col.surface);
        let base = sub_id(MIXEUR, p.id as usize);
        let pistes = mixeur().pistes();
        let n = pistes.len() + 1;
        // L'en-tête, puis les tranches
        let h = cx.commande();
        let haut = h + 2.0 * cx.coin();
        let zone = Rect::new(r.x + cx.coin(), r.y + haut, r.w - 2.0 * cx.coin(), r.h - haut - cx.coin());
        let large = (zone.w / n as f32).min(120.0 * d);
        if let Some(cible) = self.eq.clone() {
            if !self.editeur(cx, c, r, base, &cible) {
                self.eq = None;
            }
            return format!("égaliseur · {}", cible.nom());
        }
        let mut ouvre = None;
        if large < 76.0 * d || zone.h < 280.0 * d {
            ouvre = liste(cx, c, r, base, &pistes);
        } else {
            entete(cx, r, base);
            // Les tranches, centrées ; Général à droite, après un filet
            let x0 = zone.x + (zone.w - large * n as f32) / 2.0;
            for (k, piste) in pistes.iter().enumerate() {
                if tranche_piste(cx, Rect::new(x0 + k as f32 * large, zone.y, large, zone.h), sub_id(base, hachage(&piste.nom)), piste) {
                    ouvre = Some(Cible::Piste(piste.nom.clone()));
                }
            }
            let g = Rect::new(x0 + (n - 1) as f32 * large, zone.y, large, zone.h);
            if n > 1 {
                cx.dl.rrect(Rect::new(g.x, g.y, d.max(1.0), g.h), 0.0, col.ligne);
            }
            if tranche_generale(cx, g, sub_id(base, 1)) {
                ouvre = Some(Cible::Sortie);
            }
        }
        if ouvre.is_some() {
            self.eq = ouvre;
            self.tiree = None;
        }
        let joue = pistes.iter().filter(|p| p.niveau.is_some()).count();
        match pistes.len() {
            0 => "aucune piste".into(),
            t => format!("{joue}/{t} en lecture · {}", sortie()),
        }
    }
}

/// La voie d'essai et l'état de la sortie, en haut.
fn entete(cx: &mut Cx, r: Rect, base: Id) {
    let (d, col, h) = (cx.dp, cx.couleurs(), cx.commande());
    let x = r.x + cx.coin();
    let y = r.y + cx.coin();
    let essai = mixage::essai_actif();
    let label = "Voie d'essai";
    if cx.pill(sub_id(base, 2), x, y, label, essai) {
        if let Err(e) = mixage::essai(!essai) {
            crate::android::log(&format!("voie d'essai : {e}"));
        }
    }
    let tx = x + cx.pill_width(label) + 12.0 * d;
    cx.clip(Some(Rect::new(tx, y, (r.right() - tx).max(0.0), h)));
    let info = if mixeur().pistes().is_empty() { format!("{} · une radio, un fichier audio ou la voie d'essai ouvrent une piste", sortie()) } else { sortie() };
    cx.text_at(tx, y + h / 2.0, &info, 13.0, if mixeur().erreur().is_some() { ROUGE } else { col.pale });
    cx.clip(None);
}

/// Le haut d'une tranche (nom et état) ; renvoie le bas de cet en-tête.
fn titre(cx: &mut Cx, r: Rect, nom: &str, info: &str) -> f32 {
    let (d, col) = (cx.dp, cx.couleurs());
    cx.clip(Some(r));
    cx.centered(Rect::new(r.x, r.y, r.w, 22.0 * d), nom, 15.0, col.texte);
    cx.centered(Rect::new(r.x, r.y + 20.0 * d, r.w, 18.0 * d), info, 11.0, col.pale);
    cx.clip(None);
    r.y + 44.0 * d
}

/// Un curseur vertical de volume, avec le vumètre stéréo à côté. Rend la valeur.
fn fader(cx: &mut Cx, zone: Rect, id: Id, valeur: f32, niveau: Option<[f32; 2]>, eteint: bool) -> f32 {
    let (d, col) = (cx.dp, cx.couleurs());
    let bouton = (14.0 * d, 30.0 * d);
    let (y0, y1) = (zone.y + bouton.0 / 2.0, zone.bottom() - bouton.0 / 2.0);
    let mut v = valeur;
    if let Some(g) = cx.ui.drag(id, zone) {
        v = (((y1 - g.y) / (y1 - y0)).clamp(0.0, 1.0) * 100.0).round() / 100.0;
    }
    // Le rail, et sa partie active
    let xr = zone.cx() - 8.0 * d;
    let t = 4.0 * d;
    let at = y1 - v * (y1 - y0);
    cx.dl.rrect(Rect::new(xr - t / 2.0, y0, t, y1 - y0), t / 2.0, col.ligne);
    cx.dl.rrect(Rect::new(xr - t / 2.0, at, t, y1 - at), t / 2.0, if eteint { col.pale } else { col.accent });
    // Des graduations tous les quarts
    for k in 0..=4 {
        let y = y1 - k as f32 * (y1 - y0) / 4.0;
        cx.dl.rrect(Rect::new(xr - 12.0 * d, y - 0.5 * d, 6.0 * d, d.max(1.0)), 0.0, col.ligne);
    }
    // Le vumètre : deux barres montantes, gauche et droite
    let e = 5.0 * d;
    for ch in 0..2 {
        let b = Rect::new(zone.cx() + 6.0 * d + ch as f32 * (e + 3.0 * d), y0, e, y1 - y0);
        cx.dl.rrect(b, e / 2.0, col.ligne);
        if let Some(n) = niveau {
            let m = mixage::vumetre(n[ch]);
            if m > 0.0 {
                let hm = (b.h * m).max(e);
                cx.dl.rrect(Rect::new(b.x, b.bottom() - hm, e, hm), e / 2.0, teinte(cx, n[ch], m));
            }
        }
    }
    // Le bouton du curseur
    let grand = if cx.ui.pressed(id) { 2.0 * d } else { 0.0 };
    let k = Rect::new(xr - bouton.1 / 2.0 - grand, at - bouton.0 / 2.0 - grand, bouton.1 + 2.0 * grand, bouton.0 + 2.0 * grand);
    cx.ombre(k, 4.0 * d, 4.0);
    cx.dl.rrect(k, 4.0 * d, if eteint { col.pale } else { col.accent });
    cx.dl.rrect(Rect::new(k.x + 5.0 * d, k.cy() - 0.75 * d, k.w - 10.0 * d, 1.5 * d), 0.0, col.sur_accent);
    v
}

/// La balance : un rail horizontal, le centre marqué. Rend la valeur.
fn balance(cx: &mut Cx, zone: Rect, id: Id, valeur: f32) -> f32 {
    let (d, col) = (cx.dp, cx.couleurs());
    let mut v = valeur;
    let bouton = 9.0 * d;
    let (x0, x1) = (zone.x + bouton + 4.0 * d, zone.right() - bouton - 4.0 * d);
    if let Some(g) = cx.ui.drag(id, zone) {
        v = ((((g.x - x0) / (x1 - x0)).clamp(0.0, 1.0) * 2.0 - 1.0) * 20.0).round() / 20.0;
    }
    cx.centered(Rect::new(zone.x, zone.y, zone.w, 16.0 * d), &balance_texte(v), 11.0, col.pale);
    let y = zone.bottom() - 12.0 * d;
    let t = 3.0 * d;
    cx.dl.rrect(Rect::new(x0, y - t / 2.0, x1 - x0, t), t / 2.0, col.ligne);
    let mx = (x0 + x1) / 2.0;
    cx.dl.rrect(Rect::new(mx - 0.75 * d, y - 6.0 * d, 1.5 * d, 12.0 * d), 0.0, col.pale);
    let at = mx + v * (x1 - x0) / 2.0;
    cx.dl.rrect(Rect::new(at - bouton, y - bouton, 2.0 * bouton, 2.0 * bouton), bouton, col.accent);
    v
}

/// Une tranche de piste ; renvoie vrai si sa vignette d'égaliseur est touchée.
fn tranche_piste(cx: &mut Cx, r: Rect, base: Id, p: &Piste) -> bool {
    let d = cx.dp;
    let col = cx.couleurs();
    let r = r.inset(4.0 * d);
    let info = if p.niveau.is_some() { "joue" } else { "silencieuse" };
    let haut = titre(cx, r, &p.nom, info);
    let mut g: Reglage = p.reglage;
    // Le bas : les bascules, puis la balance, puis le volume en chiffres
    let hb = 30.0 * d;
    let yb = r.bottom() - hb;
    let lb = (r.w - 2.0 * 3.0 * d) / 3.0;
    for (i, (lettre, v)) in [("M", &mut g.muet), ("S", &mut g.solo), ("P", &mut g.prioritaire)].into_iter().enumerate() {
        let c = Rect::new(r.x + i as f32 * (lb + 3.0 * d), yb, lb, hb);
        if cx.ui.click(sub_id(base, 10 + i), c, None) {
            *v = !*v;
        }
        let fond = if *v {
            if i == 0 {
                ROUGE
            } else {
                col.accent
            }
        } else if cx.ui.pressed(sub_id(base, 10 + i)) {
            col.ligne
        } else {
            col.puce
        };
        cx.dl.rrect(c, cx.theme.rayon(hb).min(8.0 * d), fond);
        cx.centered(c, lettre, 13.0, if *v { col.sur_accent } else { col.texte });
    }
    let yz = yb - 6.0 * d - 40.0 * d;
    g.balance = balance(cx, Rect::new(r.x, yz, r.w, 40.0 * d), sub_id(base, 2), g.balance);
    let ve = Rect::new(r.x, yz - 6.0 * d - VIGNETTE * d, r.w, VIGNETTE * d);
    let ouvre = vignette(cx, ve, sub_id(base, 20), &g.eq);
    let yv = ve.y - 22.0 * d;
    cx.centered(Rect::new(r.x, yv, r.w, 20.0 * d), &format!("{:.0} %", g.gain * 100.0), 13.0, col.texte);
    let eteint = g.muet;
    g.gain = fader(cx, Rect::new(r.x, haut, r.w, (yv - 6.0 * d - haut).max(20.0 * d)), sub_id(base, 1), g.gain, p.niveau, eteint);
    if g != p.reglage {
        mixeur().regler(&p.nom, g);
    }
    ouvre
}

/// La tranche Général ; renvoie vrai si sa vignette d'égaliseur est touchée.
fn tranche_generale(cx: &mut Cx, r: Rect, base: Id) -> bool {
    let d = cx.dp;
    let col = cx.couleurs();
    let m = mixeur();
    let r = r.inset(4.0 * d);
    let haut = titre(cx, r, "Général", if m.niveau().is_some() { "sortie" } else { "fermée" });
    // La vignette à la hauteur de celles des pistes
    let ve = Rect::new(r.x, r.bottom() - 30.0 * d - 6.0 * d - 40.0 * d - 6.0 * d - VIGNETTE * d, r.w, VIGNETTE * d);
    let ouvre = vignette(cx, ve, sub_id(base, 20), &m.egaliseur());
    let bas = ve.y;
    let avant = m.general.get();
    cx.centered(Rect::new(r.x, bas - 22.0 * d, r.w, 20.0 * d), &format!("{:.0} %", avant * 100.0), 13.0, col.texte);
    let v = fader(cx, Rect::new(r.x, haut, r.w, (bas - 28.0 * d - haut).max(20.0 * d)), base, avant, m.niveau(), false);
    if v != avant {
        m.general.set(v);
    }
    ouvre
}

// --- La console en liste, quand les tranches ne tiennent pas ---------------------

/// La console en liste ; renvoie l'égaliseur à ouvrir, si une vignette est touchée.
fn liste(cx: &mut Cx, c: &mut Commun, r: Rect, base: Id, pistes: &[Piste]) -> Option<Cible> {
    let scroll = sub_id(base, 3);
    let mut b = cx.body(scroll, Rect::new(r.x, r.y, r.w, r.h), &c.scrolls);
    let m = mixeur();
    let mut essai = mixage::essai_actif();
    cx.toggle(&mut b, sub_id(scroll, 3), "Voie d'essai (arpège)", &mut essai);
    if essai != mixage::essai_actif() {
        if let Err(e) = mixage::essai(essai) {
            crate::android::log(&format!("voie d'essai : {e}"));
        }
    }
    if pistes.is_empty() {
        cx.paragraph(&mut b, "Aucune piste : une radio, un fichier audio ou la voie d'essai en ouvrent une.");
    }
    let mut ouvre = None;
    for p in pistes {
        if piste_liste(cx, &mut b, sub_id(scroll, hachage(&p.nom)), p) {
            ouvre = Some(Cible::Piste(p.nom.clone()));
        }
    }
    let ligne = (cx.theme.ligne * cx.dp).round();
    entete_liste(cx, &mut b, "Général", &sortie(), m.niveau(), ligne);
    let mut g = m.general.get();
    let avant = g;
    let shown = format!("{:.0} %", g * 100.0);
    cx.slider(&mut b, sub_id(scroll, 4), "Volume général", &shown, &mut g, 0.0, 1.0);
    if g != avant {
        m.general.set((g * 20.0).round() / 20.0);
    }
    if vignette_liste(cx, &mut b, sub_id(scroll, 5), &m.egaliseur()) {
        ouvre = Some(Cible::Sortie);
    }
    if let Some(e) = m.erreur() {
        cx.paragraph(&mut b, &e);
    }
    cx.body_end(b, &mut c.scrolls);
    ouvre
}

/// Le nom d'une piste, un texte pâle et son vumètre (deux barres, gauche et droite).
fn entete_liste(cx: &mut Cx, b: &mut Body, nom: &str, info: &str, niveau: Option<[f32; 2]>, ligne: f32) {
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
            if v > 0.0 {
                cx.dl.rrect(Rect::new(x0, r.y, (large * v).max(e), e), e / 2.0, teinte(cx, n[ch], v));
            }
        }
    }
    b.y += ligne;
}

/// Une piste dans la liste ; renvoie vrai si sa vignette d'égaliseur est touchée.
fn piste_liste(cx: &mut Cx, b: &mut Body, base: Id, p: &Piste) -> bool {
    let ligne = (cx.theme.ligne * cx.dp).round();
    let info = if p.niveau.is_some() { "joue" } else { "silencieuse" };
    entete_liste(cx, b, &p.nom, info, p.niveau, ligne);
    let mut r: Reglage = p.reglage;
    let shown = format!("{:.0} %", r.gain * 100.0);
    cx.slider(b, sub_id(base, 0), "Volume", &shown, &mut r.gain, 0.0, 1.0);
    r.gain = (r.gain * 20.0).round() / 20.0;
    let shown = balance_texte(r.balance);
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
    let ouvre = vignette_liste(cx, b, sub_id(base, 8), &r.eq);
    if r != p.reglage {
        mixeur().regler(&p.nom, r);
    }
    ouvre
}

// --- L'égaliseur -------------------------------------------------------------

/// La hauteur de la vignette d'égaliseur d'une tranche, en dp.
const VIGNETTE: f32 = 34.0;
/// L'échelle verticale de la courbe, en dB de part et d'autre de 0.
const ECHELLE_DB: f32 = 15.0;
/// La fréquence d'échantillonnage des courbes affichées.
const FS: f32 = 48_000.0;

/// Les couleurs des cinq bandes.
const TEINTES_BANDES: [[u8; 4]; BANDES] = [[0xEF, 0x44, 0x44, 255], [0xF5, 0x9E, 0x0B, 255], [0x22, 0xC5, 0x5E, 255], [0x14, 0xB8, 0xA6, 255], [0xA8, 0x55, 0xF7, 255]];

/// La place d'une fréquence sur un axe logarithmique de 20 Hz à 20 kHz, de 0 à 1.
fn position(f: f32) -> f32 {
    (f / egaliseur::FREQ_MIN).ln() / (egaliseur::FREQ_MAX / egaliseur::FREQ_MIN).ln()
}

fn frequence_en(t: f32) -> f32 {
    egaliseur::FREQ_MIN * (egaliseur::FREQ_MAX / egaliseur::FREQ_MIN).powf(t.clamp(0.0, 1.0))
}

/// « 80 Hz », « 1,2 kHz ».
fn hertz(f: f32) -> String {
    if f >= 1000.0 {
        let k = f / 1000.0;
        if k >= 10.0 { format!("{k:.0} kHz") } else { format!("{k:.1} kHz").replace('.', ",") }
    } else {
        format!("{f:.0} Hz")
    }
}

fn decibels(g: f32) -> String {
    format!("{g:+.1} dB").replace('.', ",")
}

/// Trace la courbe de réponse de `e` dans `r` (± `ECHELLE_DB`), avec son épaisseur.
fn courbe(cx: &mut Cx, r: Rect, e: &Egaliseur, epais: f32, teinte: [u8; 4]) {
    let n = (r.w / (3.0 * cx.dp)).clamp(24.0, 160.0) as usize;
    let y = |db: f32| r.cy() - db.clamp(-ECHELLE_DB, ECHELLE_DB) / ECHELLE_DB * (r.h / 2.0);
    let mut avant: Option<[f32; 2]> = None;
    for i in 0..=n {
        let t = i as f32 / n as f32;
        let p = [r.x + t * r.w, y(e.reponse(frequence_en(t), FS))];
        if let Some(a) = avant {
            cx.dl.line(a, p, epais, teinte);
        }
        avant = Some(p);
    }
}

/// La vignette de l'égaliseur : la courbe en petit, « EQ » si elle est à plat.
/// Renvoie vrai au toucher.
fn vignette(cx: &mut Cx, r: Rect, id: Id, e: &Egaliseur) -> bool {
    let (d, col) = (cx.dp, cx.couleurs());
    let touche = cx.ui.click(id, r, None);
    cx.dl.rrect(r, cx.theme.rayon(r.h).min(8.0 * d), if cx.ui.pressed(id) { col.ligne } else { col.puce });
    let dedans = r.inset(4.0 * d);
    cx.dl.rrect(Rect::new(dedans.x, dedans.cy() - 0.5 * d, dedans.w, d.max(1.0)), 0.0, col.ligne);
    if e.neutre() {
        cx.centered(r, if e.actif { "EQ" } else { "EQ éteint" }, 11.0, col.pale);
    } else {
        courbe(cx, dedans, e, 1.5 * d, col.accent);
    }
    touche
}

/// La vignette, sur toute la largeur d'une liste.
fn vignette_liste(cx: &mut Cx, b: &mut Body, id: Id, e: &Egaliseur) -> bool {
    let r = Rect::new(b.x, b.y, b.w, 40.0 * cx.dp);
    b.y += r.h + cx.ecart();
    let vue = b.rect();
    if r.bottom() < vue.y || r.y > vue.bottom() {
        return false;
    }
    vignette(cx, r, id, e)
}

impl Mixeur {
    /// L'éditeur de l'égaliseur `cible`, à la place de la console. Renvoie faux pour
    /// revenir à la console.
    fn editeur(&mut self, cx: &mut Cx, c: &mut Commun, r: Rect, base: Id, cible: &Cible) -> bool {
        let (d, col, h) = (cx.dp, cx.couleurs(), cx.commande());
        let id = sub_id(base, 7);
        let mut e = cible.lire();
        let avant = e;
        // L'en-tête : retour, nom, actif, à plat
        let (x0, y0) = (r.x + cx.coin(), r.y + cx.coin());
        let mut garder = !cx.pill(sub_id(id, 1), x0, y0, "Console", false);
        let mut x = x0 + cx.pill_width("Console") + cx.ecart();
        if cx.pill(sub_id(id, 2), x, y0, "Actif", e.actif) {
            e.actif = !e.actif;
        }
        x += cx.pill_width("Actif") + cx.ecart();
        if cx.pill(sub_id(id, 3), x, y0, "À plat", false) {
            e.a_plat();
        }
        x += cx.pill_width("À plat") + cx.ecart();
        let titre = format!("Égaliseur · {}", cible.nom());
        cx.clip(Some(Rect::new(x, y0, (r.right() - x).max(0.0), h)));
        cx.text_at(x + 4.0 * d, y0 + h / 2.0, &titre, 15.0, col.texte);
        cx.clip(None);
        // La courbe et ses poignées
        let haut = y0 + h + cx.ecart();
        let gh = ((r.bottom() - haut) * 0.5).clamp(140.0 * d, 360.0 * d).min(r.bottom() - haut - 40.0 * d).max(60.0 * d);
        let cadre = Rect::new(r.x + cx.coin(), haut, r.w - 2.0 * cx.coin(), gh);
        let g = cadre.inset(10.0 * d);
        cx.dl.rrect(cadre, cx.coin().min(10.0 * d), col.puce);
        let y_db = |db: f32| g.cy() - db / ECHELLE_DB * (g.h / 2.0);
        for db in [-12.0, -6.0, 0.0, 6.0, 12.0] {
            let y = y_db(db);
            cx.dl.line([g.x, y], [g.right(), y], if db == 0.0 { 1.5 * d } else { d.max(1.0) }, col.ligne);
        }
        for f in [50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0, 10_000.0] {
            let x = g.x + position(f) * g.w;
            cx.dl.line([x, g.y], [x, g.bottom()], d.max(1.0), col.ligne);
        }
        for f in [100.0, 1000.0, 10_000.0] {
            let x = g.x + position(f) * g.w;
            cx.mono_at(x + 3.0 * d, g.bottom() - 7.0 * d, &hertz(f), 10.0, col.pale);
        }
        cx.mono_at(g.x + 3.0 * d, y_db(12.0) + 7.0 * d, "+12", 10.0, col.pale);
        cx.mono_at(g.x + 3.0 * d, y_db(-12.0) - 7.0 * d, "-12", 10.0, col.pale);
        courbe(cx, g, &e, 2.5 * d, if e.actif { col.accent } else { col.pale });
        // Tirer : la poignée la plus proche du doigt suit, en fréquence et en gain
        let point = |b: &egaliseur::Bande| [g.x + position(b.frequence) * g.w, y_db(b.gain)];
        match cx.ui.drag(sub_id(id, 4), cadre) {
            Some(t) => {
                let k = *self.tiree.get_or_insert_with(|| {
                    (0..BANDES).min_by(|&a, &b| {
                        let (pa, pb) = (point(&e.bandes[a]), point(&e.bandes[b]));
                        ((pa[0] - t.x).hypot(pa[1] - t.y)).total_cmp(&(pb[0] - t.x).hypot(pb[1] - t.y))
                    }).unwrap_or(0)
                });
                self.bande = k;
                let bd = &mut e.bandes[k];
                bd.frequence = frequence_en((t.x - g.x) / g.w).round();
                bd.gain = ((g.cy() - t.y) / (g.h / 2.0) * ECHELLE_DB).clamp(-egaliseur::GAIN_MAX, egaliseur::GAIN_MAX);
                bd.gain = (bd.gain * 2.0).round() / 2.0;
                e.actif = true;
            }
            None => self.tiree = None,
        }
        for k in 0..BANDES {
            let p = point(&e.bandes[k]);
            let rayon = if k == self.bande { 11.0 * d } else { 8.0 * d };
            let teinte = TEINTES_BANDES[k];
            cx.dl.rrect(Rect::new(p[0] - rayon - 2.0 * d, p[1] - rayon - 2.0 * d, 2.0 * (rayon + 2.0 * d), 2.0 * (rayon + 2.0 * d)), rayon + 2.0 * d, col.surface);
            cx.dl.rrect(Rect::new(p[0] - rayon, p[1] - rayon, 2.0 * rayon, 2.0 * rayon), rayon, teinte);
            cx.centered(Rect::new(p[0] - rayon, p[1] - rayon, 2.0 * rayon, 2.0 * rayon), &(k + 1).to_string(), 11.0, [255, 255, 255, 255]);
        }
        // La bande choisie : ses réglages fins
        let reste = Rect::new(r.x, cadre.bottom() + cx.ecart(), r.w, (r.bottom() - cadre.bottom() - cx.ecart()).max(0.0));
        let scroll = sub_id(id, 5);
        let mut b = cx.body(scroll, reste, &c.scrolls);
        let (yb, mut xb) = (b.y, b.x);
        for k in 0..BANDES {
            let l = (k + 1).to_string();
            if cx.pill(sub_id(id, 10 + k), xb, yb, &l, k == self.bande) {
                self.bande = k;
            }
            xb += cx.pill_width(&l) + cx.ecart();
        }
        b.y = yb + h + cx.ecart();
        let k = self.bande.min(BANDES - 1);
        let forme = match FORMES[k] {
            Forme::Grave => "étagère grave",
            Forme::Cloche => "cloche",
            Forme::Aigu => "étagère aiguë",
        };
        let bd = &mut e.bandes[k];
        cx.paragraph(&mut b, &format!("Bande {} · {forme} · {} · {}", k + 1, hertz(bd.frequence), decibels(bd.gain)));
        let mut t = position(bd.frequence);
        let avant_t = t;
        cx.slider(&mut b, sub_id(id, 20), "Fréquence", &hertz(bd.frequence), &mut t, 0.0, 1.0);
        if t != avant_t {
            bd.frequence = frequence_en(t).round();
        }
        let shown = decibels(bd.gain);
        cx.slider(&mut b, sub_id(id, 21), "Gain", &shown, &mut bd.gain, -egaliseur::GAIN_MAX, egaliseur::GAIN_MAX);
        bd.gain = (bd.gain * 2.0).round() / 2.0;
        // La largeur : sur une échelle logarithmique, plus fine vers le bas
        let (q0, q1) = (egaliseur::Q_MIN.ln(), egaliseur::Q_MAX.ln());
        let mut u = (bd.q.ln() - q0) / (q1 - q0);
        let avant_u = u;
        let nom_q = if FORMES[k] == Forme::Cloche { "Largeur (Q)" } else { "Pente (Q)" };
        cx.slider(&mut b, sub_id(id, 22), nom_q, &format!("{:.2}", bd.q).replace('.', ","), &mut u, 0.0, 1.0);
        if u != avant_u {
            bd.q = ((q0 + u * (q1 - q0)).exp() * 100.0).round() / 100.0;
        }
        if cx.button(&mut b, sub_id(id, 23), "Remettre la bande à 0 dB") {
            bd.gain = 0.0;
        }
        cx.body_end(b, &mut c.scrolls);
        if e != avant {
            cible.ecrire(e);
        }
        // Une piste disparue ferme l'éditeur
        if let Cible::Piste(n) = cible {
            garder &= mixeur().pistes().iter().any(|p| &p.nom == n);
        }
        garder
    }
}

// --- L'applet de la barre -------------------------------------------------------

/// L'applet Mixeur : dans la barre, l'icône et le vumètre de la sortie ; dans le volet,
/// le volume, la coupure et l'égaliseur (allumé ou non) de chaque piste, le volume
/// général et l'égaliseur de la sortie.
struct AppletMixeur;

fn applet() -> Box<dyn crate::applet::Applet> {
    Box::new(AppletMixeur)
}

impl crate::applet::Applet for AppletMixeur {
    fn largeur(&mut self, _cx: &mut Cx, _c: &Commun, h: f32, _compact: bool) -> f32 {
        h * 1.15
    }

    fn barre(&mut self, cx: &mut Cx, _c: &mut Commun, r: Rect, _id: Id, _compact: bool) {
        let (d, col) = (cx.dp, cx.couleurs());
        let m = mixeur();
        let ic = (r.h * 0.5).round();
        let x0 = r.x + (r.h * 0.8 - ic) / 2.0;
        // Coupé (général à zéro) : l'icône pâlit
        let couleur = if m.general.get() <= 0.0 { col.pale } else { col.texte };
        cx.icon(Rect::new(x0, r.cy() - ic / 2.0, ic, ic), Icon::Mixeur, couleur);
        // Le vumètre de la sortie : deux barres verticales, gauche et droite
        let (e, ep) = (3.0 * d, 2.0 * d);
        let haut = r.h * 0.5;
        let x = r.x + r.h * 0.8 + 2.0 * d;
        for ch in 0..2 {
            let b = Rect::new(x + ch as f32 * (e + ep), r.cy() - haut / 2.0, e, haut);
            cx.dl.rrect(b, e / 2.0, col.ligne);
            if let Some(n) = m.niveau().filter(|_| !m.en_pause()) {
                let v = mixage::vumetre(n[ch]);
                if v > 0.0 {
                    let hv = (haut * v).max(e);
                    cx.dl.rrect(Rect::new(b.x, b.bottom() - hv, e, hv), e / 2.0, teinte(cx, n[ch], v));
                }
            }
        }
    }

    fn hauteur_volet(&mut self, cx: &mut Cx, _c: &Commun, _w: f32) -> f32 {
        let d = cx.dp;
        let ligne = (cx.theme.ligne * d).round();
        let curseur = (64.0 * d * cx.theme.texte.max(1.0)).round();
        let n = mixeur().pistes().len() as f32;
        let vide = if n == 0.0 { 2.0 * 22.0 * d * cx.theme.texte + 8.0 * d } else { 0.0 };
        16.0 * d + n * (ligne + curseur + cx.commande() + 1.5 * cx.ecart()) + 2.0 * ligne + curseur + vide + cx.commande() + 2.0 * (cx.theme.marge * d).round() + 20.0 * d
    }

    fn volet(&mut self, cx: &mut Cx, c: &mut Commun, r: Rect, id: Id) -> bool {
        let scroll = sub_id(id, 0);
        let m = mixeur();
        let ligne = (cx.theme.ligne * cx.dp).round();
        let mut b = cx.body(scroll, r, &c.scrolls);
        let pistes = m.pistes();
        if pistes.is_empty() {
            cx.paragraph(&mut b, "Aucune piste : une radio, un fichier audio ou la voie d'essai en ouvrent une.");
        }
        for p in &pistes {
            let base = sub_id(scroll, hachage(&p.nom));
            entete_liste(cx, &mut b, &p.nom, if p.niveau.is_some() { "joue" } else { "silencieuse" }, p.niveau, ligne);
            let mut g = p.reglage;
            let shown = format!("{:.0} %", g.gain * 100.0);
            cx.slider(&mut b, sub_id(base, 0), "Volume", &shown, &mut g.gain, 0.0, 1.0);
            g.gain = (g.gain * 20.0).round() / 20.0;
            // Muet, et l'égaliseur allumé ou non (réglé dans le Mixeur)
            let y = b.y + cx.ecart() * 0.5;
            if cx.pill(sub_id(base, 1), b.x, y, "Muet", g.muet) {
                g.muet = !g.muet;
            }
            let x = b.x + cx.pill_width("Muet") + cx.ecart();
            if cx.pill(sub_id(base, 2), x, y, "Égaliseur", g.eq.actif) {
                g.eq.actif = !g.eq.actif;
            }
            b.y = y + cx.commande() + cx.ecart();
            if g != p.reglage {
                m.regler(&p.nom, g);
            }
        }
        entete_liste(cx, &mut b, "Général", &sortie(), m.niveau(), ligne);
        let mut g = m.general.get();
        let avant = g;
        let shown = format!("{:.0} %", g * 100.0);
        cx.slider(&mut b, sub_id(scroll, 1), "Volume général", &shown, &mut g, 0.0, 1.0);
        if g != avant {
            m.general.set((g * 20.0).round() / 20.0);
        }
        let mut eq = m.egaliseur();
        let avant = eq.actif;
        cx.toggle(&mut b, sub_id(scroll, 3), "Égaliseur de la sortie", &mut eq.actif);
        if eq.actif != avant {
            m.regler_egaliseur(&eq);
        }
        let ouvrir = cx.button(&mut b, sub_id(scroll, 2), "Ouvrir le Mixeur");
        if ouvrir {
            c.demandes.push(crate::contexte::Demande::Montrer { cle: CLE });
        }
        cx.body_end(b, &mut c.scrolls);
        ouvrir
    }

    /// Le vumètre s'anime tant qu'une voie joue.
    fn anime(&self, _c: &Commun) -> bool {
        mixeur().actif() && !mixeur().en_pause()
    }
}
