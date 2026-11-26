//! Les voies de bus (TEOR) : longueur, largeur, et l'écart entre axes des voies de bus
//! parallèles (les deux sens).
//!
//!   cargo run --release --example bus -- <lieu.reel>
use flow::reseau as rs;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let l = sage_reel::Lieu::depuis_octets(&std::fs::read(&a[1]).expect("lecture")).expect("lieu");
    let r = rs::reseau(&l);
    let bus: Vec<&rs::Arete> = r.aretes.iter().filter(|x| x.genre == sage_reel::BUS).collect();
    let long = |p: &[[f32; 3]]| p.windows(2).map(|s| (s[1][0] - s[0][0]).hypot(s[1][1] - s[0][1])).sum::<f32>();
    let total: f32 = bus.iter().map(|x| long(&x.points)).sum();
    println!("{} arêtes de bus, {:.0} m", bus.len(), total);
    let mut larg: std::collections::BTreeMap<i32, usize> = Default::default();
    for x in &bus {
        *larg.entry((x.largeur * 10.0) as i32).or_default() += 1;
    }
    println!("largeurs (dm) : {larg:?}");
    // Pour chaque point de bus, la voie de bus parallèle la plus proche (autre arête), son écart d'axe
    let mut ecarts: std::collections::BTreeMap<i32, usize> = Default::default();
    let mut exemples = Vec::new();
    for (i, x) in bus.iter().enumerate() {
        for w in x.points.windows(2) {
            let m = [(w[0][0] + w[1][0]) / 2.0, (w[0][1] + w[1][1]) / 2.0];
            let t = { let (dx, dz) = (w[1][0] - w[0][0], w[1][1] - w[0][1]); let n = dx.hypot(dz).max(1e-6); [dx / n, dz / n] };
            let mut best = f32::MAX;
            for (j, y) in bus.iter().enumerate() {
                if i == j { continue; }
                for v in y.points.windows(2) {
                    let (dx, dz) = (v[1][0] - v[0][0], v[1][1] - v[0][1]);
                    let l2 = dx * dx + dz * dz;
                    if l2 < 1e-6 || ((dx * t[0] + dz * t[1]) / l2.sqrt()).abs() < 0.94 { continue; }
                    let s = (((m[0] - v[0][0]) * dx + (m[1] - v[0][1]) * dz) / l2).clamp(0.0, 1.0);
                    best = best.min((m[0] - v[0][0] - s * dx).hypot(m[1] - v[0][1] - s * dz));
                }
            }
            if best < 30.0 {
                *ecarts.entry(best as i32).or_default() += 1;
                if exemples.len() < 12 && i % 7 == 0 { exemples.push((m, best)); }
            }
        }
    }
    println!("écarts d'axe (m) des segments à une voie de bus parallèle : {ecarts:?}");
    for (m, e) in exemples { println!("  ({:.0}, {:.0}) écart {:.1}", m[0], m[1], e); }
}
