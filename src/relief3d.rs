//! La vue 3D d'un lieu : son relief en tuiles à niveaux de détail, chacune drapée de la
//! tuile de la carte 2D de même clé (déjà dessinée : sol, eau, routes, bâtiments,
//! ombrage).
//!
//! - **Les tuiles** suivent la pyramide de la carte (`carte::boite_tuile`) : la tuile
//!   (z, i, j) a toujours `COTES` × `COTES` cases, quelle que soit sa taille. Près de
//!   l'œil, on descend jusqu'au niveau où une case vaut à peu près un pas de la grille
//!   du lieu ; au loin, de grandes tuiles grossières.
//! - **Les jupes** : chaque tuile a un rebord qui descend sous elle, ce qui cache les
//!   fentes entre deux niveaux voisins.
//! - **Les maillages** se font au fil des images (quelques millisecondes par image),
//!   les grossiers d'abord ; en attendant, l'ancêtre le plus proche déjà fait. Une
//!   retouche du lieu (`Carte::retoucher`) les périme : ils se refont, l'ancien restant
//!   visible jusque-là. Ceux qui ne servent plus s'oublient.
//! - **Les textures** sont les tuiles de la carte (`Carte::textures`), demandées aux
//!   mêmes threads ; en attendant, la partie d'un ancêtre.
//! - **Les bâtiments** se montent en volume (`toits::monter` : murs et pans du toit)
//!   près de l'œil, à moins de `BATI_RAYON`, par tuiles d'environ 300 m : chacun dans
//!   celle qui contient son centre. Ils se font dans le même temps que le relief, après
//!   lui, et se périment de même.

use crate::carte::{boite_tuile, Carte, Cle, Donnees, NIVEAU_MAX};
use crate::draw::{Maillage3d, Morceau3d, Rect, Terrain3d};
use crate::sculpture::{produit, Camera, CHAMP};
use crate::widgets::Cx;
use sage_reel::{teintes_bati, toits};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Cases par côté d'une tuile.
pub const COTES: usize = 32;
/// Une tuile se divise quand l'œil est à moins de `PRES` fois son côté.
const PRES: f32 = 2.4;
/// Le temps donné aux maillages à chaque image, en secondes.
const BUDGET: f64 = 0.008;
/// Les maillages gardés au plus.
const GARDES: usize = 600;
/// Les bâtiments se montent jusqu'à cette distance de l'œil (m).
pub const BATI_RAYON: f32 = 2000.0;
/// Le côté visé des tuiles de bâtiments (m).
const BATI_TUILE: f32 = 300.0;
/// Le ciel, et la brume au loin.
pub const CIEL: [f32; 3] = [0.80, 0.86, 0.92];

static INSTANCES: AtomicU64 = AtomicU64::new(1);

struct Morceau {
    maillage: Maillage3d,
    /// La retouche jusqu'à laquelle il est à jour, et la dernière image où il a servi.
    faite: u64,
    vu: u64,
}

/// La vue 3D d'un lieu : sa caméra, ses maillages.
pub struct Relief3d {
    pub camera: Camera,
    morceaux: HashMap<Cle, Morceau>,
    batis: HashMap<Cle, Morceau>,
    image: u64,
    num: u64,
    revision: u64,
    /// Les tuiles dessinées à la dernière image, et les tuiles de bâtiments (pour les
    /// essais et la barre).
    pub dessinees: usize,
    pub bati_dessinees: usize,
    /// Des maillages restent à faire.
    pub en_attente: bool,
}

impl Relief3d {
    /// Vue de biais, depuis le sud, sur le centre du lieu.
    pub fn new(d: &Donnees) -> Relief3d {
        let e = d.lieu.etendue();
        let (cx, cz) = (((e[0] + e[2]) / 2.0) as f32, ((e[1] + e[3]) / 2.0) as f32);
        let cote = (e[2] - e[0]).max(e[3] - e[1]) as f32;
        let camera = Camera { lacet: 0.0, tangage: 0.55, distance: cote * 0.7, cible: [cx, d.lieu.relief(cx as f64, cz as f64) as f32, cz] };
        Relief3d { camera, morceaux: HashMap::new(), batis: HashMap::new(), image: 0, num: INSTANCES.fetch_add(1, Ordering::Relaxed), revision: 0, dessinees: 0, bati_dessinees: 0, en_attente: false }
    }

    /// La projection : proche et lointain selon la distance, assez loin pour l'horizon.
    pub fn mvp(&self, aspect: f32) -> [f32; 16] {
        let fy = 1.0 / (CHAMP / 2.0).tan();
        let d = self.camera.distance;
        let (pres, loin) = ((0.01 * d).max(0.5), (80.0 * d).max(60_000.0));
        let mut p = [0.0; 16];
        p[0] = fy / aspect;
        p[5] = fy;
        p[10] = (loin + pres) / (pres - loin);
        p[11] = -1.0;
        p[14] = 2.0 * loin * pres / (pres - loin);
        produit(&p, &self.camera.vue())
    }

    /// Pose la cible sur le sol et garde l'œil au-dessus.
    pub fn poser(&mut self, d: &Donnees) {
        let l = &d.lieu;
        let e = l.etendue();
        let c = &mut self.camera;
        c.cible[0] = c.cible[0].clamp(e[0] as f32, e[2] as f32);
        c.cible[2] = c.cible[2].clamp(e[1] as f32, e[3] as f32);
        c.cible[1] = l.relief(c.cible[0] as f64, c.cible[2] as f64) as f32;
        let cote = (e[2] - e[0]).max(e[3] - e[1]) as f32;
        c.distance = c.distance.clamp(20.0, cote * 3.0);
        c.tangage = c.tangage.clamp(0.02, 1.5);
        for _ in 0..40 {
            let o = c.oeil();
            if o[1] > l.relief(o[0] as f64, o[2] as f64) as f32 + 3.0 || c.tangage >= 1.5 {
                break;
            }
            c.tangage += 0.02;
        }
    }

    /// Un doigt glisse de (dx, dy) pixels : tourner autour.
    pub fn orbiter(&mut self, dx: f32, dy: f32, dp: f32) {
        let k = 0.006 / dp.max(0.1);
        self.camera.lacet -= dx * k;
        self.camera.tangage = (self.camera.tangage + dy * k).clamp(0.02, 1.5);
    }

    /// Une manette (celle d'une télécommande) pendant `dt` s : le stick gauche avance
    /// et décale au sol (d'autant plus vite que la vue est haute), le droit tourne
    /// autour de la cible et la penche, les gâchettes rapprochent (RT) et éloignent
    /// (LT), la croix fait de même par crans plus doux.
    pub fn piloter(&mut self, m: &crate::telecommande::Manette, dt: f32) {
        use crate::telecommande::bouton;
        let morte = |v: f32| if v.abs() < 0.12 { 0.0 } else { (v - 0.12 * v.signum()) / 0.88 };
        let c = &mut self.camera;
        let (gx, gy) = (morte(m.gauche[0]), morte(m.gauche[1]));
        let vitesse = c.distance * 0.9 * if m.appuye(bouton::A) { 3.0 } else { 1.0 };
        let (sl, cl) = c.lacet.sin_cos();
        // Droite (cl, 0, -sl) ; devant au sol (-sl, 0, -cl) ; pousser vers le haut avance
        let (avant, droite) = (-gy * vitesse * dt, gx * vitesse * dt);
        c.cible[0] += droite * cl - avant * sl;
        c.cible[2] += -droite * sl - avant * cl;
        c.lacet -= morte(m.droit[0]) * 1.8 * dt;
        c.tangage = (c.tangage - morte(m.droit[1]) * 1.1 * dt).clamp(0.02, 1.5);
        let mut zoom = m.rt - m.lt;
        if m.appuye(bouton::HAUT) {
            zoom += 0.4;
        }
        if m.appuye(bouton::BAS) {
            zoom -= 0.4;
        }
        c.distance = (c.distance * (-zoom * 1.6 * dt).exp()).clamp(5.0, 200_000.0);
    }

    /// Deux doigts : rapprocher de `k` (écart relatif), glisser de (dx, dy) pixels.
    pub fn pincer(&mut self, k: f32, dx: f32, dy: f32, h: f32) {
        // Le déplacement dans le plan du sol, à l'échelle de la cible
        let px = self.camera.taille_a(self.camera.cible, 1.0, h);
        let (sl, cl) = self.camera.lacet.sin_cos();
        // Droite (cl, 0, -sl) ; devant au sol (-sl, 0, -cl)
        let (avant, droite) = (dy * px / self.camera.tangage.sin().max(0.3), -dx * px);
        self.camera.cible[0] += droite * cl - avant * sl;
        self.camera.cible[2] += -droite * sl - avant * cl;
        self.camera.distance /= k.max(1e-3);
    }

    /// Les tuiles à montrer depuis l'œil, de la plus proche à la plus lointaine.
    fn choisir(&self, d: &Donnees, mvp: &[f32; 16]) -> Vec<Cle> {
        let l = &d.lieu;
        let (_, _, c) = d.monde();
        let e = l.etendue();
        let zmax = ((c / (COTES as f32 * l.pas as f32)).log2().floor() as i32).clamp(0, NIVEAU_MAX as i32) as u8;
        let (hmin, hmax) = (l.base, l.base + 65535.0 * l.echelle);
        let o = self.camera.oeil();
        let mut v = Vec::new();
        let mut pile = vec![(0u8, 0i32, 0i32)];
        while let Some(cle) = pile.pop() {
            let b = boite_tuile(d, cle);
            if (b[0] as f64) > e[2] || (b[2] as f64) < e[0] || (b[1] as f64) > e[3] || (b[3] as f64) < e[1] || dehors(mvp, b, hmin, hmax) {
                continue;
            }
            let cote = b[2] - b[0];
            let dx = (b[0] - o[0]).max(0.0).max(o[0] - b[2]);
            let dz = (b[1] - o[2]).max(0.0).max(o[2] - b[3]);
            let dy = (o[1] - hmax).max(0.0);
            let dist = (dx * dx + dy * dy + dz * dz).sqrt();
            if cle.0 < zmax && dist < PRES * cote {
                let (z, i, j) = cle;
                for (a, b) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    pile.push((z + 1, i * 2 + a, j * 2 + b));
                }
            } else {
                v.push((dist, cle));
            }
        }
        v.sort_by(|a, b| a.0.total_cmp(&b.0));
        v.into_iter().map(|(_, c)| c).collect()
    }

    /// Les tuiles de bâtiments à montrer (à moins de `BATI_RAYON` de l'œil, dans le
    /// champ), de la plus proche à la plus lointaine.
    fn choisir_bati(&self, d: &Donnees, mvp: &[f32; 16]) -> Vec<Cle> {
        let l = &d.lieu;
        let (x0, z0, c) = d.monde();
        let e = l.etendue();
        let z = niveau_bati(c);
        let t = c / (1u32 << z) as f32;
        let (hmin, hmax) = (l.base, l.base + 65535.0 * l.echelle + 80.0);
        let o = self.camera.oeil();
        let n = (1i32 << z) - 1;
        let (i0, i1) = (((o[0] - BATI_RAYON - x0) / t).floor().max(0.0) as i32, (((o[0] + BATI_RAYON - x0) / t).floor() as i32).min(n));
        let (j0, j1) = (((o[2] - BATI_RAYON - z0) / t).floor().max(0.0) as i32, (((o[2] + BATI_RAYON - z0) / t).floor() as i32).min(n));
        let mut v = Vec::new();
        for j in j0..=j1 {
            for i in i0..=i1 {
                let cle = (z, i, j);
                let b = boite_tuile(d, cle);
                if (b[0] as f64) > e[2] || (b[2] as f64) < e[0] || (b[1] as f64) > e[3] || (b[3] as f64) < e[1] || dehors(mvp, b, hmin, hmax) {
                    continue;
                }
                let dx = (b[0] - o[0]).max(0.0).max(o[0] - b[2]);
                let dz = (b[1] - o[2]).max(0.0).max(o[2] - b[3]);
                let dy = (o[1] - hmax).max(0.0);
                let dist = (dx * dx + dy * dy + dz * dz).sqrt();
                if dist < BATI_RAYON {
                    v.push((dist, cle));
                }
            }
        }
        v.sort_by(|a, b| a.0.total_cmp(&b.0));
        v.into_iter().map(|(_, c)| c).collect()
    }

    /// Dessine le lieu de `carte` dans `zone`.
    pub fn dessiner(&mut self, cx: &mut Cx, zone: Rect, carte: &mut Carte) {
        self.image += 1;
        let ciel = CIEL.map(|c| (c * 255.0) as u8);
        cx.dl.rrect(zone, 0.0, [ciel[0], ciel[1], ciel[2], 255]);
        let donnees = carte.donnees.clone();
        let Ok(d) = donnees.read() else { return };
        self.poser(&d);
        let mvp = self.mvp(zone.w / zone.h.max(1.0));
        let voulues = self.choisir(&d, &mvp);
        // Les maillages à faire (absents ou périmés), les plus grossiers d'abord
        let mut a_faire: Vec<(u8, usize, Cle)> = Vec::new();
        for (rang, &cle) in voulues.iter().enumerate() {
            let perime = match self.morceaux.get(&cle) {
                Some(m) => !carte.a_jour(m.faite, boite_tuile(&d, cle)),
                None => true,
            };
            if perime {
                a_faire.push((cle.0, rang, cle));
                // Et ses ancêtres absents : de quoi montrer vite
                let (z, i, j) = cle;
                for k in 1..=z {
                    let a = (z - k, i >> k, j >> k);
                    if !self.morceaux.contains_key(&a) {
                        a_faire.push((a.0, rang, a));
                    }
                }
            }
        }
        a_faire.sort();
        a_faire.dedup_by_key(|x| x.2);
        let debut = std::time::Instant::now();
        let mut faits = HashSet::new();
        for &(_, _, cle) in &a_faire {
            if debut.elapsed().as_secs_f64() > BUDGET {
                break;
            }
            if !faits.insert(cle) {
                continue;
            }
            let (sommets, indices) = fabriquer(&d, cle);
            self.revision += 1;
            let id = 0x7233_6400_0000_0000u64 ^ (self.num << 40) ^ ((cle.0 as u64) << 32) ^ ((cle.1 as u64 & 0xFFFF) << 16) ^ (cle.2 as u64 & 0xFFFF);
            let maillage = Maillage3d { id, revision: self.revision, sommets: Arc::new(sommets), indices: Arc::new(indices) };
            self.morceaux.insert(cle, Morceau { maillage, faite: carte.compteur, vu: self.image });
        }
        self.en_attente = faits.len() < a_faire.iter().map(|x| x.2).collect::<HashSet<_>>().len();
        // Les bâtiments, dans le temps qui reste
        let voulus = self.choisir_bati(&d, &mvp);
        for &cle in &voulus {
            let perime = match self.batis.get(&cle) {
                Some(m) => !carte.a_jour(m.faite, boite_tuile(&d, cle)),
                None => true,
            };
            if !perime {
                continue;
            }
            if debut.elapsed().as_secs_f64() > BUDGET {
                self.en_attente = true;
                break;
            }
            let (sommets, indices) = fabriquer_bati(&d, cle);
            self.revision += 1;
            let id = 0x6261_7469_0000_0000u64 ^ (self.num << 40) ^ ((cle.0 as u64) << 32) ^ ((cle.1 as u64 & 0xFFFF) << 16) ^ (cle.2 as u64 & 0xFFFF);
            let maillage = Maillage3d { id, revision: self.revision, sommets: Arc::new(sommets), indices: Arc::new(indices) };
            self.batis.insert(cle, Morceau { maillage, faite: carte.compteur, vu: self.image });
        }
        let mut bati = Vec::new();
        for cle in &voulus {
            if let Some(m) = self.batis.get_mut(cle) {
                m.vu = self.image;
                if !m.maillage.indices.is_empty() {
                    bati.push(m.maillage.clone());
                }
            }
        }
        self.bati_dessinees = bati.len();
        drop(d);
        // Ce qui se dessine : la tuile, ou son ancêtre le plus proche déjà fait
        let mut montrees: Vec<Cle> = Vec::new();
        let mut vues = HashSet::new();
        for &cle in &voulues {
            let (z, i, j) = cle;
            if let Some(a) = (0..=z).map(|k| (z - k, i >> k, j >> k)).find(|a| self.morceaux.contains_key(a)) {
                if vues.insert(a) {
                    montrees.push(a);
                }
            }
        }
        let textures = carte.textures(&montrees);
        let mut morceaux = Vec::with_capacity(montrees.len());
        for (cle, t) in montrees.iter().zip(textures) {
            let Some(m) = self.morceaux.get_mut(cle) else { continue };
            m.vu = self.image;
            let (image, uv) = match t {
                Some((i, uv)) => (Some(i), uv),
                None => (None, [0.0, 0.0, 1.0, 1.0]),
            };
            morceaux.push(Morceau3d { maillage: m.maillage.clone(), image, uv, couleur: [0.72, 0.76, 0.62] });
        }
        self.dessinees = morceaux.len();
        let d = self.camera.distance;
        cx.dl.terrain(Terrain3d { rect: zone, mvp, oeil: self.camera.oeil(), ciel: CIEL, brume: [d * 3.0 + 2000.0, d * 12.0 + 12_000.0], morceaux, bati });
        // Les maillages qui n'ont pas servi depuis longtemps
        if self.batis.len() > GARDES / 2 {
            let mut v: Vec<(u64, Cle)> = self.batis.iter().map(|(k, m)| (m.vu, *k)).collect();
            v.sort();
            for (_, k) in v.iter().take(self.batis.len() - GARDES / 2) {
                self.batis.remove(k);
            }
        }
        if self.morceaux.len() > GARDES {
            let mut v: Vec<(u64, Cle)> = self.morceaux.iter().filter(|(k, _)| k.0 > 0).map(|(k, m)| (m.vu, *k)).collect();
            v.sort();
            for (_, k) in v.iter().take(self.morceaux.len() - GARDES) {
                self.morceaux.remove(k);
            }
        }
    }
}

/// Le niveau des tuiles de bâtiments : des tuiles d'au moins `BATI_TUILE` mètres.
fn niveau_bati(c: f32) -> u8 {
    ((c / BATI_TUILE).log2().floor().max(0.0) as u8).min(NIVEAU_MAX)
}

/// Les bâtiments de la tuile `cle` (ceux dont le centre y tombe), en volume : sommets de
/// 9 flottants (position, normale, couleur) et triangles.
pub fn fabriquer_bati(d: &Donnees, cle: Cle) -> (Vec<f32>, Vec<u32>) {
    let b = boite_tuile(d, cle);
    let l = &d.lieu;
    let mut v = Vec::new();
    for k in d.batiments_dans(b) {
        let bt = &l.batiments[k as usize];
        let c = bt.centre();
        if c[0] < b[0] || c[0] >= b[2] || c[1] < b[1] || c[1] >= b[3] || bt.contour.len() < 3 {
            continue;
        }
        let (murs, toit) = teintes_bati(bt.genre);
        toits::monter(bt, murs, toit, &mut |p, n, coul| {
            for q in p {
                v.extend_from_slice(&[q[0], q[1], q[2], n[0], n[1], n[2], coul[0], coul[1], coul[2]]);
            }
        });
    }
    let ind = (0..(v.len() / 9) as u32).collect();
    (v, ind)
}

/// La boîte (en x, z ; de `hmin` à `hmax` en hauteur) est-elle hors du champ ?
fn dehors(mvp: &[f32; 16], b: [f32; 4], hmin: f32, hmax: f32) -> bool {
    let mut coins = [[0f32; 4]; 8];
    for (k, c) in coins.iter_mut().enumerate() {
        let p = [if k & 1 == 0 { b[0] } else { b[2] }, if k & 2 == 0 { hmin } else { hmax }, if k & 4 == 0 { b[1] } else { b[3] }];
        for (r, v) in c.iter_mut().enumerate() {
            *v = mvp[r] * p[0] + mvp[4 + r] * p[1] + mvp[8 + r] * p[2] + mvp[12 + r];
        }
    }
    // Tous du même côté d'un des plans du champ
    (0..3).any(|a| coins.iter().all(|c| c[a] < -c[3]) || coins.iter().all(|c| c[a] > c[3]))
}

/// Le maillage de la tuile `cle` : sommets de 8 flottants (position, normale, texture)
/// et triangles, avec ses jupes.
pub fn fabriquer(d: &Donnees, cle: Cle) -> (Vec<f32>, Vec<u32>) {
    let l = &d.lieu;
    let b = boite_tuile(d, cle);
    let n = COTES;
    let s = (b[2] - b[0]) / n as f32;
    // Les hauteurs, avec une case de plus tout autour pour les normales
    let m = n + 3;
    let mut h = vec![0f32; m * m];
    for k in 0..m {
        let z = b[1] + (k as f32 - 1.0) * s;
        for a in 0..m {
            let x = b[0] + (a as f32 - 1.0) * s;
            h[k * m + a] = l.relief(x as f64, z as f64) as f32;
        }
    }
    let hh = |a: usize, k: usize| h[(k + 1) * m + a + 1];
    let mut v = Vec::with_capacity(((n + 1) * (n + 1) + 4 * (n + 1)) * 8);
    let sommet = |a: usize, k: usize, bas: f32| -> [f32; 8] {
        let y = hh(a, k);
        let (gx, gz) = ((h[(k + 1) * m + a + 2] - h[(k + 1) * m + a]) / (2.0 * s), (h[(k + 2) * m + a + 1] - h[k * m + a + 1]) / (2.0 * s));
        let nn = (gx * gx + 1.0 + gz * gz).sqrt();
        [b[0] + a as f32 * s, y - bas, b[1] + k as f32 * s, -gx / nn, 1.0 / nn, -gz / nn, a as f32 / n as f32, k as f32 / n as f32]
    };
    for k in 0..=n {
        for a in 0..=n {
            v.extend_from_slice(&sommet(a, k, 0.0));
        }
    }
    let mut ind = Vec::with_capacity(n * n * 6 + 4 * n * 6);
    let w = (n + 1) as u32;
    for k in 0..n as u32 {
        for a in 0..n as u32 {
            let p = k * w + a;
            ind.extend_from_slice(&[p, p + w, p + 1, p + 1, p + w, p + w + 1]);
        }
    }
    // Les jupes : chaque bord, doublé plus bas
    let jupe = s * 1.5 + 4.0;
    let bords: [Box<dyn Fn(usize) -> (usize, usize)>; 4] = [Box::new(|t| (t, 0)), Box::new(|t| (t, n)), Box::new(|t| (0, t)), Box::new(|t| (n, t))];
    for bord in &bords {
        let depart = (v.len() / 8) as u32;
        for t in 0..=n {
            let (a, k) = bord(t);
            v.extend_from_slice(&sommet(a, k, jupe));
        }
        for t in 0..n as u32 {
            let (a0, k0) = bord(t as usize);
            let (a1, k1) = bord(t as usize + 1);
            let (p0, p1) = (k0 as u32 * w + a0 as u32, k1 as u32 * w + a1 as u32);
            ind.extend_from_slice(&[p0, depart + t, p1, p1, depart + t, depart + t + 1]);
        }
    }
    (v, ind)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sage_reel::{Grille, Lieu};

    fn donnees() -> Donnees {
        // Une pente vers l'est, sur 3 × 2 km
        let pas = 2.0;
        let g = Grille::emprise(pas, 3000.0, 2000.0);
        let h: Vec<f32> = (0..g.points()).map(|i| g.point(i).0 as f32 * 0.5).collect();
        let eau = vec![false; g.points()];
        Donnees::new(Lieu::sur("Pente", 49.0, 1.0, 0.0, 0.0, pas, g, -10.0, &h, &eau, Vec::new(), Vec::new(), Vec::new()))
    }

    #[test]
    fn une_tuile_suit_le_relief_et_a_ses_jupes() {
        let d = donnees();
        let (v, ind) = fabriquer(&d, (0, 0, 0));
        let n = COTES + 1;
        assert_eq!(v.len(), (n * n + 4 * n) * 8);
        assert_eq!(ind.len(), COTES * COTES * 6 + 4 * COTES * 6);
        assert!(ind.iter().all(|&i| (i as usize) < v.len() / 8));
        // Un sommet du milieu : à la hauteur du relief, la normale penchée vers l'ouest
        let k = (n / 2 * n + n / 2) * 8;
        let (x, y, z) = (v[k], v[k + 1], v[k + 2]);
        assert!((y - d.lieu.relief(x as f64, z as f64) as f32).abs() < 0.05);
        assert!(v[k + 3] < -0.01 && v[k + 4] > 0.9);
        // Les jupes sous le bord
        let j = n * n * 8;
        assert!(v[j + 1] < v[1] - 1.0);
    }

    #[test]
    fn les_batiments_se_montent_une_fois_dans_leur_tuile() {
        use sage_reel::{Batiment, MAISON};
        let pas = 4.0;
        let g = Grille::emprise(pas, 2000.0, 2000.0);
        let h = vec![10.0; g.points()];
        let carre = |x: f32, z: f32| vec![[x, z], [x + 10.0, z], [x + 10.0, z + 8.0], [x, z + 8.0]];
        let b = vec![Batiment::plat(MAISON, 9.0, 16.0, carre(0.0, 0.0)), Batiment::plat(MAISON, 9.0, 16.0, carre(500.0, 300.0))];
        let d = Donnees::new(Lieu::sur("B", 49.0, 1.0, 0.0, 0.0, pas, g, 0.0, &h, &vec![false; g.points()], Vec::new(), b, Vec::new()));
        let (_, _, c) = d.monde();
        let z = niveau_bati(c);
        let t = c / (1u32 << z) as f32;
        assert!(t >= BATI_TUILE && t < BATI_TUILE * 2.0, "{t}");
        // Chaque bâtiment dans une seule tuile : 2 en tout sur toutes les tuiles
        let n = 1i32 << z;
        let mut murs = 0;
        let mut hauts = Vec::new();
        for j in 0..n {
            for i in 0..n {
                let (v, ind) = fabriquer_bati(&d, (z, i, j));
                assert_eq!(ind.len(), v.len() / 9);
                // Un toit plat : deux triangles vers le haut par bâtiment
                murs += v.chunks(27).filter(|t| t[4] > 0.9).count();
                hauts.extend(v.chunks(9).map(|s| s[1]));
            }
        }
        assert_eq!(murs, 4);
        assert!(hauts.iter().all(|&y| (y - 9.0).abs() < 1e-3 || (y - 16.0).abs() < 1e-3));
    }

    #[test]
    fn de_pres_les_tuiles_sont_fines_et_de_loin_grossieres() {
        let d = donnees();
        let mut r = Relief3d::new(&d);
        r.camera.distance = 8000.0;
        r.camera.tangage = 1.3;
        r.poser(&d);
        let mvp = r.mvp(1.5);
        let loin = r.choisir(&d, &mvp);
        assert!(!loin.is_empty());
        r.camera.distance = 60.0;
        r.camera.tangage = 0.3;
        r.poser(&d);
        let mvp = r.mvp(1.5);
        let pres = r.choisir(&d, &mvp);
        let zmax = |v: &[Cle]| v.iter().map(|c| c.0).max().unwrap();
        assert!(zmax(&pres) > zmax(&loin), "{} {}", zmax(&pres), zmax(&loin));
        // La plus proche en premier, et l'œil au-dessus du sol
        assert_eq!(pres[0].0, zmax(&pres));
        let o = r.camera.oeil();
        assert!(o[1] > d.lieu.relief(o[0] as f64, o[2] as f64) as f32);
    }
}
