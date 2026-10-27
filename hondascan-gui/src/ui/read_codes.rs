//! The Read codes dialog: the person ticks the systems whose trouble codes to read. Nothing is read automatically after a
//! scan or when the car wakes into a new session; the ticks are kept for the session (app.rs `ReadCodes`).
use crate::api::Dtc;
use crate::app::{App, DtcState};
use crate::systems::info;
use crate::theme::*;
use crate::widgets::{button, button_x, Kind};
use eframe::egui::{self, RichText};

/// What the last read left for one system, and the colour the rest of the app gives it: a system not read is never shown
/// as clean.
fn status(d: Option<&DtcState>) -> (String, egui::Color32) {
    let Some(d) = d else { return ("Not read".into(), MUTED_FG) };
    if d.loading { return ("Reading…".into(), MUTED_FG); }
    if let Some(e) = &d.error { return (if e.is_empty() { "Read failed".into() } else { e.clone() }, FAULT); }
    match &d.dtcs {
        None => ("Not read".into(), MUTED_FG),
        Some(v) if v.is_empty() => ("No codes".into(), OK),
        Some(v) if v.iter().any(Dtc::confirmed) => (format!("{} codes", v.len()), FAULT),
        Some(v) => (format!("{} code{}", v.len(), if v.len() == 1 { "" } else { "s" }), WARN),
    }
}

pub fn show(ctx: &egui::Context, app: &mut App) {
    // (key, name) of every found system: the keys outlive the mutable borrow of `app` the body needs
    let rows: Vec<(String, String)> = app.ecus.iter().map(|e| (e.key(), info(&e.system_type, &e.display).name)).collect();
    let mut read = false;
    let mut cancel = false;
    let m = egui::Modal::new(egui::Id::new("read-codes")).show(ctx, |ui| {
        ui.set_width(460.0);
        ui.label(RichText::new("Read trouble codes").font(bold(16.0)));
        ui.add_space(6.0);
        ui.label(RichText::new("Codes are not read automatically. Tick the systems whose codes you want to read.").color(MUTED_FG));
        ui.add_space(10.0);
        if rows.is_empty() {
            ui.label(RichText::new("No systems found. Scan the car first.").color(MUTED_FG));
        } else {
            ui.horizontal(|ui| {
                if button(ui, "Select all", Kind::Normal).clicked() { app.read_codes.set_all(&app.ecus, true); }
                if button(ui, "Select none", Kind::Normal).clicked() { app.read_codes.set_all(&app.ecus, false); }
            });
            ui.add_space(6.0);
            egui::ScrollArea::vertical().max_height(380.0).auto_shrink([false, true]).show(ui, |ui| {
                ui.set_width(ui.available_width());
                for (key, name) in &rows {
                    let mut on = app.read_codes.checked.get(key).copied().unwrap_or(false);
                    ui.horizontal(|ui| {
                        if ui.checkbox(&mut on, name).changed() { app.read_codes.checked.insert(key.clone(), on); }
                        let (text, tone) = status(app.dtcs.get(key));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.add(egui::Label::new(RichText::new(text.clone()).size(12.0).color(tone)).truncate()).on_hover_text(text);
                        });
                    });
                }
            });
        }
        ui.add_space(14.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if button_x(ui, None, "Read selected", Kind::Primary, !rows.is_empty() && app.read_codes.any(&app.ecus)).clicked() { read = true; }
            if button(ui, "Cancel", Kind::Normal).clicked() { cancel = true; }
        });
    });
    if read { app.read_selected_codes(); }
    else if cancel || m.should_close() { app.read_codes.open = false; }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_status_never_calls_a_system_that_was_not_read_clean() {
        assert_eq!(status(None).0, "Not read");
        assert_eq!(status(Some(&DtcState { loading: true, ..Default::default() })).0, "Reading…");
        assert_eq!(status(Some(&DtcState { error: Some("timeout".into()), ..Default::default() })).0, "timeout");
        assert_eq!(status(Some(&DtcState { dtcs: Some(Vec::new()), ..Default::default() })), ("No codes".into(), OK));
        assert_eq!(status(Some(&DtcState { dtcs: Some(vec![Dtc::default()]), ..Default::default() })).0, "1 code");
        let confirmed = Dtc { kind: "confirmed".into(), ..Default::default() };
        assert_eq!(status(Some(&DtcState { dtcs: Some(vec![confirmed, Dtc::default()]), ..Default::default() })), ("2 codes".into(), FAULT));
    }
}

