//! Generic OBD-II (SAE J1979) over the ISO 9141-2 K-line, read only: the way a car from before about 2008 (a 2003 CR-V, for one)
//! answers every scan tool, whatever its maker. This is not Honda's DIAG-H (diagh.rs): ABS, SRS and the other chassis modules are
//! not reachable here, only the engine/transmission ECUs that are OBD-II compliant.
//!
//! STATUS: written from SAE J1979 / ISO 9141-2 and the ELM327 datasheet, tested only against tests/common/kline_emulator.rs; never
//! run on a car. Unconfirmed until it is: that the 2003 CR-V's PCM is on ISO 9141-2 (pin 7) and not on ISO 14230-4, and that the
//! GD101's ELM mode has a K-line at all (README, "GD101").
//!
//! The adapter (transport::elm_kline, `-a elm-obd9141:PORT`) adds the `68 6A F1` header and the checksum, runs the 5-baud
//! initialisation at address 0x33 on the first request, and prints each answering ECU's frame `48 6B <ecu> data checksum`.
//! Only services 01, 02, 03, 07 and 09 can be built here: no clear (04), no test control (08), no write.
use crate::transport::elm_kline::ElmKLine;
use crate::uds::sae_code;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

#[derive(Debug)]
pub enum ObdError {
    /// the adapter or the bus failed (text from the adapter layer)
    Link(String),
    /// the request is outside the read-only set
    Refused(String),
}

impl fmt::Display for ObdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ObdError::Link(m) | ObdError::Refused(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for ObdError {}

pub const LIVE_DATA: u8 = 0x01;
pub const FREEZE_FRAME: u8 = 0x02;
pub const STORED_DTC: u8 = 0x03;
pub const PENDING_DTC: u8 = 0x07;
pub const VEHICLE_INFO: u8 = 0x09;

/// Shortest request per service the client may send; an id that is not here is refused before anything is sent.
fn min_length(service: u8) -> Option<usize> {
    match service {
        LIVE_DATA | VEHICLE_INFO => Some(2),
        FREEZE_FRAME => Some(3),
        STORED_DTC | PENDING_DTC => Some(1),
        _ => None,
    }
}

pub struct Obd<'a> {
    link: &'a ElmKLine,
}

/// One decoded mode 01 value.
#[derive(Debug, Clone, PartialEq)]
pub struct Reading {
    pub pid: u8,
    pub name: &'static str,
    pub value: f64,
    pub unit: &'static str,
}

impl<'a> Obd<'a> {
    pub fn new(link: &'a ElmKLine) -> Obd<'a> {
        Obd { link }
    }

    /// Send `request`; return each ECU's reply data that is the positive response to it (service | 0x40, then the request's
    /// parameter byte for the services that echo one).
    fn ask(&self, request: &[u8]) -> Result<Vec<(u8, Vec<u8>)>, ObdError> {
        let service = request[0];
        if min_length(service).is_none_or(|n| request.len() < n) {
            return Err(ObdError::Refused(format!("request {:02X?} is outside the read-only OBD-II set (01, 02, 03, 07, 09)", request)));
        }
        let echo = if matches!(service, LIVE_DATA | FREEZE_FRAME | VEHICLE_INFO) { 2 } else { 1 };
        let replies = self.link.obd_request(request).map_err(|e| ObdError::Link(e.to_string()))?;
        Ok(replies.into_iter().filter(|(_, d)| d.len() >= echo && d[0] == service | 0x40 && (echo == 1 || d[1] == request[1])).collect())
    }

    /// Mode 01 PIDs each ECU says it supports (PID 00, 20, 40 ... bitmaps, followed while the last bit says there is a next one).
    pub fn supported_pids(&self) -> Result<BTreeMap<u8, BTreeSet<u8>>, ObdError> {
        let mut out: BTreeMap<u8, BTreeSet<u8>> = BTreeMap::new();
        let mut more: BTreeSet<u8> = BTreeSet::new();
        for base in (0x00..=0xC0u8).step_by(0x20) {
            if base != 0 && more.is_empty() {
                break;
            }
            let mut next = BTreeSet::new();
            for (ecu, d) in self.ask(&[LIVE_DATA, base])? {
                if base != 0 && !more.contains(&ecu) || d.len() < 6 {
                    continue;
                }
                let mask = u32::from_be_bytes([d[2], d[3], d[4], d[5]]);
                let set = out.entry(ecu).or_default();
                for bit in 0..32u32 {
                    if mask >> (31 - bit) & 1 == 1 {
                        set.insert(base + bit as u8 + 1);
                    }
                }
                if mask & 1 == 1 {
                    next.insert(ecu);
                }
            }
            more = next;
        }
        Ok(out)
    }

    /// Raw data bytes of mode 01 `pid` from each ECU that answers.
    pub fn read_pid(&self, pid: u8) -> Result<Vec<(u8, Vec<u8>)>, ObdError> {
        Ok(self.ask(&[LIVE_DATA, pid])?.into_iter().map(|(e, d)| (e, d[2..].to_vec())).collect())
    }

    /// Mode 03 (stored) or 07 (pending) codes per ECU. A frame carries three code slots of two bytes; 0000 is an empty slot. An ECU
    /// that answers with no codes gives an empty list; one that does not answer is absent.
    pub fn dtcs(&self, service: u8) -> Result<BTreeMap<u8, Vec<String>>, ObdError> {
        if service != STORED_DTC && service != PENDING_DTC {
            return Err(ObdError::Refused(format!("service {service:02X} is not a DTC read")));
        }
        let mut out: BTreeMap<u8, Vec<String>> = BTreeMap::new();
        for (ecu, d) in self.ask(&[service])? {
            let list = out.entry(ecu).or_default();
            for c in d[1..].as_chunks::<2>().0.iter().filter(|c| c[0] != 0 || c[1] != 0) {
                list.push(sae_code(c[0], c[1]));
            }
        }
        Ok(out)
    }

    /// The VIN (mode 09, PID 02): message numbers 1..5 carry 4 bytes each, the first one's three leading bytes are padding (00).
    pub fn vin(&self) -> Result<Option<String>, ObdError> {
        let mut parts: BTreeMap<u8, Vec<u8>> = BTreeMap::new();
        for (_, d) in self.ask(&[VEHICLE_INFO, 0x02])? {
            if d.len() >= 3 {
                parts.entry(d[2]).or_insert_with(|| d[3..].to_vec());
            }
        }
        let bytes: Vec<u8> = parts.into_values().flatten().filter(|b| *b != 0).collect();
        Ok((bytes.len() == 17 && bytes.iter().all(|b| b.is_ascii_alphanumeric())).then(|| bytes.iter().map(|b| *b as char).collect()))
    }

    /// Mode 01 PID 01: (check engine light on, number of confirmed codes) per ECU.
    pub fn status(&self) -> Result<BTreeMap<u8, (bool, u8)>, ObdError> {
        Ok(self.read_pid(0x01)?.into_iter().filter(|(_, d)| !d.is_empty()).map(|(e, d)| (e, (d[0] & 0x80 != 0, d[0] & 0x7F))).collect())
    }
}

/// SAE J1979 scaling of the common mode 01 PIDs; None for a PID this table does not know or data of the wrong length.
pub fn decode(pid: u8, d: &[u8]) -> Option<Reading> {
    let a = *d.first()? as f64;
    let ab = || d.get(1).map(|b| a * 256.0 + *b as f64);
    let (name, value, unit) = match pid {
        0x04 => ("Engine load", a * 100.0 / 255.0, "%"),
        0x05 => ("Coolant temperature", a - 40.0, "degC"),
        0x06 => ("Short-term fuel trim, bank 1", (a - 128.0) * 100.0 / 128.0, "%"),
        0x07 => ("Long-term fuel trim, bank 1", (a - 128.0) * 100.0 / 128.0, "%"),
        0x08 => ("Short-term fuel trim, bank 2", (a - 128.0) * 100.0 / 128.0, "%"),
        0x09 => ("Long-term fuel trim, bank 2", (a - 128.0) * 100.0 / 128.0, "%"),
        0x0A => ("Fuel pressure", a * 3.0, "kPa"),
        0x0B => ("Intake manifold pressure", a, "kPa"),
        0x0C => ("Engine speed", ab()? / 4.0, "rpm"),
        0x0D => ("Vehicle speed", a, "km/h"),
        0x0E => ("Ignition timing advance", a / 2.0 - 64.0, "deg"),
        0x0F => ("Intake air temperature", a - 40.0, "degC"),
        0x10 => ("Mass air flow", ab()? / 100.0, "g/s"),
        0x11 => ("Throttle position", a * 100.0 / 255.0, "%"),
        0x14..=0x1B => ("Oxygen sensor voltage", a / 200.0, "V"),
        0x1F => ("Run time since start", ab()?, "s"),
        0x21 => ("Distance with check engine light on", ab()?, "km"),
        0x2F => ("Fuel level", a * 100.0 / 255.0, "%"),
        0x33 => ("Barometric pressure", a, "kPa"),
        0x42 => ("Control module voltage", ab()? / 1000.0, "V"),
        0x46 => ("Ambient air temperature", a - 40.0, "degC"),
        0x5C => ("Engine oil temperature", a - 40.0, "degC"),
        _ => return None,
    };
    Some(Reading { pid, name, value, unit })
}
