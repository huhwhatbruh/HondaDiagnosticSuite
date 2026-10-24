//! What HDS calls freeze data and OBD status, read on demand: the values the control unit saved when it stored a
//! trouble code, and which emissions self-tests have finished. Read-only, one read of each group (ported from
//! hondascan-app/src/components/SnapshotPanel.tsx).
use crate::api::{Catalog, DidDef, LivePacket, SignalDef};
use crate::backend::{Backend, Reply};
use crate::format::display;
use crate::names::friendly;
use crate::theme::*;
use crate::widgets::{button_x, window, Kind};
use eframe::egui::{self, vec2, Align2, Color32, RichText, Stroke, Ui};
use serde_json::{json, Value};
use std::sync::mpsc::Receiver;

#[derive(Default)]
pub struct Snap { pub rows: Option<Vec<(SignalDef, Value)>>, pub errors: Vec<String>, rx: Option<Receiver<Reply>>, groups: Vec<DidDef> }

impl Snap {
    pub fn pump(&mut self) {
        let Some(rx) = &self.rx else { return };
        let Ok(r) = rx.try_recv() else { return };
        self.rx = None;
        match r {
            Ok(v) => {
                let p: LivePacket = crate::api::parse(v).unwrap_or_default();
                let (mut rows, mut errs) = (Vec::new(), Vec::new());
                for d in p.dids {
                    match (d.error, d.values) {
                        (Some(e), _) => errs.push(format!("{:04X}: {e}", d.did)),
                        (None, Some(vals)) => if let Some(g) = self.groups.iter().find(|g| g.did == d.did) { for (def, v) in g.signals.iter().zip(vals) { rows.push((def.clone(), v)); } },
                        _ => {}
                    }
                }
                self.rows = Some(rows);
                self.errors = errs;
            }
            Err(e) => self.errors = vec![e],
        }
    }
    pub fn busy(&self) -> bool { self.rx.is_some() }
}

pub fn show(ui: &mut Ui, be: &Backend, ecu: &crate::api::Ecu, cat: Option<&Catalog>, freeze: bool, snap: &mut Snap) {
    let groups: &[DidDef] = cat.map_or(&[], |c| if freeze { &c.freeze } else { &c.readiness });
    if cat.is_some() && groups.is_empty() { return; }
    let (title, what) = if freeze {
        ("Freeze frame", "The values the control unit saved at the moment it stored a trouble code, so you can see how the car was running when the fault happened (speed, engine load, temperatures). With no code stored these read zero or blank.")
    } else {
        ("Emissions readiness", "Which of the car’s self-tests have finished since the codes were last cleared. “Not complete” is normal right after clearing codes or disconnecting the battery; the tests finish by themselves during normal driving.")
    };
    window(ui, title, "", true, |ui| {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            ui.horizontal_wrapped(|ui| { ui.set_max_width(ui.available_width() - 120.0); ui.label(RichText::new(what).size(12.0).color(MUTED_FG)); });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(12.0);
                let label = if snap.busy() { "Reading…" } else if snap.rows.is_some() { "Read again" } else { "Read" };
                if button_x(ui, Some(egui_phosphor::regular::ARROWS_CLOCKWISE), label, Kind::Normal, !snap.busy()).clicked() && cat.is_some() {
                    snap.groups = groups.to_vec();
                    snap.rx = Some(be.call("live", json!({ "address": ecu.address, "system": ecu.system, "dids": groups.iter().map(|g| g.did).collect::<Vec<_>>() })));
                }
            });
        });
        if !snap.errors.is_empty() { ui.horizontal(|ui| { ui.add_space(12.0); ui.label(RichText::new(snap.errors.join(" · ")).size(12.0).color(FAULT)); }); }
        if let Some(rows) = &snap.rows {
            let w = ui.available_width();
            let cols = ((w / 240.0).floor() as usize).max(1);
            let cw = w / cols as f32;
            let (area, _) = ui.allocate_exact_size(vec2(w, rows.len().div_ceil(cols) as f32 * 48.0), egui::Sense::hover());
            for (i, (def, v)) in rows.iter().enumerate() {
                let r = egui::Rect::from_min_size(area.min + vec2((i % cols) as f32 * cw, (i / cols) as f32 * 48.0), vec2(cw, 48.0));
                let n = friendly(def);
                let (t, u) = display(Some(v), &def.unit);
                ui.painter().text(r.left_top() + vec2(12.0, 14.0), Align2::LEFT_CENTER, &n.name, sans(12.0), MUTED_FG);
                crate::widgets::amount(ui.painter(), r.left_top() + vec2(12.0, 34.0), &t, bold(14.0), FOREGROUND, &u, sans(12.0));
                ui.painter().rect_stroke(r, 0.0, Stroke::new(1.0, Color32::from_white_alpha(14)), egui::StrokeKind::Inside);
            }
        }
        ui.add_space(6.0);
    });
}
