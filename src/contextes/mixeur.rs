//! Le contexte Mixeur : la console du mixeur audio (`crate::mixage`), partagé par
//! toute l'application. Chaque piste connue (Radio, Fichier, Essai…) a sa tranche :
//! nom, vumètre stéréo, volume au curseur vertical, balance, et les bascules Muet,
//! Solo, Prioritaire. La tranche Général, à droite, règle la sortie.
//!
//! Quand la fenêtre est trop étroite ou trop basse pour des tranches, la console se
//! range en liste (curseurs horizontaux), qui défile.

use super::hachage;
use crate::contexte::{Commun, Contexte, Etat, Genre};
use crate::draw::Rect;
use crate::mixage::{self, mixeur, Piste, Reglage};
use crate::surface::Placee;
use crate::ui::{id, sub_id, Id};
use crate::widgets::{Body, Cx, Icon};

pub const CLE: &str = "mixeur";
pub const GENRE: Genre = Genre { cle: CLE, nom: "Mixeur", legende: "Le volume de chaque piste audio", icone: Icon::Mixeur, ouvrir };

const MIXEUR: Id = id("mixeur");
const ROUGE: [u8; 4] = [0xE0, 0x3A, 0x3A, 255];
const VERT: [u8; 4] = [0x22, 0xC5, 0x5E, 255];

#[derive(Clone, Default)]
pub struct Mixeur;

fn ouvrir(_: &Etat) -> Box<dyn Contexte> {
    Box::new(Mixeur)
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
        Box::new(Mixeur)
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
        // L'en-tête : à droite de la pastille + de la fenêtre
        let h = cx.commande();
        let haut = h + 2.0 * cx.coin();
        let zone = Rect::new(r.x + cx.coin(), r.y + haut, r.w - 2.0 * cx.coin(), r.h - haut - cx.coin());
        let large = (zone.w / n as f32).min(120.0 * d);
        if large < 76.0 * d || zone.h < 280.0 * d {
            liste(cx, c, r, base, &pistes);
        } else {
            entete(cx, r, base);
            // Les tranches, centrées ; Général à droite, après un filet
            let x0 = zone.x + (zone.w - large * n as f32) / 2.0;
            for (k, piste) in pistes.iter().enumerate() {
                tranche_piste(cx, Rect::new(x0 + k as f32 * large, zone.y, large, zone.h), sub_id(base, hachage(&piste.nom)), piste);
            }
            let g = Rect::new(x0 + (n - 1) as f32 * large, zone.y, large, zone.h);
            if n > 1 {
                cx.dl.rrect(Rect::new(g.x, g.y, d.max(1.0), g.h), 0.0, col.ligne);
            }
            tranche_generale(cx, g, sub_id(base, 1));
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
    let x = r.x + 2.0 * cx.coin() + h;
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

fn tranche_piste(cx: &mut Cx, r: Rect, base: Id, p: &Piste) {
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
    let yv = yz - 22.0 * d;
    cx.centered(Rect::new(r.x, yv, r.w, 20.0 * d), &format!("{:.0} %", g.gain * 100.0), 13.0, col.texte);
    let eteint = g.muet;
    g.gain = fader(cx, Rect::new(r.x, haut, r.w, (yv - 6.0 * d - haut).max(20.0 * d)), sub_id(base, 1), g.gain, p.niveau, eteint);
    if g != p.reglage {
        mixeur().regler(&p.nom, g);
    }
}

fn tranche_generale(cx: &mut Cx, r: Rect, base: Id) {
    let d = cx.dp;
    let col = cx.couleurs();
    let m = mixeur();
    let r = r.inset(4.0 * d);
    let haut = titre(cx, r, "Général", if m.niveau().is_some() { "sortie" } else { "fermée" });
    let bas = r.bottom() - 30.0 * d - 6.0 * d - 40.0 * d;
    let avant = m.general.get();
    cx.centered(Rect::new(r.x, bas - 22.0 * d, r.w, 20.0 * d), &format!("{:.0} %", avant * 100.0), 13.0, col.texte);
    let v = fader(cx, Rect::new(r.x, haut, r.w, (bas - 28.0 * d - haut).max(20.0 * d)), base, avant, m.niveau(), false);
    if v != avant {
        m.general.set(v);
    }
}

// --- La console en liste, quand les tranches ne tiennent pas ---------------------

fn liste(cx: &mut Cx, c: &mut Commun, r: Rect, base: Id, pistes: &[Piste]) {
    let scroll = sub_id(base, 3);
    let mut b = cx.body(scroll, Rect::new(r.x, r.y, r.w, r.h), &c.scrolls);
    // La première ligne laisse la pastille + de la fenêtre
    b.y += cx.commande() + cx.coin();
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
    for p in pistes {
        piste_liste(cx, &mut b, sub_id(scroll, hachage(&p.nom)), p);
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
    if let Some(e) = m.erreur() {
        cx.paragraph(&mut b, &e);
    }
    cx.body_end(b, &mut c.scrolls);
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

fn piste_liste(cx: &mut Cx, b: &mut Body, base: Id, p: &Piste) {
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
    if r != p.reglage {
        mixeur().regler(&p.nom, r);
    }
}
