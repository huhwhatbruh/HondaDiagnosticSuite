//! The look (owner, 2026-10-24: black, not grey): solid black, matte panes a shade above it with a quiet outline, the
//! window drawn by the app (rounded, with its own Mac-style lights at the right end), one blue accent. Greys are neutral (no tint); blue is the only colour besides the status colours.
use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily, FontId, FontTweak, Stroke, TextStyle};
use eframe::epaint::text::{HintingTarget, SmoothHinting};
use std::collections::BTreeMap;

pub const BACKGROUND: Color32 = Color32::from_rgb(0, 0, 0);
pub const FOREGROUND: Color32 = Color32::from_rgb(235, 235, 235);
/// The top bar and the sidebar.
pub const PANEL: Color32 = Color32::from_rgb(12, 12, 12);
/// Panes and cards, a step above the bars (HIG Dark Mode: elevated layers are brighter than the base).
pub const CARD: Color32 = Color32::from_rgb(16, 16, 16);
/// Menus, popups, toasts: a step above the cards.
pub const POPOVER: Color32 = Color32::from_rgb(26, 26, 26);
/// A control's face, and an idle title bar.
pub const SECONDARY: Color32 = Color32::from_rgb(32, 32, 32);
/// A control's face under the pointer.
pub const HOVER: Color32 = Color32::from_rgb(46, 46, 46);
pub const MUTED: Color32 = Color32::from_rgb(22, 22, 22);
pub const MUTED_FG: Color32 = Color32::from_rgb(150, 150, 150);
pub const ACCENT: Color32 = Color32::from_rgb(18, 54, 106);
/// A primary button under the pointer: lighter than BLUE, with white text still at 4.5:1.
pub const BLUE_HOT: Color32 = Color32::from_rgb(40, 115, 205);
/// The edge of a container or control (pane, card, button, field): quiet, there to show the shape (HIG Layout:
/// group with "container shapes"), not to stand out.
pub const OUTLINE: Color32 = Color32::from_rgb(46, 46, 46);
/// A separator line inside a pane (table rows, the grid of tiles, under a title bar): quieter than OUTLINE.
pub const BORDER: Color32 = Color32::from_rgb(36, 36, 36);
pub const OK: Color32 = Color32::from_rgb(75, 198, 128);
pub const WARN: Color32 = Color32::from_rgb(237, 179, 69);
pub const FAULT: Color32 = Color32::from_rgb(245, 71, 72);
pub const BLUE: Color32 = Color32::from_rgb(46, 98, 170);
/// A bevel's light edge (22 % white) and dark edge (65 % black), as the first GUI had them.
pub const HI: Color32 = Color32::from_rgba_premultiplied(56, 56, 56, 56);
pub const LO: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 165);
pub const DATA: Color32 = Color32::from_rgb(215, 215, 215);

/// Corner radii (owner, 2026-10-24: only the window frame is round; everything inside it, notices and popups too, is square).
/// The app's own window; its panes sit well inside it, so the curves nest (HIG Toolbars: radii concentric with the container).
pub const RADIUS_FRAME: u8 = 12;
/// Menus, popups, toasts and notices.
pub const RADIUS_WINDOW: u8 = 0;
/// Cards, panes, windows and the sidebar's list: square.
pub const RADIUS_CARD: u8 = 0;
/// Buttons, fields, combo boxes: square.
pub const RADIUS_CONTROL: u8 = 0;

/// The shape that goes with a status colour, so the state never rests on colour alone (HIG Accessibility: "convey information
/// with more than color alone"); `None` for any other colour.
pub fn status_icon(tone: Color32) -> Option<&'static str> {
    use egui_phosphor::regular::{CHECK_CIRCLE, WARNING, X_CIRCLE};
    if tone == FAULT { Some(X_CIRCLE) } else if tone == WARN { Some(WARNING) } else if tone == OK { Some(CHECK_CIRCLE) } else { None }
}

pub fn sans(size: f32) -> FontId { FontId::new(size, FontFamily::Proportional) }
pub fn bold(size: f32) -> FontId { FontId::new(size, FontFamily::Name("bold".into())) }
pub fn mono(size: f32) -> FontId { FontId::new(size, FontFamily::Monospace) }
pub fn mono_bold(size: f32) -> FontId { FontId::new(size, FontFamily::Name("monobold".into())) }

pub fn install(ctx: &egui::Context) {
    let mut f = FontDefinitions::default();
    let faces = [
        ("noto", &include_bytes!("../assets/fonts/NotoSans-Regular.ttf")[..]),
        ("noto-bold", &include_bytes!("../assets/fonts/NotoSans-Bold.ttf")[..]),
        ("noto-mono", &include_bytes!("../assets/fonts/NotoSansMono-Regular.ttf")[..]),
        ("noto-mono-medium", &include_bytes!("../assets/fonts/NotoSansMono-Medium.ttf")[..]),
    ];
    // Sharp and evenly spaced (owner, 2026-10-24: the whole-pixel version was blocky and unevenly spaced). Light hinting
    // snaps stems and the baseline vertically only, so the letters keep the font's own proportions; the advances stay the
    // font's (linear metrics), and glyphs sit at quarter-pixel offsets (sub-pixel binning), so the gaps between letters are
    // equal instead of rounded to whole pixels, which was what made the spacing uneven.
    let tweak = FontTweak {
        subpixel_binning: Some(true),
        hinting: Some(true),
        hinting_target: HintingTarget::Smooth(SmoothHinting { light: true, symmetric_rendering: false, preserve_linear_metrics: true }),
        ..Default::default()
    };
    for (name, bytes) in faces {
        f.font_data.insert(name.into(), std::sync::Arc::new(FontData::from_static(bytes).tweak(tweak.clone())));
    }
    f.families.insert(FontFamily::Proportional, vec!["noto".into()]);
    f.families.insert(FontFamily::Monospace, vec!["noto-mono".into()]);
    f.families.insert(FontFamily::Name("bold".into()), vec!["noto-bold".into(), "noto".into()]);
    f.families.insert(FontFamily::Name("monobold".into()), vec!["noto-mono-medium".into(), "noto-mono".into()]);
    egui_phosphor::add_to_fonts(&mut f, egui_phosphor::Variant::Regular);
    // the icons also in the bold face (a status icon beside a bold headline)
    if let Some(bold) = f.families.get_mut(&FontFamily::Name("bold".into())) { bold.push("phosphor".into()); }
    ctx.set_fonts(f);

    let mut s = (*ctx.style_of(egui::Theme::Dark)).clone();
    s.text_styles = BTreeMap::from([
        (TextStyle::Small, sans(11.0)), (TextStyle::Body, sans(13.0)), (TextStyle::Button, sans(13.0)),
        (TextStyle::Heading, bold(18.0)), (TextStyle::Monospace, mono(12.5)),
    ]);
    s.animation_time = 0.10;   // egui's own hover/open fades, the same as motion::fade
    s.scroll_animation = egui::style::ScrollAnimation::new(1500.0, egui::Rangef::new(0.08, 0.22));   // wheel and scroll-to: quicker to arrive
    s.spacing.item_spacing = egui::vec2(8.0, 6.0);
    s.spacing.button_padding = egui::vec2(10.0, 5.0);
    // no scroll bars (owner's decision): wheel, touchpad and keys scroll; nothing is drawn, nothing takes space or catches the pointer
    s.spacing.scroll = egui::style::ScrollStyle {
        bar_width: 0.0, floating_width: 0.0, handle_min_length: 0.0, bar_inner_margin: 0.0,
        dormant_handle_opacity: 0.0, active_handle_opacity: 0.0, interact_handle_opacity: 0.0,
        dormant_background_opacity: 0.0, active_background_opacity: 0.0, interact_background_opacity: 0.0,
        ..egui::style::ScrollStyle::floating()
    };
    s.visuals = {
        let mut v = egui::Visuals::dark();
        v.panel_fill = BACKGROUND;
        v.window_fill = POPOVER;
        v.window_stroke = Stroke::new(1.0, OUTLINE);
        // what floats casts a soft shadow (HIG Color: "the virtual shadow cast by a raised object"); only matte depth, no highlight
        v.window_shadow = egui::epaint::Shadow { offset: [0, 10], blur: 28, spread: 0, color: Color32::from_black_alpha(170) };
        v.popup_shadow = egui::epaint::Shadow { offset: [0, 6], blur: 18, spread: 0, color: Color32::from_black_alpha(150) };
        v.override_text_color = Some(FOREGROUND);
        v.selection.bg_fill = BLUE;
        v.selection.stroke = Stroke::new(1.0, Color32::WHITE);
        // rounded, but not much (owner's call; the HIG's Toolbars page asks for radii concentric with the container): see RADIUS_*
        v.window_corner_radius = RADIUS_WINDOW.into();
        v.menu_corner_radius = RADIUS_WINDOW.into();
        for w in [&mut v.widgets.noninteractive, &mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
            w.corner_radius = RADIUS_CONTROL.into();
        }
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
        v.widgets.inactive.bg_fill = SECONDARY;
        v.widgets.hovered.bg_fill = HOVER;
        v.widgets.active.bg_fill = BLUE;
        v.widgets.inactive.bg_stroke = Stroke::new(1.0, OUTLINE);
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, OUTLINE);
        v
    };
    // The app is dark only. egui styles one slot per theme and follows the system's: with Windows (or Wine) in light mode the
    // stock light style took over, and every panel edge, text field and chart line drew in light-theme grey and white.
    ctx.set_theme(egui::ThemePreference::Dark);
    ctx.set_style_of(egui::Theme::Dark, s.clone());
    ctx.set_style_of(egui::Theme::Light, s);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WCAG relative luminance and contrast ratio.
    fn lum(c: Color32) -> f32 {
        let f = |v: u8| { let v = v as f32 / 255.0; if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) } };
        0.2126 * f(c.r()) + 0.7152 * f(c.g()) + 0.0722 * f(c.b())
    }
    fn ratio(a: Color32, b: Color32) -> f32 { let (x, y) = (lum(a), lum(b)); (x.max(y) + 0.05) / (x.min(y) + 0.05) }

    /// HIG Accessibility: text up to 17 pt needs 4.5:1 (WCAG AA) against its background, on every surface text sits on.
    #[test]
    fn text_colours_have_aa_contrast_on_every_surface() {
        let text = [("FOREGROUND", FOREGROUND), ("MUTED_FG", MUTED_FG), ("DATA", DATA), ("OK", OK), ("WARN", WARN), ("FAULT", FAULT)];
        let surfaces = [("BACKGROUND", BACKGROUND), ("PANEL", PANEL), ("CARD", CARD), ("POPOVER", POPOVER), ("MUTED", MUTED), ("SECONDARY", SECONDARY)];
        for (tn, t) in text {
            for (sn, s) in surfaces {
                let r = ratio(t, s);
                assert!(r >= 4.5, "{tn} on {sn} is {r:.2}:1, below 4.5:1");
            }
        }
        // a hovered control carries only its label, in FOREGROUND
        assert!(ratio(FOREGROUND, HOVER) >= 4.5);
        // white labels on the blue fills (primary button, its hover, the selected row, a window's title bar)
        for (sn, s) in [("BLUE", BLUE), ("BLUE_HOT", BLUE_HOT), ("ACCENT", ACCENT)] {
            let r = ratio(Color32::WHITE, s);
            assert!(r >= 4.5, "white on {sn} is {r:.2}:1, below 4.5:1");
        }
    }

    /// Edges are quiet but must still show. The floors (1.25:1 for an outline, 1.2:1 for a separator) are the owner's choice for
    /// this look ("the border need not be pronounced"), not a HIG or WCAG figure: the outline on every surface it bounds, a
    /// separator on the panes it divides, and the outline the stronger of the two.
    #[test]
    fn outlines_and_separators_show() {
        for (sn, s) in [("BACKGROUND", BACKGROUND), ("PANEL", PANEL), ("CARD", CARD), ("POPOVER", POPOVER), ("MUTED", MUTED)] {
            let r = ratio(OUTLINE, s);
            assert!(r >= 1.25, "OUTLINE on {sn} is {r:.2}:1, below 1.25:1");
        }
        for (sn, s) in [("PANEL", PANEL), ("CARD", CARD)] {
            let r = ratio(BORDER, s);
            assert!(r >= 1.2, "BORDER on {sn} is {r:.2}:1, below 1.2:1");
        }
        assert!(lum(BORDER) < lum(OUTLINE));
    }

    /// Windows and Wine report a light system theme; the app's style must still be the one in use (separator lines drew in egui's
    /// light grey, 190, and text fields in white, before the style was set for both themes).
    #[test]
    fn the_app_style_is_in_use_when_the_system_theme_is_light() {
        let ctx = egui::Context::default();
        install(&ctx);
        let input = egui::RawInput { system_theme: Some(egui::Theme::Light), ..Default::default() };
        let _ = ctx.run_ui(input, |_| {});
        assert_eq!(ctx.global_style().visuals.widgets.noninteractive.bg_stroke.color, BORDER);
        assert_eq!(ctx.global_style().visuals.panel_fill, BACKGROUND);
    }

    /// The wheel scrolls a tall area under the installed style (no scroll bars are drawn, and none must take the wheel with them).
    #[test]
    fn the_wheel_scrolls_with_the_theme_installed() {
        let ctx = egui::Context::default();
        install(&ctx);
        let mut offset = 0.0;
        for frame in 0..30 {
            let mut input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))), ..Default::default() };
            input.events.push(egui::Event::PointerMoved(egui::pos2(400.0, 300.0)));
            if frame == 5 {
                input.events.push(egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: egui::vec2(0.0, -200.0), phase: egui::TouchPhase::Move, modifiers: Default::default() });
            }
            let _ = ctx.run_ui(input, |ui| {
                crate::ui::frame::backdrop(ui.ctx());
                let out = egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| { ui.allocate_space(egui::vec2(100.0, 5000.0)); });
                offset = out.state.offset.y;
                crate::ui::frame::resize_edges(ui.ctx());
                crate::ui::frame::rim(ui.ctx());
            });
        }
        assert!(offset > 50.0, "the scroll offset is {offset} after a wheel turn of 200 px");
    }
}
