//! Le contexte Disque, à la façon de l'analyseur d'utilisation des disques de GNOME :
//! l'occupation du stockage en haut, puis un graphique en anneaux (le dossier au
//! centre, ses enfants autour, sur quatre niveaux) et la liste de ses enfants.
//! Toucher un dossier le met au centre ; toucher le centre remonte.
//!
//! L'analyse est commune (`Commun::analyse`) ; chaque fenêtre a son dossier au centre
//! et son élément choisi. Le centre s'enregistre par ses noms, qui survivent à une
//! nouvelle analyse (pas les indices).

use super::{hachage, ligne_fichier, Puce, TEINTES};
use crate::contexte::{Commun, Contexte, Etat, Genre};
use crate::disque::{self, Analyse, Noeud};
use crate::draw::{Color, Rect};
use crate::explorateur;
use crate::surface::Placee;
use crate::ui::{id, sub_id, Id};
use crate::widgets::{Cx, Icon};

pub const CLE: &str = "disque";
pub const GENRE: Genre = Genre { cle: CLE, nom: "Disque", legende: "L'occupation du stockage, en anneaux", icone: Icon::Anneau, ouvrir };

const DISQUE: Id = id("disque");
/// Anneaux du graphique, autour du dossier central.
const ANNEAUX: usize = 4;
/// Séparateur des noms du centre dans le fichier de l'espace.
const SEP: char = '/';

#[derive(Clone, Default)]
pub struct VueDisque {
    /// Le dossier au centre : un chemin d'indices dans l'arbre de l'analyse…
    centre: Vec<usize>,
    /// …et le même en noms, pour l'enregistrement.
    noms: Vec<String>,
    /// Les noms relus attendent que l'analyse soit prête pour devenir des indices.
    a_placer: bool,
    choisi: Option<Vec<usize>>,
}

fn ouvrir(e: &Etat) -> Box<dyn Contexte> {
    let noms: Vec<String> = e.texte("centre").map_or(Vec::new(), |t| t.split(SEP).filter(|n| !n.is_empty()).map(String::from).collect());
    Box::new(VueDisque { a_placer: !noms.is_empty(), noms, ..VueDisque::default() })
}

/// Les indices d'un chemin de noms, aussi loin qu'il mène.
fn indices(arbre: &Noeud, noms: &[String]) -> Vec<usize> {
    let mut n = arbre;
    let mut out = Vec::new();
    for nom in noms {
        let Some(i) = n.enfants.iter().position(|e| &e.nom == nom && e.dossier) else { break };
        out.push(i);
        n = &n.enfants[i];
    }
    out
}

/// Les noms d'un chemin d'indices.
fn noms(arbre: &Noeud, centre: &[usize]) -> Vec<String> {
    (1..=centre.len()).filter_map(|k| arbre.suivre(&centre[..k]).map(|n| n.nom.clone())).collect()
}

impl Contexte for VueDisque {
    fn cle(&self) -> &'static str {
        CLE
    }

    fn dupliquer(&self) -> Box<dyn Contexte> {
        Box::new(self.clone())
    }

    fn ecrire(&self, e: &mut Etat) {
        if !self.noms.is_empty() {
            e.mettre("centre", self.noms.join(&SEP.to_string()));
        }
    }

    /// Le graphique s'anime pendant l'analyse.
    fn anime(&self, c: &Commun) -> bool {
        c.analyse.as_ref().is_some_and(|a| a.en_cours())
    }

    fn dessiner(&mut self, cx: &mut Cx, c: &mut Commun, p: &Placee) -> String {
        let r = p.contenu;
        let (d, col) = (cx.dp, cx.couleurs());
        cx.dl.rrect(r, 0.0, col.surface);
        let base = sub_id(DISQUE, p.id as usize);
        let Some(analyse) = c.analyser() else { return "aucun stockage".into() };
        let racine = analyse.racine.clone();
        let resultat = analyse.resultat();
        let progres = (analyse.progres.fichiers.load(std::sync::atomic::Ordering::Relaxed), analyse.progres.octets.load(std::sync::atomic::Ordering::Relaxed));
        let ligne = (cx.theme.ligne * d).round();
        let marge = cx.pad();

        // L'occupation du système de fichiers : texte et barre
        let mut haut = r.y + marge;
        if let Some((total, libre)) = disque::espace(&racine) {
            let utilise = total.saturating_sub(libre);
            let t = format!("{} utilisés sur {} · {} libres", explorateur::taille(utilise), explorateur::taille(total), explorateur::taille(libre));
            // À droite de la pastille + du coin de la fenêtre
            let x0 = r.x + cx.coin() + cx.commande() + cx.ecart();
            cx.clip(Some(r));
            cx.text_at(x0, haut + ligne * 0.35, &t, 14.0, col.texte);
            let barre = Rect::new(x0, haut + ligne * 0.7, (r.right() - marge - x0).max(0.0), 6.0 * d);
            cx.dl.rrect(barre, 3.0 * d, col.ligne);
            let k = if total > 0 { utilise as f32 / total as f32 } else { 0.0 };
            cx.dl.rrect(Rect::new(barre.x, barre.y, barre.w * k, barre.h), 3.0 * d, col.accent);
            cx.clip(None);
            haut += ligne + marge * 0.5;
        }
        let reste = Rect::new(r.x, haut, r.w, (r.bottom() - haut).max(0.0));

        let Some((arbre, duree)) = resultat else {
            // Pendant l'analyse : ce qui est déjà compté
            let (n, o) = progres;
            cx.clip(Some(reste));
            cx.centered(Rect::new(reste.x, reste.y, reste.w, reste.h * 0.5), "Analyse du stockage…", 16.0, col.texte);
            cx.centered(Rect::new(reste.x, reste.cy() - ligne * 0.5, reste.w, ligne), &format!("{n} fichiers · {}", explorateur::taille(o)), 13.0, col.pale);
            cx.clip(None);
            return format!("analyse · {n} fichiers");
        };

        // Le centre relu du fichier de l'espace, maintenant que l'arbre est là
        if std::mem::take(&mut self.a_placer) {
            self.centre = indices(&arbre, &self.noms);
        }
        // Le dossier au centre (remis à la racine s'il ne mène plus nulle part, après
        // une nouvelle analyse)
        let mut centre = self.centre.clone();
        if arbre.suivre(&centre).is_none() {
            centre.clear();
        }
        let noeud = arbre.suivre(&centre).unwrap_or(&arbre);
        let mut choisi = self.choisi.clone().filter(|c| arbre.suivre(c).is_some());

        // Le graphique et la liste : côte à côte en paysage, l'un sur l'autre sinon
        let paysage = reste.w > reste.h * 1.15;
        let (zone, liste) = if paysage {
            let c = reste.h.min(reste.w * 0.6);
            (Rect::new(reste.x, reste.y, c, reste.h), Rect::new(reste.x + c, reste.y, reste.w - c, reste.h))
        } else {
            let c = reste.w.min(reste.h * 0.6);
            (Rect::new(reste.x, reste.y, reste.w, c), Rect::new(reste.x, reste.y + c, reste.w, reste.h - c))
        };
        let rayon = (zone.w.min(zone.h) / 2.0 - marge).max(0.0);
        let ctr = [zone.cx(), zone.cy()];
        let r0 = rayon * 0.28;
        let large = (rayon - r0) / ANNEAUX as f32;
        let ecart = 1.5 * d;
        let teinte = |s: &disque::Secteur| -> Color {
            let t = TEINTES[s.teinte % TEINTES.len()];
            // Plus loin du centre, plus pâle ; les fichiers un peu éteints
            let k = 0.17 * (s.anneau as f32 - 1.0) + if s.dossier { 0.0 } else { 0.25 };
            let m = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * k.min(0.8)) as u8;
            [m(t[0], col.surface[0]), m(t[1], col.surface[1]), m(t[2], col.surface[2]), 255]
        };
        let secteurs = if rayon > 0.0 { disque::secteurs(noeud, ANNEAUX, 3.0 * d / rayon) } else { Vec::new() };
        let q = std::f32::consts::FRAC_PI_2;
        cx.clip(Some(zone));
        cx.dl.rrect(Rect::new(ctr[0] - r0 + ecart, ctr[1] - r0 + ecart, 2.0 * (r0 - ecart), 2.0 * (r0 - ecart)), r0, col.puce);
        for s in &secteurs {
            let a = r0 + (s.anneau as f32 - 1.0) * large;
            // L'élément choisi prend la couleur d'accent
            let plein: Vec<usize> = centre.iter().chain(&s.chemin).copied().collect();
            let t = if choisi.as_ref() == Some(&plein) { col.accent } else { teinte(s) };
            cx.dl.secteur(ctr, a + ecart / 2.0, a + large - ecart / 2.0, s.a0 - q, s.a1 - q, ecart, t);
        }
        // Au centre : le nom et la taille du dossier
        let nom = if centre.is_empty() { "Stockage" } else { noeud.nom.as_str() };
        let interieur = Rect::new(ctr[0] - r0 * 0.85, ctr[1] - r0 * 0.6, r0 * 1.7, r0 * 1.2);
        cx.clip(Some(interieur.intersect(&zone)));
        cx.centered(Rect::new(interieur.x, ctr[1] - ligne * 0.45, interieur.w, ligne * 0.5), nom, 14.0, col.texte);
        cx.centered(Rect::new(interieur.x, ctr[1], interieur.w, ligne * 0.45), &explorateur::taille(noeud.taille), 12.0, col.pale);
        cx.clip(None);

        // Toucher le graphique : un dossier vient au centre, un fichier est choisi, le
        // centre remonte d'un niveau
        let graphique = sub_id(base, 1);
        cx.ui.region_add(graphique, zone, crate::ui::Sense::Click { scroll: None });
        let mut nouveau_centre: Option<Vec<usize>> = None;
        if let Some((x, y)) = cx.ui.clic_en(graphique) {
            let (dx, dy) = (x - ctr[0], y - ctr[1]);
            let dist = (dx * dx + dy * dy).sqrt();
            if dist < r0 {
                if !centre.is_empty() {
                    let mut p = centre.clone();
                    p.pop();
                    nouveau_centre = Some(p);
                }
            } else if large > 0.0 {
                let anneau = ((dist - r0) / large) as usize + 1;
                let angle = (dy.atan2(dx) + q).rem_euclid(std::f32::consts::TAU);
                if let Some(s) = disque::toucher(&secteurs, anneau, angle) {
                    let plein: Vec<usize> = centre.iter().chain(&s.chemin).copied().collect();
                    if s.dossier {
                        nouveau_centre = Some(plein);
                    } else {
                        choisi = Some(plein);
                    }
                }
            }
        }

        // La liste des enfants du centre, avec la teinte de leur secteur
        let scroll = sub_id(base, hachage(&centre[..]));
        let mut b = cx.body(scroll, liste, &c.scrolls);
        if !centre.is_empty() && ligne_fichier(cx, &mut b, sub_id(scroll, 999_999), Puce::Icone(Icon::Haut), "Dossier parent", "", false) {
            let mut p = centre.clone();
            p.pop();
            nouveau_centre = Some(p);
        }
        for (i, e) in noeud.enfants.iter().enumerate() {
            let s = disque::Secteur { chemin: vec![i], anneau: 1, a0: 0.0, a1: 0.0, teinte: i, dossier: e.dossier };
            let part = if noeud.taille > 0 { 100.0 * e.taille as f32 / noeud.taille as f32 } else { 0.0 };
            let info = format!("{} · {:>3.0} %", explorateur::taille(e.taille), part);
            let plein: Vec<usize> = centre.iter().copied().chain([i]).collect();
            let est_choisi = choisi.as_ref() == Some(&plein);
            let puce = if e.dossier || est_choisi { Puce::Teinte(teinte(&s)) } else { Puce::Icone(Icon::Fichier) };
            if ligne_fichier(cx, &mut b, sub_id(scroll, i), puce, &e.nom, &info, est_choisi) {
                if e.dossier {
                    nouveau_centre = Some(plein);
                } else {
                    choisi = Some(plein);
                }
            }
        }
        b.y += cx.ecart();
        let relancer = cx.button(&mut b, sub_id(scroll, 999_998), "Relancer l'analyse");
        cx.body_end(b, &mut c.scrolls);

        let statut = match choisi.as_ref().and_then(|c| arbre.suivre(c)) {
            Some(n) => format!("{} · {}", n.nom, explorateur::taille(n.taille)),
            None => format!("{} fichiers · {} · {:.1} s", arbre.fichiers, explorateur::taille(arbre.taille), duree),
        };
        if let Some(p) = nouveau_centre {
            centre = p;
            choisi = None;
        }
        if centre != self.centre || self.noms.len() != centre.len() {
            self.noms = noms(&arbre, &centre);
        }
        self.centre = centre;
        self.choisi = choisi;
        if relancer {
            c.analyse = Some(Analyse::lancer(racine, crate::radio::reveiller));
        }
        statut
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(nom: &str, enfants: Vec<Noeud>) -> Noeud {
        Noeud { nom: nom.into(), taille: 1, dossier: true, fichiers: 1, enfants }
    }

    #[test]
    fn le_centre_passe_des_noms_aux_indices() {
        let arbre = n("", vec![n("a", vec![]), n("b", vec![n("x", vec![]), n("y", vec![])])]);
        let i = indices(&arbre, &["b".into(), "y".into()]);
        assert_eq!(i, vec![1, 1]);
        assert_eq!(noms(&arbre, &i), vec!["b".to_string(), "y".to_string()]);
        // Un nom disparu arrête le chemin là où il mène encore
        assert_eq!(indices(&arbre, &["b".into(), "z".into()]), vec![1]);
    }
}
