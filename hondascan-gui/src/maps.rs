//! Observed engine maps (hondascan.maps): what the PCM reported at each engine speed and load, one table per signal,
//! under a headline finding. QML only draws the JSON `maps_json` builds.
//!
//! Pure data: every number, sentence and decision the egui drawing code made is made here. The egui theme colours
//! become tone ints QML switches on (0 normal, 1 warn, 2 fault); the ok-green and muted-grey distinction, the cell
//! fading, the hover-tooltips and the outline's blue are drawing, QML's.
use crate::api::{MapBaseline, MapSignal, Maps};
use serde_json::{json, Value};

// The egui theme colours this card used, as tone ints QML switches on. FOREGROUND is tone 0, WARN 1, FAULT 2, MUTED_FG 3 and OK 4
// on purpose: their distinction (calm green, quiet grey) is how the egui drew them, the JSON only says "normal".
const FOREGROUND: i64 = 0;
const MUTED_FG: i64 = 3;
const OK: i64 = 4;
const WARN: i64 = 1;
const FAULT: i64 = 2;

const ORDER: [&str; 9] = ["IG", "KLAFAVE", "KLAF", "LAMBDA", "KNRTD", "KNRTD1", "KNRTD2", "KNRTD3", "KNRTD4"];
const CYLS: [&str; 4] = ["KNRTD1", "KNRTD2", "KNRTD3", "KNRTD4"];
// hondascan.maps' rule for naming a cylinder (Maps.notes): its whole-session mean retard CYL_GAP (0.5) deg above the
// lowest, counted only once every cylinder has MIN_CELL samples (`min_cell` in the reply)
const KNOCK: f64 = 0.5;
// deg: above this whole-session mean knock retard (the single KNRTD, no per-cylinder breakdown) the headline says
// "pulling timing" rather than "a little". Wording only, not a warning: the live alerts (alerts.rs) and the backend's
// per-cell maps notes raise knock warnings, the latter against the car's own history. A session mean understates
// per-cell knock, since idle (0 deg) is in it, so it is a poor basis for a warning on its own.
const KNOCK_MEAN: f64 = 2.0;

/// What each map means, the value that means "nothing to correct", and how far off colours a cell fully (the egui's
/// wording: here it is how far the mean sits from `target` before `dev` in the cell JSON reaches 1.0).
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

/// The egui appends this to `read`'s explanation as one paragraph; it is its own field here (the lead joins them
/// with a space, the same sentence).
const AXES_NOTE: &str = "Down the side is engine speed, across the top is how hard the engine is working (air per revolution). Each row and column is the range up to its number.";

fn mean_of(s: Option<&MapSignal>) -> Option<f64> {
    let cells = &s?.cells;
    let n: f64 = cells.iter().map(|c| c[2]).sum();
    (n > 0.0).then(|| cells.iter().map(|c| c[2] * c[3]).sum::<f64>() / n)
}

/// The one thing worth saying about the engine from all the maps, before any table: (tone, big figure, text, sub).
fn headline(m: &Maps) -> Option<(i64, String, String, String)> {
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
        if hi.1 > 0.05 {
            return Some((FOREGROUND, format!("{:.1}°", hi.1), "A little knock retard, evenly spread.".into(),
                "All four cylinders pull about the same, which is the PCM doing its normal job.".into()));
        }
        return Some((FOREGROUND, "0°".into(), "No knock so far.".into(),
            trim.map_or("No knock retard on any cylinder.".into(), |t| format!("Long-term fuel trim averages {t:.2}."))));
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

/// One line on how this drive compares with the car's earlier ones: (tone, text). Never says "normal" without enough
/// history. (In the egui the line showed `baseline.rule` on hover, a tooltip: not portable, dropped.)
fn history_line(b: &MapBaseline) -> (i64, String) {
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

/// The card as QML draws it: `{title, subtitle, waiting, headline, history, tabs, selected, table, empty}`, with the
/// waiting card the `None` case (when `m` has still not arrived). `tabs` is the egui's tab row in its order and
/// filter; `table` is the selected signal's grid below its headline, or `empty` when there is nothing to table.
pub fn maps_json(m: Option<&Maps>, selected: &str) -> Value {
    let subtitle = "what this car actually did, by engine speed and load";
    let Some(m) = m else {
        return json!({ "title": "Engine maps", "subtitle": subtitle, "waiting": "Waiting for the first readings.",
            "headline": null, "history": null, "tabs": [], "selected": selected, "table": null, "empty": null });
    };
    let headline = headline(m).map(|(tone, big, text, sub)| json!({ "tone": tone, "big": big, "text": text, "sub": sub }));
    let (tone, text) = history_line(&m.baseline);
    // The egui card only draws the history line when it says something.
    let history = (!text.is_empty()).then(|| json!({ "tone": tone, "text": text }));
    // The tab row: like the egui's, a per-cylinder tab only appears once the car has reported it, and the label is
    // the signal's name with the cylinder word moved up front.
    let tabs: Vec<Value> = ORDER.iter().filter(|s| !s.starts_with("KNRTD") || **s == "KNRTD" || m.signals.get(**s).is_some_and(|x| x.samples > 0))
        .map(|&s| {
            let data = m.signals.get(s).is_some_and(|x| x.samples > 0);
            json!({ "symbol": s, "label": m.signals.get(s).map_or(s, |x| x.name.as_str()).replace("Knock retard, cylinder ", "Cylinder "), "has_data": data })
        }).collect();
    let (target, span, digits, about) = read(selected);
    let table = m.signals.get(selected).filter(|s| !s.cells.is_empty()).map(|sig| grid(m, sig, selected, (target, span, digits), about));
    // The egui draws `read`'s explanation above the table even when there is nothing to tabulate; this card says just
    // why it is blank there instead (an `about` would promise a grid that is not coming): KNRTD* and IG fill from any
    // drive, fuel readings only count warm and burning. The warm threshold is the backend's own WARM
    // (hondascan/src/maps.rs), shown with the conversion the egui card put beside it.
    let empty = table.is_none().then(|| if selected.starts_with("KNRTD") || selected == "IG" {
        "Empty until the engine has run. Every drive fills it in.".to_string()
    } else {
        let warm = 70.0;
        let f = warm * 1.8 + 32.0;
        format!("Empty until the engine has run. Fuel readings only count once the engine is warm ({warm:.0} °C, {f:.0} °F) and burning fuel, so it stays blank while it warms up and on the overrun.")
    });
    json!({ "title": "Engine maps", "subtitle": subtitle, "waiting": null, "headline": headline, "history": history,
        "tabs": tabs, "selected": selected, "table": table, "empty": empty })
}

/// One signal's table: engine speed down the side, load across the top, each cell toned by how far its mean is from
/// `target` (`span` away is full), `thin` where the egui faded (too few samples), `outlined` when it departs from the
/// car's own history. The egui hover (count, min, max and the car's usual mean and variation) is drawing: count, min
/// and max ride along in each cell, the usual line is left out.
fn grid(m: &Maps, sig: &MapSignal, sel: &str, (target, span, digits): (f64, f64, usize), about: &str) -> Value {
    let cells: Vec<&[f64; 6]> = sig.cells.iter().filter(|c| (c[0] as usize) < m.rpm.len() && (c[1] as usize) < m.load.len()).collect();
    let (Some(lo), Some(hi)) = (cells.iter().map(|c| c[0] as usize).min(), cells.iter().map(|c| c[0] as usize).max()) else {
        return json!({ "target": target, "span": span, "digits": digits, "about": about, "note": AXES_NOTE, "rpm": m.rpm, "load": m.load,
            "legend": null, "axes_error": "The engine speed and load axes did not arrive with the cells.", "cells": [] });
    };
    let mut all = Vec::with_capacity((hi - lo + 2) * m.load.len());
    for r in lo..=hi {
        for l in 0..m.load.len() {
            match cells.iter().find(|c| c[0] as usize == r && c[1] as usize == l) {
                // An empty intersection is the egui's faint placeholder.
                None => all.push(json!({ "r": r, "l": l, "text": "·", "v": null, "count": 0, "min": 0.0, "max": 0.0, "thin": false, "dev": 0.0, "tone": 0, "outlined": false })),
                Some(c) => {
                    // How far off the mean is, as data: QML fades and colours from it. The base tone is always
                    // warn-or-fault (the egui filled the cell from these two; normal tone is only an empty gap's).
                    let dev = ((c[3] - target).abs() / span).min(1.0);
                    let own = m.baseline.cells.get(sel).and_then(|v| v.iter().find(|b| b[0] as usize == r && b[1] as usize == l));
                    let thin = c[2] < m.min_cell as f64;
                    // the egui hover, word for word: the cell's own figures, and how it sits against this car's earlier drives
                    let vs = own.map_or(String::new(), |b| format!("\nthis car’s usual {:.*} ± {:.*} over {} drives · {:+.1}× its usual variation", digits + 1, b[3], digits + 1, b[4], b[2], b[5]));
                    let hover = format!("{} samples · mean {} · min {} · max {}{}{vs}", c[2], c[3], c[4], c[5], if thin { " · too few to trust" } else { "" });
                    all.push(json!({ "r": r, "l": l, "text": format!("{:.*}", digits, c[3]), "v": c[3], "count": c[2], "min": c[4], "max": c[5], "hover": hover,
                        "thin": thin, "dev": dev, "tone": if dev > 0.66 { FAULT } else { WARN },
                        "outlined": own.is_some_and(|b| departs(sel, b[5], m.baseline.z)) }));
                }
            }
        }
    }
    json!({ "target": target, "span": span, "digits": digits, "about": about, "note": AXES_NOTE, "rpm": m.rpm, "load": m.load,
        "legend": format!("{} samples · faded cells have fewer than {} · blue outline: differs from this car’s own history · hover a cell for count, min and max", sig.samples, m.min_cell),
        "axes_error": null, "cells": all })
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
        // informational only, never a warning on its own (the live alerts warn, and the cells say where): tone stays calm.
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

    /// What the card shows when two cylinders and the long-term trim have data: the tabs row, the trim headline,
    /// the car's own-history line and KLAFAVE's grid, one of whose cells the baseline departs on.
    fn card() -> Maps {
        let mut m = Maps { rpm: vec![500.0, 1000.0], load: vec![0.2, 0.4], min_cell: 20,
            baseline: MapBaseline { sessions: 3, min_sessions: 3, z: 3.0, ..Default::default() }, ..Default::default() };
        m.baseline.cells.insert("KLAFAVE".into(), vec![[0.0, 1.0, 5.0, 1.0, 0.01, 5.0], [1.0, 0.0, 5.0, 0.95, 0.01, -1.0]]);
        m.signals.insert("KNRTD1".into(), MapSignal { name: "Knock retard, cylinder 1".into(), unit: "deg".into(), samples: 40, cells: vec![[0.0, 0.0, 40.0, 2.0, 0.0, 3.0]] });
        m.signals.insert("KNRTD2".into(), MapSignal { name: "Knock retard, cylinder 2".into(), unit: "deg".into(), samples: 0, cells: vec![] });
        m.signals.insert("KLAFAVE".into(), MapSignal { name: "LT fuel trim".into(), unit: "ratio".into(), samples: 30,
            cells: vec![[0.0, 1.0, 30.0, 1.20, 1.1, 1.3], [1.0, 0.0, 10.0, 0.92, 0.9, 1.0]] });
        m
    }

    fn symbols(j: &Value) -> Vec<&str> {
        j["tabs"].as_array().unwrap().iter().map(|t| t["symbol"].as_str().unwrap()).collect()
    }

    #[test]
    fn the_card_json_is_what_the_screen_draws() {
        let m = card();
        let j = maps_json(Some(&m), "KLAFAVE");
        assert_eq!(j["title"], "Engine maps");
        assert_eq!(j["subtitle"], "what this car actually did, by engine speed and load");
        assert!(j["waiting"].is_null());
        // the tab row: the egui's order and filter, with the cylinder signal named by its signal
        assert_eq!(symbols(&j), vec!["IG", "KLAFAVE", "KLAF", "LAMBDA", "KNRTD", "KNRTD1"], "KNRTD2 has no samples: it is filtered out");
        let tab = j["tabs"].as_array().unwrap().iter().find(|t| t["symbol"] == "KNRTD1").unwrap();
        assert_eq!((tab["label"].as_str(), tab["has_data"].as_bool()), (Some("Cylinder 1"), Some(true)));
        let tab = j["tabs"].as_array().unwrap().iter().find(|t| t["symbol"] == "IG").unwrap();
        assert_eq!(tab["has_data"].as_bool(), Some(false), "IG has nothing yet: the tab is only grey (QML's)");
        // the headline: one cylinder reporting means the single-KNRTD branch is skipped and the trim says it into WARN
        assert_eq!((j["headline"]["tone"].as_i64(), j["headline"]["big"].as_str()), (Some(1), Some("1.13")));
        assert_eq!(j["headline"]["text"], "Long-term fuel trim: about 13 % extra fuel.");
        assert_eq!(j["headline"]["sub"], "1.00 means no correction.");
        // the car's own history line, WARN because a baseline cell departs
        assert_eq!((j["history"]["tone"].as_i64(), j["history"]["text"].as_str()), (Some(1), Some("Against this car’s own history (3 earlier drives) these differ from their usual: KLAFAVE (1 cell). Outlined cells below.")));
        // the table scalars
        assert_eq!((j["table"]["target"].as_f64(), j["table"]["span"].as_f64(), j["table"]["digits"].as_u64()), (Some(1.0), Some(0.15), Some(2)));
        assert_eq!(j["table"]["about"], "Long-term fuel trim: 1.00 means no correction. Above 1 the PCM adds fuel, below it takes fuel away.");
        assert_eq!(j["table"]["note"], "Down the side is engine speed, across the top is how hard the engine is working (air per revolution). Each row and column is the range up to its number.");
        assert_eq!(j["table"]["rpm"], json!([500.0, 1000.0]));
        assert_eq!(j["table"]["load"], json!([0.2, 0.4]));
        assert_eq!(j["table"]["legend"], "30 samples · faded cells have fewer than 20 · blue outline: differs from this car’s own history · hover a cell for count, min and max");
        assert_eq!(j["selected"], "KLAFAVE");
        assert!(j["empty"].is_null(), "the selection has cells: no empty state");
        // the grid: every (r, l) of the in-axis rows and all load columns, empty intersections faint
        let cells = j["table"]["cells"].as_array().unwrap();
        assert_eq!(cells.len(), 4);
        assert_eq!(cells[0], json!({ "r": 0, "l": 0, "text": "·", "v": null, "count": 0, "min": 0.0, "max": 0.0, "thin": false, "dev": 0.0, "tone": 0, "outlined": false }));
        let hot = &cells[1];
        assert_eq!((hot["r"].as_u64(), hot["l"].as_u64(), hot["text"].as_str()), (Some(0), Some(1), Some("1.20")));
        assert_eq!((hot["v"].as_f64(), hot["count"].as_f64(), hot["min"].as_f64(), hot["max"].as_f64()), (Some(1.2), Some(30.0), Some(1.1), Some(1.3)));
        assert_eq!((hot["dev"].as_f64(), hot["tone"].as_i64(), hot["thin"].as_bool(), hot["outlined"].as_bool()), (Some(1.0), Some(2), Some(false), Some(true)),
            "0.15 of span away, dinged to 1.0; a filled cell tones warn or fault, 0.66 into the span is FAULT; the baseline departs there");
        let thin = &cells[2];
        assert_eq!((thin["r"].as_u64(), thin["l"].as_u64(), thin["text"].as_str(), thin["thin"].as_bool()), (Some(1), Some(0), Some("0.92"), Some(true)), "10 < min_cell: the egui's fade is data here");
        assert_eq!(thin["tone"].as_i64(), Some(1), "below 0.66 of the span: WARN (a filled cell is never plain toned)");
        assert!((thin["dev"].as_f64().unwrap() - 8.0 / 15.0).abs() < 1e-12, "(|0.92 - 1.00| / 0.15 = 8/15)");
        assert_eq!(thin["outlined"].as_bool(), Some(false), "the baseline cell there is z -1: it does not depart (|z| <= 3)");
    }

    /// Nothing to read yet, a map with no cells, and cells that sit outside the axes: what the blank states say.
    fn axes_only() -> Maps { Maps { rpm: vec![500.0, 1000.0], load: vec![0.2, 0.4], min_cell: 20, ..Default::default() } }

    #[test]
    fn the_empty_states_say_why_the_table_is_blank() {
        // the waiting card: the first reply has not arrived
        let w = maps_json(None, "IG");
        assert_eq!(w["waiting"], "Waiting for the first readings.");
        assert!(w["headline"].is_null() && w["history"].is_null() && w["table"].is_null() && w["empty"].is_null());
        assert!(w["tabs"].as_array().unwrap().is_empty(), "the waiting card shows no tabs");
        // a KNRTD* selection with no cells: like the egui's tabs row, per-cylinder signals with nothing are not shown
        let fresh = axes_only();
        let j = maps_json(Some(&fresh), "KNRTD1");
        assert_eq!(j["empty"], "Empty until the engine has run. Every drive fills it in.");
        assert!(j["table"].is_null() && j["history"].is_null());
        assert_eq!(symbols(&j), vec!["IG", "KLAFAVE", "KLAF", "LAMBDA", "KNRTD"]);
        // a fuel selection: the empty text names the backend's warm threshold, WARM (70 °C, 158 °F)
        let j = maps_json(Some(&fresh), "KLAFAVE");
        assert_eq!(j["empty"], "Empty until the engine has run. Fuel readings only count once the engine is warm (70 °C, 158 °F) and burning fuel, so it stays blank while it warms up and on the overrun.");
        assert!(j["table"].is_null());
        // cells outside the axes: the egui grid's early-empty state, its legend and body never drawn
        let mut ghost = axes_only();
        ghost.signals.insert("IG".into(), MapSignal { name: "Spark advance".into(), unit: "deg".into(), samples: 40, cells: vec![[5.0, 0.0, 40.0, 15.0, 14.0, 16.0]] });
        let j = maps_json(Some(&ghost), "IG");
        assert_eq!(j["table"]["axes_error"], "The engine speed and load axes did not arrive with the cells.");
        assert!(j["table"]["legend"].is_null() && j["table"]["cells"].as_array().unwrap().is_empty());
        assert_eq!(j["table"]["about"], "Spark advance in degrees before top dead centre, as the PCM reports it. Compare cells at the same load: a cell far below its neighbours means the PCM is pulling timing.");
        assert_eq!((j["table"]["target"].as_f64(), j["table"]["span"].as_f64(), j["table"]["digits"].as_u64()), (Some(15.0), Some(20.0), Some(1)));
    }
}
