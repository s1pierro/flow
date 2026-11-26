//! Les arêtes du réseau d'un lieu : files, largeur, bouts, et les tracés du lieu qui les
//! portent. Usage : voir_aretes <lieu.reel> <arête>…
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let l = sage_reel::Lieu::lire(std::path::Path::new(&a[1])).expect("lecture");
    let r = flow::reseau::reseau(&l);
    let traces = flow::reseau::traces(&l);
    for s in &a[2..] {
        let k: usize = s.trim_start_matches('a').parse().unwrap();
        let e = &r.aretes[k];
        let (p, q) = (e.points[0], e.points[e.points.len() - 1]);
        let porteurs: Vec<String> = traces.iter().enumerate().filter(|(_, t)| {
            let on = |x: [f32; 3]| t.route.points.iter().any(|y| (y[0] - x[0]).hypot(y[1] - x[1]) < 0.02);
            on(p) && on(q)
        }).map(|(i, t)| format!("{}#{} {:?}", t.passage.nom(), i, t.route.files)).collect();
        println!("a{k} {:?} largeur {} genre {} ({:.0},{:.0})→({:.0},{:.0}) ; tracés {}", e.files, e.largeur, e.genre, p[0], p[1], q[0], q[1], porteurs.join(" ; "));
    }
}
