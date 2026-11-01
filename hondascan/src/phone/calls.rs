//! The calls a phone may make to the backend beyond the live document: a fixed table, one row per route, each tied to one
//! backend method with its parameters named and typed here. Nothing a client sends names a method or adds a parameter: a
//! parameter not in the row is dropped, one that does not fit its type is a 400. This is the whole surface (the desktop app's
//! own, minus the memory reads and the export, which stay with `hondascan-rpc`; `clear` is in the table although the desktop
//! app has no button for it):
//!
//!   reads            ports, status, dtcs, catalog, live, ecu_info, vehicles, vehicle, session, actuator tests and status,
//!                    write functions, recovery, maintenance, health report, self check, audit
//!   the store        reference snapshot (baseline_freeze), service markers: they write this machine's store, never the car
//!   the car session  connect, disconnect, scan, recover (hand control back)
//!   actuator tests   start (needs `confirm: true`), stop
//!   writes           run (needs `confirm: true` and the VIN), cancel
//!   clear            one system's codes (needs `confirm: true`; held, see `CLEAR_HELD`)
//!
//! The backend enforces what HDS enforces (the per-system whitelist, the VIN match, the conditions, the old value read first,
//! the time limit; actservice.rs, writeservice.rs, service.rs) and `READ_ONLY_RELEASE` in the client. The gateway adds: the
//! route is fixed, `confirm` must be the JSON boolean `true` in the body (a query string cannot carry it), a packaged
//! release refuses the whole acting group at the door, a `reflash*` write key needs `HONDASCAN_PHONE_REFLASH=1` (none is in
//! the backend's whitelist today: `write::WRITES` has the learn and meter-maintenance functions), and a test started from the phone is stopped when the phone has been
//! silent for `HANDBACK_AFTER` (the app polls every 0.25-0.5 s, so a dropped link, a closed app or a dead battery hands control
//! back to the ECU instead of waiting for the test's own time limit).
use super::Shared;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// A phone silent this long while it holds a running actuator test gets the test stopped.
pub const HANDBACK_AFTER: Duration = Duration::from_secs(6);
/// Codes are cleared on a simulated car only until the owner lifts the hold (clearing codes on a real car is held back).
pub const CLEAR_HELD: bool = true;

#[derive(Clone, Copy)]
enum P {
    Int,
    Text(usize),
    Vin,
    Adapter,
    /// optional: a list of at most 16 addresses; every other parameter is required unless wrapped in `Opt`
    Addresses,
    /// free text for a label or note, printable characters only
    Note(usize),
    /// a plain number (a mileage)
    Number,
    /// a whole number, at most 1000 (a record limit)
    Limit,
    Flag,
    /// the parameter may be left out
    Opt(&'static P),
}

#[derive(Clone, Copy, PartialEq)]
enum Group {
    Read,
    Session,
    Actuate,
    Write,
    Clear,
}

struct Route {
    path: &'static str,
    post: bool,
    method: &'static str,
    params: &'static [(&'static str, P)],
    /// the body must say `"confirm": true`
    confirm: bool,
    wait: Duration,
    group: Group,
}

const SHORT: Duration = Duration::from_secs(5);
const SCAN: Duration = Duration::from_secs(60);
/// A write can be a long transfer; the backend's single thread is busy for all of it.
const WRITE: Duration = Duration::from_secs(900);

const fn r(path: &'static str, method: &'static str, params: &'static [(&'static str, P)]) -> Route {
    Route { path, post: false, method, params, confirm: false, wait: SHORT, group: Group::Read }
}

const fn act(path: &'static str, method: &'static str, params: &'static [(&'static str, P)], group: Group, confirm: bool, wait: Duration) -> Route {
    Route { path, post: true, method, params, confirm, wait, group }
}

const AT: &[(&str, P)] = &[("address", P::Int), ("system", P::Text(24))];

const VIN: &[(&str, P)] = &[("vin", P::Vin)];

static ROUTES: &[Route] = &[
    r("/v1/ports", "ports", &[]),
    r("/v1/status", "status", &[]),
    r("/v1/dtcs", "dtcs", AT),
    r("/v1/catalog", "catalog", &[("system", P::Text(24))]),
    r("/v1/live", "live", AT),
    r("/v1/ecu_info", "ecu_info", AT),
    r("/v1/vehicles", "vehicles", &[]),
    r("/v1/vehicle", "vehicle", &[("vin", P::Vin)]),
    r("/v1/session", "session", &[("id", P::Int)]),
    r("/v1/actuator/tests", "actuator_tests", &[]),
    r("/v1/actuator/status", "actuator_status", &[]),
    r("/v1/write/functions", "write_functions", &[]),
    r("/v1/recovery", "recovery", &[]),
    r("/v1/maintenance", "maintenance", VIN),
    r("/v1/health_report", "health_report", &[("vin", P::Vin), ("save", P::Opt(&P::Flag))]),
    r("/v1/self_check", "self_check", &[]),
    r("/v1/audit", "audit", &[("vin", P::Opt(&P::Vin)), ("limit", P::Opt(&P::Limit))]),
    act("/v1/connect", "connect", &[("adapter", P::Adapter)], Group::Session, false, SCAN),
    act("/v1/disconnect", "disconnect", &[], Group::Session, false, SHORT),
    act("/v1/scan", "scan", &[("addresses", P::Addresses)], Group::Session, false, SCAN),
    act("/v1/recover", "recover", &[("adapter", P::Adapter)], Group::Session, false, SCAN),
    act("/v1/actuator/start", "actuator_start", &[("address", P::Int), ("test", P::Text(48)), ("value", P::Int)], Group::Actuate, true, SHORT),
    act("/v1/actuator/stop", "actuator_stop", &[], Group::Session, false, SHORT),
    act("/v1/write/run", "write_run", &[("address", P::Int), ("key", P::Text(48)), ("vin", P::Vin)], Group::Write, true, WRITE),
    act("/v1/write/cancel", "write_cancel", &[], Group::Session, false, SHORT),
    act("/v1/clear", "clear", AT, Group::Clear, true, SHORT),
    act("/v1/baseline/freeze", "baseline_freeze", &[("vin", P::Vin), ("label", P::Opt(&P::Note(80)))], Group::Session, false, SHORT),
    act("/v1/marker/add", "marker_add", &[("vin", P::Vin), ("kind", P::Text(24)), ("note", P::Note(2000)), ("mileage_km", P::Opt(&P::Number))], Group::Session, false, SHORT),
    act("/v1/marker/remove", "marker_remove", &[("id", P::Int)], Group::Session, false, SHORT),
];

/// Every route path, for the 405 on a wrong method.
pub fn is_call(path: &str) -> bool {
    ROUTES.iter().any(|r| r.path == path)
}

type Pending = Arc<Mutex<HashMap<String, Sender<Value>>>>;

/// The backend's single thread, reached through the same channel the gateway's own snapshot requests use.
pub struct Relay {
    /// dropped when the backend's input has ended, so that the merged reader sees its last sender go
    tx: Mutex<Option<Sender<String>>>,
    pending: Pending,
    next: AtomicUsize,
    /// when an authenticated request last arrived, and whether the phone holds a running actuator test
    seen: Mutex<Instant>,
    holding: AtomicBool,
}

impl Relay {
    pub fn new(tx: Sender<String>) -> Relay {
        Relay { tx: Mutex::new(Some(tx)), pending: Arc::default(), next: AtomicUsize::new(0), seen: Mutex::new(Instant::now()), holding: AtomicBool::new(false) }
    }

    /// The tap hands the backend's answers to ids "phone:r..." here; true when one was waited for.
    pub fn deliver(&self, msg: &Value) -> bool {
        let Some(waiting) = msg["id"].as_str().and_then(|id| self.pending.lock().unwrap().remove(id)) else { return false };
        let _ = waiting.send(msg.clone());
        true
    }

    pub fn touch(&self) {
        *self.seen.lock().unwrap() = Instant::now();
    }

    /// Err((status, text)) when the backend did not answer or refused.
    fn ask(&self, method: &str, params: Value, wait: Duration) -> Result<Value, (&'static str, String)> {
        let id = format!("phone:r{}", self.next.fetch_add(1, Ordering::Relaxed));
        let (reply, answer) = channel();
        self.pending.lock().unwrap().insert(id.clone(), reply);
        let sent = self.tx.lock().unwrap().as_ref().is_some_and(|tx| tx.send(json!({"id": id, "method": method, "params": params}).to_string()).is_ok());
        let got = if sent { answer.recv_timeout(wait).ok() } else { None };
        self.pending.lock().unwrap().remove(&id);
        match got {
            Some(m) if m.get("result").is_some() => Ok(m["result"].clone()),
            // the backend's own words: they say which condition failed
            Some(m) => Err(("422 Unprocessable Entity", m["error"]["message"].as_str().or(m["error"].as_str()).unwrap_or("refused").to_string())),
            None => Err(("504 Gateway Timeout", "the backend did not answer in time".into())),
        }
    }

    /// Stop a phone-held test once the phone has been silent too long. Runs once a second until the backend is gone.
    pub fn watchdog(self: Arc<Self>, closed: Arc<AtomicBool>) {
        while !closed.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_secs(1));
            if self.holding.load(Ordering::Relaxed) && self.seen.lock().unwrap().elapsed() > HANDBACK_AFTER {
                self.holding.store(false, Ordering::Relaxed);
                let _ = self.ask("actuator_stop", json!({}), SHORT);
            }
        }
        self.tx.lock().unwrap().take();
    }
}

fn is_vin(s: &str) -> bool {
    s.len() == 17 && s.chars().all(|c| c.is_ascii_alphanumeric())
}

/// An adapter spec as the backend takes it (`gd101-j2534`, `sim:drive`, a device path): no spaces or shell characters.
fn is_adapter(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.chars().all(|c| c.is_ascii_alphanumeric() || "_:.-/".contains(c))
}

fn text_ok(s: &str, max: usize) -> bool {
    s.len() <= max && s.chars().all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c))
}

/// One parameter from its raw text (query) or JSON value (body).
fn typed(p: P, raw: &Value) -> Option<Value> {
    let as_text = || raw.as_str().map(str::to_string);
    let as_int = || raw.as_i64().or_else(|| raw.as_str().and_then(|s| s.parse().ok())).filter(|_| !raw.is_boolean());
    match p {
        P::Int => as_int().map(|n| json!(n)),
        P::Text(max) => as_text().filter(|s| text_ok(s, max)).map(|s| json!(s)),
        P::Vin => as_text().filter(|s| is_vin(s)).map(|s| json!(s.to_ascii_uppercase())),
        P::Adapter => as_text().filter(|s| is_adapter(s)).map(|s| json!(s)),
        P::Note(max) => as_text().map(|s| s.trim().to_string()).filter(|s| s.chars().count() <= max && s.chars().all(|c| !c.is_control())).map(|s| json!(s)),
        P::Number => raw.as_f64().filter(|n| n.is_finite()).map(|n| json!(n)),
        P::Limit => as_int().filter(|n| (1..=1000).contains(n)).map(|n| json!(n)),
        P::Flag => raw.as_bool().or_else(|| raw.as_str().and_then(|s| s.parse().ok())).map(|b| json!(b)),
        P::Opt(inner) => typed(*inner, raw),
        // a list of at most 16 addresses
        P::Addresses => raw.as_array().filter(|v| v.len() <= 16).and_then(|v| v.iter().map(|x| x.as_i64()).collect::<Option<Vec<_>>>()).map(|v| json!(v)),
    }
}

fn query_pairs(query: &str) -> Map<String, Value> {
    query.split('&').filter_map(|kv| kv.split_once('=')).map(|(k, v)| (k.to_string(), json!(v))).collect()
}

fn text(status: &'static str, msg: impl Into<String>) -> (&'static str, String) {
    (status, json!({"error": msg.into()}).to_string())
}

/// Make the call for `path` (a row of ROUTES) and return (status, JSON body); None when `path` is not a call route.
/// `method` is the HTTP method. `sim_only` says the connected adapter is a simulator (for CLEAR_HELD).
pub fn call(relay: &Relay, path: &str, method: &str, query: &str, body: &[u8], sim_only: bool) -> Option<(&'static str, String)> {
    let route = ROUTES.iter().find(|r| r.path == path)?;
    if route.post != (method == "POST") {
        return Some(("405 Method Not Allowed", json!({"error": "not allowed"}).to_string()));
    }
    let acting = matches!(route.group, Group::Actuate | Group::Write | Group::Clear);
    if acting && crate::READ_ONLY_RELEASE {
        return Some(text("403 Forbidden", "this build is read-only"));
    }
    if route.group == Group::Clear && CLEAR_HELD && !sim_only {
        return Some(text("403 Forbidden", "clearing codes is on hold for a real car"));
    }
    let given: Map<String, Value> = if route.post {
        if body.is_empty() {
            Map::new()
        } else {
            match serde_json::from_slice::<Value>(body) {
                Ok(Value::Object(o)) => o,
                _ => return Some(text("400 Bad Request", "the body must be a JSON object")),
            }
        }
    } else {
        query_pairs(query)
    };
    let mut params = Map::new();
    for (name, ty) in route.params {
        let Some(raw) = given.get(*name).filter(|v| !v.is_null()) else {
            // `scan` takes no addresses to scan everything; every other parameter is required unless it is an `Opt`
            if matches!(ty, P::Addresses | P::Opt(_)) {
                continue;
            }
            return Some(text("400 Bad Request", format!("{name} is required")));
        };
        match typed(*ty, raw) {
            Some(v) => params.insert((*name).into(), v),
            None => return Some(text("400 Bad Request", format!("{name} is not valid"))),
        };
    }
    if route.method == "write_run" && params["key"].as_str().is_some_and(|k| k.starts_with("reflash")) && std::env::var("HONDASCAN_PHONE_REFLASH").as_deref() != Ok("1") {
        return Some(text("403 Forbidden", "reflash from the phone is off (HONDASCAN_PHONE_REFLASH=1 turns it on)"));
    }
    if route.confirm {
        if given.get("confirm") != Some(&json!(true)) {
            return Some(text("400 Bad Request", "confirm: true is required"));
        }
        params.insert("confirm".into(), json!(true));
    }
    let started = route.method == "actuator_start";
    Some(match relay.ask(route.method, Value::Object(params), route.wait) {
        Ok(mut v) => {
            if started {
                relay.touch();
                relay.holding.store(true, Ordering::Relaxed);
            }
            // the test is over (stopped, or ended by its own limit): nothing is left to hand back
            if route.method == "actuator_stop" || (route.method == "actuator_status" && v["active"] == json!(false)) {
                relay.holding.store(false, Ordering::Relaxed);
            }
            scrub(&mut v);
            ("200 OK", v.to_string())
        }
        Err((status, msg)) => text(status, msg),
    })
}

/// Folder and file paths are the Pi's own; the phone has no use for them.
fn scrub(v: &mut Value) {
    match v {
        Value::Object(o) => {
            for k in ["folder", "dir", "json_path", "data_dir", "path"] {
                o.remove(k);
            }
            o.values_mut().for_each(scrub);
        }
        Value::Array(a) => a.iter_mut().for_each(scrub),
        _ => {}
    }
}

/// Whether the cached snapshot says the connected adapter is a simulator.
pub fn sim_adapter(cache: &Shared) -> bool {
    let c = cache.lock().unwrap();
    c.snapshot.as_ref().and_then(|(v, _)| v["adapter"].as_str().map(crate::preflight::simulated)).unwrap_or(false)
}
