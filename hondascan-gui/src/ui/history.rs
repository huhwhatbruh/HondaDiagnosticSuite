//! Everything recorded per VIN: sessions with their headline numbers, the trouble-code history, trends across sessions,
//! each session's summary as written for agents (summary.md), the service markers and frozen baselines, and the write
//! audit. Ported from hondascan-app/src/components/HistoryView.tsx.
use crate::api::{AuditReply, BaselineMeta, ExportReply, Maintenance, Trip};
use crate::app::App;
use crate::backend::{Backend, Reply};
use crate::format::{fmt, clock, tidy};
use crate::theme::*;
use crate::ui::charts::sparkline;
use crate::ui::maintenance::{kind_label, open_path};
use crate::widgets::{button_x, table, window, Kind};
use chrono::{Local, TimeZone};
use eframe::egui::{self, pos2, vec2, Align2, Color32, RichText, Rect, Sense, Stroke, Ui};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, VecDeque};
use std::sync::mpsc::Receiver;

#[derive(Deserialize, Default, Clone)]
#[serde(default)]
struct Summary { duration_s: f64, trip: Option<Trip>, codes: Option<Codes> }
#[derive(Deserialize, Default, Clone)]
#[serde(default)]
struct Codes { present: Vec<Value> }
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
#[derive(Deserialize, Default)]
#[serde(default)]
struct VehicleRow { vin: String }
#[derive(Deserialize, Default)]
#[serde(default)]
struct SessionDetail { summary_md: String, folder: String }

#[derive(Default)]
pub struct History {
    started: bool,
    vins: Option<Vec<String>>,
    vin: Option<String>,
    data: Option<Vehicle>,
    open: Option<(u32, SessionDetail)>,
    error: Option<String>,
    rx_vins: Option<Receiver<Reply>>,
    rx_vehicle: Option<Receiver<Reply>>,
    rx_session: Option<(u32, Receiver<Reply>)>,
    rx_maint: Option<Receiver<Reply>>,
    rx_audit: Option<Receiver<Reply>>,
    maint: Option<Maintenance>,
    audit: Option<Vec<Value>>,
}

fn when(t: f64) -> String {
    Local.timestamp_opt(t as i64, 0).single().map_or("—".into(), |d| d.format("%b %-d, %Y, %-I:%M %p").to_string())
}

fn trend_name(k: &str) -> &str {
    match k {
        "CANFI.VB" => "Battery", "CANFI.ECT" => "Coolant", "CANFI.KLAFAVE" => "Long-term fuel trim", "CANFI.ROLF" => "Oil life",
        "CAN_TPMS.FLAIR" => "Front left tire", "CAN_TPMS.FRAIR" => "Front right tire", "CAN_TPMS.RLAIR" => "Rear left tire", "CAN_TPMS.RRAIR" => "Rear right tire", o => o,
    }
}

impl History {
    /// Ask the backend again (each time the screen is opened): sessions recorded since the last look show up.
    pub fn reload(&mut self) { self.started = false; }

    /// Reload the maintenance record and the audit for the VIN currently shown (after a freeze, a marker or a write).
    pub fn reload_data(&mut self, be: &Backend) {
        let Some(v) = self.vin.clone() else { return };
        self.rx_maint = Some(be.call("maintenance", json!({ "vin": v })));
        self.rx_audit = Some(be.call("audit", json!({ "vin": v, "limit": 200 })));
    }

    fn pump(&mut self, be: &Backend, current_vin: Option<&str>) {
        if !self.started {
            self.started = true;
            self.rx_vins = Some(be.call("vehicles", json!({})));
        }
        if let Some(r) = self.rx_vins.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.rx_vins = None;
            let rows: Vec<VehicleRow> = r.ok().and_then(|v| crate::api::parse(v).ok()).unwrap_or_default();
            let vins: Vec<String> = rows.into_iter().map(|v| v.vin).collect();
            if self.vin.is_none() { self.vin = current_vin.map(String::from).or_else(|| vins.first().cloned()); }
            self.vins = Some(vins);
            self.load(be);
        }
        if let Some(r) = self.rx_vehicle.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.rx_vehicle = None;
            match r.and_then(|v| crate::api::parse(v).map_err(|e| e.to_string())) {
                Ok(d) => { self.data = Some(d); self.error = None; }
                Err(e) => { self.data = None; self.error = Some(format!("Could not load this vehicle’s history: {e}")); }
            }
        }
        if let Some(r) = self.rx_maint.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.rx_maint = None;
            match r.and_then(|v| crate::api::parse(v).map_err(|e| e.to_string())) {
                Ok(m) => self.maint = Some(m),
                Err(e) => self.error = Some(format!("Could not load the service record: {e}")),
            }
        }
        if let Some(r) = self.rx_audit.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.rx_audit = None;
            match r.and_then(|v| crate::api::parse::<AuditReply>(v).map_err(|e| e.to_string())) {
                Ok(a) => self.audit = Some(a.records),
                Err(e) => self.error = Some(format!("Could not load the write audit: {e}")),
            }
        }
        if let Some((id, rx)) = &self.rx_session {
            if let Ok(r) = rx.try_recv() {
                let id = *id;
                self.rx_session = None;
                match r.and_then(|v| crate::api::parse(v).map_err(|e| e.to_string())) {
                    Ok(d) => { self.open = Some((id, d)); self.error = None; }
                    Err(e) => self.error = Some(format!("Could not load session {id}: {e}")),
                }
            }
        }
    }

    fn load(&mut self, be: &Backend) {
        if let Some(v) = &self.vin { self.rx_vehicle = Some(be.call("vehicle", json!({ "vin": v }))); self.open = None; }
        // the maintenance record and audit belong to the selected VIN: never show the last car's while the new one loads
        self.maint = None;
        self.audit = None;
        self.reload_data(be);
    }
}

/// One audit line: time, system, function and what became of it. An attempt row is "started"; a result row carries the
/// backend's outcome; a hand-back row says which hand-back it was. Nothing is invented for a missing field.
fn audit_row(r: &Value) -> Vec<(String, Color32)> {
    let event = r["event"].as_str().unwrap_or("");
    let at = r["at"].as_f64().map_or("—".into(), when);
    let system = r["system"].as_str().filter(|s| !s.is_empty()).unwrap_or(if event == "handback" { "all" } else { "—" });
    let name = r["name"].as_str().or(r["key"].as_str()).or(r["action"].as_str()).unwrap_or("—");
    let (outcome, col) = match event {
        "attempt" => ("started".to_string(), MUTED_FG),
        "result" => {
            let ok = r["ok"].as_bool() == Some(true);
            (r["outcome"].as_str().unwrap_or(if ok { "complete" } else { "unknown" }).to_string(), if ok { OK } else { FAULT })
        }
        "handback" => {
            let trigger = r["trigger"].as_str().unwrap_or("");
            let ok = r["ok"].as_bool() == Some(true);
            (format!("handed back ({trigger})"), if ok { OK } else { FAULT })
        }
        "" => ("—".to_string(), MUTED_FG),
        other => (other.to_string(), MUTED_FG),
    };
    vec![(at, FOREGROUND), (system.to_string(), FOREGROUND), (name.to_string(), FOREGROUND), (outcome, col)]
}

pub fn show(ui: &mut Ui, app: &mut App) {
    let mut remove = None;
    let mut export = false;
    {
        let h = &mut app.history;
        h.pump(&app.be, app.vin.as_deref());
        let be = &app.be;
        let Some(vins) = h.vins.clone() else { ui.label(RichText::new("Loading…").color(MUTED_FG)); return };
        if vins.is_empty() {
            ui.add_space(30.0);
            ui.label(RichText::new("No sessions recorded yet. Connect and scan: every session is saved per VIN.").size(13.0).color(MUTED_FG));
            return;
        }
        ui.horizontal(|ui| {
            ui.label(RichText::new("History").font(bold(20.0)));
            for v in &vins {
                let sel = h.vin.as_deref() == Some(v);
                if ui.selectable_label(sel, RichText::new(v).font(mono(12.0))).clicked() { h.vin = Some(v.clone()); h.load(be); }
            }
            if let Some(d) = &h.data { ui.label(RichText::new(format!("{} session{}", d.sessions.len(), if d.sessions.len() == 1 { "" } else { "s" })).size(13.0).color(MUTED_FG)); }
        });
        ui.add_space(8.0);
        if let Some(e) = &h.error { ui.label(RichText::new(e).size(13.0).color(FAULT)); ui.add_space(8.0); }
        let Some(d) = h.data.clone() else { return };

        if !d.trends.is_empty() {
            let w = ui.available_width();
            let cols = ((w / 230.0).floor() as usize).max(1);
            let cw = w / cols as f32;
            let (area, _) = ui.allocate_exact_size(vec2(w, d.trends.len().div_ceil(cols) as f32 * 112.0), Sense::hover());
            ui.painter().rect_filled(area, 0.0, CARD);
            ui.painter().rect_stroke(area, 0.0, Stroke::new(1.0, Color32::BLACK), egui::StrokeKind::Outside);
            for (i, (k, rows)) in d.trends.iter().enumerate() {
                let r = Rect::from_min_size(area.min + vec2((i % cols) as f32 * cw, (i / cols) as f32 * 112.0), vec2(cw, 112.0));
                let unit = rows.last().map_or(String::new(), |t| crate::format::convert(0.0, &t.unit).1);
                let means: VecDeque<f32> = rows.iter().map(|t| crate::format::convert(t.mean, &t.unit).0 as f32).collect();
                ui.painter().text(r.left_top() + vec2(16.0, 18.0), Align2::LEFT_CENTER, format!("{}{}", trend_name(k), if unit.is_empty() { String::new() } else { format!(" ({unit})") }), sans(12.0), MUTED_FG);
                ui.painter().text(r.left_top() + vec2(16.0, 42.0), Align2::LEFT_CENTER, fmt(*means.back().unwrap_or(&0.0) as f64), bold(20.0), FOREGROUND);
                sparkline(ui, Rect::from_min_size(pos2(r.left() + 16.0, r.top() + 54.0), vec2(cw - 32.0, 32.0)), &means);
                ui.painter().text(r.left_top() + vec2(16.0, 98.0), Align2::LEFT_CENTER, format!("per-session mean, last {}", rows.len()), sans(11.0), MUTED_FG);
                ui.painter().line_segment([r.right_top(), r.right_bottom()], Stroke::new(1.0, Color32::from_white_alpha(18)));
            }
        }
        ui.add_space(12.0);

        let mut open_id = None;
        window(ui, "Sessions", "", false, |ui| {
            let rows: Vec<Vec<(String, Color32)>> = d.sessions.iter().map(|s| {
                let tr = s.summary.as_ref().and_then(|x| x.trip.as_ref());
                let t = |x: String| (x, FOREGROUND);
                vec![t(when(s.started)), t(clock(s.summary.as_ref().map_or(0.0, |x| x.duration_s))),
                    t(tr.map_or("—".into(), |t| format!("{} mi", fmt(t.distance_km * crate::format::MI)))),
                    t(tr.and_then(|t| t.fuel.economy.as_ref()).map_or("—".into(), |e| format!("{:.1} mpg", e.mpg_us))),
                    t(s.summary.as_ref().and_then(|x| x.codes.as_ref()).map_or("—".into(), |c| c.present.len().to_string())),
                    (ended_text(s.end_reason.as_deref()), MUTED_FG)]
            }).collect();
            let sel = h.open.as_ref().and_then(|(id, _)| d.sessions.iter().position(|s| s.id == *id));
            if let Some(i) = table(ui, "sessions", &["Started", "Length", "Distance", "Economy", "Codes", "Ended"], &[2.2, 1.0, 1.0, 1.0, 0.8, 1.2], &rows, sel) { open_id = Some(d.sessions[i].id); }
        });
        if let Some(id) = open_id { h.rx_session = Some((id, be.call("session", json!({ "id": id })))); }

        if let Some((id, det)) = &h.open {
            ui.add_space(12.0);
            window(ui, &format!("Session {id}"), &det.folder, false, |ui| {
                egui::ScrollArea::vertical().max_height(520.0).show(ui, |ui| {
                    ui.add_space(8.0);
                    ui.horizontal(|ui| { ui.add_space(14.0); ui.add(egui::Label::new(RichText::new(if det.summary_md.is_empty() { "Summary not written yet." } else { &det.summary_md }).font(mono(12.0))).selectable(true)); });
                });
            });
        }
        if !d.codes.is_empty() {
            ui.add_space(12.0);
            window(ui, "Trouble-code history", "", false, |ui| {
                let rows: Vec<Vec<(String, Color32)>> = d.codes.iter().map(|c| {
                    let full = crate::api::code_with_type(&c.code, c.failure_type as i32);
                    vec![(full, FOREGROUND), (c.system.clone(), FOREGROUND), (tidy(&c.description), MUTED_FG), (when(c.first_seen), FOREGROUND), (when(c.last_seen), FOREGROUND),
                        (c.sessions.to_string(), FOREGROUND), (c.last_event.clone(), MUTED_FG)]
                }).collect();
                table(ui, "codes", &["Code", "System", "Description", "First seen", "Last seen", "Sessions", "Last"], &[0.9, 1.0, 2.6, 1.6, 1.6, 0.8, 1.0], &rows, None);
            });
        }

        // Service markers: work done between sessions, written by hand, with the odometer reading in miles.
        if let Some(m) = &h.maint {
            ui.add_space(12.0);
            let count = m.markers.len();
            window(ui, "Service markers", &format!("{count} recorded"), false, |ui| {
                if m.markers.is_empty() {
                    ui.add_space(8.0);
                    ui.horizontal(|ui| { ui.add_space(14.0); ui.label(RichText::new("No service markers yet. Add one from the health card on the overview.").size(12.5).color(MUTED_FG)); });
                    ui.add_space(8.0);
                } else {
                    let mut markers: Vec<&crate::api::Marker> = m.markers.iter().collect();
                    markers.sort_by(|a, b| b.at.total_cmp(&a.at));
                    for mk in markers {
                        ui.horizontal(|ui| {
                            ui.add_space(14.0);
                            ui.add_sized([150.0, 20.0], egui::Label::new(RichText::new(when(mk.at)).size(12.0).color(MUTED_FG)));
                            ui.add_sized([80.0, 20.0], egui::Label::new(RichText::new(kind_label(&mk.kind)).size(13.0)));
                            let note_w = (ui.available_width() - 230.0).max(90.0);
                            ui.add_sized([note_w, 20.0], egui::Label::new(RichText::new(&mk.note).size(13.0)).truncate());
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.add_space(10.0);
                                if button_x(ui, None, "Remove", Kind::Normal, true).clicked() { remove = Some(mk.id); }
                                let miles = mk.mileage_km.map_or_else(|| "—".into(), |km| format!("{} mi", fmt(km * crate::format::MI)));
                                ui.label(RichText::new(miles).size(12.0).color(MUTED_FG));
                            });
                        });
                    }
                    ui.add_space(6.0);
                }
            });
        }

        // Reference snapshots (the backend's frozen baselines): immutable, newest first.
        if let Some(m) = &h.maint {
            ui.add_space(12.0);
            let count = m.baselines.len();
            window(ui, "Reference snapshots", &format!("{count} stored"), false, |ui| {
                if m.baselines.is_empty() {
                    ui.add_space(8.0);
                    ui.horizontal(|ui| { ui.add_space(14.0); ui.label(RichText::new("No reference snapshot yet. Save one on a good day to compare later changes against it.").size(12.5).color(MUTED_FG)); });
                    ui.add_space(8.0);
                } else {
                    let mut metas: Vec<&BaselineMeta> = m.baselines.iter().collect();
                    metas.sort_by(|a, b| b.created.total_cmp(&a.created));
                    let rows: Vec<Vec<(String, Color32)>> = metas.iter().map(|b| {
                        let integrity = if b.integrity.is_empty() { "—".to_string() } else if b.integrity == "ok" { "ok".to_string() } else { "modified".to_string() };
                        let col = if b.integrity.is_empty() || b.integrity == "ok" { MUTED_FG } else { FAULT };
                        vec![(when(b.created), FOREGROUND), (if b.label.is_empty() { "—".into() } else { b.label.clone() }, FOREGROUND),
                            (b.systems_count.to_string(), FOREGROUND), (integrity, col)]
                    }).collect();
                    table(ui, "baselines", &["Saved", "Label", "Systems", "Integrity"], &[1.9, 2.2, 0.7, 0.9], &rows, None);
                }
            });
        }

        // The write audit: every attempt, its result, and every hand-back, newest first.
        if let Some(records) = &h.audit {
            ui.add_space(12.0);
            window(ui, "Write audit", &format!("{} record{}", records.len(), if records.len() == 1 { "" } else { "s" }), false, |ui| {
                if records.is_empty() {
                    ui.add_space(8.0);
                    ui.horizontal(|ui| { ui.add_space(14.0); ui.label(RichText::new("No writes recorded for this car. Every write the backend runs is logged here; an actuator test is recorded when it is handed back.").size(12.5).color(MUTED_FG)); });
                    ui.add_space(8.0);
                } else {
                    let rows: Vec<Vec<(String, Color32)>> = records.iter().map(audit_row).collect();
                    table(ui, "audit-log", &["Time", "System", "Function", "Outcome"], &[1.7, 1.0, 2.2, 1.3], &rows, None);
                }
            });
        }

        // The last health report: where it is, and a way to open it from here.
        if let Some(r) = &app.last_report {
            ui.add_space(12.0);
            window(ui, "Health report", "the last one written", false, |ui| {
                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    ui.add_space(14.0);
                    ui.label(RichText::new(format!("Verdict: {}", crate::ui::health::verdict_label(&r.verdict))).size(13.0));
                    if !r.summary.is_empty() { ui.label(RichText::new(&r.summary).size(12.5).color(MUTED_FG)); }
                    if !r.path.is_empty() {
                        if button_x(ui, Some(egui_phosphor::regular::FILE_HTML), "Open", Kind::Normal, true).clicked() { open_path(&r.path); }
                        ui.label(RichText::new(&r.path).font(mono(10.5)).color(MUTED_FG));
                    }
                });
                ui.add_space(8.0);
            });
        }

        // Backup and export: one backend call writes the JSON export and the complete archive.
        ui.add_space(12.0);
        window(ui, "Backup and export", "both files are written together", false, |ui| {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.add_space(14.0);
                let text = "The backup is a complete archive of this machine’s data (a database snapshot, vehicles, audit, reports and the app’s settings) with a manifest and checksums. The JSON export is the same records in readable form. Restore by unpacking the archive with the app closed.";
                ui.horizontal_wrapped(|ui| { ui.set_max_width((ui.available_width() - 320.0).max(120.0)); ui.label(RichText::new(text).size(12.5).color(MUTED_FG)); });
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.add_space(14.0);
                if button_x(ui, Some(egui_phosphor::regular::DOWNLOAD_SIMPLE), "Back up data", Kind::Primary, true).clicked() { export = true; }
                if button_x(ui, Some(egui_phosphor::regular::BRACKETS_CURLY), "Export JSON", Kind::Normal, true).clicked() { export = true; }
            });
            if let Some(e) = &app.last_export { paths(ui, e); }
            ui.add_space(8.0);
        });
    }
    if let Some(id) = remove { app.remove_marker(id); }
    if export { app.export_data(); }
}

/// The paths the last export wrote, each with an Open button (desktop viewer; spawn errors are ignored).
fn paths(ui: &mut Ui, e: &ExportReply) {
    ui.add_space(4.0);
    for (label, path) in [("Backup", &e.backup_path), ("JSON", &e.json_path)] {
        if path.is_empty() { continue; }
        ui.horizontal(|ui| {
            ui.add_space(14.0);
            ui.add_sized([54.0, 20.0], egui::Label::new(RichText::new(label).size(11.0).color(MUTED_FG)));
            if button_x(ui, None, "Open", Kind::Normal, true).clicked() { open_path(path); }
            ui.label(RichText::new(path).font(mono(10.5)).color(MUTED_FG));
        });
    }
    ui.label(RichText::new(format!("{} session{} · {} vehicle{} · {} bytes", e.sessions, if e.sessions == 1 { "" } else { "s" },
        e.vehicles, if e.vehicles == 1 { "" } else { "s" }, e.bytes)).size(11.0).color(MUTED_FG));
}

#[cfg(test)]
mod tests {
    use super::ended_text;

    #[test]
    fn a_session_end_reads_in_words() {
        assert_eq!(ended_text(None), "running");
        assert_eq!(ended_text(Some("backend stopped")), "app closed");
        assert_eq!(ended_text(Some("disconnected")), "disconnected");
        assert_eq!(ended_text(Some("report")), "health report");
        assert_eq!(ended_text(Some("adapter lost")), "adapter lost", "an unknown reason is shown as it came");
    }
}
