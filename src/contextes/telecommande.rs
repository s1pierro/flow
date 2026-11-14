//! Le contexte Télécommande : piloter un autre Flow par le Wi-Fi, ou se laisser piloter.
//!
//! - **Piloter** : chercher les Flow du réseau (ou taper une adresse), donner le code
//!   que l'autre appareil affiche, puis :
//!   - **Pavé** : le doigt déplace un curseur sur l'autre écran (plus vite, plus loin) ;
//!     une touche brève clique, « Tenir » garde le bouton enfoncé pour glisser ;
//!   - **Écran** : la zone est l'autre écran en miroir, à plusieurs doigts (les gestes
//!     de Flow y passent tels quels) ;
//!   - **Clavier** : le clavier de cet appareil écrit dans le champ choisi là-bas (le
//!     clavier de l'autre écran se cache) ;
//!   - **Manette** : deux surfaces à stick flottant (le contact pose le stick au repos,
//!     comme les commandes à l'écran de WhiteRoom), la croix, A B X Y, L1 R1, les
//!     gâchettes, Select et Start, à plusieurs doigts. Sous le doigt, des stries
//!     (`vibreur::Stries`, 1 mm par défaut) : chacune franchie fait vibrer l'appareil
//!     aussi brièvement qu'il le peut, une rugosité qui dit au pouce où il va. Flow s'en sert dans la 3D des Lieux ; WhiteRoom la
//!     voit comme une manette de jeu (n'y sont alors proposés que Manette et Délier).
//!   La ligne du haut dit l'aller-retour mesuré (`telecommande::Latence`).
//! - **Être piloté** : l'interrupteur ouvre l'écoute ; l'adresse et le code s'affichent.
//!
//! Le réseau est dans `crate::telecommande`.

use crate::clavier::{self, Edition, Frappe};
use crate::contexte::{Commun, Contexte, Etat, Genre};
use crate::draw::Rect;
use crate::surface::{FenetreId, Placee};
use crate::telecommande::{self as tc, bouton, Client, Manette, Message, Suivi};
use crate::vibreur::{self, Stries};
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

const ONGLETS: [&str; 5] = ["Pavé", "Écran", "Clavier", "Manette", "Délier"];
const MANETTE: usize = 3;
const DELIER: usize = 4;

/// Le doigt qui mène le pavé : où il est, quand il s'est posé, le chemin parcouru
/// (en dp), sa vitesse lissée (dp/s), s'il a quitté la zone morte ; le nombre de
/// doigts le plus grand vu pendant l'appui.
struct DoigtPave {
    id: i32,
    x: f32,
    y: f32,
    debut: Instant,
    vu: Instant,
    chemin: f32,
    vitesse: f32,
    parti: bool,
    doigts: usize,
}

/// La course d'un stick flottant, en millimètres : à cette distance du contact, il est
/// à fond, et au-delà sa base suit le doigt.
const COURSE_MM: f32 = 9.0;
/// Les écarts de stries proposés (mm ; 0 : sans vibration).
const STRIES: [f32; 6] = [0.5, 1.0, 1.5, 2.0, 3.0, 0.0];

/// Un stick flottant sur une surface : le doigt qui le tient, la base posée au contact,
/// où est le doigt, et les stries sous lui.
#[derive(Clone, Copy, Debug, Default)]
struct StickFlottant {
    doigt: Option<i32>,
    base: Option<[f32; 2]>,
    pos: Option<[f32; 2]>,
    stries: Stries,
}

impl StickFlottant {
    /// Les doigts posés sur la surface ; la course et l'écart des stries en pixels.
    /// Rend la valeur du stick (−1 à 1, y vers le bas) et si une strie est franchie.
    fn suivre(&mut self, doigts: &[(i32, f32, f32)], course: f32, pas: f32) -> ([f32; 2], bool) {
        let tenu = self.doigt.and_then(|i| doigts.iter().find(|p| p.0 == i)).or_else(|| doigts.iter().min_by_key(|p| p.0)).copied();
        let Some((i, x, y)) = tenu else {
            *self = StickFlottant::default();
            return ([0.0; 2], false);
        };
        if self.doigt != Some(i) {
            // Un autre doigt : le stick se repose sous lui
            *self = StickFlottant { doigt: Some(i), ..StickFlottant::default() };
        }
        let p = [x, y];
        let mut b = self.base.unwrap_or(p);
        let r = course.max(1.0);
        let (dx, dy) = (p[0] - b[0], p[1] - b[1]);
        let l = dx.hypot(dy);
        if l > r {
            b = [p[0] - dx / l * r, p[1] - dy / l * r];
        }
        self.base = Some(b);
        self.pos = Some(p);
        let tic = self.stries.suivre(Some(p), pas);
        ([((p[0] - b[0]) / r).clamp(-1.0, 1.0), ((p[1] - b[1]) / r).clamp(-1.0, 1.0)], tic)
    }
}

/// En deçà, un appui est une touche : le curseur ne bouge pas (dp).
const ZONE_MORTE: f32 = 6.0;

/// Le gain du pavé selon la vitesse du doigt (dp/s) : 1 en mouvement lent, pour viser
/// au pixel, jusqu'à 3,5 en geste vif, pour traverser l'écran ; la courbe est douce
/// pour que le curseur ne saute pas d'un régime à l'autre.
pub fn gain(vitesse: f32) -> f32 {
    let t = ((vitesse - 60.0) / (1400.0 - 60.0)).clamp(0.0, 1.0);
    1.0 + 2.5 * t * t * (3.0 - 2.0 * t)
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
    /// La manette : le dernier état envoyé, et quand.
    manette: Manette,
    manette_envoyee: Option<Instant>,
    /// Les aperçus montrent la vue liée sans réseau (`demo = oui`).
    demo: bool,
    /// Les deux sticks flottants, et l'écart de leurs stries (mm, 0 : aucune).
    sticks: [StickFlottant; 2],
    stries: f32,
    /// La force des stries (`vibreur::NIVEAUX`).
    force: u8,
}

fn ouvrir(e: &Etat) -> Box<dyn Contexte> {
    Box::new(Telecommande {
        client: None,
        erreur: None,
        adresse: Edition::new(e.texte("adresse").unwrap_or("")),
        code: Edition::default(),
        clavier_annonce: false,
        suivi: Suivi::default(),
        pave: None,
        tenu: false,
        recherche: None,
        tape: String::new(),
        manette: Manette::default(),
        manette_envoyee: None,
        demo: e.texte("demo") == Some("oui"),
        sticks: Default::default(),
        force: e.texte("force").and_then(|v| v.parse().ok()).filter(|v: &u8| (*v as usize) < vibreur::NIVEAUX.len()).unwrap_or(2),
        stries: e.texte("stries").and_then(|v| v.parse().ok()).filter(|v: &f32| STRIES.contains(v)).unwrap_or(1.0),
        onglet: e.texte("onglet").and_then(|o| ONGLETS.iter().position(|n| *n == o)).filter(|&i| i != DELIER).unwrap_or(0),
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
        self.demo || self.client.as_ref().is_some_and(|c| c.lie)
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
        self.sticks = Default::default();
        if !self.manette.au_repos() {
            self.manette = Manette::default();
            self.envoyer(&Message::Manette(self.manette));
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
        let cherche = self.client.as_ref().is_some_and(|cl| cl.cherche());
        if cherche && trouves.is_empty() {
            cx.paragraph(&mut b, "Recherche…");
        } else if self.recherche.is_some() && trouves.is_empty() {
            cx.paragraph(&mut b, "Aucun Flow n'a répondu. Vérifiez que l'écoute est activée là-bas, et que les deux appareils sont sur le même Wi-Fi (un réseau « invités » isole souvent ses appareils). Vous pouvez aussi taper l'adresse qu'il affiche.");
        }
        if let Some(cl) = &self.client {
            let ici = cl.local.map_or("pas d'adresse Wi-Fi".to_string(), |(a, p)| format!("{a}/{p}"));
            let e = cl.erreur.clone().map_or(String::new(), |e| format!(" · {e}"));
            cx.paragraph(&mut b, &format!("Cet appareil : {ici}{e}"));
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
        // Un WhiteRoom n'a que faire du pavé, de l'écran ni du clavier
        let jeu = self.client.as_ref().and_then(|c| c.nom_hote.as_deref()).is_some_and(|n| n.starts_with("WhiteRoom"));
        let visibles: Vec<usize> = if jeu { vec![MANETTE, DELIER] } else { (0..ONGLETS.len()).collect() };
        if !visibles.contains(&self.onglet) {
            self.quitter_onglet(c, k);
            self.onglet = MANETTE;
        }
        let noms: Vec<&str> = visibles.iter().map(|&i| ONGLETS[i]).collect();
        let actuel = visibles.iter().position(|&i| i == self.onglet).unwrap_or(0);
        let choix = visibles[cx.tabs(sub_id(base, 30), r.x + pad, r.y + pad, &noms, actuel)];
        if choix != self.onglet {
            self.quitter_onglet(c, k);
            if choix == DELIER {
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
            MANETTE => self.manette(cx, zone, base),
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
        cx.centered(surface, "Glissez pour déplacer · touchez pour cliquer · deux doigts pour défiler", 13.0, col.pale);

        let doigts = cx.ui.doigts_sur(pid);
        let premier = doigts.iter().min_by_key(|p| p.0).copied();
        let mut envois = Vec::new();
        match (premier, self.pave.as_mut()) {
            (Some((i, x, y)), Some(p)) if p.id == i => {
                // En dp : le même geste fait le même chemin quelle que soit la densité
                let (dx, dy) = ((x - p.x) / d, (y - p.y) / d);
                let pas = (dx * dx + dy * dy).sqrt();
                let dt = p.vu.elapsed().as_secs_f32().max(0.004);
                p.vu = Instant::now();
                // La vitesse lissée : une image en retard ne fait pas bondir le curseur
                p.vitesse += (pas / dt - p.vitesse) * (dt / 0.06).min(1.0);
                p.chemin += pas;
                p.x = x;
                p.y = y;
                let deux = doigts.len() >= 2;
                p.doigts = p.doigts.max(doigts.len());
                if p.chemin >= ZONE_MORTE && pas > 0.0 {
                    p.parti = true;
                    if deux {
                        // Deux doigts : le contenu suit les doigts, sans accélération
                        envois.push(Message::Defile(dx, dy));
                    } else if p.doigts < 2 {
                        let k = gain(p.vitesse);
                        envois.push(Message::Relatif(dx * k, dy * k));
                    }
                }
            }
            (Some((i, x, y)), _) => {
                if self.pave.as_ref().is_some_and(|p| p.doigts >= 2) {
                    envois.push(Message::FinDefile);
                }
                self.pave = Some(DoigtPave { id: i, x, y, debut: Instant::now(), vu: Instant::now(), chemin: 0.0, vitesse: 0.0, parti: false, doigts: doigts.len() });
            }
            (None, Some(_)) => {
                let p = self.pave.take().unwrap();
                if p.doigts >= 2 {
                    envois.push(Message::FinDefile);
                } else if !p.parti && p.debut.elapsed().as_secs_f32() < 0.35 {
                    envois.push(Message::Clic);
                }
            }
            (None, None) => {}
        }
        for m in envois {
            self.envoyer(&m);
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
        // Aux proportions de l'écran piloté, centré dans la zone
        let zone = match self.client.as_ref().and_then(|c| c.taille) {
            Some((tw, th)) if zone.w > 0.0 && zone.h > 0.0 => {
                let k = (zone.w / tw).min(zone.h / th);
                let (w, h) = (tw * k, th * k);
                Rect::new(zone.x + (zone.w - w) / 2.0, zone.y + (zone.h - h) / 2.0, w, h)
            }
            _ => zone,
        };
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

    /// La manette : gâchettes et tranches en haut ; à gauche le stick et la croix, à
    /// droite A B X Y et le second stick ; Select et Start au milieu.
    fn manette(&mut self, cx: &mut Cx, zone: Rect, base: Id) {
        let (d, col) = (cx.dp, cx.couleurs());
        let e = cx.pad() * 0.5;
        let h = cx.commande();
        let mb = sub_id(base, 60);
        let mut m = Manette::default();

        // Une touche : vraie tant qu'un doigt y est posé
        let touche = |cx: &mut Cx, id: Id, r: Rect, nom: &str| -> bool {
            cx.ui.region_add(id, r, Sense::Drag);
            let on = !cx.ui.doigts_sur(id).is_empty();
            cx.dl.rrect(r, cx.theme.rayon(r.h).min(r.w / 2.0), if on { col.accent } else { col.puce });
            cx.centered(r, nom, 15.0, if on { col.sur_accent } else { col.texte });
            on
        };

        // Les tranches : LT L1 … R1 RT
        let q = (zone.w - 3.0 * e) / 4.0;
        let haut = |i: f32| Rect::new(zone.x + i * (q + e), zone.y, q, h);
        if touche(cx, sub_id(mb, 1), haut(0.0), "LT") {
            m.lt = 1.0;
        }
        if touche(cx, sub_id(mb, 2), haut(1.0), "L1") {
            m.boutons |= bouton::L1;
        }
        if touche(cx, sub_id(mb, 3), haut(2.0), "R1") {
            m.boutons |= bouton::R1;
        }
        if touche(cx, sub_id(mb, 4), haut(3.0), "RT") {
            m.rt = 1.0;
        }

        // Le reste : deux moitiés et une colonne au milieu
        let reste = Rect::new(zone.x, zone.y + h + e, zone.w, (zone.h - h - e).max(0.0));
        let milieu = (reste.w * 0.18).min(110.0 * d);
        let mw = (reste.w - milieu - 2.0 * e) / 2.0;
        let gauche = Rect::new(reste.x, reste.y, mw, reste.h);
        let droite = Rect::new(reste.right() - mw, reste.y, mw, reste.h);
        let colonne = Rect::new(gauche.right() + e, reste.y, milieu, reste.h);
        // Une moitié se partage en deux cases : côte à côte si elle est large (paysage),
        // l'une sur l'autre sinon ; la première est à l'extérieur, ou en haut
        let cases = |r: Rect, exterieur_a_droite: bool| -> (Rect, Rect) {
            if r.w > r.h * 1.3 {
                let w = r.w / 2.0;
                let (g, d) = (Rect::new(r.x, r.y, w, r.h), Rect::new(r.x + w, r.y, w, r.h));
                if exterieur_a_droite { (d, g) } else { (g, d) }
            } else {
                (Rect::new(r.x, r.y, r.w, r.h / 2.0), Rect::new(r.x, r.y + r.h / 2.0, r.w, r.h / 2.0))
            }
        };
        let (case_stick_g, case_croix) = cases(gauche, false);
        let (case_abxy, case_stick_d) = cases(droite, true);
        let bh = h.min((colonne.h - 3.0 * e) / 4.0);
        let y0 = colonne.cy() - 2.0 * bh - 1.5 * e;
        let sel = Rect::new(colonne.x, y0, colonne.w, bh);
        let start = Rect::new(colonne.x, y0 + bh + e, colonne.w, bh);
        // L'écart des stries : chaque appui passe au suivant
        let rug = Rect::new(colonne.x, y0 + 2.0 * (bh + e), colonne.w, bh);
        if cx.ui.click(sub_id(mb, 7), rug, None) {
            let i = STRIES.iter().position(|v| *v == self.stries).unwrap_or(0);
            self.stries = STRIES[(i + 1) % STRIES.len()];
            if self.stries > 0.0 {
                vibreur::tic(self.force);
            }
        }
        cx.dl.rrect(rug, cx.theme.rayon(bh), col.puce);
        let t = if self.stries > 0.0 { format!("Stries {} mm", self.stries).replace('.', ",") } else { "Sans stries".into() };
        cx.centered(rug, &t, 13.0, col.accent);
        // Leur force : chaque appui passe à la suivante, et la fait sentir
        let fr = Rect::new(colonne.x, y0 + 3.0 * (bh + e), colonne.w, bh);
        if cx.ui.click(sub_id(mb, 8), fr, None) {
            self.force = (self.force + 1) % vibreur::NIVEAUX.len() as u8;
            vibreur::tic(self.force);
        }
        cx.dl.rrect(fr, cx.theme.rayon(bh), col.puce);
        cx.centered(fr, vibreur::NIVEAUX[self.force as usize], 13.0, col.accent);
        if touche(cx, sub_id(mb, 5), sel, "Select") {
            m.boutons |= bouton::SELECT;
        }
        if touche(cx, sub_id(mb, 6), start, "Start") {
            m.boutons |= bouton::START;
        }

        let mm = vibreur::px_par_mm(d);
        let (course, pas) = (COURSE_MM * mm, self.stries * mm);
        let mut tic = false;
        for (i, (case, nom)) in [(case_stick_g, "Stick gauche"), (case_stick_d, "Stick droit")].into_iter().enumerate() {
            let (v, t) = self.surface(cx, sub_id(mb, 10 + i), case.inset(e * 0.5), nom, i, course, pas);
            if i == 0 {
                m.gauche = v;
            } else {
                m.droit = v;
            }
            tic |= t;
        }
        if tic {
            vibreur::tic(self.force);
        }

        // La croix : une seule zone, la direction d'après le doigt (le pouce glisse
        // d'une flèche à l'autre, et vise les diagonales)
        let croix = case_croix;
        let cid = sub_id(mb, 12);
        cx.ui.region_add(cid, croix, Sense::Drag);
        let (c0, rc) = ([croix.cx(), croix.cy()], (croix.w.min(croix.h) / 2.0 - e).max(1.0));
        for (_, x, y) in cx.ui.doigts_sur(cid) {
            let (dx, dy) = ((x - c0[0]) / rc, (y - c0[1]) / rc);
            let l = dx.hypot(dy);
            if l > 0.2 {
                if dx > 0.38 * l {
                    m.boutons |= bouton::DROITE;
                }
                if dx < -0.38 * l {
                    m.boutons |= bouton::GAUCHE;
                }
                if dy > 0.38 * l {
                    m.boutons |= bouton::BAS;
                }
                if dy < -0.38 * l {
                    m.boutons |= bouton::HAUT;
                }
            }
        }
        let b = rc * 0.36;
        for (bit, ox, oy) in [(bouton::HAUT, 0.0, -1.0), (bouton::BAS, 0.0, 1.0), (bouton::GAUCHE, -1.0, 0.0), (bouton::DROITE, 1.0, 0.0)] {
            let on = m.appuye(bit);
            let r = Rect::new(c0[0] + ox * rc * 0.62 - b, c0[1] + oy * rc * 0.62 - b, 2.0 * b, 2.0 * b);
            cx.dl.rrect(r, 6.0 * d, if on { col.accent } else { col.puce });
        }
        cx.dl.rrect(Rect::new(c0[0] - b, c0[1] - b, 2.0 * b, 2.0 * b), 0.0, col.puce);

        // A B X Y : une seule zone, chaque doigt prend le bouton le plus proche
        let abxy = case_abxy;
        let aid = sub_id(mb, 13);
        cx.ui.region_add(aid, abxy, Sense::Drag);
        let (c1, ra) = ([abxy.cx(), abxy.cy()], (abxy.w.min(abxy.h) / 2.0 - e).max(1.0));
        let places = [(bouton::Y, "Y", 0.0, -1.0), (bouton::A, "A", 0.0, 1.0), (bouton::X, "X", -1.0, 0.0), (bouton::B, "B", 1.0, 0.0)];
        for (_, x, y) in cx.ui.doigts_sur(aid) {
            let (dx, dy) = ((x - c1[0]) / ra, (y - c1[1]) / ra);
            let pres = places.iter().min_by(|p, q| ((dx - p.2 * 0.62).hypot(dy - p.3 * 0.62)).total_cmp(&(dx - q.2 * 0.62).hypot(dy - q.3 * 0.62)));
            if let Some(p) = pres {
                m.boutons |= p.0;
            }
        }
        let rb = ra * 0.33;
        for (bit, nom, ox, oy) in places {
            let on = m.appuye(bit);
            let r = Rect::new(c1[0] + ox * ra * 0.62 - rb, c1[1] + oy * ra * 0.62 - rb, 2.0 * rb, 2.0 * rb);
            cx.dl.rrect(r, rb, if on { col.accent } else { col.puce });
            cx.centered(r, nom, 17.0, if on { col.sur_accent } else { col.texte });
        }

        // Envoyée à chaque changement, et toutes les 0,1 s pour dire qu'elle est là
        if m != self.manette || self.manette_envoyee.is_none_or(|t| t.elapsed().as_secs_f32() >= 0.1) {
            self.manette = m;
            self.manette_envoyee = Some(Instant::now());
            self.envoyer(&Message::Manette(m));
        }
    }

    /// Une surface à stick flottant : le contact pose le stick au repos, le doigt
    /// l'écarte jusqu'à `course` pixels. Rend sa valeur et si une strie est franchie.
    #[allow(clippy::too_many_arguments)]
    fn surface(&mut self, cx: &mut Cx, id: Id, r: Rect, nom: &str, i: usize, course: f32, pas: f32) -> ([f32; 2], bool) {
        let (d, col) = (cx.dp, cx.couleurs());
        cx.ui.region_add(id, r, Sense::Drag);
        let doigts = cx.ui.doigts_sur(id);
        let (v, tic) = self.sticks[i].suivre(&doigts, course, pas);
        cx.dl.rrect(r, 16.0 * d, [col.puce[0], col.puce[1], col.puce[2], 150]);
        let st = self.sticks[i];
        match (st.base, st.pos) {
            (Some(b), Some(p)) => {
                cx.dl.secteur(b, course - 2.0 * d, course, 0.0, std::f32::consts::TAU, 0.0, col.ligne);
                let k = 22.0 * d;
                cx.dl.rrect(Rect::new(p[0] - k, p[1] - k, 2.0 * k, 2.0 * k), k, col.accent);
            }
            _ => cx.centered(r, nom, 13.0, col.pale),
        }
        (v, tic)
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
        e.mettre("onglet", ONGLETS[self.onglet]);
        e.mettre("stries", self.stries);
        e.mettre("force", self.force);
    }

    fn avancer(&mut self, _dt: f32, _c: &Commun) {
        if let Some(cl) = self.client.as_mut() {
            cl.avancer();
        }
    }

    fn anime(&self, _c: &Commun) -> bool {
        // La socket se lit à chaque image tant qu'on attend quelque chose ; les pings
        // demandent des images régulières pendant la liaison
        self.client.as_ref().is_some_and(|cl| cl.hote.is_some() || cl.cherche())
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

#[cfg(test)]
mod tests {
    use super::{gain, StickFlottant};

    #[test]
    fn le_stick_flottant_se_pose_sous_le_doigt() {
        let mut s = StickFlottant::default();
        // Le contact : au repos
        assert_eq!(s.suivre(&[(7, 100.0, 100.0)], 50.0, 10.0), ([0.0, 0.0], false));
        // À mi-course vers la droite, deux stries franchies (une seule vibration)
        assert_eq!(s.suivre(&[(7, 125.0, 100.0)], 50.0, 10.0), ([0.5, 0.0], true));
        // Vers le haut à fond, au-delà : la base suit
        let (v, _) = s.suivre(&[(7, 100.0, 20.0)], 50.0, 10.0);
        assert_eq!(v[1], -1.0);
        // Un second doigt ne le prend pas
        let (v2, _) = s.suivre(&[(3, 0.0, 0.0), (7, 100.0, 20.0)], 50.0, 10.0);
        assert_eq!(v2, v);
        // Levé : au repos ; reposé ailleurs : au repos là
        assert_eq!(s.suivre(&[], 50.0, 10.0).0, [0.0, 0.0]);
        assert_eq!(s.suivre(&[(9, 400.0, 400.0)], 50.0, 10.0), ([0.0, 0.0], false));
    }

    #[test]
    fn le_gain_monte_doucement_avec_la_vitesse() {
        assert_eq!(gain(0.0), 1.0);
        assert_eq!(gain(60.0), 1.0);
        assert!((gain(5000.0) - 3.5).abs() < 1e-5);
        let mut avant = 1.0;
        for v in (0..1500).step_by(10) {
            let g = gain(v as f32);
            assert!(g >= avant && g - avant < 0.06, "à {v} dp/s : {avant} puis {g}");
            avant = g;
        }
    }
}
