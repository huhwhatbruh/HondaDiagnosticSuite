//! A system's trouble codes in plain words, with HDS's possible causes, and clearing (asks first). Ported from
//! hondascan-app/src/components/DtcPanel.tsx.
use crate::api::{Dtc, Ecu};
use crate::app::DtcState;
use crate::format::tidy;
use crate::theme::*;
use crate::widgets::{button, button_x, card, label, Kind};
use eframe::egui::{self, Color32, Frame, Margin, RichText, Ui};

pub enum Action { Read, Clear }

// ISO 14229 status bits, as a mechanic would say them.
fn flag(f: &str) -> &str {
    match f {
        "testFailed" => "Failing right now", "testFailedThisOperationCycle" => "Failed this drive", "pendingDTC" => "Seen, not yet confirmed",
        "confirmedDTC" => "Confirmed", "testNotCompletedSinceLastClear" => "Self-test not run since last clear",
        "testFailedSinceLastClear" => "Failed since last clear", "testNotCompletedThisOperationCycle" => "Self-test not run this drive",
        "warningIndicatorRequested" => "Warning light on", o => o,
    }
}

fn kind(k: &str) -> (&'static str, Color32) {
    match k { "confirmed" => ("Confirmed", FAULT), "pending" => ("Pending", WARN), "incomplete" => ("Self-test not run", MUTED_FG), _ => ("Stored", MUTED_FG) }
}

/// What a clear does. `14 FF FF FF` goes to one control unit and erases all of its fault memory: the codes of every
/// system it runs, their freeze data, and the DTC status bits, so every self-test reads "not completed since last
/// clear" again (ISO 14229-1 DTC status bit 4), which is what emissions readiness shows.
fn clear_text(display: &str, also: &[String]) -> String {
    let shared = if also.is_empty() { String::new() } else { format!(" The same control unit also runs the {}: its codes are erased too.", also.join(" and ")) };
    format!("This erases every stored code in this control unit ({display}) with its freeze data, and the record of which self-tests have run: \
        emissions readiness starts again from “not complete” and fills in as the car is driven, so do not clear just before an inspection.{shared} \
        Clear only after the fault is repaired; a code that is still present comes back.")
}

/// HDS help text: a key line, the code and title, then "POSSIBLE FAILURE(S)" and one cause per line.
fn parse_help(d: &Dtc) -> (Vec<String>, Vec<String>) {
    let lines: Vec<&str> = d.help.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let at = lines.iter().position(|l| { let u = l.to_uppercase(); u == "POSSIBLE FAILURE" || u == "POSSIBLE FAILURES" });
    let head = at.map_or(&lines[..], |a| &lines[..a]);
    // lines that only repeat what the card already shows: the key line (FI_136_01_P0324), the "DTC HELP" banner, the code and the title
    let repeats = |i: usize, l: &str| {
        (i == 0 && l.contains('_') && l.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-'))
            || l.eq_ignore_ascii_case("DTC HELP") || l.starts_with(&d.code) || l.to_lowercase() == d.description.to_lowercase()
    };
    let notes = head.iter().enumerate().filter(|(i, l)| !repeats(*i, l)).map(|(_, l)| l.to_string()).collect();
    (notes, at.map_or(Vec::new(), |a| lines[a + 1..].iter().map(|s| s.to_string()).collect()))
}

/// `asking`: the clear dialog. `None` closed; `Some(ticked)` open, with its confirmation box ticked or not. `also`: the
/// other systems the same control unit runs (0x0E: engine and CVT), whose codes the clear erases as well.
pub fn show(ui: &mut Ui, ecu: &Ecu, state: Option<&DtcState>, name: &str, also: &[String], asking: &mut Option<bool>) -> Option<Action> {
    let mut act = None;
    let list: &[Dtc] = state.and_then(|s| s.dtcs.as_deref()).unwrap_or(&[]);
    let loading = state.is_some_and(|s| s.loading);
    ui.horizontal(|ui| {
        let t = if loading { RichText::new("Reading…").color(MUTED_FG) } else if let Some(e) = state.and_then(|s| s.error.as_ref()) { RichText::new(e).color(FAULT) }
            else if state.is_some_and(|s| s.dtcs.is_some()) { RichText::new(format!("{} code{} stored in {name}", list.len(), if list.len() == 1 { "" } else { "s" })).color(MUTED_FG) }
            else { RichText::new("Not read yet").color(MUTED_FG) };
        ui.label(t);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if !cfg!(hondascan_packaged) && !list.is_empty() && !loading && button_x(ui, Some(egui_phosphor::regular::ERASER), "Clear codes", Kind::Normal, true).clicked() { *asking = Some(false); }
            if !loading && button_x(ui, Some(egui_phosphor::regular::ARROWS_CLOCKWISE), "Read again", Kind::Normal, true).clicked() { act = Some(Action::Read); }
        });
    });
    if let Some(mut ticked) = *asking {
        let mut close = false;
        let m = egui::Modal::new(egui::Id::new("clear-dtc")).show(ui.ctx(), |ui| {
            ui.set_width(420.0);
            ui.label(RichText::new(format!("Clear the codes in {name}?")).font(bold(16.0)));
            ui.add_space(6.0);
            ui.label(RichText::new(clear_text(&ecu.display, also)).color(MUTED_FG));
            ui.add_space(10.0);
            let what = if also.is_empty() { String::new() } else { format!(" (also the {}’s)", also.join(" and ")) };
            ui.checkbox(&mut ticked, format!("The fault is repaired: erase every stored code{what} and freeze frame in this control unit"));
            ui.add_space(14.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if button_x(ui, None, "Clear codes", Kind::Primary, ticked && !loading).clicked() { close = true; act = Some(Action::Clear); }
                if button(ui, "Keep codes", Kind::Normal).clicked() { close = true; }
            });
        });
        *asking = if close || m.should_close() { None } else { Some(ticked) };
    }
    if state.is_some_and(|s| s.dtcs.is_some()) && list.is_empty() {
        card().inner_margin(Margin::symmetric(0, 40)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("No trouble codes").font(bold(16.0)).color(OK));
                ui.label(RichText::new(format!("{name} reports no stored or pending codes.")).size(12.0).color(MUTED_FG));
            });
        });
    }
    for d in list {
        let (notes, causes) = parse_help(d);
        let (kl, kc) = kind(&d.kind);
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            Frame::NONE.inner_margin(Margin::same(16)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(RichText::new(d.full()).font(mono_bold(18.0))).on_hover_text(format!("Raw status byte from the control unit: 0x{:02X}", d.status));
                    ui.label(RichText::new(kl).size(11.0).color(kc).background_color(Color32::from_white_alpha(10)));
                    if !d.honda_code.is_empty() { ui.label(RichText::new(format!("Honda code {}", d.honda_code)).font(mono(12.0)).color(MUTED_FG)); }
                });
                ui.label(RichText::new(if d.description.is_empty() { "This code is not in the HDS database.".to_string() } else { tidy(&d.description) }).size(14.0));
                if !notes.is_empty() { ui.label(RichText::new(notes.join(" · ")).size(13.0).color(MUTED_FG)); }
                ui.horizontal_wrapped(|ui| {
                    for f in d.flags.iter().filter(|f| *f != "confirmedDTC" && *f != "pendingDTC") {
                        ui.label(RichText::new(flag(f)).size(11.0).color(MUTED_FG).background_color(Color32::from_white_alpha(10)));
                    }
                });
            });
            if !causes.is_empty() {
                ui.separator();
                Frame::NONE.inner_margin(Margin::symmetric(16, 8)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    label(ui, "Possible causes");
                    for (j, c) in causes.iter().enumerate() {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new(format!("{}", j + 1)).font(mono(12.0)).color(MUTED_FG));
                            ui.label(RichText::new(c).size(13.0));
                        });
                    }
                });
            }
        });
    }
    act
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_clear_dialog_names_the_systems_sharing_the_control_unit_and_the_readiness_reset() {
        let alone = clear_text("SRS", &[]);
        assert!(alone.contains("readiness") && !alone.contains("also runs"));
        let pcm = clear_text("PGM-FI", &["transmission".to_string()]);
        assert!(pcm.contains("also runs the transmission: its codes are erased too"));
    }

    #[test]
    fn help_text_splits_into_notes_and_causes() {
        let d = Dtc { code: "P0420".into(), description: "Catalyst efficiency".into(), help: "4R_FI_67_01_P0420\nDTC HELP\nP0420 CATALYST\nCatalyst efficiency\nWatch the rear sensor\nPOSSIBLE FAILURES\nTWC performance\nExhaust leak".into(), ..Default::default() };
        let (notes, causes) = parse_help(&d);
        assert_eq!(notes, vec!["Watch the rear sensor".to_string()]);
        assert_eq!(causes, vec!["TWC performance".to_string(), "Exhaust leak".to_string()]);
    }
}
