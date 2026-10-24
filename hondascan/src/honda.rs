//! Honda VCAN specifics: 29-bit addressing, ECU discovery, DTC lookup. Port of hondascan/honda.py, whose docstring says
//! where each fact comes from (GNA600 firmware framing, DecodeCANAddress, HDS's DID rules).
use crate::db::{Database, DtcInfo};
use crate::memory::{MemoryPolicy, FORMATS};
use crate::transport::Adapter;
use crate::transport::can::CaptureLog;
use crate::uds::{nrc_name, DtcRecord, UdsClient, UdsError, TESTER_PRESENT};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

/// What `19 xx` gives back: (availability mask, [(record, its database match)]).
pub type DtcReadout = (u8, Vec<(DtcRecord, Option<DtcInfo>)>);

pub const FUNCTIONAL_ID: u32 = 0x18DBEFF1;
/// ISO 15765-4 functional id; the only one the GD101 session of 2026-10-11 got answers on.
pub const OBD_FUNCTIONAL_ID: u32 = 0x18DB33F1;
/// The ignition module's x variable (engine speed) in the A550 image; any RAM word would do.
pub const PROBE_ADDRESS: u64 = 0xFFF9_098E;

pub fn tx_id(addr: u32) -> u32 {
    0x18DA00F1 | (addr & 0xFF) << 8
}

pub fn rx_id(addr: u32) -> u32 {
    0x18DAF100 | (addr & 0xFF)
}

/// What each DID probe returned (None: no positive reply), shared by the rules of one address.
pub type Probes = HashMap<u16, Option<Vec<u8>>>;

#[derive(Debug, Clone)]
pub struct Ecu {
    pub address: u32,
    pub system_type: String, // HDS system type ('HONDA ABS', 'PGM-FI', ...)
    pub system: String,      // database system ('CANABS', 'CANFI', ...)
    pub rule: String,
    pub probes: Probes,
}

impl std::fmt::Display for Ecu {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "0x{:02X} {} ({})", self.address, self.system_type, self.system)
    }
}

/// One row of can_addresses: conditions on comms and DID probes, as written by can_address_map.py.
pub struct Rule {
    pub text: String,
    terms: Vec<String>,
}

/// `did:XXXX=ok|noreply` with an optional `(flag=0xHEX)`: (did, wants_reply, flag).
fn parse_did_term(t: &str) -> Option<(u16, bool, Option<u32>)> {
    let rest = t.strip_prefix("did:")?;
    let (did, rest) = (rest.get(..4)?, rest.get(4..)?);
    if !did.chars().all(|c| matches!(c, '0'..='9' | 'A'..='F')) {
        return None;
    }
    let did = u16::from_str_radix(did, 16).ok()?;
    let rest = rest.strip_prefix('=')?;
    let (ok, rest) = if let Some(r) = rest.strip_prefix("noreply") { (false, r) } else { (true, rest.strip_prefix("ok")?) };
    if rest.is_empty() {
        return Some((did, ok, None));
    }
    let flag = rest.strip_prefix("(flag=0x")?.strip_suffix(')')?;
    if flag.is_empty() || !flag.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')) {
        return None;
    }
    Some((did, ok, Some(u32::from_str_radix(flag, 16).ok()?)))
}

fn parse_l8(t: &str) -> Option<(&str, i64)> {
    let rest = t.strip_prefix("l_8 ")?;
    let (op, num) = rest.split_once(' ')?;
    let digits = num.strip_prefix('-').unwrap_or(num);
    if !["==", "!=", "<=", ">=", "<", ">"].contains(&op) || digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some((op, num.parse().ok()?))
}

impl Rule {
    pub fn new(text: &str) -> Rule {
        let terms = if text.is_empty() || text == "always" { vec![] } else { text.split('&').map(|t| t.trim().to_string()).collect() };
        Rule { text: text.into(), terms }
    }

    /// True when any DID term carries a `(flag=0x..)`: its probe reads the first byte of the answer, not the 4-byte word.
    pub fn flagged(&self) -> bool {
        self.terms.iter().any(|t| parse_did_term(t).is_some_and(|(_, _, f)| f.is_some()))
    }

    pub fn dids(&self) -> Vec<u16> {
        self.terms.iter().filter_map(|t| parse_did_term(t)).map(|(d, _, _)| d).collect()
    }

    /// probe(did) -> bytes or None (no positive reply).
    pub fn matches(&self, comms_ok: bool, probe: &mut dyn FnMut(u16) -> Option<Vec<u8>>) -> bool {
        let mut last: Option<i64> = None;
        for t in &self.terms {
            if t == "comms_ok" {
                if !comms_ok {
                    return false;
                }
                continue;
            }
            if t == "comms_failed" {
                return false; // direct probing can't reproduce "listed by wakeup, but StartComms failed"
            }
            if let Some((did, wants, flag)) = parse_did_term(t) {
                let val = probe(did);
                if val.is_some() != wants {
                    return false;
                }
                if let Some(v) = val.filter(|v| !v.is_empty()) {
                    last = Some(if flag.is_some_and(|f| f & 0x400000 != 0) {
                        v[0] as i64
                    } else {
                        v.iter().take(4).fold(0i64, |a, b| a << 8 | *b as i64)
                    });
                }
                continue;
            }
            if let Some((op, n)) = parse_l8(t) {
                let Some(l) = last else { return false };
                let ok = match op {
                    "==" => l == n,
                    "!=" => l != n,
                    "<=" => l <= n,
                    ">=" => l >= n,
                    "<" => l < n,
                    _ => l > n,
                };
                if !ok {
                    return false;
                }
                continue;
            }
            return false;
        }
        true
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProbeRow {
    pub session: u8,
    pub format: Option<u8>,
    pub outcome: String,
    pub data: Option<String>,
}

pub struct HondaCan {
    pub adapter: Arc<dyn Adapter>,
    pub db: Arc<Database>,
    allow_clear: bool,
    clients: HashMap<u32, UdsClient>,
    pub functional: bool, // requests go out on OBD_FUNCTIONAL_ID, the reply filter picks the ECU
    /// The session's frame log, when one is attached. A write refuses to run without it.
    pub capture: Option<Arc<CaptureLog>>,
}

fn io_of(e: UdsError) -> Result<(), std::io::Error> {
    match e {
        UdsError::Io(e) => Err(e),
        _ => Ok(()),
    }
}

/// An OSError from the bus propagates; any other error is an ordinary failed exchange.
fn io_of_ref(e: &UdsError) -> Result<(), std::io::Error> {
    match e {
        UdsError::Io(e) => Err(std::io::Error::new(e.kind(), e.to_string())),
        _ => Ok(()),
    }
}

impl HondaCan {
    pub fn new(adapter: Arc<dyn Adapter>, db: Arc<Database>, allow_clear: bool) -> HondaCan {
        HondaCan { adapter, db, allow_clear, clients: HashMap::new(), functional: false, capture: None }
    }

    /// A note in the frame log (write start/stop, refusals); no-op without a log.
    pub fn note(&self, text: &str) {
        if let Some(log) = &self.capture {
            log.note(text);
        }
    }

    /// Arm (or disarm) `14 FF FF FF` on this handle and every client it has built: the RPC service clears
    /// through this; the CLI passes its choice to the constructor.
    pub fn set_allow_clear(&mut self, on: bool) {
        self.allow_clear = on;
        for c in self.clients.values_mut() {
            c.allow_clear = on;
        }
    }

    pub fn client(&mut self, addr: u32) -> &mut UdsClient {
        if !self.clients.contains_key(&addr) {
            let tx = if self.functional { OBD_FUNCTIONAL_ID } else { tx_id(addr) };
            let mut c = UdsClient::new(self.adapter.link(tx, rx_id(addr)));
            c.allow_clear = self.allow_clear;
            c.functional = self.functional;
            self.clients.insert(addr, c);
        }
        self.clients.get_mut(&addr).unwrap()
    }

    /// ReadMemoryByAddress on ECU `addr` through the default MemoryPolicy (RAM and flash windows, 64 bytes at most).
    /// An NRC 0x33 (SecurityAccess required) is answered with `UdsClient::security_access` and the read retried.
    /// If security_access fails, its error is returned (not the 0x33 that prompted it, which would hide why), and
    /// the client will not try again (see `security_access`); any other outcome, including a second 0x33,
    /// propagates as-is.
    /// Never read successfully on a car yet (memory.rs); the SecurityAccess exchange itself is unconfirmed too
    /// (the memory probe got NRC 0x33 before ever reaching it, README.md "Open work").
    pub fn read_memory(&mut self, addr: u32, address: u64, length: u64, fmt: u8) -> Result<Vec<u8>, UdsError> {
        let c = self.client(addr);
        c.memory = Some(MemoryPolicy::default());
        let mut r = c.read_memory(address, length, fmt);
        if let Err(UdsError::Negative { nrc: 0x33, .. }) = &r {
            r = match c.security_access() {
                Ok(()) => c.read_memory(address, length, fmt),
                Err(e) => Err(e),
            };
        }
        if let Err(UdsError::Negative { nrc: 0x33, .. }) = &r {
            // the 2015 Accord's PCM accepted the 01/02 key yet kept this read locked (2026-10-20):
            // the memory gate is the second pair's, 27 03/27 04.
            r = match c.security_access_2() {
                Ok(()) => c.read_memory(address, length, fmt),
                Err(e) => Err(e),
            };
        }
        c.memory = None;
        r
    }

    /// Which request does the ECU accept? Tries each address-and-length format in the default session, then in the
    /// extended one (10 03), stops at the first that answers. An NRC 0x33 means the read wants SecurityAccess, now
    /// attempted automatically (`read_memory` above; unconfirmed on a real car), 0x11 that the service does not exist.
    pub fn probe_memory(&mut self, addr: u32, address: u64, length: u64) -> Result<Vec<ProbeRow>, std::io::Error> {
        let mut out = vec![];
        let r = self.probe_inner(addr, address, length, &mut out);
        if out.iter().any(|r| r.session == 0x03) {
            // it went to 10 03: put it back
            if let Err(e) = self.client(addr).session(0x01) {
                io_of(e)?;
            }
        }
        r?;
        Ok(out)
    }

    fn probe_inner(&mut self, addr: u32, address: u64, length: u64, out: &mut Vec<ProbeRow>) -> Result<(), std::io::Error> {
        for session in [None, Some(0x03u8)] {
            if let Some(s) = session {
                if let Err(e) = self.client(addr).session(s) {
                    io_of_ref(&e)?;
                    out.push(ProbeRow { session: s, format: None, outcome: format!("10 03: {e}"), data: None });
                    continue;
                }
            }
            for fmt in FORMATS {
                let mut row = ProbeRow { session: session.unwrap_or(0x01), format: Some(fmt), outcome: String::new(), data: None };
                match self.read_memory(addr, address, length, fmt) {
                    Ok(d) => {
                        row.data = Some(d.iter().map(|b| format!("{b:02x}")).collect());
                        row.outcome = "ok".into();
                    }
                    Err(UdsError::Negative { nrc, .. }) => row.outcome = format!("NRC 0x{nrc:02X} {}", nrc_name(nrc)),
                    Err(UdsError::Io(e)) => return Err(e),
                    Err(e) => row.outcome = e.to_string(),
                }
                let done = row.outcome == "ok";
                out.push(row);
                if done {
                    return Ok(());
                }
            }
        }
        Ok(())
    }

    pub fn ping(&mut self, addr: u32) -> Result<bool, std::io::Error> {
        let c = self.client(addr);
        Self::ping_client(c)
    }

    fn ping_client(c: &mut UdsClient) -> Result<bool, std::io::Error> {
        match c.tester_present(Some(0)) {
            Ok(_) | Err(UdsError::Negative { .. }) => Ok(true), // a negative response is still an answer
            Err(UdsError::Io(e)) => Err(e),
            Err(_) => Ok(false),
        }
    }

    /// The addresses that answer TesterPresent (3E 00), each on its own id; if none does, the ones that answer it on
    /// OBD_FUNCTIONAL_ID, and from then on every request goes there (see `use_functional`).
    pub fn present(&mut self, addresses: &[u32], timeout: f64) -> Result<Vec<u32>, std::io::Error> {
        let mut addresses = addresses.to_vec();
        addresses.sort();
        let mut found = self.present_physical(&addresses, timeout)?;
        if found.is_empty() && self.adapter.has_broadcast() {
            found = self.answer_functional(OBD_FUNCTIONAL_ID, &addresses, timeout.max(0.3))?.into_keys().collect();
            if !found.is_empty() {
                self.use_functional(true);
            }
        } else if !found.is_empty() {
            self.use_functional(false);
        }
        Ok(found)
    }

    /// The addresses that answer 3E 00 on 0x18DA<tgt>F1. Adapters with `probe` send every request first and collect the
    /// answers in one window (raw CAN), or run the per-address requests with a short timeout (ELM); anything else is asked
    /// one address at a time.
    pub fn present_physical(&mut self, addresses: &[u32], timeout: f64) -> Result<Vec<u32>, std::io::Error> {
        if !self.adapter.has_probe() {
            let mut out = vec![];
            for a in addresses {
                let mut c = UdsClient::new(self.adapter.link(tx_id(*a), rx_id(*a)));
                if Self::ping_client(&mut c)? {
                    out.push(*a);
                }
            }
            return Ok(out);
        }
        let pairs: Vec<(u32, u32)> = addresses.iter().map(|a| (tx_id(*a), rx_id(*a))).collect();
        let answered = self.adapter.probe(&pairs, &[TESTER_PRESENT, 0x00], timeout).map_err(link_io)?;
        Ok(addresses.iter().filter(|a| answered.contains(&rx_id(**a))).cloned().collect())
    }

    /// {address: seconds after the request, or None (ELM)} of `addresses` answering one 3E 00 on `functional_id`.
    pub fn answer_functional(&mut self, functional_id: u32, addresses: &[u32], timeout: f64) -> Result<BTreeMap<u32, Option<f64>>, std::io::Error> {
        let rx: Vec<u32> = addresses.iter().map(|a| rx_id(*a)).collect();
        let got = self.adapter.broadcast(functional_id, &rx, &[TESTER_PRESENT, 0x00], timeout).map_err(link_io)?;
        Ok(got.into_iter().map(|(rx, t)| (rx & 0xFF, t)).collect())
    }

    /// Address requests to 0x18DA<tgt>F1, or to OBD_FUNCTIONAL_ID (`functional`). The 2015 Accord on the GD101
    /// (2026-10-11) ignored every 0x18DA<tgt>F1 request yet answered 3E 00, 22 F1 90 and OBD-II on 0x18DB33F1. Replies are
    /// still told apart by their id. Cause of the physical silence: unconfirmed (`hondascan wakeup` compares the ways).
    pub fn use_functional(&mut self, functional: bool) {
        if functional != self.functional {
            self.functional = functional;
            self.clients.clear();
        }
    }

    /// Who answers one functional TesterPresent (3E 00 on 0x18DBEFF1): {address: seconds after the request, or None where
    /// the adapter does not time each answer (ELM)}. Every reply id 0x18DAF100-FF is collected, also of addresses outside
    /// HDS's table.
    pub fn wakeup(&mut self, timeout: f64) -> Result<BTreeMap<u32, Option<f64>>, std::io::Error> {
        let all: Vec<u32> = (0..256).collect();
        self.answer_functional(FUNCTIONAL_ID, &all, timeout)
    }

    /// Find the ECUs among the addresses HDS knows, then resolve the systems with HDS's own DID rules. One Ecu per
    /// matching rule: DecodeCANAddress tests the rules of an address independently, so one controller can be several
    /// systems (0x0E: PGM-FI, CVT and EVPS at once), each with its own DTC table.
    pub fn discover(&mut self, addresses: Option<&[u32]>, progress: Option<&dyn Fn(u32)>) -> Result<Vec<Ecu>, std::io::Error> {
        let rules = self.db.can_rules();
        let all: Vec<u32> = rules.keys().map(|a| *a as u32).collect();
        let mut found = vec![];
        for addr in self.present(addresses.unwrap_or(&all), crate::uds::P2)? {
            if let Some(p) = progress {
                p(addr);
            }
            let mut cache: Probes = HashMap::new();
            let mut fatal: Option<std::io::Error> = None;
            let rows = rules.get(&(addr as i64)).cloned().unwrap_or_default();
            let mut hits = vec![];
            {
                let c = self.client(addr);
                let mut probe = |did: u16| -> Option<Vec<u8>> {
                    cache.entry(did).or_insert_with(|| {
                        
                        match c.read_did(did) {
                            Ok(d) => Some(d),
                            Err(UdsError::Io(e)) => {
                                fatal = Some(e);
                                None
                            }
                            Err(_) => None,
                        }
                    });
                    cache[&did].clone()
                };
                for (stype, rule_text, hint) in &rows {
                    if Rule::new(rule_text).matches(true, &mut probe) {
                        hits.push((stype.clone(), rule_text.clone(), hint.clone()));
                    }
                }
            }
            if let Some(e) = fatal {
                return Err(e);
            }
            for (stype, rule_text, hint) in &hits {
                let system = self.db.vcan_system_for_type(stype, hint).unwrap_or_default();
                found.push(Ecu { address: addr, system_type: stype.clone(), system, rule: rule_text.clone(), probes: cache.clone() });
            }
            if hits.is_empty() {
                found.push(Ecu { address: addr, system_type: "?".into(), system: String::new(), rule: "answered, no HDS rule matched".into(), probes: cache });
            }
        }
        Ok(found)
    }

    pub fn read_dtcs(&mut self, ecu: &Ecu, mask: u8) -> Result<DtcReadout, UdsError> {
        let (avail, recs) = self.client(ecu.address).read_dtcs(mask)?;
        let mut out = vec![];
        for r in recs {
            let system = (!ecu.system.is_empty()).then_some(ecu.system.as_str());
            let hit = self.db.lookup_dtc(&r.code, Some(r.failure_type as i64), system, Some(r.kind() == "pending")).into_iter().next();
            out.push((r, hit));
        }
        Ok((avail, out))
    }

    pub fn clear_dtcs(&mut self, ecu: &Ecu) -> Result<Vec<u8>, UdsError> {
        self.client(ecu.address).clear_dtcs()
    }
}

fn link_io(e: crate::transport::isotp::LinkError) -> std::io::Error {
    match e {
        crate::transport::isotp::LinkError::Io(e) => e,
        crate::transport::isotp::LinkError::IsoTp(e) => std::io::Error::other(e.0),
    }
}
