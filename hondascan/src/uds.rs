//! ISO 14229 (UDS) client restricted to the services hondascan uses. Port of hondascan/uds.py.
//!
//! Only the services in `allowed` can be sent. ClearDiagnosticInformation (0x14) additionally needs `allow_clear`,
//! ReadMemoryByAddress (0x23) a MemoryPolicy in `memory`, IOControlByIdentifier (0x2F) an IoPolicy in `io`, and
//! RoutineControl (0x31) a WritePolicy in `routine`. There is deliberately no write-data, reset or download/upload
//! support: hondascan reads, clears codes, and runs HDS's own actuator tests and the whitelisted HDS routines (write.rs) when asked.
//!
//! SecurityAccess (0x27) is read-only in the same sense: its only purpose here is unlocking a memory read a
//! MemoryPolicy already allowed but the ECU refused with NRC 0x33, so it is gated on `memory` being `Some`
//! exactly like 0x23 itself, not a separate always-open service. `security_access()` runs the exchange HDS's own
//! `CM.SecurityAccess`/`PGM_FI_UTILS.CalcTypeIIIKey` do (`hondascan/src/seedkey.rs`, decompiled from GRADE
//! bytecode and verified against the vendor interpreter by execution): request a seed (`27 01`), compute the key
//! from the reply, send it back (`27 02 KeyH KeyL`). Wire sub-functions and the 16-bit key layout confirmed
//! offline 2026-10-19 from `test.dll` + A550 ROM; measured on the car 2026-10-20: the PCM's seed reply carries
//! two seed bytes and no type byte, and the key it wants is the ECU's own ROM routine (`seedkey::calc_key_pcm`).
use crate::actuator::{self, IoPolicy, IO_CONTROL};
use crate::memory::{self, MemoryPolicy, READ_MEMORY};
use crate::reflash::{self, FlashTarget, ERASE_BEGIN, ERASE_END, ERASE_ROUTINE, FINISH_ROUTINE, REQUEST_DOWNLOAD, REQUEST_TRANSFER_EXIT, TRANSFER_DATA, WRITE_DID};
use crate::seedkey;
use crate::write::{self, WritePolicy, ROUTINE_CONTROL};
use crate::transport::isotp::{DiagLink, LinkError};
use std::fmt;
use std::time::{Duration, Instant};

pub const DIAG_SESSION: u8 = 0x10;
pub const CLEAR_DTC: u8 = 0x14;
pub const READ_DTC_INFO: u8 = 0x19;
pub const READ_DID: u8 = 0x22;
pub const SECURITY_ACCESS: u8 = 0x27;
pub const TESTER_PRESENT: u8 = 0x3E;
/// `31 01 FF 00` / `31 01 FF 01`, reflash's own fixed RoutineControl requests (reflash.rs), outside write.rs's
/// whitelist of named HDS menu procedures.
const REFLASH_ROUTINES: [[u8; 4]; 2] = [[ROUTINE_CONTROL, 0x01, (ERASE_ROUTINE >> 8) as u8, ERASE_ROUTINE as u8], [ROUTINE_CONTROL, 0x01, (FINISH_ROUTINE >> 8) as u8, FINISH_ROUTINE as u8]];
pub const P2: f64 = 0.15; // seconds an ECU gets to answer (ISO 14229 P2; the real value is a live-test item)
pub const CLEAR_ALL: [u8; 4] = [CLEAR_DTC, 0xFF, 0xFF, 0xFF];
const REQUEST_SEED: u8 = 0x01;
const SEND_KEY: u8 = 0x02;
const REQUEST_SEED_2: u8 = 0x03;
const SEND_KEY_2: u8 = 0x04;

/// Sub-functions a service may carry (None: any), or None for a service that is not allowed at all.
fn allowed(sid: u8) -> Option<Option<&'static [u8]>> {
    Some(match sid {
        // Dead for DIAG_SESSION: `check` returns before this lookup for that sid (session 2/programming is gated
        // there, on `reflash_target`, not here). Left at the plain rule anyway so this table is never read as
        // claiming 0x02 is unconditionally valid.
        DIAG_SESSION => Some(&[0x01, 0x03]),
        CLEAR_DTC | READ_DID | READ_MEMORY | IO_CONTROL | ROUTINE_CONTROL | WRITE_DID | REQUEST_DOWNLOAD | TRANSFER_DATA | REQUEST_TRANSFER_EXIT => None,
        READ_DTC_INFO => Some(&[0x01, 0x02, 0x04, 0x06, 0x0A]),
        SECURITY_ACCESS => Some(&[REQUEST_SEED, SEND_KEY, REQUEST_SEED_2, SEND_KEY_2]),
        TESTER_PRESENT => Some(&[0x00, 0x80]),
        _ => return Option::None,
    })
}

/// Request bytes after the SID that a positive response repeats (sub-function, or the DID). A reply that does not repeat
/// them belongs to an earlier request (e.g. a late answer to an attempt that was retried).
fn echo_len(sid: u8) -> usize {
    match sid {
        DIAG_SESSION | READ_DTC_INFO | SECURITY_ACCESS | TESTER_PRESENT => 1,
        READ_DID | WRITE_DID => 2,
        IO_CONTROL => 3,       // 2F: the DID and the option
        ROUTINE_CONTROL => 3,  // 31: the sub-function and the routine id
        TRANSFER_DATA => 1,    // 36: the block sequence counter
        _ => 0,
    }
}

pub fn nrc_name(nrc: u8) -> &'static str {
    match nrc {
        0x10 => "generalReject",
        0x11 => "serviceNotSupported",
        0x12 => "subFunctionNotSupported",
        0x13 => "incorrectMessageLengthOrInvalidFormat",
        0x14 => "responseTooLong",
        0x21 => "busyRepeatRequest",
        0x22 => "conditionsNotCorrect",
        0x24 => "requestSequenceError",
        0x31 => "requestOutOfRange",
        0x33 => "securityAccessDenied",
        0x35 => "invalidKey",
        0x36 => "exceedNumberOfAttempts",
        0x37 => "requiredTimeDelayNotExpired",
        0x72 => "generalProgrammingFailure",
        0x78 => "responsePending",
        0x7E => "subFunctionNotSupportedInActiveSession",
        0x7F => "serviceNotSupportedInActiveSession",
        _ => "",
    }
}

/// Python's UdsError hierarchy as one enum; `Io` is an OSError from the bus (the adapter went away), which Python lets
/// through unconverted.
#[derive(Debug)]
pub enum UdsError {
    /// No answer in time.
    NoResponse(String),
    /// The adapter or ISO-TP layer failed (lost frame, bus error, adapter error); the ECU may be fine.
    Transport(String),
    Negative { sid: u8, nrc: u8 },
    NotAllowed(String),
    /// Any other UdsError (a malformed reply).
    Other(String),
    /// A K-line (KWP2000) fault: Python's KwpError is a UdsError too.
    Kwp(crate::kwp::KwpError),
    Io(std::io::Error),
}

impl fmt::Display for UdsError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            UdsError::NoResponse(s) | UdsError::Transport(s) | UdsError::NotAllowed(s) | UdsError::Other(s) => write!(f, "{s}"),
            UdsError::Negative { sid, nrc } => write!(f, "service 0x{sid:02X}: NRC 0x{nrc:02X} {}", nrc_name(*nrc)),
            UdsError::Kwp(e) => write!(f, "{e}"),
            UdsError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for UdsError {}

impl From<String> for UdsError {
    fn from(s: String) -> Self {
        UdsError::Other(s)
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn hex_spaced(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect::<Vec<_>>().join(" ")
}

const STATUS_BITS: [&str; 8] = [
    "testFailed", "testFailedThisOperationCycle", "pendingDTC", "confirmedDTC",
    "testNotCompletedSinceLastClear", "testFailedSinceLastClear", "testNotCompletedThisOperationCycle", "warningIndicatorRequested",
];

#[derive(Debug, Clone, PartialEq)]
pub struct DtcRecord {
    pub code: String, // SAE form, e.g. 'P0420', 'B0001'
    pub failure_type: i32, // third DTC byte (Honda: subcode / failure type); -1 on the K-line, which has none
    pub status: u8,       // ISO 14229 status bits
    pub raw: Vec<u8>,
}

impl DtcRecord {
    pub fn flags(&self) -> Vec<&'static str> {
        STATUS_BITS.iter().enumerate().filter(|(i, _)| self.status >> i & 1 == 1).map(|(_, n)| *n).collect()
    }

    /// HDS's two DTC lists: 'confirmed' (status bit 3, also bit 7, the MIL request) and 'pending' (bit 2, also bits 0-1,
    /// failing now). 'stored': failed since the last clear (bit 5), passing now. 'incomplete': only the test-not-completed
    /// bits (4, 6), i.e. a self-test that has not run yet, not a fault.
    pub fn kind(&self) -> &'static str {
        if self.status & 0x88 != 0 {
            "confirmed"
        } else if self.status & 0x07 != 0 {
            "pending"
        } else if self.status & 0x20 != 0 {
            "stored"
        } else {
            "incomplete"
        }
    }
}

pub fn sae_code(hi: u8, mid: u8) -> String {
    format!("{}{:04X}", ['P', 'C', 'B', 'U'][(hi >> 6) as usize], ((hi & 0x3F) as u16) << 8 | mid as u16)
}

/// `data` is the DTCAndStatusRecord list (4 bytes each) that follows 59 02 <availability mask>.
pub fn parse_dtc_records(data: &[u8]) -> Vec<DtcRecord> {
    data.as_chunks::<4>().0.iter().map(|c| DtcRecord { code: sae_code(c[0], c[1]), failure_type: c[2] as i32, status: c[3], raw: c.to_vec() }).collect()
}

/// Anything that reads a data identifier: the CAN client, or the K-line client (its local identifier).
pub trait DidReader {
    fn read_did(&mut self, did: u16) -> Result<Vec<u8>, UdsError>;
}

impl DidReader for UdsClient {
    fn read_did(&mut self, did: u16) -> Result<Vec<u8>, UdsError> {
        UdsClient::read_did(self, did)
    }
}

pub struct UdsClient {
    pub link: Box<dyn DiagLink>,
    pub p2: f64,
    pub p2_star: f64,
    pub allow_clear: bool,
    pub retries: u32,
    pub memory: Option<MemoryPolicy>, // a MemoryPolicy opens service 0x23; None keeps it closed
    pub io: Option<IoPolicy>,         // an IoPolicy opens service 0x2F; None keeps it closed
    pub routine: Option<WritePolicy>, // a WritePolicy opens 0x31; None keeps it closed
    /// A reflash target opens session `10 02`, `27` without a MemoryPolicy, `2E F1 00/01 01/00`, reflash's own
    /// `31 01 FF 00`/`FF 01`, and `34`/`36`/`37` (reflash.rs); None keeps all of them closed exactly like the other
    /// policies above.
    pub reflash_target: Option<FlashTarget>,
    pub functional: bool,             // requests reach every ECU: a clear would clear them all
    pub late: bool,                   // a request whose answer repeats nothing (0x14, 0x23) went unanswered in time
    /// Why the last `security_access` failed after reaching the ECU. While set, `security_access` refuses to send
    /// anything: a rejected key is not retried (ECUs count invalid keys and lock out with NRC 0x36/0x37).
    security_failed: Option<String>,
}

impl UdsClient {
    pub fn new(link: Box<dyn DiagLink>) -> UdsClient {
        UdsClient { link, p2: P2, p2_star: 5.0, allow_clear: false, retries: 1, memory: None, io: None, routine: None, reflash_target: None, functional: false, late: false, security_failed: None }
    }

    fn check(&self, req: &[u8]) -> Result<(), UdsError> {
        let not = |s: String| Err(UdsError::NotAllowed(s));
        let Some(&sid) = req.first() else { return not("an empty request".into()) };
        if sid == DIAG_SESSION {
            // Session 2 (programming) is only ever sent for a reflash (README.md "The `10 02`/`34` question,
            // answered the other way round": this PCM gates 10 02 on a prior successful 27 02, which the fake
            // enforces; the client only asks for the byte-shape here).
            let sub = req.get(1).copied().unwrap_or(0) & 0x7F;
            if req.len() != 2 || !(if self.reflash_target.is_some() { [0x01, 0x02, 0x03].contains(&sub) } else { [0x01, 0x03].contains(&sub) }) {
                return not(format!("service 0x{sid:02X} sub-function {} not allowed", hex(&req[1..req.len().min(2)])));
            }
            return Ok(());
        }
        let Some(subs) = allowed(sid) else { return not(format!("service 0x{sid:02X} is not in hondascan's allowlist")) };
        if let Some(subs) = subs {
            if req.len() < 2 || !subs.contains(&(req[1] & 0x7F)) {
                return not(format!("service 0x{sid:02X} sub-function {} not allowed", hex(&req[1..req.len().min(2)])));
            }
        }
        if sid == CLEAR_DTC && !self.allow_clear {
            return not("clearing DTCs needs allow_clear=True".into());
        }
        if sid == CLEAR_DTC && req != CLEAR_ALL {
            return not(format!("only {} (all groups of one system, as HDS clears) is allowed, not {}", hex_spaced(&CLEAR_ALL), hex_spaced(req)));
        }
        if sid == READ_MEMORY {
            let Some(policy) = &self.memory else { return not("memory reads need a MemoryPolicy on the client".into()) };
            if self.functional {
                return not("not reading memory on a functional id: every ECU would answer".into());
            }
            policy.check(req).map_err(|e| UdsError::NotAllowed(e.0))?;
        }
        if sid == SECURITY_ACCESS {
            if self.memory.is_none() && self.reflash_target.is_none() {
                return not("SecurityAccess needs a MemoryPolicy or a reflash target on the client: its only uses here are unlocking a memory read and a reflash's own unlock".into());
            }
            if self.functional {
                return not("not sending SecurityAccess on a functional id: every ECU would answer".into());
            }
        }
        if sid == IO_CONTROL {
            let Some(policy) = &self.io else { return not("IO control needs an IoPolicy on the client".into()) };
            if self.functional {
                return not("not sending IO control on a functional id: every ECU would act on it".into());
            }
            policy.check(req).map_err(|e| UdsError::NotAllowed(e.0))?;
        }
        if sid == ROUTINE_CONTROL {
            if self.functional {
                return not("not sending a routine on a functional id: every ECU would run it".into());
            }
            // reflash's own fixed erase/finish routines (reflash.rs), outside write.rs's named-procedure whitelist.
            let is_reflash_routine = self.reflash_target.is_some() && req.len() == 4 && REFLASH_ROUTINES.contains(&[req[0], req[1], req[2], req[3]]);
            if !is_reflash_routine {
                let Some(policy) = &self.routine else { return not("routine control needs a WritePolicy on the client".into()) };
                policy.check(req).map_err(|e| UdsError::NotAllowed(e.0))?;
            }
        }
        if sid == CLEAR_DTC && self.functional {
            return not("not clearing: requests go to every ECU on this car (functional id), so a clear would clear every system".into());
        }
        if matches!(sid, WRITE_DID | REQUEST_DOWNLOAD | TRANSFER_DATA | REQUEST_TRANSFER_EXIT) {
            let Some(target) = &self.reflash_target else {
                return not(format!("service 0x{sid:02X} needs a reflash target on the client"));
            };
            if self.functional {
                return not(format!("not sending service 0x{sid:02X} on a functional id: every ECU would act on it"));
            }
            // The shape/range checks themselves live in reflash.rs, the same split as memory.rs/actuator.rs for
            // the other policies above.
            reflash::check(sid, req, target).map_err(|e| UdsError::NotAllowed(e.0))?;
        }
        Ok(())
    }

    /// Send `req`, return the positive response. Timeouts, busyRepeatRequest and transport errors are retried `retries`
    /// times (default: the client's); responsePending extends the wait to P2*.
    pub fn request(&mut self, req: &[u8], timeout: Option<f64>, retries: Option<u32>) -> Result<Vec<u8>, UdsError> {
        self.check(req)?;
        let sid = req[0];
        let echo = &req[1..(1 + echo_len(sid)).min(req.len())];
        let suppress = (sid == DIAG_SESSION || sid == TESTER_PRESENT) && req.len() > 1 && req[1] & 0x80 != 0;
        let retries = retries.unwrap_or(self.retries);
        if self.late {
            // drop what is still coming for the unanswered request
            while self.link.recv(self.p2).map_err(link_io)?.is_some() {}
            self.late = false;
        }
        for attempt in 0..=retries {
            let last = attempt == retries;
            let wait = timeout.filter(|t| *t != 0.0).unwrap_or(self.p2);
            let sent: Result<Option<Vec<u8>>, AwaitError> = (|| {
                self.link.send(req)?;
                if suppress {
                    return Ok(Some(vec![]));
                }
                self.await_response(sid, echo, wait, !last)
            })();
            match sent {
                Ok(Some(rsp)) => return Ok(rsp),
                Ok(None) => {}
                Err(AwaitError::Link(LinkError::IsoTp(e))) => {
                    if last {
                        return Err(UdsError::Transport(format!("{}: {e}", hex(req))));
                    }
                    continue;
                }
                Err(AwaitError::Link(LinkError::Io(e))) => return Err(UdsError::Io(e)),
                Err(AwaitError::Uds(e)) => return Err(e),
            }
            self.late = self.late || echo.is_empty(); // a late answer to this attempt would pass for the next request's
        }
        Err(UdsError::NoResponse(format!("no response to {}", hex(req))))
    }

    /// The response to the request just sent, or None to retry (timeout, or busy when `retry_busy`).
    fn await_response(&mut self, sid: u8, echo: &[u8], wait: f64, retry_busy: bool) -> Result<Option<Vec<u8>>, AwaitError> {
        let mut end = Instant::now() + crate::transport::can::secs(wait);
        loop {
            let left = end.saturating_duration_since(Instant::now()).as_secs_f64();
            let Some(rsp) = self.link.recv(left)? else { return Ok(None) };
            if rsp.first() == Some(&0x7F) && rsp.len() >= 3 && rsp[1] == sid {
                if rsp[2] == 0x78 {
                    end = Instant::now() + crate::transport::can::secs(self.p2_star);
                    continue;
                }
                if rsp[2] == 0x21 && retry_busy {
                    std::thread::sleep(Duration::from_millis(200));
                    return Ok(None);
                }
                return Err(AwaitError::Uds(UdsError::Negative { sid, nrc: rsp[2] }));
            }
            if rsp.first() == Some(&(sid + 0x40)) && rsp[1..].get(..echo.len()) == Some(echo) {
                return Ok(Some(rsp));
            }
            // anything else answers an earlier request: skip it
        }
    }

    // -- services -----------------------------------------------------------
    pub fn tester_present(&mut self, retries: Option<u32>) -> Result<Vec<u8>, UdsError> {
        self.request(&[TESTER_PRESENT, 0x00], None, retries)
    }

    pub fn session(&mut self, kind: u8) -> Result<Vec<u8>, UdsError> {
        self.request(&[DIAG_SESSION, kind], None, None)
    }

    pub fn read_did(&mut self, did: u16) -> Result<Vec<u8>, UdsError> {
        let r = self.request(&[READ_DID, (did >> 8) as u8, (did & 0xFF) as u8], None, None)?;
        Ok(r.get(3..).unwrap_or(&[]).to_vec())
    }

    /// IOControlByIdentifier; the reply repeats the identifier and the option (HDS checks the option byte).
    pub fn io_control(&mut self, did: u16, option: u8, value: Option<u32>) -> Result<Vec<u8>, UdsError> {
        let req = actuator::encode(did, option, value).map_err(|e| UdsError::NotAllowed(e.0))?;
        self.request(&req, None, None)
    }

    /// RoutineControl with the exact request a WritePolicy whitelists. No retries: a routine start must not be sent
    /// twice because one reply was lost. HDS's `V.DiagnosticXfer` allows 200 ms for it (CM.pseudo.txt 0x2551).
    pub fn routine_control(&mut self, req: &[u8]) -> Result<Vec<u8>, UdsError> {
        write::decode(req).map_err(|e| UdsError::NotAllowed(e.0))?;
        self.request(req, Some(self.p2.max(0.2)), Some(0))
    }

    /// `length` bytes at `address`. The reply must carry exactly that many; anything else is an error.
    pub fn read_memory(&mut self, address: u64, length: u64, fmt: u8) -> Result<Vec<u8>, UdsError> {
        let req = memory::encode(address, length, fmt).map_err(|e| UdsError::NotAllowed(e.0))?;
        let data = self.request(&req, None, None)?[1..].to_vec();
        if data.len() as u64 != length {
            return Err(UdsError::Other(format!("asked for {length} bytes at 0x{address:X}, got {}", data.len())));
        }
        Ok(data)
    }

    /// SecurityAccess, HDS's own exchange (`seedkey.rs`): request a seed (`27 01`), compute the key, send
    /// it (`27 02 KeyH KeyL`). Measured on the car 2026-10-20: the PCM's seed reply is two seed bytes and
    /// no type byte, and the key it wants is its own ROM's (`seedkey::calc_key_pcm`); a reply with a type
    /// byte is the layout CM.SecurityAccess parses, key = `CalcTypeIIIKey`'s for it (sub-functions 01/02
    /// and the 16-bit key layout settled offline 2026-10-19 from `test.dll` + A550 ROM).
    /// `UdsError::Other` for a seed reply that is neither 2 nor 3 bytes or a type
    /// with no defined key (32 of 121 types, seedkey.rs); any other error (NRC, transport, timeout) from either
    /// leg propagates as-is.
    ///
    /// One failure ends it for this client: the error is remembered and every later call returns it without
    /// touching the bus. The algorithm is unconfirmed on a real car, and an ECU that rejects the key counts the
    /// attempts and locks out (NRC 0x36 exceededNumberOfAttempts, 0x37 requiredTimeDelayNotExpired), so a loop
    /// of memory reads must not turn into a loop of wrong keys. A success may be repeated (a session change
    /// relocks the ECU).
    pub fn security_access(&mut self) -> Result<(), UdsError> {
        self.security_access_levels(REQUEST_SEED, SEND_KEY)
    }

    /// The second level pair, `27 03`/`27 04`: the 2015 Accord's PCM accepted the level 01/02 key
    /// (seedkey::calc_key_pcm, measured 2026-10-20) yet kept ReadMemoryByAddress locked (NRC 0x33 on
    /// the retried read), so its memory gate is guarded by this pair. Its seed length was not
    /// recorded (the level-1 reply has no type byte; CM.SecurityAccess parses a 3-byte layout with one, and the A550
    /// ROM's type-7 block at 0x3F15E would be the matching key material), so the client handles both lengths.
    pub fn security_access_2(&mut self) -> Result<(), UdsError> {
        self.security_access_levels(REQUEST_SEED_2, SEND_KEY_2)
    }

    fn security_access_levels(&mut self, request: u8, send: u8) -> Result<(), UdsError> {
        if let Some(why) = &self.security_failed {
            return Err(UdsError::Other(format!("SecurityAccess not retried after an earlier failure: {why}")));
        }
        let r = self.security_exchange(request, send);
        if let Err(e) = &r {
            if !matches!(e, UdsError::NotAllowed(_)) {
                self.security_failed = Some(e.to_string());
            }
        }
        r
    }

    fn security_exchange(&mut self, request: u8, send: u8) -> Result<(), UdsError> {
        let seed_rsp = self.request(&[SECURITY_ACCESS, request], None, None)?;
        let seed_bytes = seed_rsp.get(2..).unwrap_or(&[]);
        // The reply's length picks the routine. The 2015 Accord's PCM answers `67 01 SeedH SeedL` — two
        // seed bytes, no type byte (measured 2026-10-20, extended session, `~/hondascan-live.log`) — and
        // expects the key its own ROM computes (`seedkey::calc_key_pcm`). A reply that also carries a type
        // byte is the layout CM.SecurityAccess parses 3 bytes from: the key is CalcTypeIIIKey's for it.
        let key = match seed_bytes {
            [hi, lo] => {
                let seed = u16::from_be_bytes([*hi, *lo]);
                seedkey::calc_key_pcm(seed)
            }
            [hi, lo, t] => {
                let seed = u16::from_be_bytes([*hi, *lo]);
                seedkey::calc_key(seed, *t).ok_or_else(|| {
                    UdsError::Other(format!("SecurityAccess type 0x{t:02X} has no defined key (seedkey.rs)"))
                })?
            }
            other => {
                return Err(UdsError::Other(format!("SecurityAccess seed reply was {} bytes, expected 2 or 3", other.len())))
            }
        };
        let [key_hi, key_lo] = key.to_be_bytes();
        self.request(&[SECURITY_ACCESS, send, key_hi, key_lo], None, None)?;
        Ok(())
    }

    /// ReadDTCInformation / reportDTCByStatusMask. Returns (availability_mask, [DtcRecord]).
    pub fn read_dtcs(&mut self, mask: u8) -> Result<(u8, Vec<DtcRecord>), UdsError> {
        let rsp = self.request(&[READ_DTC_INFO, 0x02, mask], Some(self.p2.max(1.0)), None)?;
        if rsp.len() < 3 {
            return Err(UdsError::Other(format!("unexpected DTC reply {}", hex(&rsp))));
        }
        Ok((rsp[2], parse_dtc_records(&rsp[3..])))
    }

    pub fn dtc_count(&mut self, mask: u8) -> Result<Option<u16>, UdsError> {
        let rsp = self.request(&[READ_DTC_INFO, 0x01, mask], None, None)?;
        Ok((rsp.len() >= 6).then(|| (rsp[4] as u16) << 8 | rsp[5] as u16))
    }

    pub fn clear_dtcs(&mut self) -> Result<Vec<u8>, UdsError> {
        self.request(&CLEAR_ALL, Some(self.p2.max(2.0)), None)
    }

    // -- reflash (reflash.rs) ------------------------------------------------
    /// `2E F1 00 01` (before the erase routine) or `2E F1 01 00` (after it) -- the exe's own erase-mode markers.
    pub fn reflash_erase_marker(&mut self, before: bool) -> Result<(), UdsError> {
        self.request(if before { &ERASE_BEGIN } else { &ERASE_END }, None, None)?;
        Ok(())
    }

    /// `31 01 FF 00`: reflash's own erase routine, not an HDS write function (write.rs's whitelist).
    pub fn reflash_erase(&mut self) -> Result<(), UdsError> {
        self.request(&REFLASH_ROUTINES[0], Some(self.p2.max(0.2)), Some(0))?;
        Ok(())
    }

    /// `31 01 FF 01`: sent once, after every block has been transferred.
    pub fn reflash_finish(&mut self) -> Result<(), UdsError> {
        self.request(&REFLASH_ROUTINES[1], Some(self.p2.max(0.2)), Some(0))?;
        Ok(())
    }

    /// RequestDownload for one block: returns the ECU's maxNumberOfBlockLength (capped at the target's own bound).
    pub fn request_download(&mut self, address: u32, size: u32) -> Result<u16, UdsError> {
        let target = self.reflash_target.ok_or_else(|| UdsError::Other("RequestDownload needs a reflash target on the client".into()))?;
        let req = reflash::encode_request_download(&target, address, size).map_err(|e| UdsError::NotAllowed(e.0))?;
        let rsp = self.request(&req, None, None)?;
        reflash::decode_download_reply(&target, &rsp).map_err(|e| UdsError::Other(e.0))
    }

    /// TransferData for one chunk (at most the length `request_download` returned).
    pub fn transfer_data(&mut self, counter: u8, data: &[u8]) -> Result<(), UdsError> {
        let mut req = Vec::with_capacity(2 + data.len());
        req.push(TRANSFER_DATA);
        req.push(counter);
        req.extend_from_slice(data);
        self.request(&req, None, None)?;
        Ok(())
    }

    /// RequestTransferExit, sent once a block's last TransferData is acknowledged.
    pub fn request_transfer_exit(&mut self) -> Result<(), UdsError> {
        self.request(&[REQUEST_TRANSFER_EXIT], None, None)?;
        Ok(())
    }
}

enum AwaitError {
    Link(LinkError),
    Uds(UdsError),
}

impl From<LinkError> for AwaitError {
    fn from(e: LinkError) -> Self {
        AwaitError::Link(e)
    }
}

fn link_io(e: LinkError) -> UdsError {
    match e {
        LinkError::Io(e) => UdsError::Io(e),
        LinkError::IsoTp(e) => UdsError::Transport(e.0),
    }
}
