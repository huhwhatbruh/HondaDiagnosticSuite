//! The one QObject QML talks to: the app's state as properties (JSON where it is a list or a map), its actions as invokables.
//! Every backend reply comes back to the Qt thread through `CxxQtThread::queue`, so this is the only place state changes;
//! QML never sees a half-updated car.

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qml_singleton]
        #[qproperty(i32, link)] // 0 idle, 1 connecting, 2 connected
        #[qproperty(bool, scanning)]
        #[qproperty(QString, vin)]
        #[qproperty(QString, adapter)]
        #[qproperty(QString, adapters)] // [{spec, label}]
        #[qproperty(QString, last_adapter, cxx_name = "lastAdapter")]
        #[qproperty(QString, ecus)] // [{key, address, system, system_type, display, name, zone, blurb, shared}]
        #[qproperty(QString, dtcs)] // {key: {loading, dtcs, incomplete, available, error}}
        #[qproperty(QString, health)] // core::HealthSummary
        #[qproperty(QString, engine)] // {symbol: figure} for the hero (the engine glance's hero, gauge and tile signals)
        #[qproperty(QString, glances)] // [{title, gauges, tiles}] without readings: the Overview's key-number cards (Glance::layout_json)
        #[qproperty(QString, glances_live, cxx_name = "glancesLive")] // [{note, sig}], one per card: their readings (Glance::live_json)
        #[qproperty(QString, read_checked, cxx_name = "readChecked")] // {key: bool}: the systems ticked in the Read codes dialog
        #[qproperty(QString, status)] // the backend's `status` reply
        #[qproperty(QString, maps)] // the engine maps card (maps::maps_json)
        #[qproperty(QString, history)] // the History screen (history::History::history_json)
        #[qproperty(QString, selftests)] // the open system's self-tests (selftests::selftests_json), "null" when it has none
        #[qproperty(QString, selfcheck)] // the installation check on the welcome screen (selfcheck::selfcheck_json)
        #[qproperty(QString, car_wires, cxx_name = "carWires")] // the car's wire model: [{strength, path}] (car::wires), set once at start
        #[qproperty(QString, car_body, cxx_name = "carBody")] // the car's outline: {path, a, b, c, d} (car::BODY)
        #[qproperty(QString, maintenance)] // the reference snapshot card (baseline::card_json)
        #[qproperty(QString, condition)] // the Condition card (condition::card_json)
        #[qproperty(QString, freeze_info, cxx_name = "freezeInfo")] // the save-snapshot dialog's facts (baseline::freeze_info_json)
        #[qproperty(QString, act)] // the running actuator test for the bar under the header (actuator::running_json), "null" when none
        #[qproperty(QString, actuator)] // the open system's tests (actuator::tests_json)
        #[qproperty(QString, writes)] // the open system's write functions (writes::writes_json)
        #[qproperty(QString, write_running, cxx_name = "writeRunning")] // the name of the write in flight, empty when none
        #[qproperty(QString, drive)] // the This drive card (drive::drive_json)
        #[qproperty(QString, system)] // the open system's screen without readings (sysview::SysView::layout_json), "{}" when none
        #[qproperty(QString, system_live, cxx_name = "systemLive")] // its readings, rate and read errors (sysview::SysView::live_json), "{}" when none
        #[qproperty(QString, kline)] // the K-line screen (kline::Kline::view); its `mode` is "" on a CAN adapter and before any connect
        #[qproperty(QString, version)]
        #[qproperty(QString, backend_error, cxx_name = "backendError")]
        #[qproperty(i32, intro_plan, cxx_name = "introPlan")] // the startup animation: 0 not decided yet, 1 play it, 2 skip it (intro::due, once the settings are read)
        #[qproperty(bool, intro_on, cxx_name = "introOn")] // its setting, the sidebar's tick box
        #[qproperty(bool, share_ask, cxx_name = "shareAsk")] // the first-launch question about sharing a car report is due (share.rs: not answered yet, and this build has a collector)
        #[qproperty(bool, share_can, cxx_name = "shareCan")] // this build has a collector to send to (without one the app asks nothing and shows no switch)
        #[qproperty(bool, share_on, cxx_name = "shareOn")] // the answer, the sidebar's tick box
        #[qproperty(QString, share_text, cxx_name = "shareText")] // the report a send would post, for the card's "See what is sent"; empty until asked
        #[qproperty(QString, intro_sequence, cxx_name = "introSequence")] // assets/intro/sequence.json: which frame file each tick shows, set once at start (not read by QML: it may not read local files)
        type AppModel = super::AppModelRust;

        /// The person's answer to the sharing question (the card's two buttons, or the sidebar's tick box). Share turns it on, anything else
        /// turns it off; either way the card is not asked again.
        #[qinvokable]
        #[cxx_name = "shareAnswer"]
        fn share_answer(self: Pin<&mut Self>, on: bool);

        /// Ask the backend for the text a send would post (`shareText`); it is empty while there is no car to report.
        #[qinvokable]
        #[cxx_name = "sharePreview"]
        fn share_preview(self: Pin<&mut Self>);

        /// Start the backend and the polling tick. Called once, when the window is up.
        #[qinvokable]
        fn start(self: Pin<&mut Self>);

        #[qinvokable]
        #[cxx_name = "connectAdapter"]
        fn connect_adapter(self: Pin<&mut Self>, spec: &QString);

        #[qinvokable]
        fn disconnect(self: Pin<&mut Self>);

        /// Read one system's codes (key = "address:system"); nothing is read until the person asks.
        #[qinvokable]
        #[cxx_name = "readCodes"]
        fn read_codes(self: Pin<&mut Self>, key: &QString);

        /// Tick or untick a system in the Read codes dialog (the ticks are kept for the session).
        #[qinvokable]
        #[cxx_name = "setChecked"]
        fn set_checked(self: Pin<&mut Self>, key: &QString, on: bool);

        #[qinvokable]
        #[cxx_name = "setAllChecked"]
        fn set_all_checked(self: Pin<&mut Self>, on: bool);

        /// Read the codes of the ticked systems: the dialog's own action, so nothing is read without it.
        #[qinvokable]
        #[cxx_name = "readSelected"]
        fn read_selected(self: Pin<&mut Self>);

        #[qinvokable]
        fn rescan(self: Pin<&mut Self>);

        /// The K-line screen: look for the answering systems, open one (its codes and live values), or read a car over generic OBD-II.
        #[qinvokable]
        #[cxx_name = "klineScan"]
        fn kline_scan(self: Pin<&mut Self>);

        #[qinvokable]
        #[cxx_name = "klineOpen"]
        fn kline_open(self: Pin<&mut Self>, address: i32, system: &QString);

        #[qinvokable]
        #[cxx_name = "obdRead"]
        fn obd_read(self: Pin<&mut Self>);

        /// HONDASCAN_OPEN=<system type>: open that system's screen as soon as the scan finds it (for screenshots).
        #[qinvokable]
        #[cxx_name = "openHint"]
        fn open_hint(self: &Self) -> QString;

        /// HONDASCAN_SCROLL=<pixels>: scroll the page that far before the screenshot (for the parts below the fold).
        #[qinvokable]
        #[cxx_name = "scrollHint"]
        fn scroll_hint(self: &Self) -> i32;

        /// HONDASCAN_SHOT=file.png: where the window should save a screenshot of itself and quit (empty when not asked); for visual checks.
        #[qinvokable]
        #[cxx_name = "shotPath"]
        fn shot_path(self: &Self) -> QString;

        /// The startup animation began: remember today, so it plays on the first start of each day only.
        #[qinvokable]
        #[cxx_name = "introStarted"]
        fn intro_started(self: Pin<&mut Self>);

        /// The startup animation could not play (its sequence or a frame did not load): said on stderr, which the package check reads.
        #[qinvokable]
        #[cxx_name = "introFailed"]
        fn intro_failed(self: &Self, reason: &QString);

        /// The sidebar's tick box: play the startup animation from now on, or not.
        #[qinvokable]
        #[cxx_name = "chooseIntro"]
        fn choose_intro(self: Pin<&mut Self>, on: bool);

        /// Open a system's live data (loads its signal catalog first when needed).
        #[qinvokable]
        #[cxx_name = "watchSystem"]
        fn watch_system(self: Pin<&mut Self>, key: &QString);

        /// Leave the system screen: its live reads stop.
        #[qinvokable]
        #[cxx_name = "unwatchSystem"]
        fn unwatch_system(self: Pin<&mut Self>);

        #[qinvokable]
        #[cxx_name = "systemQuery"]
        fn system_query(self: Pin<&mut Self>, q: &QString);

        #[qinvokable]
        #[cxx_name = "toggleGroup"]
        fn toggle_group(self: Pin<&mut Self>, did: i32);

        #[qinvokable]
        #[cxx_name = "openAllGroups"]
        fn open_all_groups(self: Pin<&mut Self>, on: bool);

        #[qinvokable]
        #[cxx_name = "togglePin"]
        fn toggle_pin(self: Pin<&mut Self>, did: i32, index: i32);

        #[qinvokable]
        #[cxx_name = "setRunning"]
        fn set_running(self: Pin<&mut Self>, on: bool);

        /// Start a bi-directional test (the dialog already asked, and the person ticked its box). Nothing is sent unless `ticked`
        /// is true and the test and value are ones the backend offers for the open system; the request always carries `confirm: true`.
        #[qinvokable]
        #[cxx_name = "actStart"]
        fn act_start(self: Pin<&mut Self>, test: &QString, value: i32, ticked: bool);

        /// Stop the running test now: control goes back to the ECU.
        #[qinvokable]
        #[cxx_name = "actStop"]
        fn act_stop(self: Pin<&mut Self>);

        /// Run one write function (the dialog already asked, ticked the box and showed the VIN). Nothing is sent without a ticked box
        /// and the VIN the scan read; the request always carries `confirm: true`.
        #[qinvokable]
        #[cxx_name = "writeRun"]
        fn write_run(self: Pin<&mut Self>, key: &QString, ticked: bool);

        /// Cancel the running write: the backend stops the routine and leaves the session.
        #[qinvokable]
        #[cxx_name = "writeCancel"]
        fn write_cancel(self: Pin<&mut Self>);

        /// Save the state the car reports now as its reference snapshot (the dialog already asked). The backend re-checks the gate.
        #[qinvokable]
        #[cxx_name = "freezeBaseline"]
        fn freeze_baseline(self: Pin<&mut Self>, label: &QString);

        /// Add a service marker (kind, note, mileage in miles as typed). It is checked here too; the record is the app's own, nothing goes to the car.
        #[qinvokable]
        #[cxx_name = "addMarker"]
        fn add_marker(self: Pin<&mut Self>, kind: &QString, note: &QString, mileage: &QString);

        #[qinvokable]
        #[cxx_name = "removeMarker"]
        fn remove_marker(self: Pin<&mut Self>, id: i32);

        /// Read the car now and write the printable health report.
        #[qinvokable]
        #[cxx_name = "printReport"]
        fn print_report(self: Pin<&mut Self>);

        /// Write the JSON export and the complete backup.
        #[qinvokable]
        #[cxx_name = "exportData"]
        fn export_data(self: Pin<&mut Self>);

        /// Which engine map is open (a signal symbol such as IG, KLAFAVE, KNRTD1).
        #[qinvokable]
        #[cxx_name = "selectMap"]
        fn select_map(self: Pin<&mut Self>, symbol: &QString);

        /// The History screen was opened: ask the backend for the cars again (sessions recorded since the last look show up).
        #[qinvokable]
        #[cxx_name = "historyOpen"]
        fn history_open(self: Pin<&mut Self>);

        #[qinvokable]
        #[cxx_name = "historySelect"]
        fn history_select(self: Pin<&mut Self>, vin: &QString);

        #[qinvokable]
        #[cxx_name = "historySession"]
        fn history_session(self: Pin<&mut Self>, id: i32);

        /// The open system's data reference (reference::reference_json) for a search: a long table, so it is asked for, not pushed.
        #[qinvokable]
        fn reference(self: &Self, query: &QString) -> QString;

        /// Where each system's button stands on the car (frame units, `side` wide): [{key, name, zone, x, y}].
        #[qinvokable]
        #[cxx_name = "carKeys"]
        fn car_keys(self: &Self, side: f64) -> QString;

        /// Why a marker cannot be added yet ("" when it can): the dialog shows this as the person types.
        #[qinvokable]
        #[cxx_name = "markerCheck"]
        fn marker_check(self: &Self, kind: &QString, note: &QString, mileage: &QString) -> QString;

        /// Open a file with the desktop's own viewer.
        #[qinvokable]
        #[cxx_name = "openPath"]
        fn open_path(self: &Self, path: &QString);

        /// A message for the person: level 0 info, 1 success, 2 warning, 3 error.
        #[qsignal]
        fn toast(self: Pin<&mut Self>, level: i32, id: QString, title: QString, body: QString, seconds: f64);

        /// The backend said something on its own: ("car", {...}) when the ignition changes.
        #[qsignal]
        #[cxx_name = "backendEvent"]
        fn backend_event(self: Pin<&mut Self>, name: QString, data: QString);
    }

    impl cxx_qt::Threading for AppModel {}
}

use crate::adapters::adapter_list;
use crate::actuator;
use crate::history::History;
use crate::maps::maps_json;
use crate::reference::reference_json;
use crate::settings::Settings;
use crate::selfcheck::{passed, selfcheck_json, unix_now, SelfCheckState};
use crate::selftests::{for_ecu as selftests_for, selftests_json};
use crate::baseline;
use crate::maintenance::check_marker;
use crate::api::{parse, SelfCheck, ExportReply, FreezeReply, Maintenance, ReportReply, ActStatus, ActTest, Catalog, Dtc, DtcReply, Ecu, Hello, Port, ScanResult, WriteFn, WriteResult};
use crate::format::tidy;
use crate::backend::{Backend, Event, Reply};
use crate::writes;
use crate::vehicle::{health_summary, unread, DtcState, ReadCodes};
use crate::drive::drive_json;
use crate::glance::{ecu_for, Glance, DEFS};
use crate::sysview::SysView;
use crate::systems::{info, shared_with};
use core::pin::Pin;
use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QDate, QString};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

const TICK: Duration = Duration::from_millis(50);
const STATUS_EVERY: Duration = Duration::from_secs(2);
const GLANCE_EVERY: Duration = Duration::from_millis(250);
const SYSTEM_EVERY: Duration = Duration::from_millis(200);

const INFO: i32 = 0;
const SUCCESS: i32 = 1;
const WARN: i32 = 2;
const ERROR: i32 = 3;

pub struct AppModelRust {
    link: i32,
    scanning: bool,
    vin: QString,
    adapter: QString,
    adapters: QString,
    last_adapter: QString,
    ecus: QString,
    dtcs: QString,
    health: QString,
    engine: QString,
    glances: QString,
    glances_live: QString,
    status: QString,
    system: QString,
    system_live: QString,
    drive: QString,
    maps: QString,
    history: QString,
    selftests: QString,
    selfcheck: QString,
    car_wires: QString,
    car_body: QString,
    maintenance: QString,
    condition: QString,
    freeze_info: QString,
    act: QString,
    actuator: QString,
    writes: QString,
    write_running: QString,
    read_checked: QString,
    kline: QString,
    kline_state: crate::kline::Kline,
    version: QString,
    backend_error: QString,
    intro_plan: i32,
    intro_on: bool,
    share_ask: bool,
    share_can: bool,
    share_on: bool,
    share_text: QString,
    intro_sequence: QString,

    backend: Option<Arc<Backend>>,
    settings: Option<crate::settings::Store>,
    ecu_list: Vec<Ecu>,
    dtc_state: HashMap<String, DtcState>,
    read_codes: ReadCodes,
    history_state: History,
    map_sel: String,
    status_data: Option<crate::api::Status>,
    self_check: Option<SelfCheck>,
    self_check_err: Option<String>,
    backend_version: String,
    settings_writable: bool,
    maintenance_data: Option<Maintenance>,
    maintenance_err: Option<String>,
    last_report: Option<ReportReply>,
    act_tests: Vec<ActTest>,
    act_status: Option<ActStatus>,
    write_fns: Vec<WriteFn>,
    catalogs: HashMap<String, Arc<Catalog>>,
    /// One live poll per glance card (glance::DEFS), started once its system's catalog is here; the first also feeds the hero.
    glance_polls: Vec<Option<Glance>>,
    /// The system screen that is open: its live data, and the system whose catalog is being fetched for it.
    sysview: Option<SysView>,
    watching: Option<String>,
    status_at: Instant,
    status_busy: bool,
}

impl Default for AppModelRust {
    fn default() -> Self {
        AppModelRust {
            link: 0, scanning: false, vin: QString::default(), adapter: QString::default(), adapters: QString::from("[]"),
            last_adapter: QString::default(), ecus: QString::from("[]"), dtcs: QString::from("{}"), health: QString::from("{}"),
            engine: QString::from("{}"), glances: QString::from("[]"), glances_live: QString::from("[]"), status: QString::from("{}"), system: QString::from("{}"), system_live: QString::from("{}"), drive: QString::from("{}"), maps: QString::from("{}"), history: QString::from("{}"), selftests: QString::from("null"), selfcheck: QString::from("{}"), car_wires: QString::from("[]"), car_body: QString::from("{}"), maintenance: QString::from("{}"), condition: QString::from("{}"), freeze_info: QString::from("{}"), act: QString::from("null"), actuator: QString::from("{}"), writes: QString::from("{}"), write_running: QString::default(), read_checked: QString::from("{}"), kline: q(&crate::kline::Kline::default().view()), kline_state: Default::default(), version: QString::default(), backend_error: QString::default(), intro_plan: 0, intro_on: true, share_ask: false, share_can: false, share_on: false, share_text: QString::default(), intro_sequence: QString::default(),
            backend: None, settings: None, ecu_list: Vec::new(), dtc_state: HashMap::new(), read_codes: ReadCodes::default(), history_state: History::default(), map_sel: "IG".into(), status_data: None, self_check: None, self_check_err: None, backend_version: String::new(), settings_writable: false, maintenance_data: None, maintenance_err: None, last_report: None, act_tests: Vec::new(), act_status: None, write_fns: Vec::new(), catalogs: HashMap::new(),
            glance_polls: DEFS.iter().map(|_| None).collect(), sysview: None, watching: None, status_at: Instant::now(), status_busy: false,
        }
    }
}

fn ecu_json(e: &Ecu, all: &[Ecu]) -> Value {
    let i = info(&e.system_type, &e.display);
    json!({ "key": e.key(), "address": e.address, "system": e.system, "system_type": e.system_type, "display": e.display,
            "name": i.name, "zone": i.zone, "blurb": i.blurb, "shared": shared_with(e.address, &e.system_type, all) })
}

/// Each system's codes as QML shows them: the backend's fields plus `full` (P0420, or C1234-49 with its failure type) and
/// `text` (Honda's description in sentence case, or that it is not in the HDS database).
fn dtcs_json(states: &HashMap<String, DtcState>) -> Value {
    let mut v = serde_json::to_value(states).unwrap_or_default();
    for st in v.as_object_mut().into_iter().flat_map(|m| m.values_mut()) {
        for list in ["dtcs", "incomplete"] {
            for d in st[list].as_array_mut().into_iter().flatten() {
                let dtc: Dtc = serde_json::from_value(d.clone()).unwrap_or_default();
                d["full"] = json!(dtc.full());
                d["text"] = json!(if dtc.description.is_empty() { "Not in the HDS database".to_string() } else { tidy(&dtc.description) });
            }
        }
    }
    v
}

fn q(v: &Value) -> QString { QString::from(v.to_string().as_str()) }

/// Today's local date, `YYYY-MM-DD`: what the startup animation's once a day is counted in.
fn today() -> String {
    let d = QDate::current_date();
    format!("{:04}-{:02}-{:02}", d.year(), d.month(), d.day())
}

fn intro_always() -> bool { std::env::var("HONDASCAN_INTRO").is_ok_and(|v| v == "always") }

mod kline;
mod live;
mod maintenance;
mod ops;
mod screens;

impl qobject::AppModel {
    /// Send a request; `f` runs on the Qt thread with the reply.
    fn rpc<F>(&self, method: &str, params: Value, f: F)
    where F: FnOnce(Pin<&mut qobject::AppModel>, Reply) + Send + 'static {
        let qt = self.qt_thread();
        let Some(be) = self.rust().backend.clone() else { return };
        be.call(method, params, Box::new(move |r| { let _ = qt.queue(move |m| f(m, r)); }));
    }

    fn say(mut self: Pin<&mut Self>, level: i32, id: &str, title: &str, body: &str, seconds: f64) {
        self.as_mut().toast(level, QString::from(id), QString::from(title), QString::from(body), seconds);
    }

    fn publish_ecus(mut self: Pin<&mut Self>) {
        let all = self.rust().ecu_list.clone();
        let list: Vec<Value> = all.iter().map(|e| ecu_json(e, &all)).collect();
        self.as_mut().set_ecus(q(&Value::Array(list)));
    }

    fn publish_codes(mut self: Pin<&mut Self>) {
        let (dtcs, health) = {
            let r = self.rust();
            let mut health = serde_json::to_value(health_summary(&r.ecu_list, &r.dtc_state)).unwrap_or_default();
            // the systems whose codes are not known (not read, not being read, or the read failed): never shown as "no codes"
            health["unread"] = unread(&r.ecu_list, &r.dtc_state).iter().map(|e| json!(info(&e.system_type, &e.display).name)).collect();
            (dtcs_json(&r.dtc_state), health)
        };
        self.as_mut().set_dtcs(q(&dtcs));
        self.as_mut().set_health(q(&health));
        self.as_mut().publish_maintenance();
    }

    /// Forget the car: its systems, codes and live values (a new connection, or a disconnect).
    fn forget(mut self: Pin<&mut Self>) {
        {
            let mut r = self.as_mut().rust_mut();
            r.ecu_list.clear();
            r.dtc_state.clear();
            r.glance_polls = DEFS.iter().map(|_| None).collect();
            r.sysview = None;
            r.watching = None;
            r.read_codes.reset();
            r.act_tests.clear();
            r.act_status = None;
            r.write_fns.clear();
            r.maintenance_data = None;
            r.maintenance_err = None;
            r.status_data = None;
            r.kline_state = Default::default();
        }
        self.as_mut().publish_kline();
        self.as_mut().set_maps(q(&maps_json(None, "IG")));
        self.as_mut().set_write_running(QString::default());
        self.as_mut().set_vin(QString::default());
        self.as_mut().set_status(QString::from("{}"));
        self.as_mut().set_drive(q(&drive_json(None)));
        self.as_mut().publish_ecus();
        self.as_mut().publish_codes();
        self.as_mut().publish_glances();
        self.as_mut().publish_system();
        self.as_mut().publish_checked();
        self.as_mut().publish_ops();
    }

    fn publish_checked(mut self: Pin<&mut Self>) {
        let v = serde_json::to_value(&self.rust().read_codes.checked).unwrap_or_default();
        self.as_mut().set_read_checked(q(&v));
    }

    pub fn start(mut self: Pin<&mut Self>) {
        if self.rust().backend.is_some() { return; }
        let qt = self.qt_thread();
        let sink_qt = qt.clone();
        let backend = Backend::new(Arc::new(move |e| { let _ = sink_qt.queue(move |m| m.on_event(e)); }));
        self.as_mut().rust_mut().backend = Some(backend);
        self.as_mut().set_adapters(q(&adapters_json(&[])));
        let wires: Vec<Value> = crate::car::wires().unwrap_or_default().into_iter().map(|g| json!({ "strength": g.strength, "path": g.path })).collect();
        self.as_mut().set_car_wires(q(&Value::Array(wires)));
        self.as_mut().set_intro_sequence(QString::from(crate::intro::sequence()));
        let (a, b, c, d) = crate::car::BODY_MATRIX;
        self.as_mut().set_car_body(q(&json!({ "path": crate::car::BODY, "a": a, "b": b, "c": c, "d": d })));
        std::thread::spawn(move || loop {
            std::thread::sleep(TICK);
            if qt.queue(|m| m.tick()).is_err() { break; }
        });
        self.rpc("hello", json!({}), |m, r| m.on_hello(r));
        self.rpc("ports", json!({}), |m, r| m.on_ports(r));
    }

    pub fn connect_adapter(mut self: Pin<&mut Self>, spec: &QString) {
        let spec = spec.to_string();
        if let Some(st) = &mut self.as_mut().rust_mut().settings {
            let mut s = st.last.clone();
            s.adapter = crate::settings::remembered(&s.adapter, &spec);
            st.save(&s);
        }
        self.as_mut().set_link(1);
        self.as_mut().set_adapter(QString::from(spec.as_str()));
        self.as_mut().forget();
        let s = spec.clone();
        self.rpc("connect", json!({ "adapter": spec }), move |m, r| m.on_connect(&s, r));
    }

    pub fn disconnect(mut self: Pin<&mut Self>) {
        self.rpc("disconnect", json!({}), |_, _| {});
        self.as_mut().set_link(0);
        self.as_mut().forget();
    }

    pub fn open_hint(&self) -> QString { QString::from(std::env::var("HONDASCAN_OPEN").unwrap_or_default().as_str()) }

    pub fn scroll_hint(&self) -> i32 { std::env::var("HONDASCAN_SCROLL").ok().and_then(|v| v.parse().ok()).unwrap_or(0) }

    pub fn intro_started(mut self: Pin<&mut Self>) {
        if let Some(st) = &mut self.as_mut().rust_mut().settings {
            let s = Settings { intro_day: today(), ..st.last.clone() };
            st.save(&s);
        }
    }

    pub fn intro_failed(&self, reason: &QString) { eprintln!("intro: {reason}"); }

    pub fn choose_intro(mut self: Pin<&mut Self>, on: bool) {
        self.as_mut().set_intro_on(on);
        if let Some(st) = &mut self.as_mut().rust_mut().settings {
            let s = Settings { intro_off: !on, ..st.last.clone() };
            st.save(&s);
        }
    }

    pub fn shot_path(&self) -> QString { QString::from(std::env::var("HONDASCAN_SHOT").unwrap_or_default().as_str()) }

    pub fn rescan(mut self: Pin<&mut Self>) {
        // a K-line adapter has no CAN scan (its CAN link refuses every request): scanning again is that screen's own read
        match self.rust().kline_state.mode {
            "kwp" => return self.kline_scan(),
            "obd" => return self.obd_read(),
            _ => {}
        }
        if *self.link() != 2 || *self.scanning() { return; } // one scan at a time
        self.as_mut().set_scanning(true);
        self.rpc("scan", json!({}), |m, r| m.on_scan(r));
    }

    pub fn read_codes(mut self: Pin<&mut Self>, key: &QString) {
        let key = key.to_string();
        let Some(e) = self.rust().ecu_list.iter().find(|e| e.key() == key).cloned() else { return };
        self.as_mut().rust_mut().dtc_state.entry(key.clone()).or_default().loading = true;
        self.as_mut().publish_codes();
        self.rpc("dtcs", json!({ "address": e.address, "system": e.system }), move |m, r| m.on_dtcs(key, r));
    }

    pub fn set_checked(mut self: Pin<&mut Self>, key: &QString, on: bool) {
        self.as_mut().rust_mut().read_codes.checked.insert(key.to_string(), on);
        self.publish_checked();
    }

    pub fn set_all_checked(mut self: Pin<&mut Self>, on: bool) {
        let ecus = self.rust().ecu_list.clone();
        self.as_mut().rust_mut().read_codes.set_all(&ecus, on);
        self.publish_checked();
    }

    pub fn read_selected(mut self: Pin<&mut Self>) {
        let chosen: Vec<String> = { let r = self.rust(); r.read_codes.chosen(&r.ecu_list).iter().map(|e| e.key()).collect() };
        for k in chosen { self.as_mut().read_codes(&QString::from(k.as_str())); }
    }

    fn on_hello(mut self: Pin<&mut Self>, r: Reply) {
        match r {
            Ok(v) => {
                let h: Hello = parse(v).unwrap_or_default();
                self.as_mut().set_version(QString::from(h.version.as_str()));
                self.as_mut().rust_mut().backend_version = h.version.clone();
                if !h.data_dir.is_empty() {
                    let st = crate::settings::Store::open(&h.data_dir);
                    let play = !self.intro_sequence().is_empty() && crate::intro::due(&st.last, &today(), intro_always(), !self.shot_path().is_empty());
                    self.as_mut().set_intro_on(!st.last.intro_off);
                    self.as_mut().set_last_adapter(QString::from(crate::settings::remembered("", &st.last.adapter).as_str()));
                    let writable = st.writable();
                    self.as_mut().rust_mut().settings_writable = writable;
                    self.as_mut().rust_mut().settings = Some(st);
                    // last: QML answers this by starting the animation, which records today in the settings stored just above
                    self.as_mut().set_intro_plan(if play { 1 } else { 2 });
                }
                // no settings folder to remember a day in: no animation (unless asked for)
                if *self.intro_plan() == 0 {
                    let have_frames = !self.intro_sequence().is_empty();
                    self.as_mut().set_intro_plan(if intro_always() && have_frames { 1 } else { 2 });
                }
                self.as_mut().load_share();
                // check this computer at every start (offline, instant); the welcome screen shows the answer
                self.as_mut().run_self_check();
                if let Some(p) = &h.pending_recovery {
                    let n = crate::handback::interrupted(p);
                    self.as_mut().say(n.level as i32, "pending-recovery", &n.title, &n.body, n.seconds);
                }
                if let Some(a) = h.autoconnect { self.connect_adapter(&QString::from(a.as_str())); }
            }
            Err(e) => {
                self.as_mut().set_intro_plan(2);
                self.as_mut().set_backend_error(QString::from(e.as_str()));
                self.as_mut().rust_mut().self_check_err = Some(e.clone());
                self.as_mut().publish_selfcheck();
                self.say(ERROR, "exit", "The scanner backend did not start", &e, 30.0);
            }
        }
    }

    fn on_ports(mut self: Pin<&mut Self>, r: Reply) {
        if let Ok(v) = r {
            let ports: Vec<Port> = parse(v).unwrap_or_default();
            self.as_mut().set_adapters(q(&adapters_json(&ports)));
        }
    }

    fn on_connect(mut self: Pin<&mut Self>, spec: &str, r: Reply) {
        // a reply that arrives after Disconnect (or a backend restart) belongs to a link that is gone
        if *self.link() != 1 { return; }
        match r {
            Ok(v) => {
                let c: crate::api::Connected = parse(v).unwrap_or_default();
                self.as_mut().set_link(2);
                let on_kline = self.as_mut().kline_connected(spec, c.kline.as_deref());
                if let Some(w) = c.warning { self.as_mut().say(WARN, "warn-adapter", "Another program may disturb the adapter", &w, 20.0); }
                if let Some(n) = c.recovery.as_ref().and_then(crate::handback::on_connect) {
                    self.as_mut().say(n.level as i32, "handback", &n.title, &n.body, n.seconds);
                }
                if !on_kline { self.rescan(); }
            }
            Err(e) => {
                self.as_mut().set_link(0);
                self.say(ERROR, "connect", &format!("Could not open {spec}"), &e, 8.0);
            }
        }
    }

    fn on_scan(mut self: Pin<&mut Self>, r: Reply) {
        self.as_mut().set_scanning(false);
        if *self.link() != 2 { return; }
        let s: ScanResult = match r {
            Ok(v) => parse(v).unwrap_or_default(),
            Err(e) => return self.say(ERROR, "scan", "Scan failed", &e, 8.0),
        };
        let vin = if s.vin != "UNKNOWN" { s.vin.clone() } else { String::new() };
        self.as_mut().set_vin(QString::from(vin.as_str()));
        let n = s.ecus.len();
        let (sim_note, missing_note) = (crate::scannote::simulated(&s), crate::scannote::missing(&s));
        {
            let mut rm = self.as_mut().rust_mut();
            rm.ecu_list = s.ecus;
            let keys: Vec<String> = rm.ecu_list.iter().map(Ecu::key).collect();
            rm.dtc_state.retain(|k, _| keys.contains(k)); // a rescan that no longer finds a system drops its codes
            rm.glance_polls = DEFS.iter().map(|_| None).collect();
            let ecus = rm.ecu_list.clone();
            rm.read_codes.sync(&ecus);
        }
        self.as_mut().publish_checked();
        self.as_mut().publish_ecus();
        self.as_mut().publish_codes();
        self.as_mut().refresh_maintenance();
        self.rpc("actuator_tests", json!({}), |mut m, r| {
            if let Ok(v) = r { m.as_mut().rust_mut().act_tests = parse(v).unwrap_or_default(); }
            m.publish_ops();
        });
        self.rpc("write_functions", json!({}), |mut m, r| {
            if let Ok(v) = r { m.as_mut().rust_mut().write_fns = parse(v).unwrap_or_default(); }
            m.publish_ops();
        });
        self.as_mut().say(SUCCESS, "scan", &format!("{n} system{} found", if n == 1 { "" } else { "s" }), &format!("Recording session {}", s.session), 5.0);
        if let Some(note) = sim_note { self.as_mut().say(WARN, "simulated", &note.title, &note.body, 20.0); }
        if let Some(note) = missing_note { self.as_mut().say(WARN, "missing-systems", &note.title, &note.body, 30.0); }
        self.start_glances();
    }

    fn on_catalog(mut self: Pin<&mut Self>, e: Ecu, r: Reply) {
        let Ok(v) = r else { return };
        let Ok(cat) = parse::<Catalog>(v) else { return };
        self.as_mut().rust_mut().catalogs.insert(e.system.clone(), Arc::new(cat));
        for (idx, def) in DEFS.iter().enumerate() {
            let mine = self.rust().glance_polls[idx].is_none() && ecu_for(def, &self.rust().ecu_list).is_some_and(|x| x.system == e.system);
            if mine { self.as_mut().start_glance(idx, &e); }
        }
        let wanted = self.rust().watching.as_ref().and_then(|k| self.rust().ecu_list.iter().find(|x| x.key() == *k)).cloned();
        if let Some(w) = wanted.filter(|w| w.system == e.system) { self.begin_watch(&w); }
    }

    fn on_dtcs(mut self: Pin<&mut Self>, key: String, r: Reply) {
        if *self.link() != 2 { return; }
        let r = r.and_then(|v| parse::<DtcReply>(v).map_err(|e| format!("unreadable reply: {e}")));
        self.as_mut().rust_mut().dtc_state.entry(key).or_default().take(r);
        self.publish_codes();
    }

    fn on_status(mut self: Pin<&mut Self>, r: Reply) {
        self.as_mut().rust_mut().status_busy = false;
        if let Ok(v) = r {
            let status: Option<crate::api::Status> = parse(v.clone()).ok();
            if let Some(a) = status.as_ref().and_then(|s| s.actuator.clone()) {
                let was = self.rust().act_status.as_ref().is_some_and(|x| x.active);
                if a.active || was { self.as_mut().rust_mut().act_status = Some(a); self.as_mut().publish_ops(); }
            }
            self.as_mut().set_status(q(&v));
            self.as_mut().set_drive(q(&drive_json(status.as_ref())));
            let sel = self.rust().map_sel.clone();
            self.as_mut().set_maps(q(&maps_json(status.as_ref().and_then(|s| s.maps.as_ref()), &sel)));
            self.as_mut().rust_mut().status_data = status;
            self.as_mut().publish_ops();
        }
    }

    /// The monitor pushed a system's code list: say which codes are new, and keep the list (the last read's self-test records stay).
    fn on_monitor_codes(mut self: Pin<&mut Self>, data: &Value) {
        let (a, sys) = (data["address"].as_u64().unwrap_or(0) as u32, data["system"].as_str().unwrap_or(""));
        let Some(e) = self.rust().ecu_list.iter().find(|e| e.address == a && e.system == sys).cloned() else { return };
        let Ok(list) = parse::<Vec<Dtc>>(data["dtcs"].clone()) else { return };
        let before = self.rust().dtc_state.get(&e.key()).and_then(|d| d.dtcs.clone());
        let added = crate::vehicle::new_codes(before.as_deref(), &list);
        if !added.is_empty() {
            let name = info(&e.system_type, &e.display).name;
            self.as_mut().say(WARN, &format!("code-{}", e.key()), &format!("New code in {name}"), &added.join(", "), 10.0);
        }
        // a read or clear of ours may still be in flight: its reply settles `loading`
        {
            let mut rm = self.as_mut().rust_mut();
            let d = rm.dtc_state.entry(e.key()).or_default();
            d.dtcs = Some(list);
            d.error = None;
        }
        self.publish_codes();
    }

    /// The car woke into a new session: its systems and codes are read again, and the live reads start over.
    fn on_new_session(mut self: Pin<&mut Self>, data: &Value) {
        let Ok(s) = parse::<ScanResult>(data.clone()) else { return };
        {
            let mut rm = self.as_mut().rust_mut();
            rm.dtc_state.clear();
            rm.ecu_list = s.ecus;
            let ecus = rm.ecu_list.clone();
            rm.read_codes.sync(&ecus);
            rm.glance_polls = DEFS.iter().map(|_| None).collect();
        }
        self.as_mut().publish_ecus();
        self.as_mut().publish_codes();
        self.as_mut().publish_checked();
        self.start_glances();
    }

    fn on_event(mut self: Pin<&mut Self>, e: Event) {
        match e {
            Event::Backend { event, data } => {
                if event == "car" {
                    let off = data["state"] == "off";
                    self.as_mut().say(INFO, "car", if off { "Car is off: session saved" } else { "Car is on: new session" },
                        if off { "Watching for the ignition to come back." } else { "" }, 6.0);
                }
                match event.as_str() {
                    "error" => self.as_mut().say(ERROR, "backend-error", "Background read failed", data["message"].as_str().unwrap_or(""), 8.0),
                    "codes" => self.as_mut().on_monitor_codes(&data),
                    "session" if data["state"] == "started" => self.as_mut().on_new_session(&data),
                    _ => {}
                }
                if event == "actuator" {
                    if let Ok(a) = parse::<ActStatus>(data.clone()) {
                        let was = self.rust().act_status.as_ref().is_some_and(|x| x.active);
                        if let Some((bad, title, body)) = actuator::ended(was, &a) { self.as_mut().say(if bad { ERROR } else { INFO }, "act-end", &title, &body, 10.0); }
                        self.as_mut().rust_mut().act_status = Some(a);
                        self.as_mut().publish_ops();
                    }
                }
                self.backend_event(QString::from(event.as_str()), q(&data));
            }
            Event::Exit(why) => {
                // a write or test that was in flight when the backend died has to be handed back before the car is forgotten: the
                // reply to `recover` shows whether it worked, and the next call starts a fresh backend
                let writing = !self.write_running().is_empty();
                let actuating = self.rust().act_status.as_ref().is_some_and(|a| a.active);
                let saved = self.rust().settings.as_ref().map(|s| s.last.adapter.clone());
                if let Some(spec) = crate::handback::recover_adapter(writing, actuating, saved.as_deref(), &self.adapter().to_string()) {
                    let s = spec.clone();
                    self.rpc("recover", json!({ "adapter": spec }), move |m, r| m.on_recover(&s, r));
                }
                if *self.link() != 0 { self.as_mut().set_link(0); self.as_mut().forget(); }
                self.say(ERROR, "exit", "The scanner backend stopped", &why, 30.0);
            }
        }
    }

    /// Everything that polls: the status every 2 s, the glance cards' and the open system's live values back to back.
    fn tick(mut self: Pin<&mut Self>) {
        if *self.link() != 2 { return; }
        if !self.rust().status_busy && self.rust().status_at.elapsed() > STATUS_EVERY {
            {
                let mut rm = self.as_mut().rust_mut();
                rm.status_at = Instant::now();
                rm.status_busy = true;
            }
            self.rpc("status", json!({}), |m, r| m.on_status(r));
        }
        let ask_sys = self.as_mut().rust_mut().sysview.as_mut().and_then(|v| v.live.due(&[]).map(|d| (v.ecu.clone(), d)));
        // the open screen already reads its groups: a glance of the same ECU (hidden behind it) does not ask for them again
        let screen = self.rust().sysview.as_ref().map(|v| (v.ecu.address, v.live.dids.clone()));
        if let Some((e, dids)) = ask_sys {
            self.rpc("live", json!({ "address": e.address, "system": e.system, "dids": dids }), |m, r| m.on_system_live(r));
        }
        for idx in 0..DEFS.len() {
            let ask = match self.as_mut().rust_mut().glance_polls.get_mut(idx) {
                Some(Some(g)) => {
                    let skip = screen.as_ref().filter(|(a, _)| *a == g.ecu.address).map_or(&[][..], |(_, d)| d);
                    g.live.due(skip).map(|d| (g.ecu.clone(), d))
                }
                _ => None,
            };
            if let Some((e, dids)) = ask {
                self.rpc("live", json!({ "address": e.address, "system": e.system, "dids": dids }), move |m, r| m.on_glance_live(idx, r));
            }
        }
    }
}

fn adapters_json(ports: &[Port]) -> Value {
    Value::Array(adapter_list(ports).into_iter().map(|(spec, label)| json!({ "spec": spec, "label": label })).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_carry_their_full_code_and_a_readable_description() {
        let mut st = HashMap::new();
        st.insert("14:CANFI".to_string(), DtcState { dtcs: Some(vec![
            Dtc { code: "P0420".into(), failure_type: 0x49, kind: "confirmed".into(), description: "CATALYST SYSTEM EFFICIENCY BELOW THRESHOLD".into(), ..Default::default() },
            Dtc { code: "P1234".into(), failure_type: -1, ..Default::default() }]), ..Default::default() });
        let v = dtcs_json(&st);
        let list = &v["14:CANFI"]["dtcs"];
        assert_eq!(list[0]["full"], "P0420-49");
        assert_ne!(list[0]["text"], "CATALYST SYSTEM EFFICIENCY BELOW THRESHOLD", "capitals are sentence-cased");
        assert_eq!((list[1]["full"].as_str(), list[1]["text"].as_str()), (Some("P1234"), Some("Not in the HDS database")));
        assert!(v["14:CANFI"]["error"].is_null(), "the backend fields are kept");
    }
}
