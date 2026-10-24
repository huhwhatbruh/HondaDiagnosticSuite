//! The warning bar under the header: slides open and shut with the same easing as everything else, so the screen below
//! does not jump when a warning comes or goes. The backend decides what is a warning (hondascan.alerts).
use crate::api::Alert;
use crate::theme::*;
use crate::motion::ease;
use eframe::egui::{self, vec2, Color32, Sense, Ui};

#[derive(Default)]
pub struct AlertBar { shown: Vec<Alert>, open: f32, last: f64 }

impl AlertBar {
    pub fn show(&mut self, ui: &mut Ui, alerts: &[Alert]) {
        let now = ui.input(|i| i.time);
        let dt = (now - self.last).min(0.1) as f32;
        self.last = now;
        if !alerts.is_empty() { self.shown = alerts.to_vec(); }
        let target = if alerts.is_empty() { 0.0 } else { 1.0 };
        self.open += (target - self.open).clamp(-dt / 0.3, dt / 0.3);
        if self.open <= 0.0 { return; }
        let row = 34.0;
        let h = ease(self.open) * row * self.shown.len() as f32;
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::hover());
        let ui = &mut ui.new_child(egui::UiBuilder::new().max_rect(rect));
        ui.set_clip_rect(rect);
        for (i, a) in self.shown.iter().enumerate() {
            let r = egui::Rect::from_min_size(rect.min + vec2(0.0, i as f32 * row), vec2(rect.width(), row));
            let (bg, fg) = if a.fault() { (FAULT, Color32::WHITE) } else { (WARN, Color32::BLACK) };
            ui.painter().rect_filled(r, 0.0, bg);
            let pulse = 0.6 + 0.4 * ((now * 4.0).sin() as f32 * 0.5 + 0.5);
            ui.painter().circle_filled(r.left_center() + vec2(22.0, 0.0), 4.0, fg.gamma_multiply(pulse));
            ui.painter().text(r.left_center() + vec2(38.0, 0.0), egui::Align2::LEFT_CENTER, &a.text, bold(13.0), fg);
            ui.painter().line_segment([r.left_bottom(), r.right_bottom()], egui::Stroke::new(1.0, Color32::from_black_alpha(100)));
        }
        // the dot pulses at 30 fps (50 ms: egui takes a frame off the delay); sliding open or shut needs every frame
        if self.open > 0.0 && self.open < 1.0 { ui.ctx().request_repaint(); } else { ui.ctx().request_repaint_after(std::time::Duration::from_millis(50)); }
    }
}
