//! Surface de travail, à la manière de forg.x : l'espace de travail se partage en
//! fenêtres, chacune avec sa vue.
//!
//! - La surface est un arbre : une fenêtre, ou un partage en deux (côte à côte ou l'une
//!   sur l'autre) selon une proportion. Chaque fenêtre a un identifiant stable, et une
//!   vue que l'application interprète (3D, liste, texte…).
//! - Chaque fenêtre a une seule barre, en bas : l'icône et le titre de sa vue, à la
//!   couleur d'accent, puis la ligne d'état laissée à l'application (en chasse fixe), et
//!   la poignée dans le coin.
//! - **Partager** : le menu de la barre coupe la fenêtre en deux moitiés, côte à côte
//!   ou l'une sur l'autre ; la nouvelle montre la vue d'accueil (`Surface::scinder`).
//! - **Redimensionner** : tirer la poignée du coin. Elle déplace le bord droit et le bord
//!   bas de la fenêtre (les partages les plus proches qui les portent), de 10 à 90 %.
//!   Elle montre toujours un point, et un tiret le long de chaque bord qu'elle peut
//!   déplacer : les deux ensemble forment un chevron.
//! - **Menu** : toucher l'icône ou le titre ouvre le choix de la vue, le partage en
//!   deux moitiés (la nouvelle montre l'Accueil) et la fermeture.
//!   La fenêtre voisine reprend la place d'une fenêtre fermée.
//!
//! **Fenêtres libres** (« free flow », `mettre_libre`) : au lieu de paver la surface,
//! chaque fenêtre flotte à sa place, comme sur un bureau. Tirer sa barre la déplace,
//! tirer la poignée la redimensionne, la toucher la passe devant. La place se retient
//! en part de la surface, et s'écrit `libre(F1:vue x y l h;…)`, de l'arrière vers
//! l'avant. Partager coupe la place de la fenêtre en deux ; une fenêtre insérée
//! (`inserer`) se pose en cascade sur la plus en avant.
//!
//! L'application appelle `frame` (disposition, barres, gestes) avant de dessiner le
//! contenu des fenêtres, puis `dessus` (aperçu du partage, menu), qui passe au-dessus de
//! tout. Les changements (fenêtre partagée, fermée, vue changée) se lisent avec
//! `changements`, et la disposition s'écrit et se relit en texte (`ecrire_avec`,
//! `lire_avec`, qui nomment les vues comme l'application le veut). En fenêtres libres,
//! `habiller` dessine le cadre et la barre de chaque fenêtre juste avant son contenu :
//! une fenêtre de devant couvre ainsi celles de derrière, au dessin comme au toucher.

use crate::draw::Rect;
use crate::ui::{id, sub_id, Id, Sense};
use crate::widgets::{Cx, Icon};

/// L'identifiant stable d'une fenêtre.
pub type FenetreId = u64;

/// Hauteur de la barre et des lignes du menu, en part de la taille des commandes du
/// thème.
const BARRE: f32 = 0.75;
const MENU_LIGNE: f32 = 0.875;
const MENU_LARGE: f32 = 220.0;
/// Bornes d'une proportion.
const MIN: f32 = 0.1;
const MAX: f32 = 0.9;

const TITRE_ID: Id = id("surface/titre");
const POIGNEE_ID: Id = id("surface/poignee");
const FOND_MENU: Id = id("surface/fond-menu");
const LIGNE_MENU: Id = id("surface/ligne-menu");
const FOND_ID: Id = id("surface/fond-libre");
const DEPLACER_ID: Id = id("surface/deplacer");

/// Le sens d'un partage : `Colonnes`, côte à côte (la première à gauche) ; `Lignes`,
/// l'une sur l'autre (la première en haut).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Sens {
    Colonnes,
    Lignes,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Noeud {
    Fenetre { id: FenetreId, vue: u32 },
    Partage { sens: Sens, ratio: f32, a: Box<Noeud>, b: Box<Noeud> },
}

/// Une fenêtre à l'écran : tout son cadre, son contenu (au-dessus de la barre), sa
/// barre, le titre dans la barre et la ligne d'état laissée à l'application, à droite
/// du titre.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placee {
    pub id: FenetreId,
    pub vue: u32,
    pub cadre: Rect,
    pub contenu: Rect,
    pub barre: Rect,
    pub titre: Rect,
    pub etat: Rect,
    /// Où placer la première pastille de l'application : le coin haut gauche du contenu,
    /// à la marge du thème.
    pub suite: (f32, f32),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Changement {
    /// `de` s'est partagée ; `nouvelle` est née, avec la même vue.
    Partagee { de: FenetreId, nouvelle: FenetreId },
    /// Une fenêtre nouvelle, née d'un partage par le menu : elle montre la vue
    /// d'accueil, avec une instance neuve (pas une copie).
    Nouvelle(FenetreId),
    Fermee(FenetreId),
    Vue { id: FenetreId, vue: u32 },
}

/// Une fenêtre libre : sa vue et sa place, en part de la surface (x, y, largeur,
/// hauteur).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Flottante {
    pub id: FenetreId,
    pub vue: u32,
    pub place: [f32; 4],
    pub etendue: Etendue,
}

/// Comment une fenêtre libre se montre : à sa place, ou réduite (cachée, son contexte
/// reste ouvert ; la barre des tâches la réduit et la rend).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum Etendue {
    #[default]
    Normale,
    Reduite,
}

pub struct Surface {
    pub racine: Noeud,
    /// En fenêtres libres : les fenêtres, de l'arrière vers l'avant (`racine` n'est
    /// alors plus lue).
    libres: Option<Vec<Flottante>>,
    /// La barre tirée d'une fenêtre libre : sa fenêtre, et l'écart du doigt à son coin.
    deplace: Option<(FenetreId, f32, f32)>,
    suivant: FenetreId,
    /// La poignée tirée : sa fenêtre, et l'écart du doigt aux bords droit et bas.
    tirage: Option<(FenetreId, f32, f32)>,
    menu: Option<FenetreId>,
    /// La vue des fenêtres nées d'un partage par le menu (l'Accueil).
    pub accueil: u32,
    rect: Rect,
    placees: Vec<Placee>,
    changements: Vec<Changement>,
    /// La disposition a changé depuis la dernière fois qu'on l'a lu : à enregistrer.
    pub modifiee: bool,
    /// Une surface dans une fenêtre (les vues d'un contexte) : ses identifiants sont
    /// dérivés de celui-ci, pour ne pas se confondre avec ceux de l'application.
    pub base: Option<Id>,
}

/// Les deux parts d'un rectangle partagé.
fn couper(r: Rect, sens: Sens, ratio: f32) -> (Rect, Rect) {
    match sens {
        Sens::Colonnes => {
            let w = (r.w * ratio).round();
            (Rect::new(r.x, r.y, w, r.h), Rect::new(r.x + w, r.y, r.w - w, r.h))
        }
        Sens::Lignes => {
            let h = (r.h * ratio).round();
            (Rect::new(r.x, r.y, r.w, h), Rect::new(r.x, r.y + h, r.w, r.h - h))
        }
    }
}

/// Le cadre d'une fenêtre libre dans la surface `r`.
fn cadre_libre(p: [f32; 4], r: Rect) -> Rect {
    Rect::new((r.x + p[0] * r.w).round(), (r.y + p[1] * r.h).round(), (p[2] * r.w).round(), (p[3] * r.h).round())
}

impl Noeud {
    fn disposer(&self, r: Rect, out: &mut Vec<(FenetreId, u32, Rect)>) {
        match self {
            Noeud::Fenetre { id, vue } => out.push((*id, *vue, r)),
            Noeud::Partage { sens, ratio, a, b } => {
                let (ra, rb) = couper(r, *sens, *ratio);
                a.disposer(ra, out);
                b.disposer(rb, out);
            }
        }
    }

    fn max_id(&self) -> FenetreId {
        match self {
            Noeud::Fenetre { id, .. } => *id,
            Noeud::Partage { a, b, .. } => a.max_id().max(b.max_id()),
        }
    }

    fn compte(&self) -> usize {
        match self {
            Noeud::Fenetre { .. } => 1,
            Noeud::Partage { a, b, .. } => a.compte() + b.compte(),
        }
    }

    /// Déplace les bords droit et bas de la fenêtre `f` vers (x, y) : pour chaque axe,
    /// le partage le plus profond où `f` est dans la première part. `fait` : les axes
    /// déjà servis (colonnes, lignes). Renvoie vrai si `f` est dans ce nœud.
    fn tirer(&mut self, r: Rect, f: FenetreId, x: f32, y: f32, mini: f32, fait: &mut (bool, bool)) -> bool {
        let Noeud::Partage { sens, ratio, a, b } = self else {
            return matches!(self, Noeud::Fenetre { id, .. } if *id == f);
        };
        let (ra, rb) = couper(r, *sens, *ratio);
        if a.tirer(ra, f, x, y, mini, fait) {
            let (pos, debut, long, deja) = match sens {
                Sens::Colonnes => (x, r.x, r.w, &mut fait.0),
                Sens::Lignes => (y, r.y, r.h, &mut fait.1),
            };
            if !*deja && long > 0.0 {
                *deja = true;
                // Chaque part garde au moins `mini` pixels, si la place le permet
                let m = (mini / long).min(0.5);
                *ratio = ((pos - debut) / long).clamp(MIN.max(m), MAX.min(1.0 - m));
            }
            true
        } else {
            b.tirer(rb, f, x, y, mini, fait)
        }
    }

    /// Les axes sur lesquels la poignée de `f` a prise : (colonnes, lignes).
    fn prises(&self, f: FenetreId) -> Option<(bool, bool)> {
        match self {
            Noeud::Fenetre { id, .. } => (*id == f).then_some((false, false)),
            Noeud::Partage { sens, a, b, .. } => {
                if let Some((c, l)) = a.prises(f) {
                    Some(match sens {
                        Sens::Colonnes => (true, l),
                        Sens::Lignes => (c, true),
                    })
                } else {
                    b.prises(f)
                }
            }
        }
    }

    fn ecrire(&self, s: &mut String, nom: &dyn Fn(u32) -> String) {
        match self {
            Noeud::Fenetre { id, vue } => s.push_str(&format!("F{id}:{}", nom(*vue))),
            Noeud::Partage { sens, ratio, a, b } => {
                s.push(if *sens == Sens::Colonnes { 'C' } else { 'L' });
                s.push_str(&format!("{ratio:.4}("));
                a.ecrire(s, nom);
                s.push(',');
                b.ecrire(s, nom);
                s.push(')');
            }
        }
    }

    fn lire(s: &mut &str, vue_de: &dyn Fn(&str) -> Option<u32>) -> Option<Noeud> {
        let c = s.chars().next()?;
        *s = &s[c.len_utf8()..];
        let nombre = |s: &mut &str| -> Option<f64> {
            let n = s.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-')).unwrap_or(s.len());
            let v = s[..n].parse().ok()?;
            *s = &s[n..];
            Some(v)
        };
        match c {
            'F' => {
                let id = nombre(s)? as FenetreId;
                *s = s.strip_prefix(':')?;
                // La vue : un numéro ou une clé (lettres, chiffres, - et _)
                let n = s.find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_')).unwrap_or(s.len());
                let vue = vue_de(&s[..n])?;
                *s = &s[n..];
                Some(Noeud::Fenetre { id, vue })
            }
            'C' | 'L' => {
                let ratio = (nombre(s)? as f32).clamp(MIN, MAX);
                *s = s.strip_prefix('(')?;
                let a = Noeud::lire(s, vue_de)?;
                *s = s.strip_prefix(',')?;
                let b = Noeud::lire(s, vue_de)?;
                *s = s.strip_prefix(')')?;
                Some(Noeud::Partage { sens: if c == 'C' { Sens::Colonnes } else { Sens::Lignes }, ratio, a: Box::new(a), b: Box::new(b) })
            }
            _ => None,
        }
    }
}

impl Surface {
    /// Un identifiant de la surface : tel quel pour celle de l'application, dérivé de
    /// `base` pour une surface intérieure.
    fn k(&self, i: Id) -> Id {
        self.base.map_or(i, |b| sub_id(i, b as usize))
    }

    /// Une seule fenêtre, avec cette vue.
    pub fn new(vue: u32) -> Surface {
        Surface::de(Noeud::Fenetre { id: 1, vue })
    }

    fn de(racine: Noeud) -> Surface {
        let suivant = racine.max_id() + 1;
        Surface { racine, libres: None, deplace: None, suivant, tirage: None, menu: None, accueil: 0, rect: Rect::default(), placees: Vec::new(), changements: Vec::new(), modifiee: false, base: None }
    }

    /// Les fenêtres dans le rectangle `r` : identifiant, vue et cadre.
    pub fn disposer(&self, r: Rect) -> Vec<(FenetreId, u32, Rect)> {
        if let Some(l) = &self.libres {
            return l.iter().map(|f| (f.id, f.vue, cadre_libre(f.place, r))).collect();
        }
        let mut out = Vec::new();
        self.racine.disposer(r, &mut out);
        out
    }

    pub fn nombre(&self) -> usize {
        self.libres.as_ref().map_or_else(|| self.racine.compte(), Vec::len)
    }

    /// Les fenêtres placées à la dernière image.
    pub fn placees(&self) -> &[Placee] {
        &self.placees
    }

    /// La fenêtre dont le contenu contient (x, y), à la dernière image.
    pub fn fenetre_en(&self, x: f32, y: f32) -> Option<&Placee> {
        self.placees.iter().rev().find(|p| p.contenu.contains(x, y))
    }

    pub fn vue(&self, f: FenetreId) -> Option<u32> {
        if let Some(l) = &self.libres {
            return l.iter().find(|x| x.id == f).map(|x| x.vue);
        }
        fn chercher(n: &Noeud, f: FenetreId) -> Option<u32> {
            match n {
                Noeud::Fenetre { id, vue } => (*id == f).then_some(*vue),
                Noeud::Partage { a, b, .. } => chercher(a, f).or_else(|| chercher(b, f)),
            }
        }
        chercher(&self.racine, f)
    }

    /// Change la vue d'une fenêtre.
    pub fn changer_vue(&mut self, f: FenetreId, nouvelle: u32) {
        fn changer(n: &mut Noeud, f: FenetreId, nouvelle: u32) -> bool {
            match n {
                Noeud::Fenetre { id, vue } if *id == f => {
                    *vue = nouvelle;
                    true
                }
                Noeud::Fenetre { .. } => false,
                Noeud::Partage { a, b, .. } => changer(a, f, nouvelle) || changer(b, f, nouvelle),
            }
        }
        let fait = match &mut self.libres {
            Some(l) => l.iter_mut().find(|x| x.id == f).map(|x| x.vue = nouvelle).is_some(),
            None => changer(&mut self.racine, f, nouvelle),
        };
        if fait {
            self.changements.push(Changement::Vue { id: f, vue: nouvelle });
            self.modifiee = true;
        }
    }

    /// Partage la fenêtre `f` : la nouvelle prend la première part (`ratio`), avec la même
    /// vue. Renvoie son identifiant.
    pub fn partager(&mut self, f: FenetreId, sens: Sens, ratio: f32) -> Option<FenetreId> {
        self.partager_ou(f, sens, ratio, false, None)
    }

    /// Partage la fenêtre `f` : elle garde la première part (`ratio`) et la nouvelle,
    /// avec la même vue, prend la seconde (à droite, ou en bas). Renvoie son identifiant.
    pub fn partager_apres(&mut self, f: FenetreId, sens: Sens, ratio: f32) -> Option<FenetreId> {
        self.partager_ou(f, sens, ratio, true, None)
    }

    /// Partage la fenêtre `f` en deux moitiés : la nouvelle, à droite ou en bas, montre
    /// la vue `vue` (`Changement::Nouvelle`).
    pub fn scinder(&mut self, f: FenetreId, sens: Sens, vue: u32) -> Option<FenetreId> {
        self.partager_ou(f, sens, 0.5, true, Some(vue))
    }

    /// Insère une fenêtre neuve montrant `vue` sur un bord de toute la surface : à
    /// gauche (`Colonnes`, `debut`) ou en bas (`Lignes`, pas `debut`)… Elle prend une
    /// part égale à celle des fenêtres déjà rangées dans ce sens au premier niveau
    /// (`Changement::Nouvelle`). Renvoie son identifiant.
    pub fn inserer(&mut self, sens: Sens, debut: bool, vue: u32) -> FenetreId {
        fn rangs(n: &Noeud, sens: Sens) -> usize {
            match n {
                Noeud::Partage { sens: s, a, b, .. } if *s == sens => rangs(a, sens) + rangs(b, sens),
                _ => 1,
            }
        }
        let nouvelle = self.suivant;
        if let Some(l) = &mut self.libres {
            // En cascade sur la fenêtre de devant ; revient en haut à gauche au bord
            let (w, h) = if self.rect.w >= self.rect.h { (0.5, 0.6) } else { (0.85, 0.5) };
            let (mut x, mut y) = l.iter().rev().find(|f| f.etendue == Etendue::Normale).map_or((0.05, 0.05), |f| (f.place[0] + 0.04, f.place[1] + 0.05));
            if x + w > 1.0 || y + h > 1.0 {
                (x, y) = (0.03, 0.03);
            }
            l.push(Flottante { id: nouvelle, vue, place: [x, y, w, h], etendue: Etendue::Normale });
            self.suivant += 1;
            self.changements.push(Changement::Nouvelle(nouvelle));
            self.modifiee = true;
            return nouvelle;
        }
        let part = 1.0 / (rangs(&self.racine, sens) as f32 + 1.0);
        let ancienne = std::mem::replace(&mut self.racine, Noeud::Fenetre { id: 0, vue: 0 });
        let nv = Box::new(Noeud::Fenetre { id: nouvelle, vue });
        let (a, b, ratio) = if debut { (nv, Box::new(ancienne), part) } else { (Box::new(ancienne), nv, 1.0 - part) };
        self.racine = Noeud::Partage { sens, ratio: ratio.clamp(MIN, MAX), a, b };
        self.suivant += 1;
        self.changements.push(Changement::Nouvelle(nouvelle));
        self.modifiee = true;
        nouvelle
    }

    fn partager_ou(&mut self, f: FenetreId, sens: Sens, ratio: f32, apres: bool, autre: Option<u32>) -> Option<FenetreId> {
        let nouvelle = self.suivant;
        fn remplacer(n: &mut Noeud, f: FenetreId, sens: Sens, ratio: f32, nouvelle: FenetreId, apres: bool, autre: Option<u32>) -> bool {
            match n {
                Noeud::Fenetre { id, vue } if *id == f => {
                    let vue = *vue;
                    let (nv, an) = (Box::new(Noeud::Fenetre { id: nouvelle, vue: autre.unwrap_or(vue) }), Box::new(Noeud::Fenetre { id: f, vue }));
                    let (a, b) = if apres { (an, nv) } else { (nv, an) };
                    *n = Noeud::Partage { sens, ratio: ratio.clamp(MIN, MAX), a, b };
                    true
                }
                Noeud::Fenetre { .. } => false,
                Noeud::Partage { a, b, .. } => remplacer(a, f, sens, ratio, nouvelle, apres, autre) || remplacer(b, f, sens, ratio, nouvelle, apres, autre),
            }
        }
        if let Some(l) = &mut self.libres {
            // La place de la fenêtre se coupe en deux ; la nouvelle passe devant
            let i = l.iter().position(|x| x.id == f)?;
            let ratio = ratio.clamp(MIN, MAX);
            let [x, y, w, h] = l[i].place;
            let (a, b) = match sens {
                Sens::Colonnes => ([x, y, w * ratio, h], [x + w * ratio, y, w * (1.0 - ratio), h]),
                Sens::Lignes => ([x, y, w, h * ratio], [x, y + h * ratio, w, h * (1.0 - ratio)]),
            };
            let (garde, donne) = if apres { (a, b) } else { (b, a) };
            l[i].place = garde;
            l[i].etendue = Etendue::Normale;
            let vue = autre.unwrap_or(l[i].vue);
            l.push(Flottante { id: nouvelle, vue, place: donne, etendue: Etendue::Normale });
        } else if !remplacer(&mut self.racine, f, sens, ratio, nouvelle, apres, autre) {
            return None;
        }
        self.suivant += 1;
        self.changements.push(if autre.is_some() { Changement::Nouvelle(nouvelle) } else { Changement::Partagee { de: f, nouvelle } });
        self.modifiee = true;
        Some(nouvelle)
    }

    /// Ferme la fenêtre `f` : sa voisine reprend la place. La dernière ne se ferme pas.
    pub fn fermer(&mut self, f: FenetreId) -> bool {
        fn oter(n: &mut Noeud, f: FenetreId) -> bool {
            let Noeud::Partage { a, b, .. } = n else { return false };
            let reste = if matches!(**a, Noeud::Fenetre { id, .. } if id == f) {
                Some(std::mem::replace(&mut **b, Noeud::Fenetre { id: 0, vue: 0 }))
            } else if matches!(**b, Noeud::Fenetre { id, .. } if id == f) {
                Some(std::mem::replace(&mut **a, Noeud::Fenetre { id: 0, vue: 0 }))
            } else {
                None
            };
            match reste {
                Some(r) => {
                    *n = r;
                    true
                }
                None => oter(a, f) || oter(b, f),
            }
        }
        let fait = match &mut self.libres {
            Some(l) if l.len() > 1 => l.iter().position(|x| x.id == f).map(|i| l.remove(i)).is_some(),
            Some(_) => false,
            None => oter(&mut self.racine, f),
        };
        if !fait {
            return false;
        }
        if self.menu == Some(f) {
            self.menu = None;
        }
        self.changements.push(Changement::Fermee(f));
        self.modifiee = true;
        true
    }

    /// Amène les bords droit et bas de la fenêtre `f` en (x, y), dans la surface `r`.
    pub fn tirer(&mut self, r: Rect, f: FenetreId, x: f32, y: f32, mini: f32) {
        self.racine.tirer(r, f, x, y, mini, &mut (false, false));
        self.modifiee = true;
    }

    /// Les changements depuis le dernier appel.
    pub fn changements(&mut self) -> Vec<Changement> {
        std::mem::take(&mut self.changements)
    }

    /// La disposition en texte, par exemple `C0.3000(F2:0,L0.5000(F3:1,F1:0))`.
    pub fn ecrire(&self) -> String {
        self.ecrire_avec(&|v| v.to_string())
    }

    /// La disposition en texte, chaque vue écrite par `nom` (une clé, par exemple :
    /// `C0.3000(F2:cube,F1:aide)`).
    pub fn ecrire_avec(&self, nom: &dyn Fn(u32) -> String) -> String {
        if let Some(l) = &self.libres {
            let f: Vec<String> = l
                .iter()
                .map(|f| {
                    let e = match f.etendue {
                        Etendue::Normale => "",
                        Etendue::Reduite => " reduite",
                    };
                    format!("F{}:{} {:.4} {:.4} {:.4} {:.4}{e}", f.id, nom(f.vue), f.place[0], f.place[1], f.place[2], f.place[3])
                })
                .collect();
            return format!("libre({})", f.join(";"));
        }
        let mut s = String::new();
        self.racine.ecrire(&mut s, nom);
        s
    }

    /// Relit `ecrire` ; rien si le texte ne se tient pas.
    pub fn lire(texte: &str) -> Option<Surface> {
        Surface::lire_avec(texte, &|t| t.parse().ok())
    }

    /// Relit `ecrire_avec` : `vue_de` retrouve la vue d'après ce qui a été écrit.
    pub fn lire_avec(texte: &str, vue_de: &dyn Fn(&str) -> Option<u32>) -> Option<Surface> {
        let mut s = texte.trim();
        if let Some(corps) = s.strip_prefix("libre(").and_then(|c| c.strip_suffix(')')) {
            let mut l = Vec::new();
            for f in corps.split(';') {
                let mut m = f.split_whitespace();
                let (id, vue) = m.next()?.strip_prefix('F')?.split_once(':')?;
                let n: Vec<f32> = m.by_ref().take(4).map(|x| x.parse().ok()).collect::<Option<_>>()?;
                let [x, y, w, h] = n[..] else { return None };
                let etendue = match m.next() {
                    None => Etendue::Normale,
                    // Les espaces d'avant la 0.37 pouvaient agrandir une fenêtre
                    Some("agrandie") => Etendue::Normale,
                    Some("reduite") => Etendue::Reduite,
                    Some(_) => return None,
                };
                l.push(Flottante { id: id.parse().ok()?, vue: vue_de(vue)?, place: [x, y, w.max(0.05), h.max(0.05)], etendue });
            }
            let mut sf = Surface::de(Surface::pavage(&l)?);
            sf.suivant = l.iter().map(|f| f.id).max().unwrap_or(0) + 1;
            sf.libres = Some(l);
            return Some(sf);
        }
        let racine = Noeud::lire(&mut s, vue_de)?;
        s.is_empty().then(|| Surface::de(racine))
    }

    /// Vrai en fenêtres libres.
    pub fn libre(&self) -> bool {
        self.libres.is_some()
    }

    /// Passe en fenêtres libres (chacune garde la place qu'elle avait), ou revient au
    /// pavage (les fenêtres côte à côte, de gauche à droite). Les fenêtres et leurs
    /// identifiants restent.
    pub fn mettre_libre(&mut self, oui: bool) {
        if oui == self.libre() {
            return;
        }
        if oui {
            let r = if self.rect.w > 0.0 && self.rect.h > 0.0 { self.rect } else { Rect::new(0.0, 0.0, 1.0, 1.0) };
            let l = self
                .disposer(r)
                .into_iter()
                .map(|(id, vue, c)| Flottante { id, vue, place: [(c.x - r.x) / r.w, (c.y - r.y) / r.h, c.w / r.w, c.h / r.h], etendue: Etendue::Normale })
                .collect();
            self.libres = Some(l);
        } else if let Some(l) = self.libres.take() {
            if let Some(n) = Surface::pavage(&l) {
                self.racine = n;
            }
        }
        self.menu = None;
        self.tirage = None;
        self.deplace = None;
        self.modifiee = true;
    }

    /// Les fenêtres libres en pavage : côte à côte, de gauche à droite, à parts égales.
    fn pavage(l: &[Flottante]) -> Option<Noeud> {
        let mut v: Vec<&Flottante> = l.iter().collect();
        v.sort_by(|a, b| (a.place[0] + a.place[2] / 2.0).total_cmp(&(b.place[0] + b.place[2] / 2.0)));
        let mut n = None;
        for (k, f) in v.iter().enumerate().rev() {
            let feuille = Noeud::Fenetre { id: f.id, vue: f.vue };
            n = Some(match n {
                None => feuille,
                Some(reste) => Noeud::Partage { sens: Sens::Colonnes, ratio: (1.0 / (v.len() - k) as f32).clamp(MIN, MAX), a: Box::new(feuille), b: Box::new(reste) },
            });
        }
        n
    }

    /// Une fenêtre dans son cadre : sa barre en bas, le titre, la ligne d'état, la
    /// poignée.
    fn placer(cx: &mut Cx, id: FenetreId, vue: u32, c: Rect, nom: &str, boutons: f32) -> Placee {
        let d = cx.dp;
        let hb = (BARRE * cx.commande()).round();
        let bh = hb.min(c.h / 2.0);
        let b = Rect::new(c.x, c.bottom() - bh, c.w, bh);
        let ic = (bh * 0.6).round();
        let tw = cx.measure(nom, 14.0);
        let poignee = Rect::new((b.right() - bh).max(b.x), b.y, bh.min(b.w), bh);
        let titre = Rect::new(b.x, b.y, (8.0 * d + ic + 6.0 * d + tw + 10.0 * d).min(poignee.x - b.x), bh);
        let etat = Rect::new(titre.right(), b.y, (poignee.x - boutons * bh - titre.right()).max(0.0), bh);
        let suite = (c.x + cx.coin(), c.y + cx.coin());
        Placee { id, vue, cadre: c, contenu: Rect::new(c.x, c.y, c.w, c.h - bh), barre: b, titre, etat, suite }
    }

    /// La barre d'une fenêtre : le fond, le titre qui ouvre le menu.
    fn dessiner_barre(&mut self, cx: &mut Cx, p: &Placee, nom: &str, icone: Icon) {
        let (d, col) = (cx.dp, cx.couleurs());
        let (b, titre) = (p.barre, p.titre);
        let k = p.id as usize;
        let ic = (b.h * 0.6).round();
        cx.dl.rrect(b, 0.0, col.surface);
        cx.dl.rrect(Rect::new(b.x, b.y, b.w, d.max(1.0)), 0.0, col.ligne);
        // Toucher l'icône ou le titre ouvre le menu
        let tk = sub_id(self.k(TITRE_ID), k);
        cx.ui.region_add(tk, titre, Sense::Click { scroll: None });
        if cx.ui.clicked(tk) {
            self.menu = if self.menu == Some(p.id) { None } else { Some(p.id) };
        }
        let ouvert = self.menu == Some(p.id);
        if ouvert || cx.ui.pressed(tk) {
            cx.dl.rrect(titre.inset(3.0 * d), cx.theme.rayon(titre.h - 6.0 * d), col.puce);
        }
        let poignee_x = (b.right() - b.h).max(b.x);
        cx.clip(Some(Rect::new(b.x, b.y, poignee_x - b.x, b.h)));
        cx.icon(Rect::new(b.x + 8.0 * d, b.cy() - ic / 2.0, ic, ic), icone, col.accent);
        cx.text_at(b.x + 8.0 * d + ic + 6.0 * d, b.cy(), nom, 14.0, col.accent);
        cx.clip(None);
    }

    /// La poignée : un point, et un tiret le long de chaque bord qu'elle déplace.
    fn dessiner_poignee(cx: &mut Cx, p: &Placee, (px, pl): (bool, bool), appuyee: bool) {
        let (d, col) = (cx.dp, cx.couleurs());
        let b = p.barre;
        let poignee = Rect::new((b.right() - b.h).max(b.x), b.y, b.h.min(b.w), b.h);
        let couleur = if appuyee {
            col.accent
        } else if px || pl {
            col.texte
        } else {
            let [r, g, b, _] = col.pale;
            [r, g, b, 110]
        };
        let (q, t) = (poignee.inset(b.h * 0.22), 2.0 * d);
        let rd = 2.6 * d;
        cx.dl.rrect(Rect::new(q.cx() - rd, q.cy() - rd, 2.0 * rd, 2.0 * rd), rd, couleur);
        if px {
            cx.dl.line([q.right(), q.y], [q.right(), q.bottom()], t, couleur);
        }
        if pl {
            cx.dl.line([q.x, q.bottom()], [q.right(), q.bottom()], t, couleur);
        }
    }

    /// Dispose les fenêtres dans `r`, dessine leurs barres et suit les gestes. `vues` :
    /// le nom et l'icône de chaque vue, pour les titres et le menu. À appeler avant le
    /// contenu. En fenêtres libres, les barres se dessinent avec chaque fenêtre
    /// (`habiller`).
    pub fn frame(&mut self, cx: &mut Cx, r: Rect, vues: &[(&str, Icon)]) -> Vec<Placee> {
        let (d, col) = (cx.dp, cx.couleurs());
        let fil = d.max(1.0);
        let hb = (BARRE * cx.commande()).round();
        self.rect = r;
        let nom_de = |vue: u32| vues.get(vue as usize).copied().unwrap_or(("?", Icon::Help));
        if self.libres.is_some() {
            let tenu = |s: &Surface, f: FenetreId| cx.ui.pressed(sub_id(s.k(DEPLACER_ID), f as usize)) || cx.ui.pressed(sub_id(s.k(POIGNEE_ID), f as usize));
            if self.deplace.is_some_and(|t| !tenu(self, t.0)) {
                self.deplace = None;
            }
            if self.tirage.is_some_and(|t| !tenu(self, t.0)) {
                self.tirage = None;
            }
            // Une fenêtre touchée passe devant
            let appuis: Vec<(f32, f32)> = cx.ui.appuis().to_vec();
            for (x, y) in appuis {
                let touchee = self.disposer(r).into_iter().zip(self.libres.as_ref().unwrap()).rev().find(|((_, _, c), f)| f.etendue != Etendue::Reduite && c.contains(x, y)).map(|(_, f)| f.id);
                if let Some(f) = touchee {
                    self.passer_devant(f);
                }
            }
            // Le bureau, puis les fenêtres qui se montrent
            cx.dl.rrect(r, 0.0, col.puce);
            let visibles: Vec<(FenetreId, u32, Rect)> = self.disposer(r).into_iter().zip(self.libres.as_ref().unwrap()).filter(|(_, f)| f.etendue != Etendue::Reduite).map(|(x, _)| x).collect();
            self.placees = visibles.into_iter().map(|(id, vue, c)| Surface::placer(cx, id, vue, c, nom_de(vue).0, 0.0)).collect();
            return self.placees.clone();
        }
        let cadres = self.disposer(r);
        self.placees.clear();
        let mut tire = false;
        for (id, vue, c) in cadres {
            let k = id as usize;
            let (nom, icone) = nom_de(vue);
            let p = Surface::placer(cx, id, vue, c, nom, 0.0);
            self.placees.push(p);
            self.dessiner_barre(cx, &p, nom, icone);
            let b = p.barre;
            let poignee = Rect::new((b.right() - b.h).max(b.x), b.y, b.h.min(b.w), b.h);
            // La poignée déplace les bords qu'elle a en prise
            let (px, pl) = self.racine.prises(id).unwrap_or((false, false));
            let pk = sub_id(self.k(POIGNEE_ID), k);
            if px || pl {
                if let Some(g) = cx.ui.drag(pk, poignee) {
                    let (_, ox, oy) = *self.tirage.get_or_insert((id, c.right() - g.x, c.bottom() - g.y));
                    if g.moved && self.tirage.is_some_and(|t| t.0 == id) {
                        tire = true;
                        let (x, y) = (if px { g.x + ox } else { 0.0 }, if pl { g.y + oy } else { 0.0 });
                        self.racine.tirer(r, id, x, y, hb + 16.0 * d, &mut (!px, !pl));
                        self.modifiee = true;
                    }
                }
            }
            Surface::dessiner_poignee(cx, &p, (px, pl), cx.ui.pressed(pk));
            // La séparation à droite, sauf au bord de la surface
            if c.right() < r.right() - 0.5 {
                cx.dl.rrect(Rect::new(c.right() - fil, c.y, fil, c.h), 0.0, col.ligne);
            }
        }
        if !tire && !self.placees.iter().any(|p| cx.ui.pressed(sub_id(self.k(POIGNEE_ID), p.id as usize))) {
            self.tirage = None;
        }
        self.placees.clone()
    }

    /// En fenêtres libres, juste avant le contenu de la fenêtre `p` : son ombre, son
    /// fond (qui garde les doigts des fenêtres de derrière), sa barre (la tirer déplace
    /// la fenêtre) et sa poignée (qui la redimensionne). Ne fait rien en pavage.
    pub fn habiller(&mut self, cx: &mut Cx, p: &Placee, vues: &[(&str, Icon)]) {
        if self.libres.is_none() {
            return;
        }
        let (d, col) = (cx.dp, cx.couleurs());
        let (c, r, k) = (p.cadre, self.rect, p.id as usize);
        let (nom, icone) = vues.get(p.vue as usize).copied().unwrap_or(("?", Icon::Help));
        cx.dl.shadow(Rect::new(c.x, c.y + 3.0 * d, c.w, c.h), 14.0 * d, 14.0 * d, col.ombre);
        cx.dl.rrect(c.inset(-d.max(1.0)), 0.0, col.ligne);
        cx.dl.rrect(c, 0.0, col.fond);
        cx.ui.region_add(sub_id(self.k(FOND_ID), k), c, Sense::Sink);
        let hb = p.barre.h;
        let (mini_w, mini_h) = ((160.0 * d).min(r.w), (3.0 * hb).min(r.h));
        // Tirer la barre déplace la fenêtre, sans la perdre hors de la surface
        let dk = sub_id(self.k(DEPLACER_ID), k);
        if let Some(g) = cx.ui.drag(dk, p.barre) {
            let (_, ox, oy) = *self.deplace.get_or_insert((p.id, g.x - c.x, g.y - c.y));
            if g.moved && self.deplace.is_some_and(|t| t.0 == p.id) {
                let garde = (80.0 * d).min(c.w);
                let x = (g.x - ox).clamp(r.x - c.w + garde, r.right() - garde);
                let y = (g.y - oy).clamp(r.y, (r.bottom() - c.h).max(r.y));
                self.placer_libre(p.id, Rect::new(x, y, c.w, c.h));
            }
        }
        self.dessiner_barre(cx, p, nom, icone);
        let b = p.barre;
        // La poignée : le coin bas droit suit le doigt
        let poignee = Rect::new((b.right() - b.h).max(b.x), b.y, b.h.min(b.w), b.h);
        let pk = sub_id(self.k(POIGNEE_ID), k);
        if let Some(g) = cx.ui.drag(pk, poignee) {
            let (_, ox, oy) = *self.tirage.get_or_insert((p.id, c.right() - g.x, c.bottom() - g.y));
            if g.moved && self.tirage.is_some_and(|t| t.0 == p.id) {
                let w = (g.x + ox - c.x).clamp(mini_w, (r.right() - c.x).max(mini_w));
                let h = (g.y + oy - c.y).clamp(mini_h, (r.bottom() - c.y).max(mini_h));
                self.placer_libre(p.id, Rect::new(c.x, c.y, w, h));
            }
        }
        Surface::dessiner_poignee(cx, p, (true, true), cx.ui.pressed(pk));
    }

    /// Les fenêtres libres, de l'arrière vers l'avant (rien en pavage).
    pub fn libres(&self) -> &[Flottante] {
        self.libres.as_deref().unwrap_or(&[])
    }

    pub fn etendue(&self, f: FenetreId) -> Option<Etendue> {
        self.libres().iter().find(|x| x.id == f).map(|x| x.etendue)
    }

    /// La fenêtre libre de devant, parmi celles qui se montrent.
    pub fn devant(&self) -> Option<FenetreId> {
        self.libres().iter().rev().find(|x| x.etendue != Etendue::Reduite).map(|x| x.id)
    }

    fn passer_devant(&mut self, f: FenetreId) {
        if let Some(l) = &mut self.libres {
            if let Some(i) = l.iter().position(|x| x.id == f) {
                if i + 1 < l.len() {
                    let x = l.remove(i);
                    l.push(x);
                    self.modifiee = true;
                }
            }
        }
    }

    /// Réduit ou rétablit la fenêtre libre `f` ; rétablie, elle passe devant.
    pub fn etendre(&mut self, f: FenetreId, e: Etendue) {
        let Some(x) = self.libres.as_mut().and_then(|l| l.iter_mut().find(|x| x.id == f)) else { return };
        x.etendue = e;
        self.modifiee = true;
        if self.menu == Some(f) {
            self.menu = None;
        }
        if e != Etendue::Reduite {
            self.passer_devant(f);
        }
    }

    /// Ce que fait la barre des tâches : une fenêtre réduite ou cachée se montre et
    /// passe devant ; celle de devant se réduit.
    pub fn basculer(&mut self, f: FenetreId) {
        match self.etendue(f) {
            Some(Etendue::Reduite) => self.etendre(f, Etendue::Normale),
            Some(_) if self.devant() == Some(f) => self.etendre(f, Etendue::Reduite),
            Some(_) => self.passer_devant(f),
            None => {}
        }
    }

    /// Range la fenêtre libre `f` dans le cadre `c` (en part de la surface).
    fn placer_libre(&mut self, f: FenetreId, c: Rect) {
        let r = self.rect;
        if r.w <= 0.0 || r.h <= 0.0 {
            return;
        }
        if let Some(x) = self.libres.as_mut().and_then(|l| l.iter_mut().find(|x| x.id == f)) {
            x.place = [(c.x - r.x) / r.w, (c.y - r.y) / r.h, c.w / r.w, c.h / r.h];
            self.modifiee = true;
        }
    }

    /// Le menu ouvert, par-dessus le contenu des fenêtres.
    pub fn dessus(&mut self, cx: &mut Cx, vues: &[(&str, Icon)]) {
        let (d, col) = (cx.dp, cx.couleurs());
        let Some(p) = self.menu.and_then(|f| self.placees.iter().find(|p| p.id == f).copied()) else {
            self.menu = None;
            return;
        };
        // Toucher hors du menu le referme
        if cx.ui.click(self.k(FOND_MENU), self.rect, None) {
            self.menu = None;
            return;
        }
        // Les vues, puis la ligne des partages (deux icônes), puis la fermeture
        let fermer = self.nombre() > 1;
        let lignes = vues.len() + 1 + fermer as usize;
        let (w, lh) = ((MENU_LARGE * d * cx.theme.texte).min(self.rect.w), (MENU_LIGNE * cx.commande()).round());
        let h = lh * lignes as f32 + 8.0 * d;
        // Au-dessus du titre, qui l'a ouvert
        let x = (p.titre.x + 4.0 * d).min(self.rect.right() - w).max(self.rect.x);
        let y = (p.barre.y - h - 2.0 * d).max(self.rect.y);
        let m = Rect::new(x, y, w, h);
        cx.ui.region_add(self.k(id("surface/menu-fond")), m, Sense::Sink);
        cx.dl.shadow(Rect::new(m.x, m.y + 2.0 * d, m.w, m.h), 10.0 * d, 10.0 * d, col.ombre);
        cx.dl.rrect(m, 10.0 * d, col.surface);
        let ic = (lh * 0.5).round();
        for i in 0..lignes {
            let l = Rect::new(m.x + 4.0 * d, m.y + 4.0 * d + i as f32 * lh, m.w - 8.0 * d, lh);
            let k = sub_id(self.k(LIGNE_MENU), i);
            let clic = cx.ui.click(k, l, None);
            if cx.ui.pressed(k) {
                cx.dl.rrect(l, 8.0 * d, col.puce);
            }
            if let Some((nom, icone)) = vues.get(i).copied() {
                let actuelle = i as u32 == p.vue;
                let teinte = if actuelle { col.accent } else { col.texte };
                cx.icon(Rect::new(l.x + 10.0 * d, l.cy() - ic / 2.0, ic, ic), icone, if actuelle { col.accent } else { col.pale });
                cx.text_at(l.x + 18.0 * d + ic, l.cy(), nom, 15.0, teinte);
                if clic {
                    self.menu = None;
                    if !actuelle {
                        self.changer_vue(p.id, i as u32);
                    }
                }
            } else if i == vues.len() {
                // Deux moitiés : côte à côte, et l'une sur l'autre ; la nouvelle fenêtre
                // montre l'Accueil
                cx.dl.rrect(Rect::new(l.x + 8.0 * d, l.y, l.w - 16.0 * d, d.max(1.0)), 0.0, col.ligne);
                for (k, (sens, icone)) in [(Sens::Colonnes, Icon::ScinderColonnes), (Sens::Lignes, Icon::ScinderLignes)].into_iter().enumerate() {
                    let moitie = Rect::new(l.x + k as f32 * l.w / 2.0, l.y, l.w / 2.0, l.h);
                    let mk = sub_id(sub_id(self.k(LIGNE_MENU), 1000), k);
                    let appui = cx.ui.click(mk, moitie.inset(2.0 * d), None);
                    if cx.ui.pressed(mk) {
                        cx.dl.rrect(moitie.inset(2.0 * d), 8.0 * d, col.puce);
                    }
                    cx.icon(Rect::new(moitie.cx() - ic / 2.0, l.cy() - ic / 2.0, ic, ic), icone, col.texte);
                    if appui {
                        self.menu = None;
                        self.scinder(p.id, sens, self.accueil);
                    }
                }
            } else {
                cx.dl.rrect(Rect::new(l.x + 8.0 * d, l.y, l.w - 16.0 * d, d.max(1.0)), 0.0, col.ligne);
                cx.icon(Rect::new(l.x + 10.0 * d, l.cy() - ic / 2.0, ic, ic).inset(ic * 0.1), Icon::Close, col.pale);
                cx.text_at(l.x + 18.0 * d + ic, l.cy(), "Fermer la fenêtre", 15.0, col.pale);
                if clic {
                    self.fermer(p.id);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draw::DrawList;
    use crate::text::Text;
    use crate::ui::{Event, Ui};

    const R: Rect = Rect::new(0., 0., 1000., 800.);
    const NOMS: [(&str, Icon); 2] = [("3D", Icon::Cube), ("Liste", Icon::List)];

    struct Banc {
        s: Surface,
        ui: Ui,
        text: Text,
        dl: DrawList,
        t: f64,
    }

    impl Banc {
        fn new() -> Banc {
            let mut b = Banc { s: Surface::new(0), ui: Ui::new(1.0), text: Text::new(), dl: DrawList::default(), t: 0.0 };
            b.image();
            b
        }
        fn image(&mut self) -> Vec<Placee> {
            self.dl.clear();
            let mut cx = Cx { ui: &mut self.ui, text: &mut self.text, dl: &mut self.dl, dp: 1.0, theme: Default::default(), palette: None };
            let p = self.s.frame(&mut cx, R, &NOMS);
            for q in &p {
                self.s.habiller(&mut cx, q, &NOMS);
            }
            self.s.dessus(&mut cx, &NOMS);
            self.ui.end_frame();
            p
        }
        fn evt(&mut self, e: Event) {
            self.t += 0.05;
            self.ui.event(e, self.t);
        }
        /// Un doigt glisse de a à b, une image entre chaque pas, puis se lève.
        fn glisser(&mut self, a: (f32, f32), b: (f32, f32)) {
            self.evt(Event::Down { id: 0, x: a.0, y: a.1 });
            self.image();
            for k in 1..=10 {
                let t = k as f32 / 10.0;
                self.evt(Event::Move { id: 0, x: a.0 + (b.0 - a.0) * t, y: a.1 + (b.1 - a.1) * t });
                self.image();
            }
            self.evt(Event::Up { id: 0 });
            self.image();
            self.image();
        }
        fn toucher(&mut self, x: f32, y: f32) {
            self.evt(Event::Down { id: 0, x, y });
            self.evt(Event::Up { id: 0 });
            self.image();
            self.image();
        }
    }

    #[test]
    fn les_fenetres_pavent_la_surface() {
        let mut s = Surface::new(0);
        let n = s.partager(1, Sens::Colonnes, 0.3).unwrap();
        s.partager(1, Sens::Lignes, 0.5).unwrap();
        let f = s.disposer(R);
        assert_eq!(f.len(), 3);
        assert_eq!(f.iter().map(|(_, _, r)| r.w * r.h).sum::<f32>(), R.w * R.h);
        // La nouvelle prend la première part, avec la même vue
        assert_eq!(f[0], (n, 0, Rect::new(0., 0., 300., 800.)));
        assert_eq!(s.changements(), vec![Changement::Partagee { de: 1, nouvelle: 2 }, Changement::Partagee { de: 1, nouvelle: 3 }]);
    }

    #[test]
    fn partager_apres_place_la_nouvelle_a_droite() {
        let mut s = Surface::new(0);
        let n = s.partager_apres(1, Sens::Colonnes, 0.4).unwrap();
        let f = s.disposer(R);
        assert_eq!(f[0], (1, 0, Rect::new(0., 0., 400., 800.)));
        assert_eq!(f[1], (n, 0, Rect::new(400., 0., 600., 800.)));
        assert_eq!(s.changements(), vec![Changement::Partagee { de: 1, nouvelle: n }]);
    }

    #[test]
    fn fermer_rend_la_place_a_la_voisine() {
        let mut s = Surface::new(0);
        s.partager(1, Sens::Colonnes, 0.3);
        s.partager(1, Sens::Lignes, 0.5);
        assert!(s.fermer(3));
        let f = s.disposer(R);
        assert_eq!(f.iter().map(|x| x.0).collect::<Vec<_>>(), vec![2, 1]);
        assert_eq!(f[1].2, Rect::new(300., 0., 700., 800.));
        assert!(s.fermer(2));
        assert!(!s.fermer(1), "la dernière reste");
        assert_eq!(s.racine, Noeud::Fenetre { id: 1, vue: 0 });
        // Un identifiant ne resert jamais
        assert_eq!(s.partager(1, Sens::Lignes, 0.5), Some(4));
    }

    #[test]
    fn la_disposition_se_relit() {
        let mut s = Surface::new(0);
        s.partager(1, Sens::Colonnes, 0.3);
        s.partager(1, Sens::Lignes, 0.5);
        s.changer_vue(3, 1);
        let t = s.ecrire();
        assert_eq!(t, "C0.3000(F2:0,L0.5000(F3:1,F1:0))");
        let r = Surface::lire(&t).unwrap();
        assert_eq!(r.racine, s.racine);
        assert!(Surface::lire("C0.3(F1:0").is_none());
        // Avec des noms de vues
        let noms = ["cube", "liste"];
        let t = s.ecrire_avec(&|v| noms[v as usize].to_string());
        assert_eq!(t, "C0.3000(F2:cube,L0.5000(F3:liste,F1:cube))");
        let r = Surface::lire_avec(&t, &|n| noms.iter().position(|x| *x == n).map(|i| i as u32)).unwrap();
        assert_eq!(r.racine, s.racine);
        assert!(Surface::lire("n'importe quoi").is_none());
    }

    #[test]
    fn la_poignee_deplace_les_bords_droit_et_bas() {
        let mut s = Surface::new(0);
        s.partager(1, Sens::Colonnes, 0.5); // 2 | 1
        s.partager(2, Sens::Lignes, 0.5); // 3 sur 2, à gauche
        assert_eq!(s.racine.prises(3), Some((true, true)));
        assert_eq!(s.racine.prises(2), Some((true, false)));
        assert_eq!(s.racine.prises(1), Some((false, false)));
        s.tirer(R, 3, 700.0, 200.0, 50.0);
        let f = s.disposer(R);
        assert_eq!(f[0], (3, 0, Rect::new(0., 0., 700., 200.)));
        // Bornée à 90 %
        s.tirer(R, 3, 2000.0, 790.0, 50.0);
        let f = s.disposer(R);
        assert_eq!(f[0].2, Rect::new(0., 0., 900., 720.));
    }

    #[test]
    fn le_menu_change_la_vue_et_ferme() {
        let mut b = Banc::new();
        b.s.partager(1, Sens::Colonnes, 0.5);
        b.image();
        b.changements();
        // Le titre de la fenêtre de droite ouvre le menu, au-dessus de la barre
        b.toucher(520., 785.);
        assert_eq!(b.s.menu, Some(1));
        // Quatre lignes de 42 (deux vues, les partages, la fermeture) : le menu commence
        // à 764 - 176 - 2, ses lignes 4 plus bas
        let ligne = |i: f32| 586. + 4. + 42. * (i + 0.5);
        b.toucher(600., ligne(1.));
        assert_eq!(b.s.vue(1), Some(1));
        assert_eq!(b.s.menu, None);
        // Puis « Fermer la fenêtre »
        b.toucher(520., 785.);
        b.toucher(600., ligne(3.));
        assert_eq!(b.s.nombre(), 1);
        assert_eq!(b.s.changements(), vec![Changement::Vue { id: 1, vue: 1 }, Changement::Fermee(1)]);
        // Toucher à côté referme le menu sans rien changer
        b.toucher(20., 785.);
        assert!(b.s.menu.is_some());
        b.toucher(900., 100.);
        assert_eq!(b.s.menu, None);
        assert_eq!(b.s.nombre(), 1);
    }

    #[test]
    fn le_menu_scinde_et_la_nouvelle_montre_l_accueil() {
        let mut b = Banc::new();
        b.s.accueil = 1;
        b.image();
        b.changements();
        // Une seule fenêtre : trois lignes (deux vues, les partages), sans fermeture
        b.toucher(20., 785.);
        assert_eq!(b.s.menu, Some(1));
        let ligne = 764. - (42. * 3. + 8.) - 2. + 4. + 42. * 2.5;
        // La moitié droite : l'une sur l'autre
        let m = (MENU_LARGE).min(1000.);
        b.toucher(4. + 4. + m * 0.75, ligne);
        assert_eq!(b.s.nombre(), 2);
        assert_eq!(b.s.changements(), vec![Changement::Nouvelle(2)]);
        let p = b.image();
        assert_eq!(p[1].id, 2);
        assert_eq!(p[1].vue, 1);
        assert_eq!(p[1].cadre, Rect::new(0., 400., 1000., 400.));
        assert_eq!(p[0].vue, 0);
    }

    #[test]
    fn la_barre_porte_titre_etat_et_poignee() {
        let mut b = Banc::new();
        b.s.partager(1, Sens::Colonnes, 0.5);
        let p = b.image();
        assert_eq!(p[0].contenu, Rect::new(0., 0., 500., 764.));
        assert_eq!(p[0].barre, Rect::new(0., 764., 500., 36.));
        assert_eq!(p[0].titre.x, 0.);
        assert_eq!(p[0].etat.x, p[0].titre.right());
        assert_eq!(p[0].etat.right(), 464.);
        assert_eq!(p[1].suite, (500. + 14., 14.));
    }

    #[test]
    fn la_poignee_se_tire_au_doigt() {
        let mut b = Banc::new();
        b.s.partager(1, Sens::Colonnes, 0.5);
        b.image();
        // La poignée de la fenêtre de gauche, dans son coin bas droit
        b.glisser((486., 788.), (586., 700.));
        let p = b.image();
        assert!((p[0].cadre.w - 600.).abs() <= 1.0, "{:?}", p[0].cadre);
    }

    impl Banc {
        fn changements(&mut self) -> Vec<Changement> {
            self.s.changements()
        }
    }

    #[test]
    fn les_fenetres_libres_gardent_leur_place_et_s_ecrivent() {
        let mut b = Banc::new();
        b.s.partager_apres(1, Sens::Colonnes, 0.4);
        b.image();
        b.s.mettre_libre(true);
        let p = b.image();
        // Chacune garde la place qu'elle avait dans le pavage
        assert_eq!(p.iter().map(|p| p.cadre).collect::<Vec<_>>(), vec![Rect::new(0., 0., 400., 800.), Rect::new(400., 0., 600., 800.)]);
        let t = b.s.ecrire();
        assert!(t.starts_with("libre(F1:0 0.0000 0.0000 0.4000 1.0000;F2:0 0.4000"), "{t}");
        let mut relue = Surface::lire(&t).unwrap();
        assert!(relue.libre() && relue.nombre() == 2 && relue.ecrire() == t);
        // Une fenêtre insérée se pose en cascade, devant ; on revient au pavage
        let n = relue.inserer(Sens::Colonnes, true, 1);
        assert_eq!(relue.disposer(R).last().map(|x| (x.0, x.1)), Some((n, 1)));
        relue.mettre_libre(false);
        // Rangées de gauche à droite selon le milieu de leur place
        let mut ids: Vec<_> = relue.disposer(R).iter().map(|x| x.0).collect();
        assert_eq!(ids[0], 1);
        ids.sort();
        assert_eq!(ids, vec![1, 2, n]);
        assert_eq!(relue.disposer(R).iter().map(|x| x.2.w).sum::<f32>(), R.w);
    }

    #[test]
    fn tirer_la_barre_deplace_et_toucher_passe_devant() {
        let mut b = Banc::new();
        b.s.mettre_libre(true);
        b.image();
        b.s.inserer(Sens::Colonnes, true, 1);
        b.s.inserer(Sens::Colonnes, true, 1);
        let p = b.image();
        let (deux, trois) = (p[1], p[2]);
        assert_eq!(p.iter().map(|p| p.id).collect::<Vec<_>>(), vec![1, 2, 3]);
        // Toucher la 2 là où elle n'est pas couverte la passe devant
        b.toucher(deux.cadre.x + 10., deux.cadre.y + 10.);
        assert_eq!(b.image().iter().map(|p| p.id).collect::<Vec<_>>(), vec![1, 3, 2]);
        // Tirer la barre de la 3 (hors du titre) la déplace d'autant
        let a = (trois.etat.cx(), trois.barre.cy());
        b.glisser(a, (a.0 - 100., a.1 - 50.));
        let q = b.image().into_iter().find(|p| p.id == 3).unwrap();
        assert!((q.cadre.x - (trois.cadre.x - 100.)).abs() < 1.5 && (q.cadre.y - (trois.cadre.y - 50.)).abs() < 1.5, "{:?} {:?}", q.cadre, trois.cadre);
        // La poignée l'agrandit
        let pg = (q.barre.right() - q.barre.h / 2., q.barre.cy());
        b.glisser(pg, (pg.0 + 60., pg.1 + 40.));
        let z = b.image().into_iter().find(|p| p.id == 3).unwrap();
        assert!((z.cadre.w - (q.cadre.w + 60.)).abs() < 1.5 && (z.cadre.h - (q.cadre.h + 40.)).abs() < 1.5, "{:?} {:?}", z.cadre, q.cadre);
    }

    #[test]
    fn la_barre_des_taches_reduit_et_rend() {
        let mut b = Banc::new();
        b.s.mettre_libre(true);
        b.image();
        let n = b.s.inserer(Sens::Colonnes, true, 1);
        let q = *b.image().iter().find(|p| p.id == n).unwrap();
        // Plus de boutons dans la barre : l'état va jusqu'à la poignée
        assert!(q.etat.right() >= q.barre.right() - 1.5 * q.barre.h);
        // Les espaces d'avant agrandissaient : relus, la fenêtre est normale
        assert!(Surface::lire(&b.s.ecrire().replacen(')', " agrandie)", 1)).is_some());
        // Réduite : elle quitte l'écran mais pas la surface
        b.s.basculer(n);
        assert_eq!(b.s.etendue(n), Some(Etendue::Reduite));
        assert!(b.image().iter().all(|p| p.id != n));
        assert_eq!(b.s.disposer(R).len(), 2);
        assert_eq!(b.s.devant(), Some(1));
        let t = b.s.ecrire();
        assert!(t.contains(" reduite") && Surface::lire(&t).unwrap().ecrire() == t, "{t}");
        // La barre des tâches : la réduite se montre et passe devant ; celle de devant
        // se réduit ; une autre passe devant
        b.s.basculer(n);
        assert_eq!((b.s.etendue(n), b.s.devant()), (Some(Etendue::Normale), Some(n)));
        b.s.basculer(1);
        assert_eq!(b.s.devant(), Some(1));
        b.s.basculer(1);
        assert_eq!((b.s.etendue(1), b.s.devant()), (Some(Etendue::Reduite), Some(n)));
    }
}
