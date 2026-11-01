//! The key numbers of a system type, as gauges and tiles, in a quiet "reference" card on the Overview.
//! Each one owns a live poll of just the groups it shows. The Engine one also feeds the hero at the top (its
//! headline numbers are polled with it, so they share one poll).
use crate::api::{Catalog, Ecu};
use crate::live::Live;
use crate::names::friendly;
use crate::sysview::{figure, is_stopped, signal_json};
use serde_json::{json, Value};
use std::sync::Arc;

pub struct Def { pub types: &'static [&'static str], pub title: &'static str, pub from: &'static str, pub hero: &'static [&'static str],
    pub gauges: &'static [&'static str], pub tiles: &'static [&'static str] }

// Engine: the headline numbers (hero) are polled with it, so they share one poll. Transmission: CVT fluid temperature,
// drive and driven pulley speeds; driven pulley pressure, start-clutch slip, pulley ratio, shift map (in words) and road
// slope (CANCVT 0x2301 / 0x2322, both read by the monitor anyway).
pub const DEFS: [Def; 2] = [
    Def { types: &["PGM-FI", "DIESEL"], title: "Engine", from: "engine computer", hero: &["NE", "ECT", "IG", "KNRTD", "KNRTD1", "KNRTD2", "KNRTD3", "KNRTD4"],
        gauges: &["VSP", "VB"], tiles: &["ECT", "IAT", "MAP", "APP", "FLEVELF"] },
    Def { types: &["CVT"], title: "Transmission", from: "transmission computer", hero: &[],
        gauges: &["TATF", "NDR", "NDN", "VSP"], tiles: &["PDN", "RATIO", "SFTMAPST", "ESCP", "SLOPE"] },
];

/// The system a glance shows: the first found of its types that has a database system name.
pub fn ecu_for(def: &Def, ecus: &[Ecu]) -> Option<Ecu> {
    ecus.iter().find(|e| def.types.contains(&e.system_type.as_str()) && !e.system.is_empty()).cloned()
}

pub struct Glance { pub def: &'static Def, pub ecu: Ecu, pub cat: Arc<Catalog>, pub live: Live }

impl Glance {
    /// Polls only the groups of the signals this glance shows (the ones the catalog has).
    pub fn new(def: &'static Def, ecu: Ecu, cat: Arc<Catalog>) -> Self {
        let mut dids: Vec<u32> = def.hero.iter().chain(def.gauges).chain(def.tiles).filter_map(|s| cat.find(s).map(|f| f.0)).collect();
        dids.sort_unstable();
        dids.dedup();
        let live = Live::new(dids);
        Glance { def, ecu, cat, live }
    }

    /// Every symbol of the hero, gauges and tiles with its figure (the hero reads its numbers from here).
    pub fn figures(&self) -> Value {
        let mut m = serde_json::Map::new();
        for s in self.def.hero.iter().chain(self.def.gauges).chain(self.def.tiles) {
            if let Some((did, i, def)) = self.cat.find(s) { m.insert(s.to_string(), figure(s, &def.unit, self.live.value(did, i))); }
        }
        Value::Object(m)
    }

    /// The card's structure without a reading in it (its title and, per gauge and tile of this car, `{did, i, symbol, name, about,
    /// raw}`): the same string between updates, so QML keeps its gauges and tiles; `live_json` has what moves.
    pub fn layout_json(&self) -> Value {
        let draw = |syms: &[&str]| -> Vec<Value> {
            syms.iter().filter_map(|s| self.cat.find(s).map(|(did, i, def)| {
                let n = friendly(def);
                json!({ "did": did, "i": i, "symbol": def.symbol, "name": n.name, "about": n.about, "raw": n.raw })
            })).collect()
        };
        json!({ "title": self.def.title, "gauges": draw(self.def.gauges), "tiles": draw(self.def.tiles) })
    }

    /// What changes with every update: the note (it carries the rate) and, by `"did:i"`, each figure as `signal_json` has it.
    pub fn live_json(&self) -> Value {
        let stopped = is_stopped(&self.cat, &self.live);
        let mut sig = serde_json::Map::new();
        for s in self.def.gauges.iter().chain(self.def.tiles) {
            if let Some((did, i, _)) = self.cat.find(s) { sig.insert(format!("{did}:{i}"), signal_json(&self.cat, &self.live, did, i, stopped, false)); }
        }
        json!({ "note": format!("Live from the {}{}", self.def.from, if self.live.ms > 0 { format!(" · {} ms per update", self.live.ms) } else { String::new() }), "sig": sig })
    }

    /// The card in one piece (the tests read it; the app publishes `layout_json` and `live_json` apart).
    #[cfg(test)]
    pub fn card_json(&self) -> Value {
        let stopped = is_stopped(&self.cat, &self.live);
        let draw = |syms: &[&str]| -> Vec<Value> {
            syms.iter().filter_map(|s| self.cat.find(s).map(|(did, i, _)| signal_json(&self.cat, &self.live, did, i, stopped, true))).collect()
        };
        json!({
            "title": self.def.title,
            "note": format!("Live from the {}{}", self.def.from, if self.live.ms > 0 { format!(" · {} ms per update", self.live.ms) } else { String::new() }),
            "gauges": draw(self.def.gauges), "tiles": draw(self.def.tiles),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{DidDef, SignalDef};

    fn sig(s: &str, unit: &str) -> SignalDef { SignalDef { symbol: s.into(), description: s.into(), unit: unit.into(), ..Default::default() } }
    fn cat() -> Arc<Catalog> {
        Arc::new(Catalog { dids: vec![
            DidDef { did: 0x2610, signals: vec![sig("ABSTH", "%"), sig("ECT", "degC"), sig("NE", "RPM"), sig("VSP", "km/h")] },
            DidDef { did: 0x2622, signals: vec![sig("KNRTD", "deg")] },
            DidDef { did: 0x2700, signals: vec![sig("VB", "V"), sig("MAP", "kPa")] },
        ], ..Default::default() })
    }
    fn pcm() -> Ecu { Ecu { address: 0x10, system: "CANFI".into(), system_type: "PGM-FI".into(), ..Default::default() } }

    #[test]
    fn a_glance_polls_only_the_groups_of_the_signals_it_shows_that_the_car_has() {
        let g = Glance::new(&DEFS[0], pcm(), cat());
        assert_eq!(g.live.dids, vec![0x2610, 0x2622, 0x2700], "NE, ECT, VSP / KNRTD / VB, MAP; IG, IAT, APP and the rest are not in this catalog");
        let cvt = Glance::new(&DEFS[1], pcm(), cat());
        assert_eq!(cvt.live.dids, vec![0x2610], "of the transmission's signals only VSP is in this catalog");
    }

    #[test]
    fn the_card_lists_the_gauges_and_tiles_the_car_has_and_the_figures_cover_the_hero_too() {
        let mut g = Glance::new(&DEFS[0], pcm(), cat());
        g.live.answered_for_test(json!({"dids": [{"did": 0x2610, "values": [0, 90, 800, 0]}, {"did": 0x2700, "values": [14.2, 35.0]}]}));
        let c = g.card_json();
        let names = |k: &str| c[k].as_array().unwrap().iter().map(|s| s["symbol"].as_str().unwrap().to_string()).collect::<Vec<_>>();
        assert_eq!(names("gauges"), ["VSP", "VB"]);
        assert_eq!(names("tiles"), ["ECT", "MAP"]);
        assert_eq!(c["title"], "Engine");
        assert_eq!(c["note"], "Live from the engine computer");
        let f = g.figures();
        assert_eq!(f["NE"]["text"], "800");
        assert_eq!(f["ECT"]["unit"], "°F");
        assert!(f.get("IG").is_none(), "a signal the catalog lacks has no figure");
        // the split form: the layout holds names and no reading, the live part holds every figure by "did:i"
        let (l, v) = (g.layout_json(), g.live_json());
        assert_eq!(l["gauges"][0]["name"], c["gauges"][0]["name"]);
        assert!(l["gauges"][0].get("text").is_none(), "a layout has no reading in it");
        assert_eq!(v["sig"]["9744:3"]["text"], c["gauges"][0]["text"], "VSP is group 0x2610 (9744), index 3");
        assert_eq!(v["note"], c["note"]);
    }

    #[test]
    fn the_glance_is_for_the_first_system_of_its_types_with_a_database_name() {
        let ecus = [Ecu { system_type: "PGM-FI".into(), ..Default::default() }, pcm()];
        assert_eq!(ecu_for(&DEFS[0], &ecus).unwrap().system, "CANFI", "one without a system name is skipped");
        assert!(ecu_for(&DEFS[1], &ecus).is_none());
    }
}
