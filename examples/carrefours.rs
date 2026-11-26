//! État des lieux des carrefours matérialisés :
//! `cargo run --release --example carrefours -- lieu.reel x z [rayon]`.
//! Choisit près de (x, z) un carrefour de chaque sorte (T, croix, Y aigu, groupe de
//! nœuds proches), matérialise sa tuile et compte ses triangles près du nœud.
use flow::carte::Donnees;
use flow::reseau::Passage;

const R: f32 = 12.0;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let l = sage_reel::Lieu::lire(std::path::Path::new(&a[1])).expect("lieu");
    let (cx, cz): (f32, f32) = (a[2].parse().unwrap(), a[3].parse().unwrap());
    let rayon: f32 = a.get(4).and_then(|s| s.parse().ok()).unwrap_or(600.0);
    let d = Donnees::new(l);
    let r = d.reseau();
    let niv = flow::relief3d::niveau_fin(&d);
    let (x0, z0, c) = d.monde();
    let t = c / (1u32 << niv) as f32;
    // Les carrefours : nœuds d'au moins trois routes au sol, hors nœuds de passage
    let routes = |n: usize| -> Vec<usize> { r.noeuds[n].aretes.iter().copied().filter(|&k| r.aretes[k].genre != sage_reel::PIETONNE).collect() };
    let carrefours: Vec<usize> = (0..r.noeuds.len())
        .filter(|&n| {
            let p = r.noeuds[n].p;
            let v = routes(n);
            (p[0] - cx).hypot(p[1] - cz) < rayon && v.len() >= 3 && v.iter().all(|&k| r.aretes[k].passage == Passage::Sol)
        })
        .collect();
    println!("{} carrefours d'au moins 3 routes à moins de {rayon} m", carrefours.len());
    let dir = |n: usize, k: usize| -> [f32; 2] {
        let ar = &r.aretes[k];
        let pts: Vec<[f32; 3]> = if ar.a == n { ar.points.clone() } else { ar.points.iter().rev().copied().collect() };
        let o = pts[0];
        let q = pts.iter().find(|p| (p[0] - o[0]).hypot(p[1] - o[1]) > 4.0).copied().unwrap_or(pts[pts.len() - 1]);
        let l = (q[0] - o[0]).hypot(q[1] - o[1]).max(1e-6);
        [(q[0] - o[0]) / l, (q[1] - o[1]) / l]
    };
    let sorte = |n: usize| -> &'static str {
        let p = r.noeuds[n].p;
        if carrefours.iter().any(|&m| m != n && (r.noeuds[m].p[0] - p[0]).hypot(r.noeuds[m].p[1] - p[1]) < 15.0) {
            return "groupe";
        }
        let v = routes(n);
        let mut ang: Vec<f32> = v.iter().map(|&k| { let d = dir(n, k); d[1].atan2(d[0]) }).collect();
        ang.sort_by(f32::total_cmp);
        let ecarts: Vec<f32> = (0..ang.len()).map(|i| { let e = ang[(i + 1) % ang.len()] - ang[i]; if e <= 0.0 { e + std::f32::consts::TAU } else { e } }).collect();
        let min = ecarts.iter().copied().fold(f32::MAX, f32::min).to_degrees();
        match v.len() {
            3 if min < 50.0 => "Y aigu",
            3 => "T",
            4 => "croix",
            _ => "5 et plus",
        }
    };
    let mut compte = std::collections::BTreeMap::new();
    for &n in &carrefours {
        *compte.entry(sorte(n)).or_insert(0) += 1;
    }
    println!("par sorte : {compte:?}");
    let champ = d.champ_voirie();
    let mut simples = std::collections::BTreeMap::new();
    for &n in &carrefours {
        if champ.simple(n) {
            *simples.entry(sorte(n)).or_insert(0) += 1;
        }
    }
    println!("d'une seule pièce (carrefours simples) : {simples:?}");
    // Un exemple de chaque sorte, le plus proche du point, loin des bords de sa tuile
    for s in ["T", "croix", "Y aigu", "groupe", "5 et plus"] {
        let mut v: Vec<usize> = carrefours.iter().copied().filter(|&n| sorte(n) == s).filter(|&n| {
            let p = r.noeuds[n].p;
            let (fx, fz) = ((p[0] - x0) / t, (p[1] - z0) / t);
            let (ex, ez) = ((fx - fx.floor()) * t, (fz - fz.floor()) * t);
            ex > R + 3.0 && ex < t - R - 3.0 && ez > R + 3.0 && ez < t - R - 3.0
        }).collect();
        v.sort_by(|&a, &b| { let (pa, pb) = (r.noeuds[a].p, r.noeuds[b].p); ((pa[0] - cx).hypot(pa[1] - cz)).total_cmp(&(pb[0] - cx).hypot(pb[1] - cz)) });
        for &n in v.iter().take(2) {
            let p = r.noeuds[n].p;
            let cle = (niv, ((p[0] - x0) / t).floor() as i32, ((p[1] - z0) / t).floor() as i32);
            let t0 = std::time::Instant::now();
            let (sv, ind, _) = flow::materialisation::tuile(&d, cle);
            let st = flow::relief3d::SOMMET;
            let pos = |k: u32| { let o = k as usize * st; [sv[o], sv[o + 1], sv[o + 2]] };
            // La matière : 5 + m × 0,1 (`maquette::matiere::sommet`) ; le sol nu en dessous (100)
            let mat = |k: u32| { let v = sv[k as usize * st + 8]; if v >= 4.95 { ((v - 5.0) * 10.0).round() as u16 } else { 100 } };
            let (mut tot, mut petits, mut fins, mut murs, mut chaussee) = (0, 0, 0, 0, 0);
            let mut par_mat = std::collections::BTreeMap::new();
            for tri in ind.chunks(3) {
                let [a, b, cc] = [pos(tri[0]), pos(tri[1]), pos(tri[2])];
                let g = [(a[0] + b[0] + cc[0]) / 3.0, (a[2] + b[2] + cc[2]) / 3.0];
                if (g[0] - p[0]).hypot(g[1] - p[1]) > R {
                    continue;
                }
                tot += 1;
                let m = mat(tri[0]);
                *par_mat.entry(m).or_insert(0) += 1;
                let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
                let w = [cc[0] - a[0], cc[1] - a[1], cc[2] - a[2]];
                let nrm = [u[1] * w[2] - u[2] * w[1], u[2] * w[0] - u[0] * w[2], u[0] * w[1] - u[1] * w[0]];
                let aire2 = (nrm[0] * nrm[0] + nrm[1] * nrm[1] + nrm[2] * nrm[2]).sqrt();
                let l = |p: [f32; 3], q: [f32; 3]| ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2) + (p[2] - q[2]).powi(2)).sqrt();
                let lmax = l(a, b).max(l(b, cc)).max(l(cc, a));
                if aire2 * 0.5 < 0.05 {
                    petits += 1;
                }
                // Fin : hauteur relative au plus grand côté sous 5 %
                if lmax > 1e-6 && aire2 / lmax < 0.05 * lmax {
                    fins += 1;
                }
                let vertical = aire2 > 1e-9 && (nrm[1] / aire2).abs() < 0.3;
                if vertical && m == flow::maquette::matiere::BETON {
                    murs += 1;
                }
                if m == flow::maquette::matiere::CHAUSSEE {
                    chaussee += 1;
                }
            }
            let v = routes(n);
            let larg: Vec<String> = v.iter().map(|&k| format!("{:.1}", r.aretes[k].largeur)).collect();
            println!("\n{s}{} : nœud {n} ({:.1}, {:.1}, h {:.2}), {} routes (largeurs {}), tuile {:?} en {:.1} s", if champ.simple(n) { " (d'une pièce)" } else { "" }, p[0], p[1], p[2], v.len(), larg.join(" "), cle, t0.elapsed().as_secs_f32());
            println!("  à moins de {R} m : {tot} triangles, {chaussee} de chaussée, {petits} petits (< 0,05 m²), {fins} très fins, {murs} murs de béton ; par matière {par_mat:?}");
            println!("  plaques maillées en anneaux (repli) jusqu'ici : {}", flow::materialisation::PLAQUES_REPLI.load(std::sync::atomic::Ordering::Relaxed));
            println!("  pilote : vue3d 0 1.0 40 {:.1} {:.1} {:.1}", p[0], p[2], p[1]);
        }
    }
}
