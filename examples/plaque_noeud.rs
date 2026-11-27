//! Les profils des voies d'un nœud après le nivellement et les plaques de carrefour :
//!   cargo run --release --example plaque_noeud -- <lieu.reel> n   (ou x z)
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let l = sage_reel::Lieu::lire(std::path::Path::new(&a[1])).expect("lieu");
    let mut r = flow::reseau::nivele(&l);
    // Un numéro de nœud, ou « x z » (le nœud le plus proche)
    let n: usize = if a.len() > 3 {
        let (x, z): (f32, f32) = (a[2].parse().unwrap(), a[3].parse().unwrap());
        (0..r.noeuds.len()).min_by(|&i, &j| (r.noeuds[i].p[0] - x).hypot(r.noeuds[i].p[1] - z).total_cmp(&(r.noeuds[j].p[0] - x).hypot(r.noeuds[j].p[1] - z))).unwrap()
    } else {
        a[2].parse().unwrap()
    };
    let avant = r.clone();
    let b = flow::plaques::poser(&mut r);
    println!("{b:?}");
    for (titre, r) in [("avant", &avant), ("après", &r)] {
        let nd = &r.noeuds[n];
        println!("{titre} : nœud {n} ({:.1};{:.1}) h {:.2}", nd.p[0], nd.p[1], nd.p[2]);
        for &k in &nd.aretes {
            let e = &r.aretes[k];
            let q: Vec<[f32; 3]> = if e.a == n { e.points.clone() } else { e.points.iter().rev().copied().collect() };
            let mut c = 0.0f32;
            let mut s = String::new();
            for j in 0..q.len() {
                if j > 0 { c += (q[j][0] - q[j - 1][0]).hypot(q[j][1] - q[j - 1][1]); }
                if c <= 30.0 { s += &format!(" {:.0}:{:.2}", c, q[j][2]); }
            }
            println!("  a{k} genre {} l {:.1} :{s}", e.genre, e.largeur);
        }
    }
}
