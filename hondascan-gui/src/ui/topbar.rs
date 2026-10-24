//! The header, and the title bar at once (the window has no native frame): adapter, connect / scan, recording, VIN,
//! link state, and the window's own minimise / maximise / close. Empty space drags the window; double click maximises.
use crate::app::{App, Link};
use crate::format::clock;
use crate::theme::*;
use crate::widgets::{button_x, Kind};
use eframe::egui::{self, vec2, Align, Color32, Layout, RichText, Sense, Stroke, Ui, ViewportCommand};

pub const PRESETS: [(&str, &str); 6] = [
    ("gd101-j2534", "GODIAG GD101 (J2534 mode, all modules)"), ("gd101", "GODIAG GD101 (ELM mode)"),
    ("elm", "ELM327 / OBDLink · auto"), ("socketcan:can0", "SocketCAN · can0"),
    ("sim:drive", "Simulated car · driving"), ("sim", "Simulated car · parked"),
];

/// The presets, then every serial port found (the GD101 offers both its modes), then "custom".
pub fn adapter_list(ports: &[crate::api::Port]) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = PRESETS.iter().map(|(s, l)| (s.to_string(), l.to_string())).collect();
    for p in ports {
        if p.gd101 {
            v.push((format!("gd101-j2534:{}", p.device), format!("GD101 J2534 · {}", p.device)));
            v.push((format!("elm:{}", p.device), format!("GD101 ELM · {}", p.device)));
        } else {
            v.push((format!("elm:{}", p.device), format!("{} · {}", if p.product.is_empty() { "Serial" } else { &p.product }, p.device)));
        }
    }
    v.push(("custom".into(), "Custom adapter spec…".into()));
    v
}

fn control(ui: &mut Ui, kind: u8) -> bool {
    let (r, resp) = ui.allocate_exact_size(vec2(32.0, 26.0), Sense::click());
    let hot = resp.hovered();
    let close = kind == 2;
    crate::widgets::key(ui, r, if hot && close { FAULT } else if hot { Color32::from_rgb(46, 46, 46) } else { SECONDARY }, resp.is_pointer_button_down_on());
    let (c, s) = (r.center(), Stroke::new(1.4, if hot && close { Color32::WHITE } else { FOREGROUND }));
    match kind {
        0 => { ui.painter().line_segment([c + vec2(-5.0, 4.0), c + vec2(5.0, 4.0)], s); }
        1 => { ui.painter().rect_stroke(egui::Rect::from_center_size(c, vec2(10.0, 9.0)), 0.0, s, egui::StrokeKind::Middle); }
        _ => {
            ui.painter().line_segment([c + vec2(-4.5, -4.5), c + vec2(4.5, 4.5)], s);
            ui.painter().line_segment([c + vec2(-4.5, 4.5), c + vec2(4.5, -4.5)], s);
        }
    }
    resp.clicked()
}

fn toggle_maximized(ctx: &egui::Context) {
    let m = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
    ctx.send_viewport_cmd(ViewportCommand::Maximized(!m));
}

pub fn show(ui: &mut Ui, app: &mut App) {
    let bar = ui.max_rect();
    let drag = ui.interact(bar, ui.id().with("drag"), Sense::click_and_drag());
    if drag.drag_started() { ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag); }
    if drag.double_clicked() { toggle_maximized(ui.ctx()); }
    ui.painter().line_segment([bar.left_bottom(), bar.right_bottom()], Stroke::new(1.0, Color32::from_black_alpha(150)));
    ui.painter().line_segment([bar.left_top(), bar.right_top()], Stroke::new(1.0, HI));

    ui.scope_builder(egui::UiBuilder::new().max_rect(bar.shrink2(vec2(12.0, 0.0))).layout(Layout::left_to_right(Align::Center)), |ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        if let Some(v) = &app.voxels {
            let (r, _) = ui.allocate_exact_size(vec2(50.0, 42.0), Sense::hover());
            crate::ui::logo::show(ui, v, r, ui.input(|i| i.time));
        }
        ui.label(RichText::new("hondascan").font(bold(14.0)));
        ui.add_space(6.0);
        let connected = app.link == Link::Connected;
        let combo = ui.scope(|ui| {
            if app.link != Link::Idle { ui.disable(); }
            ui.spacing_mut().interact_size.y = 28.0;
            ui.spacing_mut().button_padding = vec2(10.0, 6.0);
            ui.visuals_mut().widgets.inactive.bg_fill = BACKGROUND;
            ui.visuals_mut().widgets.hovered.bg_fill = Color32::from_rgb(20, 20, 20);
            ui.visuals_mut().widgets.inactive.weak_bg_fill = BACKGROUND;
            ui.visuals_mut().widgets.hovered.weak_bg_fill = Color32::from_rgb(20, 20, 20);
            let idx = app.choice.min(app.adapters.len() - 1);
            let shown = if app.link == Link::Idle { app.adapters[idx].1.clone() } else { app.adapters.iter().find(|(s, _)| *s == app.adapter).map_or(app.adapter.clone(), |p| p.1.clone()) };
            egui::ComboBox::from_id_salt("adapter").width(260.0).selected_text(shown).show_ui(ui, |ui| {
                let labels: Vec<String> = app.adapters.iter().map(|a| a.1.clone()).collect();
                for (i, label) in labels.iter().enumerate() { ui.selectable_value(&mut app.choice, i, label.as_str()); }
            }).response
        });
        crate::widgets::sunken(ui, combo.inner.rect);
        if app.link == Link::Idle && app.adapters[app.choice.min(app.adapters.len() - 1)].0 == "custom" {
            let r = ui.add(egui::TextEdit::singleline(&mut app.custom).font(mono(12.0)).desired_width(250.0).margin(vec2(8.0, 6.0))
                .hint_text("socketcan:can1, j2534:/path/lib.so, replay:file.log"));
            crate::widgets::sunken(ui, r.rect);
        }
        if connected {
            if button_x(ui, Some(egui_phosphor::regular::PLUGS), "Disconnect", Kind::Normal, true).clicked() { app.disconnect(); }
            let label = if app.scanning { "Scanning…" } else { "Rescan" };
            let f5 = !app.scanning && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F5));
            if button_x(ui, Some(egui_phosphor::regular::ARROWS_CLOCKWISE), label, Kind::Normal, !app.scanning).on_hover_text("Rescan (F5)").clicked() || f5 { app.scan(); }
        } else {
            let label = if app.link == Link::Connecting { "Connecting…" } else { "Connect" };
            if button_x(ui, Some(egui_phosphor::regular::PLUGS_CONNECTED), label, Kind::Primary, app.link == Link::Idle && !app.spec().is_empty()).clicked() { let spec = app.spec(); app.connect(&spec); }
        }

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if control(ui, 2) { ui.ctx().send_viewport_cmd(ViewportCommand::Close); }
            if control(ui, 1) { toggle_maximized(ui.ctx()); }
            if control(ui, 0) { ui.ctx().send_viewport_cmd(ViewportCommand::Minimized(true)); }
            ui.add_space(8.0);
            let (dot_color, text) = match app.link {
                Link::Connected => (OK, app.adapter.clone()), Link::Connecting => (WARN, "Connecting…".into()), Link::Idle => (MUTED_FG, "Not connected".into()),
            };
            ui.label(RichText::new(text).size(12.0).color(MUTED_FG));
            let (r, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
            ui.painter().circle_filled(r.center(), 3.0, dot_color);
            ui.add_space(8.0);
            if let Some(v) = &app.vin {
                ui.label(RichText::new(v).font(mono(12.0)));
                ui.label(RichText::new("VIN").font(bold(11.0)).color(MUTED_FG));
            }
            if let Some(s) = &app.status {
                if s.asleep { ui.label(RichText::new("Car off · waiting").size(12.0).color(MUTED_FG)); }
                if let (true, Some(t0)) = (s.recording, s.started) {
                    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64());
                    ui.label(RichText::new(clock(now - t0)).font(mono(12.0)).color(MUTED_FG));
                    ui.label(RichText::new("REC").font(bold(12.0)).color(FAULT));
                    let (r, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
                    let pulse = 0.5 + 0.5 * (ui.input(|i| i.time) * 4.0).sin() as f32 * 0.5 + 0.25;
                    ui.painter().circle_filled(r.center(), 3.0, FAULT.gamma_multiply(pulse));
                }
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::Port;

    #[test]
    fn ports_follow_the_presets_and_custom_is_last() {
        let l = adapter_list(&[Port { device: "/dev/gd101".into(), product: "GODIAG".into(), gd101: true }, Port { device: "/dev/ttyUSB0".into(), product: String::new(), gd101: false }]);
        let specs: Vec<&str> = l.iter().map(|a| a.0.as_str()).collect();
        assert_eq!(&specs[..2], ["gd101-j2534", "gd101"]);
        assert_eq!(&specs[6..], ["gd101-j2534:/dev/gd101", "elm:/dev/gd101", "elm:/dev/ttyUSB0", "custom"]);
        assert_eq!(l[8].1, "Serial · /dev/ttyUSB0");
    }
}
