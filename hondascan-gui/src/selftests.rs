//! Self-tests (the car's own monitors) of one system: which have run since the first read of the session and which
//! are still waiting. The backend derives them from the DTC records that carry only the "test not completed" bits
//! (selftests.py); a test that leaves the waiting list ran and passed, since a failure would come back as a trouble
//! code. QML only draws the JSON `selftests_json` makes.
use crate::api::{code_with_type, Ecu, SelfTestItem, SelfTestSys};
use crate::format::tidy;
use serde_json::{json, Value};

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

/// One of the card's two lists as QML draws it. A code is the failure-type-spelled form (C1234-49); the text falls back
/// to the HDS database's absence line. The egui coloured a list's title and codes (OK / MUTED_FG): that is drawing, so
/// no tone travels with them (it would be 0 everywhere).
fn list(title: &str, passed: bool, items: &[SelfTestItem]) -> Value {
    json!({ "title": title, "passed": passed, "items": items.iter().map(|d| json!({
        "code": code_with_type(&d.code, d.failure_type),
        "text": if d.description.is_empty() { "not in the HDS database".to_string() } else { tidy(&d.description) },
    })).collect::<Vec<_>>() })
}

/// The card as QML draws it: the headline, the note, then the lists in the egui's order ("Done, passed" before "Still
/// waiting"); an empty list is skipped, as the egui's `list` did with its early return.
pub fn selftests_json(s: &SelfTestSys) -> Value {
    let lists: Vec<Value> = [("Done, passed", true, &s.completed), ("Still waiting", false, &s.incomplete)].into_iter()
        .filter(|(_, _, items)| !items.is_empty()).map(|(title, passed, items)| list(title, passed, items)).collect();
    json!({
        "headline": headline(s),
        "note": "The car runs these checks by itself while it is driven (some need a warm engine or a steady cruise). One that finishes has passed; a failure would show up as a trouble code.",
        "lists": lists,
    })
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

    #[test]
    fn a_list_of_tests_shows_full_codes_and_hides_empty_lists() {
        let s = SelfTestSys { at_start: 4, now: 3, completed: vec![item("P0420"), SelfTestItem {
            code: "C1234".into(), failure_type: 0x49, description: "CATALYST SYSTEM EFFICIENCY BELOW THRESHOLD".into() }],
            incomplete: vec![] };
        let j = selftests_json(&s);
        assert_eq!(j["headline"], "2 of 4 self-tests have run since this session began; 3 still waiting.",
            "the card's own headline: first count is the completed list's length, then the egui's odd use of at_start");
        assert_eq!(j["note"], "The car runs these checks by itself while it is driven (some need a warm engine or a steady cruise). One that finishes has passed; a failure would show up as a trouble code.");
        let lists = j["lists"].as_array().unwrap();
        assert_eq!(lists.len(), 1, "nothing still waits: that list is skipped, as the egui's early return did");
        assert_eq!(lists[0]["title"], "Done, passed");
        let items = lists[0]["items"].as_array().unwrap();
        assert_eq!((items[0]["code"].as_str(), items[0]["text"].as_str()), (Some("P0420"), Some("not in the HDS database")),
            "no description: the database-absence sentence, not an empty text");
        assert_eq!(items[1]["code"], "C1234-49", "the failure-type byte joins the code");
        assert_eq!(items[1]["text"], "Catalyst system efficiency below threshold", "tidy's sentence case");
        // a system whose tests all still wait keeps only the second list
        let waiting = SelfTestSys { at_start: 2, now: 2, incomplete: vec![item("P0300")], ..Default::default() };
        let j = selftests_json(&waiting);
        let lists = j["lists"].as_array().unwrap();
        assert_eq!(lists.len(), 1);
        assert_eq!(lists[0]["title"], "Still waiting");
    }
}
