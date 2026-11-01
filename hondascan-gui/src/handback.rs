//! Handing control back to the car after a write or a test was interrupted: whether the app must ask the backend to do it, and what it
//! tells the person about the outcome (at start-up, on connecting, and after the backend died mid-operation). Pure functions, so the
//! decisions that matter most when something goes wrong are tested apart from the QObject that sends the request.
use crate::api::{Pending, Recovery};

/// The levels the toasts have (the model's INFO / SUCCESS / WARN / ERROR).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Level { Info = 0, Success = 1, Warn = 2, Error = 3 }

#[derive(Debug, PartialEq)]
pub struct Notice { pub level: Level, pub title: String, pub body: String, pub seconds: f64 }

fn notice(level: Level, title: &str, body: impl Into<String>, seconds: f64) -> Notice { Notice { level, title: title.into(), body: body.into(), seconds } }

fn sent(r: &Recovery) -> String { if r.actions.is_empty() { String::new() } else { format!(" Sent: {}.", r.actions.join(", ")) } }

fn why_not(r: &Recovery) -> String { r.error.clone().unwrap_or_else(|| "no requests were sent, and the backend did not say why".into()) }

/// The backend exited: the adapter to ask a fresh backend to `recover` on, or None when nothing was in flight (or no adapter is known).
/// The adapter saved in the settings wins over the one in use, as the egui app did.
pub fn recover_adapter(writing: bool, actuating: bool, saved: Option<&str>, current: &str) -> Option<String> {
    if !writing && !actuating { return None; }
    let spec = saved.filter(|s| !s.is_empty()).unwrap_or(current);
    (!spec.is_empty()).then(|| spec.to_string())
}

/// The answer to `recover` (or why there was none).
pub fn after_recover(spec: &str, r: Result<Recovery, String>) -> Notice {
    match r {
        Ok(r) if !r.pending => notice(Level::Info, "Nothing to hand back", format!("{spec} had no interrupted write or test recorded."), 8.0),
        Ok(r) if r.handed_back => notice(Level::Success, "Control handed back to the car", format!("The interrupted operation was ended on {spec}.{}", sent(&r)), 15.0),
        Ok(r) => notice(Level::Error, "Control was not handed back", format!("{spec}: {}. The operation is kept; fix the cause and connect again.", why_not(&r)), 30.0),
        Err(e) => notice(Level::Error, "Could not hand control back", format!("{spec}: {e}. The interrupted operation is kept until a hand-back succeeds."), 30.0),
    }
}

/// The hand-back the backend ran by itself when connecting (a write or test was interrupted when it last stopped); None when none was pending.
pub fn on_connect(r: &Recovery) -> Option<Notice> {
    if !r.pending { return None; }
    Some(if r.handed_back {
        notice(Level::Success, "Control handed back to the car", format!("A write or test was interrupted when the backend last stopped; its hand-back has now run.{}", sent(r)), 15.0)
    } else {
        notice(Level::Error, "Control was not handed back", format!("{}. The interrupted operation is kept; fix the cause and reconnect.", why_not(r)), 30.0)
    })
}

/// At start-up: an operation the last run left in flight (the backend hands control back when it connects).
pub fn interrupted(p: &Pending) -> Notice {
    let what = if p.kind == "actuator" { "a bi-directional test" } else { "a write" };
    let which = if p.name.is_empty() { String::new() } else { format!(" ({})", p.name) };
    let car = if p.vin.is_empty() { "the car".to_string() } else { p.vin.clone() };
    notice(Level::Warn, "A write was interrupted", format!("The app stopped while {what}{which} was in flight on {car}. The backend hands control back when it connects."), 20.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(pending: bool, handed_back: bool, actions: &[&str], error: Option<&str>) -> Recovery {
        Recovery { pending, handed_back, actions: actions.iter().map(|s| s.to_string()).collect(), error: error.map(String::from) }
    }

    #[test]
    fn a_backend_that_dies_with_nothing_in_flight_asks_for_no_hand_back() {
        assert_eq!(recover_adapter(false, false, Some("gd101"), "sim"), None);
    }

    #[test]
    fn a_write_or_a_test_in_flight_asks_for_a_hand_back_on_the_saved_adapter_else_the_one_in_use() {
        assert_eq!(recover_adapter(true, false, Some("gd101-j2534:/dev/ttyACM0"), "sim"), Some("gd101-j2534:/dev/ttyACM0".into()), "the saved adapter wins");
        assert_eq!(recover_adapter(false, true, Some(""), "elm:/dev/ttyUSB0"), Some("elm:/dev/ttyUSB0".into()), "an empty saved one falls back to the one in use");
        assert_eq!(recover_adapter(true, true, None, ""), None, "with no adapter known there is nothing to connect to");
    }

    #[test]
    fn the_outcome_of_a_hand_back_is_told_as_it_was() {
        assert_eq!(after_recover("sim", Ok(rec(false, false, &[], None))).title, "Nothing to hand back");
        let ok = after_recover("gd101", Ok(rec(true, true, &["stop 2F", "session 10"], None)));
        assert_eq!((ok.level, ok.body.as_str()), (Level::Success, "The interrupted operation was ended on gd101. Sent: stop 2F, session 10."));
        let bad = after_recover("gd101", Ok(rec(true, false, &[], Some("no response"))));
        assert_eq!((bad.level, bad.seconds), (Level::Error, 30.0));
        assert!(bad.body.starts_with("gd101: no response. The operation is kept"), "{}", bad.body);
        assert!(after_recover("gd101", Ok(rec(true, false, &[], None))).body.contains("the backend did not say why"), "an unexplained failure says so");
        let gone = after_recover("gd101", Err("backend exited".into()));
        assert_eq!((gone.level, gone.title.as_str()), (Level::Error, "Could not hand control back"));
        assert!(gone.body.contains("kept until a hand-back succeeds"));
    }

    #[test]
    fn connecting_reports_a_hand_back_only_when_one_was_pending() {
        assert_eq!(on_connect(&rec(false, false, &[], None)), None);
        assert_eq!(on_connect(&rec(true, true, &[], None)).unwrap().level, Level::Success);
        let n = on_connect(&rec(true, false, &[], Some("adapter busy"))).unwrap();
        assert_eq!((n.level, n.body.as_str()), (Level::Error, "adapter busy. The interrupted operation is kept; fix the cause and reconnect."));
    }

    #[test]
    fn start_up_names_what_the_last_run_left_in_flight() {
        let p = Pending { kind: "actuator".into(), name: "Radiator fan".into(), vin: "1HGCR2F3XFA000000".into(), ..Default::default() };
        assert!(interrupted(&p).body.starts_with("The app stopped while a bi-directional test (Radiator fan) was in flight on 1HGCR2F3XFA000000."));
        let w = interrupted(&Pending::default());
        assert!(w.body.starts_with("The app stopped while a write was in flight on the car."), "{}", w.body);
    }
}
