//! Diagnostic des rubans de la Matérialisation de Flow 0.120 sur un lieu :
//! plis des onglets et écarts de hauteur aux nœuds après lissage.
//!
//!   cargo run --release --example rubans -- <lieu.reel> x z rayon
use flow::reseau as rs;

const LISSAGE: f32 = 10.0;
fn lisser(p: &[[f32; 3]]) -> Vec<[f32; 3]> {
    let mut s = vec![0.0f32; p.len()];
    for i in 1..p.len() {
        s[i] = s[i - 1] + (p[i][0] - p[i - 1][0]).hypot(p[i][1] - p[i - 1][1]);
    }
    (0..p.len())
        .map(|i| {
            let (mut somme, mut poids) = (0.0, 0.0);
            for j in 0..p.len() {
                let w = (1.0 - (s[j] - s[i]).abs() / LISSAGE).max(0.0);
                somme += w * p[j][2];
                poids += w;
            }
            [p[i][0], p[i][1], somme / poids]
        })
        .collect()
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let l = sage_reel::Lieu::depuis_octets(&std::fs::read(&a[1]).unwrap()).unwrap();
    let z = [a[2].parse::<f32>().unwrap(), a[3].parse().unwrap(), a[4].parse().unwrap()];
    let dedans = |p: [f32; 3]| (p[0] - z[0]).hypot(p[1] - z[1]) < z[2];
    let r = rs::nivele(&l);
    let (mut points, mut plis, mut virages60) = (0, 0, 0);
    let mut ends: std::collections::HashMap<usize, Vec<f32>> = Default::default();
    let mut routes = 0;
    for ar in r.aretes.iter().filter(|x| x.genre != sage_reel::PIETONNE && x.points.iter().any(|p| dedans(*p))) {
        routes += 1;
        let w = (ar.largeur * 0.5).max(1.0);
        let p = lisser(&ar.points);
        ends.entry(ar.a).or_default().push(p[0][2]);
        ends.entry(ar.b).or_default().push(p[p.len() - 1][2]);
        for i in 1..p.len().saturating_sub(1) {
            points += 1;
            let d = |a: [f32; 3], b: [f32; 3]| { let l = (b[0] - a[0]).hypot(b[1] - a[1]); ([(b[0] - a[0]) / l, (b[1] - a[1]) / l], l) };
            let ((u, l1), (v, l2)) = (d(p[i - 1], p[i]), d(p[i], p[i + 1]));
            let c = (u[0] * v[0] + u[1] * v[1]).clamp(-1.0, 1.0);
            if c < 0.5 {
                virages60 += 1;
            }
            // L'onglet recule de w·tan(θ/2) côté intérieur : pli s'il dépasse un segment voisin
            let t = ((1.0 - c) / (1.0 + c).max(1e-6)).sqrt();
            if w * t > l1.min(l2) {
                plis += 1;
            }
        }
    }
    let ecarts: Vec<f32> = ends.values().filter(|v| v.len() > 1).map(|v| v.iter().cloned().fold(f32::MIN, f32::max) - v.iter().cloned().fold(f32::MAX, f32::min)).collect();
    println!("{routes} routes, {points} points intérieurs : {plis} plis d'onglet, {virages60} virages > 60°");
    println!("{} nœuds partagés : écart des bouts lissés > 5 cm : {}, > 20 cm : {}, > 50 cm : {}, max {:.2}", ecarts.len(), ecarts.iter().filter(|&&e| e > 0.05).count(), ecarts.iter().filter(|&&e| e > 0.2).count(), ecarts.iter().filter(|&&e| e > 0.5).count(), ecarts.iter().cloned().fold(0.0, f32::max));
}
