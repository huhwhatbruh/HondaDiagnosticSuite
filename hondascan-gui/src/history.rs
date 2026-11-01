//! Everything recorded per VIN: sessions with their headline numbers, the trouble-code history, the trends, each
//! session's summary, the service markers and frozen baselines, and the write audit.
//! This is the state and decisions; QML only draws the JSON `history_json` makes. No drawing and no
//! socket here — the lead sends the `(method, params)` calls the call helpers name and feeds the replies back in.
use crate::api::Trip;
use crate::format::{clock, convert, fmt, tidy};
use crate::maintenance::kind_label;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;

#[derive(Deserialize, Default, Clone)]
#[serde(default)]
struct Summary { duration_s: f64, trip: Option<Trip>, codes: Option<Codes> }
#[derive(Deserialize, Default, Clone)]
#[serde(default)]
struct Codes { present: Vec<Value> }
#[derive(Deserialize, Default, Clone)]
#[serde(default)]
struct SessionRow { id: u32, started: f64, end_reason: Option<String>, summary: Option<Summary> }
#[derive(Deserialize, Default, Clone)]
#[serde(default)]
struct CodeHist { system: String, code: String, failure_type: i64, first_seen: f64, last_seen: f64, sessions: u32, last_event: String, description: String }
#[derive(Deserialize, Default, Clone)]
#[serde(default)]
struct Trend { mean: f64, unit: String }
#[derive(Deserialize, Default, Clone)]
#[serde(default)]
struct Vehicle { sessions: Vec<SessionRow>, codes: Vec<CodeHist>, trends: BTreeMap<String, Vec<Trend>> }
#[derive(Deserialize, Default, Clone)]
#[serde(default)]
struct VehicleRow { vin: String }
#[derive(Deserialize, Default, Clone)]
#[serde(default)]
struct SessionDetail { summary_md: String, folder: String }

/// How a session ended, in words: the backend's reasons are its own (`disconnected` is the Disconnect button, `backend stopped`
/// the app closing, `report` a health report read); anything else is shown as it came.
fn ended_text(reason: Option<&str>) -> String {
    match reason {
        None => "running".into(),
        Some("backend stopped") => "app closed".into(),
        Some("report") => "health report".into(),
        Some(other) => other.into(),
    }
}

/// A trend key's readable name; a key the backend adds later is shown as it came.
fn trend_name(k: &str) -> &str {
    match k {
        "CANFI.VB" => "Battery", "CANFI.ECT" => "Coolant", "CANFI.KLAFAVE" => "Long-term fuel trim", "CANFI.ROLF" => "Oil life",
        "CAN_TPMS.FLAIR" => "Front left tire", "CAN_TPMS.FRAIR" => "Front right tire", "CAN_TPMS.RLAIR" => "Rear left tire", "CAN_TPMS.RRAIR" => "Rear right tire", o => o,
    }
}

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// A civil date from a count of Unix days (Howard Hinnant's algorithm): the era shifts do the leap centuries.
fn civil_from_days(z: i64) -> (i64, usize, usize) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097); // days into the era
    let yoe = (doe - doe / 1461 + doe / 36_524 - doe / 146_096) / 365; // years into the era
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // days into the year, counted from March 1
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (y + if m <= 2 { 1 } else { 0 }, m as usize, d as usize)
}

/// The machine's offset from UTC at that moment, in seconds, where the C library gives it (every platform but Windows).
fn local_offset(secs: i64) -> Option<i64> {
    #[cfg(unix)]
    // SAFETY: `localtime_r` writes only into the `tm` it is given, and the time it reads is a plain integer.
    unsafe {
        let t = secs as libc::time_t;
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&t, &mut tm).is_null() { None } else { Some(tm.tm_gmtoff as i64) }
    }
    #[cfg(not(unix))]
    { let _ = secs; None }
}

/// A Unix time as the person's clock shows it: "Nov 14, 2023, 10:13 PM", in the machine's local time, the same as the
/// reference snapshot card. Where the offset is not known the time is UTC and says so.
fn when(t: f64) -> String {
    match local_offset(t as i64) {
        Some(offset) => when_at(t, offset),
        None => format!("{} UTC", when_at(t, 0)),
    }
}

/// `when` for a given offset from UTC: month and day unpadded, a 12-hour clock.
fn when_at(t: f64, offset: i64) -> String {
    let secs = t as i64 + offset;
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    let rest = secs.rem_euclid(86_400);
    let (h, mi) = (rest / 3_600, rest % 3_600 / 60);
    let (h, am_pm) = if h == 0 || h == 12 { (12, if h == 0 { "AM" } else { "PM" }) } else if h < 12 { (h, "AM") } else { (h - 12, "PM") };
    format!("{} {}, {}, {}:{:02} {}", MONTHS[m - 1], d, y, h, mi, am_pm)
}

/// One audit line: time, system, function and what became of it, the line's texts as the egui wrote them with the
/// outcome's tone (0 normal, 2 fault; the ok-vs-muted distinction is drawing, so ok reads 0). An attempt row is
/// "started"; a result row carries the backend's outcome (or `complete`/`unknown` when it sent none); a hand-back row
/// says which hand-back it was. Nothing is invented for a missing field.
fn audit_row(r: &Value) -> Vec<(String, i64)> {
    let event = r["event"].as_str().unwrap_or("");
    let at = r["at"].as_f64().map_or("—".into(), when);
    let system = r["system"].as_str().filter(|s| !s.is_empty()).unwrap_or(if event == "handback" { "all" } else { "—" });
    let name = r["name"].as_str().or(r["key"].as_str()).or(r["action"].as_str()).unwrap_or("—");
    let (outcome, tone) = match event {
        "attempt" => ("started".to_string(), 0),
        "result" => {
            let ok = r["ok"].as_bool() == Some(true);
            (r["outcome"].as_str().unwrap_or(if ok { "complete" } else { "unknown" }).to_string(), if ok { 0 } else { 2 })
        }
        "handback" => {
            let trigger = r["trigger"].as_str().unwrap_or("");
            let ok = r["ok"].as_bool() == Some(true);
            (format!("handed back ({trigger})"), if ok { 0 } else { 2 })
        }
        "" => ("—".to_string(), 0),
        other => (other.to_string(), 0),
    };
    vec![(at, 0), (system.to_string(), 0), (name.to_string(), 0), (outcome, tone)]
}

/// The History screen's state: what is loaded, what is selected, and what the last replies said. The lead owns the
/// sockets — it sends the calls (the call helpers below) and feeds the replies to `on_*`. `report` and `export` are
/// the app-level replies the egui read from the App, and `ecus` the systems the scan found; the model fills them.
#[derive(Default)]
pub struct History {
    started: bool, // `reload`/`ensure_started` manage it: the first look asks the backend for the car list again
    pub vins: Option<Vec<String>>,
    pub vin: Option<String>,
    data: Option<Vehicle>,
    open: Option<(u32, SessionDetail)>,
    pub error: Option<String>,
    pub maint: Option<crate::api::Maintenance>,
    pub audit: Option<Vec<Value>>,
    pub report: Option<crate::api::ReportReply>,
    pub export: Option<crate::api::ExportReply>,
    pub ecus: Vec<crate::api::Ecu>,
}

impl History {
    /// The backend calls the screen makes, as `(method, params)` pairs for the model to send. Removing a marker and
    /// exporting run from the app level, not from this screen.
    pub fn vehicles_call() -> (String, Value) { ("vehicles".into(), json!({})) }
    pub fn vehicle_call(vin: &str) -> (String, Value) { ("vehicle".into(), json!({ "vin": vin })) }
    pub fn session_call(id: u32) -> (String, Value) { ("session".into(), json!({ "id": id })) }
    pub fn maintenance_call(vin: &str) -> (String, Value) { ("maintenance".into(), json!({ "vin": vin })) }
    pub fn audit_call(vin: &str) -> (String, Value) { ("audit".into(), json!({ "vin": vin, "limit": 200 })) }

    /// Ask the backend again (each time the screen is opened): sessions recorded since the last look show up.
    pub fn reload(&mut self) { self.started = false; }

    /// The pump's first ask: the list of cars this machine knows — exactly once, until the next `reload`.
    pub fn ensure_started(&mut self) -> Option<(String, Value)> {
        if self.started { None } else { self.started = true; Some(Self::vehicles_call()) }
    }

    /// The vehicles reply: rows parsed leniently, a failed or unparsable reply an empty list (the screen still shows),
    /// and what `load` asks next — the vehicle, the maintenance record and the audit, in that order.
    pub fn on_vehicles(&mut self, r: crate::backend::Reply, current_vin: Option<&str>) -> Vec<(String, Value)> {
        let rows: Vec<VehicleRow> = r.ok().and_then(|v| crate::api::parse(v).ok()).unwrap_or_default();
        let vins: Vec<String> = rows.into_iter().map(|v| v.vin).collect();
        if self.vin.is_none() { self.vin = current_vin.map(String::from).or_else(|| vins.first().cloned()); }
        self.vins = Some(vins);
        self.load()
    }

    /// A VIN tab clicked: load the car's data and its maintenance record and audit — and never show the last car's
    /// record while the new one loads, and no open session card once a vehicle load starts.
    pub fn select(&mut self, vin: &str) -> Vec<(String, Value)> {
        self.vin = Some(vin.to_string());
        self.load()
    }

    fn load(&mut self) -> Vec<(String, Value)> {
        let vin = self.vin.clone();
        let mut calls = Vec::new();
        if let Some(v) = &vin {
            calls.push(Self::vehicle_call(v));
            self.open = None; // the vehicle load starts: no card from the previous car's session
        }
        self.maint = None;
        self.audit = None;
        calls.extend(self.reload_record());
        calls
    }

    pub fn on_vehicle(&mut self, r: crate::backend::Reply) {
        match r.and_then(|v| crate::api::parse::<Vehicle>(v).map_err(|e| e.to_string())) {
            Ok(d) => { self.data = Some(d); self.error = None; }
            Err(e) => { self.data = None; self.error = Some(format!("Could not load this vehicle’s history: {e}")); }
        }
    }

    pub fn on_maintenance(&mut self, r: crate::backend::Reply) {
        match r.and_then(|v| crate::api::parse::<crate::api::Maintenance>(v).map_err(|e| e.to_string())) {
            Ok(m) => self.maint = Some(m),
            Err(e) => self.error = Some(format!("Could not load the service record: {e}")),
        }
    }

    pub fn on_audit(&mut self, r: crate::backend::Reply) {
        match r.and_then(|v| crate::api::parse::<crate::api::AuditReply>(v).map_err(|e| e.to_string())) {
            Ok(a) => self.audit = Some(a.records),
            Err(e) => self.error = Some(format!("Could not load the write audit: {e}")),
        }
    }

    /// Reload the maintenance record and the audit for the VIN currently shown (after a freeze, a marker or a write).
    pub fn reload_record(&mut self) -> Vec<(String, Value)> {
        let Some(v) = self.vin.clone() else { return Vec::new() };
        vec![Self::maintenance_call(&v), Self::audit_call(&v)]
    }

    pub fn on_session(&mut self, id: u32, r: crate::backend::Reply) {
        match r.and_then(|v| crate::api::parse::<SessionDetail>(v).map_err(|e| e.to_string())) {
            Ok(d) => { self.open = Some((id, d)); self.error = None; }
            Err(e) => self.error = Some(format!("Could not load session {id}: {e}")),
        }
    }

    /// Everything the screen draws, in one JSON. The lead draws a card only where the value is not null (or the list
    /// not empty), as the egui did — and `loading` and `empty` are early returns: in those states the egui drew one
    /// muted line ("Loading…" for loading, the `empty` sentence for an empty list) and nothing else at all — no
    /// title, no cards, so every other key here is null.
    /// A code's system as the sidebar names it when the scan found that module; the module's HDS name otherwise.
    fn system_name(&self, system: &str) -> String {
        self.ecus.iter().find(|e| e.system == system)
            .map_or_else(|| system.to_string(), |e| crate::systems::info(&e.system_type, &e.display).name)
    }

    pub fn history_json(&self) -> Value {
        let Some(vins) = &self.vins else {
            return json!({ "loading": true, "empty": null, "title": null, "vins": [], "cars_count": null, "error": null,
                "trends": [], "sessions": null, "open": null, "codes": null, "markers": null, "baselines": null,
                "audit": null, "report": null, "backup": null });
        };
        if vins.is_empty() {
            return json!({ "loading": false, "empty": "No sessions recorded yet. Connect and scan: every session is saved per VIN.",
                "title": null, "vins": [], "cars_count": null, "error": null, "trends": [], "sessions": null, "open": null,
                "codes": null, "markers": null, "baselines": null, "audit": null, "report": null, "backup": null });
        }
        let vins: Vec<Value> = vins.iter().map(|v| json!({ "vin": v, "selected": self.vin.as_deref() == Some(v.as_str()) })).collect();
        let cars_count = self.data.as_ref().map(|d| {
            let n = d.sessions.len();
            format!("{n} session{}", if n == 1 { "" } else { "s" })
        });
        let trends: Vec<Value> = self.data.as_ref().map_or(Vec::new(), |d| d.trends.iter().map(|(k, rows)| {
            let (value, unit) = rows.last().map_or((0.0, String::new()), |t| convert(t.mean, &t.unit));
            json!({ "name": trend_name(k), "unit": unit, "value": fmt(value),
                "values": rows.iter().map(|t| json!(convert(t.mean, &t.unit).0)).collect::<Vec<_>>(),
                "note": format!("per-session mean, last {}", rows.len()) })
        }).collect());
        let sessions = self.data.as_ref().map(|d| json!({
            "headers": ["Started", "Length", "Distance", "Economy", "Codes", "Ended"],
            "selected": self.open.as_ref().and_then(|(id, _)| d.sessions.iter().find(|s| s.id == *id)).map(|s| s.id),
            "rows": d.sessions.iter().map(|s| {
                let tr = s.summary.as_ref().and_then(|x| x.trip.as_ref());
                json!({ "id": s.id, "at": s.started, "cells": [
                    { "text": when(s.started), "tone": 0 },
                    { "text": clock(s.summary.as_ref().map_or(0.0, |x| x.duration_s)), "tone": 0 },
                    { "text": tr.map_or("—".into(), |t| format!("{} mi", fmt(t.distance_km * crate::format::MI))), "tone": 0 },
                    { "text": tr.and_then(|t| t.fuel.economy.as_ref()).map_or("—".into(), |e| format!("{:.1} mpg", e.mpg_us)), "tone": 0 },
                    { "text": s.summary.as_ref().and_then(|x| x.codes.as_ref()).map_or("—".into(), |c| c.present.len().to_string()), "tone": 0 },
                    { "text": ended_text(s.end_reason.as_deref()), "tone": 0 } ] })
            }).collect::<Vec<_>>()
        }));
        let open = self.open.as_ref().map(|(id, det)| json!({
            "id": id, "title": format!("Session {id}"), "folder": det.folder,
            "summary": if det.summary_md.is_empty() { "Summary not written yet." } else { det.summary_md.as_str() } }));
        let codes = self.data.as_ref().filter(|d| !d.codes.is_empty()).map(|d| json!({
            "headers": ["Code", "System", "Description", "First seen", "Last seen", "Sessions", "Last"],
            "rows": d.codes.iter().map(|c| json!({ "cells": [
                { "text": crate::api::code_with_type(&c.code, c.failure_type as i32), "tone": 0 },
                { "text": self.system_name(&c.system), "tone": 0 },
                { "text": tidy(&c.description), "tone": 0 },
                { "text": when(c.first_seen), "tone": 0 },
                { "text": when(c.last_seen), "tone": 0 },
                { "text": c.sessions.to_string(), "tone": 0 },
                { "text": c.last_event, "tone": 0 } ],
                "first": c.first_seen, "last": c.last_seen })).collect::<Vec<_>>()
        }));
        let markers = self.maint.as_ref().map(|m| {
            let mut ms: Vec<&crate::api::Marker> = m.markers.iter().collect(); // newest first, as the egui sorted them
            ms.sort_by(|a, b| b.at.total_cmp(&a.at));
            let rows: Vec<Value> = ms.iter().map(|mk| json!({ "id": mk.id, "at": mk.at, "when": when(mk.at), "kind": kind_label(&mk.kind),
                "note": mk.note, "miles": mk.mileage_km.map_or_else(|| "—".into(), |km| format!("{} mi", fmt(km * crate::format::MI))) })).collect();
            json!({ "count": format!("{} recorded", m.markers.len()),
                "empty": m.markers.is_empty().then_some("No service markers yet. Add one from the health card on the overview."),
                "rows": rows })
        });
        let baselines = self.maint.as_ref().map(|m| {
            let mut bs: Vec<&crate::api::BaselineMeta> = m.baselines.iter().collect(); // immutable, newest first
            bs.sort_by(|a, b| b.created.total_cmp(&a.created));
            let rows: Vec<Value> = bs.iter().map(|b| {
                let integrity = if b.integrity.is_empty() { "—" } else if b.integrity == "ok" { "ok" } else { "modified" };
                json!({ "created": b.created, "saved": when(b.created), "label": if b.label.is_empty() { "—" } else { b.label.as_str() },
                    "systems": b.systems_count, "integrity": integrity,
                    "tone": if b.integrity.is_empty() || b.integrity == "ok" { 0 } else { 2 } })
            }).collect();
            json!({ "count": format!("{} stored", m.baselines.len()),
                "empty": m.baselines.is_empty().then_some("No reference snapshot yet. Save one on a good day to compare later changes against it."),
                "rows": rows })
        });
        let audit = self.audit.as_ref().map(|records| json!({
            "count": format!("{} record{}", records.len(), if records.len() == 1 { "" } else { "s" }),
            "empty": records.is_empty().then_some("No writes recorded for this car. Every write the backend runs is logged here; an actuator test is recorded when it is handed back."),
            "rows": records.iter().map(|r| { let row = audit_row(r);
                json!({ "time": row[0].0, "system": row[1].0, "function": row[2].0, "outcome": row[3].0, "tone": row[3].1 }) }).collect::<Vec<_>>()
        }));
        let report = self.report.as_ref().map(|r| json!({
            // the egui named the verdict through ui/health.rs `verdict_label`, which is not ported here: the raw
            // backend verdict string goes out and the QML screen names it
            "title": "Health report", "subtitle": "the last one written",
            "verdict": r.verdict, "verdict_text": crate::baseline::verdict_label(&r.verdict), "summary": r.summary, "path": r.path }));
        let backup = json!({
            "title": "Backup and export", "subtitle": "both files are written together",
            "note": "The backup is a complete archive of this machine’s data (a database snapshot, vehicles, audit, reports and the app’s settings) with a manifest and checksums. The JSON export is the same records in readable form. Restore by unpacking the archive with the app closed.",
            "buttons": ["Back up data", "Export JSON"],
            "paths": self.export.as_ref().map_or(Vec::new(), |e| [("Backup", &e.backup_path), ("JSON", &e.json_path)].iter()
                .filter(|(_, p)| !p.is_empty()).map(|(l, p)| json!({ "label": l, "path": p })).collect::<Vec<_>>()),
            "stats": self.export.as_ref().map(|e| format!("{} session{} · {} vehicle{} · {} bytes", e.sessions,
                if e.sessions == 1 { "" } else { "s" }, e.vehicles, if e.vehicles == 1 { "" } else { "s" }, e.bytes)) });
        json!({ "loading": false, "empty": null, "title": "History", "vins": vins, "cars_count": cars_count, "error": self.error,
            "trends": trends, "sessions": sessions, "open": open, "codes": codes, "markers": markers,
            "baselines": baselines, "audit": audit, "report": report, "backup": backup })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn history(vins: &[&str], data: &Value) -> History {
        let mut h = History::default();
        h.on_vehicles(Ok(json!(vins.iter().map(|v| json!({ "vin": v })).collect::<Vec<_>>())), None);
        h.on_vehicle(Ok(data.clone()));
        h
    }

    #[test]
    fn a_session_end_reads_in_words() {
        assert_eq!(ended_text(None), "running");
        assert_eq!(ended_text(Some("backend stopped")), "app closed");
        assert_eq!(ended_text(Some("disconnected")), "disconnected");
        assert_eq!(ended_text(Some("report")), "health report");
        assert_eq!(ended_text(Some("adapter lost")), "adapter lost", "an unknown reason is shown as it came");
    }

    #[test]
    fn a_timestamp_reads_month_day_year_and_a_12_hour_clock() {
        assert_eq!(when_at(0.0, 0), "Jan 1, 1970, 12:00 AM");
        assert_eq!(when_at(0.0, -5 * 3600), "Dec 31, 1969, 7:00 PM", "a zone west of Greenwich is the day before");
        assert_eq!(when_at(1_700_000_000.0, 0), "Nov 14, 2023, 10:13 PM");
    }

    #[test]
    fn opening_asks_for_the_cars_and_then_one_car_s_three_calls() {
        let mut h = History::default();
        let (m, p) = h.ensure_started().unwrap();
        assert_eq!((m.as_str(), p), ("vehicles", json!({})));
        assert_eq!(h.ensure_started(), None, "the first ask happens once, until the next reload");
        assert_eq!(h.vins, None);
        let rows = || json!([{ "vin": "VIN1" }, { "vin": "VIN2" }]);
        let calls = h.on_vehicles(Ok(rows()), None);
        assert_eq!(h.vin.as_deref(), Some("VIN1"), "the first car is shown");
        assert_eq!(calls, vec![("vehicle".into(), json!({ "vin": "VIN1" })), ("maintenance".into(), json!({ "vin": "VIN1" })),
            ("audit".into(), json!({ "vin": "VIN1", "limit": 200 }))]);
        assert!(h.maint.is_none() && h.audit.is_none());
        assert_eq!(h.select("VIN2"), vec![("vehicle".into(), json!({ "vin": "VIN2" })),
            ("maintenance".into(), json!({ "vin": "VIN2" })), ("audit".into(), json!({ "vin": "VIN2", "limit": 200 }))]);
        assert_eq!(h.vins.as_ref().map(|v| v.len()), Some(2), "the car list itself does not change");

        let mut h = History::default();
        h.on_vehicles(Ok(rows()), Some("VIN2"));
        assert_eq!(h.vin.as_deref(), Some("VIN2"), "the car the app already knows wins");
        // a failed or unparsable reply makes an empty car list, as the egui's unwrap_or_default did
        let mut h = History::default();
        let none = h.on_vehicles(Err("boom".into()), None);
        assert!(none.is_empty());
        assert_eq!(h.vins, Some(Vec::new()));
    }

    #[test]
    fn a_failed_vehicle_load_reads_the_vehicles_error_in_words() {
        let mut h = History::default();
        h.on_vehicle(Err("boom".into()));
        assert_eq!(h.error.as_deref(), Some("Could not load this vehicle’s history: boom"));
        assert!(h.data.is_none());
        h.on_vehicle(Ok(json!({ "sessions": [], "codes": [], "trends": {} })));
        assert!(h.error.is_none() && h.data.is_some(), "a good vehicle reply clears the error");
        h.on_maintenance(Err("boom".into()));
        assert_eq!(h.error.as_deref(), Some("Could not load the service record: boom"));
        h.on_maintenance(Ok(json!({ "vin": "VIN1" })));
        assert!(h.maint.is_some());
        assert!(h.error.is_some(), "the egui only clears the error on a vehicle or session reply");
        h.on_audit(Err("boom".into()));
        assert_eq!(h.error.as_deref(), Some("Could not load the write audit: boom"));
        h.on_audit(Ok(json!({ "records": [] })));
        assert!(h.audit.as_ref().is_some_and(|a| a.is_empty()));
        h.on_session(7, Err("no session 7".into()));
        assert_eq!(h.error.as_deref(), Some("Could not load session 7: no session 7"));
        h.on_session(7, Ok(json!({ "summary_md": "", "folder": "/sessions/7" })));
        assert_eq!(h.open.as_ref().map(|(id, _)| *id), Some(7));
        assert!(h.error.is_none(), "a good session reply clears the error");
        let mut h = History::default();
        h.on_vehicle(Ok(json!(5)));
        assert!(h.error.is_some(), "an unparsable reply counts as the same error");
    }

    #[test]
    fn an_audit_line_reads_time_system_function_and_outcome() {
        let mut h = History { vins: Some(vec!["VIN1".into()]), ..Default::default() }; // past the loading early return, the card is drawn
        h.audit = Some(vec![
            json!({ "event": "attempt", "at": 0.0, "system": "CANFI", "name": "E-VTC learn" }),
            json!({ "event": "result", "at": 1.0, "ok": true, "outcome": "complete" }),
            json!({ "event": "result", "at": 2.0, "ok": true }), // no outcome sent: the egui said "complete"
            json!({ "event": "result", "at": 3.0, "ok": false }), // no outcome sent: "unknown", a fault tone
            json!({ "event": "handback", "at": 4.0, "trigger": "report", "ok": true }),
            json!({ "event": "handback", "at": 5.0, "trigger": "tester", "ok": false }),
            json!({}), // nothing at all: every field falls back, nothing is invented
        ]);
        let j = h.history_json();
        let rows = j["audit"]["rows"].as_array().unwrap();
        assert_eq!(rows[0]["time"], when(0.0));
        assert_eq!(rows[0]["system"], "CANFI");
        assert_eq!(rows[0]["function"], "E-VTC learn");
        assert_eq!(rows[0]["outcome"], "started");
        assert_eq!(rows[0]["tone"].as_i64(), Some(0));
        assert_eq!(rows[1]["system"], "—");
        assert_eq!(rows[1]["function"], "—");
        assert_eq!(rows[1]["outcome"], "complete");
        assert_eq!(rows[1]["tone"].as_i64(), Some(0));
        assert_eq!(rows[2]["outcome"], "complete");
        assert_eq!(rows[3]["outcome"], "unknown");
        assert_eq!(rows[3]["tone"].as_i64(), Some(2));
        assert_eq!(rows[4]["system"], "all", "a hand-back is for every system");
        assert_eq!(rows[4]["outcome"], "handed back (report)");
        assert_eq!(rows[4]["tone"].as_i64(), Some(0));
        assert_eq!(rows[5]["tone"].as_i64(), Some(2), "a hand-back that failed is a fault");
        assert_eq!(rows[6]["time"], "—");
        assert_eq!(rows[6]["system"], "—");
        assert_eq!(rows[6]["function"], "—");
        assert_eq!(rows[6]["outcome"], "—");
        assert_eq!(rows[6]["tone"].as_i64(), Some(0));
    }

    #[test]
    fn summary_not_written_yet_says_so() {
        let mut h = History { vins: Some(vec!["VIN1".into()]), ..Default::default() }; // past the loading early return, the card is drawn
        assert!(h.history_json()["open"].is_null(), "no open card while nothing is open");
        h.open = Some((7, SessionDetail { summary_md: String::new(), folder: "/sessions/7".into() }));
        assert_eq!(h.history_json()["open"]["summary"], "Summary not written yet.", "the egui's placeholder, verbatim");
        h.open = Some((8, SessionDetail { summary_md: "Wrote the summary.".into(), folder: "/sessions/8".into() }));
        let j = h.history_json();
        assert_eq!(j["open"]["title"], "Session 8");
        assert_eq!(j["open"]["summary"], "Wrote the summary.", "the written summary itself otherwise");
    }

    #[test]
    fn the_card_carries_everything_a_data_full_history_draws() {
        let none = History::default();
        assert_eq!(none.history_json()["loading"], true, "no car list yet: only the Loading line, nothing else is drawn");
        assert!(none.history_json()["sessions"].is_null());
        let mut h = History::default();
        assert_eq!(h.on_vehicles(Ok(json!([{ "vin": "VIN1" }])), None).len(), 3, "vehicle, maintenance, audit");
        let empty = { let mut e = History::default(); e.on_vehicles(Ok(json!([])), None); e };
        let e = empty.history_json();
        assert_eq!(e["empty"], "No sessions recorded yet. Connect and scan: every session is saved per VIN.");
        assert!(e["sessions"].is_null() && e["backup"].is_null() && e["title"].is_null(), "the egui's early return: one line, nothing else");
        let data = json!({
            "sessions": [
                { "id": 7, "started": 1_700_000_000.0, "end_reason": "disconnected",
                  "summary": { "duration_s": 125.0, "trip": { "duration_s": 125.0, "distance_km": 10.0,
                    "fuel": { "economy": { "mpg_us": 31.46 } } }, "codes": { "present": [] } } },
                { "id": 8, "started": 1_760_000_000.0, "end_reason": "report" },
                { "id": 9, "started": 1_760_100_000.0 } ],
            "codes": [{ "system": "CANFI", "code": "P0420", "failure_type": 0x49, "first_seen": 0.0,
                "last_seen": 1_700_000_000.0, "sessions": 3, "last_event": "confirmed",
                "description": "CATALYST SYSTEM EFFICIENCY BELOW THRESHOLD" }],
            "trends": { "CANFI.VB": [{ "mean": 12.5, "unit": "V" }], "CAN_TPMS.FLAIR": [{ "mean": 235.0, "unit": "kPa" }] }
        });
        h = history(&["VIN1"], &Value::Null);
        h.on_vehicle(Ok(data));
        h.on_maintenance(Ok(json!({
            "vin": "VIN1",
            "markers": [{ "id": 3, "at": 1.7e9, "kind": "oil", "note": "5W-30 and filter", "mileage_km": 132000.0, "created": 1.7e9 }],
            "baselines": [
                { "id": 5, "created": 1_500_000_000.0, "label": "Before", "healthy": true, "sha256": "x",
                    "integrity": "sha mismatch", "systems_count": 2 },
                { "id": 7, "created": 1.7e9, "label": "After service", "healthy": true, "sha256": "ab",
                    "integrity": "ok", "systems_count": 2 }] })));
        h.on_audit(Ok(json!({ "records": [{ "event": "attempt", "at": 0.0, "system": "CANFI", "name": "E-VTC learn" }] })));
        h.report = Some(crate::api::parse(json!({ "verdict": "attention", "summary": "1 new code since the baseline",
            "path": "reports/a.html" })).unwrap());
        h.export = Some(crate::api::parse(json!({ "backup_path": "backups/a.tar.gz", "json_path": "exports/a.json",
            "sessions": 3, "vehicles": 1, "bytes": 123 })).unwrap());
        h.open = Some((7, SessionDetail { summary_md: "## Written for agents".into(), folder: "/sessions/7".into() }));
        let j = h.history_json();
        assert_eq!(j["loading"], false);
        assert!(j["empty"].is_null());
        assert_eq!(j["title"], "History");
        assert_eq!(j["open"]["title"], "Session 7");
        assert_eq!(j["open"]["summary"], "## Written for agents");
        assert_eq!(j["open"]["folder"], "/sessions/7");
        assert_eq!(j["vins"][0]["vin"], "VIN1");
        assert_eq!(j["vins"][0]["selected"], true);
        assert_eq!(j["cars_count"], "3 sessions");
        let trends = j["trends"].as_array().unwrap();
        assert_eq!([trends[0]["name"].as_str(), trends[0]["unit"].as_str(), trends[0]["value"].as_str(), trends[0]["note"].as_str()],
            [Some("Battery"), Some("V"), Some("12.5"), Some("per-session mean, last 1")]);
        assert_eq!(trends[1]["name"], "Front left tire", "the trend names");
        assert_eq!(trends[1]["value"], "34.1", "235 kPa is 34.1 psi");
        let s = &j["sessions"];
        assert_eq!(s["selected"], 7, "the open row is the selected one");
        let cells: Vec<Vec<&str>> = s["rows"].as_array().unwrap().iter()
            .map(|r| r["cells"].as_array().unwrap().iter().map(|x| x["text"].as_str().unwrap()).collect()).collect();
        let (at7, at8) = (when(1_700_000_000.0), when(1_760_000_000.0));
        assert_eq!(cells[0], [at7.as_str(), "2:05", "6.21 mi", "31.5 mpg", "0", "disconnected"]);
        assert_eq!(cells[1], [at8.as_str(), "0:00", "—", "—", "—", "health report"]);
        assert_eq!(cells[2][5], "running", "no end reason yet: it says running");
        let c = &j["codes"];
        let code_cells: Vec<&str> = c["rows"][0]["cells"].as_array().unwrap().iter().map(|x| x["text"].as_str().unwrap()).collect();
        assert_eq!(code_cells[0], "P0420-49", "the failure-type byte rides along");
        assert_eq!(code_cells[1], "CANFI", "a module the scan did not find keeps its HDS name");
        assert_eq!(code_cells[2], "Catalyst system efficiency below threshold");
        h.ecus = vec![crate::api::Ecu { address: 0x10, system_type: "PGM-FI".into(), system: "CANFI".into(), ..Default::default() }];
        assert_eq!(h.history_json()["codes"]["rows"][0]["cells"][1]["text"], "Engine", "a scanned module is named as the sidebar names it");
        assert_eq!(c["rows"][0]["first"], json!(0.0));
        assert_eq!(c["rows"][0]["last"], json!(1_700_000_000.0), "the raw seconds ride along for QML");
        assert_eq!(code_cells[3], when(0.0));
        let m = &j["markers"];
        assert_eq!(m["count"], "1 recorded");
        assert_eq!(m["rows"][0]["kind"], "Oil");
        assert_eq!(m["rows"][0]["miles"], "82,021 mi", "132000 km as miles");
        assert_eq!(m["rows"][0]["when"], when(1.7e9));
        assert_eq!(m["rows"][0]["note"], "5W-30 and filter");
        let b = &j["baselines"];
        assert_eq!(b["count"], "2 stored");
        assert_eq!(b["rows"][0]["label"], "After service");
        assert_eq!(b["rows"][0]["integrity"], "ok");
        assert_eq!(b["rows"][0]["tone"].as_i64(), Some(0));
        assert_eq!(b["rows"][0]["systems"].as_i64(), Some(2));
        assert_eq!(b["rows"][0]["saved"], when(1.7e9));
        assert_eq!(b["rows"][1]["label"], "Before");
        assert_eq!(b["rows"][1]["integrity"], "modified");
        assert_eq!(b["rows"][1]["tone"].as_i64(), Some(2), "newest first, and a changed snapshot reads modified at a fault tone");
        assert_eq!(j["audit"]["count"], "1 record");
        assert_eq!(j["report"]["verdict"], "attention", "the raw verdict string: the naming lives on the screen");
        assert_eq!(j["report"]["title"], "Health report");
        let p = j["backup"]["paths"].as_array().unwrap();
        assert_eq!((p[0]["label"].as_str(), p[0]["path"].as_str()), (Some("Backup"), Some("backups/a.tar.gz")));
        assert_eq!((p[1]["label"].as_str(), p[1]["path"].as_str()), (Some("JSON"), Some("exports/a.json")));
        assert_eq!(j["backup"]["stats"], "3 sessions · 1 vehicle · 123 bytes");
        assert_eq!(j["backup"]["note"], "The backup is a complete archive of this machine’s data (a database snapshot, vehicles, audit, reports and the app’s settings) with a manifest and checksums. The JSON export is the same records in readable form. Restore by unpacking the archive with the app closed.");

        // an empty record for the car: the cards still carry their count and their one empty line, as the egui drew them
        let mut e = history(&["VIN1"], &json!({ "sessions": [], "codes": [], "trends": {} }));
        e.on_maintenance(Ok(json!({ "vin": "VIN1" })));
        e.on_audit(Ok(json!({ "records": [] })));
        let e = e.history_json();
        assert!(e["codes"].is_null() && e["trends"].as_array().unwrap().is_empty(), "no codes, no trends card");
        assert!(e["sessions"]["rows"].as_array().unwrap().is_empty() && e["cars_count"] == "0 sessions");
        assert_eq!(e["markers"]["count"], "0 recorded");
        assert_eq!(e["markers"]["empty"], "No service markers yet. Add one from the health card on the overview.");
        assert_eq!(e["baselines"]["count"], "0 stored");
        assert_eq!(e["baselines"]["empty"], "No reference snapshot yet. Save one on a good day to compare later changes against it.");
        assert_eq!(e["audit"]["count"], "0 records");
        assert_eq!(e["audit"]["empty"], "No writes recorded for this car. Every write the backend runs is logged here; an actuator test is recorded when it is handed back.");
    }
}
