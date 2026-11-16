//! Les scènes du réseau pour la vue du Banc.

use sage_reel::reseau::{Contrainte, Passage, Reseau};

/// Le réseau autour de (x, z), à moins de `rayon` mètres, en rubans colorés par
/// passage (le sol gris, les ponts orange, les tunnels violets) ; les bouts libres
/// marqués en rouge, les nœuds où le passage change en jaune.
pub fn scene(r: &Reseau, x: f32, z: f32, rayon: f32) -> flow_banc::Scene {
let s_ = r;
    let mut s = flow_banc::Scene::default();
    let dedans = |p: [f32; 3]| (p[0] - x).hypot(p[1] - z) < rayon;
    for a in &s_.aretes {
        let c = match a.passage {
            Passage::Sol => [0.62, 0.62, 0.66],
            Passage::Pont => [0.95, 0.6, 0.2],
            Passage::Tunnel => [0.6, 0.35, 0.85],
        };
        let w = (a.largeur * 0.5).max(0.8);
        for seg in a.points.windows(2) {
            let (p, q) = (seg[0], seg[1]);
            if !dedans(p) && !dedans(q) {
                continue;
            }
            let (dx, dz) = (q[0] - p[0], q[1] - p[1]);
            let l = dx.hypot(dz).max(1e-6);
            let (nx, nz) = (-dz / l * w, dx / l * w);
            let v = |m: [f32; 3], sx: f32| [m[0] + nx * sx, m[2], m[1] + nz * sx];
            s.polygone(&[v(p, -1.0), v(p, 1.0), v(q, 1.0), v(q, -1.0)], c);
            s.polygone(&[v(p, -1.0), v(q, -1.0), v(q, 1.0), v(p, 1.0)], c);
        }
    }
    for n in s_.noeuds.iter().filter(|n| dedans(n.p)) {
        let mut p: Vec<Passage> = n.aretes.iter().map(|&k| s_.aretes[k].passage).collect();
        p.sort_by_key(|x| *x as u8);
        p.dedup();
        let c = if n.aretes.len() == 1 {
            [0.9, 0.15, 0.15]
        } else if p.len() > 1 {
            [0.95, 0.85, 0.1]
        } else {
            continue;
        };
        let (h, y) = (2.0, n.p[2] + 0.3);
        s.polygone(&[[n.p[0] - h, y, n.p[1] - h], [n.p[0] - h, y, n.p[1] + h], [n.p[0] + h, y, n.p[1] + h], [n.p[0] + h, y, n.p[1] - h]], c);
        s.polygone(&[[n.p[0] - h, y, n.p[1] - h], [n.p[0] + h, y, n.p[1] - h], [n.p[0] + h, y, n.p[1] + h], [n.p[0] - h, y, n.p[1] + h]], c);
    }
    s
}

/// Les contraintes autour de (x, z) en barres debout : du bas (la voie dessous ou l'eau)
/// à la hauteur voulue, vertes si elles sont tenues, rouges sinon, une marque blanche à
/// la hauteur d'aujourd'hui.
pub fn scene_contraintes(r: &Reseau, v: &[Contrainte], x: f32, z: f32, rayon: f32, s: &mut flow_banc::Scene) {
    for c in v {
        let p = r.point(c.haut);
        if (p[0] - x).hypot(p[1] - z) > rayon {
            continue;
        }
        let bas = c.bas.map_or(c.plancher, |b| r.hauteur(b));
        let voulu = bas + c.ecart;
        let col = if r.manque(c) > 0.01 { [0.95, 0.2, 0.2] } else { [0.25, 0.85, 0.35] };
        let e = 0.5;
        let barre = |s: &mut flow_banc::Scene, y0: f32, y1: f32, col: [f32; 3], e: f32| {
            let q = |dx: f32, y: f32, dz: f32| [p[0] + dx, y, p[1] + dz];
            for (a, b) in [((-e, -e), (e, -e)), ((e, -e), (e, e)), ((e, e), (-e, e)), ((-e, e), (-e, -e))] {
                s.polygone(&[q(a.0, y0, a.1), q(b.0, y0, b.1), q(b.0, y1, b.1), q(a.0, y1, a.1)], col);
            }
        };
        barre(s, bas, voulu, col, e);
        let h = r.hauteur(c.haut);
        barre(s, h - 0.15, h + 0.15, [1.0, 1.0, 1.0], 1.2);
    }
}
