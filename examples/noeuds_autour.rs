//! Les nœuds du réseau nivelé près d'un point : numéro, position, arêtes (genre,
//! passage, longueur, largeur).
//!   cargo run --release --example noeuds_autour -- <lieu.reel> x z r
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let l = sage_reel::Lieu::lire(std::path::Path::new(&a[1])).expect("lieu");
    let (x, z, d): (f32, f32, f32) = (a[2].parse().unwrap(), a[3].parse().unwrap(), a[4].parse().unwrap());
    let r = flow::reseau::nivele(&l);
    for (i, n) in r.noeuds.iter().enumerate() {
        if (n.p[0] - x).hypot(n.p[1] - z) > d {
            continue;
        }
        let ar: Vec<String> = n.aretes.iter().map(|&k| {
            let e = &r.aretes[k];
            let lg: f32 = e.points.windows(2).map(|s| (s[1][0] - s[0][0]).hypot(s[1][1] - s[0][1])).sum();
            format!("#{k} g{} {:?} {lg:.0}m l{:.1}", e.genre, e.passage, e.largeur)
        }).collect();
        println!("{i} ({:.1};{:.1};{:.1}) : {}", n.p[0], n.p[1], n.p[2], ar.join(", "));
    }
}
