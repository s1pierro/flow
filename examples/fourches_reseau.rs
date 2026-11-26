//! Les fourches du réseau : un nœud au sol où une voie large se partage en deux voies
//! plus étroites qui partent presque dans le même sens.
//!   cargo run --release --example fourches -- <lieu.reel> [x z rayon]
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let l = sage_reel::Lieu::lire(std::path::Path::new(&a[1])).expect("lieu");
    let zone: Option<(f32, f32, f32)> = (a.len() >= 5).then(|| (a[2].parse().unwrap(), a[3].parse().unwrap(), a[4].parse().unwrap()));
    let r = flow::reseau::nivele(&l);
    let dir = |k: usize, n: usize| -> [f32; 2] {
        let e = &r.aretes[k];
        let (p, q) = if e.a == n { (e.points[0], e.points[1.min(e.points.len() - 1)]) } else { let m = e.points.len(); (e.points[m - 1], e.points[m.saturating_sub(2)]) };
        // un peu plus loin que le premier segment
        let mut q = q;
        let pts: Vec<[f32; 3]> = if e.a == n { e.points.clone() } else { e.points.iter().rev().cloned().collect() };
        for s in &pts {
            if (s[0] - p[0]).hypot(s[1] - p[1]) > 15.0 { q = *s; break; }
            q = *s;
        }
        let (x, z) = (q[0] - p[0], q[1] - p[1]);
        let d = x.hypot(z).max(1e-6);
        [x / d, z / d]
    };
    let mut n_f = 0;
    for (k, nd) in r.noeuds.iter().enumerate() {
        let ar: Vec<usize> = nd.aretes.iter().copied().filter(|&e| r.aretes[e].genre <= 2 && r.aretes[e].passage == flow::reseau::Passage::Sol).collect();
        if ar.len() != 3 || nd.aretes.len() != 3 {
            continue;
        }
        if let Some((x, z, d)) = zone { if (nd.p[0] - x).hypot(nd.p[1] - z) > d { continue; } }
        let w: Vec<f32> = ar.iter().map(|&e| r.aretes[e].largeur).collect();
        let ds: Vec<[f32; 2]> = ar.iter().map(|&e| dir(e, k)).collect();
        for t in 0..3 {
            let (b1, b2) = ((t + 1) % 3, (t + 2) % 3);
            let ang = (ds[b1][0] * ds[b2][0] + ds[b1][1] * ds[b2][1]).clamp(-1.0, 1.0).acos().to_degrees();
            let opp1 = (ds[t][0] * ds[b1][0] + ds[t][1] * ds[b1][1]) < -0.7;
            let opp2 = (ds[t][0] * ds[b2][0] + ds[t][1] * ds[b2][1]) < -0.7;
            if ang < 45.0 && opp1 && opp2 && w[t] > w[b1].max(w[b2]) + 1.0 {
                n_f += 1;
                println!("({:.0};{:.0}) tronc {:.1} m, branches {:.1} et {:.1} m, écart {:.0}°", nd.p[0], nd.p[1], w[t], w[b1], w[b2], ang);
            }
        }
    }
    println!("{n_f} fourches");
}
