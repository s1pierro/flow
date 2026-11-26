//! Compare la hauteur des routes du réseau nivelé avec et sans les voies piétonnes.
//!
//!   cargo run --release --example pietonnes -- <lieu.reel> [x z rayon]
use flow::reseau as rs;
use std::collections::HashMap;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let o = std::fs::read(&a[1]).expect("lecture");
    let l = sage_reel::Lieu::depuis_octets(&o).expect("lieu");
    let zone = if a.len() >= 5 { Some([a[2].parse::<f32>().unwrap(), a[3].parse().unwrap(), a[4].parse().unwrap()]) } else { None };
    let dedans = |p: [f32; 3]| zone.is_none_or(|z| (p[0] - z[0]).hypot(p[1] - z[1]) < z[2]);
    let tout = rs::nivele(&l);
    let mut sans = l.clone();
    sans.routes.retain(|r| r.genre != sage_reel::PIETONNE);
    sans.ponts.retain(|r| r.genre != sage_reel::PIETONNE);
    let seul = rs::nivele(&sans);
    // Les hauteurs du réseau complet, par point au décimètre
    let cle = |p: [f32; 3]| ((p[0] * 10.0).round() as i64, (p[1] * 10.0).round() as i64);
    let mut h: HashMap<(i64, i64), f32> = HashMap::new();
    for ar in tout.aretes.iter().filter(|x| x.genre != sage_reel::PIETONNE) {
        for p in &ar.points {
            h.insert(cle(*p), p[2]);
        }
    }
    // Les bosses le long d'une voie : écart d'un point à la moyenne de ses voisins
    let bosses = |r: &rs::Reseau| {
        let mut v: Vec<(f32, [f32; 3], usize)> = Vec::new();
        for (k, ar) in r.aretes.iter().enumerate().filter(|(_, x)| x.genre != sage_reel::PIETONNE) {
            for w in ar.points.windows(3) {
                if dedans(w[1]) {
                    v.push(((w[1][2] - 0.5 * (w[0][2] + w[2][2])).abs(), w[1], k));
                }
            }
        }
        v.sort_by(|a, b| b.0.total_cmp(&a.0));
        v
    };
    let (bt, bs) = (bosses(&tout), bosses(&seul));
    let somme = |v: &[(f32, [f32; 3], usize)], s: f32| v.iter().filter(|x| x.0 > s).count();
    println!("points de route : {} (avec piétonnes) / {} (sans)", bt.len(), bs.len());
    for s in [0.05, 0.1, 0.2, 0.5] {
        println!("  bosses > {s} m : {} avec, {} sans", somme(&bt, s), somme(&bs, s));
    }
    let mut ecarts: Vec<(f32, [f32; 3], f32)> = Vec::new();
    for ar in &seul.aretes {
        for p in &ar.points {
            if let Some(&ht) = h.get(&cle(*p)) {
                if dedans(*p) {
                    ecarts.push(((ht - p[2]).abs(), *p, ht));
                }
            }
        }
    }
    ecarts.sort_by(|a, b| b.0.total_cmp(&a.0));
    println!("écart de hauteur des routes (avec − sans) sur {} points communs :", ecarts.len());
    for s in [0.05, 0.1, 0.2, 0.5, 1.0] {
        println!("  > {s} m : {}", ecarts.iter().filter(|x| x.0 > s).count());
    }
    for e in ecarts.iter().take(12) {
        println!("  {:.2} m en ({:.0}, {:.0}) : sans {:.2}, avec {:.2}", e.0, e.1[0], e.1[1], e.1[2], e.2);
    }
    if let Some(e) = ecarts.first() {
        println!("contraintes autour de ({:.0}, {:.0}), avec :", e.1[0], e.1[1]);
        autour(&l, &tout, e.1[0], e.1[1], 30.0);
        println!("sans :");
        autour(&sans, &seul, e.1[0], e.1[1], 30.0);
    }
    println!("plus grosses bosses avec piétonnes :");
    for b in bt.iter().take(10) {
        println!("  {:.2} m en ({:.0}, {:.0}) arête {} genre {}", b.0, b.1[0], b.1[1], b.2, tout.aretes[b.2].genre);
    }
}

/// Les contraintes autour d'un point, dans un réseau.
#[allow(dead_code)]
pub fn autour(l: &sage_reel::Lieu, r: &rs::Reseau, x: f32, z: f32, rayon: f32) {
    let c = rs::contraintes(r, l);
    for c in &c {
        let p = r.point(c.haut);
        if (p[0] - x).hypot(p[1] - z) > rayon {
            continue;
        }
        let (ah, hb) = (&r.aretes[c.haut.0], c.bas.map(|b| (&r.aretes[b.0], r.hauteur(b))));
        println!("  {:?} en ({:.0}, {:.0}) : haut arête {} genre {} {:?} h {:.2} ; bas {:?}", c.genre, p[0], p[1], c.haut.0, ah.genre, ah.passage, p[2], hb.map(|(a, h)| (a.genre, a.passage, (h * 100.0).round() / 100.0)));
    }
}
