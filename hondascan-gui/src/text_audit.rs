//! Guards for the words on the QML screens: a phrase list
//! that must not appear in a string the person reads, and the numbers quoted in those strings checked against the
//! backend's own constants. Compiled only for tests: it guards screens, it is not one.
use std::fs;
use std::path::Path;

const QML: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/qml");
const BACKEND: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../hondascan/src");

/// Words about where the data did not come from. A card says what it shows, not what it is not.
const STALE: &[&str] = &["not Honda", "Honda’s stored", "Honda's stored", "stored tables", "factory spec", "base map", "basemap", "not how the engine", "does not record"];

fn source(dir: &str, name: &str) -> String { fs::read_to_string(Path::new(dir).join(name)).unwrap_or_else(|e| panic!("{name}: {e}")) }

/// The banned phrases a line carries, matching either case: card text may shout.
fn banned(line: &str) -> Vec<&'static str> {
    STALE.iter().copied().filter(|b| line.to_lowercase().contains(&b.to_lowercase())).collect()
}

/// `pub const NAME: f64 = 2.0;` -> 2.0 (the backend's own text, at test time)
fn constant(file: &str, name: &str) -> f64 {
    let src = source(BACKEND, file);
    let line = src.lines().find(|l| l.contains(&format!("pub const {name}:"))).unwrap_or_else(|| panic!("{file} has no {name}"));
    line.split('=').nth(1).and_then(|v| v.split(';').next()).and_then(|v| v.trim().parse().ok()).unwrap_or_else(|| panic!("{name} does not parse: {line}"))
}

/// `readonly property real warnAt: 2    // note` -> 2.0
fn qml_number(src: &str, name: &str) -> f64 {
    let line = src.lines().find(|l| l.contains(&format!("property real {name}:"))).unwrap_or_else(|| panic!("Hero.qml has no {name}"));
    line.split(':').nth(1).and_then(|v| v.split("//").next()).and_then(|v| v.trim().parse().ok()).unwrap_or_else(|| panic!("{name} does not parse: {line}"))
}

#[test]
fn no_screen_text_carries_leftover_wording() {
    let mut hits = Vec::new();
    for entry in fs::read_dir(QML).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if !name.ends_with(".qml") { continue; }
        for (n, line) in fs::read_to_string(&path).unwrap().lines().enumerate() {
            if line.trim_start().starts_with("//") { continue; } // a comment is not on the card
            for bad in banned(line) { hits.push(format!("{name}:{}: “{bad}”", n + 1)); }
        }
    }
    // the words most cards now get from Rust (the part before each file's own tests: strings in tests describe cases, not cards)
    for name in ["actuator", "baseline", "drive", "glance", "history", "kline", "maps", "reference", "selfcheck", "selftests", "writes"] {
        let src = source(concat!(env!("CARGO_MANIFEST_DIR"), "/src"), &format!("{name}.rs"));
        for (n, line) in src.split("#[cfg(test)]").next().unwrap_or("").lines().enumerate() {
            if line.trim_start().starts_with("//") || !line.contains('"') { continue; }
            for bad in banned(line) { hits.push(format!("{name}.rs:{}: “{bad}”", n + 1)); }
        }
    }
    assert!(hits.is_empty(), "wording about what the data is not, in a string the person reads (say what the card shows instead):\n{}", hits.join("\n"));
}

#[test]
fn the_knock_figure_marks_at_the_thresholds_the_backend_uses() {
    let hero = source(QML, "Hero.qml");
    assert_eq!(qml_number(&hero, "warnAt"), constant("alerts.rs", "KNOCK_WARN"), "Hero.qml warnAt must mirror KNOCK_WARN in hondascan/src/alerts.rs");
    assert_eq!(qml_number(&hero, "faultAt"), constant("alerts.rs", "KNOCK_FAULT"), "Hero.qml faultAt must mirror KNOCK_FAULT in hondascan/src/alerts.rs");
}

#[test]
fn the_drive_card_quotes_the_fuel_constants_and_the_economy_minimum_the_backend_uses() {
    let (afr, density) = (constant("trip.rs", "AFR"), constant("trip.rs", "DENSITY"));
    let drive = source(concat!(env!("CARGO_MANIFEST_DIR"), "/src"), "drive.rs");
    let drive = drive.split("#[cfg(test)]").next().unwrap_or("");
    assert!(drive.contains(&format!("airflow ÷ {afr} ÷ {density} g/cc")), "drive.rs fuel-flow note must quote AFR {afr} and density {density} (hondascan/src/trip.rs)");
    let trip = source(BACKEND, "trip.rs");
    assert!(trip.contains("km >= 0.1") && drive.contains("needs 0.1 km of driving"), "the economy note and trip.rs `economy` must agree on the 0.1 km minimum");
}

#[test]
fn the_maps_card_names_the_warm_engine_threshold_the_backend_uses() {
    let warm = constant("maps.rs", "WARM");
    let both = format!("({warm:.0} °C, {:.0} °F)", warm * 9.0 / 5.0 + 32.0);
    let maps = crate::maps::maps_json(Some(&crate::api::Maps::default()), "LAMBDA");
    let text = maps["empty"].as_str().expect("an empty fuel map says why");
    assert!(text.contains(&both), "the maps card must say {both}, the warm-engine limit in hondascan/src/maps.rs: {text}");
}

#[test]
fn the_condition_card_quotes_the_gate_the_backend_uses() {
    let (warm, hot) = (constant("condition.rs", "WARM_ECT"), constant("condition.rs", "HOT_ECT"));
    let cruise = constant("condition.rs", "CRUISE_KMH");
    let line = crate::condition::GATE_LINE;
    let f = crate::condition::f_of_c;
    assert!(line.contains(&format!("({:.0} to {:.0} °F)", f(warm), f(hot))), "the gate line must give {warm} and {hot} degC in °F: {line}");
    assert!(line.contains(&format!("from {:.0} mph", cruise * crate::condition::MPH_PER_KMH)), "the gate line must give {cruise} km/h in mph: {line}");
    let src = source(BACKEND, "condition.rs");
    assert!(src.contains("pub const IDLE_RPM: (f64, f64) = (500.0, 1000.0);") && line.contains("500 to 1000 rpm"), "idle range must agree with condition.rs");
}

#[test]
fn the_checker_catches_a_banned_phrase() {
    assert!(banned("This is not Honda's stored data").contains(&"Honda's stored"), "the checker must catch a banned phrase");
    assert!(banned("Say what the card shows.").is_empty(), "an ordinary sentence passes");
}

#[test]
fn the_share_card_says_how_often_a_report_goes_out_as_the_backend_does() {
    let gap = constant("share.rs", "MIN_GAP");
    assert_eq!(gap, 86400.0, "ShareDialog.qml says a report goes out about once a day; MIN_GAP in hondascan/src/share.rs must stay one day");
    assert!(source(QML, "ShareDialog.qml").contains("about once a day"), "ShareDialog.qml must say how often reports go out");
}
