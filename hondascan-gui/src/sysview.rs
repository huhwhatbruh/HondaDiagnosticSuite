//! One system's live data as the System screen shows it: pinned gauges on top, then every signal grouped by data identifier,
//! searchable. Only open groups and pinned signals are read, so narrowing the view speeds up the update rate; one group starts
//! open (see `default_open`), not all of them: over an ELM adapter every extra group costs a bus exchange per update.
//! This is the state and decisions; QML only draws the JSON `to_json` makes.
use crate::api::{Catalog, Ecu, SignalDef};
use crate::format::{convert, display, state_off, state_on};
use crate::live::Live;
use crate::names::{explain, friendly};
use crate::ranges::{scale_for, tone_of, Tone};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::Arc;

pub type Key = (u32, usize);

/// What a figure the car cannot give while it stands still shows instead of a stale number.
pub const NOT_WHILE_STOPPED: &str = "not valid while stopped";

pub struct SysView {
    pub ecu: Ecu,
    pub cat: Arc<Catalog>,
    pub live: Live,
    query: String,
    opened: Option<HashSet<u32>>, // None: `default_open`
    running: bool,
    pins: Option<Vec<Key>>,       // None: the system's defaults
}

/// One live figure as QML draws it: the value and text in display units (US), its gauge scale and its tone (0 normal, 1 warn, 2 fault).
pub fn figure(sym: &str, unit: &str, raw: Option<&Value>) -> Value {
    let (text, shown) = display(raw, unit);
    let num = raw.and_then(crate::api::num);
    let scale = scale_for(sym, unit);
    let tone = match tone_of(scale.and_then(|s| s.2), num) { Tone::Data => 0, Tone::Warn => 1, Tone::Fault => 2 };
    json!({ "v": num.map(|x| convert(x, unit).0), "text": text, "unit": shown, "min": scale.map(|s| s.0), "max": scale.map(|s| s.1), "tone": tone })
}

/// What a sparkline can show: at most `SPARK_POINTS` readings (every n-th, the newest kept), to two decimals; its scale is its own, so
/// the unit does not matter.
const SPARK_POINTS: usize = 40;

fn thin(h: Option<&std::collections::VecDeque<f32>>) -> Vec<f64> {
    let Some(h) = h else { return Vec::new() };
    let step = h.len().div_ceil(SPARK_POINTS).max(1);
    let mut v: Vec<f64> = h.iter().rev().step_by(step).map(|x| (f64::from(*x) * 100.0).round() / 100.0).collect();
    v.reverse();
    v
}

/// Whether the car is standing: the vehicle speed signal reads 0.
pub fn is_stopped(cat: &Catalog, live: &Live) -> bool { cat.find("VSP").is_some_and(|(d, i, _)| live.number(d, i) == Some(0.0)) }

/// One signal as QML draws it (a row, a tile or a gauge): its figure, names, state, note, history.
pub fn signal_json(cat: &Catalog, live: &Live, did: u32, i: usize, stopped: bool, detail: bool) -> Value {
    let def = &cat.dids.iter().find(|d| d.did == did).map(|d| &d.signals[i]).expect("a signal of the catalog");
    let named = friendly(def);
    let raw = live.value(did, i);
    let hidden = stopped && def.invalid_stopped;
    let mut f = figure(&def.symbol, &def.unit, if hidden { None } else { raw });
    let state = raw.and_then(Value::as_str).map_or("", |s| if state_on(s) { "on" } else if state_off(s) { "off" } else { "" });
    let note = if hidden { Some(NOT_WHILE_STOPPED.to_string()) } else { explain(&def.symbol, raw.and_then(crate::api::num)) };
    let hist = thin(live.history(did, i));
    let o = f.as_object_mut().expect("an object");
    o.insert("did".into(), json!(did));
    o.insert("i".into(), json!(i));
    o.insert("symbol".into(), json!(def.symbol));
    o.insert("name".into(), json!(named.name));
    if detail {
        o.insert("about".into(), json!(named.about));
        o.insert("raw".into(), json!(named.raw));
    }
    o.insert("is_text".into(), json!(raw.is_some_and(Value::is_string)));
    o.insert("state".into(), json!(state));
    o.insert("note".into(), json!(note));
    o.insert("hidden".into(), json!(hidden));
    o.insert("hist".into(), json!(hist));
    f
}

impl SysView {
    pub fn new(ecu: Ecu, cat: Arc<Catalog>, pins: Option<Vec<Key>>) -> Self {
        let live = Live::new(Vec::new());
        let mut v = SysView { ecu, cat, live, query: String::new(), opened: None, running: true, pins };
        v.sync_polled();
        v
    }

    fn default_pins(&self) -> Vec<Key> {
        let syms: &[&str] = if self.ecu.system_type == "CVT" { &["NDR", "NDN", "TATF", "RATIO"] } else { &["NE", "VSP", "ECT", "VB"] };
        syms.iter().filter_map(|s| self.cat.find(s).map(|f| (f.0, f.1))).collect()
    }

    pub fn pins(&self) -> Vec<Key> { self.pins.clone().unwrap_or_else(|| self.default_pins()) }

    fn needle(&self) -> String { self.query.trim().to_lowercase() }
    fn searching(&self) -> bool { self.needle().chars().count() >= 2 }

    fn matches(&self, did: u32, s: &SignalDef) -> bool {
        let q = self.needle();
        q.is_empty() || s.symbol.to_lowercase().contains(&q) || s.description.to_lowercase().contains(&q)
            || friendly(s).name.to_lowercase().contains(&q) || format!("{did:04x}").contains(&q)
    }

    /// The groups with a signal that matches the search, each with its matching signals' positions.
    fn groups(&self) -> Vec<(u32, Vec<usize>)> {
        self.cat.dids.iter().filter_map(|d| {
            let v: Vec<usize> = d.signals.iter().enumerate().filter(|(_, s)| self.matches(d.did, s)).map(|(i, _)| i).collect();
            (!v.is_empty()).then_some((d.did, v))
        }).collect()
    }

    /// What is open until someone opens or closes a group: while searching, every group with a match (so the results are
    /// visible); otherwise the group of the system's first default gauge, which is polled for the gauge anyway (the first
    /// group in the list is usually the ECU's identification bytes).
    fn open_set(&self, groups: &[(u32, Vec<usize>)]) -> HashSet<u32> {
        if let Some(o) = &self.opened { return o.clone(); }
        if self.searching() { return groups.iter().map(|g| g.0).collect(); }
        self.default_pins().first().map(|k| k.0)
            .or_else(|| self.cat.dids.iter().find(|d| d.signals.len() > 1).map(|d| d.did)).into_iter().collect()
    }

    /// The groups to read now: the pinned signals' and the open groups' (none while paused).
    pub fn polled(&self) -> Vec<u32> {
        if !self.running { return Vec::new(); }
        let groups = self.groups();
        let open = self.open_set(&groups);
        let mut p: Vec<u32> = self.pins().iter().map(|k| k.0).chain(groups.iter().filter(|(d, _)| open.contains(d)).map(|g| g.0)).collect();
        p.sort_unstable();
        p.dedup();
        p
    }

    fn sync_polled(&mut self) { self.live.dids = self.polled(); }

    pub fn set_query(&mut self, q: &str) { self.query = q.to_string(); self.sync_polled(); }
    pub fn set_running(&mut self, on: bool) { self.running = on; self.sync_polled(); }

    /// Open a closed group or close an open one.
    pub fn toggle_group(&mut self, did: u32) {
        let groups = self.groups();
        let mut open = self.open_set(&groups);
        if !open.remove(&did) { open.insert(did); }
        self.opened = Some(open);
        self.sync_polled();
    }

    /// "Open all" opens every group shown; "Close all" closes them.
    pub fn open_all(&mut self, on: bool) {
        self.opened = Some(if on { self.groups().iter().map(|g| g.0).collect() } else { HashSet::new() });
        self.sync_polled();
    }

    /// Pin a signal as a gauge, or unpin it; returns the pins to remember.
    pub fn toggle_pin(&mut self, k: Key) -> Vec<Key> {
        let mut pins = self.pins();
        match pins.iter().position(|p| *p == k) { Some(i) => { pins.remove(i); } None => pins.push(k) }
        self.pins = Some(pins.clone());
        self.sync_polled();
        pins
    }

    fn stopped(&self) -> bool { is_stopped(&self.cat, &self.live) }

    fn signal(&self, did: u32, i: usize, stopped: bool) -> Value { signal_json(&self.cat, &self.live, did, i, stopped, false) }

    /// Everything the screen shows in one piece (the tests read it; the app publishes `layout_json` and `live_json` apart).
    #[cfg(test)]
    pub fn to_json(&self) -> Value {
        let stopped = self.stopped();
        self.tree(&|d, i| self.signal(d, i, stopped), true)
    }

    /// The screen's structure without a reading in it: it changes only when a group opens or closes, a pin comes or goes, the
    /// search or the pause changes, so QML keeps its rows between updates. Each signal is `{did, i, name}`; `live_json` has the rest.
    pub fn layout_json(&self) -> Value {
        self.tree(&|did, i| {
            let def = &self.cat.dids.iter().find(|d| d.did == did).expect("a group of the catalog").signals[i];
            json!({ "did": did, "i": i, "name": friendly(def).name })
        }, false)
    }

    /// What changes with every update: the rate, each group's read error and, by `"did:i"`, every shown signal as `signal_json` has it.
    pub fn live_json(&self) -> Value {
        let stopped = self.stopped();
        let groups = self.groups();
        let open = self.open_set(&groups);
        let mut sig = serde_json::Map::new();
        let shown = groups.iter().filter(|(d, _)| open.contains(d)).flat_map(|(d, idx)| idx.iter().map(move |i| (*d, *i)));
        for (d, i) in self.pins().into_iter().filter(|k| self.has(*k)).chain(shown) {
            sig.insert(format!("{d}:{i}"), self.signal(d, i, stopped));
        }
        let err: serde_json::Map<String, Value> = self.live.errors.iter().map(|(d, e)| (d.to_string(), json!(e))).collect();
        json!({ "ms": self.live.ms, "err": err, "sig": sig })
    }

    fn has(&self, (did, i): Key) -> bool { self.cat.dids.iter().any(|g| g.did == did && i < g.signals.len()) }

    /// The tree both forms share; `sig` makes one signal's entry, `dynamic` puts in what moves with every update (the rate, read errors).
    fn tree(&self, sig: &dyn Fn(u32, usize) -> Value, dynamic: bool) -> Value {
        let groups = self.groups();
        let open = self.open_set(&groups);
        let pins: Vec<Value> = self.pins().into_iter().filter(|k| self.has(*k)).map(|(d, i)| sig(d, i)).collect();
        let shown: Vec<Value> = groups.iter().map(|(did, idx)| {
            let is_open = open.contains(did);
            let mut g = json!({ "did": did, "hex": format!("0x{did:04X}"), "count": idx.len(), "open": is_open,
                    "signals": if is_open { idx.iter().map(|i| sig(*did, *i)).collect::<Vec<_>>() } else { Vec::new() } });
            if dynamic { g["error"] = json!(self.live.errors.get(did)); }
            g
        }).collect();
        let polled = self.polled();
        let mut v = json!({ "key": self.ecu.key(), "query": self.query, "running": self.running, "polled": polled.len(),
                "total": self.cat.dids.iter().map(|d| d.signals.len()).sum::<usize>(), "any_open": groups.iter().any(|g| open.contains(&g.0)),
                "pins": pins, "groups": shown });
        if dynamic { v["ms"] = json!(self.live.ms); }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::DidDef;

    fn sig(s: &str, unit: &str) -> SignalDef { SignalDef { symbol: s.into(), description: s.into(), unit: unit.into(), ..Default::default() } }
    fn view() -> SysView {
        let cat = Catalog { dids: vec![
            DidDef { did: 0x2600, signals: vec![sig("ECUID1", ""), sig("ECUID2", "")] },
            DidDef { did: 0x2610, signals: vec![sig("ECT", "degC"), sig("NE", "RPM"), sig("IAT", "degC")] },
            DidDef { did: 0x2620, signals: vec![sig("VB", "V"), sig("MAP", "kPa")] },
        ], ..Default::default() };
        SysView::new(Ecu { address: 0x0E, system: "CANFI".into(), system_type: "PGM-FI".into(), ..Default::default() }, Arc::new(cat), None)
    }

    #[test]
    fn it_starts_with_the_default_gauges_and_only_their_group_open() {
        let v = view();
        assert_eq!(v.pins(), vec![(0x2610, 1), (0x2610, 0), (0x2620, 0)], "NE, ECT, VB (this catalog has no VSP)");
        assert_eq!(v.polled(), vec![0x2610, 0x2620], "the pins' groups; the id bytes group is not read");
        assert_eq!(v.live.dids, v.polled());
    }

    #[test]
    fn opening_a_group_reads_it_and_closing_stops_reading_it() {
        let mut v = view();
        v.toggle_group(0x2600);
        assert!(v.polled().contains(&0x2600));
        v.toggle_group(0x2600);
        assert!(!v.polled().contains(&0x2600));
        v.open_all(false);
        assert_eq!(v.polled(), vec![0x2610, 0x2620], "pins are always read");
        v.open_all(true);
        assert_eq!(v.polled().len(), 3);
    }

    #[test]
    fn a_search_of_two_letters_or_more_opens_the_groups_with_a_match() {
        let mut v = view();
        v.set_query("map");
        let j = v.to_json();
        let groups = j["groups"].as_array().unwrap();
        assert_eq!(groups.len(), 1, "only the group with a match is listed");
        assert_eq!((groups[0]["did"].as_u64(), groups[0]["open"].as_bool()), (Some(0x2620), Some(true)));
        assert_eq!(groups[0]["signals"].as_array().unwrap().len(), 1, "and only the matching signal");
        // one letter still narrows the list, but opens nothing by itself: the default group stays the only one open
        v.set_query("m");
        let j = v.to_json();
        let open: Vec<(u64, bool)> = j["groups"].as_array().unwrap().iter().map(|g| (g["did"].as_u64().unwrap(), g["open"].as_bool().unwrap())).collect();
        assert_eq!(open, vec![(0x2610, true), (0x2620, false)]);
    }

    #[test]
    fn pause_reads_nothing_and_pins_come_and_go() {
        let mut v = view();
        v.set_running(false);
        assert!(v.polled().is_empty() && v.live.dids.is_empty());
        v.set_running(true);
        let pins = v.toggle_pin((0x2610, 2));
        assert!(pins.contains(&(0x2610, 2)) && v.to_json()["pins"].as_array().unwrap().len() == 4);
        v.toggle_pin((0x2610, 2));
        assert_eq!(v.pins().len(), 3);
    }

    #[test]
    fn a_figure_not_valid_while_stopped_shows_a_note_not_a_number() {
        let cat = Catalog { dids: vec![DidDef { did: 1, signals: vec![
            sig("VSP", "km/h"), SignalDef { invalid_stopped: true, ..sig("ABC", "V") }] }], ..Default::default() };
        let mut v = SysView::new(Ecu::default(), Arc::new(cat), Some(vec![]));
        v.live.answered_for_test(json!({"dids": [{"did": 1, "values": [0, 5]}]}));
        let j = v.to_json();
        let s = &j["groups"][0]["signals"][1];
        assert_eq!((s["hidden"].as_bool(), s["text"].as_str(), s["note"].as_str()), (Some(true), Some("—"), Some(NOT_WHILE_STOPPED)));
    }

    #[test]
    fn a_figure_is_in_us_units_with_its_scale_and_tone() {
        let hot = figure("ECT", "degC", Some(&json!(118.0)));
        assert_eq!((hot["text"].as_str(), hot["unit"].as_str(), hot["tone"].as_i64()), (Some("244"), Some("°F"), Some(2)), "118 C is overheating");
        assert_eq!(figure("ECT", "degC", Some(&json!(90.0)))["tone"], 0);
        let none = figure("NE", "RPM", None);
        assert!(none["v"].is_null() && none["text"] == "—", "no reading is a dash, not zero");
    }


    #[test]
    fn every_group_open_with_full_history_stays_a_few_hundred_kilobytes() {
        let dids: Vec<DidDef> = (0..20).map(|g| DidDef { did: 0x2600 + g, signals: (0..38).map(|i| sig(&format!("SIG{g}_{i}"), "degC")).collect() }).collect();
        let cat = Catalog { dids, ..Default::default() };
        let mut v = SysView::new(Ecu::default(), Arc::new(cat), None);
        v.open_all(true);
        for k in 0..130 {
            let vals: Vec<Value> = (0..20).map(|g| json!({"did": 0x2600 + g, "values": (0..38).map(|i| 20.0 + f64::from(k) * 0.37 + f64::from(i) * 1.1).collect::<Vec<_>>()})).collect();
            v.live.answered_for_test(json!({"dids": vals}));
        }
        let j = v.to_json();
        let n = j.to_string().len();
        // 1.78 MB per update (five a second) before the rows dropped their explanations and the history was thinned to what a sparkline shows
        assert!(n < 450_000, "759 signals open with 120 readings each came to {n} bytes");
        let row = &j["groups"][0]["signals"][0];
        assert!(row.get("about").is_none() && row.get("raw").is_none(), "a row shows neither, so it does not carry them");
        let hist = row["hist"].as_array().unwrap();
        assert!(hist.len() <= SPARK_POINTS && hist.len() > SPARK_POINTS / 2, "{} points", hist.len());
        assert_eq!(hist.last().unwrap().as_f64(), Some(67.73), "the newest reading is kept, to two decimals");
    }

    #[test]
    fn a_short_history_is_kept_whole_and_a_missing_one_is_empty() {
        let h: std::collections::VecDeque<f32> = [1.0_f32, 2.5, 3.0].into_iter().collect();
        assert_eq!(thin(Some(&h)), vec![1.0, 2.5, 3.0]);
        assert!(thin(None).is_empty());
    }
}
