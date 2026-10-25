//! The key numbers of a system type, as gauges and tiles, in a quiet "reference" window (ported from the Overview's
//! Glance). Each one owns a live poll of just the groups it shows.
use crate::app::App;
use crate::live::Live;
use crate::theme::*;
use crate::ui::tile;
use crate::widgets::window;
use eframe::egui::{self, vec2, Rect, Stroke, Ui};

pub struct Def { pub types: &'static [&'static str], pub title: &'static str, pub from: &'static str, pub hero: &'static [&'static str],
    pub gauges: &'static [&'static str], pub tiles: &'static [&'static str] }

// Engine: the headline numbers (hero) are polled with it, so they share one poll. Transmission: CVT fluid temperature,
// drive and driven pulley speeds; driven pulley pressure, start-clutch slip, pulley ratio, shift map (in words) and road
// slope (CANCVT 0x2301 / 0x2322, both read by the monitor anyway).
pub const DEFS: [Def; 2] = [
    Def { types: &["PGM-FI", "DIESEL"], title: "Engine", from: "engine computer", hero: &["NE", "ECT", "IG", "KNRTD", "KNRTD1", "KNRTD2", "KNRTD3", "KNRTD4"],
        gauges: &["VSP", "VB"], tiles: &["ECT", "IAT", "MAP", "APP", "FLEVELF"] },
    Def { types: &["CVT"], title: "Transmission", from: "transmission computer", hero: &[],
        gauges: &["TATF", "NDR", "NDN", "VSP"], tiles: &["PDN", "RATIO", "SFTMAPST", "ESCP", "SLOPE"] },
];

pub struct Glance { pub def: &'static Def, pub live: Option<Live> }

const CELL: f32 = 170.0;

pub fn show(ui: &mut Ui, app: &mut App, idx: usize) {
    let def = app.glances[idx].def;
    let Some(ecu) = app.ecus.iter().find(|e| def.types.contains(&e.system_type.as_str()) && !e.system.is_empty()).cloned() else { return };
    let note = format!("Live from the {}{}", def.from, app.glances[idx].live.as_ref().filter(|l| l.ms > 0).map_or(String::new(), |l| format!(" · {} ms per update", l.ms)));
    window(ui, def.title, &note, true, |ui| {
        let (Some(live), Some(cat)) = (app.glances[idx].live.as_ref(), app.catalogs.get(&ecu.system)) else {
            ui.add_space(10.0);
            ui.horizontal(|ui| { ui.add_space(14.0); ui.label(egui::RichText::new("Loading…").color(MUTED_FG)); });
            return;
        };
        let stopped = cat.find("VSP").is_some_and(|(d, i, _)| live.number(d, i) == Some(0.0));
        let w = ui.available_width();
        let cols = ((w / CELL).floor() as usize).max(1);
        let cw = w / cols as f32;
        let have = |s: &&&str| cat.find(s).is_some();
        let (gauges, tiles): (Vec<&&str>, Vec<&&str>) = (def.gauges.iter().filter(have).collect(), def.tiles.iter().filter(have).collect());
        let rows = |n: usize| n.div_ceil(cols);
        let (gh, th) = (cw.min(170.0) + 10.0, 118.0);
        let total = rows(gauges.len()) as f32 * gh + rows(tiles.len()) as f32 * th;
        let (area, _) = ui.allocate_exact_size(vec2(w, total + 1.0), egui::Sense::hover());
        let line = Stroke::new(1.0, BORDER);
        let mut y = area.top();
        for (set, h, as_gauge) in [(&gauges, gh, true), (&tiles, th, false)] {
            for (n, s) in set.iter().enumerate() {
                let r = Rect::from_min_size(egui::pos2(area.left() + (n % cols) as f32 * cw, y + (n / cols) as f32 * h), vec2(cw, h));
                tile::show(ui, r, live, cat, s, as_gauge, stopped);
                ui.painter().line_segment([r.right_top(), r.right_bottom()], line);
                ui.painter().line_segment([r.left_bottom(), r.right_bottom()], line);
            }
            y += rows(set.len()) as f32 * h;
        }
    });
}
