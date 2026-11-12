//! Les applets de la barre de l'application, à la façon du tableau de bord de GNOME 2 :
//! de petits éléments de contrôle ou de suivi, posés dans la barre du bas, qui
//! pilotent ce qui tourne en arrière-plan (la radio, le mixeur, une copie de
//! fichiers…) sans ouvrir de fenêtre.
//!
//! - Un contexte déclare (ou non) son applet dans son `Genre` (`applet`). Il y en a au
//!   plus un par contexte, partagé par toute l'application, qu'une fenêtre de ce
//!   contexte soit ouverte ou non : un applet suit l'état commun (`Commun`, le
//!   lecteur de la radio, le mixeur…), pas une instance.
//! - Dans la barre, l'applet a la largeur qu'il demande (`largeur`) : une icône, un
//!   texte court, un vumètre, ses propres commandes… À 0, il se retire (rien à
//!   montrer). Les applets se rangent à droite, avant l'heure, dans l'ordre des
//!   contextes. S'ils ne tiennent pas tous, ils passent en version compacte, et la
//!   priorité va aux plus à droite (l'Énergie, près de l'heure) : ceux qui ne tiennent
//!   toujours pas sont laissés de côté.
//! - Toucher l'applet (hors de ses propres commandes) ouvre son volet, au-dessus de
//!   la barre : des réglages et un suivi plus complets. Toucher ailleurs le referme.
//! - Le choix des applets montrés se fait dans les paramètres (onglet Barre) et
//!   s'enregistre dans `interface.txt` (`applets = radio mixeur fichiers`).

use crate::contexte::{Commun, Genre};
use crate::contextes::GENRES;
use crate::draw::Rect;
use crate::ui::{id, sub_id, Id, Sense};
use crate::widgets::Cx;

const APPLETS: Id = id("applets");
const DEHORS: Id = id("applets-dehors");
const VOLET: Id = id("applets-volet");

/// Un applet : sa part de la barre, et son volet.
pub trait Applet {
    /// Le nom montré dans les paramètres, s'il diffère de celui du contexte.
    fn nom(&self) -> Option<&'static str> {
        None
    }

    /// La largeur voulue dans la barre de hauteur `h`, en pixels ; 0 : rien à montrer
    /// pour l'instant. `compact` : la barre manque de place, l'essentiel seulement.
    fn largeur(&mut self, cx: &mut Cx, c: &Commun, h: f32, compact: bool) -> f32;

    /// Dessine l'applet dans la barre, dans `r` (la largeur demandée, compacte ou non).
    /// Ses propres commandes (un bouton de lecture…) prennent des identifiants dérivés
    /// de `id` (`sub_id`) : elles passent devant le toucher qui ouvre le volet.
    fn barre(&mut self, cx: &mut Cx, c: &mut Commun, r: Rect, id: Id, compact: bool);

    /// La hauteur voulue pour le volet, de largeur `w` ; 0 : pas de volet. Le volet
    /// défile si elle ne tient pas.
    fn hauteur_volet(&mut self, _cx: &mut Cx, _c: &Commun, _w: f32) -> f32 {
        0.0
    }

    /// Dessine le volet dans `r`. Renvoie vrai pour le refermer (une commande qui ouvre
    /// le contexte, par exemple).
    fn volet(&mut self, _cx: &mut Cx, _c: &mut Commun, _r: Rect, _id: Id) -> bool {
        false
    }

    /// Vrai s'il faut redessiner sans attendre d'événement (un vumètre, un avancement).
    fn anime(&self, _c: &Commun) -> bool {
        false
    }
}

/// Les contextes qui ont un applet, dans l'ordre de `GENRES`.
pub fn disponibles() -> impl Iterator<Item = &'static Genre> {
    GENRES.iter().filter(|g| g.applet.is_some())
}

/// Le nom d'un applet, pour les paramètres : le sien, ou celui de son contexte.
pub fn nom(g: &Genre) -> &'static str {
    g.applet.and_then(|f| f().nom()).unwrap_or(g.nom)
}

/// Les applets montrés par défaut : tous.
pub fn par_defaut() -> Vec<String> {
    disponibles().map(|g| g.cle.to_string()).collect()
}

/// L'applet du contexte `cle` est-il dans la barre ?
pub fn montre(c: &Commun, cle: &str) -> bool {
    c.applets.iter().any(|a| a == cle)
}

/// Montre ou retire l'applet du contexte `cle` ; la liste garde l'ordre des contextes.
pub fn montrer(c: &mut Commun, cle: &str, oui: bool) {
    let avant: Vec<String> = c.applets.clone();
    c.applets = disponibles().map(|g| g.cle).filter(|k| if *k == cle { oui } else { avant.iter().any(|a| a == k) }).map(String::from).collect();
}

/// Les applets de l'application, et celui dont le volet est ouvert.
pub struct Applets {
    liste: Vec<(&'static str, Box<dyn Applet>)>,
    /// Le contexte dont le volet est ouvert.
    pub ouvert: Option<&'static str>,
    /// Où chaque applet a été placé à la dernière image.
    zones: Vec<(&'static str, Rect)>,
}

impl Default for Applets {
    fn default() -> Applets {
        Applets { liste: GENRES.iter().filter_map(|g| g.applet.map(|f| (g.cle, f()))).collect(), ouvert: None, zones: Vec::new() }
    }
}

impl Applets {
    /// Vrai s'il faut redessiner : un applet montré qui s'anime.
    pub fn anime(&self, c: &Commun) -> bool {
        self.zones.iter().any(|(k, _)| self.liste.iter().any(|(cle, a)| cle == k && a.anime(c)))
    }

    /// Les applets placés à la dernière image, et leur zone dans la barre.
    pub fn zones(&self) -> &[(&'static str, Rect)] {
        &self.zones
    }

    /// Place et dessine les applets choisis dans la zone libre `r` de la barre, serrés à
    /// droite. Ceux qui ne tiennent pas sont laissés de côté.
    pub fn barre(&mut self, cx: &mut Cx, c: &mut Commun, r: Rect) {
        self.zones.clear();
        if r.w <= 0.0 || r.h < 1.0 {
            self.ouvert = None;
            return;
        }
        // Tous en entier s'ils tiennent ; sinon en compact, de droite à gauche tant
        // qu'il y a de la place
        let choisis: Vec<usize> = (0..self.liste.len()).filter(|&i| c.applets.iter().any(|k| k == self.liste[i].0)).collect();
        let mut largeurs = |compact: bool| -> Vec<(usize, f32)> { choisis.iter().map(|&i| (i, self.liste[i].1.largeur(cx, c, r.h, compact).round())).filter(|(_, w)| *w > 0.0).collect() };
        let entiers = largeurs(false);
        let compact = entiers.iter().map(|(_, w)| w).sum::<f32>() > r.w;
        let mut places = if compact {
            let mut gardes = Vec::new();
            let mut total = 0.0;
            for (i, w) in largeurs(true).into_iter().rev() {
                if total + w <= r.w {
                    gardes.insert(0, (i, w));
                    total += w;
                }
            }
            gardes
        } else {
            entiers
        };
        places.retain(|(_, w)| *w > 0.0);
        let total: f32 = places.iter().map(|(_, w)| w).sum();
        let col = cx.couleurs();
        let mut x = r.right() - total;
        for (i, w) in places {
            let (cle, a) = &mut self.liste[i];
            let z = Rect::new(x, r.y, w, r.h);
            let id = sub_id(APPLETS, i);
            let touche = cx.ui.click(id, z, None);
            let ouvert = self.ouvert == Some(*cle);
            if ouvert {
                cx.dl.rrect(z, 0.0, col.puce);
                cx.dl.rrect(Rect::new(z.x, z.bottom() - 3.0 * cx.dp, z.w, 3.0 * cx.dp), 0.0, col.accent);
            } else if cx.ui.pressed(id) {
                cx.dl.rrect(z, 0.0, col.puce);
            }
            a.barre(cx, c, z, id, compact);
            if touche {
                self.ouvert = if ouvert { None } else { Some(*cle) };
            }
            self.zones.push((cle, z));
            x += w;
        }
        // Un applet qui s'est retiré referme son volet
        if let Some(k) = self.ouvert {
            if !self.zones.iter().any(|(z, _)| *z == k) {
                self.ouvert = None;
            }
        }
    }

    /// Le volet ouvert, au-dessus de la barre, dans `haut` (l'écran sans la barre).
    /// Toucher en dehors le referme.
    pub fn volet(&mut self, cx: &mut Cx, c: &mut Commun, haut: Rect) {
        let Some(k) = self.ouvert else { return };
        let Some(&(_, z)) = self.zones.iter().find(|(z, _)| *z == k) else { return };
        let Some(i) = self.liste.iter().position(|(cle, _)| *cle == k) else { return };
        let (d, col) = (cx.dp, cx.couleurs());
        let m = (8.0 * d).round();
        let w = (340.0 * d).min(haut.w - 2.0 * m).round();
        let a = &mut self.liste[i].1;
        let h = a.hauteur_volet(cx, c, w).min(haut.h - 2.0 * m).round();
        if h <= 0.0 || w <= 0.0 {
            self.ouvert = None;
            return;
        }
        if cx.ui.click(DEHORS, haut, None) {
            self.ouvert = None;
            return;
        }
        let x = (z.cx() - w / 2.0).clamp(haut.x + m, haut.right() - m - w);
        let r = Rect::new(x, haut.bottom() - h - m, w, h);
        cx.ui.region_add(VOLET, r, Sense::Sink);
        let rayon = cx.theme.rayon(cx.commande()) * 0.5;
        cx.ombre(r, rayon, 12.0);
        cx.dl.rrect(r, rayon, col.ligne);
        cx.dl.rrect(r.inset(d.max(1.0)), rayon, col.surface);
        if a.volet(cx, c, r.inset(d.max(1.0)), sub_id(VOLET, i)) {
            self.ouvert = None;
        }
    }
}
