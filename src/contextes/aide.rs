//! Le contexte Aide : comment se servir des fenêtres.

use crate::contexte::{Commun, Contexte, Etat, Genre};
use crate::surface::Placee;
use crate::ui::{id, sub_id, Id};
use crate::widgets::{Cx, Icon};

pub const CLE: &str = "aide";
pub const GENRE: Genre = Genre { cle: CLE, nom: "Aide", legende: "Le mode d'emploi de Flow", icone: Icon::Help, ouvrir, applet: None };

const AIDE: Id = id("aide-fenetre");

pub struct Aide;

fn ouvrir(_: &Etat) -> Box<dyn Contexte> {
    Box::new(Aide)
}

impl Contexte for Aide {
    fn cle(&self) -> &'static str {
        CLE
    }

    fn dupliquer(&self) -> Box<dyn Contexte> {
        Box::new(Aide)
    }

    fn dessiner(&mut self, cx: &mut Cx, c: &mut Commun, p: &Placee) -> String {
        let r = p.contenu;
        cx.dl.rrect(r, 0.0, cx.couleurs().surface);
        let mut b = cx.body(sub_id(AIDE, p.id as usize), r, &c.scrolls);
        cx.section(&mut b, "FENÊTRES");
        cx.paragraph(&mut b, "Touchez l'icône ou le titre d'une fenêtre pour ouvrir son menu : choisir son contexte, la partager en deux (les deux icônes au-dessus de « Fermer la fenêtre » : côte à côte, ou l'une sous l'autre ; la nouvelle s'ouvre sur l'Accueil), ou la fermer.");
        cx.paragraph(&mut b, "La poignée du coin bas droit déplace les bords de la fenêtre : un tiret montre chaque bord qu'elle peut déplacer.");
        cx.section(&mut b, "CONTEXTES");
        cx.paragraph(&mut b, "Chaque fenêtre a son propre contexte et son propre état : deux fenêtres Fichiers montrent chacune leur dossier. La disposition et l'état des fenêtres sont retrouvés au lancement suivant.");
        if let Some(f) = &c.fichier_espace {
            cx.paragraph(&mut b, &format!("L'espace de travail est enregistré dans {f}, un fichier texte."));
        }
        cx.section(&mut b, "PARAMÈTRES");
        cx.paragraph(&mut b, "Les réglages de l'application (apparence, clavier) sont dans le panneau des paramètres, ouvert par la pastille aux curseurs de la barre du bas.");
        cx.body_end(b, &mut c.scrolls);
        "Aide".to_string()
    }
}
