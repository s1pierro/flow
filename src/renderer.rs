//! Tout OpenGL est ici : la 3D dans chaque vue de l'espace de travail, puis
//! l'interface par-dessus, sur tout l'écran.

use crate::demo::Scene;
use crate::draw::{DrawList, Rect, Vertex};
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
const DEPTH_TEST: u32 = 0x0B71;
const SCISSOR: u32 = 0x0C11;
const COLOR_BIT: u32 = 0x4000;
const DEPTH_BIT: u32 = 0x0100;

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

const CUBE_VS: &str = "#version 300 es
precision highp float;
layout(location=0) in vec3 a_pos;
layout(location=1) in vec3 a_normal;
layout(location=2) in vec3 a_tint;
layout(location=3) in vec2 a_uv;
uniform mat4 u_mvp;
out vec3 v_normal; out vec3 v_tint; out vec2 v_uv;
void main() {
  v_normal = a_normal; v_tint = a_tint; v_uv = a_uv;
  gl_Position = u_mvp * vec4(a_pos, 1.0);
}";

const CUBE_FS: &str = "#version 300 es
precision highp float;
in vec3 v_normal; in vec3 v_tint; in vec2 v_uv;
uniform vec3 u_color;
uniform float u_mix;
out vec4 o_color;
void main() {
  vec3 base = mix(u_color, v_tint, u_mix);
  vec3 n = normalize(v_normal);
  float light = 0.55 + 0.45 * max(dot(n, normalize(vec3(0.4, 0.8, 0.5))), 0.0);
  vec2 e = min(v_uv, 1.0 - v_uv);
  float edge = smoothstep(0.0, 0.035, min(e.x, e.y));
  o_color = vec4(base * light * mix(0.72, 1.0, edge), 1.0);
}";

fn cube_mesh() -> Vec<f32> {
    // Normale, axes de la face, teinte.
    let faces: [([f32; 3], [f32; 3], [f32; 3], [f32; 3]); 6] = [
        ([0., 0., 1.], [1., 0., 0.], [0., 1., 0.], [0.95, 0.35, 0.35]),
        ([0., 0., -1.], [-1., 0., 0.], [0., 1., 0.], [0.25, 0.75, 0.55]),
        ([1., 0., 0.], [0., 0., -1.], [0., 1., 0.], [0.3, 0.5, 0.95]),
        ([-1., 0., 0.], [0., 0., 1.], [0., 1., 0.], [0.95, 0.75, 0.25]),
        ([0., 1., 0.], [1., 0., 0.], [0., 0., -1.], [0.9, 0.9, 0.95]),
        ([0., -1., 0.], [1., 0., 0.], [0., 0., 1.], [0.6, 0.45, 0.85]),
    ];
    let mut v = Vec::with_capacity(36 * 11);
    for (n, u, w, tint) in faces {
        for (a, b) in [(0., 0.), (1., 0.), (0., 1.), (0., 1.), (1., 0.), (1., 1.)] {
            for i in 0..3 {
                v.push(n[i] + (2.0 * a - 1.0) * u[i] + (2.0 * b - 1.0) * w[i]);
            }
            v.extend_from_slice(&n);
            v.extend_from_slice(&tint);
            v.extend_from_slice(&[a, b]);
        }
    }
    v
}

pub struct Renderer {
    gl: GlApi,
    ui_program: u32,
    u_size: i32,
    ui_vao: u32,
    ui_vbo: u32,
    atlas: u32,
    cube_program: u32,
    u_mvp: i32,
    u_color: i32,
    u_mix: i32,
    cube_vao: u32,
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
            let cube_program = program(&gl, CUBE_VS, CUBE_FS)?;
            let loc = |p: u32, n: &str| (gl.glGetUniformLocation)(p, cstr(n).as_ptr());
            let u_size = loc(ui_program, "u_size");
            let (u_mvp, u_color, u_mix) = (loc(cube_program, "u_mvp"), loc(cube_program, "u_color"), loc(cube_program, "u_mix"));

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

            let mesh = cube_mesh();
            let (mut cube_vao, mut cube_vbo) = (0, 0);
            (gl.glGenVertexArrays)(1, &mut cube_vao);
            (gl.glGenBuffers)(1, &mut cube_vbo);
            (gl.glBindVertexArray)(cube_vao);
            (gl.glBindBuffer)(ARRAY_BUFFER, cube_vbo);
            (gl.glBufferData)(ARRAY_BUFFER, (mesh.len() * 4) as isize, mesh.as_ptr() as *const c_void, 0x88E4);
            for (i, n, off) in [(0u32, 3, 0usize), (1, 3, 12), (2, 3, 24), (3, 2, 36)] {
                (gl.glEnableVertexAttribArray)(i);
                (gl.glVertexAttribPointer)(i, n, FLOAT, 0, 44, off as *const c_void);
            }
            (gl.glBindVertexArray)(0);

            Ok(Renderer { gl, ui_program, u_size, ui_vao, ui_vbo, atlas, cube_program, u_mvp, u_color, u_mix, cube_vao })
        }
    }

    pub fn gl(&self) -> &GlApi {
        &self.gl
    }

    /// Rectangle en pixels (origine en haut) vers OpenGL (origine en bas).
    fn gl_rect(r: Rect, h: f32) -> (i32, i32, i32, i32) {
        (r.x.round() as i32, (h - r.bottom()).round() as i32, r.w.round().max(0.0) as i32, r.h.round().max(0.0) as i32)
    }

    pub fn draw(&self, w: f32, h: f32, scene: &Scene, dl: &DrawList, text: &mut Text) {
        let gl = &self.gl;
        unsafe {
            (gl.glDisable)(SCISSOR);
            (gl.glViewport)(0, 0, w as i32, h as i32);
            let f = |i: usize| scene.fond[i] as f32 / 255.0;
            (gl.glClearColor)(f(0), f(1), f(2), 1.0);
            (gl.glClear)(COLOR_BIT | DEPTH_BIT);

            // 3D, chaque vue limitée à sa fenêtre
            (gl.glEnable)(DEPTH_TEST);
            (gl.glDisable)(BLEND);
            (gl.glEnable)(SCISSOR);
            (gl.glUseProgram)(self.cube_program);
            (gl.glUniform3f)(self.u_color, scene.color[0], scene.color[1], scene.color[2]);
            (gl.glUniform1f)(self.u_mix, scene.tint);
            (gl.glBindVertexArray)(self.cube_vao);
            for v in &scene.vues {
                let (x, y, vw, vh) = Self::gl_rect(v.rect, h);
                if vw <= 0 || vh <= 0 {
                    continue;
                }
                (gl.glScissor)(x, y, vw, vh);
                (gl.glViewport)(x, y, vw, vh);
                // Chaque vue a sa propre profondeur
                (gl.glClear)(DEPTH_BIT);
                (gl.glUniformMatrix4fv)(self.u_mvp, 1, 0, v.mvp.as_ptr());
                (gl.glDrawArrays)(TRIANGLES, 0, 36);
            }

            // Interface, sur tout l'écran
            (gl.glDisable)(DEPTH_TEST);
            (gl.glDisable)(SCISSOR);
            (gl.glViewport)(0, 0, w as i32, h as i32);
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
