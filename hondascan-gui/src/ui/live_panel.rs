//! Every live signal of a system, searchable and grouped by data identifier, with pinned gauges on top. Only open
//! groups and pinned signals are read, so narrowing the view speeds up the update rate. One group starts open (see
//! `default_open`), not all of them: over an ELM adapter every extra group costs a bus exchange per update (ported from
//! hondascan-app/src/components/LivePanel.tsx).
use crate::api::{Catalog, SignalDef};
use crate::format::{convert, display, fmt, state_off, state_on};
use crate::live::Live;
use crate::names::{explain, friendly};
use crate::theme::*;
use crate::ui::{charts::sparkline, tile};
use crate::widgets::{button, button_x, Kind};
use eframe::egui::{self, pos2, vec2, Align2, Color32, Rect, RichText, Sense, Stroke, Ui};
use std::collections::HashSet;

pub type Key = (u32, usize);

pub struct LiveUi {
    pub live: Live,
    pub query: String,
    pub opened: Option<HashSet<u32>>, // None: `default_open`
    pub running: bool,
    pub pins: Option<Vec<Key>>,       // None: the system's defaults
    pub expanded: HashSet<Key>,
}

impl LiveUi {
    pub fn new(live: Live) -> Self { LiveUi { live, query: String::new(), opened: None, running: true, pins: None, expanded: HashSet::new() } }

    fn default_pins(cat: &Catalog, system_type: &str) -> Vec<Key> {
        let syms: &[&str] = if system_type == "CVT" { &["NDR", "NDN", "TATF", "RATIO"] } else { &["NE", "VSP", "ECT", "VB"] };
        syms.iter().filter_map(|s| cat.find(s).map(|f| (f.0, f.1))).collect()
    }

}

/// What is open until someone opens or closes a group: while searching, every group with a match (so the results are
/// visible); otherwise the group of the system's first default gauge, which is polled for the gauge anyway (the first
/// group in the list is usually the ECU's identification bytes).
fn default_open(cat: &Catalog, system_type: &str, searching: bool, groups: &[(u32, Vec<(usize, &SignalDef)>)]) -> HashSet<u32> {
    // `HONDASCAN_OPEN_ALL=1`: every group open at start-up, the worst case for the frame cost (with `HONDASCAN_PROFILE`)
    static ALL: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if searching || *ALL.get_or_init(|| std::env::var_os("HONDASCAN_OPEN_ALL").is_some()) { return groups.iter().map(|g| g.0).collect(); }
    LiveUi::default_pins(cat, system_type).first().map(|k| k.0).or_else(|| cat.dids.iter().find(|d| d.signals.len() > 1).map(|d| d.did)).into_iter().collect()
}

fn matches(q: &str, did: u32, s: &SignalDef) -> bool {
    q.is_empty() || s.symbol.to_lowercase().contains(q) || s.description.to_lowercase().contains(q) || friendly(s).name.to_lowercase().contains(q)
        || format!("{did:04x}").contains(q)
}

pub fn show(ui: &mut Ui, st: &mut LiveUi, cat: &Catalog, system_type: &str) {
    let pins = st.pins.clone().unwrap_or_else(|| LiveUi::default_pins(cat, system_type));
    let q = st.query.trim().to_lowercase();
    let groups: Vec<(u32, Vec<(usize, &SignalDef)>)> = cat.dids.iter().filter_map(|d| {
        let v: Vec<_> = d.signals.iter().enumerate().filter(|(_, s)| matches(&q, d.did, s)).collect();
        (!v.is_empty()).then_some((d.did, v))
    }).collect();
    let open = st.opened.clone().unwrap_or_else(|| default_open(cat, system_type, q.chars().count() >= 2, &groups));
    let mut polled: Vec<u32> = pins.iter().map(|k| k.0).chain(groups.iter().filter(|(d, _)| open.contains(d)).map(|g| g.0)).collect();
    polled.sort_unstable();
    polled.dedup();
    st.live.dids = if st.running { polled.clone() } else { Vec::new() };
    let stopped = cat.find("VSP").is_some_and(|(d, i, _)| st.live.number(d, i) == Some(0.0));

    // pinned gauges
    let mut toggle_pin: Option<Key> = None;
    if !pins.is_empty() {
        let w = ui.available_width();
        let cols = ((w / 170.0).floor() as usize).max(1);
        let cw = w / cols as f32;
        let rows = pins.len().div_ceil(cols);
        let (area, _) = ui.allocate_exact_size(vec2(w, rows as f32 * (cw + 10.0)), Sense::hover());
        ui.painter().rect_filled(area, 0.0, CARD);
        ui.painter().rect_stroke(area, 0.0, Stroke::new(1.0, Color32::BLACK), egui::StrokeKind::Outside);
        for (n, k) in pins.iter().enumerate() {
            let Some(def) = cat.dids.iter().find(|d| d.did == k.0).and_then(|d| d.signals.get(k.1)) else { continue };
            let r = Rect::from_min_size(area.min + vec2((n % cols) as f32 * cw, (n / cols) as f32 * (cw + 10.0)), vec2(cw, cw + 10.0));
            tile::show(ui, r, &st.live, cat, &def.symbol, true, stopped);
            if ui.interact(Rect::from_min_size(r.right_top() - vec2(24.0, 0.0), vec2(24.0, 24.0)), ui.id().with(("unpin", k.0, k.1)), Sense::click()).on_hover_text("Unpin").clicked() { toggle_pin = Some(*k); }
        }
    }
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut st.query).hint_text(format!("{}  Search {} signals…", egui_phosphor::regular::MAGNIFYING_GLASS, cat.dids.iter().map(|d| d.signals.len()).sum::<usize>())).desired_width(320.0));
        if button(ui, if open.is_empty() { "Open all" } else { "Close all" }, Kind::Normal).clicked() {
            st.opened = Some(if open.is_empty() { groups.iter().map(|g| g.0).collect() } else { HashSet::new() });
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if button_x(ui, Some(if st.running { egui_phosphor::regular::PAUSE } else { egui_phosphor::regular::PLAY }), if st.running { "Pause" } else { "Resume" }, if st.running { Kind::Normal } else { Kind::Primary }, true).clicked() { st.running = !st.running; }
            if st.running && !polled.is_empty() { ui.label(RichText::new(format!("{} group{} · {} ms per update", polled.len(), if polled.len() == 1 { "" } else { "s" }, st.live.ms)).size(12.0).color(MUTED_FG)); }
        });
    });
    ui.add_space(6.0);

    let mut toggles: Vec<u32> = Vec::new();
    for (did, sigs) in &groups {
        let is_open = open.contains(did);
        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
        ui.painter().rect_filled(r, 0.0, crate::motion::mix(CARD, SECONDARY, crate::motion::fade(ui.ctx(), resp.id.with("hot"), resp.hovered())));
        ui.painter().rect_stroke(r, 0.0, Stroke::new(1.0, Color32::BLACK), egui::StrokeKind::Inside);
        let tri = if is_open { egui_phosphor::regular::CARET_DOWN } else { egui_phosphor::regular::CARET_RIGHT };
        ui.painter().text(r.left_center() + vec2(12.0, 0.0), Align2::LEFT_CENTER, tri, sans(13.0), MUTED_FG);
        let t = ui.painter().text(r.left_center() + vec2(30.0, 0.0), Align2::LEFT_CENTER, "Data group", bold(13.0), FOREGROUND);
        let t2 = ui.painter().text(pos2(t.right() + 6.0, r.center().y), Align2::LEFT_CENTER, format!("0x{did:04X}"), mono_bold(12.5), FOREGROUND);
        ui.painter().text(pos2(t2.right() + 10.0, r.center().y), Align2::LEFT_CENTER, format!("{} signals", sigs.len()), sans(12.0), MUTED_FG);
        if let Some(e) = st.live.errors.get(did) { ui.painter().text(r.right_center() - vec2(12.0, 0.0), Align2::RIGHT_CENTER, e, sans(12.0), FAULT); }
        if resp.clicked() { toggles.push(*did); }
        // a group opens and closes by growing and shrinking: its rows are laid out in full, then shown through a window of
        // `grown` of their last measured height (the first time, an estimate)
        let (grown, hid) = (crate::motion::open(ui.ctx(), r_id(&resp, *did), is_open), ui.id().with(("group_h", did)));
        if grown <= 0.0 { continue; }
        let full = ui.data(|d| d.get_temp::<f32>(hid)).unwrap_or(sigs.len() as f32 * 38.0);
        let w = ui.available_width();
        let window = if grown < 1.0 { Some(ui.allocate_space(vec2(w, full * grown)).1) } else { None };
        let mut body = ui.new_child(egui::UiBuilder::new().max_rect(match window { Some(win) => Rect::from_min_size(win.min, vec2(w, f32::INFINITY)), None => ui.available_rect_before_wrap() }).layout(*ui.layout()));
        if let Some(win) = window { body.set_clip_rect(win.intersect(ui.clip_rect())); }
        { let ui = &mut body;
        for (i, def) in sigs {
            let key = (*did, *i);
            let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 38.0), Sense::click());
            // the page scrolls, not the list, so every row of an open group is here each frame: one that is out of view takes its
            // place and is not laid out (a search or "Open all" can open hundreds of the engine's 759; each costs about 25 us a frame)
            let visible = ui.is_rect_visible(r);
            if !visible && !st.expanded.contains(&key) { continue; }
            let named = friendly(def);
            if visible {
            let hot = crate::motion::fade(ui.ctx(), resp.id.with("hot"), resp.hovered());
            if hot > 0.0 { ui.painter().rect_filled(r, 0.0, SECONDARY.gamma_multiply(hot)); }
            let pinned = pins.contains(&key);
            let star = Rect::from_center_size(r.left_center() + vec2(22.0, 0.0), vec2(22.0, 22.0));
            let sr = ui.interact(star, ui.id().with(("pin", did, i)), Sense::click()).on_hover_text(if pinned { "Unpin" } else { "Pin to the top" });
            ui.painter().text(star.center(), Align2::CENTER_CENTER, egui_phosphor::regular::STAR, sans(15.0), if pinned { WARN } else { Color32::from_white_alpha(70) });
            if sr.clicked() { toggle_pin = Some(key); }
            let value = st.live.value(*did, *i);
            let hidden = stopped && def.invalid_stopped;
            ui.painter().text(r.left_top() + vec2(44.0, 12.0), Align2::LEFT_CENTER, &named.name, sans(13.0), FOREGROUND);
            let sub = if hidden { Some(tile::NOT_WHILE_STOPPED.to_string()) } else { explain(&def.symbol, value.and_then(|v| v.as_f64())).or(named.about.clone()) };
            if let Some(sub) = sub { ui.painter().text(r.left_top() + vec2(44.0, 27.0), Align2::LEFT_CENTER, sub.chars().take(110).collect::<String>(), sans(11.0), MUTED_FG); }
            if let Some(h) = st.live.history(*did, *i) { sparkline(ui, Rect::from_min_size(pos2(r.right() - 330.0, r.top() + 6.0), vec2(150.0, 26.0)), h); }
            let (text, unit) = display(if hidden { None } else { value }, &def.unit);
            let col = match value.and_then(|v| v.as_str()) { Some(s) if state_on(s) => OK, Some(s) if state_off(s) => MUTED_FG, _ => FOREGROUND };
            let u = ui.painter().text(r.right_center() - vec2(14.0, 0.0), Align2::RIGHT_CENTER, &unit, sans(12.0), MUTED_FG);
            ui.painter().text(pos2(u.left() - if unit.is_empty() { -14.0 } else { 5.0 }, r.center().y), Align2::RIGHT_CENTER, &text, mono_bold(13.0), col);
            ui.painter().line_segment([r.left_bottom(), r.right_bottom()], Stroke::new(1.0, Color32::from_white_alpha(12)));
            if resp.on_hover_text(&named.raw).clicked() && !st.expanded.remove(&key) { st.expanded.insert(key); }
            }
            let reveal = crate::motion::open(ui.ctx(), ui.id().with(("detail", did, i)), st.expanded.contains(&key));
            if reveal > 0.0 { detail(ui, &st.live, key, def, &named.raw, reveal); }
        } }
        let measured = body.min_rect().height();
        if window.is_none() {
            let h = measured;
            ui.data_mut(|d| d.insert_temp(hid, h));
            ui.advance_cursor_after_rect(body.min_rect());
        }
    }
    if groups.is_empty() { ui.add_space(20.0); ui.vertical_centered(|ui| ui.label(RichText::new(format!("No signal matches “{}”.", st.query)).color(MUTED_FG))); }
    for d in toggles {
        let mut s = open.clone();
        if !s.remove(&d) { s.insert(d); }
        st.opened = Some(s);
    }
    if let Some(k) = toggle_pin {
        let mut p = pins.clone();
        if let Some(at) = p.iter().position(|x| *x == k) { p.remove(at); } else { p.push(k); }
        st.pins = Some(p);
    }
}

fn r_id(resp: &egui::Response, did: u32) -> egui::Id { resp.id.with(("open", did)) }

/// The expanded row for one signal, `reveal` (0..1) of the way open: it unrolls from the row above, its content clipped to what has opened.
fn detail(ui: &mut Ui, live: &Live, k: Key, def: &SignalDef, raw: &str, reveal: f32) {
    let hist = live.history(k.0, k.1);
    let xs: Vec<f64> = hist.map(|h| h.iter().map(|v| convert(*v as f64, &def.unit).0).collect()).unwrap_or_default();
    let u = convert(0.0, &def.unit).1;
    let help = def.help.clone().unwrap_or_default();
    let galley = ui.painter().layout(help.clone(), sans(12.0), MUTED_FG, ui.available_width() - 300.0);
    let h = 100.0 + if help.is_empty() { 0.0 } else { galley.size().y + 10.0 };
    let (shown, _) = ui.allocate_exact_size(vec2(ui.available_width(), h * reveal), Sense::hover());
    let r = Rect::from_min_size(shown.min, vec2(shown.width(), h));
    let ui = &mut ui.new_child(egui::UiBuilder::new().max_rect(shown));   // paints through a window of the part that has opened
    ui.set_clip_rect(shown.intersect(ui.clip_rect()));
    ui.set_opacity(crate::motion::ease(reveal));
    ui.painter().rect_filled(r, 0.0, BACKGROUND);
    if let Some(hh) = hist.filter(|h| h.len() > 1) { sparkline(ui, Rect::from_min_size(r.min + vec2(44.0, 10.0), vec2(r.width() - 340.0, 70.0)), hh); }
    else { ui.painter().text(r.min + vec2(44.0, 24.0), Align2::LEFT_CENTER, "No numeric history yet.", sans(12.0), MUTED_FG); }
    if !xs.is_empty() {
        let stats = [("Min", xs.iter().cloned().fold(f64::MAX, f64::min)), ("Average", xs.iter().sum::<f64>() / xs.len() as f64), ("Max", xs.iter().cloned().fold(f64::MIN, f64::max))];
        for (n, (l, v)) in stats.iter().enumerate() {
            let x = r.right() - 250.0 + n as f32 * 80.0;
            ui.painter().text(pos2(x, r.top() + 18.0), Align2::LEFT_CENTER, *l, sans(11.0), MUTED_FG);
            ui.painter().text(pos2(x, r.top() + 38.0), Align2::LEFT_CENTER, format!("{} {u}", fmt(*v)), bold(13.0), FOREGROUND);
        }
    }
    let y = r.top() + 88.0;
    if let Some(s) = &def.states { ui.painter().text(pos2(r.left() + 44.0, y - 8.0), Align2::LEFT_CENTER, format!("Possible states: {}", s.join(" · ")), sans(12.0), MUTED_FG); }
    if !help.is_empty() { ui.painter().galley(pos2(r.left() + 44.0, y), galley, MUTED_FG); }
    ui.painter().text(pos2(r.left() + 44.0, r.bottom() - 8.0), Align2::LEFT_CENTER, raw, mono(11.0), MUTED_FG);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::DidDef;

    fn sig(s: &str) -> SignalDef { SignalDef { symbol: s.into(), ..Default::default() } }
    fn cat() -> Catalog {
        Catalog { dids: vec![DidDef { did: 0x2600, signals: vec![sig("ECUID1"), sig("ECUID2")] }, DidDef { did: 0x2610, signals: vec![sig("NE"), sig("VSP")] }], ..Default::default() }
    }

    #[test]
    fn the_first_group_opens_where_the_gauges_are_not_at_the_identification_bytes() {
        let c = cat();
        assert_eq!(default_open(&c, "PGM-FI", false, &[]), HashSet::from([0x2610]));
        let no_gauges = Catalog { dids: vec![c.dids[0].clone()], ..Default::default() };
        assert_eq!(default_open(&no_gauges, "PGM-FI", false, &[]), HashSet::from([0x2600]));
    }

    #[test]
    fn searching_opens_every_group_with_a_match() {
        let c = cat();
        let hit = [(0x2600, vec![(0, &c.dids[0].signals[0])])];
        assert_eq!(default_open(&c, "PGM-FI", true, &hit), HashSet::from([0x2600]));
    }
}
