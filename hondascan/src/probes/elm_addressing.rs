//! Which addressing does the car answer through an ELM327 adapter? One run, read-only (3E 00 / 3E 80 only).
//!
//! The 2015 Accord on the GD101 answered 3E 00 on the OBD-II functional id 0x18DB33F1 but nothing on its own id 0x18DA0EF1. The second
//! run (2026-10-11 12:44, README "Fourth capture") narrowed it: every header set with ATCP18 + ATSH got no answer, even 0x18DB33F1, and
//! only the adapter's power-up header did. So the suspect is how this clone takes a header. Every row starts from ATZ (no state carried
//! over) and tests one way of setting it:
//!
//!   physical, ATSH                   what hondascan sends now (ATSHDA0EF1, no ATCP: 0x18 is the default)
//!   physical, ATCP18 + ATSH          what it sent until then
//!   physical, ATSH 4 bytes           ATSH18DA0EF1 (STN and some clones; the ELM327 v1.x answers '?')
//!   physical, ATSH + ATSP7           protocol selected again after the header: some clones take it only then
//!   physical, ATSH spaced            ATSH DA 0E F1, the datasheet's own spelling, in case the parser wants it
//!   physical, ATSH, no ATCRA         the adapter's automatic receive filter instead of hondascan's ATCRA
//!   physical, 0x55 / 0x00 padding    HDS's exact frame: DLC 8, unused bytes 0x55 (SendTP FUN_0813953c), via ATCAF0
//!   physical after 3E 80             HDS's keep-alive on 0x18DBEFF1 first (FUN_08121b6e), then the physical request
//!   functional 18DB33F1, ...         the same header commands on the id that is known to answer
//!   functional 18DB33F1, default     no header command at all: the control (answered in both runs)
//!   monitor                          ATMA for a few seconds (off by default: the adapter hung after it once)
//!
//! Usage (engine running, so the battery is not drained; nothing else may have the port open):
//!   elm-addressing [--port /dev/gd101] [--target 0E] [--log FILE] [--monitor SECONDS]
use crate::honda::{rx_id, tx_id, FUNCTIONAL_ID, OBD_FUNCTIONAL_ID};
use crate::transport::can::CaptureLog;
use crate::transport::elm327::{Elm327, Inner};
use crate::transport::elm_link::{frames, ElmErr};
use std::collections::BTreeMap;
use std::io::Write;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How the 29-bit id reaches the adapter.
const STYLES: [&str; 6] = ["ATSH", "ATCP18 + ATSH", "ATSH 4 bytes", "ATSH + ATSP7", "ATSH spaced", "ATSH, no ATCRA"];

fn header_cmds(style: &str, i: u32) -> Vec<String> {
    let low = i & 0xFFFFFF;
    match style {
        "ATSH" | "ATSH, no ATCRA" => vec![format!("ATSH{low:06X}")],
        "ATCP18 + ATSH" => vec![format!("ATCP{:02X}", i >> 24), format!("ATSH{low:06X}")],
        "ATSH 4 bytes" => vec![format!("ATSH{i:08X}")],
        "ATSH + ATSP7" => vec![format!("ATSH{low:06X}"), "ATSP7".into()],
        _ => vec![format!("ATSH {:02X} {:02X} {:02X}", (low >> 16) & 0xFF, (low >> 8) & 0xFF, low & 0xFF)], // "ATSH spaced"
    }
}

fn raw(payload: &[u8], pad: u8) -> String {
    let mut d = vec![payload.len() as u8];
    d.extend(payload);
    d.resize(8, pad);
    d.iter().map(|b| format!("{b:02X}")).collect()
}

/// Send one line of hex; true if a frame from `rx` came back. Adapter faults are errors.
fn answered(elm: &mut Inner, rx: u32, text: &str, timeout: f64) -> Result<bool, ElmErr> {
    let out = elm.cmd(text, timeout + 2.0)?;
    Ok(frames(elm, &out)?.iter().any(|(cid, _)| *cid == rx))
}

enum Row {
    Style(&'static str, Option<u32>),
    Padded(u8),
    AfterKeepAlive,
}

fn rows(target: u32) -> Vec<(String, Row)> {
    let phys = tx_id(target);
    let mut v: Vec<(String, Row)> = STYLES.iter().map(|s| (format!("physical {phys:08X}, {s}"), Row::Style(s, Some(phys)))).collect();
    v.push((format!("physical {phys:08X}, 0x55 padding (HDS frame)"), Row::Padded(0x55)));
    v.push((format!("physical {phys:08X}, 0x00 padding"), Row::Padded(0x00)));
    v.push((format!("physical {phys:08X} after 3E 80 on {FUNCTIONAL_ID:08X}"), Row::AfterKeepAlive));
    v.extend(STYLES.iter().map(|s| (format!("functional {OBD_FUNCTIONAL_ID:08X}, {s}"), Row::Style(s, Some(OBD_FUNCTIONAL_ID)))));
    v.push((format!("functional {OBD_FUNCTIONAL_ID:08X}, adapter default header"), Row::Style("ATSH", None)));
    v
}

fn ask(elm: &mut Inner, target: u32, can_id: Option<u32>, text: &str, style: &str, before: &[String]) -> Result<bool, ElmErr> {
    let rx = rx_id(target);
    elm.init()?; // ATZ: no header, filter or format left from a row
    for c in before {
        elm.cmd(c, 1.0)?;
    }
    if let Some(id) = can_id {
        for c in header_cmds(style, id) {
            elm.must_cmd(&c)?;
        }
    }
    if style != "ATSH, no ATCRA" {
        elm.cmd(&format!("ATCRA{rx:08X}"), 2.0)?;
    }
    answered(elm, rx, text, 1.0)
}

fn run_row(elm: &mut Inner, target: u32, row: &Row) -> Result<bool, ElmErr> {
    let phys = tx_id(target);
    match row {
        Row::Style(style, id) => ask(elm, target, *id, "3E00", style, &[]),
        Row::Padded(pad) => ask(elm, target, Some(phys), &raw(&[0x3E, 0x00], *pad), "ATSH", &["ATCAF0".to_string()]),
        Row::AfterKeepAlive => {
            let mut before = header_cmds("ATSH", FUNCTIONAL_ID);
            before.push("3E80".into()); // suppress reply: NO DATA expected
            ask(elm, target, Some(phys), "3E00", "ATSH", &before)
        }
    }
}

/// ATMA for `seconds`; the distinct ids seen, with a count.
fn monitor(elm: &mut Inner, seconds: f64) -> Result<BTreeMap<String, u32>, ElmErr> {
    elm.cmd("ATCRA", 2.0)?; // filter off
    if let Some(l) = &elm.log {
        l.note("ELM> ATMA");
    }
    elm.raw_write(b"ATMA\r");
    let mut buf: Vec<u8> = vec![];
    let end = Instant::now() + Duration::from_secs_f64(seconds);
    while Instant::now() < end {
        buf.extend(elm.raw_read(end.saturating_duration_since(Instant::now()).as_secs_f64()));
    }
    elm.raw_write(b"\r"); // any character stops monitoring
    let end = Instant::now() + Duration::from_secs_f64(2.0);
    let after_atma = |b: &[u8]| -> bool {
        let tail = b.windows(4).rposition(|w| w == b"ATMA").map_or(b, |p| &b[p + 4..]);
        tail[tail.len().saturating_sub(4)..].contains(&b'>')
    };
    while !after_atma(&buf) && Instant::now() < end {
        buf.extend(elm.raw_read(end.saturating_duration_since(Instant::now()).as_secs_f64()));
    }
    let mut ids = BTreeMap::new();
    for ln in String::from_utf8_lossy(&buf).replace('\r', "\n").lines() {
        let ln: String = ln.trim().replace(' ', "");
        if ln.len() > 8 && ln.chars().all(|c| "0123456789ABCDEF".contains(c)) {
            elm.note(&format!("ATMA {ln}"));
            *ids.entry(ln[..8].to_string()).or_insert(0) += 1;
        }
    }
    Ok(ids)
}

pub fn run(port: &str, target: u32, log_path: &str, monitor_s: f64, out: &mut dyn Write) -> Result<i32, String> {
    // before anything is opened or probed: from_secs_f64 in monitor() would panic on this
    if Duration::try_from_secs_f64(monitor_s).is_err() {
        return Err(format!("argument --monitor: {monitor_s} must be a finite number >= 0"));
    }
    let log = Arc::new(CaptureLog::at(std::path::Path::new(log_path)).map_err(|e| e.to_string())?);
    let elm = Elm327::open(port, None, 400, Some(log.clone())).map_err(|e| e.to_string())?;
    let _ = writeln!(out, "{}", elm.with(|i| i.describe()));
    let result = elm.with(|inn| -> Result<(), String> {
        for (name, row) in rows(target) {
            let got = match run_row(inn, target, &row) {
                Ok(true) => "answered ".to_string(),
                Ok(false) => "no answer".to_string(),
                Err(e @ ElmErr::Reset { .. }) => {
                    // settings lost: set up again, so the next row is valid
                    inn.recover(&e).map_err(|x| x.to_string())?;
                    "ADAPTER RESET, row not valid".to_string()
                }
                Err(e) => format!("ERROR {e}"), // a bus error line, or a command the adapter refused
            };
            log.note(&format!("RESULT {name}: {}", got.trim()));
            let _ = writeln!(out, "{got}  {name}");
        }
        if monitor_s != 0.0 {
            let ids = monitor(inn, monitor_s).map_err(|e| e.to_string())?;
            let _ = writeln!(out, "monitor {monitor_s:.0} s: {}", if ids.is_empty() { "nothing".to_string() } else { ids.iter().map(|(i, n)| format!("{i} x{n}")).collect::<Vec<_>>().join(", ") });
        }
        Ok(())
    });
    elm.with(|i| {
        let _ = i.cmd("ATPC", 1.0);
        i.close_port();
    });
    result?;
    let _ = writeln!(out, "log: {log_path}");
    Ok(0)
}
