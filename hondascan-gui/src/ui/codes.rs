//! All trouble codes of every system, in one list (ported from the Overview's "Trouble codes" panel).
use crate::app::{App, View};
use crate::format::tidy;
use crate::systems::info;
use crate::theme::*;
use crate::widgets::window;
use eframe::egui::{self, vec2, Align2, RichText, Sense, Ui};

pub fn show(ui: &mut Ui, app: &mut App, height: f32) {
    let faults: Vec<(usize, crate::api::Dtc)> = app.ecus.iter().enumerate()
        .flat_map(|(i, e)| app.dtcs.get(&e.key()).and_then(|d| d.dtcs.clone()).unwrap_or_default().into_iter().map(move |d| (i, d))).collect();
    let confirmed = faults.iter().filter(|(_, d)| d.confirmed()).count();
    let reading = app.scanning || app.dtcs.values().any(|d| d.loading);
    let unread: Vec<String> = crate::app::unread(&app.ecus, &app.dtcs).iter().map(|e| info(&e.system_type, &e.display).name).collect();
    let note = format!("{} systems{}", app.ecus.len(), app.scan_seconds.map_or(String::new(), |s| format!(" · scanned in {s} s")));
    let mut open = None;
    window(ui, "Trouble codes", &note, false, |ui| {
        ui.set_min_height(height - 30.0);
        let tone = if reading { MUTED_FG } else if confirmed > 0 { FAULT } else if !faults.is_empty() || !unread.is_empty() { WARN } else { OK };
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.add_space(14.0);
            let text = if reading { "Reading…".to_string() } else if faults.is_empty() && unread.is_empty() { "All clear".into() }
                else if faults.is_empty() { format!("{} not read", unread.len()) }
                else { format!("{} code{}", faults.len(), if faults.len() > 1 { "s" } else { "" }) };
            ui.label(RichText::new(text).font(bold(20.0)).color(tone));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(14.0);
                if !faults.is_empty() { ui.label(RichText::new(format!("{confirmed} confirmed · {} pending", faults.len() - confirmed)).size(12.0).color(MUTED_FG)); }
            });
        });
        ui.add_space(4.0);
        ui.separator();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for (i, d) in &faults {
                let e = &app.ecus[*i];
                let desc = if d.description.is_empty() { "Not in the HDS database".to_string() } else { tidy(&d.description) };
                let galley = ui.painter().layout(desc, sans(12.5), MUTED_FG, ui.available_width() - 56.0);
                let h = 34.0 + galley.size().y;
                let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::click());
                if resp.hovered() { ui.painter().rect_filled(r, 0.0, MUTED); }
                let c = if d.confirmed() { FAULT } else { WARN };
                ui.painter().circle_filled(r.left_top() + vec2(18.0, 20.0), 3.0, c);
                ui.painter().text(r.left_top() + vec2(32.0, 18.0), Align2::LEFT_CENTER, d.full(), mono_bold(13.0), FOREGROUND);
                ui.painter().text(r.right_top() + vec2(-14.0, 18.0), Align2::RIGHT_CENTER, info(&e.system_type, &e.display).name, sans(12.0), MUTED_FG);
                ui.painter().galley(r.left_top() + vec2(32.0, 30.0), galley, MUTED_FG);
                ui.painter().line_segment([r.left_bottom(), r.right_bottom()], egui::Stroke::new(1.0, BORDER));
                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() { open = Some(e.key()); }
            }
            if !unread.is_empty() && !reading {
                ui.add_space(12.0);
                ui.horizontal_wrapped(|ui| { ui.add_space(14.0); ui.label(RichText::new(format!("Codes not known (the read failed): {}. Open the system to read again.", unread.join(", "))).size(12.0).color(WARN)); });
            }
            if faults.is_empty() && unread.is_empty() && !reading {
                ui.add_space(12.0);
                ui.horizontal(|ui| { ui.add_space(14.0); ui.label(RichText::new("No stored or pending codes in any system.").size(12.0).color(MUTED_FG)); });
            }
        });
    });
    if let Some(k) = open { app.view = View::System(k); }
}
