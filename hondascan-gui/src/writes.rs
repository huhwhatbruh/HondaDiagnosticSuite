//! Writing functions: the ones the backend offers for a system (hondascan write.rs, each cited to the HDS function that
//! defines it), the dialog that asks before every run, and the result notice. This changes stored data in the car's computer,
//! so it is deliberately heavier than a test: the car's VIN has to be known, the box has to be ticked, and the backend checks
//! HDS's own conditions and reads the old value before it writes anything. This module builds nothing for the backend unless
//! the VIN is known, the box was ticked and the function is one the backend offers for this system.
use crate::api::{Ecu, WriteFn, WriteResult};
use serde_json::{json, Value};

/// The write functions for this system (the database system name), in the backend's order.
pub fn for_system<'a>(writes: &'a [WriteFn], ecu: &Ecu) -> Vec<&'a WriteFn> {
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

/// A finished run's line for a notice.
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

/// The VIN as the dialog needs it: known, or not (an empty or "UNKNOWN" VIN is not known).
fn known(vin: Option<&str>) -> Option<&str> { vin.filter(|v| !v.is_empty() && *v != "UNKNOWN") }

/// The Writes tab: each function with its timing, conditions, read-back and source, and the run dialog's words.
pub fn writes_json(ecu: &Ecu, writes: &[WriteFn], vin: Option<&str>) -> Value {
    let vin = known(vin);
    let list: Vec<Value> = for_system(writes, ecu).iter().map(|w| json!({
        "key": w.key, "name": w.name, "when": when(w),
        "no_stop": if w.stop.is_empty() { "HDS sends no stop" } else { "" },
        "conditions": w.conditions, "read_back": format!("Read-back: {}", w.read_back), "source": format!("HDS: {}", w.source),
        "can_run": vin.is_some(),
        "dialog": {
            "title": format!("Run “{}”?", w.name),
            "body": format!("This sends a routine HDS itself uses to the car's computer ({}); it changes stored adaptation data. It has been \
                tested against a fake ECU only and has never run on a car, so treat the outcome as unproven until the read-back says otherwise.", ecu.display),
            "sends": if w.kind == "routine_status" { format!("Sends: {} · monitor {}", w.start, w.monitor) }
                else { format!("Sends: {} (read-back DID 0x{:04X} byte {}, mask 0x{:02X})", w.start, w.did, w.byte, w.mask) },
            "reads_back": format!("Reads back: {}", w.read_back),
            "stop_note": if w.stop.is_empty() { "HDS sends no stop for this one; it polls the read-back until the routine reports done." } else { "" },
            "conditions_lead": "The conditions are checked from live data before anything is sent:",
            "vin_line": match vin {
                Some(v) => format!("The car's VIN is {v}; a mismatch refuses before anything is sent."),
                None => "The VIN was not read: run a scan first. Without it the backend will refuse.".into(),
            },
        },
    })).collect();
    json!({
        "writes": list,
        "none": "No write functions are offered for this module. Only functions HDS itself defines are ever listed.",
        "intro": "These change data stored in the car's computer (adaptation values). Each one asks first, checks the conditions HDS's own \
            function checks, reads the old value and only then sends the routine the car's computer already knows. Nothing here has run on a car yet.",
        "tick": "I understand this writes stored data to the car, the car is parked with the ignition on, and I accept that this \
            function has never been run on a car",
        "vin_known": vin.is_some(),
    })
}

/// The request that runs a write, or why it is not built: the VIN the scan read (the backend re-reads the ECU's VIN and refuses a
/// mismatch), the ticked box, and a function the backend offers for this system. The backend refuses without `confirm: true`,
/// checks HDS's conditions, keeps the old value and logs every frame.
pub fn run_params(ecu: &Ecu, writes: &[WriteFn], key: &str, vin: Option<&str>, ticked: bool) -> Result<Value, String> {
    if !ticked { return Err("The box was not ticked: nothing was sent.".into()); }
    let Some(vin) = known(vin) else { return Err("The VIN was not read: run a scan first. Nothing was sent.".into()) };
    let w = for_system(writes, ecu).into_iter().find(|w| w.key == key).ok_or("This system has no such function: nothing was sent.")?;
    Ok(json!({ "address": ecu.address, "key": w.key, "vin": vin, "confirm": true }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(key: &str, system: &str) -> WriteFn { WriteFn { key: key.into(), system: system.into(), ..Default::default() } }
    fn pcm() -> Ecu { Ecu { address: 0x0E, system: "CANFI".into(), display: "PGM-FI".into(), ..Default::default() } }

    #[test]
    fn only_the_writes_of_this_system_are_listed() {
        let all = [w("evtc_learn", "CANFI"), w("ckp_learn", "CANFI"), w("abs_something", "CANABS")];
        let keys: Vec<&str> = for_system(&all, &pcm()).iter().map(|w| w.key.as_str()).collect();
        assert_eq!(keys, ["evtc_learn", "ckp_learn"]);
        assert!(for_system(&all, &Ecu::default()).is_empty(), "a module without a database system has no writes");
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

    #[test]
    fn nothing_is_built_for_the_backend_without_the_vin_the_tick_and_a_function_of_this_system() {
        let all = [w("evtc_learn", "CANFI"), w("abs_something", "CANABS")];
        let vin = Some("1HGCR2F3XFA000000");
        assert!(run_params(&pcm(), &all, "evtc_learn", vin, false).is_err(), "unticked: nothing is sent");
        for bad in [None, Some(""), Some("UNKNOWN")] { assert!(run_params(&pcm(), &all, "evtc_learn", bad, true).is_err(), "no known VIN, no write: {bad:?}"); }
        assert!(run_params(&pcm(), &all, "abs_something", vin, true).is_err(), "a function of another system is not run from this one");
        assert!(run_params(&pcm(), &all, "nope", vin, true).is_err());
        let p = run_params(&pcm(), &all, "evtc_learn", vin, true).unwrap();
        assert_eq!(p, json!({ "address": 14, "key": "evtc_learn", "vin": "1HGCR2F3XFA000000", "confirm": true }), "confirm is always true in what is sent");
    }

    #[test]
    fn the_run_button_waits_for_a_known_vin_and_the_dialog_says_which_state_it_is_in() {
        let all = [WriteFn { name: "E-VTC learn".into(), start: "31 01 02 18 02".into(), kind: "routine_status".into(), monitor: "22 7F00".into(),
            timeout_s: 8.0, ..w("evtc_learn", "CANFI") }];
        let no_vin = writes_json(&pcm(), &all, None);
        assert_eq!((no_vin["writes"][0]["can_run"].as_bool(), no_vin["vin_known"].as_bool()), (Some(false), Some(false)));
        assert!(no_vin["writes"][0]["dialog"]["vin_line"].as_str().unwrap().starts_with("The VIN was not read"));
        let with = writes_json(&pcm(), &all, Some("1HGCR2F3XFA000000"));
        assert_eq!(with["writes"][0]["can_run"], true);
        assert_eq!(with["writes"][0]["dialog"]["sends"], "Sends: 31 01 02 18 02 · monitor 22 7F00");
        assert_eq!(with["writes"][0]["no_stop"], "HDS sends no stop");
        assert_eq!(with["writes"][0]["when"], "sends 31 01 02 18 02 · then polls 22 7F00 for up to 8 s");
    }
}
