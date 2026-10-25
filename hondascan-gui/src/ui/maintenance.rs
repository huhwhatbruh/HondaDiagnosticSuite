//! The maintenance dialogs: freezing a healthy baseline (an immutable record of what the car reports now) and adding a
//! service marker (a note about work done). Both ask first and state what will be stored; the backend enforces the
//! freeze gate again and refuses a marker without a note. Nothing here sends anything to the car.
use crate::app::{can_freeze, App};
use crate::theme::*;
use crate::widgets::{button, button_x, Kind};
use eframe::egui::{self, RichText, Ui};

/// The marker kinds the backend accepts (rpc.rs `marker_add`); no free text, so the history stays sortable.
pub const KINDS: [&str; 9] = ["oil", "tires", "brakes", "battery", "filter", "fluid", "inspection", "repair", "other"];

/// 1 mile = 1.609344 km exactly; markers are stored in kilometres and shown in miles.
pub const KM_PER_MILE: f64 = 1.609344;

pub fn miles_to_km(miles: f64) -> f64 { miles * KM_PER_MILE }

/// A kind as it is shown: "oil" is "Oil".
pub fn kind_label(kind: &str) -> String {
    let mut c = kind.chars();
    c.next().map_or_else(|| "Other".to_string(), |f| f.to_uppercase().collect::<String>() + c.as_str())
}

/// The freeze confirmation's state: an optional label to find it by later.
#[derive(Default, Clone, PartialEq, Debug)]
pub struct FreezeDialog { pub label: String }

/// The marker dialog's state: kind, note and the optional mileage as typed (miles).
#[derive(Clone, PartialEq, Debug)]
pub struct MarkerDialog { pub kind: String, pub note: String, pub mileage_miles: String }

impl Default for MarkerDialog {
    fn default() -> Self { MarkerDialog { kind: KINDS[0].into(), note: String::new(), mileage_miles: String::new() } }
}

/// A checked marker, ready to send: the note trimmed, the mileage in miles (the app converts it to km).
#[derive(Clone, PartialEq, Debug)]
pub struct MarkerInput { pub kind: String, pub note: String, pub mileage_miles: Option<f64> }

/// What is wrong with a marker, or the marker itself. A note is required and at most 2000 characters; the mileage, when
/// given, is a number of miles (commas accepted, as people write them) and never negative.
pub fn check_marker(kind: &str, note: &str, mileage: &str) -> Result<MarkerInput, String> {
    if !KINDS.contains(&kind) { return Err("Pick a kind of work.".into()); }
    let note = note.trim();
    if note.is_empty() { return Err("A note is required: say what was done.".into()); }
    if note.chars().count() > 2000 { return Err("The note is longer than 2000 characters.".into()); }
    let typed = mileage.trim().replace([',', '_'], "");
    let miles = if typed.is_empty() { None } else {
        let m: f64 = typed.parse().map_err(|_| "The mileage must be a number of miles, e.g. 82,400.".to_string())?;
        if !m.is_finite() || m < 0.0 { return Err("The mileage cannot be negative.".into()); }
        Some(m)
    };
    Ok(MarkerInput { kind: kind.to_string(), note: note.to_string(), mileage_miles: miles })
}

/// Open a file with the desktop's own viewer. Spawn errors are ignored: a machine with no desktop still records the
/// path, and saying so would only add noise to a successful write.
pub fn open_path(path: &str) {
    let opener = if cfg!(target_os = "macos") { "open" } else if cfg!(target_os = "windows") { "explorer" } else { "xdg-open" };
    let _ = std::process::Command::new(opener).arg(path).spawn();
}

/// The freeze confirmation. `app.freeze` is Some while it is open; a submission starts the backend call and closes it.
pub fn show_freeze(ui: &mut Ui, app: &mut App) {
    let Some(mut d) = app.freeze.take() else { return };
    let Some(vin) = app.vin.clone().filter(|v| !v.is_empty()) else { return };
    let gate = can_freeze(&app.ecus, &app.dtcs, Some(&vin));
    let systems = app.ecus.len();
    let stored: usize = app.dtcs.values().filter_map(|s| s.dtcs.as_ref()).flatten().filter(|c| c.kind == "stored").count();
    let not_run: usize = app.dtcs.values().map(|s| s.incomplete.len()).sum();
    let (mut close, mut freeze) = (false, false);
    let m = egui::Modal::new(egui::Id::new("freeze-baseline")).show(ui.ctx(), |ui| {
        ui.set_width(480.0);
        ui.label(RichText::new("Save a reference snapshot?").font(bold(16.0)));
        ui.add_space(6.0);
        ui.label(RichText::new("This stores what the car reports right now (trouble codes, which systems answered, readiness and self-tests) as the \
            reference the next health report is compared against. It says what this car looked like on the day; it does not record \
            how the engine performs. It is kept as it is: a snapshot can never be edited or deleted, \
            only added to. Stored (history) codes are recorded as they are; a confirmed code, a pending code or an unread system \
            blocks saving, because then there is no clean state to compare against.").color(MUTED_FG));
        ui.add_space(8.0);
        ui.label(RichText::new(format!("VIN {vin} · {systems} system{} · {stored} stored code{}", if systems == 1 { "" } else { "s" },
            if stored == 1 { "" } else { "s" })).font(mono(11.5)).color(MUTED_FG));
        if not_run > 0 {
            ui.add_space(4.0);
            ui.label(RichText::new(format!("{not_run} self-test{} not run yet: recorded as they are (normal on a used car or after a battery \
                disconnect). When they finish, the next report shows it as a readiness change.", if not_run == 1 { " has" } else { "s have" })).size(12.5).color(MUTED_FG));
        }
        ui.add_space(8.0);
        ui.label(RichText::new("Label (optional)").size(12.5).color(MUTED_FG));
        ui.add(egui::TextEdit::singleline(&mut d.label).hint_text("Reference snapshot").desired_width(f32::INFINITY));
        if let Err(why) = &gate {
            ui.add_space(6.0);
            ui.label(RichText::new(why).size(12.5).color(FAULT));
        }
        ui.add_space(14.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if button_x(ui, Some(egui_phosphor::regular::FLOPPY_DISK), "Save snapshot", Kind::Primary, gate.is_ok()).clicked() { freeze = true; }
            if button(ui, "Cancel", Kind::Normal).clicked() { close = true; }
        });
    });
    if freeze { app.freeze_baseline(d.label.trim()); }
    else if !(close || m.should_close()) { app.freeze = Some(d); }
}

/// The marker dialog. `app.marker` is Some while it is open; a submission sends the marker and closes it.
pub fn show_marker(ui: &mut Ui, app: &mut App) {
    let Some(mut d) = app.marker.take() else { return };
    if app.vin.is_none() { return; }
    let checked = check_marker(&d.kind, &d.note, &d.mileage_miles);
    let (mut close, mut add) = (false, false);
    let m = egui::Modal::new(egui::Id::new("add-marker")).show(ui.ctx(), |ui| {
        ui.set_width(480.0);
        ui.label(RichText::new("Add a service marker?").font(bold(16.0)));
        ui.add_space(6.0);
        ui.label(RichText::new("A note kept with this car’s history: what was done, when, and how far the car had gone. It is written \
            to the app’s own record; nothing is sent to the car.").color(MUTED_FG));
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Kind").size(12.5).color(MUTED_FG));
            egui::ComboBox::from_id_salt("marker-kind").width(160.0).selected_text(kind_label(&d.kind)).show_ui(ui, |ui| {
                for k in KINDS { ui.selectable_value(&mut d.kind, k.to_string(), kind_label(k)); }
            });
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Note").size(12.5).color(MUTED_FG));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(format!("{} / 2000", d.note.chars().count())).font(mono(10.5)).color(MUTED_FG));
            });
        });
        ui.add(egui::TextEdit::multiline(&mut d.note).hint_text("5W-30 oil and filter, done at the dealer").desired_rows(3).desired_width(f32::INFINITY));
        ui.add_space(6.0);
        ui.label(RichText::new("Mileage (optional, in miles)").size(12.5).color(MUTED_FG));
        ui.add(egui::TextEdit::singleline(&mut d.mileage_miles).hint_text("82,400").desired_width(160.0));
        if let Err(why) = &checked {
            ui.add_space(6.0);
            ui.label(RichText::new(why).size(12.5).color(if d.note.is_empty() { MUTED_FG } else { FAULT }));
        }
        ui.add_space(14.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if button_x(ui, Some(egui_phosphor::regular::WRENCH), "Add marker", Kind::Primary, checked.is_ok()).clicked() { add = true; }
            if button(ui, "Cancel", Kind::Normal).clicked() { close = true; }
        });
    });
    if add {
        if let Ok(input) = checked { app.add_marker(&input.kind, &input.note, input.mileage_miles); }
    } else if !(close || m.should_close()) { app.marker = Some(d); }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_note_and_kind_are_required_and_the_note_is_trimmed() {
        let m = check_marker("oil", "  5W-30 and filter  ", "").unwrap();
        assert_eq!(m.note, "5W-30 and filter");
        assert_eq!(m.kind, "oil");
        assert!(m.mileage_miles.is_none(), "no mileage typed means none sent");
        assert!(check_marker("oil", "   ", "").is_err());
        assert!(check_marker("wax", "Something", "").is_err(), "only the backend's kinds are offered");
    }

    #[test]
    fn the_note_is_capped_at_2000_characters() {
        let ok = "ä".repeat(2000);
        assert!(check_marker("repair", &ok, "").is_ok(), "2000 characters is allowed (umlauts count once)");
        assert!(check_marker("repair", &format!("{ok}ä"), "").is_err(), "2001 is not");
        assert!(check_marker("repair", "", "").unwrap_err().contains("note"));
    }

    #[test]
    fn mileage_is_read_as_miles_and_commas_are_accepted() {
        assert_eq!(check_marker("tires", "Four new", "82,400").unwrap().mileage_miles, Some(82400.0));
        assert_eq!(check_marker("tires", "Four new", " 0 ").unwrap().mileage_miles, Some(0.0));
        assert!(check_marker("tires", "Four new", "soon").is_err());
        assert!(check_marker("tires", "Four new", "-1").is_err(), "a negative odometer is a typo");
        assert_eq!(miles_to_km(100.0), 160.9344, "markers are stored in kilometres");
    }

    #[test]
    fn kinds_are_named_in_the_dialog_and_the_history_the_same_way() {
        assert_eq!(kind_label("oil"), "Oil");
        assert_eq!(kind_label("tires"), "Tires");
        assert_eq!(kind_label(""), "Other");
        assert_eq!(KINDS.len(), 9);
        assert!(KINDS.iter().all(|k| kind_label(k) != *k), "every kind gets a readable name");
    }
}
