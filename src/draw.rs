//! Liste de dessin 2D : tout est un quadrilatère, dont le nuanceur calcule la forme
//! (rectangle arrondi par champ de distance) ou lit la couverture (texte).
//! Coordonnées en pixels, origine en haut à gauche.

#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }
    pub fn right(&self) -> f32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }
    pub fn cx(&self) -> f32 {
        self.x + self.w / 2.0
    }
    pub fn cy(&self) -> f32 {
        self.y + self.h / 2.0
    }
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }
    pub fn inset(&self, d: f32) -> Rect {
        Rect::new(self.x + d, self.y + d, (self.w - 2.0 * d).max(0.0), (self.h - 2.0 * d).max(0.0))
    }
    pub fn intersect(&self, o: &Rect) -> Rect {
        let (x, y) = (self.x.max(o.x), self.y.max(o.y));
        Rect::new(x, y, (self.right().min(o.right()) - x).max(0.0), (self.bottom().min(o.bottom()) - y).max(0.0))
    }
}

/// Couleur RVBA, alpha non prémultiplié.
pub type Color = [u8; 4];

#[derive(Clone, Copy, Default)]
#[repr(C)]
pub struct Vertex {
    pub pos: [f32; 2],
    /// Position relative au centre de la forme.
    pub local: [f32; 2],
    /// Demi-largeur, demi-hauteur, rayon (négatif : texte), flou.
    pub shape: [f32; 4],
    pub uv: [f32; 2],
    pub color: Color,
}

/// Tranche de sommets dessinée avec une même zone de découpe, ou une vue 3D
/// (`vue3d`, un indice dans `DrawList::vues3d`) dessinée à cette place.
#[derive(Clone, Copy)]
pub struct Cmd {
    pub first: usize,
    pub count: usize,
    pub clip: Option<Rect>,
    pub vue3d: Option<usize>,
    /// Une image (un indice dans `DrawList::images`) : ses six sommets la portent.
    pub image: Option<usize>,
    /// Un terrain (un indice dans `DrawList::terrains`), dessiné à cette place.
    pub terrain: Option<usize>,
}

/// Une image RVBA (alpha prémultiplié), `largeur` × `hauteur`. Le moteur de rendu la
/// garde sur la carte graphique tant que (`id`, `revision`) ne change pas.
#[derive(Clone, Debug)]
pub struct Image {
    pub id: u64,
    pub revision: u64,
    pub largeur: u32,
    pub hauteur: u32,
    pub pixels: std::sync::Arc<Vec<u8>>,
}

/// Un maillage à dessiner en 3D : par sommet, position puis normale (6 flottants),
/// et des triangles. Le moteur de rendu le garde sur la carte graphique tant que
/// (`id`, `revision`) ne change pas.
#[derive(Clone, Debug)]
pub struct Maillage3d {
    pub id: u64,
    pub revision: u64,
    pub sommets: std::sync::Arc<Vec<f32>>,
    pub indices: std::sync::Arc<Vec<u32>>,
}

/// Un morceau de terrain : un maillage aux sommets de 8 flottants (position, normale,
/// coordonnées de texture de 0 à 1) et son image, dont il prend la partie
/// `uv` (u0, v0, largeur, hauteur) ; sans image, la couleur `couleur`.
#[derive(Clone, Debug)]
pub struct Morceau3d {
    pub maillage: Maillage3d,
    pub image: Option<Image>,
    pub uv: [f32; 4],
    pub couleur: [f32; 3],
}

/// Un terrain vu par `mvp` dans un rectangle de l'écran : ses morceaux texturés (les
/// pixels transparents de l'image ne se dessinent pas), noyés dans la brume (couleur
/// `ciel`) de `brume[0]` à `brume[1]` mètres de l'œil.
#[derive(Clone, Debug)]
pub struct Terrain3d {
    pub rect: Rect,
    pub mvp: [f32; 16],
    pub oeil: [f32; 3],
    pub ciel: [f32; 3],
    pub brume: [f32; 2],
    pub morceaux: Vec<Morceau3d>,
}

/// Une vue 3D : un maillage vu par `mvp` dans un rectangle de l'écran. `vue` (la
/// rotation de la caméra, en colonnes) éclaire depuis le regard.
#[derive(Clone, Debug)]
pub struct Vue3d {
    pub rect: Rect,
    pub mvp: [f32; 16],
    pub vue: [f32; 16],
    pub maillage: Maillage3d,
    /// La couleur de la matière, de 0 à 1.
    pub couleur: [f32; 3],
    /// Un second maillage dessiné par-dessus, dans la même profondeur (des faces
    /// choisies, surlignées), et sa couleur.
    pub dessus: Option<(Maillage3d, [f32; 3])>,
}

#[derive(Default)]
pub struct DrawList {
    pub vertices: Vec<Vertex>,
    pub cmds: Vec<Cmd>,
    pub vues3d: Vec<Vue3d>,
    pub terrains: Vec<Terrain3d>,
    pub images: Vec<Image>,
    clip: Option<Rect>,
}

impl DrawList {
    pub fn clear(&mut self) {
        self.vertices.clear();
        self.cmds.clear();
        self.vues3d.clear();
        self.terrains.clear();
        self.images.clear();
        self.clip = None;
    }

    pub fn set_clip(&mut self, clip: Option<Rect>) {
        self.clip = clip;
    }

    fn push(&mut self, q: [Vertex; 4]) {
        let clip = self.clip;
        match self.cmds.last_mut() {
            Some(c) if c.clip == clip && c.vue3d.is_none() && c.image.is_none() && c.terrain.is_none() => c.count += 6,
            _ => self.cmds.push(Cmd { first: self.vertices.len(), count: 6, clip, vue3d: None, image: None, terrain: None }),
        }
        self.vertices.extend_from_slice(&[q[0], q[1], q[2], q[2], q[1], q[3]]);
    }

    /// Quadrilatère aligné, agrandi de `e` pour laisser place à l'antialiasing ou au flou.
    fn shaped(&mut self, r: Rect, e: f32, radius: f32, soft: f32, color: Color) {
        let (hw, hh) = (r.w / 2.0, r.h / 2.0);
        let shape = [hw, hh, radius.min(hw).min(hh).max(0.0), soft];
        let (cx, cy) = (r.cx(), r.cy());
        let corner = |sx: f32, sy: f32| Vertex {
            pos: [cx + sx * (hw + e), cy + sy * (hh + e)],
            local: [sx * (hw + e), sy * (hh + e)],
            shape,
            uv: [0.0, 0.0],
            color,
        };
        self.push([corner(-1., -1.), corner(1., -1.), corner(-1., 1.), corner(1., 1.)]);
    }

    /// Une vue 3D, dessinée à cette place : ce qui suit passe par-dessus. Elle reste
    /// dans la zone de découpe courante.
    pub fn vue3d(&mut self, v: Vue3d) {
        let clip = self.clip;
        self.vues3d.push(v);
        self.cmds.push(Cmd { first: self.vertices.len(), count: 0, clip, vue3d: Some(self.vues3d.len() - 1), image: None, terrain: None });
    }

    /// Un terrain, dessiné à cette place, dans la zone de découpe courante.
    pub fn terrain(&mut self, t: Terrain3d) {
        let clip = self.clip;
        self.terrains.push(t);
        self.cmds.push(Cmd { first: self.vertices.len(), count: 0, clip, vue3d: None, image: None, terrain: Some(self.terrains.len() - 1) });
    }

    /// Une image étirée dans `r`, dans la zone de découpe courante.
    pub fn image(&mut self, r: Rect, img: Image) {
        self.image_partie(r, img, [0.0, 0.0], [1.0, 1.0]);
    }

    /// La partie `uv0`–`uv1` (de 0 à 1) d'une image, étirée dans `r`.
    pub fn image_partie(&mut self, r: Rect, img: Image, uv0: [f32; 2], uv1: [f32; 2]) {
        let clip = self.clip;
        self.images.push(img);
        self.cmds.push(Cmd { first: self.vertices.len(), count: 6, clip, vue3d: None, image: Some(self.images.len() - 1), terrain: None });
        let v = |x: f32, y: f32, u: f32, w: f32| Vertex { pos: [x, y], local: [0.0, 0.0], shape: [0.0, 0.0, -3.0, 0.0], uv: [u, w], color: [255, 255, 255, 255] };
        let q = [v(r.x, r.y, uv0[0], uv0[1]), v(r.right(), r.y, uv1[0], uv0[1]), v(r.x, r.bottom(), uv0[0], uv1[1]), v(r.right(), r.bottom(), uv1[0], uv1[1])];
        self.vertices.extend_from_slice(&[q[0], q[1], q[2], q[2], q[1], q[3]]);
    }

    pub fn rrect(&mut self, r: Rect, radius: f32, color: Color) {
        self.shaped(r, 1.0, radius, 0.0, color);
    }

    /// Ombre douce : à dessiner juste avant la forme.
    pub fn shadow(&mut self, r: Rect, radius: f32, soft: f32, color: Color) {
        self.shaped(r, soft * 2.0, radius, soft, color);
    }

    /// Trait à bouts ronds de `a` à `b`.
    pub fn line(&mut self, a: [f32; 2], b: [f32; 2], thick: f32, color: Color) {
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let len = (dx * dx + dy * dy).sqrt().max(1e-3);
        let (ux, uy) = (dx / len, dy / len);
        let (hx, hy) = (len / 2.0 + thick / 2.0, thick / 2.0);
        let (cx, cy) = ((a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0);
        let (ex, ey) = (hx + 1.0, hy + 1.0);
        let shape = [hx, hy, thick / 2.0, 0.0];
        let corner = |sx: f32, sy: f32| {
            let (lx, ly) = (sx * ex, sy * ey);
            Vertex { pos: [cx + lx * ux - ly * uy, cy + lx * uy + ly * ux], local: [lx, ly], shape, uv: [0.0, 0.0], color }
        };
        self.push([corner(-1., -1.), corner(1., -1.), corner(-1., 1.), corner(1., 1.)]);
    }

    /// Secteur d'anneau plein, de centre `c`, entre les rayons `r0` et `r1`, de l'angle
    /// `a0` à `a1` (radians, sens horaire à l'écran, 0 vers la droite). `ecart` laisse un
    /// liseré de cette largeur (en pixels) sur ses bords droits, pour séparer les
    /// secteurs voisins. La forme est calculée dans le nuanceur, antialiasée.
    #[allow(clippy::too_many_arguments)]
    pub fn secteur(&mut self, c: [f32; 2], r0: f32, r1: f32, a0: f32, a1: f32, ecart: f32, color: Color) {
        if r1 <= r0 || a1 <= a0 {
            return;
        }
        // Le rectangle qui l'enveloppe : ses quatre coins et les axes qu'il traverse
        let mut b = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
        let mut prendre = |a: f32, r: f32| {
            let (x, y) = (a.cos() * r, a.sin() * r);
            b = [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)];
        };
        for a in [a0, a1] {
            prendre(a, r0);
            prendre(a, r1);
        }
        let quart = std::f32::consts::FRAC_PI_2;
        let mut k = (a0 / quart).ceil();
        while k * quart < a1 {
            prendre(k * quart, r1);
            k += 1.0;
        }
        let e = 1.5;
        let shape = [r0, r1, -2.0, ecart];
        let v = |x: f32, y: f32| Vertex { pos: [c[0] + x, c[1] + y], local: [x, y], shape, uv: [a0, a1], color };
        self.push([v(b[0] - e, b[1] - e), v(b[2] + e, b[1] - e), v(b[0] - e, b[3] + e), v(b[2] + e, b[3] + e)]);
    }

    /// Bande horizontale d'un pixel de haut, de `x0` à `x1` sur la ligne `y` : lissée
    /// à ses bouts, nette en haut et en bas pour que les bandes voisines se joignent.
    /// Le remplissage des formes SVG en est fait.
    pub fn bande(&mut self, y: f32, x0: f32, x1: f32, color: Color) {
        let hw = ((x1 - x0) / 2.0).max(0.0);
        let cx = (x0 + x1) / 2.0;
        let e = 1.0;
        let shape = [hw, 1.0e5, 0.0, 0.0];
        let v = |sx: f32, yy: f32| Vertex { pos: [cx + sx * (hw + e), yy], local: [sx * (hw + e), 0.0], shape, uv: [0.0, 0.0], color };
        self.push([v(-1.0, y), v(1.0, y), v(-1.0, y + 1.0), v(1.0, y + 1.0)]);
    }

    pub fn glyph(&mut self, r: Rect, uv0: [f32; 2], uv1: [f32; 2], color: Color) {
        let v = |x: f32, y: f32, u: f32, w: f32| Vertex { pos: [x, y], local: [0.0, 0.0], shape: [0.0, 0.0, -1.0, 0.0], uv: [u, w], color };
        self.push([
            v(r.x, r.y, uv0[0], uv0[1]),
            v(r.right(), r.y, uv1[0], uv0[1]),
            v(r.x, r.bottom(), uv0[0], uv1[1]),
            v(r.right(), r.bottom(), uv1[0], uv1[1]),
        ]);
    }
}
