//! Ce qui se rejoint près d'un point : chaque route du lieu (hauteurs brutes, telles
//! qu'écrites) et le relief, puis les arêtes du réseau brut et nivelé.
//!   cargo run --release --example noeud -- <lieu.reel> x z [r]
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let l = sage_reel::Lieu::lire(std::path::Path::new(&a[1])).expect("lieu");
    let (x, z): (f32, f32) = (a[2].parse().unwrap(), a[3].parse().unwrap());
    let d: f32 = a.get(4).and_then(|s| s.parse().ok()).unwrap_or(3.0);
    println!("relief {:.1}", l.relief(x as f64, z as f64));
    for (k, r) in l.routes.iter().enumerate() {
        for (i, p) in r.points.iter().enumerate() {
            if (p[0] - x).hypot(p[1] - z) < d {
                let prof: Vec<String> = r.points.iter().map(|q| format!("{:.1}/{:.1}", q[2], l.relief(q[0] as f64, q[1] as f64))).collect();
                println!("route {k} genre {} pt {i}/{} : {}", r.genre, r.points.len(), prof.join(" "));
            }
        }
    }
    let mut n = flow::reseau::reseau(&l);
    let c = flow::reseau::contraintes(&n, &l);
    flow::reseau::niveler(&mut n, &c, 50);
    for k in &c {
        let h = n.point(k.haut);
        if (h[0] - x).hypot(h[1] - z) < 40.0 {
            println!("contrainte {:?} haut #{} à ({:.0};{:.0}) h {:.1} bas {:?} écart {} implicite {} manque {:.1}", k.genre, k.haut.0, h[0], h[1], h[2], k.bas.map(|b| (b.0, n.hauteur(b))), k.ecart, k.implicite, n.manque(k));
        }
    }
    for (k, e) in n.aretes.iter().enumerate() {
        for (i, p) in e.points.iter().enumerate() {
            if (i == 0 || i == e.points.len() - 1) && (p[0] - x).hypot(p[1] - z) < d {
                let prof: Vec<String> = e.points.iter().map(|q| format!("{:.1}", q[2])).collect();
                println!("nivelé #{k} {:?} : {}", e.passage, prof.join(" "));
            }
        }
    }
}
