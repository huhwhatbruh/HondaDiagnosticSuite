//! JSON-lines RPC over stdin/stdout, for the desktop app (hondascan-gui/) or any other front end. Port of hondascan/rpc.py.
//!
//! One request per line, answered in order (the bus is one resource, so everything runs on one thread):
//!   -> {"id": 1, "method": "scan", "params": {}}
//!   <- {"id": 1, "result": {...}}            or  {"id": 1, "error": {"message": "...", "type": "NoResponse"}}
//!   <- {"event": "scan" | "codes" | "car" | "session" | "error" | "actuator", "data": {...}}   unsolicited
//!
//! Methods: hello, ports, connect {adapter}, disconnect, scan {addresses?}, dtcs {address, system}, clear {address, system,
//! confirm: true}, catalog {system}, live {address, system, dids?}, read {address, did}, ecu_info {address, system}, status,
//! vehicles, vehicle {vin}, session {id}, maintenance {vin}, baseline_freeze {vin, label?}, marker_add {vin, kind, note,
//! mileage_km?}, marker_remove {id}, self_check, health_report {vin?, save?}, export {out?}, audit {vin?, limit?},
//! memory_read, memory_probe, ram_watch,
//! actuator_tests, actuator_start, actuator_stop, actuator_status, write_functions,
//! write_run {address, key, vin, confirm: true}, write_cancel, recovery {}, recover {adapter?}.
//! `recovery` reports the pending write/actuator record (if any); `recover` hands it back, using the current
//! connection or opening `adapter` just for that. Live values come as arrays in catalog order. `maintenance` reads the
//! frozen baselines and service markers; `baseline_freeze` stores the current healthy scan (a store-only write, never a
//! car write); markers record work done between sessions. `self_check` and `export` are offline: they read this
//! installation's data directory and never open an adapter. `health_report` reads the connected car and writes the
//! report (`save` defaults to true); `audit` is the write trail, newest first (`limit` defaults to 200, 0 means all).
//!
//! Between requests the service runs its monitor: watch list, trouble-code checks, snapshots, car on/off. A reader thread
//! hands lines over a channel, so this works the same on every platform.
use crate::db::Database;
use crate::memservice::num;
use crate::service::{Emit, Service, ServiceError};
use crate::sim::PROFILE_NAMES;
use crate::store::Store;
use crate::transport::{serial_ports, GD101_USB_ID};
use crate::value::{dumps, Json};
use serde_json::{json, Map, Value};
use std::io::{BufRead, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const IDLE: f64 = 1.0; // longest wait between monitor checks when nothing is due

fn type_error(m: String) -> ServiceError {
    ServiceError { kind: "TypeError".into(), message: m }
}

/// The keyword arguments of one call, checked the way Python's `**params` checks them.
struct Args<'a> {
    method: &'a str,
    p: &'a Map<String, Value>,
    used: Vec<&'static str>,
}

impl<'a> Args<'a> {
    fn new(method: &'a str, p: &'a Map<String, Value>) -> Args<'a> {
        Args { method, p, used: vec![] }
    }

    fn get(&mut self, k: &'static str) -> Option<&'a Value> {
        self.used.push(k);
        self.p.get(k)
    }

    fn required(&mut self, k: &'static str) -> Result<&'a Value, ServiceError> {
        self.get(k).ok_or_else(|| type_error(format!("{}() missing 1 required positional argument: '{k}'", self.method)))
    }

    fn int(&mut self, k: &'static str) -> Result<i64, ServiceError> {
        let v = self.required(k)?;
        match v {
            Value::Number(_) | Value::String(_) => num(v),
            other => Err(type_error(format!("int() argument must be a string or a number, not {}", crate::actservice::py_repr(other)))),
        }
    }

    fn string(&mut self, k: &'static str, default: &str) -> Result<String, ServiceError> {
        match self.get(k) {
            None => Ok(default.to_string()),
            Some(Value::String(s)) => Ok(s.clone()),
            Some(o) => Err(type_error(format!("{k} must be a string, not {}", crate::actservice::py_repr(o)))),
        }
    }

    /// An optional argument: absent or null is None, a string is Some, anything else is the existing type error.
    fn maybe_string(&mut self, k: &'static str) -> Result<Option<String>, ServiceError> {
        match self.get(k) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(s)) => Ok(Some(s.clone())),
            Some(o) => Err(type_error(format!("{k} must be a string, not {}", crate::actservice::py_repr(o)))),
        }
    }

    fn flag(&mut self, k: &'static str) -> bool {
        self.get(k).is_some_and(|v| v == &Value::Bool(true)) // Python compares `confirm is True`
    }

    fn done(&self) -> Result<(), ServiceError> {
        for k in self.p.keys() {
            if !self.used.contains(&k.as_str()) {
                return Err(type_error(format!("{}() got an unexpected keyword argument '{k}'", self.method)));
            }
        }
        Ok(())
    }
}

pub struct Session {
    pub svc: Service,
}

impl Session {
    pub fn new(db: Arc<Database>, emit: Emit, store: Option<Arc<Store>>) -> Session {
        Session { svc: Service::new(db, emit, store) }
    }

    pub fn hello(&mut self) -> Value {
        let data_dir = self.svc.store().map(|s| s.root.to_string_lossy().into_owned()).unwrap_or_default();
        let root = self.svc.store().ok().map(|s| s.root.clone());
        let pending = root.as_ref().and_then(|r| crate::audit::read_pending(r));
        let audit = root.as_ref().map(|r| crate::audit::writes_path(r).to_string_lossy().into_owned()).unwrap_or_default();
        json!({"db": self.svc.db.path.to_string_lossy(), "sim_profiles": PROFILE_NAMES.iter().filter(|p| **p != "empty").collect::<Vec<_>>(),
               "adapter": if self.svc.adapter.is_some() { json!(self.svc.spec) } else { Value::Null }, "data_dir": data_dir,
               "version": env!("CARGO_PKG_VERSION"), "audit": audit, "pending_recovery": pending,
               "autoconnect": std::env::var("HONDASCAN_ADAPTER").ok().filter(|s| !s.is_empty())}) // connect at start-up
    }

    pub fn ports(&self) -> Value {
        serial_ports().into_iter().map(|(d, v, p, prod)| json!({"device": d, "vid": v, "pid": p, "product": prod, "gd101": (v.as_str(), p.as_str()) == GD101_USB_ID})).collect::<Vec<_>>().into()
    }

    pub fn call(&mut self, method: &str, params: &Value) -> Result<Value, ServiceError> {
        let empty = Map::new();
        let p = match params {
            Value::Object(o) => o,
            Value::Null => &empty,
            _ => return Err(type_error("params must be an object".into())),
        };
        let mut a = Args::new(method, p);
        let s = &mut self.svc;
        let r = match method {
            "hello" => {
                a.done()?;
                self.hello()
            }
            "ports" => {
                a.done()?;
                self.ports()
            }
            "connect" => {
                let spec = a.string("adapter", "")?;
                if a.p.get("adapter").is_none() {
                    return Err(type_error("connect() missing 1 required positional argument: 'adapter'".into()));
                }
                a.done()?;
                s.connect(&spec)?
            }
            "disconnect" => {
                let reason = a.string("reason", "disconnected")?;
                a.done()?;
                s.disconnect(&reason)
            }
            "scan" => {
                let addrs = match a.get("addresses") {
                    None | Some(Value::Null) => None,
                    Some(Value::Array(v)) => Some(v.iter().map(|x| num(x).map(|n| n as u32)).collect::<Result<Vec<_>, _>>()?),
                    Some(o) => return Err(type_error(format!("addresses must be a list, not {}", crate::actservice::py_repr(o)))),
                };
                a.done()?;
                s.scan(addrs)?
            }
            "dtcs" => {
                let (addr, system) = (a.int("address")?, a.string("system", "")?);
                a.done()?;
                s.dtcs(addr, &system)?
            }
            "clear" => {
                let (addr, system, confirm) = (a.int("address")?, a.string("system", "")?, a.flag("confirm"));
                a.done()?;
                s.clear(addr, &system, confirm)?
            }
            "catalog" => {
                let system = a.string("system", "")?;
                a.done()?;
                s.catalog(&system)
            }
            "live" => {
                let (addr, system) = (a.int("address")?, a.string("system", "")?);
                let dids = match a.get("dids") {
                    None | Some(Value::Null) => None,
                    Some(Value::Array(v)) => Some(v.iter().map(num).collect::<Result<Vec<_>, _>>()?),
                    Some(o) => return Err(type_error(format!("dids must be a list, not {}", crate::actservice::py_repr(o)))),
                };
                a.done()?;
                s.live_values(addr, &system, dids.as_deref())?
            }
            "read" => {
                let (addr, did) = (a.int("address")?, a.int("did")?);
                a.done()?;
                s.read(addr, did)?
            }
            "ecu_info" => {
                let (addr, system) = (a.int("address")?, a.string("system", "")?);
                a.done()?;
                s.ecu_info(addr, &system)?
            }
            "status" => {
                a.done()?;
                s.status()
            }
            "vehicles" => {
                a.done()?;
                s.vehicles()?
            }
            "vehicle" => {
                let vin = a.string("vin", "")?;
                a.done()?;
                s.vehicle(&vin)?
            }
            "session" => {
                let id = a.int("id")?;
                a.done()?;
                s.session_detail(id)?
            }
            "memory_read" => {
                let addr = a.int("address")?;
                let m = a.required("memory_address")?.clone();
                let length = a.int("length")?;
                let fmt = a.get("fmt").cloned().unwrap_or(json!(0x24));
                let (extended, confirm) = (a.flag("extended"), a.flag("confirm"));
                a.done()?;
                s.memory_read(addr, &m, length, &fmt, extended, confirm)?
            }
            "memory_probe" => {
                let addr = a.int("address")?;
                let m = a.get("memory_address").cloned();
                let confirm = a.flag("confirm");
                a.done()?;
                s.memory_probe(addr, m.as_ref(), confirm)?
            }
            "ram_watch" => {
                let addr = a.int("address")?;
                let profile = a.string("profile", "ignition")?;
                let fmt = a.get("fmt").cloned().unwrap_or(json!(0x24));
                let extended = a.flag("extended");
                let block = a.get("block").map(num).transpose()?;
                let (force, confirm) = (a.flag("force_profile"), a.flag("confirm"));
                a.done()?;
                s.ram_watch(addr, &profile, &fmt, extended, block, force, confirm)?
            }
            "actuator_tests" => {
                a.done()?;
                s.actuator_tests()
            }
            "actuator_start" => {
                let addr = a.int("address")?;
                let test = a.string("test", "")?;
                let value = a.required("value")?.clone();
                let confirm = a.flag("confirm");
                a.done()?;
                s.actuator_start(addr, &test, &value, confirm)?
            }
            "actuator_stop" => {
                a.done()?;
                s.actuator_stop()
            }
            "actuator_status" => {
                a.done()?;
                s.actuator_status()
            }
            "write_functions" => {
                a.done()?;
                s.write_functions()
            }
            "write_run" => {
                let addr = a.int("address")?;
                let key = a.string("key", "")?;
                let vin = a.string("vin", "")?;
                let confirm = a.flag("confirm");
                a.done()?;
                s.write_run(addr, &key, &vin, confirm)?
            }
            "write_cancel" => {
                a.done()?;
                s.write_cancel()
            }
            "recovery" => {
                a.done()?;
                json!({"pending": s.pending()})
            }
            "recover" => {
                let adapter = a.string("adapter", "")?;
                a.done()?;
                s.recover(Some(adapter.as_str()).filter(|x| !x.trim().is_empty()))?
            }
            "maintenance" => {
                let vin = a.string("vin", "")?;
                if vin.trim().is_empty() {
                    return Err(ServiceError::value("maintenance needs the car's VIN"));
                }
                a.done()?;
                s.maintenance(&vin)?
            }
            "baseline_freeze" => {
                let (vin, label) = (a.string("vin", "")?, a.string("label", "")?);
                a.done()?;
                let label = label.trim();
                s.freeze_baseline(&vin, Some(label).filter(|l| !l.is_empty()))?
            }
            "marker_add" => {
                let (vin, kind) = (a.string("vin", "")?, a.string("kind", "")?);
                let note = a.string("note", "")?;
                if a.p.get("note").is_none() {
                    return Err(type_error("marker_add() missing 1 required positional argument: 'note'".into()));
                }
                let mileage = match a.get("mileage_km") {
                    None | Some(Value::Null) => None,
                    Some(Value::Number(n)) => n.as_f64(),
                    Some(o) => return Err(type_error(format!("mileage_km must be a number, not {}", crate::actservice::py_repr(o)))),
                };
                a.done()?;
                s.marker_add(&vin, &kind, &note, mileage)?
            }
            "marker_remove" => {
                let id = a.int("id")?;
                a.done()?;
                s.marker_remove(id)?
            }
            "self_check" => {
                a.done()?;
                s.self_check()?
            }
            "health_report" => {
                let vin = a.maybe_string("vin")?;
                let save = match a.get("save") {
                    None => true,
                    Some(v) => v.as_bool().unwrap_or(false),
                };
                a.done()?;
                s.health_report(vin.as_deref().filter(|v| !v.trim().is_empty()), save)?
            }
            "export" => {
                let out = a.maybe_string("out")?;
                a.done()?;
                s.export(out.as_deref().filter(|o| !o.trim().is_empty()).map(std::path::Path::new))?
            }
            "audit" => {
                let vin = a.maybe_string("vin")?;
                let limit = match a.get("limit") {
                    None | Some(Value::Null) => 200,
                    Some(n) => num(n)? as usize,
                };
                a.done()?;
                let root = s.root()?;
                json!({"path": crate::audit::writes_path(&root).to_string_lossy().into_owned(),
                       "records": crate::audit::read(&root, vin.as_deref().filter(|v| !v.trim().is_empty()), limit)})
            }
            _ => return Err(ServiceError::value(format!("unknown method {}", crate::actservice::py_repr(&json!(method))))),
        };
        Ok(r)
    }
}

/// One background step; a task never ends the backend: its error is reported and the loop goes on.
pub fn tick(s: &mut Session, send: &dyn Fn(Value)) -> f64 {
    match s.svc.tick() {
        Ok(next) => next.unwrap_or(IDLE),
        Err(e) => {
            eprintln!("{}: {}", e.kind, e.message);
            send(json!({"event": "error", "data": {"message": format!("{}: {}", e.kind, e.message)}}));
            IDLE
        }
    }
}

/// Serve requests from `inp` and write answers and events to `out` until the input ends; then disconnect.
pub fn serve(inp: Box<dyn BufRead + Send>, out: Arc<Mutex<dyn Write + Send>>, db: Option<Arc<Database>>, store: Option<Arc<Store>>) -> Result<(), String> {
    let out2 = out.clone();
    let send = move |msg: Value| {
        let mut o = out2.lock().unwrap();
        let _ = writeln!(o, "{}", dumps(&msg, Json::Compact));
        let _ = o.flush();
    };
    let (tx, rx) = channel::<Option<String>>();
    // `write_run` blocks the loop below, so the reader thread is the only place a `write_cancel` can be noticed while it runs.
    let write_cancel = Arc::new(AtomicBool::new(false));
    let cancel_seen = write_cancel.clone();
    std::thread::spawn(move || {
        for line in inp.lines() {
            match line {
                Ok(l) => {
                    if l.contains("\"write_cancel\"") && serde_json::from_str::<Value>(&l).is_ok_and(|v| v.get("method").and_then(Value::as_str) == Some("write_cancel")) {
                        cancel_seen.store(true, Ordering::Relaxed);
                    }
                    if tx.send(Some(l)).is_err() {
                        return;
                    }
                }
                Err(_) => break,
            }
        }
        let _ = tx.send(None);
    });
    let db = match db {
        Some(d) => d,
        None => Arc::new(Database::open(None)?),
    };
    let emit_send = send.clone();
    let emit: Emit = Arc::new(move |ev: &str, data: Value| emit_send(json!({"event": ev, "data": data})));
    let mut s = Session::new(db, emit, store);
    s.svc.write_cancel = write_cancel;
    let mut wait = IDLE;
    loop {
        let line = match rx.recv_timeout(Duration::from_secs_f64(wait.clamp(0.0, IDLE))) {
            Err(RecvTimeoutError::Timeout) => {
                wait = tick(&mut s, &send);
                continue;
            }
            Err(RecvTimeoutError::Disconnected) | Ok(None) => break,
            Ok(Some(l)) => l,
        };
        if line.trim().is_empty() {
            continue;
        }
        let mut rid = Value::Null;
        let answer = match serde_json::from_str::<Value>(&line) {
            Err(e) => Err(ServiceError { kind: "JSONDecodeError".into(), message: e.to_string() }),
            Ok(req) => {
                rid = req.get("id").cloned().unwrap_or(Value::Null);
                match req.get("method").and_then(Value::as_str) {
                    Some(m) => s.call(m, req.get("params").unwrap_or(&Value::Null)),
                    None => Err(ServiceError { kind: "KeyError".into(), message: "'method'".into() }),
                }
            }
        };
        match answer {
            Ok(r) => send(json!({"id": rid, "result": r})),
            Err(e) => {
                let message = if e.message.is_empty() { e.kind.clone() } else { e.message.clone() };
                send(json!({"id": rid, "error": {"message": message, "type": e.kind}}));
            }
        }
        wait = tick(&mut s, &send); // one background task between requests, so a busy app cannot starve it
    }
    s.svc.disconnect("backend stopped");
    Ok(())
}
