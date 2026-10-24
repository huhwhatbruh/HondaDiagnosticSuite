//! Self-tests (OBD monitors) an ECU has not completed, from the DTC records that carry only the test-not-completed
//! status bits. Port of hondascan/selftests.py: the list at the first read of a session is the set of tests still to
//! run since the engine started, and each code that leaves it is a test that ran and passed.
use serde_json::{json, Map, Value};
use std::collections::{BTreeSet, HashMap};

pub struct Dtc {
    pub code: String,
    pub failure_type: i64,
    pub description: String,
}

type Key = (String, i64);

#[derive(Default)]
pub struct SelfTests {
    first: Vec<(String, BTreeSet<Key>)>, // system -> keys, in first-seen order
    now: HashMap<String, BTreeSet<Key>>,
    info: HashMap<(String, String, i64), String>, // (system, code, failure type) -> description
}

impl SelfTests {
    pub fn update(&mut self, system: &str, records: &[Dtc]) {
        let keys: BTreeSet<Key> = records.iter().map(|d| (d.code.clone(), d.failure_type)).collect();
        for d in records {
            self.info.insert((system.into(), d.code.clone(), d.failure_type), d.description.clone());
        }
        if !self.first.iter().any(|(s, _)| s == system) {
            self.first.push((system.into(), keys.clone()));
        }
        self.now.insert(system.into(), keys);
    }

    pub fn summary(&self) -> Value {
        let named = |system: &str, keys: Vec<&Key>| -> Vec<Value> {
            keys.into_iter()
                .map(|(c, f)| {
                    let d = self.info.get(&(system.into(), c.clone(), *f)).cloned().unwrap_or_default();
                    json!({"code": c, "failure_type": f, "description": d})
                })
                .collect()
        };
        let mut out = Map::new();
        for (s, first) in &self.first {
            let now = &self.now[s];
            out.insert(
                s.clone(),
                json!({"at_start": first.len(), "now": now.len(),
                       "completed": named(s, first.difference(now).collect()),
                       "incomplete": named(s, now.iter().collect())}),
            );
        }
        Value::Object(out)
    }
}
