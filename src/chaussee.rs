//! La chaussée en volume dans la maquette, posée sur le réseau nivelé du lieu
//! (`sage_reel::reseau`, `Donnees::reseau`). Chaque arête au sol devient un ruban : le dessus à la hauteur
//! du profil, épais de `EPAISSEUR` ; au-dessus, le terrain est dégagé (la voie ne
//! s'enfouit pas). Les ponts et les tunnels viendront ensuite.

use crate::booleens::{Forme, Op};
use crate::maquette as mq;
use sage_reel::reseau::{Arete, Passage, Reseau};

/// L'épaisseur de la chaussée (m).
pub const EPAISSEUR: f32 = 0.5;
/// La hauteur du dégagement au-dessus de la chaussée (m).
const DEGAGE: f32 = 40.0;

/// Le ruban d'une polyligne (x, z, h) de demi-largeur `w`, de `h + bas` à `h + haut` :
/// un prisme fermé, faces dehors.
pub fn ruban(p: &[[f32; 3]], w: f32, bas: f32, haut: f32) -> Forme {
    let mut q: Vec<[f32; 3]> = Vec::with_capacity(p.len());
    for &x in p {
        if q.last().is_none_or(|l: &[f32; 3]| (l[0] - x[0]).hypot(l[1] - x[1]) > 0.05) {
            q.push(x);
        }
    }
    let n = q.len();
    if n < 2 {
        return Forme::default();
    }
    let normale = |a: [f32; 3], b: [f32; 3]| {
        let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
        let l = dx.hypot(dz).max(1e-6);
        (-dz / l, dx / l)
    };
    let mut f = Forme::default();
    for i in 0..n {
        let n0 = if i > 0 { normale(q[i - 1], q[i]) } else { normale(q[0], q[1]) };
        let n1 = if i + 1 < n { normale(q[i], q[i + 1]) } else { n0 };
        let (mut nx, mut nz) = (n0.0 + n1.0, n0.1 + n1.1);
        let l = nx.hypot(nz);
        let (nx0, nz0) = if l < 1e-3 { n0 } else { (nx / l, nz / l) };
        // L'onglet : la demi-largeur tenue le long des deux segments (borné)
        let c = (nx0 * n0.0 + nz0 * n0.1).max(0.5);
        nx = nx0 * w / c;
        nz = nz0 * w / c;
        let (x, z, h) = (q[i][0], q[i][1], q[i][2]);
        // 4 sommets par point : gauche haut, droite haut, gauche bas, droite bas
        f.sommets.push([x + nx, h + haut, z + nz]);
        f.sommets.push([x - nx, h + haut, z - nz]);
        f.sommets.push([x + nx, h + bas, z + nz]);
        f.sommets.push([x - nx, h + bas, z - nz]);
    }
    for i in 0..n as u32 - 1 {
        let (a, b) = (4 * i, 4 * (i + 1));
        f.faces.push(vec![a, a + 1, b + 1, b]); // dessus
        f.faces.push(vec![a + 2, b + 2, b + 3, a + 3]); // dessous
        f.faces.push(vec![a, b, b + 2, a + 2]); // gauche
        f.faces.push(vec![a + 1, a + 3, b + 3, b + 1]); // droite
    }
    let z = 4 * (n as u32 - 1);
    f.faces.push(vec![0, 2, 3, 1]);
    f.faces.push(vec![z, z + 1, z + 3, z + 2]);
    if f.volume() < 0.0 {
        f.faces.iter_mut().for_each(|p| p.reverse());
    }
    f
}

/// Les morceaux des arêtes au sol qui touchent la boîte `b` (élargie de `marge`) : les
/// suites de points dont un segment l'approche.
fn morceaux<'a>(r: &'a Reseau, b: [f32; 4], marge: f32) -> Vec<(&'a Arete, Vec<[f32; 3]>)> {
    let mut v = Vec::new();
    for a in r.aretes.iter().filter(|a| a.passage == Passage::Sol) {
        let mut cur: Vec<[f32; 3]> = Vec::new();
        for w in a.points.windows(2) {
            let (p, q) = (w[0], w[1]);
            let dedans = p[0].max(q[0]) >= b[0] - marge && p[0].min(q[0]) <= b[2] + marge && p[1].max(q[1]) >= b[1] - marge && p[1].min(q[1]) <= b[3] + marge;
            if dedans {
                if cur.is_empty() {
                    cur.push(p);
                }
                cur.push(q);
            } else if !cur.is_empty() {
                v.push((a, std::mem::take(&mut cur)));
            }
        }
        if !cur.is_empty() {
            v.push((a, cur));
        }
    }
    v
}

fn pave(a: [f32; 3], b: [f32; 3]) -> Forme {
    let p = |i: usize| [if i & 1 == 0 { a[0] } else { b[0] }, if i & 2 == 0 { a[1] } else { b[1] }, if i & 4 == 0 { a[2] } else { b[2] }];
    let mut f = Forme { sommets: (0..8).map(p).collect(), faces: vec![vec![0, 4, 6, 2], vec![1, 3, 7, 5], vec![0, 1, 5, 4], vec![2, 6, 7, 3], vec![0, 2, 3, 1], vec![4, 5, 7, 6]], matieres: Vec::new() };
    if f.volume() < 0.0 {
        f.faces.iter_mut().for_each(|p| p.reverse());
    }
    f
}

/// Ce que la dernière tuile a coûté : (rubans, étapes sautées, secondes).
pub static DERNIERE: std::sync::Mutex<(usize, usize, f32)> = std::sync::Mutex::new((0, 0, 0.0));

/// La tuile de Flow, puis la chaussée : le dégagement ôté, les rubans unis, le tout
/// coupé à la tuile.
pub fn tuile(d: &crate::carte::Donnees, cle: (u8, i32, i32)) -> (Vec<f32>, Vec<u32>, Vec<u8>, Forme) {
    let t0 = std::time::Instant::now();
    let r = {
        let _t = crate::suivi::tache("réseau");
        d.reseau()
    };
    let r: &Reseau = &r;
    let b = crate::carte::boite_tuile(d, cle);
    let def = {
        let _t = crate::suivi::tache("défonce");
        mq::defonce(d, cle)
    };
    let rem = {
        let _t = crate::suivi::tache("remblai");
        mq::remblai_morceaux(d, cle)
    };
    let epa = {
        let _t = crate::suivi::tache("épaulement");
        mq::epaulement_morceaux(d, cle)
    };
    let mut formes_deg = Vec::new();
    let mut formes_ch = Vec::new();
    let mut t_rubans = crate::suivi::tache("rubans");
    for (a, pts) in morceaux(r, b, 10.0) {
        let w = (a.largeur * 0.5).max(1.0);
        formes_deg.push(ruban(&pts, w, -0.01, DEGAGE).en(mq::matiere::REMBLAI));
        formes_ch.push(ruban(&pts, w, -EPAISSEUR, 0.0).en(mq::matiere::SOL));
    }
    t_rubans.detail(format!("{} rubans", formes_ch.len()));
    drop(t_rubans);
    let socle = mq::socle(d);
    let boite = pave([b[0], socle - 50.0, b[1]], [b[2], 2000.0, b[3]]);
    let mut etapes: Vec<(Op, &Forme)> = rem.iter().map(|m| (Op::Union, m)).collect();
    etapes.extend(epa.iter().map(|m| (Op::Difference, m)));
    etapes.push((Op::Difference, &def));
    etapes.extend(formes_deg.iter().map(|m| (Op::Difference, m)));
    etapes.extend(formes_ch.iter().map(|m| (Op::Union, m)));
    etapes.push((Op::Intersection, &boite));
    let solide = {
        let _t = crate::suivi::tache("solide");
        mq::solide(d, cle)
    };
    let mut t = crate::suivi::tache("booléens");
    let (f, sautees) = match crate::booleens::enchainer(&solide, &etapes) {
        Ok(x) => x,
        Err(_) => (solide, etapes.len()),
    };
    t.detail(format!("{} étapes, {sautees} sautées", etapes.len()));
    drop(t);
    let f = {
        let _t = crate::suivi::tache("dissolution");
        crate::dissolution::dissoudre(&f)
    };
    let (v, ind) = {
        let _t = crate::suivi::tache("habillage");
        mq::habiller(d, &f, b)
    };
    let px = {
        let _t = crate::suivi::tache("texture");
        mq::texture(d, b)
    };
    *DERNIERE.lock().unwrap() = (formes_ch.len(), sautees, t0.elapsed().as_secs_f32());
    (v, ind, px, def)
}
