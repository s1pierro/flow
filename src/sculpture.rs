//! L'argile : un maillage de triangles qu'on sculpte à coups de pinceau, et la
//! caméra qui le regarde. Rien ici ne dépend de l'interface (voir
//! `contextes/argile.rs`) ; les fichiers sont en glTF binaire (`crate::glb`).
//!
//! - Le maillage part d'une boule (un icosaèdre découpé) ; « subdiviser » (Loop)
//!   multiplie ses triangles par quatre en l'adoucissant.
//! - Un coup de pinceau touche les sommets à moins d'un rayon de son centre, avec
//!   une atténuation douce. Avec la symétrie, le coup se répète de l'autre côté du
//!   plan x = 0 ; un sommet proche du plan ne reçoit que le plus fort des deux.

use std::collections::HashMap;
use std::sync::Arc;

pub type V3 = [f32; 3];

pub fn ajouter(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
pub fn oter(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
pub fn fois(a: V3, k: f32) -> V3 {
    [a[0] * k, a[1] * k, a[2] * k]
}
pub fn scalaire(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
pub fn vectoriel(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
pub fn norme(a: V3) -> f32 {
    scalaire(a, a).sqrt()
}
pub fn unitaire(a: V3) -> V3 {
    let n = norme(a);
    if n > 1e-12 { fois(a, 1.0 / n) } else { [0.0, 0.0, 0.0] }
}
/// Le reflet par le plan de symétrie x = 0.
pub fn miroir(a: V3) -> V3 {
    [-a[0], a[1], a[2]]
}

/// Au-delà, « subdiviser » refuse : le pinceau deviendrait lent.
pub const TRIANGLES_MAX: usize = 400_000;

#[derive(Clone, Debug, PartialEq)]
pub struct Maillage {
    pub positions: Vec<V3>,
    /// Partagés entre les instantanés tant que la topologie ne change pas.
    pub triangles: Arc<Vec<[u32; 3]>>,
    pub normales: Vec<V3>,
    voisins: Arc<Vec<Vec<u32>>>,
}

impl Maillage {
    pub fn nouveau(positions: Vec<V3>, triangles: Vec<[u32; 3]>) -> Maillage {
        let mut m = Maillage { positions, triangles: Arc::new(triangles), normales: Vec::new(), voisins: Arc::new(Vec::new()) };
        m.relier();
        m.normaliser();
        m
    }

    /// Une boule de rayon `rayon` : l'icosaèdre découpé `niveau` fois.
    pub fn boule(niveau: u32, rayon: f32) -> Maillage {
        let t = (1.0 + 5f32.sqrt()) / 2.0;
        let mut p: Vec<V3> = [
            [-1.0, t, 0.0],
            [1.0, t, 0.0],
            [-1.0, -t, 0.0],
            [1.0, -t, 0.0],
            [0.0, -1.0, t],
            [0.0, 1.0, t],
            [0.0, -1.0, -t],
            [0.0, 1.0, -t],
            [t, 0.0, -1.0],
            [t, 0.0, 1.0],
            [-t, 0.0, -1.0],
            [-t, 0.0, 1.0],
        ]
        .iter()
        .map(|v| fois(unitaire(*v), rayon))
        .collect();
        let mut f: Vec<[u32; 3]> = vec![
            [0, 11, 5],
            [0, 5, 1],
            [0, 1, 7],
            [0, 7, 10],
            [0, 10, 11],
            [1, 5, 9],
            [5, 11, 4],
            [11, 10, 2],
            [10, 7, 6],
            [7, 1, 8],
            [3, 9, 4],
            [3, 4, 2],
            [3, 2, 6],
            [3, 6, 8],
            [3, 8, 9],
            [4, 9, 5],
            [2, 4, 11],
            [6, 2, 10],
            [8, 6, 7],
            [9, 8, 1],
        ];
        for _ in 0..niveau {
            let mut milieux: HashMap<(u32, u32), u32> = HashMap::new();
            let mut g = Vec::with_capacity(f.len() * 4);
            let mut milieu = |a: u32, b: u32, p: &mut Vec<V3>| {
                *milieux.entry((a.min(b), a.max(b))).or_insert_with(|| {
                    p.push(fois(unitaire(ajouter(p[a as usize], p[b as usize])), rayon));
                    p.len() as u32 - 1
                })
            };
            for [a, b, c] in f {
                let (ab, bc, ca) = (milieu(a, b, &mut p), milieu(b, c, &mut p), milieu(c, a, &mut p));
                g.extend_from_slice(&[[a, ab, ca], [ab, b, bc], [ca, bc, c], [ab, bc, ca]]);
            }
            f = g;
        }
        Maillage::nouveau(p, f)
    }

    /// Un instantané pour annuler : les positions, et les triangles partagés.
    pub fn instantane(&self) -> Instantane {
        Instantane { positions: self.positions.clone(), triangles: self.triangles.clone() }
    }

    /// Revient à un instantané (la topologie n'est refaite que si elle a changé).
    pub fn restaurer(&mut self, i: Instantane) {
        let meme = Arc::ptr_eq(&i.triangles, &self.triangles) && i.positions.len() == self.positions.len();
        self.positions = i.positions;
        if !meme {
            self.triangles = i.triangles;
            self.relier();
        }
        self.normaliser();
    }

    /// Refait les voisinages (après un changement de topologie).
    fn relier(&mut self) {
        let mut v: Vec<Vec<u32>> = vec![Vec::new(); self.positions.len()];
        for t in self.triangles.iter() {
            for k in 0..3 {
                let (a, b) = (t[k], t[(k + 1) % 3]);
                v[a as usize].push(b);
                v[b as usize].push(a);
            }
        }
        for l in &mut v {
            l.sort_unstable();
            l.dedup();
        }
        self.voisins = Arc::new(v);
    }

    /// Refait les normales des sommets (moyenne des faces, pondérée par leur aire).
    pub fn normaliser(&mut self) {
        let mut n = vec![[0.0f32; 3]; self.positions.len()];
        for t in self.triangles.iter() {
            let [a, b, c] = t.map(|i| self.positions[i as usize]);
            let f = vectoriel(oter(b, a), oter(c, a));
            for i in t {
                n[*i as usize] = ajouter(n[*i as usize], f);
            }
        }
        self.normales = n.into_iter().map(unitaire).collect();
    }

    /// Subdivision de Loop : chaque triangle en quatre, la forme adoucie. Faux si le
    /// maillage est déjà trop fin.
    pub fn subdiviser(&mut self) -> bool {
        if self.triangles.len() * 4 > TRIANGLES_MAX {
            return false;
        }
        let n = self.positions.len();
        let cle = |a: u32, b: u32| (a.min(b), a.max(b));
        // Pour chaque arête : ses deux sommets opposés, et combien de faces la bordent
        let mut aretes: HashMap<(u32, u32), (u32, u32, u8)> = HashMap::new();
        for t in self.triangles.iter() {
            for k in 0..3 {
                let (a, b, c) = (t[k], t[(k + 1) % 3], t[(k + 2) % 3]);
                let e = aretes.entry(cle(a, b)).or_insert((c, u32::MAX, 0));
                if e.2 == 1 {
                    e.1 = c;
                }
                e.2 += 1;
            }
        }
        let p = &self.positions;
        let mut bord = vec![false; n];
        let mut nouveaux = p.clone();
        let mut indice: HashMap<(u32, u32), u32> = HashMap::with_capacity(aretes.len());
        let mut cles: Vec<_> = aretes.keys().copied().collect();
        cles.sort_unstable();
        for (a, b) in cles {
            let (o1, o2, c) = aretes[&(a, b)];
            let (pa, pb) = (p[a as usize], p[b as usize]);
            let q = if c == 2 {
                ajouter(fois(ajouter(pa, pb), 3.0 / 8.0), fois(ajouter(p[o1 as usize], p[o2 as usize]), 1.0 / 8.0))
            } else {
                bord[a as usize] = true;
                bord[b as usize] = true;
                fois(ajouter(pa, pb), 0.5)
            };
            indice.insert((a, b), nouveaux.len() as u32);
            nouveaux.push(q);
        }
        // Les anciens sommets se rapprochent de leurs voisins (ceux du bord restent)
        for v in 0..n {
            let vs = &self.voisins[v];
            let k = vs.len();
            if bord[v] || k < 3 {
                continue;
            }
            let beta = if k == 3 { 3.0 / 16.0 } else { 3.0 / (8.0 * k as f32) };
            let somme = vs.iter().fold([0.0; 3], |s, w| ajouter(s, p[*w as usize]));
            nouveaux[v] = ajouter(fois(p[v], 1.0 - k as f32 * beta), fois(somme, beta));
        }
        let mut f = Vec::with_capacity(self.triangles.len() * 4);
        for &[a, b, c] in self.triangles.iter() {
            let (ab, bc, ca) = (indice[&cle(a, b)], indice[&cle(b, c)], indice[&cle(c, a)]);
            f.extend_from_slice(&[[a, ab, ca], [ab, b, bc], [ca, bc, c], [ab, bc, ca]]);
        }
        *self = Maillage::nouveau(nouveaux, f);
        true
    }

    /// Le premier point touché par le rayon (origine, direction unitaire) : la
    /// distance et le point.
    pub fn toucher(&self, o: V3, d: V3) -> Option<(f32, V3)> {
        let mut mieux: Option<f32> = None;
        for t in self.triangles.iter() {
            let [a, b, c] = t.map(|i| self.positions[i as usize]);
            let (e1, e2) = (oter(b, a), oter(c, a));
            let h = vectoriel(d, e2);
            let det = scalaire(e1, h);
            if det.abs() < 1e-12 {
                continue;
            }
            let inv = 1.0 / det;
            let s = oter(o, a);
            let u = scalaire(s, h) * inv;
            if !(0.0..=1.0).contains(&u) {
                continue;
            }
            let q = vectoriel(s, e1);
            let v = scalaire(d, q) * inv;
            if v < 0.0 || u + v > 1.0 {
                continue;
            }
            let dist = scalaire(e2, q) * inv;
            if dist > 1e-5 && mieux.is_none_or(|m| dist < m) {
                mieux = Some(dist);
            }
        }
        mieux.map(|t| (t, ajouter(o, fois(d, t))))
    }

    /// Les sommets sous un coup de pinceau : (sommet, poids de 0 à 1, côté miroir).
    pub fn influence(&self, centre: V3, rayon: f32, symetrie: bool) -> Vec<(u32, f32, bool)> {
        let r2 = rayon * rayon;
        let cm = miroir(centre);
        let poids = |p: V3, c: V3| {
            let d = oter(p, c);
            let x = scalaire(d, d) / r2;
            if x < 1.0 { (1.0 - x) * (1.0 - x) } else { 0.0 }
        };
        let mut v = Vec::new();
        for (i, p) in self.positions.iter().enumerate() {
            let w = poids(*p, centre);
            let wm = if symetrie { poids(*p, cm) } else { 0.0 };
            if w > 0.0 || wm > 0.0 {
                v.push((i as u32, w.max(wm), wm > w));
            }
        }
        v
    }

    /// Les sommets et triangles, prêts pour la carte graphique : position et normale
    /// par sommet.
    pub fn plat(&self) -> Vec<f32> {
        let mut v = Vec::with_capacity(self.positions.len() * 6);
        for (p, n) in self.positions.iter().zip(&self.normales) {
            v.extend_from_slice(p);
            v.extend_from_slice(n);
        }
        v
    }

    pub fn indices(&self) -> Vec<u32> {
        self.triangles.iter().flatten().copied().collect()
    }

    /// La boîte qui l'enveloppe : (min, max).
    pub fn boite(&self) -> (V3, V3) {
        let mut a = [f32::MAX; 3];
        let mut b = [f32::MIN; 3];
        for p in &self.positions {
            for k in 0..3 {
                a[k] = a[k].min(p[k]);
                b[k] = b[k].max(p[k]);
            }
        }
        (a, b)
    }
}

/// Un état du maillage, pour annuler ou rétablir.
#[derive(Clone, Debug, PartialEq)]
pub struct Instantane {
    pub positions: Vec<V3>,
    pub triangles: Arc<Vec<[u32; 3]>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pinceau {
    /// Pousse la matière vers l'extérieur.
    Gonfler,
    /// La creuse.
    Creuser,
    /// Rapproche chaque sommet de ses voisins.
    Lisser,
    /// Ramène la surface sur un plan.
    Aplatir,
    /// Resserre la matière vers le centre du coup (des arêtes vives).
    Pincer,
    /// Saisit la matière et l'emmène avec le doigt.
    Tirer,
}

impl Pinceau {
    pub const TOUS: [Pinceau; 6] = [Pinceau::Gonfler, Pinceau::Creuser, Pinceau::Lisser, Pinceau::Aplatir, Pinceau::Pincer, Pinceau::Tirer];

    pub fn nom(self) -> &'static str {
        match self {
            Pinceau::Gonfler => "Gonfler",
            Pinceau::Creuser => "Creuser",
            Pinceau::Lisser => "Lisser",
            Pinceau::Aplatir => "Aplatir",
            Pinceau::Pincer => "Pincer",
            Pinceau::Tirer => "Tirer",
        }
    }

    pub fn depuis(nom: &str) -> Option<Pinceau> {
        Pinceau::TOUS.into_iter().find(|p| p.nom() == nom)
    }
}

/// Un coup de pinceau en `centre`, de rayon `rayon` (en unités du modèle) et de
/// force `force` (0 à 1). `Tirer` passe par `Saisie`.
pub fn appliquer(m: &mut Maillage, pinceau: Pinceau, centre: V3, rayon: f32, force: f32, symetrie: bool) {
    coup(m, pinceau, centre, rayon, force, symetrie);
    m.normaliser();
}

/// Un coup sans refaire les normales : pour enchaîner les coups d'un même geste,
/// puis appeler `Maillage::normaliser` une fois.
pub fn coup(m: &mut Maillage, pinceau: Pinceau, centre: V3, rayon: f32, force: f32, symetrie: bool) {
    let zone = m.influence(centre, rayon, symetrie);
    if zone.is_empty() {
        return;
    }
    // La normale moyenne du côté direct (sinon, celle du miroir reflétée)
    let mut n = [0.0; 3];
    let mut c0 = [0.0; 3];
    let mut total = 0.0;
    for &(i, w, cote) in &zone {
        let (ni, pi) = (m.normales[i as usize], m.positions[i as usize]);
        let (ni, pi) = if cote { (miroir(ni), miroir(pi)) } else { (ni, pi) };
        n = ajouter(n, fois(ni, w));
        c0 = ajouter(c0, fois(pi, w));
        total += w;
    }
    let n = unitaire(n);
    let c0 = fois(c0, 1.0 / total.max(1e-9));
    let cote = |miroite: bool, v: V3| if miroite { miroir(v) } else { v };
    match pinceau {
        Pinceau::Gonfler | Pinceau::Creuser => {
            let s = if pinceau == Pinceau::Gonfler { 1.0 } else { -1.0 };
            for &(i, w, mi) in &zone {
                let p = &mut m.positions[i as usize];
                *p = ajouter(*p, fois(cote(mi, n), s * rayon * 0.06 * force * w));
            }
        }
        Pinceau::Lisser => {
            let cibles: Vec<V3> = zone
                .iter()
                .map(|&(i, _, _)| {
                    let vs = &m.voisins[i as usize];
                    let somme = vs.iter().fold([0.0; 3], |s, v| ajouter(s, m.positions[*v as usize]));
                    if vs.is_empty() { m.positions[i as usize] } else { fois(somme, 1.0 / vs.len() as f32) }
                })
                .collect();
            for (&(i, w, _), c) in zone.iter().zip(cibles) {
                let p = &mut m.positions[i as usize];
                *p = ajouter(*p, fois(oter(c, *p), 0.5 * force * w));
            }
        }
        Pinceau::Aplatir => {
            for &(i, w, mi) in &zone {
                let (nn, cc) = (cote(mi, n), cote(mi, c0));
                let p = &mut m.positions[i as usize];
                *p = oter(*p, fois(nn, scalaire(oter(*p, cc), nn) * 0.3 * force * w));
            }
        }
        Pinceau::Pincer => {
            for &(i, w, mi) in &zone {
                let (nn, cc) = (cote(mi, n), cote(mi, centre));
                let p = &mut m.positions[i as usize];
                let v = oter(cc, *p);
                let v = oter(v, fois(nn, scalaire(v, nn)));
                *p = ajouter(*p, fois(v, 0.15 * force * w));
            }
        }
        Pinceau::Tirer => {}
    }
}

/// La matière saisie par `Tirer` : les sommets, leur poids, leur côté, et où ils
/// étaient.
#[derive(Clone, Debug, PartialEq)]
pub struct Saisie {
    zone: Vec<(u32, f32, bool)>,
    depart: Vec<V3>,
}

impl Saisie {
    pub fn prendre(m: &Maillage, centre: V3, rayon: f32, symetrie: bool) -> Saisie {
        let zone = m.influence(centre, rayon, symetrie);
        let depart = zone.iter().map(|z| m.positions[z.0 as usize]).collect();
        Saisie { zone, depart }
    }

    /// Emmène la matière saisie de `d` depuis son départ.
    pub fn deplacer(&self, m: &mut Maillage, d: V3) {
        for (&(i, w, mi), p) in self.zone.iter().zip(&self.depart) {
            let d = if mi { miroir(d) } else { d };
            m.positions[i as usize] = ajouter(*p, fois(d, w));
        }
        m.normaliser();
    }
}

// --- La caméra --------------------------------------------------------------------

/// L'angle de vue vertical, en radians.
pub const CHAMP: f32 = 0.7;

/// Une caméra qui tourne autour de `cible`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub lacet: f32,
    pub tangage: f32,
    pub distance: f32,
    pub cible: V3,
}

impl Default for Camera {
    fn default() -> Camera {
        Camera { lacet: 0.5, tangage: 0.3, distance: 4.2, cible: [0.0; 3] }
    }
}

/// Produit de deux matrices 4×4 en colonnes : `a` après `b`.
pub fn produit(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
    let mut c = [0.0; 16];
    for col in 0..4 {
        for lig in 0..4 {
            c[col * 4 + lig] = (0..4).map(|k| a[k * 4 + lig] * b[col * 4 + k]).sum();
        }
    }
    c
}

impl Camera {
    pub fn oeil(&self) -> V3 {
        let (sl, cl) = self.lacet.sin_cos();
        let (st, ct) = self.tangage.sin_cos();
        ajouter(self.cible, fois([ct * sl, st, ct * cl], self.distance))
    }

    /// Les axes du regard : devant, droite, haut.
    pub fn axes(&self) -> (V3, V3, V3) {
        let f = unitaire(oter(self.cible, self.oeil()));
        let r = unitaire(vectoriel(f, [0.0, 1.0, 0.0]));
        let u = vectoriel(r, f);
        (f, r, u)
    }

    /// Du monde au regard (en colonnes).
    pub fn vue(&self) -> [f32; 16] {
        let (f, r, u) = self.axes();
        let e = self.oeil();
        [r[0], u[0], -f[0], 0.0, r[1], u[1], -f[1], 0.0, r[2], u[2], -f[2], 0.0, -scalaire(r, e), -scalaire(u, e), scalaire(f, e), 1.0]
    }

    pub fn mvp(&self, aspect: f32) -> [f32; 16] {
        let fy = 1.0 / (CHAMP / 2.0).tan();
        let (pres, loin) = (0.02 * self.distance, 50.0 * self.distance);
        let mut p = [0.0; 16];
        p[0] = fy / aspect;
        p[5] = fy;
        p[10] = (loin + pres) / (pres - loin);
        p[11] = -1.0;
        p[14] = 2.0 * loin * pres / (pres - loin);
        produit(&p, &self.vue())
    }

    /// Le rayon qui passe par le point (nx, ny) de la vue, de -1 à 1 (y vers le
    /// haut) : origine et direction unitaire.
    pub fn rayon(&self, aspect: f32, nx: f32, ny: f32) -> (V3, V3) {
        let (f, r, u) = self.axes();
        let k = (CHAMP / 2.0).tan();
        (self.oeil(), unitaire(ajouter(f, ajouter(fois(r, nx * k * aspect), fois(u, ny * k)))))
    }

    /// Le rayon du monde qui couvre `px` pixels à la profondeur de `p`, dans une
    /// vue haute de `h` pixels.
    pub fn taille_a(&self, p: V3, px: f32, h: f32) -> f32 {
        let (f, _, _) = self.axes();
        let profondeur = scalaire(oter(p, self.oeil()), f).max(1e-3);
        px / (h / 2.0) * (CHAMP / 2.0).tan() * profondeur
    }

    /// Où le rayon (o, d) traverse le plan face au regard qui passe par `p`.
    pub fn sur_plan(&self, p: V3, o: V3, d: V3) -> V3 {
        let (f, _, _) = self.axes();
        let t = scalaire(oter(p, o), f) / scalaire(d, f).max(1e-6);
        ajouter(o, fois(d, t))
    }

    /// Cadre une boîte : la vise et recule assez pour la voir entière.
    pub fn cadrer(&mut self, (a, b): (V3, V3)) {
        self.cible = fois(ajouter(a, b), 0.5);
        let r = norme(oter(b, a)) / 2.0;
        self.distance = (r / (CHAMP / 2.0).sin() * 1.15).max(0.1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_boule_est_close_et_tournee_vers_l_exterieur() {
        let m = Maillage::boule(2, 1.0);
        assert_eq!(m.triangles.len(), 320);
        assert_eq!(m.positions.len(), 162);
        for (p, n) in m.positions.iter().zip(&m.normales) {
            assert!((norme(*p) - 1.0).abs() < 1e-5);
            assert!(scalaire(*p, *n) > 0.9);
        }
        // Chaque arête borde deux faces : 5 ou 6 voisins par sommet
        assert!(m.voisins.iter().all(|v| v.len() == 5 || v.len() == 6));
    }

    #[test]
    fn subdiviser_adoucit_sans_trouer() {
        let mut m = Maillage::boule(1, 1.0);
        assert!(m.subdiviser());
        assert_eq!(m.triangles.len(), 320);
        assert_eq!(m.positions.len(), 162);
        // Loop rétrécit un peu, sans rien déchirer
        for p in &m.positions {
            let r = norme(*p);
            assert!(r > 0.8 && r < 1.0, "{r}");
        }
        for (p, n) in m.positions.iter().zip(&m.normales) {
            assert!(scalaire(unitaire(*p), *n) > 0.9);
        }
        let mut gros = Maillage::boule(1, 1.0);
        gros.triangles = Arc::new(vec![[0, 1, 2]; TRIANGLES_MAX / 2]);
        assert!(!gros.subdiviser());
    }

    #[test]
    fn le_rayon_touche_la_boule() {
        let m = Maillage::boule(3, 1.0);
        let (t, p) = m.toucher([0.0, 0.0, 5.0], [0.0, 0.0, -1.0]).unwrap();
        assert!((t - 4.0).abs() < 0.02, "{t}");
        assert!((p[2] - 1.0).abs() < 0.02);
        assert!(m.toucher([0.0, 3.0, 5.0], [0.0, 0.0, -1.0]).is_none());
    }

    #[test]
    fn les_pinceaux() {
        let base = Maillage::boule(3, 1.0);
        let haut = [0.0, 1.0, 0.0];
        let pres = |m: &Maillage| m.positions.iter().enumerate().max_by(|a, b| a.1[1].total_cmp(&b.1[1])).unwrap().0;
        let i = pres(&base);
        // Gonfler soulève le haut, creuser l'enfonce
        let mut m = base.clone();
        appliquer(&mut m, Pinceau::Gonfler, haut, 0.5, 1.0, false);
        assert!(m.positions[i][1] > 1.02);
        let mut m = base.clone();
        appliquer(&mut m, Pinceau::Creuser, haut, 0.5, 1.0, false);
        assert!(m.positions[i][1] < 0.98);
        // Hors de portée, rien ne bouge
        assert_eq!(m.positions.iter().filter(|p| p[1] < 0.0).count(), base.positions.iter().filter(|p| p[1] < 0.0).count());
        let bas = base.positions.iter().position(|p| p[1] < -0.99).unwrap();
        assert_eq!(m.positions[bas], base.positions[bas]);
        // Lisser rapproche une bosse de ses voisins
        let mut m = base.clone();
        m.positions[i] = [0.0, 1.5, 0.0];
        appliquer(&mut m, Pinceau::Lisser, [0.0, 1.5, 0.0], 0.3, 1.0, false);
        assert!(m.positions[i][1] < 1.4);
        // Aplatir ramène le haut vers un plan
        let mut m = base.clone();
        for _ in 0..20 {
            appliquer(&mut m, Pinceau::Aplatir, haut, 0.6, 1.0, false);
        }
        let ys: Vec<f32> = m.positions.iter().filter(|p| p[0].hypot(p[2]) < 0.2 && p[1] > 0.0).map(|p| p[1]).collect();
        let (lo, hi) = ys.iter().fold((f32::MAX, f32::MIN), |a, y| (a.0.min(*y), a.1.max(*y)));
        assert!(hi - lo < 0.01, "{lo} {hi}");
        // Tirer emmène le sommet touché de tout le déplacement
        let mut m = base.clone();
        let s = Saisie::prendre(&m, base.positions[i], 0.4, false);
        s.deplacer(&mut m, [0.0, 0.5, 0.0]);
        assert!((m.positions[i][1] - 1.5).abs() < 1e-4);
    }

    #[test]
    fn la_symetrie_reflete_le_coup() {
        let base = Maillage::boule(3, 1.0);
        let mut m = base.clone();
        let c = unitaire([0.7, 0.3, 0.6]);
        appliquer(&mut m, Pinceau::Gonfler, c, 0.4, 1.0, true);
        // Chaque sommet et son reflet ont bougé pareil
        for (k, p) in base.positions.iter().enumerate() {
            if let Some(j) = base.positions.iter().position(|q| norme(oter(*q, miroir(*p))) < 1e-4) {
                let (a, b) = (m.positions[k], miroir(m.positions[j]));
                assert!(norme(oter(a, b)) < 1e-4, "{k} {j}");
            }
        }
        assert_ne!(m, base);
    }

    #[test]
    fn la_camera_vise_et_lance_des_rayons() {
        let c = Camera { lacet: 0.0, tangage: 0.0, distance: 5.0, cible: [0.0; 3] };
        assert!(norme(oter(c.oeil(), [0.0, 0.0, 5.0])) < 1e-5);
        // Le centre de la vue vise la cible
        let (o, d) = c.rayon(1.5, 0.0, 0.0);
        assert!(norme(oter(ajouter(o, fois(d, 5.0)), [0.0; 3])) < 1e-4);
        // La cible se projette au centre, un point en haut vers le haut
        let m = c.mvp(1.5);
        let proj = |p: V3| {
            let x = m[0] * p[0] + m[4] * p[1] + m[8] * p[2] + m[12];
            let y = m[1] * p[0] + m[5] * p[1] + m[9] * p[2] + m[13];
            let w = m[3] * p[0] + m[7] * p[1] + m[11] * p[2] + m[15];
            (x / w, y / w)
        };
        let (x, y) = proj([0.0; 3]);
        assert!(x.abs() < 1e-5 && y.abs() < 1e-5);
        let (_, y) = proj([0.0, 1.0, 0.0]);
        assert!(y > 0.0);
        // Un rayon au bord haut de la vue arrive sur ce point projeté
        let (o, d) = c.rayon(1.5, 0.0, y);
        let p = c.sur_plan([0.0, 1.0, 0.0], o, d);
        assert!(norme(oter(p, [0.0, 1.0, 0.0])) < 1e-3, "{p:?}");
        // Sa taille : la demi-hauteur de la vue couvre tan(champ/2)·distance
        assert!((c.taille_a([0.0; 3], 100.0, 200.0) - (CHAMP / 2.0).tan() * 5.0).abs() < 1e-4);
    }
}
