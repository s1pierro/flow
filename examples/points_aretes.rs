//! Les points des arêtes du réseau d'un lieu, avec l'angle de chaque coude.
//! Usage : points_aretes <lieu.reel> <arête>…
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let l = sage_reel::Lieu::lire(std::path::Path::new(&a[1])).expect("lecture");
    let r = flow::reseau::reseau(&l);
    // Un point x,z : les arêtes du nœud le plus proche
    let mut ks: Vec<usize> = Vec::new();
    for s in &a[2..] {
        if let Some((x, z)) = s.split_once(',') {
            let (x, z): (f32, f32) = (x.parse().unwrap(), z.parse().unwrap());
            let n = (0..r.noeuds.len()).min_by(|&i, &j| { let d = |n: usize| (r.noeuds[n].p[0] - x).hypot(r.noeuds[n].p[1] - z); d(i).total_cmp(&d(j)) }).unwrap();
            println!("nœud n{n} ({:.2}, {:.2})", r.noeuds[n].p[0], r.noeuds[n].p[1]);
            ks.extend(r.noeuds[n].aretes.iter().copied());
        } else {
            ks.push(s.trim_start_matches('a').parse().unwrap());
        }
    }
    for k in ks {
        let e = &r.aretes[k];
        println!("a{k} n{}→n{} largeur {} files {:?}", e.a, e.b, e.largeur, e.files);
        let p = &e.points;
        let mut cumul = 0.0;
        for i in 0..p.len() {
            if i > 0 { cumul += (p[i][0] - p[i - 1][0]).hypot(p[i][1] - p[i - 1][1]); }
            let coude = if i > 0 && i + 1 < p.len() {
                let a1 = (p[i][1] - p[i - 1][1]).atan2(p[i][0] - p[i - 1][0]);
                let a2 = (p[i + 1][1] - p[i][1]).atan2(p[i + 1][0] - p[i][0]);
                let mut d = (a2 - a1).to_degrees();
                while d > 180.0 { d -= 360.0 } while d < -180.0 { d += 360.0 }
                format!("coude {d:+.1}°")
            } else { String::new() };
            println!("  {:>6.1} m  ({:.2}, {:.2})  {}", cumul, p[i][0], p[i][1], coude);
        }
    }
}
