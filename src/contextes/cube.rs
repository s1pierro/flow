//! Le contexte Cube : une vue 3D, avec sa caméra. Les gestes la font tourner, zoomer
//! et se déplacer ; les réglages (couleur, champ de vision, rotation) sont communs à
//! toutes les fenêtres Cube (`Commun::atelier`).

use crate::contexte::{Commun, Contexte, Etat, Genre, Vue3d};
use crate::math;
use crate::surface::Placee;
use crate::ui::{id, sub_id, Gesture, Id};
use crate::widgets::{Cx, Icon};

pub const CLE: &str = "cube";
pub const GENRE: Genre = Genre { cle: CLE, nom: "Cube", icone: Icon::Cube, ouvrir };

const FIT: Id = id("recadrer");
const SPIN: Id = id("rotation");

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub yaw: f32,
    pub pitch: f32,
    pub dist: f32,
    pub target: [f32; 3],
}

pub const HOME: Camera = Camera { yaw: 0.65, pitch: 0.45, dist: 6.0, target: [0.0, 0.0, 0.0] };

#[derive(Clone)]
pub struct Cube {
    pub cam: Camera,
    /// Le dernier recadrage général vu (`Atelier::recadrage`).
    recadrage: u32,
}

fn ouvrir(e: &Etat) -> Box<dyn Contexte> {
    let cam = match e.nombres("camera")[..] {
        [yaw, pitch, dist, x, y, z] => Camera { yaw, pitch: pitch.clamp(-1.5, 1.5), dist: dist.clamp(1.8, 24.0), target: [x, y, z] },
        _ => HOME,
    };
    Box::new(Cube { cam, recadrage: 0 })
}

impl Contexte for Cube {
    fn cle(&self) -> &'static str {
        CLE
    }

    fn dupliquer(&self) -> Box<dyn Contexte> {
        Box::new(self.clone())
    }

    fn ecrire(&self, e: &mut Etat) {
        let c = &self.cam;
        // L'angle ramené à un tour, pour rester lisible après une longue rotation
        let yaw = c.yaw.rem_euclid(std::f32::consts::TAU);
        e.mettre("camera", format!("{yaw:.3} {:.3} {:.3} {:.3} {:.3} {:.3}", c.pitch, c.dist, c.target[0], c.target[1], c.target[2]));
    }

    fn avancer(&mut self, dt: f32, c: &Commun) {
        if c.atelier.recadrage != self.recadrage {
            self.recadrage = c.atelier.recadrage;
            self.cam = HOME;
        }
        if c.atelier.rotation {
            self.cam.yaw += c.atelier.vitesse.to_radians() * dt;
        }
    }

    fn anime(&self, c: &Commun) -> bool {
        c.atelier.rotation
    }

    fn geste(&mut self, g: &Gesture, p: &Placee, c: &Commun) {
        let fov = c.atelier.champ;
        let c = &mut self.cam;
        c.yaw -= g.orbit.0 * 0.008;
        c.pitch = (c.pitch + g.orbit.1 * 0.008).clamp(-1.5, 1.5);
        c.dist = (c.dist / g.zoom).clamp(1.8, 24.0);
        // Déplacement à l'échelle de l'image : le cube suit les doigts.
        let k = 2.0 * c.dist * (fov.to_radians() / 2.0).tan() / p.contenu.h.max(1.0);
        let (sy, cy, sp, cp) = (c.yaw.sin(), c.yaw.cos(), c.pitch.sin(), c.pitch.cos());
        let (right, up) = ([cy, 0.0, -sy], [-sy * sp, cp, -cy * sp]);
        for i in 0..3 {
            c.target[i] += -g.pan.0 * k * right[i] + g.pan.1 * k * up[i];
        }
    }

    fn vue3d(&self, p: &Placee, c: &Commun) -> Option<Vue3d> {
        if p.contenu.w < 1.0 || p.contenu.h < 1.0 {
            return None;
        }
        let cam = self.cam;
        let (sy, cy, sp, cp) = (cam.yaw.sin(), cam.yaw.cos(), cam.pitch.sin(), cam.pitch.cos());
        let eye = [cam.target[0] + cam.dist * sy * cp, cam.target[1] + cam.dist * sp, cam.target[2] + cam.dist * cy * cp];
        let view = math::look_at(eye, cam.target, [0.0, 1.0, 0.0]);
        // Le rapport suit la fenêtre : c'est lui qui « adapte » la 3D à la place.
        let proj = math::perspective(c.atelier.champ.to_radians(), p.contenu.w / p.contenu.h, 0.1, 100.0);
        let model = math::scale(c.atelier.taille);
        Some(Vue3d { rect: p.contenu, mvp: math::mul(&proj, &math::mul(&view, &model)) })
    }

    fn dessiner(&mut self, cx: &mut Cx, c: &mut Commun, p: &Placee) -> String {
        let k = p.id as usize;
        let r = p.contenu;
        let (gap, m, h) = (cx.ecart(), cx.coin(), cx.commande());
        let (w1, w2) = (cx.pill_width("Recadrer"), cx.pill_width("Rotation"));
        if r.h > 3.0 * h && r.w > w1 + w2 + gap + 2.0 * m {
            let x = r.cx() - (w1 + w2 + gap) / 2.0;
            let y = r.bottom() - h - m;
            if cx.pill(sub_id(FIT, k), x, y, "Recadrer", false) {
                self.cam = HOME;
            }
            if cx.pill(sub_id(SPIN, k), x + w1 + gap, y, "Rotation", c.atelier.rotation) {
                c.atelier.rotation = !c.atelier.rotation;
            }
        }
        format!("1 objet · 12 tri · n°{}", p.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_camera_se_relit() {
        let c = Cube { cam: Camera { yaw: 1.25, pitch: -0.5, dist: 9.0, target: [1.0, 2.0, -3.0] }, recadrage: 0 };
        let mut e = Etat::default();
        c.ecrire(&mut e);
        let mut e2 = Etat::default();
        ouvrir(&e).ecrire(&mut e2);
        assert_eq!(e, e2);
        assert!(e.texte("camera").unwrap().starts_with("1.250 -0.500 9.000"));
    }
}
