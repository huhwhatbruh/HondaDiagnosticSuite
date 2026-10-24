//! The look: dark, square, one blue accent, HDS's bevels redrawn for dark (see hondascan-app/src/index.css, the web
//! app this was ported from). Colours are the sRGB values of the oklch tokens there.
use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily, FontId, Stroke, TextStyle};
use std::collections::BTreeMap;

pub const BACKGROUND: Color32 = Color32::from_rgb(10, 11, 13);
pub const FOREGROUND: Color32 = Color32::from_rgb(235, 235, 235);
pub const PANEL: Color32 = Color32::from_rgb(18, 20, 23);
pub const CARD: Color32 = Color32::from_rgb(15, 17, 19);
pub const POPOVER: Color32 = Color32::from_rgb(22, 24, 28);
pub const SECONDARY: Color32 = Color32::from_rgb(34, 36, 40);
pub const MUTED: Color32 = Color32::from_rgb(25, 27, 30);
pub const MUTED_FG: Color32 = Color32::from_rgb(148, 153, 159);
pub const ACCENT: Color32 = Color32::from_rgb(18, 54, 106);
pub const BORDER: Color32 = Color32::from_rgb(53, 56, 61);
pub const OK: Color32 = Color32::from_rgb(75, 198, 128);
pub const WARN: Color32 = Color32::from_rgb(237, 179, 69);
pub const FAULT: Color32 = Color32::from_rgb(245, 71, 72);
pub const BLUE: Color32 = Color32::from_rgb(21, 108, 221);
pub const DATA: Color32 = Color32::from_rgb(211, 216, 222);
pub const TITLE_A: Color32 = Color32::from_rgb(0, 71, 162);
pub const TITLE_B: Color32 = Color32::from_rgb(18, 34, 57);
pub const HI: Color32 = Color32::from_rgba_premultiplied(56, 56, 56, 56); // bevel light edge, 22 % white
pub const LO: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 165); // bevel dark edge, 65 % black

pub fn sans(size: f32) -> FontId { FontId::new(size, FontFamily::Proportional) }
pub fn bold(size: f32) -> FontId { FontId::new(size, FontFamily::Name("bold".into())) }
pub fn mono(size: f32) -> FontId { FontId::new(size, FontFamily::Monospace) }
pub fn mono_bold(size: f32) -> FontId { FontId::new(size, FontFamily::Name("monobold".into())) }

pub fn install(ctx: &egui::Context) {
    let mut f = FontDefinitions::default();
    let faces = [
        ("geist", &include_bytes!("../assets/fonts/Geist-Regular.ttf")[..]),
        ("geist-semibold", &include_bytes!("../assets/fonts/Geist-SemiBold.ttf")[..]),
        ("geist-mono", &include_bytes!("../assets/fonts/GeistMono-Regular.ttf")[..]),
        ("geist-mono-medium", &include_bytes!("../assets/fonts/GeistMono-Medium.ttf")[..]),
    ];
    for (name, bytes) in faces {
        f.font_data.insert(name.into(), std::sync::Arc::new(FontData::from_static(bytes)));
    }
    f.families.insert(FontFamily::Proportional, vec!["geist".into()]);
    f.families.insert(FontFamily::Monospace, vec!["geist-mono".into()]);
    f.families.insert(FontFamily::Name("bold".into()), vec!["geist-semibold".into(), "geist".into()]);
    f.families.insert(FontFamily::Name("monobold".into()), vec!["geist-mono-medium".into(), "geist-mono".into()]);
    egui_phosphor::add_to_fonts(&mut f, egui_phosphor::Variant::Regular);
    ctx.set_fonts(f);

    let mut s = (*ctx.global_style()).clone();
    s.text_styles = BTreeMap::from([
        (TextStyle::Small, sans(11.0)), (TextStyle::Body, sans(13.0)), (TextStyle::Button, sans(13.0)),
        (TextStyle::Heading, bold(18.0)), (TextStyle::Monospace, mono(12.5)),
    ]);
    s.animation_time = 0.10;   // egui's own hover/open fades, the same as motion::fade
    s.scroll_animation = egui::style::ScrollAnimation::new(1500.0, egui::Rangef::new(0.08, 0.22));   // wheel and scroll-to: quicker to arrive
    s.spacing.item_spacing = egui::vec2(8.0, 6.0);
    s.spacing.button_padding = egui::vec2(10.0, 5.0);
    s.visuals = {
        let mut v = egui::Visuals::dark();
        v.panel_fill = BACKGROUND;
        v.window_fill = POPOVER;
        v.window_stroke = Stroke::new(1.0, Color32::BLACK);
        v.override_text_color = Some(FOREGROUND);
        v.selection.bg_fill = BLUE;
        v.selection.stroke = Stroke::new(1.0, Color32::WHITE);
        v.window_corner_radius = 0.into();
        v.menu_corner_radius = 0.into();
        for w in [&mut v.widgets.noninteractive, &mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
            w.corner_radius = 0.into();
        }
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
        v.widgets.inactive.bg_fill = SECONDARY;
        v.widgets.hovered.bg_fill = Color32::from_rgb(46, 49, 54);
        v.widgets.active.bg_fill = BLUE;
        v.widgets.inactive.bg_stroke = Stroke::new(1.0, Color32::BLACK);
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, Color32::BLACK);
        v
    };
    ctx.set_global_style(s);
}
