//! The Overview's car: the hologram model's wires seen straight down, nose to the right, in plain light grey over a dark
//! silhouette, with every found system pinned at its place and coloured by its trouble-code state. The wires are the welcome
//! screen's (`carmodel.rs`, drawn by `turntable::wire_callback` with an orthographic matrix, so the one GPU buffer serves both);
//! the silhouette is the outline of the 2013-2017 Accord traced from a line blueprint for the first web app
//! (hondascan-app/src/components/CarBody.tsx), its SVG path parsed and flattened here. Blueprint pixel space 1388 x 592,
//! scaled into a 1000 x 440 frame. Labels sit on the card's top and bottom edges with a leader line to their pin, spaced so
//! none overlap.
use crate::app::{App, Health, View};
use crate::systems::{info, zone};
use crate::theme::*;
use eframe::egui::{self, pos2, vec2, Align2, Color32, Pos2, Rect, Sense, Shape, Stroke, Ui};

const BODY: &str = "M2 296 C2 205 26 118 110 74 C160 52 230 42 300 40 L1150 37 C1250 40 1318 78 1356 150 C1376 196 1386 250 1386 296 C1386 342 1376 396 1356 442 C1318 514 1250 552 1150 555 L300 552 C230 550 160 540 110 518 C26 474 2 387 2 296 Z";

/// Wire brightness: half the hologram's, so the pins and labels stay the brightest things on the card.
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
    // blueprint -> frame (CarBody's matrix) -> screen (viewBox 0 -14 1000 468)
    let outline: Vec<Pos2> = parse(BODY).iter().map(|p| pos2(rect.left() + (50.0 + 0.6484 * p.x) * sc, rect.top() + (28.4 + 0.6484 * p.y + 14.0) * sc)).collect();
    ui.painter().add(Shape::convex_polygon(outline, Color32::from_rgb(13, 16, 21), Stroke::new(1.4, Color32::from_white_alpha(90))));
    wire_top_down(ui, rect);
    systems(ui, app, rect, sc);
}

/// The hologram's wires seen straight down, nose to the right, filling the same part of the frame as the vector car (x 51..949).
/// Model metres: Y up, nose +Z, driver's left +X (up on the screen); `k` is frame units a metre.
fn wire_top_down(ui: &Ui, rect: Rect) {
    const K: f32 = 898.0 / 5.636;
    let mut m = [0.0f32; 16];
    m[8] = K / 500.0; m[1] = K / 234.0; m[6] = 0.5; m[14] = -0.85; m[15] = 1.0;
    let shared = ui.data_mut(|d| d.get_temp_mut_or_default::<std::sync::Arc<std::sync::Mutex<crate::ui::turntable::Shared>>>(egui::Id::new("car_wire")).clone());
    ui.painter().add(crate::ui::turntable::wire_callback(shared, rect, m, -10.0, GAIN * ui.opacity(), true));
}

/// Where each label sits when it is pulled off the car: sorted by x along the top or bottom edge, kept `gap` frame units apart (a label's width on screen).
fn label_slots(pins: &[(usize, f32)], gap: f32) -> Vec<f32> {
    let mut xs: Vec<f32> = pins.iter().map(|p| p.1).collect();
    for i in 1..xs.len() { xs[i] = xs[i].max(xs[i - 1] + gap); }
    if let Some(&last) = xs.last() { let over = (last - 940.0).max(0.0); xs.iter_mut().for_each(|x| *x -= over); }
    for i in (0..xs.len().saturating_sub(1)).rev() { xs[i] = xs[i].min(xs[i + 1] - gap); }
    xs
}

fn systems(ui: &mut Ui, app: &mut App, rect: Rect, sc: f32) {
    // systems, side by side where several share a place
    let mut by_zone: Vec<(String, Vec<usize>)> = Vec::new();
    for (i, e) in app.ecus.iter().enumerate() {
        let z = info(&e.system_type, &e.display).zone.to_string();
        match by_zone.iter_mut().find(|(k, _)| *k == z) { Some((_, v)) => v.push(i), None => by_zone.push((z, vec![i])) }
    }
    // (ecu index, x, y in frame units, label above?)
    let mut pins: Vec<(usize, f32, f32, bool)> = Vec::new();
    for (z, list) in &by_zone {
        let (zx, zy, _) = zone(z);
        for (n, &i) in list.iter().enumerate() {
            pins.push((i, zx + (n as f32 - (list.len() as f32 - 1.0) / 2.0) * 62.0, zy + 14.0, zy + 14.0 < 234.0));
        }
    }
    let mut label_at: Vec<(usize, f32)> = Vec::new();   // where each label goes along its edge
    for above in [true, false] {
        let mut side: Vec<(usize, f32)> = pins.iter().filter(|p| p.3 == above).map(|p| (p.0, p.1)).collect();
        side.sort_by(|a, b| a.1.total_cmp(&b.1));
        let xs = label_slots(&side, 150.0 / sc);
        label_at.extend(side.iter().zip(xs).map(|(s, x)| (s.0, x)));
    }
    let mut open: Option<String> = None;
    for &(i, px, py, above) in &pins {
        let e = &app.ecus[i];
        let c = pos2(rect.left() + px * sc, rect.top() + py * sc);
        let r = 17.0 * sc;
        let resp = ui.interact(Rect::from_center_size(c, vec2(r * 2.4, r * 2.4)), ui.id().with(("car", e.key())), Sense::click());
        let health = app.health(e);
        let col = health.color();
        let selected = matches!(&app.view, View::System(k) if *k == e.key());
        let nm = info(&e.system_type, &e.display).name;
        let hot = crate::motion::fade(ui.ctx(), resp.id.with("hot"), resp.hovered());
        let ink = crate::motion::mix(MUTED_FG, FOREGROUND, if selected { 1.0 } else { hot });
        let r = r * (1.0 + 0.1 * hot);   // the pin swells a little under the pointer
        let lx = label_at.iter().find(|l| l.0 == i).map_or(px, |l| l.1);
        let row = rect.top() + (if above { 22.0 } else { 446.0 }) * sc;
        let end = pos2(rect.left() + lx * sc, row + if above { 12.0 } else { -12.0 });
        ui.painter().line_segment([c + vec2(0.0, if above { -r } else { r }), end], Stroke::new(1.0, col.gamma_multiply(0.55)));
        ui.painter().circle(c, r, crate::motion::mix(CARD, ACCENT, hot), Stroke::new(if selected { 2.5 } else { 1.5 }, col));
        ui.painter().text(c, Align2::CENTER_CENTER, info(&e.system_type, &e.display).icon, sans((r * 1.05).max(14.0)), if matches!(health, Health::Ok | Health::Unknown) { FOREGROUND } else { col });
        let count = app.dtcs.get(&e.key()).and_then(|d| d.dtcs.as_ref()).map_or(0, |d| d.len());
        if count > 0 {
            let br = Rect::from_center_size(c + vec2(r * 0.85, -r * 0.85), vec2(18.0 * sc.max(0.8), 15.0 * sc.max(0.8)));
            ui.painter().rect_filled(br, 2.0, col);
            ui.painter().text(br.center(), Align2::CENTER_CENTER, count.to_string(), mono_bold(11.0), BACKGROUND);
        }
        ui.painter().text(pos2(rect.left() + lx * sc, row), Align2::CENTER_CENTER, &nm, sans(12.5), ink);
        let resp = resp.on_hover_text(format!("{nm} · {} (0x{:X})", e.display, e.address)).on_hover_cursor(egui::CursorIcon::PointingHand);
        if resp.clicked() { open = Some(e.key()); }
    }
    if let Some(k) = open { app.view = View::System(k); }
}
