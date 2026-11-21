//! Les groupes de nœuds de carrefour proches : `cargo run --release --example groupes --
//! lieu.reel [lat lon]...`. Statistiques sur toute la carte, puis le détail autour de
//! chaque point donné en degrés.
use flow::carte::Donnees;

const PROCHE: f32 = 15.0;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let l = sage_reel::Lieu::lire(std::path::Path::new(&a[1])).expect("lieu");
    let d = Donnees::new(l);
    let r = d.reseau();
    let champ = d.champ_voirie();
    let routes = |n: usize| -> Vec<usize> { r.noeuds[n].aretes.iter().copied().filter(|&k| r.aretes[k].genre != sage_reel::PIETONNE).collect() };
    // Les nœuds de carrefour : au moins deux routes, pas un passage
    let carrefour = |n: usize| { let v = routes(n); v.len() >= 3 || (v.len() == 2 && flow::champ_voirie::passage_simple(&r, n).is_none()) };
    let noeuds: Vec<usize> = (0..r.noeuds.len()).filter(|&n| carrefour(n)).collect();
    // Groupes par union-find à moins de PROCHE m (grille de 16 m)
    let mut parent: Vec<usize> = (0..noeuds.len()).collect();
    fn racine(p: &mut Vec<usize>, i: usize) -> usize { let mut i = i; while p[i] != i { p[i] = p[p[i]]; i = p[i]; } i }
    let mut grille: std::collections::HashMap<(i32, i32), Vec<usize>> = std::collections::HashMap::new();
    for (i, &n) in noeuds.iter().enumerate() {
        let p = r.noeuds[n].p;
        grille.entry(((p[0] / 16.0).floor() as i32, (p[1] / 16.0).floor() as i32)).or_default().push(i);
    }
    for (i, &n) in noeuds.iter().enumerate() {
        let p = r.noeuds[n].p;
        let (cx, cz) = ((p[0] / 16.0).floor() as i32, (p[1] / 16.0).floor() as i32);
        for x in cx - 1..=cx + 1 { for z in cz - 1..=cz + 1 {
            for &j in grille.get(&(x, z)).into_iter().flatten() {
                let q = r.noeuds[noeuds[j]].p;
                if j != i && (p[0] - q[0]).hypot(p[1] - q[1]) < PROCHE {
                    let (ri, rj) = (racine(&mut parent, i), racine(&mut parent, j));
                    parent[ri] = rj;
                }
            }
        }}
    }
    let mut groupes: std::collections::HashMap<usize, Vec<usize>> = std::collections::HashMap::new();
    for i in 0..noeuds.len() { let g = racine(&mut parent, i); groupes.entry(g).or_default().push(noeuds[i]); }
    let mut tailles = std::collections::BTreeMap::new();
    let mut simples = 0;
    let mut en_plaque = 0;
    for g in groupes.values() {
        *tailles.entry(g.len().min(10)).or_insert(0) += 1;
        if g.len() == 1 && champ.simple(g[0]) { simples += 1; }
        if g.len() >= 2 && g.iter().all(|&n| champ.simple(n)) { en_plaque += 1; }
    }
    println!("{} nœuds de carrefour, {} groupes (à moins de {PROCHE} m) ; par taille (10 = 10 et plus) : {tailles:?} ; seuls et d'une pièce : {simples} ; groupes en une plaque : {en_plaque}", noeuds.len(), groupes.len());
    // Quelques groupes en une plaque près du centre (807, 804)
    let mut faits: Vec<&Vec<usize>> = groupes.values().filter(|g| g.len() >= 2 && g.iter().all(|&n| champ.simple(n))).collect();
    let centre = |g: &Vec<usize>| { let n = g.len() as f32; let (x, z) = g.iter().fold((0.0, 0.0), |a, &k| (a.0 + r.noeuds[k].p[0], a.1 + r.noeuds[k].p[1])); (x / n, z / n) };
    faits.sort_by(|a, b| { let (ca, cb) = (centre(a), centre(b)); (ca.0 - 807.0).hypot(ca.1 - 804.0).total_cmp(&(cb.0 - 807.0).hypot(cb.1 - 804.0)) });
    for g in faits.iter().take(6) {
        let (x, z) = centre(g);
        println!("groupe en plaque de {} nœuds en ({x:.1}, {z:.1}), h {:.1}", g.len(), r.noeuds[g[0]].p[2]);
    }
    let mut i = 2;
    while i + 1 < a.len() {
        let (lat, lon): (f64, f64) = (a[i].parse().unwrap(), a[i + 1].parse().unwrap());
        i += 2;
        let (x, z) = d.lieu.vers_jeu(lat, lon);
        let (x, z) = (x as f32, z as f32);
        println!("\n({lat}, {lon}) → x {x:.1} z {z:.1}, sol {:.1}", d.lieu.relief(x as f64, z as f64));
        let mut v: Vec<usize> = noeuds.iter().copied().filter(|&n| { let p = r.noeuds[n].p; (p[0] - x).hypot(p[1] - z) < 60.0 }).collect();
        v.sort_by(|&p, &q| { let (a, b) = (r.noeuds[p].p, r.noeuds[q].p); (a[0] - x).hypot(a[1] - z).total_cmp(&(b[0] - x).hypot(b[1] - z)) });
        for &n in &v {
            let p = r.noeuds[n].p;
            let g = groupes.values().find(|g| g.contains(&n)).map_or(0, |g| g.len());
            let w: Vec<String> = routes(n).iter().map(|&k| {
                let ar = &r.aretes[k];
                let pts: Vec<[f32; 3]> = if ar.a == n { ar.points.clone() } else { ar.points.iter().rev().copied().collect() };
                let q = pts.iter().find(|q| (q[0] - p[0]).hypot(q[1] - p[1]) > 4.0).copied().unwrap_or(pts[pts.len() - 1]);
                let ang = (q[1] - p[1]).atan2(q[0] - p[0]).to_degrees();
                let long: f32 = pts.windows(2).map(|s| (s[1][0] - s[0][0]).hypot(s[1][1] - s[0][1])).sum();
                format!("{:.1} m g{} {:.0}° l{:.0}", ar.largeur, ar.genre, ang, long)
            }).collect();
            println!("  nœud {n} ({:.1}, {:.1}, h {:.1}) à {:.0} m, groupe de {g}{} : {}", p[0], p[1], p[2], (p[0] - x).hypot(p[1] - z), if champ.simple(n) { ", d'une pièce" } else { "" }, w.join(" ; "));
        }
    }
}
