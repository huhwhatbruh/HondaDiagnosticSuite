//! RPC methods for actuator tests (actuator.rs, actuatorrun.rs). Port of hondascan/actservice.py. Never run on a car yet.
//!
//! One test at a time. Starting needs `confirm: true`, the service keeps it alive between requests (`tick`), and every way out
//! hands control back: `actuator_stop`, the time limit, a failed condition, an error, `disconnect` and the backend stopping
//! (rpc::serve calls disconnect at the end).
//!
//! Before `ActuatorRun::start` may send anything, the in-flight record goes to `<data dir>/audit/pending.json` (audit.rs):
//! if the process dies while a test is running, recovery reads it and sends the return-control request. The record is
//! removed on a failed start, on `actuator_stop`, when `tick` sees the test end, and on disconnect; if one is already
//! there, a new test is refused until it has been recovered.
use crate::actuator::condition;
use crate::actuatorrun::{monotonic_clock, ActuatorRun, Reader, RunError};
use crate::audit;
use crate::service::{Service, ServiceError, ENGINE};
use serde_json::{json, Value};

/// Python's `repr()` of a JSON value, for error messages.
pub fn py_repr(v: &Value) -> String {
    match v {
        Value::String(s) => format!("'{s}'"),
        other => crate::journal::py(other),
    }
}

impl Service {
    /// Drop the pending actuator record (if this service wrote one) and sync the audit directory. A removal failure
    /// leaves the file for the next recovery, which is safer than pretending it is gone.
    fn clear_actuator_pending(&mut self) {
        let Some(path) = self.pending_actuator.take() else { return };
        let root = path.parent().and_then(|p| p.parent()).map(std::path::Path::to_path_buf).unwrap_or_default();
        let _ = audit::clear_pending(&root);
    }

    pub fn actuator_tests(&self) -> Value {
        self.tests()
            .iter()
            .map(|t| {
                json!({"key": t.key, "system": t.system, "name": t.name, "did": t.did, "values": t.values,
                       "conditions": t.conditions.iter().filter_map(|c| condition(c)).map(|c| c.text).collect::<Vec<_>>(),
                       "hold_s": t.hold_s, "source": t.source, "state": t.state})
            })
            .collect::<Vec<_>>()
            .into()
    }

    pub fn actuator_start(&mut self, address: i64, test_key: &str, value: &Value, confirm: bool) -> Result<Value, ServiceError> {
        if !confirm {
            return Err(ServiceError::value("an actuator test needs confirm: true"));
        }
        if self.actuator.as_ref().is_some_and(|a| a.active) {
            return Err(ServiceError::value("a test is already running: stop it first"));
        }
        let Some(t) = self.tests().iter().find(|t| t.key == test_key).copied() else {
            let keys: Vec<&str> = self.tests().iter().map(|t| t.key).collect();
            return Err(ServiceError::value(format!("unknown test {test_key:?} ({})", keys.join(", "))));
        };
        let int = value.as_i64().filter(|_| !value.is_boolean());
        let Some(v) = int else {
            return Err(ServiceError::value(format!("{}: the value must be an integer, not {}", t.key, py_repr(value))));
        };
        self.hc()?;
        // The whitelist is per system: the scan must have found that system at this address (ecu_of would make up an entry with
        // the asked-for system for any address, so it is not used here).
        if !self.ecus.iter().any(|e| e.address as i64 == address && e.system == t.system) {
            return Err(ServiceError::value(format!("{} drives {}, which the scan did not find at address {address}", t.key, t.system)));
        }

        // The pending record goes down before the start may reach the ECU. It names the address F190 is read at for the
        // VIN check: the engine ECU on this car, else the test's own address.
        let root = self.store()?.root.clone();
        let pending_path = audit::pending_path(&root);
        if pending_path.exists() {
            return Err(ServiceError::value("a pending operation is already recorded; recover it first"));
        }
        let vin_address = self.ecus.iter().find(|e| ENGINE.contains(&e.system_type.as_str())).map(|e| e.address).unwrap_or(address as u32);
        let frame_log = self.capture.as_ref().and_then(|c| c.path()).map(|p| p.to_string_lossy().into_owned());
        let pending = json!({
            "schema": audit::SCHEMA,
            "at": crate::transport::can::now(),
            "pid": std::process::id(),
            "kind": "actuator",
            "vin": self.vin.clone().unwrap_or_else(|| "UNKNOWN".into()),
            "vin_address": vin_address,
            "address": address,
            "system": t.system,
            "name": t.name,
            "frame_log": frame_log,
            "test": t.key,
            "io_did": t.did,
            "value": v,
        });
        if let Err(e) = audit::write_pending(&root, &pending) {
            return Err(ServiceError { kind: "OSError".into(), message: format!("the pending record could not be written, so nothing was sent: {e}") });
        }
        self.pending_actuator = Some(pending_path);

        let reader = Reader::new(&self.live, t.system, &ActuatorRun::symbols(&t));
        let mut run = ActuatorRun::new(address as u32, t, reader, 1.0, monotonic_clock());
        let started = run.start(self.hc.as_mut().unwrap(), v as u16, true);
        let status = match started {
            Ok(s) => s,
            Err(RunError::Condition(c)) => {
                self.clear_actuator_pending();
                return Err(ServiceError::value(c.to_string()));
            }
            Err(RunError::Value(m)) => {
                self.clear_actuator_pending();
                return Err(ServiceError::value(m));
            }
            Err(RunError::Runtime(m)) => {
                self.clear_actuator_pending();
                return Err(ServiceError { kind: "RuntimeError".into(), message: m });
            }
            Err(RunError::Uds(e)) => {
                self.clear_actuator_pending();
                return Err(e.into());
            }
        };
        self.actuator = Some(run);
        (self.emit)("actuator", status.clone());
        Ok(status)
    }

    pub fn actuator_stop(&mut self) -> Value {
        if self.actuator.is_none() || self.hc.is_none() {
            self.clear_actuator_pending();
            return json!({"active": false});
        }
        let status = {
            let (Some(run), Some(hc)) = (self.actuator.as_mut(), self.hc.as_mut()) else { unreachable!() };
            let status = run.stop(hc, "stopped by request");
            (self.emit)("actuator", status.clone());
            status
        };
        self.clear_actuator_pending();
        status
    }

    pub fn actuator_status(&self) -> Value {
        self.actuator.as_ref().map_or(json!({"active": false}), |a| a.status())
    }
}

/// Keep a running test going; returns its next wake-up in seconds. A test that has ended (time limit, condition, error)
/// has already handed control back, so its pending record goes too.
pub fn tick(svc: &mut Service) -> Option<f64> {
    if svc.actuator.as_ref().is_none_or(|r| !r.active) {
        svc.clear_actuator_pending();
        return None;
    }
    let (Some(run), Some(hc)) = (svc.actuator.as_mut(), svc.hc.as_mut()) else { return None };
    let wait = run.tick(hc);
    if !run.active {
        let status = run.status();
        (svc.emit)("actuator", status);
    }
    if !svc.actuator.as_ref().is_some_and(|r| r.active) {
        svc.clear_actuator_pending();
    }
    wait
}

/// Every way out hands control back: a disconnect ends a running test first, and its pending record goes with it.
pub fn stop_on_disconnect(svc: &mut Service, reason: &str) {
    let had_run = svc.actuator.is_some();
    {
        let (Some(run), Some(hc)) = (svc.actuator.as_mut(), svc.hc.as_mut()) else { return };
        if run.active {
            let status = run.stop(hc, reason);
            (svc.emit)("actuator", status);
        }
    }
    if had_run {
        svc.clear_actuator_pending();
    }
}

pub fn add_status(svc: &Service, out: &mut Value) {
    out["actuator"] = svc.actuator_status();
}
