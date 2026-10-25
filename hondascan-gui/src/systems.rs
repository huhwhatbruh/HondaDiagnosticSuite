//! What each HDS system type is, in plain words, and where it sits on the car drawing (ported from
//! hondascan-app/src/lib/systems.ts). The type names are HDS's (can_addresses.system_type).
pub struct Info { pub name: String, pub blurb: String, pub zone: &'static str, pub icon: &'static str }

/// Car drawing coordinates, viewBox 0 0 1000 440, nose to the right: (x, y, label above?).
pub fn zone(id: &str) -> (f32, f32, bool) {
    match id {
        "engine" => (840.0, 220.0, true), "transmission" => (745.0, 272.0, false), "brakes" => (771.0, 47.0, true),
        "motor" => (760.0, 318.0, false), "steering" => (622.0, 150.0, false), "meter" => (690.0, 160.0, true),
        "hud" => (650.0, 228.0, false), "camera" => (572.0, 236.0, false), "airbags" => (488.0, 222.0, true),
        "gateway" => (640.0, 292.0, false), "front" => (925.0, 220.0, true), "tires" => (254.0, 47.0, true),
        "parking" => (254.0, 318.0, false), "shifter" => (450.0, 290.0, false), "battery" => (340.0, 222.0, true),
        "rear" => (254.0, 222.0, true), _ => (400.0, 330.0, false),
    }
}

pub fn info(system_type: &str, display: &str) -> Info {
    let t = system_type;
    let (name, zone, blurb): (&str, &'static str, &str) = match t {
        "PGM-FI" | "DIESEL" => ("Engine", "engine", "Engine control: fuel injection, ignition, emissions."),
        "CVT" | "EAT" | "DCT" | "i-SHIFT" => ("Transmission", "transmission", "Automatic transmission control."),
        "SBW" => ("Shift-by-wire", "shifter", "Electronic gear selector."),
        "HONDA ABS" => ("ABS / VSA", "brakes", "Anti-lock brakes and stability assist modulator."),
        "ESB" => ("Electric servo brake", "brakes", "Brake-by-wire servo unit."),
        "EPB" => ("Parking brake", "parking", "Electric parking brake."),
        "EPS" => ("Power steering", "steering", "Electric power steering assist."),
        "SRS" => ("Airbags (SRS)", "airbags", "Supplemental restraint system: airbags and their sensors."),
        "EPT_L" | "EPT_R" => ("Seat-belt pretensioner", "airbags", "Motorised seat-belt pretensioner."),
        "METER" => ("Instrument cluster", "meter", "Gauges, warning lights and odometer."),
        "HUD" => ("Head-up display", "hud", "Windshield projected display."),
        "TPMS" => ("Tire pressure", "tires", "Tire pressure monitoring."),
        "CANGW" | "CANGWB" => ("Gateway", "gateway", "Links the car’s CAN networks; the diagnostic port talks through it."),
        "IDAS" | "MVCS" => ("Camera", "camera", "Camera-based driver assistance."),
        "AVAS" => ("Pedestrian alert", "front", "Acoustic vehicle alerting sound."),
        "SH-AWD" => ("SH-AWD", "rear", "Torque-vectoring all-wheel drive unit."),
        _ if t.ends_with("BAT") => ("High-voltage battery", "battery", "Hybrid / EV battery management."),
        _ if t.contains("MOT") || t.contains("MG") || t.contains("TMU") => ("Electric motor", "motor", "Hybrid / EV motor and inverter control."),
        _ => (if display.is_empty() { t } else { display }, "other", ""),
    };
    let blurb = if blurb.is_empty() { format!("HDS system type {t}.") } else { blurb.to_string() };
    Info { name: name.to_string(), blurb, zone, icon: icon_of(zone, t) }
}

/// Several systems at one address are one computer (the Accord's PCM runs the engine and the CVT at 0x0E).
pub fn shared_with(address: u32, system_type: &str, all: &[crate::api::Ecu]) -> bool {
    all.iter().any(|x| x.address == address && x.system_type != system_type)
}

/// The icon for a system's zone (Phosphor, regular weight; the web app used the Lucide equivalents).
pub fn icon_of(zone: &str, system_type: &str) -> &'static str {
    use egui_phosphor::regular as i;
    match (zone, system_type) {
        ("engine", _) => i::ENGINE, ("transmission", _) | ("shifter", _) | ("rear", _) => i::GEAR, ("brakes", _) => i::DISC,
        ("parking", _) => i::LETTER_CIRCLE_P, ("steering", _) => i::STEERING_WHEEL, ("airbags", _) => i::SHIELD_CHECK,
        ("meter", _) => i::GAUGE, ("hud", _) => i::SCAN, ("tires", _) => i::TIRE, ("gateway", _) => i::NETWORK,
        ("camera", _) => i::CAMERA, ("front", _) => i::SPEAKER_HIGH, ("battery", _) => i::BATTERY_CHARGING, ("motor", _) => i::LIGHTNING,
        (_, "?") => i::CPU, _ => i::CAR,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_types_carry_their_own_blurb_and_unknown_ones_fall_back_to_the_display_name() {
        let cvt = info("CVT", "");
        assert_eq!((cvt.name.as_str(), cvt.zone, cvt.blurb.as_str()), ("Transmission", "transmission", "Automatic transmission control."));
        assert_eq!(info("HVBAT", "").name, "High-voltage battery");
        let odd = info("XYZ", "Seat memory");
        assert_eq!((odd.name.as_str(), odd.zone, odd.blurb.as_str()), ("Seat memory", "other", "HDS system type XYZ."));
        assert_eq!(info("XYZ", "").name, "XYZ");
    }
}
