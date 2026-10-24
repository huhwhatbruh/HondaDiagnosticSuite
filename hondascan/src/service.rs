//! The scanner as a long-running service: one adapter, the systems found on it, a recorded session per VIN, and the monitor
//! that keeps reading while the car is on. hondascan.rpc exposes it to front ends. Port of hondascan/service.py.
//!
//! All bus traffic goes through `read_dids`, `dtcs` and `scan`, so every exchange is timed, recorded and counted as proof of
//! life for the car-off detection. Reads stay inside the UDS allowlist (uds.rs).
use crate::actuator::{Test, TESTS};
use crate::actuatorrun::ActuatorRun;
use crate::db::Database;
use crate::history;
use crate::honda::{Ecu as HondaEcu, HondaCan};
use crate::ident::{read_ecu_info, read_identity, read_vin};
use crate::livedata::{LiveData, Reading, Signal};
use crate::monitor::{Monitor, Task};
use crate::recorder::{dtc_records, Def, EcuInfo, Recorder};
use crate::signalhelp::SignalHelp;
use crate::store::Store;
use crate::transport::can::{now, CaptureLog};
use crate::transport::{open_adapter, Adapter};
use crate::trip::fuel_flow_lph;
use crate::uds::UdsError;
use crate::value::{round_to, Val};
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;

/// Seconds on a clock that only moves forward (Python's time.monotonic()), from the first call.
pub fn mono() -> f64 {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs_f64()
}

pub const NO_UNIT: [&str; 3] = ["", "<no units>", "-1"];
pub const ENGINE: [&str; 2] = ["PGM-FI", "DIESEL"];
/// data-list usage -> recorded kind
pub const SNAPSHOT_KINDS: [(&str, &str); 2] = [("FreezeData", "freeze"), ("OBD Status", "status")];

pub fn unit_of(sig: &Signal) -> String {
    let u = sig.out_unit();
    if NO_UNIT.contains(&u) { String::new() } else { u.to_string() }
}

pub fn value_of(r: &Reading) -> Val {
    if !r.error.is_empty() {
        Val::Null
    } else if let Some(t) = &r.text {
        Val::Text(t.clone())
    } else {
        r.value.map_or(Val::Null, Val::Num)
    }
}

pub fn val_json(v: &Val) -> Value {
    match v {
        Val::Num(n) => json!(n),
        Val::Text(s) => json!(s),
        Val::Null => Value::Null,
    }
}

/// An error as the RPC reports it: the message, and the name of the Python exception class it corresponds to.
#[derive(Debug, Clone)]
pub struct ServiceError {
    pub kind: String,
    pub message: String,
}

impl ServiceError {
    pub fn value(m: impl Into<String>) -> ServiceError {
        ServiceError { kind: "ValueError".into(), message: m.into() }
    }
}

impl std::fmt::Display for ServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl From<UdsError> for ServiceError {
    fn from(e: UdsError) -> Self {
        let kind = match &e {
            UdsError::NoResponse(_) => "NoResponse",
            UdsError::Transport(_) => "TransportError",
            UdsError::Negative { .. } => "NegativeResponse",
            UdsError::NotAllowed(_) => "NotAllowed",
            UdsError::Io(_) => "OSError",
            UdsError::Other(_) => "UdsError",
            UdsError::Kwp(k) => k.kind(),
        };
        ServiceError { kind: kind.into(), message: e.to_string() }
    }
}

impl From<std::io::Error> for ServiceError {
    fn from(e: std::io::Error) -> Self {
        ServiceError { kind: "OSError".into(), message: e.to_string() }
    }
}

impl From<crate::transport::OpenError> for ServiceError {
    fn from(e: crate::transport::OpenError) -> Self {
        ServiceError { kind: e.kind.into(), message: e.message }
    }
}

impl From<String> for ServiceError {
    fn from(m: String) -> Self {
        ServiceError::value(m)
    }
}

pub type Emit = Arc<dyn Fn(&str, Value) + Send + Sync>;

pub struct Service {
    pub db: Arc<Database>,
    /// Set by the RPC reader thread when a `write_cancel` request arrives while `write_run` blocks the loop.
    pub write_cancel: Arc<AtomicBool>,
    pub emit: Emit,
    pub live: Arc<LiveData>,
    pub help: SignalHelp,
    catalogs: HashMap<String, Value>,   // `catalog` answers, built once per system: they only depend on the database
    pub monitor: Monitor,
    /// Overrides the maps' (warm-engine temperature, samples per cell); tests set it with `set_map_limits`.
    map_limits: Option<(f64, f64)>,
    store: Option<Arc<Store>>,
    pub adapter: Option<Arc<dyn Adapter>>,
    pub hc: Option<HondaCan>,
    pub capture: Option<Arc<CaptureLog>>,
    pub rec: Option<Recorder>,
    pub spec: String,
    pub ecus: Vec<EcuInfo>,
    pub vin: Option<String>,
    pub ident_queue: Vec<u32>,
    prim: HashMap<String, HashMap<i64, HashSet<String>>>, // system -> {did: symbols it is the primary source of}
    pub actuator: Option<ActuatorRun>,
    /// The pending actuator record this service wrote (`audit/pending.json`), so stop/tick/disconnect can remove it.
    pub(crate) pending_actuator: Option<std::path::PathBuf>,
    tests: Vec<Test>, // the whitelisted actuator tests (tests replace the list with `set_tests`)
    pub gate: Option<((u32, String), usize)>, // (address, profile) of the last identity check that passed, with the scan generation it was made against
    pub scan_generation: usize,
}

impl Service {
    pub fn new(db: Arc<Database>, emit: Emit, store: Option<Arc<Store>>) -> Service {
        let live = Arc::new(LiveData::new(db.clone()));
        Service {
            write_cancel: Arc::new(AtomicBool::new(false)),
            help: SignalHelp::new(db.clone()), catalogs: HashMap::new(), monitor: Monitor::new(live.clone()), live, db, emit, store, adapter: None, hc: None, capture: None, rec: None,
            spec: String::new(), ecus: vec![], vin: None, ident_queue: vec![], prim: HashMap::new(), actuator: None, pending_actuator: None, tests: TESTS.to_vec(), gate: None, scan_generation: 0, map_limits: None,
        }
    }

    /// The whitelisted actuator tests.
    pub fn tests(&self) -> &[Test] {
        &self.tests
    }

    /// Tests only: replace the actuator tests, e.g. to shorten a time limit.
    pub fn set_tests(&mut self, tests: Vec<Test>) {
        self.tests = tests;
    }

    /// Tests only: override the maps' (warm-engine temperature, samples per cell).
    pub fn set_map_limits(&mut self, limits: Option<(f64, f64)>) {
        self.map_limits = limits;
    }

    pub fn store(&mut self) -> Result<Arc<Store>, ServiceError> {
        if self.store.is_none() {
            self.store = Some(Arc::new(Store::open(None).map_err(|e| ServiceError { kind: "OperationalError".into(), message: e.to_string() })?));
        }
        Ok(self.store.clone().unwrap())
    }

    /// The data directory every audit/pending path is relative to.
    pub fn root(&mut self) -> Result<std::path::PathBuf, ServiceError> {
        Ok(self.store()?.root.clone())
    }

    // -- recovery ---------------------------------------------------------------------
    /// The pending write/actuator record, or None. A malformed file comes back as an object with `error` and `raw`
    /// (audit::read_pending), never as something a caller may act on.
    pub fn pending(&mut self) -> Option<Value> {
        let root = self.store().ok()?.root.clone();
        crate::audit::read_pending(&root)
    }

    /// Hand a recorded in-flight operation back to the car: on the current connection when there is one, otherwise by
    /// opening `adapter` (or the last connection's spec) for exactly that. Never starts a write or a session.
    pub fn recover(&mut self, adapter: Option<&str>) -> Result<Value, ServiceError> {
        let root = self.root()?;
        let Some(pending) = crate::audit::read_pending(&root) else {
            return Ok(json!({"pending": false, "handed_back": false, "actions": [], "error": Value::Null}));
        };
        if let Some(hc) = self.hc.as_mut() {
            return Ok(crate::handback::run(hc, &root, &pending, "recover"));
        }
        let spec = adapter
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .or_else(|| (!self.spec.trim().is_empty()).then(|| self.spec.clone()))
            .ok_or_else(|| ServiceError::value("recover needs an adapter when no adapter is connected"))?;
        let opened = open_adapter(&spec, None)?;
        let mut hc = HondaCan::new(opened.clone(), self.db.clone(), false);
        let result = crate::handback::run(&mut hc, &root, &pending, "recover");
        drop(hc);
        opened.close();
        Ok(result)
    }

    // -- adapter ---------------------------------------------------------------------
    pub fn connect(&mut self, adapter: &str) -> Result<Value, ServiceError> {
        self.disconnect("disconnected");
        let capture = Arc::new(CaptureLog::new());
        capture.note(&format!("hondascan session, adapter {adapter}"));
        self.capture = Some(capture.clone());
        let opened = match open_adapter(adapter, Some(capture)) {
            Ok(a) => a,
            Err(e) => {
                self.disconnect("disconnected");
                return Err(e.into());
            }
        };
        let mut reply = self.attach(adapter, opened);
        // A write/actuator the last process did not finish is handed back now, on the new connection, before anyone
        // can start another. Its failure does not fail the connect: the reply says what happened.
        if let Ok(root) = self.root() {
            if let Some(pending) = crate::audit::read_pending(&root) {
                if let Some(hc) = self.hc.as_mut() {
                    reply["recovery"] = crate::handback::run(hc, &root, &pending, "connect");
                }
            }
        }
        Ok(reply)
    }

    /// Use an adapter that is already open (connect does this after opening one from its spec).
    pub fn attach(&mut self, spec: &str, opened: Arc<dyn Adapter>) -> Value {
        self.spec = spec.to_string();
        self.hc = Some(HondaCan::new(opened.clone(), self.db.clone(), false));
        if let Some(hc) = self.hc.as_mut() {
            hc.capture = self.capture.clone();
        }
        let description = opened.describe();
        let warning = opened.path().and_then(|p| crate::transport::modem_manager_warning(&p));
        self.adapter = Some(opened);
        json!({"adapter": spec, "description": description, "warning": warning})
    }

    pub fn disconnect(&mut self, reason: &str) -> Value {
        crate::actservice::stop_on_disconnect(self, reason);
        // If a write (or an actuator record the stop above could not clear) is still pending, hand it back on the
        // connection that is about to go away. On success the pending file is gone, so a later connect sees none.
        if let Ok(root) = self.root() {
            if let Some(pending) = crate::audit::read_pending(&root) {
                if let Some(hc) = self.hc.as_mut() {
                    let _ = crate::handback::run(hc, &root, &pending, "disconnect");
                }
            }
        }
        self.end(reason);
        if let Some(a) = self.adapter.take() {
            a.close();
        }
        self.hc = None;
        self.capture = None;
        self.ecus = vec![];
        self.vin = None;
        self.scan_generation += 1;
        self.monitor.reset(&[], 0.0);
        json!({})
    }

    pub fn hc(&mut self) -> Result<&mut HondaCan, ServiceError> {
        self.hc.as_mut().ok_or_else(|| ServiceError::value("not connected to an adapter"))
    }

    fn end(&mut self, reason: &str) {
        if let Some(mut rec) = self.rec.take() {
            let s = rec.stop(reason);
            // The recorder detached the frame file; a write must not run unlogged.
            if let Some(hc) = self.hc.as_mut() {
                hc.capture = None;
            }
            let vin = rec.vin.clone();
            if let Ok(store) = self.store() {
                let _ = history::write_vehicle(&store, &vin);
            }
            (self.emit)("session", json!({"state": "ended", "reason": reason, "id": s["id"]}));
        }
    }

    // -- discovery and session start ----------------------------------------------------
    pub fn scan(&mut self, addresses: Option<Vec<u32>>) -> Result<Value, ServiceError> {
        let t0 = Instant::now();
        let emit = self.emit.clone();
        let found = {
            let progress = move |a: u32| emit("scan", json!({"address": a}));
            self.hc()?.discover(addresses.as_deref(), Some(&progress))?
        };
        let mut ecus = vec![];
        for e in &found {
            let display = if e.system.is_empty() { None } else { self.db.system(&e.system) }.map_or(e.system_type.clone(), |s| s.display);
            ecus.push(EcuInfo::new(e.address, &e.system_type, &e.system, &display, &e.rule));
        }
        let seconds = round_to(t0.elapsed().as_secs_f64(), 3);
        let mut order: Vec<u32> = ecus.iter().map(|e| e.address).collect::<BTreeSet<_>>().into_iter().collect();
        order.sort_by_key(|a| ecus.iter().find(|x| x.address == *a).is_some_and(|x| !ENGINE.contains(&x.system_type.as_str())));
        let vin = read_vin(self.hc()?, &order)?.unwrap_or_else(|| "UNKNOWN".to_string());
        self.vin = Some(vin.clone());
        self.end("rescan");
        let store = self.store()?;
        // Build the recorder first: it re-attaches the capture file (recorder.rs), and if it fails no state that a
        // write reads may have moved -- self.ecus must not point at a scan whose frames are not being recorded.
        let rec = Recorder::with_map_limits(store, self.capture.clone(), &vin, &self.spec, ecus.clone(), seconds, self.map_limits)?;
        self.ecus = ecus.clone();
        self.scan_generation += 1;
        if let Some(hc) = self.hc.as_mut() {
            hc.capture = self.capture.clone();
        }
        let (sid, folder) = (rec.sid, rec.journal.folder.to_string_lossy().into_owned());
        self.rec = Some(rec);
        self.monitor.reset(&ecus, mono());
        self.ident_queue = ecus.iter().map(|e| e.address).collect::<BTreeSet<_>>().into_iter().collect();
        Ok(json!({"ecus": ecus.iter().map(EcuInfo::to_json).collect::<Vec<_>>(), "seconds": seconds, "vin": vin, "session": sid, "folder": folder}))
    }

    /// The scan's entry for this address (and system), or a made-up one for an address that was not found.
    pub fn ecu_of(&self, address: i64, system: &str) -> EcuInfo {
        self.ecus
            .iter()
            .find(|e| e.address as i64 == address && (system.is_empty() || e.system == system))
            .cloned()
            .unwrap_or_else(|| EcuInfo::new(address as u32, "?", system, "", ""))
    }

    // -- reads --------------------------------------------------------------------------
    /// {did: symbols it is the first data-list group to carry}, built once per system.
    pub fn primary_map(&mut self, system: &str) -> &HashMap<i64, HashSet<String>> {
        if !self.prim.contains_key(system) {
            let mut seen: HashSet<String> = HashSet::new();
            let mut out = HashMap::new();
            for did in self.live.dids(system, "DataList") {
                let syms: HashSet<String> = self.live.signals(system, did).iter().map(|s| s.symbol.clone()).filter(|s| !seen.contains(s)).collect();
                seen.extend(syms.iter().cloned());
                out.insert(did, syms);
            }
            self.prim.insert(system.to_string(), out);
        }
        &self.prim[system]
    }

    /// The outcome of every exchange (ok: the ECU answered, even if with a refusal): proof of life for the car-off check, and
    /// for the recorder, which does not count the reads that found the car gone.
    pub fn seen(&mut self, address: i64, did: i64, ok: bool) {
        self.monitor.seen(address, did, mono(), ok);
        if ok {
            if let Some(r) = self.rec.as_mut() {
                r.answered(address as u32);
            }
        }
    }

    pub fn read_dids(&mut self, ecu: &EcuInfo, dids: Option<&[i64]>, kind: &str) -> Result<Value, ServiceError> {
        let t_now = now();
        let system = ecu.system.clone();
        self.hc()?;
        let list: Vec<i64> = match dids {
            Some(d) if !d.is_empty() => d.to_vec(),
            _ => self.live.dids(&system, "DataList").into_iter().filter(|d| !self.live.signals(&system, *d).is_empty()).collect(),
        };
        let mut out = vec![];
        for did in list {
            let sigs = self.live.signals(&system, did);
            let t0 = Instant::now();
            let data = self.hc.as_mut().unwrap().client(ecu.address).read_did(did as u16);
            let data = match data {
                Ok(d) => d,
                Err(UdsError::Io(e)) => return Err(e.into()),
                Err(e) => {
                    let gone = matches!(e, UdsError::NoResponse(_) | UdsError::Transport(_));
                    self.seen(ecu.address as i64, did, !gone);
                    if matches!(e, UdsError::Negative { nrc: 0x31, .. }) {
                        // requestOutOfRange: this car has no such group
                        self.monitor.unsupported.insert((ecu.address, did));
                    }
                    let refused = matches!(e, UdsError::Negative { .. });
                    if let Some(r) = self.rec.as_mut() {
                        r.failed(ecu, did, &e.to_string(), refused);
                    }
                    out.push(json!({"did": did, "error": e.to_string()}));
                    continue;
                }
            };
            let ms = t0.elapsed().as_secs_f64() * 1000.0;
            let values: Vec<Val> = sigs.iter().map(|s| value_of(&s.decode(&data))).collect();
            self.seen(ecu.address as i64, did, true);
            let primary: HashSet<String> = self.primary_map(&system).get(&did).cloned().unwrap_or_default();
            if let Some(r) = self.rec.as_mut() {
                r.timing(ecu, ms);
                let defs: Vec<Def> = sigs.iter().map(|s| Def { symbol: s.symbol.clone(), unit: unit_of(s) }).collect();
                r.observe(ecu, did, &defs, &values, t_now, kind, Some(&primary));
            }
            out.push(json!({"did": did, "values": values.iter().map(val_json).collect::<Vec<_>>()}));
        }
        Ok(json!({"t": t_now, "dids": out}))
    }

    pub fn live_values(&mut self, address: i64, system: &str, dids: Option<&[i64]>) -> Result<Value, ServiceError> {
        let ecu = self.ecu_of(address, system);
        self.read_dids(&ecu, dids, "poll")
    }

    pub fn dtcs(&mut self, address: i64, system: &str) -> Result<Value, ServiceError> {
        let ecu = self.ecu_of(address, system);
        let honda = HondaEcu { address: ecu.address, system_type: ecu.system_type.clone(), system: ecu.system.clone(), rule: String::new(), probes: HashMap::new() };
        let (avail, recs) = match self.hc()?.read_dtcs(&honda, 0xFF) {
            Ok(r) => r,
            Err(e @ (UdsError::NoResponse(_) | UdsError::Transport(_))) => {
                self.seen(ecu.address as i64, -1, false);
                return Err(e.into());
            }
            Err(e) => return Err(e.into()),
        };
        self.seen(ecu.address as i64, -1, true);
        let mine = self.owned(&ecu);
        let mut dtcs: Vec<Value> = vec![];
        for (r, info) in recs.iter().filter(|(r, _)| mine(&r.code)) {
            dtcs.push(json!({
                "code": r.code, "failure_type": r.failure_type, "status": r.status, "kind": r.kind(), "flags": r.flags(),
                "honda_code": info.as_ref().filter(|i| !i.honda_code.is_empty() && i.honda_code != "0-0").map_or(String::new(), |i| i.honda_code.clone()),
                "description": info.as_ref().map_or(String::new(), |i| i.description.clone()), "help": info.as_ref().map_or(String::new(), |i| i.help.clone()),
            }));
        }
        let incomplete: Vec<Value> = dtcs.iter().filter(|d| d["kind"] == "incomplete").cloned().collect();
        let dtcs: Vec<Value> = dtcs.into_iter().filter(|d| d["kind"] != "incomplete").collect();
        if let Some(rec) = self.rec.as_mut() {
            rec.selftests.update(ecu.name(), &dtc_records(&incomplete));
            let ch = rec.dtcs(&ecu, &dtcs, now());
            let any = |k: &str| ch[k].as_array().is_some_and(|a| !a.is_empty());
            if (any("appeared") || any("present")) && !ecu.system.is_empty() {
                // freeze data of a new code, or of one stored before we connected: first in line
                let frz: Vec<(EcuInfo, i64)> = self.live.dids(&ecu.system, "FreezeData").into_iter().filter(|d| !self.live.signals(&ecu.system, *d).is_empty()).map(|d| (ecu.clone(), d)).collect();
                self.monitor.snap_queue.splice(0..0, frz);
            }
        }
        Ok(json!({"available": avail, "dtcs": dtcs, "incomplete": incomplete}))
    }

    /// Where several systems share an address (the Accord's PCM: PGM-FI and CVT at 0x0E) the ECU sends one list; each code
    /// belongs to the first of them whose DTC table has it (each code is in one table), else the engine.
    fn owned(&self, ecu: &EcuInfo) -> Box<dyn Fn(&str) -> bool> {
        let mut sorted: Vec<&EcuInfo> = self.ecus.iter().collect();
        sorted.sort_by_key(|e| !ENGINE.contains(&e.system_type.as_str()));
        let systems: Vec<String> = sorted.iter().filter(|e| e.address == ecu.address && !e.system.is_empty()).map(|e| e.system.clone()).collect();
        if systems.len() < 2 {
            return Box::new(|_| true);
        }
        let (db, mine) = (self.db.clone(), ecu.system.clone());
        Box::new(move |code| systems.iter().find(|s| db.has_dtc(s, code)).unwrap_or(&systems[0]) == &mine)
    }

    pub fn clear(&mut self, address: i64, system: &str, confirm: bool) -> Result<Value, ServiceError> {
        if !confirm {
            return Err(ServiceError::value("clear needs confirm: true"));
        }
        let ecu = self.ecu_of(address, system);
        let hc = self.hc()?;
        hc.set_allow_clear(true);
        let r = hc.client(ecu.address).clear_dtcs();
        hc.set_allow_clear(false);
        r?;
        if let Some(rec) = self.rec.as_mut() {
            rec.cleared(&ecu, now());
        }
        Ok(json!({}))
    }

    pub fn read(&mut self, address: i64, did: i64) -> Result<Value, ServiceError> {
        let data = self.hc()?.client(address as u32).read_did(did as u16)?;
        let ascii: String = data.iter().map(|b| if *b < 0x80 { *b as char } else { '\u{fffd}' }).collect();
        Ok(json!({"hex": data.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" "), "ascii": ascii}))
    }

    pub fn ecu_info(&mut self, address: i64, system: &str) -> Result<Value, ServiceError> {
        let ecu = self.ecu_of(address, system);
        let db = self.db.clone();
        Ok(read_ecu_info(&db, &ecu.system, self.hc()?.client(ecu.address))?)
    }

    /// The system's data list (`dids`), plus the groups HDS reads for a freeze frame and for emissions readiness.
    pub fn catalog(&mut self, system: &str) -> Value {
        if let Some(c) = self.catalogs.get(system) {
            return c.clone();
        }
        let c = self.build_catalog(system);
        self.catalogs.insert(system.to_string(), c.clone());
        c
    }

    fn build_catalog(&mut self, system: &str) -> Value {
        let helps = self.help.for_system(system);
        let groups = |usage: &str| -> Vec<Value> {
            let mut out = vec![];
            for did in self.live.dids(system, usage) {
                let sigs = self.live.signals(system, did);
                if sigs.is_empty() {
                    continue;
                }
                let signals: Vec<Value> = sigs
                    .iter()
                    .map(|s| {
                        let states = s.converters.first().filter(|c| !c.states.is_empty()).map(|c| c.states.iter().map(|(_, _, t)| t.clone()).collect::<Vec<_>>());
                        let h = helps.get(&s.symbol);
                        json!({"symbol": s.symbol, "description": s.description, "unit": unit_of(s), "states": states,
                               "help": h.map(|x| x.0.clone()), "invalid_stopped": h.is_some_and(|x| x.1)})
                    })
                    .collect();
                out.push(json!({"did": did, "signals": signals}));
            }
            out
        };
        let usages: BTreeSet<String> = self.db.query("SELECT DISTINCT usage FROM can_packet_usage WHERE system=?", [system], |r| r.get(0)).into_iter().collect();
        json!({"system": system, "dids": groups("DataList"),
               "freeze": if usages.contains("FreezeData") { groups("FreezeData") } else { vec![] },
               "readiness": if usages.contains("OBD Status") { groups("OBD Status") } else { vec![] }})
    }

    // -- monitor ---------------------------------------------------------------------------
    /// Run at most one due background task. Returns seconds until the next one (None: nothing scheduled).
    pub fn tick(&mut self) -> Result<Option<f64>, ServiceError> {
        let wait = crate::actservice::tick(self);
        let base = self.monitor_tick()?;
        Ok(match (base, wait) {
            (None, w) => w,
            (b, None) => b,
            (Some(b), Some(w)) => Some(b.min(w)),
        })
    }

    fn monitor_tick(&mut self) -> Result<Option<f64>, ServiceError> {
        if self.hc.is_none() || (self.rec.is_none() && !self.monitor.asleep) {
            return Ok(None);
        }
        let now_m = mono();
        if !self.ident_queue.is_empty() && !self.monitor.asleep {
            let addr = self.ident_queue.remove(0);
            let standard = read_identity(self.hc.as_mut().unwrap().client(addr))?;
            // the F18x DIDs are refused on this car; the ECU ID and part number (HDS's own ways in) are what tells the
            // calibration of a module, so each system at this address records them next to whatever F18x answered
            let db = self.db.clone();
            let ecus: Vec<EcuInfo> = self.ecus.iter().filter(|e| e.address == addr).cloned().collect();
            let mut idents = vec![];
            for e in &ecus {
                let mut ident = standard.clone();
                if !e.system.is_empty() {
                    if let (Some(m), Value::Object(info)) = (ident.as_object_mut(), read_ecu_info(&db, &e.system, self.hc.as_mut().unwrap().client(addr))?) {
                        m.extend(info.into_iter().filter(|(_, v)| !v.is_null()));
                    }
                }
                idents.push(ident);
            }
            for (e, ident) in ecus.iter().zip(&idents) {
                for x in self.ecus.iter_mut().filter(|x| x.address == addr && x.system_type == e.system_type && x.system == e.system) {
                    x.ident = Some(ident.clone());
                }
                if let Some(rec) = self.rec.as_mut() {
                    rec.set_ident(e, ident);
                    rec.identity(e, ident);
                }
            }
            return Ok(Some(0.0));
        }
        let (due, task) = self.monitor.next_task(now_m);
        let (Some(due), Some(task)) = (due, task) else { return Ok(None) };
        if due > now_m {
            return Ok(Some(due - now_m));
        }
        let result = self.run(&task);
        let mut escaped = None;
        match result {
            Ok(()) => {}
            Err(e) if e.kind == "OSError" => {
                // the adapter went away: say so, and count it towards car off
                self.seen(-1, -1, false);
                (self.emit)("error", json!({"message": e.message}));
            }
            Err(e) if ["NoResponse", "TransportError", "NegativeResponse", "NotAllowed", "UdsError"].contains(&e.kind.as_str()) => {}
            Err(e) => escaped = Some(e), // a bug or a bad state: it propagates, after the task has moved on
        }
        // even a failed task moves on, so one bad task cannot repeat forever
        self.monitor.done(&task, mono());
        if self.rec.is_some() && self.monitor.car_gone() {
            self.end("car off");
            self.monitor.sleep(mono());
            (self.emit)("car", json!({"state": "off"}));
        }
        match escaped {
            Some(e) => Err(e),
            None => Ok(Some(0.0)),
        }
    }

    fn run(&mut self, task: &Task) -> Result<(), ServiceError> {
        match task {
            Task::Dtc(e) => {
                let r = self.dtcs(e.address as i64, &e.system)?;
                let mut data = json!({"address": e.address, "system": e.system});
                if let (Some(d), Some(r)) = (data.as_object_mut(), r.as_object()) {
                    d.extend(r.clone());
                }
                (self.emit)("codes", data);
            }
            Task::Watch(e, did) => {
                self.read_dids(e, Some(&[*did]), "watch")?;
            }
            Task::Snapshot(e, did) => {
                let kind = SNAPSHOT_KINDS.iter().find(|(usage, _)| self.live.dids(&e.system, usage).contains(did)).map_or("snapshot", |(_, k)| *k);
                self.read_dids(e, Some(&[*did]), kind)?;
            }
            Task::Probe(e) => {
                let ok = self.hc()?.ping(e.address)?;
                self.seen(e.address as i64, -1, ok);
            }
            Task::Wake => {
                let addrs: Vec<u32> = self.monitor.ecus.iter().map(|e| e.address).collect::<BTreeSet<_>>().into_iter().collect();
                if !addrs.is_empty() && !self.hc()?.present(&addrs, crate::uds::P2)?.is_empty() {
                    (self.emit)("car", json!({"state": "on"}));
                    self.monitor.asleep = false;
                    let mut started = self.scan(None)?;
                    if let Some(o) = started.as_object_mut() {
                        o.insert("state".into(), json!("started"));
                    }
                    (self.emit)("session", started);
                }
            }
        }
        Ok(())
    }

    pub fn status(&mut self) -> Value {
        let store_root = self.store().map(|s| s.root.to_string_lossy().into_owned()).unwrap_or_default();
        let mut out = json!({"connected": self.hc.is_some(), "adapter": self.spec, "vin": self.vin, "asleep": self.monitor.asleep,
                             "recording": self.rec.is_some(), "data_dir": store_root});
        let Some(r) = self.rec.as_mut() else {
            let o = out.as_object_mut().unwrap();
            for (k, v) in [("session", Value::Null), ("folder", Value::Null), ("started", Value::Null), ("reads", Value::Null), ("signals_seen", json!(0)), ("trip", Value::Null),
                           ("maps", Value::Null), ("alerts", json!([])), ("fuel_flow_lph", Value::Null), ("self_tests", json!({}))] {
                o.insert(k.into(), v);
            }
            crate::actservice::add_status(self, &mut out);
            return out;
        };
        let g = r.last_value("GAIR");
        let fc = r.last_value("FC") == Val::Text("ON".into());
        let flow = g.num().map(|g| round_to(fuel_flow_lph(g, fc), 2));
        let alerts: Vec<Value> = r.alerts.active(now()).into_iter().map(|a| json!({"id": a.id, "level": a.level, "title": a.title, "text": a.text})).collect();
        let o = out.as_object_mut().unwrap();
        for (k, v) in [
            ("session", json!(r.sid)), ("folder", json!(r.journal.folder.to_string_lossy())), ("started", json!(r.t0)),
            ("reads", json!({"ok": r.reads_ok, "failed": r.reads_failed})), ("signals_seen", json!(r.signals_seen())), ("trip", r.trip.summary()),
            ("maps", r.maps.out()), ("alerts", json!(alerts)), ("fuel_flow_lph", json!(flow)), ("self_tests", r.selftests.summary()),
        ] {
            o.insert(k.into(), v);
        }
        crate::actservice::add_status(self, &mut out);
        out
    }


    // -- maintenance --------------------------------------------------------------------------
    /// This car's frozen baselines and service markers (maintenance.rs). Reads the store only; it does not touch the car.
    pub fn maintenance(&mut self, vin: &str) -> Result<Value, ServiceError> {
        crate::maintenance::maintenance(self, vin)
    }

    /// Freeze the current scan as a healthy baseline, after the gate in maintenance::freeze.
    pub fn freeze_baseline(&mut self, vin: &str, label: Option<&str>) -> Result<Value, ServiceError> {
        crate::maintenance::freeze(self, vin, label)
    }

    pub fn marker_add(&mut self, vin: &str, kind: &str, note: &str, mileage_km: Option<f64>) -> Result<Value, ServiceError> {
        crate::maintenance::add_marker(self, vin, kind, note, mileage_km)
    }

    pub fn marker_remove(&mut self, id: i64) -> Result<Value, ServiceError> {
        crate::maintenance::remove_marker(self, id)
    }

    // -- report, export and the installation self-check ---------------------------------------
    /// The installation's self-check (selfcheck.rs): the data directory, the database, the audit trail, and what this
    /// machine has for talking to the car. Reads only; it never opens an adapter.
    pub fn self_check(&mut self) -> Result<Value, ServiceError> {
        let root = self.root()?;
        Ok(crate::selfcheck::run(&root, Some(&self.db)))
    }

    /// Read the car and build the health report against its newest baseline (report.rs). With `save` the HTML and the
    /// JSON are written under `<root>/reports/` and the reply carries both paths; without it they are empty strings.
    pub fn health_report(&mut self, vin: Option<&str>, save: bool) -> Result<Value, ServiceError> {
        let report = crate::report::build(self, vin)?;
        let (path, json_path) = if save {
            let (html_path, json_path) = crate::report::save(&report, &self.root()?)?;
            (html_path.display().to_string(), json_path.display().to_string())
        } else {
            (String::new(), String::new())
        };
        Ok(json!({
            "path": path,
            "json_path": json_path,
            "verdict": report["verdict"].clone(),
            "summary": report["summary"].clone(),
            "report": report,
        }))
    }

    /// The JSON export and the `.tar.gz` backup of the data directory (backup.rs). `out` names the JSON file; the
    /// archive sits next to it. Reads only; it never opens an adapter.
    pub fn export(&mut self, out: Option<&std::path::Path>) -> Result<Value, ServiceError> {
        let store = self.store()?;
        let root = store.root.clone();
        crate::backup::export(&store, &root, out)
    }

    // -- history ----------------------------------------------------------------------------
    pub fn vehicles(&mut self) -> Result<Value, ServiceError> {
        Ok(json!(self.store()?.vehicles()))
    }

    pub fn vehicle(&mut self, vin: &str) -> Result<Value, ServiceError> {
        Ok(history::vehicle(&*self.store()?, vin))
    }

    pub fn session_detail(&mut self, id: i64) -> Result<Value, ServiceError> {
        Ok(history::session_detail(&*self.store()?, id)?)
    }
}

#[allow(dead_code)]
fn _unused(_: &Service) {}
