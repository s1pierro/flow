//! L'application : l'espace de travail partagé en fenêtres, chacune portant une
//! instance de contexte (`contexte`, `contextes`), le panneau latéral (paramètres de
//! l'application, aide, à propos), le lanceur à gauche (la liste des contextes, ouverte
//! par le logo), et en bas de l'écran la barre de l'application :
//! le logo (qui ouvre le lanceur, où sont aussi les liens des paramètres et de l'à
//! propos), les applets des contextes
//! (`applet` : radio, mixeur, énergie…) et l'heure.
//! Le bas de l'écran est réservé aux gestes du système, mais les appuis y restent
//! disponibles.
//!
//! L'espace (disposition, contexte et état de chaque fenêtre) s'écrit en texte avec
//! `espace` et se relit avec `ouvrir_espace` ; c'est le fichier `espace.txt`.

use crate::clavier::Edition;
use crate::contexte::{Commun, Contexte, Demande, Etat};
use crate::contextes::{self, GENRES};
use crate::draw::{Color, Rect};
use crate::espace::Espace;
use crate::parametres;
use crate::reglages::Reglages;
use crate::shell::{Layout, Shell, Side};
use crate::surface::{Changement, Etendue, FenetreId, Flottante, Placee, Surface};
use crate::ui::{id, sub_id, Id};
use crate::widgets::{Cx, Icon};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const BARRE: Id = id("barre-application");
const LOGO: Id = id("logo");
const TACHES: Id = id("barre-taches");
const LANCEUR: Id = id("lanceur");
const LANCEUR_LIGNE: Id = id("lanceur-ligne");

/// Ce que le moteur de rendu doit savoir de l'image : l'espace de travail, et le
/// fond de l'écran.
#[derive(Clone, Copy, Debug)]
pub struct Scene {
    pub workspace: Rect,
    pub fond: Color,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum EndContent {
    Parametres,
    Help,
    APropos,
}

pub struct App {
    pub shell: Shell,
    /// Ce que partagent toutes les fenêtres : apparence, clavier, stockage…
    pub commun: Commun,
    /// L'espace de travail, partagé en fenêtres.
    pub surface: Surface,
    /// L'instance de contexte de chaque fenêtre.
    fenetres: HashMap<FenetreId, Box<dyn Contexte>>,
    /// L'heure (heures, minutes), donnée par le système ; rien hors d'Android. La
    /// batterie est relevée par l'orchestrateur (`commun.orchestre`).
    pub heure: Option<(u8, u8)>,
    end_content: EndContent,
    /// L'onglet ouvert du panneau des paramètres, et son champ d'essai du clavier.
    pub onglet_parametres: usize,
    essai: Edition,
    /// Les applets de la barre, et le volet ouvert.
    pub applets: crate::applet::Applets,
    /// Où était chaque contexte du lanceur à la dernière image (vide s'il est fermé).
    pub zones_lanceur: Vec<Rect>,
    /// Où étaient ses liens, Paramètres puis À propos.
    pub zones_liens: Vec<Rect>,
}

/// Les liens du bas du lanceur : ils ouvrent le panneau de droite.
const LIENS: [(&str, EndContent); 2] = [("Paramètres", EndContent::Parametres), ("À propos", EndContent::APropos)];

/// La hauteur d'un lien du lanceur, et l'écart qui les sépare des bords.
fn mesures_liens(cx: &Cx) -> (f32, f32) {
    ((36.0 * cx.dp).round(), (10.0 * cx.dp).round())
}

/// La place que prennent les liens en bas du lanceur : empilés en paysage, sur une
/// ligne en portrait (renvoie largeur et hauteur de leur zone).
fn zone_liens(cx: &mut Cx, paysage: bool) -> (f32, f32) {
    let (h, m) = mesures_liens(cx);
    let largeurs: Vec<f32> = LIENS.iter().map(|(n, _)| cx.measure(n, 15.0) + 2.0 * m).collect();
    if paysage {
        (largeurs.iter().fold(0.0f32, |a, &b| a.max(b)) + 2.0 * m, LIENS.len() as f32 * h + 2.0 * m)
    } else {
        (largeurs.iter().sum::<f32>() + 2.0 * m, h + m)
    }
}

/// La place des contextes dans le lanceur, depuis son coin haut gauche (sans le
/// défilement), et la taille qu'il demande. Paysage : une liste, large de son plus
/// long élément, marges comprises (renvoie la largeur). Portrait : des tuiles côte à
/// côte, à la ligne quand la largeur `w` est pleine (renvoie la hauteur).
fn places_lanceur(cx: &mut Cx, paysage: bool, w: f32) -> (Vec<Rect>, f32) {
    let d = cx.dp;
    let h = (cx.theme.ligne * d).round();
    let (icone, marge, interieur) = ((24.0 * d).round(), (14.0 * d).round(), (12.0 * d).round());
    let textes: Vec<f32> = GENRES.iter().map(|g| cx.measure(g.nom, 16.0)).collect();
    if paysage {
        let large = (marge + icone + marge + textes.iter().fold(0.0f32, |a, &b| a.max(b)) + marge).ceil();
        let large = large.max(zone_liens(cx, true).0.ceil());
        let haut = (8.0 * d).round();
        ((0..GENRES.len()).map(|i| Rect::new(0.0, haut + i as f32 * h, large, h)).collect(), large)
    } else {
        let (ecart, bord) = ((8.0 * d).round(), (12.0 * d).round());
        let (mut x, mut y, mut places) = (bord, bord, Vec::new());
        for t in textes {
            let tw = (interieur + icone + 10.0 * d + t + interieur + 4.0 * d).ceil().min(w - 2.0 * bord);
            if x > bord && x + tw > w - bord {
                x = bord;
                y += h + ecart;
            }
            places.push(Rect::new(x, y, tw, h));
            x += tw + ecart;
        }
        (places, y + h + bord + zone_liens(cx, false).1)
    }
}

impl App {
    pub fn new(dp: f32) -> App {
        App {
            shell: Shell::new(dp),
            commun: Commun::default(),
            surface: Surface::new(0),
            fenetres: HashMap::new(),
            heure: None,
            end_content: EndContent::Parametres,
            onglet_parametres: 0,
            essai: Edition::default(),
            applets: crate::applet::Applets::default(),
            zones_lanceur: Vec::new(),
            zones_liens: Vec::new(),
        }
    }

    /// Vrai s'il faut redessiner sans attendre un événement.
    pub fn animating(&self) -> bool {
        self.shell.animating()
            || self.commun.clavier.anime()
            || crate::telecommande::anime()
            || self.applets.anime(&self.commun)
            || self.surface.placees().iter().any(|p| self.fenetres.get(&p.id).is_some_and(|i| i.anime(&self.commun)))
    }

    /// Une pastille ouvre son contenu, ou referme le panneau s'il l'affiche déjà.
    fn toggle_end(&mut self, content: EndContent) {
        if self.shell.end.open && self.end_content == content {
            self.shell.end.open = false;
        } else {
            self.end_content = content;
            self.shell.end.open = true;
        }
    }

    /// Les réglages de l'interface à enregistrer : l'apparence et le clavier.
    pub fn reglages(&self) -> Reglages {
        Reglages { theme: self.commun.theme, clavier: self.commun.clavier.reglages, gestes: self.commun.gestes, applets: self.commun.applets.clone(), veille: self.commun.veille }
    }

    /// Applique des réglages relus.
    pub fn appliquer(&mut self, r: Reglages) {
        self.commun.theme = r.theme;
        self.commun.clavier.reglages = r.clavier;
        self.commun.gestes = r.gestes;
        self.commun.applets = r.applets;
        self.commun.veille = r.veille;
    }

    /// L'espace de travail en texte : la disposition et l'état de chaque fenêtre.
    pub fn espace(&self) -> String {
        let fenetres = self.surface.ecrire_avec(&|v| contextes::genre(v).cle.to_string());
        let sections = self
            .surface
            .disposer(Rect::default())
            .into_iter()
            .map(|(f, _, _)| {
                let mut e = Etat::default();
                if let Some(i) = self.fenetres.get(&f) {
                    i.ecrire(&mut e);
                }
                (f, e)
            })
            .collect();
        Espace { fenetres: Some(fenetres), commun: Etat::default(), sections }.ecrire()
    }

    /// Rouvre un espace de travail écrit par `espace`. Une disposition illisible garde
    /// les fenêtres actuelles ; un contexte inconnu devient le contexte de repli.
    pub fn ouvrir_espace(&mut self, texte: &str) {
        let e = Espace::lire(texte);
        let repli = contextes::indice(contextes::REPLI).unwrap_or(0);
        let Some(s) = e.fenetres.as_deref().and_then(|t| Surface::lire_avec(t, &|n| Some(contextes::indice(n).unwrap_or(repli)))) else { return };
        self.surface = s;
        self.commun.libre = self.surface.libre();
        self.suspendre();
        self.fenetres.clear();
        for (f, vue, _) in self.surface.disposer(Rect::default()) {
            self.fenetres.insert(f, (contextes::genre(vue).ouvrir)(&e.section(f)));
        }
    }

    /// La racine du stockage, donnée par Android ; au retour dans l'application, les
    /// contextes relisent ce qui a pu changer sur le disque.
    pub fn racine_en(&mut self, racine: Option<PathBuf>) {
        self.commun.racine = racine;
        // Les dictionnaires de syntaxe ont pu être retouchés
        self.commun.syntaxes = None;
        for i in self.fenetres.values_mut() {
            i.retour(&self.commun);
        }
    }

    /// L'application passe en arrière-plan : chaque instance met à l'abri ce qui n'est
    /// pas enregistré (à appeler avant d'enregistrer l'espace).
    pub fn suspendre(&mut self) {
        for i in self.fenetres.values_mut() {
            i.mettre_a_l_abri(&self.commun);
        }
    }

    /// Traite une demande d'instance.
    fn traiter(&mut self, d: Demande) {
        match d {
            Demande::Ouvrir { depuis, cle, etat } => {
                // Un document déjà ouvert ne s'ouvre pas deux fois
                if let Some(doc) = etat.texte("document").map(Path::new) {
                    if self.fenetres.values().any(|i| i.cle() == cle && i.document() == Some(doc)) {
                        return;
                    }
                }
                let Some(vue) = contextes::indice(cle) else { return };
                if self.surface.vue(depuis).is_none() {
                    return;
                }
                // Le nouveau contexte remplace celui de la fenêtre qui demande ; l'ancienne
                // instance met à l'abri ce qu'elle n'a pas enregistré
                if let Some(mut ancienne) = self.fenetres.insert(depuis, (contextes::genre(vue).ouvrir)(&etat)) {
                    ancienne.mettre_a_l_abri(&self.commun);
                }
                self.surface.changer_vue(depuis, vue);
            }
            Demande::OuvrirACote { depuis, cle, etat } => {
                if let Some(doc) = etat.texte("document").map(Path::new) {
                    if self.fenetres.values().any(|i| i.cle() == cle && i.document() == Some(doc)) {
                        return;
                    }
                }
                let Some(vue) = contextes::indice(cle) else { return };
                let Some(p) = self.surface.placees().iter().find(|p| p.id == depuis).copied() else { return };
                let sens = if p.cadre.w >= p.cadre.h { crate::surface::Sens::Colonnes } else { crate::surface::Sens::Lignes };
                let Some(n) = self.surface.partager(depuis, sens, 0.5) else { return };
                self.fenetres.insert(n, (contextes::genre(vue).ouvrir)(&etat));
                self.surface.changer_vue(n, vue);
            }
            Demande::Montrer { cle } => {
                let Some(vue) = contextes::indice(cle) else { return };
                if self.surface.placees().iter().any(|p| p.vue == vue) {
                    return;
                }
                // La plus grande fenêtre se scinde dans son sens le plus long
                let Some(p) = self.surface.placees().iter().max_by(|a, b| (a.cadre.w * a.cadre.h).total_cmp(&(b.cadre.w * b.cadre.h))) else { return };
                let (f, sens) = (p.id, if p.cadre.w >= p.cadre.h { crate::surface::Sens::Colonnes } else { crate::surface::Sens::Lignes });
                self.surface.scinder(f, sens, vue);
            }
        }
    }

    /// (Re)lit l'historique des radios, rangé à côté des réglages.
    pub fn historique_en(&mut self, chemin: PathBuf) {
        self.commun.historique = crate::radio::Historique::ouvrir(chemin);
    }

    /// Les frappes du clavier vont au champ ciblé : celui d'essai du panneau des
    /// paramètres, ou celui de l'instance qui le possède.
    fn frappes(&mut self, cx: &mut Cx, zone: Rect, dt: f32) {
        let mut frappes = self.commun.clavier.saisir(cx, zone, dt);
        // Et celles d'une télécommande
        frappes.extend(crate::telecommande::frappes());
        for frappe in frappes {
            let Some(cible) = self.commun.clavier.cible else { continue };
            if cible == parametres::ESSAI {
                if self.essai.appliquer(frappe) {
                    self.commun.clavier.fermer();
                }
                continue;
            }
            for (f, i) in self.fenetres.iter_mut() {
                if i.frappe(*f, cible, frappe, &mut self.commun) {
                    break;
                }
            }
        }
    }

    /// Chaque fenêtre a une instance du contexte qu'elle montre : la fenêtre née d'un
    /// partage reçoit une copie, une fenêtre fermée perd la sienne, et une fenêtre qui
    /// change de contexte en reçoit une nouvelle.
    fn suivre_fenetres(&mut self, placees: &[Placee]) {
        for c in self.surface.changements() {
            match c {
                Changement::Partagee { de, nouvelle } if !self.fenetres.contains_key(&nouvelle) => {
                    if let Some(copie) = self.fenetres.get(&de).map(|i| i.dupliquer()) {
                        self.fenetres.insert(nouvelle, copie);
                    }
                }
                Changement::Partagee { .. } => {}
                Changement::Fermee(f) => {
                    if let Some(mut i) = self.fenetres.remove(&f) {
                        i.mettre_a_l_abri(&self.commun);
                    }
                }
                Changement::Vue { .. } | Changement::Nouvelle(_) => {}
            }
        }
        for p in placees {
            let g = contextes::genre(p.vue);
            if self.fenetres.get(&p.id).is_none_or(|i| i.cle() != g.cle) {
                if let Some(mut ancienne) = self.fenetres.insert(p.id, (g.ouvrir)(&Etat::default())) {
                    ancienne.mettre_a_l_abri(&self.commun);
                }
            }
        }
    }

    /// Déclare à l'orchestrateur les instances ouvertes, dans l'ordre des fenêtres.
    fn recenser(&mut self) {
        let o = &mut self.commun.orchestre;
        o.instances.clear();
        // Toutes les fenêtres, réduites comprises : leur contexte reste ouvert
        for (f, _, _) in self.surface.disposer(Rect::default()) {
            if let Some(i) = self.fenetres.get(&f) {
                o.instances.push(crate::orchestre::Instance { fenetre: f, cle: i.cle(), document: i.document().map(Path::to_path_buf) });
            }
        }
    }

    pub fn frame(&mut self, cx: &mut Cx, screen: Rect, dt: f32) -> Scene {
        cx.theme = self.commun.theme;
        cx.ui.reglages_gestes = self.commun.gestes;
        // La barre de l'application, tout en bas
        let hb = cx.commande().min(screen.h / 4.0);
        let barre = Rect::new(screen.x, screen.bottom() - hb, screen.w, hb);
        let screen = Rect::new(screen.x, screen.y, screen.w, screen.h - hb);
        let haut = screen;
        // Le clavier ouvert prend le bas de l'écran, ou flotte par-dessus ; ses frappes
        // vont au champ qui a le focus
        let (screen, zone_clavier) = self.commun.clavier.reserver(cx, screen);
        let flottant = self.commun.clavier.reglages.flottant;
        if !flottant {
            self.frappes(cx, zone_clavier, dt);
        }
        // Le lanceur prend la taille de sa liste
        self.shell.start.voulu = Some(places_lanceur(cx, screen.w >= screen.h, screen.w).1);
        let l = self.shell.layout(screen, dt);
        let vues = contextes::vues();
        self.surface.accueil = contextes::indice(contextes::accueil::CLE).unwrap_or(0);
        self.surface.mettre_libre(self.commun.libre);
        let placees = self.surface.frame(cx, l.workspace, &vues);
        let libre = self.surface.libre();
        self.suivre_fenetres(&placees);
        for p in &placees {
            if let Some(i) = self.fenetres.get_mut(&p.id) {
                i.avancer(dt, &self.commun);
            }
        }
        for p in &placees {
            if !self.fenetres.contains_key(&p.id) {
                continue;
            }
            // En fenêtres libres : le cadre et la barre juste avant le contenu, et tout
            // le dessin de la fenêtre restreint à son cadre
            self.surface.habiller(cx, p, &vues);
            if libre {
                cx.ui.socle = Some(p.cadre);
                cx.clip(None);
            }
            let Some(i) = self.fenetres.get_mut(&p.id) else { continue };
            let etat = i.dessiner(cx, &mut self.commun, p);
            // La ligne d'état, à droite du titre, en chasse fixe
            let (d, col, e) = (cx.dp, cx.couleurs(), p.etat);
            cx.clip(Some(e));
            cx.mono_at(e.x + 4.0 * d, e.cy(), &etat, 12.0, col.pale);
            cx.ui.socle = None;
            cx.clip(None);
        }
        // L'orchestrateur : le recensement des instances ouvertes, puis ses tâches
        // (empreintes, enregistrements)
        self.recenser();
        self.commun.orchestrer(cx.ui.doigts() == 0);
        for d in std::mem::take(&mut self.commun.demandes) {
            self.traiter(d);
        }
        self.surface.dessus(cx, &vues);
        // Le lanceur ouvert : en paysage, il descend jusqu'en bas de l'écran et la barre
        // se rabat à sa droite ; en portrait, il monte sous la barre et la pousse vers le
        // haut
        let barre = if l.landscape {
            let lanceur = (l.start.right() - screen.x).clamp(0.0, barre.w);
            Rect::new(barre.x + lanceur, barre.y, barre.w - lanceur, barre.h)
        } else {
            // Lanceur fermé, la barre reste en bas : son bord haut est celui de l'écran
            // réduit, qui avec le clavier ouvert est le haut du clavier (la barre
            // couvrait alors celle du clavier et son bouton Baisser)
            let monte = screen.bottom() - l.start.y > 0.5;
            Rect::new(barre.x, if monte { l.start.y.min(barre.y) } else { barre.y }, barre.w, barre.h)
        };
        self.barre(cx, barre);

        self.end_panel(cx, &l);
        self.lanceur(cx, &l, hb);
        self.handles(cx, &l, screen, hb);
        // Le volet d'un applet, par-dessus les fenêtres et les panneaux
        self.applets.volet(cx, &mut self.commun, haut);
        // Flottant, le clavier déclare ses touches en dernier : elles passent devant tout
        if flottant {
            self.frappes(cx, zone_clavier, dt);
        }
        self.commun.clavier.dessiner(cx);
        // Le retour visuel des gestes, par-dessus tout
        let apercus = cx.ui.gestes_actifs();
        crate::gestes::dessiner_retour(cx, &self.commun.gestes, &apercus);
        // Le curseur d'une télécommande en mode pavé, par-dessus le reste
        if let Some((x, y, tenu)) = crate::telecommande::curseur() {
            let d = cx.dp;
            let col = cx.couleurs();
            let (a, b) = ([x, y], [x + 7.0 * d, y + 17.0 * d]);
            let c = [x + 13.0 * d, y + 11.0 * d];
            for (p, q) in [(a, b), (b, c), (c, a)] {
                cx.dl.line(p, q, 4.5 * d, [0, 0, 0, 200]);
            }
            for (p, q) in [(a, b), (b, c), (c, a)] {
                cx.dl.line(p, q, 2.0 * d, if tenu { col.accent } else { [255, 255, 255, 255] });
            }
        }
        Scene { workspace: l.workspace, fond: cx.couleurs().fond }
    }

    /// La barre de l'application : pastilles aplaties à gauche (le logo, les
    /// paramètres, l'aide), à droite les applets et l'heure.
    fn barre(&mut self, cx: &mut Cx, r: Rect) {
        if r.h < 1.0 {
            return;
        }
        let (d, col) = (cx.dp, cx.couleurs());
        cx.ui.region_add(BARRE, r, crate::ui::Sense::Sink);
        cx.dl.rrect(r, 0.0, col.surface);
        cx.dl.rrect(Rect::new(r.x, r.y, r.w, d.max(1.0)), 0.0, col.ligne);
        if cx.pastille_plate(LOGO, Rect::new(r.x, r.y, r.h, r.h), Icon::Rustine, self.shell.start.open) {
            self.shell.start.toggle();
        }
        // À droite : l'heure tout au bord, puis les applets (la batterie en est un :
        // Énergie, de l'Orchestre)
        let mut x = r.right() - cx.pad();
        if let Some((h, m)) = self.heure {
            let s = format!("{h:02}:{m:02}");
            let w = cx.measure(&s, 15.0);
            x -= w;
            cx.text_at(x, r.cy(), &s, 15.0, col.texte);
            x -= cx.pad();
        }
        let mut debut = r.x + r.h + cx.pad();
        // En fenêtres libres, la barre des tâches : une case par fenêtre, à droite du
        // logo ; elle prend au plus la moitié de la place
        if self.surface.libre() {
            debut = self.taches(cx, Rect::new(debut, r.y, ((x - debut) / 2.0).max(0.0), r.h));
        }
        self.applets.barre(cx, &mut self.commun, Rect::new(debut, r.y, (x - debut).max(0.0), r.h));
    }

    /// La barre des tâches du Free flow : l'icône du contexte de chaque fenêtre, par
    /// ordre d'ouverture ; à l'accent celle de devant, pâle une fenêtre réduite, un
    /// trait sous celles qui se montrent. La toucher montre la fenêtre (et la passe
    /// devant), ou réduit celle de devant. Renvoie où la barre s'arrête.
    fn taches(&mut self, cx: &mut Cx, r: Rect) -> f32 {
        let (d, col) = (cx.dp, cx.couleurs());
        let mut l: Vec<Flottante> = self.surface.libres().to_vec();
        l.sort_by_key(|f| f.id);
        if l.is_empty() || r.w < r.h * 0.6 {
            return r.x;
        }
        let w = (r.w / l.len() as f32).clamp(r.h * 0.6, r.h);
        let devant = self.surface.devant();
        let vues = contextes::vues();
        let mut x = r.x;
        let mut touchee = None;
        for f in &l {
            if x + w > r.right() + 0.5 {
                break;
            }
            let case = Rect::new(x, r.y, w, r.h);
            let k = sub_id(TACHES, f.id as usize);
            if cx.ui.click(k, case, None) {
                touchee = Some(f.id);
            }
            if cx.ui.pressed(k) {
                cx.dl.rrect(case.inset(3.0 * d), cx.theme.rayon(case.h - 6.0 * d), col.puce);
            }
            let reduite = f.etendue == Etendue::Reduite;
            let teinte = if Some(f.id) == devant { col.accent } else if reduite { col.ligne } else { col.texte };
            let icone = vues.get(f.vue as usize).map_or(Icon::Help, |v| v.1);
            let ic = (r.h * 0.5).min(w * 0.7);
            cx.icon(Rect::new(case.cx() - ic / 2.0, case.cy() - ic / 2.0, ic, ic), icone, teinte);
            if !reduite {
                let tw = (w * 0.4).round();
                cx.dl.rrect(Rect::new(case.cx() - tw / 2.0, case.bottom() - 6.0 * d, tw, 2.5 * d), 1.25 * d, teinte);
            }
            x += w;
        }
        if let Some(f) = touchee {
            self.surface.basculer(f);
        }
        x + cx.pad()
    }

    fn panel_frame(&self, cx: &mut Cx, id: Id, r: Rect, side: Side, landscape: bool) {
        cx.ui.region_add(id, r, crate::ui::Sense::Sink);
        let col = cx.couleurs();
        cx.dl.rrect(r, 0.0, col.surface);
        let t = cx.dp.max(1.0);
        let line = match (side, landscape) {
            (Side::Start, true) => Rect::new(r.right() - t, r.y, t, r.h),
            (Side::End, true) => Rect::new(r.x, r.y, t, r.h),
            (Side::Start, false) => Rect::new(r.x, r.bottom() - t, r.w, t),
            (Side::End, false) => Rect::new(r.x, r.y, r.w, t),
        };
        cx.dl.rrect(line, 0.0, col.ligne);
    }

    /// En-tête d'un panneau : renvoie le rectangle du corps, et vrai si ✕ a été touché.
    fn panel_header(&self, cx: &mut Cx, id_close: Id, r: Rect) -> (Rect, Rect, bool) {
        let (d, col, h) = (cx.dp, cx.couleurs(), cx.commande());
        let head = h + 12.0 * d;
        let close = Rect::new(r.right() - h - 6.0 * d, r.y + 6.0 * d, h, h);
        let clicked = cx.ui.click(id_close, close, None);
        cx.dl.rrect(close, cx.theme.rayon(h), col.puce);
        cx.icon(close.inset(h * 0.25), Icon::Close, col.texte);
        cx.dl.rrect(Rect::new(r.x, r.y + head - d, r.w, d.max(1.0)), 0.0, col.ligne);
        // Les onglets partent de (x, y) ; le clip les garde à gauche du ✕
        (Rect::new(r.x + 12.0 * d, r.y + 6.0 * d, r.w - h - 30.0 * d, h), Rect::new(r.x, r.y + head, r.w, r.h - head), clicked)
    }

    fn end_panel(&mut self, cx: &mut Cx, l: &Layout) {
        if self.shell.end.anim == 0.0 {
            return;
        }
        let r = l.end;
        self.panel_frame(cx, id("panneau-fin"), r, Side::End, l.landscape);
        let (tabs, body, close) = self.panel_header(cx, id("fermer-fin"), r);
        if close {
            self.shell.end.open = false;
        }
        let c = &mut self.commun;
        match self.end_content {
            EndContent::Parametres => {
                self.onglet_parametres = cx.tabs(id("onglets-parametres"), tabs.x, tabs.y, &parametres::ONGLETS, self.onglet_parametres);
                let scroll = sub_id(id("corps-parametres"), self.onglet_parametres);
                let mut b = cx.body(scroll, body, &c.scrolls);
                parametres::panneau(cx, &mut b, scroll, self.onglet_parametres, c, &mut self.essai);
                cx.body_end(b, &mut c.scrolls);
            }
            EndContent::APropos => {
                if cx.tabs(id("onglets-aide"), tabs.x, tabs.y, &["Aide", "À propos"], 1) == 0 {
                    self.end_content = EndContent::Help;
                }
                let mut b = cx.body(id("corps-a-propos"), body, &c.scrolls);
                cx.section(&mut b, &format!("FLOW {}", env!("CARGO_PKG_VERSION")));
                cx.paragraph(&mut b, "Une application d'exploitation : des contextes (sous-applications) à disposer dans les fenêtres, pour composer son espace de travail.");
                cx.paragraph(&mut b, &format!("{} contextes : {}.", GENRES.len(), GENRES.iter().map(|g| g.nom).collect::<Vec<_>>().join(", ")));
                cx.paragraph(&mut b, "Réutilisable à des fins non commerciales, selon la licence PolyForm Noncommercial 1.0.0.");
                cx.section(&mut b, "BARRE");
                cx.paragraph(&mut b, "Le centre du bas de l'écran est réservé aux gestes du système, mais les appuis y restent libres : la barre de l'application y range le logo (il ouvre la liste des contextes, avec en bas les liens Paramètres et À propos), et les applets des contextes : la radio, le mixeur, l'avancement d'une copie de fichiers… Toucher un applet ouvre son volet ; les applets montrés se choisissent dans les paramètres (Interface, section Barre).");
                cx.body_end(b, &mut c.scrolls);
            }
            EndContent::Help => {
                if cx.tabs(id("onglets-aide"), tabs.x, tabs.y, &["Aide", "À propos"], 0) == 1 {
                    self.end_content = EndContent::APropos;
                }
                let mut b = cx.body(id("corps-aide"), body, &c.scrolls);
                cx.section(&mut b, "FENÊTRES");
                cx.paragraph(&mut b, "Touchez le titre d'une fenêtre pour choisir son contexte, la partager en deux ou la fermer.");
                cx.paragraph(&mut b, "En Free flow (Paramètres, Interface), les fenêtres flottent comme sur un bureau : tirez la barre d'une fenêtre pour la déplacer, sa poignée pour la redimensionner ; touchez-la pour la passer devant. Un contexte lancé s'ouvre dans une nouvelle fenêtre, posée en cascade. La barre des tâches, à droite du logo, montre chaque fenêtre : la toucher montre une fenêtre réduite ou cachée et la passe devant, ou réduit celle de devant.");
                cx.section(&mut b, "PANNEAU");
                cx.paragraph(&mut b, "Le logo de la barre du bas ouvre la liste des contextes (à gauche en paysage, sous la barre en portrait) : en toucher un l'ajoute à l'espace de travail, à gauche en paysage, en bas en portrait, et referme la liste.");
                cx.paragraph(&mut b, "Les liens Paramètres et À propos, en bas de la liste des contextes, ouvrent ce panneau. L'espace de travail s'adapte à la place restante.");
                cx.paragraph(&mut b, "Glissez la frontière du panneau pour le redimensionner ; un double appui rétablit sa taille.");
                cx.body_end(b, &mut c.scrolls);
            }
        }
    }

    /// Le lanceur, à gauche, de haut en bas de l'écran : la liste des contextes, sur un fond noir (thème sombre) ou
    /// blanc. Toucher un contexte l'ajoute à l'espace et referme le lanceur.
    fn lanceur(&mut self, cx: &mut Cx, l: &Layout, hb: f32) {
        self.zones_lanceur.clear();
        if self.shell.start.anim == 0.0 {
            return;
        }
        // En paysage, sur toute la hauteur, à côté de la barre rabattue ; en portrait,
        // sous la barre poussée vers le haut
        let r = if l.landscape { Rect::new(l.start.x, l.start.y, l.start.w, l.start.h + hb) } else { Rect::new(l.start.x, l.start.y + hb, l.start.w, l.start.h) };
        let (d, col) = (cx.dp, cx.couleurs());
        cx.ui.region_add(LANCEUR, r, crate::ui::Sense::Sink);
        cx.dl.rrect(r, 0.0, if cx.theme.sombre { [0, 0, 0, 255] } else { [255, 255, 255, 255] });
        if l.landscape {
            cx.dl.rrect(Rect::new(r.right() - d.max(1.0), r.y, d.max(1.0), r.h), 0.0, col.ligne);
        }
        // En bas, les liens : à droite, empilés en paysage, sur une ligne en portrait
        let (_, hz) = zone_liens(cx, l.landscape);
        let (hl, m) = mesures_liens(cx);
        let zone = Rect::new(r.x, (r.bottom() - hz).max(r.y), r.w, hz.min(r.h));
        self.zones_liens.clear();
        let mut lien = None;
        let mut x = zone.right() - m;
        for (i, (nom, contenu)) in LIENS.iter().enumerate().rev() {
            let w = cx.measure(nom, 15.0) + 2.0 * m;
            let z = if l.landscape {
                Rect::new(zone.right() - m - w, zone.y + m + i as f32 * hl, w, hl)
            } else {
                x -= w;
                Rect::new(x, zone.y, w, hl)
            };
            let lid = sub_id(id("lanceur-lien"), i);
            if cx.ui.click(lid, z, None) {
                lien = Some(*contenu);
            }
            let actif = self.shell.end.open && (self.end_content == *contenu || (*contenu == EndContent::APropos && self.end_content == EndContent::Help));
            if cx.ui.pressed(lid) || actif {
                cx.dl.rrect(z, cx.theme.rayon(hl) * 0.5, col.puce);
            }
            cx.text_right(z.right() - m, z.cy(), nom, 15.0, col.accent);
            self.zones_liens.insert(0, z);
        }
        let liste = Rect::new(r.x, r.y, r.w, (zone.y - r.y).max(0.0));
        let c = &mut self.commun;
        let corps = id("corps-lanceur");
        let (places, _) = places_lanceur(cx, l.landscape, r.w);
        let mut b = cx.body(corps, liste, &c.scrolls);
        // Le haut du contenu, défilement compris
        let haut = b.y - (8.0 * d).round();
        let mut choisi = None;
        let s = (24.0 * d).round();
        for (i, (g, p)) in GENRES.iter().zip(places).enumerate() {
            let lid = sub_id(LANCEUR_LIGNE, i);
            let presse = cx.ui.pressed(lid);
            let (z, x) = if l.landscape {
                // Une ligne bord à bord ; la marge avant l'icône
                let z = Rect::new(r.x, haut + p.y, r.w, p.h);
                if presse {
                    cx.dl.rrect(z.inset(2.0 * d), cx.theme.rayon(z.h) * 0.45, col.puce);
                }
                (z, z.x + (14.0 * d).round())
            } else {
                // Une tuile
                let z = Rect::new(r.x + p.x, haut + p.y, p.w, p.h);
                cx.dl.rrect(z, cx.theme.rayon(z.h) * 0.6, if presse { col.ligne } else { col.puce });
                (z, z.x + (12.0 * d).round())
            };
            if cx.ui.click(lid, z, Some(corps)) {
                choisi = Some(i as u32);
            }
            self.zones_lanceur.push(z);
            cx.icon(Rect::new(x, z.cy() - s / 2.0, s, s), g.icone, col.accent);
            cx.clip(Some(z.intersect(&liste)));
            cx.text_at(x + s + 10.0 * d, z.cy(), g.nom, 16.0, col.texte);
            cx.clip(Some(liste));
        }
        // La hauteur du contenu, pour le défilement
        b.y = self.zones_lanceur.last().map_or(haut, |z| z.bottom());
        cx.body_end(b, &mut c.scrolls);
        if let Some(vue) = choisi {
            self.lancer(vue, l.landscape);
        }
        // Un lien ouvre son panneau (ou le referme) ; le lanceur se referme
        if let Some(contenu) = lien {
            self.shell.start.open = false;
            let deja = self.shell.end.open && (self.end_content == contenu || (contenu == EndContent::APropos && self.end_content == EndContent::Help));
            if deja {
                self.shell.end.open = false;
            } else {
                self.toggle_end(contenu);
            }
        }
    }

    /// Ajoute le contexte `vue` à l'espace de travail : à la place d'une fenêtre
    /// d'Accueil s'il y en a une, sinon dans une fenêtre insérée à gauche (paysage) ou
    /// en bas (portrait). Le lanceur se referme en même temps.
    fn lancer(&mut self, vue: u32, paysage: bool) {
        self.shell.start.open = false;
        let accueil = contextes::indice(contextes::accueil::CLE);
        if let Some(f) = self.surface.placees().iter().find(|p| Some(p.vue) == accueil && Some(vue) != accueil).map(|p| p.id) {
            self.surface.changer_vue(f, vue);
        } else if paysage {
            self.surface.inserer(crate::surface::Sens::Colonnes, true, vue);
        } else {
            self.surface.inserer(crate::surface::Sens::Lignes, false, vue);
        }
    }

    /// Poignées de redimensionnement, sur la frontière de chaque panneau ouvert.
    fn handles(&mut self, cx: &mut Cx, l: &Layout, screen: Rect, hb: f32) {
        let d = cx.dp;
        for (side, key, shown) in [(Side::Start, id("poignee-debut"), self.shell.start.anim), (Side::End, id("poignee-fin"), self.shell.end.anim)] {
            if shown < 1.0 {
                continue;
            }
            // En portrait, le lanceur est sous la barre : sa poignée aussi
            let decale = if side == Side::Start && !l.landscape { hb } else { 0.0 };
            let mut band = self.shell.handle(l, side);
            band.y += decale;
            if let Some(drag) = cx.ui.drag(key, band) {
                self.shell.resize(side, screen, drag.x, drag.y - decale);
            }
            if cx.ui.double_tapped(key) {
                self.shell.reset_size(side, screen);
            }
            let pressed = cx.ui.pressed(key);
            let color = if pressed { cx.couleurs().accent } else { [0x6C, 0x6C, 0x80, 140] };
            let (long, thick) = (52.0 * d, 5.0 * d);
            let grip = if l.landscape {
                Rect::new(band.cx() - thick / 2.0, band.cy() - long / 2.0, thick, long)
            } else {
                Rect::new(band.cx() - long / 2.0, band.cy() - thick / 2.0, long, thick)
            };
            cx.dl.rrect(grip, thick / 2.0, color);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draw::DrawList;
    use crate::text::Text;
    use crate::ui::{Event, Ui};

    const SCREEN: Rect = Rect::new(0., 0., 1920., 1200.);

    struct Rig {
        app: App,
        ui: Ui,
        text: Text,
        dl: DrawList,
        ecran: Rect,
    }

    impl Rig {
        fn new() -> Rig {
            let mut r = Rig { app: App::new(1.5), ui: Ui::new(1.5), text: Text::new(), dl: DrawList::default(), ecran: SCREEN };
            r.frames(1);
            r
        }
        fn frame(&mut self, dt: f32) -> Scene {
            self.dl.clear();
            let mut cx = Cx { ui: &mut self.ui, text: &mut self.text, dl: &mut self.dl, dp: 1.5, theme: Default::default(), palette: None };
            let s = self.app.frame(&mut cx, self.ecran, dt);
            self.ui.end_frame();
            s
        }
        fn frames(&mut self, n: usize) -> Scene {
            let mut s = self.frame(0.05);
            for _ in 1..n {
                s = self.frame(0.05);
            }
            s
        }
        fn tap(&mut self, x: f32, y: f32) {
            self.ui.event(Event::Down { id: 0, x, y }, 0.0);
            self.ui.event(Event::Up { id: 0 }, 0.05);
            self.frames(1);
        }
    }

    fn vue(cle: &str) -> u32 {
        contextes::indice(cle).unwrap()
    }

    // Centre de la n-ième pastille de la barre du bas : 0 le logo.
    fn pastille(n: f32) -> (f32, f32) {
        (72.0 * (n + 0.5), 1200.0 - 36.0)
    }

    // Ouvre le lanceur par le logo, et touche son lien `n` : 0 Paramètres, 1 À propos.
    fn lien(r: &mut Rig, n: usize) {
        let (x, _) = pastille(0.0);
        r.tap(x, r.ecran.h - 36.0);
        r.frames(60);
        let z = r.app.zones_liens[n];
        r.tap(z.cx(), z.cy());
        assert!(!r.app.shell.start.open, "le lien referme le lanceur");
    }

    #[test]
    fn la_taille_des_commandes_suit_le_theme() {
        let mut r = Rig::new();
        r.app.commun.theme.commande = 60.0;
        r.frames(1);
        // La pastille du logo fait 60 dp : la barre est plus haute
        r.tap(45.0 * 1.5, 1200.0 - 45.0);
        assert!(r.app.shell.start.open);
        assert_eq!(r.frames(60).workspace.bottom(), 1200.0 - 90.0);
    }

    #[test]
    fn le_lien_des_parametres_deploie_le_panneau_et_l_espace_s_adapte() {
        let mut r = Rig::new();
        r.frames(1);
        // Les liens, en bas à droite du lanceur, empilés
        let (x, _) = pastille(0.0);
        r.tap(x, 1200.0 - 36.0);
        r.frames(60);
        let (a, b) = (r.app.zones_liens[0], r.app.zones_liens[1]);
        let lanceur = r.app.zones_lanceur[0].right();
        assert!(a.bottom() <= b.y && b.bottom() > 1100.0 && (a.right() - b.right()).abs() < 0.5 && a.right() < lanceur && a.right() > lanceur - 30.0, "{a:?} {b:?}");
        r.tap(x, 1200.0 - 36.0);
        r.frames(60);
        lien(&mut r, 0);
        assert!(r.app.shell.end.open && r.app.end_content == EndContent::Parametres);
        let s = r.frames(60);
        assert_eq!(s.workspace.w, 1920.0 - 576.0);
        assert!(!r.app.animating());
        lien(&mut r, 0); // nouvel appui : referme
        assert!(!r.app.shell.end.open);
        assert_eq!(r.frames(60).workspace, Rect::new(0., 0., 1920., 1200. - 72.));
    }

    #[test]
    fn en_portrait_les_liens_sont_sur_une_ligne_sous_les_tuiles() {
        let mut r = Rig::new();
        r.ecran = Rect::new(0., 0., 1200., 1920.);
        r.frames(1);
        let (x, _) = pastille(0.0);
        r.tap(x, 1920.0 - 36.0);
        r.frames(60);
        let (a, b) = (r.app.zones_liens[0], r.app.zones_liens[1]);
        let tuiles = r.app.zones_lanceur.iter().fold(0.0f32, |m, z| m.max(z.bottom()));
        assert!(a.y == b.y && a.right() <= b.x && b.right() > 1150.0 && a.y >= tuiles && b.bottom() <= 1920.0, "{a:?} {b:?} {tuiles}");
    }

    // Ouvre le lanceur par le logo, et touche la ligne du contexte `cle`.
    fn lancer(r: &mut Rig, cle: &str) {
        let (x, _) = pastille(0.0);
        r.tap(x, r.ecran.h - 36.0);
        assert!(r.app.shell.start.open);
        let s = r.frames(60);
        let paysage = r.ecran.w >= r.ecran.h;
        let zones = r.app.zones_lanceur.clone();
        assert_eq!(zones.len(), GENRES.len());
        // Paysage : le lanceur, large de sa liste, pousse l'espace et la barre se rabat à
        // sa droite ; portrait : des tuiles sous la barre, qui monte d'autant
        let logo = if paysage {
            assert!(s.workspace.x > 150.0 && s.workspace.x < 400.0, "{:?}", s.workspace);
            assert!(zones.iter().all(|z| z.x == 0.0 && (z.right() - s.workspace.x).abs() < 1.0));
            (s.workspace.x + x, r.ecran.h - 36.0)
        } else {
            let bas = zones.iter().fold(0.0f32, |a, z| a.max(z.bottom()));
            assert!(zones.iter().any(|z| z.x > 0.0 && z.y == zones[0].y), "plusieurs tuiles par ligne");
            assert!((r.ecran.h - bas - 18.0 - 69.0).abs() < 1.0, "la hauteur suit les tuiles et la ligne des liens");
            (x, zones[0].y - 18.0 - 36.0)
        };
        r.tap(logo.0, logo.1);
        assert!(!r.app.shell.start.open, "le logo déplacé referme le lanceur");
        r.frames(60);
        r.tap(x, r.ecran.h - 36.0);
        r.frames(60);
        let z = r.app.zones_lanceur[vue(cle) as usize];
        r.tap(z.cx(), z.cy());
        assert!(!r.app.shell.start.open, "le lanceur se referme");
        r.frames(60);
    }

    #[test]
    fn le_lanceur_insere_a_gauche_en_paysage() {
        let mut r = Rig::new();
        r.app.surface = Surface::new(vue("texte"));
        r.frames(1);
        lancer(&mut r, "fichiers");
        let p = r.app.surface.placees().to_vec();
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].vue, vue("fichiers"));
        assert!(p[0].cadre.x < p[1].cadre.x && (p[0].cadre.w - p[1].cadre.w).abs() < 2.0);
        // Une troisième prend un tiers
        lancer(&mut r, "radio");
        let p = r.app.surface.placees().to_vec();
        assert_eq!((p.len(), p[0].vue), (3, vue("radio")));
        assert!((p[0].cadre.w - 1920.0 / 3.0).abs() < 2.0, "{:?}", p[0].cadre);
    }

    #[test]
    fn le_lanceur_insere_en_bas_en_portrait() {
        let mut r = Rig::new();
        r.ecran = Rect::new(0., 0., 1200., 1920.);
        r.app.surface = Surface::new(vue("texte"));
        r.frames(1);
        lancer(&mut r, "orchestre");
        let p = r.app.surface.placees().to_vec();
        assert_eq!((p.len(), p[1].vue), (2, vue("orchestre")));
        assert!(p[1].cadre.y > p[0].cadre.y && p[1].cadre.x == p[0].cadre.x);
    }

    #[test]
    fn le_lanceur_remplace_l_accueil() {
        let mut r = Rig::new();
        r.app.surface = Surface::new(vue(contextes::accueil::CLE));
        r.frames(1);
        lancer(&mut r, "texte");
        let p = r.app.surface.placees().to_vec();
        assert_eq!((p.len(), p[0].vue), (1, vue("texte")));
        assert_eq!(r.app.fenetres[&p[0].id].cle(), "texte");
    }

    #[test]
    fn le_lien_a_propos_remplace_le_contenu_sans_fermer() {
        let mut r = Rig::new();
        lien(&mut r, 0);
        r.frames(60);
        lien(&mut r, 1);
        assert!(r.app.shell.end.open && r.app.end_content == EndContent::APropos);
    }

    #[test]
    fn la_poignee_redimensionne_dans_l_application() {
        let mut r = Rig::new();
        r.app.surface.partager(1, crate::surface::Sens::Colonnes, 0.5);
        r.frames(1);
        // La poignée de la fenêtre de gauche : 54 px de côté, dans son coin bas droit, au-dessus de la barre de l'application
        r.ui.event(Event::Down { id: 0, x: 933., y: 1101. }, 0.0);
        r.frames(1);
        for k in 1..=10 {
            r.ui.event(Event::Move { id: 0, x: 933. + 30. * k as f32, y: 1101. }, 0.1);
            r.frames(1);
        }
        r.ui.event(Event::Up { id: 0 }, 0.2);
        r.frames(2);
        let w = r.app.surface.placees()[0].cadre.w;
        assert!((w - 1260.).abs() <= 1.0, "{w}");
    }

    #[test]
    fn chaque_fenetre_a_sa_propre_instance() {
        let mut r = Rig::new();
        r.app.ouvrir_espace("fenetres = C0.5000(F2:fichiers,F1:fichiers)\n[fenetre 2]\ndossier = /tmp\n[fenetre 1]\ndossier = /\n");
        r.frames(1);
        let t = r.app.espace();
        assert!(t.contains("[fenetre 2]\ndossier = /tmp\n") && t.contains("[fenetre 1]\ndossier = /\n"), "{t}");
        // Une fenêtre qui change de contexte change d'instance
        r.app.surface.changer_vue(2, vue("aide"));
        r.frames(1);
        assert_eq!(r.app.fenetres[&2].cle(), "aide");
        assert_eq!(r.app.fenetres[&1].cle(), "fichiers");
        // Fermée, elle n'en a plus
        r.app.surface.fermer(2);
        r.frames(1);
        assert!(!r.app.fenetres.contains_key(&2));
    }

    #[test]
    fn les_fenetres_libres_se_retrouvent_et_se_rangent() {
        let texte = "fenetres = libre(F1:fichiers 0.0500 0.0500 0.5000 0.6000;F2:aide 0.3000 0.3000 0.6000 0.6000)\n";
        let mut r = Rig::new();
        r.app.ouvrir_espace(texte);
        r.frames(2);
        assert!(r.app.commun.libre && r.app.surface.nombre() == 2);
        assert!(r.app.espace().contains("\nfenetres = libre(F1:fichiers 0.0500 0.0500 0.5000 0.6000;F2:aide"), "{}", r.app.espace());
        // Le réglage revient au pavage : les deux fenêtres restent, côte à côte
        r.app.commun.libre = false;
        r.frames(1);
        assert!(r.app.espace().contains("\nfenetres = C0.5000(F1:fichiers,F2:aide)\n"), "{}", r.app.espace());
        assert_eq!(r.app.fenetres[&2].cle(), "aide");
    }

    #[test]
    fn l_espace_de_travail_se_retrouve() {
        let texte = "fenetres = C0.4000(F3:fichiers,L0.5000(F2:radio,F1:disque))\n\
                     [fenetre 3]\ndossier = /tmp\ncaches = oui\n\
                     [fenetre 2]\nadresse = https://exemple.fr/flux\n\
                     [fenetre 1]\ncentre = Musique/Albums\n";
        let mut r = Rig::new();
        r.app.ouvrir_espace(texte);
        assert_eq!(r.app.surface.nombre(), 3);
        r.frames(1);
        let t = r.app.espace();
        assert!(t.contains("[fenetre 3]\ndossier = /tmp\ncaches = oui\n"), "{t}");
        assert!(t.contains("[fenetre 2]\nadresse = https://exemple.fr/flux\n"), "{t}");
        assert!(t.contains("[fenetre 1]\ncentre = Musique/Albums\n"), "{t}");
        // Une autre application qui relit ce texte écrit la même chose
        let mut autre = Rig::new();
        autre.app.ouvrir_espace(&t);
        autre.frames(1);
        assert_eq!(autre.app.espace(), t);
        // Un contexte inconnu (le cube d'une version précédente) devient le repli, une
        // disposition illisible ne change rien
        autre.app.ouvrir_espace("fenetres = F1:cube\n");
        assert_eq!(autre.app.surface.ecrire_avec(&|v| contextes::genre(v).cle.into()), format!("F1:{}", contextes::REPLI));
        autre.app.ouvrir_espace("fenetres = C0.5(F1:aide\n");
        assert_eq!(autre.app.surface.nombre(), 1);
    }

    #[test]
    fn un_partage_copie_l_etat_de_la_fenetre() {
        let mut r = Rig::new();
        r.app.ouvrir_espace("fenetres = F1:fichiers\n[fenetre 1]\ndossier = /tmp\n");
        r.frames(1);
        r.app.surface.partager(1, crate::surface::Sens::Colonnes, 0.5);
        r.frames(1);
        let t = r.app.espace();
        assert!(t.contains("[fenetre 2]\ndossier = /tmp\n"), "{t}");
    }

    #[test]
    fn un_fichier_texte_remplace_l_explorateur_une_seule_fois() {
        let mut r = Rig::new();
        r.app.ouvrir_espace("fenetres = F1:fichiers\n");
        r.app.surface.partager(1, crate::surface::Sens::Colonnes, 0.5);
        r.frames(1);
        let mut etat = Etat::default();
        etat.mettre("document", "/tmp/a.txt");
        r.app.commun.demandes.push(Demande::Ouvrir { depuis: 1, cle: "texte", etat: etat.clone() });
        r.frames(2);
        // La fenêtre de l'explorateur devient l'éditeur, sur le document ; rien d'autre ne bouge
        assert_eq!(r.app.surface.nombre(), 2);
        assert_eq!(r.app.surface.vue(1), Some(vue("texte")));
        assert_eq!(r.app.fenetres[&1].cle(), "texte");
        assert!(r.app.espace().contains("[fenetre 1]\ndocument = /tmp/a.txt\n"));
        // Depuis l'autre explorateur, le même document ne s'ouvre pas une seconde fois
        r.app.commun.demandes.push(Demande::Ouvrir { depuis: 2, cle: "texte", etat });
        r.frames(2);
        assert_eq!(r.app.surface.vue(2), Some(vue("fichiers")));
    }

    #[test]
    fn une_carte_de_l_accueil_ouvre_son_contexte_et_le_menu_scinde_sur_l_accueil() {
        let mut r = Rig::new();
        r.frames(1);
        assert_eq!(r.app.fenetres[&1].cle(), "accueil");
        r.app.commun.demandes.push(Demande::Ouvrir { depuis: 1, cle: "encre", etat: Etat::default() });
        r.frames(1);
        assert_eq!(r.app.fenetres[&1].cle(), "encre");
        let n = r.app.surface.scinder(1, crate::surface::Sens::Lignes, vue("accueil")).unwrap();
        r.frames(1);
        assert_eq!(r.app.fenetres[&n].cle(), "accueil");
        assert_eq!(r.app.fenetres[&1].cle(), "encre");
    }

    #[test]
    fn rien_ne_redessine_au_repos() {
        let mut r = Rig::new();
        r.frames(3);
        assert!(!r.app.animating());
    }

    #[test]
    fn un_applet_ouvre_son_volet_et_un_toucher_dehors_le_referme() {
        let mut r = Rig::new();
        // Fichiers ne se montre que pendant une opération
        let zones = r.app.applets.zones().to_vec();
        assert_eq!(zones.iter().map(|(k, _)| *k).collect::<Vec<_>>(), ["boombox", "mixeur"]);
        let z = zones[1].1;
        assert!(z.right() <= 1920.0 && z.y == 1200.0 - 72.0, "à droite, dans la barre");
        r.tap(z.cx(), z.cy());
        assert_eq!(r.app.applets.ouvert, Some("mixeur"));
        // Un toucher dans le volet le garde ouvert, un toucher ailleurs le referme
        r.tap(z.cx().min(1920.0 - 200.0), 1200.0 - 72.0 - 40.0);
        assert_eq!(r.app.applets.ouvert, Some("mixeur"));
        r.tap(400.0, 300.0);
        assert_eq!(r.app.applets.ouvert, None);
        // Toucher l'applet ouvre, le retoucher referme
        r.tap(z.cx(), z.cy());
        r.tap(z.cx(), z.cy());
        assert_eq!(r.app.applets.ouvert, None);
    }

    #[test]
    fn les_applets_retires_quittent_la_barre() {
        let mut r = Rig::new();
        crate::applet::montrer(&mut r.app.commun, "boombox", false);
        r.frames(1);
        assert_eq!(r.app.applets.zones().iter().map(|(k, _)| *k).collect::<Vec<_>>(), ["mixeur"]);
        assert_eq!(r.app.reglages().applets, ["fichiers", "mixeur", "orchestre"]);
        crate::applet::montrer(&mut r.app.commun, "boombox", true);
        assert_eq!(r.app.commun.applets, ["fichiers", "boombox", "mixeur", "orchestre"]);
    }

    #[test]
    fn un_applet_montre_son_contexte_dans_une_fenetre_scindee() {
        let mut r = Rig::new();
        assert_eq!(r.app.surface.nombre(), 1);
        r.app.commun.demandes.push(Demande::Montrer { cle: "mixeur" });
        r.frames(2);
        assert_eq!(r.app.surface.nombre(), 2);
        let p = r.app.surface.placees().iter().find(|p| p.vue == vue("mixeur")).unwrap().clone();
        assert!(r.app.fenetres.get(&p.id).is_some_and(|i| i.cle() == "mixeur"));
        // Déjà montré : rien ne change
        r.app.commun.demandes.push(Demande::Montrer { cle: "mixeur" });
        r.frames(2);
        assert_eq!(r.app.surface.nombre(), 2);
    }

    #[test]
    fn l_applet_energie_montre_le_niveau_ou_le_temps_restant() {
        use crate::orchestre::AffichageBatterie;
        let mut r = Rig::new();
        // Sans relevé (hors d'Android), pas de batterie dans la barre
        assert!(!r.app.applets.zones().iter().any(|(k, _)| *k == "orchestre"));
        // Une heure de décharge, de 80 à 70 % : 10 %/h, vide dans 7 h
        r.app.commun.orchestre.relever(80, false, 1_000_000);
        r.app.commun.orchestre.relever(70, false, 1_003_600);
        r.frames(1);
        let zones = r.app.applets.zones().to_vec();
        assert_eq!(zones.last().map(|(k, _)| *k), Some("orchestre"), "le plus à droite, près de l'heure");
        let large = |r: &Rig| r.app.applets.zones().iter().find(|(k, _)| *k == "orchestre").unwrap().1.w;
        let pourcentage = large(&r);
        r.app.commun.orchestre.reglages.affichage = AffichageBatterie::LesDeux;
        r.frames(1);
        assert!(large(&r) > pourcentage, "« 70 % · 7 h 00 » est plus long que « 70 % »");
        assert_eq!(super::contextes::orchestre::texte_energie(&r.app.commun).as_deref(), Some("70 % · 7 h 00"));
        r.app.commun.orchestre.reglages.affichage = AffichageBatterie::Temps;
        assert_eq!(super::contextes::orchestre::texte_energie(&r.app.commun).as_deref(), Some("7 h 00"));
        // En charge sans assez de recul : le niveau remplace le temps
        r.app.commun.orchestre.relever(71, true, 1_003_700);
        assert_eq!(super::contextes::orchestre::texte_energie(&r.app.commun).as_deref(), Some("71 %"));
        let z = r.app.applets.zones().iter().find(|(k, _)| *k == "orchestre").unwrap().1;
        r.tap(z.cx(), z.cy());
        assert_eq!(r.app.applets.ouvert, Some("orchestre"));
    }
}
