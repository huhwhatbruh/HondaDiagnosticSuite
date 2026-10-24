//! RPC methods for the HDS write functions (write.rs, writerun.rs). Never run on a car yet.
//!
//! `write_functions` lists the whitelist; `write_run` runs one synchronously (HDS's own wait is 10 s) and needs
//! `confirm: true`, the VIN the scan read, and an address where the scan found the function's system (so a session
//! with no scan refuses). The runner itself reads the ECU's F190 and refuses a mismatch, so the check here is a second
//! gate, not the only one. The RPC loop handles one request at a time, so `write_cancel` cannot wait in the queue: the
//! RPC reader thread sets `Service::write_cancel` the moment that line arrives, `write_run` polls the flag as its cancel
//! check, and the runner stops the routine and leaves the session (a run is bounded by HDS's wait of 10 s or the poll
//! timeout of 8 s either way). The command line cancels with Ctrl-C.
//!
//! Every run is audited (audit.rs): the attempt line and the pending record are durably written before `run.run` may
//! send a frame, and the result line and the removal of the pending record follow on every exit path. A failure to
//! write either of the first two is a refusal, not a warning; a failure to remove the pending record after a completed
//! run is reported in the result's audit detail and does not turn the run into an error.
use crate::actuator::condition;
use crate::actuatorrun::{Reader, RunError};
use crate::audit;
use crate::service::{Service, ServiceError};
use crate::write;
use crate::writerun::WriteRun;
use serde_json::{json, Value};
use std::sync::atomic::Ordering;

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02X}")).collect::<Vec<_>>().join(" ")
}

fn errors_of(v: &Value) -> Vec<String> {
    v["errors"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default()
}

impl Service {
    /// Answer to a `write_cancel` request. The flag was set when the line was read (even mid-run); clearing it here, after the
    /// run has ended, keeps a cancel that came too late (no write running) from cancelling the next run.
    pub fn write_cancel(&mut self) -> Value {
        self.write_cancel.store(false, Ordering::Relaxed);
        json!({})
    }

    pub fn write_functions(&self) -> Value {
        write::WRITES
            .iter()
            .map(|w| {
                json!({"key": w.key, "system": w.system, "name": w.name, "start": hex(w.start), "stop": w.stop.map(hex).unwrap_or_default(),
                       "kind": w.kind(),
                       "did": w.old.did, "byte": w.old.offset, "mask": w.mask(),
                       "monitor": w.monitor.map(hex).unwrap_or_default(),
                       "wait_s": w.wait_s, "poll_ms": w.poll_ms, "timeout_s": w.timeout_s,
                       "conditions": w.conditions.iter().filter_map(|c| condition(c)).map(|c| c.text).collect::<Vec<_>>(),
                       "read_back": w.read_back, "source": w.source})
            })
            .collect::<Vec<_>>()
            .into()
    }

    pub fn write_run(&mut self, address: i64, key: &str, vin: &str, confirm: bool) -> Result<Value, ServiceError> {
        if !confirm {
            return Err(ServiceError::value("a write needs confirm: true"));
        }
        let Some(w) = write::write(key).copied() else {
            let keys: Vec<&str> = write::WRITES.iter().map(|w| w.key).collect();
            return Err(ServiceError::value(format!("unknown write {key:?} ({})", keys.join(", "))));
        };
        if vin.trim().is_empty() {
            return Err(ServiceError::value("a write needs the car's VIN (the scan reads it)"));
        }
        let vin = vin.trim().to_ascii_uppercase();
        if let Some(known) = &self.vin {
            if !known.eq_ignore_ascii_case(&vin) {
                return Err(ServiceError::value(format!("VIN mismatch: this session scanned {known}, the write asks {vin}")));
            }
        }
        // The whitelist is per system: the scan must have found that system at this address (the runner's VIN read is the
        // stronger gate, but a wrong address must send nothing).
        if !self.ecus.iter().any(|e| e.address as i64 == address && e.system == w.system) {
            return Err(ServiceError::value(format!("{} writes {}, which the scan did not find at address {address}", w.key, w.system)));
        }
        let reader = Reader::new(&self.live, w.system, &WriteRun::symbols(&w));
        let mut run = WriteRun::new(address as u32, w, reader);
        // A write whose ECU does not answer F190 (the meter) verifies the VIN through the system named in `vin_system`.
        if let Some(sys) = w.vin_system {
            run.vin_address = self.ecus.iter().find(|e| e.system == sys).map(|e| e.address).or(Some(address as u32));
        }

        // The audit and pending records go down before the runner may send anything. A failure here is a refusal.
        // Check the connection first: a write with no adapter must not leave a record that looks in flight.
        self.hc()?;
        let root = self.store()?.root.clone();
        if audit::pending_path(&root).exists() {
            return Err(ServiceError::value("a pending operation is already recorded; recover it first"));
        }
        let frame_log = self.capture.as_ref().and_then(|c| c.path()).map(|p| p.to_string_lossy().into_owned());
        let request = hex(w.start);
        let mut attempt = audit::record("attempt");
        attempt["action"] = json!("write");
        attempt["vin"] = json!(vin);
        attempt["system"] = json!(w.system);
        attempt["address"] = json!(address);
        attempt["key"] = json!(w.key);
        attempt["name"] = json!(w.name);
        attempt["request"] = json!(request);
        attempt["frame_log"] = json!(frame_log);
        attempt["old_value"] = Value::Null;
        if let Err(e) = audit::append(&root, &attempt) {
            return Err(ServiceError { kind: "OSError".into(), message: format!("the audit record could not be written, so nothing was sent: {e}") });
        }
        let pending = json!({
            "schema": audit::SCHEMA,
            "at": crate::transport::can::now(),
            "pid": std::process::id(),
            "kind": "write",
            "vin": vin,
            "vin_address": run.vin_address.unwrap_or(address as u32),
            "address": address,
            "system": w.system,
            "name": w.name,
            "frame_log": frame_log,
            "key": w.key,
            "session": w.session,
            "stop": w.stop.map(hex),
            "request": request,
        });
        if let Err(e) = audit::write_pending(&root, &pending) {
            // The attempt is already in the trail: close it, or the record would show a write that never ended.
            let mut result = audit::record("result");
            result["vin"] = json!(vin);
            result["system"] = json!(w.system);
            result["address"] = json!(address);
            result["key"] = json!(w.key);
            result["name"] = json!(w.name);
            result["request"] = json!(request);
            result["frame_log"] = json!(frame_log);
            result["ok"] = json!(false);
            result["outcome"] = json!("refused");
            result["old"] = Value::Null;
            result["new"] = Value::Null;
            result["error_value"] = Value::Null;
            result["errors"] = json!([format!("the pending record could not be written: {e}")]);
            result["detail"] = json!("nothing was sent");
            let _ = audit::append(&root, &result);
            return Err(ServiceError { kind: "OSError".into(), message: format!("the pending record could not be written, so nothing was sent: {e}") });
        }

        let cancel = self.write_cancel.clone();
        let hc = self.hc()?;
        let mut service_error = None;
        let mut returned = Value::Null;
        let (ok, outcome, old, new, error_value, errors) = match run.run(hc, &vin, true, &|| cancel.load(Ordering::Relaxed)) {
            Ok(v) => {
                let ok = v["ok"] == json!(true);
                returned = v.clone();
                (ok, if ok { "complete" } else { "unconfirmed" }, v["old"].clone(), v["new"].clone(), v["error_value"].clone(), errors_of(&v))
            }
            Err(RunError::Condition(c)) => {
                let m = c.to_string();
                service_error = Some(ServiceError::value(m.clone()));
                (false, "refused", Value::Null, Value::Null, Value::Null, vec![m])
            }
            Err(RunError::Value(m)) => {
                service_error = Some(ServiceError::value(m.clone()));
                (false, "refused", Value::Null, Value::Null, Value::Null, vec![m])
            }
            Err(RunError::Runtime(m)) => {
                let outcome = if m.contains("cancelled") {
                    "cancelled"
                } else if m.contains("the start may have reached the ECU") {
                    "unconfirmed"
                } else {
                    "error"
                };
                service_error = Some(ServiceError { kind: "RuntimeError".into(), message: m.clone() });
                (false, outcome, Value::Null, Value::Null, Value::Null, vec![m])
            }
            Err(RunError::Uds(e)) => {
                let m = e.to_string();
                service_error = Some(e.into());
                (false, "error", Value::Null, Value::Null, Value::Null, vec![m])
            }
        };

        // The pending record is done with whichever way the run ended (the runner hands the ECU back itself). If it
        // cannot be removed, the result's detail says so; a completed write is still a completed write.
        let mut detail = None;
        if let Err(e) = audit::clear_pending(&root) {
            detail = Some(format!("the pending record could not be removed: {e}"));
        }
        let mut result = audit::record("result");
        result["vin"] = json!(vin);
        result["system"] = json!(w.system);
        result["address"] = json!(address);
        result["key"] = json!(w.key);
        result["name"] = json!(w.name);
        result["request"] = json!(request);
        result["frame_log"] = json!(frame_log);
        result["ok"] = json!(ok);
        result["outcome"] = json!(outcome);
        result["old"] = old;
        result["new"] = new;
        result["error_value"] = error_value;
        result["errors"] = json!(errors);
        if let Some(d) = detail {
            result["detail"] = json!(d);
        }
        let audit_problem = audit::append(&root, &result).err().map(|e| format!("the result could not be recorded in the audit: {e}"));

        match service_error {
            Some(mut e) => {
                if let Some(p) = audit_problem {
                    e.message = format!("{}; {p}", e.message);
                }
                Err(e)
            }
            None => {
                if let (Some(p), Some(o)) = (audit_problem, returned.as_object_mut()) {
                    o.insert("audit_error".into(), json!(p));
                }
                Ok(returned)
            }
        }
    }
}
