//! Live warnings: what is wrong with the engine right now, from the last WINDOW seconds of what it reports.
//! Ported from hondascan/alerts.py (hondascan's rules of thumb, not Honda's limits), then the knock and misfire rules
//! were deliberately changed (see README.md, "Deliberate deviations"), so the alert golden is a stability check now.
use crate::value::{pysum, Named, Val};
use std::collections::{BTreeMap, VecDeque};

pub const WINDOW: f64 = 10.0;
pub const HOLD: f64 = 8.0; // an alert stays this long after its condition last held
pub const KNOCK_WARN: f64 = 2.0;
pub const KNOCK_FAULT: f64 = 6.0;
pub const KNOCK_HITS: usize = 3;
// Knock retard spiking during one brief hard acceleration is the knock control doing its job, so mild knock alerts
// only when it is sustained: at least this share of the window's samples are knocking.
pub const KNOCK_SHARE: f64 = 0.5;
// Heavy knock (at or above KNOCK_FAULT) alerts at a lower window share than mild knock: sustained heavy retard is the
// damaging case, but one hard-acceleration burst (about a third of a window at ~3 reads/s) still stays below it.
pub const KNOCK_FAULT_SHARE: f64 = 0.4;
// g/rev (GAIR / revolutions per second, the maps' load axis): while the engine pulls this hard or harder anywhere in the
// window, knock retard is the knock control working and no knock alert is raised; sustained knock at lighter load
// (cruise, idle) is the abnormal case. Measured on the 2015 Accord, session 115 (2026-10-18, 54 min): 90 % of the
// samples with KNRTD >= 2 deg were at >= 0.6 g/rev (median 0.97), and 1,233 of its 1,283 samples at >= 6 deg.
pub const KNOCK_LOAD: f64 = 0.6;
pub const KNOCK_GAP: f64 = 1.5;
pub const FRESH: f64 = 2.0; // s: an engine speed from another group this recent still names the rpm of a misfire counter's rise
pub const ECT_WARN: f64 = 105.0;
pub const ECT_FAULT: f64 = 115.0;
// V, the 10 s mean of the ECU's own VB while running. Honda's PCM commands 12.5-14.5 V over LIN and runs it low on
// purpose at idle, and VB reads below that target: on the 2015 Accord, session 115 (2026-10-18), healthy 10 s means went
// down to 11.99 V, mostly stopped at idle (VB median 12.70 V while running), so the old 12.2 V limit fired 8 times on a
// sound system. 11.8 V is under every healthy window measured and is where the app's VB gauge already marks discharging.
pub const VB_LOW: f64 = 11.8;
pub const VB_HIGH: f64 = 15.0;
pub const TRIM: f64 = 0.15;
// Misfires counted on one cylinder this session: a stray one or two happen on healthy engines (the 2015 Accord counted
// one on cylinder 1 in a clean 30-min drive, 2026-10-18), and the PCM only sets P030x after many. Rules of thumb.
pub const MISFIRE_WARN: f64 = 5.0;
pub const MISFIRE_FAULT: f64 = 20.0;
const MISFIRE: [&str; 3] = ["NMCYLOBS", "NMFACYL", "NMFBCYL"];
const CYL: [&str; 4] = ["KNRTD1", "KNRTD2", "KNRTD3", "KNRTD4"];
const WATCHED: [&str; 8] = ["KNRTD", "KNRTD1", "KNRTD2", "KNRTD3", "KNRTD4", "ECT", "VB", "KLAFAVE"];

#[derive(Clone, Debug, PartialEq)]
pub struct Alert {
    pub id: String,
    pub level: &'static str,
    pub title: String,
    pub text: String,
}

#[derive(Default)]
pub struct Alerts {
    win: BTreeMap<&'static str, VecDeque<(f64, f64)>>, // symbol -> (t, value)
    misfire: Vec<(String, (f64, f64))>,                  // counter symbol -> (count of rises so far, last value seen)
    rises: BTreeMap<u32, Vec<Option<f64>>>,              // cylinder -> rpm at each rise of one of its counters
    t: Option<f64>,
    ne: Option<(f64, f64)>, // (t, rpm) of the last engine speed seen in any group: the car reports it apart from the counters
    load: VecDeque<(f64, f64)>, // (t, g/rev) over the last WINDOW, from NE and GAIR (one group on the 2015 Accord, 0x2610)
    running: bool,
    held: Vec<(String, (f64, Alert))>, // id -> (data time it last held, alert), in insertion order
}

impl Alerts {
    pub fn new() -> Alerts {
        Alerts { win: WATCHED.iter().map(|s| (*s, VecDeque::new())).collect(), ..Default::default() }
    }

    pub fn feed(&mut self, t: f64, named: &Named) {
        self.t = Some(t);
        if let Some(n) = named.get("NE").and_then(Val::num) {
            self.ne = Some((t, n));
            self.running = n > 400.0;
            if let Some(g) = named.get("GAIR").and_then(Val::num).filter(|_| n > 400.0) {
                self.load.push_back((t, g / (n / 60.0))); // the maps' load, g per revolution
            }
        }
        let ne = self.ne.filter(|(u, _)| t - u <= FRESH).map(|(_, n)| n);
        for s in WATCHED {
            if let Some(v) = named.get(s).and_then(Val::num) {
                self.win.get_mut(s).unwrap().push_back((t, v));
            }
        }
        let mut syms: Vec<&String> = named.keys().collect();
        syms.sort(); // Python walks the dict in insertion order; the result does not depend on it
        for s in syms {
            if !MISFIRE.iter().any(|m| s.starts_with(m)) {
                continue;
            }
            let (Some(v), Some(cyl)) = (named[s].num(), s.chars().last().and_then(|c| c.to_digit(10))) else { continue };
            let Some(i) = self.misfire.iter().position(|(k, _)| k == s) else {
                self.misfire.push((s.clone(), (0.0, v))); // first sight: remember the value, nothing counted yet
                continue;
            };
            let (count, last) = self.misfire[i].1;
            if v > last {
                // a rise adds to this counter's running total, so a windowed counter that blips 0,1,0,1 counts 2, not 1
                // (NMCYLOBS is cumulative and counts its net increase); name an rpm once, only when the rise lifts the
                // cylinder's count -- one misfire shows on two or three of its counters, read at different times.
                let before = self.cyl_count(cyl);
                self.misfire[i].1 = (count + v - last, v);
                if self.cyl_count(cyl) > before {
                    self.rises.entry(cyl).or_default().push(ne);
                }
            } else if v < last {
                self.misfire[i].1 .1 = v; // the counter fell back (its window reset): measure the next rise from here
            }
        }
        for q in self.win.values_mut().chain(std::iter::once(&mut self.load)) {
            while q.front().is_some_and(|(u, _)| *u < t - WINDOW) {
                q.pop_front();
            }
        }
    }

    /// This cylinder's misfire count so far: the largest running total among its counters. A single misfire can show
    /// on NMCYLOBS and on NMFACYL/NMFBCYL, so the counters are compared, not added together.
    fn cyl_count(&self, cyl: u32) -> f64 {
        self.misfire
            .iter()
            .filter(|(k, _)| k.chars().last().and_then(|c| c.to_digit(10)) == Some(cyl))
            .map(|(_, (count, _))| *count)
            .fold(0.0, f64::max)
    }

    /// {cylinder: misfires counted this session}, the cylinder's count where it is above zero.
    pub fn misfires(&self) -> BTreeMap<u32, f64> {
        let mut out = BTreeMap::new();
        for (s, (count, _)) in &self.misfire {
            if *count > 0.0 {
                let cyl = s.chars().last().and_then(|c| c.to_digit(10)).unwrap_or(0);
                let e = out.entry(cyl).or_insert(0.0);
                *e = f64::max(*e, *count);
            }
        }
        out
    }

    /// The rpm at which a cylinder's misfire count rose (those that were known): the engine speed when the rise was read,
    /// which can be some seconds after the misfire itself (the counters are polled slowly).
    pub fn rise_rpms(&self, cyl: u32) -> Vec<f64> {
        self.rises.get(&cyl).map(|v| v.iter().flatten().filter(|r| **r != 0.0).cloned().collect()).unwrap_or_default()
    }

    fn vals(&self, s: &str) -> Vec<f64> {
        self.win[s].iter().map(|(_, v)| *v).collect()
    }

    /// The knock retard of a signal when it is worth an alert: (level, window mean, window peak). Alerts when knock is
    /// *sustained* -- KNOCK_HITS samples at or above KNOCK_WARN over at least KNOCK_SHARE of the window, so one brief
    /// spike under hard throttle (the knock control doing its job) is not enough -- or when it is *heavy*: KNOCK_HITS
    /// samples at or above KNOCK_FAULT over KNOCK_FAULT_SHARE of the window, a lower bar than mild knock, because
    /// sustained heavy retard is the damaging case (cloud review, 2026-10-18) while one hard-acceleration burst stays
    /// below it.
    fn knocking(&self, s: &str) -> Option<(&'static str, f64, f64)> {
        let v = self.vals(s);
        if v.is_empty() {
            return None;
        }
        let n = v.len() as f64;
        let warn_hits = v.iter().filter(|x| **x >= KNOCK_WARN).count();
        let fault_hits = v.iter().filter(|x| **x >= KNOCK_FAULT).count();
        let sustained = warn_hits >= KNOCK_HITS && warn_hits as f64 >= KNOCK_SHARE * n;
        let heavy = fault_hits >= KNOCK_HITS && fault_hits as f64 >= KNOCK_FAULT_SHARE * n;
        if !(sustained || heavy) {
            return None;
        }
        let mean = pysum(&v) / n;
        let peak = v.iter().cloned().fold(f64::MIN, f64::max);
        Some((if heavy || mean >= KNOCK_FAULT { "fault" } else { "warn" }, mean, peak))
    }

    /// The alerts that hold now (`now` in the same clock as feed's t), worst first.
    pub fn active(&mut self, now: f64) -> Vec<Alert> {
        let Some(t) = self.t.filter(|t| now - t <= WINDOW) else {
            self.held.clear();
            return vec![];
        };
        let mut out: Vec<Alert> = vec![];
        let mut add = |id: &str, level: &'static str, title: &str, text: String| {
            out.push(Alert { id: id.into(), level, title: title.into(), text });
        };

        // Knock while the engine pulls hard is the knock control working (KNOCK_LOAD), so only knock at lighter load
        // alerts. A car that reports no GAIR gives no load: then every sustained or heavy knock alerts, as before.
        let pulling = self.load.iter().any(|(_, l)| *l >= KNOCK_LOAD);
        let at = if self.load.is_empty() { "" } else { " at light load" };
        let mut peaks: Vec<(usize, (&'static str, f64, f64))> = vec![];
        for (i, s) in CYL.iter().enumerate() {
            if let Some(k) = self.knocking(s).filter(|_| !pulling) {
                peaks.push((i, k));
            }
        }
        if !peaks.is_empty() {
            let low = CYL.iter().map(|s| self.vals(s).into_iter().reduce(f64::max).unwrap_or(0.0)).fold(f64::MAX, f64::min);
            for (i, (level, mean, p)) in &peaks {
                let n = i + 1;
                let odd = if p - low >= KNOCK_GAP && peaks.len() < CYL.len() { " (the odd one out)" } else { "" };
                add(&format!("knock{n}"), level, &format!("Cylinder {n} keeps knocking"),
                    format!("Cylinder {n} keeps knocking{at}{odd}: the PCM is pulling {mean:.1}° on average, up to {p:.1}°. Have it checked."));
            }
        } else if let Some((level, mean, p)) = self.knocking("KNRTD").filter(|_| !pulling) {
            add("knock", level, "The engine keeps knocking",
                format!("The engine keeps knocking{at}: the PCM is pulling {mean:.1}° on average, up to {p:.1}°. Have it checked."));
        }
        let ect = self.vals("ECT");
        if let Some(m) = ect.iter().cloned().reduce(f64::max).filter(|m| *m > ECT_WARN) {
            add("coolant", if m > ECT_FAULT { "fault" } else { "warn" }, "Coolant is hot", format!("Coolant is {m:.0} °C."));
        }
        let vb = self.vals("VB");
        if !vb.is_empty() && self.running {
            let mean = pysum(&vb) / vb.len() as f64;
            if !(VB_LOW..=VB_HIGH).contains(&mean) {
                add("charging", "warn", "Charging voltage is off", format!("Charging voltage is {mean:.2} V (expected {VB_LOW}-{VB_HIGH} V)."));
            }
        }
        let counts = self.misfires();
        if let Some(worst) = counts.values().cloned().reduce(f64::max).filter(|w| *w >= MISFIRE_WARN) {
            let what: Vec<String> = counts
                .iter()
                .map(|(c, n)| {
                    let rpms = self.rise_rpms(*c);
                    let at = if rpms.is_empty() {
                        String::new()
                    } else {
                        format!(" (at {} rpm)", rpms.iter().map(|r| format!("{r:.0}")).collect::<Vec<_>>().join(", "))
                    };
                    format!("cylinder {c}: {n:.0}{at}")
                })
                .collect();
            add("misfire", if worst >= MISFIRE_FAULT { "fault" } else { "warn" }, "Misfires counted",
                format!("Misfires this session, {}. Worth a look at the plugs and coils.", what.join("; ")));
        }
        let trim = self.vals("KLAFAVE");
        if !trim.is_empty() {
            let mean = pysum(&trim) / trim.len() as f64;
            if (mean - 1.0).abs() > TRIM {
                add("trim", "warn", "Fuel trim is off", format!("Long-term fuel trim is {mean:.2} (1.00 is none): the PCM is correcting hard."));
            }
        }
        for a in &out {
            match self.held.iter_mut().find(|(i, _)| *i == a.id) {
                Some(h) => h.1 = (t, a.clone()),
                None => self.held.push((a.id.clone(), (t, a.clone()))),
            }
        }
        let shown: Vec<String> = out.iter().map(|a| a.id.clone()).collect();
        out.extend(self.held.iter().filter(|(i, (u, _))| !shown.contains(i) && t - u <= HOLD).map(|(_, (_, a))| a.clone()));
        self.held.retain(|(_, (u, _))| t - u <= HOLD);
        let (mut faults, rest): (Vec<_>, Vec<_>) = out.into_iter().partition(|a| a.level == "fault");
        faults.extend(rest);
        faults
    }
}
