//! Les tracés du réseau nivelé en surimpression sur la maquette (étape 5, sans
//! volume) : ce que Flow dessine par-dessus une tuile quand Lieux est sur le banc.
//! Rien n'entre dans les tuiles. Réglages par la commande `traces` du module.

use flow::carte::Donnees;
use flow_banc::{Traces, Traits};
use sage_reel::reseau::Passage;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Les réglages : hauteur au-dessus de la chaussée (m), demi-largeur des croix des
/// nœuds (m), et ce qui est tracé.
pub struct Reglages {
    pub dessus: f32,
    pub bras: f32,
    pub noeuds: bool,
    pub ponts: bool,
    pub tunnels: bool,
    /// La bande de remblai sous les voies au sol : montrée ou non ; elle descend
    /// jusqu'à `sous` mètres sous le terrain, et déborde de la voie de `marge` mètres
    /// de chaque côté.
    pub remblai: bool,
    pub sous: f32,
    pub marge: f32,
    /// Le dessus de la bande dépasse le tracé (posé à `dessus` au-dessus de la
    /// chaussée) de `haut` mètres.
    pub haut: f32,
}

pub static REGLAGES: Mutex<Reglages> = Mutex::new(Reglages { dessus: 1.0, bras: 1.5, noeuds: true, ponts: true, tunnels: true, remblai: true, sous: 0.3, marge: 0.0, haut: 0.5 });

/// La couleur de chaque genre de voie (`sage_reel::GRANDE` … `BUS`), et son nom.
const GENRES: [([f32; 3], &str); sage_reel::GENRES as usize] = [
    ([0.95, 0.25, 0.1], "grande (rouge)"),
    ([1.0, 0.78, 0.05], "route (jaune)"),
    ([0.92, 0.92, 0.92], "service (blanc)"),
    ([1.0, 0.45, 0.75], "piétonne (rose)"),
    ([0.62, 0.38, 0.16], "chemin (brun)"),
    ([0.25, 0.72, 0.25], "sentier (vert)"),
    ([0.2, 0.45, 1.0], "bus (bleu)"),
];
/// Les épaisseurs (pixels) : au sol, en pont ; en tunnel, la couleur assombrie.
const EPAISSEUR_SOL: f32 = 2.0;
const EPAISSEUR_PONT: f32 = 4.0;
const SOMBRE_TUNNEL: f32 = 0.45;
const NOEUD: [f32; 3] = [0.08, 0.08, 0.1];

static CALCUL: AtomicBool = AtomicBool::new(false);

/// Le segment p → q (x, z, h) coupé à la boîte b (Liang–Barsky), ou rien.
fn couper(p: [f32; 3], q: [f32; 3], b: [f32; 4]) -> Option<([f32; 3], [f32; 3])> {
    let (dx, dz) = (q[0] - p[0], q[1] - p[1]);
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    for (pk, qk) in [(-dx, p[0] - b[0]), (dx, b[2] - p[0]), (-dz, p[1] - b[1]), (dz, b[3] - p[1])] {
        if pk == 0.0 {
            if qk < 0.0 {
                return None;
            }
        } else {
            let t = qk / pk;
            if pk < 0.0 {
                t0 = t0.max(t);
            } else {
                t1 = t1.min(t);
            }
        }
    }
    if t0 > t1 {
        return None;
    }
    let at = |t: f32| [p[0] + t * dx, p[1] + t * dz, p[2] + t * (q[2] - p[2])];
    Some((at(t0), at(t1)))
}

/// La couleur de la bande de remblai selon le genre de la voie (même ordre que
/// `GENRES`), la plus distincte possible : terres pour les voies carrossables, gris
/// pour les chemins.
const REMBLAIS: [([f32; 3], &str); sage_reel::GENRES as usize] = [
    ([0.50, 0.33, 0.18], "grande : brun foncé"),
    ([0.74, 0.58, 0.38], "route : brun"),
    ([0.90, 0.62, 0.22], "service : ocre"),
    ([0.95, 0.89, 0.76], "piétonne : beige clair"),
    ([0.58, 0.58, 0.60], "chemin : gris"),
    ([0.52, 0.62, 0.30], "sentier : olive"),
    ([0.40, 0.50, 0.70], "bus : ardoise"),
];

/// Les suites de points (x, z, h) des arêtes au sol coupées à la boîte `b` : un
/// segment coupé à l'entrée ou à la sortie commence ou finit une suite.
fn suites(a: &sage_reel::reseau::Arete, b: [f32; 4]) -> Vec<Vec<[f32; 3]>> {
    let mut v: Vec<Vec<[f32; 3]>> = Vec::new();
    let mut cur: Vec<[f32; 3]> = Vec::new();
    for w in a.points.windows(2) {
        match couper(w[0], w[1], b) {
            Some((p, q)) => {
                if cur.last() != Some(&p) {
                    if cur.len() >= 2 {
                        v.push(std::mem::take(&mut cur));
                    }
                    cur.clear();
                    cur.push(p);
                }
                cur.push(q);
            }
            None => {
                if cur.len() >= 2 {
                    v.push(std::mem::take(&mut cur));
                }
                cur.clear();
            }
        }
    }
    if cur.len() >= 2 {
        v.push(cur);
    }
    v
}

/// La bande d'une suite de points, de demi-largeur `w` : dessus à la hauteur de la
/// voie, dessous `sous` mètres sous le terrain (ou sous la voie, si plus bas), murs sur
/// les côtés et aux bouts ; dans la scène `s`.
fn bande(s: &mut flow_banc::Scene, d: &Donnees, q: &[[f32; 3]], w: f32, sous: f32, haut: f32, couleur: [f32; 3]) {
    let n = q.len();
    let normale = |a: [f32; 3], b: [f32; 3]| {
        let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
        let l = dx.hypot(dz).max(1e-6);
        (-dz / l, dx / l)
    };
    // Pour chaque point : gauche et droite, en haut et en bas
    let mut g = Vec::with_capacity(n);
    for i in 0..n {
        let n0 = if i > 0 { normale(q[i - 1], q[i]) } else { normale(q[0], q[1]) };
        let n1 = if i + 1 < n { normale(q[i], q[i + 1]) } else { n0 };
        let (sx, sz) = (n0.0 + n1.0, n0.1 + n1.1);
        let l = sx.hypot(sz);
        let (ux, uz) = if l < 1e-3 { n0 } else { (sx / l, sz / l) };
        let c = (ux * n0.0 + uz * n0.1).max(0.5);
        let (nx, nz) = (ux * w / c, uz * w / c);
        let (x, z, h) = (q[i][0], q[i][1], q[i][2]);
        let terre = |x: f32, z: f32| d.lieu.relief(x as f64, z as f64) as f32;
        let gh = [x + nx, h + haut, z + nz];
        let dh = [x - nx, h + haut, z - nz];
        let gb = [gh[0], terre(gh[0], gh[2]).min(h) - sous, gh[2]];
        let db = [dh[0], terre(dh[0], dh[2]).min(h) - sous, dh[2]];
        g.push((gh, dh, gb, db));
    }
    for i in 0..n - 1 {
        let (a, b) = (g[i], g[i + 1]);
        // dessus, côtés (vus de dehors : l'ordre est corrigé par la normale du triangle,
        // tracé des deux côtés)
        s.polygone(&[a.0, b.0, b.1, a.1], couleur);
        s.polygone(&[a.0, a.2, b.2, b.0], couleur.map(|x| x * 0.8));
        s.polygone(&[a.1, b.1, b.3, a.3], couleur.map(|x| x * 0.8));
    }
    let (a, z) = (g[0], g[n - 1]);
    s.polygone(&[a.0, a.1, a.3, a.2], couleur.map(|x| x * 0.8));
    s.polygone(&[z.0, z.2, z.3, z.1], couleur.map(|x| x * 0.8));
}

/// Les faces de la tuile `cle` : la bande de remblai sous les voies au sol.
pub fn faces(cle: (u8, i32, i32)) -> Result<flow_banc::Faces, String> {
    let d: Arc<Donnees> = crate::LIEU.lock().unwrap().clone().ok_or("pas de lieu")?;
    let Some(r) = d.reseau_pret() else { return Ok(flow_banc::Faces::PasPret) };
    let g = REGLAGES.lock().unwrap();
    let mut s = flow_banc::Scene::default();
    if g.remblai {
        let b = flow::carte::boite_tuile(&d, cle);
        for a in r.aretes.iter().filter(|a| a.passage == Passage::Sol) {
            let w = a.largeur * 0.5 + g.marge;
            for q in suites(a, b) {
                let c = REMBLAIS[(a.genre as usize).min(REMBLAIS.len() - 1)].0;
                bande(&mut s, &d, &q, w, g.sous, g.dessus + g.haut, c);
            }
        }
    }
    Ok(flow_banc::Faces::Faites(s))
}

/// Les tracés de la tuile `cle` du dernier lieu reçu.
pub fn traces(cle: (u8, i32, i32)) -> Result<Traces, String> {
    let d: Arc<Donnees> = crate::LIEU.lock().unwrap().clone().ok_or("pas de lieu")?;
    let Some(r) = d.reseau_pret() else {
        // Le réseau nivelé se calcule à part (quelques secondes) : Flow redemandera
        if !CALCUL.swap(true, Ordering::SeqCst) {
            let d = d.clone();
            std::thread::spawn(move || {
                d.reseau();
                CALCUL.store(false, Ordering::SeqCst);
            });
        }
        return Ok(Traces::PasPret);
    };
    let g = REGLAGES.lock().unwrap();
    let b = flow::carte::boite_tuile(&d, cle);
    // (x, z, h) → (x, h + dessus, z)
    let pos = |p: [f32; 3]| [p[0], p[2] + g.dessus, p[1]];
    // Un groupe par genre et par passage (sol, pont, tunnel), puis les nœuds
    let ng = GENRES.len();
    let mut groupes: Vec<Traits> = Vec::with_capacity(3 * ng + 1);
    for (c, e) in [(1.0, EPAISSEUR_SOL), (1.0, EPAISSEUR_PONT), (SOMBRE_TUNNEL, EPAISSEUR_SOL)] {
        for (couleur, _) in GENRES {
            groupes.push(Traits { couleur: couleur.map(|x| x * c), epaisseur: e, points: Vec::new() });
        }
    }
    groupes.push(Traits { couleur: NOEUD, epaisseur: 3.0, points: Vec::new() });
    for a in &r.aretes {
        let genre = (a.genre as usize).min(ng - 1);
        let k = match a.passage {
            Passage::Sol => genre,
            Passage::Pont if g.ponts => ng + genre,
            Passage::Tunnel if g.tunnels => 2 * ng + genre,
            _ => continue,
        };
        for w in a.points.windows(2) {
            if let Some((p, q)) = couper(w[0], w[1], b) {
                groupes[k].points.push(pos(p));
                groupes[k].points.push(pos(q));
            }
        }
    }
    if g.noeuds {
        let e = g.bras;
        for n in &r.noeuds {
            let p = n.p;
            if p[0] < b[0] || p[0] > b[2] || p[1] < b[1] || p[1] > b[3] {
                continue;
            }
            let pts = [[p[0] - e, p[1], p[2]], [p[0] + e, p[1], p[2]], [p[0], p[1] - e, p[2]], [p[0], p[1] + e, p[2]]];
            groupes[3 * ng].points.extend(pts.map(pos));
        }
    }
    Ok(Traces::Faits(groupes.into_iter().filter(|t| !t.points.is_empty()).collect()))
}

/// Les plus longues suites d'arêtes du genre nommé : leur milieu (x, z), longueur,
/// passage ; et le compte.
fn trouver(nom: &str) -> Result<String, String> {
    let genre = GENRES.iter().position(|g| g.1.starts_with(nom)).ok_or("genre inconnu (grande, route, service, piétonne, chemin, sentier, bus)")?;
    let d: Arc<Donnees> = crate::LIEU.lock().unwrap().clone().ok_or("pas de lieu")?;
    let r = d.reseau_pret().ok_or("réseau pas encore calculé")?;
    let mut v: Vec<(f32, [f32; 3], &str)> = r
        .aretes
        .iter()
        .filter(|a| a.genre as usize == genre)
        .map(|a| {
            let l: f32 = a.points.windows(2).map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1])).sum();
            (l, a.points[a.points.len() / 2], a.passage.nom())
        })
        .collect();
    let total: f32 = v.iter().map(|x| x.0).sum();
    v.sort_by(|a, b| b.0.total_cmp(&a.0));
    let lignes: Vec<String> = v.iter().take(12).map(|(l, p, s)| format!("  {:.0} m · {s} · milieu {:.0} {:.0} (h {:.1})", l, p[0], p[1], p[2])).collect();
    Ok(format!("{} arêtes {}, {:.1} km ; les plus longues :\n{}", v.len(), GENRES[genre].1, total / 1000.0, lignes.join("\n")))
}

/// Les arêtes qui passent à moins de `r` de (x, z) : par genre et passage, leur
/// longueur dans le cercle ; et chacune des ponts (largeur, nœuds).
fn autour(x: f32, z: f32, r: f32) -> Result<String, String> {
    let d: Arc<Donnees> = crate::LIEU.lock().unwrap().clone().ok_or("pas de lieu")?;
    let res = d.reseau_pret().ok_or("réseau pas encore calculé")?;
    let mut compte: std::collections::BTreeMap<(usize, &str), (usize, f32)> = Default::default();
    let mut ponts = Vec::new();
    for (k, a) in res.aretes.iter().enumerate() {
        let dedans: f32 = a.points.windows(2).filter(|w| (w[0][0] - x).hypot(w[0][1] - z) < r || (w[1][0] - x).hypot(w[1][1] - z) < r).map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1])).sum();
        if dedans <= 0.0 {
            continue;
        }
        let e = compte.entry((a.genre as usize, a.passage.nom())).or_default();
        e.0 += 1;
        e.1 += dedans;
        if a.passage == Passage::Pont {
            let p = a.points[a.points.len() / 2];
            ponts.push(format!("  arête {k} : {} · largeur {:.1} m · {:.0} m · milieu {:.0} {:.0} h {:.1}", GENRES.get(a.genre as usize).map(|g| g.1).unwrap_or("?"), a.largeur, dedans, p[0], p[1], p[2]));
        }
    }
    let lignes: Vec<String> = compte.iter().map(|((g, s), (n, l))| format!("  {} · {s} : {n} arêtes, {:.0} m", GENRES.get(*g).map(|x| x.1).unwrap_or("?"), l)).collect();
    Ok(format!("autour de {x} {z} (rayon {r} m) :\n{}\nponts :\n{}", lignes.join("\n"), ponts.join("\n")))
}

/// Les lignes de transport dont le tracé passe à moins de `r` de (x, z), et où
/// (sol, pont, tunnel).
fn lignes(x: f32, z: f32, r: f32) -> Result<String, String> {
    let d: Arc<Donnees> = crate::LIEU.lock().unwrap().clone().ok_or("pas de lieu")?;
    let mut v: Vec<String> = Vec::new();
    for l in &d.lieu.transports.lignes {
        let mut ou: Vec<&str> = Vec::new();
        for m in &l.trace {
            if m.points.iter().any(|p| (p[0] - x).hypot(p[1] - z) < r) {
                let n = match m.ou {
                    sage_reel::transports::PONT => "pont",
                    sage_reel::transports::TUNNEL => "tunnel",
                    _ => "sol",
                };
                if !ou.contains(&n) {
                    ou.push(n);
                }
            }
        }
        if !ou.is_empty() {
            v.push(format!("  {} ({}) : {}", l.titre(), l.nom, ou.join(", ")));
        }
    }
    v.sort();
    Ok(format!("{} lignes à moins de {r} m de {x} {z} :\n{}", v.len(), v.join("\n")))
}

/// Les arêtes à moins de `r` de (x, z) que suivent des lignes de bus : pour chacune,
/// son genre, sa largeur, et les lignes dont le parcours la longe (à moins de 3 m sur
/// au moins 80 % de ses points).
fn suivies(x: f32, z: f32, r: f32) -> Result<String, String> {
    let d: Arc<Donnees> = crate::LIEU.lock().unwrap().clone().ok_or("pas de lieu")?;
    let res = d.reseau_pret().ok_or("réseau pas encore calculé")?;
    let bus: Vec<&sage_reel::transports::Ligne> = d.lieu.transports.lignes.iter().filter(|l| l.mode == sage_reel::transports::BUS).collect();
    let pres = |p: [f32; 3], l: &sage_reel::transports::Ligne| {
        l.trace.iter().any(|m| m.points.windows(2).any(|w| {
            let (a, b) = (w[0], w[1]);
            let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
            let t = (((p[0] - a[0]) * dx + (p[1] - a[1]) * dz) / (dx * dx + dz * dz).max(1e-9)).clamp(0.0, 1.0);
            (p[0] - a[0] - t * dx).hypot(p[1] - a[1] - t * dz) < 3.0
        }))
    };
    let mut v = Vec::new();
    for (k, a) in res.aretes.iter().enumerate() {
        if !a.points.iter().any(|p| (p[0] - x).hypot(p[1] - z) < r) {
            continue;
        }
        let mut noms: Vec<String> = Vec::new();
        for l in &bus {
            let n = a.points.iter().filter(|p| pres(**p, l)).count();
            if n * 5 >= a.points.len() * 4 && !noms.contains(&l.numero) {
                noms.push(l.numero.clone());
            }
        }
        if !noms.is_empty() {
            let l: f32 = a.points.windows(2).map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1])).sum();
            let p = a.points[a.points.len() / 2];
            v.push(format!("  arête {k} : {} · {} · largeur {:.1} · {:.0} m · milieu {:.0} {:.0} · lignes {}", GENRES.get(a.genre as usize).map(|g| g.1).unwrap_or("?"), a.passage.nom(), a.largeur, l, p[0], p[1], noms.join(" ")));
        }
    }
    Ok(format!("{} arêtes suivies par des bus à moins de {r} m de {x} {z} :\n{}", v.len(), v.join("\n")))
}

/// La commande `traces …` du module : les réglages.
pub fn commande(m: &[&str]) -> Result<String, String> {
    if m.get(1) == Some(&"suivies") {
        let v: Vec<f32> = m[2..].iter().filter_map(|s| s.parse().ok()).collect();
        let [x, z, r] = v[..] else { return Err("traces suivies <x> <z> <rayon>".into()) };
        return suivies(x, z, r);
    }
    if m.get(1) == Some(&"lignes") {
        let v: Vec<f32> = m[2..].iter().filter_map(|s| s.parse().ok()).collect();
        let [x, z, r] = v[..] else { return Err("traces lignes <x> <z> <rayon>".into()) };
        return lignes(x, z, r);
    }
    if m.get(1) == Some(&"autour") {
        let v: Vec<f32> = m[2..].iter().filter_map(|s| s.parse().ok()).collect();
        let [x, z, r] = v[..] else { return Err("traces autour <x> <z> <rayon>".into()) };
        return autour(x, z, r);
    }
    if m.get(1) == Some(&"trouver") {
        return trouver(m.get(2).copied().unwrap_or("bus"));
    }
    let mut g = REGLAGES.lock().unwrap();
    let oui = |s: Option<&&str>| matches!(s.copied(), Some("oui" | "1"));
    match (m.get(1).copied(), m.get(2).and_then(|s| s.parse::<f32>().ok())) {
        (Some("dessus"), Some(x)) => g.dessus = x,
        (Some("bras"), Some(x)) => g.bras = x,
        (Some("noeuds"), _) => g.noeuds = oui(m.get(2)),
        (Some("ponts"), _) => g.ponts = oui(m.get(2)),
        (Some("tunnels"), _) => g.tunnels = oui(m.get(2)),
        (Some("remblai"), _) => g.remblai = oui(m.get(2)),
        (Some("sous"), Some(x)) => g.sous = x,
        (Some("marge"), Some(x)) => g.marge = x,
        (Some("haut"), Some(x)) => g.haut = x,
        (None, _) => {}
        _ => return Err("traces [dessus <m> | bras <m> | sous <m> | marge <m> | haut <m> | noeuds|ponts|tunnels|remblai oui|non] (puis : traces refaire, côté Lieux)".into()),
    }
    let legende: Vec<&str> = GENRES.iter().map(|x| x.1).collect();
    let remblais: Vec<&str> = REMBLAIS.iter().map(|x| x.1).collect();
    Ok(format!("traces : dessus {} m, bras {} m, nœuds {}, ponts {}, tunnels {} ; remblai {} (sous {} m, marge {} m, haut {} m au-dessus du tracé)\ngenres : {} ; ponts épais, tunnels sombres, nœuds noirs\nremblais : {}", g.dessus, g.bras, g.noeuds, g.ponts, g.tunnels, g.remblai, g.sous, g.marge, g.haut, legende.join(", "), remblais.join(", ")))
}
