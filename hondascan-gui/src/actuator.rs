//! Bi-directional tests: the ones the backend offers for a system (hondascan actuator.py, each cited to the HDS function that
//! defines it), the dialog that asks before every start, and the bar that is on screen for as long as one runs, with Stop.
//! The app only shows the rules; the backend enforces them (confirm, HDS's conditions, the time limit, control handed back).
//! What this module adds on top is that nothing is built for the backend unless the person ticked the box and the value is
//! one HDS sends for that test.
use crate::api::{ActStatus, ActTest, Ecu};
use serde_json::{json, Value};

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

/// The tests of `ecu` as the Tests tab draws them: each with its two buttons, conditions, where it comes from, and the
/// confirmation dialog's words for each value. `busy` disables the buttons while a test runs.
pub fn tests_json(ecu: &Ecu, tests: &[ActTest], act: Option<&ActStatus>) -> Value {
    let busy = act.is_some_and(|a| a.active);
    let list: Vec<Value> = for_system(tests, ecu).iter().map(|t| json!({
        "key": t.key, "name": t.name, "did": format!("0x{:04X}", t.did), "hold": format!("up to {:.0} s", t.hold_s),
        "conditions": t.conditions,
        "effect": if t.state.is_empty() { String::new() } else { format!("See the effect in Live data: {}", t.state.join(", ")) },
        "source": format!("HDS: {}", t.source),
        "values": t.values.iter().rev().map(|v| json!({
            "value": v, "label": format!("Switch {}…", value_word(t, *v)),
            "title": format!("Switch the {} {}?", t.name.to_lowercase(), value_word(t, *v)),
            "body": format!("The car’s own computer ({}) will be told to drive this part for up to {:.0} seconds, then it takes over again. \
                The conditions below are checked before it starts and while it runs; if one stops holding, the test ends.", ecu.display, t.hold_s),
        })).collect::<Vec<_>>(),
    })).collect();
    json!({
        "tests": list, "busy": busy,
        "none": "No bi-directional tests are offered for this module. Only tests that HDS itself defines, each checked against its own conditions, are ever listed.",
        "intro": "These make the car move or switch something so you can see it work. Each one asks first, runs for a limited time, stops itself, and gives control back to the car if anything goes wrong or the adapter is unplugged.",
        "tick": "The car is stationary and parked, nobody is near the part that will move, and I can stop the test at any time",
    })
}

/// The running test as the bar under the header shows it, or null when none runs.
pub fn running_json(act: Option<&ActStatus>, tests: &[ActTest]) -> Value {
    let Some(a) = act.filter(|a| a.active) else { return Value::Null };
    let what = tests.iter().find(|t| t.key == a.test).map_or_else(|| format!("(sent {})", a.value), |t| format!("switched {}", value_word(t, a.value)));
    json!({ "text": format!("Test running: {} {} · {}", a.name, what, countdown(a)) })
}

/// The request that starts a test, or why it is not built. The person must have ticked the box (the dialog's button is off
/// until then; this is the second lock), the test must be one the backend offers for this system, and the value one HDS sends
/// for it. The backend refuses without `confirm: true`, checks HDS's conditions, enforces the time limit and hands control back.
pub fn start_params(ecu: &Ecu, tests: &[ActTest], key: &str, value: i64, ticked: bool) -> Result<Value, String> {
    if !ticked { return Err("The box was not ticked: nothing was sent.".into()); }
    let t = for_system(tests, ecu).into_iter().find(|t| t.key == key).ok_or("This system has no such test: nothing was sent.")?;
    if !t.values.contains(&value) { return Err(format!("{value} is not a value HDS sends for this test: nothing was sent.")); }
    Ok(json!({ "address": ecu.address, "test": t.key, "value": value, "confirm": true }))
}

/// The notice when a test ends by itself (the backend's `actuator` event): None while it was not running or has no reason.
pub fn ended(was_active: bool, a: &ActStatus) -> Option<(bool, String, String)> {
    if a.active || a.reason.is_empty() || !was_active { return None; }
    let bad = !a.errors.is_empty();
    Some((bad, format!("{} ended", a.name),
        format!("Control is back with the car: {}.{}", a.reason, if bad { format!(" {}", a.errors.join("; ")) } else { String::new() })))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(key: &str, system: &str) -> ActTest { ActTest { key: key.into(), system: system.into(), values: vec![0, 1], ..Default::default() } }
    fn pcm() -> Ecu { Ecu { address: 0x0E, system: "CANFI".into(), display: "PGM-FI".into(), ..Default::default() } }

    #[test]
    fn only_the_tests_of_this_system_are_listed() {
        let all = [t("radiator_fan", "CANFI"), t("brake", "CANABS"), t("ac_clutch", "CANFI")];
        let keys: Vec<&str> = for_system(&all, &pcm()).iter().map(|t| t.key.as_str()).collect();
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

    #[test]
    fn nothing_is_built_for_the_backend_unless_the_box_is_ticked_and_the_test_and_value_are_known() {
        let all = [t("radiator_fan", "CANFI"), t("brake", "CANABS")];
        assert!(start_params(&pcm(), &all, "radiator_fan", 1, false).is_err(), "unticked: nothing is sent");
        assert!(start_params(&pcm(), &all, "brake", 1, true).is_err(), "a test of another system is not started from this one");
        assert!(start_params(&pcm(), &all, "radiator_fan", 7, true).is_err(), "a value HDS does not send");
        assert!(start_params(&pcm(), &all, "nope", 1, true).is_err());
        let p = start_params(&pcm(), &all, "radiator_fan", 1, true).unwrap();
        assert_eq!(p, json!({ "address": 14, "test": "radiator_fan", "value": 1, "confirm": true }), "confirm is always true in what is sent");
    }

    #[test]
    fn the_tests_tab_names_each_value_and_waits_while_one_runs() {
        let all = [ActTest { name: "Radiator fan".into(), did: 0x2F00, hold_s: 20.0, source: "PGMFI_INSP rad_fan_act".into(),
            conditions: vec!["Vehicle speed 0".into()], state: vec!["FANC".into()], ..t("radiator_fan", "CANFI") }];
        let idle = tests_json(&pcm(), &all, None);
        assert_eq!(idle["busy"], false);
        let first = &idle["tests"][0];
        assert_eq!((first["did"].as_str(), first["hold"].as_str()), (Some("0x2F00"), Some("up to 20 s")));
        assert_eq!(first["values"][0]["label"], "Switch on…", "the values come on first, as the egui buttons did (right to left)");
        assert_eq!(first["values"][0]["title"], "Switch the radiator fan on?");
        assert_eq!(first["effect"], "See the effect in Live data: FANC");
        let running = ActStatus { active: true, remaining: Some(4.2), name: "Radiator fan".into(), test: "radiator_fan".into(), value: 1, ..Default::default() };
        assert_eq!(tests_json(&pcm(), &all, Some(&running))["busy"], true);
        assert_eq!(running_json(Some(&running), &all)["text"], "Test running: Radiator fan switched on · 5 s left");
        assert!(running_json(None, &all).is_null());
        assert!(running_json(Some(&ActStatus::default()), &all).is_null(), "a test that is no longer active shows no bar");
    }

    #[test]
    fn a_test_that_ends_by_itself_is_told_once_with_its_reason() {
        let done = ActStatus { name: "Radiator fan".into(), reason: "time limit".into(), ..Default::default() };
        let (bad, title, body) = ended(true, &done).unwrap();
        assert!(!bad && title == "Radiator fan ended" && body == "Control is back with the car: time limit.");
        assert!(ended(false, &done).is_none(), "it was not running");
        let failed = ActStatus { errors: vec!["no reply".into(), "stop failed".into()], ..done.clone() };
        let (bad, _, body) = ended(true, &failed).unwrap();
        assert!(bad && body.ends_with(" no reply; stop failed"));
        assert!(ended(true, &ActStatus { active: true, ..done }).is_none(), "still running");
    }
}
