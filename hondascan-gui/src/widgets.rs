//! The few pieces the whole UI is built from: bevels, windows with a title bar, bevelled buttons.
use crate::theme::*;
use eframe::egui::{self, epaint::Vertex, pos2, vec2, Align2, Color32, CursorIcon, FontId, Mesh, Painter, Pos2, Rect, Response, Sense, Shape, Stroke, Ui};

/// A light edge top-left and a dark one bottom-right (raised), or the other way round (sunken).
pub fn bevel(ui: &Ui, r: Rect, raised: bool) { bevel_a(ui, r, raised, 1.0) }

/// `bevel`, faded to `alpha` (a selection fading in or out).
pub fn bevel_a(ui: &Ui, r: Rect, raised: bool, alpha: f32) {
    let (a, b) = if raised { (HI, LO) } else { (LO, HI) };
    edges(ui.painter(), r, a.gamma_multiply(alpha), b.gamma_multiply(alpha));
}

/// A one-pixel edge inside `r`: `tl` along the top and left, `br` along the bottom and right.
fn edges(p: &Painter, r: Rect, tl: Color32, br: Color32) {
    let (lt, rt, lb, rb) = (r.left_top(), r.right_top(), r.left_bottom(), r.right_bottom());
    p.line_segment([lt + vec2(0.5, 0.5), rt + vec2(-0.5, 0.5)], Stroke::new(1.0, tl));
    p.line_segment([lt + vec2(0.5, 0.5), lb + vec2(0.5, -0.5)], Stroke::new(1.0, tl));
    p.line_segment([lb + vec2(0.5, -0.5), rb + vec2(-0.5, -0.5)], Stroke::new(1.0, br));
    p.line_segment([rt + vec2(-0.5, 0.5), rb + vec2(-0.5, -0.5)], Stroke::new(1.0, br));
}

/// A top-to-bottom gradient.
pub fn vgradient(ui: &Ui, r: Rect, top: Color32, bottom: Color32) { quad(ui, r, [top, top, bottom, bottom]); }

/// A rectangle with a colour at each corner: top-left, top-right, bottom-right, bottom-left.
fn quad(ui: &Ui, r: Rect, colors: [Color32; 4]) {
    let mut m = Mesh::default();
    for (pos, color) in [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()].into_iter().zip(colors) {
        m.vertices.push(Vertex { pos, uv: egui::epaint::WHITE_UV, color });
    }
    m.indices.extend([0, 1, 2, 0, 2, 3]);
    ui.painter().add(Shape::mesh(m));
}

/// A raised key: lit from above (lighter top, darker bottom), black outline, a light edge top-left and a dark one
/// bottom-right, then a softer second pair inside. Pressed, it flips and sinks one pixel.
pub fn key(ui: &Ui, r: Rect, fill: Color32, down: bool) {
    let (top, bot) = if down { (fill.gamma_multiply(0.82), fill.gamma_multiply(0.95)) } else { (lighten(fill, 0.16), fill.gamma_multiply(0.8)) };
    vgradient(ui, r, top, bot);
    ui.painter().rect_stroke(r, 0.0, Stroke::new(1.0, Color32::BLACK), egui::StrokeKind::Inside);
    bevel(ui, r.shrink(1.0), !down);
    bevel(ui, r.shrink(2.0), !down);
}

fn lighten(c: Color32, t: f32) -> Color32 {
    let f = |x: u8| (x as f32 + (255.0 - x as f32) * t) as u8;
    Color32::from_rgb(f(c.r()), f(c.g()), f(c.b()))
}

/// A left-to-right gradient.
pub fn gradient(ui: &Ui, r: Rect, a: Color32, b: Color32) { quad(ui, r, [a, b, b, a]); }

/// A window: dark face, black edge, and a title bar fading from blue (`idle`: neutral grey) into the panel colour.
/// The frame every card and window sits in: the card colour with a one-pixel black edge.
pub fn card() -> egui::Frame { egui::Frame::NONE.fill(CARD).stroke(Stroke::new(1.0, Color32::BLACK)) }

pub fn window<R>(ui: &mut Ui, title: &str, note: &str, idle: bool, body: impl FnOnce(&mut Ui) -> R) -> R {
    window_with(ui, title, note, idle, |_| {}, body)
}

/// A window whose title bar also carries actions on the right (drawn right to left).
pub fn window_with<R>(ui: &mut Ui, title: &str, note: &str, idle: bool, action: impl FnOnce(&mut Ui), body: impl FnOnce(&mut Ui) -> R) -> R {
    let w = ui.available_width();
    card().show(ui, |ui| {
        ui.set_width(w - 2.0);
        ui.spacing_mut().item_spacing.y = 0.0;
        let (bar, _) = ui.allocate_exact_size(vec2(w - 2.0, 28.0), Sense::hover());
        let (a, b) = if idle { (BORDER, PANEL) } else { (TITLE_A, TITLE_B) };
        gradient(ui, bar, a, b);
        ui.painter().line_segment([bar.left_top(), bar.right_top()], Stroke::new(1.0, HI));
        ui.painter().line_segment([bar.left_bottom(), bar.right_bottom()], Stroke::new(1.0, LO));
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

/// A bevelled button. Returns the click response.
pub fn button(ui: &mut Ui, text: &str, kind: Kind) -> Response { button_x(ui, None, text, kind, true) }

/// A bevelled button with an optional icon (a Phosphor glyph) before its text. Disabled, it is dimmed and does not
/// react; it stays visible so the user can see the action exists.
pub fn button_x(ui: &mut Ui, icon: Option<&str>, text: &str, kind: Kind, enabled: bool) -> Response {
    let full = match icon { Some(i) => format!("{i}  {text}"), None => text.to_string() };
    let g = ui.painter().layout_no_wrap(full, sans(13.0), FOREGROUND);
    let size = vec2(g.size().x + 26.0, 28.0);
    let (r, resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    let resp = if enabled { resp.on_hover_cursor(CursorIcon::PointingHand) } else { resp };
    let down = enabled && resp.is_pointer_button_down_on();
    let hot = crate::motion::fade(ui.ctx(), resp.id.with("hot"), enabled && resp.hovered());
    let fill = match kind {
        Kind::Primary => crate::motion::mix(BLUE, Color32::from_rgb(45, 128, 235), hot),
        Kind::Normal => crate::motion::mix(SECONDARY, Color32::from_rgb(46, 49, 54), hot),
    };
    if ui.is_rect_visible(r) {
        let a = if enabled { 1.0 } else { 0.45 };
        key(ui, r, fill.gamma_multiply(a), down);
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
    ui.painter().line_segment([h.left_bottom(), h.right_bottom()], Stroke::new(1.0, Color32::from_white_alpha(20)));
    for (n, row) in rows.iter().enumerate() {
        let (r, resp) = ui.allocate_exact_size(vec2(w, 32.0), Sense::click());
        if !ui.is_rect_visible(r) { continue; }   // a row scrolled out of view keeps its place and is not laid out
        let resp = resp.on_hover_cursor(CursorIcon::PointingHand);
        let hot = crate::motion::fade(ui.ctx(), resp.id.with("hot"), resp.hovered());
        if selected == Some(n) { ui.painter().rect_filled(r, 0.0, ACCENT.gamma_multiply(0.6)); } else if hot > 0.0 { ui.painter().rect_filled(r, 0.0, SECONDARY.gamma_multiply(hot)); }
        for (i, (x, (t, c))) in xs.iter().zip(row).enumerate() {
            let font = if i == 0 && t.chars().next().is_some_and(|c| c.is_ascii_uppercase()) && t.len() <= 9 && id.contains("codes") { mono(13.0) } else { sans(13.0) };
            let clip = ui.painter().with_clip_rect(Rect::from_min_size(pos2(r.left() + x, r.top()), vec2(shares[i] / total * w - 6.0, r.height())));
            clip.text(pos2(r.left() + x + 14.0, r.center().y), Align2::LEFT_CENTER, t, font, *c);
        }
        ui.painter().line_segment([r.left_bottom(), r.right_bottom()], Stroke::new(1.0, Color32::from_white_alpha(12)));
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

/// A sunken well (lists, fields): a lit lip along the bottom and right outside it, a black outline, three rings inside
/// (shadow top-left, light bottom-right, each lighter the deeper it goes) and a soft shadow falling from the top and
/// left edges onto the floor of the well. Dark surfaces hide a plain dark edge, so the lit lip does the work.
pub fn sunken(ui: &Ui, r: Rect) {
    let p = ui.painter();
    let lip = Stroke::new(1.0, Color32::from_rgb(74, 79, 87));
    p.line_segment([r.left_bottom() + vec2(0.0, 1.5), r.right_bottom() + vec2(1.5, 1.5)], lip);
    p.line_segment([r.right_top() + vec2(1.5, 0.0), r.right_bottom() + vec2(1.5, 1.5)], lip);
    p.rect_stroke(r, 0.0, Stroke::new(1.0, Color32::BLACK), egui::StrokeKind::Inside);
    // the shadow on the floor: darkest at the top and left edges, gone after a few pixels
    let inner = r.shrink(1.0);
    let top = Rect::from_min_size(inner.min, vec2(inner.width(), 10.0));
    vgradient(ui, top, Color32::from_black_alpha(150), Color32::from_black_alpha(0));
    gradient(ui, Rect::from_min_size(inner.min, vec2(8.0, inner.height())), Color32::from_black_alpha(110), Color32::from_black_alpha(0));
    let rings = [
        (1.0, Color32::from_rgb(0, 0, 0), Color32::from_rgb(66, 71, 79)),
        (2.0, Color32::from_rgb(6, 6, 8), Color32::from_rgb(46, 50, 56)),
        (3.0, Color32::from_rgb(14, 15, 18), Color32::from_rgb(30, 33, 38)),
    ];
    for (ring, shadow, light) in rings {
        edges(p, r.shrink(ring), shadow, light);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_condition_splits_into_plain_words_and_vendor_detail() {
        assert_eq!(super::split_detail("engine not running (HDS EngNotRun: engine speed 0, ignition on)"),
            ("engine not running", Some("HDS EngNotRun: engine speed 0, ignition on")));
        assert_eq!(super::split_detail("vehicle speed 0"), ("vehicle speed 0", None));
        assert_eq!(super::split_detail("battery 10.5-20 V (HDS BattVltgChk 0x2904-0x4E20 mV)").0, "battery 10.5-20 V");
        assert_eq!(super::split_detail("an open (bracket"), ("an open (bracket", None), "no closing bracket, so no split");
    }
}
