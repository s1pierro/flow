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

/// Les variantes de la fabrication des tuiles ; la première est celle de Flow.
const VARIANTES: &[&str] = &["flow", "nu"];
static VARIANTE: AtomicUsize = AtomicUsize::new(0);

struct Commandes {
    revision: u64,
}

impl Module for Commandes {
    fn nouveau() -> Self {
        Commandes { revision: 1 }
    }

    fn commande(&mut self, m: &[&str]) -> Result<String, String> {
        match m[0] {
            "etat" => {}
            "variante" => {
                let n = m.get(1).ok_or(format!("variante <{}>", VARIANTES.join("|")))?;
                let k = VARIANTES.iter().position(|v| v == n).ok_or(format!("variante inconnue : {n} ({})", VARIANTES.join(", ")))?;
                VARIANTE.store(k, Ordering::Relaxed);
                return Ok(format!("variante {n} : banc recharger, ou banc lieux non puis oui, pour refaire les tuiles"));
            }
            x => return Err(format!("commande inconnue : {x} (etat, variante)")),
        }
        Ok(format!("lieux sur le banc · variante {}", VARIANTES[VARIANTE.load(Ordering::Relaxed)]))
    }

    fn revision(&self) -> u64 {
        self.revision
    }

    fn scene(&self) -> Scene {
        Scene::default()
    }
}

struct Fabrique {
    donnees: Option<Donnees>,
}

impl ModuleLieux for Fabrique {
    fn nouveau() -> Self {
        Fabrique { donnees: None }
    }

    fn lieu(&mut self, octets: &[u8]) -> Result<(), String> {
        let l = sage_reel::Lieu::depuis_octets(octets).ok_or("lieu illisible")?;
        self.donnees = Some(Donnees::new(l));
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
