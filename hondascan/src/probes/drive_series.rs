//! Decode one ECU's data-list replies from a recording's frames.log into a time series, with the database's converters.
//!
//! For looking at a drive after the fact (what the engine did when a misfire counter rose). Prints a row whenever a watched symbol
//! changes, then min/mean/max of each.
//!
//!   drive-series FRAMES.LOG [--system CANFI] [--address 0E] [SYMBOL ...]      (default symbols: NE ECT KNRTD KLAFAVE and the misfire counters)
use crate::db::Database;
use crate::livedata::LiveData;
use crate::transport::replay::parse_line;
use std::collections::{BTreeMap, HashSet};
use std::fmt::Write as _;
use std::sync::Arc;

const MISFIRE: [&str; 3] = ["NMCYLOBS", "NMFACYL", "NMFBCYL"];
const DEFAULT: [&str; 4] = ["NE", "ECT", "KNRTD", "KLAFAVE"];

/// Python's `format(x, 'g')` (six significant digits).
pub fn fmt_g(x: f64) -> String {
    if x == 0.0 {
        return "0".into();
    }
    if !x.is_finite() {
        return if x.is_nan() { "nan".into() } else if x > 0.0 { "inf".into() } else { "-inf".into() };
    }
    let e = format!("{:.5e}", x); // d.ddddde[-]x
    let (mant, exp) = e.split_once('e').unwrap();
    let exp: i32 = exp.parse().unwrap();
    if !(-4..6).contains(&exp) {
        let m = if mant.contains('.') { mant.trim_end_matches('0').trim_end_matches('.') } else { mant };
        return format!("{m}e{}{:02}", if exp < 0 { '-' } else { '+' }, exp.abs());
    }
    let decimals = (5 - exp).max(0) as usize;
    let s = format!("{x:.decimals$}");
    if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s }
}

/// (time, payload) of one reassembled reply.
pub type TimedPayload = (f64, Vec<u8>);
/// A reply mid-reassembly: (first frame's time, bytes so far, bytes still needed).
type PartialFrame = (f64, Vec<u8>, usize);
/// Per symbol: (min, max, count, total).
type Stats = BTreeMap<String, (f64, f64, u64, f64)>;
/// A decoded series' working state: (symbol -> last value, the stats, the first timestamp, the last-printed row).
type SeriesState = (BTreeMap<String, f64>, Stats, Option<f64>, Option<BTreeMap<String, f64>>);

/// ISO-TP-reassembled (time, payload) from the ECU at `address` (the first frame's time), from `18DAF1<address>` lines tagged RX.
pub fn replies(text: &str, address: u32) -> Vec<TimedPayload> {
    let (mut out, mut cur): (Vec<TimedPayload>, Option<PartialFrame>) = (vec![], None);
    for ln in text.lines() {
        let Some((t, ident, d, tx)) = parse_line(ln) else { continue };
        if tx || ident.len() != 8 || !ident.to_uppercase().starts_with("18DAF1") || u32::from_str_radix(&ident[6..], 16).ok() != Some(address) || d.is_empty() {
            continue;
        }
        match d[0] >> 4 {
            0 => out.push((t, d[1..d.len().min(1 + (d[0] & 0xF) as usize)].to_vec())),
            1 if d.len() >= 2 => cur = Some((t, d[2..].to_vec(), (((d[0] & 0xF) as usize) << 8) | d[1] as usize)),
            2 => {
                if let Some((t0, buf, need)) = cur.as_mut() {
                    buf.extend(&d[1..]);
                    if buf.len() >= *need {
                        out.push((*t0, buf[..*need].to_vec()));
                        cur = None;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

pub fn series(frames: &str, db: Arc<Database>, system: &str, address: u32, symbols: &[String]) -> std::io::Result<String> {
    let text = String::from_utf8_lossy(&std::fs::read(frames)?).into_owned();
    let live = LiveData::new(db);
    let want: Vec<&str> = if symbols.is_empty() { DEFAULT.to_vec() } else { symbols.iter().map(String::as_str).collect() };
    let listed: HashSet<i64> = live.dids(system, "DataList").into_iter().collect(); // the data list only: freeze-frame groups repeat symbols (ECT, NE)
    let (mut state, mut stats, mut t0, mut shown): SeriesState = (BTreeMap::new(), BTreeMap::new(), None, None);
    let mut out = String::new();
    for (t, p) in replies(&text, address) {
        if p.len() < 3 || p[0] != 0x62 || !listed.contains(&((p[1] as i64) << 8 | p[2] as i64)) {
            continue;
        }
        let t0v = *t0.get_or_insert(t);
        for s in live.signals(system, (p[1] as i64) << 8 | p[2] as i64).iter() {
            if want.contains(&s.symbol.as_str()) || (symbols.is_empty() && MISFIRE.iter().any(|m| s.symbol.starts_with(m))) {
                if let Some(v) = s.decode(&p[3..]).value {
                    state.insert(s.symbol.clone(), v);
                    let e = stats.entry(s.symbol.clone()).or_insert((v, v, 0, 0.0));
                    *e = (e.0.min(v), e.1.max(v), e.2 + 1, e.3 + v);
                }
            }
        }
        if !state.is_empty() && shown.as_ref() != Some(&state) {
            shown = Some(state.clone());
            let row: Vec<String> = state.iter().map(|(k, v)| format!("{k}={}", fmt_g(*v))).collect();
            let _ = writeln!(out, "{:8.1}  {}", t - t0v, row.join("  "));
        }
    }
    let _ = writeln!(out, "\nsymbol          min      mean       max   reads");
    for (k, (lo, hi, n, tot)) in &stats {
        let _ = writeln!(out, "{k:12} {:>8} {:9.3} {:>9} {n:7}", fmt_g(*lo), tot / *n as f64, fmt_g(*hi));
    }
    Ok(out)
}
