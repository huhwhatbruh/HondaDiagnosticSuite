//! One recorded session: everything read goes through here into the journal, the store and the summary. Port of
//! hondascan/recorder.py.
//!
//! Statistics (count, min, max, sum, last value) are kept in memory for every signal read, by anyone (the app's live view,
//! the monitor's watch list, snapshots), and written to the store when the session flushes; samples of the watch list are
//! stored at most once per SAMPLE_EVERY seconds per data group; snapshots are stored whole.
use crate::alerts::Alerts;
use crate::journal::{py, Journal};
use crate::maps::Maps;
use crate::selftests::{Dtc, SelfTests};
use crate::store::{EcuRow, StatRow, Store};
use crate::transport::can::{now, CaptureLog};
use crate::trip::{Stat, Trip};
use crate::value::{round_to, Named, Val};
use serde_json::{json, Map, Value};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const SAMPLE_EVERY: f64 = 1.0;
pub const FLUSH_EVERY: f64 = 5.0;
pub const SUMMARY_EVERY: f64 = 60.0;
const ENGINE: [&str; 2] = ["PGM-FI", "DIESEL"];
const LIVE: [&str; 3] = ["poll", "watch", "snapshot"]; // reads of present values (snapshot = a data-list group)
const WHOLE: [&str; 3] = ["snapshot", "freeze", "status"]; // stored in full: snapshots, freeze-frame and OBD-status groups

/// One system found on the car, as the service lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct EcuInfo {
    pub address: u32,
    pub system_type: String,
    pub system: String,
    pub display: String,
    pub rule: String,
    pub ident: Option<Value>,
}

impl EcuInfo {
    pub fn new(address: u32, system_type: &str, system: &str, display: &str, rule: &str) -> EcuInfo {
        EcuInfo { address, system_type: system_type.into(), system: system.into(), display: display.into(), rule: rule.into(), ident: None }
    }

    /// The name a system is recorded under: its database system, or its HDS type when it has none.
    pub fn name(&self) -> &str {
        if self.system.is_empty() { &self.system_type } else { &self.system }
    }

    pub fn to_json(&self) -> Value {
        let mut m = json!({"address": self.address, "system_type": self.system_type, "system": self.system, "display": self.display, "rule": self.rule});
        if let Some(i) = &self.ident {
            m["ident"] = i.clone();
        }
        m
    }
}

/// A signal's symbol and unit, as Recorder.observe takes them.
#[derive(Debug, Clone)]
pub struct Def {
    pub symbol: String,
    pub unit: String,
}

type CodeKey = (String, String, i64);

fn dtc_key(ecu: &EcuInfo, d: &Value) -> CodeKey {
    (ecu.name().to_string(), d["code"].as_str().unwrap_or("").to_string(), d["failure_type"].as_i64().unwrap_or(0))
}

fn val_json(v: &Val) -> Value {
    match v {
        Val::Num(n) => json!(n),
        Val::Text(s) => json!(s),
        Val::Null => Value::Null,
    }
}

/// Python's `str(value)` of a decoded value.
fn val_str(v: &Val) -> String {
    match v {
        Val::Num(n) => py(&json!(n)),
        Val::Text(s) => s.clone(),
        Val::Null => "None".into(),
    }
}

struct SigStat {
    symbol: String,
    unit: String,
    stat: Stat,
    last: Val,
}

pub struct Recorder {
    store: Arc<Store>,
    capture: Option<Arc<CaptureLog>>,
    pub vin: String,
    pub adapter: String,
    pub ecus: Vec<EcuInfo>,
    pub t0: f64,
    pub journal: Journal,
    pub sid: i64,
    prev_codes: Option<BTreeSet<CodeKey>>, // None: first session of this car
    scan_s: f64,
    pub trip: Trip,
    pub maps: Maps,
    pub alerts: Alerts,
    stats: Vec<((String, i64, usize), SigStat)>,
    last_sample: HashMap<(String, i64), f64>,
    codes: Vec<(CodeKey, Value)>, // current state, in the order they were read
    pub selftests: SelfTests,
    pub first_read: HashSet<String>,
    pub appeared: Vec<Value>,
    pub gone: Vec<Value>,
    pub cleared: Vec<Value>,
    pub reads_ok: u64,
    pub reads_failed: u64,
    rejected: Vec<(String, String)>,    // "system DID" -> error: the ECU answered with a negative response
    unanswered: Vec<(String, u64)>,     // "system DID" -> reads that got no answer (timeout, transport error)
    silence: Vec<(u32, String)>,        // (address, key) of the unanswered reads since that ECU last answered
    latency: Vec<(String, Stat)>,       // system -> Stat, in ms
    t_flush: Instant,
    t_summary: Instant,
    identified: HashSet<(u32, String)>,
}

impl Recorder {
    pub fn new(store: Arc<Store>, capture: Option<Arc<CaptureLog>>, vin: &str, adapter: &str, ecus: Vec<EcuInfo>, scan_s: f64) -> std::io::Result<Recorder> {
        Recorder::with_map_limits(store, capture, vin, adapter, ecus, scan_s, None)
    }

    /// As `new`, with the maps' (warm-engine temperature, samples per cell) overridden: for tests that run a short drive.
    pub fn with_map_limits(store: Arc<Store>, capture: Option<Arc<CaptureLog>>, vin: &str, adapter: &str, ecus: Vec<EcuInfo>, scan_s: f64, limits: Option<(f64, f64)>) -> std::io::Result<Recorder> {
        let t0 = now();
        let stamp = crate::localtime::folder_stamp(t0);
        let base: PathBuf = store.root.join("vehicles").join(vin).join(&stamp);
        let mut folder = base.clone();
        let mut n = 1;
        while folder.exists() {
            // two sessions in one second (a quick rescan) get their own folders
            n += 1;
            folder = PathBuf::from(format!("{}-{n}", base.display()));
        }
        let journal = Journal::new(&folder)?;
        let sid = store.start_session(vin, t0, adapter, &folder.to_string_lossy()).map_err(std::io::Error::other)?;
        if let Some(c) = &capture {
            c.attach(&folder.join("frames.log"))?;
        }
        let prev = store.last_session(vin, sid);
        let prev_codes = prev.map(|p| store.session_codes(p).into_iter().collect());
        let prior = store.prior_maps(vin, sid, 30);
        let maps = limits.map_or_else(|| Maps::new(&prior), |(warm, min_cell)| Maps::with_limits(&prior, warm, min_cell));
        let r = Recorder {
            store, capture, vin: vin.into(), adapter: adapter.into(), ecus, t0, journal, sid, prev_codes, scan_s,
            trip: Trip::new(), maps, alerts: Alerts::new(), stats: vec![], last_sample: HashMap::new(),
            codes: vec![], selftests: SelfTests::default(), first_read: HashSet::new(), appeared: vec![], gone: vec![], cleared: vec![],
            reads_ok: 0, reads_failed: 0, rejected: vec![], unanswered: vec![], silence: vec![], latency: vec![],
            t_flush: Instant::now(), t_summary: Instant::now(), identified: HashSet::new(),
        };
        r.journal.event("session_start", json!({"vin": vin, "adapter": adapter, "scan_s": scan_s, "ecus": r.ecus.iter().map(EcuInfo::to_json).collect::<Vec<_>>()}));
        Ok(r)
    }

    // -- observations -------------------------------------------------------------
    /// `primary`: the symbols this group is the main source of (the first data-list group that carries them). Only those feed
    /// the trip maths: other groups may repeat a symbol with stale data.
    #[allow(clippy::too_many_arguments)]
    pub fn observe(&mut self, ecu: &EcuInfo, did: i64, defs: &[Def], values: &[Val], t: f64, kind: &str, primary: Option<&HashSet<String>>) {
        let system = ecu.name().to_string();
        self.reads_ok += 1;
        let mut named = Named::new();
        for (i, (d, v)) in defs.iter().zip(values).enumerate() {
            let key = (system.clone(), did, i);
            let idx = match self.stats.iter().position(|(k, _)| *k == key) {
                Some(p) => p,
                None => {
                    self.stats.push((key, SigStat { symbol: d.symbol.clone(), unit: d.unit.clone(), stat: Stat::default(), last: Val::Null }));
                    self.stats.len() - 1
                }
            };
            let st = &mut self.stats[idx].1;
            st.last = v.clone();
            if let Some(n) = v.num() {
                st.stat.add(n);
            }
            if primary.is_none_or(|p| p.contains(&d.symbol)) {
                named.entry(d.symbol.clone()).or_insert_with(|| v.clone());
            }
        }
        if LIVE.contains(&kind) {
            let role = ecu.system_type.as_str();
            if ENGINE.contains(&role) {
                self.trip.engine(t, &named);
                self.maps.feed(t, &named);
                self.alerts.feed(t, &named);
            } else if role == "HONDA ABS" {
                self.trip.brakes(t, &named);
            } else if role == "METER" {
                self.trip.meter(t, &named);
            }
        }
        let jvals: Value = values.iter().map(val_json).collect();
        if WHOLE.contains(&kind) || (kind == "watch" && t - self.last_sample.get(&(system.clone(), did)).copied().unwrap_or(0.0) >= SAMPLE_EVERY) {
            self.last_sample.insert((system.clone(), did), t);
            self.store.sample(self.sid, t, &system, did, kind, &jvals);
        }
        if WHOLE.contains(&kind) {
            self.journal.event("snapshot", json!({"system": system, "did": did, "values": jvals}));
        }
        self.tick();
    }

    /// A read that failed. `refused`: the ECU said no (negative response); otherwise it did not answer, which on a car that is
    /// on points at a timeout (P2) too short for it.
    pub fn failed(&mut self, ecu: &EcuInfo, did: i64, err: &str, refused: bool) {
        self.reads_failed += 1;
        let key = format!("{} {:04X}", ecu.name(), did);
        let first = !self.rejected.iter().any(|(k, _)| *k == key) && !self.unanswered.iter().any(|(k, _)| *k == key);
        if refused {
            if !self.rejected.iter().any(|(k, _)| *k == key) {
                self.rejected.push((key.clone(), err.into()));
            }
        } else {
            match self.unanswered.iter_mut().find(|(k, _)| *k == key) {
                Some((_, n)) => *n += 1,
                None => self.unanswered.push((key.clone(), 1)),
            }
            self.silence.push((ecu.address, key));
        }
        if first {
            self.journal.event("read_failed", json!({"system": ecu.system, "address": ecu.address, "did": did, "error": err, "refused": refused}));
        }
    }

    /// This ECU answered: its unanswered reads so far were not the car switching off. Another ECU's answer says nothing about
    /// it (2026-10-15: the SRS, which stays powered, answered while the PCM had already gone quiet).
    pub fn answered(&mut self, address: u32) {
        self.silence.retain(|(a, _)| *a != address);
    }

    pub fn timing(&mut self, ecu: &EcuInfo, ms: f64) {
        match self.latency.iter_mut().find(|(k, _)| k == ecu.name()) {
            Some((_, s)) => s.add(ms),
            None => {
                let mut s = Stat::default();
                s.add(ms);
                self.latency.push((ecu.name().to_string(), s));
            }
        }
    }

    /// The latest value of the first signal read with this symbol, or Null.
    pub fn last_value(&self, symbol: &str) -> Val {
        self.stats.iter().find(|(_, s)| s.symbol == symbol).map_or(Val::Null, |(_, s)| s.last.clone())
    }

    /// The latest value and range of every signal read so far, for read-only viewers (the phone gateway, phone.rs).
    pub fn latest(&self) -> Vec<Value> {
        self.stats
            .iter()
            .map(|((system, did, _), st)| {
                json!({"system": system, "did": did, "symbol": st.symbol, "unit": st.unit, "value": val_json(&st.last),
                       "lo": (st.stat.n > 0).then_some(st.stat.lo), "hi": (st.stat.n > 0).then_some(st.stat.hi)})
            })
            .collect()
    }

    pub fn signals_seen(&self) -> usize {
        self.stats.len()
    }

    /// (system, did) of every signal read so far, in the order first read.
    pub fn stat_keys(&self) -> Vec<(String, i64)> {
        self.stats.iter().map(|((s, d, _), _)| (s.clone(), *d)).collect()
    }

    /// A DTC read of one system. Returns {'appeared', 'gone', 'present'} against the previous read.
    pub fn dtcs(&mut self, ecu: &EcuInfo, dtcs: &[Value], t: f64) -> Value {
        let system = ecu.name().to_string();
        let mut now_map: Vec<(CodeKey, Value)> = vec![];
        for d in dtcs {
            let mut full = d.clone();
            full["system"] = json!(system);
            let k = dtc_key(ecu, d);
            match now_map.iter_mut().find(|(x, _)| *x == k) {
                Some(e) => e.1 = full,
                None => now_map.push((k, full)),
            }
        }
        let before: Vec<(CodeKey, Value)> = self.codes.iter().filter(|(k, _)| k.0 == system).cloned().collect();
        let first = !self.first_read.contains(&system);
        self.first_read.insert(system.clone());
        let (mut appeared, mut gone, mut present) = (vec![], vec![], vec![]);
        for (k, d) in &now_map {
            if first || !before.iter().any(|(b, _)| b == k) {
                let event = if first { "present" } else { "appeared" };
                self.store_dtc(ecu, d, t, event);
                if first { present.push(d.clone()) } else { appeared.push(d.clone()) }
            }
        }
        for (k, d) in &before {
            if !now_map.iter().any(|(x, _)| x == k) {
                self.store_dtc(ecu, d, t, "gone");
                gone.push(d.clone());
            }
        }
        self.codes.retain(|(k, _)| k.0 != system);
        self.codes.extend(now_map);
        self.appeared.extend(appeared.clone());
        self.gone.extend(gone.clone());
        let codes: Vec<String> = dtcs.iter().map(|d| format!("{}-{:02X} {}", py(&d["code"]), d["failure_type"].as_i64().unwrap_or(0), py(&d["kind"]))).collect();
        self.journal.event(
            "dtc_read",
            json!({"system": system, "address": ecu.address, "count": dtcs.len(), "codes": codes, "appeared": appeared, "gone": gone, "present": present}),
        );
        json!({"appeared": appeared, "gone": gone, "present": present})
    }

    fn store_dtc(&self, ecu: &EcuInfo, d: &Value, t: f64, event: &str) {
        self.store.dtc(
            self.sid, &self.vin, t, ecu.name(), ecu.address, d["code"].as_str().unwrap_or(""), d["failure_type"].as_i64().unwrap_or(0),
            d["status"].as_i64().unwrap_or(0), d["kind"].as_str().unwrap_or(""), event, d.get("description").and_then(Value::as_str).unwrap_or(""),
        );
    }

    pub fn cleared(&mut self, ecu: &EcuInfo, t: f64) {
        let system = ecu.name().to_string();
        let mine: Vec<(CodeKey, Value)> = self.codes.iter().filter(|(k, _)| k.0 == system).cloned().collect();
        for (_, d) in &mine {
            self.store_dtc(ecu, d, t, "cleared");
            self.cleared.push(d.clone());
        }
        self.codes.retain(|(k, _)| k.0 != system);
        self.journal.event("dtc_cleared", json!({"system": system, "address": ecu.address}));
    }

    /// The service learned an identity for this system: what Python does by writing `e['ident']` into the ECU dict the
    /// recorder shares with the service. It shows in the summary's ECU list and is recorded at the end if not before.
    pub fn set_ident(&mut self, ecu: &EcuInfo, ident: &Value) {
        for e in self.ecus.iter_mut().filter(|e| e.address == ecu.address && e.system_type == ecu.system_type && e.system == ecu.system) {
            e.ident = Some(ident.clone());
        }
    }

    pub fn identity(&mut self, ecu: &EcuInfo, ident: &Value) {
        let key = (ecu.address, ecu.system_type.clone());
        if self.identified.insert(key) {
            self.store.ecu(self.sid, &EcuRow { address: ecu.address, system_type: &ecu.system_type, system: &ecu.system, display: &ecu.display }, ident);
            self.journal.event("identity", json!({"address": ecu.address, "system": ecu.system, "ident": ident}));
            // a module that now reports another ECU ID or part number than in this car's last session: new software
            // (a dealer update), another calibration, or another module. The tool cannot tell which; the owner can.
            let was = self.store.identities(&self.vin).into_iter().rev().find(|r| r["session"] != self.sid && r["address"] == ecu.address && r["system"] == ecu.system);
            let now = |k: &str| ident.get(k).filter(|v| v.is_string()).cloned().unwrap_or(Value::Null);
            if let Some(was) = was.filter(|w| (!now("ecu_id").is_null() && w["ecu_id"] != now("ecu_id")) || (!now("part_number").is_null() && w["part_number"] != now("part_number"))) {
                self.journal.event(
                    "calibration_changed",
                    json!({"address": ecu.address, "system": ecu.system, "session_before": was["session"], "before": {"ecu_id": was["ecu_id"], "part_number": was["part_number"]},
                           "now": {"ecu_id": now("ecu_id"), "part_number": now("part_number")}}),
                );
            }
        }
    }

    pub fn event(&self, ev: &str, data: Value) {
        self.journal.event(ev, data);
    }

    // -- output ---------------------------------------------------------------------
    pub fn tick(&mut self) {
        let n = Instant::now();
        if n.duration_since(self.t_flush) >= Duration::from_secs_f64(FLUSH_EVERY) {
            self.t_flush = n;
            if let Err(e) = self.store.flush() {
                self.journal.event("store_flush_failed", json!({"error": e.to_string()}));
            }
            self.journal.flush();
        }
        if n.duration_since(self.t_summary) >= Duration::from_secs_f64(SUMMARY_EVERY) {
            self.t_summary = n;
            self.write(None);
        }
    }

    pub fn summary(&self, reason: Option<&str>) -> Value {
        let present: Vec<Value> = self.codes.iter().map(|(_, d)| d.clone()).collect();
        let first = self.prev_codes.is_none();
        let keys_now: HashSet<&CodeKey> = self.codes.iter().map(|(k, _)| k).collect();
        let new: Vec<Value> = match &self.prev_codes {
            None => vec![],
            Some(prev) => self.codes.iter().filter(|(k, _)| !prev.contains(k)).map(|(_, d)| d.clone()).collect(),
        };
        let resolved: Vec<Value> = match &self.prev_codes {
            None => vec![],
            Some(prev) => prev
                .iter()
                .filter(|k| !keys_now.contains(k) && self.first_read.contains(&k.0))
                .map(|(s, c, f)| json!({"system": s, "code": c, "failure_type": f, "kind": "was present"}))
                .collect(),
        };
        let lat: Map<String, Value> = self
            .latency
            .iter()
            .map(|(k, v)| (k.clone(), json!(format!("{:.0}/{:.0}/{:.0}", v.lo, v.sum / v.n as f64, v.hi))))
            .collect();
        let trip = self.trip.summary();
        let rejected: Map<String, Value> = self.rejected.iter().map(|(k, v)| (k.clone(), json!(v))).collect();
        let unanswered: Map<String, Value> = self.unanswered.iter().map(|(k, v)| (k.clone(), json!(v))).collect();
        let notes = self.notes(&trip);
        json!({
            "id": self.sid, "vin": self.vin, "adapter": self.adapter, "started": self.t0,
            "ended": if reason.is_some() { json!(now()) } else { Value::Null }, "end_reason": reason,
            "duration_s": round_to(now() - self.t0, 1), "scan_s": self.scan_s,
            "ecus": self.ecus.iter().map(EcuInfo::to_json).collect::<Vec<_>>(),
            "reads": {"ok": self.reads_ok, "failed": self.reads_failed}, "rejected": rejected, "unanswered": unanswered,
            "latency_ms": lat, "trip": trip, "maps": self.maps.out(),
            // Rust-only (no Python original to port from): the factory-calibration checks in maps.rs, kept out
            // of "maps" and "notes" above so those two stay byte-for-byte comparable with the frozen fixtures.
            "factory": self.maps.factory_summary(), "factory_notes": self.maps.factory_notes(),
            "first_session": first,
            "codes": {"present": present, "new_since_last": new, "resolved_since_last": resolved,
                      "appeared": self.appeared, "gone": self.gone, "cleared": self.cleared},
            "self_tests": self.selftests.summary(),
            "notes": notes, "signals_seen": self.stats.len(),
        })
    }

    /// Plain hints with their thresholds; they are hondascan's rules of thumb, not Honda's limits.
    pub fn notes(&self, trip: &Value) -> Vec<String> {
        let mut out = vec![];
        let bv = &trip["battery_v"];
        // Honda's PCM commands 12.5-14.5 V over LIN (import-car.com, "Honda LIN Line Controlled Charging Systems"); the ECU's own
        // VB reads under the alternator target, by up to about 0.5 V at idle (2015 Accord, session 115, 2026-10-18: 10 s means
        // down to 11.99 V on a sound system). This checks the whole session's mean (13.04 V there), so 12.2 V still leaves room;
        // the live alert, which sees 10 s at a time, uses alerts::VB_LOW (11.8 V).
        if let Some(mean) = bv["engine_running"]["mean"].as_f64() {
            if !(12.2..=14.9).contains(&mean) {
                out.push(format!("charging voltage averaged {} V (Honda's charging control targets 12.5-14.5 V)", py(&bv["engine_running"]["mean"])));
            }
        }
        if let Some(min) = bv["engine_off"]["min"].as_f64() {
            if min < 12.2 {
                out.push(format!("battery fell to {} V with the engine off (below 12.2 V suggests a low charge)", py(&bv["engine_off"]["min"])));
            }
        }
        if let Some(max) = trip["coolant_c"]["max"].as_f64() {
            if max > 105.0 {
                out.push(format!("coolant reached {} degC", py(&trip["coolant_c"]["max"])));
            }
        }
        for (cyl, n) in self.alerts.misfires() {
            let rpm: Vec<String> = self.alerts.rise_rpms(cyl).iter().map(|r| format!("{r:.0}")).collect();
            out.push(format!("misfires counted on cylinder {cyl}: {n:.0}{}", if rpm.is_empty() { String::new() } else { format!(" (at {} rpm)", rpm.join(", ")) }));
        }
        out.extend(self.maps.notes());
        if trip["gaps"].as_i64().unwrap_or(0) != 0 {
            out.push(format!("{} gaps longer than 5 s in the engine data (distance not bridged across them)", trip["gaps"]));
        }
        if !self.rejected.is_empty() {
            out.push(format!("{} data groups were refused by the car (negative response, see Bus)", self.rejected.len()));
        }
        if !self.unanswered.is_empty() {
            out.push(format!(
                "{} reads of {} data groups got no answer while the car was on (see Bus; a timeout too short for the ECU?)",
                self.unanswered.iter().map(|(_, n)| n).sum::<u64>(),
                self.unanswered.len()
            ));
        }
        if self.capture.as_ref().is_some_and(|c| c.capped()) {
            out.push("the frame capture reached its size cap and stopped".into());
        }
        out
    }

    fn write(&mut self, reason: Option<&str>) -> Value {
        let s = self.summary(reason);
        let rows: Vec<StatRow> = self
            .stats
            .iter()
            .map(|((system, did, idx), st)| StatRow {
                system: system.clone(), did: *did, idx: *idx as i64, symbol: st.symbol.clone(), unit: st.unit.clone(), n: st.stat.n as i64,
                lo: (st.stat.n > 0).then_some(st.stat.lo), hi: (st.stat.n > 0).then_some(st.stat.hi), total: st.stat.sum, last: val_str(&st.last),
            })
            .collect();
        self.store.stats(self.sid, &rows);
        if let Err(e) = self.store.flush() {
            self.journal.event("store_flush_failed", json!({"error": e.to_string()}));
        }
        self.journal.write_summary(&s);
        s
    }

    pub fn stop(&mut self, reason: &str) -> Value {
        if reason == "car off" {
            // the reads that found the car gone are not the car failing to answer
            for (_, key) in std::mem::take(&mut self.silence) {
                if let Some(p) = self.unanswered.iter().position(|(k, _)| *k == key) {
                    self.unanswered[p].1 -= 1;
                    if self.unanswered[p].1 == 0 {
                        self.unanswered.remove(p);
                    }
                }
            }
        }
        for e in self.ecus.clone() {
            self.identity(&e, &e.ident.clone().unwrap_or_else(|| json!({})));
        }
        let s = self.write(Some(reason));
        let _ = self.store.end_session(self.sid, now(), reason, &s);
        self.journal.event("session_end", json!({"reason": reason}));
        self.journal.close();
        if let Some(c) = &self.capture {
            c.detach();
        }
        s
    }
}

/// Selftests records from DTC dicts, as the service passes the incomplete ones.
pub fn dtc_records(dtcs: &[Value]) -> Vec<Dtc> {
    dtcs.iter()
        .map(|d| Dtc {
            code: d["code"].as_str().unwrap_or("").into(),
            failure_type: d["failure_type"].as_i64().unwrap_or(0),
            description: d.get("description").and_then(Value::as_str).unwrap_or("").into(),
        })
        .collect()
}
