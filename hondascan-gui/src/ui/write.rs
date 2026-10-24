//! Writing functions: the ones the backend offers for a system (hondascan write.rs, each cited to the HDS function
//! that defines it), a dialog that asks before every run, and a result toast. This changes stored data in the car's
//! computer, so the dialog is deliberately heavier than a test's: the car's VIN has to be known, the box has to be
//! ticked, and the backend checks HDS's own conditions and reads the old value before it writes anything.
use crate::api::{WriteFn, WriteResult};
use crate::theme::*;
use crate::widgets::{button, button_x, card, condition_line, label, Kind};
use eframe::egui::{self, Color32, Frame, Margin, RichText, Ui};

pub enum Action { Run(String) }

/// The run dialog's state: which function, and whether the box is ticked.
#[derive(Clone, PartialEq, Debug)]
pub struct Asking { pub key: String, pub ticked: bool }

/// The write functions for this system (the database system name), in the backend's order.
pub fn for_system<'a>(writes: &'a [WriteFn], ecu: &crate::api::Ecu) -> Vec<&'a WriteFn> {
    if ecu.system.is_empty() { return Vec::new(); }
    writes.iter().filter(|w| w.system == ecu.system).collect()
}

fn bit(v: Option<bool>) -> &'static str {
    match v {
        Some(true) => "1",
        Some(false) => "0",
        None => "?",
    }
}

/// A finished run's line for a toast.
pub fn result_text(r: &WriteResult) -> String {
    let read = if r.kind == "routine_status" {
        // The meter's proof is the routine status itself: HDS's pending value while the reset runs, the final status
        // byte when the poll ends (0 is complete; the runner reports anything else as a failure).
        let v = |v: Option<u32>| v.map_or_else(|| "?".to_string(), |x| format!("0x{x:02X}"));
        format!("routine status 0x{:02X} -> {}", r.pending.unwrap_or(0), v(r.new_value))
    } else if r.poll_ms > 0 {
        let n = |v: Option<u32>| v.map_or_else(|| "?".to_string(), |x| x.to_string());
        format!("SLPLMOD {} -> {}", n(r.old_value), n(r.new_value))
    } else {
        format!("read-back {} -> {}", bit(r.old), bit(r.new))
    };
    if r.errors.is_empty() { read } else { format!("{read}; {}", r.errors.join("; ")) }
}

/// What the card says HDS does after the start: a fixed wait, a poll window, or HDS's routine-status monitor.
fn when(w: &WriteFn) -> String {
    if !w.monitor.is_empty() {
        format!("sends {} · then polls {} for up to {:.0} s", w.start, w.monitor, w.timeout_s)
    } else if w.poll_ms > 0 {
        format!("sends {} · polls every {} ms for up to {:.0} s", w.start, w.poll_ms, w.timeout_s)
    } else {
        format!("sends {} · waits {:.0} s", w.start, w.wait_s)
    }
}

/// The bar on screen while a write runs (the backend call blocks for HDS's wait, up to 10 s). True when Cancel was clicked: the
/// backend stops the routine and leaves the session, and the pending `write_run` then answers with the cancel.
pub fn running_bar(ui: &mut Ui, running: Option<&str>) -> bool {
    let Some(name) = running else { return false };
    let mut cancel = false;
    Frame::NONE.fill(WARN).inner_margin(Margin::symmetric(16, 6)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("Writing: {name} · the routine stops and the session is left if you cancel")).font(bold(13.0)).color(Color32::BLACK));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if button_x(ui, Some(egui_phosphor::regular::STOP), "Cancel", Kind::Primary, true).clicked() { cancel = true; }
            });
        });
    });
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
    cancel
}

pub fn show(ui: &mut Ui, ecu: &crate::api::Ecu, writes: &[&WriteFn], vin: Option<&str>, asking: &mut Option<Asking>) -> Option<Action> {
    let mut out = None;
    if writes.is_empty() {
        ui.label(RichText::new("No write functions are offered for this module. Only functions HDS itself defines are ever listed.").color(MUTED_FG));
        return None;
    }
    ui.label(RichText::new("These change data stored in the car's computer (adaptation values). Each one asks first, checks the conditions HDS's own \
        function checks, reads the old value and only then sends the routine the car's computer already knows. Nothing here has run on a car yet.")
        .size(13.0).color(MUTED_FG));
    ui.add_space(8.0);
    for w in writes {
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            Frame::NONE.inner_margin(Margin::same(16)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&w.name).font(bold(16.0)));
                    ui.label(RichText::new(when(w)).font(mono(11.0)).color(MUTED_FG));
                    if w.stop.is_empty() {
                        ui.label(RichText::new("HDS sends no stop").font(mono(11.0)).color(MUTED_FG));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if button_x(ui, None, "Run…", Kind::Normal, vin.is_some()).clicked() {
                            *asking = Some(Asking { key: w.key.clone(), ticked: false });
                        }
                    });
                });
                label(ui, "HDS checks before it will write");
                for c in &w.conditions { condition_line(ui, c); }
                ui.add_space(4.0);
                ui.label(RichText::new(format!("Read-back: {}", w.read_back)).size(12.5).color(MUTED_FG));
                ui.add_space(4.0);
                egui::CollapsingHeader::new(RichText::new("Where this comes from").size(11.5).color(MUTED_FG)).id_salt(("write-src", &w.key)).show(ui, |ui| {
                    ui.label(RichText::new(format!("HDS: {}", w.source)).font(mono(10.5)).color(MUTED_FG));
                });
            });
        });
        ui.add_space(8.0);
    }
    if let Some(mut a) = asking.clone() {
        let Some(w) = writes.iter().find(|w| w.key == a.key) else { *asking = None; return out };
        let mut close = false;
        let modal = egui::Modal::new(egui::Id::new("run-write")).show(ui.ctx(), |ui| {
            ui.set_width(500.0);
            ui.label(RichText::new(format!("Run “{}”?", w.name)).font(bold(16.0)));
            ui.add_space(6.0);
            ui.label(RichText::new(format!("This sends a routine HDS itself uses to the car's computer ({}); it changes stored adaptation data. It has been \
                tested against a fake ECU only and has never run on a car, so treat the outcome as unproven until the read-back says otherwise.", ecu.display)).color(MUTED_FG));
            ui.add_space(6.0);
            if w.kind == "routine_status" {
                ui.label(RichText::new(format!("Sends: {} · monitor {}", w.start, w.monitor)).font(mono(11.0)));
            } else {
                ui.label(RichText::new(format!("Sends: {} (read-back DID 0x{:04X} byte {}, mask 0x{:02X})", w.start, w.did, w.byte, w.mask)).font(mono(11.0)));
            }
            ui.label(RichText::new(format!("Reads back: {}", w.read_back)).size(12.5).color(MUTED_FG));
            if w.stop.is_empty() {
                ui.label(RichText::new("HDS sends no stop for this one; it polls the read-back until the routine reports done.").size(12.5).color(MUTED_FG));
            }
            ui.add_space(6.0);
            ui.label(RichText::new("The conditions are checked from live data before anything is sent:").size(12.5));
            for c in &w.conditions { condition_line(ui, c); }
            ui.add_space(6.0);
            ui.label(RichText::new(match vin {
                Some(v) => format!("The car's VIN is {v}; a mismatch refuses before anything is sent."),
                None => "The VIN was not read: run a scan first. Without it the backend will refuse.".into(),
            }).size(12.5).color(if vin.is_some() { MUTED_FG } else { FAULT }));
            ui.add_space(10.0);
            ui.checkbox(&mut a.ticked, "I understand this writes stored data to the car, the car is parked with the ignition on, and I accept that this \
                function has never been run on a car");
            ui.add_space(14.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if button_x(ui, None, "Run", Kind::Primary, a.ticked && vin.is_some()).clicked() { close = true; out = Some(Action::Run(w.key.clone())); }
                if button(ui, "Cancel", Kind::Normal).clicked() { close = true; }
            });
        });
        // escape and a click on the backdrop dismiss the dialog, the same as cancel
        if modal.should_close() { close = true; }
        if close { *asking = None; } else { *asking = Some(a); }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(key: &str, system: &str) -> WriteFn { WriteFn { key: key.into(), system: system.into(), ..Default::default() } }

    #[test]
    fn only_the_writes_of_this_system_are_listed() {
        let all = [w("evtc_learn", "CANFI"), w("ckp_learn", "CANFI"), w("abs_something", "CANABS")];
        let pcm = crate::api::Ecu { system: "CANFI".into(), ..Default::default() };
        let keys: Vec<&str> = for_system(&all, &pcm).iter().map(|w| w.key.as_str()).collect();
        assert_eq!(keys, ["evtc_learn", "ckp_learn"]);
        assert!(for_system(&all, &crate::api::Ecu::default()).is_empty(), "a module without a database system has no writes");
    }

    #[test]
    fn the_result_line_names_the_read_back_and_any_problems() {
        let r = WriteResult { old: Some(false), new: Some(true), ..Default::default() };
        assert_eq!(result_text(&r), "read-back 0 -> 1");
        let r = WriteResult { old: Some(false), new: Some(false), errors: vec!["stop routine: no response".into()], ..Default::default() };
        assert_eq!(result_text(&r), "read-back 0 -> 0; stop routine: no response");
    }

    #[test]
    fn a_polling_write_names_its_status_code() {
        let r = WriteResult { old_value: Some(0), new_value: Some(2), poll_ms: 100, ..Default::default() };
        assert_eq!(result_text(&r), "SLPLMOD 0 -> 2");
        let r = WriteResult { old_value: Some(0), new_value: Some(3), error_value: Some(1), poll_ms: 100,
            errors: vec!["routine reported an error".into()], ..Default::default() };
        assert_eq!(result_text(&r), "SLPLMOD 0 -> 3; routine reported an error");
    }

    #[test]
    fn a_routine_status_write_names_the_status_byte() {
        // The meter's proof is the routine status: HDS's pending value 0x7F while the reset runs, 0 when complete.
        let r = WriteResult { kind: "routine_status".into(), pending: Some(0x7F), new_value: Some(0), ..Default::default() };
        assert_eq!(result_text(&r), "routine status 0x7F -> 0x00");
        let r = WriteResult { kind: "routine_status".into(), pending: Some(0x7F), new_value: Some(5), error_value: Some(5),
            errors: vec!["routine status 0x05 (HDS treats any value other than 0x7F or 0x00 as failure)".into()], ..Default::default() };
        assert_eq!(result_text(&r), "routine status 0x7F -> 0x05; routine status 0x05 (HDS treats any value other than 0x7F or 0x00 as failure)");
    }

    #[test]
    fn the_card_says_which_timing_hds_uses() {
        let mut f = w("evtc_learn", "CANFI");
        f.start = "31 01 02 18 02".into();
        f.wait_s = 2.0;
        assert_eq!(when(&f), "sends 31 01 02 18 02 · waits 2 s");
        f.poll_ms = 100;
        f.timeout_s = 8.0;
        assert_eq!(when(&f), "sends 31 01 02 18 02 · polls every 100 ms for up to 8 s");
    }
}
