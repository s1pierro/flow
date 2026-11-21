//! Les nœuds de raccord à deux routes qui ne s'enchaînent pas (largeurs ou passages
//! différents) : `cargo run --release --example raccords -- lieu.reel [x z]`.
use flow::carte::Donnees;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let l = sage_reel::Lieu::lire(std::path::Path::new(&a[1])).expect("lieu");
    let d = Donnees::new(l);
    let r = d.reseau();
    let routes = |n: usize| -> Vec<usize> { r.noeuds[n].aretes.iter().copied().filter(|&k| r.aretes[k].genre != sage_reel::PIETONNE).collect() };
    let mut par_ecart = std::collections::BTreeMap::new();
    let mut deux = 0;
    let mut genres = std::collections::BTreeMap::new();
    let mut passages = 0;
    for n in 0..r.noeuds.len() {
        let v = routes(n);
        if v.len() != 2 || v[0] == v[1] {
            continue;
        }
        let (x, y) = (&r.aretes[v[0]], &r.aretes[v[1]]);
        let e = (x.largeur - y.largeur).abs();
        if e < 0.6 {
            continue;
        }
        deux += 1;
        *par_ecart.entry(format!("{:.1}", e)).or_insert(0) += 1;
        *genres.entry((x.genre.min(y.genre), x.genre.max(y.genre))).or_insert(0) += 1;
        if x.passage != y.passage {
            passages += 1;
        }
    }
    println!("nœuds à deux routes de largeurs différentes (≥ 0,6 m) : {deux}, dont {passages} entre deux passages différents");
    println!("par écart de largeur (m) : {par_ecart:?}");
    println!("par paire de genres : {genres:?}");
    let total2 = (0..r.noeuds.len()).filter(|&n| routes(n).len() == 2).count();
    println!("nœuds à deux routes en tout : {total2}");
    if a.len() >= 4 {
        let (cx, cz): (f32, f32) = (a[2].parse().unwrap(), a[3].parse().unwrap());
        let mut v: Vec<usize> = (0..r.noeuds.len()).filter(|&n| !routes(n).is_empty()).collect();
        v.sort_by(|&p, &q| { let (a, b) = (r.noeuds[p].p, r.noeuds[q].p); (a[0] - cx).hypot(a[1] - cz).total_cmp(&(b[0] - cx).hypot(b[1] - cz)) });
        for &n in v.iter().take(4) {
            let p = r.noeuds[n].p;
            let w: Vec<String> = routes(n).iter().map(|&k| format!("{:.1} m genre {} {:?}", r.aretes[k].largeur, r.aretes[k].genre, r.aretes[k].passage)).collect();
            println!("nœud {n} ({:.1}, {:.1}) à {:.1} m : {}", p[0], p[1], (p[0] - cx).hypot(p[1] - cz), w.join(" ; "));
        }
    }
}
