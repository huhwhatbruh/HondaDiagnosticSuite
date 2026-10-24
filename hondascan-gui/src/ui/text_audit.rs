//! Guards for the words on the cards. Two kinds of mistake have crept in: wording left over from how the project got here
//! ("observed, not Honda's stored tables" on the engine maps card, "not a factory specification" on snapshots: true, but about
//! the research, not about the card), and numbers quoted in a sentence that the backend has since changed. So: a phrase list
//! that must not appear in a string the person reads, and the quoted numbers checked against the backend's own constants.
use std::fs;
use std::path::Path;

const UI: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/src/ui");
const BACKEND: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../hondascan/src");

/// Words about where the data did not come from. A card says what it shows, not what it is not.
const STALE: &[&str] = &["not Honda", "Honda’s stored", "Honda's stored", "stored tables", "factory spec", "base map", "basemap"];

fn source(dir: &str, name: &str) -> String { fs::read_to_string(Path::new(dir).join(name)).unwrap_or_else(|e| panic!("{name}: {e}")) }

/// `pub const NAME: f64 = 70.0;` -> 70.0
fn constant(file: &str, name: &str) -> f64 {
    let src = source(BACKEND, file);
    let line = src.lines().find(|l| l.contains(&format!("pub const {name}:"))).unwrap_or_else(|| panic!("{file} has no {name}"));
    line.split('=').nth(1).and_then(|v| v.split(';').next()).and_then(|v| v.trim().parse().ok()).unwrap_or_else(|| panic!("{name} does not parse: {line}"))
}

#[test]
fn no_card_text_carries_leftover_wording() {
    let mut hits = Vec::new();
    for entry in fs::read_dir(UI).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if name == "text_audit.rs" || !name.ends_with(".rs") { continue; }
        // the text before this file's own tests: strings in tests describe cases, not cards
        let src = fs::read_to_string(&path).unwrap();
        let body = src.split("#[cfg(test)]").next().unwrap_or("");
        for (n, line) in body.lines().enumerate() {
            let t = line.trim_start();
            if t.starts_with("//") || !line.contains('"') { continue; }
            for bad in STALE { if line.to_lowercase().contains(&bad.to_lowercase()) { hits.push(format!("{name}:{}: “{bad}”", n + 1)); } }
        }
    }
    assert!(hits.is_empty(), "wording about what the data is not, in a string the person reads (say what the card shows instead):\n{}", hits.join("\n"));
}

#[test]
fn the_maps_card_names_the_warm_engine_threshold_the_backend_uses() {
    let c = constant("maps.rs", "WARM");
    let both = format!("({c:.0} °C, {:.0} °F)", c * 9.0 / 5.0 + 32.0);
    assert!(source(UI, "maps.rs").contains(&both), "ui/maps.rs must say {both}: the warm-engine limit in hondascan/src/maps.rs");
}

#[test]
fn the_drive_card_quotes_the_fuel_constants_and_the_economy_minimum_the_backend_uses() {
    let (afr, density) = (constant("trip.rs", "AFR"), constant("trip.rs", "DENSITY"));
    let drive = source(UI, "drive.rs");
    assert!(drive.contains(&format!("airflow ÷ {afr} ÷ {density} g/cc")), "ui/drive.rs fuel-flow note must quote AFR {afr} and density {density} (hondascan/src/trip.rs)");
    let trip = source(BACKEND, "trip.rs");
    assert!(trip.contains("km >= 0.1") && drive.contains("needs 0.1 km of driving"), "the economy note and trip.rs `economy` must agree on the 0.1 km minimum");
}
