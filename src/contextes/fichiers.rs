//! Le contexte Fichiers : un explorateur. Chaque fenêtre a son dossier ouvert et son
//! choix d'afficher les fichiers cachés. Un dossier s'ouvre au toucher, un fichier
//! audio se lit dans la piste « Fichier » du mixeur, un fichier texte s'ouvre dans
//! l'éditeur, un SVG dans Encre : l'un ou l'autre prend la place de l'explorateur
//! dans la fenêtre.
//!
//! Les entrées sont des cartes de 3,5 pour 1 : à gauche l'empreinte d'un dossier (sa
//! place dans la synthèse du stockage) ou le badge du type MIME d'un fichier, à
//! droite le nom, une légende (taille, part du parent ou type) et la date.

use super::{abreger, empreinte, hachage, TEINTES};
use crate::contexte::{Commun, Contexte, Demande, Etat, Genre};
use crate::disque;
use crate::draw::Rect;
use crate::explorateur::{self, Dossier, Famille, Nature};
use crate::radio;
use crate::surface::Placee;
use crate::ui::{id, sub_id, Id};
use crate::widgets::{Cx, Icon};
use std::path::PathBuf;

pub const CLE: &str = "fichiers";
pub const GENRE: Genre = Genre { cle: CLE, nom: "Fichiers", legende: "Parcourir le stockage, ouvrir les documents", icone: Icon::Dossier, ouvrir };

const FICHIERS: Id = id("fichiers");
/// Le rapport largeur sur hauteur des cartes.
const RAPPORT: f32 = 3.5;
/// Largeur visée d'une carte (dp) : le nombre de colonnes en découle.
const CARTE_LARGE: f32 = 260.0;
/// Hauteur minimale d'une carte (dp), quand la fenêtre est étroite.
const CARTE_MIN: f32 = 56.0;

/// Le badge du type d'un fichier : un carré teinté selon sa famille, son extension
/// au centre.
fn badge(cx: &mut Cx, r: Rect, texte: &str, famille: Famille) {
    let teinte = match famille {
        Famille::Audio => TEINTES[4],
        Famille::Image => TEINTES[2],
        Famille::Video => TEINTES[1],
        Famille::Texte => TEINTES[0],
        Famille::Document => TEINTES[6],
        Famille::Archive => TEINTES[3],
        Famille::Application => TEINTES[5],
        Famille::Inconnue => [0x8A, 0x8F, 0x98, 255],
    };
    cx.dl.rrect(r, r.w * 0.22, teinte);
    // Le coin replié, en haut à droite
    let p = r.w * 0.28;
    cx.dl.rrect(Rect::new(r.right() - p, r.y, p, p), r.w * 0.06, [255, 255, 255, 70]);
    let taille = (r.h / cx.dp / cx.theme.texte * if texte.chars().count() > 3 { 0.24 } else { 0.3 }).clamp(8.0, 20.0);
    cx.centered(Rect::new(r.x, r.y + r.h * 0.12, r.w, r.h * 0.8), texte, taille, [255, 255, 255, 255]);
}

/// Une date de modification : l'heure si c'est aujourd'hui, sinon le jour.
fn date(t: u64, maintenant: u64) -> String {
    let (j, m, a) = crate::android::date_de(t);
    if crate::android::date_de(maintenant) == (j, m, a) {
        let (h, mi) = crate::android::heure_de(t);
        format!("Modifié aujourd'hui à {h:02}:{mi:02}")
    } else {
        format!("Modifié le {j:02}/{m:02}/{a}")
    }
}

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
        // Les empreintes viennent de la synthèse de l'analyse du stockage, enregistrée
        // sur le disque et refaite au plus une fois par jour
        let synthese = c.empreintes();
        let situation = synthese.as_ref().and_then(|s| {
            let rel = dossier.chemin.strip_prefix(&s.racine).ok()?;
            let noms: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
            disque::situer(&s.arbre, &noms)
        });
        // Un défilement par dossier : en revenant au parent, on retrouve sa position
        let scroll = sub_id(base, hachage(&dossier.chemin));
        let lecture = radio::lecteur_fichiers().adresse.clone();
        let ligne = (cx.theme.ligne * d).round();
        let mut aller: Option<PathBuf> = None;
        // Le fil d'Ariane, fixe en haut de la fenêtre : chaque étape s'ouvre au toucher
        // (c'est par lui qu'on remonte) ; trop long, on voit la fin
        let pad = 14.0 * d;
        let bande = Rect::new(r.x, r.y, r.w, (ligne + 8.0 * d).min(r.h));
        cx.ui.region_add(sub_id(base, 999_997), bande, crate::ui::Sense::Sink);
        let etapes = explorateur::ariane(&dossier.chemin, c.racine.as_deref());
        let sep = "  ›  ";
        let largeur: f32 = etapes.iter().map(|(n, _)| cx.measure(n, 15.0)).sum::<f32>() + cx.measure(sep, 15.0) * (etapes.len() as f32 - 1.0);
        let x0 = bande.x + pad;
        let place = (bande.right() - pad - x0).max(0.0);
        let mut x = x0 + (place - largeur).min(0.0);
        let cy = bande.y + 4.0 * d + ligne / 2.0;
        cx.clip(Some(Rect::new(x0, bande.y, place, bande.h)));
        for (i, (nom, chemin)) in etapes.iter().enumerate() {
            let w = cx.measure(nom, 15.0);
            let zone = Rect::new(x - 4.0 * d, bande.y, w + 8.0 * d, bande.h);
            let derniere = i + 1 == etapes.len();
            let k = sub_id(base, 1_000_000 + i);
            if cx.ui.click(k, zone, None) && !derniere {
                aller = Some(chemin.clone());
            }
            if cx.ui.pressed(k) && !derniere {
                cx.dl.rrect(zone.inset(4.0 * d), 6.0 * d, col.puce);
            }
            cx.text_at(x, cy, nom, 15.0, if derniere { col.texte } else { col.accent });
            x += w;
            if !derniere {
                cx.text_at(x, cy, sep, 15.0, col.pale);
                x += cx.measure(sep, 15.0);
            }
        }
        cx.clip(None);
        cx.dl.rrect(Rect::new(bande.x, bande.bottom() - d.max(1.0), bande.w, d.max(1.0)), 0.0, col.ligne);
        // Sous le fil, les entrées défilent
        let corps = Rect::new(r.x, bande.bottom(), r.w, (r.h - bande.h).max(0.0));
        let mut b = cx.body(scroll, corps, &c.scrolls);
        b.y -= 8.0 * d;
        if let Some(e) = &dossier.erreur {
            let aide = if e == "Accès refusé" { format!("{e}. Pour parcourir tout le stockage, accordez à Flow l'accès à tous les fichiers dans les réglages d'Android.") } else { format!("{e}.") };
            cx.paragraph(&mut b, &aide);
        } else if dossier.entrees.is_empty() {
            cx.paragraph(&mut b, "Dossier vide.");
        }
        // Les entrées, en cartes de 3,5 pour 1, sur autant de colonnes que la largeur
        // le permet
        let ecart = cx.ecart().max(8.0 * d);
        b.y += ecart;
        let colonnes = (((b.w + ecart) / (CARTE_LARGE * d + ecart)).floor() as usize).max(1);
        let large = (b.w - ecart * (colonnes - 1) as f32) / colonnes as f32;
        let haute = (large / RAPPORT).max(CARTE_MIN * d * cx.theme.texte).round();
        let vue = b.rect();
        let maintenant = disque::maintenant();
        for (i, e) in dossier.entrees.iter().enumerate() {
            let carte = Rect::new(b.x + (i % colonnes) as f32 * (large + ecart), b.y + (i / colonnes) as f32 * (haute + ecart), large, haute);
            if carte.bottom() < vue.y || carte.y > vue.bottom() {
                continue;
            }
            let chemin = dossier.chemin.join(&e.nom);
            let joue = lecture.as_deref() == chemin.to_str();
            let k = sub_id(base, i);
            let clic = cx.ui.click(k, carte, Some(b.id()));
            let appui = cx.ui.pressed(k);
            let rayon = cx.theme.rayon(carte.h).min(14.0 * d);
            cx.dl.rrect(carte, rayon, if appui || joue { col.accent } else { col.ligne });
            cx.dl.rrect(carte.inset(d.max(1.0)), rayon, if appui { col.puce } else { col.fond });
            // À gauche, un carré : l'empreinte du dossier, ou le badge du type du fichier
            let pad = (carte.h * 0.16).clamp(6.0 * d, 14.0 * d);
            let cote = carte.h - 2.0 * pad;
            let carre = Rect::new(carte.x + pad, carte.y + pad, cote, cote);
            let legende = if e.dossier() {
                let sous = situation.as_ref().and_then(|p| Some((p.enfant(&e.nom)?, p.noeud.enfants.iter().find(|x| x.nom == e.nom)?.taille, p.noeud.taille)));
                match sous {
                    Some(((arcs, teinte), t, parent)) => {
                        empreinte(cx, carre, &arcs, TEINTES[teinte % TEINTES.len()]);
                        let part = if parent > 0 { t as f64 / parent as f64 * 100.0 } else { 0.0 };
                        let part = if part >= 10.0 { format!("{part:.0} %") } else { format!("{part:.1} %").replace('.', ",") };
                        format!("Dossier · {} · {part} du parent", explorateur::taille(t))
                    }
                    None => {
                        cx.dl.rrect(carre, cote * 0.22, col.puce);
                        cx.icon(carre.inset(cote * 0.2), Icon::Dossier, col.accent);
                        // Trop petit pour la synthèse, ou pas encore analysé
                        if synthese.is_some() { "Dossier · presque vide".to_string() } else { "Dossier".to_string() }
                    }
                }
            } else {
                let (type_mime, famille) = explorateur::mime(&e.nom);
                badge(cx, carre, &explorateur::badge(&e.nom), famille);
                format!("{type_mime} · {}", explorateur::taille(e.taille))
            };
            let date = e.modifie.and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok()).map(|t| date(t.as_secs(), maintenant));
            // À droite : le titre, la légende, la date
            let x = carre.right() + pad;
            let w = (carte.right() - pad - x).max(0.0);
            cx.clip(Some(Rect::new(x, carte.y, w, carte.h).intersect(&vue)));
            let t = cx.theme.texte;
            let lignes = if date.is_some() && carte.h >= 64.0 * d * t { 3.0 } else { 2.0 };
            let y0 = carte.cy() - (20.0 + 16.0 * (lignes - 1.0)) * d * t / 2.0;
            let titre = abreger(cx, &e.nom, 15.0, w);
            cx.text_at(x, y0 + 10.0 * d * t, &titre, 15.0, if joue { col.accent } else { col.texte });
            let legende = abreger(cx, &legende, 12.0, w);
            cx.text_at(x, y0 + 28.0 * d * t, &legende, 12.0, col.pale);
            if let (Some(date), 3.0) = (&date, lignes) {
                cx.text_at(x, y0 + 44.0 * d * t, date, 12.0, col.pale);
            }
            cx.clip(Some(vue));
            if clic {
                match e.nature {
                    Nature::Dossier => aller = Some(chemin),
                    Nature::Audio if joue => radio::lecteur_fichiers().arreter(),
                    Nature::Audio => radio::lecteur_fichiers().jouer(&chemin.to_string_lossy()),
                    Nature::Texte => {
                        let mut etat = Etat::default();
                        etat.mettre("document", chemin.display());
                        c.demandes.push(Demande::Ouvrir { depuis: p.id, cle: super::texte::CLE, etat });
                    }
                    Nature::Image if chemin.extension().is_some_and(|x| x.eq_ignore_ascii_case("svg")) => {
                        let mut etat = Etat::default();
                        etat.mettre("document", chemin.display());
                        c.demandes.push(Demande::Ouvrir { depuis: p.id, cle: super::encre::CLE, etat });
                    }
                    _ => {}
                }
            }
        }
        b.y += dossier.entrees.len().div_ceil(colonnes) as f32 * (haute + ecart);
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
