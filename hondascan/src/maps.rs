//! Observed engine maps: what the PCM did at each engine speed and load, built from the values it reports (not Honda's
//! stored calibration tables). Port of hondascan/maps.py, which documents the axes, signals and the rules of thumb.
use crate::mapbaseline::{Baseline, Cell, Cells, CYL, MIN_SESSIONS};
use crate::value::{jnum, pysum, round_to, Named, Val};
use serde_json::{json, Map, Value};

const SIGNALS: [(&str, &str, &str); 9] = [
    ("IG", "Spark advance", "deg"),
    ("KLAF", "ST fuel trim", "ratio"),
    ("KLAFAVE", "LT fuel trim", "ratio"),
    ("LAMBDA", "Lambda", "λ"),
    ("KNRTD", "Knock retard", "deg"),
    ("KNRTD1", "Knock retard, cylinder 1", "deg"),
    ("KNRTD2", "Knock retard, cylinder 2", "deg"),
    ("KNRTD3", "Knock retard, cylinder 3", "deg"),
    ("KNRTD4", "Knock retard, cylinder 4", "deg"),
];
const FUEL: [&str; 3] = ["KLAF", "KLAFAVE", "LAMBDA"];
const NOTE_ORDER: [&str; 7] = ["KLAFAVE", "IG", "LAMBDA", "KNRTD", "KNRTD1", "KNRTD2", "KNRTD3"]; // + KNRTD4 below
pub const RPM: [f64; 12] = [500.0, 1000.0, 1500.0, 2000.0, 2500.0, 3000.0, 3500.0, 4000.0, 4500.0, 5000.0, 5500.0, 6000.0];
pub const WARM: f64 = 70.0;
pub const FRESH: f64 = 2.0;
const VTC_PAIR: f64 = 1.0; // s: the intake VTC signals ride in two DIDs that are read apart, so their read times may differ by at most this to pair
pub const MIN_RPM: f64 = 400.0;
pub const TRIM_FLAG: f64 = 0.10;
pub const KNOCK_FLAG: f64 = 2.0;
pub const MIN_CELL: f64 = 20.0;
pub const CYL_GAP: f64 = 0.5; // deg: whole-session mean retard one cylinder shows above the lowest
pub const VTC_FLAG: f64 = 5.0; // deg: mean |actual - target| over the session that is worth a note
pub const MIN_VTC_SAMPLES: u64 = 20; // fewest samples before a VTC mean is worth a note

/// Running mean/max of |actual - target| for one factory-map health check (intake or exhaust VTC).
#[derive(Default, Clone, Copy)]
struct Deviation {
    n: u64,
    sum_abs: f64,
    max_abs: f64,
}

impl Deviation {
    fn add(&mut self, actual: f64, target: f64) {
        let d = (actual - target).abs();
        self.n += 1;
        self.sum_abs += d;
        self.max_abs = self.max_abs.max(d);
    }

    fn mean_abs(&self) -> f64 {
        self.sum_abs / self.n as f64
    }
}

/// The load axis, upper edges 0.2 ... 1.4 g/rev, computed the way Python does (0.2 * i) so the edges are identical.
pub fn load() -> [f64; 7] {
    std::array::from_fn(|i| 0.2 * (i + 1) as f64)
}

/// A cell's range in words: bins are named by their upper edge, and the last bin also holds everything above.
pub fn band(r: usize, l: usize) -> String {
    let load = load();
    let rpm = if r == RPM.len() - 1 { format!("above {}", RPM[RPM.len() - 2]) } else { format!("{}-{}", RPM[r] - 500.0, RPM[r]) };
    let ld = if l == load.len() - 1 { format!("above {:.1}", load[load.len() - 2]) } else { format!("{:.1}-{:.1}", load[l] - 0.2, load[l]) };
    format!("{rpm} rpm, {ld} g/rev")
}

pub fn bin_of(v: f64, edges: &[f64]) -> usize {
    edges.iter().position(|e| v <= *e).unwrap_or(edges.len() - 1)
}

fn fixed(sym: &str, m: f64) -> Option<bool> {
    match sym {
        "KLAFAVE" => Some((m - 1.0).abs() > TRIM_FLAG),
        "KNRTD" | "KNRTD1" | "KNRTD2" | "KNRTD3" | "KNRTD4" => Some(m > KNOCK_FLAG),
        _ => None, // IG and LAMBDA are only judged against history
    }
}

pub struct Maps {
    base: Baseline,
    cells: Vec<(&'static str, Cells)>, // symbol -> cells in the order they were first filled (summation order matters)
    seen: Vec<(&'static str, u64)>,
    ne: Option<f64>,
    gair: Option<f64>,
    ect: Option<f64>,
    fc: bool,
    t_ne: Option<f64>,
    warm: f64,
    min_cell: f64,
    vtc_in: Deviation, // intake VTC: live VTCCMD vs live VTCANGL, no map needed (both use converter 2514)
    vtc_ex: Deviation, // exhaust VTC: live EXVTCCMDEA vs live EXVTCABSAEA, no map needed
    vtc_in_angl: Option<(f64, f64)>, // last (t, VTCANGL): it arrives in DID 0x2612, VTCCMD in 0x2662, so one feed never holds both
}

impl Maps {
    /// prior: this car's earlier sessions' out() values, newest first.
    pub fn new(prior: &[Value]) -> Maps {
        Maps::with_limits(prior, WARM, MIN_CELL)
    }

    /// As `new`, with the warm-engine temperature and the samples a cell needs set by the caller (tests that run a short drive).
    pub fn with_limits(prior: &[Value], warm: f64, min_cell: f64) -> Maps {
        let rpm = json!(RPM.map(|x| x as i64));
        let load = json!(load().map(|x| round_to(x, 2)));
        Maps {
            base: Baseline::new(prior, &rpm, &load, min_cell),
            cells: SIGNALS.iter().map(|(s, _, _)| (*s, vec![])).collect(),
            seen: vec![],
            ne: None,
            gair: None,
            ect: None,
            fc: false,
            t_ne: None,
            warm,
            min_cell,
            vtc_in: Deviation::default(),
            vtc_ex: Deviation::default(),
            vtc_in_angl: None,
        }
    }

    /// One engine data group: remember the operating point, then count every mapped signal in it.
    pub fn feed(&mut self, t: f64, named: &Named) {
        let num = |k: &str| named.get(k).and_then(Val::num);
        if let (Some(ne), Some(g)) = (num("NE"), num("GAIR")) {
            (self.ne, self.gair, self.t_ne) = (Some(ne), Some(g), Some(t));
        }
        if let Some(e) = num("ECT") {
            self.ect = Some(e);
        }
        if let Some(f) = named.get("FC") {
            self.fc = f.is_text("ON");
        }
        let (Some(t_ne), Some(ne), Some(gair)) = (self.t_ne, self.ne, self.gair) else { return };
        if t - t_ne > FRESH || ne < MIN_RPM {
            return;
        }
        // VTCCMD (DID 0x2662 byte 29) and VTCANGL (DID 0x2612 byte 24) share converter 2514 (raw*0.5-20 deg,
        // hds-xref/decoded/converters.csv), so comparing them needs no unit arithmetic and no ROM map: the
        // ECU already reports its own commanded intake angle directly, the same way EXVTCCMDEA does for the
        // exhaust side below. (This replaced a ROM-map regression fit, corr 0.858 against VTCANGL -- not
        // exact, and more than this check needs now that the ECU's own target is available directly.)
        // The two signals ride in different DIDs that the monitor reads at different cadences (0x2612 every
        // 0.5 s, 0x2662 every 2 s, monitor.rs), so one `named` never holds both: remember the latest actual,
        // and each time the commanded angle arrives compare it against that actual when it is fresh -- one
        // sample per command report, never a re-pair against the same command, the way Alerts pairs NE with
        // the misfire counters.
        if let Some(angl) = num("VTCANGL") {
            self.vtc_in_angl = Some((t, angl));
        }
        if let Some(cmd) = num("VTCCMD") {
            if let Some((ta, angl)) = self.vtc_in_angl {
                if (t - ta).abs() <= VTC_PAIR {
                    self.vtc_in.add(angl, cmd);
                }
            }
        }
        if let (Some(cmd), Some(act)) = (num("EXVTCCMDEA"), num("EXVTCABSAEA")) {
            self.vtc_ex.add(act, cmd);
        }
        let cell: Cell = (bin_of(ne, &RPM), bin_of(gair / (ne / 60.0), &load()));
        for (sym, _, _) in SIGNALS {
            let Some(v) = num(sym) else { continue };
            if FUEL.contains(&sym) && (self.fc || self.ect.is_none_or(|e| e < self.warm)) {
                continue;
            }
            let cells = &mut self.cells.iter_mut().find(|(s, _)| *s == sym).unwrap().1;
            match cells.iter_mut().find(|(c, _)| *c == cell) {
                Some((_, c)) => {
                    c[0] += 1.0;
                    c[1] += v;
                    c[2] = c[2].min(v);
                    c[3] = c[3].max(v);
                }
                None => cells.push((cell, [1.0, v, v, v])),
            }
            match self.seen.iter_mut().find(|(s, _)| *s == sym) {
                Some((_, n)) => *n += 1,
                None => self.seen.push((sym, 1)),
            }
        }
    }

    fn cells_of(&self, sym: &str) -> &Cells {
        &self.cells.iter().find(|(s, _)| *s == sym).unwrap().1
    }

    /// JSON-ready: the axes, then per signal its filled cells as [rpm bin, load bin, n, mean, min, max].
    pub fn out(&self) -> Value {
        let mut signals = Map::new();
        for (s, name, unit) in SIGNALS {
            let mut sorted: Vec<_> = self.cells_of(s).iter().collect();
            sorted.sort_by_key(|(c, _)| *c);
            let cells: Vec<Value> = sorted
                .into_iter()
                .map(|((r, l), c)| json!([r, l, c[0] as u64, jnum(round_to(c[1] / c[0], 3)), jnum(round_to(c[2], 3)), jnum(round_to(c[3], 3))]))
                .collect();
            let samples = self.seen.iter().find(|(x, _)| *x == s).map_or(0, |(_, n)| *n);
            signals.insert(s.into(), json!({"name": name, "unit": unit, "samples": samples, "cells": cells}));
        }
        let all: Vec<(&str, &Cells)> = SIGNALS.iter().map(|(s, _, _)| (*s, self.cells_of(s))).collect();
        json!({"rpm": RPM.map(|x| x as i64), "load": load().map(|x| round_to(x, 2)), "load_unit": "g/rev", "min_cell": self.min_cell as i64,
               "signals": signals, "baseline": self.base.summary(&all)})
    }

    /// The VTC checks (both now a direct comparison of two live signals sharing one converter, no ROM map or fit
    /// involved): deliberately not part of `out()` or `notes()`, which the golden tests (tests/golden.rs,
    /// tests/golden_recorder.rs) pin byte-for-byte to the Python original at 06f9d7d -- a build that never had
    /// this Rust-only feature. Callers (recorder.rs) add this under its own key instead.
    pub fn factory_summary(&self) -> Value {
        let dev = |d: &Deviation| json!({"samples": d.n, "mean_abs_deg": jnum(round_to(d.mean_abs(), 2)), "max_abs_deg": jnum(round_to(d.max_abs, 2))});
        json!({
            "vtc_intake": if self.vtc_in.n > 0 { dev(&self.vtc_in) } else { Value::Null },
            "vtc_exhaust": if self.vtc_ex.n > 0 { dev(&self.vtc_ex) } else { Value::Null },
        })
    }

    /// The words for one cell: the signal's mean `m` over `n` samples at the cell's engine speed and load.
    fn text(sym: &str, cell: Cell, m: f64, n: f64) -> String {
        let at = format!("at {} ({} samples)", band(cell.0, cell.1), n as u64);
        match sym {
            "KLAFAVE" => {
                let correcting = if (m - 1.0).abs() > TRIM_FLAG { format!(": the PCM is correcting by {:.0} %", (m - 1.0).abs() * 100.0) } else { String::new() };
                format!("long-term fuel trim averaged {m:.2} {at}{correcting}")
            }
            "IG" => format!("spark advance averaged {m:.1} deg {at}"),
            "LAMBDA" => format!("lambda averaged {m:.3} {at}"),
            _ => {
                let what = if sym == "KNRTD" { "knock retard".to_string() } else { format!("cylinder {} knock retard", &sym[5..]) };
                format!("{what} averaged {m:.1} deg {at}")
            }
        }
    }

    /// One cell's note. With enough history the car's own baseline decides; otherwise the fixed limit does.
    fn flag(&self, sym: &str, cell: Cell, c: &[f64; 4]) -> Option<String> {
        let m = c[1] / c[0];
        let own = if c[0] >= self.min_cell { self.base.cell(sym, cell, m) } else { None };
        if let Some((k, pm, sd, z)) = own {
            return Baseline::flagged(sym, z).then(|| {
                format!("{}: {z:+.1} standard deviations from this car's {k} earlier sessions ({pm:.2} +/- {sd:.2}) [own history]", Self::text(sym, cell, m, c[0]))
            });
        }
        (c[0] >= self.min_cell && fixed(sym, m) == Some(true)).then(|| {
            format!("{} [fixed limit: {} earlier sessions, {MIN_SESSIONS} needed for the car's own baseline]", Self::text(sym, cell, m, c[0]), self.base.used)
        })
    }

    pub fn notes(&self) -> Vec<String> {
        let mut out = vec![];
        for sym in NOTE_ORDER.iter().chain(["KNRTD4"].iter()) {
            let mut sorted: Vec<_> = self.cells_of(sym).iter().collect();
            sorted.sort_by_key(|(c, _)| *c);
            for (cell, c) in sorted {
                out.extend(self.flag(sym, *cell, c));
            }
        }
        let mut tot: Vec<(f64, usize)> = vec![];
        for (i, y) in CYL.iter().enumerate() {
            let cs = self.cells_of(y);
            let n: f64 = cs.iter().map(|(_, c)| c[0]).sum();
            if n >= self.min_cell {
                tot.push((pysum(&cs.iter().map(|(_, c)| c[1]).collect::<Vec<_>>()) / n, i + 1));
            }
        }
        let own: Vec<Option<(usize, f64, f64, f64)>> =
            if tot.len() == CYL.len() { CYL.iter().zip(&tot).map(|(y, t)| self.base.cylinder(y, t.0)).collect() } else { vec![] };
        for (&(m, i), got) in tot.iter().zip(&own) {
            if let Some((k, pm, sd, z)) = got {
                if Baseline::flagged(CYL[i - 1], *z) {
                    out.push(format!(
                        "cylinder {i} knock retard averaged {m:.1} deg over the session: {z:+.1} standard deviations from this car's {k} earlier sessions ({pm:.2} +/- {sd:.2}) [own history]"
                    ));
                }
            }
        }
        if tot.len() == CYL.len() && own.iter().any(Option::is_none) {
            let cmp = |a: &&(f64, usize), b: &&(f64, usize)| a.partial_cmp(b).unwrap();
            let (hi, lo) = (tot.iter().max_by(cmp).unwrap(), tot.iter().min_by(cmp).unwrap());
            if hi.0 - lo.0 > CYL_GAP {
                out.push(format!(
                    "cylinder {} pulls {:.1} deg more knock retard than cylinder {} ({:.1} against {:.1} deg, whole session): a single cylinder knocking points at it [fixed limit: {} earlier sessions, {MIN_SESSIONS} needed for the car's own baseline]",
                    hi.1, hi.0 - lo.0, lo.1, hi.0, lo.0, self.base.used
                ));
            }
        }
        out
    }

    /// The VTC checks, not the car's own history: each compares two live signals that share one converter (so
    /// no unit arithmetic and no ROM map or fit is involved in either), the ECU's own commanded angle against
    /// what it reports actually happened. An earlier version of the intake check used a ROM-map regression fit
    /// (corr 0.858 against VTCANGL) before VTCCMD -- the ECU's own direct report of the same commanded angle --
    /// was found; a factory-map-based knock-retard-limit check was also dropped before shipping (owner's
    /// instruction, 2026-10-16, "no assumed data"): its unit and which of three family maps applies were both
    /// unconfirmed. See README.md "RWD files" for what a car test would need to confirm that one.
    /// Deliberately not folded into `notes()` above -- see `factory_summary()`.
    pub fn factory_notes(&self) -> Vec<String> {
        let mut out = vec![];
        if self.vtc_in.n >= MIN_VTC_SAMPLES && self.vtc_in.mean_abs() > VTC_FLAG {
            out.push(format!(
                "intake VTC averaged {:.1} deg from its commanded angle over the session (worst sample {:.1} deg, {} samples): the cam may not be tracking command [VTCCMD vs VTCANGL, DID 0x2662/0x2612, two live signals compared directly, no ROM map or fit]",
                self.vtc_in.mean_abs(), self.vtc_in.max_abs, self.vtc_in.n
            ));
        }
        if self.vtc_ex.n >= MIN_VTC_SAMPLES && self.vtc_ex.mean_abs() > VTC_FLAG {
            out.push(format!(
                "exhaust VTC averaged {:.1} deg from its commanded angle over the session (worst sample {:.1} deg, {} samples): the cam may not be tracking command [EXVTCCMDEA vs EXVTCABSAEA, DID 0x266C, two live signals compared directly, no ROM map or fit]",
                self.vtc_ex.mean_abs(), self.vtc_ex.max_abs, self.vtc_ex.n
            ));
        }
        out
    }
}
