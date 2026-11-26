//! Le profil en long d'une suite d'arêtes du réseau nivelé : abscisse, x, z, hauteur,
//! relief.
//!   cargo run --release --example profil -- <lieu.reel> k1 k2 …
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let l = sage_reel::Lieu::lire(std::path::Path::new(&a[1])).expect("lieu");
    let r = flow::reseau::nivele(&l);
    for k in &a[2..] {
        let k: usize = k.parse().unwrap();
        let e = &r.aretes[k];
        println!("#{k} {:?} genre {} ({} → {})", e.passage, e.genre, e.a, e.b);
        let mut s = 0.0f32;
        for (i, p) in e.points.iter().enumerate() {
            if i > 0 {
                s += (p[0] - e.points[i - 1][0]).hypot(p[1] - e.points[i - 1][1]);
            }
            println!("  {s:6.1}  ({:7.1};{:7.1})  h {:6.2}  relief {:6.2}", p[0], p[1], p[2], l.relief(p[0] as f64, p[1] as f64));
        }
    }
}
