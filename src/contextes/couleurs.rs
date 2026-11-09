//! Le contexte Couleurs : la liste des couleurs du cube. Le choix est commun à toutes
//! les fenêtres (`Atelier::couleur`).

use crate::contexte::{Commun, Contexte, Etat, Genre, COULEURS};
use crate::surface::Placee;
use crate::ui::{id, sub_id, Id};
use crate::widgets::{Cx, Icon};

pub const CLE: &str = "couleurs";
pub const GENRE: Genre = Genre { cle: CLE, nom: "Couleurs", icone: Icon::List, ouvrir };

const LISTE: Id = id("liste-fenetre");

pub struct Couleurs;

fn ouvrir(_: &Etat) -> Box<dyn Contexte> {
    Box::new(Couleurs)
}

impl Contexte for Couleurs {
    fn cle(&self) -> &'static str {
        CLE
    }

    fn dupliquer(&self) -> Box<dyn Contexte> {
        Box::new(Couleurs)
    }

    fn dessiner(&mut self, cx: &mut Cx, c: &mut Commun, p: &Placee) -> String {
        let r = p.contenu;
        cx.dl.rrect(r, 0.0, cx.couleurs().surface);
        let scroll = sub_id(LISTE, p.id as usize);
        let mut b = cx.body(scroll, r, &c.scrolls);
        for (i, (name, color)) in COULEURS.iter().enumerate() {
            if cx.row(&mut b, sub_id(scroll, i), name, Some(*color), i == c.atelier.couleur) {
                c.atelier.couleur = i;
            }
        }
        cx.body_end(b, &mut c.scrolls);
        format!("{} couleurs · {}", COULEURS.len(), COULEURS[c.atelier.couleur].0)
    }
}
