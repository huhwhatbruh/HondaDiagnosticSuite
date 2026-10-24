//! What the store knows about a car, shaped for front ends and for vehicle.md. Port of hondascan/history.py.
use crate::journal::render_vehicle;
use crate::store::Store;
use serde_json::{json, Map, Value};

pub const TRENDS: [(&str, &str); 8] = [
    ("CANFI", "VB"), ("CANFI", "ECT"), ("CANFI", "KLAFAVE"), ("CANFI", "ROLF"),
    ("CAN_TPMS", "FLAIR"), ("CAN_TPMS", "FRAIR"), ("CAN_TPMS", "RLAIR"), ("CAN_TPMS", "RRAIR"),
];

pub fn vehicle(store: &Store, vin: &str) -> Value {
    let mut sessions = store.sessions(vin, 200);
    for s in sessions.iter_mut() {
        // the list view needs the headline numbers, not every event
        let sm = s.get("summary").cloned().unwrap_or(Value::Null);
        let mut slim = Map::new();
        for k in ["duration_s", "trip", "codes", "notes", "reads"] {
            slim.insert(k.into(), sm.get(k).cloned().unwrap_or(Value::Null));
        }
        s["summary"] = Value::Object(slim);
    }
    json!({"vin": vin, "sessions": sessions, "codes": store.dtc_history(vin), "trends": trends(store, vin), "calibration": calibration(store, vin)})
}

/// Each module's identity over the car's life: one entry per run of sessions that reported the same ECU ID and part number,
/// oldest first, with the session range. Two entries for one module mean its software, or the module itself, changed.
pub fn calibration(store: &Store, vin: &str) -> Vec<Value> {
    let mut out: Vec<Value> = vec![];
    for r in store.identities(vin) {
        let module = |e: &Value| e["address"] == r["address"] && e["system"] == r["system"];
        // compare with the newest entry of the same module, not the last one written: modules interleave
        match out.iter_mut().rev().find(|e| module(e)) {
            Some(e) if e["ecu_id"] == r["ecu_id"] && e["part_number"] == r["part_number"] => {
                e["last_session"] = r["session"].clone();
                e["last_seen"] = r["started"].clone();
                e["sessions"] = json!(e["sessions"].as_i64().unwrap_or(0) + 1);
            }
            _ => out.push(json!({"address": r["address"], "system": r["system"], "ecu_id": r["ecu_id"], "ecu_model": r["ecu_model"], "part_number": r["part_number"],
                                 "first_session": r["session"], "last_session": r["session"], "sessions": 1, "first_seen": r["started"], "last_seen": r["started"]})),
        }
    }
    out
}

pub fn trends(store: &Store, vin: &str) -> Map<String, Value> {
    let mut out = Map::new();
    for (sy, sym) in TRENDS {
        let rows = store.trend(vin, sy, sym, 100);
        if !rows.is_empty() {
            out.insert(format!("{sy}.{sym}"), Value::Array(rows));
        }
    }
    out
}

pub fn session_detail(store: &Store, sid: i64) -> Result<Value, String> {
    let dir = store.session_dir(sid).ok_or_else(|| format!("no session {sid}"))?;
    let md = std::fs::read_to_string(std::path::Path::new(&dir).join("summary.md")).unwrap_or_default();
    Ok(json!({"id": sid, "folder": dir, "summary_md": md}))
}

/// vehicle.md from every session (no limit), so the totals of older sessions are complete.
pub fn write_vehicle(store: &Store, vin: &str) -> std::io::Result<()> {
    let md = render_vehicle(vin, &store.sessions(vin, -1), &store.dtc_history(vin), &trends(store, vin));
    std::fs::write(store.root.join("vehicles").join(vin).join("vehicle.md"), md)
}
