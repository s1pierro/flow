//! Mesure la voirie 3D d'un lieu : `cargo run --release --example voirie -- lieu.reel lat lon`.
use flow::carte::{Donnees, NIVEAU_MAX};
use flow::relief3d::{fabriquer, COTES, VOIRIE_CASE};
use std::time::Instant;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let t = Instant::now();
    let l = sage_reel::Lieu::lire(std::path::Path::new(&a[1])).expect("lieu");
    println!("lecture {:.2} s", t.elapsed().as_secs_f32());
    let t = Instant::now();
    let d = Donnees::new(l);
    d.attendre_voirie();
    println!("index + voirie {:.2} s : {} rubans, {} places", t.elapsed().as_secs_f32(), d.voirie().1.rubans.len(), d.voirie().1.places.len());
    let t = Instant::now();
    let _ = flow::voirie3d::Voirie::new(&d.lieu, &d.tabliers);
    println!("voirie seule {:.2} s", t.elapsed().as_secs_f32());
    let (la, lo): (f64, f64) = (a[2].parse().unwrap(), a[3].parse().unwrap());
    let (x, z) = d.lieu.vers_jeu(la, lo);
    let (x0, z0, c) = d.monde();
    let niv = (0..=NIVEAU_MAX).find(|&n| c / (1u32 << n) as f32 / COTES as f32 <= VOIRIE_CASE).unwrap();
    let cote = c / (1u32 << niv) as f32;
    let (i, j) = (((x as f32 - x0) / cote) as i32, ((z as f32 - z0) / cote) as i32);
    let t = Instant::now();
    let mut n = 0;
    for dj in -1..=1 {
        for di in -1..=1 {
            let (v, ind) = fabriquer(&d, (niv, i + di, j + dj));
            n += ind.len() / 3;
            let _ = v;
        }
    }
    // La rue la plus pentue (la plus grande pente moyenne d'un ruban de plus de 80 m)
    let vo = d.voirie().1;
    let mut pente = vo.rubans.iter().filter_map(|r| {
        let l: f32 = r.points.windows(2).map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1])).sum();
        (l > 80.0 && sage_reel::carrossable(r.genre)).then(|| ((r.points[r.points.len() - 1][2] - r.points[0][2]).abs() / l, r.points[r.points.len() / 2]))
    }).collect::<Vec<_>>();
    pente.sort_by(|a, b| b.0.total_cmp(&a.0));
    for p in pente.iter().take(5) {
        println!("pente {:.0} % en {:.0} {:.1} {:.0}", p.0 * 100.0, p.1[0], p.1[2], p.1[1]);
    }
    println!("9 tuiles de {cote:.0} m : {:.1} ms la tuile, {} triangles", t.elapsed().as_secs_f32() * 1000.0 / 9.0, n);
}
