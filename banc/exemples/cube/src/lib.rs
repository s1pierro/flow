//! Le module d'exemple du banc : une grille de cubes qui peut tourner.
//!
//! Commandes : `taille <m>`, `grille <n>`, `couleur <r> <v> <b>` (de 0 à 1),
//! `tourner <tours par seconde>` (0 : arrêt), `etat`.

use flow_banc::{module, Module, Scene};

struct Cubes {
    taille: f32,
    grille: u32,
    couleur: [f32; 3],
    vitesse: f32,
    angle: f32,
    revision: u64,
}

impl Cubes {
    fn change(&mut self) {
        self.revision += 1;
    }
}

fn nombre(m: &[&str], i: usize) -> Result<f32, String> {
    m.get(i).ok_or("valeur manquante")?.replace(',', ".").parse::<f32>().map_err(|_| format!("« {} » n'est pas un nombre", m[i]))
}

impl Module for Cubes {
    fn nouveau() -> Cubes {
        Cubes { taille: 1.0, grille: 6, couleur: [0.35, 0.8, 0.45], vitesse: 0.0, angle: 0.0, revision: 1 }
    }

    fn commande(&mut self, m: &[&str]) -> Result<String, String> {
        match m[0] {
            "taille" => self.taille = nombre(m, 1)?.clamp(0.01, 100.0),
            "grille" => self.grille = (nombre(m, 1)? as u32).clamp(1, 40),
            "couleur" => self.couleur = [nombre(m, 1)?, nombre(m, 2)?, nombre(m, 3)?].map(|c| c.clamp(0.0, 1.0)),
            "tourner" => self.vitesse = nombre(m, 1)?,
            "etat" => {}
            x => return Err(format!("commande inconnue : {x} (taille, grille, couleur, tourner, etat)")),
        }
        self.change();
        Ok(format!("cubes {}×{} de {} m, couleur {:?}, {} tour/s", self.grille, self.grille, self.taille, self.couleur, self.vitesse))
    }

    fn pas(&mut self, dt: f32) {
        if self.vitesse != 0.0 {
            self.angle += dt * self.vitesse * std::f32::consts::TAU;
            self.change();
        }
    }

    fn revision(&self) -> u64 {
        self.revision
    }

    fn scene(&self) -> Scene {
        let mut s = Scene::default();
        let n = self.grille as i32;
        let pas = self.taille * 1.6;
        let (sa, ca) = self.angle.sin_cos();
        let tourne = |p: [f32; 3], o: [f32; 3]| {
            let (x, z) = (p[0] - o[0], p[2] - o[2]);
            [o[0] + x * ca - z * sa, p[1], o[2] + x * sa + z * ca]
        };
        for i in 0..n {
            for j in 0..n {
                let o = [(i as f32 - (n - 1) as f32 / 2.0) * pas, self.taille / 2.0, (j as f32 - (n - 1) as f32 / 2.0) * pas];
                let h = self.taille / 2.0;
                let c = |k: usize| tourne([o[0] + if k & 1 == 0 { -h } else { h }, o[1] + if k & 2 == 0 { -h } else { h }, o[2] + if k & 4 == 0 { -h } else { h }], o);
                let teinte = [self.couleur[0] * (0.8 + 0.2 * (i as f32 / n as f32)), self.couleur[1], self.couleur[2] * (0.8 + 0.2 * (j as f32 / n as f32))];
                for f in [[0, 4, 6, 2], [1, 3, 7, 5], [0, 1, 5, 4], [2, 6, 7, 3], [0, 2, 3, 1], [4, 5, 7, 6]] {
                    s.polygone(&f.map(c), teinte);
                }
            }
        }
        s
    }
}

module!(Cubes);
