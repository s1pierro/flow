//! Texte : polices du système, glyphes dessinés à la demande dans un atlas.
//! Aucun appel OpenGL : le moteur de rendu envoie l'atlas quand `dirty` est levé.

use crate::draw::{Color, DrawList, Rect};
use std::collections::HashMap;

pub const ATLAS: usize = 2048;
const FONTS: [&str; 3] = ["/system/fonts/RobotoStatic-Regular.ttf", "/system/fonts/Roboto-Regular.ttf", "/system/fonts/DroidSans.ttf"];
/// La police à chasse fixe, pour les lignes d'état ; à défaut, la police principale.
const MONO: [&str; 3] = ["/system/fonts/DroidSansMono.ttf", "/system/fonts/RobotoMono-Regular.ttf", "/system/fonts/CutiveMono.ttf"];

#[derive(Clone, Copy)]
struct Glyph {
    rect: Rect, // relatif au point de base
    uv0: [f32; 2],
    uv1: [f32; 2],
    advance: f32,
}

pub struct Text {
    font: Option<fontdue::Font>,
    mono: Option<fontdue::Font>,
    glyphs: HashMap<(char, u32, bool), Glyph>,
    pub atlas: Vec<u8>,
    pub dirty: bool,
    /// Levé quand l'atlas plein a été vidé : les glyphes déjà posés dans l'image en
    /// cours pointent vers l'ancien, il faut redessiner une image.
    pub refaire: bool,
    shelf: (usize, usize, usize), // x, y, hauteur de la rangée courante
}

impl Text {
    /// Sans police lisible, l'application reste utilisable, le texte n'est simplement pas dessiné.
    pub fn new() -> Text {
        let charger = |chemins: &[&str]| {
            chemins.iter().find_map(|p| {
                let bytes = std::fs::read(p).ok()?;
                fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default()).ok()
            })
        };
        let (font, mono) = (charger(&FONTS), charger(&MONO));
        Text { font, mono, glyphs: HashMap::new(), atlas: vec![0; ATLAS * ATLAS], dirty: true, refaire: false, shelf: (1, 1, 0) }
    }

    fn glyph(&mut self, c: char, px: f32, mono: bool) -> Option<Glyph> {
        let key = (c, px.to_bits(), mono);
        if let Some(g) = self.glyphs.get(&key) {
            return Some(*g);
        }
        let font = if mono { self.mono.as_ref().or(self.font.as_ref()) } else { self.font.as_ref() };
        let (m, bitmap) = font?.rasterize(c, px);
        let (w, h) = (m.width, m.height);
        let (mut x, mut y, mut row) = self.shelf;
        if x + w + 1 >= ATLAS {
            (x, y, row) = (1, y + row + 1, 0);
        }
        if y + h + 1 >= ATLAS {
            // Atlas plein (beaucoup de tailles différentes) : on le vide et on le
            // remplit de nouveau, plutôt que de laisser des caractères invisibles qui
            // prennent quand même leur place
            if self.glyphs.is_empty() || h + 2 >= ATLAS || w + 2 >= ATLAS {
                return None;
            }
            self.glyphs.clear();
            self.atlas.fill(0);
            self.refaire = true;
            (x, y, row) = (1, 1, 0);
        }
        for line in 0..h {
            let at = (y + line) * ATLAS + x;
            self.atlas[at..at + w].copy_from_slice(&bitmap[line * w..(line + 1) * w]);
        }
        self.shelf = (x + w + 1, y, row.max(h));
        self.dirty = true;
        let t = ATLAS as f32;
        let g = Glyph {
            rect: Rect::new(m.xmin as f32, -(m.ymin as f32) - h as f32, w as f32, h as f32),
            uv0: [x as f32 / t, y as f32 / t],
            uv1: [(x + w) as f32 / t, (y + h) as f32 / t],
            advance: m.advance_width,
        };
        self.glyphs.insert(key, g);
        Some(g)
    }

    pub fn measure(&mut self, s: &str, px: f32) -> f32 {
        self.mesurer(s, px, false)
    }

    /// `mono` : la police à chasse fixe.
    pub fn mesurer(&mut self, s: &str, px: f32, mono: bool) -> f32 {
        s.chars().map(|c| self.glyph(c, px, mono).map_or(px * 0.55, |g| g.advance)).sum()
    }

    /// Décalage entre le centre d'une ligne et son point de base.
    pub fn center_offset(&self, px: f32) -> f32 {
        match self.font.as_ref().and_then(|f| f.horizontal_line_metrics(px)) {
            Some(m) => (m.ascent + m.descent) / 2.0,
            None => px * 0.35,
        }
    }

    pub fn draw(&mut self, dl: &mut DrawList, x: f32, baseline: f32, s: &str, px: f32, color: Color) {
        self.dessiner(dl, x, baseline, s, px, color, false);
    }

    /// `mono` : la police à chasse fixe.
    #[allow(clippy::too_many_arguments)]
    pub fn dessiner(&mut self, dl: &mut DrawList, x: f32, baseline: f32, s: &str, px: f32, color: Color, mono: bool) {
        let (mut pen, base) = (x, baseline.round());
        for c in s.chars() {
            if let Some(g) = self.glyph(c, px, mono) {
                if g.rect.w > 0.0 {
                    let r = Rect::new((pen + g.rect.x).round(), base + g.rect.y, g.rect.w, g.rect.h);
                    dl.glyph(r, g.uv0, g.uv1, color);
                }
                pen += g.advance;
            }
        }
    }

    /// Découpe `s` en lignes de largeur `w` au plus, aux espaces.
    pub fn wrap(&mut self, s: &str, px: f32, w: f32) -> Vec<String> {
        let mut lines = Vec::new();
        let mut line = String::new();
        for word in s.split_whitespace() {
            let candidate = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
            if self.measure(&candidate, px) > w && !line.is_empty() {
                lines.push(std::mem::replace(&mut line, word.to_string()));
            } else {
                line = candidate;
            }
        }
        if !line.is_empty() {
            lines.push(line);
        }
        lines
    }
}

impl Default for Text {
    fn default() -> Text {
        Text::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn l_atlas_plein_se_vide_au_lieu_de_perdre_des_caracteres() {
        let mut t = Text::new();
        if t.font.is_none() {
            return; // pas de police système ici
        }
        // De grandes tailles remplissent l'atlas
        let mut px = 40.0;
        while !t.refaire && px < 2000.0 {
            for c in "abcdefghijklmnopqrstuvwxyz".chars() {
                t.glyph(c, px, false);
            }
            px += 7.0;
        }
        assert!(t.refaire, "l'atlas aurait dû se remplir");
        assert!(t.glyph('k', 61.0, false).is_some());
    }
}
