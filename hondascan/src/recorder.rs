//! One recorded session: everything read goes through here into the journal, the store and the summary. Port of
//! hondascan/recorder.py.
//!
//! Statistics (count, min, max, sum, last value) are kept in memory for every signal read, by anyone (the app's live view,
//! the monitor's watch list, snapshots), and written to the store when the session flushes; samples of the watch list are
//! stored at most once per SAMPLE_EVERY seconds per data group; snapshots are stored whole.
use crate::alerts::Alerts;
use crate::journal::{py, Journal};
use crate::condition::Condition;
use crate::mapbaseline::MIN_SESSIONS;
use crate::maps::Maps;
use crate::selftests::{Dtc, SelfTests};
use crate::store::{EcuRow, StatRow, Store};
use crate::transport::can::{now, CaptureLog};
use crate::trip::{Stat, Trip};
use crate::value::{round_to, Named, Val};
use regex::Regex;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const SAMPLE_EVERY: f64 = 1.0;
pub const FLUSH_EVERY: f64 = 5.0;
pub const SUMMARY_EVERY: f64 = 60.0;
const ENGINE: [&str; 2] = ["PGM-FI", "DIESEL"];
const LIVE: [&str; 3] = ["poll", "watch", "snapshot"]; // reads of present values (snapshot = a data-list group)
const WHOLE: [&str; 3] = ["snapshot", "freeze", "status"]; // stored in full: snapshots, freeze-frame and OBD-status groups

/// What an ECU's data group holds in its first reply after the ECU starts answering, before its first sensor conversion:
/// (symbol, value). Seen on a 2014 Odyssey's road-trip recording (two drives of about 105 minutes): the first read of the
/// group after each wake-up had ECT -40 degC, IAT -40 degC, TCAT -40 degC, BARO 0 kPa and fuel level 0 %, and the next
/// read of every one of them was a plausible value. Raw 0 of these converters is the value listed here.
///
/// The rule is the first read of the group since the ECU started answering AND this value, never the value alone: a real
/// -40 degC on a cold day is a later read and counts (an unanswered read re-arms the rule, see `Recorder::failed`). Another
/// first read of a group (VB during cranking, NE, VSP, ...) is a measurement and counts at once: the cranking dip of the
/// battery (9.1, 10.0, 10.2, 11.2 V in the first reads of a session) is in the statistics.
const WAKE_SENTINELS: [(&str, f64); 6] = [("ECT", -40.0), ("IAT", -40.0), ("TCAT", -40.0), ("BARO", 0.0), ("FLI", 0.0), ("FLEVELF", 0.0)];

fn wake_sentinel(symbol: &str, v: &Val) -> bool {
    v.num().is_some_and(|x| WAKE_SENTINELS.iter().any(|(s, sentinel)| *s == symbol && x == *sentinel))
}

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
    /// Who answered the scan's all-ECU wakeup (None: none was sent); shown in the summary only when set.
    pub wakeup: Option<Vec<u32>>,
    pub trip: Trip,
    pub maps: Maps,
    pub condition: Condition,
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
    silence: BTreeMap<(u32, String), u64>, // (address, key) -> unanswered reads since that ECU last answered
    latency: Vec<(String, Stat)>,       // system -> Stat, in ms
    t_flush: Instant,
    t_summary: Instant,
    identified: HashSet<(u32, String)>,
    woken: HashSet<(String, i64)>,        // (system, did) read since that ECU last started answering
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
            store, capture, vin: vin.into(), adapter: adapter.into(), ecus, t0, journal, sid, prev_codes, scan_s, wakeup: None,
            trip: Trip::new(), maps, condition: Condition::new(), alerts: Alerts::new(), stats: vec![], last_sample: HashMap::new(),
            codes: vec![], selftests: SelfTests::default(), first_read: HashSet::new(), appeared: vec![], gone: vec![], cleared: vec![],
            reads_ok: 0, reads_failed: 0, rejected: vec![], unanswered: vec![], silence: BTreeMap::new(), latency: vec![],
            t_flush: Instant::now(), t_summary: Instant::now(), identified: HashSet::new(), woken: HashSet::new(),
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
        let first = self.woken.insert((system.clone(), did)); // the first reply of this group since the ECU started answering
        for (i, (d, v)) in defs.iter().zip(values).enumerate() {
            let key = (system.clone(), did, i);
            let idx = match self.stats.iter().position(|(k, _)| *k == key) {
                Some(p) => p,
                None => {
                    self.stats.push((key, SigStat { symbol: d.symbol.clone(), unit: d.unit.clone(), stat: Stat::default(), last: Val::Null }));
                    self.stats.len() - 1
                }
            };
            if first && wake_sentinel(&d.symbol, v) {
                continue; // not a measurement yet (WAKE_SENTINELS): out of min/max/mean, the trends, the trip, the maps
            }
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
                self.condition.feed(t, &named);
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
            // silent: when it answers again its first reply of every group is a wake-up reply once more (WAKE_SENTINELS)
            self.woken.retain(|(s, _)| s != ecu.name());
            match self.unanswered.iter_mut().find(|(k, _)| *k == key) {
                Some((_, n)) => *n += 1,
                None => self.unanswered.push((key.clone(), 1)),
            }
            *self.silence.entry((ecu.address, key)).or_default() += 1;
        }
        if first {
            self.journal.event("read_failed", json!({"system": ecu.system, "address": ecu.address, "did": did, "error": err, "refused": refused}));
        }
    }

    /// This ECU answered: its unanswered reads so far were not the car switching off. Another ECU's answer says nothing about
    /// it (2026-10-15: the SRS, which stays powered, answered while the PCM had already gone quiet).
    pub fn answered(&mut self, address: u32) {
        self.silence.retain(|(a, _), _| *a != address);
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
        let mut out = json!({
            "id": self.sid, "vin": self.vin, "adapter": self.adapter, "started": self.t0,
            "ended": if reason.is_some() { json!(now()) } else { Value::Null }, "end_reason": reason,
            "duration_s": round_to(now() - self.t0, 1), "scan_s": self.scan_s,
            "ecus": self.ecus.iter().map(EcuInfo::to_json).collect::<Vec<_>>(),
            "reads": {"ok": self.reads_ok, "failed": self.reads_failed}, "rejected": rejected, "unanswered": unanswered,
            "latency_ms": lat, "trip": trip, "maps": self.maps.out(),
            // Rust-only (no Python original to port from): the factory-calibration checks in maps.rs, kept out
            // of "maps" and "notes" above so those two stay byte-for-byte comparable with the frozen fixtures.
            "factory": self.maps.factory_summary(), "factory_notes": self.maps.factory_notes(),
            // Rust-only too: the Condition layer's per-regime statistics (condition.rs), read back by `condition::evaluate`.
            "condition": self.condition.out(!self.codes.iter().any(|(_, d)| matches!(d["kind"].as_str(), Some("confirmed" | "pending"))), &present),
            "first_session": first,
            "codes": {"present": present, "new_since_last": new, "resolved_since_last": resolved,
                      "appeared": self.appeared, "gone": self.gone, "cleared": self.cleared},
            "self_tests": self.selftests.summary(),
            "notes": notes, "signals_seen": self.stats.len(),
        });
        if let Some(w) = &self.wakeup {
            out["wakeup"] = json!(w); // only when sent, so the frozen summaries of earlier sessions stay comparable
        }
        let missing = self.missing_core();
        if !missing.is_empty() {
            out["missing"] = crate::preflight::missing_json(&missing); // likewise: only when a core system is silent
        }
        out
    }

    /// The systems every CAN Honda has (preflight.rs) that did not answer the scan.
    pub fn missing_core(&self) -> Vec<(u32, &'static str)> {
        let got: Vec<(u32, &str)> = self.ecus.iter().map(|e| (e.address, e.system_type.as_str())).collect();
        crate::preflight::missing_core(&got)
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
        // No note on the engine-off voltage: key-on seconds with unknown loads say nothing about the charge (the 2014 Odyssey read
        // 11.6 V for 15 s right after a drive, and a load test found its battery fine). The trip summary still shows the numbers.
        if let Some(max) = trip["coolant_c"]["max"].as_f64() {
            if max > 105.0 {
                out.push(format!("coolant reached {} degC", py(&trip["coolant_c"]["max"])));
            }
        }
        // The engine speed a misfire counter was read at is not the speed of the misfires (the counters are polled slowly), so no rpm here.
        for (cyl, n) in self.alerts.misfires() {
            out.push(format!("misfires counted on cylinder {cyl}: {n:.0}"));
        }
        out.extend(grouped_cell_notes(self.maps.notes()));
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
            for ((_, key), n) in std::mem::take(&mut self.silence) {
                if let Some(p) = self.unanswered.iter().position(|(k, _)| *k == key) {
                    self.unanswered[p].1 -= n;
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

/// One per-cell note that `maps::notes()` gives, read back: its signal's label, the cell's words up to the baseline tag
/// (`head`), the mean, the cell's range and what flagged it.
struct CellNote {
    head: String,
    mean: f64,
    rpm: (f64, Option<f64>), // lower and upper edge; None for the top bin ("above")
    load: (f64, Option<f64>),
    basis: Basis,
}

#[derive(Clone, Copy, PartialEq)]
enum Basis {
    Own,          // the car's own history decided the flag
    Fixed(usize), // the fixed limit did, with this many earlier sessions of the car
}

/// The words `maps.rs` gives one flagged cell of fuel trim or knock retard (`Maps::text`, then the tag of `Maps::flag`).
const CELL_NOTE: &str = r"^(?P<head>(?P<label>long-term fuel trim|knock retard|cylinder [1-4] knock retard) averaged (?P<mean>-?[0-9.]+)(?: deg)? at (?P<rpm>above [0-9]+|[0-9]+-[0-9]+) rpm, (?P<load>above [0-9.]+|[0-9.]+-[0-9.]+) g/rev \([0-9]+ samples\)(?P<corr>: the PCM is correcting by [0-9]+ %)?)(?P<tail>.*)$";

fn edges(s: &str) -> Option<(f64, Option<f64>)> {
    match s.strip_prefix("above ") {
        Some(lo) => Some((lo.parse().ok()?, None)),
        None => {
            let (lo, hi) = s.split_once('-')?;
            Some((lo.parse().ok()?, Some(hi.parse().ok()?)))
        }
    }
}

fn read_cell_note(cell: &Regex, fixed: &Regex, note: &str) -> Option<(String, CellNote)> {
    let c = cell.captures(note)?;
    let tail = &c["tail"];
    let basis = if tail.contains("[own history]") { Basis::Own } else { Basis::Fixed(fixed.captures(tail)?[1].parse().ok()?) };
    let cn = CellNote { head: c["head"].to_string(), mean: c["mean"].parse().ok()?, rpm: edges(&c["rpm"])?, load: edges(&c["load"])?, basis };
    Some((c["label"].to_string(), cn))
}

/// The lowest lower edge to the highest upper edge, "lo+" when the top bin is among them.
fn cover(spans: impl Iterator<Item = (f64, Option<f64>)>, dp: usize) -> String {
    let (mut lo, mut hi, mut open) = (f64::MAX, f64::MIN, false);
    for (l, h) in spans {
        lo = lo.min(l);
        match h {
            Some(h) => hi = hi.max(h),
            None => open = true,
        }
    }
    if open {
        format!("{lo:.dp$}+")
    } else {
        format!("{lo:.dp$}-{hi:.dp$}")
    }
}

/// One line for one signal's flagged cells: how many, the speed and load they cover, the worst cell (the highest knock
/// retard, or the furthest trim from 1.00) in its own words, and the baseline tag once.
fn group_line(label: &str, cells: &[CellNote]) -> String {
    let k = cells.len();
    let own = cells.iter().filter(|c| c.basis == Basis::Own).count();
    let used = cells.iter().find_map(|c| if let Basis::Fixed(u) = c.basis { Some(u) } else { None }).unwrap_or(0);
    let tag = if own == k {
        "[own history]".to_string()
    } else if own == 0 {
        format!("[fixed limit: {used} earlier sessions, {MIN_SESSIONS} needed for the car's own baseline]")
    } else {
        format!("[own history for {own} of {k} cells; fixed limit for the other {}: {used} earlier sessions, {MIN_SESSIONS} needed for the car's own baseline]", k - own)
    };
    let key = |c: &CellNote| if label == "long-term fuel trim" { (c.mean - 1.0).abs() } else { c.mean };
    let worst = cells.iter().max_by(|a, b| key(a).total_cmp(&key(b))).expect("a group has a cell");
    let rpm = cover(cells.iter().map(|c| c.rpm), 0);
    let load = cover(cells.iter().map(|c| c.load), 1);
    let word = if k == 1 { "cell" } else { "cells" };
    format!("{label} flagged in {k} {word} across {rpm} rpm, {load} g/rev; worst: {} {tag}", worst.head)
}

/// `maps::notes()` gives one note per flagged cell of fuel trim and knock retard, and of each cylinder's knock. Here they
/// become one line per signal (`group_line`). Nothing is judged again: the flags and numbers are the cells' own, and any
/// other note passes through unchanged, after the grouped lines.
pub fn grouped_cell_notes(notes: Vec<String>) -> Vec<String> {
    let cell = Regex::new(CELL_NOTE).expect("CELL_NOTE is a valid pattern");
    let fixed = Regex::new(r"\[fixed limit: ([0-9]+) earlier sessions").expect("a valid pattern");
    let mut groups: Vec<(String, Vec<CellNote>)> = vec![];
    let mut others = vec![];
    for n in notes {
        match read_cell_note(&cell, &fixed, &n) {
            Some((label, c)) => match groups.iter_mut().find(|(l, _)| *l == label) {
                Some((_, v)) => v.push(c),
                None => groups.push((label, vec![c])),
            },
            None => others.push(n),
        }
    }
    let mut out: Vec<String> = groups.iter().map(|(label, cells)| group_line(label, cells)).collect();
    out.extend(others);
    out
}

#[cfg(test)]
mod tests {
    use super::grouped_cell_notes;

    const OWN: &str = " +4.1 standard deviations from this car's 4 earlier sessions (1.20 +/- 0.30) [own history]";
    const FIXED: &str = " [fixed limit: 4 earlier sessions, 3 needed for the car's own baseline]";

    #[test]
    fn own_history_cells_get_one_tag_for_the_whole_signal() {
        let notes = vec![
            format!("knock retard averaged 3.0 deg at 2000-2500 rpm, 0.6-0.8 g/rev (30 samples):{OWN}"),
            format!("knock retard averaged 5.5 deg at 2500-3000 rpm, 0.8-1.0 g/rev (40 samples):{OWN}"),
        ];
        assert_eq!(
            grouped_cell_notes(notes),
            ["knock retard flagged in 2 cells across 2000-3000 rpm, 0.6-1.0 g/rev; worst: knock retard averaged 5.5 deg at 2500-3000 rpm, 0.8-1.0 g/rev (40 samples) [own history]"]
        );
    }

    #[test]
    fn a_mix_of_own_history_and_the_fixed_limit_is_said_once_with_counts() {
        let notes = vec![
            format!("knock retard averaged 3.0 deg at 2000-2500 rpm, 0.6-0.8 g/rev (30 samples):{OWN}"),
            format!("knock retard averaged 6.0 deg at 1500-2000 rpm, 1.0-1.2 g/rev (25 samples){FIXED}"),
        ];
        assert_eq!(
            grouped_cell_notes(notes),
            ["knock retard flagged in 2 cells across 1500-2500 rpm, 0.6-1.2 g/rev; worst: knock retard averaged 6.0 deg at 1500-2000 rpm, 1.0-1.2 g/rev (25 samples) [own history for 1 of 2 cells; fixed limit for the other 1: 4 earlier sessions, 3 needed for the car's own baseline]"]
        );
    }

    #[test]
    fn the_worst_trim_is_the_furthest_from_one_and_keeps_its_correction_words() {
        let notes = vec![
            "long-term fuel trim averaged 0.85 at 2000-2500 rpm, 0.6-0.8 g/rev (25 samples): the PCM is correcting by 15 %".to_string() + FIXED,
            "long-term fuel trim averaged 1.12 at 1000-1500 rpm, above 1.2 g/rev (50 samples): the PCM is correcting by 12 %".to_string() + FIXED,
        ];
        let out = grouped_cell_notes(notes);
        assert_eq!(out.len(), 1, "{out:?}");
        assert!(out[0].starts_with("long-term fuel trim flagged in 2 cells across 1000-2500 rpm, 0.6+ g/rev; worst: long-term fuel trim averaged 0.85 at 2000-2500 rpm, 0.6-0.8 g/rev (25 samples): the PCM is correcting by 15 % [fixed limit"), "{out:?}");
        assert_eq!(out[0].matches('[').count(), 1, "the tag is written once: {out:?}");
    }

    #[test]
    fn one_cell_is_one_line_and_other_notes_keep_their_place() {
        let notes = vec![
            "spark advance averaged 24.0 deg at 1500-2000 rpm, 0.2-0.4 g/rev (30 samples) [own history]".to_string(),
            format!("cylinder 4 knock retard averaged 4.0 deg at 3000-3500 rpm, 0.2-0.4 g/rev (20 samples){FIXED}"),
            "cylinder 4 knock retard averaged 4.0 deg over the session: +3.5 standard deviations [own history]".to_string(),
        ];
        assert_eq!(
            grouped_cell_notes(notes),
            [
                "cylinder 4 knock retard flagged in 1 cell across 3000-3500 rpm, 0.2-0.4 g/rev; worst: cylinder 4 knock retard averaged 4.0 deg at 3000-3500 rpm, 0.2-0.4 g/rev (20 samples) [fixed limit: 4 earlier sessions, 3 needed for the car's own baseline]",
                "spark advance averaged 24.0 deg at 1500-2000 rpm, 0.2-0.4 g/rev (30 samples) [own history]",
                "cylinder 4 knock retard averaged 4.0 deg over the session: +3.5 standard deviations [own history]",
            ]
        );
    }
}
