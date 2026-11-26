//! L'origine des nœuds du réseau, telle que la construction l'a tracée
//! (`crate::reseau::Origine`) : un point des données (les tracés du lieu : OSM,
//! coupé aux ponts et aux tunnels, ou retouché), le bord de la carte, ou un point que
//! le réseau a ajouté (raccord d'un bout libre, croisement à niveau).

use crate::reseau::{Origine, Reseau};

/// Le nœud `n` est un point des données téléchargées.
pub fn osm(r: &Reseau, n: usize) -> bool {
    r.noeuds[n].origine == Origine::Donnees
}

/// L'origine du nœud `n`, en une ligne de l'encart.
pub fn texte(r: &Reseau, n: usize) -> String {
    let nd = &r.noeuds[n];
    match nd.origine {
        Origine::Donnees => {
            let passages: std::collections::HashSet<_> = nd.aretes.iter().map(|&k| r.aretes[k].passage).collect();
            if passages.len() > 1 {
                "Origine : OSM, passage du sol à un pont ou un tunnel".into()
            } else if nd.aretes.len() == 1 {
                "Origine : OSM, bout de voie".into()
            } else {
                format!("Origine : OSM, point commun à {} voies", nd.aretes.len())
            }
        }
        Origine::Bord => "Origine : bord de la carte (voie coupée)".into(),
        Origine::Raccord => format!("Origine : réseau, bout libre raccordé à moins de {:.0} m (point ajouté)", crate::reseau::RACCORD),
        Origine::RaccordSommet => "Origine : réseau, bout libre raccordé sur un point existant".into(),
        Origine::Croisement => "Origine : réseau, croisement de deux voies sans point commun (point ajouté)".into(),
    }
}
