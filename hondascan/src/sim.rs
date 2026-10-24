//! Simulated Honda VCAN vehicle on an in-process CAN bus, for tests and for trying the CLI without a car. Port of
//! hondascan/sim.py.
//!
//! Each simulated ECU answers ISO 14229 requests on 0x18DA<addr>F1 -> 0x18DAF1<addr> through real ISO-TP framing, so
//! everything above the CAN frame (segmentation, flow control, NRC 0x78 handling, discovery rules, DTC decoding) is
//! exercised. The bus behaves like SocketCAN (receive filters, a 10-frame transmit queue). The data are invented; codes
//! are real database codes.
use crate::actuator::TESTS;
use crate::db::Database;
use crate::honda::{rx_id, tx_id, FUNCTIONAL_ID};
use crate::simcapture::{self, CaptureEcu};
use crate::simdrive::Drive;
use crate::transport::can::{CaptureLog, VirtualBus};
use crate::transport::isotp::{DiagLink, HDS_PAD};
use crate::transport::{Adapter, RawCanAdapter};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub type Values = BTreeMap<(u16, usize), Vec<u8>>;
pub type Overlay = Arc<dyn Fn(u16) -> Values + Send + Sync>;

/// An overlay that always returns these values (the DID filter happens when it is applied).
pub fn overlay_fn(v: Values) -> Overlay {
    Arc::new(move |_| v.clone())
}

/// Write the `values` of this DID over its data, in place; values past the end are dropped.
pub fn overlay(data: &mut [u8], did: u16, values: &Values) {
    for ((vd, off), b) in values {
        if *vd == did && off + b.len() <= data.len() {
            data[*off..off + b.len()].copy_from_slice(b);
        }
    }
}

/// DID 0x2600 bytes 0-4 (ECUID1-5) of the 2015 Accord's PCM as measured (README.md, "Measured on the car (2026-10-12)"): HDS's ECU 5A0-A510.
pub const ECUID_5A0_A510: [u8; 5] = [0x42, 0x15, 0x00, 0x00, 0x01];
pub const RAM_START: u64 = 0xFFF8_0000;

/// 0x20000 bytes of RAM from 0xFFF80000 with the ignition module's variables set to plausible values (big endian).
pub fn ram_image() -> Vec<u8> {
    let mut ram = vec![0u8; 0x20000];
    for (address, value, size) in [(0xFFF9098Eu64, 800u64, 2usize), (0xFFF906D2, 2500, 2), (0xFFF90B0C, 300, 2), (0xFFF94520, 1, 1), (0xFFF918B0 + 2 * 5, 150, 2), (0xFFF918B0 + 2 * 0x38, 130, 2)] {
        let o = (address - RAM_START) as usize;
        ram[o..o + size].copy_from_slice(&value.to_be_bytes()[8 - size..]);
    }
    ram
}

/// ISO 14229-1: to a functional request an ECU sends no negative response with these codes (it stays silent).
const SILENT_ON_FUNCTIONAL: [u8; 5] = [0x11, 0x12, 0x31, 0x7E, 0x7F];

/// What an ECU answers: nothing, one response, or several (responsePending first).
pub enum Reply {
    None,
    One(Vec<u8>),
    Many(Vec<Vec<u8>>),
}

fn nrc(sid: u8, code: u8) -> Vec<u8> {
    vec![0x7F, sid, code]
}

/// A RoutineControl the fake knows (write.rs / writerun.rs): the routine ids it accepts (HDS's meter menu runs items
/// A and 1-5 through one procedure, so the fake lists all six ids), a fixed option record (None:
/// no bytes after the id, as HDS's CVT_ISL sends), and a read-back field that only takes `value` `wait_s` seconds after
/// the start. When `status` is Some, HDS's proof is `RoutineControl(3)` instead: 0x7F before the wait, then that status
/// byte. Every other id, control and option is refused; a start while another routine is still running is a
/// requestSequenceError (a completed one does not block: HDS's USM_Item menu resets several items in one session), and
/// a status poll or stop for an id other than the one started is a requestSequenceError. `needs_extended` is false for
/// the meter reset, which HDS sends in the default session.
#[derive(Debug, Clone, Copy)]
pub struct Routine {
    pub ids: &'static [u16],
    pub option: Option<u8>,
    pub did: u16,
    pub offset: usize,
    pub mask: u8,
    pub value: u8,
    pub wait_s: f64,
    pub status: Option<u8>,
    pub needs_extended: bool,
}

/// A strict fake ECU. `memory` ({start: bytes}) makes it answer ReadMemoryByAddress the strict way: only the formats in
/// `mem_formats` (any other: NRC 0x13, as the PCM does), exact request length, a read lying wholly inside one region, at
/// most `mem_max` bytes, in the extended session if `mem_extended`, and not before a real SecurityAccess exchange
/// (`security_access`, checked against `seedkey::calc_key`) if `mem_security` (NRC 0x33 until then; with `pcm_lock`, the car's
/// gate, NRC 0x33 for good). Without memory:
/// serviceNotSupported.
pub struct SimEcu {
    pub addr: u32,
    pub system: String,       // database system whose DIDs this ECU serves
    pub also: Vec<String>,    // more systems in the same controller (HDS allows 0x10: PGM-FI + EVPS)
    pub dids: BTreeMap<u16, Vec<u8>>,
    pub values: Values,       // (did, offset) -> bytes written over the zero-filled DID data
    pub dtcs: Vec<[u8; 4]>,   // (hi, mid, failure_type, status)
    pub pending_on_clear: bool,
    pub requests: Vec<Vec<u8>>,
    pub overlay: Option<Overlay>, // values that change over time
    pub offline: bool,        // the ECU stays silent, as with the ignition off
    pub functional: bool,     // answers requests on the functional id 0x18DBEFF1 (unconfirmed for Honda)
    pub memory: Option<BTreeMap<u64, Vec<u8>>>,
    pub mem_formats: Vec<u8>,
    pub mem_max: u64,
    pub mem_extended: bool,
    pub mem_security: bool,
    pub extended: bool,
    pub unlocked: bool,
    /// The 2015 Accord PCM's real gate (see `with_pcm_security`): 2-byte seeds, a level 2, and 0x23 locked for good.
    pub pcm_lock: bool,
    pcm_seeds_given: usize,
    pcm_bad_keys: u8,
    pcm_pending: Option<(u8, u16)>, // (level request byte, seed) of the last 27 01 / 27 03 on a pcm_lock ECU
    pub sec_type: u8,               // the type byte security_access hands out with the seed (default: 1, has a defined key)
    sec_pending: Option<(u16, u8)>, // (seed, type) sent by the last 27 01, waiting for 27 02
    /// IOControlByIdentifier: {did: (allowed values, needs_engine_off)}. Strict: extended session only, which ends after `s3`
    /// seconds without a request (dropping every override); an override not refreshed within `io_timeout` is dropped too.
    pub io: Option<BTreeMap<u16, (Vec<u16>, bool)>>,
    pub io_timeout: f64,
    pub s3: f64,
    pub io_active: BTreeMap<u16, (u16, Instant)>,
    pub engine_running: bool,
    /// The one RoutineControl this ECU accepts (None: service 0x31 not supported). Its DID read-back bit (or monitored
    /// status) appears only after `wait_s`; a second start before a stop is a sequence error and a stop without a start
    /// is refused.
    pub routine: Option<Routine>,
    routine_started: Option<(u16, Instant)>,
    routine_done: bool,
    /// Test hook: the first memory read is answered this many seconds late (a reply after P2).
    pub slow_memory_once: Option<f64>,
    /// Fault injection: (request prefix, replies to lose). The ECU acts on a matching request, then its reply never arrives.
    pub drop_replies: Option<(Vec<u8>, usize)>,
    last: Instant,
}

impl SimEcu {
    pub fn new(addr: u32, system: &str, dids: &[(u16, &[u8])], dtcs: &[[u8; 4]]) -> SimEcu {
        SimEcu {
            addr,
            system: system.into(),
            also: vec![],
            dids: dids.iter().map(|(d, b)| (*d, b.to_vec())).collect(),
            values: BTreeMap::new(),
            dtcs: dtcs.to_vec(),
            pending_on_clear: false,
            requests: vec![],
            overlay: None,
            offline: false,
            functional: true,
            memory: None,
            mem_formats: vec![0x24],
            mem_max: 32,
            mem_extended: false,
            mem_security: false,
            extended: false,
            unlocked: true,
            pcm_lock: false,
            pcm_seeds_given: 0,
            pcm_bad_keys: 0,
            pcm_pending: None,
            sec_type: 1,
            sec_pending: None,
            io: None,
            io_timeout: 3.0,
            s3: 5.0,
            io_active: BTreeMap::new(),
            engine_running: false,
            routine: None,
            routine_started: None,
            routine_done: false,
            slow_memory_once: None,
            drop_replies: None,
            last: Instant::now(),
        }
    }

    pub fn with_memory(mut self, mem: BTreeMap<u64, Vec<u8>>) -> SimEcu {
        self.memory = Some(mem);
        self
    }

    pub fn with_security(mut self) -> SimEcu {
        self.mem_security = true;
        self.unlocked = false;
        self
    }

    /// The 2015 Accord PCM's memory gate as measured on the car (2026-10-20; README "Spark inputs", "The `0x23` memory
    /// gate"; commit 414a935): `27 01` answers `67 01 hh ll` (2 seed bytes, no type byte), `27 02` wants
    /// `seedkey::calc_key_pcm(seed)` and answers `67 02` (seed 0x6635 -> key 0x6A48 and seed 0x4FBA -> key 0x5F18, both
    /// accepted), a wrong key is NRC 0x35. Level 2 (`27 03` / `27 04`) rejected the level-1 key with NRC 0x35. `23` stays
    /// NRC 0x33 after a valid level 1, in both sessions: its lock flag 0xFFF944ED is set once from ROM byte 0x45812
    /// (= 0x01) and nothing at run time clears it, so `unlocked` is never set here. Unconfirmed: whether the car
    /// answers `27 03` with a 2-byte seed (this fake does) and what its level-2 key is (none is accepted).
    /// The seeds are the two measured ones, handed out in turn. The attempt counter is ASSUMED, not measured (the car's
    /// NRC 0x36/0x37 behaviour was never observed; the client stops after one rejected key, uds.rs): ISO 14229's
    /// pattern of `PCM_MAX_KEY_TRIES` wrong keys, the last one answered 0x36, then 0x37 for every 27 (the delay never
    /// expires in a fake), reset by a valid level-1 key. A stricter-than-car fake is the safe side for the client.
    pub fn with_pcm_security(mut self) -> SimEcu {
        self = self.with_security();
        self.pcm_lock = true;
        self
    }

    pub fn with_routine(mut self, routine: Routine) -> SimEcu {
        self.routine = Some(routine);
        self
    }

    fn expire(&mut self) {
        let now = Instant::now();
        if self.extended && (now - self.last).as_secs_f64() > self.s3 {
            self.extended = false;
            self.io_active.clear();
            // A routine that needs the extended session and had not finished when S3 ran out is aborted (a strict guess:
            // nothing decoded says what the PCM does; a lenient fake would hide a missing keepalive).
            let s3_end = self.last + Duration::from_secs_f64(self.s3);
            if let (Some(r), Some((_, t))) = (self.routine, self.routine_started) {
                if r.needs_extended && !self.routine_done && t + Duration::from_secs_f64(r.wait_s) > s3_end {
                    self.routine_started = None;
                }
            }
        }
        let timeout = self.io_timeout;
        self.io_active.retain(|_, (_, t)| (now - *t).as_secs_f64() <= timeout);
        if let (Some(r), Some((_, t))) = (self.routine, self.routine_started) {
            if (now - t).as_secs_f64() >= r.wait_s {
                self.routine_done = true;
            }
        }
        self.last = now;
    }

    fn routine_control(&mut self, req: &[u8]) -> Vec<u8> {
        let Some(r) = self.routine else { return nrc(0x31, 0x11) };
        if req.len() < 4 {
            return nrc(0x31, 0x13);
        }
        if r.needs_extended && !self.extended {
            return nrc(0x31, 0x7F);
        }
        let (control, rid) = (req[1], (req[2] as u16) << 8 | req[3] as u16);
        if !r.ids.contains(&rid) {
            return nrc(0x31, 0x31); // requestOutOfRange: HDS defines no other routine here
        }
        let want = 4 + r.option.map_or(0, |_| 1);
        if req.len() != want || r.option.is_some_and(|o| req[4] != o) {
            return nrc(0x31, 0x13); // longer, shorter or an option HDS never sends
        }
        match control {
            0x01 => {
                if self.engine_running {
                    return nrc(0x31, 0x22); // conditionsNotCorrect: HDS requires the engine not running
                }
                if self.routine_started.is_some() && !self.routine_done {
                    return nrc(0x31, 0x24); // requestSequenceError: the previous routine is still running
                }
                self.routine_started = Some((rid, Instant::now()));
                self.routine_done = false;
                vec![0x71, 0x01, req[2], req[3]]
            }
            0x03 => {
                let Some(status) = r.status else { return nrc(0x31, 0x12) };
                let Some((sid, t)) = self.routine_started else { return nrc(0x31, 0x24) };
                if sid != rid {
                    return nrc(0x31, 0x24); // another routine's status: HDS polls the one it started
                }
                if (Instant::now() - t).as_secs_f64() >= r.wait_s {
                    self.routine_done = true;
                }
                let value = if self.routine_done { status } else { 0x7F }; // HDS's own pending value
                vec![0x71, 0x03, req[2], req[3], value, 0, 0]
            }
            0x02 if r.status.is_some() => nrc(0x31, 0x12), // HDS sends no stop for the meter routine
            0x02 => {
                if self.routine_started.is_none_or(|(sid, _)| sid != rid) {
                    return nrc(0x31, 0x24); // nothing running, or another routine than the one started
                }
                self.routine_started = None;
                vec![0x71, 0x02, req[2], req[3]]
            }
            _ => nrc(0x31, 0x12), // subFunctionNotSupported: only start and stop are whitelisted
        }
    }

    fn io_control(&mut self, req: &[u8]) -> Vec<u8> {
        let Some(io) = &self.io else { return nrc(0x2F, 0x11) };
        if req.len() < 4 {
            return nrc(0x2F, 0x13);
        }
        let (did, option) = ((req[1] as u16) << 8 | req[2] as u16, req[3]);
        let Some((values, needs_engine_off)) = io.get(&did) else { return nrc(0x2F, 0x31) };
        if !self.extended {
            return nrc(0x2F, 0x7F);
        }
        match option {
            0 => {
                if req.len() != 4 {
                    return nrc(0x2F, 0x13);
                }
                self.io_active.remove(&did);
            }
            3 => {
                if req.len() != 6 {
                    return nrc(0x2F, 0x13);
                }
                let value = (req[4] as u16) << 8 | req[5] as u16;
                if !values.contains(&value) {
                    return nrc(0x2F, 0x31);
                }
                if *needs_engine_off && self.engine_running {
                    return nrc(0x2F, 0x22);
                }
                self.io_active.insert(did, (value, Instant::now()));
            }
            _ => return nrc(0x2F, 0x31),
        }
        vec![0x6F, req[1], req[2], option]
    }

    /// SecurityAccess (an NRC on any rule broken): `27 01` hands out a fixed (seed, type) and remembers it; `27
    /// 02 <keyH><keyL>` checks the key against `seedkey::calc_key` for that same pair -- the real algorithm, not a
    /// stub, so this fake actually enforces the seed/key exchange rather than granting access unconditionally
    /// (CLAUDE.md "Guard rails": fakes at least as strict as the hardware). Default `sec_type` is 1. This 3-byte-seed
    /// layout is HDS's CM.SecurityAccess one, a hypothetical for other ECUs: the 2015 PCM does NOT behave so (it is
    /// `with_pcm_security`).
    fn security_access(&mut self, req: &[u8]) -> Vec<u8> {
        if !self.mem_security {
            return nrc(0x27, 0x11);
        }
        if self.pcm_lock {
            return self.pcm_security_access(req);
        }
        match req.get(1) {
            Some(0x01) if req.len() == 2 => {
                let (seed, type_) = (0x1234, self.sec_type);
                self.sec_pending = Some((seed, type_));
                vec![0x67, 0x01, (seed >> 8) as u8, (seed & 0xFF) as u8, type_]
            }
            Some(0x02) if req.len() == 4 => {
                let Some((seed, type_)) = self.sec_pending.take() else { return nrc(0x27, 0x24) }; // requestSequenceError
                let key = (req[2] as u16) << 8 | req[3] as u16;
                if crate::seedkey::calc_key(seed, type_) == Some(key) {
                    self.unlocked = true;
                    vec![0x67, 0x02]
                } else {
                    nrc(0x27, 0x35) // invalidKey
                }
            }
            _ => nrc(0x27, 0x13),
        }
    }

    /// The PCM's SecurityAccess, as `with_pcm_security` documents. Key checks use `calc_key_pcm` for level 1; level 2
    /// accepts no key.
    fn pcm_security_access(&mut self, req: &[u8]) -> Vec<u8> {
        const SEEDS: [u16; 2] = [0x6635, 0x4FBA]; // measured on the car, 2026-10-20
        const PCM_MAX_KEY_TRIES: u8 = 3; // assumed (ISO 14229 practice), not measured
        if self.pcm_bad_keys >= PCM_MAX_KEY_TRIES {
            return nrc(0x27, 0x37); // requiredTimeDelayNotExpired
        }
        match req.get(1) {
            Some(&lvl @ (0x01 | 0x03)) if req.len() == 2 => {
                let seed = SEEDS[self.pcm_seeds_given % SEEDS.len()];
                self.pcm_seeds_given += 1;
                self.pcm_pending = Some((lvl, seed));
                vec![0x67, lvl, (seed >> 8) as u8, seed as u8]
            }
            Some(&lvl @ (0x02 | 0x04)) if req.len() == 4 => {
                let Some((asked, seed)) = self.pcm_pending.take() else { return nrc(0x27, 0x24) };
                if asked + 1 != lvl {
                    return nrc(0x27, 0x24);
                }
                let key = (req[2] as u16) << 8 | req[3] as u16;
                if lvl == 0x02 && key == crate::seedkey::calc_key_pcm(seed) {
                    self.pcm_bad_keys = 0;
                    vec![0x67, 0x02] // accepted, yet 0x23 stays locked (see read_memory)
                } else {
                    self.pcm_bad_keys += 1;
                    nrc(0x27, if self.pcm_bad_keys >= PCM_MAX_KEY_TRIES { 0x36 } else { 0x35 })
                }
            }
            _ => nrc(0x27, 0x13),
        }
    }

    /// The ReadMemoryByAddress answer for `req` (an NRC on any rule broken).
    fn read_memory(&self, req: &[u8]) -> Vec<u8> {
        let Some(mem) = &self.memory else { return nrc(0x23, 0x11) };
        if req.len() < 2 {
            return nrc(0x23, 0x13);
        }
        let fmt = req[1];
        let (alen, mlen) = ((fmt & 0x0F) as usize, (fmt >> 4) as usize);
        if !((1..=4).contains(&alen) && (1..=4).contains(&mlen)) || req.len() != 2 + alen + mlen {
            return nrc(0x23, 0x13);
        }
        if !self.mem_formats.contains(&fmt) {
            return nrc(0x23, 0x13); // the 2015 PCM answers every other format with 0x13 (car, 2026-10-15)
        }
        if self.mem_extended && !self.extended {
            return nrc(0x23, 0x7F);
        }
        if !self.unlocked || self.pcm_lock {
            return nrc(0x23, 0x33); // with pcm_lock: for good, whatever 27 answered (the NRC order against 0x31 is not measured)
        }
        let be = |b: &[u8]| b.iter().fold(0u64, |v, x| v << 8 | *x as u64);
        let (address, length) = (be(&req[2..2 + alen]), be(&req[2 + alen..]));
        if !(1..=self.mem_max).contains(&length) {
            return nrc(0x23, 0x31);
        }
        for (start, data) in mem {
            if *start <= address && address + length <= start + data.len() as u64 {
                let o = (address - start) as usize;
                let mut out = vec![0x63];
                out.extend(&data[o..o + length as usize]);
                return out;
            }
        }
        nrc(0x23, 0x31)
    }

    pub fn handle(&mut self, req: &[u8]) -> Reply {
        self.requests.push(req.to_vec());
        let reply = self.answer(req);
        if let Some((prefix, left)) = &mut self.drop_replies {
            if *left > 0 && req.starts_with(prefix) {
                *left -= 1;
                return Reply::None; // the ECU acted on the request; the reply was lost on the bus
            }
        }
        reply
    }

    fn answer(&mut self, req: &[u8]) -> Reply {
        if self.offline {
            return Reply::None;
        }
        self.expire();
        let sid = req[0];
        match sid {
            0x3E => {
                if req.len() > 1 && req[1] & 0x80 != 0 { Reply::None } else { Reply::One(vec![0x7E, req.get(1).copied().unwrap_or(0)]) }
            }
            0x23 => {
                if let Some(s) = self.slow_memory_once.take() {
                    std::thread::sleep(Duration::from_secs_f64(s));
                }
                Reply::One(self.read_memory(req))
            }
            0x2F => Reply::One(self.io_control(req)),
            0x31 => Reply::One(self.routine_control(req)),
            0x27 => Reply::One(self.security_access(req)),
            0x10 if req.len() == 2 && (req[1] == 1 || req[1] == 3) => {
                self.extended = req[1] == 3;
                if !self.extended {
                    self.io_active.clear(); // leaving the extended session ends every override
                }
                Reply::One(vec![0x50, req[1], 0x00, 0x32, 0x01, 0xF4])
            }
            0x22 if req.len() == 3 => {
                let did = (req[1] as u16) << 8 | req[2] as u16;
                match self.dids.get(&did) {
                    Some(d) => {
                        let mut data = d.clone();
                        overlay(&mut data, did, &self.values);
                        if let Some(o) = &self.overlay {
                            overlay(&mut data, did, &o(did));
                        }
                        // The learn's read-back field appears only after the fake's full wait (and stays after the stop,
                        // a guess: the car's flag is only known from HDS's success test).
                        if let Some(r) = self.routine {
                            if did == r.did && self.routine_done {
                                if let Some(b) = data.get_mut(r.offset) {
                                    *b = (*b & !r.mask) | (r.value & r.mask);
                                }
                            }
                        }
                        let mut out = vec![0x62, req[1], req[2]];
                        out.extend(data);
                        Reply::One(out)
                    }
                    None => Reply::One(nrc(0x22, 0x31)),
                }
            }
            0x19 if req.len() == 3 && req[1] == 0x02 => {
                let mut out = vec![0x59, 0x02, 0xFF];
                for d in self.dtcs.iter().filter(|d| d[3] & req[2] != 0) {
                    out.extend(d);
                }
                Reply::One(out)
            }
            0x19 if req.len() == 3 && req[1] == 0x01 => {
                let n = self.dtcs.iter().filter(|d| d[3] & req[2] != 0).count();
                Reply::One(vec![0x59, 0x01, 0xFF, 0x01, (n >> 8) as u8, (n & 0xFF) as u8])
            }
            0x14 if req.len() == 4 => {
                self.dtcs.clear();
                if self.pending_on_clear { Reply::Many(vec![vec![0x7F, 0x14, 0x78], vec![0x54]]) } else { Reply::One(vec![0x54]) }
            }
            _ => Reply::One(nrc(sid, 0x11)),
        }
    }
}

/// An ECU on the simulated bus: a scripted one (boxed: it is by far the larger variant), or one that plays back a recorded car.
pub enum SimNode {
    Scripted(Box<SimEcu>),
    Captured(CaptureEcu),
}

impl SimNode {
    pub fn addr(&self) -> u32 {
        match self {
            SimNode::Scripted(e) => e.addr,
            SimNode::Captured(e) => e.addr,
        }
    }

    pub fn system(&self) -> &str {
        match self {
            SimNode::Scripted(e) => &e.system,
            SimNode::Captured(e) => &e.system,
        }
    }

    pub fn also(&self) -> &[String] {
        match self {
            SimNode::Scripted(e) => &e.also,
            SimNode::Captured(e) => &e.also,
        }
    }

    /// The ids it listens on for requests.
    pub fn listens(&self) -> Vec<u32> {
        match self {
            SimNode::Scripted(e) => vec![tx_id(e.addr), FUNCTIONAL_ID],
            SimNode::Captured(e) => e.listens.clone(),
        }
    }

    pub fn functional(&self) -> bool {
        match self {
            SimNode::Scripted(e) => e.functional,
            SimNode::Captured(_) => true,
        }
    }

    pub fn set_offline(&mut self, offline: bool) {
        match self {
            SimNode::Scripted(e) => e.offline = offline,
            SimNode::Captured(e) => e.offline = offline,
        }
    }

    pub fn requests(&self) -> &[Vec<u8>] {
        match self {
            SimNode::Scripted(e) => &e.requests,
            SimNode::Captured(e) => &e.requests,
        }
    }

    pub fn handle(&mut self, req: &[u8]) -> Reply {
        match self {
            SimNode::Scripted(e) => e.handle(req),
            SimNode::Captured(e) => e.handle(req).map_or(Reply::None, Reply::One),
        }
    }

    pub fn scripted(&mut self) -> Option<&mut SimEcu> {
        match self {
            SimNode::Scripted(e) => Some(e),
            _ => None,
        }
    }
}

fn accord() -> Vec<SimEcu> {
    let mut pcm = SimEcu::new(0x10, "CANFI", &[(0x2600, &[1]), (0xF190, b"1HGCR2F3XFA000000"), (0xF187, b"37820-5A2-A57"), (0xF18C, b"SIM0000010")], &[[0x04, 0x20, 0x00, 0x2F]]);
    pcm.values = Values::from([
        ((0x2610, 6), 3200u16.to_be_bytes().to_vec()), // NE 800 rpm (0.25 * raw)
        ((0x2610, 11), vec![130]),                    // ECT 90 degC (raw - 40)
        ((0x2610, 18), vec![0]),                      // VSP 0 km/h
    ]);
    let mut abs = SimEcu::new(0x28, "CANABS", &[(0x4000, &[1]), (0xF187, b"57100-T2F-A51")], &[[0x40, 0x10, 0x49, 0x09]]);
    abs.pending_on_clear = true;
    abs.values = Values::from([
        ((0x4004, 15), vec![0x01]),                             // BLSSTS brake switch on
        ((0x4008, 8), 84312u32.to_be_bytes()[1..].to_vec()),    // TPMS_ODOINIU: DWS calibrated at 84312 km
    ]);
    vec![
        pcm,
        SimEcu::new(0x1E, "CANCVT", &[(0x2300, &[1])], &[]),
        abs,
        SimEcu::new(0x30, "CANEPS", &[(0x4800, &[1])], &[[0x02, 0x03, 0x00, 0x20]]), // P0203, stored only: failed since the last clear (bit 5), passing now
        SimEcu::new(0x53, "CANSRS", &[(0x8000, &[1])], &[[0x80, 0x01, 0x13, 0x08], [0x80, 0x01, 0x1A, 0x08]]),
        SimEcu::new(0x60, "CAN_METER", &[(0x7000, &[1])], &[]),
        SimEcu::new(0x26, "CAN_TPMS", &[(0x6000, &[1])], &[]),
        SimEcu::new(0xEF, "CANGW", &[(0xC000, &[1])], &[]),
    ]
}

/// The recorded car on the J2534 path: which database systems the PCM (PGM-FI and CVT), the SRS and the meter are.
const J2534_CAR: &[(u32, &[&str])] = &[(0x0E, &["CANFI", "CANCVT"]), (0x53, &["CANSRS"]), (0x60, &["CAN_METER"])];

pub const PROFILE_NAMES: [&str; 14] = ["accord2015", "accord2015-ram", "accord2015-ram-locked", "accord2015-io", "accord2015-write", "accord2015-write-cvt", "accord2015-write-meter", "accord2015-stored", "drive", "evps", "accord2015-gd101", "accord2015-gd101-j2534", "accord2015-gd101-j2534-drive-home", "empty"];

/// The routine ids of the meter's US smart-maintenance resets that HDS's USM_Item menu defines (`sub_7231`, Meterapp
/// 2207-2244) and this car offers: item A (0x7021, always offered) and items 1-5 (0x7022-0x7026, their enable bits set;
/// the car's DID 0x7068 byte 7 = 0xF8). Items 6-7 (0x7027/0x7028) are in HDS's table but their bits are clear here, so
/// the fake refuses them: HDS's menu would never offer them, and the fake stays on the strict side of what the ECU is
/// known to do.
const METER_USM_IDS: [u16; 6] = [0x7021, 0x7022, 0x7023, 0x7024, 0x7025, 0x7026];

/// The meter's DID 0x7068 as the car answered `22 70 68` (the J2534 recording of 2026-10-12, first reply, 54 bytes; the
/// database's packet size for CAN_METER 0x7068 is 54 too). byte 7 = the item enable bits (0xF8: items 1-5 offered, 6-9
/// not), byte 9 bit 0x01 = ITAFLAG and bit 0x02 = IT3JDGMETH (both 0 here), bytes 10-11 = OUT_USS_ITARDI (item A's
/// remaining value, 0x003C = 60), 12-13 = item 1 (0x0C1E), 14-15 = item 2 (0x17F9), 16-17 = item 3 (0 — with
/// IT3JDGMETH = 0 item 3 is not judged by distance and HDS displays "----"), 18-19 = item 4 (0x73F9), 20-21 = item 5
/// (0x7FFF). The tail (bytes 22-53: items 6-9's distances and the remaining-days fields) reads zero in the recording.
const METER_7068: [u8; 54] = [
    0x5F, 0xFF, 0xF0, 0x00, 0x00, 0x00, 0x00, 0xF8, 0x00, 0x00, 0x00, 0x3C, 0x0C, 0x1E, 0x17, 0xF9,
    0x00, 0x00, 0x73, 0xF9, 0x7F, 0xFF, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

pub fn profile(name: &str) -> Result<Vec<SimNode>, String> {
    let scripted = |v: Vec<SimEcu>| v.into_iter().map(|e| SimNode::Scripted(Box::new(e))).collect::<Vec<_>>();
    Ok(match name {
        // Roughly a 2013-2016 Accord: engine, CVT, VSA, EPS, SRS, gauges, TPMS, gateway.
        "accord2015" | "drive" => scripted(accord()), // 'drive': the same car, driving the cycle in simdrive.rs
        // The PCM with a RAM image, to try memory reads without a car (the real ECU's rules are unknown: this is a strict guess).
        "accord2015-ram" => {
            let mut e = SimEcu::new(0x10, "CANFI", &[(0x2600, &ECUID_5A0_A510)], &[]).with_memory(BTreeMap::from([(RAM_START, ram_image())]));
            e.mem_extended = true;
            scripted(vec![e])
        }
        // The PCM as measured on the car (2026-10-15 probe, 2026-10-20 SecurityAccess; README "The `0x23` memory gate"):
        // only format 0x14 is parsed (every other: 0x13); 27 01 answers a 2-byte seed, 27 02 takes calc_key_pcm and
        // answers 67 02, level 2 (27 03/04) rejects that key (0x35), and 23 stays NRC 0x33 in both sessions after any of
        // it, because the lock flag 0xFFF944ED is never cleared at run time. Nothing can read this memory, as on the car.
        // For a memory that can be read, use "accord2015-ram" (a hypothetical, not the car).
        "accord2015-ram-locked" => {
            let mut e = SimEcu::new(0x10, "CANFI", &[(0x2600, &ECUID_5A0_A510)], &[]).with_memory(BTreeMap::from([(RAM_START, ram_image())])).with_pcm_security();
            e.mem_formats = vec![0x14];
            scripted(vec![e])
        }
        // The PCM with the actuator tests and the signals HDS checks before them (engine off, park, battery, A/C off).
        "accord2015-io" => {
            let mut e = SimEcu::new(0x10, "CANFI", &[(0x2600, &ECUID_5A0_A510)], &[]);
            e.io = Some(TESTS.iter().map(|t| (t.did, (t.values.to_vec(), true))).collect());
            e.values = Values::from([((0x2610, 22), vec![126]), ((0x2661, 7), vec![0x10])]); // VB 12.6 V; ATNP: PNP switch bit (mask 0x10) = P-N
            scripted(vec![e])
        }
        // The PCM with the first whitelisted write: E-VTC learn (routine 0x0218, option 02), engine off and in park,
        // a VIN, and the read-back DID. The 10 s wait is real; tests shorten `routine.wait_s`.
        "accord2015-write" => {
            let mut e = SimEcu::new(
                0x10,
                "CANFI",
                &[(0x2600, &ECUID_5A0_A510), (0xF190, b"1HGCR2F3XFA000000"), (0x2610, &[0; 24]), (0x2661, &[0; 8])],
                &[],
            );
            e.values = Values::from([((0x2610, 6), vec![0, 0]), ((0x2661, 7), vec![0x10])]); // NE 0 rpm; ATNP = P-N
            e.routine = Some(Routine { ids: &[0x218], option: Some(0x02), did: 0x266C, offset: 37, mask: 0x80, value: 0x80, wait_s: 10.0, status: None, needs_extended: true });
            scripted(vec![e])
        }
        // The CVT with the second whitelisted write: CVT_ISL initial learn stage one (routine 0x1148, no option bytes),
        // park (ATPP = 1), engine off (NEW = 0), and the SLPLMOD/SLPLERR read-back. The fake completes at once; the real
        // HDS poll window is 8 s.
        "accord2015-write-cvt" => {
            let mut e = SimEcu::new(0x0E, "CANCVT", &[(0xF190, b"1HGCR2F3XFA000000")], &[]);
            e.values = Values::from([
                ((0x2321, 5), vec![0x80]), // ATPP = 1
                ((0x2301, 7), vec![0, 0]), // NEW = 0 rpm
                ((0x2340, 29), vec![0]),   // SLPLMOD = 0
                ((0x2340, 30), vec![0]),   // SLPLERR = 0
            ]);
            e.routine = Some(Routine { ids: &[0x1148], option: None, did: 0x2340, offset: 29, mask: 0xFF, value: 2, wait_s: 0.0, status: None, needs_extended: true });
            scripted(vec![e])
        }
        // The meter with the third whitelisted write family: US smart maintenance resets, items A and 1-5 (routines
        // 0x7021-0x7026, no option bytes, proof `RoutineControl(3)` status) in the default session, plus the PCM whose
        // F190 answers the VIN gate (the meter refuses F190 with 7F 22 31, so the car's VIN check reads the PCM). The
        // fake completes at once; hondascan's 10 s timeout is a safety bound. HDS's proof is the status byte only: it
        // never re-reads DID 0x7068, so the routine's read-back overlay is a no-op here (mask 0).
        "accord2015-write-meter" => {
            let pcm = SimEcu::new(0x10, "CANFI", &[(0xF190, b"1HGCR2F3XFA000000")], &[]);
            let mut meter = SimEcu::new(0x60, "CAN_METER", &[(0x7010, &[0; 12]), (0x7068, &METER_7068)], &[]);
            meter.values = Values::from([
                ((0x7010, 11), vec![0x80]), // FNC_USSM = 1: the menu gate HDS lists the resets under
            ]);
            meter.routine = Some(Routine { ids: &METER_USM_IDS, option: None, did: 0x7068, offset: 10, mask: 0, value: 0, wait_s: 0.0, status: Some(0), needs_extended: false });
            scripted(vec![pcm, meter])
        }
        // The EPS alone with its stored-only code (the default profile's EPS DTC): the health aggregate must call
        // this car's faults "no" (nothing confirmed or pending) and its history "yes", and still exit 1 — a stored
        // code means the ECU was faulted since the last clear, and the docs count stored in the exit code.
        "accord2015-stored" => {
            let e = SimEcu::new(0x30, "CANEPS", &[(0x4800, &[1])], &[[0x02, 0x03, 0x00, 0x20]]);
            scripted(vec![e])
        }
        // two systems behind one address: engine and EV power supply
        "evps" => {
            let mut e = SimEcu::new(0x10, "CANFI", &[(0x2600, &[1]), (0x3000, &[1]), (0xF190, b"1HGCR2F3XFA000000")], &[[0x04, 0x20, 0x00, 0x2F]]);
            e.also = vec!["CANEVPS".into()];
            scripted(vec![e, SimEcu::new(0x28, "CANABS", &[(0x4000, &[1])], &[])])
        }
        // The real car: the 2015 Accord's PCM as recorded on the GD101 drive of 2026-10-11 (simcapture.rs).
        "accord2015-gd101" => simcapture::load("accord2015_gd101", &[(0x0E, &["CANFI", "CANCVT"])], false)?.into_iter().map(SimNode::Captured).collect(),
        // The same car on the GD101's J2534 mode, a 48 minute drive (2026-10-12): physical addressing reaches the PCM (PGM-FI and
        // CVT), the SRS and the meter. The ABS and EPS never answered from the DLC, so they are absent.
        "accord2015-gd101-j2534" => simcapture::load("accord2015_gd101_j2534", J2534_CAR, true)?.into_iter().map(SimNode::Captured).collect(),
        // The same car and adapter on the drive home of 2026-10-15 (session 113, 1,972 s, 30.3 km): the same three ECUs and the same
        // requests answered or refused, with that drive's values (profiles/accord2015_gd101_j2534_drive_home.json.gz).
        "accord2015-gd101-j2534-drive-home" => simcapture::load("accord2015_gd101_j2534_drive_home", J2534_CAR, true)?.into_iter().map(SimNode::Captured).collect(),
        "empty" => vec![],
        _ => return Err(format!("unknown sim profile {name:?} ({})", PROFILE_NAMES.join(", "))),
    })
}

/// With a database, every DID the system serves is answered with zero-filled data of HDS's packet size (recorded ECUs answer
/// only what they did).
pub fn fill_from_db(nodes: &mut [SimNode], db: &Database) {
    for n in nodes.iter_mut() {
        let SimNode::Scripted(e) = n else { continue };
        let systems: Vec<&str> = std::iter::once(e.system.as_str()).chain(e.also.iter().map(String::as_str)).collect();
        let q = format!("SELECT did, MAX(size) FROM can_packets WHERE system IN ({}) GROUP BY did", vec!["?"; systems.len()].join(","));
        for (did, size) in db.query(&q, rusqlite::params_from_iter(systems.iter()), |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))) {
            let mut data = e.dids.get(&(did as u16)).cloned().unwrap_or_default();
            data.resize(size as usize, 0);
            e.dids.insert(did as u16, data);
        }
    }
}

/// A raw CAN adapter whose bus has the simulated ECUs on it.
pub struct SimulatedVehicle {
    raw: RawCanAdapter,
    vbus: VirtualBus,
    pub ecus: Vec<Arc<Mutex<SimNode>>>,
    stop: Arc<AtomicBool>,
    threads: Mutex<Vec<JoinHandle<()>>>,
}

impl SimulatedVehicle {
    /// `db`: with a database, every DID the system serves is answered with zero-filled data of HDS's packet size (recorded
    /// ECUs answer only what they did). None: the default database if there is one.
    pub fn new(profile_name: &str, log: Option<Arc<CaptureLog>>, db: Option<&Database>) -> Result<SimulatedVehicle, String> {
        Self::from_nodes(profile_name, profile(profile_name)?, log, db)
    }

    /// A vehicle with exactly these ECUs. `profile_name` only matters for "drive", which gives the ECUs the scripted drive.
    pub fn from_nodes(profile_name: &str, mut nodes: Vec<SimNode>, log: Option<Arc<CaptureLog>>, db: Option<&Database>) -> Result<SimulatedVehicle, String> {
        let own = if db.is_none() { Database::open(None).ok() } else { None };
        if let Some(db) = db.or(own.as_ref()) {
            fill_from_db(&mut nodes, db);
        }
        if profile_name == "drive" {
            let drive = Arc::new(Mutex::new(Drive::default()));
            let t0 = Instant::now();
            for n in nodes.iter_mut() {
                if let SimNode::Scripted(e) = n {
                    let (d, s) = (drive.clone(), e.system.clone());
                    e.overlay = Some(Arc::new(move |did| {
                        let all = d.lock().unwrap().values(&s, t0.elapsed().as_secs_f64());
                        all.into_iter().filter(|((vd, _), _)| *vd == did).collect()
                    }));
                }
            }
        }
        let vbus = VirtualBus::new();
        let raw = RawCanAdapter::new(Arc::new(vbus.endpoint(log)), "VirtualEndpoint");
        let car = SimulatedVehicle { raw, vbus, ecus: nodes.into_iter().map(|n| Arc::new(Mutex::new(n))).collect(), stop: Arc::new(AtomicBool::new(false)), threads: Mutex::new(vec![]) };
        car.start();
        Ok(car)
    }

    fn start(&self) {
        let mut ready = vec![];
        for e in &self.ecus {
            let (addr, listens) = {
                let g = e.lock().unwrap();
                (g.addr(), g.listens())
            };
            for request_id in listens {
                let (tx, rx) = std::sync::mpsc::channel();
                ready.push(rx);
                let ecu = e.clone();
                let stop = self.stop.clone();
                let endpoint = Arc::new(self.vbus.endpoint(None));
                let pad = self.raw.pad;
                let h = std::thread::spawn(move || serve(ecu, addr, request_id, endpoint, pad, stop, tx));
                self.threads.lock().unwrap().push(h);
            }
        }
        for r in ready {
            // a frame sent before a thread has joined the bus would never reach that ECU
            let _ = r.recv_timeout(Duration::from_secs(5));
        }
    }

    pub fn ecu(&self, addr: u32) -> Arc<Mutex<SimNode>> {
        self.ecus.iter().find(|e| e.lock().unwrap().addr() == addr).expect("no such simulated ECU").clone()
    }

    /// A new endpoint on the vehicle's bus (a device model in a test attaches here).
    pub fn endpoint(&self) -> crate::transport::can::VirtualEndpoint {
        self.vbus.endpoint(None)
    }

    pub fn adapter(&self) -> &RawCanAdapter {
        &self.raw
    }
}

fn serve(ecu: Arc<Mutex<SimNode>>, addr: u32, request_id: u32, endpoint: Arc<crate::transport::can::VirtualEndpoint>, pad: Option<u8>, stop: Arc<AtomicBool>, ready: std::sync::mpsc::Sender<()>) {
    let mut link = crate::transport::isotp::IsoTpLink::with(endpoint, rx_id(addr), request_id, Some(tx_id(addr)));
    link.pad = pad;
    let _ = ready.send(());
    let functional = request_id != tx_id(addr);
    while !stop.load(Ordering::Relaxed) {
        let Ok(Some(req)) = link.recv(0.05) else { continue };
        if req.is_empty() {
            continue;
        }
        let reply = {
            let mut g = ecu.lock().unwrap();
            if functional && !g.functional() {
                continue;
            }
            g.handle(&req)
        };
        let replies = match reply {
            Reply::None => vec![],
            Reply::One(r) => vec![r],
            Reply::Many(v) => v,
        };
        for r in replies {
            if r.is_empty() || functional && r[0] == 0x7F && r.len() > 2 && SILENT_ON_FUNCTIONAL.contains(&r[2]) {
                continue;
            }
            let _ = link.send(&r);
        }
    }
}

impl Adapter for SimulatedVehicle {
    fn bus(&self) -> Option<Arc<dyn crate::transport::can::CanBus>> {
        self.raw.bus()
    }

    fn sim(&self) -> Option<&SimulatedVehicle> {
        Some(self)
    }

    fn link(&self, tx_id: u32, rx_id: u32) -> Box<dyn DiagLink> {
        self.raw.link(tx_id, rx_id)
    }

    fn describe(&self) -> String {
        let a: Vec<String> = self.ecus.iter().map(|e| format!("0x{:02X}", e.lock().unwrap().addr())).collect();
        format!("simulated vehicle, ECUs at {}", if a.is_empty() { "none".to_string() } else { a.join(", ") })
    }

    fn close(&self) {
        self.stop.store(true, Ordering::Relaxed);
        for t in self.threads.lock().unwrap().drain(..) {
            let _ = t.join();
        }
    }

    fn has_probe(&self) -> bool {
        true
    }

    fn probe(&self, pairs: &[(u32, u32)], payload: &[u8], timeout: f64) -> Result<std::collections::BTreeSet<u32>, crate::transport::isotp::LinkError> {
        self.raw.probe(pairs, payload, timeout)
    }

    fn has_broadcast(&self) -> bool {
        true
    }

    fn broadcast(&self, tx_id: u32, rx_ids: &[u32], payload: &[u8], timeout: f64) -> Result<BTreeMap<u32, Option<f64>>, crate::transport::isotp::LinkError> {
        self.raw.broadcast(tx_id, rx_ids, payload, timeout)
    }
}

pub const SIM_PAD: u8 = HDS_PAD;

impl Drop for SimulatedVehicle {
    /// The serving threads end with the vehicle, as Python's daemon threads end with the process.
    fn drop(&mut self) {
        Adapter::close(self);
    }
}
