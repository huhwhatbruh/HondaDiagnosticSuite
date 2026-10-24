//! Live data for Honda CAN systems: turn the bytes of the DIDs a system serves into named values.
//! Port of hondascan/livedata.py, which documents the part/mask layout and the converter equations (HDS Converter.dll's,
//! checked against the DLL by hds-xref/tools/convverify.py).
use crate::big::Big;
use crate::uds::{DidReader, UdsError};
use crate::db::Database;
use rusqlite::params;
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
pub struct Converter {
    pub id: i64,
    pub equation: i64,
    pub name: String,
    pub unit: String,
    pub coefficients: Vec<f64>,
    pub points: Vec<(f64, f64)>,
    pub states: Vec<(f64, f64, String)>, // (low, high, text)
}

impl Converter {
    pub fn convert(&self, xs: &[f64]) -> Option<f64> {
        let c = &self.coefficients;
        let x = xs[0];
        match self.equation {
            0 => Some(if c.len() == 2 { c[0] * x + c[1] } else { 0.0 }),
            1 => {
                if c.len() != 3 {
                    return Some(0.0);
                }
                let t = c[0] * x + c[1];
                (t != 0.0).then(|| c[2] / t)
            }
            2 => Some(if c.len() != 3 || xs.len() < 2 { 0.0 } else { c[0] * xs[0] + c[1] * xs[1] + c[2] }),
            -1 => {
                // CMapConverter::Convert: the segment ends at the first point beyond x, so at a repeated x (a step)
                // the later point counts; at the last point its y.
                let p = &self.points;
                if p.len() < 2 || x < p[0].0 || x > p[p.len() - 1].0 {
                    return Some(0.0);
                }
                let Some(i) = p.iter().position(|(px, _)| *px > x) else { return Some(p[p.len() - 1].1) };
                let ((x0, y0), (x1, y1)) = (p[i - 1], p[i]);
                Some(y0 + (y1 - y0) / (x1 - x0) * (x - x0))
            }
            _ => None,
        }
    }

    pub fn state_text(&self, value: f64) -> Option<&str> {
        self.states.iter().find(|(lo, hi, _)| *lo <= value && value <= *hi).map(|(_, _, t)| t.as_str())
    }
}

#[derive(Debug, Clone)]
pub struct Signal {
    pub system: String,
    pub did: i64,
    pub id: i64,
    pub symbol: String,
    pub description: String,
    pub unit: String,
    pub parts: Vec<(usize, u32)>, // (offset, mask) in order
    pub converters: Vec<Arc<Converter>>, // the one matching `unit` first
}

/// The raw number a signal's bytes form: one integer, or (equation 2) the separate inputs.
#[derive(Debug, Clone, PartialEq)]
pub enum Raw {
    One(Big),
    Many(Vec<Big>),
}

#[derive(Debug, Clone)]
pub struct Reading {
    pub symbol: String,
    pub unit: String, // the signal's output unit
    pub raw: Option<Raw>,
    pub value: Option<f64>, // None when it cannot be computed
    pub text: Option<String>, // state text for state signals
    pub error: String,
}

impl Signal {
    /// The unit of the decoded value: the converter's, else the signal's.
    pub fn out_unit(&self) -> &str {
        match self.converters.first() {
            Some(c) if !c.unit.is_empty() => &c.unit,
            _ => &self.unit,
        }
    }

    pub fn raw_parts(&self, data: &[u8]) -> Option<Vec<u8>> {
        self.parts
            .iter()
            .map(|&(off, mask)| {
                let shift = if mask != 0 { mask.trailing_zeros() } else { 0 };
                data.get(off).map(|b| ((*b as u32 & mask) >> shift) as u8)
            })
            .collect()
    }

    pub fn raw(&self, parts: &[u8]) -> Big {
        let mut v = Big::default();
        for ((_, mask), p) in self.parts.iter().zip(parts) {
            v.shl_or(mask.count_ones(), *p as u64);
        }
        v
    }

    pub fn decode(&self, data: &[u8]) -> Reading {
        let mk = |raw, value, text, error: &str| Reading { symbol: self.symbol.clone(), unit: self.out_unit().into(), raw, value, text, error: error.into() };
        let Some(parts) = self.raw_parts(data) else { return mk(None, None, None, "DID data too short") };
        let Some(conv) = self.converters.first() else {
            let raw = self.raw(&parts);
            let f = raw.to_f64();
            return mk(Some(Raw::One(raw)), Some(f), None, "");
        };
        let xs: Vec<Big> = if conv.equation == 2 { parts.iter().map(|p| Big::from_u64(*p as u64)).collect() } else { vec![self.raw(&parts)] };
        let fx: Vec<f64> = xs.iter().map(Big::to_f64).collect();
        let raw = if xs.len() == 1 { Raw::One(xs[0].clone()) } else { Raw::Many(xs) };
        if !conv.states.is_empty() {
            return mk(Some(raw), Some(fx[0]), conv.state_text(fx[0]).map(String::from), "");
        }
        mk(Some(raw), conv.convert(&fx), None, "")
    }
}

impl Reading {
    /// '' when the reading is a state text (it has no unit), else the signal's output unit.
    pub fn unit(&self) -> &str {
        if self.text.is_some() { "" } else { &self.unit }
    }

    pub fn display(&self) -> String {
        if !self.error.is_empty() {
            return self.error.clone();
        }
        if let Some(t) = &self.text {
            return t.clone();
        }
        let Some(v) = self.value else { return "---".into() };
        let s = if v.fract() == 0.0 { format!("{v:.0}") } else { format!("{v:.3}").trim_end_matches('0').trim_end_matches('.').to_string() };
        let u = self.unit();
        if !u.is_empty() && u != "<no units>" && u != "-1" { format!("{s} {u}") } else { s }
    }
}

/// Converter row -> the parsed converter, or None where it failed to parse; built once, not per read.
type ConverterCache = Mutex<HashMap<i64, Option<Arc<Converter>>>>;

/// One read batch: [(did, its error, or None, [decoded value])]; rejected DIDs are reported, not raised.
pub type DidReads = Vec<(i64, Option<String>, Vec<Reading>)>;

/// (system, did) -> that group's signals; built once, not per read.
type SignalMap = Mutex<HashMap<(String, i64), Arc<Vec<Signal>>>>;
/// (system, usage) -> the DIDs: the database does not change while we run.
type DidMap = Mutex<HashMap<(String, String), Arc<Vec<i64>>>>;

pub struct LiveData {
    db: Arc<Database>,
    conv: ConverterCache,
    signals: SignalMap,
    dids: DidMap,
    warmed: Mutex<HashSet<String>>,                          // systems whose converters `warm` has loaded
}

impl LiveData {
    pub fn new(db: Arc<Database>) -> LiveData {
        LiveData { db, conv: Mutex::new(HashMap::new()), signals: Mutex::new(HashMap::new()), dids: Mutex::new(HashMap::new()), warmed: Mutex::new(HashSet::new()) }
    }

    pub fn converter(&self, cid: i64) -> Option<Arc<Converter>> {
        if let Some(c) = self.conv.lock().unwrap().get(&cid) {
            return c.clone();
        }
        let head = self.db.query_one(
            "SELECT c.equation, c.name, COALESCE(u.name, '') FROM converters c LEFT JOIN units u ON u.id = c.out_unit WHERE c.id=?",
            [cid],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)),
        );
        let conv = head.map(|(equation, name, unit)| {
            Arc::new(Converter {
                id: cid,
                equation,
                name,
                unit,
                coefficients: self.db.query("SELECT value FROM converter_coefficients WHERE converter=? ORDER BY idx", [cid], |r| r.get(0)),
                points: self.db.query("SELECT x, y FROM converter_map WHERE converter=? ORDER BY idx", [cid], |r| Ok((r.get(0)?, r.get(1)?))),
                states: self.db.query("SELECT low, high, text FROM converter_states WHERE converter=? ORDER BY idx", [cid], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))),
            })
        });
        self.conv.lock().unwrap().insert(cid, conv.clone());
        conv
    }

    /// Load the converters a system's signals use, four queries in all rather than four per converter (the engine uses 485:
    /// 1,940 queries, about 30 ms, each with a file lock). Rows that do not read are dropped, as in `converter`.
    fn warm(&self, system: &str) {
        if !self.warmed.lock().unwrap().insert(system.to_string()) {
            return;
        }
        let lists: Vec<String> = self.db.query("SELECT DISTINCT converters FROM can_signal_parts WHERE system=?", [system], |r| r.get(0));
        let mut ids: Vec<i64> = lists.iter().flat_map(|l| l.split_whitespace().filter_map(|x| x.parse().ok())).collect();
        ids.sort_unstable();
        ids.dedup();
        ids.retain(|i| !self.conv.lock().unwrap().contains_key(i));
        for chunk in ids.chunks(400) {
            let marks = vec!["?"; chunk.len()].join(",");
            let ask = |sql: &str| format!("{sql} IN ({marks})");
            let heads: Vec<(i64, i64, String, String)> = self.db.query(
                &ask("SELECT c.id, c.equation, c.name, COALESCE(u.name, '') FROM converters c LEFT JOIN units u ON u.id = c.out_unit WHERE c.id"),
                rusqlite::params_from_iter(chunk), |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)));
            let mut coefficients: HashMap<i64, Vec<f64>> = HashMap::new();
            for (c, v) in self.db.query(&format!("{} ORDER BY converter, idx", ask("SELECT converter, value FROM converter_coefficients WHERE converter")),
                rusqlite::params_from_iter(chunk), |r| Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?))) { coefficients.entry(c).or_default().push(v); }
            let mut points: HashMap<i64, Vec<(f64, f64)>> = HashMap::new();
            for (c, x, y) in self.db.query(&format!("{} ORDER BY converter, idx", ask("SELECT converter, x, y FROM converter_map WHERE converter")),
                rusqlite::params_from_iter(chunk), |r| Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?, r.get::<_, f64>(2)?))) { points.entry(c).or_default().push((x, y)); }
            let mut states: HashMap<i64, Vec<(f64, f64, String)>> = HashMap::new();
            for (c, lo, hi, t) in self.db.query(&format!("{} ORDER BY converter, idx", ask("SELECT converter, low, high, text FROM converter_states WHERE converter")),
                rusqlite::params_from_iter(chunk), |r| Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?, r.get::<_, f64>(2)?, r.get::<_, String>(3)?))) { states.entry(c).or_default().push((lo, hi, t)); }
            let mut cache = self.conv.lock().unwrap();
            let mut made: HashMap<i64, Arc<Converter>> = heads.into_iter().map(|(id, equation, name, unit)| (id, Arc::new(Converter {
                id, equation, name, unit,
                coefficients: coefficients.remove(&id).unwrap_or_default(), points: points.remove(&id).unwrap_or_default(), states: states.remove(&id).unwrap_or_default(),
            }))).collect();
            for id in chunk { cache.entry(*id).or_insert_with(|| made.remove(id)); }
        }
    }

    /// DIDs HDS lists for the system's data list (all packet DIDs if it has no usage rows).
    pub fn dids(&self, system: &str, usage: &str) -> Vec<i64> {
        let key = (system.to_string(), usage.to_string());
        if let Some(d) = self.dids.lock().unwrap().get(&key) {
            return d.to_vec();
        }
        let d = self.query_dids(system, usage);
        self.dids.lock().unwrap().insert(key, Arc::new(d.clone()));
        d
    }

    /// The uncached lookup. Measured 2026-10-21 on the simulated car: asking the database on every call was about 370 queries a
    /// second from the background monitor alone (`dids` is called by every live read), each taking a file lock and stat-ing the
    /// journal files.
    fn query_dids(&self, system: &str, usage: &str) -> Vec<i64> {
        let d = self.db.query("SELECT did FROM can_packet_usage WHERE system=? AND usage=? ORDER BY rowid", params![system, usage], |r| r.get(0));
        if !d.is_empty() {
            return d;
        }
        let d = self.db.query("SELECT did FROM can_packets WHERE system=? ORDER BY rowid", [system], |r| r.get(0));
        if !d.is_empty() {
            return d;
        }
        self.db.query("SELECT DISTINCT did FROM can_signal_parts WHERE system=? ORDER BY did", [system], |r| r.get(0)) // K-line
    }

    /// Read and decode. Returns [(did, error or None, [Reading])]; DIDs the ECU rejects are reported, not raised. Without an
    /// explicit list, only DIDs that carry a signal are read: the bus round-trip is the cost. An OS error from the bus is raised.
    pub fn read(&self, client: &mut dyn DidReader, system: &str, dids: Option<&[i64]>) -> Result<DidReads, std::io::Error> {
        let list: Vec<i64> = match dids {
            Some(d) if !d.is_empty() => d.to_vec(),
            _ => self.dids(system, "DataList").into_iter().filter(|d| !self.signals(system, *d).is_empty()).collect(),
        };
        let mut out = vec![];
        for did in list {
            match client.read_did(did as u16) {
                Ok(data) => out.push((did, None, self.signals(system, did).iter().map(|s| s.decode(&data)).collect())),
                Err(UdsError::Io(e)) => return Err(e),
                Err(e) => out.push((did, Some(e.to_string()), vec![])),
            }
        }
        Ok(out)
    }

    pub fn signals(&self, system: &str, did: i64) -> Arc<Vec<Signal>> {
        let key = (system.to_string(), did);
        if let Some(s) = self.signals.lock().unwrap().get(&key) {
            return s.clone();
        }
        self.warm(system);
        let rows: Vec<(i64, String, String, String, i64, i64, String)> = self.db.query(
            "SELECT signal, symbol, description, unit, offset, mask, converters FROM can_signal_parts WHERE system=? AND did=? ORDER BY signal, part",
            params![system, did],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)),
        );
        let mut out: Vec<Signal> = vec![];
        let mut at: HashMap<i64, usize> = HashMap::new();   // signal id -> its place in `out`: a signal's parts are rows of its own
        for (sg, sym, desc, unit, off, mask, convs) in rows {
            if let Entry::Vacant(e) = at.entry(sg) {
                e.insert(out.len());
                let mut cs: Vec<Arc<Converter>> = convs.split_whitespace().filter_map(|x| x.parse().ok()).filter_map(|c| self.converter(c)).collect();
                cs.sort_by_key(|c| c.unit != unit); // stable: the converter whose unit matches first
                out.push(Signal { system: system.into(), did, id: sg, symbol: sym, description: desc, unit, parts: vec![], converters: cs });
            }
            out[at[&sg]].parts.push((off as usize, mask as u32));
        }
        let out = Arc::new(out);
        self.signals.lock().unwrap().insert(key, out.clone());
        out
    }
}
