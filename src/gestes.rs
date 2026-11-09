//! Les gestes élaborés, à la manière de tnt.js (github.com/s1pierro/tnt.js) : une
//! machine à états par zone à gestes, qui transforme les doigts en événements.
//!
//! ```text
//! Repos ─(1 doigt)─► Tap ─(durée)─► Appui ─(durée)─► AppuiLong
//!                     │                 └──(dépl. ≥ dist)──► Annule, Repos
//!                     ├─(dépl. ≥ dist)─► Saisie ─(lever)─► Repos
//!                     └─(2e doigt)────► Pincement ─(lever)─► Repos
//! 5 doigts : tout geste est annulé.
//! ```
//!
//! - **Tap, Appui, AppuiLong** : au lever, exactement un des trois, selon la durée,
//!   avec son intensité (où il en était entre ses deux seuils) et sa précision (le plus
//!   grand écart du doigt à son point de départ).
//! - **Saisie** : quand le doigt s'éloigne de `dist` de son départ, un curseur déporté
//!   apparaît au point de départ ; il suit le doigt au bout d'une barre rigide de
//!   longueur `dist`. Le doigt ne cache plus le point de travail.
//! - **Pincement** : deux doigts ; échelle (rapport des écarts), centre et son
//!   déplacement depuis l'événement précédent.
//!
//! Les coordonnées sont celles de l'écran, en pixels. `Ui` tient un `Moteur` par zone
//! (`Ui::gestes`) ; un contexte lit les événements de l'image, et Flow dessine le
//! retour visuel (`dessiner_retour`).

use crate::draw::Rect;
use crate::widgets::Cx;

/// Les réglages des gestes, communs à toute l'application.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReglagesGestes {
    /// La distance qui engage la saisie, et la longueur de la barre, en dp. À 0, pas de
    /// curseur déporté : la saisie part après `SEUIL` dp et le curseur reste sous le doigt.
    pub dist: f32,
    /// La durée qui sépare le tap de l'appui, puis l'appui de l'appui long, en s.
    pub appui: f32,
    pub appui_long: f32,
    /// Montrer le retour visuel (contact, durée, barre et curseur).
    pub retour: bool,
}

impl Default for ReglagesGestes {
    fn default() -> ReglagesGestes {
        ReglagesGestes { dist: 60.0, appui: 0.5, appui_long: 1.5, retour: true }
    }
}

/// Sans curseur déporté, le glissement qui engage la saisie (ou annule un appui), en dp.
pub const SEUIL: f32 = 10.0;

impl ReglagesGestes {
    /// Ramène les réglages dans leurs bornes (l'appui long après l'appui). Une distance
    /// sous 12 dp désactive le curseur déporté.
    pub fn borner(mut self) -> ReglagesGestes {
        self.dist = if self.dist < 12.0 { 0.0 } else { self.dist.clamp(24.0, 160.0) };
        self.appui = self.appui.clamp(0.15, 1.5);
        self.appui_long = self.appui_long.clamp(self.appui + 0.2, 4.0);
        self
    }
}

/// L'état d'une zone à gestes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Etat {
    Repos,
    Tap,
    Appui,
    AppuiLong,
    Saisie,
    Pincement,
}

impl Etat {
    pub fn nom(self) -> &'static str {
        match self {
            Etat::Repos => "repos",
            Etat::Tap => "tap",
            Etat::Appui => "appui",
            Etat::AppuiLong => "appui long",
            Etat::Saisie => "saisie",
            Etat::Pincement => "pincement",
        }
    }
}

type P = (f32, f32);

/// Un événement de geste.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Geste {
    /// La zone change d'état.
    Etat(Etat),
    /// Contact bref. `intensite` : 0 (vif) à 1 (presque un appui) ; `precision` : le
    /// plus grand écart du doigt à son départ, en pixels.
    Tap { x: f32, y: f32, intensite: f32, precision: f32 },
    /// Contact entre les deux seuils ; `intensite` de 0 à 1 entre eux.
    Appui { x: f32, y: f32, intensite: f32, precision: f32 },
    /// Contact au-delà du second seuil, de `au_dela` secondes.
    AppuiLong { x: f32, y: f32, au_dela: f32, precision: f32 },
    /// Un appui annulé : le doigt a bougé de plus de `dist`.
    Annule { x: f32, y: f32 },
    /// La saisie commence : le curseur (x, y) apparaît au départ du doigt.
    SaisieDebut { x: f32, y: f32, doigt: P },
    SaisieDeplace { x: f32, y: f32, doigt: P },
    /// La saisie finit : le curseur, où il est apparu, et le chemin parcouru.
    SaisieFin { x: f32, y: f32, depart: P, vecteur: P },
    /// Une saisie annulée (cinq doigts).
    SaisieAnnulee { x: f32, y: f32 },
    PincementDebut { centre: P },
    /// `echelle` : écart des doigts rapporté à l'écart initial ; `deplacement` : du
    /// centre, depuis l'événement précédent.
    Pincement { echelle: f32, centre: P, deplacement: P },
    PincementFin { echelle: f32, duree: f32 },
}

/// Ce qu'il faut pour dessiner le retour visuel d'une zone.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Apercu {
    pub etat: Etat,
    pub doigt: P,
    pub second: Option<P>,
    pub curseur: Option<P>,
    /// Le temps écoulé depuis le contact, en s.
    pub duree: f32,
}

/// La machine à états d'une zone.
#[derive(Clone, Debug)]
pub struct Moteur {
    pub etat: Etat,
    /// Les doigts suivis par le geste en cours : le premier, puis le second.
    doigts: Vec<(i32, P)>,
    /// Tous les doigts posés sur la zone, suivis ou non (pour les cinq doigts).
    poses: usize,
    debut: f64,
    maintenant: f64,
    depart: P,
    ecart: f32,
    curseur: P,
    saisie_depart: P,
    pincement: (f32, P, f32),
    sortie: Vec<Geste>,
}

fn dist(a: P, b: P) -> f32 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

fn milieu(a: P, b: P) -> P {
    ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0)
}

/// Le point à `d` de `p`, dans la direction de `vers`.
fn au_bout(p: P, vers: P, d: f32) -> P {
    let n = dist(p, vers).max(1e-4);
    (p.0 + (vers.0 - p.0) / n * d, p.1 + (vers.1 - p.1) / n * d)
}

impl Default for Moteur {
    fn default() -> Moteur {
        Moteur { etat: Etat::Repos, doigts: Vec::new(), poses: 0, debut: 0.0, maintenant: 0.0, depart: (0.0, 0.0), ecart: 0.0, curseur: (0.0, 0.0), saisie_depart: (0.0, 0.0), pincement: (1.0, (0.0, 0.0), 1.0), sortie: Vec::new() }
    }
}

impl Moteur {
    fn passer(&mut self, e: Etat) {
        if self.etat != e {
            self.etat = e;
            self.sortie.push(Geste::Etat(e));
        }
        if e == Etat::Repos {
            // Les doigts encore posés ne comptent plus pour ce geste
            self.doigts.clear();
        }
    }

    /// Les événements depuis le dernier appel.
    pub fn prendre(&mut self) -> Vec<Geste> {
        std::mem::take(&mut self.sortie)
    }

    pub fn apercu(&self) -> Option<Apercu> {
        if self.etat == Etat::Repos {
            return None;
        }
        let doigt = self.doigts.first()?.1;
        Some(Apercu {
            etat: self.etat,
            doigt,
            second: self.doigts.get(1).map(|d| d.1),
            curseur: (self.etat == Etat::Saisie).then_some(self.curseur),
            duree: (self.maintenant - self.debut) as f32,
        })
    }

    /// Vrai tant qu'un changement d'état dépend du temps qui passe.
    pub fn en_attente(&self) -> bool {
        matches!(self.etat, Etat::Tap | Etat::Appui)
    }

    pub fn poser(&mut self, id: i32, x: f32, y: f32, t: f64, r: &ReglagesGestes) {
        self.poses += 1;
        self.temps(t, r);
        if self.poses >= 5 {
            self.cinq_doigts();
            return;
        }
        match self.etat {
            Etat::Repos => {
                self.doigts = vec![(id, (x, y))];
                self.debut = t;
                self.depart = (x, y);
                self.ecart = 0.0;
                self.passer(Etat::Tap);
            }
            Etat::Tap => {
                // Un second doigt : pincement
                self.doigts.push((id, (x, y)));
                let (a, b) = (self.doigts[0].1, (x, y));
                self.pincement = (dist(a, b).max(1.0), milieu(a, b), 1.0);
                self.debut = t;
                self.passer(Etat::Pincement);
                self.sortie.push(Geste::PincementDebut { centre: milieu(a, b) });
            }
            _ => {}
        }
    }

    /// `barre` : la longueur de la barre du curseur déporté (0 : pas de curseur
    /// déporté) ; `seuil` : le glissement qui engage la saisie ; en pixels.
    pub fn bouger(&mut self, id: i32, x: f32, y: f32, t: f64, r: &ReglagesGestes, barre: f32, seuil: f32) {
        self.temps(t, r);
        let Some(i) = self.doigts.iter().position(|f| f.0 == id) else { return };
        self.doigts[i].1 = (x, y);
        let p = (x, y);
        match self.etat {
            Etat::Tap | Etat::Appui | Etat::AppuiLong => {
                self.ecart = self.ecart.max(dist(p, self.depart));
                if self.ecart < seuil {
                    return;
                }
                if self.etat == Etat::Tap {
                    // Le curseur apparaît au départ du doigt, au bout de la barre
                    self.curseur = au_bout(p, self.depart, barre);
                    self.saisie_depart = self.curseur;
                    self.passer(Etat::Saisie);
                    self.sortie.push(Geste::SaisieDebut { x: self.curseur.0, y: self.curseur.1, doigt: p });
                } else {
                    self.sortie.push(Geste::Annule { x, y });
                    self.passer(Etat::Repos);
                }
            }
            Etat::Saisie if i == 0 => {
                // La barre rigide : le curseur reste à `d` du doigt, dans sa direction
                self.curseur = au_bout(p, self.curseur, barre);
                self.sortie.push(Geste::SaisieDeplace { x: self.curseur.0, y: self.curseur.1, doigt: p });
            }
            Etat::Pincement if self.doigts.len() == 2 => {
                let (a, b) = (self.doigts[0].1, self.doigts[1].1);
                let (init, avant, _) = self.pincement;
                let centre = milieu(a, b);
                let echelle = dist(a, b) / init;
                self.pincement = (init, centre, echelle);
                self.sortie.push(Geste::Pincement { echelle, centre, deplacement: (centre.0 - avant.0, centre.1 - avant.1) });
            }
            _ => {}
        }
    }

    pub fn lever(&mut self, id: i32, t: f64, r: &ReglagesGestes) {
        self.poses = self.poses.saturating_sub(1);
        self.temps(t, r);
        let Some(i) = self.doigts.iter().position(|f| f.0 == id) else { return };
        let p = self.doigts[i].1;
        let dt = (t - self.debut) as f32;
        let precision = self.ecart;
        match self.etat {
            Etat::Tap | Etat::Appui | Etat::AppuiLong => {
                let (x, y) = p;
                // Classé par la durée : aucun relâché ne se perd
                self.sortie.push(if dt < r.appui {
                    Geste::Tap { x, y, intensite: (dt / r.appui).clamp(0.0, 1.0), precision }
                } else if dt < r.appui_long {
                    Geste::Appui { x, y, intensite: ((dt - r.appui) / (r.appui_long - r.appui)).clamp(0.0, 1.0), precision }
                } else {
                    Geste::AppuiLong { x, y, au_dela: dt - r.appui_long, precision }
                });
            }
            Etat::Saisie if i == 0 => {
                let (c, s) = (self.curseur, self.saisie_depart);
                self.sortie.push(Geste::SaisieFin { x: c.0, y: c.1, depart: s, vecteur: (c.0 - s.0, c.1 - s.1) });
            }
            Etat::Pincement => self.sortie.push(Geste::PincementFin { echelle: self.pincement.2, duree: dt }),
            _ => return,
        }
        self.passer(Etat::Repos);
    }

    /// Le temps passe : tap, puis appui, puis appui long.
    pub fn temps(&mut self, t: f64, r: &ReglagesGestes) {
        self.maintenant = t;
        let dt = (t - self.debut) as f32;
        if self.etat == Etat::Tap && dt >= r.appui && self.doigts.len() == 1 {
            self.passer(Etat::Appui);
        }
        if self.etat == Etat::Appui && dt >= r.appui_long {
            self.passer(Etat::AppuiLong);
        }
    }

    fn cinq_doigts(&mut self) {
        if self.etat == Etat::Saisie {
            self.sortie.push(Geste::SaisieAnnulee { x: self.curseur.0, y: self.curseur.1 });
        }
        self.passer(Etat::Repos);
    }

    /// Le système a repris les doigts : tout s'arrête.
    pub fn annuler(&mut self) {
        self.poses = 0;
        self.cinq_doigts();
    }
}

/// Le retour visuel des gestes en cours, par-dessus tout : sous le doigt, un anneau
/// qui se remplit du tap à l'appui long ; pendant une saisie, la barre et le curseur
/// déporté ; pendant un pincement, le trait entre les doigts.
pub fn dessiner_retour(cx: &mut Cx, r: &ReglagesGestes, apercus: &[Apercu]) {
    if !r.retour {
        return;
    }
    let (d, col) = (cx.dp, cx.couleurs());
    let q = std::f32::consts::FRAC_PI_2;
    let tau = std::f32::consts::TAU;
    let voile = |c: [u8; 4], a: u8| [c[0], c[1], c[2], a];
    for a in apercus {
        let (x, y) = a.doigt;
        let rayon = 38.0 * d;
        match a.etat {
            Etat::Tap | Etat::Appui | Etat::AppuiLong => {
                // L'anneau : le premier tour jusqu'à l'appui, le second jusqu'à l'appui long
                cx.dl.secteur([x, y], rayon - 3.0 * d, rayon, -q, tau - q, 0.0, voile(col.texte, 50));
                let (k, teinte) = match a.etat {
                    Etat::Tap => (a.duree / r.appui, voile(col.texte, 160)),
                    Etat::Appui => ((a.duree - r.appui) / (r.appui_long - r.appui), col.accent),
                    _ => (1.0, [0xE0, 0x3A, 0x3A, 255]),
                };
                let k = k.clamp(0.0, 1.0);
                if k > 0.0 {
                    cx.dl.secteur([x, y], rayon - 3.0 * d, rayon, -q, tau * k - q, 0.0, teinte);
                }
                cx.dl.rrect(Rect::new(x - 5.0 * d, y - 5.0 * d, 10.0 * d, 10.0 * d), 5.0 * d, voile(teinte, 200));
            }
            Etat::Saisie if r.dist <= 0.0 => {
                cx.dl.rrect(Rect::new(x - 12.0 * d, y - 12.0 * d, 24.0 * d, 24.0 * d), 12.0 * d, voile(col.accent, 90));
            }
            Etat::Saisie => {
                let c = a.curseur.unwrap_or(a.doigt);
                cx.dl.rrect(Rect::new(x - 12.0 * d, y - 12.0 * d, 24.0 * d, 24.0 * d), 12.0 * d, voile(col.accent, 60));
                cx.dl.line([x, y], [c.0, c.1], 2.0 * d, voile(col.accent, 180));
                cx.dl.rrect(Rect::new(c.0 - 8.0 * d, c.1 - 8.0 * d, 16.0 * d, 16.0 * d), 8.0 * d, col.surface);
                cx.dl.secteur([c.0, c.1], 5.0 * d, 8.0 * d, -q, tau - q, 0.0, col.accent);
                cx.dl.rrect(Rect::new(c.0 - 1.5 * d, c.1 - 1.5 * d, 3.0 * d, 3.0 * d), 1.5 * d, col.accent);
            }
            Etat::Pincement => {
                if let Some(b) = a.second {
                    cx.dl.line([x, y], [b.0, b.1], 2.0 * d, voile(col.accent, 140));
                    for p in [a.doigt, b] {
                        cx.dl.rrect(Rect::new(p.0 - 12.0 * d, p.1 - 12.0 * d, 24.0 * d, 24.0 * d), 12.0 * d, voile(col.accent, 70));
                    }
                    let m = milieu(a.doigt, b);
                    cx.dl.rrect(Rect::new(m.0 - 4.0 * d, m.1 - 4.0 * d, 8.0 * d, 8.0 * d), 4.0 * d, col.accent);
                }
            }
            Etat::Repos => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const R: ReglagesGestes = ReglagesGestes { dist: 60.0, appui: 0.5, appui_long: 1.5, retour: true };
    const D: f32 = 60.0;

    fn sans_etats(v: Vec<Geste>) -> Vec<Geste> {
        v.into_iter().filter(|g| !matches!(g, Geste::Etat(_))).collect()
    }

    #[test]
    fn tap_appui_et_appui_long_selon_la_duree() {
        let mut m = Moteur::default();
        m.poser(1, 10.0, 10.0, 0.0, &R);
        m.bouger(1, 13.0, 14.0, 0.1, &R, D, D);
        m.lever(1, 0.25, &R);
        assert_eq!(m.prendre(), vec![Geste::Etat(Etat::Tap), Geste::Tap { x: 13.0, y: 14.0, intensite: 0.5, precision: 5.0 }, Geste::Etat(Etat::Repos)]);

        m.poser(1, 0.0, 0.0, 1.0, &R);
        m.temps(1.6, &R);
        assert_eq!(m.etat, Etat::Appui);
        m.lever(1, 2.0, &R);
        assert_eq!(sans_etats(m.prendre()), vec![Geste::Appui { x: 0.0, y: 0.0, intensite: 0.5, precision: 0.0 }]);

        m.poser(1, 0.0, 0.0, 3.0, &R);
        m.temps(4.6, &R);
        assert_eq!(m.etat, Etat::AppuiLong);
        m.lever(1, 5.0, &R);
        let g = sans_etats(m.prendre());
        assert!(matches!(g[..], [Geste::AppuiLong { au_dela, .. }] if (au_dela - 0.5).abs() < 1e-4), "{g:?}");
    }

    #[test]
    fn la_saisie_deporte_le_curseur_au_bout_de_la_barre() {
        let mut m = Moteur::default();
        m.poser(1, 100.0, 100.0, 0.0, &R);
        m.bouger(1, 130.0, 100.0, 0.05, &R, D, D);
        assert_eq!(m.etat, Etat::Tap, "pas encore à la distance");
        m.bouger(1, 160.0, 100.0, 0.1, &R, D, D);
        assert_eq!(m.etat, Etat::Saisie);
        // Le curseur est au départ du doigt
        assert_eq!(sans_etats(m.prendre()), vec![Geste::SaisieDebut { x: 100.0, y: 100.0, doigt: (160.0, 100.0) }]);
        // Le doigt monte : le curseur suit, toujours à 60 px
        m.bouger(1, 160.0, 40.0, 0.2, &R, D, D);
        let a = m.apercu().unwrap();
        let c = a.curseur.unwrap();
        assert!((dist(c, (160.0, 40.0)) - 60.0).abs() < 1e-3);
        m.lever(1, 0.3, &R);
        let g = sans_etats(m.prendre());
        assert!(matches!(g.last(), Some(Geste::SaisieFin { depart: (100.0, 100.0), .. })), "{g:?}");
        assert_eq!(m.etat, Etat::Repos);
    }

    #[test]
    fn sans_barre_le_curseur_reste_sous_le_doigt() {
        let mut m = Moteur::default();
        m.poser(1, 100.0, 100.0, 0.0, &R);
        m.bouger(1, 112.0, 100.0, 0.05, &R, 0.0, 10.0);
        assert_eq!(sans_etats(m.prendre()), vec![Geste::SaisieDebut { x: 112.0, y: 100.0, doigt: (112.0, 100.0) }]);
        m.bouger(1, 200.0, 150.0, 0.1, &R, 0.0, 10.0);
        assert_eq!(m.apercu().unwrap().curseur, Some((200.0, 150.0)));
    }

    #[test]
    fn un_appui_qui_bouge_est_annule() {
        let mut m = Moteur::default();
        m.poser(1, 0.0, 0.0, 0.0, &R);
        m.temps(0.7, &R);
        m.bouger(1, 80.0, 0.0, 0.8, &R, D, D);
        assert_eq!(sans_etats(m.prendre()), vec![Geste::Annule { x: 80.0, y: 0.0 }]);
        // Le doigt resté posé ne fait plus rien
        m.lever(1, 1.0, &R);
        assert!(m.prendre().is_empty());
    }

    #[test]
    fn deux_doigts_pincent() {
        let mut m = Moteur::default();
        m.poser(1, 100.0, 100.0, 0.0, &R);
        m.poser(2, 200.0, 100.0, 0.05, &R);
        assert_eq!(m.etat, Etat::Pincement);
        m.bouger(2, 300.0, 100.0, 0.1, &R, D, D);
        let g = sans_etats(m.prendre());
        assert_eq!(g[0], Geste::PincementDebut { centre: (150.0, 100.0) });
        assert_eq!(g[1], Geste::Pincement { echelle: 2.0, centre: (200.0, 100.0), deplacement: (50.0, 0.0) });
        m.lever(1, 0.5, &R);
        assert!(matches!(sans_etats(m.prendre())[..], [Geste::PincementFin { echelle, .. }] if echelle == 2.0));
        // Le second doigt, encore posé, n'entame pas de geste
        m.bouger(2, 0.0, 0.0, 0.6, &R, D, D);
        m.lever(2, 0.7, &R);
        assert!(m.prendre().is_empty());
    }

    #[test]
    fn cinq_doigts_annulent() {
        let mut m = Moteur::default();
        m.poser(1, 0.0, 0.0, 0.0, &R);
        m.bouger(1, 100.0, 0.0, 0.1, &R, D, D);
        m.prendre();
        for i in 2..=5 {
            m.poser(i, 0.0, 0.0, 0.2, &R);
        }
        assert_eq!(m.prendre(), vec![Geste::SaisieAnnulee { x: 40.0, y: 0.0 }, Geste::Etat(Etat::Repos)]);
    }

    #[test]
    fn les_reglages_restent_dans_leurs_bornes() {
        let r = ReglagesGestes { dist: 1.0, appui: 2.0, appui_long: 0.1, retour: false }.borner();
        assert_eq!((r.dist, r.appui), (0.0, 1.5));
        assert_eq!(ReglagesGestes { dist: 15.0, ..R }.borner().dist, 24.0);
        assert!(r.appui_long > r.appui);
    }
}
