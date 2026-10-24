//! Application state and the frame loop: connection, the systems found, their trouble codes, the polled status, and
//! which screen is open. Mirrors hondascan-app/src/hooks/useVehicle.ts.
mod debug;
mod events;

use crate::api::*;
use crate::backend::{Backend, Event, Reply};
use crate::live::Live;
use crate::theme;
use crate::ui::{system::{SysUi, Tab}, alerts::AlertBar, glance::{self, Glance}, sidebar, toasts::{Level, Toasts}, topbar};
use eframe::egui::{self, Frame, Margin};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub enum View { Overview, History, System(String) }
#[derive(PartialEq, Clone, Copy)]
pub enum Link { Idle, Connecting, Connected }
#[derive(PartialEq, Clone, Copy)]
pub enum Health { Unknown, Fault, Pending, Ok }

impl Health {
    pub fn color(self) -> egui::Color32 {
        match self { Health::Fault => theme::FAULT, Health::Pending => theme::WARN, Health::Ok => theme::OK, Health::Unknown => theme::MUTED_FG }
    }
}

#[derive(Default)]
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

/// Systems whose codes are not known (read failed, or not read yet and not being read): never shown as "no codes".
pub fn unread<'a>(ecus: &'a [Ecu], dtcs: &HashMap<String, DtcState>) -> Vec<&'a Ecu> {
    ecus.iter().filter(|e| dtcs.get(&e.key()).is_none_or(|d| !d.loading && d.dtcs.is_none())).collect()
}

/// One system's row of the health summary. `unknown` when its codes are not known (not read, being read, or the read
/// failed): such a system is never shown as clean. `not_run` counts the self-tests the ECU reports as not completed
/// (that system's emissions-readiness picture), `available` the status bits the read reported.
#[derive(Clone, Debug, PartialEq)]
pub struct SysHealth {
    pub key: String, pub name: String, pub confirmed: usize, pub pending: usize, pub stored: usize,
    pub not_run: usize, pub available: Option<u32>, pub unknown: bool,
}

/// The car's health at a glance: one row per found system, the car's totals, and how many systems are not known.
/// Derived from what has been read (`app.ecus` + `app.dtcs`); it talks to no one.
#[derive(Default)]
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
#[derive(Default, Clone, Debug, PartialEq)]
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

enum Job {
    Ports, Cleared(Ecu), Hello, Connect(String), Scan, Status, Dtcs(String), Catalog(String), ActTests, ActStart(String),
    ActStop, Writes, WriteRun(String), WriteCancel,
    SelfCheck, Maintenance(String), Freeze(String), MarkerAdd, MarkerRemove(i64), Report, Export, Audit(String), Recover(String),
}

pub struct App {
    pub be: Arc<Backend>,
    ev: Receiver<Event>,
    jobs: Vec<(Job, Receiver<Reply>)>,
    pub link: Link,
    pub adapter: String,
    pub adapters: Vec<(String, String)>, // (spec, label); the last is the custom one
    pub choice: usize,
    pub custom: String,
    pub settings: Option<crate::settings::Store>,
    pub voxels: Option<crate::ui::logo::Voxels>, // the logo in the header
    pub turntable: Option<crate::ui::turntable::Turntable>, // the welcome screen's car; None once connected (its textures are freed)
    pub ecus: Vec<Ecu>,
    pub vin: Option<String>,
    pub scanning: bool,
    pub scan_seconds: Option<f64>,
    pub dtcs: HashMap<String, DtcState>,
    catalog_rx: std::sync::mpsc::Receiver<(String, Catalog)>,   // catalogs converted off the UI thread (6-9 ms for the engine's)
    catalog_tx: std::sync::mpsc::Sender<(String, Catalog)>,
    pub catalogs: HashMap<String, std::sync::Arc<Catalog>>,   // shared: a system view takes a handle each frame, not a copy of 700 signals
    pub status: Option<Status>,
    pub act_tests: Vec<ActTest>,
    pub act: Option<ActStatus>, // the last bi-directional test status (reply, event or poll)
    pub writes: Vec<WriteFn>,
    pub last_write: Option<WriteResult>, // the last finished write, for the module view
    status_at: Instant,
    pub toasts: Toasts,
    alert_bar: AlertBar,
    told: Vec<String>,
    pub view: View,
    screen: (String, f64),   // which screen is showing and since when (`motion::entered`)
    pub map_sel: String,
    pub sysui: Option<SysUi>,
    pub history: crate::ui::history::History,
    /// This car's frozen baselines and service markers (rpc.rs `maintenance`): the health card and the dialogs read it.
    pub maintenance: Option<Maintenance>,
    pub maintenance_err: Option<String>,
    /// The backend's installation check (`self_check`), plus what only the app can check (its own version, the settings folder).
    pub self_check: Option<SelfCheck>,
    pub self_check_err: Option<String>,
    pub backend_version: String,
    /// Whether the settings folder could be written when the check was last run (the probe is not repeated every frame).
    pub settings_writable: bool,
    /// The dialogs, Some while open (ui/maintenance.rs): the freeze confirmation and the service marker.
    pub freeze: Option<crate::ui::maintenance::FreezeDialog>,
    pub marker: Option<crate::ui::maintenance::MarkerDialog>,
    pub last_report: Option<ReportReply>,
    pub last_export: Option<ExportReply>,
    pub audit: Vec<Value>,
    pub pending_recovery: Option<Pending>,
    pending_tab: Option<String>,
    pub glances: Vec<Glance>,
    autoconnect: Option<String>,
    shot: Option<debug::Shot>,
    prof: Option<debug::Profile>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, logo: Option<image::RgbaImage>) -> Self {
        theme::install(&cc.egui_ctx);
        let (be, ev) = Backend::new(cc.egui_ctx.clone());
        let (catalog_tx, catalog_rx) = std::sync::mpsc::channel();
        let mut app = App {
            jobs: Vec::new(), ev, link: Link::Idle, adapter: String::new(), adapters: crate::ui::topbar::adapter_list(&[]), choice: 0, custom: "j2534:/usr/lib/libj2534.so".into(), settings: None, voxels: None, turntable: None, ecus: Vec::new(), vin: None,
            scanning: false, scan_seconds: None, dtcs: HashMap::new(), catalogs: HashMap::new(), catalog_rx, catalog_tx, status: None, act_tests: Vec::new(), act: None, writes: Vec::new(), last_write: None,
            status_at: Instant::now(), toasts: Toasts::default(), alert_bar: AlertBar::default(), told: Vec::new(),
            view: View::Overview, screen: (String::new(), 0.0), map_sel: "IG".into(), sysui: None, history: Default::default(), maintenance: None, maintenance_err: None,
            self_check: None, self_check_err: None, backend_version: String::new(), settings_writable: false,
            freeze: None, marker: None,
            last_report: None, last_export: None, audit: Vec::new(), pending_recovery: None, pending_tab: None,
            prof: std::env::var_os("HONDASCAN_PROFILE").map(|_| debug::Profile::new()), glances: glance::DEFS.iter().map(|def| Glance { def, live: None }).collect(), autoconnect: None, be,
            shot: std::env::var("HONDASCAN_SCREENSHOT").ok().map(|path| debug::Shot {
                path, sent: false, after: std::env::var("HONDASCAN_SCREENSHOT_AFTER").ok().and_then(|v| v.parse().ok()).unwrap_or(8.0) }),
        };
        app.voxels = logo.map(|i| crate::ui::logo::Voxels::build(&i, 40, 33, 3));
        app.job(Job::Hello, "hello", json!({}));
        app.job(Job::Ports, "ports", json!({}));
        app
    }

    fn job(&mut self, kind: Job, method: &str, params: serde_json::Value) {
        let rx = self.be.call(method, params);
        self.jobs.push((kind, rx));
    }

    pub fn connect(&mut self, spec: &str) {
        if let Some(st) = &mut self.settings { let mut s = st.last.clone(); s.adapter = spec.to_string(); s.custom = self.custom.clone(); st.save(&s); }
        self.link = Link::Connecting;
        self.adapter = spec.to_string();
        self.forget();
        self.job(Job::Connect(spec.to_string()), "connect", json!({ "adapter": spec }));
    }

    pub fn disconnect(&mut self) {
        let _ = self.be.call("disconnect", json!({}));
        self.link = Link::Idle;
        self.status = None;
        self.forget();
    }

    pub fn scan(&mut self) {
        self.scanning = true;
        self.job(Job::Scan, "scan", json!({}));
    }

    /// Select the adapter used last time, once both the list and the settings are here.
    fn pick_saved(&mut self) {
        let Some(st) = &self.settings else { return };
        if let Some(i) = self.adapters.iter().position(|(spec, _)| *spec == st.last.adapter) { self.choice = i; }
        else if !st.last.adapter.is_empty() { self.choice = self.adapters.len() - 1; self.custom = st.last.adapter.clone(); }
    }

    /// The spec to connect with: the chosen preset or port, or the custom text.
    pub fn spec(&self) -> String {
        let (spec, _) = &self.adapters[self.choice.min(self.adapters.len() - 1)];
        if spec == "custom" { self.custom.trim().to_string() } else { spec.clone() }
    }

    /// Write each system's pins if they changed (the adapter is saved when connecting).
    pub fn remember(&mut self) {
        let Some(st) = &mut self.settings else { return };
        let mut s = st.last.clone();
        if let (Some(sys), Some(ecu)) = (&self.sysui, self.ecus.iter().find(|e| matches!(&self.view, View::System(k) if *k == e.key()))) {
            if let Some(p) = &sys.live.pins { s.pins.insert(ecu.system.clone(), p.clone()); }
        }
        st.save(&s);
    }

    fn forget(&mut self) {
        self.ecus.clear();
        self.dtcs.clear();
        self.act_tests.clear();
        self.act = None;
        self.writes.clear();
        self.last_write = None;
        self.vin = None;
        self.maintenance = None;
        self.maintenance_err = None;
        self.audit.clear();
        for g in &mut self.glances { g.live = None; }
        self.view = View::Overview;
    }

    /// Clear a system's codes (the dialog already asked), then read them again. `14 FF FF FF` goes to the control unit
    /// at that address, so every system it runs (0x0E: engine and CVT) loses its codes: all of them show as being read
    /// until the clear is answered (no second clear can be started meanwhile) and are read again afterwards.
    pub fn clear(&mut self, e: &Ecu) {
        for x in self.ecus.iter().filter(|x| x.address == e.address) { self.dtcs.entry(x.key()).or_default().loading = true; }
        let rx = self.be.call("clear", json!({ "address": e.address, "system": e.system, "confirm": true }));
        self.jobs.push((Job::Cleared(e.clone()), rx));
    }

    /// Start a bi-directional test (the dialog already asked and the person ticked the box): the backend refuses without
    /// `confirm: true`, checks HDS's conditions, and hands control back at the time limit, on an error and on disconnect.
    pub fn act_start(&mut self, e: &Ecu, t: &ActTest, value: i64) {
        let rx = self.be.call("actuator_start", json!({ "address": e.address, "test": t.key, "value": value, "confirm": true }));
        self.jobs.push((Job::ActStart(t.name.clone()), rx));
    }

    /// Stop the running test now: control goes back to the ECU.
    pub fn act_stop(&mut self) { self.job(Job::ActStop, "actuator_stop", json!({})); }

    /// The write in flight, if any (its `write_run` call has not answered yet).
    pub fn write_running(&self) -> Option<String> {
        self.jobs.iter().find_map(|(j, _)| if let Job::WriteRun(n) = j { Some(n.clone()) } else { None })
    }

    /// Cancel the running write: the backend's reader thread sees this while `write_run` blocks, so the run stops early.
    pub fn write_cancel(&mut self) { self.job(Job::WriteCancel, "write_cancel", json!({})); }

    /// Run one write function (the dialog already asked, ticked the box and showed the VIN): the backend refuses without
    /// `confirm: true`, re-reads the ECU's VIN, checks HDS's conditions and keeps the old value.
    pub fn write_start(&mut self, e: &Ecu, w: &WriteFn) {
        let vin = self.vin.clone().unwrap_or_default();
        let rx = self.be.call("write_run", json!({ "address": e.address, "key": w.key, "vin": vin, "confirm": true }));
        self.jobs.push((Job::WriteRun(w.name.clone()), rx));
    }

    /// Load this car's frozen baselines and service markers (the health card reads them; the dialogs refresh them).
    pub fn refresh_maintenance(&mut self) {
        let Some(vin) = self.vin.clone() else { return };
        let rx = self.be.call("maintenance", json!({ "vin": vin }));
        self.jobs.push((Job::Maintenance(vin), rx));
    }

    /// Ask the backend to check this computer again (offline: it opens no adapter). The app's own folder check runs here,
    /// once, not on every frame the checklist is open.
    pub fn run_self_check(&mut self) {
        self.settings_writable = self.settings.as_ref().is_some_and(|s| s.writable());
        self.job(Job::SelfCheck, "self_check", json!({}));
    }

    /// Freeze the state the car reports now as its healthy baseline (the dialog already asked; the backend re-checks the
    /// gate and refuses if the car is not healthy any more).
    pub fn freeze_baseline(&mut self, label: &str) {
        let Some(vin) = self.vin.clone() else { return };
        let label = label.trim();
        let params = if label.is_empty() { json!({ "vin": vin }) } else { json!({ "vin": vin, "label": label }) };
        let rx = self.be.call("baseline_freeze", params);
        self.jobs.push((Job::Freeze(vin), rx));
    }

    /// Add a service marker. The dialog validates it; `mileage_miles` becomes the kilometres the backend stores.
    pub fn add_marker(&mut self, kind: &str, note: &str, mileage_miles: Option<f64>) {
        let Some(vin) = self.vin.clone() else { return };
        let mut params = json!({ "vin": vin, "kind": kind, "note": note });
        if let Some(m) = mileage_miles { params["mileage_km"] = json!(crate::ui::maintenance::miles_to_km(m)); }
        self.job(Job::MarkerAdd, "marker_add", params);
    }

    /// Remove one service marker (the row's Remove button, in History).
    pub fn remove_marker(&mut self, id: i64) {
        let rx = self.be.call("marker_remove", json!({ "id": id }));
        self.jobs.push((Job::MarkerRemove(id), rx));
    }

    /// Read the car now and write the printable health report (HTML and JSON) into `reports/`.
    pub fn print_report(&mut self) {
        let params = self.vin.clone().map_or_else(|| json!({}), |v| json!({ "vin": v }));
        self.job(Job::Report, "health_report", params);
    }

    /// Write the JSON export and the complete backup: the one call writes both files.
    pub fn export_data(&mut self) { self.job(Job::Export, "export", json!({})); }

    /// The write audit for this car, newest first (200 records fills the table generously).
    pub fn load_audit(&mut self) {
        let Some(vin) = self.vin.clone() else { return };
        let rx = self.be.call("audit", json!({ "vin": vin, "limit": 200 }));
        self.jobs.push((Job::Audit(vin), rx));
    }

    pub fn read_dtcs(&mut self, e: &Ecu) {
        self.dtcs.entry(e.key()).or_default().loading = true;
        self.job(Job::Dtcs(e.key()), "dtcs", json!({ "address": e.address, "system": e.system }));
    }

    pub fn health(&self, e: &Ecu) -> Health {
        match self.dtcs.get(&e.key()).and_then(|d| d.dtcs.as_ref()) {
            None => Health::Unknown,
            Some(v) if v.iter().any(Dtc::confirmed) => Health::Fault,
            Some(v) if !v.is_empty() => Health::Pending,
            Some(_) => Health::Ok,
        }
    }

    /// The car's health summary over the found systems (the overview's health header and its rows).
    pub fn health_summary(&self) -> HealthSummary { health_summary(&self.ecus, &self.dtcs) }

    /// The engine's live values and signal catalog, once loaded.
    pub fn engine(&self) -> Option<&Live> { self.glances.first().and_then(|g| g.live.as_ref()) }
    pub fn engine_cat(&self) -> Option<&Catalog> { self.engine().and_then(|l| self.catalogs.get(&l.ecu.system).map(|c| &**c)) }

    /// Keep the open system's state in step with the view: a new one when the view changes, its catalog requested.
    fn sync_system(&mut self) {
        let View::System(key) = &self.view else { self.sysui = None; return };
        let key = key.clone();
        if self.sysui.as_ref().is_none_or(|s| s.key != key) {
            let Some(e) = self.ecus.iter().find(|e| e.key() == key).cloned() else { return };
            let tab = if self.dtcs.get(&key).and_then(|d| d.dtcs.as_ref()).is_some_and(|d| !d.is_empty()) { Tab::Codes } else { Tab::Live };
            if !e.system.is_empty() && !self.catalogs.contains_key(&e.system) && !self.jobs.iter().any(|(j, _)| matches!(j, Job::Catalog(s) if *s == e.system)) {
                self.job(Job::Catalog(e.system.clone()), "catalog", json!({ "system": e.system }));
            }
            let tab = match self.pending_tab.take().as_deref() { Some("codes") => Tab::Codes, Some("data") => Tab::Data, Some("about") => Tab::About, Some("tests") => Tab::Tests, Some("writes") => Tab::Writes, Some(_) => Tab::Live, None => tab };
            let mut ui = SysUi::new(&e, tab, &self.be);
            ui.live.pins = self.settings.as_ref().and_then(|s| s.last.pins.get(&e.system).cloned());
            self.sysui = Some(ui);
        }
    }

}

impl eframe::App for App {
    fn clear_color(&self, _: &egui::Visuals) -> [f32; 4] { theme::BACKGROUND.to_normalized_gamma_f32() }

    /// Everything that is not drawing: backend replies, polling, announcements.
    fn logic(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.pump(ctx);
        self.announce(ctx);
        self.shoot(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let t0 = Instant::now();
        self.draw(ui, frame);
        if let Some(p) = &mut self.prof { p.add(t0.elapsed(), ui.ctx().repaint_causes(), ui.ctx().cumulative_frame_nr()); }
    }

    fn on_exit(&mut self, _: Option<&eframe::glow::Context>) { self.be.shutdown(); }
}

impl App {
    fn draw(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let alerts = self.status.as_ref().map(|s| s.alerts.clone()).unwrap_or_default();

        egui::Panel::top("top").exact_size(48.0).frame(Frame::NONE.fill(theme::PANEL)).show(ui, |ui| topbar::show(ui, self));
        egui::Panel::top("alerts").size_range(0.0..=f32::INFINITY).frame(Frame::NONE).show_separator_line(false).show(ui, |ui| {
            let mut b = std::mem::take(&mut self.alert_bar);
            b.show(ui, &alerts);
            self.alert_bar = b;
        });
        egui::Panel::top("act").size_range(0.0..=f32::INFINITY).frame(Frame::NONE).show_separator_line(false).show(ui, |ui| {
            if let Some(crate::ui::actuator::Action::Stop) = crate::ui::actuator::running_bar(ui, self.act.as_ref(), &self.act_tests) { self.act_stop(); }
        });
        egui::Panel::top("wrun").size_range(0.0..=f32::INFINITY).frame(Frame::NONE).show_separator_line(false).show(ui, |ui| {
            if crate::ui::write::running_bar(ui, self.write_running().as_deref()) { self.write_cancel(); }
        });
        egui::Panel::left("side").exact_size(240.0).resizable(false).frame(Frame::NONE.fill(theme::PANEL).inner_margin(Margin::same(8)))
            .show(ui, |ui| sidebar::show(ui, self));
        egui::CentralPanel::default_margins().frame(Frame::NONE.fill(theme::BACKGROUND).inner_margin(Margin::same(16))).show(ui, |ui| {
            // a screen of its own (and a tab of its own) keeps its own scroll position and fades in when it is switched to
            let key = match (&self.view, &self.sysui) {
                (View::Overview, _) => "overview".to_string(),
                (View::History, _) => "history".to_string(),
                (View::System(k), s) => format!("{k}/{}", s.as_ref().map_or(0, |s| s.tab as u8)),
            };
            if self.screen.0 != key { self.screen = (key.clone(), if self.screen.0.is_empty() { f64::NEG_INFINITY } else { ctx.input(|i| i.time) }); }
            let t = crate::motion::entered(&ctx, self.screen.1);
            let mut sa = egui::ScrollArea::vertical().auto_shrink([false, false]).id_salt(&key);
            if let Some(y) = std::env::var("HONDASCAN_SCROLL").ok().and_then(|v| v.parse::<f32>().ok()) { sa = sa.vertical_scroll_offset(y); } // screenshots below the fold
            sa.show(ui, |ui| { ui.set_opacity(crate::motion::ease(t)); ui.add_space((1.0 - crate::motion::ease(t)) * 14.0); match &self.view {
                View::Overview => crate::ui::overview::show(ui, self),
                View::System(k) => { let k = k.clone(); crate::ui::system::show(ui, self, &k) }
                View::History => crate::ui::history::show(ui, self),
            } });
        });
        crate::ui::frame::resize_edges(&ctx);
        let mut t = std::mem::take(&mut self.toasts);
        t.show(&ctx);
        self.toasts = t;
        // the logo turns at 30 frames a second. egui subtracts one predicted frame (16 ms) from the delay it is given, so 33 ms
        // became 60 frames a second: asking for 50 ms is what makes 30
        ctx.request_repaint_after(Duration::from_millis(if self.voxels.is_some() { 50 } else { 100 }));
    }
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
}
