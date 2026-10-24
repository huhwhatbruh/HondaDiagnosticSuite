//! Gauge scales and healthy bands per signal, in the signal's own unit (ported from hondascan-app/src/lib/ranges.ts).
//! Unknown symbols fall back to a scale by unit, with no band. Every band says where it comes from.
use crate::format::convert;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Tone { Data, Warn, Fault }

pub struct Range { pub scale: (f64, f64), bands: &'static [(f64, Tone)], above: Tone }

// VB: engine running, Honda's charging control commands 12.5-14.5 V over LIN (import-car.com, "Honda LIN Line
// Controlled Charging Systems") and runs it low on purpose at idle; VB reads under that target. Measured on the 2015
// Accord, session 115 (2026-10-18): median 12.70 V while running, 0.6 % of readings under 12.0 V and 2.0 % under 12.2 V,
// mostly stopped at idle on a sound system, so normal starts at 12.0 V (was 12.2). Below 11.8 V the battery is discharging; above 15.0 V
// overcharging.
const VB: Range = Range { scale: (10.0, 15.0), bands: &[(11.8, Tone::Fault), (12.0, Tone::Warn), (14.9, Tone::Data), (15.0, Tone::Warn)], above: Tone::Fault };
// ECT: cold is normal while warming up, so only heat is coloured: roughly 80-100 C is healthy; above 105 C hot, above
// 115 C overheating (typical figures for water-cooled petrol engines; no Honda service-manual figure found yet).
const ECT: Range = Range { scale: (-40.0, 140.0), bands: &[(105.0, Tone::Data), (115.0, Tone::Warn)], above: Tone::Fault };
// TATF: CVT fluid roughly 70-100 C; above 110 C ages the fluid, above 125 C is overheating (typical figures).
const TATF: Range = Range { scale: (-40.0, 140.0), bands: &[(110.0, Tone::Data), (125.0, Tone::Warn)], above: Tone::Fault };

/// Scale for a signal, in display units: its own entry, else by unit.
pub fn scale_for(symbol: &str, unit: &str) -> Option<(f64, f64, Option<&'static Range>)> {
    let own = match symbol { "VB" => Some(&VB), "ECT" => Some(&ECT), "TATF" => Some(&TATF), _ => None };
    if let Some(r) = own {
        return Some((convert(r.scale.0, unit).0, convert(r.scale.1, unit).0, Some(r)));
    }
    let s = match convert(0.0, unit).1.as_str() {
        "rpm" => (0.0, 8000.0), "mph" | "MPH" => (0.0, 150.0), "°F" => (-40.0, 280.0), // display units: convert() made km/h and °C imperial
        "V" => (0.0, 16.0), "%" => (0.0, 100.0), "psi" => (0.0, convert(120.0, "kPa").0), // intake pressure: 0-120 kPa
        _ => return None,
    };
    Some((s.0, s.1, None))
}

/// How a value (source units) sits in its range: Data (healthy or no band), Warn or Fault.
pub fn tone_of(r: Option<&Range>, value: Option<f64>) -> Tone {
    let (Some(r), Some(v)) = (r, value) else { return Tone::Data };
    r.bands.iter().find(|(limit, _)| v < *limit).map_or(r.above, |(_, t)| *t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scales_are_in_display_units_and_bands_in_source_units() {
        let (lo, hi, band) = scale_for("ECT", "degC").unwrap();
        assert_eq!((lo, hi), (-40.0, 284.0));
        assert_eq!(tone_of(band, Some(100.0)), Tone::Data);
        assert_eq!(tone_of(band, Some(110.0)), Tone::Warn);
        assert_eq!(tone_of(band, Some(120.0)), Tone::Fault);
        assert_eq!(tone_of(band, None), Tone::Data);
        let vb = scale_for("VB", "V").unwrap().2;
        // 12.0 V is a sound system's idle dip (session 115), so it is normal now; 11.9 V is the warning band
        assert_eq!([11.0, 11.9, 12.0, 13.8, 14.95, 15.5].map(|v| tone_of(vb, Some(v))), [Tone::Fault, Tone::Warn, Tone::Data, Tone::Data, Tone::Warn, Tone::Fault]);
        assert_eq!(scale_for("VSP", "km/h").map(|s| (s.0, s.1)), Some((0.0, 150.0)));
        assert!(scale_for("X", "Ohm").is_none());
    }
}
