//! Le contexte Télécommande : piloter un autre Flow par le Wi-Fi, ou se laisser piloter.
//!
//! - **Piloter** : chercher les Flow du réseau (ou taper une adresse), donner le code
//!   que l'autre appareil affiche, puis :
//!   - **Pavé** : le doigt déplace un curseur sur l'autre écran (plus vite, plus loin) ;
//!     une touche brève clique, « Tenir » garde le bouton enfoncé pour glisser ;
//!   - **Écran** : la zone est l'autre écran en miroir, à plusieurs doigts (les gestes
//!     de Flow y passent tels quels) ;
//!   - **Clavier** : le clavier de cet appareil écrit dans le champ choisi là-bas (le
//!     clavier de l'autre écran se cache).
//!   La ligne du haut dit l'aller-retour mesuré (`telecommande::Latence`).
//! - **Être piloté** : l'interrupteur ouvre l'écoute ; l'adresse et le code s'affichent.
//!
//! Le réseau est dans `crate::telecommande`.

use crate::clavier::{self, Edition, Frappe};
use crate::contexte::{Commun, Contexte, Etat, Genre};
use crate::draw::Rect;
use crate::surface::{FenetreId, Placee};
use crate::telecommande::{self as tc, Client, Message, Suivi};
use crate::ui::{id, sub_id, Id, Sense};
use crate::widgets::{Cx, Icon};
use std::net::Ipv4Addr;
use std::time::Instant;

pub const CLE: &str = "telecommande";
pub const GENRE: Genre = Genre { cle: CLE, nom: "Télécommande", legende: "Piloter un autre Flow par le Wi-Fi, ou se laisser piloter", icone: Icon::Telecommande, ouvrir, applet: None };

const TELE: Id = id("telecommande");
const ADRESSE: Id = id("telecommande-adresse");
const CODE: Id = id("telecommande-code");
const SAISIE: Id = id("telecommande-saisie");

const ONGLETS: [&str; 4] = ["Pavé", "Écran", "Clavier", "Délier"];

/// Le doigt qui mène le pavé : où il est, quand il s'est posé, le chemin parcouru.
struct DoigtPave {
    id: i32,
    x: f32,
    y: f32,
    debut: Instant,
    chemin: f32,
}

pub struct Telecommande {
    client: Option<Client>,
    erreur: Option<String>,
    adresse: Edition,
    code: Edition,
    onglet: usize,
    /// L'onglet Clavier est-il annoncé à l'appareil piloté ?
    clavier_annonce: bool,
    suivi: Suivi,
    pave: Option<DoigtPave>,
    tenu: bool,
    recherche: Option<Instant>,
    /// Les dernières frappes envoyées, pour les voir.
    tape: String,
    dt: f32,
}

fn ouvrir(e: &Etat) -> Box<dyn Contexte> {
    Box::new(Telecommande {
        client: None,
        erreur: None,
        adresse: Edition::new(e.texte("adresse").unwrap_or("")),
        code: Edition::default(),
        onglet: 0,
        clavier_annonce: false,
        suivi: Suivi::default(),
        pave: None,
        tenu: false,
        recherche: None,
        tape: String::new(),
        dt: 0.016,
    })
}

impl Telecommande {
    fn client(&mut self) -> Option<&mut Client> {
        if self.client.is_none() {
            match Client::new() {
                Ok(c) => self.client = Some(c),
                Err(e) => self.erreur = Some(e),
            }
        }
        self.client.as_mut()
    }

    fn lie(&self) -> bool {
        self.client.as_ref().is_some_and(|c| c.lie)
    }

    fn envoyer(&self, m: &Message) {
        if let Some(c) = &self.client {
            c.envoyer(m);
        }
    }

    /// Quitte l'onglet courant : doigts levés, bouton lâché, clavier rendu.
    fn quitter_onglet(&mut self, c: &mut Commun, k: usize) {
        for m in self.suivi.messages(&[]) {
            self.envoyer(&m);
        }
        self.pave = None;
        if std::mem::take(&mut self.tenu) {
            self.envoyer(&Message::Bouton(false));
        }
        if std::mem::take(&mut self.clavier_annonce) {
            self.envoyer(&Message::Clavier(false));
        }
        if c.clavier.cible == Some(sub_id(SAISIE, k)) {
            c.clavier.fermer();
        }
    }

    fn delier(&mut self, c: &mut Commun, k: usize) {
        self.quitter_onglet(c, k);
        if let Some(cl) = self.client.as_mut() {
            cl.delier();
        }
    }

    /// Avant la liaison : chercher, choisir, donner le code ; et être piloté.
    fn reglages(&mut self, cx: &mut Cx, c: &mut Commun, r: Rect, k: usize) -> String {
        let base = sub_id(TELE, k);
        let mut b = cx.body(sub_id(base, 1), r, &c.scrolls);
        cx.section(&mut b, "Piloter un autre appareil");
        cx.paragraph(&mut b, "Sur l'autre appareil, ouvrez la Télécommande et activez « Accepter une télécommande » : il affiche son adresse et un code. Les deux doivent être sur le même Wi-Fi.");
        if cx.button(&mut b, sub_id(base, 10), "Chercher sur le réseau") {
            self.recherche = Some(Instant::now());
            if let Some(cl) = self.client() {
                cl.chercher();
            }
        }
        let trouves = self.client.as_ref().map_or(Vec::new(), |cl| cl.trouves.clone());
        let mien = tc::adresse_locale();
        for (i, (a, nom)) in trouves.iter().enumerate() {
            let ip = a.ip().to_string();
            let soi = mien.is_some_and(|m| m.to_string() == ip);
            let etiquette = format!("{nom} · {ip}{}", if soi { " (cet appareil)" } else { "" });
            if cx.row(&mut b, sub_id(base, 100 + i), &etiquette, None, self.adresse.texte == ip) {
                self.adresse = Edition::new(&ip);
                c.clavier.ouvrir(sub_id(CODE, k));
            }
        }
        if self.recherche.is_some_and(|t| t.elapsed().as_secs_f32() < 2.0) && trouves.is_empty() {
            cx.paragraph(&mut b, "Recherche…");
        } else if self.recherche.is_some() && trouves.is_empty() {
            cx.paragraph(&mut b, "Aucun Flow n'a répondu. Tapez son adresse, ou vérifiez que l'écoute y est activée.");
        }
        clavier::champ(cx, &mut b, sub_id(ADRESSE, k), "Adresse, par exemple 192.168.1.20", &mut self.adresse, &mut c.clavier);
        clavier::champ(cx, &mut b, sub_id(CODE, k), "Code à quatre chiffres", &mut self.code, &mut c.clavier);
        if cx.button(&mut b, sub_id(base, 11), "Lier") {
            self.lier(c);
        }
        if let Some(cl) = &self.client {
            if cl.refuse {
                cx.paragraph(&mut b, "Code refusé.");
            } else if cl.hote.is_some() && !cl.lie {
                cx.paragraph(&mut b, "En attente de l'autre appareil…");
            }
        }
        if let Some(e) = &self.erreur {
            let e = e.clone();
            cx.paragraph(&mut b, &e);
        }

        cx.section(&mut b, "Être piloté");
        let etat = tc::etat_hote();
        let mut ecoute = etat.is_some();
        cx.toggle(&mut b, sub_id(base, 20), "Accepter une télécommande", &mut ecoute);
        if ecoute != etat.is_some() {
            if let Err(e) = tc::ecouter(ecoute) {
                self.erreur = Some(e);
            }
        }
        if let Some((code, lie, vu, recus, erreur)) = tc::etat_hote() {
            let ip = mien.map_or("pas de réseau".to_string(), |a| a.to_string());
            cx.paragraph(&mut b, &format!("Adresse : {ip}  ·  Code : {code:04}"));
            let l = match (lie, vu) {
                (Some(a), Some(v)) => format!("Piloté par {} · dernier message il y a {:.1} s · {recus} reçus", a.ip(), v).replace('.', ","),
                _ => "Aucune télécommande liée.".into(),
            };
            cx.paragraph(&mut b, &l);
            if let Some(e) = erreur {
                cx.paragraph(&mut b, &e);
            }
        }
        cx.body_end(b, &mut c.scrolls);
        if tc::etat_hote().is_some_and(|e| e.1.is_some()) { "piloté".into() } else { "non liée".into() }
    }

    fn lier(&mut self, c: &mut Commun) {
        c.clavier.fermer();
        let ip: Result<Ipv4Addr, _> = self.adresse.texte.trim().parse();
        let code: Result<u32, _> = self.code.texte.trim().parse();
        match (ip, code) {
            (Ok(ip), Ok(code)) => {
                self.erreur = None;
                if let Some(cl) = self.client() {
                    cl.lier(ip, code);
                }
            }
            (Err(_), _) => self.erreur = Some("Adresse illisible.".into()),
            _ => self.erreur = Some("Le code est fait de quatre chiffres.".into()),
        }
    }

    /// Une fois lié : les onglets, la latence et la zone de pilotage.
    fn piloter(&mut self, cx: &mut Cx, c: &mut Commun, r: Rect, k: usize) -> String {
        let base = sub_id(TELE, k);
        let (d, col) = (cx.dp, cx.couleurs());
        let pad = cx.pad();
        let h = cx.commande();
        cx.clip(Some(r));
        let choix = cx.tabs(sub_id(base, 30), r.x + pad, r.y + pad, &ONGLETS, self.onglet);
        if choix != self.onglet {
            self.quitter_onglet(c, k);
            if choix == 3 {
                self.delier(c, k);
                cx.clip(None);
                return "déliée".into();
            }
            self.onglet = choix;
        }
        // La latence
        let (lat, ip) = match &self.client {
            Some(cl) => (cl.latence.clone(), cl.hote.map(|a| a.ip().to_string()).unwrap_or_default()),
            None => (Default::default(), String::new()),
        };
        let ms = |v: Option<f32>| v.map_or("–".to_string(), |v| format!("{v:.0}"));
        let pertes = if lat.envoyes > 0 { 100.0 * lat.perdus as f32 / lat.envoyes as f32 } else { 0.0 };
        let ligne = format!("{ip} · aller-retour {} ms (médiane {}, pire {}) · pertes {pertes:.0} %", ms(lat.dernier), ms(lat.mediane()), ms(lat.pire()));
        let y = r.y + pad + h + pad * 0.5;
        let lh = (cx.theme.ligne * d * 0.6).round();
        cx.text_at(r.x + pad, y + lh * 0.5, &ligne, 13.0, col.pale);
        let zone = Rect::new(r.x + pad, y + lh + pad * 0.5, r.w - 2.0 * pad, (r.bottom() - y - lh - pad * 1.5).max(0.0));
        let statut = format!("{} ms", ms(lat.mediane()));

        match self.onglet {
            0 => self.pave(cx, zone, base),
            1 => self.ecran(cx, zone, base),
            _ => self.clavier(cx, c, zone, k),
        }
        cx.clip(None);
        statut
    }

    fn pave(&mut self, cx: &mut Cx, zone: Rect, base: Id) {
        let (d, col) = (cx.dp, cx.couleurs());
        let h = cx.commande();
        let e = cx.pad() * 0.5;
        let surface = Rect::new(zone.x, zone.y, zone.w, (zone.h - h - e).max(0.0));
        let pid = sub_id(base, 40);
        cx.ui.region_add(pid, surface, Sense::Drag);
        cx.dl.rrect(surface, 14.0 * d, col.puce);
        cx.centered(surface, "Glissez pour déplacer · touchez pour cliquer", 13.0, col.pale);

        let doigts = cx.ui.doigts_sur(pid);
        let premier = doigts.iter().min_by_key(|p| p.0).copied();
        match (premier, self.pave.as_mut()) {
            (Some((i, x, y)), Some(p)) if p.id == i => {
                let (dx, dy) = (x - p.x, y - p.y);
                let pas = (dx * dx + dy * dy).sqrt();
                if pas > 0.0 {
                    // L'accélération : plus le doigt va vite, plus le curseur va loin
                    let vitesse = pas / self.dt.max(0.004) / surface.w.max(1.0);
                    let k = 1.0 + vitesse.min(3.0) * 0.6;
                    let w = surface.w.max(1.0);
                    let m = Message::Relatif(dx / w * k, dy / w * k);
                    p.chemin += pas;
                    p.x = x;
                    p.y = y;
                    if let Some(c) = &self.client {
                        c.envoyer(&m);
                    }
                }
            }
            (Some((i, x, y)), _) => self.pave = Some(DoigtPave { id: i, x, y, debut: Instant::now(), chemin: 0.0 }),
            (None, Some(_)) => {
                let p = self.pave.take().unwrap();
                if p.debut.elapsed().as_secs_f32() < 0.3 && p.chemin < 12.0 * d {
                    self.envoyer(&Message::Clic);
                }
            }
            (None, None) => {}
        }

        // Les boutons : Clic, et Tenir (le bouton reste enfoncé pour glisser)
        let y = surface.bottom() + e;
        let w = (zone.w - e) / 2.0;
        let clic = Rect::new(zone.x, y, w, h);
        let tenir = Rect::new(zone.x + w + e, y, w, h);
        if cx.ui.click(sub_id(base, 41), clic, None) {
            self.envoyer(&Message::Clic);
        }
        if cx.ui.click(sub_id(base, 42), tenir, None) {
            self.tenu = !self.tenu;
            self.envoyer(&Message::Bouton(self.tenu));
        }
        let rayon = cx.theme.rayon(h);
        cx.dl.rrect(clic, rayon, if cx.ui.pressed(sub_id(base, 41)) { col.ligne } else { col.puce });
        cx.centered(clic, "Clic", 15.0, col.accent);
        cx.dl.rrect(tenir, rayon, if self.tenu { col.accent } else { col.puce });
        cx.centered(tenir, if self.tenu { "Lâcher" } else { "Tenir" }, 15.0, if self.tenu { col.sur_accent } else { col.accent });
    }

    fn ecran(&mut self, cx: &mut Cx, zone: Rect, base: Id) {
        let (d, col) = (cx.dp, cx.couleurs());
        let eid = sub_id(base, 50);
        cx.ui.region_add(eid, zone, Sense::Drag);
        cx.dl.rrect(zone, 14.0 * d, col.puce);
        cx.centered(zone, "L'autre écran, en miroir", 13.0, col.pale);
        let (w, h) = (zone.w.max(1.0), zone.h.max(1.0));
        let doigts: Vec<(i32, f32, f32)> = cx.ui.doigts_sur(eid).into_iter().map(|(i, x, y)| (i, ((x - zone.x) / w).clamp(0.0, 1.0), ((y - zone.y) / h).clamp(0.0, 1.0))).collect();
        for &(_, x, y) in &doigts {
            let (px, py) = (zone.x + x * w, zone.y + y * h);
            cx.dl.rrect(Rect::new(px - 14.0 * d, py - 14.0 * d, 28.0 * d, 28.0 * d), 14.0 * d, [col.accent[0], col.accent[1], col.accent[2], 110]);
        }
        for m in self.suivi.messages(&doigts) {
            self.envoyer(&m);
        }
    }

    fn clavier(&mut self, cx: &mut Cx, c: &mut Commun, zone: Rect, k: usize) {
        let col = cx.couleurs();
        let cible = sub_id(SAISIE, k);
        if !self.clavier_annonce {
            self.clavier_annonce = true;
            self.envoyer(&Message::Clavier(true));
            c.clavier.ouvrir(cible);
        }
        let ouvert = c.clavier.cible == Some(cible);
        let h = cx.commande();
        let bouton = Rect::new(zone.x, zone.y, zone.w, h);
        if cx.ui.click(sub_id(cible, 1), bouton, None) {
            if ouvert {
                c.clavier.fermer();
            } else {
                c.clavier.ouvrir(cible);
            }
        }
        cx.dl.rrect(bouton, cx.theme.rayon(h), col.puce);
        cx.centered(bouton, if ouvert { "Baisser le clavier" } else { "Ouvrir le clavier" }, 15.0, col.accent);
        let t = Rect::new(zone.x, zone.y + h, zone.w, (zone.h - h).max(0.0));
        cx.centered(Rect::new(t.x, t.y, t.w, h * 1.5), "Les frappes vont au champ choisi sur l'autre appareil.", 13.0, col.pale);
        let fin: String = self.tape.chars().rev().take(40).collect::<Vec<_>>().into_iter().rev().collect();
        cx.centered(Rect::new(t.x, t.y + h * 1.5, t.w, h), &fin, 16.0, col.texte);
    }
}

impl Contexte for Telecommande {
    fn cle(&self) -> &'static str {
        CLE
    }

    fn dupliquer(&self) -> Box<dyn Contexte> {
        let mut e = Etat::default();
        self.ecrire(&mut e);
        ouvrir(&e)
    }

    fn ecrire(&self, e: &mut Etat) {
        if !self.adresse.texte.is_empty() {
            e.mettre("adresse", self.adresse.texte.clone());
        }
    }

    fn avancer(&mut self, dt: f32, _c: &Commun) {
        self.dt = dt;
        if let Some(cl) = self.client.as_mut() {
            cl.avancer();
        }
    }

    fn anime(&self, _c: &Commun) -> bool {
        // La socket se lit à chaque image tant qu'on attend quelque chose ; les pings
        // demandent des images régulières pendant la liaison
        self.client.as_ref().is_some_and(|cl| cl.hote.is_some()) || self.recherche.is_some_and(|t| t.elapsed().as_secs_f32() < 2.5)
    }

    fn frappe(&mut self, f: FenetreId, cible: Id, frappe: Frappe, c: &mut Commun) -> bool {
        let k = f as usize;
        if cible == sub_id(SAISIE, k) {
            self.envoyer(&Message::Touche(frappe));
            match frappe {
                Frappe::Car(ch) => self.tape.push(ch),
                Frappe::Effacer => {
                    self.tape.pop();
                }
                Frappe::Entree | Frappe::Ligne => self.tape.push('⏎'),
                _ => {}
            }
            return true;
        }
        let ed = if cible == sub_id(ADRESSE, k) {
            &mut self.adresse
        } else if cible == sub_id(CODE, k) {
            &mut self.code
        } else {
            return false;
        };
        if ed.appliquer(frappe) {
            if cible == sub_id(ADRESSE, k) {
                c.clavier.ouvrir(sub_id(CODE, k));
            } else {
                self.lier(c);
            }
        }
        true
    }

    fn dessiner(&mut self, cx: &mut Cx, c: &mut Commun, p: &Placee) -> String {
        let r = p.contenu;
        let k = p.id as usize;
        let col = cx.couleurs();
        cx.dl.rrect(r, 0.0, col.surface);
        if self.lie() {
            self.piloter(cx, c, r, k)
        } else {
            if self.clavier_annonce || self.tenu {
                self.quitter_onglet(c, k);
            }
            self.reglages(cx, c, r, k)
        }
    }
}
