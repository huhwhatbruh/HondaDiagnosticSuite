//! The shapes of what the backend sends (hondascan/src/rpc.rs, service.rs), parsed leniently: a field the
//! backend does not send is its default, never an error.
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct Ecu { pub address: u32, pub system_type: String, pub system: String, pub display: String, pub rule: String }

impl Ecu {
    /// One system: an address can hold several (0x10: PGM-FI and EVPS).
    pub fn key(&self) -> String { format!("{}:{}", self.address, if self.system.is_empty() { &self.system_type } else { &self.system }) }
}

#[derive(Deserialize, Clone, Default, Debug, PartialEq)]
#[serde(default)]
pub struct Dtc { pub code: String, pub failure_type: i32, pub status: u32, pub kind: String, pub flags: Vec<String>,
    pub honda_code: String, pub description: String, pub help: String }

impl Dtc {
    /// A code with its failure-type byte when it has one: P0420, C1234-49.
    pub fn full(&self) -> String { code_with_type(&self.code, self.failure_type) }
    pub fn confirmed(&self) -> bool { self.kind == "confirmed" }
}

/// P0420, or C1234-49 when the failure-type byte is set; 0 or less means none (the backend sends -1 for K-line codes).
pub fn code_with_type(code: &str, failure_type: i32) -> String {
    if failure_type <= 0 { code.to_string() } else { format!("{code}-{failure_type:02X}") }
}

#[derive(Deserialize, Default, Debug)]
#[serde(default)]
pub struct DtcReply { pub available: u32, pub dtcs: Vec<Dtc>, pub incomplete: Vec<Dtc> }

#[derive(Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct SignalDef { pub symbol: String, pub description: String, pub unit: String, pub states: Option<Vec<String>>,
    pub help: Option<String>, pub invalid_stopped: bool }

#[derive(Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct DidDef { pub did: u32, pub signals: Vec<SignalDef> }

#[derive(Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct Catalog { pub system: String, pub dids: Vec<DidDef>, pub freeze: Vec<DidDef>, pub readiness: Vec<DidDef> }

impl Catalog {
    /// The first signal with this symbol: (did, index in its group, definition).
    pub fn find(&self, symbol: &str) -> Option<(u32, usize, &SignalDef)> {
        self.dids.iter().find_map(|d| d.signals.iter().position(|s| s.symbol == symbol).map(|i| (d.did, i, &d.signals[i])))
    }
}

#[derive(Deserialize, Default, Debug)]
#[serde(default)]
pub struct DidValues { pub did: u32, pub values: Option<Vec<Value>>, pub error: Option<String> }

#[derive(Deserialize, Default, Debug)]
#[serde(default)]
pub struct LivePacket { pub t: f64, pub dids: Vec<DidValues> }

#[derive(Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct Alert { pub id: String, pub level: String, pub title: String, pub text: String }

impl Alert { pub fn fault(&self) -> bool { self.level == "fault" } }

#[derive(Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct MinMax { pub min: f64, pub mean: f64, pub max: f64 }

#[derive(Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct Economy { pub km_per_l: f64, pub l_per_100km: f64, pub mpg_us: f64 }

#[derive(Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct Fuel { pub engine_counter_l: Option<f64>, pub maf_estimate_l: Option<f64>, pub economy: Option<Economy> }

#[derive(Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct Times { pub engine: f64, pub idle: f64 }

#[derive(Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct Trip { pub duration_s: f64, pub distance_km: f64, pub times_s: Times, pub engine_run_s: Option<f64>, pub fuel: Fuel,
    pub lateral_g: Option<MinMax>, pub brake_pressure_bar: Option<MinMax> }

#[derive(Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct MapSignal { pub name: String, pub unit: String, pub samples: u32, pub cells: Vec<[f64; 6]> }

#[derive(Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct Maps { pub rpm: Vec<f64>, pub load: Vec<f64>, pub min_cell: u32, pub signals: std::collections::BTreeMap<String, MapSignal>, pub baseline: MapBaseline }

/// This car's own history for the maps (hondascan mapbaseline.py): how many earlier sessions there are, and for every current cell
/// that could be compared [rpm bin, load bin, sessions, earlier mean, spread, z]; `cylinders` is the same for each cylinder's whole-session mean.
#[derive(Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct MapBaseline { pub sessions: u32, pub min_sessions: u32, pub z: f64, pub rule: String, pub cells: std::collections::BTreeMap<String, Vec<[f64; 6]>>, pub cylinders: Vec<CylBase> }

#[derive(Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct CylBase { pub signal: String, pub cylinder: u32, pub now: f64, pub sessions: u32, pub mean: f64, pub sd: f64, pub z: f64 }

#[derive(Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct Reads { pub ok: u32, pub failed: u32 }

#[derive(Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct Status {
    pub connected: bool, pub adapter: String, pub vin: Option<String>, pub asleep: bool, pub recording: bool,
    pub session: Option<u32>, pub folder: Option<String>, pub started: Option<f64>, pub reads: Option<Reads>,
    pub trip: Option<Trip>, pub maps: Option<Maps>, pub alerts: Vec<Alert>,
    pub fuel_flow_lph: Option<f64>, pub actuator: Option<ActStatus>,
    pub self_tests: std::collections::HashMap<String, SelfTestSys>,
}

/// One system's self-tests (selftests.py summary): how many were waiting at the first read, how many now, and which.
#[derive(Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct SelfTestSys { pub at_start: u32, pub now: u32, pub completed: Vec<SelfTestItem>, pub incomplete: Vec<SelfTestItem> }

#[derive(Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct SelfTestItem { pub code: String, pub failure_type: i32, pub description: String }

/// A bi-directional test the backend offers (actuator.py TESTS): what it drives, the values HDS sends, what must hold, its time limit.
#[derive(Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct ActTest { pub key: String, pub system: String, pub name: String, pub did: u32, pub values: Vec<i64>, pub conditions: Vec<String>,
    pub hold_s: f64, pub source: String, pub state: Vec<String> }

/// The running test (actuatorrun.py status): `active` is false once control is back with the ECU, `reason` says why it ended.
#[derive(Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct ActStatus { pub test: String, pub name: String, pub active: bool, pub value: i64, pub elapsed: Option<f64>, pub remaining: Option<f64>,
    pub reason: String, pub errors: Vec<String> }

/// A write function the backend offers (write.rs WRITES): the exact requests, the read-back and HDS's own conditions.
/// `stop` is empty when HDS's procedure sends none (CVT_ISL); `poll_ms` > 0 means HDS polls the read-back instead of
/// waiting a fixed time.
#[derive(Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct WriteFn { pub key: String, pub system: String, pub name: String, pub start: String, pub stop: String,
    pub kind: String, pub did: u32, pub byte: usize, pub mask: u32, pub monitor: String, pub wait_s: f64, pub poll_ms: u64, pub timeout_s: f64,
    pub conditions: Vec<String>, pub read_back: String, pub source: String }

/// One write run (writerun.py status): old and new are the read-back success bit, `null` when it was not read; the
/// `*_value` fields carry HDS's status code for a DID-poll write (SLPLMOD/SLPLERR). A routine-status proof (the
/// meter's reset) carries `kind` = "routine_status" with HDS's pending status in `pending` and the final status byte
/// in `new_value`.
#[derive(Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct WriteResult { pub write: String, pub name: String, pub ok: bool, pub kind: String, pub pending: Option<u32>,
    pub old: Option<bool>, pub new: Option<bool>,
    pub old_value: Option<u32>, pub new_value: Option<u32>, pub error_value: Option<u32>, pub poll_ms: u64,
    pub timeout_s: f64, pub errors: Vec<String> }

#[derive(Deserialize, Default, Debug)]
#[serde(default)]
pub struct ScanResult { pub ecus: Vec<Ecu>, pub seconds: f64, pub vin: String, pub session: u32, pub folder: String }

/// What the backend did when it took control back on `connect` (rpc.rs recover): `pending` is true when a write or test
/// was still recorded as in flight, `handed_back` when the requests that end it went out, `error` when they did not.
#[derive(Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct Recovery { pub pending: bool, pub handed_back: bool, pub actions: Vec<String>, pub error: Option<String> }

#[derive(Deserialize, Default, Debug)]
#[serde(default)]
pub struct Connected { pub adapter: String, pub description: String, pub warning: Option<String>, pub recovery: Option<Recovery> }

/// The in-flight write or bi-directional test recorded in `audit/pending.json` when the last backend process died
/// (rpc.rs pending_recovery): what it was, so the app can say why control is being handed back.
#[derive(Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct Pending {
    pub schema: u32, pub at: f64, pub pid: u32, pub kind: String, pub vin: String, pub vin_address: u32,
    pub address: u32, pub system: String, pub name: String, pub frame_log: Option<String>,
    pub key: Option<String>, pub test: Option<String>,
}

#[derive(Deserialize, Default, Debug)]
#[serde(default)]
pub struct Hello { pub autoconnect: Option<String>, pub data_dir: String, pub version: String, pub audit: String, pub pending_recovery: Option<Pending> }

/// One line of the installation check (selfcheck.rs): ok, warn or fail, with what was found.
#[derive(Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct Check { pub id: String, pub title: String, pub status: String, pub detail: String }

#[derive(Deserialize, Default, Debug)]
#[serde(default)]
pub struct SelfCheck { pub ok: bool, pub version: String, pub data_dir: String, pub checks: Vec<Check> }

/// One frozen baseline as the maintenance list names it (no systems: the full payload is `latest`).
#[derive(Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct BaselineMeta { pub id: i64, pub created: f64, pub label: String, pub healthy: bool, pub sha256: String,
    pub integrity: String, pub systems_count: u32 }

#[derive(Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct ReadinessSignal { pub symbol: String, pub value: Option<f64>, pub unit: String, pub state: Option<String> }

#[derive(Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct ReadinessGroup { pub did: u32, pub signals: Vec<ReadinessSignal> }

#[derive(Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct BaselineSystem { pub address: u32, pub system: String, pub system_type: String, pub display: String,
    pub answered: bool, pub available: u32, pub dtcs: Vec<Dtc>, pub incomplete: Vec<Dtc>, pub readiness: Vec<ReadinessGroup> }

#[derive(Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct BaselineTotals { pub systems: u32, pub faults: u32, pub pending: u32, pub stored: u32, pub incomplete: u32 }

/// The immutable healthy-baseline payload `baseline_freeze` stores: what the car reported at that moment. `session` is
/// absent when the baseline was frozen without one, `readiness` empty where the catalog has no OBD status groups.
#[derive(Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct Baseline { pub id: i64, pub schema: u32, pub vin: String, pub created: f64, pub label: String,
    pub session: Option<u32>, pub adapter: String, pub healthy: bool, pub systems: Vec<BaselineSystem>, pub totals: BaselineTotals }

/// A service marker: work done between sessions, with an optional odometer reading (kilometres, as stored).
#[derive(Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct Marker { pub id: i64, pub at: f64, pub kind: String, pub note: String, pub mileage_km: Option<f64>, pub created: f64 }

#[derive(Deserialize, Default, Debug)]
#[serde(default)]
pub struct Maintenance { pub vin: String, pub baselines: Vec<BaselineMeta>, pub latest: Option<Baseline>, pub markers: Vec<Marker> }

#[derive(Deserialize, Default, Debug)]
#[serde(default)]
pub struct FreezeReply { pub baseline: Option<Baseline>, pub frozen: bool }

#[derive(Deserialize, Default, Debug)]
#[serde(default)]
pub struct ReportReply { pub path: String, pub json_path: String, pub verdict: String, pub summary: String, pub report: Value }

impl ReportReply {
    /// The VIN the report is about, when the report object carries one (the app shows it only for this car).
    pub fn vin(&self) -> Option<&str> {
        self.report.get("vin").and_then(Value::as_str).or_else(|| self.report.get("header")?.get("vin")?.as_str())
    }
}

#[derive(Deserialize, Default, Debug)]
#[serde(default)]
pub struct ExportReply { pub json_path: String, pub backup_path: String, pub sessions: u32, pub vehicles: u32, pub bytes: u64 }

#[derive(Deserialize, Default, Debug)]
#[serde(default)]
pub struct AuditReply { pub path: String, pub records: Vec<Value> }

/// A decoded signal's value as a number, when it is one.
pub fn num(v: &Value) -> Option<f64> { v.as_f64() }

#[derive(Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct EcuInfo { pub ecu_id: Option<String>, pub ecu_model: Option<String>, pub part_number: Option<String>, pub part_number_raw: Option<String> }

/// The backend sends `null` for "no value yet" where a number is expected (a trip with no distance); dropping nulls
/// first lets every such field take its default instead of failing the whole message.
pub fn strip_nulls(v: Value) -> Value {
    match v {
        Value::Object(m) => Value::Object(m.into_iter().filter(|(_, v)| !v.is_null()).map(|(k, v)| (k, strip_nulls(v))).collect()),
        Value::Array(a) => Value::Array(a.into_iter().map(strip_nulls).collect()),
        o => o,
    }
}

pub fn parse<T: serde::de::DeserializeOwned>(v: Value) -> Result<T, serde_json::Error> { serde_json::from_value(strip_nulls(v)) }

#[derive(Deserialize, Default, Debug)]
#[serde(default)]
pub struct Port { pub device: String, pub product: String, pub gd101: bool }

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn nulls_take_the_default_instead_of_failing_the_message() {
        let s: Status = parse(json!({"connected": true, "vin": null, "trip": {"duration_s": 5.0, "distance_km": null,
            "fuel": {"economy": null}}, "alerts": null})).unwrap();
        assert!(s.connected && s.vin.is_none() && s.alerts.is_empty());
        assert_eq!(s.trip.unwrap().distance_km, 0.0);
        assert!(serde_json::from_value::<Status>(json!({"trip": {"distance_km": null}})).is_err(), "what parse() is for");
        let d: DtcReply = parse(json!({"dtcs": [{"code": "C1234", "failure_type": 0x49, "kind": "confirmed", "help": null}]})).unwrap();
        assert_eq!(d.dtcs[0].full(), "C1234-49");
        assert!(d.dtcs[0].confirmed());
        let d: DtcReply = parse(json!({"dtcs": [{"code": "P0420", "failure_type": -1}]})).unwrap();
        assert_eq!(d.dtcs[0].full(), "P0420", "the K-line's -1 has no failure type, never -FFFFFFFF");
    }

    #[test]
    fn a_hello_from_an_older_backend_still_parses() {
        let h: Hello = parse(json!({"autoconnect": null, "data_dir": "/tmp/x"})).unwrap();
        assert_eq!(h.data_dir, "/tmp/x");
        assert!(h.version.is_empty() && h.audit.is_empty() && h.pending_recovery.is_none());
        let h: Hello = parse(json!({"data_dir": "/tmp/x", "version": "0.1.0", "audit": "/tmp/x/audit/writes.jsonl",
            "pending_recovery": null})).unwrap();
        assert_eq!(h.version, "0.1.0");
        assert!(h.pending_recovery.is_none(), "a null recovery is no recovery");
        let h: Hello = parse(json!({"pending_recovery": {"schema": 1, "at": 1.5e9, "pid": 42, "kind": "write",
            "vin": "1HGCV1F3XLA000001", "vin_address": 0x18, "address": 0x10, "system": "CANFI", "name": "E-VTC learn",
            "key": "evtc_learn", "frame_log": null, "test": null, "request": "31 01 02 18 02", "session": 3}})).unwrap();
        let p = h.pending_recovery.unwrap();
        assert_eq!((p.schema, p.kind.as_str(), p.name.as_str()), (1, "write", "E-VTC learn"));
        assert_eq!((p.vin_address, p.address), (0x18, 0x10));
        assert!(p.frame_log.is_none() && p.key.as_deref() == Some("evtc_learn") && p.test.is_none());
    }

    #[test]
    fn an_installation_check_parses_and_tolerates_a_short_check() {
        let sc: SelfCheck = parse(json!({"ok": false, "version": "0.1.0", "data_dir": "/d", "checks": [
            {"id": "backend/version", "title": "Backend version", "status": "ok", "detail": "0.1.0"},
            {"id": "database", "title": "Database", "status": "fail"},
            {"id": "udev", "title": "udev rule", "status": "warn", "detail": null}]})).unwrap();
        assert!(!sc.ok);
        assert_eq!(sc.checks.len(), 3);
        assert_eq!(sc.checks[1].status, "fail");
        assert!(sc.checks[1].detail.is_empty(), "a missing detail is an empty string");
        assert_eq!(sc.checks[0].detail, "0.1.0");
        let sc: SelfCheck = parse(json!({"ok": true})).unwrap();
        assert!(sc.ok && sc.checks.is_empty() && sc.version.is_empty());
    }

    #[test]
    fn a_connect_reply_may_carry_the_hand_back_result() {
        let c: Connected = parse(json!({"adapter": "gd101-j2534:/dev/gd101", "warning": null, "recovery": null})).unwrap();
        assert!(c.recovery.is_none());
        let c: Connected = parse(json!({"adapter": "sim:drive", "recovery": {"pending": true, "handed_back": true,
            "actions": ["2F 00 00", "10 01"], "error": null}})).unwrap();
        let r = c.recovery.unwrap();
        assert!(r.pending && r.handed_back && r.error.is_none());
        assert_eq!(r.actions, ["2F 00 00", "10 01"]);
        let c: Connected = parse(json!({"recovery": {"pending": true, "handed_back": false, "error": "VIN mismatch"}})).unwrap();
        let r = c.recovery.unwrap();
        assert!(!r.handed_back && r.actions.is_empty() && r.error.as_deref() == Some("VIN mismatch"));
    }

    #[test]
    fn maintenance_parses_the_meta_list_and_the_full_latest_baseline() {
        let m: Maintenance = parse(json!({
            "vin": "1HGCV1F3XLA000001",
            "baselines": [{"id": 7, "created": 1.7e9, "label": "After service", "healthy": true, "sha256": "ab", "integrity": "ok", "systems_count": 2}],
            "latest": {"id": 7, "schema": 1, "vin": "1HGCV1F3XLA000001", "created": 1.7e9, "label": "After service",
                "session": null, "adapter": "sim:drive", "healthy": true,
                "systems": [{"address": 16, "system": "CANFI", "system_type": "PGM-FI", "display": "PGM-FI", "answered": true, "available": 9,
                    "dtcs": [{"code": "P0420", "failure_type": -1, "kind": "stored"}],
                    "incomplete": [{"code": "P1000", "failure_type": -1, "description": null}],
                    "readiness": [{"did": 0x1A01, "signals": [{"symbol": "CAT", "value": null, "unit": "", "state": "complete"},
                        {"symbol": "EVAP", "value": 1.0, "unit": "x", "state": null}]}]},
                    {"address": 83, "system": "CANSRS", "display": "SRS", "system_type": "SRS", "answered": false}],
                "totals": {"systems": 2, "faults": 0, "pending": 0, "stored": 1, "incomplete": 1}},
            "markers": [{"id": 3, "at": 1.7e9, "kind": "oil", "note": "5W-30 and filter", "mileage_km": 132000.0, "created": 1.7e9},
                {"id": 4, "at": 1.7e9, "kind": "tires", "note": null, "mileage_km": null, "created": 1.7e9}]})).unwrap();
        assert_eq!(m.baselines[0].systems_count, 2);
        assert_eq!(m.baselines[0].integrity, "ok");
        let b = m.latest.unwrap();
        assert_eq!((b.id, b.schema), (7, 1));
        assert!(b.session.is_none() && b.healthy);
        assert_eq!(b.systems.len(), 2);
        assert!(b.systems[0].answered && b.systems[0].dtcs[0].kind == "stored");
        assert_eq!(b.systems[0].incomplete[0].code, "P1000");
        assert_eq!(b.systems[0].readiness[0].signals[0].state.as_deref(), Some("complete"));
        assert!(b.systems[0].readiness[0].signals[1].value == Some(1.0));
        assert!(b.systems[0].readiness[0].signals[1].state.is_none());
        assert!(!b.systems[1].answered && b.systems[1].dtcs.is_empty() && b.systems[1].readiness.is_empty());
        assert_eq!((b.totals.systems, b.totals.stored, b.totals.incomplete), (2, 1, 1));
        assert_eq!(m.markers[0].mileage_km, Some(132000.0));
        assert!(m.markers[1].note.is_empty() && m.markers[1].mileage_km.is_none(), "nulls are dropped, not fatal");
        let empty: Maintenance = parse(json!({"vin": "X", "baselines": null, "latest": null, "markers": null})).unwrap();
        assert!(empty.baselines.is_empty() && empty.latest.is_none() && empty.markers.is_empty());
    }

    #[test]
    fn freeze_report_export_and_audit_replies_parse() {
        let f: FreezeReply = parse(json!({"frozen": true, "baseline": {"id": 9, "vin": "X", "created": 1.7e9, "healthy": true,
            "systems": [], "totals": {"systems": 0}}})).unwrap();
        assert!(f.frozen && f.baseline.as_ref().is_some_and(|b| b.id == 9));
        let f: FreezeReply = parse(json!({"baseline": null})).unwrap();
        assert!(!f.frozen && f.baseline.is_none());

        let r: ReportReply = parse(json!({"path": "reports/a.html", "json_path": "reports/a.json", "verdict": "attention",
            "summary": "1 new code since the baseline", "report": {"vin": "VIN1", "verdict": "attention"}})).unwrap();
        assert_eq!((r.verdict.as_str(), r.vin()), ("attention", Some("VIN1")));
        let r: ReportReply = parse(json!({"path": "reports/a.html", "report": null})).unwrap();
        assert!(r.report.is_null() && r.vin().is_none() && r.summary.is_empty(), "a reply without the report object still shows its path");
        let r: ReportReply = parse(json!({"report": {"header": {"vin": "V2"}}})).unwrap();
        assert_eq!(r.vin(), Some("V2"));

        let e: ExportReply = parse(json!({"json_path": "exports/a.json", "backup_path": "backups/a.tar.gz",
            "sessions": 12, "vehicles": 1, "bytes": 123456})).unwrap();
        assert_eq!((e.sessions, e.vehicles, e.bytes), (12, 1, 123456));
        let e: ExportReply = parse(json!({})).unwrap();
        assert!(e.json_path.is_empty() && e.bytes == 0, "an older backend's export reply is not fatal");

        let a: AuditReply = parse(json!({"path": "audit/writes.jsonl", "records": [
            {"schema": 1, "at": 1.7e9, "event": "attempt", "system": "CANFI", "name": "E-VTC learn"},
            {"event": "result", "ok": true, "outcome": "complete"}]})).unwrap();
        assert_eq!(a.path, "audit/writes.jsonl");
        assert_eq!(a.records.len(), 2);
        assert_eq!(a.records[1]["outcome"], "complete");
        let a: AuditReply = parse(json!({"records": null})).unwrap();
        assert!(a.records.is_empty());
    }
}
