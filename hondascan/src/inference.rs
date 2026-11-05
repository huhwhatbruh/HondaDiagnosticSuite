//! Reasoning over the Condition findings: what several findings say together, and what an active trouble code says
//! about this drive's data. Read-only, pure functions of JSON (`condition::evaluate` calls `interpret`).
//!
//! Two kinds of output, both in the same shape as a Condition finding so every front end shows them without change:
//! - **pattern** findings (id `x_*`): a rule over the direction and spread of several features. They are hypotheses
//!   (`"hypothesis": true`, confidence "low"): the data names a pattern, never a part. The rules are ordinary
//!   fuel-trim / misfire / cam-timing diagnostic reasoning, hondascan's own, not Honda's; each says so in its `source`.
//! - **code** findings (id `code_*`): an active (confirmed or pending) trouble code, tied to the Condition check that
//!   watches the same system, and whether this drive's data agrees with it. The code families are the SAE J2012 ones
//!   (the shipped database's `dtc` table carries them with Honda's descriptions: P0011 "VTC A Bank1 System
//!   Malfunction/Timing Over-Advanced", P0171 "FUEL SYSTEM TOO LEAN", P0069 "BAROMETRIC PRESSURE (BARO) SENSOR ...").
//!   A code is the PCM's own verdict; the data only corroborates or fails to.
use crate::condition::{fixed_limit, level};
use serde_json::{json, Value};

/// Idle misfires needed before the cylinder spread is read, and the share one cylinder needs to be "the" cylinder.
const SPREAD_MIN: u64 = 5;
const ONE_CYL_SHARE: f64 = 0.7;
/// Checks that moved together before a common cause is suggested.
const COMMON_CAUSE_MIN: usize = 3;

fn finding<'a>(fs: &'a [Value], id: &str) -> Option<&'a Value> {
    fs.iter().find(|f| f["id"] == id)
}

fn raised(fs: &[Value], id: &str) -> bool {
    finding(fs, id).is_some_and(|f| matches!(f["verdict"].as_str(), Some("watch" | "concern")))
}

fn evidence<'a>(fs: &'a [Value], feature: &str) -> Option<&'a Value> {
    fs.iter().filter_map(|f| f["evidence"].as_array()).flatten().find(|e| e["feature"] == feature)
}

/// (level 0..2, direction) of one feature: level from the car's own history when it has some, else hondascan's fixed
/// limit (level 1 at most); direction is the sign of (now - baseline), or of (now - neutral) without a baseline.
fn shift(fs: &[Value], feature: &str) -> Option<(u8, f64)> {
    let e = evidence(fs, feature)?;
    let now = e["now"].as_f64()?;
    let lvl = if e.get("z").is_some() { level(e) } else { u8::from(fixed_limit(e)) };
    let base = e["baseline"].as_f64().unwrap_or(if feature.ends_with("klafave") { 1.0 } else { 0.0 });
    Some((lvl, now - base))
}

fn flagged(fs: &[Value], feature: &str) -> Option<f64> {
    shift(fs, feature).filter(|(l, _)| *l > 0).map(|(_, d)| d)
}

fn steady(fs: &[Value], feature: &str) -> bool {
    shift(fs, feature).is_some_and(|(l, _)| l == 0)
}

fn pattern(id: &str, title: &str, verdict: &str, saw: String, why: &str, action: &str, uses: &[&str]) -> Value {
    json!({"id": id, "title": title, "hypothesis": true, "verdict": verdict, "confidence": "low", "saw": saw, "why": why, "action": action,
           "evidence": [], "uses": uses,
           "source": "hondascan's reasoning over the findings listed in `uses` (standard fuel-trim, misfire and cam-timing diagnosis); not a Honda procedure"})
}

/// Fuel trim read by regime. Idle-only positive = unmetered air, which matters most at low flow; the same shift at all
/// loads = the fuel or the airflow measurement is off at every flow; cruise-only = trouble that shows at high flow.
fn trim_pattern(fs: &[Value]) -> Option<Value> {
    let (idle, cruise) = (flagged(fs, "idle.klafave"), flagged(fs, "cruise.klafave"));
    let dir = |d: f64| if d > 0.0 { "more fuel than before" } else { "less fuel than before" };
    let uses = ["fuel_trim"];
    match (idle, cruise) {
        (Some(i), None) if steady(fs, "cruise.klafave") && i > 0.0 => Some(pattern(
            "x_trim_idle_only", "Fuel trim shifted at idle only", "watch",
            "the long-term trim adds fuel at idle but not at cruise".into(),
            "air that bypasses the airflow measurement matters most when total flow is small, so a leak shows at idle and fades at cruise (a hypothesis)",
            "look for an intake or vacuum leak after the airflow sensor; repeat the drive before acting", &uses)),
        (Some(i), Some(c)) if i.signum() == c.signum() => Some(pattern(
            "x_trim_all_loads", "Fuel trim shifted at every load", "watch",
            format!("the long-term trim takes {} at idle and at cruise", dir(i)),
            "a shift that does not fade with load points at the fuel supply or the airflow reading itself rather than at one leak (a hypothesis)",
            "check fuel pressure and the airflow sensor; note the fuel used", &uses)),
        (None, Some(c)) if steady(fs, "idle.klafave") && c > 0.0 => Some(pattern(
            "x_trim_cruise_only", "Fuel trim shifted at cruise only", "watch",
            "the long-term trim adds fuel at cruise but not at idle".into(),
            "a supply that keeps up at idle but not at higher flow fits a weak pump, a restricted filter or a small injector flow loss (a hypothesis)",
            "check fuel pressure under load", &uses)),
        _ => None,
    }
}

/// Misfire pattern: which cylinder(s) the idle counters name, and whether a lean trim sits beside them.
fn misfire_pattern(fs: &[Value], now: &Value) -> Option<Value> {
    if !raised(fs, "misfire") {
        return None;
    }
    let by_cyl: Vec<(String, u64)> = now["misfire_by_cyl"].as_object()?.iter().filter_map(|(k, v)| Some((k.clone(), v.as_u64()?))).collect();
    let total: u64 = by_cyl.iter().map(|(_, n)| n).sum();
    let (top, top_n) = by_cyl.iter().max_by_key(|(_, n)| *n).map(|(c, n)| (c.clone(), *n))?;
    let lean = flagged(fs, "idle.klafave").is_some_and(|d| d > 0.0);
    if total >= SPREAD_MIN && top_n as f64 / total as f64 >= ONE_CYL_SHARE {
        return Some(pattern(
            "x_misfire_one_cylinder", &format!("Misfires concentrated on cylinder {top}"), "watch",
            format!("{top_n} of {total} idle misfires are on cylinder {top}"),
            "one cylinder misfiring while the others run clean points at what only that cylinder has: its plug, coil or injector (a hypothesis)",
            &format!("inspect cylinder {top}'s plug and coil first; swapping the coil with a neighbour's shows whether the misfire follows it"), &["misfire"]));
    }
    let cylinders = by_cyl.iter().filter(|(_, n)| *n > 0).count();
    if total >= SPREAD_MIN && cylinders >= 3 {
        let lean_note = if lean { ", and the long-term trim is adding fuel (a lean mixture misfires on every cylinder)" } else { "" };
        return Some(pattern(
            "x_misfire_spread", "Misfires spread over several cylinders", "watch",
            format!("{total} idle misfires over {cylinders} cylinders, none above {:.0}%{lean_note}", ONE_CYL_SHARE * 100.0),
            "misfires on many cylinders at once point at something they share (fuel, air, spark supply), not at one coil (a hypothesis)",
            "check the fuel and air side (trim, leaks, fuel quality) before replacing coils", &["misfire", "fuel_trim"]));
    }
    None
}

/// Idle speed unsteady while the PCM counts no extra misfires.
fn roughness_pattern(fs: &[Value]) -> Option<Value> {
    flagged(fs, "idle.ne_sd").filter(|d| *d > 0.0)?;
    if flagged(fs, "idle.misfire_per_min").is_some() {
        return None;
    }
    let leak = flagged(fs, "idle.klafave").is_some_and(|d| d > 0.0);
    Some(pattern(
        "x_idle_rough_no_misfire", "Idle speed unsteady without counted misfires", "watch",
        format!("the idle speed varies more than before while the misfire counters stay in range{}", if leak { ", and the long-term trim adds fuel at idle" } else { "" }),
        "an idle that hunts without misfires fits the idle air control, a carbon-coated throttle body or a vacuum leak better than ignition (a hypothesis)",
        "inspect the throttle body and the intake for leaks", &["misfire", "fuel_trim"]))
}

/// Barometric pressure moved away from the car's usual and a trim moved too: the weather or the altitude is context.
fn baro_context(fs: &[Value]) -> Option<Value> {
    shift(fs, "idle.baro").filter(|(l, _)| *l > 0)?;
    if !raised(fs, "fuel_trim") {
        return None;
    }
    Some(pattern(
        "x_baro_context", "Pressure differs from earlier drives", "watch",
        "the barometric pressure at idle is outside this car's usual range while a fuel trim is too".into(),
        "a different altitude or a weather front changes air density, which the trim partly absorbs; the trim finding is weaker evidence on this drive (a hypothesis)",
        "compare with a drive at your usual pressure before acting on the trim", &["baro", "fuel_trim"]))
}

/// Cam timing: where the deviation shows (idle = lowest oil pressure), and whether knock moved with it.
fn cam_pattern(fs: &[Value]) -> Option<Value> {
    let idle = flagged(fs, "idle.vtc_dev").is_some();
    let cruise = flagged(fs, "cruise.vtc_dev").is_some() || flagged(fs, "cruise.exvtc_dev").is_some();
    let knock = raised(fs, "knock");
    if idle && !cruise && steady(fs, "cruise.vtc_dev") && !knock {
        return Some(pattern(
            "x_cam_idle_only", "Cam tracking worse at idle only", "watch",
            "the intake cam follows its command less closely at idle but normally at cruise".into(),
            "the VTC actuator is driven by engine oil pressure, which is lowest at idle, so a deviation that vanishes off idle fits low oil pressure or a sticky actuator (a hypothesis)",
            "check the oil level and grade first; HDS's VTC test is the vendor's next step (not run from here)", &["vtc"]));
    }
    if (idle || cruise) && knock {
        return Some(pattern(
            "x_cam_and_knock", "Cam tracking and knock moved together", "watch",
            "the cam angle deviation and the knock retard are both outside this car's range".into(),
            "two timing-related signals moving together is more likely one cause than two (a hypothesis): the fuel, the oil or the engine's mechanical condition",
            "note the fuel and the oil level; repeat the drive", &["vtc", "knock"]));
    }
    None
}

/// Several checks moved at once.
fn common_cause(fs: &[Value]) -> Option<Value> {
    let moved: Vec<&str> = ["fuel_trim", "misfire", "knock", "vtc"].into_iter().filter(|id| raised(fs, id)).collect();
    (moved.len() >= COMMON_CAUSE_MIN).then(|| {
        pattern(
            "x_common_cause", "Several checks moved at once", "watch",
            format!("{} are all outside this car's own range", moved.join(", ")),
            "three or more systems drifting in one drive point at something they share (fuel quality, a recent service, the oil) more than at separate faults (a hypothesis)",
            "think about what changed since the earlier drives (fuel, service, weather) before chasing each check", &moved)
    })
}

// ---- trouble codes -----------------------------------------------------------------------------------------------------

/// (Condition check id, what the code says) for the SAE families this layer has data for. A code outside them has no
/// signal here to corroborate it, so it is not listed.
fn family(code: &str) -> Option<(&'static str, String)> {
    let n: u32 = code.strip_prefix('P').and_then(|d| d.parse().ok())?; // P0171 -> 171: the four digits are decimal
    Some(match n {
        171 | 174 => ("fuel_trim", "fuel system too lean".into()),
        172 | 175 => ("fuel_trim", "fuel system too rich".into()),
        300 => ("misfire", "random or multiple cylinder misfire".into()),
        301..=312 => ("misfire", format!("misfire on cylinder {}", n - 300)),
        324..=334 => ("knock", "knock sensor or knock control".into()),
        10..=25 => ("vtc", "cam phasing (VTC) or cam/crank correlation".into()),
        69 | 106..=109 => ("baro", "barometric or manifold pressure sensor".into()),
        _ => return None,
    })
}

fn active_codes(now: &Value) -> Vec<&Value> {
    now["codes"].as_array().map(|a| a.iter().filter(|c| matches!(c["kind"].as_str(), Some("confirmed" | "pending"))).collect()).unwrap_or_default()
}

fn code_finding(c: &Value, fs: &[Value], now: &Value) -> Option<Value> {
    let code = c["code"].as_str()?;
    let (check, meaning) = family(code)?;
    let confirmed = c["kind"] == "confirmed";
    let f = finding(fs, check);
    let verdict_of = f.and_then(|f| f["verdict"].as_str()).unwrap_or("insufficient");
    let mut supported = matches!(verdict_of, "watch" | "concern");
    let mut agree = match verdict_of {
        "watch" | "concern" => "this drive's data agrees: ".to_string() + f.and_then(|f| f["saw"].as_str()).unwrap_or(""),
        "normal" => "this drive's data on that system is within this car's range, so the fault may be intermittent or need conditions this drive did not reach".into(),
        _ => "this drive has too little in-gate data on that system to say".into(),
    };
    // direction and cylinder checks sharpen the statement
    if check == "fuel_trim" {
        let want_lean = meaning.contains("lean");
        let d = flagged(fs, "idle.klafave").or_else(|| flagged(fs, "cruise.klafave"));
        match d {
            Some(d) if (d > 0.0) != want_lean => {
                supported = false;
                agree = format!("the trim moved the opposite way ({}) from what the code says; the code may date from an earlier condition", if d > 0.0 { "adding fuel" } else { "removing fuel" });
            }
            Some(_) => {
                supported = true;
                agree = format!("this drive's long-term trim moved the same way as the code ({})", if want_lean { "adding fuel" } else { "removing fuel" });
            }
            None => {}
        }
    }
    if let Some(cyl) = code.strip_prefix("P03").and_then(|s| s.parse::<u32>().ok()).filter(|n| (1..=12).contains(n)) {
        if let Some((top, n)) = now["misfire_by_cyl"].as_object().and_then(|m| m.iter().filter_map(|(k, v)| Some((k, v.as_u64()?))).max_by_key(|(_, n)| *n)).filter(|(_, n)| *n > 0) {
            supported = top == &cyl.to_string();
            agree = if supported { format!("the idle misfire counters name the same cylinder ({n} on cylinder {top})") } else { format!("the idle misfire counters name cylinder {top} ({n}), not cylinder {cyl}") };
        }
    }
    let desc = c["description"].as_str().unwrap_or("");
    Some(json!({"id": format!("code_{code}"), "title": format!("{code}: {}", if desc.is_empty() { &meaning } else { desc }),
                "verdict": if confirmed { "concern" } else { "watch" }, "confidence": if supported { "medium" } else { "low" },
                "saw": format!("{} code, {meaning}; {agree}", c["kind"].as_str().unwrap_or("")),
                "why": "the PCM set this code itself; this drive's data is read only to see whether it points the same way",
                "action": if supported { "the code and the data agree: follow the vendor's troubleshooting for this code" } else { "follow the vendor's troubleshooting for this code; the drive data does not settle it" },
                "evidence": [], "check": check,
                "source": "trouble code from the PCM (SAE J2012 family; description from the shipped database `dtc` table); corroboration by hondascan's reasoning"}))
}

/// Code findings, then pattern findings. `fs` are the Condition findings (already computed for this drive).
pub fn interpret(now: &Value, fs: &[Value]) -> Vec<Value> {
    let mut out: Vec<Value> = active_codes(now).into_iter().filter_map(|c| code_finding(c, fs, now)).collect();
    out.extend([trim_pattern(fs), misfire_pattern(fs, now), roughness_pattern(fs), baro_context(fs), cam_pattern(fs), common_cause(fs)].into_iter().flatten());
    out
}

/// Active codes this layer has no signal for (shown as a count, not as findings).
pub fn unlinked_codes(now: &Value) -> usize {
    active_codes(now).into_iter().filter(|c| c["code"].as_str().and_then(family).is_none()).count()
}
