//! What gets read while the car is on, and how often. Port of hondascan/monitor.py: the backend runs one due task whenever
//! it is idle, so the app's own requests never wait behind more than one bus exchange.
//!
//! - watch list (WATCH): data groups worth a regular look, each at its own interval. A group the app has just read counts as
//!   read, so the live view and the monitor never fetch the same group twice.
//! - trouble codes of every system every DTC_EVERY seconds (a new code is reported at once, with its freeze data).
//! - a full snapshot (every data, freeze-data and OBD-status group of every system) at the start and every SNAPSHOT_EVERY
//!   seconds, one group per idle slot.
//! - car off: after a failed read the next task is a quick presence check (TesterPresent, no retry, one P2), so the verdict
//!   does not wait behind slow reads. Each check asks the next address along, so one module that goes quiet on its own (an
//!   ACC-only unit) is not taken for the car. ABSENT_AFTER failures in a row end the session; then every WAKE_EVERY seconds
//!   the systems found are probed (TesterPresent), and when one answers a new session starts.
use crate::livedata::LiveData;
use crate::recorder::EcuInfo;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// HDS system type -> [(DID, seconds between reads)], in the order the Python dict lists them (ties go to the first).
pub fn watch(system_type: &str) -> &'static [(i64, f64)] {
    match system_type {
        // PGM-FI 0x2664 carries the brake switch BKSW; CVT 0x2322 the four wheel speeds (VLVABSFL..RR), both live on the 2015
        // Accord, whose ABS does not answer on the DLC (session 2026-10-12_091210).
        "PGM-FI" => &[(0x2610, 0.5), (0x2612, 0.5), (0x2660, 1.0), (0x2664, 0.5), (0x2611, 2.0), (0x2662, 2.0), (0x2694, 2.0), (0x2613, 30.0), (0x2663, 10.0), (0x266C, 30.0)],
        "DIESEL" => &[(0x2610, 0.5), (0x2612, 0.5)],
        "CVT" => &[(0x2301, 1.0), (0x2322, 0.5)], // shaft speeds, CVTF temperature; ratio, pressure, wheel speeds
        "HONDA ABS" => &[(0x4004, 0.5), (0x4005, 0.5), (0x4008, 300.0)],
        "METER" => &[(0x7022, 30.0), (0x7029, 30.0), (0x702A, 10.0), (0x702B, 5.0)],
        "TPMS" => &[(0x6001, 30.0)],
        _ => &[],
    }
}

pub const DTC_EVERY: f64 = 120.0;
pub const SNAPSHOT_EVERY: f64 = 600.0;
pub const ABSENT_AFTER: u32 = 4;
pub const WAKE_EVERY: f64 = 5.0;
pub const SNAPSHOT_USAGES: [&str; 3] = ["DataList", "FreezeData", "OBD Status"];

/// One snapshot slot: ((address, system, did) for dedup, (the ECU, the did) to read).
type SnapJob = ((u32, String, i64), (EcuInfo, i64));

/// One system: an address can hold several (0x10: PGM-FI and EVPS), each with its own codes and data.
pub fn key(e: &EcuInfo) -> (u32, String) {
    (e.address, e.system.clone())
}

#[derive(Debug, Clone, PartialEq)]
pub enum Task {
    Dtc(EcuInfo),
    Watch(EcuInfo, i64),
    Snapshot(EcuInfo, i64),
    Probe(EcuInfo),
    Wake,
}

pub struct Monitor {
    live: Arc<LiveData>,
    pub ecus: Vec<EcuInfo>,
    last: HashMap<(u32, i64), f64>,           // (address, did) -> time read
    pub unsupported: HashSet<(u32, i64)>,     // the car answered NRC 0x31 (no such group): not asked again
    addresses: Vec<u32>,
    dtc_due: HashMap<(u32, String), f64>,
    pub snap_queue: Vec<(EcuInfo, i64)>,
    snap_due: f64,
    fails: u32,
    failed_at: Option<u32>,
    pub asleep: bool,
    wake_due: f64,
    /// seconds between trouble-code reads, and between wake-up probes (tests shorten them, as the Python tests do)
    pub dtc_every: f64,
    pub wake_every: f64,
}

impl Monitor {
    pub fn new(live: Arc<LiveData>) -> Monitor {
        let mut m = Monitor {
            live, ecus: vec![], last: HashMap::new(), unsupported: HashSet::new(), addresses: vec![], dtc_due: HashMap::new(),
            snap_queue: vec![], snap_due: 0.0, fails: 0, failed_at: None, asleep: false, wake_due: 0.0, dtc_every: DTC_EVERY, wake_every: WAKE_EVERY,
        };
        m.reset(&[], 0.0);
        m
    }

    pub fn reset(&mut self, ecus: &[EcuInfo], now: f64) {
        self.ecus = ecus.iter().filter(|e| !e.system.is_empty()).cloned().collect();
        self.last.clear();
        self.unsupported.clear();
        let mut a: Vec<u32> = self.ecus.iter().map(|e| e.address).collect();
        a.sort();
        a.dedup();
        self.addresses = a;
        self.dtc_due = self.ecus.iter().map(|e| (key(e), now)).collect();
        self.snap_queue = vec![];
        self.snap_due = now;
        self.fails = 0;
        self.failed_at = None;
        self.asleep = false;
        self.wake_due = 0.0;
    }

    /// Every read, by anyone: refreshes the schedule and tracks whether the car still answers.
    pub fn seen(&mut self, address: i64, did: i64, t: f64, ok: bool) {
        if ok {
            self.last.insert((address as u32, did), t);
            self.fails = 0;
        } else {
            if self.fails == 0 {
                self.failed_at = (address >= 0).then_some(address as u32);
            }
            self.fails += 1;
        }
    }

    pub fn car_gone(&self) -> bool {
        self.fails >= ABSENT_AFTER
    }

    fn watch_items(&self) -> Vec<(EcuInfo, i64, f64)> {
        let mut out = vec![];
        for e in &self.ecus {
            let served: HashSet<i64> = self.live.dids(&e.system, "DataList").into_iter().collect();
            for &(did, every) in watch(&e.system_type) {
                if served.contains(&did) && !self.unsupported.contains(&(e.address, did)) && !self.live.signals(&e.system, did).is_empty() {
                    out.push((e.clone(), did, every));
                }
            }
        }
        out
    }

    /// (due time, task) of the most urgent job.
    pub fn next_task(&mut self, now: f64) -> (Option<f64>, Option<Task>) {
        if self.asleep {
            return (Some(self.wake_due), Some(Task::Wake));
        }
        if self.fails != 0 && !self.ecus.is_empty() {
            return (Some(now), Some(Task::Probe(self.probe_target())));
        }
        let mut due: Vec<(f64, Task)> = self.ecus.iter().map(|e| (self.dtc_due[&key(e)], Task::Dtc(e.clone()))).collect();
        for (e, did, every) in self.watch_items() {
            let t = self.last.get(&(e.address, did)).map_or(0.0, |l| l + every); // never read: due at once, whatever the clock says
            due.push((t, Task::Watch(e, did)));
        }
        let mut best: (Option<f64>, Option<Task>) = (None, None);
        for (t, task) in due {
            if best.0.is_none_or(|b| t < b) {
                best = (Some(t), Some(task));
            }
        }
        if self.snap_queue.is_empty() && now >= self.snap_due {
            let mut queue: Vec<SnapJob> = vec![];
            for e in &self.ecus {
                for u in SNAPSHOT_USAGES {
                    for d in self.live.dids(&e.system, u) {
                        if self.live.signals(&e.system, d).is_empty() {
                            continue;
                        }
                        let k = (e.address, e.system.clone(), d);
                        match queue.iter_mut().find(|(x, _)| *x == k) {
                            Some(slot) => slot.1 = (e.clone(), d),
                            None => queue.push((k, (e.clone(), d))),
                        }
                    }
                }
            }
            self.snap_queue = queue.into_iter().map(|(_, v)| v).collect();
            self.snap_due = now + SNAPSHOT_EVERY;
        }
        if let Some((e, did)) = self.snap_queue.first() {
            if best.0.is_none_or(|b| b > now) {
                // idle time goes to the snapshot, one group per slot; anything due now still goes first
                return (Some(now), Some(Task::Snapshot(e.clone(), *did)));
            }
        }
        best
    }

    /// The address after the one that failed first, then the next: each check asks a different module.
    fn probe_target(&self) -> EcuInfo {
        let a = &self.addresses;
        let start = self.failed_at.and_then(|f| a.iter().position(|x| *x == f)).map_or(-1, |p| p as i64);
        let addr = a[((start + self.fails as i64).rem_euclid(a.len() as i64)) as usize];
        self.ecus.iter().find(|e| e.address == addr).cloned().unwrap()
    }

    pub fn done(&mut self, task: &Task, now: f64) {
        match task {
            Task::Dtc(e) => {
                self.dtc_due.insert(key(e), now + self.dtc_every);
            }
            Task::Snapshot(e, d) => {
                if let Some(p) = self.snap_queue.iter().position(|(x, y)| x == e && y == d) {
                    self.snap_queue.remove(p);
                }
            }
            Task::Wake => self.wake_due = now + self.wake_every,
            _ => {}
        }
    }

    pub fn sleep(&mut self, now: f64) {
        self.asleep = true;
        self.wake_due = now + self.wake_every;
        self.snap_queue = vec![];
    }
}
