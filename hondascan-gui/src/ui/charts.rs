//! Small charts drawn straight on the GPU: a sparkline for history and an arc gauge for values with a natural range
//! (ported from hondascan-app/src/components/charts.tsx).
use crate::ranges::Tone;
use crate::theme::*;
use eframe::egui::{pos2, vec2, Align2, Color32, Pos2, Rect, Shape, Stroke, Ui};
use std::collections::VecDeque;

pub fn tone_color(t: Tone) -> Color32 {
    match t { Tone::Data => DATA, Tone::Warn => WARN, Tone::Fault => FAULT }
}

/// A steady value is a line through the middle, not along the bottom edge.
pub fn sparkline(ui: &Ui, r: Rect, values: &VecDeque<f32>) {
    if values.len() < 2 { return; }
    let (lo, hi) = values.iter().fold((f32::MAX, f32::MIN), |(a, b), v| (a.min(*v), b.max(*v)));
    let span = hi - lo;
    let n = (values.len() - 1) as f32;
    let pts: Vec<Pos2> = values.iter().enumerate().map(|(i, v)| {
        let y = if span > 0.0 { 0.93 - (v - lo) / span * 0.87 } else { 0.5 };
        pos2(r.left() + i as f32 / n * r.width(), r.top() + y * r.height())
    }).collect();
    ui.painter().add(Shape::line(pts, Stroke::new(1.25, Color32::from_white_alpha(150))));
}

fn arc(c: Pos2, r: f32, from: f32, to: f32) -> Vec<Pos2> {
    let steps = (((to - from).abs() / 4.0).ceil() as usize).max(2);
    (0..=steps).map(|i| { let a = (from + (to - from) * i as f32 / steps as f32).to_radians(); c + vec2(a.cos(), a.sin()) * r }).collect()
}

/// A 270 degree arc, filled to `frac` (0..1), with the value in the middle and the name under it; out of range, its icon above the value.
pub fn gauge(ui: &Ui, r: Rect, frac: Option<f32>, text: &str, unit: &str, label: &str, tone: Tone) {
    let (start, sweep) = (135.0f32, 270.0f32);
    let size = r.width().min(r.height());
    let c = r.center() - vec2(0.0, size * 0.02);
    let rad = size * 0.42;
    let col = tone_color(tone);
    ui.painter().add(Shape::line(arc(c, rad, start, start + sweep), Stroke::new(size * 0.03, Color32::from_white_alpha(24))));
    if let Some(f) = frac {
        let f = f.clamp(0.0, 1.0);
        if f > 0.004 { ui.painter().add(Shape::line(arc(c, rad, start, start + sweep * f), Stroke::new(size * 0.03, col))); }
    }
    for i in 0..=10 {
        let a = (start + sweep * i as f32 / 10.0).to_radians();
        let d = vec2(a.cos(), a.sin());
        ui.painter().line_segment([c + d * rad * 0.79, c + d * rad * 0.845], Stroke::new(1.0, Color32::from_white_alpha(50)));
    }
    // a warning or a fault also shows as its icon, never by the arc's colour alone (HIG Accessibility)
    if let Some(icon) = status_icon(col) { ui.painter().text(c - vec2(0.0, size * 0.29), Align2::CENTER_CENTER, icon, sans(14.0), col); }
    ui.painter().text(c - vec2(0.0, size * 0.125), Align2::CENTER_CENTER, text, bold((size * 0.17).clamp(16.0, 30.0)), FOREGROUND);
    ui.painter().text(c + vec2(0.0, size * 0.025), Align2::CENTER_CENTER, unit, sans(12.0), MUTED_FG);
    ui.painter().text(pos2(r.center().x, r.bottom() - 8.0), Align2::CENTER_CENTER, label, sans(11.0), MUTED_FG);
}
