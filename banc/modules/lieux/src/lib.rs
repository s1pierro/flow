//! Lieux sur le banc : les tuiles de la maquette fabriquées ici, par une copie de
//! Flow (la crate `flow`, en dépendance). Pour essayer une variante : la recopier ici
//! (depuis `flow::maquette`) et la modifier, la brancher dans `tuile`, construire
//! (`banc/construire.sh banc/modules/lieux`), puis `banc recharger` : Lieux refait ses
//! tuiles. Une fois éprouvée, elle est entérinée dans Flow.
//!
//! Commandes (la liste) : `etat`, `variante <nom>` (la fabrication utilisée, `flow` par défaut :
//! celle de Flow telle quelle).

use flow::carte::Donnees;
use flow_banc::{lieux, module, Module, ModuleLieux, Scene, TuileFaite};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

mod reseau;

/// Le dernier lieu reçu, pour les commandes (le diagnostic du réseau).
static LIEU: Mutex<Option<Arc<Donnees>>> = Mutex::new(None);

/// Les variantes de la fabrication des tuiles ; la première est celle de Flow.
const VARIANTES: &[&str] = &["flow", "nu"];
static VARIANTE: AtomicUsize = AtomicUsize::new(0);

struct Commandes {
    revision: u64,
    /// Le réseau construit (`reseau`), et la scène montrée : le réseau autour d'un point.
    reseau: Option<reseau::Reseau>,
    zone: (f32, f32, f32),
    scene: Scene,
}

impl Commandes {
    fn montrer(&mut self) {
        if let Some(r) = &self.reseau {
            self.scene = r.scene(self.zone.0, self.zone.1, self.zone.2);
            self.revision += 1;
        }
    }
}

impl Module for Commandes {
    fn nouveau() -> Self {
        Commandes { revision: 1, reseau: None, zone: (-217.0, 841.0, 400.0), scene: Scene::default() }
    }

    fn commande(&mut self, m: &[&str]) -> Result<String, String> {
        match m[0] {
            "etat" => {}
            "diagnostic" => {
                let d = LIEU.lock().unwrap().clone().ok_or("pas encore de lieu : banc lieux oui, Lieux ouvert en Maquette")?;
                return Ok(reseau::diagnostic(&d.lieu));
            }
            "reseau" => {
                let d = LIEU.lock().unwrap().clone().ok_or("pas encore de lieu : banc lieux oui, Lieux ouvert en Maquette")?;
                let t0 = std::time::Instant::now();
                let r = reseau::reseau(&d.lieu);
                let dt = t0.elapsed().as_secs_f32();
                let s = format!("{}\nconstruit en {:.2} s", r.resume(), dt);
                self.reseau = Some(r);
                self.montrer();
                return Ok(s);
            }
            "zone" => {
                let n: Vec<f32> = m[1..].iter().filter_map(|x| x.parse().ok()).collect();
                let [x, z, r] = n[..] else { return Err("zone <x> <z> <rayon>".into()) };
                self.zone = (x, z, r);
                self.montrer();
                return Ok(format!("zone {x} {z} {r} : {} triangles", self.scene.indices.len() / 3));
            }
            "variante" => {
                let n = m.get(1).ok_or(format!("variante <{}>", VARIANTES.join("|")))?;
                let k = VARIANTES.iter().position(|v| v == n).ok_or(format!("variante inconnue : {n} ({})", VARIANTES.join(", ")))?;
                VARIANTE.store(k, Ordering::Relaxed);
                return Ok(format!("variante {n} : banc recharger, ou banc lieux non puis oui, pour refaire les tuiles"));
            }
            x => return Err(format!("commande inconnue : {x} (etat, variante, diagnostic, reseau, zone)")),
        }
        Ok(format!("lieux sur le banc · variante {}", VARIANTES[VARIANTE.load(Ordering::Relaxed)]))
    }

    fn revision(&self) -> u64 {
        self.revision
    }

    fn scene(&self) -> Scene {
        self.scene.clone()
    }
}

struct Fabrique {
    donnees: Option<Arc<Donnees>>,
}

impl ModuleLieux for Fabrique {
    fn nouveau() -> Self {
        Fabrique { donnees: None }
    }

    fn lieu(&mut self, octets: &[u8]) -> Result<(), String> {
        let l = sage_reel::Lieu::depuis_octets(octets).ok_or("lieu illisible")?;
        let d = Arc::new(Donnees::new(l));
        *LIEU.lock().unwrap() = Some(d.clone());
        self.donnees = Some(d);
        Ok(())
    }

    fn tuile(&mut self, cle: (u8, i32, i32)) -> Result<TuileFaite, String> {
        let d = self.donnees.as_ref().ok_or("pas de lieu")?;
        let (sommets, indices, pixels) = match VARIANTE.load(Ordering::Relaxed) {
            1 => nu(d, cle),
            _ => {
                let (s, i, p, _) = flow::maquette::fabriquer_avec_defonce(d, cle);
                (s, i, p)
            }
        };
        Ok(TuileFaite { sommets, indices, pixels })
    }
}

/// La variante `nu` : le solide creusé de ses lits, sans remblai ni épaulement.
fn nu(d: &Donnees, cle: (u8, i32, i32)) -> (Vec<f32>, Vec<u32>, Vec<u8>) {
    use flow::booleens::Op;
    use flow::maquette as mq;
    let b = flow::carte::boite_tuile(d, cle);
    let def = mq::defonce(d, cle);
    let f = flow::dissolution::dissoudre(&mq::composer(mq::solide(d, cle), &[(Op::Difference, &def)]));
    let (v, ind) = mq::habiller(d, &f, b);
    (v, ind, mq::texture(d, b))
}

module!(Commandes);
lieux!(Fabrique);
