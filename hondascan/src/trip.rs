//! What a drive was like, computed from the values the car reports. Port of hondascan/trip.py (distance, fuel,
//! economy); the driving-score feature (acceleration/braking events, brake-application and stop counts, the
//! 0-100 score) was dropped 2026-10-19, owner's instruction -- it never had anything to validate it against and
//! duplicated the plain time-above-4500-rpm and brake-pressure stats this module already reports. The Python
//! original still has it (git history, hondascan/trip.py).
use crate::value::{jnum, jopt, round_to, Named, Val};
use serde_json::{json, Map, Value};

pub const GAP: f64 = 5.0;
pub const HIGH_RPM: f64 = 4500.0;
pub const AFR: f64 = 14.7;
pub const DENSITY: f64 = 0.745;
pub const L_PER_GAL: f64 = 3.785411784;
pub const KM_PER_MI: f64 = 1.609344;

#[derive(Default, Clone)]
pub struct Stat {
    pub n: u64,
    pub lo: f64,
    pub hi: f64,
    pub sum: f64,
}

impl Stat {
    pub fn add(&mut self, v: f64) {
        if self.n == 0 || v < self.lo {
            self.lo = v;
        }
        if self.n == 0 || v > self.hi {
            self.hi = v;
        }
        self.n += 1;
        self.sum += v;
    }

    pub fn out(&self, nd: usize) -> Value {
        if self.n == 0 {
            return Value::Null;
        }
        json!({"min": round_to(self.lo, nd), "mean": round_to(self.sum / self.n as f64, nd), "max": round_to(self.hi, nd)})
    }
}

#[derive(Default)]
struct Secs {
    engine: f64,
    idle: f64,
    moving: f64,
    high_rpm: f64,
    fuel_cut: f64,
    braking: f64,
}

#[derive(Default)]
struct Ttrip {
    first: Option<f64>,
    last: f64,
    used: f64,
}

#[derive(Default)]
pub struct Trip {
    t0: Option<f64>,
    t_last: Option<f64>,
    km: f64,
    gaps: u64,
    secs: Secs,
    pub speed: Stat,
    pub rpm: Stat,
    pub ect: Stat,
    pub vb_run: Stat,
    pub vb_off: Stat,
    pub pmc: Stat,
    pub lat: Stat,
    prev: Option<(f64, f64, f64, bool)>, // (t, vsp, ne, fc) of the last engine sample
    ttrip: Ttrip,
    maf_g: f64,
    odo: Vec<(&'static str, (f64, f64))>,
    meter_miles: bool,
    brake_on: bool,
    brake_src: Option<&'static str>,
    warm_start: Option<f64>,
    warm_at: Option<f64>,
    fc: bool,
    t_brake: Option<f64>,
    run_s: Option<f64>, // ERUNTM, the largest seen
}

impl Trip {
    pub fn new() -> Trip {
        Trip::default()
    }

    /// One engine data group. Speed and rpm come in one group, the fuel counter and fuel cut in another;
    /// each is taken from whichever read carries it.
    pub fn engine(&mut self, t: f64, v: &Named) {
        if let Some(x) = v.get("FC") {
            self.fc = x.is_text("ON");
        }
        if let Some(x) = v.get("BKSW") {
            self.brake(t, "engine", x.is_on());
        }
        let num = |k: &str| v.get(k).and_then(Val::num);
        let (ttrip, odo, gair, ect, vb) = (num("TTRIP"), num("DSTODO"), num("GAIR"), num("ECT"), num("VB"));
        if let Some(run) = num("ERUNTM") {
            if run < 65535.0 {
                self.run_s = Some(run.max(self.run_s.unwrap_or(0.0)));
            }
        }
        if let Some(c) = ttrip {
            self.counter(c);
        }
        if let Some(o) = odo {
            self.odo_add("engine", o);
        }
        let (Some(vsp), Some(ne)) = (num("VSP"), num("NE")) else { return };
        self.t0.get_or_insert(t);
        let fc = self.fc;
        if let Some((pt, pv, pne, pfc)) = self.prev {
            let dt = t - pt;
            if 0.0 < dt && dt <= GAP {
                self.km += (pv + vsp) / 2.0 * dt / 3600.0;
                if pne > 0.0 {
                    self.secs.engine += dt;
                    if pv < 1.0 { self.secs.idle += dt } else { self.secs.moving += dt }
                    self.secs.high_rpm += if pne > HIGH_RPM { dt } else { 0.0 };
                    self.secs.fuel_cut += if pfc { dt } else { 0.0 };
                }
                if let (Some(g), false) = (gair, pfc) {
                    self.maf_g += g / AFR * dt;
                }
            } else if dt > GAP {
                self.gaps += 1;
            }
        }
        self.prev = Some((t, vsp, ne, fc));
        self.speed.add(vsp);
        if ne > 0.0 {
            self.rpm.add(ne);
        }
        if let Some(e) = ect {
            self.ect.add(e);
            let start = *self.warm_start.get_or_insert(e);
            if self.warm_at.is_none() && start < 60.0 && e >= 80.0 {
                self.warm_at = Some(t);
            }
        }
        if let Some(b) = vb {
            if ne > 0.0 { self.vb_run.add(b) } else { self.vb_off.add(b) }
        }
        self.t_last = Some(t);
    }

    pub fn brakes(&mut self, t: f64, v: &Named) {
        if let Some(x) = v.get("BLSSTS") {
            self.brake(t, "abs", x.is_on());
        }
        let num = |k: &str| v.get(k).and_then(Val::num);
        if let (Some(p), true) = (num("PMC"), self.brake_on) {
            self.pmc.add(p);
        }
        if let Some(lat) = num("CGLAT") {
            self.lat.add(lat.abs());
        }
    }

    pub fn meter(&mut self, _t: f64, v: &Named) {
        if let Some(Val::Text(u)) = v.get("FNC_DISTUNIT") {
            if u == "mile" || u == "km" {
                self.meter_miles = u == "mile";
            }
        }
        if let Some(o) = v.get("OUT_ODO").and_then(Val::num) {
            self.odo_add("meter", o);
        }
    }

    fn brake(&mut self, t: f64, src: &'static str, on: bool) {
        if self.brake_src.is_some_and(|s| s != src) {
            return; // one switch only: two sources would interleave
        }
        self.brake_src = Some(src);
        if let (true, Some(tb)) = (self.brake_on, self.t_brake) {
            if 0.0 < t - tb && t - tb <= GAP {
                self.secs.braking += t - tb;
            }
        }
        self.brake_on = on;
        self.t_brake = Some(t);
    }

    fn counter(&mut self, cc: f64) {
        let tt = &mut self.ttrip;
        match tt.first {
            None => tt.first = Some(cc),
            Some(_) if cc >= tt.last => tt.used += cc - tt.last,
            Some(_) => tt.used += cc, // counter reset (new driving cycle)
        }
        tt.last = cc;
    }

    fn odo_add(&mut self, src: &'static str, km: f64) {
        match self.odo.iter_mut().find(|(s, _)| *s == src) {
            Some((_, (_, hi))) => *hi = km,
            None => self.odo.push((src, (km, km))),
        }
    }

    fn odo_km(&self) -> Value {
        let mut m = Map::new();
        for (k, (lo, hi)) in &self.odo {
            let mut d = hi - lo;
            if self.meter_miles && *k == "meter" {
                d = round_to(d * KM_PER_MI, 1);
            }
            m.insert((*k).into(), jnum(d));
        }
        Value::Object(m)
    }

    pub fn summary(&self) -> Value {
        let km = self.km;
        let fuel_l = Some(self.ttrip.used / 1000.0).filter(|x| *x != 0.0); // 0: counter absent or stuck
        let est_l = Some(self.maf_g).filter(|x| *x != 0.0).map(|g| g / DENSITY / 1000.0);
        let s = &self.secs;
        json!({
            "duration_s": self.t0.map_or(json!(0), |t0| jnum(round_to(self.t_last.unwrap_or(t0) - t0, 1))),
            "distance_km": round_to(km, 3), "odometer_km": self.odo_km(),
            "gaps": self.gaps,
            "times_s": {"engine": round_to(s.engine, 1), "idle": round_to(s.idle, 1), "moving": round_to(s.moving, 1),
                        "high_rpm": round_to(s.high_rpm, 1), "fuel_cut": round_to(s.fuel_cut, 1), "braking": round_to(s.braking, 1)},
            "engine_run_s": jopt(self.run_s),
            "fuel": {"engine_counter_l": jopt(fuel_l.map(|l| round_to(l, 3))),
                     "maf_estimate_l": jopt(est_l.filter(|x| *x != 0.0).map(|l| round_to(l, 3))),
                     "economy": economy(km, fuel_l.or(est_l)),
                     "economy_source": if fuel_l.is_some() { json!("engine counter (TTRIP)") }
                                       else if est_l.is_some() { json!("MAF estimate") } else { Value::Null }},
            "speed_kmh": self.speed.out(1), "rpm": self.rpm.out(0), "coolant_c": self.ect.out(1),
            "warmup_s": self.warm_at.map_or(Value::Null, |w| jnum(round_to(w - self.t0.unwrap_or(w), 1))),
            "battery_v": {"engine_running": self.vb_run.out(2), "engine_off": self.vb_off.out(2)},
            "brake_pressure_bar": self.pmc.out(1), "lateral_g": self.lat.out(2),
        })
    }
}

pub fn economy(km: f64, litres: Option<f64>) -> Value {
    match litres {
        Some(l) if l != 0.0 && km >= 0.1 => json!({
            "km_per_l": round_to(km / l, 2), "l_per_100km": round_to(l / km * 100.0, 2),
            "mpg_us": round_to((km / KM_PER_MI) / (l / L_PER_GAL), 1)}),
        _ => Value::Null,
    }
}

/// Instantaneous fuel flow from MAF (estimate): g/s of air / AFR / density -> litres per hour.
pub fn fuel_flow_lph(gair: f64, fuel_cut: bool) -> f64 {
    if fuel_cut { 0.0 } else { gair / AFR / DENSITY * 3.6 }
}
