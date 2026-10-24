//! A car's own baseline for the observed maps: this session's cell means against the same cells in its earlier
//! sessions. Port of hondascan/mapbaseline.py, which documents the rule (hondascan's own, not Honda's).
//!
//! One deliberate difference: Python's `statistics.mean/stdev` do exact rational arithmetic; here the mean and spread
//! use exactly-summed (Shewchuk) terms and one division, which can differ from Python in the last bit. Every output
//! that reaches a report is rounded to 2-3 decimals, and the golden test compares those.
use crate::value::{jnum, pysum, round_to};
use serde_json::{json, Map, Value};
use std::collections::HashMap;

pub const MIN_SESSIONS: usize = 3;
pub const Z: f64 = 3.0;
// Pinned where they are defined: the thresholds the docs state (at least MIN_SESSIONS earlier sessions; a cell counts as
// differing once it is more than Z spread units from its baseline mean). Changing either is a documented-rule change.
const _: () = assert!(MIN_SESSIONS == 3 && Z == 3.0);
const MAX_SESSIONS: usize = 30; // the most recent sessions that are read back
pub const CYL: [&str; 4] = ["KNRTD1", "KNRTD2", "KNRTD3", "KNRTD4"];
const ONE_SIDED: [&str; 5] = ["KNRTD", "KNRTD1", "KNRTD2", "KNRTD3", "KNRTD4"]; // only "more than before" is flagged
pub const RULE: &str = "now against the same cell in earlier sessions of this car (each session counts once): flagged when it is more than 3 \
standard deviations of those session means away, with a floor of one signal step on the spread; needs 3+ earlier sessions \
with the cell filled (hondascan's rule, not Honda's)";

/// One raw step of the signal; LAMBDA's 0.01 is hondascan's choice (see the Python docstring). None for a symbol
/// this module does not know: `stats` then skips it rather than panic on a prior session's JSON, which this
/// module does not control the shape of (an older store, or a future signal added to `maps::SIGNALS` without a
/// matching case here).
pub fn floor(sym: &str) -> Option<f64> {
    match sym {
        "IG" | "KNRTD" | "KNRTD1" | "KNRTD2" | "KNRTD3" | "KNRTD4" => Some(0.5),
        "KLAFAVE" => Some(1.0 / 128.0),
        "LAMBDA" => Some(0.01),
        _ => None,
    }
}

pub type Cell = (usize, usize);
/// One engine-map cell as Maps keeps it: [n, sum, lo, hi].
pub type Cells = Vec<(Cell, [f64; 4])>;

/// (sessions, prior mean, spread)
pub type Stats = (usize, f64, f64);

fn mean(xs: &[f64]) -> f64 {
    exact_sum(xs) / xs.len() as f64
}

/// A correctly rounded sum (Shewchuk's msum, as in Python's math.fsum).
fn exact_sum(xs: &[f64]) -> f64 {
    let mut partials: Vec<f64> = vec![];
    for &x0 in xs {
        let mut x = x0;
        let mut i = 0;
        for j in 0..partials.len() {
            let mut y = partials[j];
            if x.abs() < y.abs() {
                std::mem::swap(&mut x, &mut y);
            }
            let hi = x + y;
            let lo = y - (hi - x);
            if lo != 0.0 {
                partials[i] = lo;
                i += 1;
            }
            x = hi;
        }
        partials.truncate(i);
        partials.push(x);
    }
    partials.iter().rev().fold(0.0, |a, p| a + p)
}

fn stdev(xs: &[f64]) -> f64 {
    let m = mean(xs);
    let dev: Vec<f64> = xs.iter().map(|x| (x - m) * (x - m)).collect();
    (exact_sum(&dev) / (xs.len() - 1) as f64).sqrt()
}

#[derive(Default)]
pub struct Baseline {
    min_cell: f64,
    cells: HashMap<String, HashMap<Cell, Stats>>,
    whole: HashMap<String, Stats>,
    pub used: usize,
}

impl Baseline {
    /// priors: earlier sessions' `maps.out()` values, newest first. rpm / load: the current axes.
    pub fn new(priors: &[Value], rpm: &Value, load: &Value, min_cell: f64) -> Baseline {
        let mut cells: HashMap<String, HashMap<Cell, Vec<f64>>> = HashMap::new();
        let mut whole: HashMap<String, Vec<f64>> = HashMap::new();
        let mut used = 0;
        for p in priors.iter().take(MAX_SESSIONS) {
            let Some(signals) = p.get("signals").and_then(Value::as_object) else { continue };
            if !p.get("rpm").is_some_and(|r| crate::value::same(r, rpm)) || !p.get("load").is_some_and(|l| crate::value::same(l, load)) {
                continue;
            }
            used += 1;
            for (sym, sig) in signals {
                if sym == "KLAF" {
                    continue; // the short-term trim swings around 1 by design; KLAFAVE is the one compared
                }
                let cs: Vec<&Vec<Value>> = sig
                    .get("cells")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(Value::as_array).filter(|c| c.len() >= 4 && c[2].as_f64().unwrap_or(0.0) >= min_cell).collect())
                    .unwrap_or_default();
                for c in &cs {
                    let key = (c[0].as_u64().unwrap_or(0) as usize, c[1].as_u64().unwrap_or(0) as usize);
                    cells.entry(sym.clone()).or_default().entry(key).or_default().push(c[3].as_f64().unwrap_or(0.0));
                }
                if CYL.contains(&sym.as_str()) {
                    let n: f64 = cs.iter().map(|c| c[2].as_f64().unwrap_or(0.0)).sum();
                    if n > 0.0 {
                        let w = pysum(&cs.iter().map(|c| c[2].as_f64().unwrap_or(0.0) * c[3].as_f64().unwrap_or(0.0)).collect::<Vec<_>>()) / n;
                        whole.entry(sym.clone()).or_default().push(w);
                    }
                }
            }
        }
        let cells = cells
            .into_iter()
            .map(|(sym, d)| {
                let st = d.into_iter().filter_map(|(c, m)| stats(&m, &sym).map(|s| (c, s))).collect();
                (sym, st)
            })
            .collect();
        let whole = whole.into_iter().filter_map(|(sym, m)| stats(&m, &sym).map(|s| (sym, s))).collect();
        Baseline { min_cell, cells, whole, used }
    }

    /// (sessions, prior mean, spread, z) for one filled cell of the current session (`now` is its mean), or None.
    pub fn cell(&self, sym: &str, cell: Cell, now: f64) -> Option<(usize, f64, f64, f64)> {
        z(self.cells.get(sym)?.get(&cell).copied(), now)
    }

    /// The same for one cylinder's whole-session knock mean.
    pub fn cylinder(&self, sym: &str, now: f64) -> Option<(usize, f64, f64, f64)> {
        z(self.whole.get(sym).copied(), now)
    }

    pub fn flagged(sym: &str, z: f64) -> bool {
        if ONE_SIDED.contains(&sym) { z > Z } else { z.abs() > Z }
    }

    /// JSON for status: how much history there is, and z for every current cell that could be compared.
    pub fn summary(&self, cells: &[(&str, &Cells)]) -> Value {
        let mut by_cell = Map::new();
        for (sym, cs) in cells {
            let mut sorted: Vec<&(Cell, [f64; 4])> = cs.iter().collect();
            sorted.sort_by_key(|(c, _)| *c);
            let rows: Vec<Value> = sorted
                .into_iter()
                .filter(|(_, c)| c[0] >= self.min_cell)
                .filter_map(|((r, l), c)| {
                    let (k, m, sd, z) = self.cell(sym, (*r, *l), c[1] / c[0])?;
                    Some(json!([r, l, k, jnum(round_to(m, 3)), jnum(round_to(sd, 3)), jnum(round_to(z, 2))]))
                })
                .collect();
            if !rows.is_empty() {
                by_cell.insert((*sym).into(), Value::Array(rows));
            }
        }
        let mut cyl = vec![];
        for sym in CYL {
            let cs = cells.iter().find(|(s, _)| *s == sym).map(|(_, c)| *c).unwrap();
            let n: f64 = cs.iter().map(|(_, c)| c[0]).sum();
            if n < self.min_cell {
                continue;
            }
            let now = pysum(&cs.iter().map(|(_, c)| c[1]).collect::<Vec<_>>()) / n;
            if let Some((k, m, sd, z)) = self.cylinder(sym, now) {
                let i = CYL.iter().position(|c| *c == sym).unwrap() + 1;
                cyl.push(json!({"signal": sym, "cylinder": i, "now": jnum(round_to(now, 3)), "sessions": k,
                                "mean": jnum(round_to(m, 3)), "sd": jnum(round_to(sd, 3)), "z": jnum(round_to(z, 2))}));
            }
        }
        json!({"sessions": self.used, "min_sessions": MIN_SESSIONS, "z": Z, "rule": RULE, "cells": by_cell, "cylinders": cyl})
    }
}

/// (sessions, prior mean, spread), or None when there are fewer than MIN_SESSIONS means to compare with, or `sym`
/// has no known floor (a prior session's JSON naming a signal this build does not recognize).
fn stats(means: &[f64], sym: &str) -> Option<Stats> {
    let f = floor(sym)?;
    (means.len() >= MIN_SESSIONS).then(|| (means.len(), mean(means), stdev(means).max(f)))
}

fn z(st: Option<Stats>, now: f64) -> Option<(usize, f64, f64, f64)> {
    st.map(|(k, m, sd)| (k, m, sd, (now - m) / sd))
}

#[cfg(test)]
mod tests {
    use super::floor;

    #[test]
    fn floor_is_none_for_a_symbol_it_does_not_know() {
        assert_eq!(floor("LAMBDA"), Some(0.01));
        assert_eq!(floor("NOT_A_SIGNAL"), None);
    }
}
