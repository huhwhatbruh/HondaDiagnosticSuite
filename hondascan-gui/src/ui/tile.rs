//! One live signal as a gauge (units with a natural range) or a big number with its history (ported from
//! hondascan-app/src/components/LiveTile.tsx).
use crate::api::Catalog;
use crate::format::{convert, display};
use crate::live::Live;
use crate::names::{explain, friendly};
use crate::ranges::{scale_for, tone_of};
use crate::theme::*;
use crate::ui::charts::{gauge, sparkline};
use eframe::egui::{self, vec2, Align2, Color32, Rect, Ui};

pub const NOT_WHILE_STOPPED: &str = "not valid while stopped";

/// Draw the tile for `symbol` in `r`. Returns false when the system has no such signal.
pub fn show(ui: &mut Ui, r: Rect, live: &Live, cat: &Catalog, symbol: &str, as_gauge: bool, stopped: bool) -> bool {
    let Some((did, i, def)) = cat.find(symbol) else { return false };
    let named = friendly(def);
    let value = live.value(did, i);
    let hidden = stopped && def.invalid_stopped;
    let resp = ui.interact(r, ui.id().with(("tile", symbol)), egui::Sense::hover());
    let tip = [named.about.clone().unwrap_or_default(), named.raw.clone()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join("\n\n");
    let resp = if tip.is_empty() { resp } else { resp.on_hover_text(tip) };
    let scale = scale_for(&def.symbol, &def.unit);
    if let (true, Some((lo, hi, band)), false) = (as_gauge, scale, value.is_some_and(|v| v.is_string())) {
        let num = value.and_then(|v| v.as_f64()).filter(|_| !hidden);
        let (text, unit) = display(if hidden { None } else { value }, &def.unit);
        // the arc eases to each new reading, so data arriving ten times a second moves like an instrument's needle; the number is exact
        let frac = num.map(|n| crate::motion::settle(ui.ctx(), resp.id.with("arc"), ((convert(n, &def.unit).0 - lo) / (hi - lo)) as f32));
        gauge(ui, r.shrink2(vec2(14.0, 8.0)), frac, &text, &unit, &named.name, tone_of(band, value.and_then(|v| v.as_f64())));
        return true;
    }
    let p = ui.painter();
    p.text(r.left_top() + vec2(16.0, 18.0), Align2::LEFT_CENTER, &named.name, sans(12.0), MUTED_FG);
    let (text, unit) = display(if hidden { None } else { value }, &def.unit);
    let col = match value.and_then(|v| v.as_str()) { Some(s) if crate::format::state_on(s) => OK, Some(s) if crate::format::state_off(s) => MUTED_FG, _ => FOREGROUND };
    crate::widgets::amount(p, r.left_top() + vec2(16.0, 46.0), &text, bold(24.0), col, &unit, sans(12.0));
    let note = if hidden { Some(NOT_WHILE_STOPPED.to_string()) } else { explain(&def.symbol, value.and_then(|v| v.as_f64())) };
    if let Some(n) = note { p.text(r.left_top() + vec2(16.0, 70.0), Align2::LEFT_CENTER, n, sans(11.5), if hidden { MUTED_FG } else { Color32::from_white_alpha(200) }); }
    if let Some(h) = live.history(did, i) { sparkline(ui, Rect::from_min_size(egui::pos2(r.left() + 16.0, r.bottom() - 36.0), vec2(r.width() - 32.0, 26.0)), h); }
    true
}
