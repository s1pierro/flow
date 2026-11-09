//! Clavier virtuel, dessiné par Rustine comme le reste de l'interface, et ses champs
//! de texte.
//!
//! - Dispositions AZERTY et QWERTY, une couche de chiffres et de symboles (`?123`).
//! - Majuscule : un appui pour la lettre suivante, deux appuis rapprochés pour la
//!   verrouiller.
//! - Appui long sur une lettre : ses variantes (é è ê ë…), choisies en glissant le
//!   doigt, écrites au lâcher.
//! - Effacer se répète tant qu'on le tient ; glisser sur la barre d'espace déplace le
//!   curseur.
//! - Une bulle montre la touche sous le doigt (réglable).
//!
//! Le clavier occupe le bas de l'écran quand un champ a le focus : l'application lui
//! retire sa hauteur avant de disposer le reste. Flottant (réglage), il ne retire rien :
//! plus étroit, il passe par-dessus l'interface et se déplace en tirant son bandeau.
//! `saisir` lit les doigts et renvoie les frappes (en fin d'image s'il flotte, pour
//! passer devant tout le reste) ; `dessiner` le dessine par-dessus tout.

use crate::draw::Rect;
use crate::ui::{id, sub_id, Id, Sense};
use crate::widgets::{Body, Cx, Icon};

/// Les réglages du clavier, qui font partie de ceux de l'interface.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReglagesClavier {
    pub disposition: Disposition,
    /// Hauteur d'une touche, en dp.
    pub touche: f32,
    /// Durée d'un appui long (variantes), en secondes.
    pub appui_long: f32,
    /// Une bulle montre la touche sous le doigt.
    pub apercu: bool,
    /// Le clavier flotte par-dessus l'interface au lieu de prendre le bas de l'écran.
    pub flottant: bool,
    /// La place du clavier flottant dans l'écran : de 0 (à gauche, en haut) à 1 (à
    /// droite, en bas).
    pub position: (f32, f32),
}

impl Default for ReglagesClavier {
    fn default() -> ReglagesClavier {
        ReglagesClavier { disposition: Disposition::Azerty, touche: 50.0, appui_long: 0.35, apercu: true, flottant: false, position: (0.5, 1.0) }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Disposition {
    Azerty,
    Qwerty,
}

impl Disposition {
    pub const TOUTES: [Disposition; 2] = [Disposition::Azerty, Disposition::Qwerty];

    pub fn nom(self) -> &'static str {
        match self {
            Disposition::Azerty => "AZERTY",
            Disposition::Qwerty => "QWERTY",
        }
    }
}

/// Ce que le clavier envoie au champ qui a le focus.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Frappe {
    Car(char),
    Effacer,
    /// Entrée : valide le champ.
    Entree,
    Gauche,
    Droite,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Sorte {
    Car(char),
    Maj,
    Effacer,
    Entree,
    Espace,
    Couche,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Maj {
    Non,
    Une,
    Verrou,
}

/// Une touche tenue : depuis quand, où est le doigt, ce qu'elle a déjà fait.
#[derive(Clone, Debug)]
struct Tenue {
    id: Id,
    sorte: Sorte,
    rect: Rect,
    debut: f64,
    depart: (f32, f32),
    doigt: (f32, f32),
    /// Effacements déjà faits en répétition.
    repete: u32,
    /// Pas de curseur déjà faits en glissant sur l'espace.
    curseur: i32,
    /// Les variantes ouvertes par l'appui long.
    variantes: Option<Vec<char>>,
}

const FOND: Id = id("clavier/fond");
const TOUCHE: Id = id("clavier/touche");
const DISPOSITION: Id = id("clavier/disposition");
const BAISSER: Id = id("clavier/baisser");
const DEPLACER: Id = id("clavier/deplacer");

pub struct Clavier {
    pub reglages: ReglagesClavier,
    /// Le champ qui reçoit les frappes ; le clavier est ouvert tant qu'il y en a un.
    pub cible: Option<Id>,
    symboles: bool,
    maj: Maj,
    dernier_maj: f64,
    temps: f64,
    tenues: Vec<Tenue>,
    /// Les touches de la dernière image : rectangle, sorte, identifiant.
    touches: Vec<(Rect, Sorte, Id)>,
    rect: Rect,
    /// L'écran où le clavier flottant se déplace, et l'écart du doigt qui tire son
    /// bandeau au coin du clavier.
    ecran: Rect,
    prise: Option<(f32, f32)>,
}

/// Les variantes d'une touche, en minuscules.
fn variantes(c: char) -> &'static str {
    match c {
        'e' => "éèêë€",
        'a' => "àâæäá",
        'c' => "ç©",
        'u' => "ùûüú",
        'i' => "îïíì",
        'o' => "ôœöóò",
        'y' => "ÿý",
        'n' => "ñ",
        's' => "ß",
        '.' => "…·",
        ',' => ";",
        '\'' => "’‘`",
        '"' => "«»“”",
        '-' => "–—_",
        '€' => "$£¥",
        '%' => "‰",
        '?' => "¿",
        '!' => "¡",
        '(' => "[{<",
        ')' => "]}>",
        '/' => "\\|",
        '*' => "×÷",
        '+' => "±=",
        '_' => "~^`",
        '#' => "§",
        '1' => "¹½",
        '2' => "²",
        '3' => "³",
        '0' => "°",
        _ => "",
    }
}

fn haut(c: char) -> char {
    c.to_uppercase().next().unwrap_or(c)
}

/// Les rangées de touches : sorte et largeur (en touches).
fn rangees(d: Disposition, symboles: bool) -> Vec<Vec<(Sorte, f32)>> {
    let lettres = |s: &str| s.chars().map(|c| (Sorte::Car(c), 1.0)).collect::<Vec<_>>();
    let mut r = if symboles {
        vec![lettres("1234567890"), lettres("@#€%&-+()/"), [vec![(Sorte::Car('_'), 1.5)], lettres("*\"':;!?")].concat()]
    } else {
        match d {
            Disposition::Azerty => vec![lettres("azertyuiop"), lettres("qsdfghjklm"), [vec![(Sorte::Maj, 1.5)], lettres("wxcvbn'")].concat()],
            Disposition::Qwerty => vec![lettres("qwertyuiop"), lettres("asdfghjkl"), [vec![(Sorte::Maj, 1.5)], lettres("zxcvbnm")].concat()],
        }
    };
    r[2].push((Sorte::Effacer, 1.5));
    r.push(vec![(Sorte::Couche, 1.5), (Sorte::Car(','), 1.0), (Sorte::Espace, 5.0), (Sorte::Car('.'), 1.0), (Sorte::Entree, 1.5)]);
    r
}

impl Default for Clavier {
    fn default() -> Clavier {
        Clavier::new(ReglagesClavier::default())
    }
}

impl Clavier {
    pub fn new(reglages: ReglagesClavier) -> Clavier {
        Clavier { reglages, cible: None, symboles: false, maj: Maj::Non, dernier_maj: -1.0, temps: 0.0, tenues: Vec::new(), touches: Vec::new(), rect: Rect::default(), ecran: Rect::default(), prise: None }
    }

    pub fn ouvert(&self) -> bool {
        self.cible.is_some()
    }

    /// Vrai tant qu'une touche est tenue : l'appui long et la répétition ont besoin
    /// d'images sans attendre un événement.
    pub fn anime(&self) -> bool {
        !self.tenues.is_empty()
    }

    fn barre(&self, cx: &Cx) -> f32 {
        (self.reglages.touche * 0.72 * cx.dp).round()
    }

    /// La hauteur du clavier ouvert, en pixels (0 s'il est fermé).
    pub fn hauteur(&self, cx: &Cx) -> f32 {
        if !self.ouvert() {
            return 0.0;
        }
        let d = cx.dp;
        self.barre(cx) + 4.0 * (self.reglages.touche * d).round() + 6.0 * d
    }

    /// Le reste de l'écran une fois le clavier posé en bas, et la place du clavier.
    /// Flottant, l'écran reste entier et le clavier se pose à sa place réglée.
    pub fn reserver(&mut self, cx: &Cx, ecran: Rect) -> (Rect, Rect) {
        self.ecran = ecran;
        let h = self.hauteur(cx).min(ecran.h * 0.6);
        if self.reglages.flottant {
            let w = (8.4 * self.reglages.touche * cx.dp).min(ecran.w - 16.0 * cx.dp).max(1.0);
            let (px, py) = self.reglages.position;
            let r = Rect::new(ecran.x + px.clamp(0.0, 1.0) * (ecran.w - w).max(0.0), ecran.y + py.clamp(0.0, 1.0) * (ecran.h - h).max(0.0), w, h);
            return (ecran, if h > 0.0 { r } else { Rect::default() });
        }
        (Rect::new(ecran.x, ecran.y, ecran.w, ecran.h - h), Rect::new(ecran.x, ecran.bottom() - h, ecran.w, h))
    }

    pub fn ouvrir(&mut self, cible: Id) {
        if self.cible != Some(cible) {
            self.cible = Some(cible);
            self.symboles = false;
        }
    }

    pub fn fermer(&mut self) {
        self.cible = None;
        self.tenues.clear();
        self.maj = Maj::Non;
    }

    fn majuscule(&self) -> bool {
        self.maj != Maj::Non
    }

    /// Les variantes à proposer pour une touche, à la casse courante.
    fn choix(&self, c: char) -> Vec<char> {
        let v = variantes(c);
        if v.is_empty() {
            return Vec::new();
        }
        let maj = self.majuscule() && c.is_alphabetic();
        std::iter::once(c).chain(v.chars()).map(|x| if maj { haut(x) } else { x }).collect()
    }

    /// Le rectangle des variantes au-dessus d'une touche, et la largeur d'une case.
    fn bande(&self, t: &Tenue, n: usize) -> (Rect, f32) {
        let cw = t.rect.w.max(self.rect.w / 12.0);
        let w = cw * n as f32;
        let x = (t.rect.cx() - cw / 2.0).min(self.rect.right() - w).max(self.rect.x);
        (Rect::new(x, t.rect.y - t.rect.h * 1.1, w, t.rect.h), cw)
    }

    fn valider(&mut self, t: &Tenue, out: &mut Vec<Frappe>) {
        match t.sorte {
            Sorte::Car(c) => {
                let ch = match &t.variantes {
                    Some(v) => {
                        let (b, cw) = self.bande(t, v.len());
                        v[(((t.doigt.0 - b.x) / cw).floor().max(0.0) as usize).min(v.len() - 1)]
                    }
                    None if self.majuscule() => haut(c),
                    None => c,
                };
                out.push(Frappe::Car(ch));
                if self.maj == Maj::Une {
                    self.maj = Maj::Non;
                }
            }
            Sorte::Maj => {
                self.maj = if self.temps - self.dernier_maj < 0.35 && self.maj == Maj::Une {
                    Maj::Verrou
                } else if self.maj == Maj::Non {
                    Maj::Une
                } else {
                    Maj::Non
                };
                self.dernier_maj = self.temps;
            }
            Sorte::Effacer if t.repete == 0 => out.push(Frappe::Effacer),
            Sorte::Espace if t.curseur == 0 && (t.doigt.0 - t.depart.0).abs() < t.rect.h * 0.3 => {
                out.push(Frappe::Car(' '));
                if self.maj == Maj::Une {
                    self.maj = Maj::Non;
                }
            }
            Sorte::Entree => out.push(Frappe::Entree),
            Sorte::Couche => self.symboles = !self.symboles,
            _ => {}
        }
    }

    /// Lit les doigts sur le clavier posé dans `r` et renvoie les frappes. À appeler
    /// tôt dans l'image ; `dessiner` le dessine plus tard.
    pub fn saisir(&mut self, cx: &mut Cx, r: Rect, dt: f32) -> Vec<Frappe> {
        let mut out = Vec::new();
        self.temps += dt as f64;
        self.rect = r;
        self.touches.clear();
        if !self.ouvert() || r.h < 1.0 {
            self.tenues.clear();
            return out;
        }
        let d = cx.dp;
        cx.ui.region_add(FOND, r, Sense::Sink);
        // La barre du haut : la disposition (un appui passe à la suivante), baisser ; le
        // clavier flottant se déplace en la tirant
        let bh = self.barre(cx);
        if self.reglages.flottant {
            match cx.ui.drag(DEPLACER, Rect::new(r.x, r.y, r.w, bh)) {
                Some(g) => {
                    let (ox, oy) = *self.prise.get_or_insert((g.x - r.x, g.y - r.y));
                    let e = self.ecran;
                    let (lx, ly) = ((e.w - r.w).max(1.0), (e.h - r.h).max(1.0));
                    self.reglages.position = (((g.x - ox - e.x) / lx).clamp(0.0, 1.0), ((g.y - oy - e.y) / ly).clamp(0.0, 1.0));
                }
                None => self.prise = None,
            }
        }
        let disp = Rect::new(r.x + 6.0 * d, r.y, cx.measure(self.reglages.disposition.nom(), 13.0) + 24.0 * d, bh);
        if cx.ui.click(DISPOSITION, disp, None) {
            let toutes = Disposition::TOUTES;
            let i = toutes.iter().position(|x| *x == self.reglages.disposition).unwrap_or(0);
            self.reglages.disposition = toutes[(i + 1) % toutes.len()];
        }
        if cx.ui.click(BAISSER, Rect::new(r.right() - bh * 1.4, r.y, bh * 1.4, bh), None) {
            self.fermer();
            return out;
        }
        // Les touches
        let kh = (self.reglages.touche * d).round();
        let (pad, y0) = (3.0 * d, r.y + bh);
        let unite = (r.w - 2.0 * pad) / 10.0;
        let mut n = 0;
        for (i, rangee) in rangees(self.reglages.disposition, self.symboles).iter().enumerate() {
            let total: f32 = rangee.iter().map(|t| t.1).sum();
            let mut x = r.x + pad + (10.0 - total) * unite / 2.0;
            for &(sorte, l) in rangee {
                let k = Rect::new(x, y0 + i as f32 * kh, l * unite, kh);
                self.touches.push((k, sorte, sub_id(TOUCHE, n + if self.symboles { 1000 } else { 0 })));
                x += l * unite;
                n += 1;
            }
        }
        let mut vues = Vec::new();
        for (k, sorte, tid) in self.touches.clone() {
            match cx.ui.drag(tid, k) {
                Some(g) => {
                    vues.push(tid);
                    let maintenant = self.temps;
                    let i = match self.tenues.iter().position(|t| t.id == tid) {
                        Some(i) => i,
                        None => {
                            self.tenues.push(Tenue { id: tid, sorte, rect: k, debut: maintenant, depart: (g.x, g.y), doigt: (g.x, g.y), repete: 0, curseur: 0, variantes: None });
                            self.tenues.len() - 1
                        }
                    };
                    let tenue = (maintenant - self.tenues[i].debut) as f32;
                    self.tenues[i].doigt = (g.x, g.y);
                    self.tenues[i].rect = k;
                    match sorte {
                        Sorte::Car(c) if self.tenues[i].variantes.is_none() && tenue >= self.reglages.appui_long => {
                            let v = self.choix(c);
                            if !v.is_empty() {
                                self.tenues[i].variantes = Some(v);
                            }
                        }
                        Sorte::Effacer if tenue >= 0.45 => {
                            let voulu = ((tenue - 0.45) / 0.06) as u32 + 1;
                            while self.tenues[i].repete < voulu {
                                out.push(Frappe::Effacer);
                                self.tenues[i].repete += 1;
                            }
                        }
                        Sorte::Espace => {
                            let pas = ((g.x - self.tenues[i].depart.0) / (12.0 * d)) as i32;
                            let t = &mut self.tenues[i];
                            while t.curseur < pas {
                                out.push(Frappe::Droite);
                                t.curseur += 1;
                            }
                            while t.curseur > pas {
                                out.push(Frappe::Gauche);
                                t.curseur -= 1;
                            }
                            // Un aller-retour reste un déplacement, pas une espace
                            if pas != 0 {
                                t.repete = 1;
                            }
                        }
                        _ => {}
                    }
                }
                None => {
                    if let Some(i) = self.tenues.iter().position(|t| t.id == tid) {
                        let mut t = self.tenues.remove(i);
                        if t.sorte == Sorte::Espace && t.repete > 0 {
                            t.curseur = t.curseur.max(1);
                        }
                        self.valider(&t, &mut out);
                    } else if let Some((x, y)) = cx.ui.clic_en(tid) {
                        let t = Tenue { id: tid, sorte, rect: k, debut: self.temps, depart: (x, y), doigt: (x, y), repete: 0, curseur: 0, variantes: None };
                        self.valider(&t, &mut out);
                    }
                }
            }
        }
        // Une touche disparue (la couche a changé sous le doigt) est oubliée
        self.tenues.retain(|t| vues.contains(&t.id));
        out
    }

    /// Dessine le clavier ouvert, et les bulles des touches tenues par-dessus tout.
    pub fn dessiner(&self, cx: &mut Cx) {
        if !self.ouvert() || self.touches.is_empty() {
            return;
        }
        let (d, col, r) = (cx.dp, cx.couleurs(), self.rect);
        let bh = self.barre(cx);
        if self.reglages.flottant {
            let rr = cx.theme.rayon(bh) * 0.6;
            cx.dl.shadow(Rect::new(r.x, r.y + 3.0 * d, r.w, r.h), rr, 14.0 * d, col.ombre);
            cx.dl.rrect(r, rr, col.fond);
            // La poignée du bandeau
            let tenu = self.prise.is_some();
            cx.dl.rrect(Rect::new(r.cx() - 18.0 * d, r.y + bh / 2.0 - 2.0 * d, 36.0 * d, 4.0 * d), 2.0 * d, if tenu { col.accent } else { col.ligne });
        } else {
            cx.dl.rrect(r, 0.0, col.fond);
            cx.dl.rrect(Rect::new(r.x, r.y, r.w, d.max(1.0)), 0.0, col.ligne);
        }
        cx.text_at(r.x + 18.0 * d, r.y + bh / 2.0, self.reglages.disposition.nom(), 13.0, col.pale);
        let b = Rect::new(r.right() - bh * 1.4, r.y, bh * 1.4, bh);
        cx.icon(Rect::new(b.cx() - bh * 0.3, b.cy() - bh * 0.3, bh * 0.6, bh * 0.6), Icon::Baisser, col.pale);
        let rayon = cx.theme.rayon(self.reglages.touche * d) * 0.35;
        for &(k, sorte, tid) in &self.touches {
            let c = k.inset((cx.theme.marge * 0.25 * d).round());
            let speciale = !matches!(sorte, Sorte::Car(_) | Sorte::Espace);
            let appui = cx.ui.pressed(tid);
            let fond = if appui {
                col.ligne
            } else if sorte == Sorte::Maj && self.majuscule() {
                col.accent
            } else if speciale {
                col.puce
            } else {
                col.surface
            };
            if cx.theme.marge >= 1.0 {
                cx.dl.shadow(Rect::new(c.x, c.y + d, c.w, c.h), rayon, 2.0 * d, col.ombre);
                cx.dl.rrect(c, rayon, fond);
            } else {
                // Aplati : les touches jointives restent séparées par un filet
                cx.dl.rrect(c, 0.0, col.ligne);
                cx.dl.rrect(c.inset(d.max(1.0) / 2.0), 0.0, fond);
            }
            let ic = Rect::new(c.cx() - c.h * 0.22, c.cy() - c.h * 0.22, c.h * 0.44, c.h * 0.44);
            match sorte {
                Sorte::Car(ch) => {
                    let s = if self.majuscule() { haut(ch) } else { ch }.to_string();
                    let w = cx.measure(&s, 20.0);
                    cx.text_at(c.cx() - w / 2.0, c.cy(), &s, 20.0, col.texte);
                    // Un point discret signale les variantes
                    if !variantes(ch).is_empty() {
                        cx.dl.rrect(Rect::new(c.right() - 7.0 * d, c.y + 4.0 * d, 3.0 * d, 3.0 * d), 1.5 * d, col.pale);
                    }
                }
                Sorte::Maj => {
                    cx.icon(ic, Icon::Maj, if self.majuscule() { col.sur_accent } else { col.texte });
                    if self.maj == Maj::Verrou {
                        cx.dl.rrect(Rect::new(ic.x + ic.w * 0.25, ic.bottom() + 2.0 * d, ic.w * 0.5, 2.0 * d), d, col.sur_accent);
                    }
                }
                Sorte::Effacer => cx.icon(ic, Icon::Effacer, col.texte),
                Sorte::Entree => cx.icon(ic, Icon::Entree, col.texte),
                Sorte::Espace => {
                    let w = cx.measure("espace", 12.0);
                    cx.text_at(c.cx() - w / 2.0, c.cy(), "espace", 12.0, col.pale);
                }
                Sorte::Couche => {
                    let s = if self.symboles { "ABC" } else { "?123" };
                    let w = cx.measure(s, 14.0);
                    cx.text_at(c.cx() - w / 2.0, c.cy(), s, 14.0, col.texte);
                }
            }
        }
        // Les bulles : les variantes, ou la touche sous le doigt
        for t in &self.tenues {
            let Sorte::Car(ch) = t.sorte else { continue };
            let rr = cx.theme.rayon(t.rect.h) * 0.4;
            if let Some(v) = &t.variantes {
                let (b, cw) = self.bande(t, v.len());
                let choisie = (((t.doigt.0 - b.x) / cw).floor().max(0.0) as usize).min(v.len() - 1);
                cx.dl.shadow(Rect::new(b.x, b.y + 2.0 * d, b.w, b.h), rr, 8.0 * d, col.ombre);
                cx.dl.rrect(b, rr, col.surface);
                for (i, c) in v.iter().enumerate() {
                    let cel = Rect::new(b.x + i as f32 * cw, b.y, cw, b.h);
                    let on = i == choisie;
                    if on {
                        cx.dl.rrect(cel.inset(3.0 * d), rr, col.accent);
                    }
                    let s = c.to_string();
                    let w = cx.measure(&s, 20.0);
                    cx.text_at(cel.cx() - w / 2.0, cel.cy(), &s, 20.0, if on { col.sur_accent } else { col.texte });
                }
            } else if self.reglages.apercu {
                let s = if self.majuscule() { haut(ch) } else { ch }.to_string();
                let b = Rect::new(t.rect.x - t.rect.w * 0.1, t.rect.y - t.rect.h * 1.15, t.rect.w * 1.2, t.rect.h * 1.1);
                cx.dl.shadow(Rect::new(b.x, b.y + 2.0 * d, b.w, b.h), rr, 8.0 * d, col.ombre);
                cx.dl.rrect(b, rr, col.surface);
                let w = cx.measure(&s, 28.0);
                cx.text_at(b.cx() - w / 2.0, b.cy(), &s, 28.0, col.texte);
            }
        }
    }
}

/// Un texte en cours d'édition, et son curseur (en caractères).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Edition {
    pub texte: String,
    pub curseur: usize,
}

impl Edition {
    pub fn new(texte: &str) -> Edition {
        Edition { texte: texte.to_string(), curseur: texte.chars().count() }
    }

    fn octet(&self, i: usize) -> usize {
        self.texte.char_indices().nth(i).map_or(self.texte.len(), |c| c.0)
    }

    /// Applique une frappe ; vrai pour Entrée.
    pub fn appliquer(&mut self, f: Frappe) -> bool {
        let n = self.texte.chars().count();
        self.curseur = self.curseur.min(n);
        match f {
            Frappe::Car(c) => {
                let o = self.octet(self.curseur);
                self.texte.insert(o, c);
                self.curseur += 1;
            }
            Frappe::Effacer if self.curseur > 0 => {
                let o = self.octet(self.curseur - 1);
                self.texte.remove(o);
                self.curseur -= 1;
            }
            Frappe::Gauche => self.curseur = self.curseur.saturating_sub(1),
            Frappe::Droite => self.curseur = (self.curseur + 1).min(n),
            Frappe::Entree => return true,
            _ => {}
        }
        false
    }
}

/// Un champ de texte d'une ligne dans une colonne de composants. Le toucher lui donne
/// le focus et place le curseur ; le clavier s'ouvre alors pour lui.
pub fn champ(cx: &mut Cx, b: &mut Body, cle: Id, vide: &str, ed: &mut Edition, clavier: &mut Clavier) {
    let (d, col) = (cx.dp, cx.couleurs());
    let r = Rect::new(b.x, b.y + (cx.theme.marge * 0.4 * d).round(), b.w, cx.commande());
    let focus = clavier.cible == Some(cle);
    let pad = cx.pad() * 0.75;
    // Le texte défile pour garder le curseur visible
    let avant: String = ed.texte.chars().take(ed.curseur).collect();
    let xc = cx.measure(&avant, 16.0);
    let decalage = (xc - (r.w - 2.0 * pad)).max(0.0);
    cx.ui.region_add(cle, r, Sense::Click { scroll: Some(b.id()) });
    if let Some((x, _)) = cx.ui.clic_en(cle) {
        clavier.ouvrir(cle);
        // Le curseur va au plus près du doigt
        let cible = x - (r.x + pad - decalage);
        let mut meilleur = (0, f32::MAX);
        let mut acc = String::new();
        for (i, c) in std::iter::once(None).chain(ed.texte.chars().map(Some)).enumerate() {
            if let Some(c) = c {
                acc.push(c);
            }
            let e = (cx.measure(&acc, 16.0) - cible).abs();
            if e < meilleur.1 {
                meilleur = (i, e);
            }
        }
        ed.curseur = meilleur.0;
    }
    cx.dl.rrect(r, cx.theme.rayon(r.h) * 0.4, if focus { col.accent } else { col.ligne });
    cx.dl.rrect(r.inset(if focus { 2.0 } else { 1.0 } * d), cx.theme.rayon(r.h) * 0.4 - d, col.surface);
    let interieur = Rect::new(r.x + pad, r.y, r.w - 2.0 * pad, r.h);
    cx.clip(Some(interieur.intersect(&b.rect())));
    if ed.texte.is_empty() {
        cx.text_at(interieur.x, r.cy(), vide, 16.0, col.pale);
    } else {
        cx.text_at(interieur.x - decalage, r.cy(), &ed.texte, 16.0, col.texte);
    }
    if focus {
        let x = interieur.x - decalage + cx.measure(&avant, 16.0);
        let h = cx.px(16.0) * 1.2;
        cx.dl.rrect(Rect::new(x, r.cy() - h / 2.0, 2.0 * d, h), d, col.accent);
    }
    // On revient au découpage de la colonne, pas à aucun : la suite resterait sinon
    // visible (et touchable) hors de la fenêtre
    cx.clip(Some(b.rect()));
    b.y += r.h + (cx.theme.marge * 1.2 * d).round();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draw::DrawList;
    use crate::text::Text;
    use crate::ui::{Event, Ui};

    const ECRAN: Rect = Rect::new(0., 0., 1000., 800.);

    struct Banc {
        c: Clavier,
        ui: Ui,
        text: Text,
        dl: DrawList,
        t: f64,
        ed: Edition,
    }

    impl Banc {
        fn new() -> Banc {
            let mut c = Clavier::default();
            c.ouvrir(1);
            let mut b = Banc { c, ui: Ui::new(1.0), text: Text::new(), dl: DrawList::default(), t: 0.0, ed: Edition::default() };
            b.image(0.0);
            b
        }
        fn image(&mut self, dt: f32) {
            self.dl.clear();
            let mut cx = Cx { ui: &mut self.ui, text: &mut self.text, dl: &mut self.dl, dp: 1.0, theme: Default::default() };
            let (_, r) = self.c.reserver(&cx, ECRAN);
            for f in self.c.saisir(&mut cx, r, dt) {
                self.ed.appliquer(f);
            }
            self.c.dessiner(&mut cx);
            self.ui.end_frame();
        }
        /// Le centre de la touche `n` (dans l'ordre des rangées).
        fn touche(&self, n: usize) -> (f32, f32) {
            let k = self.c.touches[n].0;
            (k.cx(), k.cy())
        }
        fn taper(&mut self, n: usize) {
            let (x, y) = self.touche(n);
            self.t += 0.1;
            self.ui.event(Event::Down { id: 0, x, y }, self.t);
            self.ui.event(Event::Up { id: 0 }, self.t + 0.05);
            self.image(0.05);
        }
        /// Tenir la touche `n` pendant `s` secondes, glisser de `dx`, lâcher.
        fn tenir(&mut self, n: usize, s: f32, dx: f32) {
            let (x, y) = self.touche(n);
            self.ui.event(Event::Down { id: 0, x, y }, self.t);
            let pas = 20;
            for k in 1..=pas {
                self.ui.event(Event::Move { id: 0, x: x + dx * k as f32 / pas as f32, y }, self.t);
                self.image(s / pas as f32);
            }
            self.ui.event(Event::Up { id: 0 }, self.t);
            self.image(0.0);
        }
    }

    // AZERTY : a=0 z=1 e=2 … p=9 ; q=10 … m=19 ; maj=20 w=21 … '=27 effacer=28 ;
    // ?123=29 ,=30 espace=31 .=32 entrée=33

    #[test]
    fn taper_ecrit_et_la_majuscule_ne_vaut_qu_une_fois() {
        let mut b = Banc::new();
        b.taper(20);
        b.taper(4); // T
        b.taper(2);
        b.taper(31);
        b.taper(0);
        assert_eq!(b.ed.texte, "Te a");
        b.taper(28);
        assert_eq!(b.ed.texte, "Te ");
        // Deux appuis rapprochés verrouillent la majuscule
        b.taper(20);
        b.taper(20);
        b.taper(0);
        b.taper(1);
        assert_eq!(b.ed.texte, "Te AZ");
    }

    #[test]
    fn l_appui_long_propose_les_variantes() {
        let mut b = Banc::new();
        // Tenir « e », puis glisser d'une case vers la droite : é
        let w = b.c.touches[2].0.w.max(1000.0 / 12.0);
        b.tenir(2, 0.6, w * 1.2);
        assert_eq!(b.ed.texte, "é");
        // Un appui trop court écrit la lettre
        b.tenir(2, 0.2, 0.0);
        assert_eq!(b.ed.texte, "ée");
    }

    #[test]
    fn effacer_se_repete_et_l_espace_deplace_le_curseur() {
        let mut b = Banc::new();
        b.ed = Edition::new("abcdefghij");
        b.tenir(28, 0.45 + 0.06 * 4.5, 0.0);
        assert!(b.ed.texte.len() <= 6, "{}", b.ed.texte);
        let reste = b.ed.texte.clone();
        // Glisser sur l'espace : le curseur recule, sans écrire d'espace
        b.tenir(31, 0.3, -36.0);
        assert_eq!(b.ed.texte, reste);
        assert_eq!(b.ed.curseur, reste.chars().count() - 3);
        b.taper(0);
        assert_eq!(b.ed.curseur, reste.chars().count() - 2);
    }

    #[test]
    fn la_couche_des_symboles() {
        let mut b = Banc::new();
        b.taper(29);
        b.taper(0);
        b.taper(29);
        b.taper(0);
        assert_eq!(b.ed.texte, "1a");
    }

    #[test]
    fn l_edition_suit_le_curseur() {
        let mut e = Edition::new("été");
        e.appliquer(Frappe::Gauche);
        e.appliquer(Frappe::Car('x'));
        assert_eq!((e.texte.as_str(), e.curseur), ("étxé", 3));
        e.appliquer(Frappe::Effacer);
        e.appliquer(Frappe::Effacer);
        assert_eq!((e.texte.as_str(), e.curseur), ("éé", 1));
        assert!(e.appliquer(Frappe::Entree));
    }
}
