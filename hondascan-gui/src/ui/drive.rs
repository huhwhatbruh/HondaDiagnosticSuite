//! This drive: distance, fuel and economy. Every number comes from hondascan.trip (ported from
//! hondascan-app/src/components/DriveCard.tsx).
use crate::api::Status;
use crate::format::{clock, fmt, GAL, MI};
use crate::theme::*;
use crate::widgets::window;
use eframe::egui::{self, vec2, Align2, Color32, RichText, Sense, Stroke, Ui};

fn stat(ui: &mut Ui, r: egui::Rect, label: &str, text: &str, unit: &str, color: Color32, hint: &str) {
    let resp = ui.interact(r, ui.id().with(label), Sense::hover());
    if !hint.is_empty() { resp.on_hover_text(hint); }
    let p = ui.painter();
    p.text(r.left_top() + vec2(16.0, 20.0), Align2::LEFT_CENTER, label, sans(12.0), MUTED_FG);
    crate::widgets::amount(p, r.left_top() + vec2(16.0, 46.0), text, bold(21.0), color, unit, sans(12.0));
    let line = Stroke::new(1.0, Color32::from_white_alpha(18));
    p.line_segment([r.right_top(), r.right_bottom()], line);
    p.line_segment([r.left_bottom(), r.right_bottom()], line);
}

pub fn show(ui: &mut Ui, status: Option<&Status>) {
    let t = status.filter(|s| s.recording).and_then(|s| s.trip.as_ref()).filter(|t| t.duration_s > 0.0);
    let Some((s, t)) = status.zip(t) else {
        window(ui, "This drive", "", false, |ui| {
            ui.add_space(8.0);
            let text = if status.is_some_and(|s| s.asleep) { "The car is off. The last drive is saved in History." } else { "Starts with the first engine readings." };
            ui.horizontal(|ui| { ui.add_space(14.0); ui.label(RichText::new(text).size(13.0).color(MUTED_FG)); });
            ui.add_space(8.0);
        });
        return;
    };
    let idle = if t.times_s.engine > 0.0 { (t.times_s.idle / t.times_s.engine * 100.0).round() } else { 0.0 };
    let fuel = t.fuel.engine_counter_l.or(t.fuel.maf_estimate_l);
    let dash = "—".to_string();
    let stats: [(&str, String, &str, Color32, &str); 6] = [
        ("Distance", fmt(t.distance_km * MI), "mi", FOREGROUND, "integral of vehicle speed over time"),
        ("Engine time", clock(t.engine_run_s.unwrap_or(t.times_s.engine)), "", FOREGROUND, "since engine start (PCM)"),
        ("Fuel used", fuel.map_or(dash.clone(), |l| format!("{:.2}", l / GAL)), if fuel.is_some() { "gal" } else { "" }, FOREGROUND, "the engine's own fuel counter, else estimated from airflow"),
        ("Economy", t.fuel.economy.as_ref().map_or(dash.clone(), |e| format!("{:.1}", e.mpg_us)), if t.fuel.economy.is_some() { "mpg" } else { "" }, FOREGROUND, "needs 0.1 km of driving"),
        ("Fuel flow now", s.fuel_flow_lph.map_or(dash.clone(), |l| format!("{:.2}", l / GAL)), if s.fuel_flow_lph.is_some() { "gal/h" } else { "" }, FOREGROUND, "airflow ÷ 14.7 ÷ 0.745 g/cc; zero during fuel cut"),
        ("Idle", format!("{idle}"), "%", FOREGROUND, "engine running, car standing"),
    ];
    window(ui, "This drive", &format!("Recording {} · saved per VIN", clock(t.duration_s)), false, |ui| {
        let w = ui.available_width();
        let cols = if w > 1000.0 { 6 } else if w > 600.0 { 4 } else { 2 };
        let cw = w / cols as f32;
        let rows = stats.len().div_ceil(cols);
        let (area, _) = ui.allocate_exact_size(vec2(w, rows as f32 * 66.0), Sense::hover());
        for (i, (l, v, u, c, h)) in stats.iter().enumerate() {
            let r = egui::Rect::from_min_size(area.min + vec2((i % cols) as f32 * cw, (i / cols) as f32 * 66.0), vec2(cw, 66.0));
            stat(ui, r, l, v, u, *c, h);
        }
        ui.add_space(8.0);
    });
}
