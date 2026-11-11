//! Tout OpenGL est ici : l'interface, sur tout l'écran, et les vues 3D qu'elle
//! contient, dessinées à leur place dans la liste (avec leur propre profondeur).

use crate::draw::{Color, DrawList, Rect, Vertex, Vue3d};
use std::cell::RefCell;
use crate::gles::GlApi;
use crate::text::{Text, ATLAS};
use std::ffi::{c_char, c_void, CString};
use std::mem::{offset_of, size_of};

const TRIANGLES: u32 = 4;
const FLOAT: u32 = 0x1406;
const UBYTE: u32 = 0x1401;
const ARRAY_BUFFER: u32 = 0x8892;
const TEXTURE_2D: u32 = 0x0DE1;
const RED: u32 = 0x1903;
const BLEND: u32 = 0x0BE2;
const SCISSOR: u32 = 0x0C11;
const COLOR_BIT: u32 = 0x4000;
const DEPTH_BIT: u32 = 0x0100;
const DEPTH_TEST: u32 = 0x0B71;
const ELEMENT_ARRAY_BUFFER: u32 = 0x8893;
const UINT: u32 = 0x1405;

const MAILLAGE_VS: &str = "#version 300 es
precision highp float;
layout(location=0) in vec3 a_pos;
layout(location=1) in vec3 a_normal;
uniform mat4 u_mvp;
uniform mat4 u_vue;
out vec3 v_n;
void main() {
  v_n = mat3(u_vue) * a_normal;
  gl_Position = u_mvp * vec4(a_pos, 1.0);
}";

/// Une terre mate : une lumière principale venue d'en haut à gauche du regard, un
/// peu de contre-jour, un reflet discret et un liseré sur les bords.
const MAILLAGE_FS: &str = "#version 300 es
precision highp float;
in vec3 v_n;
uniform vec3 u_couleur;
out vec4 o_color;
void main() {
  vec3 n = normalize(v_n);
  if (!gl_FrontFacing) n = -n;
  vec3 l = normalize(vec3(-0.45, 0.6, 0.66));
  float d = max(dot(n, l), 0.0);
  float fill = max(dot(n, normalize(vec3(0.7, -0.3, 0.4))), 0.0) * 0.22;
  float rim = pow(1.0 - max(n.z, 0.0), 3.0) * 0.22;
  float spec = pow(max(dot(n, normalize(l + vec3(0.0, 0.0, 1.0))), 0.0), 36.0) * 0.16;
  vec3 c = u_couleur * (0.26 + 0.74 * d + fill) + vec3(rim + spec);
  o_color = vec4(pow(c, vec3(1.0 / 1.1)), 1.0);
}";

/// Un maillage tenu sur la carte graphique.
struct Tampon3d {
    id: u64,
    revision: u64,
    vao: u32,
    vbo: u32,
    ebo: u32,
    n: i32,
}

const UI_VS: &str = "#version 300 es
precision highp float;
layout(location=0) in vec2 a_pos;
layout(location=1) in vec2 a_local;
layout(location=2) in vec4 a_shape;
layout(location=3) in vec2 a_uv;
layout(location=4) in vec4 a_color;
uniform vec2 u_size;
out vec2 v_local; out vec4 v_shape; out vec2 v_uv; out vec4 v_color;
void main() {
  v_local = a_local; v_shape = a_shape; v_uv = a_uv; v_color = a_color;
  vec2 p = a_pos / u_size * 2.0 - 1.0;
  gl_Position = vec4(p.x, -p.y, 0.0, 1.0);
}";

// highp : en mediump (16 bits sur Mali), les distances en pixels manquent de précision.
const UI_FS: &str = "#version 300 es
precision highp float;
in vec2 v_local; in vec4 v_shape; in vec2 v_uv; in vec4 v_color;
uniform sampler2D u_atlas;
out vec4 o_color;
void main() {
  float a;
  if (v_shape.z < -1.5) {
    // Secteur d'anneau : rayons (x, y), angles (uv), écart sur les bords droits (w)
    float r = length(v_local);
    float dr = max(v_shape.x - r, r - v_shape.y);
    float demi = 0.5 * (v_uv.y - v_uv.x);
    float milieu = v_uv.x + demi;
    float ang = atan(v_local.y, v_local.x);
    float da = abs(mod(ang - milieu + 3.14159265, 6.2831853) - 3.14159265) - demi;
    float d = demi >= 3.1415 ? dr : max(dr, da * max(r, 1.0) + 0.5 * v_shape.w);
    a = 1.0 - smoothstep(-0.6, 0.6, d);
  } else if (v_shape.z < 0.0) {
    a = texture(u_atlas, v_uv).r;
  } else {
    vec2 q = abs(v_local) - v_shape.xy + v_shape.z;
    float d = length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - v_shape.z;
    float s = max(v_shape.w, 0.6);
    a = 1.0 - smoothstep(-s, s, d);
  }
  a *= v_color.a;
  o_color = vec4(v_color.rgb * a, a);
}";

pub struct Renderer {
    gl: GlApi,
    ui_program: u32,
    u_size: i32,
    ui_vao: u32,
    ui_vbo: u32,
    atlas: u32,
    maillage_program: u32,
    u_mvp: i32,
    u_vue: i32,
    u_couleur: i32,
    tampons: RefCell<Vec<Tampon3d>>,
}

fn cstr(s: &str) -> CString {
    CString::new(s).unwrap()
}

unsafe fn compile(gl: &GlApi, kind: u32, src: &str) -> Result<u32, String> {
    let s = (gl.glCreateShader)(kind);
    let c = cstr(src);
    let p = c.as_ptr();
    (gl.glShaderSource)(s, 1, &p, std::ptr::null());
    (gl.glCompileShader)(s);
    let mut ok = 0;
    (gl.glGetShaderiv)(s, 0x8B81, &mut ok);
    if ok == 0 {
        let mut log = [0u8; 1024];
        (gl.glGetShaderInfoLog)(s, 1024, std::ptr::null_mut(), log.as_mut_ptr() as *mut c_char);
        return Err(String::from_utf8_lossy(&log).trim_end_matches('\0').to_string());
    }
    Ok(s)
}

unsafe fn program(gl: &GlApi, vs: &str, fs: &str) -> Result<u32, String> {
    let p = (gl.glCreateProgram)();
    (gl.glAttachShader)(p, compile(gl, 0x8B31, vs)?);
    (gl.glAttachShader)(p, compile(gl, 0x8B30, fs)?);
    (gl.glLinkProgram)(p);
    let mut ok = 0;
    (gl.glGetProgramiv)(p, 0x8B82, &mut ok);
    if ok == 0 {
        let mut log = [0u8; 1024];
        (gl.glGetProgramInfoLog)(p, 1024, std::ptr::null_mut(), log.as_mut_ptr() as *mut c_char);
        return Err(String::from_utf8_lossy(&log).trim_end_matches('\0').to_string());
    }
    Ok(p)
}

impl Renderer {
    /// À créer avec un contexte courant.
    pub fn new(gl: GlApi) -> Result<Renderer, String> {
        unsafe {
            let ui_program = program(&gl, UI_VS, UI_FS)?;
            let loc = |p: u32, n: &str| (gl.glGetUniformLocation)(p, cstr(n).as_ptr());
            let u_size = loc(ui_program, "u_size");

            let (mut ui_vao, mut ui_vbo) = (0, 0);
            (gl.glGenVertexArrays)(1, &mut ui_vao);
            (gl.glGenBuffers)(1, &mut ui_vbo);
            (gl.glBindVertexArray)(ui_vao);
            (gl.glBindBuffer)(ARRAY_BUFFER, ui_vbo);
            let stride = size_of::<Vertex>() as i32;
            let attrs: [(u32, i32, u32, u8, usize); 5] = [
                (0, 2, FLOAT, 0, offset_of!(Vertex, pos)),
                (1, 2, FLOAT, 0, offset_of!(Vertex, local)),
                (2, 4, FLOAT, 0, offset_of!(Vertex, shape)),
                (3, 2, FLOAT, 0, offset_of!(Vertex, uv)),
                (4, 4, UBYTE, 1, offset_of!(Vertex, color)),
            ];
            for (i, n, t, norm, off) in attrs {
                (gl.glEnableVertexAttribArray)(i);
                (gl.glVertexAttribPointer)(i, n, t, norm, stride, off as *const c_void);
            }

            let mut atlas = 0;
            (gl.glGenTextures)(1, &mut atlas);
            (gl.glBindTexture)(TEXTURE_2D, atlas);
            (gl.glPixelStorei)(0x0CF5, 1);
            (gl.glTexImage2D)(TEXTURE_2D, 0, 0x8229, ATLAS as i32, ATLAS as i32, 0, RED, UBYTE, std::ptr::null());
            for (k, v) in [(0x2801, 0x2601), (0x2800, 0x2601), (0x2802, 0x812F), (0x2803, 0x812F)] {
                (gl.glTexParameteri)(TEXTURE_2D, k, v);
            }

            (gl.glBindVertexArray)(0);

            let maillage_program = program(&gl, MAILLAGE_VS, MAILLAGE_FS)?;
            let u_mvp = loc(maillage_program, "u_mvp");
            let u_vue = loc(maillage_program, "u_vue");
            let u_couleur = loc(maillage_program, "u_couleur");

            Ok(Renderer { gl, ui_program, u_size, ui_vao, ui_vbo, atlas, maillage_program, u_mvp, u_vue, u_couleur, tampons: RefCell::new(Vec::new()) })
        }
    }

    pub fn gl(&self) -> &GlApi {
        &self.gl
    }

    /// Rectangle en pixels (origine en haut) vers OpenGL (origine en bas).
    fn gl_rect(r: Rect, h: f32) -> (i32, i32, i32, i32) {
        (r.x.round() as i32, (h - r.bottom()).round() as i32, r.w.round().max(0.0) as i32, r.h.round().max(0.0) as i32)
    }

    /// Le maillage d'une vue sur la carte graphique, envoyé s'il est nouveau ou a
    /// changé : (vao, nombre d'indices).
    unsafe fn tampon(&self, v: &Vue3d) -> (u32, i32) {
        let gl = &self.gl;
        let m = &v.maillage;
        let mut tampons = self.tampons.borrow_mut();
        let i = match tampons.iter().position(|t| t.id == m.id) {
            Some(i) => i,
            None => {
                let (mut vao, mut vbo, mut ebo) = (0, 0, 0);
                (gl.glGenVertexArrays)(1, &mut vao);
                (gl.glGenBuffers)(1, &mut vbo);
                (gl.glGenBuffers)(1, &mut ebo);
                (gl.glBindVertexArray)(vao);
                (gl.glBindBuffer)(ARRAY_BUFFER, vbo);
                (gl.glBindBuffer)(ELEMENT_ARRAY_BUFFER, ebo);
                for (k, off) in [(0u32, 0usize), (1, 12)] {
                    (gl.glEnableVertexAttribArray)(k);
                    (gl.glVertexAttribPointer)(k, 3, FLOAT, 0, 24, off as *const c_void);
                }
                (gl.glBindVertexArray)(0);
                tampons.push(Tampon3d { id: m.id, revision: u64::MAX, vao, vbo, ebo, n: 0 });
                tampons.len() - 1
            }
        };
        let t = &mut tampons[i];
        if t.revision != m.revision {
            (gl.glBindVertexArray)(t.vao);
            (gl.glBindBuffer)(ARRAY_BUFFER, t.vbo);
            (gl.glBufferData)(ARRAY_BUFFER, (m.sommets.len() * 4) as isize, m.sommets.as_ptr() as *const c_void, 0x88E8);
            (gl.glBindBuffer)(ELEMENT_ARRAY_BUFFER, t.ebo);
            (gl.glBufferData)(ELEMENT_ARRAY_BUFFER, (m.indices.len() * 4) as isize, m.indices.as_ptr() as *const c_void, 0x88E8);
            (gl.glBindVertexArray)(0);
            t.revision = m.revision;
            t.n = m.indices.len() as i32;
        }
        (t.vao, t.n)
    }

    /// Une vue 3D, dans son rectangle et la zone de découpe ; l'état de l'interface
    /// est rétabli ensuite.
    unsafe fn dessiner_vue(&self, w: f32, h: f32, v: &Vue3d, clip: Option<Rect>) {
        let gl = &self.gl;
        let r = clip.map_or(v.rect, |c| v.rect.intersect(&c));
        if r.w < 1.0 || r.h < 1.0 || v.maillage.indices.is_empty() {
            return;
        }
        let (vao, n) = self.tampon(v);
        let (x, y, cw, ch) = Self::gl_rect(r, h);
        (gl.glEnable)(SCISSOR);
        (gl.glScissor)(x, y, cw, ch);
        let (x, y, vw, vh) = Self::gl_rect(v.rect, h);
        (gl.glViewport)(x, y, vw, vh);
        (gl.glEnable)(DEPTH_TEST);
        (gl.glDisable)(BLEND);
        (gl.glClear)(DEPTH_BIT);
        (gl.glUseProgram)(self.maillage_program);
        (gl.glUniformMatrix4fv)(self.u_mvp, 1, 0, v.mvp.as_ptr());
        (gl.glUniformMatrix4fv)(self.u_vue, 1, 0, v.vue.as_ptr());
        (gl.glUniform3f)(self.u_couleur, v.couleur[0], v.couleur[1], v.couleur[2]);
        (gl.glBindVertexArray)(vao);
        (gl.glDrawElements)(TRIANGLES, n, UINT, std::ptr::null());
        // Retour à l'interface
        (gl.glDisable)(DEPTH_TEST);
        (gl.glEnable)(BLEND);
        (gl.glViewport)(0, 0, w as i32, h as i32);
        (gl.glUseProgram)(self.ui_program);
        (gl.glBindVertexArray)(self.ui_vao);
        (gl.glBindBuffer)(ARRAY_BUFFER, self.ui_vbo);
    }

    /// Efface l'écran avec la couleur `fond`, puis dessine l'interface.
    pub fn draw(&self, w: f32, h: f32, fond: Color, dl: &DrawList, text: &mut Text) {
        let gl = &self.gl;
        unsafe {
            (gl.glDisable)(SCISSOR);
            (gl.glViewport)(0, 0, w as i32, h as i32);
            let f = |i: usize| fond[i] as f32 / 255.0;
            (gl.glClearColor)(f(0), f(1), f(2), 1.0);
            (gl.glClear)(COLOR_BIT);

            // Interface, sur tout l'écran
            (gl.glEnable)(BLEND);
            (gl.glBlendFunc)(1, 0x0303); // couleurs prémultipliées
            (gl.glUseProgram)(self.ui_program);
            (gl.glUniform2f)(self.u_size, w, h);
            (gl.glActiveTexture)(0x84C0);
            (gl.glBindTexture)(TEXTURE_2D, self.atlas);
            if text.dirty {
                (gl.glTexSubImage2D)(TEXTURE_2D, 0, 0, 0, ATLAS as i32, ATLAS as i32, RED, UBYTE, text.atlas.as_ptr() as *const c_void);
                text.dirty = false;
            }
            (gl.glBindVertexArray)(self.ui_vao);
            (gl.glBindBuffer)(ARRAY_BUFFER, self.ui_vbo);
            (gl.glBufferData)(ARRAY_BUFFER, (dl.vertices.len() * size_of::<Vertex>()) as isize, dl.vertices.as_ptr() as *const c_void, 0x88E0);
            for c in &dl.cmds {
                if let Some(i) = c.vue3d {
                    self.dessiner_vue(w, h, &dl.vues3d[i], c.clip);
                    continue;
                }
                match c.clip {
                    Some(r) => {
                        let (x, y, cw, ch) = Self::gl_rect(r, h);
                        (gl.glEnable)(SCISSOR);
                        (gl.glScissor)(x, y, cw, ch);
                    }
                    None => (gl.glDisable)(SCISSOR),
                }
                (gl.glDrawArrays)(TRIANGLES, c.first as i32, c.count as i32);
            }
            (gl.glBindVertexArray)(0);
        }
    }
}
