//! Le contexte Gestes : un terrain d'essai des gestes élaborés (`crate::gestes`).
//!
//! Le terrain porte des pastilles de couleur. Un tap en choisit une, un appui change
//! sa couleur, un appui long l'enlève (ou en crée une sur le vide). Une saisie déplace
//! la pastille sous le curseur déporté, ou trace un trait sur le vide. Deux doigts
//! pincent la pastille choisie. À côté, le journal des événements et l'historique des
//! états montrent ce que reçoit un contexte, pour apprendre et régler les gestes.
//!
//! C'est aussi l'exemple à suivre pour un contexte qui veut des gestes :
//! `cx.ui.gestes(id, zone)` déclare la zone et rend les événements de l'image.

use super::TEINTES;
use crate::contexte::{Commun, Contexte, Etat, Genre};
use crate::draw::{Color, Rect};
use crate::gestes::{self, Geste};
use crate::surface::Placee;
use crate::ui::{id, sub_id, Id};
use crate::widgets::{Cx, Icon};
use std::time::Instant;

pub const CLE: &str = "gestes";
pub const GENRE: Genre = Genre { cle: CLE, nom: "Gestes", icone: Icon::Toucher, ouvrir };

const GESTES: Id = id("gestes-terrain");

/// Une pastille du terrain : sa place (de 0 à 1 dans le terrain), son rayon en dp, sa
/// teinte.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Objet {
    x: f32,
    y: f32,
    r: f32,
    teinte: usize,
}

/// Un repère qui s'efface : ce qui vient d'arriver, et où.
#[derive(Clone)]
struct Marque {
    x: f32,
    y: f32,
    texte: String,
    age: f32,
}

const DUREE_MARQUE: f32 = 1.4;
const JOURNAL: usize = 40;
const HISTORIQUE: usize = 10;

#[derive(Clone)]
pub struct Terrain {
    objets: Vec<Objet>,
    choisi: Option<usize>,
    traces: Vec<Vec<(f32, f32)>>,
    /// La saisie en cours : l'objet saisi et l'écart entre le curseur et son centre
    /// (relatif), ou un trait (`None`) ; la place d'origine de l'objet.
    saisie: Option<(Option<usize>, (f32, f32), (f32, f32))>,
    /// Le pincement en cours : l'objet pincé et son rayon de départ.
    pincement: Option<(usize, f32)>,
    marques: Vec<Marque>,
    journal: Vec<String>,
    historique: Vec<(gestes::Etat, Instant)>,
    /// Le terrain à l'image précédente, pour passer des pixels aux coordonnées relatives.
    zone: Rect,
}

fn depart() -> Vec<Objet> {
    vec![Objet { x: 0.3, y: 0.35, r: 34.0, teinte: 0 }, Objet { x: 0.65, y: 0.3, r: 26.0, teinte: 3 }, Objet { x: 0.5, y: 0.7, r: 42.0, teinte: 2 }]
}

fn ouvrir(e: &Etat) -> Box<dyn Contexte> {
    let n = e.nombres("objets");
    let objets: Vec<Objet> = if e.texte("objets").is_some() {
        n.chunks_exact(4).map(|c| Objet { x: c[0].clamp(0.0, 1.0), y: c[1].clamp(0.0, 1.0), r: c[2].clamp(12.0, 120.0), teinte: c[3] as usize % TEINTES.len() }).collect()
    } else {
        depart()
    };
    Box::new(Terrain::new(objets))
}

impl Terrain {
    fn new(objets: Vec<Objet>) -> Terrain {
        Terrain { objets, choisi: None, traces: Vec::new(), saisie: None, pincement: None, marques: Vec::new(), journal: Vec::new(), historique: Vec::new(), zone: Rect::new(0.0, 0.0, 1.0, 1.0) }
    }

    fn relatif(&self, x: f32, y: f32) -> (f32, f32) {
        let z = self.zone;
        ((x - z.x) / z.w.max(1.0), (y - z.y) / z.h.max(1.0))
    }

    /// L'objet sous le point (en pixels), le plus haut d'abord.
    fn sous(&self, x: f32, y: f32, dp: f32) -> Option<usize> {
        let z = self.zone;
        self.objets.iter().rposition(|o| (z.x + o.x * z.w - x).hypot(z.y + o.y * z.h - y) <= o.r * dp + 6.0 * dp)
    }

    fn marquer(&mut self, x: f32, y: f32, texte: impl Into<String>) {
        let (x, y) = self.relatif(x, y);
        self.marques.push(Marque { x, y, texte: texte.into(), age: 0.0 });
    }

    fn noter(&mut self, ligne: String) {
        self.journal.insert(0, ligne);
        self.journal.truncate(JOURNAL);
    }

    /// Un événement : ce qu'il fait sur le terrain, et sa ligne au journal.
    fn recevoir(&mut self, g: Geste, dp: f32) {
        let z = self.zone;
        // Les positions du journal : en dp, depuis le coin du terrain
        let pos = |x: f32, y: f32| format!("{:>4.0} {:>4.0}", (x - z.x) / dp, (y - z.y) / dp);
        let pc = |k: f32| format!("{:>3.0} %", k * 100.0);
        match g {
            Geste::Etat(e) => {
                self.historique.insert(0, (e, Instant::now()));
                self.historique.truncate(HISTORIQUE);
            }
            Geste::Tap { x, y, intensite, precision } => {
                self.noter(format!("tap        {}  {}  ±{:.0}", pos(x, y), pc(intensite), precision / dp));
                self.choisi = self.sous(x, y, dp);
                self.marquer(x, y, "tap");
            }
            Geste::Appui { x, y, intensite, precision } => {
                self.noter(format!("appui      {}  {}  ±{:.0}", pos(x, y), pc(intensite), precision / dp));
                if let Some(i) = self.sous(x, y, dp) {
                    self.objets[i].teinte = (self.objets[i].teinte + 1) % TEINTES.len();
                    self.choisi = Some(i);
                    self.marquer(x, y, "appui : couleur");
                } else {
                    self.marquer(x, y, "appui");
                }
            }
            Geste::AppuiLong { x, y, au_dela, precision } => {
                self.noter(format!("appui long {}  +{:.1} s ±{:.0}", pos(x, y), au_dela, precision / dp));
                if let Some(i) = self.sous(x, y, dp) {
                    self.objets.remove(i);
                    self.choisi = None;
                    self.marquer(x, y, "appui long : enlevée");
                } else {
                    let (rx, ry) = self.relatif(x, y);
                    let teinte = self.objets.len() % TEINTES.len();
                    self.objets.push(Objet { x: rx, y: ry, r: 30.0, teinte });
                    self.choisi = Some(self.objets.len() - 1);
                    self.marquer(x, y, "appui long : créée");
                }
            }
            Geste::Annule { x, y } => {
                self.noter(format!("annulé     {}", pos(x, y)));
                self.marquer(x, y, "annulé");
            }
            Geste::SaisieDebut { x, y, .. } => {
                self.noter(format!("saisie     {}", pos(x, y)));
                let (rx, ry) = self.relatif(x, y);
                self.saisie = Some(match self.sous(x, y, dp) {
                    Some(i) => {
                        let o = self.objets[i];
                        self.choisi = Some(i);
                        (Some(i), (o.x - rx, o.y - ry), (o.x, o.y))
                    }
                    None => {
                        self.traces.push(vec![(rx, ry)]);
                        if self.traces.len() > 12 {
                            self.traces.remove(0);
                        }
                        (None, (0.0, 0.0), (0.0, 0.0))
                    }
                });
            }
            Geste::SaisieDeplace { x, y, .. } => {
                let (rx, ry) = self.relatif(x, y);
                match self.saisie {
                    Some((Some(i), (ex, ey), _)) => {
                        if let Some(o) = self.objets.get_mut(i) {
                            o.x = (rx + ex).clamp(0.0, 1.0);
                            o.y = (ry + ey).clamp(0.0, 1.0);
                        }
                    }
                    Some((None, ..)) => {
                        if let Some(t) = self.traces.last_mut() {
                            t.push((rx, ry));
                        }
                    }
                    None => {}
                }
            }
            Geste::SaisieFin { x, y, vecteur, .. } => {
                self.noter(format!("lâché      {}  ⟶ {:.0} {:.0}", pos(x, y), vecteur.0 / dp, vecteur.1 / dp));
                self.saisie = None;
            }
            Geste::SaisieAnnulee { x, y } => {
                self.noter(format!("saisie annulée {}", pos(x, y)));
                // La pastille revient à sa place ; le trait s'efface
                match self.saisie.take() {
                    Some((Some(i), _, (ox, oy))) => {
                        if let Some(o) = self.objets.get_mut(i) {
                            (o.x, o.y) = (ox, oy);
                        }
                    }
                    Some((None, ..)) => {
                        self.traces.pop();
                    }
                    None => {}
                }
                self.marquer(x, y, "annulé");
            }
            Geste::PincementDebut { centre } => {
                self.noter(format!("pincement  {}", pos(centre.0, centre.1)));
                let i = self.sous(centre.0, centre.1, dp).or(self.choisi);
                self.pincement = i.and_then(|i| self.objets.get(i).map(|o| (i, o.r)));
                if let Some((i, _)) = self.pincement {
                    self.choisi = Some(i);
                }
            }
            Geste::Pincement { echelle, deplacement, .. } => {
                if let Some((i, r0)) = self.pincement {
                    let (w, h) = (z.w.max(1.0), z.h.max(1.0));
                    if let Some(o) = self.objets.get_mut(i) {
                        o.r = (r0 * echelle).clamp(12.0, 120.0);
                        o.x = (o.x + deplacement.0 / w).clamp(0.0, 1.0);
                        o.y = (o.y + deplacement.1 / h).clamp(0.0, 1.0);
                    }
                }
            }
            Geste::PincementFin { echelle, duree } => {
                self.noter(format!("fin pinc.  × {echelle:.2}  {duree:.1} s"));
                self.pincement = None;
            }
        }
    }

    fn terrain(&mut self, cx: &mut Cx, z: Rect, base: Id) {
        let (d, col) = (cx.dp, cx.couleurs());
        cx.dl.rrect(z, 0.0, col.fond);
        cx.clip(Some(z));
        // Une grille légère, tous les 40 dp
        let pas = 40.0 * d;
        let grille = [col.ligne[0], col.ligne[1], col.ligne[2], 90];
        let mut x = z.x + pas;
        while x < z.right() {
            cx.dl.rrect(Rect::new(x, z.y, 1.0, z.h), 0.0, grille);
            x += pas;
        }
        let mut y = z.y + pas;
        while y < z.bottom() {
            cx.dl.rrect(Rect::new(z.x, y, z.w, 1.0), 0.0, grille);
            y += pas;
        }
        let point = |p: (f32, f32)| [z.x + p.0 * z.w, z.y + p.1 * z.h];
        for t in &self.traces {
            for w in t.windows(2) {
                cx.dl.line(point(w[0]), point(w[1]), 3.0 * d, [col.accent[0], col.accent[1], col.accent[2], 170]);
            }
        }
        for (i, o) in self.objets.iter().enumerate() {
            let [x, y] = point((o.x, o.y));
            let r = o.r * d;
            let teinte = TEINTES[o.teinte % TEINTES.len()];
            cx.ombre(Rect::new(x - r, y - r, 2.0 * r, 2.0 * r), r, 8.0);
            cx.dl.rrect(Rect::new(x - r, y - r, 2.0 * r, 2.0 * r), r, teinte);
            if self.choisi == Some(i) {
                cx.dl.secteur([x, y], r + 3.0 * d, r + 6.0 * d, 0.0, std::f32::consts::TAU, 0.0, col.texte);
            }
        }
        for m in &self.marques {
            let k = 1.0 - (m.age / DUREE_MARQUE).clamp(0.0, 1.0);
            let a = (k * 255.0) as u8;
            let [x, y] = point((m.x, m.y));
            let r = (14.0 + 26.0 * (1.0 - k)) * d;
            cx.dl.secteur([x, y], r - 2.0 * d, r, 0.0, std::f32::consts::TAU, 0.0, [col.accent[0], col.accent[1], col.accent[2], a]);
            let c: Color = [col.texte[0], col.texte[1], col.texte[2], a];
            cx.text_at(x + r + 4.0 * d, y, &m.texte, 13.0, c);
        }
        cx.clip(None);
        // Les commandes du terrain, déclarées après lui : elles passent devant
        let h = cx.commande();
        let e = cx.coin();
        let w = cx.pill_width("Effacer");
        if cx.pill(sub_id(base, 2), z.right() - w - e, z.bottom() - h - e, "Effacer", false) {
            self.traces.clear();
            self.objets = depart();
            self.choisi = None;
        }
    }

    fn journal(&mut self, cx: &mut Cx, c: &mut Commun, r: Rect, base: Id) {
        let (d, col) = (cx.dp, cx.couleurs());
        cx.dl.rrect(r, 0.0, col.surface);
        let mut b = cx.body(sub_id(base, 3), r, &c.scrolls);
        // L'état courant, en grand
        let courant = self.historique.first().map_or(gestes::Etat::Repos, |h| h.0);
        cx.section(&mut b, "ÉTAT");
        let ligne = 20.0 * d;
        cx.text_at(b.x, b.y + ligne / 2.0, courant.nom(), 20.0, col.accent);
        b.y += ligne + 8.0 * d;
        cx.section(&mut b, "HISTORIQUE DES ÉTATS");
        let lm = 18.0 * d;
        if self.historique.is_empty() {
            cx.paragraph(&mut b, "Touchez le terrain.");
        }
        for (k, (e, t)) in self.historique.iter().enumerate() {
            // La durée d'un état : jusqu'à l'état suivant (plus récent), ou jusqu'à maintenant
            let fin = if k == 0 { Instant::now() } else { self.historique[k - 1].1 };
            let duree = fin.duration_since(*t).as_secs_f32();
            let texte = if k == 0 && *e == gestes::Etat::Repos { format!("{:<11}", e.nom()) } else { format!("{:<11}{:>6.0} ms", e.nom(), duree * 1000.0) };
            cx.mono_at(b.x, b.y + lm / 2.0, &texte, 12.0, if k == 0 { col.texte } else { col.pale });
            b.y += lm;
        }
        cx.section(&mut b, "ÉVÉNEMENTS");
        for (k, l) in self.journal.iter().enumerate() {
            cx.mono_at(b.x, b.y + lm / 2.0, l, 12.0, if k == 0 { col.texte } else { col.pale });
            b.y += lm;
        }
        cx.section(&mut b, "ESSAYER");
        for s in [
            "Tap : choisit une pastille.",
            "Appui (tenir un instant) : change sa couleur. L'anneau sous le doigt montre la durée : il se remplit jusqu'à l'appui, puis jusqu'à l'appui long.",
            "Appui long : enlève la pastille, ou en crée une sur le vide.",
            "Glisser : un curseur apparaît là où le doigt s'est posé, au bout d'une barre. Il déplace la pastille sans que le doigt la cache, ou trace un trait sur le vide.",
            "Deux doigts : pincez pour agrandir la pastille choisie, et la déplacer.",
            "Cinq doigts : annulent le geste en cours.",
            "Le curseur déporté se désactive en mettant sa distance à zéro : la saisie part alors dès que le doigt glisse, et le curseur reste sous lui.",
            "Les durées et la longueur de la barre se règlent dans les paramètres, onglet Gestes.",
        ] {
            cx.paragraph(&mut b, s);
        }
        cx.body_end(b, &mut c.scrolls);
    }
}

impl Contexte for Terrain {
    fn cle(&self) -> &'static str {
        CLE
    }

    fn dupliquer(&self) -> Box<dyn Contexte> {
        Box::new(Terrain::new(self.objets.clone()))
    }

    fn ecrire(&self, e: &mut Etat) {
        let n: Vec<String> = self.objets.iter().map(|o| format!("{:.3} {:.3} {:.0} {}", o.x, o.y, o.r, o.teinte)).collect();
        e.mettre("objets", n.join(" "));
    }

    fn avancer(&mut self, dt: f32, _c: &Commun) {
        for m in &mut self.marques {
            m.age += dt;
        }
        self.marques.retain(|m| m.age < DUREE_MARQUE);
    }

    fn anime(&self, _c: &Commun) -> bool {
        !self.marques.is_empty()
    }

    fn dessiner(&mut self, cx: &mut Cx, c: &mut Commun, p: &Placee) -> String {
        let r = p.contenu;
        let d = cx.dp;
        let base = sub_id(GESTES, p.id as usize);
        // Côte à côte quand la fenêtre est large, l'un sur l'autre sinon
        let (terrain, journal) = if r.w > 1.2 * r.h {
            let w = (r.w * 0.62).round();
            (Rect::new(r.x, r.y, w, r.h), Rect::new(r.x + w, r.y, r.w - w, r.h))
        } else {
            let h = (r.h * 0.6).round();
            (Rect::new(r.x, r.y, r.w, h), Rect::new(r.x, r.y + h, r.w, r.h - h))
        };
        self.zone = terrain;
        for g in cx.ui.gestes(sub_id(base, 1), terrain) {
            self.recevoir(g, d);
        }
        self.terrain(cx, terrain, base);
        cx.dl.rrect(if r.w > 1.2 * r.h { Rect::new(journal.x, r.y, d.max(1.0), r.h) } else { Rect::new(r.x, journal.y, r.w, d.max(1.0)) }, 0.0, cx.couleurs().ligne);
        self.journal(cx, c, journal, base);
        let etat = self.historique.first().map_or(gestes::Etat::Repos, |h| h.0);
        format!("{} pastille{} · {}", self.objets.len(), if self.objets.len() > 1 { "s" } else { "" }, etat.nom())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terrain() -> Terrain {
        let mut t = Terrain::new(depart());
        t.zone = Rect::new(0.0, 0.0, 1000.0, 1000.0);
        t
    }

    #[test]
    fn appui_long_cree_puis_enleve() {
        let mut t = terrain();
        t.recevoir(Geste::AppuiLong { x: 900.0, y: 900.0, au_dela: 0.0, precision: 0.0 }, 1.0);
        assert_eq!(t.objets.len(), 4);
        assert_eq!(t.choisi, Some(3));
        t.recevoir(Geste::AppuiLong { x: 905.0, y: 900.0, au_dela: 0.0, precision: 0.0 }, 1.0);
        assert_eq!(t.objets.len(), 3);
        assert_eq!(t.journal.len(), 2);
    }

    #[test]
    fn la_saisie_deplace_la_pastille_sans_sauter() {
        let mut t = terrain();
        // Le curseur apparaît à 10 px du centre de la première pastille
        t.recevoir(Geste::SaisieDebut { x: 310.0, y: 350.0, doigt: (370.0, 350.0) }, 1.0);
        t.recevoir(Geste::SaisieDeplace { x: 410.0, y: 450.0, doigt: (470.0, 450.0) }, 1.0);
        let o = t.objets[0];
        assert!((o.x - 0.4).abs() < 1e-4 && (o.y - 0.45).abs() < 1e-4, "{o:?}");
        // Cinq doigts : elle revient
        t.recevoir(Geste::SaisieAnnulee { x: 0.0, y: 0.0 }, 1.0);
        assert_eq!((t.objets[0].x, t.objets[0].y), (0.3, 0.35));
    }

    #[test]
    fn l_etat_se_relit() {
        let t = terrain();
        let mut e = Etat::default();
        t.ecrire(&mut e);
        let mut e2 = Etat::default();
        ouvrir(&e).ecrire(&mut e2);
        assert_eq!(e.texte("objets"), e2.texte("objets"));
        // Tout enlevé : le terrain reste vide
        let mut e = Etat::default();
        e.mettre("objets", "");
        let mut e3 = Etat::default();
        ouvrir(&e).ecrire(&mut e3);
        assert_eq!(e3.texte("objets"), Some(""));
    }
}
