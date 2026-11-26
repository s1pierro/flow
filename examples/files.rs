//! Les files des arêtes du réseau près d'un point.
//!   cargo run --release --example files -- <lieu.reel> x z r
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let l = sage_reel::Lieu::lire(std::path::Path::new(&a[1])).expect("lieu");
    let (x, z, d): (f32, f32, f32) = (a[2].parse().unwrap(), a[3].parse().unwrap(), a[4].parse().unwrap());
    let r = flow::reseau::nivele(&l);
    for (k, e) in r.aretes.iter().enumerate() {
        if e.genre == sage_reel::PIETONNE || !e.points.iter().any(|p| (p[0] - x).hypot(p[1] - z) < d) {
            continue;
        }
        let pts: Vec<String> = e.points.iter().map(|p| format!("({:.1};{:.1})", p[0], p[1])).collect();
        println!("#{k} genre {} l {:.1} {:?} : {}", e.genre, e.largeur, e.files, pts.join(" "));
    }
}
