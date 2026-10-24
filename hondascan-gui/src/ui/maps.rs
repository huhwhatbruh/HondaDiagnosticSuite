//! Observed engine maps (hondascan.maps): what the PCM reported at each engine speed and load, one table per signal,
//! under a headline finding (ported from hondascan-app/src/components/MapsCard.tsx). Not Honda's stored calibration
//! (those would come from the backend's memory reads, which nothing here asks for; see CLAUDE.md, "Guard rails").
use crate::api::{MapBaseline, MapSignal, Maps};
use crate::theme::*;
use crate::widgets::{button, window, Kind};
use eframe::egui::{self, vec2, Align2, Color32, RichText, Ui};

const ORDER: [&str; 9] = ["IG", "KLAFAVE", "KLAF", "LAMBDA", "KNRTD", "KNRTD1", "KNRTD2", "KNRTD3", "KNRTD4"];
const CYLS: [&str; 4] = ["KNRTD1", "KNRTD2", "KNRTD3", "KNRTD4"];
// hondascan.maps' rule for naming a cylinder (Maps.notes): its whole-session mean retard CYL_GAP (0.5) deg above the
// lowest, counted only once every cylinder has MIN_CELL samples (`min_cell` in the reply)
const KNOCK: f64 = 0.5;
// deg: above this whole-session mean knock retard (the single KNRTD, no per-cylinder breakdown) the headline says
// "pulling timing" rather than "a little". Wording only, not a warning: the live alerts (alerts.rs) and the per-cell
// notes below raise knock warnings, the latter against the car's own history. A session mean understates per-cell
// knock, since idle (0 deg) is in it, so it is a poor basis for a warning on its own.
const KNOCK_MEAN: f64 = 2.0;

/// What each map means, the value that means "nothing to correct", and how far off colours a cell fully.
fn read(sym: &str) -> (f64, f64, usize, &'static str) {
    match sym {
        "IG" => (15.0, 20.0, 1, "Spark advance in degrees before top dead centre, as the PCM reports it. Compare cells at the same load: a cell far below its neighbours means the PCM is pulling timing."),
        "KLAFAVE" => (1.0, 0.15, 2, "Long-term fuel trim: 1.00 means no correction. Above 1 the PCM adds fuel, below it takes fuel away."),
        "KLAF" => (1.0, 0.1, 2, "Short-term fuel trim: 1.00 means no correction. It swings around 1 in normal closed-loop running."),
        "LAMBDA" => (1.0, 0.1, 2, "Measured air-fuel ratio as lambda: 1.00 is the ideal mix, above is lean, below is rich."),
        "KNRTD" => (0.0, 4.0, 1, "Knock retard in degrees: spark pulled back after knock. Zero is good; steady values mean poor fuel, heat or carbon."),
        _ => (0.0, 4.0, 1, "Knock retard of one cylinder. One cylinder far above the others points at that cylinder."),
    }
}

fn mean_of(s: Option<&MapSignal>) -> Option<f64> {
    let cells = &s?.cells;
    let n: f64 = cells.iter().map(|c| c[2]).sum();
    (n > 0.0).then(|| cells.iter().map(|c| c[2] * c[3]).sum::<f64>() / n)
}

/// The one thing worth saying about the engine from all the maps, before any table: (colour, big figure, text, sub).
fn headline(m: &Maps) -> Option<(Color32, String, String, String)> {
    let cyl: Vec<(usize, f64)> = CYLS.iter().enumerate()
        .filter_map(|(i, s)| mean_of(m.signals.get(*s).filter(|x| x.samples >= m.min_cell)).map(|v| (i + 1, v))).collect();
    let trim = mean_of(m.signals.get("KLAFAVE"));
    if cyl.len() == 4 {
        let hi = cyl.iter().copied().fold(cyl[0], |a, b| if b.1 > a.1 { b } else { a });
        let lo = cyl.iter().map(|c| c.1).fold(f64::MAX, f64::min);
        if hi.1 - lo > KNOCK {
            return Some((WARN, format!("{:.1}°", hi.1), format!("Cylinder {} pulls the most timing.", hi.0),
                format!("On average it has been retarded {:.1}° more than the quietest cylinder. A single cylinder knocking points at that cylinder.", hi.1 - lo)));
        }
        if hi.1 > 0.05 { return Some((FOREGROUND, format!("{:.1}°", hi.1), "A little knock retard, evenly spread.".into(), "All four cylinders pull about the same, which is the PCM doing its normal job.".into())); }
        return Some((FOREGROUND, "0°".into(), "No knock so far.".into(), trim.map_or("No knock retard on any cylinder.".into(), |t| format!("Long-term fuel trim averages {t:.2}."))));
    }
    // Cars that report only the single KNRTD, not a per-cylinder breakdown (the 2015 Accord refuses group 0x2694):
    // headline from it rather than falling through to fuel trim as if the car reported no knock at all. This only
    // describes the knock figure, it never warns on it (see KNOCK_MEAN); a long-term fuel trim that is itself out of
    // range still outranks a benign knock figure, the way it did before this branch existed, so when the trim is the
    // thing worth flagging we fall through to the trim headline below.
    let trim_ok = trim.is_none_or(|t| (t - 1.0).abs() <= 0.1);
    if let Some(k) = mean_of(m.signals.get("KNRTD").filter(|x| x.samples >= m.min_cell)).filter(|_| trim_ok) {
        let sub = |lead: &str| format!("{lead}{}", trim.map_or(String::new(), |t| format!(" Long-term fuel trim averages {t:.2}.")));
        return Some(match k {
            k if k > KNOCK_MEAN => (FOREGROUND, format!("{k:.1}°"), "The engine is pulling timing for knock.".into(),
                sub("Averaged over the whole session, idle included, so this is fairly steady retard. The live view flags knock as it happens, and the tables below show where.")),
            k if k > 0.05 => (FOREGROUND, format!("{k:.1}°"), "A little knock retard.".into(),
                sub("Averaged over the whole session, idle included; brief retard under load is the PCM doing its job.")),
            _ => (FOREGROUND, "0°".into(), "No knock so far.".into(), sub("The PCM has not pulled any timing for knock.")),
        });
    }
    if let Some(t) = trim {
        let w = if (t - 1.0).abs() < 0.02 { "the PCM is not correcting anything".to_string() } else { format!("about {:.0} % {} fuel", ((t - 1.0) * 100.0).abs(), if t > 1.0 { "extra" } else { "less" }) };
        return Some((if (t - 1.0).abs() > 0.1 { WARN } else { FOREGROUND }, format!("{t:.2}"), format!("Long-term fuel trim: {w}."), "1.00 means no correction.".into()));
    }
    None
}

/// Is this z a departure from the car's own history? Knock retard counts only above it (less knock is not a finding), the rest either way.
fn departs(sym: &str, z: f64, limit: f64) -> bool { if sym.starts_with("KNRTD") { z > limit } else { z.abs() > limit } }

/// One line on how this drive compares with the car's earlier ones: (colour, text). Never says "normal" without enough history.
fn history_line(b: &MapBaseline) -> (Color32, String) {
    if b.min_sessions == 0 { return (MUTED_FG, String::new()); }
    if b.sessions < b.min_sessions {
        return (MUTED_FG, format!("Not enough history yet: {} earlier drive{} recorded, {} needed to compare this car with itself. The fixed limits apply until then.",
            b.sessions, if b.sessions == 1 { "" } else { "s" }, b.min_sessions));
    }
    let bad: Vec<String> = b.cells.iter().filter_map(|(s, cs)| {
        let n = cs.iter().filter(|c| departs(s, c[5], b.z)).count();
        (n > 0).then(|| format!("{} ({n} cell{})", s, if n == 1 { "" } else { "s" }))
    }).chain(b.cylinders.iter().filter(|c| departs(&c.signal, c.z, b.z)).map(|c| format!("cylinder {}", c.cylinder))).collect();
    let compared: usize = b.cells.values().map(Vec::len).sum();
    if bad.is_empty() && compared == 0 && b.cylinders.is_empty() {
        (MUTED_FG, format!("{} earlier drives recorded, but no cell of this drive has been filled often enough in {} of them to compare yet. The fixed limits apply.", b.sessions, b.min_sessions))
    } else if bad.is_empty() {
        (OK, format!("Against this car’s own history ({} earlier drives): {compared} cells compared, none is unusual for this car.", b.sessions))
    } else {
        (WARN, format!("Against this car’s own history ({} earlier drives) these differ from their usual: {}. Outlined cells below.", b.sessions, bad.join(", ")))
    }
}

/// The headline as a big figure with its sentence beside it.
fn banner(ui: &mut Ui, (col, big, text, sub): (Color32, String, String, String)) {
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.add_space(20.0);
        ui.label(RichText::new(big).font(bold(60.0)).color(col));
        ui.add_space(12.0);
        ui.vertical(|ui| {
            ui.add_space(14.0);
            ui.label(RichText::new(text).font(bold(18.0)));
            ui.label(RichText::new(sub).size(13.0).color(MUTED_FG));
        });
    });
    ui.add_space(8.0);
}

pub fn show(ui: &mut Ui, maps: Option<&Maps>, sel: &mut String) {
    window(ui, "Engine maps", "what this car actually did, by engine speed and load", false, |ui| {
        let Some(m) = maps else { return empty(ui, "Waiting for the first readings.") };
        if let Some(h) = headline(m) { banner(ui, h); }
        let (hc, ht) = history_line(&m.baseline);
        if !ht.is_empty() {
            ui.horizontal_wrapped(|ui| { ui.add_space(20.0); ui.label(RichText::new(ht).size(13.0).color(hc)).on_hover_text(&m.baseline.rule); });
            ui.add_space(8.0);
        }
        ui.separator();
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            ui.add_space(20.0);
            for s in ORDER.iter().filter(|s| !s.starts_with("KNRTD") || **s == "KNRTD" || m.signals.get(**s).is_some_and(|x| x.samples > 0)) {
                let name = m.signals.get(*s).map_or(*s, |x| x.name.as_str()).replace("Knock retard, cylinder ", "Cylinder ");
                if button(ui, &name, if *sel == *s { Kind::Primary } else { Kind::Normal }).clicked() { *sel = s.to_string(); }
            }
        });
        let (target, span, digits, about) = read(sel);
        ui.horizontal_wrapped(|ui| { ui.add_space(20.0); ui.label(RichText::new(format!("{about} Down the side is engine speed, across the top is how hard the engine is working (air per revolution).")).size(12.0).color(MUTED_FG)); });
        let Some(sig) = m.signals.get(sel.as_str()).filter(|s| !s.cells.is_empty()) else {
            return empty(ui, if sel.starts_with("KNRTD") || sel == "IG" { "Empty until the engine has run. Every drive fills it in." }
                else { "Empty until the engine has run. Fuel readings only count once the engine is warm (70 °C, 158 °F) and burning fuel, so it stays blank while it warms up and on the overrun." });
        };
        grid(ui, m, sig, sel, (target, span, digits));
    });
}

/// One signal's table: engine speed down the side, load across the top, each cell coloured by how far its mean is from `target`
/// (`span` away is full colour), faded when thin, outlined when it departs from the car's own history.
fn grid(ui: &mut Ui, m: &Maps, sig: &MapSignal, sel: &str, (target, span, digits): (f64, f64, usize)) {
    let cells: Vec<&[f64; 6]> = sig.cells.iter().filter(|c| (c[0] as usize) < m.rpm.len() && (c[1] as usize) < m.load.len()).collect();
    let (Some(lo), Some(hi)) = (cells.iter().map(|c| c[0] as usize).min(), cells.iter().map(|c| c[0] as usize).max()) else {
        return empty(ui, "The engine speed and load axes did not arrive with the cells.");
    };
    let (rw, ch, rh) = (80.0, ((ui.available_width() - 60.0 - 80.0) / m.load.len() as f32).max(40.0), 26.0);
    let (area, _) = ui.allocate_exact_size(vec2(ui.available_width(), (hi - lo + 2) as f32 * rh + 14.0), egui::Sense::hover());
    let o = area.min + vec2(20.0, 8.0);
    ui.painter().text(o + vec2(rw - 6.0, rh / 2.0), Align2::RIGHT_CENTER, "rpm \\ g/rev", sans(11.0), MUTED_FG);
    for (j, l) in m.load.iter().enumerate() { ui.painter().text(o + vec2(rw + j as f32 * ch + ch / 2.0, rh / 2.0), Align2::CENTER_CENTER, format!("≤{l}"), sans(11.0), MUTED_FG); }
    for r in lo..=hi {
        let y = o.y + (r - lo + 1) as f32 * rh;
        if !ui.is_rect_visible(egui::Rect::from_min_size(egui::pos2(area.left(), y), vec2(area.width(), rh))) { continue; }   // out of view: not laid out
        ui.painter().text(egui::pos2(o.x + rw - 6.0, y + rh / 2.0), Align2::RIGHT_CENTER, format!("≤{}", m.rpm[r]), sans(11.0), MUTED_FG);
        for j in 0..m.load.len() {
            let cell = egui::Rect::from_min_size(egui::pos2(o.x + rw + j as f32 * ch, y), vec2(ch - 1.0, rh - 1.0));
            match cells.iter().find(|c| c[0] as usize == r && c[1] as usize == j) {
                None => { ui.painter().rect_filled(cell, 0.0, Color32::from_white_alpha(8)); ui.painter().text(cell.center(), Align2::CENTER_CENTER, "·", sans(11.0), Color32::from_white_alpha(50)); }
                Some(c) => {
                    let dev = ((c[3] - target).abs() / span).min(1.0) as f32;
                    let base = if dev > 0.66 { FAULT } else { WARN };
                    let thin = c[2] < m.min_cell as f64;
                    let fill = CARD.lerp_to_gamma(base, dev * 0.7);
                    let a = if thin { 0.5 } else { 1.0 };
                    ui.painter().rect_filled(cell, 0.0, fill.gamma_multiply(a));
                    ui.painter().text(cell.center(), Align2::CENTER_CENTER, format!("{:.*}", digits, c[3]), mono(12.0), FOREGROUND.gamma_multiply(a));
                    let own = m.baseline.cells.get(sel).and_then(|v| v.iter().find(|b| b[0] as usize == r && b[1] as usize == j));
                    if own.is_some_and(|b| departs(sel, b[5], m.baseline.z)) {
                        ui.painter().rect_stroke(cell, 0.0, egui::Stroke::new(2.0, BLUE), egui::StrokeKind::Inside);
                    }
                    let vs = own.map_or(String::new(), |b| format!("\nthis car’s usual {:.*} ± {:.*} over {} drives · {:+.1}× its usual variation", digits + 1, b[3], digits + 1, b[4], b[2], b[5]));
                    ui.interact(cell, ui.id().with(("cell", r, j)), egui::Sense::hover())
                        .on_hover_text(format!("{} samples · mean {} · min {} · max {}{}{vs}", c[2], c[3], c[4], c[5], if thin { " · too few to trust" } else { "" }));
                }
            }
        }
    }
    ui.horizontal(|ui| { ui.add_space(20.0); ui.label(RichText::new(format!("{} samples · faded cells have fewer than {} · blue outline: differs from this car’s own history · hover a cell for count, min and max", sig.samples, m.min_cell)).size(11.0).color(MUTED_FG)); });
    ui.add_space(8.0);
}

fn empty(ui: &mut Ui, text: &str) {
    ui.add_space(12.0);
    ui.horizontal_wrapped(|ui| { ui.add_space(20.0); ui.label(RichText::new(text).size(13.0).color(MUTED_FG)); });
    ui.add_space(12.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn knock(samples: u32, cyl1: f64) -> Maps {
        let mut m = Maps { rpm: vec![500.0; 12], load: vec![0.2; 7], min_cell: 20, ..Default::default() };
        for (i, s) in CYLS.iter().enumerate() {
            let mean = if i == 0 { cyl1 } else { 0.0 };
            m.signals.insert(s.to_string(), MapSignal { name: s.to_string(), unit: "deg".into(), samples, cells: vec![[1.0, 1.0, samples as f64, mean, 0.0, mean]] });
        }
        m
    }

    #[test]
    fn a_cylinder_is_named_by_the_backends_rule_only() {
        assert!(headline(&knock(1, 1.0)).is_none(), "one sample is not enough (hondascan.maps MIN_CELL)");
        let h = headline(&knock(20, 1.0)).unwrap();
        assert!(h.0 == WARN && h.2.contains("Cylinder 1"));
        assert!(headline(&knock(20, 0.4)).unwrap().2.contains("evenly"));
    }

    /// A car that reports only the single KNRTD (no KNRTD1-4), like the 2015 Accord: the headline is the knock
    /// figure, not the fuel trim it used to fall back to. `trim` sets the long-term fuel trim mean.
    fn single_knrtd(mean: f64) -> Maps { single_knrtd_trim(mean, 1.04) }
    fn single_knrtd_trim(mean: f64, trim: f64) -> Maps {
        let mut m = Maps { rpm: vec![500.0; 12], load: vec![0.2; 7], min_cell: 20, ..Default::default() };
        m.signals.insert("KNRTD".into(), MapSignal { name: "KNRTD".into(), unit: "deg".into(), samples: 100, cells: vec![[1.0, 1.0, 100.0, mean, 0.0, mean]] });
        m.signals.insert("KLAFAVE".into(), MapSignal { name: "KLAFAVE".into(), unit: "".into(), samples: 100, cells: vec![[1.0, 1.0, 100.0, trim, trim, trim]] });
        m
    }

    #[test]
    fn single_knock_signal_leads_the_headline_not_fuel_trim() {
        // informational only, never a warning on its own (the live alerts and per-cell notes warn): colour stays calm.
        let (c, big, head, _) = headline(&single_knrtd(0.72)).unwrap(); // session 115 mean
        assert!(c == FOREGROUND && big == "0.7°" && head.contains("knock retard"), "{big} / {head}");
        let (c, big, head, _) = headline(&single_knrtd(3.0)).unwrap();
        assert!(c == FOREGROUND && big == "3.0°" && head.contains("pulling timing"), "{big} / {head}");
        let (_, big, head, _) = headline(&single_knrtd(0.0)).unwrap();
        assert!(big == "0°" && head.contains("No knock"), "{big} / {head}");
    }

    #[test]
    fn an_out_of_range_fuel_trim_still_takes_the_headline_from_a_benign_knock_figure() {
        // Regression from the first cut of the single-KNRTD branch (cloud review): the branch returned on every path,
        // so a long-term trim well off 1.00 was buried under a calm knock headline instead of warning.
        let (c, big, head, _) = headline(&single_knrtd_trim(0.5, 1.15)).unwrap();
        assert!(c == WARN && big == "1.15" && head.contains("fuel trim"), "{big} / {head}");
        // in-range trim does not steal the headline: the knock figure still leads.
        assert!(headline(&single_knrtd_trim(0.5, 1.04)).unwrap().0 == FOREGROUND);
    }

    fn base(sessions: u32) -> MapBaseline { MapBaseline { sessions, min_sessions: 3, z: 3.0, ..Default::default() } }

    #[test]
    fn the_history_line_never_calls_a_car_normal_without_enough_history() {
        let (c, t) = history_line(&base(2));
        assert!(c == MUTED_FG && t.contains("2 earlier drives recorded, 3 needed") && t.contains("fixed limits"));
        assert!(history_line(&base(1)).1.contains("1 earlier drive recorded"));
        assert!(history_line(&MapBaseline::default()).1.is_empty(), "an old backend sends no baseline: say nothing");
    }

    #[test]
    fn departures_name_their_signal_and_knock_only_counts_upwards() {
        let mut b = base(5);
        b.cells.insert("KLAFAVE".into(), vec![[5.0, 3.0, 5.0, 1.04, 0.01, -4.0], [5.0, 2.0, 5.0, 1.04, 0.01, 0.5]]);
        b.cells.insert("KNRTD".into(), vec![[5.0, 3.0, 5.0, 1.0, 0.2, -6.0]]);          // far less knock than usual: not a finding
        let (c, t) = history_line(&b);
        assert!(c == WARN && t.contains("KLAFAVE (1 cell)") && !t.contains("KNRTD"), "{t}");
        b.cylinders.push(crate::api::CylBase { signal: "KNRTD4".into(), cylinder: 4, z: 5.0, ..Default::default() });
        assert!(history_line(&b).1.contains("cylinder 4"));
        let mut calm = base(5);
        assert!(history_line(&calm).0 == MUTED_FG, "5 drives on record but nothing compared: not 'normal'");
        calm.cells.insert("KLAFAVE".into(), vec![[5.0, 3.0, 5.0, 1.04, 0.01, 0.5]]);
        assert!(history_line(&calm).0 == OK);
    }
}
