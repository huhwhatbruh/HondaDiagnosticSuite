//! The welcome screen's car: the owner's hologram model drawn live in 3D as a wiregram (`carmodel.rs`: its feature edges,
//! baked by `bakecar`), so every angle is a true view and the motion is as smooth as the screen. (An earlier version flipped
//! through 72 pre-rendered frames 5 degrees apart: the edges jump tens of pixels between neighbours, so every cross-fade
//! between them showed as ghost copies fading in and out.) It turns by itself and can be dragged: left/right spins it,
//! up/down tilts it, a throw dies away. Lines add up (additive, no depth test), teal for the body, orange for the interior,
//! dimmer with distance, with a band of light sweeping up the car. One buffer of about 0.95 MB, four draw calls, in an
//! `egui_glow` paint callback; the buffer exists only while the welcome screen is up (`App::turntable` is dropped once connected).
use super::carmodel::{self, Group};
use crate::theme::BLUE;
use eframe::egui::{self, pos2, vec2, Color32, Pos2, Rect, Sense, Shape, Stroke, Ui};
use eframe::egui_glow::{self, ShaderVersion};
use eframe::glow::{self, HasContext};
use std::sync::{Arc, Mutex};

const SPIN: f32 = 18.0;          // degrees a second on its own
const PER_PX: f32 = 0.4;         // degrees of yaw per pixel dragged
const TILT_PER_PX: f32 = 0.25;   // degrees of pitch per pixel dragged
const FRICTION: f32 = 2.2;       // a throw dies away with this rate a second
const PITCH: f32 = 17.0;         // resting view from above, degrees; drifts back to it after a drag
const DIST: f32 = 9.5;           // camera distance, metres
const HALF_WIDTH: f32 = 3.25;    // metres seen left and right of the car's centre at that distance
/// Muted slate blue for the body, burnt orange for the interior, greys for tyres and lamps; the baked pack's colours are brighter.
const PALETTE: [[f32; 3]; 4] = [[0.80, 0.42, 0.14], [0.45, 0.50, 0.58], [0.22, 0.45, 0.80], [0.55, 0.65, 0.80]];
const CALM: f32 = 0.5;           // lines add up where they cross, so the baked strengths blow out to white; halve them
const CENTRE_Y: f32 = 0.62;      // the car's middle height

pub type Mat = [f32; 16];            // column-major, as GL wants it

fn mul(a: &Mat, b: &Mat) -> Mat { std::array::from_fn(|i| (0..4).map(|k| a[k * 4 + i % 4] * b[(i / 4) * 4 + k]).sum()) }
fn translate(x: f32, y: f32, z: f32) -> Mat { [1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., x, y, z, 1.] }
fn rot_y(a: f32) -> Mat { let (s, c) = a.sin_cos(); [c, 0., -s, 0., 0., 1., 0., 0., s, 0., c, 0., 0., 0., 0., 1.] }
fn rot_x(a: f32) -> Mat { let (s, c) = a.sin_cos(); [1., 0., 0., 0., 0., c, s, 0., 0., -s, c, 0., 0., 0., 0., 1.] }
fn perspective(fy: f32, aspect: f32, near: f32, far: f32) -> Mat {
    let f = 1.0 / (fy / 2.0).tan();
    [f / aspect, 0., 0., 0., 0., f, 0., 0., 0., 0., (far + near) / (near - far), -1., 0., 0., 2.0 * far * near / (near - far), 0.]
}

const VS: &str = "uniform mat4 mvp; uniform float unit; in vec4 a_pos; out float v_y; out float v_w;
void main() { vec3 p = a_pos.xyz * unit; v_y = p.y; gl_Position = mvp * vec4(p, 1.0); v_w = gl_Position.w; }";
// the group's colour times its strength, dimmer the further from the camera, with a band of light sweeping up the car
// (`scan` is its height in metres)
const FS: &str = "uniform vec3 col; uniform float strength; uniform float scan; in float v_y; in float v_w; out vec4 o;
void main() {
  float depth = clamp(1.45 - (v_w - 6.0) / 6.0, 0.30, 1.0);
  float band = exp(-pow((v_y - scan) / 0.13, 2.0));
  o = vec4(col * strength * depth * (1.0 + 0.5 * band), 1.0);
}";

pub struct Gpu { gl: Arc<glow::Context>, prog: glow::Program, vao: glow::VertexArray, vbo: glow::Buffer, groups: Vec<Group>,
    mvp: Option<glow::UniformLocation>, col: Option<glow::UniformLocation>, strength: Option<glow::UniformLocation>, scan: Option<glow::UniformLocation> }

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
            gl.link_program(prog);
            if !gl.get_program_link_status(prog) { return Err(gl.get_program_info_log(prog)); }
            for s in shaders { gl.detach_shader(prog, s); gl.delete_shader(s); }

            let (vao, vbo) = (gl.create_vertex_array()?, gl.create_buffer()?);
            gl.bind_vertex_array(Some(vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, wire.verts, glow::STATIC_DRAW);
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 4, glow::SHORT, false, 8, 0);   // x, y, z in 0.1 mm, converted as they are (not normalised)
            gl.use_program(Some(prog));
            gl.uniform_1_f32(gl.get_uniform_location(prog, "unit").as_ref(), carmodel::UNIT);
            Ok(Gpu { gl: gl.clone(), vao, vbo, groups: wire.groups,
                mvp: gl.get_uniform_location(prog, "mvp"), col: gl.get_uniform_location(prog, "col"),
                strength: gl.get_uniform_location(prog, "strength"), scan: gl.get_uniform_location(prog, "scan"), prog })
        }
    }

    fn draw(&self, mvp: &Mat, scan: f32, gain: f32, mono: bool) {
        let gl = &self.gl;
        unsafe {
            gl.use_program(Some(self.prog));
            gl.uniform_matrix_4_f32_slice(self.mvp.as_ref(), false, mvp);
            gl.uniform_1_f32(self.scan.as_ref(), scan);
            gl.bind_vertex_array(Some(self.vao));
            gl.enable(glow::BLEND);
            gl.blend_equation(glow::FUNC_ADD);
            gl.blend_func(glow::ONE, glow::ONE);   // lines add: where they cross they glow, as a hologram's do
            for (i, g) in self.groups.iter().enumerate() {
                let c = if mono { [0.85; 3] } else { PALETTE[i.min(3)] };   // mono: every group plain light grey
                gl.uniform_3_f32(self.col.as_ref(), c[0], c[1], c[2]);
                gl.uniform_1_f32(self.strength.as_ref(), g.strength * gain * if mono { 1.0 } else { CALM });
                gl.draw_arrays(glow::LINES, g.start, g.count);
            }
        }
    }
}

impl Drop for Gpu {
    fn drop(&mut self) {
        unsafe { self.gl.delete_program(self.prog); self.gl.delete_vertex_array(self.vao); self.gl.delete_buffer(self.vbo); }
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
        if let Some(g) = &s.gpu { g.draw(&mvp, scan, gain, mono); }
    })) }
}

pub struct Turntable {
    shared: Arc<Mutex<Shared>>,
    yaw: f32,                    // degrees, 0 = nose towards the viewer
    pitch: f32,                  // degrees above the horizon
    speed: f32,                  // degrees a second from a throw, dying away
}

impl Turntable {
    pub fn new() -> Turntable { Turntable { shared: Arc::default(), yaw: 35.0, pitch: PITCH, speed: 0.0 } }

    /// Where the point (x, 0, z) on the floor lands in `rect`.
    fn floor(mvp: &Mat, rect: Rect, x: f32, z: f32) -> Pos2 {
        let c = |r: usize| mvp[r] * x + mvp[8 + r] * z + mvp[12 + r];
        let (cx, cy, cw) = (c(0), c(1), c(3));
        pos2(rect.left() + (cx / cw * 0.5 + 0.5) * rect.width(), rect.top() + (0.5 - cy / cw * 0.5) * rect.height())
    }

    /// Draw it `width` points wide, half as high.
    pub fn show(&mut self, ui: &mut Ui, width: f32) {
        let (rect, resp) = ui.allocate_exact_size(vec2(width, width * 0.5), Sense::click_and_drag());
        let dt = ui.input(|i| i.stable_dt).min(0.1);
        if resp.dragged() {
            let d = resp.drag_delta();
            self.speed = 0.6 * self.speed + 0.4 * d.x * PER_PX / dt.max(0.004);
            self.yaw += d.x * PER_PX;
            self.pitch = (self.pitch + d.y * TILT_PER_PX).clamp(3.0, 70.0);
        } else {
            self.speed *= (-FRICTION * dt).exp();
            self.yaw += (SPIN + self.speed) * dt;
            self.pitch += (PITCH - self.pitch) * (1.0 - (-0.5 * dt).exp());
        }
        self.yaw = self.yaw.rem_euclid(360.0);
        if resp.hovered() || resp.dragged() { ui.ctx().set_cursor_icon(if resp.dragged() { egui::CursorIcon::Grabbing } else { egui::CursorIcon::Grab }); }

        let aspect = rect.width() / rect.height();
        let fy = 2.0 * (HALF_WIDTH / DIST / aspect).atan();
        let view = mul(&translate(0.0, 0.0, -DIST), &mul(&rot_x(self.pitch.to_radians()), &mul(&rot_y(self.yaw.to_radians()), &translate(0.0, -CENTRE_Y, 0.0))));
        let mvp = mul(&perspective(fy, aspect, 1.0, 40.0), &view);

        // a floor ring and a ring of ticks under the car, in the same projection
        let paint = ui.painter_at(rect);
        let ring = |radius: f32, a: u8| {
            let pts: Vec<Pos2> = (0..=96).map(|i| { let t = i as f32 / 96.0 * std::f32::consts::TAU; Self::floor(&mvp, rect, radius * t.cos(), radius * t.sin()) }).collect();
            paint.add(Shape::line(pts, Stroke::new(1.0, BLUE.gamma_multiply(a as f32 / 255.0))));
        };
        ring(2.7, 90);
        ring(2.88, 40);
        for i in 0..24 {
            let t = i as f32 / 24.0 * std::f32::consts::TAU;
            let len = if i % 6 == 0 { 0.35 } else { 0.15 };
            let (a, b) = (Self::floor(&mvp, rect, 2.7 * t.cos(), 2.7 * t.sin()), Self::floor(&mvp, rect, (2.7 + len) * t.cos(), (2.7 + len) * t.sin()));
            paint.line_segment([a, b], Stroke::new(1.0, Color32::from_rgba_unmultiplied(BLUE.r(), BLUE.g(), BLUE.b(), 110)));
        }

        let scan = (ui.input(|i| i.time) as f32 * 0.35).fract() * 2.3 - 0.3;   // metres up the car, a little beyond both ends
        ui.painter().add(wire_callback(self.shared.clone(), rect, mvp, scan, 1.0, false));
        if ui.input(|i| i.viewport().focused != Some(false)) { ui.ctx().request_repaint(); }   // always moving, at the screen's rate; it rests while the window is in the background
    }
}
