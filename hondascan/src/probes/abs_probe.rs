//! Read-only probe: what makes the 2015 Accord's ABS (0x28), EPS (0x30) and TPMS (0x26) answer on the DLC?
//!
//! README.md "ABS and EPS": physical 3E 00 / 10 03 / 22 4000 to them is silent, only 0x0E, 0x53 and 0x60 answer. This runs the
//! untried steps in order and, after each, re-asks every address 0x00-0xFF (3E 00 on its own id) and listens passively, so any change
//! shows. Car on, engine running. Only requests inside the read-only allowlist are sent: 3E 00/80, 10 03, 22 <DID> (here C000/C010 to
//! the gateways, 4000/4800/6000 to the modules).
//!
//!   abs-probe gd101-j2534 [--log FILE]       (adapter spec as for `hondascan -a`)
use crate::honda::{rx_id, tx_id, FUNCTIONAL_ID, OBD_FUNCTIONAL_ID};
use crate::transport::can::{CanFrame, CaptureLog};
use crate::transport::{open_adapter, Adapter};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const TARGETS: [(u32, &str); 3] = [(0x28, "ABS/VSA"), (0x30, "EPS"), (0x26, "TPMS")];
pub const GATEWAYS: [u32; 2] = [0xEF, 0xEE];
const ALLOWED: [&[u8]; 8] = [&[0x3E, 0x00], &[0x3E, 0x80], &[0x10, 0x03], &[0x22, 0xC0, 0x00], &[0x22, 0xC0, 0x10], &[0x22, 0x40, 0x00], &[0x22, 0x48, 0x00], &[0x22, 0x60, 0x00]];

pub struct Probe {
    adapter: Arc<dyn Adapter>,
    pub baseline: BTreeSet<u32>,
}

impl Probe {
    pub fn new(adapter: Arc<dyn Adapter>) -> Probe {
        Probe { adapter, baseline: BTreeSet::new() }
    }

    /// Ask on `tx` and collect the answers on `rxs`. A payload off the allowlist is refused before anything is sent.
    pub fn ask(&self, tx: u32, rxs: &[u32], payload: &[u8], timeout: f64) -> Result<BTreeMap<u32, Option<f64>>, String> {
        if !ALLOWED.contains(&payload) {
            let hex: String = payload.iter().map(|b| format!("{b:02x}")).collect();
            return Err(format!("payload {hex} is not on the probe allowlist"));
        }
        self.adapter.broadcast(tx, rxs, payload, timeout).map_err(|e| e.to_string())
    }

    /// Addresses answering 3E 00 on their own id.
    pub fn sweep(&self, timeout: f64) -> Result<BTreeSet<u32>, String> {
        let pairs: Vec<(u32, u32)> = (0..256).map(|x| (tx_id(x), rx_id(x))).collect();
        Ok(self.adapter.probe(&pairs, &[0x3E, 0x00], timeout).map_err(|e| e.to_string())?.into_iter().map(|rx| rx & 0xFF).collect())
    }

    pub fn listen(&self, seconds: f64) -> BTreeMap<u32, u32> {
        let mut seen = BTreeMap::new();
        let Some(bus) = self.adapter.bus() else { return seen };
        let end = Instant::now() + Duration::from_secs_f64(seconds);
        while let Some(f) = bus.recv(end.saturating_duration_since(Instant::now()).as_secs_f64()) {
            *seen.entry(f.can_id).or_insert(0) += 1;
        }
        seen
    }

    fn report(&self, out: &mut dyn Write, step: &str) -> Result<BTreeSet<u32>, String> {
        let now = self.sweep(1.0)?;
        let new: Vec<&u32> = now.iter().filter(|x| !self.baseline.contains(x)).collect();
        let hex = |v: &mut dyn Iterator<Item = &u32>| v.map(|x| format!("{x:02X}")).collect::<Vec<_>>().join(" ");
        let _ = writeln!(out, "  -> answering: {}   new: {}", hex(&mut now.iter()), if new.is_empty() { "none".to_string() } else { hex(&mut new.iter().copied()) });
        for (t, name) in TARGETS {
            if now.contains(&t) && !self.baseline.contains(&t) {
                let _ = writeln!(out, "  *** {name} 0x{t:02X} ANSWERS after: {step}");
            }
        }
        Ok(now)
    }
}

pub fn run(spec: &str, log: Option<&str>, out: &mut dyn Write) -> Result<i32, String> {
    let log = match log {
        Some(p) => Some(Arc::new(CaptureLog::at(std::path::Path::new(p)).map_err(|e| e.to_string())?)),
        None => None,
    };
    let adapter = open_adapter(spec, log).map_err(|e| e.to_string())?;
    let mut p = Probe::new(adapter.clone());
    let r = steps(&mut p, &adapter, out);
    adapter.close();
    r
}

fn steps(p: &mut Probe, adapter: &Arc<dyn Adapter>, out: &mut dyn Write) -> Result<i32, String> {
    let hexs = |s: &BTreeSet<u32>| s.iter().map(|x| format!("{x:02X}")).collect::<Vec<_>>().join(" ");
    let _ = writeln!(out, "0. baseline sweep (3E 00 on each address)");
    p.baseline = p.sweep(1.0)?;
    let _ = writeln!(out, "  answering: {}", hexs(&p.baseline));
    if p.baseline.is_empty() {
        let _ = writeln!(out, "  nothing answers: car off or adapter not connected. Stop.");
        return Ok(1);
    }
    let bus = adapter.bus().ok_or("this adapter has no raw CAN bus")?;
    for (fid, name) in [(FUNCTIONAL_ID, "HDS functional"), (OBD_FUNCTIONAL_ID, "OBD functional")] {
        let _ = writeln!(out, "1. functional 3E 80 x10 on {fid:08X} ({name}), 0.3 s apart, then sweep");
        for _ in 0..10 {
            bus.send(&CanFrame::new(fid, vec![2, 0x3E, 0x80, 0x55, 0x55, 0x55, 0x55, 0x55])).map_err(|e| e.to_string())?;
            std::thread::sleep(Duration::from_millis(300));
        }
        p.report(out, &format!("3E 80 on {fid:08X}"))?;
    }
    let _ = writeln!(out, "2. 22 C000 / 22 C010 to the gateways 0xEF / 0xEE (their own ids)");
    for g in GATEWAYS {
        for did in [[0xC0u8, 0x00], [0xC0, 0x10]] {
            let got = p.ask(tx_id(g), &[rx_id(g)], &[&[0x22][..], &did].concat(), 0.5)?;
            let _ = writeln!(out, "  0x{g:02X} 22 {:02X}{:02X}: {}", did[0], did[1], if got.is_empty() { "silent" } else { "answered" });
        }
    }
    p.report(out, "gateway reads")?;
    for (fid, name) in [(FUNCTIONAL_ID, "HDS functional"), (OBD_FUNCTIONAL_ID, "OBD functional")] {
        let _ = writeln!(out, "3. functional 10 03 on {fid:08X} ({name}), then sweep");
        let all: Vec<u32> = (0..256).map(rx_id).collect();
        let got = p.ask(fid, &all, &[0x10, 0x03], 1.0)?;
        let _ = writeln!(out, "  answered: {}", if got.is_empty() { "none".to_string() } else { got.keys().map(|rx| format!("{:02X}", rx & 0xFF)).collect::<Vec<_>>().join(" ") });
        p.report(out, &format!("10 03 on {fid:08X}"))?;
    }
    let _ = writeln!(out, "4. physical reads of the target DIDs, in case a session or wake changed anything");
    for (t, did) in [(0x28u32, [0x40u8, 0x00]), (0x30, [0x48, 0x00]), (0x26, [0x60, 0x00])] {
        let got = p.ask(tx_id(t), &[rx_id(t)], &[&[0x22][..], &did].concat(), 0.5)?;
        let name = TARGETS.iter().find(|x| x.0 == t).map_or("", |x| x.1);
        let _ = writeln!(out, "  0x{t:02X} ({name}) 22 {:02X}{:02X}: {}", did[0], did[1], if got.is_empty() { "silent" } else { "ANSWERED" });
    }
    let _ = writeln!(out, "5. passive listen, 5 s");
    let seen = p.listen(5.0);
    let _ = writeln!(out, "  frames: {}", if seen.is_empty() { "none".to_string() } else { seen.iter().map(|(i, n)| format!("{i:08X} x{n}")).collect::<Vec<_>>().join(", ") });
    Ok(0)
}
