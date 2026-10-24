//! The logo as a spinning 3D pixel sculpture: the PNG is reduced to a coarse grid, every opaque cell becomes a small stack
//! of cubes, and the whole thing turns about its vertical axis, drawn face by face with the painter (back to front, lit
//! from the upper left). Colour: the logo's own light and dark, mapped onto the app's blue.
use crate::theme::BLUE;
use eframe::egui::{epaint::{Vertex, WHITE_UV}, pos2, Color32, Mesh, Pos2, Rect, Shape, Stroke, Ui};
use image::{imageops, RgbaImage};

pub struct Voxels { w: usize, h: usize, depth: usize, cells: Vec<Option<Color32>> }

/// Light to dark blue, by the logo's brightness at that cell.
fn ramp(lum: f32) -> Color32 {
    let (dark, light) = ([8.0, 22.0, 52.0], [140.0, 180.0, 232.0]);
    let mid = [BLUE.r() as f32, BLUE.g() as f32, BLUE.b() as f32];
    let t = lum.clamp(0.0, 1.0);
    let c = |i: usize| if t < 0.5 { dark[i] + (mid[i] - dark[i]) * t * 2.0 } else { mid[i] + (light[i] - mid[i]) * (t - 0.5) * 2.0 };
    Color32::from_rgb(c(0) as u8, c(1) as u8, c(2) as u8)
}

impl Voxels {
    /// `w` x `h` cells (the picture's aspect ratio should match), `depth` cubes thick. A cell exists where the picture is opaque.
    pub fn build(img: &RgbaImage, w: usize, h: usize, depth: usize) -> Voxels {
        let small = imageops::resize(img, w as u32, h as u32, imageops::FilterType::Triangle);
        let cells = small.pixels().map(|p| {
            if p[3] < 110 { return None; }
            let lum = (0.3 * p[0] as f32 + 0.59 * p[1] as f32 + 0.11 * p[2] as f32) / 255.0;
            Some(ramp(1.0 - lum * 0.85))   // chrome: the bright metal reads as the deep blue, shadow lines as light edges
        }).collect();
        Voxels { w, h, depth, cells }
    }
    fn at(&self, x: i32, y: i32) -> Option<Color32> {
        (x >= 0 && y >= 0 && (x as usize) < self.w && (y as usize) < self.h).then(|| self.cells[y as usize * self.w + x as usize]).flatten()
    }
}

fn shade(c: Color32, k: f32) -> Color32 {
    Color32::from_rgb((c.r() as f32 * k).min(255.0) as u8, (c.g() as f32 * k).min(255.0) as u8, (c.b() as f32 * k).min(255.0) as u8)
}

/// Mix `c` towards `to` by `k` (0..1).
fn mix(c: Color32, to: Color32, k: f32) -> Color32 {
    let f = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * k) as u8;
    Color32::from_rgb(f(c.r(), to.r()), f(c.g(), to.g()), f(c.b(), to.b()))
}

/// Draw the model turning, `t` seconds in, filling `rect` (the model is fitted and centred). A scanner sweeps up and
/// down it: the cubes it passes light up softly, and a thin line marks where it is.
///
/// Draw order is exact, not sorted: for cubes on a grid seen without perspective, drawing the grid back to front along
/// each axis (the direction each axis points away from the viewer decides which end comes first) can never put a far
/// face over a near one, so nothing flickers between frames. All faces go into one mesh: one shape instead of thousands.
pub fn show(ui: &Ui, v: &Voxels, rect: Rect, t: f64) {
    let (sin_a, cos_a) = ((t * 1.1) as f32).sin_cos();
    let (sin_t, cos_t) = 0.22f32.sin_cos();                       // a slight downward tilt shows the top faces
    let rot = |p: [f32; 3]| {
        let (x, z) = (p[0] * cos_a + p[2] * sin_a, -p[0] * sin_a + p[2] * cos_a);
        let (y, z) = (p[1] * cos_t - z * sin_t, p[1] * sin_t + z * cos_t);
        [x, y, z]
    };
    let radius = (v.w as f32).hypot(v.depth as f32) / 2.0;
    let scale = (rect.width() / (radius * 2.0)).min(rect.height() / (v.h as f32 * 1.1));
    let c = rect.center();
    let proj = |p: [f32; 3]| { let r = rot(p); pos2(c.x + r[0] * scale, c.y + r[1] * scale) };
    let light = [-0.45f32, -0.55, 0.7];
    let (hw, hh, hd) = (v.w as f32 / 2.0, v.h as f32 / 2.0, v.depth as f32 / 2.0);

    // which way each axis points away from the viewer: the third row of the rotation
    let far = [-sin_a * cos_t, sin_t, cos_a * cos_t];
    let order = |n: usize, desc: bool| -> Vec<i32> { let r: Vec<i32> = (0..n as i32).collect(); if desc { r.into_iter().rev().collect() } else { r } };
    let (xs, ys, zs) = (order(v.w, far[0] > 0.0), order(v.h, far[1] > 0.0), order(v.depth, far[2] > 0.0));

    let scan_row = (0.5 + 0.5 * ((t * 1.3) as f32).sin()) * v.h as f32;          // 0 = top .. h = bottom, there and back
    let reach = (v.h as f32 * 0.08).max(1.5);
    let glow = Color32::from_rgb(170, 215, 255);
    let mut mesh = Mesh::default();
    mesh.vertices.reserve(v.w * v.h * 8);
    let mut quad = |pts: [Pos2; 4], col: Color32| {
        let m = pts.iter().fold(Pos2::ZERO, |a, p| a + p.to_vec2()) / 4.0;
        let base = mesh.vertices.len() as u32;
        for p in pts { mesh.vertices.push(Vertex { pos: m + (p - m) * 1.05, uv: WHITE_UV, color: col }); }   // 5 % larger: no seams between faces
        mesh.indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    };
    for &y in &ys {
        for &x in &xs {
            let Some(col) = v.at(x, y) else { continue };
            let (x0, x1, y0, y1) = (x as f32 - hw, x as f32 + 1.0 - hw, y as f32 - hh, y as f32 + 1.0 - hh);
            let near = (1.0 - (y as f32 + 0.5 - scan_row).abs() / reach).clamp(0.0, 1.0);
            let col = if near > 0.0 { mix(col, glow, near * 0.4) } else { col };
            for &z in &zs {
                let (z0, z1) = (z as f32 - hd, z as f32 + 1.0 - hd);
                // (normal, corners, hidden behind a neighbour?)
                let sides: [([f32; 3], [[f32; 3]; 4], bool); 6] = [
                    ([0.0, 0.0, -1.0], [[x0, y0, z0], [x1, y0, z0], [x1, y1, z0], [x0, y1, z0]], z > 0),
                    ([0.0, 0.0, 1.0], [[x0, y0, z1], [x1, y0, z1], [x1, y1, z1], [x0, y1, z1]], z + 1 < v.depth as i32),
                    ([-1.0, 0.0, 0.0], [[x0, y0, z0], [x0, y0, z1], [x0, y1, z1], [x0, y1, z0]], v.at(x - 1, y).is_some()),
                    ([1.0, 0.0, 0.0], [[x1, y0, z0], [x1, y0, z1], [x1, y1, z1], [x1, y1, z0]], v.at(x + 1, y).is_some()),
                    ([0.0, -1.0, 0.0], [[x0, y0, z0], [x1, y0, z0], [x1, y0, z1], [x0, y0, z1]], v.at(x, y - 1).is_some()),
                    ([0.0, 1.0, 0.0], [[x0, y1, z0], [x1, y1, z0], [x1, y1, z1], [x0, y1, z1]], v.at(x, y + 1).is_some()),
                ];
                for (n, corners, hidden) in sides {
                    if hidden { continue; }
                    let rn = rot(n);
                    if rn[2] >= 0.0 { continue; }                    // faces away from the viewer (+z is away)
                    let lit = 0.62 + 0.38 * (-(rn[0] * light[0] + rn[1] * light[1] + rn[2] * light[2])).max(0.0);   // mostly ambient: not glossy
                    quad(corners.map(proj), shade(col, lit));
                }
            }
        }
    }
    let p = ui.painter();
    p.add(Shape::mesh(mesh));

    // The scanner line follows the model: it exists only where there are cubes on the row it is at, runs along the face
    // that is turned towards the viewer, and wraps round the thickness at the ends of each run (so it hugs the outline).
    let row = (scan_row as i32).clamp(0, v.h as i32 - 1);
    let ymod = scan_row - hh;
    let toward_viewer_is_front = rot([0.0, 0.0, -1.0])[2] < 0.0;
    let zf = if toward_viewer_is_front { -hd } else { hd };
    let core = Stroke::new(2.0, Color32::from_rgba_unmultiplied(200, 230, 255, 220));
    let halo = Stroke::new(5.0, Color32::from_rgba_unmultiplied(110, 170, 240, 34));
    let line = |a: [f32; 3], b: [f32; 3]| {
        let (pa, pb) = (proj(a), proj(b));
        p.line_segment([pa, pb], halo);
        p.line_segment([pa, pb], core);
    };
    let mut x = 0;
    while x < v.w as i32 {
        if v.at(x, row).is_none() { x += 1; continue; }
        let start = x;
        while v.at(x, row).is_some() { x += 1; }
        let (xl, xr) = (start as f32 - hw, x as f32 - hw);
        line([xl, ymod, zf], [xr, ymod, zf]);                          // along the visible face
        for xe in [xl, xr] { line([xe, ymod, -hd], [xe, ymod, hd]); }   // round the thickness at both ends of the run
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_opaque_cells_become_cubes_and_dark_metal_is_the_light_blue() {
        let mut img = RgbaImage::new(8, 8);
        for y in 2..6 { for x in 2..6 { img.put_pixel(x, y, image::Rgba([255, 255, 255, 255])); } }   // a white 4 x 4 square, the rest transparent
        img.put_pixel(2, 2, image::Rgba([0, 0, 0, 255]));
        let v = Voxels::build(&img, 8, 8, 3);
        assert_eq!(v.cells.iter().flatten().count(), 16);
        let (dark, bright) = (v.at(2, 2).unwrap(), v.at(4, 4).unwrap());
        assert!(dark.b() > bright.b() && dark.r() > bright.r());                     // black pixel -> light edge, white metal -> deep blue
        assert!(v.at(0, 0).is_none() && v.at(-1, 0).is_none() && v.at(8, 3).is_none());
    }
}
