//! The Overview's car, seen straight down, nose to the right: the hologram model's wires (`carmodel.rs`, drawn by
//! `turntable::wire_callback` with an orthographic matrix) over a dark body outline, and every found system a button at its
//! place on it: its icon in the colour of its trouble-code state, with a count; the brakes' system stands inside the front
//! wheel at the driver's side and the tire pressure's above it. The body is the outline of the 2013-2017 Accord traced from a line blueprint for the first web app
//! (hondascan-app/src/components/CarBody.tsx), its SVG path parsed and flattened here; blueprint pixel space 1388 x 592,
//! scaled into a 1000 x 440 frame.
use crate::app::{App, View};
use crate::systems::{info, zone};
use crate::theme::*;
use eframe::egui::{self, pos2, vec2, Align2, Color32, Pos2, Rect, Sense, Shape, Stroke, Ui};

const BODY: &str = "M2 296 C2 205 26 118 110 74 C160 52 230 42 300 40 L1150 37 C1250 40 1318 78 1356 150 C1376 196 1386 250 1386 296 C1386 342 1376 396 1356 442 C1318 514 1250 552 1150 555 L300 552 C230 550 160 540 110 518 C26 474 2 387 2 296 Z";

/// The systems that sit by a wheel: (zone, wheel x, wheel y in frame units, where the wire model's wheel is; which side of
/// the wheel the button stands, 1 toward the middle of the car, -1 outside it). Both are at the front wheel on the driver's
/// side (up on the screen): the brakes' system inside it, the tire pressure's above it.
const AXLES: [(&str, f32, f32, f32); 2] = [("brakes", 771.0, 96.0, 1.0), ("tires", 771.0, 96.0, -1.0)];
const WHEEL: (f32, f32) = (124.0, 40.0);

/// Wire brightness: half the hologram's, so the buttons stay the brightest things on the card.
const GAIN: f32 = 0.5;

/// SVG path data (absolute M L C, closed by Z) as one flattened polyline.
fn parse(d: &str) -> Vec<Pos2> {
    let spaced = d.replace('M', " M ").replace('L', " L ").replace('C', " C ").replace('Z', " Z ");
    let mut toks = spaced.split_whitespace();
    let mut out: Vec<Pos2> = Vec::new();
    while let Some(cmd) = toks.next() {
        let mut v = Vec::new();
        for _ in 0..match cmd { "M" | "L" => 2, "C" => 6, _ => 0 } { v.push(toks.next().and_then(|t| t.parse::<f32>().ok()).unwrap_or(0.0)); }
        match cmd {
            "M" | "L" => out.push(pos2(v[0], v[1])),
            "C" => {
                let p0 = *out.last().unwrap();
                let (p1, p2, p3) = (pos2(v[0], v[1]), pos2(v[2], v[3]), pos2(v[4], v[5]));
                for i in 1..=14 {
                    let s = i as f32 / 14.0; let u = 1.0 - s;
                    out.push((p0.to_vec2() * (u * u * u) + p1.to_vec2() * (3.0 * u * u * s) + p2.to_vec2() * (3.0 * u * s * s) + p3.to_vec2() * (s * s * s)).to_pos2());
                }
            }
            _ => {}
        }
    }
    out
}

pub fn show(ui: &mut Ui, app: &mut App) {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(w, w * 468.0 / 1000.0), Sense::hover());
    let sc = rect.width() / 1000.0;
    let at = |x: f32, y: f32| pos2(rect.left() + x * sc, rect.top() + y * sc);
    // blueprint -> frame (CarBody's matrix) -> screen (viewBox 0 -14 1000 468)
    let outline: Vec<Pos2> = parse(BODY).iter().map(|p| at(50.0 + 0.6484 * p.x, 28.4 + 0.6484 * p.y + 14.0)).collect();
    let p = ui.painter();
    p.add(Shape::convex_polygon(outline, CARD,Stroke::new(1.4, Color32::from_white_alpha(90))));
    wire_top_down(ui, rect);
    let mut open: Option<String> = None;
    let mut names: Vec<(Pos2, String)> = Vec::new();
    wheels(ui, app, rect, sc, &mut open, &mut names);
    pins(ui, app, rect, sc, &mut open, &mut names);
    // a name shows under the pointer or beside the open system, over everything else
    for (c, nm) in names {
        let g = ui.painter().layout_no_wrap(nm, sans(12.5), FOREGROUND);
        let r = Rect::from_center_size(c, g.size() + vec2(14.0, 8.0));
        ui.painter().add(crate::widgets::well_shapes(r));
        ui.painter().galley(r.center() - g.size() / 2.0, g, FOREGROUND);
    }
    if let Some(k) = open { app.view = View::System(k); }
}

/// The hologram's wires seen straight down, nose to the right, filling the same part of the frame as the body outline (x 51..949).
/// Model metres: Y up, nose +Z, driver's left +X (up on the screen); `k` is frame units a metre.
fn wire_top_down(ui: &Ui, rect: Rect) {
    const K: f32 = 898.0 / 5.636;
    let mut m = [0.0f32; 16];
    m[8] = K / 500.0; m[1] = K / 234.0; m[6] = 0.5; m[14] = -0.85; m[15] = 1.0;
    let shared = ui.data_mut(|d| d.get_temp_mut_or_default::<std::sync::Arc<std::sync::Mutex<crate::ui::turntable::Shared>>>(egui::Id::new("car_wire")).clone());
    ui.painter().add(crate::ui::turntable::wire_callback(shared, rect, m, -10.0, GAIN * ui.opacity(), true));
}

/// The first system of a zone: the one a wheel stands for.
fn owner(app: &App, zone_id: &str) -> Option<usize> {
    (0..app.ecus.len()).find(|&i| info(&app.ecus[i].system_type, &app.ecus[i].display).zone == zone_id)
}

fn key_side(sc: f32) -> f32 { (58.0 * sc).max(46.0) }

/// One system's button at `r`: its icon in the colour of its state on a round dark disc (blue while it is open), the number
/// of codes in a corner, its name on hover.
fn key_button(ui: &mut Ui, app: &App, r: Rect, i: usize, open: &mut Option<String>, names: &mut Vec<(Pos2, String)>) {
    let e = &app.ecus[i];
    let inf = info(&e.system_type, &e.display);
    let resp = ui.interact(r, ui.id().with(("car", e.key())), Sense::click());
    let health = app.health(e);
    let col = health.color();
    let selected = matches!(&app.view, View::System(k) if *k == e.key());
    let hot = crate::motion::fade(ui.ctx(), resp.id.with("hot"), resp.hovered());
    // a push button (a cylinder): its top drops only while the click is held; the open system's is blue; the icon is in the state colour
    let down = resp.is_pointer_button_down_on();
    let (c, rad) = (r.center(), r.width() * 0.4);
    let p = ui.painter();
    p.add(crate::widgets::cylinder_shapes(c, rad, if selected { ACCENT } else { Color32::from_gray(10) }, down, hot));
    let at = crate::widgets::cylinder_top(c, rad, down);
    let size = rad * 1.0;
    p.text(at + vec2(0.0, 1.0), Align2::CENTER_CENTER, inf.icon, sans(size), Color32::from_black_alpha(190));
    p.text(at, Align2::CENTER_CENTER, inf.icon, sans(size), if selected { Color32::WHITE } else { col });
    let count = app.dtcs.get(&e.key()).and_then(|d| d.dtcs.as_ref()).map_or(0, |d| d.len());
    if count > 0 {
        let br = Rect::from_center_size(at + vec2(rad * 0.8, -rad * 0.8), vec2(18.0, 15.0));
        p.rect_filled(br, 2.0, col);
        p.text(br.center(), Align2::CENTER_CENTER, count.to_string(), mono_bold(11.0), BACKGROUND);
    }
    if resp.hovered() || selected { names.push((r.center_bottom() + vec2(0.0, 18.0), inf.name.clone())); }
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    if resp.clicked() { *open = Some(e.key()); }
}

/// The systems that stand by a wheel (`AXLES`): the button beside the wheel, not on it.
fn wheels(ui: &mut Ui, app: &App, rect: Rect, sc: f32, open: &mut Option<String>, names: &mut Vec<(Pos2, String)>) {
    for (z, x, y, inward) in AXLES {
        let Some(i) = owner(app, z) else { continue };
        let w = Rect::from_center_size(pos2(rect.left() + x * sc, rect.top() + y * sc), vec2(WHEEL.0, WHEEL.1) * sc);
        let side = key_side(sc);
        let k = Rect::from_center_size(w.center() + vec2(0.0, inward * (w.height() / 2.0 + 6.0 + side / 2.0)), vec2(side, side));
        key_button(ui, app, k, i, open, names);
    }
}

/// Every other system: a key at its place on the car. Several at one place stand side by side.
fn pins(ui: &mut Ui, app: &App, rect: Rect, sc: f32, open: &mut Option<String>, names: &mut Vec<(Pos2, String)>) {
    let mut by_zone: Vec<(String, Vec<usize>)> = Vec::new();
    for (i, e) in app.ecus.iter().enumerate() {
        let z = info(&e.system_type, &e.display).zone.to_string();
        if AXLES.iter().any(|a| a.0 == z && owner(app, a.0) == Some(i)) { continue; }
        match by_zone.iter_mut().find(|(k, _)| *k == z) { Some((_, v)) => v.push(i), None => by_zone.push((z, vec![i])) }
    }
    let side = key_side(sc);
    for (z, list) in &by_zone {
        let (zx, zy, _) = zone(z);
        for (n, &i) in list.iter().enumerate() {
            let c = pos2(rect.left() + (zx + (n as f32 - (list.len() as f32 - 1.0) / 2.0) * 62.0) * sc, rect.top() + (zy + 14.0) * sc);
            key_button(ui, app, Rect::from_center_size(c, vec2(side, side)), i, open, names);
        }
    }
}
