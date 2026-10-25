//! The car's wires on the GPU (`carmodel.rs`: the owner's model's feature edges, baked by `bakecar`), drawn by the Overview's
//! Vehicle card (`car.rs`). Lines add up (additive, no depth test), dimmer with distance. Each line is a quad widened on the
//! screen and anti-aliased by its coverage, in one buffer and four draw calls in an `egui_glow` paint callback. (The welcome
//! screen's spinning 3D car was removed 2026-10-24, owner: it heated the machine and told the person nothing.)
use super::carmodel::{self, Group};
use eframe::egui::{self, Rect};
use eframe::egui_glow::{self, ShaderVersion};
use eframe::glow::{self, HasContext};
use std::sync::{Arc, Mutex};

/// Blue and white: blue interior, grey tyres, white body, bright white lamps (interior, tyres and wheels, body, lamps).
const PALETTE: [[f32; 3]; 4] = [[0.25, 0.45, 0.85], [0.50, 0.54, 0.60], [0.88, 0.92, 1.00], [1.00, 1.00, 1.00]];
const CALM: f32 = 0.5;           // lines add up where they cross, so the baked strengths blow out to white; halve them
const LINE_PT: f32 = 0.75;       // line width in points: thin, since lines add up where they cross

pub type Mat = [f32; 16];            // column-major, as GL wants it


// Each segment is a quad of four vertices (each carries its own end and the other end, `side` in the fourth short) that the
// vertex shader widens to `width` physical pixels on the screen, so a line is anti-aliased by its own coverage at any
// display scale; a GL line is one aliased pixel wide whatever the screen.
const VS: &str = "uniform mat4 mvp; uniform float unit; uniform vec2 vp; uniform float width;
in vec4 a_pos; in vec4 a_other; out float v_y; out float v_w; out float v_across;
void main() {
  vec3 p = a_pos.xyz * unit; vec3 q = a_other.xyz * unit;
  vec4 cp = mvp * vec4(p, 1.0); vec4 cq = mvp * vec4(q, 1.0);
  vec2 sp = cp.xy / cp.w * 0.5 * vp; vec2 sq = cq.xy / cq.w * 0.5 * vp;
  vec2 d = sq - sp; float l = length(d); d = l > 1e-4 ? d / l : vec2(1.0, 0.0);
  float h = width * 0.5 + 1.0;
  vec2 off = vec2(-d.y, d.x) * a_pos.w * h / (0.5 * vp);   // no cap beyond the ends: lines add up, so overlapping joints would be dots
  gl_Position = vec4(cp.xy + off * cp.w, cp.z, cp.w);
  v_y = p.y; v_w = cp.w; v_across = a_pos.w * h;
}";
// the group's colour times its strength and the line's coverage, dimmer the further from the camera, with a band of light
// sweeping up the car (`scan` is its height in metres)
const FS: &str = "uniform vec3 col; uniform float strength; uniform float scan; uniform float width; in float v_y; in float v_w; in float v_across; out vec4 o;
void main() {
  float depth = clamp(1.45 - (v_w - 6.0) / 6.0, 0.30, 1.0);
  float band = exp(-pow((v_y - scan) / 0.13, 2.0));
  float cover = clamp(width * 0.5 + 0.5 - abs(v_across), 0.0, 1.0);
  o = vec4(col * strength * depth * (1.0 + 0.5 * band) * cover, 1.0);
}";

pub struct Gpu { gl: Arc<glow::Context>, prog: glow::Program, vao: glow::VertexArray, vbo: glow::Buffer, ebo: glow::Buffer, groups: Vec<Group>,
    vp: Option<glow::UniformLocation>, width: Option<glow::UniformLocation>, mvp: Option<glow::UniformLocation>, col: Option<glow::UniformLocation>, strength: Option<glow::UniformLocation>, scan: Option<glow::UniformLocation> }

/// The wire's segments (consecutive vertex pairs, 4 x i16 each) as quads: per segment four vertices of 8 shorts (this end with
/// its side, -1 or 1, in place of the unused w; then the other end), and six u32 indices, both as bytes.
fn quads(verts: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let v = |i: usize| -> [i16; 4] { std::array::from_fn(|k| i16::from_le_bytes([verts[i * 8 + k * 2], verts[i * 8 + k * 2 + 1]])) };
    let n = verts.len() / 16;
    let (mut out, mut idx) = (Vec::with_capacity(n * 64), Vec::with_capacity(n * 24));
    for s in 0..n {
        let (a, b) = (v(2 * s), v(2 * s + 1));
        for (end, other, side) in [(a, b, 1i16), (a, b, -1), (b, a, 1), (b, a, -1)] {
            for x in [end[0], end[1], end[2], side, other[0], other[1], other[2], 0] { out.extend_from_slice(&x.to_le_bytes()); }
        }
        let q = 4 * s as u32;
        for i in [q, q + 1, q + 2, q + 2, q + 1, q + 3] { idx.extend_from_slice(&i.to_le_bytes()); }
    }
    (out, idx)
}

impl Gpu {
    fn new(gl: &Arc<glow::Context>) -> Result<Gpu, String> {
        let wire = carmodel::parse(carmodel::PACK).ok_or("car_wire.pack does not parse")?;
        unsafe {
            let head = match ShaderVersion::get(gl) {
                ShaderVersion::Gl140 => "#version 140\n",
                ShaderVersion::Es300 => "#version 300 es\nprecision highp float;\n",
                _ => return Err("OpenGL too old for the car".into()),
            };
            let prog = gl.create_program()?;
            let mut shaders = Vec::new();
            for (kind, src) in [(glow::VERTEX_SHADER, VS), (glow::FRAGMENT_SHADER, FS)] {
                let s = gl.create_shader(kind)?;
                gl.shader_source(s, &format!("{head}{src}"));
                gl.compile_shader(s);
                if !gl.get_shader_compile_status(s) { return Err(gl.get_shader_info_log(s)); }
                gl.attach_shader(prog, s);
                shaders.push(s);
            }
            gl.bind_attrib_location(prog, 0, "a_pos");
            gl.bind_attrib_location(prog, 1, "a_other");
            gl.link_program(prog);
            if !gl.get_program_link_status(prog) { return Err(gl.get_program_info_log(prog)); }
            for s in shaders { gl.detach_shader(prog, s); gl.delete_shader(s); }

            let (vao, vbo, ebo) = (gl.create_vertex_array()?, gl.create_buffer()?, gl.create_buffer()?);
            gl.bind_vertex_array(Some(vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            let (quads, indices) = quads(wire.verts);
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, &quads, glow::STATIC_DRAW);
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(ebo));
            gl.buffer_data_u8_slice(glow::ELEMENT_ARRAY_BUFFER, &indices, glow::STATIC_DRAW);
            for (loc, at) in [(0, 0), (1, 8)] {
                gl.enable_vertex_attrib_array(loc);
                gl.vertex_attrib_pointer_f32(loc, 4, glow::SHORT, false, 16, at);   // x, y, z in 0.1 mm, converted as they are (not normalised)
            }
            gl.use_program(Some(prog));
            gl.uniform_1_f32(gl.get_uniform_location(prog, "unit").as_ref(), carmodel::UNIT);
            Ok(Gpu { gl: gl.clone(), vao, vbo, ebo, groups: wire.groups,
                vp: gl.get_uniform_location(prog, "vp"), width: gl.get_uniform_location(prog, "width"),
                mvp: gl.get_uniform_location(prog, "mvp"), col: gl.get_uniform_location(prog, "col"),
                strength: gl.get_uniform_location(prog, "strength"), scan: gl.get_uniform_location(prog, "scan"), prog })
        }
    }

    fn draw(&self, mvp: &Mat, vp: [f32; 2], width: f32, scan: f32, gain: f32, mono: bool) {
        let gl = &self.gl;
        unsafe {
            gl.use_program(Some(self.prog));
            gl.uniform_matrix_4_f32_slice(self.mvp.as_ref(), false, mvp);
            gl.uniform_1_f32(self.scan.as_ref(), scan);
            gl.uniform_2_f32(self.vp.as_ref(), vp[0], vp[1]);
            gl.uniform_1_f32(self.width.as_ref(), width);
            gl.bind_vertex_array(Some(self.vao));
            gl.enable(glow::BLEND);
            gl.blend_equation(glow::FUNC_ADD);
            gl.blend_func(glow::ONE, glow::ONE);   // lines add: where they cross they glow, as a hologram's do
            for (i, g) in self.groups.iter().enumerate() {
                let c = if mono { [0.85; 3] } else { PALETTE[i.min(3)] };   // mono: every group plain light grey
                gl.uniform_3_f32(self.col.as_ref(), c[0], c[1], c[2]);
                gl.uniform_1_f32(self.strength.as_ref(), g.strength * gain * if mono { 1.0 } else { CALM });
                gl.draw_elements(glow::TRIANGLES, g.count / 2 * 6, glow::UNSIGNED_INT, g.start / 2 * 6 * 4);
            }
        }
    }
}

impl Drop for Gpu {
    fn drop(&mut self) {
        unsafe { self.gl.delete_program(self.prog); self.gl.delete_vertex_array(self.vao); self.gl.delete_buffer(self.vbo); self.gl.delete_buffer(self.ebo); }
    }
}

#[derive(Default)]
pub struct Shared { gpu: Option<Gpu>, failed: bool }

/// A paint callback that draws the wires with `mvp`; the buffer is made on first use and kept in `shared`.
pub fn wire_callback(shared: Arc<Mutex<Shared>>, rect: Rect, mvp: Mat, scan: f32, gain: f32, mono: bool) -> egui::PaintCallback {
    egui::PaintCallback { rect, callback: Arc::new(egui_glow::CallbackFn::new(move |info, painter| {
        // egui_glow clamps the callback's viewport to the window, so a car scrolled partly off an edge was squeezed into the
        // visible part (it stretched as the page scrolled). Set the whole rect, off-screen part included; the scissor egui
        // already set to the scroll area's clip rect cuts it off.
        let (ppx, h) = (info.pixels_per_point, info.screen_size_px[1] as f32);
        let (l, r) = ((ppx * info.viewport.min.x).round(), (ppx * info.viewport.max.x).round());
        let (t, b) = ((ppx * info.viewport.min.y).round(), (ppx * info.viewport.max.y).round());
        unsafe { painter.gl().viewport(l as i32, (h - b) as i32, (r - l) as i32, (b - t) as i32); }
        let mut s = shared.lock().unwrap();
        if s.gpu.is_none() && !s.failed {
            match Gpu::new(painter.gl()) { Ok(g) => s.gpu = Some(g), Err(e) => { eprintln!("car: {e}"); s.failed = true; } }
        }
        // a line 0.75 physical pixels wide at 1x, the same fraction of a point on a denser screen
        if let Some(g) = &s.gpu { g.draw(&mvp, [r - l, b - t], LINE_PT * ppx, scan, gain, mono); }
    })) }
}

