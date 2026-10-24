//! Hand a recorded in-flight write or actuator test back to the car.
//!
//! A write or actuator test writes `<data dir>/audit/pending.json` before it sends anything (audit.rs). If the process
//! dies before the normal hand-back, the next connection (or the app's crash recovery) calls this module with that
//! record: it checks the car is the one the operation was recorded for, consults the same whitelist the run did, sends
//! only the hand-back requests that whitelist defines, and appends what happened to the audit trail.
//!
//! The order is deliberate and is the safety property: read the VIN first (`F190` at `vin_address`) and refuse to send
//! any hand-back frame until it matches the pending record. A malformed record, an unreadable VIN, a VIN mismatch or a
//! key/test that is not in the whitelist is a refusal with the pending file left in place, never a best-effort send.
use crate::actuator::{IoPolicy, RETURN_CONTROL, TESTS};
use crate::audit;
use crate::honda::HondaCan;
use crate::ident::{vin_of, VIN_DID};
use crate::write::{write, WritePolicy};
use serde_json::{json, Value};
use std::path::Path;

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02X}")).collect::<Vec<_>>().join(" ")
}

fn num(v: &Value) -> Option<u32> {
    v.as_u64().and_then(|n| u32::try_from(n).ok())
}

/// Send `req` once, retry once on error, and record every attempt and every error. True when a send succeeded.
fn send(hc: &mut HondaCan, address: u32, req: &[u8], what: &str, actions: &mut Vec<String>, errors: &mut Vec<String>) -> bool {
    for _ in 0..2 {
        actions.push(hex(req));
        match hc.client(address).request(req, None, Some(0)) {
            Ok(_) => return true,
            Err(e) => errors.push(format!("{what} {}: {e}", hex(req))),
        }
    }
    false
}

/// Validate the pending record, check the car's VIN and send the whitelisted hand-back frames. `Err(reason)` is a
/// refusal and nothing was sent. `Ok(left_default)` means every hand-back frame was attempted and reports whether the
/// final `10 01` was accepted; per-request errors are in `errors`.
fn execute(hc: &mut HondaCan, pending: &Value, actions: &mut Vec<String>, errors: &mut Vec<String>) -> Result<bool, String> {
    if !pending.is_object() {
        return Err("the pending record is not a JSON object".into());
    }
    if let Some(e) = pending.get("error").and_then(Value::as_str) {
        return Err(format!("the pending record cannot be acted on: {e}"));
    }
    if pending.get("schema").and_then(Value::as_u64) != Some(audit::SCHEMA) {
        return Err(format!("the pending record has no supported schema (expected {})", audit::SCHEMA));
    }
    let Some(kind) = pending.get("kind").and_then(Value::as_str) else {
        return Err("the pending record has no kind".into());
    };
    if kind != "write" && kind != "actuator" {
        return Err(format!("the pending record has an unknown kind {kind:?}"));
    }
    let Some(vin) = pending.get("vin").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()) else {
        return Err("the pending record has no VIN to check the car against".into());
    };
    let Some(vin_address) = pending.get("vin_address").and_then(num) else {
        return Err("the pending record has no vin_address".into());
    };
    let Some(address) = pending.get("address").and_then(num) else {
        return Err("the pending record has no address".into());
    };

    // The gate: nothing below may send a frame unless the car reports the VIN the operation was recorded for.
    let reported = match hc.client(vin_address).read_did(VIN_DID) {
        Ok(d) => d,
        Err(e) => {
            return Err(format!(
                "the car's VIN could not be read at address 0x{vin_address:02X} (F190): {e}; nothing was sent"
            ));
        }
    };
    let Some(reported) = vin_of(&reported) else {
        return Err(format!(
            "the car's VIN could not be read at address 0x{vin_address:02X} (F190): the ECU did not return a VIN; nothing was sent"
        ));
    };
    if !reported.eq_ignore_ascii_case(vin) {
        return Err(format!("VIN mismatch: the pending operation was recorded for {vin}, the car reports {reported}; nothing was sent"));
    }

    match kind {
        "write" => {
            let Some(key) = pending.get("key").and_then(Value::as_str) else {
                return Err("the pending write has no key".into());
            };
            let Some(w) = write(key) else {
                return Err(format!("{key:?} is not an HDS-defined write in the whitelist; nothing was sent"));
            };
            // The stop comes from the whitelist, never from the pending file: a tampered record cannot invent a request.
            if let Some(stop) = w.stop {
                // A crash can drop the link while the default session is the only one left, and this ECU accepts the
                // routine stop only in the session the write entered. Enter it first (the whitelist's, not the pending's).
                if let Some(session) = w.session {
                    send(hc, address, &[0x10, session], "diagnostic session", actions, errors);
                }
                hc.client(address).routine = Some(WritePolicy::single(*w));
                send(hc, address, stop, "stop", actions, errors);
            }
            Ok(send(hc, address, &[0x10, 0x01], "session 01", actions, errors))
        }
        _ => {
            let Some(key) = pending.get("test").and_then(Value::as_str) else {
                return Err("the pending actuator test has no test key".into());
            };
            let Some(t) = TESTS.iter().find(|t| t.key == key) else {
                return Err(format!("{key:?} is not an HDS-defined actuator test in the whitelist; nothing was sent"));
            };
            if let Some(did) = pending.get("io_did").and_then(num) {
                if did != u32::from(t.did) {
                    return Err(format!(
                        "the pending io_did 0x{did:04X} is not the whitelisted test's 0x{:04X}; nothing was sent",
                        t.did
                    ));
                }
            }
            hc.client(address).io = Some(IoPolicy::single(*t));
            // The test entered the extended session; a fresh connection starts in the default one, where 2F is refused.
            send(hc, address, &[0x10, 0x03], "diagnostic session", actions, errors);
            let return_control = [0x2F, (t.did >> 8) as u8, (t.did & 0xFF) as u8, RETURN_CONTROL];
            send(hc, address, &return_control, "return control", actions, errors);
            Ok(send(hc, address, &[0x10, 0x01], "session 01", actions, errors))
        }
    }
}

/// The hand-back frames only, with the pending file left for the caller to remove (this is what the tests drive).
/// Use `run` for the whole recovery: VIN gate, removal and audit trail.
pub fn hand_back(hc: &mut HondaCan, pending: &Value) -> Value {
    let (mut actions, mut errors) = (vec![], vec![]);
    let result = execute(hc, pending, &mut actions, &mut errors);
    let handed_back = matches!(&result, Ok(true));
    let error = match result {
        Err(why) => Some(why),
        Ok(_) => (!errors.is_empty()).then(|| errors.join("; ")),
    };
    json!({"pending": true, "handed_back": handed_back, "actions": actions, "error": error})
}

/// Whether `pending` is a write record complete enough to give an audit identity (a malformed file is reported, but
/// no result is invented for it).
fn is_write_record(pending: &Value) -> bool {
    pending.get("schema").and_then(Value::as_u64) == Some(audit::SCHEMA)
        && pending.get("kind").and_then(Value::as_str) == Some("write")
        && pending.get("vin").and_then(Value::as_str).is_some_and(|s| !s.trim().is_empty())
        && pending.get("key").and_then(Value::as_str).is_some()
        && pending.get("address").and_then(num).is_some()
}

/// Recover one pending operation: validate, hand the car back, remove the pending record on success, and append the
/// audit trail (`result` with `outcome: "interrupted"` for a write, then the `handback` event). `trigger` names the way
/// recovery was reached (`connect`, `recover`, `disconnect`) in the audit and the interrupted result.
pub fn run(hc: &mut HondaCan, root: &Path, pending: &Value, trigger: &str) -> Value {
    let (mut actions, mut errors) = (vec![], vec![]);
    let result = execute(hc, pending, &mut actions, &mut errors);
    let handed_back = match result {
        Err(why) => {
            errors.push(why);
            false
        }
        Ok(false) => false, // the ECU never confirmed the return to the default session; keep the record
        Ok(true) => match audit::clear_pending(root) {
            Ok(()) => true,
            Err(e) => {
                errors.push(format!("the pending record could not be removed: {e}"));
                false
            }
        },
    };
    let ok = handed_back && errors.is_empty();
    let error = (!errors.is_empty()).then(|| errors.join("; "));

    // A write that was in flight when the process died gets its result line: the run itself never returned, so the
    // only truthful outcome is "interrupted". Result first, handback second, as the maintenance contract specifies.
    let mut audit_errors = vec![];
    if is_write_record(pending) {
        let mut r = audit::record("result");
        for k in ["vin", "system", "address", "key", "name", "request", "frame_log"] {
            r[k] = pending.get(k).cloned().unwrap_or(Value::Null);
        }
        r["ok"] = json!(false);
        r["outcome"] = json!("interrupted");
        r["old"] = Value::Null;
        r["new"] = Value::Null;
        r["error_value"] = Value::Null;
        r["errors"] = json!(errors);
        r["detail"] = json!(if handed_back {
            format!("the write did not return; handed back after {trigger}")
        } else {
            format!("the write did not return; recovery after {trigger} did not complete")
        });
        if let Err(e) = audit::append(root, &r) {
            audit_errors.push(format!("the interrupted result could not be recorded: {e}"));
        }
    }
    let mut hb = audit::record("handback");
    hb["trigger"] = json!(trigger);
    hb["pending"] = pending.clone();
    hb["ok"] = json!(ok);
    hb["actions"] = json!(actions);
    hb["errors"] = json!(errors);
    if let Err(e) = audit::append(root, &hb) {
        audit_errors.push(format!("the hand-back record could not be written: {e}"));
    }

    let error = match (error, audit_errors.is_empty()) {
        (e, true) => e,
        (None, false) => Some(audit_errors.join("; ")),
        (Some(e), false) => Some(format!("{e}; {}", audit_errors.join("; "))),
    };
    json!({"pending": true, "handed_back": handed_back, "actions": actions, "error": error})
}
