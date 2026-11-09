//! Composants tactiles : pastilles, pilules, onglets, interrupteurs, curseurs,
//! listes défilantes. Mode immédiat : chaque image redécrit l'interface.

use crate::draw::{Color, DrawList, Rect};
use crate::text::Text;
use crate::ui::{sub_id, Id, Sense, Ui};
use std::collections::HashMap;

/// Les couleurs de l'interface, tirées du thème.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Couleurs {
    pub fond: Color,
    pub surface: Color,
    pub texte: Color,
    pub pale: Color,
    pub accent: Color,
    /// Le texte et les icônes posés sur l'accent.
    pub sur_accent: Color,
    pub puce: Color,
    pub ligne: Color,
    pub ombre: Color,
}

/// Les accents proposés : nom, teinte sur fond clair, teinte sur fond sombre.
pub const ACCENTS: [(&str, Color, Color); 6] = [
    ("Indigo", [0x2A, 0x22, 0xD8, 255], [0x8B, 0x85, 0xFF, 255]),
    ("Bleu", [0x1D, 0x6F, 0xE0, 255], [0x5A, 0xA2, 0xFF, 255]),
    ("Turquoise", [0x0F, 0x76, 0x6E, 255], [0x2D, 0xD4, 0xBF, 255]),
    ("Vert", [0x15, 0x80, 0x3D, 255], [0x4A, 0xDE, 0x80, 255]),
    ("Orange", [0xC2, 0x41, 0x0C, 255], [0xFB, 0x92, 0x3C, 255]),
    ("Rose", [0xBE, 0x18, 0x5D, 255], [0xF4, 0x72, 0xB6, 255]),
];

/// L'apparence, réglable : la taille des commandes (pastilles, pilules, boutons,
/// onglets) et des lignes de liste en dp, l'échelle du texte, l'arrondi (0 : coins
/// presque droits, 1 : bouts ronds), l'accent et le thème sombre. `marge` : l'écart
/// autour des commandes, en dp ; à 0, l'affichage est aplati (commandes jointives,
/// sans ombre). `padding` : l'espace entre le bord d'une commande et son contenu, en
/// dp, jamais nul.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Theme {
    pub commande: f32,
    pub ligne: f32,
    pub texte: f32,
    pub arrondi: f32,
    pub accent: usize,
    pub sombre: bool,
    pub marge: f32,
    pub padding: f32,
}

impl Default for Theme {
    fn default() -> Theme {
        Theme { commande: 48.0, ligne: 48.0, texte: 1.0, arrondi: 1.0, accent: 0, sombre: false, marge: 10.0, padding: 16.0 }
    }
}

impl Theme {
    pub fn couleurs(&self) -> Couleurs {
        let (_, clair, fonce) = ACCENTS[self.accent.min(ACCENTS.len() - 1)];
        if self.sombre {
            Couleurs {
                fond: [0x14, 0x14, 0x1C, 255],
                surface: [0x1E, 0x1E, 0x2A, 255],
                texte: [0xEC, 0xEC, 0xF4, 255],
                pale: [0x9A, 0x9A, 0xB0, 255],
                accent: fonce,
                sur_accent: [0x14, 0x14, 0x1C, 255],
                puce: [0x2C, 0x2C, 0x3A, 255],
                ligne: [0x34, 0x34, 0x46, 255],
                ombre: [0, 0, 0, 110],
            }
        } else {
            Couleurs {
                fond: [0xF3, 0xF2, 0xFA, 255],
                surface: [255, 255, 255, 255],
                texte: [0x1B, 0x1B, 0x24, 255],
                pale: [0x6C, 0x6C, 0x80, 255],
                accent: clair,
                sur_accent: [255, 255, 255, 255],
                puce: [0xEF, 0xEE, 0xF6, 255],
                ligne: [0xE3, 0xE2, 0xEC, 255],
                ombre: [0x1B, 0x1B, 0x40, 60],
            }
        }
    }

    /// Le rayon des coins d'une commande de hauteur `h`.
    pub fn rayon(&self, h: f32) -> f32 {
        h / 2.0 * (0.12 + 0.88 * self.arrondi.clamp(0.0, 1.0))
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Icon {
    List,
    Sliders,
    Help,
    Close,
    /// Trois points : le menu d'une fenêtre.
    Menu,
    /// Un cube en perspective : une vue 3D.
    Cube,
    Plus,
    /// Les touches spéciales du clavier : majuscule, effacer, entrée, baisser le clavier.
    Maj,
    Effacer,
    Entree,
    Baisser,
    /// La rustine de l'application : bord orange denté, centre noir.
    Rustine,
    /// Une batterie remplie à `niveau` (0 à 100) ; en charge, un éclair.
    Batterie { niveau: u8, charge: bool },
    /// Un poste de radio, et les commandes de lecture.
    Radio,
    Lecture,
    Pause,
    Stop,
    /// L'explorateur : un dossier, un fichier, et par nature une note, une photo, un
    /// film ; une flèche vers le dossier parent.
    Dossier,
    Fichier,
    Note,
    Photo,
    Film,
    Haut,
    /// L'occupation du disque : un anneau en trois secteurs.
    Anneau,
}

/// Les deux couleurs de la rustine, quel que soit le thème.
pub const RUSTINE_ORANGE: Color = [0xF5, 0x82, 0x20, 255];
pub const RUSTINE_NOIR: Color = [0x1E, 0x1E, 0x25, 255];

/// Défilement de chaque zone : position et hauteur du contenu.
pub type Scrolls = HashMap<Id, (f32, f32)>;

pub struct Cx<'a> {
    pub ui: &'a mut Ui,
    pub text: &'a mut Text,
    pub dl: &'a mut DrawList,
    pub dp: f32,
    pub theme: Theme,
}

/// Colonne de composants dans une zone défilante.
pub struct Body {
    pub x: f32,
    pub w: f32,
    pub y: f32,
    top: f32,
    id: Id,
    rect: Rect,
}

impl Body {
    /// L'identifiant de la zone défilante, pour qu'un glissement parti d'un composant
    /// la fasse défiler.
    pub fn id(&self) -> Id {
        self.id
    }

    /// Le rectangle visible de la zone.
    pub fn rect(&self) -> Rect {
        self.rect
    }
}

impl Cx<'_> {
    pub fn couleurs(&self) -> Couleurs {
        self.theme.couleurs()
    }

    /// La hauteur des commandes, en pixels.
    pub fn commande(&self) -> f32 {
        (self.theme.commande * self.dp).round()
    }

    /// L'écart entre deux commandes voisines, en pixels.
    pub fn ecart(&self) -> f32 {
        (self.theme.marge * self.dp).round()
    }

    /// La marge entre le bord d'une zone et les commandes posées dans son coin.
    pub fn coin(&self) -> f32 {
        (self.theme.marge * 1.4 * self.dp).round()
    }

    /// L'espace entre le bord d'une commande et son contenu, en pixels.
    pub fn pad(&self) -> f32 {
        (self.theme.padding.max(1.0) * self.dp).round()
    }

    /// L'ombre d'une commande, sauf en affichage aplati (marge nulle).
    pub fn ombre(&mut self, r: Rect, rayon: f32, flou: f32) {
        if self.theme.marge >= 1.0 {
            let c = self.couleurs().ombre;
            self.dl.shadow(Rect::new(r.x, r.y + 2.0 * self.dp, r.w, r.h), rayon, flou * self.dp, c);
        }
    }

    /// Une taille de texte (dp) en pixels, à l'échelle du thème.
    pub fn px(&self, size: f32) -> f32 {
        (size * self.theme.texte * self.dp).round()
    }

    pub fn measure(&mut self, s: &str, size: f32) -> f32 {
        let px = self.px(size);
        self.text.measure(s, px)
    }

    fn label(&mut self, x: f32, cy: f32, s: &str, size: f32, color: Color) {
        let px = self.px(size);
        let base = cy + self.text.center_offset(px);
        self.text.draw(self.dl, x, base, s, px, color);
    }

    pub fn text_at(&mut self, x: f32, cy: f32, s: &str, size: f32, color: Color) {
        self.label(x, cy, s, size, color);
    }

    /// Texte en police à chasse fixe (lignes d'état), centré verticalement sur `cy`.
    /// Largeur d'un texte en chasse fixe.
    pub fn mesure_mono(&mut self, s: &str, size: f32) -> f32 {
        let px = self.px(size);
        self.text.mesurer(s, px, true)
    }

    pub fn mono_at(&mut self, x: f32, cy: f32, s: &str, size: f32, color: Color) {
        let px = self.px(size);
        let base = cy + self.text.center_offset(px);
        self.text.dessiner(self.dl, x, base, s, px, color, true);
    }

    /// Texte calé à droite sur `right`.
    pub fn text_right(&mut self, right: f32, cy: f32, s: &str, size: f32, color: Color) {
        let w = self.measure(s, size);
        self.label(right - w, cy, s, size, color);
    }

    pub fn centered(&mut self, r: Rect, s: &str, size: f32, color: Color) {
        let w = self.measure(s, size);
        self.label(r.cx() - w / 2.0, r.cy(), s, size, color);
    }

    pub fn clip(&mut self, c: Option<Rect>) {
        self.dl.set_clip(c);
        self.ui.set_clip(c);
    }

    pub fn icon(&mut self, r: Rect, icon: Icon, color: Color) {
        let (cx, cy, u) = (r.cx(), r.cy(), r.w.min(r.h) / 24.0);
        let t = 2.2 * u;
        let p = |x: f32, y: f32| [cx + x * u, cy + y * u];
        match icon {
            Icon::List => {
                for y in [-6.0, 0.0, 6.0] {
                    self.dl.rrect(Rect::new(cx - 8.5 * u - 1.3 * u, cy + (y - 1.3) * u, 2.6 * u, 2.6 * u), 1.3 * u, color);
                    self.dl.line(p(-4.0, y), p(9.0, y), t, color);
                }
            }
            Icon::Sliders => {
                for (y, k) in [(-6.0, 3.0), (0.0, -3.5), (6.0, 2.0)] {
                    self.dl.line(p(-9.0, y), p(9.0, y), 1.6 * u, color);
                    self.dl.rrect(Rect::new(cx + (k - 2.5) * u, cy + (y - 2.5) * u, 5.0 * u, 5.0 * u), 2.5 * u, color);
                }
            }
            Icon::Help => self.centered(r, "?", 0.85 * r.h / self.dp / self.theme.texte, color),
            Icon::Menu => {
                for y in [-6.5, 0.0, 6.5] {
                    self.dl.rrect(Rect::new(cx - 1.7 * u, cy + (y - 1.7) * u, 3.4 * u, 3.4 * u), 1.7 * u, color);
                }
            }
            Icon::Cube => {
                let (a, b) = (8.7, 5.0);
                let v = [p(0.0, -10.0), p(a, -b), p(a, b), p(0.0, 10.0), p(-a, b), p(-a, -b)];
                for i in 0..6 {
                    self.dl.line(v[i], v[(i + 1) % 6], 1.8 * u, color);
                }
                // Les arêtes vues depuis le coin le plus proche
                let o = p(0.0, 0.0);
                for k in [1, 3, 5] {
                    self.dl.line(o, v[k], 1.8 * u, color);
                }
            }
            Icon::Maj => {
                let v = [p(0.0, -9.0), p(8.0, 0.0), p(4.0, 0.0), p(4.0, 8.0), p(-4.0, 8.0), p(-4.0, 0.0), p(-8.0, 0.0)];
                for i in 0..v.len() {
                    self.dl.line(v[i], v[(i + 1) % v.len()], 1.8 * u, color);
                }
            }
            Icon::Effacer => {
                let v = [p(-10.0, 0.0), p(-4.0, -7.0), p(9.0, -7.0), p(9.0, 7.0), p(-4.0, 7.0)];
                for i in 0..v.len() {
                    self.dl.line(v[i], v[(i + 1) % v.len()], 1.8 * u, color);
                }
                self.dl.line(p(-1.0, -3.0), p(5.0, 3.0), 1.8 * u, color);
                self.dl.line(p(5.0, -3.0), p(-1.0, 3.0), 1.8 * u, color);
            }
            Icon::Entree => {
                self.dl.line(p(8.0, -7.0), p(8.0, 2.0), t, color);
                self.dl.line(p(8.0, 2.0), p(-8.0, 2.0), t, color);
                self.dl.line(p(-8.0, 2.0), p(-3.5, -2.5), t, color);
                self.dl.line(p(-8.0, 2.0), p(-3.5, 6.5), t, color);
            }
            Icon::Baisser => {
                self.dl.line(p(-7.0, -3.0), p(0.0, 4.0), t, color);
                self.dl.line(p(0.0, 4.0), p(7.0, -3.0), t, color);
            }
            Icon::Rustine => {
                // Huit dents arrondies autour d'un disque orange, puis le centre noir
                let r = r.w.min(r.h) / 2.0;
                for k in 0..8 {
                    let a = k as f32 * std::f32::consts::FRAC_PI_4 - std::f32::consts::FRAC_PI_2;
                    let (x, y) = (cx + 0.8 * r * a.cos(), cy + 0.8 * r * a.sin());
                    let t = 0.22 * r;
                    self.dl.rrect(Rect::new(x - t, y - t, 2.0 * t, 2.0 * t), t, RUSTINE_ORANGE);
                }
                self.dl.rrect(Rect::new(cx - 0.86 * r, cy - 0.86 * r, 1.72 * r, 1.72 * r), 0.86 * r, RUSTINE_ORANGE);
                self.dl.rrect(Rect::new(cx - 0.66 * r, cy - 0.66 * r, 1.32 * r, 1.32 * r), 0.66 * r, RUSTINE_NOIR);
            }
            Icon::Batterie { niveau, charge } => {
                let (w, h, e) = (18.0 * u, 10.0 * u, 1.5 * u);
                let b = Rect::new(cx - w / 2.0 - u, cy - h / 2.0, w, h);
                self.dl.rrect(b, 2.0 * u, color);
                self.dl.rrect(b.inset(e), 1.0 * u, self.theme.couleurs().surface);
                let plein = b.inset(e + u);
                let f = (niveau.min(100) as f32 / 100.0) * plein.w;
                let teinte = if charge { self.theme.couleurs().accent } else if niveau <= 15 { [0xE0, 0x3A, 0x3A, 255] } else { color };
                self.dl.rrect(Rect::new(plein.x, plein.y, f, plein.h), 0.6 * u, teinte);
                self.dl.rrect(Rect::new(b.right(), cy - 2.5 * u, 2.0 * u, 5.0 * u), u, color);
                if charge {
                    let s = self.theme.couleurs().surface;
                    self.dl.line(p(1.0, -5.5), p(-2.0, 0.5), 2.2 * u, s);
                    self.dl.line(p(-2.0, 0.5), p(1.0, 0.5), 2.2 * u, s);
                    self.dl.line(p(1.0, 0.5), p(-2.0, 5.5), 2.2 * u, s);
                }
            }
            Icon::Plus => {
                self.dl.line(p(-8.0, 0.0), p(8.0, 0.0), t, color);
                self.dl.line(p(0.0, -8.0), p(0.0, 8.0), t, color);
            }
            Icon::Radio => {
                // Le boîtier, l'anse, le haut-parleur et le cadran
                let e = 1.9 * u;
                for (a, b) in [((-9.5, -5.5), (9.5, -5.5)), ((9.5, -5.5), (9.5, 9.5)), ((9.5, 9.5), (-9.5, 9.5)), ((-9.5, 9.5), (-9.5, -5.5))] {
                    self.dl.line(p(a.0, a.1), p(b.0, b.1), e, color);
                }
                self.dl.line(p(-4.0, -5.5), p(5.0, -10.0), e, color);
                self.dl.rrect(Rect::new(cx - 8.0 * u, cy + 1.5 * u, 6.0 * u, 6.0 * u), 3.0 * u, color);
                self.dl.line(p(-6.5, -1.5), p(6.5, -1.5), 2.6 * u, color);
                self.dl.line(p(4.0, 2.5), p(4.0, 7.0), 2.2 * u, color);
            }
            Icon::Lecture => {
                // Triangle plein, empilé en traits verticaux arrondis
                let n = 10;
                for i in 0..=n {
                    let x = -5.0 + 12.0 * i as f32 / n as f32;
                    let h = 7.5 * (1.0 - (x + 5.0) / 13.0);
                    self.dl.line(p(x, -h), p(x, h), 1.8 * u, color);
                }
            }
            Icon::Pause => {
                for x in [-6.5, 2.5] {
                    self.dl.rrect(Rect::new(cx + x * u, cy - 7.5 * u, 4.0 * u, 15.0 * u), 1.2 * u, color);
                }
            }
            Icon::Dossier => {
                self.dl.rrect(Rect::new(cx - 10.0 * u, cy - 8.5 * u, 9.0 * u, 5.0 * u), 1.5 * u, color);
                self.dl.rrect(Rect::new(cx - 10.0 * u, cy - 5.5 * u, 20.0 * u, 14.0 * u), 2.0 * u, color);
            }
            Icon::Fichier | Icon::Note | Icon::Photo | Icon::Film => {
                let e = 1.8 * u;
                let trait_ = |s: &mut Self, l: &[(f32, f32)]| {
                    for w in l.windows(2) {
                        s.dl.line(p(w[0].0, w[0].1), p(w[1].0, w[1].1), e, color);
                    }
                };
                match icon {
                    Icon::Fichier => {
                        trait_(self, &[(-6.5, -9.5), (2.5, -9.5), (7.0, -5.0), (7.0, 9.5), (-6.5, 9.5), (-6.5, -9.5)]);
                        trait_(self, &[(2.5, -9.5), (2.5, -5.0), (7.0, -5.0)]);
                    }
                    Icon::Note => {
                        self.dl.rrect(Rect::new(cx - 7.5 * u, cy + 2.5 * u, 7.0 * u, 7.0 * u), 3.5 * u, color);
                        trait_(self, &[(-1.4, 6.0), (-1.4, -9.0), (6.5, -6.0), (6.5, -3.0), (-1.4, -6.0)]);
                    }
                    Icon::Photo => {
                        trait_(self, &[(-9.5, -7.5), (9.5, -7.5), (9.5, 7.5), (-9.5, 7.5), (-9.5, -7.5)]);
                        trait_(self, &[(-7.0, 5.0), (-2.0, -1.0), (2.0, 3.0), (4.0, 1.0), (7.0, 5.0)]);
                        self.dl.rrect(Rect::new(cx + 2.5 * u, cy - 5.0 * u, 3.0 * u, 3.0 * u), 1.5 * u, color);
                    }
                    _ => {
                        trait_(self, &[(-9.5, -7.5), (9.5, -7.5), (9.5, 7.5), (-9.5, 7.5), (-9.5, -7.5)]);
                        trait_(self, &[(-2.5, -3.5), (3.5, 0.0), (-2.5, 3.5), (-2.5, -3.5)]);
                    }
                }
            }
            Icon::Anneau => {
                let q = std::f32::consts::FRAC_PI_2;
                for (a0, a1) in [(-q, 0.55), (0.55, 2.2), (2.2, 3.0 * q)] {
                    self.dl.secteur([cx, cy], 4.5 * u, 10.5 * u, a0, a1, 1.6 * u, color);
                }
            }
            Icon::Haut => {
                self.dl.line(p(0.0, 8.0), p(0.0, -8.0), t, color);
                self.dl.line(p(-6.5, -1.5), p(0.0, -8.0), t, color);
                self.dl.line(p(6.5, -1.5), p(0.0, -8.0), t, color);
            }
            Icon::Stop => self.dl.rrect(Rect::new(cx - 7.0 * u, cy - 7.0 * u, 14.0 * u, 14.0 * u), 2.0 * u, color),
            Icon::Close => {
                self.dl.line(p(-6.0, -6.0), p(6.0, 6.0), t, color);
                self.dl.line(p(6.0, -6.0), p(-6.0, 6.0), t, color);
            }
        }
    }

    /// Bouton flottant à icône, carré de la taille des commandes, à partir de (x, y).
    pub fn pastille(&mut self, id: Id, x: f32, y: f32, icon: Icon, active: bool) -> bool {
        let h = self.commande();
        let r = Rect::new(x, y, h, h);
        let c = self.couleurs();
        let clicked = self.ui.click(id, r, None);
        let pressed = self.ui.pressed(id);
        let rayon = self.theme.rayon(h);
        self.ombre(r, rayon, 8.0);
        let bg = if active { c.accent } else if pressed { c.puce } else { c.surface };
        self.dl.rrect(r, rayon, bg);
        let inset = (self.pad() * 0.6).min(r.w * 0.35);
        self.icon(r.inset(inset), icon, if active { c.sur_accent } else { c.accent });
        clicked
    }

    /// Pastille aplatie, pour une barre : carrée, sans marge ni ombre.
    pub fn pastille_plate(&mut self, id: Id, r: Rect, icon: Icon, active: bool) -> bool {
        let c = self.couleurs();
        let clicked = self.ui.click(id, r, None);
        if active {
            self.dl.rrect(r, 0.0, c.accent);
        } else if self.ui.pressed(id) {
            self.dl.rrect(r, 0.0, c.puce);
        }
        let inset = (self.pad() * 0.6).min(r.w * 0.35);
        self.icon(r.inset(inset), icon, if active { c.sur_accent } else { c.accent });
        clicked
    }

    /// Pilule flottante, à la taille des commandes, à partir de (x, y).
    pub fn pill(&mut self, id: Id, x: f32, y: f32, label: &str, active: bool) -> bool {
        let r = Rect::new(x, y, self.pill_width(label), self.commande());
        let c = self.couleurs();
        let clicked = self.ui.click(id, r, None);
        let pressed = self.ui.pressed(id);
        let rayon = self.theme.rayon(r.h);
        self.ombre(r, rayon, 6.0);
        self.dl.rrect(r, rayon, if active { c.accent } else if pressed { c.puce } else { c.surface });
        self.centered(r, label, 15.0, if active { c.sur_accent } else { c.texte });
        clicked
    }

    pub fn pill_width(&mut self, label: &str) -> f32 {
        self.measure(label, 15.0) + 2.0 * self.pad()
    }

    /// Rangée d'onglets, à la taille des commandes, à partir de (x, y). Renvoie
    /// l'onglet choisi.
    pub fn tabs(&mut self, id: Id, x: f32, y: f32, labels: &[&str], selected: usize) -> usize {
        let (mut x, mut result) = (x, selected);
        let (h, c) = (self.commande(), self.couleurs());
        for (i, label) in labels.iter().enumerate() {
            let w = self.measure(label, 15.0) + 2.0 * self.pad();
            let t = Rect::new(x, y, w, h);
            if self.ui.click(sub_id(id, i), t, None) {
                result = i;
            }
            let on = i == selected;
            self.dl.rrect(t, self.theme.rayon(h), if on { c.accent } else { c.puce });
            self.centered(t, label, 15.0, if on { c.sur_accent } else { c.texte });
            x += w + (self.theme.marge * 0.6 * self.dp).round();
        }
        result
    }

    pub fn body(&mut self, id: Id, rect: Rect, scrolls: &Scrolls) -> Body {
        self.ui.region_add(id, rect, Sense::Scroll);
        let delta = self.ui.take_scroll(id);
        let (off, content) = scrolls.get(&id).copied().unwrap_or((0.0, 0.0));
        let off = (off - delta).clamp(0.0, (content - rect.h).max(0.0));
        self.clip(Some(rect));
        let pad = 14.0 * self.dp;
        Body { x: rect.x + pad, w: rect.w - 2.0 * pad, y: rect.y + 8.0 * self.dp - off, top: rect.y - off, id, rect }
    }

    pub fn body_end(&mut self, b: Body, scrolls: &mut Scrolls) {
        self.clip(None);
        let content = b.y - b.top + 12.0 * self.dp;
        let off = b.rect.y - b.top;
        scrolls.insert(b.id, (off, content));
        if content > b.rect.h {
            let h = (b.rect.h * b.rect.h / content).max(32.0 * self.dp);
            let y = b.rect.y + off / (content - b.rect.h) * (b.rect.h - h);
            self.dl.rrect(Rect::new(b.rect.right() - 6.0 * self.dp, y, 3.0 * self.dp, h), 1.5 * self.dp, [0x6C, 0x6C, 0x80, 120]);
        }
    }

    pub fn section(&mut self, b: &mut Body, title: &str) {
        b.y += 10.0 * self.dp;
        let c = self.couleurs();
        self.label(b.x, b.y + 12.0 * self.dp * self.theme.texte, title, 12.5, c.pale);
        b.y += (10.0 + 18.0 * self.theme.texte) * self.dp;
    }

    pub fn paragraph(&mut self, b: &mut Body, s: &str) {
        let (px, c, k) = (self.px(15.0), self.couleurs(), self.theme.texte);
        for line in self.text.wrap(s, px, b.w) {
            self.label(b.x, b.y + 11.0 * self.dp * k, &line, 15.0, c.texte);
            b.y += 22.0 * self.dp * k;
        }
        b.y += 8.0 * self.dp;
    }

    /// Ligne de liste bord à bord. Renvoie vrai à l'appui.
    pub fn row(&mut self, b: &mut Body, id: Id, label: &str, swatch: Option<Color>, selected: bool) -> bool {
        let r = Rect::new(b.rect.x, b.y, b.rect.w, (self.theme.ligne * self.dp).round());
        let c = self.couleurs();
        let clicked = self.ui.click(id, r, Some(b.id));
        if self.ui.pressed(id) || selected {
            self.dl.rrect(r.inset(2.0 * self.dp), self.theme.rayon(r.h) * 0.45, c.puce);
        }
        let mut x = b.x;
        if let Some(sw) = swatch {
            let s = 22.0 * self.dp;
            self.dl.rrect(Rect::new(x, r.cy() - s / 2.0, s, s), 6.0 * self.dp * self.theme.arrondi.max(0.2), sw);
            x += s + 12.0 * self.dp;
        }
        self.label(x, r.cy(), label, 16.0, if selected { c.accent } else { c.texte });
        b.y += r.h;
        clicked
    }

    pub fn toggle(&mut self, b: &mut Body, id: Id, label: &str, value: &mut bool) {
        let r = Rect::new(b.x, b.y, b.w, (self.theme.ligne * self.dp).round());
        let c = self.couleurs();
        if self.ui.click(id, r, Some(b.id)) {
            *value = !*value;
        }
        self.label(b.x, r.cy(), label, 16.0, c.texte);
        let (w, h) = (46.0 * self.dp, 26.0 * self.dp);
        let track = Rect::new(r.right() - w, r.cy() - h / 2.0, w, h);
        let rt = self.theme.rayon(h);
        self.dl.rrect(track, rt, if *value { c.accent } else { c.ligne });
        let k = h - 6.0 * self.dp;
        let kx = if *value { track.right() - k - 3.0 * self.dp } else { track.x + 3.0 * self.dp };
        self.dl.rrect(Rect::new(kx, track.y + 3.0 * self.dp, k, k), self.theme.rayon(k), if *value { c.sur_accent } else { c.surface });
        b.y += r.h;
    }

    /// Curseur. Il laisse à sa gauche une marge de la taille d'une pastille et de son
    /// écart : le doigt y fait défiler la colonne, même quand les curseurs se suivent.
    pub fn slider(&mut self, b: &mut Body, id: Id, label: &str, shown: &str, value: &mut f32, lo: f32, hi: f32) {
        let marge = (self.commande() + self.ecart()).min(b.w * 0.4);
        let r = Rect::new(b.x + marge, b.y, b.w - marge, (64.0 * self.dp * self.theme.texte.max(1.0)).round());
        let c = self.couleurs();
        let ty = r.y + 16.0 * self.dp * self.theme.texte;
        self.label(r.x, ty, label, 16.0, c.texte);
        self.text_right(r.right(), ty, shown, 14.0, c.pale);
        let track_y = r.bottom() - 20.0 * self.dp;
        let knob = 11.0 * self.dp;
        let (x0, x1) = (r.x + knob, r.right() - knob);
        if let Some(d) = self.ui.drag(id, r) {
            *value = lo + ((d.x - x0) / (x1 - x0)).clamp(0.0, 1.0) * (hi - lo);
        }
        let t = 4.0 * self.dp;
        let at = x0 + (*value - lo) / (hi - lo) * (x1 - x0);
        self.dl.rrect(Rect::new(x0, track_y - t / 2.0, x1 - x0, t), t / 2.0, c.ligne);
        self.dl.rrect(Rect::new(x0, track_y - t / 2.0, at - x0, t), t / 2.0, c.accent);
        let grow = if self.ui.pressed(id) { 2.0 * self.dp } else { 0.0 };
        self.dl.rrect(Rect::new(at - knob - grow, track_y - knob - grow, 2.0 * (knob + grow), 2.0 * (knob + grow)), self.theme.rayon(2.0 * (knob + grow)), c.accent);
        b.y += r.h;
    }

    pub fn button(&mut self, b: &mut Body, id: Id, label: &str) -> bool {
        let r = Rect::new(b.x, b.y + (self.theme.marge * 0.4 * self.dp).round(), b.w, self.commande());
        let c = self.couleurs();
        let clicked = self.ui.click(id, r, Some(b.id));
        self.dl.rrect(r, self.theme.rayon(r.h), if self.ui.pressed(id) { c.ligne } else { c.puce });
        self.centered(r, label, 15.0, c.accent);
        b.y += r.h + (self.theme.marge * 1.2 * self.dp).round();
        clicked
    }
}
