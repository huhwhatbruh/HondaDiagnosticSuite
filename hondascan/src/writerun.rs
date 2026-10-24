//! Running one write function (write.rs): VIN match, HDS's conditions, the old value, the session, the routine, the
//! proof and the guaranteed return to the default session.
//!
//! STATUS: tested against the strict fake ECUs only; never run on a real car. Three shapes:
//! - E-VTC (`EVTCLEARN.LearnSub`): check the codes, check the engine/park conditions, read the old value, enter session 3,
//!   start routine 0x0218, wait 10 s, read EVTCZPEND, and leave the session. HDS's normal path does not send the stop
//!   after a success; this runner matches that and sends `31 02 02 18 02` only when it aborts or the read-back does not
//!   confirm the learn.
//! - CVT_ISL stage one (`sub_af69`/`sub_abf4`): check the CVT DTCs and conditions (`ATPP == 1`, `NEW == 0`), read
//!   SLPLMOD/SLPLERR, enter session 3, start routine 0x1148, then poll SLPLMOD every 100 ms for up to 8 s. HDS sends no
//!   stop for this procedure, so none is sent here. The ignition-cycle continuation is not reproduced (write.rs).
//! - Meter US smart maintenance resets, items A and 1-5 (`sub_65f9`): no DTC or session checks. Item A has no
//!   condition (HDS always offers it); items 1-5 require their enable bit (`OUT_USS_ITnEN = 1`, read like HDS's
//!   USM_Item menu build). Read the item's old value in DID 0x7068, start its routine (0x7021 for item A,
//!   0x7022-0x7026 for items 1-5), then issue `31 03 70 2x` until the routine status is 0x7F no more; 0 is
//!   complete. There is no stop. The VIN comes from the PCM's F190 because the meter refuses F190 (the car answered
//!   `7F 22 31`). hondascan's 10 s timeout is a safety bound: HDS's loop has none.
use crate::actuator::{condition, Condition};
use crate::actuatorrun::{ConditionFailed, Reader, RunError};
use crate::honda::HondaCan;
use crate::ident::{vin_of, VIN_DID};
use crate::uds::UdsError;
use crate::write::{Proof, Write, WritePolicy};
use serde_json::{json, Value};
use std::fmt;
use std::time::{Duration, Instant};

/// Seconds between checks of the caller's cancel flag while waiting.
const CANCEL_STEP: f64 = 0.1;

pub struct WriteRun {
    pub address: u32,
    /// Where F190 is read. None: the same ECU as `address`. A meter target needs the PCM's VIN instead.
    pub vin_address: Option<u32>,
    pub write: Write,
    pub reader: Reader,
    /// HDS's fixed wait before a single read-back; tests shrink it with `set_timing` so they need not sleep.
    wait_s: f64,
    /// Pacing between read-back reads (0: a single read after `wait_s`); hondascan's chosen floor, not an HDS
    /// interval. HDS's own loops have no wait.
    poll_ms: u64,
    /// HDS's poll window, or hondascan's safety bound where HDS has none.
    timeout_s: f64,
    /// Seconds between TesterPresent requests during the fixed wait; well inside the 5 s S3 timer.
    keepalive_s: f64,
    pub old: Option<Vec<u8>>,
    pub new: Option<Vec<u8>>,
    pub errors: Vec<String>,
}

impl WriteRun {
    pub fn new(address: u32, write: Write, reader: Reader) -> WriteRun {
        WriteRun {
            address,
            vin_address: None,
            write,
            reader,
            wait_s: write.wait_s,
            poll_ms: write.poll_ms,
            timeout_s: write.timeout_s,
            keepalive_s: 2.0,
            old: None,
            new: None,
            errors: vec![],
        }
    }

    /// Tests only: shrink HDS's waits so a test need not sleep.
    pub fn set_timing(&mut self, wait_s: f64, poll_ms: u64, timeout_s: f64) {
        (self.wait_s, self.poll_ms, self.timeout_s) = (wait_s, poll_ms, timeout_s);
    }

    /// Tests only: a keepalive interval short enough for a fake whose S3 timer is shortened too.
    pub fn set_keepalive(&mut self, secs: f64) {
        self.keepalive_s = secs;
    }

    /// The database symbols the conditions read.
    pub fn symbols(write: &Write) -> std::collections::BTreeSet<String> {
        write.symbols()
    }

    pub fn failed(&self, hc: &mut HondaCan) -> Vec<String> {
        let readings = self.reader.read(hc.client(self.address));
        self.write.conditions.iter().filter_map(|n| condition(n)).filter(|c: &&Condition| !c.check.holds(&readings)).map(|c| c.text.to_string()).collect()
    }

    /// Run the write once. `vin` is the person's assertion of the car's VIN; the ECU's F190 answer must match it.
    /// `cancel` is polled during HDS's wait and poll loop (Ctrl-C at the command line; always false in tests).
    pub fn run(&mut self, hc: &mut HondaCan, vin: &str, confirm: bool, cancel: &dyn Fn() -> bool) -> Result<Value, RunError> {
        if !confirm {
            return Err(RunError::Value("a write needs confirm: true".into()));
        }
        if !hc.capture.as_ref().is_some_and(|c| c.is_attached() && !c.capped()) {
            return Err(RunError::Value("a write needs a frame log (--log FILE): every frame must be recorded, and this capture is not attached to a file or already hit its cap".into()));
        }
        let vin = vin.trim().to_ascii_uppercase();
        if vin.is_empty() {
            return Err(RunError::Value("a write needs the car's VIN: pass --vin VIN (scan prints it)".into()));
        }
        let vin_address = self.vin_address.unwrap_or(self.address);
        let reported = hc.client(vin_address).read_did(VIN_DID)?;
        let reported = vin_of(&reported).ok_or_else(|| RunError::Value("the ECU did not return a VIN; refusing a write without the VIN match".into()))?;
        if reported != vin {
            return Err(RunError::Value(format!("VIN mismatch: the car reports {reported}, the command says {vin}")));
        }

        // HDS's evtclearn_8_13.$CODE: CM.ChkDtc() then PGM_FI_UTILS.ShiftCheck(); CVT_ISL sub_abf4:
        // CM.DisplayAllDtcs() == 0. The meter's sub_65f9 checks neither. hondascan reads every status group
        // (19 02 FF), at least as strict as HDS, where HDS checks DTCs.
        if self.write.check_dtcs {
            let (_, dtcs) = hc.client(self.address).read_dtcs(0xFF)?;
            if !dtcs.is_empty() {
                let codes: Vec<String> = dtcs.iter().map(|d| d.code.clone()).collect();
                return Err(RunError::Condition(ConditionFailed(vec![format!("no DTCs stored (HDS CM.ChkDtc/DisplayAllDtcs): found {}", codes.join(", "))])));
            }
        }
        let failed = self.failed(hc);
        if !failed.is_empty() {
            return Err(RunError::Condition(ConditionFailed(failed)));
        }

        // The old value first: HDS's own first check (count 0 for CVT_ISL) and the read-back both need it stored.
        let old = hc.client(self.address).read_did(self.write.old.did)?;
        let Some(old_value) = self.write.old_value(&old) else {
            return Err(RunError::Value(format!(
                "DID {:04X} is {} bytes and has no {} byte(s) at {} for {}",
                self.write.old.did, old.len(), self.write.old.len, self.write.old.offset, self.write.old.label
            )));
        };
        self.old = Some(old.clone());
        if matches!(self.write.proof, Proof::Did(_)) {
            if self.write.old_allowed.is_empty() {
                if self.write.reads(&old) == Some(true) {
                    return Err(RunError::Value(format!("{} already reads 1 before the learn; the read-back could not prove it ran", self.write.read_back)));
                }
            } else if !u8::try_from(old_value).is_ok_and(|v| self.write.old_allowed.contains(&v)) {
                let allowed: Vec<String> = self.write.old_allowed.iter().map(u8::to_string).collect();
                return Err(RunError::Value(format!(
                    "{} would not take HDS's first count here: HDS accepts {} and the car reads {old_value}",
                    self.write.read_back,
                    allowed.join("/")
                )));
            }
        }

        hc.note(&format!("write {} ({}): start {}", self.write.key, self.write.name, hex(self.write.start)));
        hc.client(self.address).routine = Some(WritePolicy::single(self.write));
        let r = self.exchange(hc, cancel);
        hc.client(self.address).routine = None;
        r
    }

    fn exchange(&mut self, hc: &mut HondaCan, cancel: &dyn Fn() -> bool) -> Result<Value, RunError> {
        let addr = self.address;
        let mut left_session = false;
        let started = (|| -> Result<(), UdsError> {
            if let Some(session) = self.write.session {
                // Set before the request: the ECU may have entered the session even when its reply never arrived.
                left_session = session == 3;
                hc.client(addr).session(session)?;
            }
            hc.client(addr).routine_control(self.write.start)?;
            Ok(())
        })();
        if let Err(e) = started {
            self.stop(hc); // the start may have reached the ECU even when its reply did not
            self.leave(hc, left_session);
            return Err(self.ambiguous(hc, e));
        }

        let ok = if self.write.status().is_some() {
            match self.poll_status(hc, cancel) {
                Ok(ok) => ok,
                Err(e) => {
                    self.stop(hc);
                    self.leave(hc, left_session);
                    return Err(e);
                }
            }
        } else if self.poll_ms > 0 {
            match self.poll(hc, cancel) {
                Ok(ok) => ok,
                Err(e) => {
                    self.stop(hc);
                    self.leave(hc, left_session);
                    return Err(e);
                }
            }
        } else {
            match self.hold(hc, left_session, cancel) {
                Ok(true) => {}
                Ok(false) => {
                    self.stop(hc);
                    self.leave(hc, left_session);
                    return Err(RunError::Runtime("cancelled during HDS's wait; the routine was stopped".into()));
                }
                Err(e) => {
                    self.stop(hc);
                    self.leave(hc, left_session);
                    return Err(e.into());
                }
            }
            match hc.client(addr).read_did(self.write.old.did) {
                Ok(d) => {
                    self.new = Some(d.clone());
                    self.write.reads(&d).unwrap_or(false)
                }
                Err(e) => {
                    self.stop(hc);
                    self.leave(hc, left_session);
                    return Err(e.into());
                }
            }
        };
        if !ok {
            self.stop(hc);
        }
        self.leave(hc, left_session);
        Ok(self.status(ok))
    }

    /// HDS's DID poll-loop shape (`sub_abf4`, 4336-4375): read SLPLMOD immediately, then at hondascan's `poll_ms` pace
    /// until success, a reported error, the timeout, or the caller cancels. HDS's loop has no explicit wait. A read
    /// that the ECU refuses is retried (HDS does not treat it as success); a bus error propagates.
    fn poll(&mut self, hc: &mut HondaCan, cancel: &dyn Fn() -> bool) -> Result<bool, RunError> {
        let deadline = Instant::now() + Duration::from_secs_f64(self.timeout_s.max(0.0));
        let mut last: Option<u8> = None;
        let mut last_err: Option<String> = None;
        loop {
            if cancel() {
                return Err(RunError::Runtime("cancelled during the read-back poll".into()));
            }
            match hc.client(self.address).read_did(self.write.old.did) {
                Ok(d) => {
                    let value = self.write.value(&d);
                    self.new = Some(d.clone());
                    if self.write.reads(&d) == Some(true) {
                        return Ok(true);
                    }
                    if self.write.is_error(&d) {
                        let code = self.write.error_code(&d);
                        self.errors.push(match code {
                            Some(c) => format!("SLPLMOD = {value:?} (HDS error status), SLPLERR = {c}"),
                            None => format!("SLPLMOD = {value:?} (HDS error status)"),
                        });
                        return Ok(false);
                    }
                    last = value;
                }
                Err(UdsError::Io(e)) => return Err(RunError::Uds(UdsError::Io(e))),
                Err(e) => last_err = Some(e.to_string()),
            }
            let now = Instant::now();
            if now >= deadline {
                let mut what = match last {
                    Some(v) => format!("SLPLMOD stayed {v}"),
                    None => "SLPLMOD was never read".into(),
                };
                if let Some(e) = &last_err {
                    what.push_str(&format!(" ({e})"));
                }
                self.errors.push(format!("{what} for {} s; HDS calls that a timeout", crate::journal::py(&json!(self.timeout_s)).trim_end_matches(".0")));
                return Ok(false);
            }
            let left = deadline.saturating_duration_since(now);
            if !sleep(left.min(Duration::from_millis(self.poll_ms.max(1))), cancel) {
                return Err(RunError::Runtime("cancelled during the read-back poll".into()));
            }
        }
    }

    /// HDS's meter status loop (`sub_65f9`): `CM.RoutineControlWithStatus(3, rid, -1, &l_2, 6)` until the status is
    /// not 0x7F. HDS stops on any error and sends no stop; this does the same. The reply is the full ISO 14229 reply
    /// `71 03 <rid_hi> <rid_lo> <status>`; the status is byte 4 (the comms server strips `71 03` before HDS's own
    /// check, which is byte 2 of the six-byte buffer). HDS's loop has no timeout; hondascan bounds it.
    fn poll_status(&mut self, hc: &mut HondaCan, cancel: &dyn Fn() -> bool) -> Result<bool, RunError> {
        let monitor = self.write.monitor.ok_or_else(|| RunError::Value("a routine-status write needs a monitor request".into()))?;
        let status = self.write.status().ok_or_else(|| RunError::Value("a routine-status write needs a status proof".into()))?;
        let rid = [self.write.start[2], self.write.start[3]];
        let deadline = Instant::now() + Duration::from_secs_f64(self.timeout_s.max(0.0));
        loop {
            if cancel() {
                return Err(RunError::Runtime("cancelled during the routine-status poll".into()));
            }
            match hc.client(self.address).routine_control(monitor) {
                Ok(rsp) => {
                    if rsp.len() < 5 || rsp[0] != 0x71 || rsp[1] != 0x03 || rsp.get(2..4) != Some(&rid[..]) {
                        return Err(RunError::Uds(UdsError::Other(format!(
                            "routine status reply is not 71 03 {:02X} {:02X} <status>: {}",
                            rid[0], rid[1], hex(&rsp)
                        ))));
                    }
                    let value = rsp[4];
                    self.new = Some(vec![value]);
                    if value == status.success {
                        return Ok(true);
                    }
                    if value != status.pending {
                        self.errors.push(format!(
                            "routine status 0x{value:02X} (HDS treats any value other than 0x{:02X} or 0x{:02X} as failure)",
                            status.pending, status.success
                        ));
                        return Ok(false);
                    }
                }
                Err(UdsError::Io(e)) => return Err(RunError::Uds(UdsError::Io(e))),
                Err(e) => {
                    self.errors.push(format!("routine status request: {e}"));
                    return Ok(false);
                }
            }
            let now = Instant::now();
            if now >= deadline {
                self.errors.push(format!(
                    "routine status stayed 0x{:02X} for {} s (hondascan safety bound; HDS's sub_65f9 has no timeout)",
                    status.pending,
                    crate::journal::py(&json!(self.timeout_s)).trim_end_matches(".0")
                ));
                return Ok(false);
            }
            let left = deadline.saturating_duration_since(now);
            if !sleep(left.min(Duration::from_millis(self.poll_ms.max(1))), cancel) {
                return Err(RunError::Runtime("cancelled during the routine-status poll".into()));
            }
        }
    }

    /// HDS's fixed wait, with a TesterPresent every `keepalive_s` while the extended session is open: a session left silent
    /// for longer than its S3 timer (5 s in ISO 14229; the fake enforces it) falls back to the default session, and an
    /// ECU may abort a routine that needs the extended one. hondascan's addition: no HDS source says how HDS holds the
    /// session through this wait (its comms server is not decoded). False when cancelled; a bus error propagates, a
    /// refused or unanswered TesterPresent is only recorded (the read-back then shows whether the routine survived).
    fn hold(&mut self, hc: &mut HondaCan, in_session: bool, cancel: &dyn Fn() -> bool) -> Result<bool, UdsError> {
        let end = Instant::now() + Duration::from_secs_f64(self.wait_s.max(0.0));
        loop {
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(true);
            }
            let step = if in_session { left.min(Duration::from_secs_f64(self.keepalive_s)) } else { left };
            if !sleep(step, cancel) {
                return Ok(false);
            }
            if in_session && Instant::now() < end {
                match hc.client(self.address).tester_present(Some(0)) {
                    Err(UdsError::Io(e)) => return Err(UdsError::Io(e)),
                    Err(e) => self.errors.push(format!("tester present: {e}")),
                    Ok(_) => {}
                }
            }
        }
    }

    /// A start whose reply never arrived may still have run. After the stop and the return to session 1, read the proof DID
    /// once and say what the car shows, in the error and in the frame log, so the person knows whether to run it again.
    /// Refusals (a negative reply) are not ambiguous and pass through unchanged.
    fn ambiguous(&mut self, hc: &mut HondaCan, e: UdsError) -> RunError {
        if !matches!(e, UdsError::NoResponse(_) | UdsError::Transport(_)) {
            return e.into();
        }
        let shows = match hc.client(self.address).read_did(self.write.old.did) {
            Ok(d) => {
                let (now, before) = (self.write.value(&d), self.old.as_ref().and_then(|o| self.write.value(o)));
                self.new = Some(d);
                format!("DID {:04X} shows {} = {now:?} (before the start: {before:?})", self.write.old.did, self.write.old.label)
            }
            Err(r) => format!("DID {:04X} could not be read back ({r})", self.write.old.did),
        };
        let msg = format!(
            "{e}; the start may have reached the ECU, so the routine was stopped and session 01 restored. Afterwards {shows}. \
             Check the car before running {} again",
            self.write.key
        );
        hc.note(&format!("write {}: outcome unknown. {msg}", self.write.key));
        RunError::Runtime(msg)
    }

    fn stop(&mut self, hc: &mut HondaCan) {
        let Some(stop) = self.write.stop else { return }; // HDS's CVT_ISL and meter procedures send no stop
        if let Err(e) = hc.client(self.address).routine_control(stop) {
            self.errors.push(format!("stop routine: {e}"));
        }
    }

    fn leave(&mut self, hc: &mut HondaCan, left_session: bool) {
        if !left_session {
            return;
        }
        if let Err(e) = hc.client(self.address).session(0x01) {
            self.errors.push(format!("session 01: {e}"));
        }
    }

    pub fn status(&self, ok: bool) -> Value {
        let bit = |d: &Option<Vec<u8>>| d.as_ref().and_then(|d| self.write.reads(d));
        let value = |d: &Option<Vec<u8>>| d.as_ref().and_then(|d| self.write.value(d));
        let old_value = self.old.as_ref().and_then(|d| self.write.old_value(d));
        let (old_bit, new_bit) = if matches!(self.write.proof, Proof::Did(_)) {
            (bit(&self.old), bit(&self.new))
        } else {
            (None, None)
        };
        let new_value = match self.write.proof {
            Proof::Did(_) => value(&self.new).map(u32::from),
            Proof::Status(_) => self.new.as_ref().and_then(|d| d.first().copied()).map(u32::from),
        };
        let error_value = match self.write.proof {
            Proof::Did(_) => self.new.as_ref().filter(|d| self.write.is_error(d)).and_then(|d| self.write.error_code(d)).map(u32::from),
            Proof::Status(_) if !ok => new_value,
            Proof::Status(_) => None,
        };
        json!({
            "write": self.write.key,
            "name": self.write.name,
            "system": self.write.system,
            "kind": self.write.kind(),
            "ok": ok,
            "did": format!("{:04X}", self.write.old.did),
            "byte": self.write.old.offset,
            "mask": format!("{:02X}", self.write.mask()),
            "old": old_bit,
            "new": new_bit,
            "old_value": old_value,
            "new_value": new_value,
            "pending": self.write.status().map(|s| s.pending),
            "old_label": self.write.old.label,
            "error_value": error_value,
            "poll_ms": self.poll_ms,
            "timeout_s": self.timeout_s,
            "conditions": self.write.conditions,
            "read_back": self.write.read_back,
            "session": self.write.session,
            "check_dtcs": self.write.check_dtcs,
            "source": self.write.source,
            "errors": self.errors,
        })
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Sleep `d`, polling `cancel` every 0.1 s or less. False when cancelled.
fn sleep(d: Duration, cancel: &dyn Fn() -> bool) -> bool {
    let end = Instant::now() + d;
    loop {
        if cancel() {
            return false;
        }
        let left = end.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return true;
        }
        std::thread::sleep(left.min(Duration::from_secs_f64(CANCEL_STEP)));
    }
}

impl fmt::Debug for WriteRun {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("WriteRun")
            .field("address", &self.address)
            .field("vin_address", &self.vin_address)
            .field("write", &self.write.key)
            .field("wait_s", &self.wait_s)
            .field("poll_ms", &self.poll_ms)
            .field("timeout_s", &self.timeout_s)
            .finish()
    }
}
