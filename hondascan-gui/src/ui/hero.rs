//! The first thing on the Overview: one sentence about how the engine is doing, and the numbers that tell you
//! (engine speed, coolant, spark advance, knock). Ported from hondascan-app/src/components/EngineHero.tsx.
use crate::app::App;
use crate::format::display;
use crate::theme::*;
use crate::widgets::{card, label};
use eframe::egui::{self, vec2, RichText, Stroke, Ui};

const CYL: [&str; 4] = ["KNRTD1", "KNRTD2", "KNRTD3", "KNRTD4"];
// deg; hondascan.alerts' KNOCK_WARN and KNOCK_FAULT (a bar warning needs knock to be sustained or heavy, not one
// reading; here a single reading is only coloured)
const KNOCK_WARN: f64 = 2.0;
const KNOCK_FAULT: f64 = 6.0;

fn big(ui: &mut Ui, app: &App, name: &str, symbol: &str) {
    let (text, unit) = app.engine().zip(app.engine_cat()).and_then(|(l, c)| {
        c.find(symbol).map(|(did, i, def)| display(l.value(did, i), &def.unit))
    }).unwrap_or(("—".into(), String::new()));
    ui.vertical(|ui| {
        label(ui, name);
        let g = ui.painter().layout_no_wrap(text.clone(), bold(48.0), FOREGROUND);
        let (r, _) = ui.allocate_exact_size(vec2(g.size().x + 90.0, g.size().y + 2.0), egui::Sense::hover());
        crate::widgets::amount(ui.painter(), r.left_center(), &text, bold(48.0), FOREGROUND, &unit, sans(14.0));
    });
}

/// Knock: one figure per cylinder where the car has them, else the single figure it reports for the engine.
fn knock(ui: &mut Ui, app: &App) {
    let cat = app.engine_cat();
    let live = app.engine();
    let per_cyl = cat.zip(live).and_then(|(c, l)| c.find("KNRTD1").filter(|(did, _, _)| !l.errors.contains_key(did)));
    if per_cyl.is_none() { big(ui, app, "Knock retard", "KNRTD"); return; }
    let (c, l) = (cat.unwrap(), live.unwrap());
    ui.vertical(|ui| {
        label(ui, "Knock retard, by cylinder");
        ui.add_space(6.0);
        egui::Grid::new("cyl").num_columns(2).spacing([24.0, 8.0]).show(ui, |ui| {
            for (n, s) in CYL.iter().enumerate() {
                let v = c.find(s).and_then(|(did, i, _)| l.number(did, i));
                let hot = v.is_some_and(|x| x >= KNOCK_WARN);
                let hc = if v.is_some_and(|x| x >= KNOCK_FAULT) { FAULT } else { WARN };
                let (r, _) = ui.allocate_exact_size(vec2(70.0, 20.0), egui::Sense::hover());
                ui.painter().line_segment([r.left_top(), r.left_bottom()], Stroke::new(2.0, if hot { hc } else { BORDER }));
                let t = v.map_or("—".to_string(), |x| format!("{x:.1}°"));
                ui.painter().text(r.left_center() + vec2(10.0, 0.0), egui::Align2::LEFT_CENTER, (n + 1).to_string(), mono(12.0), MUTED_FG);
                ui.painter().text(r.left_center() + vec2(26.0, 0.0), egui::Align2::LEFT_CENTER, t,
                    if hot { mono_bold(14.0) } else { mono(14.0) }, if hot { hc } else { FOREGROUND });
                if n % 2 == 1 { ui.end_row(); }
            }
        });
    });
}

pub fn show(ui: &mut Ui, app: &App) {
    let alerts = app.status.as_ref().map(|s| s.alerts.as_slice()).unwrap_or(&[]);
    let codes: Vec<_> = app.dtcs.values().filter_map(|d| d.dtcs.as_ref()).flatten().collect();
    let confirmed = codes.iter().filter(|d| d.confirmed()).count();
    let reading = app.scanning || app.dtcs.values().any(|d| d.loading);
    let unread = crate::app::unread(&app.ecus, &app.dtcs).len();
    let rpm = app.engine().zip(app.engine_cat()).and_then(|(l, c)| c.find("NE").and_then(|(d, i, _)| l.number(d, i)));
    let running = rpm.is_some_and(|r| r > 400.0);
    let tone = match alerts.first() {
        Some(a) => if a.fault() { FAULT } else { WARN },
        None if confirmed > 0 => FAULT,
        None if !codes.is_empty() || unread > 0 => WARN,
        None => OK,
    };
    let (head, sub) = if let Some(a) = alerts.first() {
        (a.title.clone() + ".", a.text.clone() + &if alerts.len() > 1 { format!(" ({} more warning{} in the bar above.)", alerts.len() - 1, if alerts.len() > 2 { "s" } else { "" }) } else { String::new() })
    } else if reading { ("Reading your car…".into(), "Asking every system for its codes.".into()) }
    else if !codes.is_empty() {
        (format!("{} trouble code{} to look at.", codes.len(), if codes.len() > 1 { "s" } else { "" }), "Open one on the right for what Honda says can cause it.".into())
    } else if unread > 0 {
        (format!("{unread} system{} did not give {} codes.", if unread > 1 { "s" } else { "" }, if unread > 1 { "their" } else { "its" }),
            "Not known whether they hold codes: open a system to read again.".into())
    } else if running { ("Nothing flagged right now.".into(), "No active warnings or trouble codes — the bar above stays empty while all is well.".into()) }
    else { ("Ignition on, engine off.".into(), "Start it and these numbers come alive.".into()) };

    let w = ui.available_width();
    card(ui, egui::Margin { left: 27, right: 24, top: 22, bottom: 24 }, |ui| {
        ui.set_width(w - 53.0);
        let head = if tone == OK { head } else { format!("{}  {head}", status_icon(tone).unwrap_or_default()) };
        ui.label(RichText::new(head).font(bold(30.0)).color(if tone == OK { FOREGROUND } else { tone }));
        ui.label(RichText::new(sub).font(sans(14.0)).color(MUTED_FG));
        ui.add_space(18.0);
        ui.horizontal_top(|ui| {
            let col = (w - 53.0) / 4.0;
            ui.spacing_mut().item_spacing.x = 0.0;
            for (n, s) in [("Engine speed", "NE"), ("Coolant", "ECT"), ("Spark advance", "IG")] {
                ui.allocate_ui_with_layout(vec2(col, 80.0), egui::Layout::top_down(egui::Align::Min), |ui| { ui.set_min_size(vec2(col, 80.0)); big(ui, app, n, s) });
            }
            ui.allocate_ui_with_layout(vec2(col, 80.0), egui::Layout::top_down(egui::Align::Min), |ui| { ui.set_min_size(vec2(col, 80.0)); knock(ui, app) });
        });
    });
}
