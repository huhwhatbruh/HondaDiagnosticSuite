//! What the scan result says about the drive about to be recorded: a simulated car, or systems every CAN Honda has that
//! did not answer. Both are shown as toasts the person has time to read; the backend's `missing` / `simulated` come from
//! `hondascan/src/preflight.rs`.
use crate::api::ScanResult;

pub struct Note { pub title: String, pub body: String }

pub fn simulated(s: &ScanResult) -> Option<Note> {
    s.simulated.then(|| Note {
        title: "Simulated car".into(),
        body: "Every reading here is made up and is filed under a made-up VIN. To record your car, pick its adapter and connect.".into(),
    })
}

pub fn missing(s: &ScanResult) -> Option<Note> {
    if s.missing.is_empty() { return None; }
    let names: Vec<&str> = s.missing.iter().map(|m| m.name.as_str()).collect();
    let list = match names.as_slice() {
        [one] => one.to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
        [] => unreachable!(),
    };
    Some(Note {
        title: format!("{list} did not answer"),
        body: format!("Every CAN Honda has {}, so this drive will have no data from {}. Ignition on and engine running? Press Scan again.",
            if names.len() == 1 { "it" } else { "them" }, if names.len() == 1 { "it" } else { "them" }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::MissingSystem;

    fn scan(simulated: bool, missing: &[&str]) -> ScanResult {
        ScanResult { simulated, missing: missing.iter().map(|n| MissingSystem { address: 0x28, name: n.to_string() }).collect(), ..Default::default() }
    }

    #[test]
    fn a_simulated_scan_is_called_simulated_and_a_real_one_is_not() {
        assert!(simulated(&scan(true, &[])).is_some_and(|n| n.title == "Simulated car"));
        assert!(simulated(&scan(false, &[])).is_none());
    }

    #[test]
    fn the_silent_systems_are_named() {
        assert!(missing(&scan(false, &[])).is_none());
        assert_eq!(missing(&scan(false, &["ABS/VSA"])).unwrap().title, "ABS/VSA did not answer");
        assert_eq!(missing(&scan(false, &["ABS/VSA", "EPS"])).unwrap().title, "ABS/VSA and EPS did not answer");
        assert_eq!(missing(&scan(false, &["ABS/VSA", "EPS", "SRS"])).unwrap().title, "ABS/VSA, EPS and SRS did not answer");
    }
}
