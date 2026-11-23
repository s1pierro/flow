//! Les plaques (carrefours, groupes, fourches) près d'un point : nœuds, boîte, bouches.
//! `cargo run --release --example plaques -- lieu.reel x z r`.
use flow::carte::Donnees;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let l = sage_reel::Lieu::lire(std::path::Path::new(&a[1])).expect("lieu");
    let (x, z, r): (f32, f32, f32) = (a[2].parse().unwrap(), a[3].parse().unwrap(), a[4].parse().unwrap());
    let d = Donnees::new(l);
    let c = d.champ_voirie();
    for car in c.carrefours_dans([x - r, z - r, x + r, z + r], 0.0) {
        println!("plaque {:?} aplanir {} boîte ({:.1};{:.1})–({:.1};{:.1}), {} points", car.noeuds, car.aplanir, car.boite[0], car.boite[1], car.boite[2], car.boite[3], car.contour.len());
        for (k, b) in &car.bouches {
            println!("  bouche {k} : axe ({:.1};{:.1}) h {:.2}, bords ({:.1};{:.1}) ({:.1};{:.1})", b.q[0], b.q[1], b.q[2], b.g[0], b.g[1], b.d[0], b.d[1]);
        }
    }
}
