//! The Overview's car, seen straight down, nose to the right, in a 1000 x 440 frame (the first web app's CarBody coordinates):
//! the hologram model's wires as SVG path data, the body outline, and where every found system's button stands. QML draws what
//! this makes. The wires are `assets/car/car_wire.pack`, baked from the owner's model (`package/src/bin/bakecar.rs` documents the format): line segments in metres, Y up, nose along +Z, the driver's left at +X,
//! in four groups (interior, tyres and wheels, body, lamps), each with its brightness.
use crate::api::Ecu;
use crate::systems::{info, zone};
use serde_json::{json, Value};

const PACK: &[u8] = include_bytes!("../assets/car/car_wire.pack");

/// metres per stored unit
const UNIT: f32 = 1.0 / 10_000.0;
/// frame units a metre: the model's length fills x 51..949
const K: f32 = 898.0 / 5.636;
/// The car's centre line and nose direction in the frame.
const CENTRE: (f32, f32) = (500.0, 220.0);

/// The body outline, traced from a line blueprint (blueprint pixel space 1388 x 592) for the first web app's CarBody; the matrix
/// below puts it in the frame.
pub const BODY: &str = "M2 296 C2 205 26 118 110 74 C160 52 230 42 300 40 L1150 37 C1250 40 1318 78 1356 150 C1376 196 1386 250 1386 296 C1386 342 1376 396 1356 442 C1318 514 1250 552 1150 555 L300 552 C230 550 160 540 110 518 C26 474 2 387 2 296 Z";
pub const BODY_MATRIX: (f32, f32, f32, f32) = (0.6484, 50.0, 0.6484, 28.4); // x' = a x + b, y' = c y + d

pub struct Group { pub strength: f32, pub path: String }

/// The wire groups as SVG path data in frame units, one `M x y L x y` per segment, positions to a tenth of a unit.
pub fn wires() -> Option<Vec<Group>> {
    let word = |at: usize| Some(u32::from_le_bytes(PACK.get(at..at + 4)?.try_into().ok()?));
    if PACK.get(..4)? != b"CARW" { return None; }
    let n = word(4)? as usize;
    let mut counts = Vec::new();
    let mut total = 0usize;
    for g in 0..n {
        let at = 8 + g * 20;
        let count = word(at)? as usize;
        counts.push((count, f32::from_bits(word(at + 4)?)));
        total += count;
    }
    let verts = PACK.get(8 + n * 20..)?;
    if verts.len() != total * 8 { return None; }
    let point = |i: usize| {
        let v = |k: usize| i16::from_le_bytes([verts[i * 8 + k * 2], verts[i * 8 + k * 2 + 1]]) as f32 * UNIT;
        let (x, z) = (v(0), v(2));
        (CENTRE.0 + K * z, CENTRE.1 - K * x)
    };
    let mut start = 0;
    let mut out = Vec::new();
    for (count, strength) in counts {
        let mut path = String::with_capacity(count * 14);
        for seg in 0..count / 2 {
            let (a, b) = (point(start + seg * 2), point(start + seg * 2 + 1));
            path.push_str(&format!("M{:.1} {:.1}L{:.1} {:.1}", a.0, a.1, b.0, b.1));
        }
        out.push(Group { strength, path });
        start += count;
    }
    Some(out)
}

/// The systems that sit by a wheel: (zone, wheel x, wheel y in frame units, which side of the wheel the button stands, 1 toward the
/// middle of the car, -1 outside it). Both are at the front wheel on the driver's side (up on the screen): the brakes' system inside
/// it, the tire pressure's above it.
const AXLES: [(&str, f32, f32, f32); 2] = [("brakes", 771.0, 96.0, 1.0), ("tires", 771.0, 96.0, -1.0)];
const WHEEL: (f32, f32) = (124.0, 40.0);
/// Several systems at one place stand side by side this far apart.
const SPACING: f32 = 62.0;
/// The frame is shown 14 units down from where its y = 0 is (the viewBox 0 -14 1000 468).
const VIEWBOX_DROP: f32 = 14.0;

/// The first system of a zone: the one a wheel stands for.
fn owner(ecus: &[Ecu], zone_id: &str) -> Option<usize> {
    (0..ecus.len()).find(|&i| info(&ecus[i].system_type, &ecus[i].display).zone == zone_id)
}

/// Where each system's button stands, as `[{key, name, x, y}]` in frame units (the button's centre, `side` wide): the wheel
/// systems beside their wheel, every other system at its zone, several at one zone side by side.
pub fn keys_json(ecus: &[Ecu], side: f32) -> Value {
    let mut out: Vec<Value> = Vec::new();
    let mut put = |e: &Ecu, x: f32, y: f32| out.push(json!({ "key": e.key(), "name": info(&e.system_type, &e.display).name,
        "zone": info(&e.system_type, &e.display).zone, "x": x, "y": y }));
    for (z, x, y, inward) in AXLES {
        if let Some(i) = owner(ecus, z) { put(&ecus[i], x, y + inward * (WHEEL.1 / 2.0 + 6.0 + side / 2.0)); }
    }
    let mut by_zone: Vec<(String, Vec<usize>)> = Vec::new();
    for (i, e) in ecus.iter().enumerate() {
        let z = info(&e.system_type, &e.display).zone.to_string();
        if AXLES.iter().any(|a| a.0 == z && owner(ecus, a.0) == Some(i)) { continue; }
        match by_zone.iter_mut().find(|(k, _)| *k == z) { Some((_, v)) => v.push(i), None => by_zone.push((z, vec![i])) }
    }
    for (z, list) in &by_zone {
        let (zx, zy, _) = zone(z);
        for (n, &i) in list.iter().enumerate() {
            put(&ecus[i], zx + (n as f32 - (list.len() as f32 - 1.0) / 2.0) * SPACING, zy + VIEWBOX_DROP);
        }
    }
    Value::Array(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ecu(address: u32, system: &str, system_type: &str) -> Ecu { Ecu { address, system: system.into(), system_type: system_type.into(), ..Default::default() } }

    #[test]
    fn the_embedded_pack_becomes_four_groups_of_segments_inside_the_frame() {
        let g = wires().expect("pack parses");
        assert_eq!(g.len(), 4);
        let segments: usize = g.iter().map(|g| g.path.matches('M').count()).sum();
        assert_eq!(segments, 59_648, "the baked model's segment count (README: 59,648 segments)");
        assert!(g.iter().all(|g| g.strength > 0.0 && g.path.starts_with('M')));
        // every vertex lies in the 1000 x 440 frame the car is drawn in
        for grp in &g {
            for n in grp.path.split(['M', 'L']).filter(|s| !s.is_empty()).take(2000) {
                let (x, y): (f32, f32) = n.split_once(' ').map(|(a, b)| (a.parse().unwrap(), b.parse().unwrap())).unwrap();
                assert!((0.0..=1000.0).contains(&x) && (0.0..=470.0).contains(&y), "{n}");
            }
        }
    }

    #[test]
    fn wheel_systems_stand_by_their_wheel_and_the_rest_at_their_zone_side_by_side() {
        let ecus = [ecu(0x28, "CANABS", "HONDA ABS"), ecu(0x26, "CAN_TPMS", "TPMS"), ecu(0x10, "CANFI", "PGM-FI"), ecu(0x1E, "CANCVT", "CVT"), ecu(0x60, "CAN_METER", "METER")];
        let k = keys_json(&ecus, 58.0);
        let at = |key: &str| { let v = k.as_array().unwrap().iter().find(|v| v["key"] == key).unwrap(); (v["x"].as_f64().unwrap(), v["y"].as_f64().unwrap()) };
        let abs = at("40:CANABS");
        let tpms = at("38:CAN_TPMS");
        assert_eq!((abs.0, tpms.0), (771.0, 771.0));
        assert!(abs.1 > 96.0 && tpms.1 < 96.0, "the brakes' system inside the wheel, the tire pressure's above it");
        assert!((abs.1 - 96.0 - (20.0 + 6.0 + 29.0)).abs() < 1e-6, "half a wheel, a gap and half a button from the wheel's centre");
        let (eng, cvt, meter) = (at("16:CANFI"), at("30:CANCVT"), at("96:CAN_METER"));
        assert_eq!(eng, (840.0, 234.0), "the engine zone, 14 down for the viewBox");
        assert_eq!(cvt.1, 272.0 + 14.0);
        assert_eq!(meter.0, 690.0);
        // two systems in one zone: the PCM's engine and the EVPS share the engine zone
        let two = keys_json(&[ecu(0x10, "CANFI", "PGM-FI"), ecu(0x10, "CANEVPS", "DIESEL")], 58.0);
        let xs: Vec<f64> = two.as_array().unwrap().iter().map(|v| v["x"].as_f64().unwrap()).collect();
        assert_eq!(xs, vec![840.0 - 31.0, 840.0 + 31.0]);
    }
}
