//! Running one actuator test (actuator.rs): conditions, periodic refresh, time limit and the guaranteed hand-back. Port of
//! hondascan/actuatorrun.py.
//!
//! STATUS: tested against the strict fake ECU only; never run on a real car. The shape is HDS's (PGMFI_INSP rad_fan_act):
//! confirm, check the conditions, enter the extended session, send the command, re-send it about once a second while the
//! person watches, and on the way out `2F <id> 00` then back to the default session. The conditions are re-checked at every
//! refresh, so starting the engine, rolling or losing the battery ends the test.
use crate::actuator::{condition, IoPolicy, Test, RETURN_CONTROL, SHORT_TERM};
use crate::honda::HondaCan;
use crate::livedata::{LiveData, Reading, Signal};
use crate::uds::{UdsClient, UdsError};
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::sync::Arc;
use std::time::Instant;

#[derive(Debug, Clone, PartialEq)]
pub struct ConditionFailed(pub Vec<String>);

impl fmt::Display for ConditionFailed {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "conditions not met: {}", self.0.join("; "))
    }
}

/// What a run can fail with: its own refusals, or a UDS/bus error.
#[derive(Debug)]
pub enum RunError {
    Value(String),
    Runtime(String),
    Condition(ConditionFailed),
    Uds(UdsError),
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            RunError::Value(s) | RunError::Runtime(s) => write!(f, "{s}"),
            RunError::Condition(c) => write!(f, "{c}"),
            RunError::Uds(u) => write!(f, "{u}"),
        }
    }
}

impl From<UdsError> for RunError {
    fn from(e: UdsError) -> Self {
        RunError::Uds(e)
    }
}

/// read(client) -> {symbol: Reading} for the database `symbols` of a system, each from the first data list DID that carries
/// it. A symbol the car does not answer is simply absent, which fails any condition that needs it.
pub struct Reader {
    plan: Vec<(String, (i64, Signal))>,
}

impl Reader {
    /// A reader that finds nothing (every condition then fails).
    pub fn empty() -> Reader {
        Reader { plan: vec![] }
    }

    pub fn new(live: &LiveData, system: &str, symbols: &BTreeSet<String>) -> Reader {
        let mut plan: Vec<(String, (i64, Signal))> = vec![];
        for did in live.dids(system, "DataList") {
            for s in live.signals(system, did).iter() {
                if symbols.contains(&s.symbol) && !plan.iter().any(|(k, _)| *k == s.symbol) {
                    plan.push((s.symbol.clone(), (did, s.clone())));
                }
            }
        }
        Reader { plan }
    }

    pub fn read(&self, client: &mut UdsClient) -> HashMap<String, Reading> {
        let mut cache: HashMap<i64, Option<Vec<u8>>> = HashMap::new();
        let mut out = HashMap::new();
        for (symbol, (did, s)) in &self.plan {
            let data = cache.entry(*did).or_insert_with(|| client.read_did(*did as u16).ok());
            if let Some(d) = data {
                out.insert(symbol.clone(), s.decode(d));
            }
        }
        out
    }
}

pub struct ActuatorRun {
    pub address: u32,
    pub test: Test,
    pub reader: Reader,
    pub refresh_s: f64,
    clock: Arc<dyn Fn() -> f64 + Send + Sync>,
    pub active: bool,
    pub value: Option<u16>,
    pub reason: Option<String>,
    pub errors: Vec<String>,
    t0: f64,
    last: f64,
}

pub fn monotonic_clock() -> Arc<dyn Fn() -> f64 + Send + Sync> {
    let t = Instant::now();
    Arc::new(move || t.elapsed().as_secs_f64())
}

impl ActuatorRun {
    pub fn new(address: u32, test: Test, reader: Reader, refresh_s: f64, clock: Arc<dyn Fn() -> f64 + Send + Sync>) -> ActuatorRun {
        ActuatorRun { address, test, reader, refresh_s, clock, active: false, value: None, reason: None, errors: vec![], t0: 0.0, last: 0.0 }
    }

    /// The database symbols the conditions and the displayed state read.
    pub fn symbols(test: &Test) -> BTreeSet<String> {
        let mut s: BTreeSet<String> = test.conditions.iter().filter_map(|n| condition(n)).flat_map(|c| c.symbols.iter().map(|x| x.to_string())).collect();
        s.extend(test.state.iter().map(|x| x.to_string()));
        s
    }

    pub fn failed(&self, hc: &mut HondaCan) -> Vec<String> {
        let readings = self.reader.read(hc.client(self.address));
        self.test.conditions.iter().filter_map(|n| condition(n)).filter(|c| !c.check.holds(&readings)).map(|c| c.text.to_string()).collect()
    }

    pub fn start(&mut self, hc: &mut HondaCan, value: u16, confirm: bool) -> Result<Value, RunError> {
        if !confirm {
            return Err(RunError::Value("an actuator test needs confirm: true".into()));
        }
        if self.active {
            return Err(RunError::Runtime("a test is already running".into()));
        }
        if !self.test.values.contains(&value) {
            let v: Vec<String> = self.test.values.iter().map(u16::to_string).collect();
            return Err(RunError::Value(format!("{}: value {value} is not one HDS sends ({})", self.test.key, v.join(", "))));
        }
        let failed = self.failed(hc);
        if !failed.is_empty() {
            return Err(RunError::Condition(ConditionFailed(failed)));
        }
        hc.client(self.address).io = Some(IoPolicy::single(self.test));
        let r = hc.client(self.address).session(0x03).and_then(|_| hc.client(self.address).io_control(self.test.did, SHORT_TERM, Some(value as u32)));
        if let Err(e) = r {
            self.hand_back(hc);
            return Err(e.into());
        }
        self.active = true;
        self.value = Some(value);
        self.reason = None;
        self.errors.clear();
        self.t0 = (self.clock)();
        self.last = self.t0;
        Ok(self.status())
    }

    /// Keep the test going; returns seconds until the next call is due, or None when no test is running.
    pub fn tick(&mut self, hc: &mut HondaCan) -> Option<f64> {
        if !self.active {
            return None;
        }
        let now = (self.clock)();
        if now - self.t0 >= self.test.hold_s {
            self.stop(hc, "time limit");
            return None;
        }
        if now - self.last >= self.refresh_s {
            let failed = self.failed(hc);
            if !failed.is_empty() {
                self.stop(hc, &ConditionFailed(failed).to_string());
                return None;
            }
            match hc.client(self.address).io_control(self.test.did, SHORT_TERM, Some(self.value.unwrap_or(0) as u32)) {
                Ok(_) => self.last = now,
                Err(e) => {
                    self.stop(hc, &format!("error: {e}"));
                    return None;
                }
            }
        }
        Some((self.refresh_s - ((self.clock)() - self.last)).max(0.0))
    }

    /// Hand control back (twice if it fails) and leave the extended session. Safe to call again.
    pub fn stop(&mut self, hc: &mut HondaCan, reason: &str) -> Value {
        if self.active || hc.client(self.address).io.is_some() {
            self.hand_back(hc);
            self.active = false;
            self.reason = Some(reason.into());
        }
        self.status()
    }

    fn hand_back(&mut self, hc: &mut HondaCan) {
        hc.client(self.address).io = Some(IoPolicy::single(self.test));
        for _ in 0..2 {
            match hc.client(self.address).io_control(self.test.did, RETURN_CONTROL, None) {
                Ok(_) => break,
                Err(e) => self.errors.push(format!("return control: {e}")), // a vanished adapter must not stop the second try or the cleanup
            }
        }
        if let Err(e) = hc.client(self.address).session(0x01) {
            self.errors.push(format!("session 01: {e}"));
        }
        hc.client(self.address).io = None;
    }

    pub fn status(&self) -> Value {
        let now = (self.clock)();
        let r2 = |x: f64| crate::value::round_to(x, 2);
        json!({"test": self.test.key, "name": self.test.name, "active": self.active, "value": self.value,
               "elapsed": if self.active { json!(r2(now - self.t0)) } else { Value::Null },
               "remaining": if self.active { json!(r2(self.test.hold_s - (now - self.t0))) } else { Value::Null },
               "reason": self.reason, "errors": self.errors})
    }
}
