//! Navigation: the overview, history, and every system found with its health at a glance.
use crate::app::{App, DtcState, View};
use crate::systems::{info, shared_with};
use crate::theme::*;
use crate::widgets::label;
use eframe::egui::{self, vec2, Align2, Color32, Sense, Stroke, Ui};

fn item(ui: &mut Ui, active: bool, icon: &str, title: &str, sub: Option<&str>, right: impl FnOnce(&egui::Painter, egui::Rect)) -> bool {
    let h = if sub.is_some() { 38.0 } else { 28.0 };
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::click());
    // the selection itself is one rect that glides between items (`glide`); here its position is recorded, and the text and hover fade
    if active { ui.data_mut(|d| d.insert_temp(egui::Id::new("sidebar_active"), r)); }
    let (on, hot) = (crate::motion::fade(ui.ctx(), resp.id.with("on"), active), crate::motion::fade(ui.ctx(), resp.id.with("hot"), resp.hovered()));
    let p = ui.painter();
    if hot > 0.0 { p.rect_filled(r, 0.0, SECONDARY.gamma_multiply(hot * (1.0 - on))); }
    let col = crate::motion::mix(FOREGROUND, Color32::WHITE, on);
    let y = if sub.is_some() { r.top() + 13.0 } else { r.center().y };
    p.text(egui::pos2(r.left() + 20.0, r.center().y), Align2::CENTER_CENTER, icon, sans(16.0), crate::motion::mix(MUTED_FG, Color32::WHITE, on));
    p.text(egui::pos2(r.left() + 38.0, y), Align2::LEFT_CENTER, title, sans(13.0), col);
    if let Some(s) = sub { p.text(egui::pos2(r.left() + 38.0, r.top() + 28.0), Align2::LEFT_CENTER, s, sans(11.0), MUTED_FG); }
    right(p, r);
    resp.clicked()
}

/// The selection: a blue bevelled rect that slides to the selected item (its top and bottom each settle on their own, so it
/// stretches a little on the way, as it would on a screen of glass). Drawn into two reserved spots in the paint order, one
/// below everything (for the Overview and History rows) and one just above the systems' well (which paints over the first).
fn glide(ui: &Ui, under: egui::layers::ShapeIdx, in_well: egui::layers::ShapeIdx, well: egui::Rect) {
    let Some(r) = ui.data(|d| d.get_temp::<egui::Rect>(egui::Id::new("sidebar_active"))) else { return };
    let top = crate::motion::settle(ui.ctx(), egui::Id::new("sb_top"), r.top());
    let bottom = crate::motion::settle(ui.ctx(), egui::Id::new("sb_bottom"), r.bottom());
    let g = egui::Rect::from_min_max(egui::pos2(r.left(), top), egui::pos2(r.right(), bottom));
    let shape = egui::Shape::Vec(vec![
        egui::Shape::rect_filled(g, 0.0, ACCENT),
        egui::Shape::line_segment([g.left_top() + vec2(0.5, 0.5), g.right_top() + vec2(-0.5, 0.5)], Stroke::new(1.0, HI)),
        egui::Shape::line_segment([g.left_top() + vec2(0.5, 0.5), g.left_bottom() + vec2(0.5, -0.5)], Stroke::new(1.0, HI)),
        egui::Shape::line_segment([g.left_bottom() + vec2(0.5, -0.5), g.right_bottom() + vec2(-0.5, -0.5)], Stroke::new(1.0, LO)),
        egui::Shape::line_segment([g.right_top() + vec2(-0.5, 0.5), g.right_bottom() + vec2(-0.5, -0.5)], Stroke::new(1.0, LO)),
    ]);
    ui.painter().set(under, shape.clone());
    ui.painter().with_clip_rect(well).set(in_well, shape);
}

pub fn show(ui: &mut Ui, app: &mut App) {
    ui.spacing_mut().item_spacing.y = 2.0;
    let under = ui.painter().add(egui::Shape::Noop);
    let ov = matches!(app.view, View::Overview);
    if item(ui, ov, egui_phosphor::regular::SQUARES_FOUR, "Overview", None, |_, _| {}) { app.view = View::Overview; }
    let hi = matches!(app.view, View::History);
    if item(ui, hi, egui_phosphor::regular::CLOCK_COUNTER_CLOCKWISE, "History", None, |_, _| {}) { app.view = View::History; app.history.reload(); }
    ui.add_space(14.0);
    ui.horizontal(|ui| {
        ui.add_space(4.0);
        label(ui, "Systems");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if app.scanning { ui.spinner(); } else if !app.ecus.is_empty() { ui.label(egui::RichText::new(app.ecus.len().to_string()).font(bold(11.0)).color(MUTED_FG)); }
        });
    });
    let list = ui.available_rect_before_wrap();
    ui.painter().rect_filled(list, 0.0, BACKGROUND);
    crate::widgets::sunken(ui, list);
    let in_well = ui.painter().add(egui::Shape::Noop);
    let ecus = app.ecus.clone();
    ui.scope_builder(egui::UiBuilder::new().max_rect(list.shrink(3.0)), |ui| egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        for e in &ecus {
            let i = info(&e.system_type, &e.display);
            let key = e.key();
            let active = matches!(&app.view, View::System(k) if *k == key);
            let health = app.health(e);
            let n = app.dtcs.get(&key).and_then(|d: &DtcState| d.dtcs.as_ref()).map_or(0, |d| d.len());
            let loading = app.dtcs.get(&key).is_some_and(|d| d.loading);
            let sub = shared_with(e.address, &e.system_type, &ecus).then_some("Powertrain computer");
            let clicked = item(ui, active, i.icon, &i.name, sub, |p, r| {
                let c = health.color();
                if n > 0 {
                    let rr = egui::Rect::from_center_size(r.right_center() - vec2(18.0, 0.0), vec2(20.0, 18.0));
                    p.rect_stroke(rr, 0.0, Stroke::new(1.0, c.gamma_multiply(0.6)), egui::StrokeKind::Inside);
                    p.text(rr.center(), Align2::CENTER_CENTER, n.to_string(), crate::theme::mono_bold(11.0), c);
                } else if !loading {
                    p.circle_filled(r.right_center() - vec2(16.0, 0.0), 3.0, c.gamma_multiply(0.8));
                }
            });
            if clicked { app.view = View::System(key); }
        }
        if ecus.is_empty() && !app.scanning {
            ui.add_space(24.0);
            ui.vertical_centered(|ui| ui.label(egui::RichText::new("Connect and scan to list the car’s systems.").size(12.0).color(MUTED_FG)));
        }
    }));
    glide(ui, under, in_well, list.shrink(3.0));
}
