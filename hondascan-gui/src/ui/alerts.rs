//! The warnings under the header, as inset notices: slide open and shut with the same easing as everything else, so the screen below
//! does not jump when a warning comes or goes. The backend decides what is a warning (hondascan.alerts).
use crate::api::Alert;
use crate::theme::*;
use crate::motion::ease;
use eframe::egui::{self, vec2, Sense, Ui};

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
        let row = 42.0;   // a 34 px notice and 8 px between it and the next
        let h = ease(self.open) * row * self.shown.len() as f32 + 8.0 * ease(self.open);
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::hover());
        let ui = &mut ui.new_child(egui::UiBuilder::new().max_rect(rect));
        ui.set_clip_rect(rect);
        for (i, a) in self.shown.iter().enumerate() {
            // an inset notice on a dark surface tinted with the status colour, not a bar of that colour: the text keeps the
            // normal light-on-dark contrast and the icon carries the status (never colour alone)
            let r = egui::Rect::from_min_size(rect.min + vec2(8.0, 8.0 + i as f32 * row), vec2(rect.width() - 16.0, row - 8.0));
            let tone = if a.fault() { FAULT } else { WARN };
            ui.painter().add(crate::widgets::outlined(ui.ctx(), r, RADIUS_WINDOW, BACKGROUND.lerp_to_gamma(tone, 0.12), BACKGROUND.lerp_to_gamma(tone, 0.38)));
            let pulse = if crate::motion::reduced() { 1.0 } else { 0.75 + 0.25 * ((now * 4.0).sin() as f32 * 0.5 + 0.5) };
            ui.painter().text(r.left_center() + vec2(14.0, 0.0), egui::Align2::LEFT_CENTER, status_icon(tone).unwrap_or(""), sans(16.0), tone.gamma_multiply(pulse));
            ui.painter().text(r.left_center() + vec2(40.0, 0.0), egui::Align2::LEFT_CENTER, &a.text, sans(13.0), FOREGROUND);
        }
        // the dot pulses at 30 fps (50 ms: egui takes a frame off the delay); sliding open or shut needs every frame
        if self.open > 0.0 && self.open < 1.0 { ui.ctx().request_repaint(); } else if !crate::motion::reduced() { ui.ctx().request_repaint_after(std::time::Duration::from_millis(50)); }
    }
}
