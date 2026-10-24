//! One ECU behind an ELM327-compatible adapter: send a single-frame request, reassemble the printed frames.
//! Port of hondascan/transport/elm_link.py.
use super::can::CanFrame;
use super::elm327::{Elm327, Inner};
use super::isotp::{DiagLink, IsoTpError, LinkError};
use std::fmt;
use std::sync::Arc;

/// Reply lines that are errors, compared without spaces.
pub const ERRORS: [&str; 12] = [
    "CANERROR", "BUFFERFULL", "BUSERROR", "BUSBUSY", "DATAERROR", "RXERROR", "STOPPED", "UNABLETOCONNECT", "FBERROR", "LVRESET", "ACTALERT", "ERR",
];

/// Python's ElmError / ElmBusError / ElmReset (all IsoTpError subclasses).
#[derive(Debug, Clone, PartialEq)]
pub enum ElmErr {
    Error(String),
    /// The adapter reported a bus error for this request (not an answer, not an adapter fault).
    Bus(String),
    /// The adapter rebooted (settings lost), vanished or hung; `lost`: reopen the port before setting it up again.
    Reset { msg: String, lost: bool },
}

impl ElmErr {
    pub fn error(m: impl Into<String>) -> ElmErr {
        ElmErr::Error(m.into())
    }

    pub fn message(&self) -> &str {
        match self {
            ElmErr::Error(m) | ElmErr::Bus(m) => m,
            ElmErr::Reset { msg, .. } => msg,
        }
    }
}

impl fmt::Display for ElmErr {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.message())
    }
}

impl std::error::Error for ElmErr {}

impl From<ElmErr> for LinkError {
    fn from(e: ElmErr) -> Self {
        LinkError::IsoTp(IsoTpError(e.message().to_string()))
    }
}

impl From<std::io::Error> for ElmErr {
    fn from(e: std::io::Error) -> Self {
        ElmErr::Error(e.to_string())
    }
}

pub struct Elm327Link {
    pub elm: Arc<Elm327>,
    pub tx_id: u32,
    pub rx_id: Option<u32>,
    pending: Vec<Vec<u8>>,
}

impl Elm327Link {
    pub fn new(elm: Arc<Elm327>, tx_id: u32, rx_id: Option<u32>) -> Elm327Link {
        Elm327Link { elm, tx_id, rx_id, pending: vec![] }
    }
}

impl DiagLink for Elm327Link {
    fn send(&mut self, payload: &[u8]) -> Result<(), LinkError> {
        let (tx, rx) = (self.tx_id, self.rx_id.unwrap_or(0));
        let r = self.elm.with(|inn| {
            inn.retrying(&mut |i| {
                i.select(tx, rx, true)?;
                let t = i.timeout * 8.0 + 2.0;
                exchange(i, tx, Some(rx), payload, t)
            })
        })?;
        self.pending = r;
        Ok(())
    }

    fn recv(&mut self, _timeout: f64) -> Result<Option<Vec<u8>>, LinkError> {
        Ok(if self.pending.is_empty() { None } else { Some(self.pending.remove(0)) })
    }
}

/// Send one single-frame request (addressing already selected); return the complete answers.
pub fn exchange(inn: &mut Inner, tx_id: u32, rx_id: Option<u32>, payload: &[u8], timeout: f64) -> Result<Vec<Vec<u8>>, ElmErr> {
    let text = request(inn, tx_id, payload, timeout)?;
    reassemble(inn, rx_id, &text)
}

/// Send one single-frame request; return the adapter's printed reply.
pub fn request(inn: &mut Inner, tx_id: u32, payload: &[u8], timeout: f64) -> Result<String, ElmErr> {
    if payload.len() > 7 {
        return Err(ElmErr::error("ELM327 can only send single-frame requests"));
    }
    if let Some(log) = &inn.log {
        let mut d = vec![payload.len() as u8];
        d.extend(payload);
        log.write(&CanFrame::new(tx_id, d), "TX");
    }
    let hex: String = payload.iter().map(|b| format!("{b:02X}")).collect();
    inn.cmd(&hex, timeout)
}

/// (id, data) of every frame the adapter printed, each logged; an adapter error returns ElmBusError.
pub fn frames(inn: &Inner, text: &str) -> Result<Vec<(u32, Vec<u8>)>, ElmErr> {
    let mut out = vec![];
    for ln in text.lines() {
        let ln: String = ln.trim().replace(' ', "");
        if ln.is_empty() || ln.starts_with("SEARCHING") || ln.starts_with("BUSINIT") || ln.starts_with("NODATA") || ln == "OK" {
            continue;
        }
        if ERRORS.iter().any(|e| ln.starts_with(e)) || ln == "?" {
            return Err(ElmErr::Bus(ln));
        }
        let head: String = ln.chars().take(8).collect();
        let tail: String = ln.chars().skip(8).collect();
        let (Ok(cid), Some(data)) = (u32::from_str_radix(&head, 16), unhex(&tail)) else { continue };
        if let Some(log) = &inn.log {
            log.write(&CanFrame::new(cid, data.clone()), "RX");
        }
        out.push((cid, data));
    }
    Ok(out)
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) || !s.is_ascii() {
        return None;
    }
    (0..s.len() / 2).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).ok()).collect()
}

pub fn reassemble(inn: &Inner, rx_id: Option<u32>, text: &str) -> Result<Vec<Vec<u8>>, ElmErr> {
    let (mut msgs, mut cur, mut need): (Vec<Vec<u8>>, Option<Vec<u8>>, usize) = (vec![], None, 0);
    let mut seq = 1u8; // the consecutive-frame count, restarted by every first frame
    for (cid, data) in frames(inn, text)? {
        if Some(cid) != rx_id || data.is_empty() {
            continue;
        }
        match data[0] >> 4 {
            0 => msgs.push(data[1..data.len().min(1 + (data[0] & 0xF) as usize)].to_vec()),
            1 if data.len() >= 2 => {
                need = (((data[0] & 0xF) as usize) << 8) | data[1] as usize;
                cur = Some(data[2..].to_vec());
                seq = 1;
            }
            2 => {
                if let Some(c) = cur.as_mut() {
                    // a lost, duplicated or reordered CF would append the wrong bytes: the count must match
                    if data[0] & 0xF != seq {
                        return Err(ElmErr::error(format!("consecutive frame out of sequence (got {}, want {})", data[0] & 0xF, seq)));
                    }
                    c.extend(&data[1..]);
                    if c.len() >= need {
                        msgs.push(c[..need].to_vec());
                        cur = None;
                    }
                    seq = (seq + 1) & 0xF;
                }
            }
            _ => {}
        }
    }
    Ok(msgs)
}
