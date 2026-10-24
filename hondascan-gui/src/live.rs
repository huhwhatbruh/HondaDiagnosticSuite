//! Live values of one system: polled back to back from the backend (one request in flight), kept outside the widgets
//! so a frame only reads them. Same behaviour as hondascan-app/src/lib/live.ts, including dropping a group the car
//! answered "out of range" (NRC 0x31).
use crate::api::{num, Ecu, LivePacket};
use crate::backend::{Backend, Reply};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

const HISTORY: usize = 120;

pub struct Live {
    pub ecu: Ecu,
    pub dids: Vec<u32>,
    pub vals: HashMap<(u32, usize), Value>,
    pub hist: HashMap<(u32, usize), VecDeque<f32>>,
    pub errors: HashMap<u32, String>,
    pub ms: u32,
    none: HashSet<u32>,
    rx: Option<(Receiver<Reply>, Instant)>,
    next: Instant,
}

impl Live {
    pub fn new(ecu: Ecu, dids: Vec<u32>) -> Self {
        Live { ecu, dids, vals: HashMap::new(), hist: HashMap::new(), errors: HashMap::new(), ms: 0, none: HashSet::new(), rx: None, next: Instant::now() }
    }

    pub fn value(&self, did: u32, i: usize) -> Option<&Value> { self.vals.get(&(did, i)) }
    pub fn number(&self, did: u32, i: usize) -> Option<f64> { self.value(did, i).and_then(num) }
    pub fn history(&self, did: u32, i: usize) -> Option<&VecDeque<f32>> { self.hist.get(&(did, i)) }

    /// Take a finished reply and ask again when `every` has passed. Call once per frame.
    pub fn pump(&mut self, be: &Backend, every: Duration) {
        if let Some((rx, t0)) = &self.rx {
            let t0 = *t0;
            match rx.try_recv() {
                Ok(r) => {
                    let ms = t0.elapsed().as_millis() as u32;
                    self.rx = None;
                    self.next = t0 + every;
                    match r {
                        Ok(v) => { self.ms = ms; self.take(crate::api::parse(v).unwrap_or_default()); }
                        Err(e) => self.fail(&e),
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => { self.rx = None; self.next = Instant::now() + every; }
                Err(_) => {}
            }
        }
        let ask: Vec<u32> = self.dids.iter().copied().filter(|d| !self.none.contains(d)).collect();
        if self.rx.is_none() && !ask.is_empty() && Instant::now() >= self.next {
            let p = json!({ "address": self.ecu.address, "system": self.ecu.system, "dids": ask });
            self.rx = Some((be.call("live", p), Instant::now()));
        }
    }

    /// A group that could not be read has no current value: the last one is dropped (it would show as live), the history stays.
    fn lose(&mut self, did: u32, why: String) {
        self.vals.retain(|(d, _), _| *d != did);
        self.errors.insert(did, why);
    }

    /// The whole request failed (backend stopped, adapter gone): every group asked for is lost, none is refused for good.
    fn fail(&mut self, why: &str) {
        for did in self.dids.clone() { self.lose(did, why.to_string()); }
    }

    fn take(&mut self, p: LivePacket) {
        for d in p.dids {
            if let Some(e) = d.error {
                if e.contains("NRC 0x31") { self.none.insert(d.did); }
                self.lose(d.did, e);
                continue;
            }
            self.errors.remove(&d.did);
            for (i, v) in d.values.unwrap_or_default().into_iter().enumerate() {
                if let Some(x) = num(&v) {
                    let h = self.hist.entry((d.did, i)).or_default();
                    h.push_back(x as f32);
                    if h.len() > HISTORY { h.pop_front(); }
                }
                self.vals.insert((d.did, i), v);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packet(v: Value) -> LivePacket { crate::api::parse(v).unwrap() }

    #[test]
    fn values_errors_and_refused_groups() {
        let mut l = Live::new(Ecu::default(), vec![0x2610, 0x2694]);
        l.take(packet(json!({"t": 1.0, "dids": [
            {"did": 0x2610, "values": [800, "ON", null]},
            {"did": 0x2694, "error": "service 0x22: NRC 0x31 requestOutOfRange"}]})));
        assert_eq!(l.number(0x2610, 0), Some(800.0));
        assert_eq!(l.value(0x2610, 1), Some(&json!("ON")));
        assert!(l.history(0x2610, 1).is_none(), "text has no history");
        assert!(l.errors.contains_key(&0x2694) && l.none.contains(&0x2694), "a refused group is not asked again");
        l.take(packet(json!({"dids": [{"did": 0x2610, "values": [900, "ON"]}]})));
        l.take(packet(json!({"dids": [{"did": 0x2610, "error": "no response"}]})));
        assert!(l.value(0x2610, 0).is_none() && l.history(0x2610, 0).is_some(), "a failed read leaves no stale value, only the history");
        l.take(packet(json!({"dids": [{"did": 0x2610, "values": [900]}]})));
        l.fail("backend stopped");
        assert!(l.value(0x2610, 0).is_none() && l.errors[&0x2610] == "backend stopped" && !l.none.contains(&0x2610));
        l.take(packet(json!({"dids": [{"did": 0x2610, "values": [900]}]})));
        for i in 0..(HISTORY + 5) { l.take(packet(json!({"dids": [{"did": 0x2610, "values": [i]}]}))); }
        assert_eq!(l.history(0x2610, 0).unwrap().len(), HISTORY);
        assert_eq!(l.history(0x2610, 0).unwrap().back(), Some(&((HISTORY + 4) as f32)));
    }
}
