//! One system: what it is, its trouble codes, its live data and how it was identified (ported from
//! hondascan-app/src/components/SystemView.tsx).
use crate::api::{EcuInfo, Ecu};
use crate::app::{App, Health, View};
use crate::backend::Reply;
use crate::live::Live;
use crate::systems::{info, shared_with};
use crate::theme::*;
use crate::ui::{actuator, dtc_panel, live_panel::{self, LiveUi}, reference, snapshot::{self, Snap}, write};
use crate::widgets::window;
use eframe::egui::{self, vec2, RichText, Sense, Ui};
use serde_json::json;
use std::sync::mpsc::Receiver;

#[derive(Clone, Copy, PartialEq)]
pub enum Tab { Live, Codes, Tests, Writes, Data, About }

pub struct SysUi {
    pub key: String,
    pub tab: Tab,
    pub live: LiveUi,
    pub ref_query: String,
    pub freeze: Snap,
    pub readiness: Snap,
    pub asking: Option<bool>, // the clear-codes dialog (dtc_panel::show)
    pub asking_test: Option<actuator::Asking>, // the start-test dialog (actuator::show)
    pub asking_write: Option<write::Asking>, // the run-write dialog (write::show)
    pub info: Option<EcuInfo>,
    pub info_done: bool,
    info_rx: Option<Receiver<Reply>>,
}

impl SysUi {
    pub fn new(e: &Ecu, tab: Tab, be: &crate::backend::Backend) -> Self {
        let rx = be.call("ecu_info", json!({ "address": e.address, "system": e.system }));
        SysUi { key: e.key(), tab, live: LiveUi::new(Live::new(e.clone(), Vec::new())), ref_query: String::new(), freeze: Snap::default(), readiness: Snap::default(),
            asking: None, asking_test: None, asking_write: None, info: None, info_rx: Some(rx), info_done: false }
    }
    pub fn pump(&mut self) {
        self.freeze.pump();
        self.readiness.pump();
        if let Some(rx) = &self.info_rx {
            if let Ok(r) = rx.try_recv() { self.info = r.ok().and_then(|v| crate::api::parse(v).ok()); self.info_rx = None; self.info_done = true; }
        }
    }
}

fn badge(h: Health) -> &'static str {
    match h { Health::Fault => "Fault", Health::Pending => "Pending code", Health::Ok => "No codes", Health::Unknown => "Not read" }
}

pub fn show(ui: &mut Ui, app: &mut App, key: &str) {
    let Some(ecu) = app.ecus.iter().find(|e| e.key() == key).cloned() else { app.view = View::Overview; return };
    let i = info(&ecu.system_type, &ecu.display);
    let health = app.health(&ecu);
    let (label, col) = (badge(health), health.color());
    let n = app.dtcs.get(key).and_then(|d| d.dtcs.as_ref()).map_or(0, |d| d.len());
    let shared = shared_with(ecu.address, &ecu.system_type, &app.ecus);
    ui.horizontal(|ui| {
        ui.label(RichText::new(&i.name).font(bold(20.0)));
        let (r, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
        ui.painter().circle_filled(r.center(), 3.0, col);
        ui.label(RichText::new(label).font(bold(12.0)).color(col));
    });
    let others: Vec<String> = app.ecus.iter().filter(|x| x.address == ecu.address && x.system_type != ecu.system_type).map(|x| info(&x.system_type, &x.display).name.to_lowercase()).collect();
    let extra = if shared { format!(" Part of the powertrain computer, which also runs the {}.", others.join(" and ")) } else { String::new() };
    ui.label(RichText::new(format!("{}{extra}", i.blurb)).size(13.0).color(MUTED_FG));
    ui.add_space(8.0);

    // tabs
    let tab = app.sysui.as_ref().map_or(Tab::Live, |s| s.tab);
    let mut pick = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 22.0;
        let mut names = vec![(Tab::Live, "Live data".to_string()), (Tab::Codes, if n > 0 { format!("Trouble codes  {n}") } else { "Trouble codes".into() })];
        // The public release only reads (package.rs builds with --cfg hondascan_packaged): no Tests or Writes tab; the backend refuses them anyway.
        if !cfg!(hondascan_packaged) && !actuator::for_system(&app.act_tests, &ecu).is_empty() { names.push((Tab::Tests, "Tests".into())); }
        if !cfg!(hondascan_packaged) && !write::for_system(&app.writes, &ecu).is_empty() { names.push((Tab::Writes, "Writes".into())); }
        names.push((Tab::Data, "Data reference".into()));
        names.push((Tab::About, "Module".into()));
        let mut under = None;   // the selected tab's left, right and bottom
        for (t, name) in names {
            // sized in bold always, so choosing a tab does not nudge its neighbours
            let w = ui.painter().layout_no_wrap(name.clone(), bold(13.0), FOREGROUND).size().x;
            let (r, resp) = ui.allocate_exact_size(vec2(w, 26.0), Sense::click());
            let lit = crate::motion::fade(ui.ctx(), resp.id.with("lit"), t == tab || resp.hovered());
            ui.painter().text(r.left_top() + vec2(0.0, 4.0), egui::Align2::LEFT_TOP, &name, if t == tab { bold(13.0) } else { sans(13.0) }, crate::motion::mix(MUTED_FG, FOREGROUND, lit));
            if t == tab { under = Some((r.left(), r.right(), r.bottom() - 2.0)); }
            if resp.clicked() { pick = Some(t); }
        }
        // one underline that slides and resizes to the selected tab
        if let Some((l, r, y)) = under {
            let (l, r) = (crate::motion::settle(ui.ctx(), ui.id().with("tab_l"), l), crate::motion::settle(ui.ctx(), ui.id().with("tab_r"), r));
            ui.painter().rect_filled(egui::Rect::from_min_max(egui::pos2(l, y), egui::pos2(r, y + 2.0)), 0.0, BLUE);
        }
    });
    ui.separator();
    if let Some(t) = pick { if let Some(s) = &mut app.sysui { s.tab = t; } }
    let tab = app.sysui.as_ref().map_or(Tab::Live, |s| s.tab);
    ui.add_space(6.0);

    let cat = app.catalogs.get(&ecu.system).cloned();
    match tab {
        Tab::Live => {
            if ecu.system.is_empty() { ui.label(RichText::new("HDS has no live-data definitions for this module.").color(MUTED_FG)); }
            else if let (Some(c), Some(s)) = (&cat, &mut app.sysui) { live_panel::show(ui, &mut s.live, c, &ecu.system_type); }
            else { ui.label(RichText::new("Loading signal definitions…").color(MUTED_FG)); }
        }
        Tab::Codes => {
            let state = app.dtcs.get(key);
            let mut asking = app.sysui.as_ref().and_then(|s| s.asking);
            let act = dtc_panel::show(ui, &ecu, state, &i.name, &others, &mut asking);
            if let Some(s) = &mut app.sysui { s.asking = asking; }
            match act { Some(dtc_panel::Action::Read) => app.read_dtcs(&ecu), Some(dtc_panel::Action::Clear) => app.clear(&ecu), None => {} }
            ui.add_space(10.0);
            if let Some(s) = &mut app.sysui {
                snapshot::show(ui, &app.be, &ecu, cat.as_deref(), true, &mut s.freeze);
                ui.add_space(10.0);
                snapshot::show(ui, &app.be, &ecu, cat.as_deref(), false, &mut s.readiness);
                if let Some(st) = app.status.as_ref().and_then(|x| crate::ui::selftests::for_ecu(&x.self_tests, &ecu)) {
                    ui.add_space(10.0);
                    crate::ui::selftests::show(ui, st);
                }
            }
        }
        Tab::Tests => {
            let list: Vec<crate::api::ActTest> = actuator::for_system(&app.act_tests, &ecu).into_iter().cloned().collect();
            let refs: Vec<&crate::api::ActTest> = list.iter().collect();
            let mut asking = app.sysui.as_ref().and_then(|s| s.asking_test.clone());
            let act = actuator::show(ui, &ecu, &refs, app.act.as_ref(), &mut asking);
            if let Some(s) = &mut app.sysui { s.asking_test = asking; }
            match act {
                Some(actuator::Action::Start(key, v)) => if let Some(t) = list.iter().find(|t| t.key == key) { app.act_start(&ecu, t, v) },
                Some(actuator::Action::Stop) => app.act_stop(),
                None => {}
            }
        }
        Tab::Writes => {
            let list: Vec<crate::api::WriteFn> = write::for_system(&app.writes, &ecu).into_iter().cloned().collect();
            let refs: Vec<&crate::api::WriteFn> = list.iter().collect();
            let mut asking = app.sysui.as_ref().and_then(|s| s.asking_write.clone());
            let act = write::show(ui, &ecu, &refs, app.vin.as_deref(), &mut asking);
            if let Some(s) = &mut app.sysui { s.asking_write = asking; }
            if let Some(write::Action::Run(key)) = act {
                if let Some(w) = list.iter().find(|w| w.key == key) { app.write_start(&ecu, w); }
            }
            if let Some(r) = app.last_write.as_ref().filter(|r| list.iter().any(|w| w.key == r.write)) {
                ui.add_space(10.0);
                ui.label(RichText::new(format!("Last run: {} ({}).", if r.ok { "complete" } else { "did not confirm" }, write::result_text(r))).size(12.5).color(MUTED_FG));
            }
        }
        Tab::Data => if let Some(s) = &mut app.sysui { reference::show(ui, cat.as_deref(), &ecu.system, &mut s.ref_query) },
        Tab::About => about(ui, app, &ecu, shared, &others),
    }
}

fn about(ui: &mut Ui, app: &App, ecu: &Ecu, shared: bool, others: &[String]) {
    window(ui, "Module", "", false, |ui| {
        ui.add_space(8.0);
        let a = ecu.address;
        let row = |ui: &mut Ui, k: &str, v: String, m: bool| {
            ui.horizontal(|ui| {
                ui.add_space(14.0);
                ui.add_sized([180.0, 20.0], egui::Label::new(RichText::new(k).size(13.0).color(MUTED_FG)));
                ui.label(RichText::new(v).font(if m { mono(12.5) } else { sans(13.0) }));
            });
        };
        row(ui, "Diagnostic address", format!("0x{a:02X} · the module’s number on the diagnostic bus{}", if shared { format!(", shared with the {}", others.join(" and ")) } else { String::new() }), false);
        row(ui, "CAN ids", format!("request 0x{:08X} · reply 0x{:08X}", 0x18da00f1u32 | (a << 8), 0x18daf100u32 | a), true);
        row(ui, "HDS system type", ecu.system_type.clone(), false);
        row(ui, "Database system", if ecu.system.is_empty() { "—".into() } else { ecu.system.clone() }, true);
        row(ui, "Identified by", if ecu.rule.is_empty() { "—".into() } else { ecu.rule.clone() }, true);
        match app.sysui.as_ref() {
            Some(s) if !s.info_done => row(ui, "Unit identity", "Reading…".into(), false),
            Some(SysUi { info: Some(i), .. }) if i.ecu_id.is_some() || i.part_number.is_some() => {
                if let Some(m) = &i.ecu_model { row(ui, "Unit model", format!("{m} · matched in Honda’s ECU list. This is the unit’s calibration identity."), false); }
                if let Some(id) = &i.ecu_id { row(ui, "ECU ID", id.clone(), true); }
                if let Some(p) = &i.part_number { row(ui, "Part number", format!("{p} · as the unit reports it ({})", i.part_number_raw.clone().unwrap_or_default()), false); }
            }
            _ => row(ui, "Unit identity", "This module does not report one in a form HDS knows.".into(), false),
        }
        ui.add_space(8.0);
    });
}
