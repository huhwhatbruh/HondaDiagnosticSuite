//! The car's health at a glance: one row per found system with its trouble-code counts and readiness, and the car's
//! totals. Every row opens the system (the car page's deep-links). Under them sits the newest frozen healthy baseline,
//! what changed against it, and the three maintenance actions. Read-only: it talks to no one (the app loads the data).
use crate::app::{can_freeze, App, Health, Link, SysHealth, View};
use crate::theme::*;
use crate::ui::maintenance::{self, FreezeDialog, MarkerDialog};
use crate::widgets::{button_x, label, window, Kind};
use chrono::{Local, TimeZone};
use eframe::egui::{self, vec2, Align2, Color32, Frame, Margin, RichText, Sense, Ui};

/// A row's dot colour: the same rules as the car page's circles (an unread system is never green).
fn tone(h: &SysHealth) -> Health {
    if h.unknown { Health::Unknown }
    else if h.confirmed > 0 { Health::Fault }
    else if h.pending > 0 || h.stored > 0 { Health::Pending }
    else { Health::Ok }
}

/// The report verdict in words (rpc.rs `health_report`): what a person should take from it.
pub fn verdict_label(v: &str) -> &'static str {
    match v { "healthy" => "healthy", "attention" => "needs attention", "unknown" => "unknown (not everything could be read)", _ => "no verdict" }
}

fn verdict_color(v: &str) -> Color32 {
    match v { "healthy" => OK, "attention" => WARN, _ => MUTED_FG }
}

fn frozen_at(t: f64) -> String {
    Local.timestamp_opt(t as i64, 0).single().map_or("—".into(), |d| d.format("%b %-d, %Y, %-I:%M %p").to_string())
}

pub fn show(ui: &mut Ui, app: &mut App) {
    let s = app.health_summary();
    let reading = app.scanning || app.dtcs.values().any(|d| d.loading);
    let note = if s.systems.is_empty() { "nothing found yet".to_string() }
        else { format!("{} of {} systems read", s.systems.len() - s.unknown, s.systems.len()) };
    let mut open = None;
    window(ui, "Health", &note, false, |ui| {
        ui.add_space(8.0);
        // The car's totals. A system that was not read (or whose read failed) is never folded into a clean bill.
        let tone_all = if s.systems.is_empty() || s.unknown > 0 { WARN } else if s.faults > 0 { FAULT } else if s.pending > 0 { WARN } else { OK };
        let text = if s.systems.is_empty() { "No systems found".into() }
            else if s.unknown > 0 { format!("{} system{} not read", s.unknown, if s.unknown == 1 { "" } else { "s" }) }
            else if s.faults > 0 { format!("{} confirmed fault{}", s.faults, if s.faults == 1 { "" } else { "s" }) }
            else if s.pending > 0 { format!("{} pending code{}", s.pending, if s.pending == 1 { "" } else { "s" }) }
            else if reading { "Reading…".into() }
            else { "All clear".into() };
        ui.horizontal(|ui| {
            ui.add_space(14.0);
            ui.label(RichText::new(text).font(bold(20.0)).color(tone_all));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(14.0);
                let mut bits: Vec<String> = vec![];
                if s.faults > 0 { bits.push(format!("{} confirmed", s.faults)); }
                if s.pending > 0 { bits.push(format!("{} pending", s.pending)); }
                if s.stored > 0 { bits.push(format!("{} stored", s.stored)); }
                if s.not_run > 0 { bits.push(format!("{} self-test{} not run", s.not_run, if s.not_run == 1 { "" } else { "s" })); }
                if !bits.is_empty() { ui.label(RichText::new(bits.join(" · ")).size(12.0).color(MUTED_FG)); }
            });
        });
        ui.add_space(4.0);
        ui.separator();
        for h in &s.systems {
            let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
            if resp.hovered() { ui.painter().rect_filled(r, 4.0, MUTED); }
            let col = tone(h).color();
            let (y, x) = (r.center().y, r.left() + 14.0);
            ui.painter().circle_filled(egui::pos2(x, y), 3.0, col);
            ui.painter().text(egui::pos2(x + 14.0, y), Align2::LEFT_CENTER, &h.name, sans(13.0), FOREGROUND);
            let counts = if h.unknown { "not read".to_string() } else {
                let mut bits: Vec<String> = vec![];
                if h.confirmed > 0 { bits.push(format!("{} confirmed", h.confirmed)); }
                if h.pending > 0 { bits.push(format!("{} pending", h.pending)); }
                if h.stored > 0 { bits.push(format!("{} stored", h.stored)); }
                if bits.is_empty() { "no codes".into() } else { bits.join(" · ") }
            };
            let detail = if !h.unknown && h.not_run > 0 { format!("{} · {} self-test{} not run", counts, h.not_run, if h.not_run == 1 { "" } else { "s" }) } else { counts };
            ui.painter().text(r.right_center() + vec2(-14.0, 0.0), Align2::RIGHT_CENTER, &detail, sans(12.0), if h.unknown { MUTED_FG } else { col });
            let tip = if h.unknown { format!("{}: codes not known (open the system to read them)", h.name) }
                else { format!("{}: {}. Click to open.", h.name, detail) };
            if resp.on_hover_text(tip).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() { open = Some(h.key.clone()); }
            ui.painter().line_segment([r.left_bottom(), r.right_bottom()], egui::Stroke::new(1.0, Color32::from_white_alpha(14)));
        }
        ui.add_space(8.0);
        ui.separator();
        Frame::NONE.inner_margin(Margin::symmetric(14, 10)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            baseline_card(ui, app);
        });
    });
    if let Some(k) = open { app.view = View::System(k); }
    maintenance::show_freeze(ui, app);
    maintenance::show_marker(ui, app);
}

/// The newest frozen healthy baseline: when it was taken, its label and integrity, what changed since, and the three
/// actions. A freeze is offered only when `can_freeze` agrees, with the reason on the button's hover text.
fn baseline_card(ui: &mut Ui, app: &mut App) {
    ui.horizontal(|ui| {
        label(ui, "Reference snapshot");
        if app.link != Link::Connected {
            ui.label(RichText::new("connect and scan the car to save one").size(11.5).color(MUTED_FG));
        }
    });
    ui.add_space(2.0);
    if let Some(e) = &app.maintenance_err {
        ui.label(RichText::new(format!("Could not load the service record: {e}")).size(12.0).color(FAULT));
    }
    let latest = app.maintenance.as_ref().and_then(|m| m.latest.clone());
    let meta = latest.as_ref().and_then(|b| app.maintenance.as_ref()?.baselines.iter().find(|m| m.id == b.id).cloned());
    match &latest {
        None => {
            ui.label(RichText::new(if app.vin.is_none() { "No snapshot yet. A snapshot records today's trouble codes, readiness and self-tests, so later changes can be told apart from what was always there. It is this car's own reference." }
                else { "No snapshot yet for this car. Save one while it has no active trouble codes. It records codes and readiness, not how the engine performs." }).size(12.5).color(MUTED_FG));
        }
        Some(b) => {
            let systems = meta.as_ref().map_or(b.systems.len() as u32, |m| if m.systems_count > 0 { m.systems_count } else { b.systems.len() as u32 });
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(format!("Saved {}", frozen_at(b.created))).font(bold(14.0)));
                if !b.label.is_empty() { ui.label(RichText::new(format!("“{}”", b.label)).size(13.0)); }
                let stored = b.totals.stored;
                ui.label(RichText::new(format!("{systems} system{} · {} stored code{}", if systems == 1 { "" } else { "s" },
                    stored, if stored == 1 { "" } else { "s" })).size(12.0).color(MUTED_FG));
            });
            if let Some(m) = &meta {
                if !m.integrity.is_empty() && m.integrity != "ok" {
                    ui.label(RichText::new("The stored snapshot was modified: its contents no longer match the checksum taken when it was saved, so the comparison below cannot be trusted. Save a new one while the car has no active trouble codes.")
                        .size(12.0).color(FAULT));
                }
            }
            let d = crate::app::baseline_diff(b, &app.dtcs);
            ui.add_space(4.0);
            if d.new.is_empty() && d.still == 0 && d.resolved.is_empty() && d.readiness_changes.is_empty() {
                ui.label(RichText::new("No change since the snapshot.").size(12.5).color(MUTED_FG));
            } else {
                ui.horizontal_wrapped(|ui| {
                    if !d.new.is_empty() {
                        let tip = d.new.iter().map(|(s, c)| format!("{} {s}", c.full())).collect::<Vec<_>>().join("\n");
                        let col = if d.attention() { FAULT } else { WARN };
                        ui.label(RichText::new(format!("{} new{}", d.new.len(), if d.new_stored() == d.new.len() { " (stored)" } else { "" })).size(13.0).color(col)).on_hover_text(tip);
                        ui.label(RichText::new("·").size(13.0).color(MUTED_FG));
                    }
                    if d.still > 0 { ui.label(RichText::new(format!("{} still present", d.still)).size(13.0).color(MUTED_FG)); ui.label(RichText::new("·").size(13.0).color(MUTED_FG)); }
                    if !d.resolved.is_empty() {
                        let tip = d.resolved.iter().map(|(s, c)| format!("{} {s}", c.full())).collect::<Vec<_>>().join("\n");
                        ui.label(RichText::new(format!("{} resolved", d.resolved.len())).size(13.0).color(OK)).on_hover_text(tip);
                        ui.label(RichText::new("·").size(13.0).color(MUTED_FG));
                    }
                    if !d.readiness_changes.is_empty() {
                        ui.label(RichText::new(format!("{} readiness change{}", d.readiness_changes.len(), if d.readiness_changes.len() == 1 { "" } else { "s" }))
                            .size(13.0).color(WARN)).on_hover_text(d.readiness_changes.join("\n"));
                    }
                });
            }
        }
    }
    ui.add_space(8.0);
    let gate = can_freeze(&app.ecus, &app.dtcs, app.vin.as_deref());
    let vin = app.vin.clone();
    ui.horizontal_wrapped(|ui| {
        let r = button_x(ui, Some(egui_phosphor::regular::SNOWFLAKE), "Save reference snapshot", Kind::Primary, gate.is_ok());
        let r = if let Err(why) = &gate { r.on_hover_text(format!("Not yet: {why}.")) }
            else { r.on_hover_text("Stores today's trouble codes, readiness and self-tests as this car's own reference. It cannot be edited or removed later.") };
        if r.clicked() { app.freeze = Some(FreezeDialog::default()); }
        let r = button_x(ui, Some(egui_phosphor::regular::WRENCH), "Add service marker", Kind::Normal, vin.is_some());
        let r = if vin.is_some() { r.on_hover_text("A note about work done: kind, what was done, and how far the car had gone.") }
            else { r.on_hover_text("The car's VIN has not been read yet: scan it first.") };
        if r.clicked() { app.marker = Some(MarkerDialog::default()); }
        let r = button_x(ui, Some(egui_phosphor::regular::FILE_HTML), "Print health report", Kind::Normal, app.link == Link::Connected);
        let r = if app.link == Link::Connected { r.on_hover_text("Reads the car now, compares it with the snapshot and writes a printable report.") }
            else { r.on_hover_text("Connect to the car first: the report reads the codes live.") };
        if r.clicked() { app.print_report(); }
    });
    if let Some(r) = app.last_report.as_ref().filter(|r| r.vin().is_none() || r.vin() == app.vin.as_deref()) {
        if !r.summary.is_empty() || !r.path.is_empty() {
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(format!("Last report: {}", verdict_label(&r.verdict))).size(12.0).color(verdict_color(&r.verdict)));
                if !r.summary.is_empty() { ui.label(RichText::new(&r.summary).size(12.0).color(MUTED_FG)); }
                if !r.path.is_empty() {
                    if button_x(ui, Some(egui_phosphor::regular::FILE_HTML), "Open", Kind::Normal, true).clicked() { maintenance::open_path(&r.path); }
                    ui.label(RichText::new(&r.path).font(mono(10.5)).color(MUTED_FG));
                }
            });
        }
    }
}
