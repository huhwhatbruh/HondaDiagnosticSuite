//! The Condition layer: what a drive says about the engine's health, judged against this car's own earlier drives.
//! Read-only: it consumes values the car already reports; it sends nothing.
//!
//! Two halves. `Condition` is a streaming accumulator fed the same engine groups as `maps::Maps`; it keeps per-regime
//! statistics (idle, steady cruise) of the samples that pass the valid-state gate and writes them into the session summary
//! (`summary["condition"]`). `evaluate` compares one session's statistics with the car's earlier sessions and returns one
//! finding per check, each with a verdict, the evidence signals, a confidence, what to do, and its source.
//!
//! The valid-state gate is HDS's own, read from the decompiled `.umf` modules (hds-xref/decompiled):
//! - ECT at least 75 degC: `HealthCheck.umf`, `sub_1d8b3` (ValveClearance_HCHK), `PGM_FI_UTILS.EngWarmUp(..., 75)`.
//! - ECT below 100 degC: `PGMFI_INSP.umf`, `sub_29f02` (AF_IMBALANCE condition check, "ECT >= 100C" stops the test).
//! - Idle is 500 to 1000 rpm with vehicle speed 0: `HealthCheck.umf`, `sub_1d43e` (VSP <= 0, 500000 < NE < 1000000 in the
//!   call's milli-rpm unit; the bargraph there is labelled rpm).
//! - No trouble codes: the same modules call `CM.ChkDtc()` before the test runs.
//!
//! Steady cruise has no HDS counterpart (HDS runs its checks at idle); its definition below is hondascan's.
//!
//! HDS's valve-clearance check measures no lash: it holds the engine at that idle and watches the misfire counter CMFIRE
//! (`HealthCheck.umf`, `sub_1d599`, 60 s and 120 s windows). Which of its two result messages means pass is unconfirmed.
use crate::mapbaseline::MIN_SESSIONS;
use crate::value::{jnum, round_to, Named, Val};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, VecDeque};

pub const WARM_ECT: f64 = 75.0;
pub const HOT_ECT: f64 = 100.0;
pub const IDLE_RPM: (f64, f64) = (500.0, 1000.0);
pub const CRUISE_KMH: f64 = 40.0; // hondascan's choice: at or above this, with the speed steady
pub const CRUISE_BAND: f64 = 3.0; // km/h: largest speed range over CRUISE_SPAN for "steady" (hondascan's choice)
pub const CRUISE_SPAN: f64 = 4.0; // s
const FRESH: f64 = 2.0; // s: an engine speed or vehicle speed this old still names the regime
const VTC_PAIR: f64 = 1.0; // s: the intake VTC command and angle ride in different DIDs (same pairing as maps.rs)
pub const MIN_N: u64 = 30; // fewest in-gate samples before a mean is used
pub const MIN_N_SPREAD: u64 = 60; // and before a spread (idle rpm roughness, stuck-value test)
pub const MIN_IDLE_S: f64 = 60.0; // idle seconds before a misfire rate is used (HDS's shorter window is 60 s)
pub const Z_WATCH: f64 = 3.0; // same convention as mapbaseline.rs
pub const Z_CONCERN: f64 = 6.0;
pub const BARO_RANGE: (f64, f64) = (60.0, 110.0); // kPa: sea level is about 101; 4,000 m is about 62
const BARO_MIN_SESSIONS: usize = 8;
const BARO_MIN_SPAN: f64 = 5.0; // kPa
const BARO_R: f64 = 0.7;

/// (feature, label, unit, spread floor, two-sided). The floors are one signal step where the converter gives one
/// (KLAFAVE raw/128, VTC raw*0.5 deg per converter 2514) and hondascan's choice otherwise.
const FEATURES: [(&str, &str, &str, f64, bool); 12] = [
    ("idle.ne_sd", "Engine-speed spread at idle", "rpm", 3.0, false),
    ("idle.klafave", "Long-term fuel trim at idle", "ratio", 1.0 / 128.0, true),
    ("idle.klaf", "Short-term fuel trim at idle", "ratio", 0.01, true),
    ("idle.knrtd", "Knock retard at idle", "deg", 0.5, false),
    ("idle.baro", "Barometric pressure", "kPa", 1.0, true),
    ("idle.vtc_dev", "Intake cam angle off its command at idle", "deg", 0.5, false),
    ("idle.misfire_per_min", "Misfires per minute at idle", "1/min", 0.2, false),
    ("cruise.klafave", "Long-term fuel trim at cruise", "ratio", 1.0 / 128.0, true),
    ("cruise.klaf", "Short-term fuel trim at cruise", "ratio", 0.01, true),
    ("cruise.knrtd", "Knock retard at cruise", "deg", 0.5, false),
    ("cruise.vtc_dev", "Intake cam angle off its command at cruise", "deg", 0.5, false),
    ("cruise.exvtc_dev", "Exhaust cam angle off its command at cruise", "deg", 0.5, false),
];

fn spec(name: &str) -> &'static (&'static str, &'static str, &'static str, f64, bool) {
    FEATURES.iter().find(|f| f.0 == name).expect("known feature")
}

/// Running mean and spread (Welford).
#[derive(Default, Clone, Copy)]
struct Run {
    n: u64,
    mean: f64,
    m2: f64,
}

impl Run {
    fn add(&mut self, x: f64) {
        self.n += 1;
        let d = x - self.mean;
        self.mean += d / self.n as f64;
        self.m2 += d * (x - self.mean);
    }
    fn sd(&self) -> f64 {
        if self.n < 2 { 0.0 } else { (self.m2 / (self.n - 1) as f64).sqrt() }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Regime {
    Idle,
    Cruise,
}

pub struct Condition {
    last: BTreeMap<&'static str, (f64, f64)>, // watched symbol -> (t, value)
    speeds: VecDeque<(f64, f64)>,
    runs: BTreeMap<&'static str, Run>,
    idle_ne: Run,
    klaf_idle: Run,
    seconds: [f64; 2], // idle, cruise
    t_prev: Option<f64>,
    counters: BTreeMap<String, f64>,
    misfires: BTreeMap<u32, u64>, // idle misfires by cylinder
}

const WATCHED: [&str; 12] = ["NE", "ECT", "VSP", "FC", "KLAF", "KLAFAVE", "KNRTD", "VTCCMD", "VTCANGL", "BARO", "EXVTCCMDEA", "EXVTCABSAEA"];
const MISFIRE: [&str; 3] = ["NMCYLOBS", "NMFACYL", "NMFBCYL"];

impl Default for Condition {
    fn default() -> Self {
        Condition::new()
    }
}

impl Condition {
    pub fn new() -> Condition {
        Condition {
            last: BTreeMap::new(),
            speeds: VecDeque::new(),
            runs: BTreeMap::new(),
            idle_ne: Run::default(),
            klaf_idle: Run::default(),
            seconds: [0.0; 2],
            t_prev: None,
            counters: BTreeMap::new(),
            misfires: BTreeMap::new(),
        }
    }

    fn fresh(&self, sym: &str, t: f64) -> Option<f64> {
        self.last.get(sym).filter(|(ts, _)| t - ts <= FRESH).map(|(_, v)| *v)
    }

    fn regime(&self, t: f64) -> Option<Regime> {
        let ect = self.last.get("ECT")?.1;
        if !(WARM_ECT..HOT_ECT).contains(&ect) || self.last.get("FC").is_some_and(|f| f.1 == 1.0) {
            return None;
        }
        let (ne, vsp) = (self.fresh("NE", t)?, self.fresh("VSP", t)?);
        if vsp <= 0.0 && ne > IDLE_RPM.0 && ne < IDLE_RPM.1 {
            return Some(Regime::Idle);
        }
        let steady = self.speeds.len() >= 3
            && self.speeds.back()?.0 - self.speeds.front()?.0 >= CRUISE_SPAN
            && self.speeds.iter().map(|s| s.1).fold(f64::MIN, f64::max) - self.speeds.iter().map(|s| s.1).fold(f64::MAX, f64::min) <= CRUISE_BAND;
        (vsp >= CRUISE_KMH && steady).then_some(Regime::Cruise)
    }

    fn put(&mut self, name: &'static str, x: f64) {
        self.runs.entry(name).or_default().add(x);
    }

    /// One engine data group (the same call as `Maps::feed`).
    pub fn feed(&mut self, t: f64, named: &Named) {
        let num = |k: &str| named.get(k).and_then(Val::num);
        for sym in WATCHED {
            if sym == "FC" {
                if let Some(f) = named.get("FC") {
                    self.last.insert("FC", (t, f64::from(f.is_on())));
                }
            } else if let Some(v) = num(sym) {
                self.last.insert(sym, (t, v));
            }
        }
        if let Some(v) = num("VSP") {
            self.speeds.push_back((t, v));
            while self.speeds.front().is_some_and(|s| t - s.0 > CRUISE_SPAN + 1.0) {
                self.speeds.pop_front();
            }
        }
        let regime = self.regime(t);
        if let (Some(r), Some(tp)) = (regime, self.t_prev) {
            self.seconds[usize::from(r == Regime::Cruise)] += (t - tp).clamp(0.0, FRESH);
        }
        self.t_prev = Some(t);
        // misfire counters: a rise counts for the regime it happens in (idle only is kept)
        for (sym, v) in named {
            let Some(n) = v.num() else { continue };
            let Some(pre) = MISFIRE.iter().find(|m| sym.starts_with(**m)) else { continue };
            let prev = self.counters.insert(sym.clone(), n);
            if let (Some(p), Some(Regime::Idle)) = (prev, regime) {
                if n > p {
                    let cyl: u32 = sym[pre.len()..].parse().unwrap_or(0);
                    *self.misfires.entry(cyl).or_default() += (n - p) as u64;
                }
            }
        }
        let Some(r) = regime else { return };
        let tag = |idle: &'static str, cruise: &'static str| if r == Regime::Idle { idle } else { cruise };
        if let Some(ne) = num("NE") {
            if r == Regime::Idle {
                self.idle_ne.add(ne);
            }
        }
        if let Some(x) = num("KLAF") {
            if r == Regime::Idle {
                self.klaf_idle.add(x);
            }
            self.put(tag("idle.klaf", "cruise.klaf"), x);
        }
        for (sym, idle, cruise) in [("KLAFAVE", "idle.klafave", "cruise.klafave"), ("KNRTD", "idle.knrtd", "cruise.knrtd")] {
            if let Some(x) = num(sym) {
                self.put(tag(idle, cruise), x);
            }
        }
        if let (Some(x), Regime::Idle) = (num("BARO"), r) {
            self.put("idle.baro", x);
        }
        // intake VTC: command and angle share converter 2514 (raw*0.5-20 deg), so the difference needs no unit arithmetic
        if let (Some(cmd), Some((ta, ang))) = (num("VTCCMD"), self.last.get("VTCANGL").copied()) {
            if (t - ta).abs() <= VTC_PAIR {
                self.put(tag("idle.vtc_dev", "cruise.vtc_dev"), (ang - cmd).abs());
            }
        }
        if let (Some(cmd), Some(act), Regime::Cruise) = (num("EXVTCCMDEA"), num("EXVTCABSAEA"), r) {
            self.put("cruise.exvtc_dev", (act - cmd).abs());
        }
    }

    /// The session's statistics, JSON-ready. `dtc_free`: no confirmed or pending code during the session; `codes`: the codes present
    /// when the summary is written (kept so `inference` can tie them to this drive's data).
    pub fn out(&self, dtc_free: bool, codes: &[Value]) -> Value {
        let mut features = Map::new();
        for (name, run) in &self.runs {
            features.insert((*name).into(), json!({"v": jnum(round_to(run.mean, 4)), "n": run.n}));
        }
        if self.idle_ne.n > 0 {
            features.insert("idle.ne_sd".into(), json!({"v": jnum(round_to(self.idle_ne.sd(), 3)), "n": self.idle_ne.n}));
        }
        let total: u64 = self.misfires.values().sum();
        if self.seconds[0] >= MIN_IDLE_S {
            features.insert("idle.misfire_per_min".into(), json!({"v": jnum(round_to(total as f64 / (self.seconds[0] / 60.0), 3)), "n": self.seconds[0].round() as u64}));
        }
        let by_cyl: Map<String, Value> = self.misfires.iter().map(|(c, n)| (c.to_string(), json!(n))).collect();
json!({"schema": 1, "dtc_free": dtc_free, "idle_s": round_to(self.seconds[0], 1), "cruise_s": round_to(self.seconds[1], 1),
               "codes": codes.iter().map(|c| json!({"code": c["code"], "kind": c["kind"], "system": c["system"], "description": c["description"]})).collect::<Vec<_>>(),
               "misfires_idle": total, "misfire_by_cyl": by_cyl, "idle_klaf_sd": round_to(self.klaf_idle.sd(), 4), "idle_klaf_n": self.klaf_idle.n,
               "features": features})
    }
}

// ---- evaluation ---------------------------------------------------------------------------------------------------

fn median(xs: &mut [f64]) -> f64 {
    xs.sort_by(f64::total_cmp);
    let n = xs.len();
    if n % 2 == 1 { xs[n / 2] } else { (xs[n / 2 - 1] + xs[n / 2]) / 2.0 }
}

fn value(session: &Value, name: &str, min_n: u64) -> Option<f64> {
    let f = session.get("features")?.get(name)?;
    (f["n"].as_u64()? >= min_n).then(|| f["v"].as_f64()).flatten()
}

fn usable(session: &Value) -> bool {
    session["dtc_free"].as_bool() == Some(true)
}

fn min_n(name: &str) -> u64 {
    if name.ends_with("ne_sd") { MIN_N_SPREAD } else if name.ends_with("misfire_per_min") { 1 } else { MIN_N }
}

/// One feature against the car's history: robust z = (now - median) / max(1.4826 * MAD, floor); needs MIN_SESSIONS earlier
/// usable sessions with the feature filled.
fn evidence(name: &str, now: &Value, priors: &[Value]) -> Option<Value> {
    let (_, label, unit, floor, two) = *spec(name);
    let x = value(now, name, min_n(name))?;
    let mut hist: Vec<f64> = priors.iter().filter(|p| usable(p)).filter_map(|p| value(p, name, min_n(name))).collect();
    let k = hist.len();
    let mut e = json!({"feature": name, "signal": label, "unit": unit, "now": jnum(round_to(x, 3)), "samples": now["features"][name]["n"], "sessions": k,
                       "two_sided": two});
    if k >= MIN_SESSIONS {
        let m = median(&mut hist);
        let mut dev: Vec<f64> = hist.iter().map(|h| (h - m).abs()).collect();
        let spread = (1.4826 * median(&mut dev)).max(floor);
        e["baseline"] = jnum(round_to(m, 3));
        e["spread"] = jnum(round_to(spread, 3));
        e["z"] = jnum(round_to((x - m) / spread, 2));
    }
    Some(e)
}

pub(crate) fn level(e: &Value) -> u8 {
    let Some(z) = e["z"].as_f64() else { return 0 };
    let z = if e["two_sided"].as_bool() == Some(true) { z.abs() } else { z };
    if z > Z_CONCERN { 2 } else if z > Z_WATCH { 1 } else { 0 }
}

/// hondascan's fixed limits (maps.rs / alerts.rs), used only for a feature with fewer than MIN_SESSIONS earlier sessions.
pub(crate) fn fixed_limit(e: &Value) -> bool {
    let x = e["now"].as_f64().unwrap_or(0.0);
    match e["feature"].as_str().unwrap_or("") {
        n if n.ends_with("klafave") => (x - 1.0).abs() > crate::maps::TRIM_FLAG,
        n if n.ends_with("knrtd") => x > crate::maps::KNOCK_FLAG,
        n if n.ends_with("vtc_dev") => x > crate::maps::VTC_FLAG,
        _ => false,
    }
}

struct Check {
    id: &'static str,
    title: &'static str,
    features: &'static [&'static str],
    why: &'static str,
    action: &'static str,
    source: &'static str,
}

const CHECKS: [Check; 4] = [
    Check {
        id: "vtc",
        title: "Cam timing tracking",
        features: &["idle.vtc_dev", "cruise.vtc_dev", "cruise.exvtc_dev"],
        why: "the cam angle follows the PCM's command less closely than it did; a sticking VTC actuator, low oil pressure or a drifted cam learn all show this",
        action: "repeat the drive to see whether it persists; if it does, HDS's VTC test and E-VTC learn are the vendor's next step (not run from this layer)",
        source: "VTCCMD/VTCANGL (DIDs 0x2662/0x2612, converter 2514) and EXVTCCMDEA/EXVTCABSAEA (DID 0x266C); HDS PGMFI_INSP.umf VTC_TEST, EX_VTC_TEST, EVTCLEARN",
    },
    Check {
        id: "fuel_trim",
        title: "Fuel trim",
        features: &["idle.klafave", "cruise.klafave", "idle.klaf", "cruise.klaf"],
        why: "the PCM is correcting the fuel more than it did; a long-term trim above 1.00 means it adds fuel (air leak, dirty airflow sensor, weak fuel delivery), below 1.00 it removes fuel (leaking injector, rich sensor error). A shift at idle but not at cruise points at an air leak (a hypothesis)",
        action: "check for intake air leaks and the airflow sensor; repeat the drive before acting",
        source: "KLAF and KLAFAVE (DID 0x2611, raw/128); HDS PGMFI_INSP.umf af_imbalance entry 0x2908e",
    },
    Check {
        id: "misfire",
        title: "Misfire and idle roughness",
        features: &["idle.misfire_per_min", "idle.ne_sd"],
        why: "the engine counts more misfires, or its idle speed is less steady, than before; worn spark plugs or a weak coil are the usual causes (a hypothesis: the data does not name the part)",
        action: "if the counts sit on one cylinder, inspect that cylinder's plug and coil first",
        source: "misfire counters NMCYLOBSn, NMFACYLn, NMFBCYLn (DIDs 0x266C, 0x2663), NE (DID 0x2610); HDS PGMFI_INSP.umf misfire_7_73",
    },
    Check {
        id: "knock",
        title: "Knock retard",
        features: &["idle.knrtd", "cruise.knrtd"],
        why: "the PCM is pulling more spark timing for knock at light load than it did; carbon build-up, low-octane fuel or hot running can cause it",
        action: "note the fuel used; if it persists on good fuel, look for carbon deposits or cooling problems",
        source: "KNRTD (DID 0x2662, converter 1860)",
    },
];

fn confidence(sessions: usize, samples: u64, flagged: usize) -> &'static str {
    if flagged >= 2 && sessions >= 10 && samples >= 100 { "high" } else if sessions >= MIN_SESSIONS { "medium" } else { "low" }
}

fn verdict_name(l: u8) -> &'static str {
    ["normal", "watch", "concern"][l as usize]
}

fn run_check(c: &Check, now: &Value, priors: &[Value]) -> Value {
    let ev: Vec<Value> = c.features.iter().filter_map(|f| evidence(f, now, priors)).collect();
    if ev.is_empty() {
        return json!({"id": c.id, "title": c.title, "verdict": "insufficient", "confidence": "none", "saw": "not enough in-gate samples of this signal in this drive",
                      "why": c.why, "action": "drive again with a warm idle of at least a minute and a steady cruise", "evidence": [], "source": c.source});
    }
    let mut lvl = ev.iter().map(level).max().unwrap_or(0);
    let flagged_hist = ev.iter().filter(|e| level(e) > 0).count();
    let mut flagged_fixed = vec![];
    for e in &ev {
        if e.get("z").is_none() && fixed_limit(e) {
            lvl = lvl.max(1);
            flagged_fixed.push(e);
        }
    }
    if c.id == "misfire" && now["misfires_idle"].as_u64().unwrap_or(0) >= crate::alerts::MISFIRE_WARN as u64 && ev.iter().all(|e| e.get("z").is_none()) {
        lvl = lvl.max(1);
    }
    let sessions = ev.iter().map(|e| e["sessions"].as_u64().unwrap_or(0) as usize).max().unwrap_or(0);
    let samples = ev.iter().map(|e| e["samples"].as_u64().unwrap_or(0)).max().unwrap_or(0);
    let conf = if lvl == 0 { confidence(sessions, samples, 0) } else { confidence(sessions, samples, flagged_hist) };
    let saw = ev
        .iter()
        .map(|e| match e.get("z") {
            Some(z) => format!("{}: {} {} now, {} {} before (spread {}), z {}, {} earlier drives", e["signal"].as_str().unwrap_or(""), e["now"], e["unit"].as_str().unwrap_or(""),
                               e["baseline"], e["unit"].as_str().unwrap_or(""), e["spread"], z, e["sessions"]),
            None => format!("{}: {} {} now, {} earlier drive{} (3 needed; hondascan's fixed limit applies)", e["signal"].as_str().unwrap_or(""), e["now"],
                            e["unit"].as_str().unwrap_or(""), e["sessions"], if e["sessions"] == 1 { "" } else { "s" }),
        })
        .collect::<Vec<_>>()
        .join("; ");
    let mut f = json!({"id": c.id, "title": c.title, "verdict": verdict_name(lvl), "confidence": conf, "saw": saw, "evidence": ev, "source": c.source});
    if lvl > 0 {
        f["why"] = json!(c.why);
        f["action"] = json!(c.action);
        if c.id == "misfire" {
            if let Some((cyl, n)) = now["misfire_by_cyl"].as_object().and_then(|m| m.iter().filter_map(|(k, v)| Some((k, v.as_u64()?))).max_by_key(|(_, n)| *n)) {
                let total = now["misfires_idle"].as_u64().unwrap_or(0).max(1);
                f["saw"] = json!(format!("{}; {} of {} idle misfires on cylinder {}", f["saw"].as_str().unwrap_or(""), n, total, cyl));
            }
        }
    } else {
        f["why"] = json!("nothing to act on");
        f["action"] = json!("none");
    }
    f
}

fn pearson(xs: &[f64], ys: &[f64]) -> Option<f64> {
    let n = xs.len() as f64;
    let (mx, my) = (xs.iter().sum::<f64>() / n, ys.iter().sum::<f64>() / n);
    let (mut sxy, mut sxx, mut syy) = (0.0, 0.0, 0.0);
    for (x, y) in xs.iter().zip(ys) {
        sxy += (x - mx) * (y - my);
        sxx += (x - mx).powi(2);
        syy += (y - my).powi(2);
    }
    (sxx > 0.0 && syy > 0.0).then(|| sxy / (sxx * syy).sqrt())
}

fn baro_check(now: &Value, priors: &[Value]) -> Value {
    let source = "BARO (DID 0x2610, kPa); HDS DTC P0069 (barometric / manifold pressure correlation) is Honda's own check of this sensor";
    let Some(b) = value(now, "idle.baro", MIN_N) else {
        return json!({"id": "baro", "title": "Barometric compensation", "verdict": "insufficient", "confidence": "none", "saw": "no barometric reading at warm idle in this drive",
                      "why": "the PCM corrects air density from this sensor", "action": "drive again with a warm idle", "evidence": [], "source": source});
    };
    let ev = json!({"feature": "idle.baro", "signal": "Barometric pressure", "unit": "kPa", "now": jnum(round_to(b, 1)), "samples": now["features"]["idle.baro"]["n"]});
    let mk = |verdict: &str, conf: &str, saw: String, why: &str, action: &str| {
        json!({"id": "baro", "title": "Barometric compensation", "verdict": verdict, "confidence": conf, "saw": saw, "why": why, "action": action, "evidence": [ev.clone()], "source": source})
    };
    if !(BARO_RANGE.0..=BARO_RANGE.1).contains(&b) {
        return mk("concern", "medium", format!("barometric pressure {b:.1} kPa is outside {}-{} kPa, which no road on earth reaches", BARO_RANGE.0, BARO_RANGE.1),
                  "a barometric sensor that reads wrongly leaves the PCM correcting air density against a false number", "check the sensor and its wiring");
    }
    let mut pairs: Vec<(f64, f64)> = priors.iter().filter(|p| usable(p)).filter_map(|p| Some((value(p, "idle.baro", MIN_N)?, value(p, "idle.klafave", MIN_N)?))).collect();
    if let (Some(t), true) = (value(now, "idle.klafave", MIN_N), usable(now)) {
        pairs.push((b, t));
    }
    let span = pairs.iter().map(|p| p.0).fold(f64::MIN, f64::max) - pairs.iter().map(|p| p.0).fold(f64::MAX, f64::min);
    if pairs.len() < BARO_MIN_SESSIONS || span < BARO_MIN_SPAN {
        return mk("normal", "low", format!("barometric pressure {b:.1} kPa, in range; whether the PCM corrects for it needs {BARO_MIN_SESSIONS} drives spanning {BARO_MIN_SPAN} kPa (have {} drives, {span:.1} kPa)", pairs.len()),
                  "nothing to act on", "none");
    }
    let (xs, ys): (Vec<f64>, Vec<f64>) = pairs.into_iter().unzip();
    match pearson(&xs, &ys) {
        Some(r) if r.abs() >= BARO_R => mk("watch", "low", format!("the long-term trim moves with barometric pressure (r {r:.2} over {} drives spanning {span:.1} kPa)", xs.len()),
                                          "a trim that follows pressure suggests the correction for air density is incomplete (a hypothesis; weather also moves with load and temperature)", "repeat on drives at different pressures before acting"),
        Some(r) => mk("normal", "low", format!("the long-term trim does not follow barometric pressure (r {r:.2} over {} drives spanning {span:.1} kPa): the PCM's correction holds", xs.len()), "nothing to act on", "none"),
        None => mk("normal", "low", "barometric pressure in range; the trim or pressure did not vary enough to compare".into(), "nothing to act on", "none"),
    }
}

fn stuck_check(now: &Value) -> Value {
    let n = now["features"]["idle.ne_sd"]["n"].as_u64().unwrap_or(0);
    let sd = now["features"]["idle.ne_sd"]["v"].as_f64();
    let kn = now["idle_klaf_n"].as_u64().unwrap_or(0);
    let ksd = now["idle_klaf_sd"].as_f64();
    let mut stuck = vec![];
    if n >= MIN_N_SPREAD && sd == Some(0.0) {
        stuck.push(format!("engine speed read exactly the same {n} times at idle"));
    }
    if kn >= MIN_N_SPREAD && ksd == Some(0.0) {
        stuck.push(format!("short-term fuel trim read exactly the same {kn} times at idle"));
    }
    let source = "NE (DID 0x2610), KLAF (DID 0x2611): hondascan's rule, not Honda's";
    if stuck.is_empty() {
        let enough = n >= MIN_N_SPREAD || kn >= MIN_N_SPREAD;
        return json!({"id": "stuck", "title": "Frozen readings", "verdict": if enough { "normal" } else { "insufficient" }, "confidence": if enough { "medium" } else { "none" },
                      "saw": if enough { "engine speed and trim move between reads" } else { "too few idle samples" }, "why": "nothing to act on", "action": "none", "evidence": [], "source": source});
    }
    json!({"id": "stuck", "title": "Frozen readings", "verdict": "concern", "confidence": "medium", "saw": stuck.join("; "),
           "why": "a running engine's speed and a closed-loop trim never repeat one value this long; the value is frozen at the adapter, the gateway or the sensor", "action": "reconnect the adapter and read again; if it repeats, suspect the sensor path",
           "evidence": [], "source": source})
}

/// Valve-lash suspicion. A hypothesis only: no lash measurement exists in the data (HDS's valve-clearance check holds the
/// idle and watches the misfire counter; it reads no clearance). It is raised only when knock retard and (cam tracking or
/// misfire) are all out of the car's own range together; it is never raised from one signal and never above "watch".
fn lash_hypothesis(findings: &[Value]) -> Value {
    let v = |id: &str| findings.iter().find(|f| f["id"] == id).map_or("insufficient", |f| f["verdict"].as_str().unwrap_or(""));
    let hit = |id: &str| matches!(v(id), "watch" | "concern");
    let raised = hit("knock") && (hit("vtc") || hit("misfire"));
    json!({"id": "valve_lash", "title": "Valve lash (hypothesis)", "hypothesis": true, "verdict": if raised { "watch" } else { "normal" }, "confidence": "low",
           "saw": if raised { "knock retard and cam tracking or misfire are all outside this car's own range together" } else { "no combination of knock, cam tracking and misfire outside this car's range" },
           "why": if raised { "worn or mis-set valve lash can change cam timing response and combustion together, but so can many other faults; nothing here measures lash" } else { "nothing to act on" },
           "action": if raised { "treat as a hint only; the vendor's procedure is a feeler-gauge check of the clearances" } else { "none" },
           "evidence": [], "source": "no lash signal exists; HDS HealthCheck.umf ValveClearance_HCHK (valveclearance.$CODE entry 0x1d33a) is an idle-hold gate that watches CMFIRE"})
}

/// Findings for one session's `condition` JSON against the car's earlier sessions' (any order). With active trouble codes the
/// drive is not part of any baseline and HDS runs no check, so the checks are read as corroboration of the codes only
/// (`corroboration_only`); `inference::interpret` then adds the code findings and the cross-signal patterns.
pub fn evaluate(now: &Value, priors: &[Value]) -> Value {
    let gate_ok = usable(now);
    let used = priors.iter().filter(|p| usable(p)).count();
    let mut findings: Vec<Value> = CHECKS.iter().map(|c| run_check(c, now, priors)).collect();
    findings.push(baro_check(now, priors));
    findings.push(stuck_check(now));
    let lash = lash_hypothesis(&findings);
    findings.push(lash);
    let more = crate::inference::interpret(now, &findings);
    if !gate_ok {
        // with codes set, the trend findings are context for the codes, so only the code findings are kept besides them
        findings.retain(|f| f["verdict"] != "normal" && f["verdict"] != "insufficient");
    }
    findings.extend(more);
    let unlinked = crate::inference::unlinked_codes(now);
    json!({"gate": {"dtc_free": gate_ok, "idle_s": now["idle_s"], "cruise_s": now["cruise_s"],
                    "rule": "ECT 75-100 degC; idle 500-1000 rpm at 0 km/h (HDS ValveClearance_HCHK, AF_IMBALANCE); steady cruise from 40 km/h; no trouble codes"},
           "earlier_drives": used, "needed": MIN_SESSIONS, "findings": findings, "corroboration_only": !gate_ok,
           "note": if gate_ok { Value::Null } else { json!(format!("Trouble codes are set: this drive is not added to the baseline, and its data is read only to see whether it agrees with the codes{}",
                if unlinked > 0 { format!(" ({unlinked} active code{} have no signal here to check)", if unlinked == 1 { "" } else { "s" }) } else { String::new() })) }})
}

/// The findings for one recorded session (the car's newest with statistics when `session` is None) against that car's
/// earlier sessions. An unknown VIN has no history, so it is refused: statistics of different cars never mix.
pub fn for_session(store: &crate::store::Store, vin: Option<&str>, session: Option<i64>) -> Result<Value, String> {
    let vin = vin.map(String::from).or_else(|| store.last_real_vin());
    let (sid, vin, now) = store
        .condition_of(vin.as_deref(), session)
        .ok_or("no recorded session with condition statistics: drive with a build that has this layer, or pass --vin / --session")?;
    if vin.is_empty() || vin == "UNKNOWN" {
        return Err("that session has no VIN, so there is no history to compare with".into());
    }
    let priors = store.prior_condition(&vin, sid, 30);
    let mut out = evaluate(&now, &priors);
    out["session"] = json!(sid);
    out["vin"] = json!(vin);
    Ok(out)
}

/// The findings as plain text, one block each.
pub fn render(report: &Value) -> String {
    let mut o = format!("Condition: session {}, {} earlier usable drive(s), {} needed for a history verdict\n", report["session"], report["earlier_drives"], report["needed"]);
    o += &format!("Idle {} s, steady cruise {} s in the valid state ({})\n", report["gate"]["idle_s"], report["gate"]["cruise_s"], report["gate"]["rule"].as_str().unwrap_or(""));
    if let Some(n) = report["note"].as_str() {
        o += &format!("{n}\n");
    }
    for f in report["findings"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
        let g = |k: &str| f[k].as_str().unwrap_or("").to_string();
        o += &format!("\n[{}] {} ({} confidence){}\n  saw:    {}\n  why:    {}\n  do:     {}\n  source: {}\n", g("verdict").to_uppercase(), g("title"), g("confidence"),
                      if f["hypothesis"] == true { " HYPOTHESIS" } else { "" }, g("saw"), g("why"), g("action"), g("source"));
    }
    o
}
