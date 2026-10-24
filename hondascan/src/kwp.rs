//! ISO 14230 (KWP2000) framing and a read-only service client, for the K-line systems HDS calls VKWP (58 of its 182 systems).
//! Port of hondascan/kwp.py, whose docstring has the status (never run on a car) and the sources.
//!
//! Frame: Fmt [Tgt Src] [Len] data... Checksum. Fmt bits 7-6 select the addressing (00 none, 10 physical, 11 functional), bits
//! 5-0 the data length (0: a length byte follows). The checksum is the sum of all earlier bytes modulo 256.
use crate::transport::isotp::LinkError;
use crate::uds::{sae_code, DtcRecord};
use std::fmt;
use std::time::{Duration, Instant};

pub const START_COMMUNICATION: u8 = 0x81;
pub const STOP_COMMUNICATION: u8 = 0x82;
pub const START_SESSION: u8 = 0x10;
pub const READ_FREEZE_FRAME: u8 = 0x12;
pub const READ_DTC: u8 = 0x13;
pub const CLEAR_DTC: u8 = 0x14;
pub const READ_DTC_BY_STATUS: u8 = 0x18;
pub const READ_ECU_ID: u8 = 0x1A;
pub const READ_BY_LID: u8 = 0x21;
pub const TESTER_PRESENT: u8 = 0x3E;
/// ISO 14230-3 groupOfDTC: 0x0000 powertrain, 0x4000 chassis, 0x8000 body, 0xC000 network, 0xFF00 all
pub const ALL_GROUPS: u16 = 0xFF00;
pub const CLEAR_ALL: [u8; 3] = [CLEAR_DTC, (ALL_GROUPS >> 8) as u8, (ALL_GROUPS & 0xFF) as u8];
/// the tester's source address on the K-line (HDS/OBD convention; the 0xF1 of the CAN ids is the same)
pub const TESTER: u8 = 0xF1;

/// Request ids the client may send; the value is the set of allowed second bytes (None: any). 0x30 (IO control), 0x27 (security),
/// 0x2E/0x3B (writes), 0x31/0x32 (routines), 0x34-0x37 (transfer) and 0x11 (reset) are not here.
fn allowed(sid: u8) -> Option<Option<&'static [u8]>> {
    Some(match sid {
        START_COMMUNICATION | STOP_COMMUNICATION | READ_FREEZE_FRAME | READ_DTC | CLEAR_DTC | READ_DTC_BY_STATUS | READ_ECU_ID | READ_BY_LID => None,
        START_SESSION => Some(&[0x81]),
        TESTER_PRESENT => Some(&[0x01, 0x02]),
        _ => return Option::None,
    })
}

/// Shortest well-formed request per service (the id plus its parameters); anything shorter is refused before it is sent.
fn min_length(sid: u8) -> usize {
    match sid {
        START_SESSION | READ_FREEZE_FRAME | READ_BY_LID | READ_ECU_ID | TESTER_PRESENT => 2,
        READ_DTC_BY_STATUS => 4,
        CLEAR_DTC => 3,
        _ => 1,
    }
}

/// Request bytes after the id that the positive response repeats; a reply that does not repeat them answers an earlier request.
fn echo_len(sid: u8) -> usize {
    if sid == READ_BY_LID || sid == READ_ECU_ID { 1 } else { 0 }
}

pub fn nrc_name(nrc: u8) -> &'static str {
    match nrc {
        0x10 => "generalReject",
        0x11 => "serviceNotSupported",
        0x12 => "subFunctionNotSupported-invalidFormat",
        0x21 => "busyRepeatRequest",
        0x22 => "conditionsNotCorrectOrRequestSequenceError",
        0x23 => "routineNotComplete",
        0x31 => "requestOutOfRange",
        0x33 => "securityAccessDenied",
        0x35 => "invalidKey",
        0x36 => "exceedNumberOfAttempts",
        0x37 => "requiredTimeDelayNotExpired",
        0x78 => "requestCorrectlyReceived-ResponsePending",
        _ => "",
    }
}

/// Python's KwpError family (all UdsErrors, so code written for the UDS client handles K-line faults the same way).
#[derive(Debug, Clone, PartialEq)]
pub enum KwpError {
    Error(String),
    NoResponse(String),
    Negative { sid: u8, nrc: u8 },
    NotAllowed(String),
    /// The adapter or the bus failed (bad init, bus error, lost adapter); the ECU may be fine.
    Transport(String),
}

impl fmt::Display for KwpError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            KwpError::Error(s) | KwpError::NoResponse(s) | KwpError::NotAllowed(s) | KwpError::Transport(s) => write!(f, "{s}"),
            KwpError::Negative { sid, nrc } => write!(f, "service 0x{sid:02X}: NRC 0x{nrc:02X} {}", nrc_name(*nrc)),
        }
    }
}

impl std::error::Error for KwpError {}

impl KwpError {
    /// The Python class name.
    pub fn kind(&self) -> &'static str {
        match self {
            KwpError::Error(_) => "KwpError",
            KwpError::NoResponse(_) => "KwpNoResponse",
            KwpError::Negative { .. } => "KwpNegative",
            KwpError::NotAllowed(_) => "KwpNotAllowed",
            KwpError::Transport(_) => "KwpTransportError",
        }
    }
}

fn err<T>(s: impl Into<String>) -> Result<T, KwpError> {
    Err(KwpError::Error(s.into()))
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn checksum(data: &[u8]) -> u8 {
    data.iter().fold(0u32, |a, b| a + *b as u32) as u8
}

/// The frame for `data` (service id + parameters). Without `target`: no address bytes (mode 00).
pub fn build(data: &[u8], target: Option<u8>, source: u8, functional: bool, length_byte: bool) -> Result<Vec<u8>, KwpError> {
    let n = data.len();
    if n == 0 || n > 255 {
        return err("a frame carries 1-255 data bytes");
    }
    let mode = match target {
        None => 0,
        Some(_) if functional => 0xC0,
        Some(_) => 0x80,
    };
    let use_len_byte = length_byte || n > 63;
    let mut frame = vec![mode | if use_len_byte { 0 } else { n as u8 }];
    if let Some(t) = target {
        frame.extend([t, source]);
    }
    if use_len_byte {
        frame.push(n as u8);
    }
    frame.extend(data);
    frame.push(checksum(&frame));
    Ok(frame)
}

/// (data, target, source) of a complete frame; target and source are None in address mode 00.
pub type ParsedFrame = (Vec<u8>, Option<u8>, Option<u8>);

/// Errors if the length or the checksum is wrong.
pub fn parse(frame: &[u8]) -> Result<ParsedFrame, KwpError> {
    if frame.len() < 3 {
        return err(format!("frame too short: {}", hex(frame)));
    }
    let fmt = frame[0];
    let addressed = fmt >> 6 != 0;
    let mut i = 1 + if addressed { 2 } else { 0 };
    let (target, source) = if addressed { (Some(frame[1]), Some(frame[2])) } else { (None, None) };
    let mut n = (fmt & 0x3F) as usize;
    if n == 0 {
        if frame.len() <= i {
            return err(format!("frame too short: {}", hex(frame)));
        }
        n = frame[i] as usize;
        i += 1;
    }
    if frame.len() != i + n + 1 {
        return err(format!("length {n} does not match frame {}", hex(frame)));
    }
    let want = checksum(&frame[..frame.len() - 1]);
    if want != frame[frame.len() - 1] {
        return err(format!("bad checksum in {} (expected {:02X})", hex(frame), want));
    }
    Ok((frame[i..i + n].to_vec(), target, source))
}

/// HDS's ECU ID for a K-line body ECU (BE_UTILS.CreateRealEcuId): the 6 bytes of LID 0x14 (or 0x40) followed by one destination
/// byte, `destination - 48` (HDS's GetDestinationId returns 49..53, so 1..5), as 14 uppercase hex digits.
pub fn real_ecu_id(raw: &[u8], destination: i64) -> Result<String, KwpError> {
    if raw.len() != 6 {
        return err(format!("the ECU ID is made from 6 bytes, not {}", raw.len()));
    }
    let mut v = raw.to_vec();
    v.push(((destination - 48) & 0xFF) as u8);
    Ok(v.iter().map(|b| format!("{b:02X}")).collect())
}

/// The DTCs of a ReadDTCByStatus reply: 58 <count>, then 3 bytes per DTC (DTC high, DTC low, status), as uds.DtcRecord. The
/// code is read the way HUTILS.GetDtcDispData reads DTC_METHOD 2: there is no failure-type byte, so failure_type is -1, the
/// value HUTILS writes into the method-2 key. Strict: the count must match.
pub fn dtc_records(rsp: &[u8]) -> Result<Vec<DtcRecord>, KwpError> {
    if rsp.len() < 2 || rsp[0] != READ_DTC_BY_STATUS + 0x40 {
        return err(format!("not a ReadDTCByStatus reply: {}", hex(rsp)));
    }
    let (n, body) = (rsp[1] as usize, &rsp[2..]);
    if body.len() != 3 * n {
        return err(format!("{n} DTC(s) announced, {} record bytes in {}", body.len(), hex(rsp)));
    }
    Ok(body.as_chunks::<3>().0.iter().map(|c| DtcRecord { code: sae_code(c[0], c[1]), failure_type: -1, status: c[2], raw: c.to_vec() }).collect())
}

/// A link with `send(data)` and `recv(timeout)`, both on service payloads (the link adds or strips the header and the checksum).
pub trait KwpLink {
    fn send(&mut self, data: &[u8]) -> Result<(), LinkError>;
    /// bytes, or None on timeout
    fn recv(&mut self, timeout: f64) -> Option<Vec<u8>>;
}

impl<L: KwpLink> crate::uds::DidReader for KwpClient<L> {
    /// LiveData.read's name for it: the local identifier is the K-line's data identifier.
    fn read_did(&mut self, lid: u16) -> Result<Vec<u8>, crate::uds::UdsError> {
        let lid = u8::try_from(lid).map_err(|_| crate::uds::UdsError::Kwp(KwpError::Error(format!("local identifier {lid} does not fit a byte"))))?;
        self.read_by_lid(lid).map_err(crate::uds::UdsError::Kwp)
    }
}

pub struct KwpClient<L: KwpLink> {
    pub link: L,
    pub p2: f64,
    pub p2_star: f64,
    pub allow_clear: bool,
    pub retries: u32,
}

impl<L: KwpLink> KwpClient<L> {
    pub fn new(link: L) -> KwpClient<L> {
        KwpClient { link, p2: 0.2, p2_star: 5.0, allow_clear: false, retries: 1 }
    }

    fn check(&self, req: &[u8]) -> Result<(), KwpError> {
        let not = |s: String| Err(KwpError::NotAllowed(s));
        let sid = req[0];
        if crate::READ_ONLY_RELEASE && sid == CLEAR_DTC {
            return not(format!("KWP service 0x{sid:02X}: {}", crate::NOT_IN_RELEASE));
        }
        let Some(subs) = allowed(sid) else { return not(format!("KWP service 0x{sid:02X} is not in hondascan's allowlist")) };
        if req.len() < min_length(sid) {
            return not(format!("KWP service 0x{sid:02X}: request {} is too short", hex(req)));
        }
        if let Some(subs) = subs {
            if req.len() < 2 || !subs.contains(&req[1]) {
                return not(format!("KWP service 0x{sid:02X} parameter {} not allowed", hex(&req[1..req.len().min(2)])));
            }
        }
        if sid == CLEAR_DTC && !self.allow_clear {
            return not("clearing DTCs needs allow_clear=True".into());
        }
        if sid == CLEAR_DTC && req != CLEAR_ALL {
            let sp = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<Vec<_>>().join(" ");
            return not(format!("only {} (all groups of one ECU) is allowed, not {}", sp(&CLEAR_ALL), sp(req)));
        }
        Ok(())
    }

    pub fn request(&mut self, req: &[u8], timeout: Option<f64>) -> Result<Vec<u8>, KwpError> {
        self.check(req)?;
        let sid = req[0];
        let echo = &req[1..(1 + echo_len(sid)).min(req.len())];
        for attempt in 0..=self.retries {
            if let Err(e) = self.link.send(req) {
                if attempt == self.retries {
                    return Err(KwpError::Transport(format!("{}: {e}", hex(req))));
                }
                continue;
            }
            let mut end = Instant::now() + Duration::from_secs_f64(timeout.filter(|t| *t != 0.0).unwrap_or(self.p2));
            while let Some(rsp) = self.link.recv(end.saturating_duration_since(Instant::now()).as_secs_f64()) {
                if rsp.first() == Some(&0x7F) && rsp.len() >= 3 && rsp[1] == sid {
                    if rsp[2] == 0x78 {
                        end = Instant::now() + Duration::from_secs_f64(self.p2_star);
                        continue;
                    }
                    if rsp[2] == 0x21 && attempt < self.retries {
                        std::thread::sleep(Duration::from_millis(200));
                        break;
                    }
                    return Err(KwpError::Negative { sid, nrc: rsp[2] });
                }
                if rsp.first() == Some(&(sid + 0x40)) && rsp[1..].get(..echo.len()) == Some(echo) {
                    return Ok(rsp);
                }
                // anything else answers an earlier request: skip it
            }
        }
        Err(KwpError::NoResponse(format!("no response to {}", hex(req))))
    }

    // -- services -----------------------------------------------------------
    pub fn tester_present(&mut self) -> Result<Vec<u8>, KwpError> {
        self.request(&[TESTER_PRESENT, 0x01], None)
    }

    /// ReadECUIdentification: the identification record (bytes after the echoed option).
    pub fn read_ecu_id(&mut self, option: u8) -> Result<Vec<u8>, KwpError> {
        Ok(self.request(&[READ_ECU_ID, option], None)?.get(2..).unwrap_or(&[]).to_vec())
    }

    /// ReadDataByLocalIdentifier: the data record for local identifier `lid`.
    pub fn read_by_lid(&mut self, lid: u8) -> Result<Vec<u8>, KwpError> {
        Ok(self.request(&[READ_BY_LID, lid], None)?.get(2..).unwrap_or(&[]).to_vec())
    }

    /// ReadDTCByStatus (0x18), every group by default: the records as uds::DtcRecord (see dtc_records).
    pub fn read_dtcs(&mut self, group: u16) -> Result<Vec<DtcRecord>, KwpError> {
        let rsp = self.request(&[READ_DTC_BY_STATUS, 0x02, (group >> 8) as u8, (group & 0xFF) as u8], Some(self.p2.max(1.0)))?;
        dtc_records(&rsp)
    }

    pub fn clear_dtcs(&mut self) -> Result<Vec<u8>, KwpError> {
        self.request(&CLEAR_ALL, Some(self.p2.max(2.0)))
    }
}
