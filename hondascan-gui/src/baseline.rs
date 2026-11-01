//! The reference snapshot card under Health and the save-snapshot dialog's facts: the newest frozen baseline, what changed since,
//! the three actions with the reasons they may be off, and the last report.
//! The screens only draw what `card_json` and `freeze_info_json` say. Nothing here sends anything.
use crate::api::{Ecu, Maintenance, ReportReply};
use crate::vehicle::{baseline_diff, can_freeze, DtcState};
use serde_json::{json, Value};
use std::collections::HashMap;

/// The report verdict in words (rpc.rs `health_report`): what a person should take from it.
pub fn verdict_label(v: &str) -> &'static str {
    match v { "healthy" => "healthy", "attention" => "needs attention", "unknown" => "unknown (not everything could be read)", _ => "no verdict" }
}

fn verdict_tone(v: &str) -> i32 { match v { "healthy" => 4, "attention" => 1, _ => 3 } }

fn plural(n: usize, one: &str, many: &str) -> String { format!("{n} {}", if n == 1 { one } else { many }) }

/// Tones: 0 normal, 1 warn, 2 fault, 3 muted, 4 ok.
pub fn card_json(m: Option<&Maintenance>, err: Option<&str>, ecus: &[Ecu], dtcs: &HashMap<String, DtcState>, vin: Option<&str>,
                 connected: bool, report: Option<&ReportReply>) -> Value {
    let gate = can_freeze(ecus, dtcs, vin);
    let snapshot = m.and_then(|m| m.latest.as_ref().map(|b| {
        let meta = m.baselines.iter().find(|x| x.id == b.id);
        let systems = meta.map_or(b.systems.len() as u32, |x| if x.systems_count > 0 { x.systems_count } else { b.systems.len() as u32 }) as usize;
        let stored = b.totals.stored as usize;
        let d = baseline_diff(b, dtcs);
        let mut parts: Vec<Value> = Vec::new();
        let list = |v: &[(String, crate::api::Dtc)]| v.iter().map(|(s, c)| format!("{} {s}", c.full())).collect::<Vec<_>>().join("\n");
        if !d.new.is_empty() {
            parts.push(json!({ "text": format!("{} new{}", d.new.len(), if d.new_stored() == d.new.len() { " (stored)" } else { "" }),
                "tone": if d.attention() { 2 } else { 1 }, "tip": list(&d.new) }));
        }
        if d.still > 0 { parts.push(json!({ "text": format!("{} still present", d.still), "tone": 3, "tip": "" })); }
        if !d.resolved.is_empty() { parts.push(json!({ "text": format!("{} resolved", d.resolved.len()), "tone": 4, "tip": list(&d.resolved) })); }
        if !d.readiness_changes.is_empty() {
            parts.push(json!({ "text": format!("{} readiness change{}", d.readiness_changes.len(), if d.readiness_changes.len() == 1 { "" } else { "s" }),
                "tone": 1, "tip": d.readiness_changes.join("\n") }));
        }
        json!({
            "created": b.created, "label": b.label,
            "counts": format!("{} · {} stored code{}", plural(systems, "system", "systems"), stored, if stored == 1 { "" } else { "s" }),
            "tampered": meta.is_some_and(|x| !x.integrity.is_empty() && x.integrity != "ok"),
            "tampered_text": "The stored snapshot was modified: its contents no longer match the checksum taken when it was saved, so the comparison below cannot be trusted. Save a new one while the car has no active trouble codes.",
            "no_change": parts.is_empty(), "no_change_text": "No change since the snapshot.", "diff": parts,
        })
    }));
    let report = report.filter(|r| r.vin().is_none() || r.vin() == vin).filter(|r| !r.summary.is_empty() || !r.path.is_empty()).map(|r| json!({
        "text": format!("Last report: {}", verdict_label(&r.verdict)), "tone": verdict_tone(&r.verdict), "summary": r.summary, "path": r.path }));
    json!({
        "err": err.map(|e| format!("Could not load the service record: {e}")),
        "hint_connect": if connected { "" } else { "connect and scan the car to save one" },
        "snapshot": snapshot,
        "none_text": if vin.is_none() { "No snapshot yet. A snapshot records today's trouble codes, readiness and self-tests, so later changes can be told apart from what was always there. It is saved while the car has no active trouble codes, and cannot be edited or removed later." }
            else { "No snapshot yet for this car. Save one while it has no active trouble codes. It records the codes and the readiness." },
        "can_freeze": gate.is_ok(),
        "freeze_hint": match &gate { Err(why) => format!("Not yet: {why}."), Ok(()) => "Stores today's trouble codes, readiness and self-tests as this car's own reference. It cannot be edited or removed later.".into() },
        "can_marker": vin.is_some(),
        "marker_hint": if vin.is_some() { "A note about work done: kind, what was done, and how far the car had gone." } else { "The car's VIN has not been read yet: scan it first." },
        "can_report": connected,
        "report_hint": if connected { "Reads the car now, compares it with the snapshot and writes a printable report." } else { "Connect to the car first: the report reads the codes live." },
        "report": report,
        "kinds": crate::maintenance::KINDS.iter().map(|k| json!({ "id": k, "label": crate::maintenance::kind_label(k) })).collect::<Vec<_>>(),
    })
}

/// The save-snapshot dialog's facts: the VIN line, the self-tests not run, and why saving is blocked (null when it is not).
pub fn freeze_info_json(ecus: &[Ecu], dtcs: &HashMap<String, DtcState>, vin: Option<&str>) -> Value {
    let systems = ecus.len();
    let stored: usize = dtcs.values().filter_map(|s| s.dtcs.as_ref()).flatten().filter(|c| c.kind == "stored").count();
    let not_run: usize = dtcs.values().map(|s| s.incomplete.len()).sum();
    json!({
        "vin_line": format!("VIN {} · {} · {}", vin.unwrap_or(""), plural(systems, "system", "systems"), plural(stored, "stored code", "stored codes")),
        "not_run": if not_run > 0 { format!("{not_run} self-test{} not run yet: recorded as they are (normal on a used car or after a battery \
            disconnect). When they finish, the next report shows it as a readiness change.", if not_run == 1 { " has" } else { "s have" }) } else { String::new() },
        "blocked": can_freeze(ecus, dtcs, vin).err(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{Baseline, BaselineMeta, BaselineSystem, Dtc};

    fn pcm() -> Ecu { Ecu { address: 0x0E, system: "CANFI".into(), system_type: "PGM-FI".into(), ..Default::default() } }
    fn read(dtcs: Vec<Dtc>) -> HashMap<String, DtcState> { HashMap::from([(pcm().key(), DtcState { dtcs: Some(dtcs), ..Default::default() })]) }
    fn code(kind: &str) -> Dtc { Dtc { code: "P0420".into(), failure_type: -1, kind: kind.into(), ..Default::default() } }

    #[test]
    fn with_no_snapshot_the_card_says_so_and_the_gate_explains_a_blocked_save() {
        let vin = Some("1HGCR2F3XFA000000");
        let j = card_json(None, None, &[pcm()], &read(vec![code("confirmed")]), vin, true, None);
        assert!(j["snapshot"].is_null() && j["none_text"].as_str().unwrap().starts_with("No snapshot yet for this car"));
        assert_eq!(j["can_freeze"], false);
        assert!(j["freeze_hint"].as_str().unwrap().starts_with("Not yet: ") && j["freeze_hint"].as_str().unwrap().contains("confirmed"));
        let ok = card_json(None, None, &[pcm()], &read(vec![]), vin, true, None);
        assert_eq!((ok["can_freeze"].as_bool(), ok["can_marker"].as_bool(), ok["can_report"].as_bool()), (Some(true), Some(true), Some(true)));
        let off = card_json(None, None, &[pcm()], &read(vec![]), None, false, None);
        assert_eq!((off["can_freeze"].as_bool(), off["can_marker"].as_bool(), off["can_report"].as_bool()), (Some(false), Some(false), Some(false)));
        assert!(off["none_text"].as_str().unwrap().starts_with("No snapshot yet. A snapshot records"));
    }

    #[test]
    fn the_diff_names_what_is_new_still_there_and_resolved() {
        let base = Baseline { id: 1, created: 1.7e9, label: "Healthy".into(), systems: vec![BaselineSystem { address: 0x0E, system: "CANFI".into(),
            system_type: "PGM-FI".into(), answered: true, dtcs: vec![code("stored")], ..Default::default() }], ..Default::default() };
        let m = Maintenance { latest: Some(base.clone()), baselines: vec![BaselineMeta { id: 1, integrity: "ok".into(), systems_count: 1, ..Default::default() }], ..Default::default() };
        let same = card_json(Some(&m), None, &[pcm()], &read(vec![code("stored")]), Some("V"), true, None);
        assert_eq!(same["snapshot"]["diff"][0]["text"], "1 still present");
        let gone = card_json(Some(&m), None, &[pcm()], &read(vec![]), Some("V"), true, None);
        assert_eq!((gone["snapshot"]["diff"][0]["text"].as_str(), gone["snapshot"]["diff"][0]["tone"].as_i64()), (Some("1 resolved"), Some(4)));
        let worse = card_json(Some(&m), None, &[pcm()], &read(vec![code("stored"), Dtc { code: "P0171".into(), kind: "confirmed".into(), failure_type: -1, ..Default::default() }]), Some("V"), true, None);
        assert_eq!((worse["snapshot"]["diff"][0]["text"].as_str(), worse["snapshot"]["diff"][0]["tone"].as_i64()), (Some("1 new"), Some(2)), "a new confirmed code needs attention");
        let tampered = Maintenance { baselines: vec![BaselineMeta { id: 1, integrity: "changed".into(), ..Default::default() }], latest: Some(base), ..Default::default() };
        assert_eq!(card_json(Some(&tampered), None, &[pcm()], &read(vec![]), Some("V"), true, None)["snapshot"]["tampered"], true);
    }

    #[test]
    fn a_report_of_another_car_is_not_shown_and_the_dialog_counts_what_it_saves() {
        let other = ReportReply { summary: "fine".into(), verdict: "healthy".into(), report: json!({ "vin": "OTHERVIN" }), ..Default::default() };
        assert!(card_json(None, None, &[pcm()], &read(vec![]), Some("MYVIN"), true, Some(&other))["report"].is_null());
        let mine = ReportReply { report: json!({ "vin": "MYVIN" }), ..other };
        assert_eq!(card_json(None, None, &[pcm()], &read(vec![]), Some("MYVIN"), true, Some(&mine))["report"]["text"], "Last report: healthy");
        let st = HashMap::from([(pcm().key(), DtcState { dtcs: Some(vec![code("stored")]), incomplete: vec![code("")], ..Default::default() })]);
        let f = freeze_info_json(&[pcm()], &st, Some("MYVIN"));
        assert_eq!(f["vin_line"], "VIN MYVIN · 1 system · 1 stored code");
        assert!(f["not_run"].as_str().unwrap().starts_with("1 self-test has not run yet"));
        assert!(f["blocked"].is_null());
    }
}
