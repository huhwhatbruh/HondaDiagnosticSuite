//! The car at a glance. Ported piece by piece from hondascan-app/src/components/Overview.tsx.
use crate::app::{App, Link};
use crate::theme::*;
use crate::ui::{car, codes, drive, glance, health, hero, maps};
use crate::widgets::{button_x, window, Kind};
use eframe::egui::{self, vec2, Layout, RichText, Ui};

const SIDE: f32 = 360.0;
const GAP: f32 = 12.0;

pub fn show(ui: &mut Ui, app: &mut App) {
    if app.link != Link::Connected { return welcome(ui, app); }
    ui.spacing_mut().item_spacing.y = GAP;
    hero::show(ui, app);
    health::show(ui, app);
    let w = ui.available_width();
    let (left, h) = (w - SIDE - GAP, (w - SIDE - GAP) * 0.468 + 56.0);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = GAP;
        ui.allocate_ui_with_layout(vec2(left, h), Layout::top_down(egui::Align::Min), |ui| {
            window(ui, "Vehicle", "Select a system for its codes and live data", false, |ui| { ui.set_min_height(h - 30.0); ui.add_space(8.0); car::show(ui, app); });
        });
        ui.allocate_ui_with_layout(vec2(SIDE, h), Layout::top_down(egui::Align::Min), |ui| codes::show(ui, app, h));
    });
    drive::show(ui, app.status.as_ref());
    let mut sel = std::mem::take(&mut app.map_sel);
    maps::show(ui, app.status.as_ref().and_then(|s| s.maps.as_ref()), &mut sel);
    app.map_sel = sel;
    for i in 0..app.glances.len() { glance::show(ui, app, i); }
}

fn welcome(ui: &mut Ui, app: &mut App) {
    let steps = [
        ("Plug in", "Connect the adapter to the car’s diagnostic port (under the dashboard, driver’s side)."),
        ("Ignition on", "Turn the key to ON (II). The engine may stay off."),
        ("Connect", "Pick the adapter above and press Connect. hondascan finds every system by itself."),
    ];
    let w = ui.available_width().min(940.0);
    ui.vertical_centered(|ui| {
        ui.set_max_width(w);
        ui.add_space(8.0);
        ui.add_space(150.0);
        ui.add_space(6.0);
        ui.label(RichText::new("Let’s look at your car").font(bold(22.0)));
        ui.label(RichText::new("Honda diagnostics: every system, its trouble codes and all live data.").size(13.0).color(MUTED_FG));
        ui.add_space(14.0);
        let gap = 12.0;
        let cw = (w - 2.0 * gap) / 3.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (i, (t, d)) in steps.iter().enumerate() {
                ui.allocate_ui_with_layout(vec2(cw, 92.0), Layout::top_down(egui::Align::Min), |ui| {
                    crate::widgets::card(ui, egui::Margin::same(14), |ui| {
                        ui.set_width(cw - 30.0);
                        ui.set_min_height(64.0);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(format!("0{}", i + 1)).font(mono(12.0)).color(MUTED_FG));
                            ui.label(RichText::new(*t).font(bold(14.0)));
                        });
                        ui.label(RichText::new(*d).size(12.0).color(MUTED_FG));
                    });
                });
            }
        });
        ui.add_space(18.0);
        crate::ui::selfcheck::show(ui, app);
        ui.add_space(18.0);
        if button_x(ui, Some(egui_phosphor::regular::PLAY), "No car nearby? Try the simulated one", Kind::Normal, app.link == Link::Idle).clicked() { app.connect("sim:drive"); }
    });
}
