//! This drive: distance, fuel and economy. Every number comes from hondascan.trip (the backend's `status`), shown in US units.
use crate::api::Status;
use crate::format::{clock, fmt, GAL, MI};
use serde_json::{json, Value};

/// The card as QML draws it: `{note, message, stats: [{label, text, unit, hint}]}`; no stats while there is no drive to show.
pub fn drive_json(status: Option<&Status>) -> Value {
    let trip = status.filter(|s| s.recording).and_then(|s| s.trip.as_ref()).filter(|t| t.duration_s > 0.0);
    let Some((s, t)) = status.zip(trip) else {
        let asleep = status.is_some_and(|s| s.asleep);
        let message = if asleep { "The car is off. The last drive is saved in History." } else { "Starts with the first engine readings." };
        return json!({ "note": "", "message": message, "stats": [] });
    };
    let idle = if t.times_s.engine > 0.0 { (t.times_s.idle / t.times_s.engine * 100.0).round() } else { 0.0 };
    let fuel = t.fuel.engine_counter_l.or(t.fuel.maf_estimate_l);
    let dash = || "—".to_string();
    let stat = |label: &str, text: String, unit: &str, hint: &str| json!({ "label": label, "text": text, "unit": unit, "hint": hint });
    let unit_if = |known: bool, unit: &'static str| if known { unit } else { "" };
    json!({
        "note": format!("Recording {} · saved per VIN", clock(t.duration_s)),
        "message": "",
        "stats": [
            stat("Distance", fmt(t.distance_km * MI), "mi", "integral of vehicle speed over time"),
            stat("Engine time", clock(t.engine_run_s.unwrap_or(t.times_s.engine)), "", "since engine start (PCM)"),
            stat("Fuel used", fuel.map_or_else(dash, |l| format!("{:.2}", l / GAL)), unit_if(fuel.is_some(), "gal"),
                 "the engine's own fuel counter, else estimated from airflow"),
            stat("Economy", t.fuel.economy.as_ref().map_or_else(dash, |e| format!("{:.1}", e.mpg_us)), unit_if(t.fuel.economy.is_some(), "mpg"),
                 "needs 0.1 km of driving"),
            stat("Fuel flow now", s.fuel_flow_lph.map_or_else(dash, |l| format!("{:.2}", l / GAL)), unit_if(s.fuel_flow_lph.is_some(), "gal/h"),
                 "airflow ÷ 14.7 ÷ 0.745 g/cc; zero during fuel cut"),
            stat("Idle", format!("{idle}"), "%", "engine running, car standing"),
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{Economy, Fuel, Times, Trip};

    fn driving(trip: Trip) -> Status { Status { recording: true, trip: Some(trip), ..Default::default() } }

    #[test]
    fn a_drive_is_shown_in_us_units_and_what_is_unknown_is_a_dash() {
        let s = driving(Trip { duration_s: 125.0, distance_km: 10.0, times_s: Times { engine: 100.0, idle: 25.0 },
            fuel: Fuel { engine_counter_l: Some(3.785411784), ..Default::default() }, ..Default::default() });
        let j = drive_json(Some(&s));
        let stats = j["stats"].as_array().unwrap();
        let get = |l: &str| stats.iter().find(|x| x["label"] == l).unwrap();
        assert_eq!((get("Distance")["text"].as_str(), get("Distance")["unit"].as_str()), (Some("6.21"), Some("mi")));
        assert_eq!((get("Fuel used")["text"].as_str(), get("Fuel used")["unit"].as_str()), (Some("1.00"), Some("gal")));
        assert_eq!((get("Economy")["text"].as_str(), get("Economy")["unit"].as_str()), (Some("—"), Some("")), "no economy yet: a dash and no unit");
        assert_eq!(get("Idle")["text"], "25");
        assert_eq!(j["note"], "Recording 2:05 · saved per VIN");
    }

    #[test]
    fn the_fuel_counter_wins_over_the_airflow_estimate_and_economy_has_its_unit() {
        let both = driving(Trip { duration_s: 1.0, fuel: Fuel { engine_counter_l: Some(3.785411784), maf_estimate_l: Some(7.570823568),
            economy: Some(Economy { mpg_us: 31.46, ..Default::default() }) }, ..Default::default() });
        let j = drive_json(Some(&both));
        assert_eq!(j["stats"][2]["text"], "1.00", "the engine's own counter, not the estimate");
        assert_eq!((j["stats"][3]["text"].as_str(), j["stats"][3]["unit"].as_str()), (Some("31.5"), Some("mpg")));
    }

    #[test]
    fn without_a_drive_it_says_why() {
        assert_eq!(drive_json(None)["message"], "Starts with the first engine readings.");
        let asleep = Status { asleep: true, ..Default::default() };
        assert_eq!(drive_json(Some(&asleep))["message"], "The car is off. The last drive is saved in History.");
        assert!(drive_json(Some(&asleep))["stats"].as_array().unwrap().is_empty());
    }
}
