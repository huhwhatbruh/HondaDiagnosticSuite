//! The few pieces the whole UI is built from: matte panes, windows with a title bar, buttons.
use crate::theme::*;
use eframe::egui::{self, pos2, vec2, Align2, Color32, CursorIcon, FontId, InnerResponse, Margin, Painter, Pos2, Rect, Response, Sense, Shape, Stroke, Ui};

/// How far a corner's easing reaches along each edge, as a multiple of its radius. A circular arc meets the straight edge
/// at one point where the curvature jumps, which the eye reads as a corner; Apple's "continuous" corners ease in over a
/// longer run, and a superellipse of exponent 4 does the same.
const SMOOTH: f32 = 1.45;

/// The outline of `r` with continuous corners of the given radii (north-west, north-east, south-east, south-west; 0 is
/// square), clockwise, so it can be filled as a convex polygon with egui's anti-aliased edge.
pub fn squircle(r: Rect, radii: [f32; 4]) -> Vec<Pos2> {
    let max = r.width().min(r.height()) / 2.0;
    let mut pts = Vec::new();
    let corners = [(r.left_top(), vec2(1.0, 1.0)), (r.right_top(), vec2(-1.0, 1.0)), (r.right_bottom(), vec2(-1.0, -1.0)), (r.left_bottom(), vec2(1.0, -1.0))];
    for (q, ((corner, inward), radius)) in corners.into_iter().zip(radii).enumerate() {
        let e = (radius * SMOOTH).min(max);
        if e <= 0.0 { pts.push(corner); continue; }
        let centre = corner + inward * e;
        let n = (6.0 + e / 2.0).clamp(8.0, 16.0) as usize;
        for i in 0..=n {
            let t = i as f32 / n as f32 * std::f32::consts::FRAC_PI_2;
            // the north-west quarter of |x|^4 + |y|^4 = 1, from the left edge to the top edge, then turned a quarter per corner
            // (cos of a quarter turn is -4e-8 in f32: its square root would be NaN, hence the clamp)
            let mut v = vec2(-t.cos().max(0.0).sqrt(), -t.sin().max(0.0).sqrt());
            for _ in 0..q { v = vec2(-v.y, v.x); }
            pts.push(centre + v * e);
        }
    }
    pts
}

/// `pts` (convex) cut to the rows between `y0` and `y1`.
fn band(pts: &[Pos2], y0: f32, y1: f32) -> Vec<Pos2> {
    fn cut(pts: &[Pos2], keep: impl Fn(Pos2) -> f32) -> Vec<Pos2> {
        let mut out = Vec::new();
        for (i, &a) in pts.iter().enumerate() {
            let b = pts[(i + 1) % pts.len()];
            let (da, db) = (keep(a), keep(b));
            if da >= 0.0 { out.push(a); }
            if (da >= 0.0) != (db >= 0.0) { out.push(a + (b - a) * (da / (da - db))); }
        }
        out
    }
    cut(&cut(pts, |p| p.y - y0), |p| y1 - p.y)
}

/// `c` lit up by `amount` (0..1) towards white, keeping its transparency.
fn lit(c: Color32, amount: f32) -> Color32 {
    let a = c.a() as f32 / 255.0;
    Color32::from_rgb(c.r(), c.g(), c.b()).lerp_to_gamma(Color32::WHITE, amount).gamma_multiply(a)
}

fn poly(pts: Vec<Pos2>, fill: Color32) -> Shape { Shape::convex_polygon(pts, fill, Stroke::NONE) }

/// Brightness levels (of 255) the surface gains at the very top of a pane, easing to none over `RAMP_PT` points.
const RAMP_TOP: f32 = 10.0;
const RAMP_PT: f32 = 48.0;
/// Texture rows for that ramp: two per point, so it is still one row per pixel at 2x.
const RAMP_ROWS: usize = 96;

/// The ramp as a texture of white with alpha, 8 columns wide and repeated: one whole number of brightness levels per
/// texel, chosen by an ordered (Bayer) dither. Ten levels of grey cannot be drawn smoothly in 8 bits, so a plain gradient
/// shows as ten bars; spread over the pixels as a fine pattern the steps cannot be seen. (A blended texel of alpha `n`
/// adds about `n` levels to the surface under it, whatever that is.)
fn ramp_texture(ctx: &egui::Context) -> egui::TextureId {
    let key = egui::Id::new("ramp-dither");
    if let Some(t) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(key)) { return t.id(); }
    let bayer = |x: usize, y: usize| -> f32 {
        const M2: [[usize; 2]; 2] = [[0, 2], [3, 1]];
        ((0..3).map(|l| M2[(y >> l) & 1][(x >> l) & 1] * 4usize.pow(l as u32)).sum::<usize>() as f32 + 0.5) / 64.0
    };
    let mut pixels = Vec::with_capacity(8 * RAMP_ROWS);
    for row in 0..RAMP_ROWS {
        let d = RAMP_TOP * (1.0 - (row as f32 + 0.5) / RAMP_ROWS as f32);
        for x in 0..8 {
            let n = (d.floor() + if d.fract() > bayer(x, row % 8) { 1.0 } else { 0.0 }) as u8;
            pixels.push(Color32::from_rgba_premultiplied(n, n, n, n));
        }
    }
    let opts = egui::TextureOptions { magnification: egui::TextureFilter::Nearest, minification: egui::TextureFilter::Nearest,
        wrap_mode: egui::TextureWrapMode::Repeat, mipmap_mode: None };
    let handle = ctx.load_texture("ramp-dither", egui::ColorImage::new([8, RAMP_ROWS], pixels), opts);
    let id = handle.id();
    ctx.data_mut(|d| d.insert_temp(key, handle));
    id
}

/// `pts` (convex) as a triangle fan, each vertex coloured and mapped by `vertex`.
fn fan(pts: &[Pos2], texture: egui::TextureId, vertex: impl Fn(Pos2) -> (egui::Pos2, Color32)) -> Shape {
    let mut mesh = egui::Mesh::with_texture(texture);
    for &p in pts {
        let (uv, color) = vertex(p);
        mesh.vertices.push(egui::epaint::Vertex { pos: p, uv, color });
    }
    for i in 1..pts.len().saturating_sub(1) as u32 { mesh.add_triangle(0, i, i + 1); }
    Shape::mesh(mesh)
}

/// A pane with an outline, matte but with depth: continuous corners, the outline a filled band 1 px wide (not a stroke: at
/// 1x a 1 px stroke is split over two pixels wherever it curves), lit along the top edge and fading down the sides as if
/// lit from above, and the surface a few levels lighter at the top, dithered so it has no visible steps. Dark Mode depth
/// by brightness, no gloss or bevel.
pub fn outlined(ctx: &egui::Context, r: Rect, radius: u8, fill: Color32, edge: Color32) -> Shape {
    let rad = radius as f32;
    let outer = squircle(r, [rad; 4]);
    let inner = squircle(r.shrink(1.0), [(rad - 1.0).max(0.0); 4]);
    let lit_h = (r.height() / 3.0).min(24.0);
    let mut shapes = vec![poly(outer.clone(), edge)];
    // the edge, brightest at the top and easing out over `lit_h`: a band a pixel tall each. Each polygon runs from the top
    // down to its own bottom and they are drawn longest first, so the anti-aliased seam between two bands blends their two
    // colours instead of letting the plain edge colour show through.
    let n = lit_h.ceil() as usize;
    for k in (0..n).rev() {
        let (t, y1) = ((k as f32 + 0.5) / n as f32, r.top() + lit_h * (k + 1) as f32 / n as f32);
        shapes.push(poly(band(&outer, r.top(), y1), lit(edge, 0.28 * (1.0 - t).powf(1.5))));
    }
    shapes.push(poly(inner.clone(), fill));
    // the surface, a little lighter at the top: the dithered ramp over the first `RAMP_PT` points
    let ramp = (r.height() / 2.0).min(RAMP_PT);
    let ppp = ctx.pixels_per_point();
    let tint = Color32::from_white_alpha(fill.a());
    shapes.push(fan(&band(&inner, r.top(), r.top() + ramp), ramp_texture(ctx), |p| (pos2(p.x * ppp / 8.0, (p.y - r.top()) / ramp), tint)));
    Shape::Vec(shapes)
}

/// A pane: CARD with an outline.
pub fn pane(ui: &Ui, r: Rect) { ui.painter().add(outlined(ui.ctx(), r, RADIUS_CARD, CARD, OUTLINE)); }

/// A one-pixel edge inside `r`: `tl` along the top and left, `br` along the bottom and right.
fn edges(r: Rect, tl: Color32, br: Color32) -> [Shape; 4] {
    let (lt, rt, lb, rb) = (r.left_top(), r.right_top(), r.left_bottom(), r.right_bottom());
    [(lt + vec2(0.5, 0.5), rt + vec2(-0.5, 0.5), tl), (lt + vec2(0.5, 0.5), lb + vec2(0.5, -0.5), tl),
     (lb + vec2(0.5, -0.5), rb + vec2(-0.5, -0.5), br), (rt + vec2(-0.5, 0.5), rb + vec2(-0.5, -0.5), br)]
        .map(|(a, b, c)| Shape::line_segment([a, b], Stroke::new(1.0, c)))
}

/// A top-to-bottom gradient over `r`, a mesh (colours are interpolated per pixel, so it has no bands).
fn vgradient(r: Rect, top: Color32, bottom: Color32) -> Shape {
    let mut m = egui::Mesh::default();
    for (pos, color) in [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()].into_iter().zip([top, top, bottom, bottom]) {
        m.vertices.push(egui::epaint::Vertex { pos, uv: egui::epaint::WHITE_UV, color });
    }
    m.indices.extend([0, 1, 2, 0, 2, 3]);
    Shape::mesh(m)
}

/// A raised key (button face), square: lit from above (lighter top, darker bottom), a black outline, a light edge top-left
/// and a dark one bottom-right, then a softer second pair inside. Pressed, it flips: darker and sunken. `amount` (0..1) fades
/// the whole key in, for a hover.
pub fn key_shapes(r: Rect, fill: Color32, down: bool, amount: f32) -> Shape {
    let (top, bot) = if down { (fill.lerp_to_gamma(Color32::BLACK, 0.18), fill.lerp_to_gamma(Color32::BLACK, 0.05)) } else { (lit(fill, 0.16), fill.lerp_to_gamma(Color32::BLACK, 0.2)) };
    let f = |c: Color32| c.gamma_multiply(amount);
    let (a, b) = if down { (LO, HI) } else { (HI, LO) };
    let mut shapes = vec![vgradient(r, f(top), f(bot)), Shape::rect_stroke(r, 0.0, Stroke::new(1.0, f(Color32::BLACK)), egui::StrokeKind::Inside)];
    shapes.extend(edges(r.shrink(1.0), f(a), f(b)));
    shapes.extend(edges(r.shrink(2.0), f(a), f(b)));
    Shape::Vec(shapes)
}

/// Where a cylinder button's flat top is centred (for its icon): its base is a little below `c`, and the top stands a wall's
/// height above the base, so pressing it moves the top down.
pub fn cylinder_top(c: Pos2, r: f32, down: bool) -> Pos2 { c + vec2(0.0, r * 0.1 - r * if down { 0.04 } else { 0.24 }) }

/// A round push button, a cylinder seen a little from above, flat colours (owner, 2026-10-24: no shading, shaded it looked like
/// a ball): a dark grey side wall and a flat top in `fill`, told apart by one thin light edge. Pressed, the top drops onto the base
/// and darkens. `hot` (0..1) brightens the top under the pointer.
pub fn cylinder_shapes(c: Pos2, r: f32, fill: Color32, down: bool, hot: f32) -> Shape {
    let base = c + vec2(0.0, r * 0.1);
    let top = cylinder_top(c, r, down);
    let side = Color32::from_gray(if down { 30 } else { 44 });
    let face = if down { fill.lerp_to_gamma(Color32::BLACK, 0.4) } else { lit(fill, 0.25 * hot) };
    Shape::Vec(vec![
        Shape::circle_filled(base + vec2(0.0, if down { 0.5 } else { 1.5 }), r, Color32::from_black_alpha(200)),
        Shape::circle_filled(base, r, side),
        Shape::rect_filled(Rect::from_min_max(pos2(c.x - r, top.y), pos2(c.x + r, base.y)), 0.0, side),
        Shape::circle_filled(top, r, face),
        Shape::circle_stroke(top, r, Stroke::new(1.0, Color32::from_white_alpha(46))),
    ])
}

pub fn key(ui: &Ui, r: Rect, fill: Color32, down: bool) { ui.painter().add(key_shapes(r, fill, down, 1.0)); }

/// A slightly raised face (the selected and hovered sidebar rows): a flat fill and one light/dark bevel, as the first GUI's
/// selection had it. `amount` (0..1) fades it in.
pub fn plate_shapes(r: Rect, fill: Color32, amount: f32) -> Shape {
    let f = |c: Color32| c.gamma_multiply(amount);
    let mut shapes = vec![Shape::rect_filled(r, 0.0, f(fill))];
    shapes.extend(edges(r, f(HI), f(LO)));
    Shape::Vec(shapes)
}

/// A sunken well (the systems list, the adapter box): the surface, an outline, and two bevels the other way round from a key.
pub fn well_shapes(r: Rect) -> Shape {
    let mut shapes = vec![Shape::rect_filled(r, 0.0, BACKGROUND), Shape::rect_stroke(r, 0.0, Stroke::new(1.0, OUTLINE), egui::StrokeKind::Inside)];
    shapes.extend(edges(r.shrink(1.0), LO, HI));
    shapes.extend(edges(r.shrink(2.0), LO, HI));
    Shape::Vec(shapes)
}

/// True while the last input was the keyboard. A click also focuses a control, but only keyboard use shows the ring.
pub fn track_input(ctx: &egui::Context) {
    let id = egui::Id::new("keyboard-focus");
    let (key, pointer) = ctx.input(|i| (i.events.iter().any(|e| matches!(e, egui::Event::Key { pressed: true, .. })), i.pointer.any_pressed()));
    if key { ctx.data_mut(|d| d.insert_temp(id, true)); } else if pointer { ctx.data_mut(|d| d.insert_temp(id, false)); }
}

/// The ring around a control that has keyboard focus (HIG Accessibility: controls reachable and visible by keyboard).
pub fn focus_ring(ui: &Ui, r: Rect, radius: u8, resp: &Response) {
    if resp.has_focus() && ui.data(|d| d.get_temp::<bool>(egui::Id::new("keyboard-focus"))).unwrap_or(false) {
        ui.painter().rect_stroke(r.expand(2.0), radius.saturating_add(2), Stroke::new(2.0, BLUE_HOT), egui::StrokeKind::Outside);
    }
}

/// The frame every card and window sits in: an outlined pane (see `outlined`) with `margin` inside it. The pane is
/// painted into a slot reserved below the content, once the body has said how big it is.
pub fn card<R>(ui: &mut Ui, margin: Margin, body: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    let under = ui.painter().add(Shape::Noop);
    let one = |m: i8| m.saturating_add(1);
    let out = egui::Frame::NONE.inner_margin(Margin { left: one(margin.left), right: one(margin.right), top: one(margin.top), bottom: one(margin.bottom) }).show(ui, body);
    ui.painter().set(under, outlined(ui.ctx(), out.response.rect, RADIUS_CARD, CARD, OUTLINE));
    out
}

pub fn window<R>(ui: &mut Ui, title: &str, note: &str, idle: bool, body: impl FnOnce(&mut Ui) -> R) -> R {
    window_with(ui, title, note, idle, |_| {}, body)
}

/// A window whose title bar also carries actions on the right (drawn right to left).
pub fn window_with<R>(ui: &mut Ui, title: &str, note: &str, idle: bool, action: impl FnOnce(&mut Ui), body: impl FnOnce(&mut Ui) -> R) -> R {
    let w = ui.available_width();
    card(ui, Margin::ZERO, |ui| {
        ui.set_width(w - 2.0);
        ui.spacing_mut().item_spacing.y = 0.0;
        let (bar, _) = ui.allocate_exact_size(vec2(w - 2.0, 28.0), Sense::hover());
        // the title bar is a flat strip: blue when the window has something to say, a lighter grey when idle
        let tint = if idle { SECONDARY } else { ACCENT };
        let r = (RADIUS_CARD as f32 - 1.0).max(0.0);
        ui.painter().add(poly(squircle(bar, [r, r, 0.0, 0.0]), tint));
        ui.painter().line_segment([bar.left_bottom(), bar.right_bottom()], Stroke::new(1.0, BORDER));
        let t = ui.painter().text(bar.left_center() + vec2(10.0, 0.0), Align2::LEFT_CENTER, title, bold(13.0), Color32::WHITE);
        if !note.is_empty() {
            ui.painter().text(pos2(t.right() + 12.0, bar.center().y), Align2::LEFT_CENTER, note, sans(12.0),
                Color32::from_white_alpha(170));
        }
        // a detached child: drawing the actions must not move the layout cursor of the content below
        let mut bar_ui = ui.new_child(egui::UiBuilder::new().max_rect(bar.shrink2(vec2(8.0, 0.0))).layout(egui::Layout::right_to_left(egui::Align::Center)));
        bar_ui.set_clip_rect(bar);
        bar_ui.spacing_mut().item_spacing.x = 6.0;
        action(&mut bar_ui);
        ui.spacing_mut().item_spacing.y = 6.0;
        body(ui)
    }).inner
}

pub enum Kind { Normal, Primary }

/// A button. Returns the click response.
pub fn button(ui: &mut Ui, text: &str, kind: Kind) -> Response { button_x(ui, None, text, kind, true) }

/// A button with an optional icon (a Phosphor glyph) before its text. Disabled, it is dimmed and does not
/// react; it stays visible so the user can see the action exists.
pub fn button_x(ui: &mut Ui, icon: Option<&str>, text: &str, kind: Kind, enabled: bool) -> Response {
    let full = match icon { Some(i) => format!("{i}  {text}"), None => text.to_string() };
    let g = ui.painter().layout_no_wrap(full, sans(13.0), FOREGROUND);
    let size = vec2(g.size().x + 26.0, 28.0);
    let (r, resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    let resp = if enabled { resp.on_hover_cursor(CursorIcon::PointingHand) } else { resp };
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, text));
    let down = enabled && resp.is_pointer_button_down_on();
    let hot = crate::motion::fade(ui.ctx(), resp.id.with("hot"), enabled && resp.hovered());
    let fill = match kind {
        Kind::Primary => crate::motion::mix(BLUE, BLUE_HOT, hot),
        Kind::Normal => crate::motion::mix(SECONDARY, HOVER, hot),
    };
    if ui.is_rect_visible(r) {
        let a = if enabled { 1.0 } else { 0.45 };
        key(ui, r, fill.lerp_to_gamma(CARD, 1.0 - a), down);   // disabled: faded towards the card, still opaque
        focus_ring(ui, r, RADIUS_CONTROL, &resp);
        let off = if down { vec2(0.0, 1.0) } else { vec2(0.0, 0.0) };
        let col = if matches!(kind, Kind::Primary) { Color32::WHITE } else { FOREGROUND };
        ui.painter().galley(r.center() - g.size() / 2.0 + off, g, col.gamma_multiply(a));
    }
    resp
}

/// Small label above a value: bold, uppercase, tracked out a little.
pub fn label(ui: &mut Ui, text: &str) {
    ui.label(egui::RichText::new(text.to_uppercase()).font(bold(11.0)).color(MUTED_FG));
}

/// A condition as the backend words it, "engine not running (HDS EngNotRun: engine speed 0, ignition on)": the plain words
/// first, the vendor detail muted beside them. Text with no parenthesis is shown as it is.
pub fn condition_line(ui: &mut Ui, text: &str) {
    let (plain, detail) = split_detail(text);
    ui.horizontal_wrapped(|ui| {
        ui.label(egui::RichText::new(format!("• {plain}")).size(13.0));
        if let Some(d) = detail { ui.label(egui::RichText::new(d).size(11.0).color(MUTED_FG)); }
    });
}

/// `("engine not running", Some("HDS EngNotRun: engine speed 0, ignition on"))` for the example above.
fn split_detail(text: &str) -> (&str, Option<&str>) {
    match text.split_once(" (").and_then(|(p, rest)| rest.strip_suffix(')').map(|d| (p, d))) {
        Some((p, d)) => (p, Some(d)),
        None => (text, None),
    }
}

/// A plain table: header row, then rows of text cells. Column widths are shares of the width. Returns the clicked row.
pub fn table(ui: &mut Ui, id: &str, heads: &[&str], shares: &[f32], rows: &[Vec<(String, Color32)>], selected: Option<usize>) -> Option<usize> {
    let w = ui.available_width();
    let total: f32 = shares.iter().sum();
    let xs: Vec<f32> = shares.iter().scan(0.0, |a, s| { let x = *a; *a += s / total * w; Some(x) }).collect();
    let mut clicked = None;
    let (h, _) = ui.allocate_exact_size(vec2(w, 28.0), Sense::hover());
    for (x, t) in xs.iter().zip(heads) { ui.painter().text(pos2(h.left() + x + 14.0, h.center().y), Align2::LEFT_CENTER, *t, sans(12.0), MUTED_FG); }
    ui.painter().line_segment([h.left_bottom(), h.right_bottom()], Stroke::new(1.0, BORDER));
    for (n, row) in rows.iter().enumerate() {
        let (r, resp) = ui.allocate_exact_size(vec2(w, 32.0), Sense::click());
        if !ui.is_rect_visible(r) { continue; }   // a row scrolled out of view keeps its place and is not laid out
        let resp = resp.on_hover_cursor(CursorIcon::PointingHand);
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, row.iter().map(|c| c.0.as_str()).collect::<Vec<_>>().join(", ")));
        focus_ring(ui, r, 0, &resp);
        let hot = crate::motion::fade(ui.ctx(), resp.id.with("hot"), resp.hovered());
        if selected == Some(n) { ui.painter().rect_filled(r, 0.0, ACCENT.gamma_multiply(0.6)); } else if hot > 0.0 { ui.painter().rect_filled(r, 0.0, SECONDARY.gamma_multiply(hot)); }
        for (i, (x, (t, c))) in xs.iter().zip(row).enumerate() {
            let font = if i == 0 && t.chars().next().is_some_and(|c| c.is_ascii_uppercase()) && t.len() <= 9 && id.contains("codes") { mono(13.0) } else { sans(13.0) };
            let clip = ui.painter().with_clip_rect(Rect::from_min_size(pos2(r.left() + x, r.top()), vec2(shares[i] / total * w - 6.0, r.height())));
            clip.text(pos2(r.left() + x + 14.0, r.center().y), Align2::LEFT_CENTER, t, font, *c);
        }
        ui.painter().line_segment([r.left_bottom(), r.right_bottom()], Stroke::new(1.0, BORDER));
        if resp.clicked() { clicked = Some(n); }
    }
    clicked
}

/// A number with its unit after it, the unit sitting exactly on the number's baseline (the font's own metrics: a smaller
/// text aligned to the bottom of the line would sit below the digits' feet). `left_center`: where the number's left edge
/// and vertical middle go. Returns the rectangle of the number and its unit together.
pub fn amount(p: &Painter, left_center: Pos2, text: &str, font: FontId, color: Color32, unit: &str, unit_font: FontId) -> Rect {
    let baseline_of = |g: &egui::Galley| g.rows.first().and_then(|r| r.row.glyphs.first()).map_or(g.size().y * 0.8, |gl| gl.pos.y);
    let g = p.layout_no_wrap(text.to_string(), font, color);
    let top = left_center.y - g.size().y / 2.0;
    let (w, base) = (g.size().x, baseline_of(&g));
    let mut rect = Rect::from_min_size(pos2(left_center.x, top), g.size());
    p.galley(rect.min, g, color);
    if !unit.is_empty() {
        let u = p.layout_no_wrap(unit.to_string(), unit_font, MUTED_FG);
        let at = pos2(left_center.x + w + 5.0, top + base - baseline_of(&u));
        rect = rect.union(Rect::from_min_size(at, u.size()));
        p.galley(at, u, MUTED_FG);
    }
    rect
}

#[cfg(test)]
mod tests {
    fn area(pts: &[eframe::egui::Pos2]) -> f32 {
        (0..pts.len()).map(|i| { let (a, b) = (pts[i], pts[(i + 1) % pts.len()]); a.x * b.y - b.x * a.y }).sum::<f32>() / 2.0
    }

    #[test]
    fn a_squircle_stays_inside_its_rectangle_runs_clockwise_and_touches_all_four_edges() {
        use eframe::egui::{pos2, Rect};
        let r = Rect::from_min_max(pos2(10.0, 20.0), pos2(210.0, 120.0));
        let pts = super::squircle(r, [10.0; 4]);
        assert!(pts.iter().all(|p| r.expand(0.01).contains(*p)), "a point left the rectangle");
        assert!(area(&pts) > 0.0, "on screen (y down) a clockwise outline has a positive shoelace area");
        let (l, rt) = (pts.iter().any(|p| (p.x - r.left()).abs() < 0.01), pts.iter().any(|p| (p.x - r.right()).abs() < 0.01));
        let (t, b) = (pts.iter().any(|p| (p.y - r.top()).abs() < 0.01), pts.iter().any(|p| (p.y - r.bottom()).abs() < 0.01));
        assert!(l && rt && t && b);
        assert!(area(&pts) < r.area() && area(&pts) > r.area() * 0.97, "corners cut a little, not much: {}", area(&pts) / r.area());
        assert_eq!(super::squircle(r, [0.0; 4]).len(), 4, "radius 0 is the plain rectangle");
    }

    #[test]
    fn a_band_keeps_only_its_rows() {
        use eframe::egui::{pos2, Rect};
        let pts = super::squircle(Rect::from_min_max(pos2(0.0, 0.0), pos2(100.0, 100.0)), [12.0; 4]);
        let b = super::band(&pts, 30.0, 50.0);
        assert!(b.iter().all(|p| p.y >= 29.999 && p.y <= 50.001));
        assert!(b.iter().any(|p| (p.y - 30.0).abs() < 0.01) && b.iter().any(|p| (p.y - 50.0).abs() < 0.01));
        assert!(super::band(&pts, 200.0, 300.0).is_empty(), "a band outside the shape is empty");
    }

    /// A custom-painted button reaches the accessibility tree with its text, so a screen reader can name it.
    #[test]
    fn a_button_is_named_for_assistive_technology() {
        let ctx = eframe::egui::Context::default();
        ctx.enable_accesskit();
        let input = eframe::egui::RawInput { screen_rect: Some(eframe::egui::Rect::from_min_size(eframe::egui::Pos2::ZERO, eframe::egui::vec2(400.0, 200.0))), ..Default::default() };
        let out = ctx.run_ui(input, |ui| { super::button(ui, "Connect", super::Kind::Primary); });
        let update = out.platform_output.accesskit_update.expect("accesskit is on, so the frame carries a tree");
        assert!(update.nodes.iter().any(|(_, n)| n.label() == Some("Connect")), "no node is labelled Connect");
    }

    #[test]
    fn a_condition_splits_into_plain_words_and_vendor_detail() {
        assert_eq!(super::split_detail("engine not running (HDS EngNotRun: engine speed 0, ignition on)"),
            ("engine not running", Some("HDS EngNotRun: engine speed 0, ignition on")));
        assert_eq!(super::split_detail("vehicle speed 0"), ("vehicle speed 0", None));
        assert_eq!(super::split_detail("battery 10.5-20 V (HDS BattVltgChk 0x2904-0x4E20 mV)").0, "battery 10.5-20 V");
        assert_eq!(super::split_detail("an open (bracket"), ("an open (bracket", None), "no closing bracket, so no split");
    }
}
