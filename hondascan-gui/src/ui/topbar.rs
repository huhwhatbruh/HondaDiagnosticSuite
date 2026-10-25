//! The header, and the title bar at once (the window has no native frame): adapter, connect / scan, recording, VIN, link state
//! and, at the right end, the window's lights. Empty space drags the window; double click zooms it.
use crate::app::{App, Link};
use crate::format::clock;
use crate::theme::*;
use crate::widgets::{button_x, Kind};
use eframe::egui::{self, pos2, vec2, Align, Color32, Layout, Rect, RichText, Sense, Stroke, Ui, ViewportCommand};

pub const PRESETS: [(&str, &str); 6] = [
    ("gd101-j2534", "GODIAG GD101 (J2534 mode, all modules)"), ("gd101", "GODIAG GD101 (ELM mode)"),
    ("elm", "ELM327 / OBDLink · auto"), ("socketcan:can0", "SocketCAN · can0"),
    ("sim:drive", "Simulated car · driving"), ("sim", "Simulated car · parked"),
];

/// The presets (no SocketCAN off Linux), then every serial port found (the GD101 offers both its modes), then "custom".
pub fn adapter_list(ports: &[crate::api::Port]) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = PRESETS.iter().filter(|(s, _)| cfg!(target_os = "linux") || !s.starts_with("socketcan")).map(|(s, l)| (s.to_string(), l.to_string())).collect();
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

/// Close, minimise and zoom as a Mac window has them (HIG Windows: a custom frame must look and behave like the system's): three
/// round lights at the right end, their glyphs shown while the pointer is over the group, grey while the window is not the focused one.
/// Returns which was clicked.
fn lights(ui: &mut Ui) -> [bool; 3] {
    const COLORS: [Color32; 3] = [Color32::from_rgb(255, 95, 87), Color32::from_rgb(254, 188, 46), Color32::from_rgb(40, 200, 64)];
    let focused = ui.ctx().input(|i| i.viewport().focused.unwrap_or(true));
    let (area, _) = ui.allocate_exact_size(vec2(60.0, 28.0), Sense::hover());
    let group = ui.rect_contains_pointer(area);
    let mut hit = [false; 3];
    for (n, color) in COLORS.into_iter().enumerate() {
        let c = pos2(area.left() + 10.0 + (2 - n) as f32 * 20.0, area.center().y);   // close outermost: zoom, minimise, close
        let resp = ui.interact(Rect::from_center_size(c, vec2(20.0, 20.0)), ui.id().with(("light", n)), Sense::click());
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, ["Close window", "Minimise window", "Zoom window"][n]));
        crate::widgets::focus_ring(ui, Rect::from_center_size(c, vec2(12.0, 12.0)), 255, &resp);
        let fill = if !focused && !group { SECONDARY } else if resp.is_pointer_button_down_on() { color.lerp_to_gamma(Color32::BLACK, 0.25) } else { color };
        ui.painter().circle_filled(c, 6.0, fill);
        if group {
            let g = Stroke::new(1.3, Color32::from_black_alpha(170));
            match n {
                0 => { ui.painter().line_segment([c + vec2(-2.2, -2.2), c + vec2(2.2, 2.2)], g); ui.painter().line_segment([c + vec2(-2.2, 2.2), c + vec2(2.2, -2.2)], g); }
                1 => { ui.painter().line_segment([c + vec2(-2.7, 0.0), c + vec2(2.7, 0.0)], g); }
                _ => { ui.painter().line_segment([c + vec2(-2.7, 0.0), c + vec2(2.7, 0.0)], g); ui.painter().line_segment([c + vec2(0.0, -2.7), c + vec2(0.0, 2.7)], g); }
            }
        }
        hit[n] = resp.clicked();
    }
    hit
}

fn toggle_maximized(ctx: &egui::Context) {
    let m = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
    ctx.send_viewport_cmd(ViewportCommand::Maximized(!m));
}

pub fn show(ui: &mut Ui, app: &mut App) {
    let bar = ui.max_rect();
    let drag = ui.interact(bar, ui.id().with("drag"), Sense::click_and_drag());
    if drag.drag_started() {
        ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
        // the window manager owns the pointer from here and the button release never reaches us: without this egui stays "dragging"
        // and every scroll area ignores the wheel until the next click
        ui.ctx().stop_dragging();
    }
    if drag.double_clicked() { toggle_maximized(ui.ctx()); }
    ui.painter().line_segment([bar.left_bottom(), bar.right_bottom()], Stroke::new(1.0, BORDER));

    ui.scope_builder(egui::UiBuilder::new().max_rect(bar.shrink2(vec2(12.0, 0.0))).layout(Layout::left_to_right(Align::Center)), |ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        ui.label(RichText::new("hondascan").font(bold(14.0)));
        ui.add_space(6.0);
        let connected = app.link == Link::Connected;
        ui.scope(|ui| {
            if app.link != Link::Idle { ui.disable(); }
            ui.spacing_mut().interact_size.y = 28.0;
            ui.spacing_mut().button_padding = vec2(10.0, 6.0);
            // the closed box is a sunken well (like the systems list), painted underneath (`slot`): the widget itself is clear
            let slot = ui.painter().add(egui::Shape::Noop);
            let v = &mut ui.visuals_mut().widgets;
            for w in [&mut v.inactive, &mut v.hovered, &mut v.active, &mut v.open] {
                w.bg_fill = Color32::TRANSPARENT; w.weak_bg_fill = Color32::TRANSPARENT; w.bg_stroke = Stroke::NONE;
            }
            let idx = app.choice.min(app.adapters.len() - 1);
            let shown = if app.link == Link::Idle { app.adapters[idx].1.clone() } else { app.adapters.iter().find(|(s, _)| *s == app.adapter).map_or(app.adapter.clone(), |p| p.1.clone()) };
            let combo = egui::ComboBox::from_id_salt("adapter").width(260.0).selected_text(shown).show_ui(ui, |ui| {
                let labels: Vec<String> = app.adapters.iter().map(|a| a.1.clone()).collect();
                for (i, label) in labels.iter().enumerate() { ui.selectable_value(&mut app.choice, i, label.as_str()); }
            });
            ui.painter().set(slot, crate::widgets::well_shapes(combo.response.rect));
        });
        if app.link == Link::Idle && app.adapters[app.choice.min(app.adapters.len() - 1)].0 == "custom" {
            ui.add(egui::TextEdit::singleline(&mut app.custom).font(mono(12.0)).desired_width(250.0).margin(vec2(8.0, 6.0))
                .hint_text(if cfg!(windows) { "j2534:NAME, slcan:COM5, replay:file.log" } else { "socketcan:can1, j2534:/path/lib.so, replay:file.log" }));
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
            let [close, minimise, zoom] = lights(ui);
                if close { ui.ctx().send_viewport_cmd(ViewportCommand::Close); }
                if minimise { ui.ctx().send_viewport_cmd(ViewportCommand::Minimized(true)); }
                if zoom { toggle_maximized(ui.ctx()); }
            ui.add_space(4.0);
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
                    let pulse = if crate::motion::reduced() { 1.0 } else { 0.5 + 0.5 * (ui.input(|i| i.time) * 4.0).sin() as f32 * 0.5 + 0.25 };
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
        assert_eq!(&specs[specs.len() - 4..], ["gd101-j2534:/dev/gd101", "elm:/dev/gd101", "elm:/dev/ttyUSB0", "custom"]);
        assert_eq!(l[l.len() - 2].1, "Serial · /dev/ttyUSB0");
    }
}
