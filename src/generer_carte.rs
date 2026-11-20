//! Les cartes : la zone de la Matérialisation, ou le lieu entier, écrits sur le disque
//! en carte (`sage-carte`), pour WhiteRoom et S.A.G.E. Lancé depuis la section Cartes du
//! panneau Voir, sans la vue Matérialisation : les tuiles sont faites une à une dans un
//! fil, sans limite de nombre (rien n'est gardé en mémoire). Chaque tuile est matérialisée
//! (`materialisation::tuile`), débarrassée de ce qui ne sert qu'à Flow (les volumes de
//! défonce montrés à part, les murs sur les bords de la tuile et son dessous, pour qu'une
//! roue ne bute pas d'une tuile à l'autre), rangée en une primitive par matière et
//! écrite dès qu'elle est faite : une génération arrêtée reprend où elle en était (les
//! tuiles déjà écrites sont relues pour l'index). Les mêmes triangles servent au rendu
//! et à la physique.

use crate::carte::{boite_tuile, Cle, Donnees};
use crate::maquette as mq;
use crate::relief3d::{tuiles_zone, ZoneFine, SOMMET};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};

/// Le sous-dossier de Flow où vont les cartes.
pub const DOSSIER: &str = "cartes";

/// L'avancement d'une génération.
#[derive(Clone, Debug, Default)]
pub struct Etat {
    pub nom: String,
    pub dossier: PathBuf,
    pub faites: usize,
    pub total: usize,
    /// Tuiles reprises d'une génération précédente.
    pub reprises: usize,
    pub triangles: usize,
    pub fini: bool,
    pub erreur: Option<String>,
    pub debut: Option<std::time::Instant>,
    pub duree: f32,
    /// Le temps passé par étape (aussi écrit dans `mesures.txt` de la carte).
    pub mesures: crate::materialisation::Mesures,
    /// Le renfort (`renfort`) : où il en est, et les tuiles qu'il a faites.
    pub renfort: Option<String>,
    pub par_renfort: usize,
}

/// Change l'état de la génération.
fn maj(f: impl FnOnce(&mut Etat)) {
    if let Ok(mut e) = ETAT.lock() {
        if let Some(e) = e.as_mut() {
            f(e);
        }
    }
}

/// La file des tuiles à faire, partagée avec le renfort : les tuiles, et combien le
/// renfort en a pris sans les avoir rendues.
type File = Arc<Mutex<(std::collections::VecDeque<Cle>, usize)>>;

/// Prend la prochaine tuile de la file (comptée en vol pour le renfort).
fn prendre(file: &File, renfort: bool) -> Option<Cle> {
    let mut f = file.lock().ok()?;
    let c = f.0.pop_front();
    if c.is_some() && renfort {
        f.1 += 1;
    }
    c
}

/// Écrit l'index (une carte partielle reste lisible).
fn indexer(carte: &Mutex<sage_carte::Carte>, dossier: &Path) -> Result<(), String> {
    let c = carte.lock().map_err(|_| "carte inaccessible")?;
    std::fs::write(dossier.join(sage_carte::INDEX), c.ecrire()).map_err(|e| e.to_string())
}

/// Écrit une tuile (par un fichier provisoire renommé ensuite).
fn ecrire_tuile(chemin: &Path, glb: &[u8]) -> Result<(), String> {
    let tmp = chemin.with_extension("glb.part");
    std::fs::write(&tmp, glb).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, chemin).map_err(|e| e.to_string())
}

/// Le renfort de la génération : en cherche un (au départ, puis toutes les
/// `renfort::RECHERCHE`), lui donne le lieu, puis lui fait faire des tuiles de la file
/// jusqu'à ce qu'elle soit vide. Une tuile perdue revient à la file.
#[allow(clippy::too_many_arguments)]
fn boucle_renfort(donnees: Arc<RwLock<Donnees>>, lieu: String, file: File, fini: Arc<AtomicBool>, carte: Arc<Mutex<sage_carte::Carte>>, dossier: PathBuf, memoire: Arc<crate::materialisation::Memoire>) {
    use crate::renfort;
    let attendre = |d: std::time::Duration| {
        let fin = std::time::Instant::now() + d;
        while std::time::Instant::now() < fin && !fini.load(Ordering::Relaxed) && !ARRET.load(Ordering::Relaxed) {
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
    };
    while !fini.load(Ordering::Relaxed) && !ARRET.load(Ordering::Relaxed) {
        if file.lock().map_or(true, |f| f.0.is_empty()) {
            return;
        }
        let Some((a, _)) = renfort::chercher() else {
            maj(|e| e.renfort = Some("aucun renfort (« Servir de renfort » dans la liste des Lieux de l'autre appareil)".into()));
            attendre(renfort::RECHERCHE);
            continue;
        };
        maj(|e| e.renfort = Some(format!("renfort trouvé : {}…", a.ip())));
        let lien = match donnees.read() {
            Ok(d) => renfort::Lien::ouvrir(a, &d, &lieu, |m| maj(|e| e.renfort = Some(m.to_string()))),
            Err(_) => return,
        };
        let mut lien = match lien {
            Ok(l) => l,
            Err(m) => {
                maj(|e| e.renfort = Some(m));
                attendre(renfort::RECHERCHE);
                continue;
            }
        };
        maj(|e| e.renfort = Some(format!("renfort : {}", a.ip())));
        while !ARRET.load(Ordering::Relaxed) {
            let Some(cle) = prendre(&file, true) else { return };
            let rendre = |file: &File, remettre: bool| {
                if let Ok(mut f) = file.lock() {
                    if remettre {
                        f.0.push_front(cle);
                    }
                    f.1 = f.1.saturating_sub(1);
                }
            };
            match lien.tuile(cle) {
                Ok(r) => {
                    let b = match donnees.read() {
                        Ok(d) => {
                            memoire.faite_ailleurs(&d, cle);
                            boite_tuile(&d, cle)
                        }
                        Err(_) => [0.0; 4],
                    };
                    let mut triangles = 0;
                    if let Some((hmin, hmax, n, glb)) = r {
                        if ecrire_tuile(&dossier.join(sage_carte::nom_tuile(cle)), &glb).is_err() {
                            rendre(&file, true);
                            return;
                        }
                        triangles = n as usize;
                        if let Ok(mut c) = carte.lock() {
                            c.tuiles.push(sage_carte::EntreeTuile { cle, boite: b, hmin, hmax, triangles: n });
                        }
                    }
                    maj(|e| {
                        e.faites += 1;
                        e.par_renfort += 1;
                        e.triangles += triangles;
                    });
                    let _ = indexer(&carte, &dossier);
                    rendre(&file, false);
                }
                Err(m) => {
                    rendre(&file, true);
                    maj(|e| e.renfort = Some(format!("renfort perdu ({m}) ; nouvelle recherche…")));
                    break;
                }
            }
        }
    }
}

/// Le bilan des mesures, écrit dans le dossier de la carte.
pub const MESURES: &str = "mesures.txt";

static ETAT: Mutex<Option<Etat>> = Mutex::new(None);
static ARRET: AtomicBool = AtomicBool::new(false);

/// L'avancement de la génération en cours ou de la dernière.
pub fn etat() -> Option<Etat> {
    ETAT.lock().ok().and_then(|e| e.clone())
}

/// Une génération tourne-t-elle ?
pub fn en_cours() -> bool {
    etat().is_some_and(|e| !e.fini)
}

/// Arrête la génération en cours (après la tuile commencée).
pub fn arreter() {
    ARRET.store(true, Ordering::Relaxed);
}

/// La zone qui couvre tout le lieu : son milieu, la demi-diagonale.
pub fn zone_entiere(d: &Donnees) -> ZoneFine {
    let [x0, z0, x1, z1] = d.lieu.etendue();
    let centre = [((x0 + x1) / 2.0) as f32, ((z0 + z1) / 2.0) as f32];
    ZoneFine { centre, rayon: ((x1 - x0).hypot(z1 - z0) / 2.0) as f32 + 1.0 }
}

/// Le nom de la carte du lieu entier.
pub fn nom_entiere(lieu: &str) -> String {
    format!("{lieu}-entier").replace(' ', "_")
}

/// Le nom d'une carte de la zone : lieu, rayon et centre.
pub fn nom_par_defaut(lieu: &str, z: ZoneFine) -> String {
    format!("{lieu}-{:.0}m-{:.0}-{:.0}", z.rayon, z.centre[0], z.centre[1]).replace(' ', "_")
}

/// Le nom d'une matière de la maquette (`maquette::matiere`), d'après le flottant
/// « matière » d'un sommet ; rien pour ce qui ne va pas dans la carte.
fn nom_matiere(f: f32) -> Option<&'static str> {
    if (f - mq::MAQUETTE_DEFONCE).abs() < 1e-3 {
        return None;
    }
    if (f - mq::MAQUETTE).abs() < 1e-3 {
        return Some("sol");
    }
    let m = ((f - 5.0) * 10.0).round() as i32;
    use mq::matiere::*;
    Some(match m as u16 {
        SOL => "sol",
        LIT => "lit",
        BERGE => "berge",
        REMBLAI => "remblai",
        QUAI => "quai",
        QUAI_MUR => "quai_mur",
        HAUT => "haut",
        CHAUSSEE => "chaussee",
        BETON => "beton",
        _ => "autre",
    })
}

/// Les matières de la carte : nom, texture (dans `Flow/matieres`, copiée), côté.
fn matieres(d: &Path) -> Vec<sage_carte::Matiere> {
    let cote = |nom: &str| {
        std::fs::read_to_string(d.join(format!("{nom}.txt"))).ok().and_then(|t| sage_matieres::Reglages::lire(&t)).map(|r| r.cote.max(0.05)).unwrap_or(1.0)
    };
    let mut v = vec![sage_carte::Matiere { nom: "sol".into(), texture: String::new(), cote: 0.0 }, sage_carte::Matiere { nom: sage_carte::EAU.into(), texture: String::new(), cote: 0.0 }];
    for (m, tex) in [("lit", "vase-lit"), ("berge", "taille-quai"), ("remblai", "pierre-quai"), ("quai", "paves-quai"), ("quai_mur", "pierre-quai"), ("chaussee", "bitume"), ("beton", "beton-clair")] {
        v.push(sage_carte::Matiere { nom: m.into(), texture: tex.into(), cote: cote(tex) });
    }
    v
}

/// La tuile `cle` en tuile de carte (rien si elle est vide).
pub(crate) fn tuile(d: &Donnees, cle: Cle, memoire: &crate::materialisation::Memoire) -> Option<sage_carte::Tuile> {
    let (v, ind, px) = crate::materialisation::tuile_carte(d, cle, memoire);
    let t0 = std::time::Instant::now();
    let b = boite_tuile(d, cle);
    let socle = mq::socle(d);
    let origine = [b[0], 0.0, b[1]];
    // Une primitive par matière, ses sommets renumérotés
    let mut prims: Vec<sage_carte::Primitive> = Vec::new();
    let mut par_nom: HashMap<&str, (usize, HashMap<u32, u32>)> = HashMap::new();
    let pos = |k: u32| [v[k as usize * SOMMET], v[k as usize * SOMMET + 1], v[k as usize * SOMMET + 2]];
    let pres = |a: f32, b: f32| (a - b).abs() < 0.01;
    for t in ind.chunks_exact(3) {
        let Some(nom) = nom_matiere(v[t[0] as usize * SOMMET + 8]) else { continue };
        let p = [pos(t[0]), pos(t[1]), pos(t[2])];
        // Les murs sur les bords de la tuile, et son dessous
        let sur_bord = [(0usize, b[0]), (0, b[2]), (2, b[1]), (2, b[3])].iter().any(|&(k, c)| p.iter().all(|q| pres(q[k], c)));
        let dessous = p.iter().all(|q| q[1] < socle + 0.01);
        if sur_bord || dessous {
            continue;
        }
        let (k, renum) = par_nom.entry(nom).or_insert_with(|| {
            prims.push(sage_carte::Primitive { matiere: nom.to_string(), ..Default::default() });
            (prims.len() - 1, HashMap::new())
        });
        let pr = &mut prims[*k];
        for &i in t {
            let j = *renum.entry(i).or_insert_with(|| {
                let s = &v[i as usize * SOMMET..][..SOMMET];
                pr.positions.push([s[0] - origine[0], s[1], s[2] - origine[2]]);
                pr.normales.push([s[3], s[4], s[5]]);
                pr.uv.push([s[6], s[7]]);
                (pr.positions.len() - 1) as u32
            });
            pr.indices.push(j);
        }
    }
    memoire.noter("rangement par matière", t0.elapsed().as_secs_f64());
    // La surface de l'eau (`maquette::surface_eau`), à part : elle ne fait pas de volume
    let (ev, ei) = mq::surface_eau(d, cle);
    if !ei.is_empty() {
        let mut e = sage_carte::Primitive { matiere: sage_carte::EAU.into(), ..Default::default() };
        for s in ev.chunks_exact(SOMMET) {
            e.positions.push([s[0] - origine[0], s[1], s[2] - origine[2]]);
            e.normales.push([s[3], s[4], s[5]]);
            e.uv.push([s[6], s[7]]);
        }
        e.indices = ei;
        prims.push(e);
    }
    if prims.is_empty() {
        return None;
    }
    let t0 = std::time::Instant::now();
    let image_sol = prims.iter().any(|p| p.matiere == "sol").then(|| sage_matieres::png::encoder(mq::TEX as u32, (mq::TEX + mq::BANDE) as u32, &px));
    memoire.noter("image du sol (PNG)", t0.elapsed().as_secs_f64());
    Some(sage_carte::Tuile { cle, origine, primitives: prims, image_sol })
}

/// L'entrée d'index d'une tuile.
pub(crate) fn entree(t: &sage_carte::Tuile, b: [f32; 4]) -> sage_carte::EntreeTuile {
    let (mut hmin, mut hmax) = (f32::MAX, f32::MIN);
    for p in &t.primitives {
        for q in &p.positions {
            hmin = hmin.min(q[1]);
            hmax = hmax.max(q[1]);
        }
    }
    sage_carte::EntreeTuile { cle: t.cle, boite: b, hmin, hmax, triangles: t.nombre_triangles() as u32 }
}

/// Lance la génération de la carte `nom` de la zone `z` dans `flow/cartes/nom`, dans un
/// fil ; `lieu` est le nom du lieu. Rien si une génération tourne déjà.
pub fn lancer(donnees: Arc<RwLock<Donnees>>, z: ZoneFine, flow: &Path, lieu: String, nom: String) {
    if en_cours() {
        return;
    }
    let dossier = flow.join(DOSSIER).join(&nom);
    let matieres_flow = flow.join("matieres");
    ARRET.store(false, Ordering::Relaxed);
    if let Ok(mut e) = ETAT.lock() {
        *e = Some(Etat { nom: nom.clone(), dossier: dossier.clone(), debut: Some(std::time::Instant::now()), ..Default::default() });
    }
    // L'appareil reste éveillé pendant la génération (rendu à la fin)
    crate::renfort::eveil(true);
    let _ = std::thread::Builder::new().name("carte".into()).spawn(move || {
        let r = (|| -> Result<(), String> {
            std::fs::create_dir_all(dossier.join(sage_carte::TUILES)).map_err(|e| e.to_string())?;
            std::fs::create_dir_all(dossier.join(sage_carte::MATIERES)).map_err(|e| e.to_string())?;
            let (cles, carte0, memoire) = {
                let d = donnees.read().map_err(|_| "données inaccessibles")?;
                let cles = tuiles_zone(&d, z);
                // Bâtiments montés et tracés lissés une fois pour toute la génération
                // (les tuiles déjà écrites, reprises, ne comptent pas)
                let a_faire: Vec<Cle> = cles.iter().copied().filter(|&k| !dossier.join(sage_carte::nom_tuile(k)).exists()).collect();
                let memoire = crate::materialisation::Memoire::pour(&d, &a_faire);
                let (_, _, c) = d.monde();
                let cote = c / (1u32 << crate::relief3d::niveau_fin(&d)) as f32;
                let h = d.lieu.relief(z.centre[0] as f64, z.centre[1] as f64) as f32;
                let carte = sage_carte::Carte { nom: nom.clone(), lieu: lieu.clone(), lat: d.lieu.lat, lon: d.lieu.lon, centre: z.centre, rayon: z.rayon, cote, depart: [z.centre[0], h + 2.0, z.centre[1]], matieres: matieres(&matieres_flow), tuiles: Vec::new() };
                (cles, carte, memoire)
            };
            maj(|e| e.total = cles.len());
            // Les matières : réglages et images copiés
            for m in &carte0.matieres {
                for ext in ["txt", "png"] {
                    let f = matieres_flow.join(format!("{}.{ext}", m.texture));
                    if !m.texture.is_empty() && f.exists() {
                        let _ = std::fs::copy(&f, dossier.join(sage_carte::MATIERES).join(format!("{}.{ext}", m.texture)));
                    }
                }
            }
            // Reprise : les tuiles déjà écrites sont relues ; les autres vont à la file
            let mut carte = carte0;
            let mut file = std::collections::VecDeque::new();
            for &cle in &cles {
                if ARRET.load(Ordering::Relaxed) {
                    return Err("arrêtée".into());
                }
                let chemin = dossier.join(sage_carte::nom_tuile(cle));
                match sage_carte::Tuile::ouvrir(&chemin) {
                    Ok(t) => {
                        let b = boite_tuile(&*donnees.read().map_err(|_| "données inaccessibles")?, cle);
                        let en = entree(&t, b);
                        maj(|e| {
                            e.faites += 1;
                            e.reprises += 1;
                            e.triangles += en.triangles as usize;
                        });
                        carte.tuiles.push(en);
                    }
                    Err(_) => file.push_back(cle),
                }
            }
            let carte = Arc::new(Mutex::new(carte));
            let file: File = Arc::new(Mutex::new((file, 0)));
            let memoire = Arc::new(memoire);
            // Le renfort, en parallèle
            let fini = Arc::new(AtomicBool::new(false));
            let aide = {
                let (donnees, lieu, file, fini, carte, dossier, memoire) = (donnees.clone(), lieu.clone(), file.clone(), fini.clone(), carte.clone(), dossier.clone(), memoire.clone());
                std::thread::Builder::new().name("carte-renfort".into()).spawn(move || boucle_renfort(donnees, lieu, file, fini, carte, dossier, memoire)).ok()
            };
            let r = (|| -> Result<(), String> {
                loop {
                    if ARRET.load(Ordering::Relaxed) {
                        return Err("arrêtée".into());
                    }
                    let Some(cle) = prendre(&file, false) else {
                        // La file est vide : attendre les tuiles encore chez le renfort
                        // (une tuile perdue revient à la file)
                        if file.lock().map_or(true, |f| f.1 == 0) {
                            return Ok(());
                        }
                        std::thread::sleep(std::time::Duration::from_millis(200));
                        continue;
                    };
                    let chemin = dossier.join(sage_carte::nom_tuile(cle));
                    let (t, b) = {
                        let d = donnees.read().map_err(|_| "données inaccessibles")?;
                        let _s = crate::suivi::tache("carte (tuile)");
                        (tuile(&d, cle, &memoire), boite_tuile(&d, cle))
                    };
                    if let Some(t) = t {
                        let en = entree(&t, b);
                        let t0 = std::time::Instant::now();
                        ecrire_tuile(&chemin, &sage_carte::glb::ecrire(&t))?;
                        memoire.noter("écriture du GLB", t0.elapsed().as_secs_f64());
                        maj(|e| e.triangles += en.triangles as usize);
                        if let Ok(mut c) = carte.lock() {
                            c.tuiles.push(en);
                        }
                    }
                    let m = memoire.mesures();
                    maj(|e| {
                        e.faites += 1;
                        e.mesures = m.clone();
                    });
                    indexer(&carte, &dossier)?;
                    let _ = std::fs::write(dossier.join(MESURES), m.texte());
                }
            })();
            fini.store(true, Ordering::Relaxed);
            if let Some(a) = aide {
                let _ = a.join();
            }
            indexer(&carte, &dossier)?;
            let _ = std::fs::write(dossier.join(MESURES), memoire.mesures().texte());
            r
        })();
        crate::renfort::eveil(false);
        maj(|e| {
            e.fini = true;
            e.duree = e.debut.map_or(0.0, |d| d.elapsed().as_secs_f32());
            if let Err(m) = &r {
                e.erreur = Some(m.clone());
            }
        });
    });
}
