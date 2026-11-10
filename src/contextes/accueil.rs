//! Le contexte Accueil : les contextes disponibles, en cartes (une icône généreuse, le
//! titre à droite, la légende dessous), sur autant de colonnes que la largeur le
//! permet. Toucher une carte ouvre ce contexte à la place de l'Accueil, dans la même
//! fenêtre. C'est le contexte des fenêtres nées du menu (« Côte à côte », « Dessous »).

use crate::contexte::{Commun, Contexte, Demande, Etat, Genre};
use crate::contextes::{couper, GENRES};
use crate::draw::Rect;
use crate::surface::Placee;
use crate::ui::{id, sub_id, Id};
use crate::widgets::{Cx, Icon};

pub const CLE: &str = "accueil";
pub const GENRE: Genre = Genre { cle: CLE, nom: "Accueil", legende: "Les contextes disponibles", icone: Icon::Maison, ouvrir };

const ACCUEIL: Id = id("accueil");
/// Largeur visée d'une carte : le nombre de colonnes en découle.
const CARTE_LARGE: f32 = 280.0;
/// Hauteur d'une carte (dp).
const CARTE_HAUTE: f32 = 92.0;

#[derive(Clone, Default)]
pub struct Accueil;

fn ouvrir(_e: &Etat) -> Box<dyn Contexte> {
    Box::new(Accueil)
}

impl Contexte for Accueil {
    fn cle(&self) -> &'static str {
        CLE
    }

    fn dupliquer(&self) -> Box<dyn Contexte> {
        Box::new(Accueil)
    }

    fn dessiner(&mut self, cx: &mut Cx, c: &mut Commun, p: &Placee) -> String {
        let r = p.contenu;
        let (d, col) = (cx.dp, cx.couleurs());
        cx.dl.rrect(r, 0.0, col.surface);
        let scroll = sub_id(ACCUEIL, p.id as usize);
        let mut b = cx.body(scroll, r, &c.scrolls);
        let haut = (40.0 * d * cx.theme.texte).round();
        cx.text_at(b.x, b.y + haut / 2.0, "Contextes", 20.0, col.texte);
        b.y += haut + cx.ecart();
        let genres: Vec<&Genre> = GENRES.iter().filter(|g| g.cle != CLE).collect();
        let ecart = cx.ecart().max(8.0 * d);
        let colonnes = (((b.w + ecart) / (CARTE_LARGE * d + ecart)).floor() as usize).clamp(1, genres.len().max(1));
        let large = (b.w - ecart * (colonnes - 1) as f32) / colonnes as f32;
        let haute = (CARTE_HAUTE * d * cx.theme.texte).round();
        let vue = b.rect();
        let mut choisi = None;
        for (i, g) in genres.iter().enumerate() {
            let (ligne, colonne) = (i / colonnes, i % colonnes);
            let carte = Rect::new(b.x + colonne as f32 * (large + ecart), b.y + ligne as f32 * (haute + ecart), large, haute);
            if carte.bottom() < vue.y || carte.y > vue.bottom() {
                continue;
            }
            let k = sub_id(scroll, 10 + i);
            if cx.ui.click(k, carte, Some(b.id())) {
                choisi = Some(g.cle);
            }
            let appui = cx.ui.pressed(k);
            let rayon = cx.theme.rayon(carte.h).min(16.0 * d);
            cx.dl.rrect(carte, rayon, if appui { col.accent } else { col.ligne });
            cx.dl.rrect(carte.inset(d.max(1.0)), rayon, if appui { col.puce } else { col.fond });
            // L'icône, généreuse, dans une pastille ; le titre à droite, la légende dessous
            let pad = cx.pad().max(10.0 * d);
            let ic = (carte.h - 2.0 * pad).clamp(24.0 * d, 64.0 * d);
            let pastille = Rect::new(carte.x + pad, carte.cy() - ic / 2.0, ic, ic);
            cx.dl.rrect(pastille, cx.theme.rayon(ic).min(ic / 2.0), col.puce);
            cx.icon(pastille.inset(ic * 0.18), g.icone, col.accent);
            let x = pastille.right() + pad;
            let w = (carte.right() - pad - x).max(0.0);
            cx.clip(Some(Rect::new(x, carte.y, w, carte.h).intersect(&vue)));
            let legende = couper(cx, g.legende, 13.0, w, 2);
            let lh = 17.0 * d * cx.theme.texte;
            let bloc = 22.0 * d * cx.theme.texte + lh * legende.len() as f32;
            let y0 = carte.cy() - bloc / 2.0;
            cx.text_at(x, y0 + 11.0 * d * cx.theme.texte, g.nom, 18.0, col.texte);
            for (n, l) in legende.iter().enumerate() {
                cx.text_at(x, y0 + 22.0 * d * cx.theme.texte + lh * (n as f32 + 0.5), l, 13.0, col.pale);
            }
            cx.clip(Some(vue));
        }
        let rangs = genres.len().div_ceil(colonnes);
        b.y += rangs as f32 * (haute + ecart);
        cx.paragraph(&mut b, "Toucher une carte ouvre le contexte dans cette fenêtre. Le menu de la barre de chaque fenêtre la partage en deux (ses deux icônes au-dessus de « Fermer la fenêtre » : côte à côte, ou l'une sous l'autre) : la nouvelle s'ouvre sur l'Accueil.");
        cx.body_end(b, &mut c.scrolls);
        if let Some(cle) = choisi {
            c.demandes.push(Demande::Ouvrir { depuis: p.id, cle, etat: Etat::default() });
        }
        format!("{} contextes", genres.len())
    }
}
