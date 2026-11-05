//! The Condition card (backend `condition` reply, hondascan/src/condition.rs): findings from the newest recorded drive
//! against this car's earlier drives. Pure data; QML draws the JSON `card_json` builds. The backend works in SI units and
//! this card shows US units, so pressure and speed are converted here.
use serde_json::{json, Value};

const OK: i64 = 4;
const WARN: i64 = 1;
const FAULT: i64 = 2;
const MUTED: i64 = 3;

pub const INHG_PER_KPA: f64 = 0.2953;
#[cfg(test)]
pub const MPH_PER_KMH: f64 = 0.621371;
#[cfg(test)]
pub fn f_of_c(c: f64) -> f64 { c * 9.0 / 5.0 + 32.0 }

/// The valid state the backend gates on, in US units (numbers from condition.rs, checked in text_audit.rs).
pub const GATE_LINE: &str = "Counted only with the engine warm (167 to 212 °F), at idle (500 to 1000 rpm, stopped) or in steady cruise from 25 mph, and no trouble codes.";

/// "99.0 kPa" -> "29.23 inHg", "60-110 kPa" -> "17.72-32.48 inHg": every number ahead of the unit.
pub fn us_units(text: &str) -> String {
    let words: Vec<&str> = text.split(' ').collect();
    let mut out: Vec<String> = Vec::with_capacity(words.len());
    for (i, w) in words.iter().enumerate() {
        let next_is_kpa = words.get(i + 1).is_some_and(|n| n.trim_end_matches([',', ';', ')', '.']) == "kPa");
        if next_is_kpa {
            let (lead, num) = match w.split_once('-') { Some((a, b)) => (Some(a), b), None => (None, *w) };
            let conv = |s: &str| s.parse::<f64>().ok().map(|v| format!("{:.2}", v * INHG_PER_KPA));
            if let (Some(c), lead_c) = (conv(num), lead.map(conv)) {
                out.push(match (lead, lead_c) { (Some(_), Some(Some(a))) => format!("{a}-{c}"), _ => c });
                continue;
            }
        }
        out.push(if w.trim_end_matches([',', ';', ')', '.']) == "kPa" { w.replace("kPa", "inHg") } else { (*w).to_string() });
    }
    out.join(" ")
}

fn tone(verdict: &str) -> i64 {
    match verdict { "normal" => OK, "watch" => WARN, "concern" => FAULT, _ => MUTED }
}

fn label(verdict: &str) -> &'static str {
    match verdict { "normal" => "Normal", "watch" => "Watch", "concern" => "Concern", _ => "Not enough data" }
}

/// The card. `reply` is the backend's `condition` result (or its error text).
pub fn card_json(reply: Option<&Value>, err: Option<&str>) -> Value {
    let Some(r) = reply else {
        return json!({"state": if err.is_some() { "none" } else { "loading" }, "text": err.map(|e| e.to_string()).unwrap_or_default(), "gate": GATE_LINE, "findings": []});
    };
    let findings: Vec<Value> = r["findings"].as_array().map(Vec::as_slice).unwrap_or(&[]).iter().map(|f| {
        let s = |k: &str| us_units(f[k].as_str().unwrap_or(""));
        let v = f["verdict"].as_str().unwrap_or("");
        json!({"id": f["id"].as_str().unwrap_or(""), "title": s("title"), "verdict": label(v), "tone": tone(v), "confidence": f["confidence"].as_str().unwrap_or(""), "hypothesis": f["hypothesis"] == true,
               "saw": s("saw"), "why": s("why"), "action": s("action"), "source": f["source"].as_str().unwrap_or("")})
    }).collect();
    // the headline counts the checks; patterns (x_*) and codes (code_*) are read from the cards below it
    let raised = findings.iter().filter(|f| (f["tone"] == WARN || f["tone"] == FAULT) && !f["id"].as_str().unwrap_or("").starts_with("x_") && !f["id"].as_str().unwrap_or("").starts_with("code_")).count();
    let earlier = r["earlier_drives"].as_u64().unwrap_or(0);
    let needed = r["needed"].as_u64().unwrap_or(3);
    let headline = if let Some(n) = r["note"].as_str() { n.to_string() }
        else if raised > 0 { format!("{raised} {} outside this car's own range", if raised == 1 { "check is" } else { "checks are" }) }
        else if earlier < needed { format!("No finding; {earlier} of {needed} earlier drives recorded, so most checks use fixed limits") }
        else { "Every check is within this car's own range".to_string() };
    json!({"state": "ok", "headline": headline, "tone": if raised > 0 { WARN } else { OK }, "gate": GATE_LINE, "earlier": earlier, "needed": needed,
           "idle_min": (r["gate"]["idle_s"].as_f64().unwrap_or(0.0) / 60.0 * 10.0).round() / 10.0, "cruise_min": (r["gate"]["cruise_s"].as_f64().unwrap_or(0.0) / 60.0 * 10.0).round() / 10.0,
           "findings": findings})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pressure_is_shown_in_inches_of_mercury() {
        assert_eq!(us_units("barometric pressure 99.0 kPa, in range"), "barometric pressure 29.23 inHg, in range");
        assert_eq!(us_units("outside 60-110 kPa, which"), "outside 17.72-32.48 inHg, which");
        assert_eq!(us_units("spanning 5 kPa (have"), "spanning 1.48 inHg (have");
        assert_eq!(us_units("no units here"), "no units here");
    }

    #[test]
    fn the_card_names_the_state_and_keeps_every_finding() {
        let reply = json!({"earlier_drives": 5, "needed": 3, "gate": {"idle_s": 150.0, "cruise_s": 90.0}, "findings": [
            {"title": "Fuel trim", "verdict": "watch", "confidence": "medium", "saw": "x", "why": "y", "action": "z", "source": "s"},
            {"title": "Knock retard", "verdict": "normal", "confidence": "medium", "saw": "x", "why": "y", "action": "none", "source": "s"},
            {"title": "Valve lash (hypothesis)", "hypothesis": true, "verdict": "insufficient", "confidence": "none", "saw": "", "why": "", "action": "", "source": "s"}]});
        let c = card_json(Some(&reply), None);
        assert_eq!(c["headline"], "1 check is outside this car's own range");
        assert_eq!(c["findings"].as_array().unwrap().len(), 3);
        assert_eq!(c["findings"][0]["tone"], WARN);
        assert_eq!(c["findings"][2]["hypothesis"], true);
        assert_eq!(c["idle_min"], 2.5);
        assert_eq!(card_json(None, Some("no recorded session"))["state"], "none");
        assert_eq!(card_json(None, None)["state"], "loading");
    }

    #[test]
    fn patterns_and_codes_are_shown_but_not_counted_as_checks() {
        let reply = json!({"earlier_drives": 5, "needed": 3, "gate": {}, "findings": [
            {"id": "fuel_trim", "title": "Fuel trim", "verdict": "watch", "confidence": "medium", "saw": "x", "why": "y", "action": "z", "source": "s"},
            {"id": "x_trim_idle_only", "title": "Fuel trim shifted at idle only", "hypothesis": true, "verdict": "watch", "confidence": "low", "saw": "x", "why": "y", "action": "z", "source": "s"}]});
        let c = card_json(Some(&reply), None);
        assert_eq!(c["headline"], "1 check is outside this car's own range");
        assert_eq!(c["findings"].as_array().unwrap().len(), 2);
        assert_eq!(c["findings"][1]["hypothesis"], true);
    }

    #[test]
    fn a_car_with_codes_says_why_nothing_was_judged() {
        let reply = json!({"earlier_drives": 5, "needed": 3, "gate": {}, "findings": [], "note": "trouble codes were present, so no inference is made"});
        assert!(card_json(Some(&reply), None)["headline"].as_str().unwrap().starts_with("trouble codes"));
    }
}
