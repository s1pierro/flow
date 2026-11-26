//! Les arêtes du réseau qui passent près d'un point : genre, passage, largeur, niveaux.
//!   cargo run --release --example aretes_autour -- <lieu.reel> x z r
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let l = sage_reel::Lieu::depuis_octets(&std::fs::read(&a[1]).unwrap()).unwrap();
    let r = flow::reseau::reseau(&l);
    let (x, z, d): (f32, f32, f32) = (a[2].parse().unwrap(), a[3].parse().unwrap(), a[4].parse().unwrap());
    for (k, e) in r.aretes.iter().enumerate() {
        if e.points.iter().any(|p| (p[0] - x).hypot(p[1] - z) < d) {
            let p0 = e.points[0]; let p1 = e.points[e.points.len() - 1];
            println!("#{k} genre {} {:?} l {:.1} ({:.0};{:.0};{:.1}) → ({:.0};{:.0};{:.1}) {} pts", e.genre, e.passage, e.largeur, p0[0], p0[1], p0[2], p1[0], p1[1], p1[2], e.points.len());
        }
    }
}
