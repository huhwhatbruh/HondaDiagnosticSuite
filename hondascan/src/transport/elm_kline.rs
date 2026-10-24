//! ELM327-compatible adapters on the K-line: ISO 14230-4 (KWP2000), the protocol of HDS's VKWP systems (kwp.rs). Port of
//! hondascan/transport/elm_kline.py.
//!
//! STATUS: written from the ELM327 datasheet's command set (ATSP4/ATSP5, ATSH with 3 header bytes, ATIIA, ATH1, ATAT1) and tested
//! only against an emulator written from the same reading; never run on a real adapter. Open points that only a real adapter
//! settles: whether the GD101's ELM mode has a K-line at all, the exact text it prints for a failed bus initialisation, and whether
//! it prints the checksum byte of a reply (parsing accepts both). Honda's DIAG-H systems are not reachable this way.
//!
//! The adapter initialises the bus itself on the first request (fast init with ATSP5, or 5-baud with ATSP4 and the target as ATIIA
//! address) and keeps the line alive; we address with ATSH `<fmt> <target> <tester>` and send the service payload as hex. With
//! headers on, each printed line is one reply frame (header, data, checksum).
use super::can::CaptureLog;
use super::elm327::{Elm327, Inner, KProto};
use super::elm_link::{ElmErr, ERRORS};
use super::isotp::{DiagLink, LinkError};
use super::Adapter;
use crate::kwp::{checksum, parse, KwpLink, TESTER};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// the first request does the bus initialisation (a 5-baud init alone is about 2 s)
pub const INIT_TIMEOUT: f64 = 12.0;
/// format byte base; the adapter puts the data length in its low bits
pub const PHYSICAL: u8 = 0x80;
pub const FUNCTIONAL: u8 = 0xC0;

/// (data, source) of a printed reply frame. Strict first; an adapter that does not print the checksum byte is accepted too.
pub fn payload_of(raw: &[u8]) -> Result<(Vec<u8>, Option<u8>), crate::kwp::KwpError> {
    match parse(raw) {
        Ok((d, _, s)) => Ok((d, s)),
        Err(_) => {
            let mut v = raw.to_vec();
            v.push(checksum(raw));
            parse(&v).map(|(d, _, s)| (d, s))
        }
    }
}

pub struct ElmKLine {
    elm: Arc<Elm327>,
}

fn not_can() -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidInput, "elm-kline is a K-line (ISO 14230) adapter: use `hondascan kwp`, or elm:/gd101 for CAN")
}

impl ElmKLine {
    pub fn open(port: &str, baud: Option<u32>, log: Option<Arc<CaptureLog>>, proto: KProto) -> Result<ElmKLine, ElmErr> {
        Ok(ElmKLine { elm: Elm327::open_mode(port, baud, 400, log, Some(proto))? })
    }

    pub fn proto(&self) -> KProto {
        self.elm.with(|i| i.kline.as_ref().map_or(KProto::Fast, |k| k.proto))
    }

    pub fn ident(&self) -> String {
        self.elm.with(|i| i.ident.clone())
    }

    pub fn kwp_link(&self, target: u8, tester: u8) -> KLineLink {
        KLineLink { elm: self.elm.clone(), target, tester, pending: vec![] }
    }
}

impl Inner {
    /// Address the next K-line request (ATSH <fmt><target><tester>, plus ATIIA for a 5-baud init); a new target needs its own
    /// initialisation.
    pub fn kline_address(&mut self, target: u8, tester: u8, functional: bool) -> Result<(), ElmErr> {
        let key = (target as u32, tester as u32, functional);
        let (current, proto) = self.kline.as_ref().map(|k| (k.addressed, k.proto)).unwrap_or((None, KProto::Fast));
        if current != Some(key) {
            let base = if functional { FUNCTIONAL } else { PHYSICAL };
            self.must_cmd(&format!("ATSH{base:02X}{target:02X}{tester:02X}"))?;
            if proto != KProto::Fast {
                self.must_cmd(&format!("ATIIA{target:02X}"))?;
            }
            if let Some(k) = self.kline.as_mut() {
                (k.addressed, k.started) = (Some(key), false); // a new target needs its own initialisation
            }
        }
        Ok(())
    }
}

impl Adapter for ElmKLine {
    fn as_kline(&self) -> Option<&ElmKLine> {
        Some(self)
    }

    /// Not a CAN adapter: Elm327's CAN addressing (ATSH from a 29-bit id, ATCRA) has no meaning on the K-line. Python raises
    /// ValueError from `link()`; here the link refuses at its first use (the same text, an I/O error kind).
    fn link(&self, _tx_id: u32, _rx_id: u32) -> Box<dyn DiagLink> {
        Box::new(RefusingLink)
    }

    fn has_probe(&self) -> bool {
        true
    }

    fn probe(&self, _pairs: &[(u32, u32)], _payload: &[u8], _timeout: f64) -> Result<BTreeSet<u32>, LinkError> {
        Err(LinkError::Io(not_can()))
    }

    fn has_broadcast(&self) -> bool {
        true
    }

    fn broadcast(&self, _tx_id: u32, _rx_ids: &[u32], _payload: &[u8], _timeout: f64) -> Result<BTreeMap<u32, Option<f64>>, LinkError> {
        Err(LinkError::Io(not_can()))
    }

    fn describe(&self) -> String {
        let (path, baud, ident, proto) = self.elm.with(|i| (i.path.clone(), i.baud, i.ident.clone(), i.kline.as_ref().map_or(KProto::Fast, |k| k.proto)));
        let v = self.elm.voltage();
        let what = match proto {
            KProto::Fast => "ISO 14230-4 fast init",
            KProto::Slow => "ISO 14230-4 5-baud init",
            KProto::Iso9141 => "ISO 9141-2 (generic OBD-II)",
        };
        format!("{path} @ {baud} baud: {ident}; {what}; battery {} V", v.map_or("None".to_string(), |v| crate::journal::py(&serde_json::json!(v))))
    }

    fn close(&self) {
        self.elm.with(|i| {
            let _ = i.cmd("ATPC", 1.0); // protocol close
            i.close_port();
        });
    }

    fn path(&self) -> Option<String> {
        Some(self.elm.with(|i| i.path.clone()))
    }
}

struct RefusingLink;

impl DiagLink for RefusingLink {
    fn send(&mut self, _payload: &[u8]) -> Result<(), LinkError> {
        Err(LinkError::Io(not_can()))
    }

    fn recv(&mut self, _timeout: f64) -> Result<Option<Vec<u8>>, LinkError> {
        Err(LinkError::Io(not_can()))
    }
}

/// One ECU on the K-line, as kwp::KwpClient wants it: send(data) and recv(timeout) on service payloads.
pub struct KLineLink {
    elm: Arc<Elm327>,
    target: u8,
    tester: u8,
    pending: Vec<Vec<u8>>,
}

impl KLineLink {
    pub fn new(adapter: &ElmKLine, target: u8) -> KLineLink {
        adapter.kwp_link(target, TESTER)
    }

    fn replies(&self, text: &str) -> Result<Vec<Vec<u8>>, ElmErr> {
        let target = self.target;
        Ok(frames(&self.elm, text, &|raw| payload_of(raw).ok().filter(|(_, s)| s.is_none_or(|s| s == target)).map(|(d, _)| (0, d)))?.into_iter().map(|(_, d)| d).collect())
    }
}

/// The reply frames in the adapter's printed `text`: each hex line goes through `parse` (frame bytes -> (source, data), None for a
/// line that is not a valid frame: the client sees the timeout and asks again); a bus error line ends the bus initialisation so the
/// next request starts it again.
/// (source address, data) of one reply frame
type Frame = (u8, Vec<u8>);

fn frames(elm: &Elm327, text: &str, parse: &dyn Fn(&[u8]) -> Option<Frame>) -> Result<Vec<Frame>, ElmErr> {
    let mut out = vec![];
    let started_off = |inn: &mut Inner| {
        if let Some(k) = inn.kline.as_mut() {
            k.started = false;
        }
    };
    for ln in text.lines() {
        let ln: String = ln.trim().replace(' ', "").to_uppercase();
        if ln.is_empty() || ln == "OK" || ln.starts_with("SEARCHING") || ln.starts_with("NODATA") {
            continue;
        }
        if ln.starts_with("BUSINIT") {
            if ln.contains("OK") && !ln.contains("ERROR") {
                continue;
            }
            elm.with(started_off);
            return Err(ElmErr::Bus(ln));
        }
        if ERRORS.iter().any(|e| ln.starts_with(e)) || ln == "?" {
            elm.with(started_off);
            return Err(ElmErr::Bus(ln));
        }
        if let Some(f) = unhex(&ln).and_then(|raw| parse(&raw)) {
            out.push(f);
        }
    }
    Ok(out)
}

/// (source, data) of a generic OBD-II reply frame: ISO 9141-2 `48 6B <ecu> data... checksum`, or an ISO 14230-4 frame (`payload_of`);
/// a missing checksum byte is accepted in both.
fn obd_payload(raw: &[u8]) -> Option<Frame> {
    if raw.len() >= 5 && raw[0] == 0x48 && raw[1] == 0x6B {
        let n = raw.len() - 1;
        let body = if checksum(&raw[..n]) == raw[n] { &raw[3..n] } else { &raw[3..] };
        return Some((raw[2], body.to_vec()));
    }
    let (data, source) = payload_of(raw).ok()?;
    Some((source?, data))
}

impl ElmKLine {
    /// Open `port` for generic OBD-II on whichever K-line protocol the car answers to `01 00`: ISO 9141-2, then ISO 14230-4 with
    /// 5-baud init, then with fast init. The first that gets a reply stays open; if none does, the error says what each one printed.
    pub fn open_obd_auto(port: &str, baud: Option<u32>, log: Option<Arc<CaptureLog>>) -> Result<ElmKLine, ElmErr> {
        let mut tried = vec![];
        for (name, proto) in [("ISO 9141-2", KProto::Iso9141), ("ISO 14230-4 5-baud init", KProto::Slow), ("ISO 14230-4 fast init", KProto::Fast)] {
            let link = ElmKLine::open(port, baud, log.clone(), proto)?;
            match link.obd_request(&[0x01, 0x00]) {
                Ok(r) if !r.is_empty() => return Ok(link),
                Ok(_) => tried.push(format!("{name}: no answer")),
                Err(e) => tried.push(format!("{name}: {e}")),
            }
            Adapter::close(&link);
        }
        Err(ElmErr::error(format!("no K-line protocol got an answer from the car ({}); is the key ON and the adapter on the OBD port?", tried.join("; "))))
    }

    /// One generic OBD-II request on an ISO 9141-2 adapter (`elm-obd9141`): the adapter adds the 68 6A F1 header and the checksum, and
    /// the first request starts the 5-baud initialisation at address 33. Returns each answering ECU's (address, data), which may be
    /// several; an empty list is "no data".
    pub fn obd_request(&self, data: &[u8]) -> Result<Vec<Frame>, ElmErr> {
        let hex: String = data.iter().map(|b| format!("{b:02X}")).collect();
        let text = self.elm.with(|inn| {
            inn.retrying(&mut |i| {
                let started = i.kline.as_ref().is_some_and(|k| k.started);
                let wait = if started { i.timeout * 8.0 + 2.0 } else { INIT_TIMEOUT };
                i.cmd(&hex, wait)
            })
        })?;
        self.elm.with(|i| {
            if let Some(k) = i.kline.as_mut() {
                k.started = true;
            }
        });
        frames(&self.elm, &text, &obd_payload)
    }
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) || !s.is_ascii() {
        return None;
    }
    (0..s.len() / 2).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).ok()).collect()
}

impl KwpLink for KLineLink {
    fn send(&mut self, data: &[u8]) -> Result<(), LinkError> {
        let (target, tester) = (self.target, self.tester);
        let hex: String = data.iter().map(|b| format!("{b:02X}")).collect();
        let text = self.elm.with(|inn| {
            inn.retrying(&mut |i| {
                i.kline_address(target, tester, false)?;
                let started = i.kline.as_ref().is_some_and(|k| k.started);
                let wait = if started { i.timeout * 8.0 + 2.0 } else { INIT_TIMEOUT };
                i.cmd(&hex, wait)
            })
        })?;
        self.elm.with(|i| {
            if let Some(k) = i.kline.as_mut() {
                k.started = true;
            }
        });
        self.pending = self.replies(&text)?;
        Ok(())
    }

    fn recv(&mut self, _timeout: f64) -> Option<Vec<u8>> {
        if self.pending.is_empty() { None } else { Some(self.pending.remove(0)) }
    }
}
