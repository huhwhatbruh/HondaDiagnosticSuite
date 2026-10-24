//! Display of values: imperial units only (the owner's choice, as in the web app), number formatting.
pub const MI: f64 = 0.621371;
pub const GAL: f64 = 3.785411784;
pub const PSI_PER_KPA: f64 = 0.145037738;

pub fn convert(v: f64, unit: &str) -> (f64, String) {
    match unit {
        "km/h" => (v * MI, "mph".into()),
        "degC" => (v * 1.8 + 32.0, "°F".into()),
        "km" | "Km" => (v * MI, "mi".into()),
        "Litre" => (v / GAL, "gal".into()),
        "RPM" => (v, "rpm".into()),
        "degF" => (v, "°F".into()),
        // HDS's spellings of units that need no conversion, written the usual way (the value is untouched)
        "deg" => (v, "°".into()),
        "deg/s" => (v, "°/s".into()),
        "N_m" => (v, "N·m".into()),
        "micros" => (v, "µs".into()),
        "Minute" => (v, "min".into()),
        "Hour" => (v, "h".into()),
        "TIMES" => (v, "times".into()),
        "mile" => (v, "mi".into()),
        "Inch" => (v, "in".into()),
        "MPH" => (v, "mph".into()),
        "Lux" => (v, "lx".into()),
        "kPa" | "kPaG" => (v * PSI_PER_KPA, "psi".into()),
        "MPa" => (v * 1000.0 * PSI_PER_KPA, "psi".into()),
        "bar" => (v * 100.0 * PSI_PER_KPA, "psi".into()),
        "hPa" | "mBar" => (v * 0.1 * PSI_PER_KPA, "psi".into()),
        "m/s2" => (v, "m/s²".into()),
        "l/min" => (v, "L/min".into()),
        "m3/h" => (v, "m³/h".into()),
        "mm3/s" => (v, "mm³/s".into()),
        "gal_us/sec" => (v, "gal/s".into()),
        "Kg" => (v, "kg".into()),
        u => (v, u.to_string()),
    }
}

/// 3 digits of precision, no more: whole numbers from 100 up, one decimal from 10, two below.
pub fn fmt(v: f64) -> String {
    let a = v.abs();
    let d = if a >= 100.0 || v.fract() == 0.0 { 0 } else if a >= 10.0 { 1 } else { 2 };
    group(&format!("{v:.d$}"))
}

fn group(s: &str) -> String {
    let (int, frac) = s.split_once('.').map_or((s, ""), |(a, b)| (a, b));
    let neg = int.starts_with('-');
    let digits: Vec<char> = int.trim_start_matches('-').chars().collect();
    let mut out = String::new();
    for (i, c) in digits.iter().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) { out.push(','); }
        out.push(*c);
    }
    format!("{}{}{}{}", if neg { "-" } else { "" }, out, if frac.is_empty() { "" } else { "." }, frac)
}

/// Honda's wording is sometimes ALL CAPITALS: sentence-case those, keep short words and anything with a digit.
pub fn tidy(s: &str) -> String {
    if s.is_empty() || s != s.to_uppercase() || !s.chars().collect::<Vec<_>>().windows(4).any(|w| w.iter().all(|c| c.is_ascii_uppercase())) { return s.to_string(); }
    const KEEP: [&str; 6] = ["EVAP", "VTEC", "PGM-FI", "SRS", "VSA", "CAN"];
    let out: Vec<String> = s.split(' ').map(|w| if w.len() < 4 || w.chars().any(|c| c.is_ascii_digit()) || KEEP.contains(&w) { w.to_string() } else { w.to_lowercase() }).collect();
    let o = out.join(" ");
    let mut c = o.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

pub fn state_on(s: &str) -> bool {
    ["ON", "YES", "OK", "OPEN", "ACTIVE", "ENGAGED", "READY", "NORMAL", "COMPLETE", "PASS"].iter().any(|p| s.to_uppercase().starts_with(p))
}
pub fn state_off(s: &str) -> bool {
    ["OFF", "NO", "CLOSE", "INACTIVE", "NOT", "---"].iter().any(|p| s.to_uppercase().starts_with(p))
}

pub fn clock(s: f64) -> String {
    let (h, m, sec) = ((s / 3600.0) as u64, ((s % 3600.0) / 60.0) as u64, (s % 60.0) as u64);
    if h > 0 { format!("{h}:{m:02}:{sec:02}") } else { format!("{m}:{sec:02}") }
}

/// A signal's value as text and unit.
pub fn display(v: Option<&serde_json::Value>, unit: &str) -> (String, String) {
    match v {
        None | Some(serde_json::Value::Null) => ("—".into(), String::new()),
        Some(serde_json::Value::String(s)) => (s.clone(), String::new()),
        Some(x) => x.as_f64().map_or(("—".into(), String::new()), |n| { let (c, u) = convert(n, unit); (fmt(c), u) }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn numbers() {
        assert_eq!(fmt(3829.4), "3,829");
        assert_eq!(fmt(18.0), "18");
        assert_eq!(fmt(7.5), "7.50");
        assert_eq!(fmt(95.26), "95.3");
        assert_eq!(fmt(-1234.0), "-1,234");
        assert_eq!(convert(100.0, "degC"), (212.0, "°F".to_string()));
        assert_eq!(clock(3725.0), "1:02:05");
        assert_eq!(convert(14.5, "deg"), (14.5, "°".to_string()));
        assert_eq!(convert(2.0, "N_m"), (2.0, "N·m".to_string()));
        let (psi, u) = convert(235.0, "kPa");
        assert!((psi - 34.08).abs() < 0.01 && u == "psi", "a tire at 235 kPa is 34.1 psi");
        assert!((convert(1.0, "bar").0 - 14.504).abs() < 0.001 && (convert(1.0, "MPa").0 - 145.04).abs() < 0.01);
        assert_eq!(convert(5.0, "V"), (5.0, "V".to_string()), "units that are already written normally pass through");
    }
}
