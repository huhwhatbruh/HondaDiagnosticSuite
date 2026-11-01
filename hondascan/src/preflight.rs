//! The checks that tell a drive that will record the car from one that will not: is the adapter a real one, is the VIN
//! the owner's car, did the systems every CAN Honda has answer. Used by `hondascan preflight`, by the scan's reply
//! (`missing`) and by the session summary, so a silent VSA/ABS is named in all three instead of being one absent row.
//!
//! The core list is what the 2015 Accord has (ABS/VSA 0x28, EPS 0x30, SRS 0x53, METER 0x60; README "Measured on the car")
//! and what HDS's tables give every CAN Honda in the database; a car without one of them gets a note, not a failure of the scan.

use serde_json::{json, Value};

/// Systems expected on a CAN Honda whose engine computer answered: (address, name).
pub const CORE: [(u32, &str); 4] = [(0x28, "ABS/VSA"), (0x30, "EPS"), (0x53, "SRS"), (0x60, "METER")];
const ENGINE: [&str; 2] = ["PGM-FI", "DIESEL"];

/// The simulated cars: `sim`, `sim:drive`, `sim:accord2015-gd101` ...
pub fn simulated(adapter: &str) -> bool {
    adapter == "sim" || adapter.starts_with("sim:")
}

/// The core systems that did not answer. Empty when no engine computer answered either: then there is nothing to say
/// beyond "nothing answered", which the scan already reports.
pub fn missing_core(ecus: &[(u32, &str)]) -> Vec<(u32, &'static str)> {
    if !ecus.iter().any(|(_, t)| ENGINE.contains(t)) {
        return vec![];
    }
    CORE.iter().filter(|(a, _)| !ecus.iter().any(|(x, _)| x == a)).cloned().collect()
}

/// `[{address, name}]` for the scan reply and the summary.
pub fn missing_json(missing: &[(u32, &str)]) -> Value {
    json!(missing.iter().map(|(a, n)| json!({"address": a, "name": n})).collect::<Vec<_>>())
}

pub struct Check {
    pub name: &'static str,
    pub ok: bool,
    pub detail: String,
}

/// `expected_vin`: the car the owner means to test (None: nothing to compare). `wakeup`: who answered the all-ECU wakeup.
pub fn evaluate(adapter: &str, vin: &str, expected_vin: Option<&str>, ecus: &[(u32, &str)], wakeup: Option<&[u32]>) -> Vec<Check> {
    let mut out = vec![];
    let sim = simulated(adapter);
    out.push(Check {
        name: "real adapter",
        ok: !sim,
        detail: if sim { format!("`{adapter}` is the simulated car: whatever it records is not this car") } else { adapter.to_string() },
    });
    let known = !vin.is_empty() && vin != "UNKNOWN";
    out.push(Check { name: "VIN read", ok: known, detail: if known { vin.to_string() } else { "the car's VIN did not come back".into() } });
    if let Some(want) = expected_vin.filter(|v| !v.is_empty()) {
        out.push(Check {
            name: "VIN is the owner's car",
            ok: known && vin == want,
            detail: if known && vin == want { "matches".into() } else { format!("read `{vin}`, expected `{want}`") },
        });
    }
    let engine = ecus.iter().any(|(_, t)| ENGINE.contains(t));
    out.push(Check { name: "engine computer", ok: engine, detail: if engine { "answered".into() } else { "no PGM-FI answered: ignition ON? engine running?".into() } });
    for (a, n) in CORE {
        let ok = ecus.iter().any(|(x, _)| *x == a);
        out.push(Check { name: n, ok, detail: if ok { format!("0x{a:02X} answered") } else { format!("0x{a:02X} did not answer: no {n} data will be recorded") } });
    }
    if let Some(w) = wakeup {
        let who: Vec<String> = w.iter().map(|a| format!("{a:02X}")).collect();
        out.push(Check {
            name: "all-ECU wakeup",
            ok: true,
            detail: if who.is_empty() { "answered by nobody".into() } else { format!("answered by {}", who.join(" ")) },
        });
    }
    out
}

pub fn passed(checks: &[Check]) -> bool {
    checks.iter().all(|c| c.ok)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: [(u32, &str); 5] = [(0x0E, "PGM-FI"), (0x28, "HONDA ABS"), (0x30, "EPS"), (0x53, "SRS"), (0x60, "METER")];

    #[test]
    fn only_the_simulated_adapters_are_simulated() {
        for s in ["sim", "sim:drive", "sim:accord2015-gd101"] {
            assert!(simulated(s), "{s}");
        }
        for s in ["gd101-j2534", "gd101-j2534:/dev/ttyACM0", "elm:/dev/ttyUSB0", "socketcan:can0", "simple"] {
            assert!(!simulated(s), "{s}");
        }
    }

    #[test]
    fn the_four_real_drives_of_2026_10_25_miss_the_vsa_and_the_eps() {
        let got = [(0x0E, "PGM-FI"), (0x0E, "CVT"), (0x53, "SRS"), (0x60, "METER")];
        assert_eq!(missing_core(&got), vec![(0x28, "ABS/VSA"), (0x30, "EPS")]);
        assert!(missing_core(&FULL).is_empty());
    }

    #[test]
    fn nothing_is_said_about_the_core_systems_when_the_engine_computer_is_silent_too() {
        assert!(missing_core(&[]).is_empty());
        assert!(missing_core(&[(0x53, "SRS")]).is_empty());
    }

    #[test]
    fn a_complete_real_scan_passes_and_each_fault_fails_by_name() {
        let ok = evaluate("gd101-j2534", "1HGCR2F30FA000001", Some("1HGCR2F30FA000001"), &FULL, Some(&[0x0E]));
        assert!(passed(&ok), "{:?}", ok.iter().filter(|c| !c.ok).map(|c| c.name).collect::<Vec<_>>());
        let failing = |c: &[Check]| c.iter().filter(|c| !c.ok).map(|c| c.name).collect::<Vec<_>>();
        assert_eq!(failing(&evaluate("sim:drive", "1HGCR2F30FA000001", None, &FULL, None)), ["real adapter"]);
        assert_eq!(failing(&evaluate("gd101-j2534", "UNKNOWN", None, &FULL, None)), ["VIN read"]);
        assert_eq!(failing(&evaluate("gd101-j2534", "1HGCR2F3XFA000000", Some("1HGCR2F30FA000001"), &FULL, None)), ["VIN is the owner's car"]);
        assert_eq!(failing(&evaluate("gd101-j2534", "1HGCR2F30FA000001", None, &FULL[..1], None)), ["ABS/VSA", "EPS", "SRS", "METER"]);
        assert_eq!(failing(&evaluate("gd101-j2534", "1HGCR2F30FA000001", None, &[], None)), ["engine computer", "ABS/VSA", "EPS", "SRS", "METER"]);
    }

    #[test]
    fn no_expected_vin_means_no_vin_comparison() {
        let c = evaluate("gd101-j2534", "1HGCR2F30FA000001", None, &FULL, None);
        assert!(c.iter().all(|c| c.name != "VIN is the owner's car"));
    }
}
