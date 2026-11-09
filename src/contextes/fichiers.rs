//! Le contexte Fichiers : un explorateur. Chaque fenêtre a son dossier ouvert et son
//! choix d'afficher les fichiers cachés. Un dossier s'ouvre au toucher, un fichier
//! audio se lit dans la piste « Fichier » du mixeur.

use super::{hachage, ligne_fichier, Puce, TEINTES};
use crate::contexte::{Commun, Contexte, Etat, Genre};
use crate::disque;
use crate::draw::Rect;
use crate::explorateur::{self, Dossier, Nature};
use crate::radio;
use crate::surface::Placee;
use crate::ui::{id, sub_id, Id};
use crate::widgets::{Cx, Icon};
use std::path::PathBuf;

pub const CLE: &str = "fichiers";
pub const GENRE: Genre = Genre { cle: CLE, nom: "Fichiers", icone: Icon::Dossier, ouvrir };

const FICHIERS: Id = id("fichiers");

#[derive(Clone, Default)]
pub struct Fichiers {
    /// Le dossier à ouvrir au premier dessin (relu du fichier de l'espace).
    chemin: Option<PathBuf>,
    dossier: Option<Dossier>,
    caches: bool,
}

fn ouvrir(e: &Etat) -> Box<dyn Contexte> {
    Box::new(Fichiers { chemin: e.texte("dossier").map(PathBuf::from), dossier: None, caches: e.oui("caches").unwrap_or(false) })
}

impl Fichiers {
    fn aller(&mut self, chemin: PathBuf) {
        self.dossier = Some(Dossier::lire(&chemin, self.caches));
        self.chemin = Some(chemin);
    }
}

impl Contexte for Fichiers {
    fn cle(&self) -> &'static str {
        CLE
    }

    fn dupliquer(&self) -> Box<dyn Contexte> {
        Box::new(self.clone())
    }

    fn ecrire(&self, e: &mut Etat) {
        if let Some(c) = &self.chemin {
            e.mettre("dossier", c.display());
        }
        e.mettre_oui("caches", self.caches);
    }

    fn retour(&mut self, _c: &Commun) {
        // Le dossier a pu changer entre-temps
        if let Some(c) = self.chemin.clone().filter(|_| self.dossier.is_some()) {
            self.aller(c);
        }
    }

    fn dessiner(&mut self, cx: &mut Cx, c: &mut Commun, p: &Placee) -> String {
        let r = p.contenu;
        let (d, col) = (cx.dp, cx.couleurs());
        cx.dl.rrect(r, 0.0, col.surface);
        if self.dossier.is_none() {
            let depart = self.chemin.clone().or_else(|| c.racine.clone()).unwrap_or_else(|| PathBuf::from("/"));
            self.aller(depart);
        }
        let dossier = self.dossier.clone().unwrap_or_default();
        let base = sub_id(FICHIERS, p.id as usize);
        // Les empreintes viennent de l'analyse du stockage, lancée au besoin
        let arbre = c.analyser().and_then(|a| a.resultat().map(|(n, _)| (n, a.racine.clone())));
        let situation = arbre.as_ref().and_then(|(n, racine)| {
            let rel = dossier.chemin.strip_prefix(racine).ok()?;
            let noms: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
            disque::placer(n, &noms)
        });
        // Un défilement par dossier : en revenant au parent, on retrouve sa position
        let scroll = sub_id(base, hachage(&dossier.chemin));
        let lecture = radio::lecteur_fichiers().adresse.clone();
        let ligne = (cx.theme.ligne * d).round();
        let mut aller: Option<PathBuf> = None;
        let mut b = cx.body(scroll, r, &c.scrolls);

        // Le fil d'Ariane : chaque étape s'ouvre au toucher ; trop long, on voit la fin
        let etapes = explorateur::ariane(&dossier.chemin, c.racine.as_deref());
        let sep = "  ›  ";
        let largeur: f32 = etapes.iter().map(|(n, _)| cx.measure(n, 15.0)).sum::<f32>() + cx.measure(sep, 15.0) * (etapes.len() as f32 - 1.0);
        // La pastille + de la fenêtre occupe le coin haut gauche : le fil part à sa droite
        let x0 = (r.x + cx.coin() + cx.commande() + cx.ecart()).max(b.x);
        let place = b.x + b.w - x0;
        let mut x = x0 + (place - largeur).min(0.0);
        cx.clip(Some(Rect::new(x0, b.y, place.max(0.0), ligne).intersect(&b.rect())));
        for (i, (nom, chemin)) in etapes.iter().enumerate() {
            let w = cx.measure(nom, 15.0);
            let zone = Rect::new(x - 4.0 * d, b.y, w + 8.0 * d, ligne);
            let derniere = i + 1 == etapes.len();
            if cx.ui.click(sub_id(base, 1_000_000 + i), zone, Some(b.id())) && !derniere {
                aller = Some(chemin.clone());
            }
            cx.text_at(x, b.y + ligne / 2.0, nom, 15.0, if derniere { col.texte } else { col.accent });
            x += w;
            if !derniere {
                cx.text_at(x, b.y + ligne / 2.0, sep, 15.0, col.pale);
                x += cx.measure(sep, 15.0);
            }
        }
        cx.clip(Some(b.rect()));
        b.y += ligne;
        cx.dl.rrect(Rect::new(b.rect().x, b.y, b.rect().w, d.max(1.0)), 0.0, col.ligne);

        if let Some(parent) = dossier.chemin.parent() {
            if ligne_fichier(cx, &mut b, sub_id(base, 999_999), Puce::Icone(Icon::Haut), "Dossier parent", "", false) {
                aller = Some(parent.to_path_buf());
            }
        }
        if let Some(e) = &dossier.erreur {
            let aide = if e == "Accès refusé" { format!("{e}. Pour parcourir tout le stockage, accordez à Flow l'accès à tous les fichiers dans les réglages d'Android.") } else { format!("{e}.") };
            cx.paragraph(&mut b, &aide);
        } else if dossier.entrees.is_empty() {
            cx.paragraph(&mut b, "Dossier vide.");
        }
        for (i, e) in dossier.entrees.iter().enumerate() {
            let chemin = dossier.chemin.join(&e.nom);
            let icone = match e.nature {
                Nature::Dossier => Icon::Dossier,
                Nature::Audio => Icon::Note,
                Nature::Image => Icon::Photo,
                Nature::Video => Icon::Film,
                _ => Icon::Fichier,
            };
            let info = if e.dossier() { String::new() } else { explorateur::taille(e.taille) };
            let joue = lecture.as_deref() == chemin.to_str();
            // L'empreinte d'un dossier : sa place dans le graphique du stockage
            let puce = match (&situation, e.dossier()) {
                (Some(p), true) => p.noeud.enfants.iter().position(|n| n.nom == e.nom && n.dossier).map(|k| {
                    let span = p.arcs.last().copied().unwrap_or((0.0, std::f32::consts::TAU));
                    let mut arcs = p.arcs.clone();
                    arcs.push(disque::arc_enfant(p.noeud, span, k));
                    Puce::Empreinte(arcs, TEINTES[p.teinte.unwrap_or(k) % TEINTES.len()])
                }),
                _ => None,
            };
            let puce = puce.unwrap_or(Puce::Icone(icone));
            if ligne_fichier(cx, &mut b, sub_id(base, i), puce, &e.nom, &info, joue) {
                match e.nature {
                    Nature::Dossier => aller = Some(chemin),
                    Nature::Audio if joue => radio::lecteur_fichiers().arreter(),
                    Nature::Audio => radio::lecteur_fichiers().jouer(&chemin.to_string_lossy()),
                    _ => {}
                }
            }
        }
        b.y += cx.ecart();
        let mut caches = self.caches;
        cx.toggle(&mut b, sub_id(base, 999_998), "Fichiers cachés", &mut caches);
        cx.body_end(b, &mut c.scrolls);
        if caches != self.caches {
            self.caches = caches;
            aller = Some(dossier.chemin.clone());
        }
        if let Some(ch) = aller {
            self.aller(ch);
        }
        let (nd, nf) = dossier.compte();
        let pluriel = |n: usize, mot: &str| format!("{n} {mot}{}", if n > 1 { "s" } else { "" });
        format!("{} · {}", pluriel(nd, "dossier"), pluriel(nf, "fichier"))
    }
}
