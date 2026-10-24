//! Replay a capture: a CanBus that answers each request frame with the reply frames recorded for it. Port of
//! hondascan/transport/replay.py.
//!
//! Input is a `--log` file of a raw CAN session (candump format with hondascan's `; TX` / `; RX` tags). A received frame is
//! attributed to the latest transmitted request it answers: the one on the paired id (ISO 15765-4 normal fixed addressing swaps the
//! two address bytes, 0x18DA<tgt><src> <-> 0x18DA<src><tgt>; 11-bit 0x7Ex <-> 0x7Ex+8), or, when no such frame was sent, the latest
//! request of all. Requests sent back to back therefore replay correctly. A functional request (0x18DB....) is answered on every
//! reply id, so the frames after it are its own, whatever was sent to those ids before. Our own flow-control and consecutive frames
//! are not requests: the frames after them belong to the request they continue. A single-frame request is matched on its PCI byte
//! and payload only: padding carries no content, and an ELM327 capture logs the frame without the adapter's padding. The recorded
//! delays are kept, so a real vehicle's traffic, timing included, becomes a repeatable test. A request that was not recorded gets no
//! answer. Repeated identical requests replay their answers in file order; after the last one it repeats.
use super::can::{BusCore, CanBus, CanFrame, CaptureLog};
use std::collections::HashMap;
use std::io;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The id that answers `can_id` under normal fixed addressing, or None.
pub fn partner(can_id: u32, extended: bool) -> Option<u32> {
    if extended && can_id >> 16 == 0x18DA {
        return Some(can_id & 0xFFFF0000 | (can_id & 0xFF) << 8 | (can_id >> 8) & 0xFF);
    }
    if !extended && (0x7E0..=0x7E7).contains(&can_id) {
        return Some(can_id + 8);
    }
    None
}

pub type Key = (u32, Vec<u8>);

pub fn request_key(can_id: u32, data: &[u8]) -> Key {
    let mut d = data.to_vec();
    if !d.is_empty() && d[0] >> 4 == 0 {
        d.truncate(1 + (d[0] & 0xF) as usize); // single frame: PCI + payload, padding dropped
    }
    (can_id, d)
}

/// `\((\d+\.\d+)\) \S+ ([0-9A-Fa-f]{3,8})#([0-9A-Fa-f]*) ; (TX|RX)` matched at the start of the line.
pub fn parse_line(ln: &str) -> Option<(f64, String, Vec<u8>, bool)> {
    let rest = ln.strip_prefix('(')?;
    let (ts, rest) = rest.split_once(") ")?;
    let (int, frac) = ts.split_once('.')?;
    if int.is_empty() || frac.is_empty() || !int.chars().all(|c| c.is_ascii_digit()) || !frac.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let (iface, rest) = rest.split_once(' ')?;
    if iface.is_empty() {
        return None;
    }
    let (ident, rest) = rest.split_once('#')?;
    if !(3..=8).contains(&ident.len()) || !ident.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let (data, dir) = rest.split_once(" ; ")?;
    let dir = dir.get(..2)?;
    if !data.chars().all(|c| c.is_ascii_hexdigit()) || data.len() % 2 != 0 || !(dir == "TX" || dir == "RX") {
        return None;
    }
    let bytes: Vec<u8> = (0..data.len() / 2).map(|i| u8::from_str_radix(&data[2 * i..2 * i + 2], 16).unwrap()).collect();
    Some((ts.parse().ok()?, ident.to_string(), bytes, dir == "TX"))
}

/// {(can_id, data): [[(delay, CanFrame), ...], ...]}
pub type Exchanges = HashMap<Key, Vec<Vec<(f64, CanFrame)>>>;

/// {(can_id, data): [[(delay, CanFrame), ...], ...]}
pub fn parse(path: &str) -> io::Result<Exchanges> {
    let text = String::from_utf8_lossy(&std::fs::read(path)?).into_owned();
    let mut out: Exchanges = HashMap::new();
    let mut latest: HashMap<u32, (Key, f64)> = HashMap::new(); // reply id -> (key, time) of the request expecting it
    let mut last: Option<(Key, f64)> = None;
    for ln in text.lines() {
        let Some((ts, ident, data, tx)) = parse_line(ln) else { continue };
        let frame = CanFrame { can_id: u32::from_str_radix(&ident, 16).unwrap(), data: data.clone(), extended: ident.len() == 8, timestamp: 0.0 };
        if tx {
            if !data.is_empty() && matches!(data[0] >> 4, 2 | 3) {
                continue; // CF / FC: continues the request before it
            }
            let key = request_key(frame.can_id, &data);
            out.entry(key.clone()).or_default().push(vec![]);
            last = Some((key.clone(), ts));
            if let Some(reply) = partner(frame.can_id, frame.extended) {
                latest.insert(reply, last.clone().unwrap());
            } else if frame.extended && frame.can_id >> 16 == 0x18DB {
                latest.clear(); // functional: every reply id now answers this request
            }
        } else if let Some(l) = &last {
            let (key, t0) = latest.get(&frame.can_id).cloned().unwrap_or_else(|| l.clone());
            if let Some(v) = out.get_mut(&key).and_then(|v| v.last_mut()) {
                v.push((ts - t0, frame));
            }
        }
    }
    Ok(out)
}

pub struct ReplayBus {
    core: BusCore,
    exchanges: Mutex<Exchanges>,
    pending: Mutex<Vec<(Instant, CanFrame)>>,
}

impl ReplayBus {
    pub fn open(path: &str, log: Option<Arc<CaptureLog>>) -> io::Result<ReplayBus> {
        Ok(ReplayBus { core: BusCore::with_log(log), exchanges: Mutex::new(parse(path)?), pending: Mutex::new(vec![]) })
    }
}

impl CanBus for ReplayBus {
    fn core(&self) -> &BusCore {
        &self.core
    }

    fn send_raw(&self, frame: &CanFrame) -> io::Result<()> {
        let mut ex = self.exchanges.lock().unwrap();
        let Some(answers) = ex.get_mut(&request_key(frame.can_id, &frame.data)) else { return Ok(()) };
        if answers.is_empty() {
            return Ok(());
        }
        let replies = if answers.len() > 1 { answers.remove(0) } else { answers[0].clone() };
        let now = Instant::now();
        let mut p = self.pending.lock().unwrap();
        p.extend(replies.into_iter().map(|(d, f)| (now + Duration::from_secs_f64(d.max(0.0)), f)));
        p.sort_by_key(|x| x.0); // stable
        Ok(())
    }

    fn recv_raw(&self, timeout: f64) -> Option<CanFrame> {
        let end = Instant::now() + Duration::from_secs_f64(timeout.max(0.0));
        loop {
            let head = self.pending.lock().unwrap().first().map(|(d, _)| *d);
            let Some(due) = head else { break };
            let wait = due.min(end).saturating_duration_since(Instant::now());
            if !wait.is_zero() {
                std::thread::sleep(wait);
            }
            if Instant::now() < due {
                return None; // the timeout came first
            }
            let (_, f) = self.pending.lock().unwrap().remove(0);
            let ids = self.ids();
            if ids.is_empty() || ids.contains(&f.can_id) {
                return Some(f);
            }
        }
        let left = end.saturating_duration_since(Instant::now());
        if !left.is_zero() {
            std::thread::sleep(left);
        }
        None
    }
}
