//! The adapters the person can pick: the presets, then every serial port found (ported from hondascan-gui's topbar.rs).
use crate::api::Port;

pub const PRESETS: [(&str, &str); 6] = [
    ("gd101-j2534", "GODIAG GD101 (J2534 mode, all modules)"), ("gd101", "GODIAG GD101 (ELM mode)"),
    ("elm", "ELM327 / OBDLink · auto"), ("socketcan:can0", "SocketCAN · can0"),
    ("sim:drive", "Simulated car · driving"), ("sim", "Simulated car · parked"),
];

/// (spec, label): the presets (no SocketCAN off Linux), then every port found (the GD101 offers both its modes), then "custom".
pub fn adapter_list(ports: &[Port]) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = PRESETS.iter()
        .filter(|(s, _)| cfg!(target_os = "linux") || !s.starts_with("socketcan"))
        .map(|(s, l)| (s.to_string(), l.to_string())).collect();
    for p in ports {
        if p.gd101 {
            v.push((format!("gd101-j2534:{}", p.device), format!("GD101 J2534 · {}", p.device)));
            v.push((format!("elm:{}", p.device), format!("GD101 ELM · {}", p.device)));
        } else {
            v.push((format!("elm:{}", p.device), format!("{} · {}", if p.product.is_empty() { "Serial" } else { &p.product }, p.device)));
        }
        // the same port on the K-line: older body systems (ISO 14230), or generic OBD-II of a car from before CAN
        v.push((format!("elm-kline:{}", p.device), format!("K-line (older body systems) · {}", p.device)));
        v.push((format!("elm-obd:{}", p.device), format!("OBD-II K-line (cars before ~2008) · {}", p.device)));
    }
    v.push(("custom".into(), "Custom adapter spec…".into()));
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ports_follow_the_presets_and_custom_comes_last() {
        let ports = [Port { device: "/dev/ttyACM0".into(), product: String::new(), gd101: true },
                     Port { device: "/dev/ttyUSB0".into(), product: "OBDLink".into(), gd101: false }];
        let l = adapter_list(&ports);
        let specs: Vec<&str> = l.iter().map(|(s, _)| s.as_str()).collect();
        assert_eq!(&specs[specs.len() - 8..], ["gd101-j2534:/dev/ttyACM0", "elm:/dev/ttyACM0", "elm-kline:/dev/ttyACM0", "elm-obd:/dev/ttyACM0",
            "elm:/dev/ttyUSB0", "elm-kline:/dev/ttyUSB0", "elm-obd:/dev/ttyUSB0", "custom"]);
        assert_eq!(l[l.len() - 4].1, "OBDLink · /dev/ttyUSB0");
        assert_eq!(l[l.len() - 3].1, "K-line (older body systems) · /dev/ttyUSB0");
        assert_eq!(l[l.len() - 2].1, "OBD-II K-line (cars before ~2008) · /dev/ttyUSB0");
    }
}
