//! Les chaînes de voies près d'un point : `cargo run --release --example tunnel -- lieu.reel x z [rayon]`.
use flow::carte::Donnees;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let l = sage_reel::Lieu::lire(std::path::Path::new(&a[1])).expect("lieu");
    let (x, z): (f32, f32) = (a[2].parse().unwrap(), a[3].parse().unwrap());
    let r: f32 = a.get(4).and_then(|s| s.parse().ok()).unwrap_or(40.0);
    let d = Donnees::new(l);
    let res = d.reseau();
    let champ = d.champ_voirie();
    println!("terrain au point : {:.2}", d.lieu.relief(x as f64, z as f64));
    for (k, c) in champ.chaines_dans([x - r, z - r, x + r, z + r], 0.0).into_iter().enumerate() {
        let pres: Vec<_> = c.points.iter().filter(|p| (p[0] - x).hypot(p[1] - z) < r).collect();
        if pres.is_empty() {
            continue;
        }
        println!("chaîne {k} : w {:.1}, {} points dont {} à moins de {r} m", c.w, c.points.len(), pres.len());
        for p in pres {
            let h = champ.h(p[0], p[1], p[2]);
            println!("  ({:7.1} {:7.1}) h {:6.2} passage {} champ {:?} terrain {:6.2}", p[0], p[1], p[2], p[5], h.map(|v| (v * 100.0).round() / 100.0), d.lieu.relief(p[0] as f64, p[1] as f64));
        }
    }
    for (k, a) in res.aretes.iter().enumerate() {
        if a.points.iter().any(|p| (p[0] - x).hypot(p[1] - z) < r) {
            let hs: Vec<String> = a.points.iter().filter(|p| (p[0] - x).hypot(p[1] - z) < r).map(|p| format!("{:.1}", p[2])).collect();
            println!("arête {k} {:?} genre {} largeur {:.1} nœuds {} {} : {}", a.passage, a.genre, a.largeur, a.a, a.b, hs.join(" "));
        }
    }
}
