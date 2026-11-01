//! What the app knows about the car, apart from how it is drawn: the codes each system reported, the health summary,
//! the baseline gate and diff, the Read-codes selection. Moved from hondascan-gui/src/app.rs; no UI types in here.
use crate::api::*;
use serde::Serialize;
use std::collections::HashMap;

#[derive(Default, Serialize)]
pub struct DtcState { pub loading: bool, pub dtcs: Option<Vec<Dtc>>, pub incomplete: Vec<Dtc>, pub available: Option<u32>, pub error: Option<String> }

impl DtcState {
    /// The outcome of a read: a reply replaces the last error, an error replaces the last list (a list that could not be
    /// read again is no longer known, so the system shows as not read rather than with codes that may be gone). The
    /// reply's self-tests-not-run records and the status bits the ECU reports come with it; an error drops those too.
    pub fn take(&mut self, r: Result<DtcReply, String>) {
        self.loading = false;
        match r {
            Ok(v) => { self.dtcs = Some(v.dtcs); self.incomplete = v.incomplete; self.available = Some(v.available); self.error = None; }
            Err(e) => { self.dtcs = None; self.incomplete = Vec::new(); self.available = None; self.error = Some(e); }
        }
    }
}

/// Which systems the person ticked in the Read codes dialog (ui/read_codes.rs): codes are read when asked, never by
/// themselves after a scan or when the car wakes into a new session. The ticks are kept for the session (a system found
/// for the first time starts ticked) and cleared when the car is forgotten.
#[derive(Default)]
pub struct ReadCodes { pub open: bool, pub checked: HashMap<String, bool> }

impl ReadCodes {
    /// Keep the ticks of the systems still found and tick a system found for the first time.
    pub fn sync(&mut self, ecus: &[Ecu]) {
        self.checked.retain(|k, _| ecus.iter().any(|e| e.key() == *k));
        for e in ecus { self.checked.entry(e.key()).or_insert(true); }
    }
    pub fn reset(&mut self) { self.open = false; self.checked.clear(); }
    pub fn is_checked(&self, e: &Ecu) -> bool { self.checked.get(&e.key()).copied().unwrap_or(false) }
    pub fn set_all(&mut self, ecus: &[Ecu], on: bool) { for e in ecus { self.checked.insert(e.key(), on); } }
    pub fn chosen<'a>(&self, ecus: &'a [Ecu]) -> Vec<&'a Ecu> { ecus.iter().filter(|e| self.is_checked(e)).collect() }
}

/// The codes in `now` that were not in `before` (matched by code and failure type), as written for a notice. A system whose codes
/// were not known before has no "new" ones: nothing to compare with.
pub fn new_codes(before: Option<&[Dtc]>, now: &[Dtc]) -> Vec<String> {
    let Some(before) = before else { return Vec::new() };
    now.iter().filter(|d| !before.iter().any(|b| same_code(b, d))).map(Dtc::full).collect()
}

/// Systems whose codes are not known (read failed, or not read yet and not being read): never shown as "no codes".
pub fn unread<'a>(ecus: &'a [Ecu], dtcs: &HashMap<String, DtcState>) -> Vec<&'a Ecu> {
    ecus.iter().filter(|e| dtcs.get(&e.key()).is_none_or(|d| !d.loading && d.dtcs.is_none())).collect()
}

/// One system's row of the health summary. `unknown` when its codes are not known (not read, being read, or the read
/// failed): such a system is never shown as clean. `not_run` counts the self-tests the ECU reports as not completed
/// (that system's emissions-readiness picture), `available` the status bits the read reported.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SysHealth {
    pub key: String, pub name: String, pub confirmed: usize, pub pending: usize, pub stored: usize,
    pub not_run: usize, pub available: Option<u32>, pub unknown: bool,
}

/// The car's health at a glance: one row per found system, the car's totals, and how many systems are not known.
/// Derived from what has been read (`app.ecus` + `app.dtcs`); it talks to no one.
#[derive(Default, Serialize)]
pub struct HealthSummary { pub systems: Vec<SysHealth>, pub faults: usize, pub pending: usize, pub stored: usize, pub not_run: usize, pub unknown: usize }

/// The health summary over every found system, computed from the DTC states (an unknown system never counts as clean).
pub fn health_summary(ecus: &[Ecu], dtcs: &HashMap<String, DtcState>) -> HealthSummary {
    let mut out = HealthSummary::default();
    for e in ecus {
        let name = crate::systems::info(&e.system_type, &e.display).name;
        let st = dtcs.get(&e.key());
        // A system still being read (or whose read failed) is unknown too: never "no codes" and never a clean bill.
        let unknown = st.is_none_or(|d| d.dtcs.is_none());
        let v = st.and_then(|d| d.dtcs.as_ref());
        let k = |kind: &str| v.map_or(0, |v| v.iter().filter(|d| d.kind == kind).count());
        let (confirmed, pending, stored) = (k("confirmed"), k("pending"), k("stored"));
        let not_run = st.map_or(0, |d| d.incomplete.len());
        out.faults += confirmed;
        out.pending += pending;
        out.stored += stored;
        out.not_run += not_run;
        out.unknown += unknown as usize;
        out.systems.push(SysHealth { key: e.key(), name, confirmed, pending, stored, not_run, available: st.and_then(|d| d.available), unknown });
    }
    out
}

/// Whether the car is in a state a healthy baseline may be frozen from, and if not, why (the freeze button's hover text
/// and the freeze dialog both say it): the VIN has to be known, every found system's `19 02` has to have answered, and
/// there may be no confirmed or pending code. Stored codes are history and a self-test not yet run is readiness still to come
/// (a used car has several until a drive cycle ends them): both are recorded in the snapshot and neither blocks it.
pub fn can_freeze(ecus: &[Ecu], dtcs: &HashMap<String, DtcState>, vin: Option<&str>) -> Result<(), String> {
    let known_vin = vin.is_some_and(|v| !v.is_empty() && v != "UNKNOWN");
    let mut why: Vec<String> = Vec::new();
    if !known_vin { why.push("the car's VIN has not been read yet".into()); }
    let (mut confirmed, mut pending) = (0, 0);
    let mut unread: Vec<String> = Vec::new();
    for e in ecus {
        let name = crate::systems::info(&e.system_type, &e.display).name;
        match dtcs.get(&e.key()) {
            Some(d) if !d.loading && d.dtcs.is_some() => {
                let list = d.dtcs.as_ref().unwrap();
                confirmed += list.iter().filter(|c| c.kind == "confirmed").count();
                pending += list.iter().filter(|c| c.kind == "pending").count();
            }
            _ => unread.push(name),
        }
    }
    if !unread.is_empty() {
        why.push(format!("{} system{} not read yet ({})", unread.len(), if unread.len() == 1 { "" } else { "s" }, unread.join(", ")));
    }
    if confirmed > 0 { why.push(format!("{confirmed} confirmed code{}", if confirmed == 1 { "" } else { "s" })); }
    if pending > 0 { why.push(format!("{pending} pending code{}", if pending == 1 { "" } else { "s" })); }
    if why.is_empty() { Ok(()) } else { Err(why.join(" · ")) }
}

/// What changed against the newest frozen baseline, system by system. A system whose codes were not read now is left out
/// (it is not "resolved"): only a fresh read can prove a code gone. `readiness_changes` names the systems whose set of
/// self-tests not run differs from the baseline's.
#[derive(Default, Clone, Debug, PartialEq, Serialize)]
pub struct BaselineDiff {
    pub new: Vec<(String, Dtc)>,
    pub resolved: Vec<(String, Dtc)>,
    pub still: usize,
    pub readiness_changes: Vec<String>,
}

impl BaselineDiff {
    /// Whether the difference alone means attention: a new code that is not merely stored history, or a readiness change.
    /// Stored-only new codes are reported but do not by themselves make the car unhealthy.
    pub fn attention(&self) -> bool { self.new.iter().any(|(_, d)| d.kind != "stored") || !self.readiness_changes.is_empty() }
    pub fn new_stored(&self) -> usize { self.new.iter().filter(|(_, d)| d.kind == "stored").count() }
}

fn same_code(a: &Dtc, b: &Dtc) -> bool { a.code == b.code && a.failure_type == b.failure_type }

/// Compare the newest frozen baseline with what the car reports now (the DTC states the app already holds).
pub fn baseline_diff(latest: &Baseline, dtcs: &HashMap<String, DtcState>) -> BaselineDiff {
    let mut out = BaselineDiff::default();
    for sys in &latest.systems {
        let name = if sys.system_type.is_empty() && sys.display.is_empty() { sys.system.clone() }
            else { crate::systems::info(&sys.system_type, &sys.display).name };
        let key = format!("{}:{}", sys.address, sys.system);
        let Some(st) = dtcs.get(&key) else { continue };
        let Some(now) = st.dtcs.as_ref() else { continue };
        for b in &sys.dtcs {
            if now.iter().any(|c| same_code(c, b)) { out.still += 1; } else { out.resolved.push((name.clone(), b.clone())); }
        }
        for c in now {
            if !sys.dtcs.iter().any(|b| same_code(b, c)) { out.new.push((name.clone(), c.clone())); }
        }
        let ids = |list: &[Dtc]| { let mut v: Vec<(String, i32)> = list.iter().map(|d| (d.code.clone(), d.failure_type)).collect(); v.sort(); v };
        let (was, is) = (ids(&sys.incomplete), ids(&st.incomplete));
        if was != is {
            out.readiness_changes.push(format!("{name}: {} self-test{} not run (baseline {})", is.len(),
                if is.len() == 1 { "" } else { "s" }, was.len()));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ecu(address: u32, system: &str, system_type: &str) -> Ecu { Ecu { address, system: system.into(), system_type: system_type.into(), ..Default::default() } }

    #[test]
    fn a_read_replaces_the_last_outcome() {
        let mut d = DtcState { loading: true, ..Default::default() };
        d.take(Err("no response to 1902ff".into()));
        assert!(!d.loading && d.dtcs.is_none() && d.error.is_some() && d.available.is_none());
        d.take(Ok(DtcReply { dtcs: vec![Dtc { code: "P0420".into(), ..Default::default() }], available: 0x09, ..Default::default() }));
        assert!(d.error.is_none() && d.dtcs.as_ref().is_some_and(|v| v.len() == 1) && d.available == Some(0x09), "a good read clears the old error");
        d.take(Err("not connected to an adapter".into()));
        assert!(d.dtcs.is_none() && d.available.is_none(), "codes that could not be read again are not shown as current");
    }

    #[test]
    fn the_health_summary_counts_what_was_read_and_distrusts_the_rest() {
        let ecus = [ecu(0x0E, "CANFI", "PGM-FI"), ecu(0x53, "CANSRS", "SRS")];
        let mut dtcs = HashMap::new();
        // PGM-FI: one confirmed, one stored, one self-test not run; the read reported the ECU's status bits.
        dtcs.insert(ecus[0].key(), DtcState {
            dtcs: Some(vec![Dtc { kind: "confirmed".into(), ..Default::default() }, Dtc { kind: "stored".into(), ..Default::default() }]),
            incomplete: vec![Dtc::default()], available: Some(0x09), ..Default::default()
        });
        // SRS: still being read: unknown, and never folded into a clean bill.
        dtcs.insert(ecus[1].key(), DtcState { loading: true, ..Default::default() });
        let s = health_summary(&ecus, &dtcs);
        assert_eq!((s.faults, s.pending, s.stored, s.not_run, s.unknown), (1, 0, 1, 1, 1));
        assert_eq!(s.systems.len(), 2);
        assert_eq!(s.systems[0].available, Some(0x09));
        assert!(s.systems[1].unknown && s.systems[1].available.is_none());
        // A failed read is unknown too, not clean.
        dtcs.insert(ecus[1].key(), DtcState { error: Some("timeout".into()), ..Default::default() });
        assert_eq!(health_summary(&ecus, &dtcs).unknown, 1);
        // A clean read with no codes is clean.
        dtcs.insert(ecus[1].key(), DtcState { dtcs: Some(vec![]), ..Default::default() });
        let s = health_summary(&ecus, &dtcs);
        assert_eq!((s.unknown, s.faults, s.pending, s.stored), (0, 1, 0, 1));
        assert_eq!(s.systems[1].name, "Airbags (SRS)");
    }

    #[test]
    fn failed_and_unread_systems_are_never_counted_as_clean() {
        let ecus = [ecu(0x0E, "CANFI", "PGM-FI"), ecu(0x53, "CANSRS", "SRS"), ecu(0x60, "CAN_METER", "METER"), ecu(0x28, "CANABS", "HONDA ABS")];
        let mut dtcs = HashMap::new();
        dtcs.insert(ecus[0].key(), DtcState { dtcs: Some(Vec::new()), ..Default::default() });
        dtcs.insert(ecus[1].key(), DtcState { error: Some("timeout".into()), ..Default::default() });
        dtcs.insert(ecus[2].key(), DtcState { loading: true, ..Default::default() });
        let names: Vec<&str> = unread(&ecus, &dtcs).iter().map(|e| e.system.as_str()).collect();
        assert_eq!(names, ["CANSRS", "CANABS"]);
    }

    #[test]
    fn a_pushed_code_list_names_only_the_codes_that_were_not_there() {
        let (a, b) = ([dtc("P0420", "confirmed")], [dtc("P0171", "pending")]);
        let both = [a[0].clone(), b[0].clone()];
        assert_eq!(new_codes(Some(&a), &both), ["P0171"]);
        assert!(new_codes(Some(&both), &a).is_empty(), "a code that went away is not new");
        assert!(new_codes(None, &a).is_empty(), "no earlier read, nothing to compare with");
        let typed = [Dtc { failure_type: 0x49, ..a[0].clone() }];
        assert_eq!(new_codes(Some(&a), &typed), ["P0420-49"], "the same code with another failure type is a new one");
    }

    fn dtc(code: &str, kind: &str) -> Dtc { Dtc { code: code.into(), failure_type: -1, kind: kind.into(), ..Default::default() } }
    fn state(dtcs: Vec<Dtc>, incomplete: Vec<Dtc>) -> DtcState { DtcState { dtcs: Some(dtcs), incomplete, ..Default::default() } }
    fn full_state(list: Vec<Dtc>) -> HashMap<String, DtcState> {
        let ecus = [ecu(0x0E, "CANFI", "PGM-FI"), ecu(0x53, "CANSRS", "SRS")];
        let mut m = HashMap::new();
        for (e, d) in ecus.iter().zip([state(list.clone(), vec![]), state(Vec::new(), vec![])]) { m.insert(e.key(), d); }
        m
    }
    fn base_system(address: u32, system: &str, dtcs: Vec<Dtc>, incomplete: Vec<Dtc>) -> BaselineSystem {
        BaselineSystem { address, system: system.into(), system_type: "PGM-FI".into(), display: "PGM-FI".into(), answered: true,
            dtcs, incomplete, ..Default::default() }
    }
    fn baseline(systems: Vec<BaselineSystem>) -> Baseline {
        Baseline { id: 1, vin: "1HGCV1F3XLA000001".into(), created: 1.7e9, label: "Healthy".into(), healthy: true, systems, ..Default::default() }
    }

    #[test]
    fn the_freeze_gate_needs_a_known_vin_every_read_and_no_active_code() {
        let ecus = [ecu(0x0E, "CANFI", "PGM-FI"), ecu(0x53, "CANSRS", "SRS")];
        let vin = Some("1HGCV1F3XLA000001");
        let mut clean = full_state(Vec::new());
        assert_eq!(can_freeze(&ecus, &clean, vin), Ok(()));
        for bad in [None, Some(""), Some("UNKNOWN")] { assert!(can_freeze(&ecus, &clean, bad).is_err(), "no VIN, no baseline"); }
        // one system still being read, one whose read failed, and one that was never read are all unknown, never clean
        for st in [DtcState { loading: true, ..Default::default() }, DtcState { error: Some("timeout".into()), ..Default::default() }] {
            clean.insert(ecus[1].key(), st);
            assert!(can_freeze(&ecus, &clean, vin).unwrap_err().contains("not read"));
        }
        clean.remove(&ecus[1].key());
        assert!(can_freeze(&ecus, &clean, vin).is_err(), "a found system with no state at all is not read either");
        // confirmed or pending codes block; stored ones do not
        let mut dtcs = full_state(vec![dtc("P0420", "confirmed")]);
        assert!(can_freeze(&ecus, &dtcs, vin).unwrap_err().contains("confirmed"));
        dtcs.insert(ecus[0].key(), state(vec![dtc("P0420", "pending")], vec![]));
        assert!(can_freeze(&ecus, &dtcs, vin).unwrap_err().contains("pending"));
        dtcs.insert(ecus[0].key(), state(vec![dtc("P0420", "stored")], vec![]));
        assert_eq!(can_freeze(&ecus, &dtcs, vin), Ok(()), "stored codes are history and allowed");
        // a self-test that has not run is recorded, not blocking: a used car has several until a drive cycle ends them
        dtcs.insert(ecus[0].key(), state(Vec::new(), vec![dtc("P1000", "")]));
        assert_eq!(can_freeze(&ecus, &dtcs, vin), Ok(()));
    }

    #[test]
    fn the_baseline_diff_counts_new_still_and_resolved_and_ignores_what_was_not_read() {
        let ecus = [ecu(0x0E, "CANFI", "PGM-FI"), ecu(0x53, "CANSRS", "SRS")];
        let clean = baseline(vec![base_system(0x0E, "CANFI", vec![], vec![])]);
        let clean_states = HashMap::from([(ecus[0].key(), state(vec![], vec![]))]);
        let d = baseline_diff(&clean, &clean_states);
        assert_eq!((d.new.len(), d.resolved.len(), d.still), (0, 0, 0));
        assert!(!d.attention());

        // a new stored code is reported but does not make the car need attention by itself
        let stored = HashMap::from([(ecus[0].key(), state(vec![dtc("P0420", "stored")], vec![]))]);
        let d = baseline_diff(&clean, &stored);
        assert_eq!(d.new.len(), 1);
        assert_eq!(d.new[0].0, "Engine", "the system is named as the rest of the app names it");
        assert_eq!(d.new_stored(), 1);
        assert!(!d.attention(), "stored history alone is not a new fault");

        // a new confirmed code does, and a code with no kind is treated as needing attention too
        let confirmed = HashMap::from([(ecus[0].key(), state(vec![dtc("P0420", "confirmed")], vec![]))]);
        assert!(baseline_diff(&clean, &confirmed).attention());

        // a code that was in the baseline and is not now is resolved, not new
        let old = baseline(vec![base_system(0x0E, "CANFI", vec![dtc("P0420", "stored")], vec![])]);
        let d = baseline_diff(&old, &clean_states);
        assert_eq!((d.resolved.len(), d.still, d.new.len()), (1, 0, 0));
        assert_eq!(d.resolved[0].1.code, "P0420");
        assert!(!d.attention(), "a code going away is good news");

        // the same code still present
        let same = baseline(vec![base_system(0x0E, "CANFI", vec![dtc("P0420", "stored")], vec![])]);
        assert_eq!(baseline_diff(&same, &stored).still, 1);

        // a system in the baseline that was not read now proves nothing: its codes are not resolved
        let two = baseline(vec![base_system(0x0E, "CANFI", vec![dtc("P0420", "stored")], vec![]), base_system(0x53, "CANSRS", vec![dtc("B0001", "stored")], vec![])]);
        let d = baseline_diff(&two, &stored);
        assert_eq!((d.resolved.len(), d.still), (0, 1), "the SRS row is left out, not counted as resolved");
        // and a system still being read (no list yet) is left out too
        let loading = HashMap::from([(ecus[0].key(), DtcState { loading: true, ..Default::default() })]);
        assert_eq!(baseline_diff(&two, &loading), BaselineDiff::default());

        // readiness: the set of self-tests not run changed
        let ready = baseline(vec![base_system(0x0E, "CANFI", vec![], vec![dtc("P1000", "")])]);
        let d = baseline_diff(&ready, &clean_states);
        assert_eq!(d.readiness_changes.len(), 1);
        assert!(d.readiness_changes[0].contains("Engine") && d.readiness_changes[0].contains("baseline 1"), "{:?}", d.readiness_changes);
        assert!(d.attention(), "a readiness change is reported as needing attention");
        let d = baseline_diff(&ready, &HashMap::from([(ecus[0].key(), state(vec![], vec![dtc("P1000", "")]))]));
        assert!(d.readiness_changes.is_empty(), "the same set is no change");
    }

    #[test]
    fn read_codes_ticks_new_systems_and_keeps_the_choice_across_a_rescan() {
        let (engine, srs) = (ecu(0x0E, "CANFI", "PGM-FI"), ecu(0x53, "CANSRS", "SRS"));
        let mut r = ReadCodes::default();
        r.sync(&[engine.clone(), srs.clone()]);
        assert!(r.is_checked(&engine) && r.is_checked(&srs), "a system found for the first time starts ticked");
        // the person unticks the engine; a rescan keeps that choice and ticks the system it finds for the first time
        r.checked.insert(engine.key(), false);
        let meter = ecu(0x60, "CAN_METER", "METER");
        r.sync(&[engine.clone(), srs.clone(), meter.clone()]);
        assert!(!r.is_checked(&engine), "an unticked system stays unticked across a rescan");
        assert!(r.is_checked(&meter), "a newly found system starts ticked");
        let chosen: Vec<String> = r.chosen(&[engine.clone(), srs.clone(), meter.clone()]).iter().map(|e| e.key()).collect();
        assert_eq!(chosen, [srs.key(), meter.key()]);
        assert!(!r.chosen(&[engine.clone(), srs.clone()]).is_empty());
        // a system the rescan no longer finds is dropped from the ticks
        r.sync(std::slice::from_ref(&srs));
        assert_eq!(r.checked.len(), 1);
        assert!(r.checked.contains_key(&srs.key()));
    }

    #[test]
    fn read_codes_set_all_and_reset() {
        let (engine, srs) = (ecu(0x0E, "CANFI", "PGM-FI"), ecu(0x53, "CANSRS", "SRS"));
        let mut r = ReadCodes { open: true, ..Default::default() };
        r.sync(&[engine.clone(), srs.clone()]);
        r.set_all(&[engine.clone(), srs.clone()], false);
        assert!(r.chosen(&[engine.clone(), srs.clone()]).is_empty(), "nothing ticked, nothing to read");
        r.set_all(&[engine.clone(), srs.clone()], true);
        assert_eq!(r.chosen(&[engine.clone(), srs.clone()]).len(), 2);
        r.reset();
        assert!(!r.open && r.checked.is_empty(), "a disconnect or a new adapter forgets the choice");
        assert!(r.chosen(&[engine, srs]).is_empty());
    }
}
