//! Les tunnels du réseau, les plus longs d'abord : longueur, genre, largeur, milieu.
//!   cargo run --release --example tunnels -- <lieu.reel>
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let l = sage_reel::Lieu::depuis_octets(&std::fs::read(&a[1]).unwrap()).unwrap();
    let r = flow::reseau::reseau(&l);
    let mut v: Vec<(f32, usize)> = r.aretes.iter().enumerate().filter(|(_, e)| e.passage == flow::reseau::Passage::Tunnel)
        .map(|(k, e)| (e.points.windows(2).map(|s| (s[1][0] - s[0][0]).hypot(s[1][1] - s[0][1])).sum(), k)).collect();
    v.sort_by(|a, b| b.0.total_cmp(&a.0));
    for (lg, k) in v.iter().take(25) {
        let e = &r.aretes[*k];
        let m = e.points[e.points.len() / 2];
        let creux = l.relief(m[0] as f64, m[1] as f64) as f32 - m[2];
        println!("#{k} {lg:.0} m genre {} l {:.1} milieu ({:.0} ; {:.0}) {:.1} m sous le relief", e.genre, e.largeur, m[0], m[1], creux);
    }
}
