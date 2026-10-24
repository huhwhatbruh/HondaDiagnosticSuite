//! Self-tests (the car's own monitors) of one system: which have run since the first read of the session and which are still
//! waiting. The backend derives them from the DTC records that carry only the "test not completed" bits (selftests.py); a test
//! that leaves the waiting list ran and passed, since a failure would come back as a trouble code.
use crate::api::{Ecu, SelfTestSys, SelfTestItem};
use crate::format::tidy;
use crate::theme::*;
use crate::widgets::window;
use eframe::egui::{RichText, Ui};

/// The summary for this system, keyed the way the backend keys it (database system, else system type).
pub fn for_ecu<'a>(all: &'a std::collections::HashMap<String, SelfTestSys>, ecu: &Ecu) -> Option<&'a SelfTestSys> {
    let key = if ecu.system.is_empty() { &ecu.system_type } else { &ecu.system };
    all.get(key).filter(|s| s.at_start > 0)
}

/// "3 of 20 self-tests have run since the car was started" — the headline of the card.
pub fn headline(s: &SelfTestSys) -> String {
    let done = s.completed.len();
    if s.now == 0 { format!("All {} self-tests have run.", s.at_start) }
    else { format!("{done} of {} self-tests have run since this session began; {} still waiting.", s.at_start, s.now) }
}

fn list(ui: &mut Ui, title: &str, items: &[SelfTestItem], color: eframe::egui::Color32) {
    if items.is_empty() { return; }
    ui.add_space(6.0);
    ui.horizontal(|ui| { ui.add_space(14.0); crate::widgets::label(ui, title); });
    for d in items {
        ui.horizontal(|ui| {
            ui.add_space(14.0);
            ui.label(RichText::new(crate::api::code_with_type(&d.code, d.failure_type)).font(mono(12.5)).color(color));
            ui.label(RichText::new(if d.description.is_empty() { "not in the HDS database".to_string() } else { tidy(&d.description) }).size(13.0));
        });
    }
}

pub fn show(ui: &mut Ui, s: &SelfTestSys) {
    window(ui, "Self-tests", "", false, |ui| {
        ui.add_space(6.0);
        ui.horizontal(|ui| { ui.add_space(14.0); ui.label(RichText::new(headline(s)).size(13.0)); });
        ui.horizontal_wrapped(|ui| {
            ui.add_space(14.0);
            ui.label(RichText::new("The car runs these checks by itself while it is driven (some need a warm engine or a steady cruise). One that finishes \
                has passed; a failure would show up as a trouble code.").size(12.0).color(MUTED_FG));
        });
        list(ui, "Done, passed", &s.completed, OK);
        list(ui, "Still waiting", &s.incomplete, MUTED_FG);
        ui.add_space(8.0);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(code: &str) -> SelfTestItem { SelfTestItem { code: code.into(), ..Default::default() } }

    #[test]
    fn the_headline_counts_what_ran_and_what_waits() {
        let s = SelfTestSys { at_start: 4, now: 3, completed: vec![item("P0420")], incomplete: vec![item("P0171"), item("P0172"), item("P0300")] };
        assert_eq!(headline(&s), "1 of 4 self-tests have run since this session began; 3 still waiting.");
        let all = SelfTestSys { at_start: 4, now: 0, completed: vec![item("a"); 4], incomplete: vec![] };
        assert_eq!(headline(&all), "All 4 self-tests have run.");
    }

    #[test]
    fn a_system_with_no_self_tests_has_no_card() {
        let mut all = std::collections::HashMap::new();
        all.insert("CANFI".to_string(), SelfTestSys { at_start: 2, now: 2, ..Default::default() });
        all.insert("CANSRS".to_string(), SelfTestSys::default());
        let pcm = Ecu { system: "CANFI".into(), ..Default::default() };
        let srs = Ecu { system: "CANSRS".into(), ..Default::default() };
        assert!(for_ecu(&all, &pcm).is_some() && for_ecu(&all, &srs).is_none());
        assert!(for_ecu(&all, &Ecu { system_type: "CANFI".into(), ..Default::default() }).is_some(), "falls back to the system type");
    }
}
