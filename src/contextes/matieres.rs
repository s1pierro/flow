//! Matières : l'atelier des textures répétables (ce qui fut Texturine). Une matière
//! est un fichier de réglages `Flow/matieres/<nom>.txt` (la crate partagée
//! `sage-matieres`), et l'image qu'il donne, `Flow/matieres/<nom>.png`, refaite à
//! chaque enregistrement : c'est elle que les volumes (la maquette des Lieux) plaquent.
//!
//! La vue montre la texture en mosaïque (pour juger les raccords) ; le panneau, à
//! droite en paysage et dessous en portrait, a deux onglets : la bibliothèque (choisir,
//! créer) et les réglages de la matière ouverte. L'image se refait dans un fil à part ;
//! le fichier s'enregistre une fois le doigt levé.
//!
//! Pilote : `matiere <nom>`, `nouvelle <genre> [nom]`, `regler <clé> <valeur…>` (les
//! clés du fichier), `mosaique <n>`, `matieres` (la liste).

use crate::contexte::{Commun, Contexte, Etat, Genre};
use crate::draw::{Image, Rect};
use crate::surface::Placee;
use crate::ui::{id, sub_id, Id};
use crate::widgets::{Body, Cx, Icon};
use sage_matieres::generateur::{nombre_court, RESOLUTIONS};
use sage_matieres::{Genre as Sorte, Reglages, Tsl};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;

pub const CLE: &str = "matieres";
pub const GENRE: Genre = Genre { cle: CLE, nom: "Matières", legende: "Les textures répétables : bois, brique, pierre…", icone: Icon::Photo, ouvrir, applet: None };

const MATIERES: Id = id("matieres");
const ONGLETS: [&str; 2] = ["Bibliothèque", "Réglages"];
const BIBLIOTHEQUE: usize = 0;
const REGLAGES: usize = 1;

/// Une image en cours de fabrication : les réglages qu'elle rend, et sa réception.
struct Fabrication {
    reglages: Reglages,
    rx: Receiver<sage_matieres::Image>,
}

pub struct Matieres {
    /// Les matières du dossier, par nom.
    liste: Vec<String>,
    listee: bool,
    /// La matière ouverte : son nom et ses réglages.
    nom: Option<String>,
    reglages: Option<Reglages>,
    /// Ce qui est enregistré (pour n'écrire qu'au changement).
    enregistre: Option<Reglages>,
    /// L'image, et les réglages qu'elle rend.
    image: Option<(Image, Reglages)>,
    fabrication: Option<Fabrication>,
    revision: u64,
    onglet: usize,
    /// La mosaïque : n × n répétitions.
    mosaique: u32,
    message: Option<String>,
}

fn ouvrir(e: &Etat) -> Box<dyn Contexte> {
    Box::new(Matieres {
        liste: Vec::new(),
        listee: false,
        nom: e.texte("matiere").filter(|n| !n.is_empty()).map(String::from),
        reglages: None,
        enregistre: None,
        image: None,
        fabrication: None,
        revision: 0,
        onglet: (e.nombre("onglet").unwrap_or(0.0) as usize).min(ONGLETS.len() - 1),
        mosaique: (e.nombre("mosaique").unwrap_or(3.0) as u32).clamp(1, 6),
        message: None,
    })
}

/// Le dossier des matières.
pub fn dossier(c: &Commun) -> Option<PathBuf> {
    c.dossier.as_ref().map(|d| d.join("matieres"))
}

/// Les réglages d'une matière du dossier, relus.
pub fn lire(dossier: &Path, nom: &str) -> Option<Reglages> {
    Reglages::lire(&std::fs::read_to_string(dossier.join(format!("{nom}.txt"))).ok()?)
}

/// Un nom de fichier sûr : lettres, chiffres, `-` et `_`.
fn nom_sur(n: &str) -> String {
    let s: String = n.trim().chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).collect();
    if s.is_empty() { "matiere".into() } else { s }
}

impl Matieres {
    fn lister(&mut self, c: &Commun) {
        self.listee = true;
        self.liste.clear();
        let Some(d) = dossier(c) else { return };
        if let Ok(it) = std::fs::read_dir(&d) {
            for e in it.flatten() {
                let p = e.path();
                if p.extension().is_some_and(|x| x == "txt") {
                    if let Some(n) = p.file_stem().and_then(|s| s.to_str()) {
                        self.liste.push(n.to_string());
                    }
                }
            }
        }
        self.liste.sort();
    }

    /// Ouvre la matière `nom` du dossier.
    fn choisir(&mut self, c: &Commun, nom: &str) -> Result<(), String> {
        let d = dossier(c).ok_or("dossier de Flow inconnu")?;
        let r = lire(&d, nom).ok_or(format!("matière illisible : {nom}"))?;
        self.nom = Some(nom.to_string());
        self.enregistre = Some(r.clone());
        self.reglages = Some(r);
        self.image = None;
        Ok(())
    }

    /// Crée une matière de ce genre (réglages par défaut), sous un nom libre.
    fn creer(&mut self, c: &Commun, sorte: Sorte, nom: Option<&str>) -> Result<String, String> {
        let d = dossier(c).ok_or("dossier de Flow inconnu")?;
        std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
        let base = nom_sur(nom.unwrap_or(sorte.id()));
        let mut n = base.clone();
        let mut k = 2;
        while d.join(format!("{n}.txt")).exists() {
            n = format!("{base}-{k}");
            k += 1;
        }
        self.nom = Some(n.clone());
        self.reglages = Some(Reglages::defaut(sorte));
        self.enregistre = None;
        self.image = None;
        self.enregistrer(c)?;
        self.lister(c);
        Ok(n)
    }

    /// Écrit les réglages et l'image (PNG) de la matière ouverte.
    fn enregistrer(&mut self, c: &Commun) -> Result<(), String> {
        let (Some(nom), Some(r)) = (&self.nom, &self.reglages) else { return Ok(()) };
        let d = dossier(c).ok_or("dossier de Flow inconnu")?;
        std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
        std::fs::write(d.join(format!("{nom}.txt")), r.ecrire()).map_err(|e| e.to_string())?;
        self.enregistre = Some(r.clone());
        Ok(())
    }

    /// L'image à jour : fabriquée dans un fil quand les réglages changent ; écrite en
    /// PNG à côté des réglages quand elle rend ce qui est enregistré.
    fn suivre(&mut self, c: &Commun) {
        if let Some(f) = &self.fabrication {
            if let Ok(img) = f.rx.try_recv() {
                let f = self.fabrication.take().unwrap();
                self.revision += 1;
                if self.enregistre.as_ref() == Some(&f.reglages) {
                    if let (Some(nom), Some(d)) = (&self.nom, dossier(c)) {
                        let _ = std::fs::write(d.join(format!("{nom}.png")), sage_matieres::png::encoder(img.l, img.h, &img.rgba));
                    }
                }
                let i = Image { id: 0x6d61_7469_0000_0000 + (self as *const _ as u64 & 0xffff_ffff), revision: self.revision, largeur: img.l, hauteur: img.h, pixels: Arc::new(img.rgba) };
                self.image = Some((i, f.reglages));
            }
        }
        let Some(r) = &self.reglages else { return };
        let a_jour = self.image.as_ref().is_some_and(|(_, ri)| ri == r);
        if !a_jour && self.fabrication.is_none() {
            let (tx, rx) = channel();
            let rr = r.clone();
            std::thread::spawn(move || {
                let _ = tx.send(sage_matieres::generer(&rr));
            });
            self.fabrication = Some(Fabrication { reglages: r.clone(), rx });
        }
    }

    fn vue(&mut self, cx: &mut Cx, zone: Rect) {
        let col = cx.couleurs();
        cx.dl.rrect(zone, 0.0, col.fond);
        let Some((img, _)) = &self.image else {
            let t = if self.reglages.is_some() { "Génération…" } else { "Choisissez ou créez une matière dans la bibliothèque." };
            cx.centered(zone, t, 15.0, col.pale);
            return;
        };
        // La mosaïque, carrée, au centre
        let n = self.mosaique.max(1);
        let cote = (zone.w.min(zone.h) - 2.0 * cx.ecart()).max(16.0);
        let t = cote / n as f32;
        let (x0, y0) = (zone.cx() - cote / 2.0, zone.cy() - cote / 2.0);
        for i in 0..n {
            for j in 0..n {
                cx.dl.image(Rect::new(x0 + i as f32 * t, y0 + j as f32 * t, t + 0.5, t + 0.5), img.clone());
            }
        }
    }

    fn panneau(&mut self, cx: &mut Cx, c: &mut Commun, r: Rect, base: Id) {
        let (d, col) = (cx.dp, cx.couleurs());
        cx.dl.rrect(r, 0.0, col.surface);
        let h = cx.commande();
        let tete = Rect::new(r.x, r.y, r.w, h + 2.0 * cx.ecart());
        let mut tb = Body::fixe(sub_id(base, 2), tete, r.x + 10.0 * d, r.w - 20.0 * d, r.y + cx.ecart());
        if let Some(o) = crate::contextes::lieux::Lieux::puces(cx, &mut tb, sub_id(base, 100), &ONGLETS, self.onglet) {
            self.onglet = o;
        }
        let tete = Rect::new(r.x, r.y, r.w, tb.y - r.y);
        cx.dl.rrect(Rect::new(r.x, tete.bottom() - 1.0, r.w, 1.0), 0.0, col.ligne);
        let corps = Rect::new(r.x, tete.bottom(), r.w, (r.bottom() - tete.bottom()).max(0.0));
        let mut b = cx.body(sub_id(base, 3), corps, &c.scrolls);
        if let Some(m) = &self.message {
            cx.paragraph(&mut b, m);
        }
        match self.onglet {
            BIBLIOTHEQUE => self.bibliotheque(cx, c, &mut b, base),
            _ => self.panneau_reglages(cx, &mut b, base),
        }
        cx.body_end(b, &mut c.scrolls);
    }

    fn bibliotheque(&mut self, cx: &mut Cx, c: &mut Commun, b: &mut Body, base: Id) {
        cx.section(b, "Nouvelle matière");
        let sortes: Vec<&str> = Sorte::TOUS.iter().map(|s| s.nom()).collect();
        if let Some(i) = crate::contextes::lieux::Lieux::puces(cx, b, sub_id(base, 200), &sortes, usize::MAX) {
            match self.creer(c, Sorte::TOUS[i], None) {
                Ok(_) => {
                    self.onglet = REGLAGES;
                    self.message = None;
                }
                Err(e) => self.message = Some(e),
            }
        }
        cx.section(b, "Matières");
        if self.liste.is_empty() {
            cx.paragraph(b, "Aucune encore : créez-en une ci-dessus.");
        }
        let mut choix = None;
        for (i, n) in self.liste.iter().enumerate() {
            let courant = self.nom.as_deref() == Some(n.as_str());
            let label = if courant { format!("{n} (ouverte)") } else { n.clone() };
            if cx.button(b, sub_id(base, 300 + i), &label) {
                choix = Some(n.clone());
            }
        }
        if let Some(n) = choix {
            match self.choisir(c, &n) {
                Ok(()) => self.message = None,
                Err(e) => self.message = Some(e),
            }
        }
    }

    fn panneau_reglages(&mut self, cx: &mut Cx, b: &mut Body, base: Id) {
        let Some(r) = &mut self.reglages else {
            cx.paragraph(b, "Aucune matière ouverte.");
            return;
        };
        cx.paragraph(b, &format!("{} · {} · {}", self.nom.as_deref().unwrap_or("?"), r.genre.nom(), r.resume()));
        cx.section(b, "Taille");
        let s = format!("{} m", nombre_court(r.cote));
        cx.slider(b, sub_id(base, 400), "Côté de la texture", &s, &mut r.cote, 0.25, 8.0);
        r.cote = (r.cote * 20.0).round() / 20.0;
        let res: Vec<String> = RESOLUTIONS.iter().map(|x| format!("{x} px")).collect();
        let res_s: Vec<&str> = res.iter().map(|s| s.as_str()).collect();
        let i = RESOLUTIONS.iter().position(|x| *x == r.resolution).unwrap_or(1);
        if let Some(k) = crate::contextes::lieux::Lieux::puces(cx, b, sub_id(base, 410), &res_s, i) {
            r.resolution = RESOLUTIONS[k];
        }
        cx.section(b, "Éléments");
        let (lo, ho) = match r.genre {
            Sorte::Bois => ("Longueur des lames", "Largeur des lames"),
            Sorte::Brique => ("Longueur des briques", "Hauteur des briques"),
            Sorte::Pierre => ("Taille des pierres", "Hauteur des assises"),
            Sorte::Vase => ("Taille des taches", "Pas des rides"),
            Sorte::Paves => ("Longueur des pavés", "Largeur des pavés"),
            Sorte::Taille => ("Longueur des blocs", "Hauteur des assises"),
        };
        let s = format!("{} cm", nombre_court(r.longueur));
        cx.slider(b, sub_id(base, 420), lo, &s, &mut r.longueur, 2.0, 400.0);
        let s = format!("{} cm", nombre_court(r.hauteur));
        cx.slider(b, sub_id(base, 421), ho, &s, &mut r.hauteur, 1.0, 100.0);
        if let Some(k) = crate::contextes::lieux::Lieux::puces(cx, b, sub_id(base, 430), r.genre.appareils(), r.appareil as usize) {
            r.appareil = k as u32;
        }
        let pc = |v: f32| format!("{:.0} %", v * 100.0);
        let s = pc(r.irregularite);
        cx.slider(b, sub_id(base, 440), "Irrégularité", &s, &mut r.irregularite, 0.0, 1.0);
        let s = format!("{} mm", nombre_court(r.joint));
        cx.slider(b, sub_id(base, 441), if r.genre == Sorte::Vase { "Fentes" } else { "Joints" }, &s, &mut r.joint, 0.0, 60.0);
        let s = pc(r.relief);
        cx.slider(b, sub_id(base, 442), "Relief", &s, &mut r.relief, 0.0, 1.0);
        if r.genre == Sorte::Bois {
            let s = nombre_court(r.cernes);
            cx.slider(b, sub_id(base, 443), "Cernes par lame", &s, &mut r.cernes, 1.0, 40.0);
            let s = format!("{} / m²", nombre_court(r.noeuds));
            cx.slider(b, sub_id(base, 444), "Nœuds", &s, &mut r.noeuds, 0.0, 20.0);
        }
        if r.genre == Sorte::Vase {
            let s = format!("{:.0} / m²", r.noeuds);
            cx.slider(b, sub_id(base, 445), "Gravillons", &s, &mut r.noeuds, 0.0, 200.0);
        }
        cx.section(b, "Surface");
        let s = pc(r.variation);
        cx.slider(b, sub_id(base, 450), "Variation", &s, &mut r.variation, 0.0, 1.0);
        let s = pc(r.grain);
        cx.slider(b, sub_id(base, 451), "Grain", &s, &mut r.grain, 0.0, 1.0);
        if cx.button(b, sub_id(base, 452), &format!("Autre tirage (n° {})", r.graine)) {
            r.graine = r.graine.wrapping_add(1);
        }
        for (k, (titre, t)) in [("Couleur", &mut r.couleur), ("Seconde couleur", &mut r.couleur2), ("Joints", &mut r.couleur_joint)].into_iter().enumerate() {
            cx.section(b, titre);
            couleur(cx, b, sub_id(base, 500 + 10 * k), t);
        }
        r.borner();
    }
}

/// Trois curseurs : teinte, saturation, luminosité, et un aperçu de la couleur.
fn couleur(cx: &mut Cx, b: &mut Body, base: Id, t: &mut Tsl) {
    let [rr, g, bb] = t.rvb();
    let h = cx.commande() * 0.5;
    let r = Rect::new(b.x, b.y, b.w, h);
    cx.dl.rrect(r, cx.theme.rayon(h), [(rr * 255.0) as u8, (g * 255.0) as u8, (bb * 255.0) as u8, 255]);
    b.y += h + cx.ecart();
    let s = format!("{:.0}°", t.t);
    cx.slider(b, sub_id(base, 0), "Teinte", &s, &mut t.t, 0.0, 360.0);
    let s = format!("{:.0} %", t.s * 100.0);
    cx.slider(b, sub_id(base, 1), "Saturation", &s, &mut t.s, 0.0, 1.0);
    let s = format!("{:.0} %", t.l * 100.0);
    cx.slider(b, sub_id(base, 2), "Luminosité", &s, &mut t.l, 0.0, 1.0);
}

impl Contexte for Matieres {
    fn cle(&self) -> &'static str {
        CLE
    }

    fn dessiner(&mut self, cx: &mut Cx, c: &mut Commun, p: &Placee) -> String {
        let base = sub_id(MATIERES, p.id as usize);
        if !self.listee {
            self.lister(c);
            if let (Some(n), None) = (self.nom.clone(), &self.reglages) {
                if self.choisir(c, &n).is_err() {
                    self.nom = None;
                }
            }
        }
        self.suivre(c);
        let r = p.contenu;
        let d = cx.dp;
        let (zone, panneau) = if r.w > r.h * 1.1 {
            let w = (340.0 * d).min(r.w * 0.42);
            (Rect::new(r.x, r.y, r.w - w, r.h), Rect::new(r.right() - w, r.y, w, r.h))
        } else {
            let h = (r.h * 0.45).max(160.0 * d).min(r.h * 0.6);
            (Rect::new(r.x, r.y, r.w, r.h - h), Rect::new(r.x, r.bottom() - h, r.w, h))
        };
        self.vue(cx, zone);
        self.panneau(cx, c, panneau, base);
        // Enregistré une fois le doigt levé
        if cx.ui.doigts() == 0 && self.reglages.is_some() && self.reglages != self.enregistre {
            if let Err(e) = self.enregistrer(c) {
                self.message = Some(format!("non enregistrée : {e}"));
            }
        }
        match (&self.nom, &self.reglages) {
            (Some(n), Some(r)) => format!("{n} · {}", r.resume()),
            _ => "Matières".into(),
        }
    }

    fn dupliquer(&self) -> Box<dyn Contexte> {
        let mut e = Etat::default();
        self.ecrire(&mut e);
        ouvrir(&e)
    }

    fn ecrire(&self, e: &mut Etat) {
        if let Some(n) = &self.nom {
            e.mettre("matiere", n);
        }
        e.mettre("onglet", self.onglet);
        e.mettre("mosaique", self.mosaique);
    }

    fn anime(&self, _c: &Commun) -> bool {
        self.fabrication.is_some()
    }

    fn piloter(&mut self, m: &[String], c: &mut Commun) -> Option<Result<String, String>> {
        Some(match m[0].as_str() {
            "matieres" => {
                self.lister(c);
                Ok(self.liste.join("\n"))
            }
            "matiere" => {
                let Some(n) = m.get(1) else { return Some(Err("matiere <nom>".into())) };
                self.choisir(c, n).map(|_| format!("matiere {n}"))
            }
            "nouvelle" => {
                let Some(s) = m.get(1).and_then(|g| Sorte::de(g)) else {
                    return Some(Err(format!("nouvelle <{}> [nom]", Sorte::TOUS.map(|s| s.id()).join("|"))));
                };
                self.creer(c, s, m.get(2).map(|s| s.as_str())).map(|n| format!("matiere {n}"))
            }
            // Une clé du fichier de réglages, mise à cette valeur
            "regler" => {
                let Some(r) = &self.reglages else { return Some(Err("aucune matière ouverte".into())) };
                if m.len() < 3 {
                    return Some(Err("regler <clé> <valeur…>".into()));
                }
                let texte = format!("{}{} = {}\n", r.ecrire(), m[1], m[2..].join(" "));
                match Reglages::lire(&texte) {
                    Some(n) => {
                        self.reglages = Some(n);
                        let _ = self.enregistrer(c);
                        Ok(self.reglages.as_ref().map(|r| r.resume()).unwrap_or_default())
                    }
                    None => Err("réglages illisibles".into()),
                }
            }
            "mosaique" => match m.get(1).and_then(|s| s.parse::<u32>().ok()) {
                Some(n) if (1..=6).contains(&n) => {
                    self.mosaique = n;
                    Ok(format!("mosaique {n}"))
                }
                _ => Err("mosaique 1..6".into()),
            },
            // L'image est-elle à jour ?
            "image" => Ok(if self.fabrication.is_some() { "en cours".into() } else if self.image.is_some() { "prete".into() } else { "aucune".into() }),
            _ => return None,
        })
    }
}
