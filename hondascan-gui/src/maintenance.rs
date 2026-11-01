//! The maintenance screens' pure logic, moved from hondascan-gui/src/ui/maintenance.rs: freezing a healthy baseline (an
//! immutable record of what the car reports now) and adding a service marker (a note about work done). The screens using
//! it come later. Nothing here draws, and nothing here sends anything to the car.

/// The marker kinds the backend accepts (rpc.rs `marker_add`); no free text, so the history stays sortable.
pub const KINDS: [&str; 9] = ["oil", "tires", "brakes", "battery", "filter", "fluid", "inspection", "repair", "other"];

/// 1 mile = 1.609344 km exactly; markers are stored in kilometres and shown in miles.
pub const KM_PER_MILE: f64 = 1.609344;

pub fn miles_to_km(miles: f64) -> f64 { miles * KM_PER_MILE }

/// A kind as it is shown: "oil" is "Oil".
pub fn kind_label(kind: &str) -> String {
    let mut c = kind.chars();
    c.next().map_or_else(|| "Other".to_string(), |f| f.to_uppercase().collect::<String>() + c.as_str())
}

/// A checked marker, ready to send: the note trimmed, the mileage in miles (the app converts it to km).
#[derive(Clone, PartialEq, Debug)]
pub struct MarkerInput { pub kind: String, pub note: String, pub mileage_miles: Option<f64> }

/// What is wrong with a marker, or the marker itself. A note is required and at most 2000 characters; the mileage, when
/// given, is a number of miles (commas accepted, as people write them) and never negative.
pub fn check_marker(kind: &str, note: &str, mileage: &str) -> Result<MarkerInput, String> {
    if !KINDS.contains(&kind) { return Err("Pick a kind of work.".into()); }
    let note = note.trim();
    if note.is_empty() { return Err("A note is required: say what was done.".into()); }
    if note.chars().count() > 2000 { return Err("The note is longer than 2000 characters.".into()); }
    let typed = mileage.trim().replace([',', '_'], "");
    let miles = if typed.is_empty() { None } else {
        let m: f64 = typed.parse().map_err(|_| "The mileage must be a number of miles, e.g. 82,400.".to_string())?;
        if !m.is_finite() || m < 0.0 { return Err("The mileage cannot be negative.".into()); }
        Some(m)
    };
    Ok(MarkerInput { kind: kind.to_string(), note: note.to_string(), mileage_miles: miles })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_note_and_kind_are_required_and_the_note_is_trimmed() {
        let m = check_marker("oil", "  5W-30 and filter  ", "").unwrap();
        assert_eq!(m.note, "5W-30 and filter");
        assert_eq!(m.kind, "oil");
        assert!(m.mileage_miles.is_none(), "no mileage typed means none sent");
        assert!(check_marker("oil", "   ", "").is_err());
        assert!(check_marker("wax", "Something", "").is_err(), "only the backend's kinds are offered");
    }

    #[test]
    fn the_note_is_capped_at_2000_characters() {
        let ok = "ä".repeat(2000);
        assert!(check_marker("repair", &ok, "").is_ok(), "2000 characters is allowed (umlauts count once)");
        assert!(check_marker("repair", &format!("{ok}ä"), "").is_err(), "2001 is not");
        assert!(check_marker("repair", "", "").unwrap_err().contains("note"));
    }

    #[test]
    fn mileage_is_read_as_miles_and_commas_are_accepted() {
        assert_eq!(check_marker("tires", "Four new", "82,400").unwrap().mileage_miles, Some(82400.0));
        assert_eq!(check_marker("tires", "Four new", " 0 ").unwrap().mileage_miles, Some(0.0));
        assert!(check_marker("tires", "Four new", "soon").is_err());
        assert!(check_marker("tires", "Four new", "-1").is_err(), "a negative odometer is a typo");
        assert_eq!(miles_to_km(100.0), 160.9344, "markers are stored in kilometres");
    }

    #[test]
    fn kinds_are_named_in_the_dialog_and_the_history_the_same_way() {
        assert_eq!(kind_label("oil"), "Oil");
        assert_eq!(kind_label("tires"), "Tires");
        assert_eq!(kind_label(""), "Other");
        assert_eq!(KINDS.len(), 9);
        assert!(KINDS.iter().all(|k| kind_label(k) != *k), "every kind gets a readable name");
    }
}
