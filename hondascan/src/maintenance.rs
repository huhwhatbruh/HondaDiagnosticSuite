//! Maintenance records: frozen baselines, service markers, and the live car read that the freeze gate and the health
//! report share. Nothing here sends anything but reads to the car (`19 02` and the catalog's readiness groups).
//!
//! A frozen baseline is the car's state at one moment, stored with a SHA-256 over its compact JSON. The store has insert
//! and read only for baselines (no update, no delete), so a row keeps what it said when it was frozen; `integrity` tells a
//! later reader whether the stored payload still hashes to the stored value.
use crate::audit;
use crate::service::{Service, ServiceError};
use crate::transport::can::now;
use crate::value::{dumps, Json};
use serde_json::{json, Value};

/// The work a service marker may record. A closed list keeps the history sortable (the GUI shows it as a menu).
pub const MARKER_KINDS: [&str; 9] = ["oil", "tires", "brakes", "battery", "filter", "fluid", "inspection", "repair", "other"];

/// The note's limit in characters, not bytes: the note may hold non-ASCII text.
const NOTE_MAX: usize = 2000;

fn store_error(e: rusqlite::Error) -> ServiceError {
    ServiceError { kind: "OperationalError".into(), message: e.to_string() }
}

fn vin_arg<'a>(vin: &'a str, what: &str) -> Result<&'a str, ServiceError> {
    let vin = vin.trim();
    if vin.is_empty() {
        return Err(ServiceError::value(format!("{what} needs the car's VIN")));
    }
    Ok(vin)
}

/// The live car: one entry per system the scan found (the PCM's CANFI and CANCVT both appear), each with its trouble
/// codes, the session's self-tests and the catalog's readiness groups. A system whose DTC read fails is reported with
/// `answered: false` and an `error`; it does not abort the snapshot, because the report still has the rest to show.
pub fn read_car(svc: &mut Service) -> Result<Value, ServiceError> {
    if svc.hc.is_none() {
        return Err(ServiceError::value("reading the car needs a connection to an adapter"));
    }
    if svc.rec.is_none() {
        return Err(ServiceError::value("reading the car needs a completed scan"));
    }
    let ecus = svc.ecus.clone();
    if ecus.is_empty() {
        return Err(ServiceError::value("the scan found no systems on this car"));
    }
    let vin = svc.vin.clone().unwrap_or_default();
    let session = svc.rec.as_ref().map(|r| r.sid);
    let adapter = svc.spec.clone();

    let mut systems = vec![];
    for e in &ecus {
        let (answered, available, dtcs, incomplete, error) = match svc.dtcs(e.address as i64, &e.system) {
            Ok(v) => (true, v["available"].as_i64().unwrap_or(0), v["dtcs"].clone(), v["incomplete"].clone(), None),
            Err(err) => (false, 0, json!([]), json!([]), Some(err.message)),
        };
        // Readiness comes from the live catalog, one 22 per OBD Status group; a refusal is an empty group, never a
        // failed snapshot (the groups are informational and a car may not implement them).
        let groups: Vec<Value> = svc.catalog(&e.system)["readiness"].as_array().cloned().unwrap_or_default();
        let readiness: Vec<Value> = if groups.is_empty() {
            vec![]
        } else {
            let dids: Vec<i64> = groups.iter().filter_map(|g| g["did"].as_i64()).collect();
            let read = svc.read_dids(e, Some(&dids), "status");
            groups.iter().map(|g| readiness_group(g, &read)).collect()
        };
        let mut system = json!({
            "address": e.address,
            "system": e.system,
            "system_type": e.system_type,
            "display": e.display,
            "answered": answered,
            "available": available,
            "dtcs": dtcs,
            "incomplete": incomplete,
            "self_tests": Value::Null,
            "readiness": readiness,
        });
        if let Some(message) = error {
            system["error"] = json!(message);
        }
        systems.push(system);
    }

    // After the DTC reads: `dtcs` updates the session's self-tests, so the summary must be taken once they have all run.
    let summaries = svc.rec.as_ref().map(|r| r.selftests.summary()).unwrap_or(Value::Null);
    for s in systems.iter_mut() {
        let key = {
            let name = s["system"].as_str().unwrap_or("");
            if name.is_empty() { s["system_type"].as_str().unwrap_or("").to_string() } else { name.to_string() }
        };
        s["self_tests"] = summaries.get(key.as_str()).cloned().unwrap_or(Value::Null);
    }

    let count_kind = |kind: &str| -> usize {
        systems.iter().map(|s| s["dtcs"].as_array().map_or(0, |a| a.iter().filter(|d| d["kind"].as_str() == Some(kind)).count())).sum()
    };
    let (faults, pending, stored) = (count_kind("confirmed"), count_kind("pending"), count_kind("stored"));
    let incomplete_count: usize = systems.iter().map(|s| s["incomplete"].as_array().map_or(0, |a| a.len())).sum();
    let unanswered: Vec<String> = systems
        .iter()
        .filter(|s| !s["answered"].as_bool().unwrap_or(false))
        .map(|s| {
            let name = s["system"].as_str().unwrap_or("");
            if name.is_empty() { s["system_type"].as_str().unwrap_or("?").to_string() } else { name.to_string() }
        })
        .collect();

    let mut problems = vec![];
    if !unanswered.is_empty() {
        problems.push(format!("no trouble-code answer from {}", unanswered.join(", ")));
    }
    if faults > 0 {
        problems.push(format!("{faults} confirmed trouble {}", if faults == 1 { "code" } else { "codes" }));
    }
    if pending > 0 {
        problems.push(format!("{pending} pending trouble {}", if pending == 1 { "code" } else { "codes" }));
    }
    // what stops a snapshot: an unanswered system or an active code. A self-test not run is recorded, not blocking: a used car
    // (or any car after a battery disconnect) has several until a drive cycle finishes them, and the report shows them
    // completing as a readiness change.
    let snapshot_ok = problems.is_empty();
    if incomplete_count > 0 {
        problems.push(format!("{incomplete_count} self-test{} not run", if incomplete_count == 1 { "" } else { "s" }));
    }
    let healthy = unanswered.is_empty() && faults == 0 && pending == 0 && incomplete_count == 0;

    Ok(json!({
        "schema": 1,
        "vin": vin,
        "session": session,
        "adapter": adapter,
        "systems": systems,
        "totals": {"systems": systems.len(), "faults": faults, "pending": pending, "stored": stored, "incomplete": incomplete_count},
        "healthy": healthy,
        "snapshot_ok": snapshot_ok,
        "problems": problems,
    }))
}

/// One catalog readiness group with the values just read: a value for numeric signals, the state text for state signals,
/// and the catalog's unit. A DID the ECU did not answer stays listed with no signals.
fn readiness_group(group: &Value, read: &Result<Value, ServiceError>) -> Value {
    let did = group["did"].as_i64().unwrap_or(0);
    let values = read
        .as_ref()
        .ok()
        .and_then(|r| r["dids"].as_array())
        .and_then(|d| d.iter().find(|d| d["did"].as_i64() == Some(did)))
        .filter(|d| d.get("error").is_none())
        .and_then(|d| d["values"].as_array());
    let Some(values) = values else { return json!({"did": did, "signals": []}) };
    let signals: Vec<Value> = group["signals"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let v = values.get(i).unwrap_or(&Value::Null);
            let (value, state) = match v {
                Value::Number(_) => (v.clone(), Value::Null),
                Value::String(_) => (Value::Null, v.clone()),
                _ => (Value::Null, Value::Null),
            };
            json!({"symbol": s["symbol"].clone(), "value": value, "unit": s["unit"].clone(), "state": state})
        })
        .collect();
    json!({"did": did, "signals": signals})
}

/// Store the car's state as a frozen baseline. The gate is the freeze contract: a current scan whose VIN matches, every
/// found system answered, no confirmed or pending codes. Stored codes are history and self-tests not yet run are readiness
/// still to come; both are recorded in the payload and neither blocks a freeze.
pub fn freeze(svc: &mut Service, vin: &str, label: Option<&str>) -> Result<Value, ServiceError> {
    let vin = vin_arg(vin, "freezing a baseline")?;
    let scanned = svc.vin.clone().unwrap_or_default();
    if scanned.is_empty() {
        return Err(ServiceError::value("freezing a baseline needs a completed scan"));
    }
    if scanned.eq_ignore_ascii_case("UNKNOWN") {
        return Err(ServiceError::value("the scan did not read the VIN, so the car cannot be identified"));
    }
    if !scanned.eq_ignore_ascii_case(vin) {
        return Err(ServiceError::value(format!("VIN mismatch: the scan read {scanned}, not {vin}")));
    }
    let snapshot = read_car(svc)?;
    if snapshot["snapshot_ok"].as_bool() != Some(true) {
        let problems: Vec<&str> = snapshot["problems"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
        return Err(ServiceError::value(format!("the car cannot be saved as a reference snapshot yet: {}", problems.join("; "))));
    }
    let healthy = snapshot["healthy"].as_bool() == Some(true);
    let label = label.unwrap_or("").trim();
    let created = now();
    let payload = json!({
        "schema": 1,
        "vin": vin,
        "created": created,
        "label": label,
        "session": snapshot["session"].clone(),
        "adapter": snapshot["adapter"].clone(),
        "healthy": healthy,
        "systems": snapshot["systems"].clone(),
        "totals": snapshot["totals"].clone(),
    });
    let sha = audit::sha256_hex(dumps(&payload, Json::Compact).as_bytes());
    let id = svc.store()?.add_baseline(vin, created, label, healthy, &dumps(&payload, Json::Spaced), &sha).map_err(store_error)?;
    let mut baseline = payload;
    baseline["id"] = json!(id);
    Ok(json!({"baseline": baseline, "frozen": true}))
}

/// One car's maintenance record: baseline metadata with an integrity verdict each, the newest readable baseline with its
/// systems, and the service markers (newest first).
pub fn maintenance(svc: &mut Service, vin: &str) -> Result<Value, ServiceError> {
    let vin = vin_arg(vin, "maintenance")?;
    let store = svc.store()?;
    let payloads = store.baseline_payloads(vin);
    let mut baselines = store.baselines(vin);
    for b in baselines.iter_mut() {
        let verdict = match b["id"].as_i64() {
            Some(id) => payloads.iter().find(|(pid, _, _)| *pid == id).map_or("modified", |(_, payload, sha)| integrity(payload, sha)),
            None => "modified",
        };
        b["integrity"] = json!(verdict);
    }
    let latest = payloads.iter().find_map(|(id, payload, _)| {
        let mut v: Value = serde_json::from_str(payload).ok()?;
        v.as_object_mut()?.insert("id".into(), json!(id));
        Some(v)
    });
    Ok(json!({"vin": vin, "baselines": baselines, "latest": latest, "markers": store.markers(vin)}))
}

/// Whether a stored payload still hashes to the value stored beside it: `"ok"` or `"modified"`. The hash is over the
/// compact JSON, so it survives the pretty layout the row is stored in; a payload that will not parse is `"modified"`.
pub fn integrity(payload: &str, sha: &str) -> &'static str {
    match serde_json::from_str::<Value>(payload) {
        Ok(v) if audit::sha256_hex(dumps(&v, Json::Compact).as_bytes()) == sha => "ok",
        _ => "modified",
    }
}

/// Record a service marker (work done between sessions) against the car. The VIN is the one the app showed; the note is
/// trimmed and kept as data. The mileage, when given, is kilometres.
pub fn add_marker(svc: &mut Service, vin: &str, kind: &str, note: &str, mileage_km: Option<f64>) -> Result<Value, ServiceError> {
    let vin = vin_arg(vin, "adding a service marker")?;
    if !MARKER_KINDS.contains(&kind) {
        return Err(ServiceError::value(format!("unknown marker kind '{kind}': pick one of {}", MARKER_KINDS.join(", "))));
    }
    let note = note.trim();
    if note.is_empty() {
        return Err(ServiceError::value("a service marker needs a note saying what was done"));
    }
    if note.chars().count() > NOTE_MAX {
        return Err(ServiceError::value(format!("the note is longer than {NOTE_MAX} characters")));
    }
    if let Some(km) = mileage_km {
        if !km.is_finite() || km < 0.0 {
            return Err(ServiceError::value("the mileage must be a number of kilometres, zero or more"));
        }
    }
    let at = now();
    let created = now();
    let id = svc.store()?.add_marker(vin, at, kind, note, mileage_km, created).map_err(store_error)?;
    Ok(json!({"marker": {"id": id, "at": at, "kind": kind, "note": note, "mileage_km": mileage_km, "created": created}}))
}

/// Remove one service marker by id. A stale id is a value error, not a silent success, so the app can say what happened.
pub fn remove_marker(svc: &mut Service, id: i64) -> Result<Value, ServiceError> {
    if !svc.store()?.remove_marker(id).map_err(store_error)? {
        return Err(ServiceError::value(format!("no service marker has id {id}")));
    }
    Ok(json!({"removed": true}))
}
