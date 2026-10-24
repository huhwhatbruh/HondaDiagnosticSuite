//! A scripted drive for the simulated car, so trip, fuel, driving-style and tire maths can be checked against known
//! answers. Port of hondascan/simdrive.py: every value is encoded with the layout and scaling HDS gives for it, so the
//! decoding path is the real one; only the numbers are invented.
use std::collections::BTreeMap;
use std::f64::consts::PI;

pub const AFR: f64 = 14.7;
pub const FUEL_DENSITY: f64 = 0.745; // g/cc gasoline
pub const SOFT_TIRE: f64 = 1.005; // right-rear wheel speed factor

/// (duration s, speed at end km/h); speed changes linearly within a segment
pub const SEGMENTS: [(f64, f64); 9] = [(10., 0.), (15., 90.), (40., 90.), (5., 0.), (10., 0.), (5., 60.), (30., 60.), (12., 0.), (13., 0.)];
pub const CYCLE: f64 = 140.0;

/// km/h and acceleration in m/s^2 at time t (s) into the drive.
pub fn speed_at(t: f64) -> (f64, f64) {
    let mut t = t.rem_euclid(CYCLE); // Python's % on floats takes the divisor's sign
    let mut v0 = SEGMENTS[SEGMENTS.len() - 1].1;
    for (dur, v1) in SEGMENTS {
        if t < dur {
            return (v0 + (v1 - v0) * t / dur, (v1 - v0) / 3.6 / dur);
        }
        t -= dur;
        v0 = v1;
    }
    (v0, 0.0)
}

/// rpm, accelerator %, airflow g/s, fuel cut, for the speed profile.
pub fn engine(t: f64) -> (f64, f64, f64, bool) {
    let (v, a) = speed_at(t);
    let rpm = 800.0 + 22.0 * v + a.max(0.0) * 700.0;
    let app = if a >= -0.05 { (8.0 + a * 25.0 + v * 0.15).clamp(0.0, 100.0) } else { 0.0 };
    let fuel_cut = a < -0.05 && v > 15.0;
    (rpm, app, 2.5 + rpm * app / 3900.0, fuel_cut)
}

/// `int(round(max(0, v))).to_bytes(n, 'big')`
pub fn u(v: f64, n: usize) -> Vec<u8> {
    let x = v.max(0.0).round_ties_even() as u64;
    x.to_be_bytes()[8 - n..].to_vec()
}

const KLAF_LAMBDA: f64 = 1.0;
const LTFT: f64 = 1.04; // the long-term trim sits 4 % rich-corrected (1.04)
const KNOCK_RAW: f64 = 0.5; // KNRTD1..4 scale, converter 1860; cylinder 4 knocks 4x harder in the sim
const ENGINE_RUN_AT_START: f64 = 2839.0; // s the engine has run when the scripted drive starts (live car, 2026-10-12)

/// Overlay values per (did, offset) for each simulated system at time t since the sim started.
#[derive(Default)]
pub struct Drive {
    pub fuel: f64, // cc injected (the engine's TTRIP counter)
    pub km: f64,   // distance driven
    last: f64,
}

impl Drive {
    /// Short-term trim: 1.00 +/- 0.03, swinging with a 6 s period.
    pub fn trim(t: f64) -> f64 {
        1.0 + 0.03 * (t * 2.0 * PI / 6.0).sin()
    }

    /// Knock retard: 3 deg only while accelerating hard (a > 1 m/s^2), else none.
    pub fn knock(t: f64) -> f64 {
        if speed_at(t).1 > 1.0 { 3.0 } else { 0.0 }
    }

    /// Advance the fuel counter and odometer to t in 50 ms steps (the truth the tests compare with).
    pub fn integrate(&mut self, t: f64) {
        while self.last < t {
            let step = f64::min(0.05, t - self.last);
            let (_, _, gair, cut) = engine(self.last);
            let (v, _) = speed_at(self.last + step / 2.0);
            self.fuel += if cut { 0.0 } else { gair / AFR / FUEL_DENSITY * step };
            self.km += v * step / 3600.0;
            self.last += step;
        }
    }

    pub fn values(&mut self, system: &str, t: f64) -> BTreeMap<(u16, usize), Vec<u8>> {
        self.integrate(t);
        let (v, a) = speed_at(t);
        let mut m = BTreeMap::new();
        let mut put = |k: (u16, usize), b: Vec<u8>| {
            m.insert(k, b);
        };
        match system {
            "CANFI" => {
                let (rpm, app, gair, cut) = engine(t);
                let ect = f64::min(90.0, 40.0 + t / 6.0); // warms up over the first five minutes
                put((0x2610, 6), u(rpm / 0.25, 2));
                put((0x2610, 11), u(ect + 40.0, 1));
                put((0x2610, 13), u(25.0 + 40.0, 1));
                put((0x2610, 15), u(30.0 + app * 0.6, 1));
                put((0x2610, 18), u(v, 1));
                put((0x2610, 22), u(14.2 / 0.1, 1));
                put((0x2610, 34), u(gair / 0.01, 2));
                put((0x2612, 8), u(app / 0.005, 2));
                put((0x2611, 8), u(KLAF_LAMBDA / 0.0000305, 2));
                put((0x2611, 10), u(Self::trim(t) * 128.0, 1));
                put((0x2611, 11), u(LTFT * 128.0, 1));
                put((0x2662, 8), u(Self::knock(t) / 0.5, 1));
                put((0x2610, 21), u((15.0 + app * 0.2 - rpm / 400.0 + 64.0) / 0.5, 1));
                for i in 0..4 {
                    put((0x2694, 7 + i), u(Self::knock(t) * (if i == 3 { 2.0 } else { 0.5 }) / KNOCK_RAW, 1));
                }
                put((0x2613, 12), u(f64::max(0.0, 62.0 - self.fuel / 500.0) / 0.1, 2));
                put((0x2660, 22), vec![if cut { 0x08 } else { 0 }]);
                put((0x2660, 52), u(self.fuel / 0.1, 2));
                put((0x2660, 43), u(123456.0 + self.km, 3));
                put((0x2660, 46), u(ENGINE_RUN_AT_START + t, 2));
                put((0x2660, 32), u(72.0 / 0.1, 2));
            }
            "CANABS" => {
                let brake = a < -0.05;
                let ws = [v, v, v, v * SOFT_TIRE]; // FL FR RL RR
                let g_lon = a / 9.80665;
                put((0x4004, 15), vec![brake as u8]);
                for (i, w) in ws.iter().enumerate() {
                    put((0x4004, 8 + i), u(w / 1.5, 1));
                }
                put((0x4005, 8), u(if brake { -a * 12.0 } else { 0.0 }, 1));
                put((0x4005, 9), vec![0]);
                put((0x4005, 22), u(125.0 + g_lon / 0.02, 1));
                put((0x4005, 24), vec![125]);
                put((0x4005, 13), u(125.0 + 0.05 * (t / 7.0).sin() / 0.02, 1));
                put((0x4005, 15), vec![125]);
            }
            "CAN_METER" => put((0x7022, 6), u(123456.0 + self.km, 3)),
            "CAN_TPMS" => {
                put((0x6001, 8), u(235.0, 2));
                put((0x6001, 10), u(235.0, 2));
                put((0x6001, 12), u(205.0, 2));
                put((0x6001, 14), u(234.0, 2));
            }
            _ => {}
        }
        m
    }
}
