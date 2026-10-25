//! Bi-directional tests: the ones the backend offers for a system (hondascan actuator.py, each cited to the HDS function that
//! defines it), a dialog that asks before every start, and the bar that is on screen for as long as one runs, with Stop.
//! The app only shows the rules; the backend enforces them (confirm, HDS's conditions, the time limit, control handed back).
use crate::api::{ActStatus, ActTest, Ecu};
use crate::theme::*;
use crate::widgets::{button, button_x, card, condition_line, label, Kind};
use eframe::egui::{self, Color32, Frame, Margin, RichText, Ui};

pub enum Action { Start(String, i64), Stop }

/// The start dialog's state: which test and value, and whether the box is ticked.
#[derive(Clone, PartialEq, Debug)]
pub struct Asking { pub key: String, pub value: i64, pub ticked: bool }

/// The tests that drive this system (the database system name), in the backend's order.
pub fn for_system<'a>(tests: &'a [ActTest], ecu: &Ecu) -> Vec<&'a ActTest> {
    if ecu.system.is_empty() { return Vec::new(); }
    tests.iter().filter(|t| t.system == ecu.system).collect()
}

/// What a value means to a person. HDS's own radiator-fan, A/C-clutch and battery-fan screens send 1 to switch the part on and 0
/// to switch it off (PGMFI_INSP: `rad_fan_act` sends 1 when the fan reads off and 0 when on; `ac_comp_sw_act` the same for ACC;
/// `batt_fan_test` sends `1 - BFAN`), and every test listed has exactly these two values. Anything else stays a bare value.
/// The battery fan's reading is an inference, not a measurement: HDS names BFAN "Battery Fan Command" and sends its opposite, so
/// 1 = on follows from BFAN = 1 meaning commanded on, which no source states (signals table, symbol BFAN) and no car run has shown.
pub fn value_word(t: &ActTest, v: i64) -> String {
    match (t.values.as_slice(), v) {
        ([0, 1], 1) => "on".into(),
        ([0, 1], 0) => "off".into(),
        _ => format!("value {v}"),
    }
}

/// "12 s left" while running; whole seconds, never below 0.
pub fn countdown(a: &ActStatus) -> String { format!("{} s left", a.remaining.unwrap_or(0.0).max(0.0).ceil() as u32) }

pub fn show(ui: &mut Ui, ecu: &Ecu, tests: &[&ActTest], act: Option<&ActStatus>, asking: &mut Option<Asking>) -> Option<Action> {
    let mut out = None;
    let running = act.filter(|a| a.active);
    if tests.is_empty() {
        ui.label(RichText::new("No bi-directional tests are offered for this module. Only tests that HDS itself defines, each checked against its own \
            conditions, are ever listed.").color(MUTED_FG));
        return None;
    }
    ui.label(RichText::new("These make the car move or switch something so you can see it work. Each one asks first, runs for a limited time, stops itself, \
        and gives control back to the car if anything goes wrong or the adapter is unplugged.").size(13.0).color(MUTED_FG));
    ui.add_space(8.0);
    for t in tests {
        card(ui, egui::Margin::ZERO, |ui| {
            ui.set_width(ui.available_width());
            Frame::NONE.inner_margin(Margin::same(16)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&t.name).font(bold(16.0)));
                    ui.label(RichText::new(format!("0x{:04X} · up to {:.0} s", t.did, t.hold_s)).font(mono(11.0)).color(MUTED_FG));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        for v in t.values.iter().rev() {
                            if button_x(ui, None, &format!("Switch {}…", value_word(t, *v)), Kind::Normal, running.is_none()).clicked() {
                                *asking = Some(Asking { key: t.key.clone(), value: *v, ticked: false });
                            }
                        }
                    });
                });
                label(ui, "Must hold before it starts");
                for c in &t.conditions { condition_line(ui, c); }
                if !t.state.is_empty() {
                    ui.add_space(4.0);
                    ui.label(RichText::new(format!("See the effect in Live data: {}", t.state.join(", "))).size(12.0).color(MUTED_FG));
                }
                ui.add_space(4.0);
                egui::CollapsingHeader::new(RichText::new("Where this comes from").size(11.5).color(MUTED_FG)).id_salt(("act-src", &t.key)).show(ui, |ui| {
                    ui.label(RichText::new(format!("HDS: {}", t.source)).font(mono(10.5)).color(MUTED_FG));
                });
            });
        });
        ui.add_space(8.0);
    }
    if let Some(mut a) = asking.clone() {
        let Some(t) = tests.iter().find(|t| t.key == a.key) else { *asking = None; return out };
        let mut close = false;
        let m = egui::Modal::new(egui::Id::new("start-test")).show(ui.ctx(), |ui| {
            ui.set_width(440.0);
            ui.label(RichText::new(format!("Switch the {} {}?", t.name.to_lowercase(), value_word(t, a.value))).font(bold(16.0)));
            ui.add_space(6.0);
            ui.label(RichText::new(format!("The car’s own computer ({}) will be told to drive this part for up to {:.0} seconds, then it takes over again. \
                The conditions below are checked before it starts and while it runs; if one stops holding, the test ends.", ecu.display, t.hold_s)).color(MUTED_FG));
            ui.add_space(6.0);
            for c in &t.conditions { condition_line(ui, c); }
            ui.add_space(10.0);
            ui.checkbox(&mut a.ticked, "The car is stationary and parked, nobody is near the part that will move, and I can stop the test at any time");
            ui.add_space(14.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if button_x(ui, None, "Start test", Kind::Primary, a.ticked).clicked() { close = true; out = Some(Action::Start(t.key.clone(), a.value)); }
                if button(ui, "Cancel", Kind::Normal).clicked() { close = true; }
            });
        });
        *asking = if close || m.should_close() { None } else { Some(a) };
    }
    out
}

/// The bar shown under the header whenever a test runs, on every screen: what, how long is left, and Stop.
pub fn running_bar(ui: &mut Ui, act: Option<&ActStatus>, tests: &[ActTest]) -> Option<Action> {
    let a = act.filter(|a| a.active)?;
    let mut out = None;
    Frame::NONE.fill(WARN).inner_margin(Margin::symmetric(16, 6)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("Test running: {} {} · {}", a.name, tests.iter().find(|t| t.key == a.test).map_or_else(|| format!("(sent {})", a.value), |t| format!("switched {}", value_word(t, a.value))), countdown(a))).font(bold(13.0)).color(Color32::BLACK));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if button_x(ui, Some(egui_phosphor::regular::STOP), "Stop now", Kind::Primary, true).clicked() { out = Some(Action::Stop); }
            });
        });
    });
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(key: &str, system: &str) -> ActTest { ActTest { key: key.into(), system: system.into(), ..Default::default() } }

    #[test]
    fn only_the_tests_of_this_system_are_listed() {
        let all = [t("radiator_fan", "CANFI"), t("brake", "CANABS"), t("ac_clutch", "CANFI")];
        let pcm = Ecu { system: "CANFI".into(), ..Default::default() };
        let keys: Vec<&str> = for_system(&all, &pcm).iter().map(|t| t.key.as_str()).collect();
        assert_eq!(keys, ["radiator_fan", "ac_clutch"]);
        assert!(for_system(&all, &Ecu::default()).is_empty(), "a module without a database system has no tests");
    }

    #[test]
    fn the_two_valued_tests_read_as_on_and_off() {
        let fan = ActTest { values: vec![0, 1], ..Default::default() };
        assert_eq!((value_word(&fan, 1), value_word(&fan, 0)), ("on".to_string(), "off".to_string()));
        let other = ActTest { values: vec![0, 1, 2], ..Default::default() };
        assert_eq!(value_word(&other, 1), "value 1", "a test with other values is never guessed at");
    }

    #[test]
    fn the_countdown_rounds_up_and_never_goes_negative() {
        let a = |r| ActStatus { remaining: r, active: true, ..Default::default() };
        assert_eq!(countdown(&a(Some(11.2))), "12 s left");
        assert_eq!(countdown(&a(Some(-0.3))), "0 s left");
        assert_eq!(countdown(&a(None)), "0 s left");
    }
}
