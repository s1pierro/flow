//! Le modèle du contexte Mailles : des objets faits de sommets et de faces
//! (polygones), la sélection (sommets, arêtes ou faces), et les opérations de
//! géométrie.
//!
//! - Les sommets sont dans le repère de la scène ; chaque objet garde sa matrice
//!   d'origine, pour se réécrire à sa place.
//! - Une face est un polygone (trois sommets ou plus, dans le sens direct vu de
//!   dehors) ; les arêtes se déduisent des faces.
//! - La sélection suit le mode : des sommets, des arêtes ou des faces. Une arête ou
//!   une face est « couverte » quand tous ses sommets le sont.
//! - `OPERATIONS` est le tableau des opérations : leur nom, à quoi elles servent, et
//!   quand elles s'appliquent. La barre rapide de la vue 3D n'en montre que celles
//!   que la sélection permet.

use crate::sculpture::{ajouter, fois, norme, oter, scalaire, unitaire, vectoriel, V3};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// Ce qu'on choisit en touchant la forme.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Sommets,
    Aretes,
    Faces,
}

impl Mode {
    pub const TOUS: [Mode; 3] = [Mode::Sommets, Mode::Aretes, Mode::Faces];

    pub fn nom(self) -> &'static str {
        match self {
            Mode::Sommets => "Sommets",
            Mode::Aretes => "Arêtes",
            Mode::Faces => "Faces",
        }
    }

    pub fn cle(self) -> &'static str {
        match self {
            Mode::Sommets => "sommets",
            Mode::Aretes => "aretes",
            Mode::Faces => "faces",
        }
    }

    pub fn depuis(t: &str) -> Option<Mode> {
        Mode::TOUS.into_iter().find(|m| m.cle() == t)
    }
}

/// Une arête : ses deux sommets, le plus petit d'abord.
pub type Arete = (u32, u32);

pub fn arete(a: u32, b: u32) -> Arete {
    (a.min(b), a.max(b))
}

/// Un objet : un nom, sa place d'origine, des sommets et des faces, ses modifieurs.
#[derive(Clone, Debug, PartialEq)]
pub struct Objet {
    /// Unique le temps d'une session (les modifieurs désignent leurs cibles ainsi) ; il
    /// ne s'enregistre pas.
    pub id: u64,
    pub nom: String,
    /// La matrice du nœud lu (en colonnes) : les sommets se réécrivent dans son repère.
    pub matrice: [f32; 16],
    pub sommets: Vec<V3>,
    pub faces: Vec<Vec<u32>>,
    pub couleur: [f32; 3],
    pub visible: bool,
    pub modifs: Vec<crate::modifieurs::Modifieur>,
}

static IDS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Un identifiant d'objet neuf.
pub fn nouvel_id() -> u64 {
    IDS.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

pub const IDENTITE: [f32; 16] = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];

/// Les faces d'un cube de côté 2 centré en `c` (dans le sens direct vu de dehors).
pub fn cube(nom: &str, c: V3, demi: f32) -> Objet {
    let s: Vec<V3> = (0..8).map(|i| [c[0] + if i & 1 == 0 { -demi } else { demi }, c[1] + if i & 2 == 0 { -demi } else { demi }, c[2] + if i & 4 == 0 { -demi } else { demi }]).collect();
    let faces = vec![vec![0, 2, 3, 1], vec![4, 5, 7, 6], vec![0, 1, 5, 4], vec![2, 6, 7, 3], vec![0, 4, 6, 2], vec![1, 3, 7, 5]];
    Objet { id: nouvel_id(), nom: nom.into(), matrice: IDENTITE, sommets: s, faces, couleur: [0.72, 0.74, 0.78], visible: true, modifs: Vec::new() }
}

impl Objet {
    /// Depuis des triangles : deux triangles voisins presque coplanaires qui forment un
    /// quadrilatère convexe se réunissent (comme à l'export, qui les avait coupés).
    pub fn depuis_triangles(nom: &str, matrice: [f32; 16], sommets: Vec<V3>, triangles: &[[u32; 3]], couleur: [f32; 3]) -> Objet {
        let mut o = Objet { id: nouvel_id(), nom: nom.into(), matrice, sommets, faces: triangles.iter().map(|t| t.to_vec()).collect(), couleur, visible: true, modifs: Vec::new() };
        o.reunir_triangles();
        o
    }

    pub fn normale(&self, f: usize) -> V3 {
        let p = &self.faces[f];
        // Newell : robuste pour un polygone quelconque
        let mut n = [0.0f32; 3];
        for i in 0..p.len() {
            let (a, b) = (self.sommets[p[i] as usize], self.sommets[p[(i + 1) % p.len()] as usize]);
            n[0] += (a[1] - b[1]) * (a[2] + b[2]);
            n[1] += (a[2] - b[2]) * (a[0] + b[0]);
            n[2] += (a[0] - b[0]) * (a[1] + b[1]);
        }
        if norme(n) < 1e-12 {
            [0.0, 0.0, 1.0]
        } else {
            unitaire(n)
        }
    }

    pub fn centre_face(&self, f: usize) -> V3 {
        centre(self.faces[f].iter().map(|&i| self.sommets[i as usize]))
    }

    /// Les arêtes, une fois chacune.
    pub fn aretes(&self) -> Vec<Arete> {
        let mut s = BTreeSet::new();
        for p in &self.faces {
            for i in 0..p.len() {
                s.insert(arete(p[i], p[(i + 1) % p.len()]));
            }
        }
        s.into_iter().collect()
    }

    /// Les faces qui bordent chaque arête.
    pub fn faces_par_arete(&self) -> HashMap<Arete, Vec<usize>> {
        let mut m: HashMap<Arete, Vec<usize>> = HashMap::new();
        for (f, p) in self.faces.iter().enumerate() {
            for i in 0..p.len() {
                m.entry(arete(p[i], p[(i + 1) % p.len()])).or_default().push(f);
            }
        }
        m
    }

    /// Les triangles des faces (en éventail) : pour l'affichage et le toucher, chacun
    /// avec sa face.
    pub fn triangles(&self) -> Vec<([u32; 3], usize)> {
        let mut t = Vec::new();
        for (f, p) in self.faces.iter().enumerate() {
            for i in 1..p.len().saturating_sub(1) {
                t.push(([p[0], p[i], p[i + 1]], f));
            }
        }
        t
    }

    pub fn boite(&self) -> Option<(V3, V3)> {
        let mut it = self.sommets.iter();
        let p = *it.next()?;
        Some(it.fold((p, p), |(a, b), q| ([a[0].min(q[0]), a[1].min(q[1]), a[2].min(q[2])], [b[0].max(q[0]), b[1].max(q[1]), b[2].max(q[2])])))
    }

    /// Retire les sommets qu'aucune face n'utilise (hors de `garder`), et renumérote.
    /// Renvoie l'ancien numéro → le nouveau.
    fn compacter(&mut self, garder: &BTreeSet<u32>) -> HashMap<u32, u32> {
        let mut utilise = vec![false; self.sommets.len()];
        for p in &self.faces {
            for &i in p {
                utilise[i as usize] = true;
            }
        }
        for &i in garder {
            if let Some(u) = utilise.get_mut(i as usize) {
                *u = true;
            }
        }
        let mut nouveau = HashMap::new();
        let mut sommets = Vec::new();
        for (i, p) in self.sommets.iter().enumerate() {
            if utilise[i] {
                nouveau.insert(i as u32, sommets.len() as u32);
                sommets.push(*p);
            }
        }
        self.sommets = sommets;
        for p in &mut self.faces {
            for i in p.iter_mut() {
                *i = nouveau[i];
            }
        }
        nouveau
    }

    /// Réunit les paires de triangles coplanaires en quadrilatères convexes.
    fn reunir_triangles(&mut self) {
        let par_arete = self.faces_par_arete();
        let mut pris = vec![false; self.faces.len()];
        let mut faces = Vec::new();
        // Les arêtes dans un ordre stable
        let mut aretes: Vec<(&Arete, &Vec<usize>)> = par_arete.iter().collect();
        aretes.sort();
        for (a, fs) in aretes {
            let [f, g] = fs[..] else { continue };
            if pris[f] || pris[g] || self.faces[f].len() != 3 || self.faces[g].len() != 3 {
                continue;
            }
            if scalaire(self.normale(f), self.normale(g)) < 0.9995 {
                continue;
            }
            // Le quadrilatère : la face f, avec le sommet opposé de g inséré dans l'arête
            let pf = &self.faces[f];
            let autre = *self.faces[g].iter().find(|&&i| i != a.0 && i != a.1).unwrap();
            let k = (0..3).find(|&k| arete(pf[k], pf[(k + 1) % 3]) == *a).unwrap();
            let quad = vec![pf[k], autre, pf[(k + 1) % 3], pf[(k + 2) % 3]];
            if !convexe(&quad.iter().map(|&i| self.sommets[i as usize]).collect::<Vec<_>>()) {
                continue;
            }
            pris[f] = true;
            pris[g] = true;
            faces.push(quad);
        }
        for (f, p) in self.faces.iter().enumerate() {
            if !pris[f] {
                faces.push(p.clone());
            }
        }
        self.faces = faces;
    }
}

fn centre(it: impl Iterator<Item = V3>) -> V3 {
    let (mut s, mut n) = ([0.0f32; 3], 0.0);
    for p in it {
        s = ajouter(s, p);
        n += 1.0;
    }
    if n > 0.0 {
        fois(s, 1.0 / n)
    } else {
        s
    }
}

/// Un quadrilatère plan est-il convexe ?
fn convexe(p: &[V3]) -> bool {
    let n = p.len();
    let mut signe = None;
    let reference = {
        let mut r = [0.0f32; 3];
        for i in 0..n {
            r = ajouter(r, vectoriel(oter(p[(i + 1) % n], p[i]), oter(p[(i + 2) % n], p[(i + 1) % n])));
        }
        r
    };
    for i in 0..n {
        let c = vectoriel(oter(p[(i + 1) % n], p[i]), oter(p[(i + 2) % n], p[(i + 1) % n]));
        let s = scalaire(c, reference) > 1e-9;
        if *signe.get_or_insert(s) != s {
            return false;
        }
    }
    true
}

/// La sélection, dans le mode courant.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Selection {
    pub sommets: BTreeSet<u32>,
    pub aretes: BTreeSet<Arete>,
    pub faces: BTreeSet<usize>,
}

impl Selection {
    pub fn vide(&self) -> bool {
        self.sommets.is_empty() && self.aretes.is_empty() && self.faces.is_empty()
    }

    /// Les sommets choisis, ou ceux des arêtes ou faces choisies.
    pub fn sommets_de(&self, o: &Objet, mode: Mode) -> BTreeSet<u32> {
        match mode {
            Mode::Sommets => self.sommets.iter().copied().filter(|&i| (i as usize) < o.sommets.len()).collect(),
            Mode::Aretes => self.aretes.iter().flat_map(|a| [a.0, a.1]).collect(),
            Mode::Faces => self.faces.iter().filter_map(|&f| o.faces.get(f)).flatten().copied().collect(),
        }
    }

    /// Les faces choisies, ou celles que la sélection couvre entièrement.
    pub fn faces_de(&self, o: &Objet, mode: Mode) -> BTreeSet<usize> {
        match mode {
            Mode::Faces => self.faces.iter().copied().filter(|&f| f < o.faces.len()).collect(),
            _ => {
                let s = self.sommets_de(o, mode);
                (0..o.faces.len()).filter(|&f| o.faces[f].iter().all(|i| s.contains(i))).collect()
            }
        }
    }

    pub fn aretes_de(&self, o: &Objet, mode: Mode) -> BTreeSet<Arete> {
        match mode {
            Mode::Aretes => self.aretes.clone(),
            _ => {
                let s = self.sommets_de(o, mode);
                o.aretes().into_iter().filter(|a| s.contains(&a.0) && s.contains(&a.1)).collect()
            }
        }
    }

    /// La même sélection, vue dans un autre mode.
    pub fn convertir(&self, o: &Objet, de: Mode, vers: Mode) -> Selection {
        let mut s = Selection::default();
        match vers {
            Mode::Sommets => s.sommets = self.sommets_de(o, de),
            Mode::Aretes => s.aretes = self.aretes_de(o, de),
            Mode::Faces => s.faces = self.faces_de(o, de),
        }
        s
    }

    /// Ajoute ou retire un élément touché.
    pub fn basculer(&mut self, e: Element) {
        fn b<T: Ord>(s: &mut BTreeSet<T>, x: T) {
            if !s.remove(&x) {
                s.insert(x);
            }
        }
        match e {
            Element::Sommet(i) => b(&mut self.sommets, i),
            Element::Arete(a) => b(&mut self.aretes, a),
            Element::Face(f) => b(&mut self.faces, f),
        }
    }

    /// Le nombre d'éléments choisis, dans le mode.
    pub fn nombre(&self, mode: Mode) -> usize {
        match mode {
            Mode::Sommets => self.sommets.len(),
            Mode::Aretes => self.aretes.len(),
            Mode::Faces => self.faces.len(),
        }
    }
}

/// Un élément touché.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Element {
    Sommet(u32),
    Arete(Arete),
    Face(usize),
}

// --- Les opérations -------------------------------------------------------------

/// Ce que les opérations regardent pour savoir si elles s'appliquent.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Contexte {
    pub mode: Mode,
    pub objets: usize,
    /// L'objet actif a des sommets.
    pub plein: bool,
    pub sommets: usize,
    pub aretes: usize,
    pub faces: usize,
    /// Des faces choisies ont plus de trois côtés.
    pub polygones: bool,
    /// Tout l'objet est choisi.
    pub tout: bool,
}

impl Contexte {
    pub fn de(objets: &[Objet], actif: usize, sel: &Selection, mode: Mode) -> Contexte {
        let Some(o) = objets.get(actif) else { return Contexte { mode, ..Contexte::default() } };
        let s = sel.sommets_de(o, mode);
        let f = sel.faces_de(o, mode);
        Contexte {
            mode,
            objets: objets.len(),
            plein: !o.sommets.is_empty(),
            sommets: s.len(),
            aretes: sel.aretes_de(o, mode).len(),
            faces: f.len(),
            polygones: f.iter().any(|&i| o.faces[i].len() > 3),
            tout: !o.sommets.is_empty() && s.len() == o.sommets.len(),
        }
    }
}

/// Une transformation qu'on mène du doigt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transfo {
    Deplacer,
    Tourner,
    Echelle,
}

/// Une entrée du tableau des opérations.
pub struct Operation {
    pub cle: &'static str,
    pub nom: &'static str,
    pub aide: &'static str,
    /// Le groupe, pour le tableau : Sélection, Transformer, Géométrie, Objet.
    pub groupe: &'static str,
    pub possible: fn(&Contexte) -> bool,
}

/// Le tableau des opérations, dans l'ordre de la barre rapide.
pub const OPERATIONS: &[Operation] = &[
    Operation { cle: "deplacer", nom: "Déplacer", aide: "Glisser emmène la sélection, face au regard", groupe: "Transformer", possible: |c| c.sommets > 0 },
    Operation { cle: "tourner", nom: "Tourner", aide: "Glisser fait tourner la sélection autour de son centre", groupe: "Transformer", possible: |c| c.sommets > 1 },
    Operation { cle: "echelle", nom: "Échelle", aide: "Glisser à droite agrandit, à gauche rétrécit", groupe: "Transformer", possible: |c| c.sommets > 1 },
    Operation { cle: "extruder", nom: "Extruder", aide: "Tire les faces choisies hors de la forme, avec leurs flancs", groupe: "Géométrie", possible: |c| c.faces > 0 },
    Operation { cle: "subdiviser", nom: "Subdiviser", aide: "Coupe chaque face choisie en quatre (ou en autant que de côtés)", groupe: "Géométrie", possible: |c| c.faces > 0 },
    Operation { cle: "fusionner", nom: "Fusionner", aide: "Réunit les sommets choisis en un seul, à leur centre", groupe: "Géométrie", possible: |c| c.sommets > 1 },
    Operation { cle: "face", nom: "Créer une face", aide: "Une face sur les sommets choisis (trois ou plus)", groupe: "Géométrie", possible: |c| c.sommets > 2 && c.faces == 0 },
    Operation { cle: "trianguler", nom: "Trianguler", aide: "Coupe les faces choisies en triangles", groupe: "Géométrie", possible: |c| c.polygones },
    Operation { cle: "retourner", nom: "Retourner", aide: "Inverse le sens des faces choisies (leur dehors)", groupe: "Géométrie", possible: |c| c.faces > 0 },
    Operation { cle: "dupliquer", nom: "Dupliquer", aide: "Copie les faces choisies, prêtes à déplacer", groupe: "Géométrie", possible: |c| c.faces > 0 },
    Operation { cle: "supprimer", nom: "Supprimer", aide: "Retire la sélection (et les faces qui en dépendent)", groupe: "Géométrie", possible: |c| c.sommets > 0 },
    Operation { cle: "tout", nom: "Tout", aide: "Choisit tout l'objet actif", groupe: "Sélection", possible: |c| c.plein && !c.tout },
    Operation { cle: "inverser", nom: "Inverser", aide: "Choisit ce qui ne l'était pas", groupe: "Sélection", possible: |c| c.sommets > 0 && !c.tout },
    Operation { cle: "rien", nom: "Rien", aide: "Vide la sélection", groupe: "Sélection", possible: |c| c.sommets > 0 },
    Operation { cle: "cube", nom: "Ajouter un cube", aide: "Un nouvel objet : un cube, au centre de la vue", groupe: "Objet", possible: |_| true },
];

pub fn operation(cle: &str) -> Option<&'static Operation> {
    OPERATIONS.iter().find(|o| o.cle == cle)
}

/// Le résultat d'une opération de géométrie : la sélection qui suit (dans son mode),
/// et la transformation à enchaîner (après une extrusion, déplacer le long de l'axe).
pub struct Suite {
    pub selection: Selection,
    pub mode: Mode,
    pub transfo: Option<(Transfo, Option<V3>)>,
}

impl Suite {
    fn faces(f: BTreeSet<usize>) -> Suite {
        Suite { selection: Selection { faces: f, ..Selection::default() }, mode: Mode::Faces, transfo: None }
    }
    fn sommets(s: BTreeSet<u32>) -> Suite {
        Suite { selection: Selection { sommets: s, ..Selection::default() }, mode: Mode::Sommets, transfo: None }
    }
}

/// Supprime la sélection : des sommets (et leurs faces), des arêtes (et leurs faces),
/// ou des faces ; les sommets qui ne servent plus disparaissent.
pub fn supprimer(o: &mut Objet, sel: &Selection, mode: Mode) -> Suite {
    match mode {
        Mode::Sommets => {
            let s = sel.sommets_de(o, mode);
            o.faces.retain(|p| !p.iter().any(|i| s.contains(i)));
            // Les sommets choisis s'en vont même seuls
            let mut garder: BTreeSet<u32> = BTreeSet::new();
            let utilises: BTreeSet<u32> = o.faces.iter().flatten().copied().collect();
            for i in 0..o.sommets.len() as u32 {
                if !s.contains(&i) && !utilises.contains(&i) {
                    garder.insert(i);
                }
            }
            o.compacter(&garder);
        }
        Mode::Aretes => {
            let a = &sel.aretes;
            o.faces.retain(|p| !(0..p.len()).any(|i| a.contains(&arete(p[i], p[(i + 1) % p.len()]))));
            o.compacter(&BTreeSet::new());
        }
        Mode::Faces => {
            let f = sel.faces_de(o, mode);
            let mut i = 0;
            o.faces.retain(|_| {
                i += 1;
                !f.contains(&(i - 1))
            });
            o.compacter(&BTreeSet::new());
        }
    }
    Suite { selection: Selection::default(), mode, transfo: None }
}

/// Extrude les faces choisies, d'un bloc : leurs sommets sont doublés, les faces
/// passent sur les doubles, et chaque arête du bord de la région reçoit un flanc.
/// La suite : déplacer le long de la normale moyenne.
pub fn extruder(o: &mut Objet, faces: &BTreeSet<usize>) -> Suite {
    let mut double: BTreeMap<u32, u32> = BTreeMap::new();
    for &f in faces {
        for i in o.faces[f].clone() {
            double.entry(i).or_insert_with(|| {
                o.sommets.push(o.sommets[i as usize]);
                o.sommets.len() as u32 - 1
            });
        }
    }
    // Les arêtes orientées des faces choisies ; une arête du bord n'a pas sa jumelle
    let mut orientees: BTreeSet<(u32, u32)> = BTreeSet::new();
    for &f in faces {
        let p = &o.faces[f];
        for i in 0..p.len() {
            orientees.insert((p[i], p[(i + 1) % p.len()]));
        }
    }
    let mut normale = [0.0f32; 3];
    for &f in faces {
        normale = ajouter(normale, o.normale(f));
    }
    let mut flancs = Vec::new();
    for &(a, b) in &orientees {
        if !orientees.contains(&(b, a)) {
            flancs.push(vec![a, b, double[&b], double[&a]]);
        }
    }
    for &f in faces {
        for i in o.faces[f].iter_mut() {
            *i = double[i];
        }
    }
    o.faces.extend(flancs);
    let axe = if norme(normale) > 1e-6 { Some(unitaire(normale)) } else { None };
    let mut s = Suite::faces(faces.clone());
    s.transfo = Some((Transfo::Deplacer, axe));
    s
}

/// Coupe chaque face choisie autour de son centre : un sommet au milieu de chaque
/// arête, un au centre, et autant de quadrilatères que de côtés. Les faces voisines
/// reçoivent les milieux de leurs arêtes coupées (pas de trou).
pub fn subdiviser(o: &mut Objet, faces: &BTreeSet<usize>) -> Suite {
    let mut milieux: HashMap<Arete, u32> = HashMap::new();
    for &f in faces {
        let p = o.faces[f].clone();
        for i in 0..p.len() {
            let a = arete(p[i], p[(i + 1) % p.len()]);
            milieux.entry(a).or_insert_with(|| {
                o.sommets.push(fois(ajouter(o.sommets[a.0 as usize], o.sommets[a.1 as usize]), 0.5));
                o.sommets.len() as u32 - 1
            });
        }
    }
    let mut nouvelles = Vec::new();
    let mut choisies = BTreeSet::new();
    let anciennes = std::mem::take(&mut o.faces);
    for (f, p) in anciennes.into_iter().enumerate() {
        let n = p.len();
        if faces.contains(&f) {
            o.sommets.push(centre(p.iter().map(|&i| o.sommets[i as usize])));
            let c = o.sommets.len() as u32 - 1;
            for i in 0..n {
                let (prec, ici, suiv) = (p[(i + n - 1) % n], p[i], p[(i + 1) % n]);
                choisies.insert(nouvelles.len());
                nouvelles.push(vec![ici, milieux[&arete(ici, suiv)], c, milieux[&arete(prec, ici)]]);
            }
        } else {
            // Une voisine : les milieux s'insèrent dans ses arêtes coupées
            let mut q = Vec::with_capacity(n + 2);
            for i in 0..n {
                q.push(p[i]);
                if let Some(&m) = milieux.get(&arete(p[i], p[(i + 1) % n])) {
                    q.push(m);
                }
            }
            nouvelles.push(q);
        }
    }
    o.faces = nouvelles;
    Suite::faces(choisies)
}

/// Réunit les sommets choisis en un seul, à leur centre ; les faces écrasées
/// disparaissent.
pub fn fusionner(o: &mut Objet, sommets: &BTreeSet<u32>) -> Suite {
    let Some(&premier) = sommets.iter().next() else { return Suite::sommets(BTreeSet::new()) };
    o.sommets[premier as usize] = centre(sommets.iter().map(|&i| o.sommets[i as usize]));
    for p in &mut o.faces {
        for i in p.iter_mut() {
            if sommets.contains(i) {
                *i = premier;
            }
        }
        // Les répétitions qui se suivent n'en font qu'une
        p.dedup();
        while p.len() > 1 && p.first() == p.last() {
            p.pop();
        }
    }
    o.faces.retain(|p| p.len() >= 3 && p.iter().collect::<BTreeSet<_>>().len() == p.len());
    let nouveau = o.compacter(&[premier].into_iter().collect());
    Suite::sommets([nouveau[&premier]].into_iter().collect())
}

/// Une face sur les sommets choisis, rangés en tour autour de leur centre, tournée
/// vers l'œil `oeil` (si on la voit de dos, elle est retournée).
pub fn creer_face(o: &mut Objet, sommets: &BTreeSet<u32>, oeil: V3) -> Suite {
    let pts: Vec<(u32, V3)> = sommets.iter().map(|&i| (i, o.sommets[i as usize])).collect();
    let c = centre(pts.iter().map(|p| p.1));
    // Le plan des points : la normale de Newell du tour grossier, sinon le regard
    let mut n = [0.0f32; 3];
    for i in 0..pts.len() {
        n = ajouter(n, vectoriel(oter(pts[i].1, c), oter(pts[(i + 1) % pts.len()].1, c)));
    }
    if norme(n) < 1e-9 {
        n = oter(oeil, c);
    }
    let n = unitaire(n);
    let u = unitaire(if norme(vectoriel(n, [0.0, 1.0, 0.0])) > 0.1 { vectoriel(n, [0.0, 1.0, 0.0]) } else { vectoriel(n, [1.0, 0.0, 0.0]) });
    let v = vectoriel(n, u);
    let mut tri: Vec<(f32, u32)> = pts.iter().map(|(i, p)| (scalaire(oter(*p, c), v).atan2(scalaire(oter(*p, c), u)), *i)).collect();
    tri.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut face: Vec<u32> = tri.into_iter().map(|t| t.1).collect();
    o.faces.push(face.clone());
    let f = o.faces.len() - 1;
    if scalaire(o.normale(f), oter(oeil, c)) < 0.0 {
        face.reverse();
        o.faces[f] = face;
    }
    Suite::faces([f].into_iter().collect())
}

/// Coupe les faces choisies en triangles (en éventail).
pub fn trianguler(o: &mut Objet, faces: &BTreeSet<usize>) -> Suite {
    let mut nouvelles = Vec::new();
    let mut choisies = BTreeSet::new();
    for (f, p) in std::mem::take(&mut o.faces).into_iter().enumerate() {
        if faces.contains(&f) && p.len() > 3 {
            for i in 1..p.len() - 1 {
                choisies.insert(nouvelles.len());
                nouvelles.push(vec![p[0], p[i], p[i + 1]]);
            }
        } else {
            if faces.contains(&f) {
                choisies.insert(nouvelles.len());
            }
            nouvelles.push(p);
        }
    }
    o.faces = nouvelles;
    Suite::faces(choisies)
}

/// Inverse le sens des faces choisies.
pub fn retourner(o: &mut Objet, faces: &BTreeSet<usize>) -> Suite {
    for &f in faces {
        o.faces[f].reverse();
    }
    Suite::faces(faces.clone())
}

/// Copie les faces choisies (avec leurs sommets) ; la suite : les déplacer.
pub fn dupliquer(o: &mut Objet, faces: &BTreeSet<usize>) -> Suite {
    let mut copie: HashMap<u32, u32> = HashMap::new();
    let mut choisies = BTreeSet::new();
    for &f in faces {
        let p: Vec<u32> = o.faces[f]
            .clone()
            .into_iter()
            .map(|i| {
                *copie.entry(i).or_insert_with(|| {
                    o.sommets.push(o.sommets[i as usize]);
                    o.sommets.len() as u32 - 1
                })
            })
            .collect();
        o.faces.push(p);
        choisies.insert(o.faces.len() - 1);
    }
    let mut s = Suite::faces(choisies);
    s.transfo = Some((Transfo::Deplacer, None));
    s
}

/// Le centre des sommets choisis.
pub fn centre_de(o: &Objet, sommets: &BTreeSet<u32>) -> V3 {
    centre(sommets.iter().map(|&i| o.sommets[i as usize]))
}

/// Tourne `p` autour de l'axe unitaire `axe` passant par `c`, de `angle` radians.
pub fn tourner(p: V3, c: V3, axe: V3, angle: f32) -> V3 {
    let v = oter(p, c);
    let (s, co) = angle.sin_cos();
    // Rodrigues
    let r = ajouter(ajouter(fois(v, co), fois(vectoriel(axe, v), s)), fois(axe, scalaire(axe, v) * (1.0 - co)));
    ajouter(c, r)
}

/// Le triangle (o, d) le plus proche touché : sa distance et sa face.
pub fn toucher(o: &Objet, orig: V3, d: V3) -> Option<(f32, usize)> {
    let mut mieux: Option<(f32, usize)> = None;
    for (t, f) in o.triangles() {
        let [a, b, c] = t.map(|i| o.sommets[i as usize]);
        let (e1, e2) = (oter(b, a), oter(c, a));
        let p = vectoriel(d, e2);
        let det = scalaire(e1, p);
        if det.abs() < 1e-12 {
            continue;
        }
        let inv = 1.0 / det;
        let s = oter(orig, a);
        let u = scalaire(s, p) * inv;
        if !(0.0..=1.0).contains(&u) {
            continue;
        }
        let q = vectoriel(s, e1);
        let v = scalaire(d, q) * inv;
        if v < 0.0 || u + v > 1.0 {
            continue;
        }
        let dist = scalaire(e2, q) * inv;
        if dist > 1e-6 && mieux.is_none_or(|m| dist < m.0) {
            mieux = Some((dist, f));
        }
    }
    mieux
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tout(o: &Objet) -> BTreeSet<usize> {
        (0..o.faces.len()).collect()
    }

    /// Chaque arête bordée par exactement deux faces, dans des sens opposés : la forme
    /// est fermée et ses faces orientées de façon cohérente.
    fn ferme(o: &Objet) -> bool {
        let mut orientees: BTreeMap<(u32, u32), usize> = BTreeMap::new();
        for p in &o.faces {
            for i in 0..p.len() {
                *orientees.entry((p[i], p[(i + 1) % p.len()])).or_default() += 1;
            }
        }
        orientees.iter().all(|(&(a, b), &n)| n == 1 && orientees.get(&(b, a)) == Some(&1))
    }

    fn dehors(o: &Objet) -> bool {
        let c = centre(o.sommets.iter().copied());
        (0..o.faces.len()).all(|f| scalaire(o.normale(f), oter(o.centre_face(f), c)) > 0.0)
    }

    #[test]
    fn le_cube_est_ferme_et_tourne_vers_dehors() {
        let c = cube("c", [0.0; 3], 1.0);
        assert_eq!((c.sommets.len(), c.faces.len(), c.aretes().len()), (8, 6, 12));
        assert!(ferme(&c) && dehors(&c));
    }

    #[test]
    fn les_triangles_se_reunissent_en_quadrilateres() {
        let c = cube("c", [0.0; 3], 1.0);
        let tris: Vec<[u32; 3]> = c.triangles().into_iter().map(|t| t.0).collect();
        let o = Objet::depuis_triangles("c", IDENTITE, c.sommets.clone(), &tris, [1.0; 3]);
        assert_eq!(o.faces.len(), 6);
        assert!(o.faces.iter().all(|p| p.len() == 4) && ferme(&o) && dehors(&o));
    }

    #[test]
    fn extruder_une_face_garde_la_forme_fermee() {
        let mut c = cube("c", [0.0; 3], 1.0);
        let s = extruder(&mut c, &[1].into_iter().collect());
        assert_eq!((c.sommets.len(), c.faces.len()), (12, 10));
        assert!(ferme(&c));
        let axe = s.transfo.unwrap().1.unwrap();
        assert!((scalaire(axe, [0.0, 0.0, 1.0]) - 1.0).abs() < 1e-5, "{axe:?}");
        // Poussée dehors, la forme reste orientée vers l'extérieur
        for i in 8..12 {
            c.sommets[i][2] += 1.0;
        }
        assert!(dehors(&c));
    }

    #[test]
    fn subdiviser_coupe_sans_trou() {
        let mut c = cube("c", [0.0; 3], 1.0);
        let s = subdiviser(&mut c, &[0].into_iter().collect());
        assert_eq!(s.selection.faces.len(), 4);
        // 4 milieux + 1 centre ; 4 nouvelles faces + 5 anciennes
        assert_eq!((c.sommets.len(), c.faces.len()), (13, 9));
        assert!(ferme(&c) && dehors(&c));
        let mut c = cube("c", [0.0; 3], 1.0);
        let t = tout(&c);
        subdiviser(&mut c, &t);
        assert_eq!(c.faces.len(), 24);
        assert!(ferme(&c) && dehors(&c));
    }

    #[test]
    fn supprimer_fusionner_et_creer_une_face() {
        let mut c = cube("c", [0.0; 3], 1.0);
        supprimer(&mut c, &Selection { faces: [1].into_iter().collect(), ..Default::default() }, Mode::Faces);
        assert_eq!((c.sommets.len(), c.faces.len()), (8, 5));
        // Le trou se rebouche par une face sur ses quatre sommets
        let trou: BTreeSet<u32> = [4, 5, 6, 7].into_iter().collect();
        creer_face(&mut c, &trou, [0.0, 0.0, 10.0]);
        assert!(ferme(&c) && dehors(&c));
        // Un sommet supprimé emporte ses trois faces
        let mut c = cube("c", [0.0; 3], 1.0);
        supprimer(&mut c, &Selection { sommets: [0].into_iter().collect(), ..Default::default() }, Mode::Sommets);
        assert_eq!((c.sommets.len(), c.faces.len()), (7, 3));
        // Fusionner une arête : deux faces deviennent des triangles
        let mut c = cube("c", [0.0; 3], 1.0);
        let s = fusionner(&mut c, &[0, 1].into_iter().collect());
        assert_eq!((c.sommets.len(), c.faces.len(), s.selection.sommets.len()), (7, 6, 1));
        assert_eq!(c.faces.iter().filter(|p| p.len() == 3).count(), 2);
        assert!(ferme(&c));
    }

    #[test]
    fn trianguler_retourner_dupliquer() {
        let mut c = cube("c", [0.0; 3], 1.0);
        let t = tout(&c);
        trianguler(&mut c, &t);
        assert_eq!(c.faces.len(), 12);
        assert!(ferme(&c) && dehors(&c));
        let mut c = cube("c", [0.0; 3], 1.0);
        let n = c.normale(2);
        retourner(&mut c, &[2].into_iter().collect());
        assert!((scalaire(n, c.normale(2)) + 1.0).abs() < 1e-5);
        let mut c = cube("c", [0.0; 3], 1.0);
        let s = dupliquer(&mut c, &[0, 1].into_iter().collect());
        assert_eq!((c.sommets.len(), c.faces.len(), s.selection.faces.len()), (16, 8, 2));
    }

    #[test]
    fn la_selection_change_de_mode_et_les_operations_s_y_adaptent() {
        let c = cube("c", [0.0; 3], 1.0);
        let s = Selection { sommets: [0, 1, 2, 3].into_iter().collect(), ..Default::default() };
        assert_eq!(s.convertir(&c, Mode::Sommets, Mode::Faces).faces, [0].into_iter().collect());
        assert_eq!(s.convertir(&c, Mode::Sommets, Mode::Aretes).aretes.len(), 4);
        let ctx = Contexte::de(std::slice::from_ref(&c), 0, &s, Mode::Sommets);
        let possibles: Vec<&str> = OPERATIONS.iter().filter(|o| (o.possible)(&ctx)).map(|o| o.cle).collect();
        assert!(possibles.contains(&"extruder") && possibles.contains(&"fusionner") && !possibles.contains(&"face"));
        let vide = Contexte::de(std::slice::from_ref(&c), 0, &Selection::default(), Mode::Faces);
        let possibles: Vec<&str> = OPERATIONS.iter().filter(|o| (o.possible)(&vide)).map(|o| o.cle).collect();
        assert_eq!(possibles, ["tout", "cube"]);
    }

    #[test]
    fn toucher_et_tourner() {
        let c = cube("c", [0.0; 3], 1.0);
        let (d, f) = toucher(&c, [0.0, 0.0, 5.0], [0.0, 0.0, -1.0]).unwrap();
        assert!((d - 4.0).abs() < 1e-5 && f == 1);
        let p = tourner([1.0, 0.0, 0.0], [0.0; 3], [0.0, 0.0, 1.0], std::f32::consts::FRAC_PI_2);
        assert!(p[0].abs() < 1e-6 && (p[1] - 1.0).abs() < 1e-6);
    }
}
