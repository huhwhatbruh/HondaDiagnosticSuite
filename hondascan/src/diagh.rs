//! Honda DIAG-H: Honda's own K-line message format (the 55 systems HDS lists as the HONDA protocol family). Port of
//! hondascan/diagh.py.
//!
//! STATUS: never run on a car or a bus. The framing and the per-family messages are HDS's data
//! (hds-xref/decoded/protocol_properties.csv, rows 1, 2, 3, 13 and 14; row 17, the immobilizer, is left out on purpose). NOT known,
//! and not guessed here: the bus-level initialisation, the baud rate, the meaning of the data bytes of a readout / inforead, and
//! what an ECU answers. No adapter in the repo can drive this line.
//!
//! Message: [command, length, data..., checksum]. `length` counts every byte of the message; the checksum makes all bytes sum to
//! 0 modulo 256. Examples from HDS: wake-up `FE 04 91 6D`, stop `AD 04 F1 5E`, keep-awake `A0 05 70 01 EA`.
use std::collections::BTreeSet;
use std::fmt;
use std::time::Instant;

pub const READOUT: u8 = 0xA0;
pub const CLEAR: u8 = 0xA1;
pub const INFOREAD: u8 = 0xA2;
pub const WAKE: u8 = 0xFE;

#[derive(Debug, Clone, PartialEq)]
pub enum DiagHError {
    Error(String),
    NotAllowed(String),
    NoResponse(String),
}

impl fmt::Display for DiagHError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            DiagHError::Error(s) | DiagHError::NotAllowed(s) | DiagHError::NoResponse(s) => write!(f, "{s}"),
        }
    }
}

impl std::error::Error for DiagHError {}

fn err<T>(s: impl Into<String>) -> Result<T, DiagHError> {
    Err(DiagHError::Error(s.into()))
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn checksum(body: &[u8]) -> u8 {
    (-(body.iter().fold(0i64, |a, b| a + *b as i64))) as u8
}

pub fn build(command: u8, data: &[u8]) -> Result<Vec<u8>, DiagHError> {
    if data.len() + 3 > 255 {
        return err("a DIAG-H message is at most 255 bytes");
    }
    let mut body = vec![command, (data.len() + 3) as u8];
    body.extend(data);
    body.push(checksum(&body));
    Ok(body)
}

/// (command, data) of one complete message; an error if the length byte or the checksum is wrong.
pub fn parse(msg: &[u8]) -> Result<(u8, Vec<u8>), DiagHError> {
    if msg.len() < 3 || msg[1] as usize != msg.len() {
        return err(format!("length byte does not match message {}", hex(msg)));
    }
    if msg.iter().fold(0u32, |a, b| a + *b as u32) & 0xFF != 0 {
        return err(format!("bad checksum in {} (expected {:02X})", hex(msg), checksum(&msg[..msg.len() - 1])));
    }
    Ok((msg[0], msg[2..msg.len() - 1].to_vec()))
}

/// The messages in a byte string that holds several (HDS's row 3 wake-up is two messages back to back).
pub fn split(stream: &[u8]) -> Result<Vec<Vec<u8>>, DiagHError> {
    let (mut out, mut i) = (vec![], 0usize);
    while i < stream.len() {
        if i + 1 >= stream.len() || stream[i + 1] < 3 || i + stream[i + 1] as usize > stream.len() {
            return err(format!("truncated message at byte {i} of {}", hex(stream)));
        }
        let n = stream[i + 1] as usize;
        out.push(stream[i..i + n].to_vec());
        i += n;
    }
    Ok(out)
}

/// One protocol row of HDS's table: the messages that open, hold and close a session with a system of that variant.
#[derive(Debug, Clone, Copy)]
pub struct Profile {
    pub row: u32,
    pub wake: &'static [u8],
    pub keep_awake: &'static [u8], // sent periodically while idle; empty when the row has none
    pub stop: &'static [u8],
    pub t2_ms: u32,
    pub t5_ms: u32,
    pub readout: Option<u8>, // HDS's CB_READOUT / CB_INFOREAD / CB_CLEAR command bytes for the row
    pub inforead: Option<u8>,
    pub clear: Option<u8>,
}

impl Profile {
    /// Command bytes a client may send for this row: its own wake-up, keep-awake and stop messages and its read commands.
    pub fn commands(&self) -> BTreeSet<u8> {
        let mut out: BTreeSet<u8> = BTreeSet::new();
        for blob in [self.wake, self.keep_awake, self.stop] {
            if !blob.is_empty() {
                out.extend(split(blob).unwrap_or_default().iter().map(|m| m[0]));
            }
        }
        out.extend([self.readout, self.inforead].into_iter().flatten());
        out
    }

    /// True when HDS uses one command byte for reading and clearing, row 13's 5C: a request cannot be told from a clear by its
    /// command, so generic requests are refused for the row; its own exact messages can still be sent.
    pub fn ambiguous(&self) -> bool {
        self.clear.is_some() && !self.clear_distinct()
    }

    /// False when the row's clear byte is also its readout byte (row 13: 5C for both).
    pub fn clear_distinct(&self) -> bool {
        self.clear.is_some_and(|c| Some(c) != self.readout && Some(c) != self.inforead)
    }
}

/// Row 17 (wake-up FE 04 B1 4D, command D8 = CB_IMMOBI) is the immobilizer: deliberately not here (CLAUDE.md: no immobilizer support).
pub const PROFILES: [Profile; 5] = [
    Profile { row: 1, wake: &[0xFE, 0x04, 0x91, 0x6D], keep_awake: &[0xA2, 0x07, 0x72, 0x00, 0x00, 0x01, 0xE4], stop: &[0xAD, 0x04, 0xF1, 0x5E], t2_ms: 500, t5_ms: 300, readout: None, inforead: Some(INFOREAD), clear: Some(CLEAR) },
    Profile { row: 2, wake: &[0xFE, 0x04, 0x91, 0x6D], keep_awake: &[0xA0, 0x05, 0x70, 0x01, 0xEA], stop: &[0xAD, 0x04, 0xF1, 0x5E], t2_ms: 500, t5_ms: 300, readout: Some(READOUT), inforead: None, clear: Some(CLEAR) },
    Profile { row: 3, wake: &[0xA0, 0x05, 0x00, 0x01, 0x5A, 0xFE, 0x04, 0x91, 0x6D], keep_awake: &[0xA0, 0x05, 0x70, 0x01, 0xEA], stop: &[0xAD, 0x04, 0xF1, 0x5E], t2_ms: 500, t5_ms: 300, readout: Some(READOUT), inforead: None, clear: Some(CLEAR) },
    Profile { row: 13, wake: &[0xFE, 0x04, 0xA9, 0x55], keep_awake: &[0x5C, 0x06, 0x20, 0x19, 0x04, 0x61], stop: &[0x5C, 0x04, 0x00, 0xA0], t2_ms: 0, t5_ms: 0, readout: Some(0x5C), inforead: None, clear: Some(0x5C) },
    Profile { row: 14, wake: &[0x20, 0x05, 0x00, 0x02, 0xD9], keep_awake: &[], stop: &[], t2_ms: 0, t5_ms: 30, readout: Some(0xB8), inforead: None, clear: Some(0xB9) },
];

pub fn profile(row: u32) -> Option<&'static Profile> {
    PROFILES.iter().find(|p| p.row == row)
}

/// Whole messages in and out: `send(message)` and `recv(timeout)` (None on timeout).
pub trait DiagHLink {
    fn send(&mut self, message: &[u8]) -> Result<(), DiagHError>;
    fn recv(&mut self, timeout: f64) -> Result<Option<Vec<u8>>, DiagHError>;
}

pub struct DiagHClient<L: DiagHLink> {
    pub link: L,
    pub profile: Profile,
    pub p2: f64,
    pub allow_clear: bool,
    pub last_sent: Option<Instant>,
}

impl<L: DiagHLink> DiagHClient<L> {
    pub fn new(link: L, profile: Profile, p2: f64, allow_clear: bool) -> DiagHClient<L> {
        DiagHClient { link, profile, p2, allow_clear, last_sent: None }
    }

    fn check(&self, command: u8) -> Result<(), DiagHError> {
        let p = &self.profile;
        let not = |s: String| Err(DiagHError::NotAllowed(s));
        if command == WAKE {
            // only the row's exact wake-up via wake(): a generic FE request could build FE 04 B1 4D (row 17, immobilizer)
            return not("wake-up messages go out only through wake(), as the row defines them".into());
        }
        if p.ambiguous() {
            return not(format!("row {}: command 0x{:02X} reads and clears alike, so generic requests are refused", p.row, p.clear.unwrap_or(0)));
        }
        if p.clear_distinct() && Some(command) == p.clear {
            if !self.allow_clear {
                return not("clearing needs allow_clear=True".into());
            }
        } else if !p.commands().contains(&command) {
            return not(format!("DIAG-H command 0x{command:02X} is not one of row {}'s read, wake-up or stop commands", p.row));
        }
        Ok(())
    }

    pub fn request(&mut self, command: u8, data: &[u8], timeout: Option<f64>) -> Result<(u8, Vec<u8>), DiagHError> {
        self.check(command)?;
        self.link.send(&build(command, data)?)?;
        self.last_sent = Some(Instant::now());
        match self.link.recv(timeout.filter(|t| *t != 0.0).unwrap_or(self.p2))? {
            None => Err(DiagHError::NoResponse(format!("no response to command 0x{command:02X}"))),
            Some(r) => parse(&r),
        }
    }

    /// Send the family's wake-up message(s); returns the replies (one per message, or an empty list).
    pub fn wake(&mut self) -> Result<Vec<(u8, Vec<u8>)>, DiagHError> {
        let mut out = vec![];
        for m in split(self.profile.wake)? {
            // the row's own exact messages: no generic request check
            self.link.send(&m)?;
            self.last_sent = Some(Instant::now());
            if let Some(r) = self.link.recv(self.p2)? {
                out.push(parse(&r)?);
            }
        }
        Ok(out)
    }

    pub fn keep_awake(&mut self) -> Result<Option<(u8, Vec<u8>)>, DiagHError> {
        if self.profile.keep_awake.is_empty() {
            return err(format!("row {} has no keep-awake message in HDS's table", self.profile.row));
        }
        self.link.send(self.profile.keep_awake)?;
        self.last_sent = Some(Instant::now());
        self.link.recv(self.p2)?.map(|r| parse(&r)).transpose()
    }

    pub fn stop(&mut self) -> Result<(), DiagHError> {
        if !self.profile.stop.is_empty() {
            self.link.send(self.profile.stop)?;
            self.link.recv(self.p2)?;
        }
        Ok(())
    }
}

/// A raw one-wire K-line port: `write(bytes)` and `read(n, timeout)` (exactly n bytes, or fewer on timeout).
pub trait RawLine {
    fn write(&mut self, data: &[u8]);
    fn read(&mut self, n: usize, timeout: f64) -> Vec<u8>;
}

/// Messages over a raw one-wire K-line, where every byte sent comes back as an echo. The echo is checked (a mismatch is a
/// collision), then one reply message is read from its length byte.
pub struct HalfDuplexLink<R: RawLine> {
    pub raw: R,
    pub echo: bool,
}

impl<R: RawLine> HalfDuplexLink<R> {
    pub fn new(raw: R, echo: bool) -> HalfDuplexLink<R> {
        HalfDuplexLink { raw, echo }
    }
}

impl<R: RawLine> DiagHLink for HalfDuplexLink<R> {
    fn send(&mut self, message: &[u8]) -> Result<(), DiagHError> {
        self.raw.write(message);
        if self.echo {
            let back = self.raw.read(message.len(), 0.5);
            if back != message {
                return err(format!("echo {} does not match what was sent ({}): bus collision or no adapter", hex(&back), hex(message)));
            }
        }
        Ok(())
    }

    fn recv(&mut self, timeout: f64) -> Result<Option<Vec<u8>>, DiagHError> {
        let head = self.raw.read(2, timeout);
        if head.len() < 2 {
            return Ok(None);
        }
        if head[1] < 3 {
            return err(format!("impossible length byte in {}", hex(&head)));
        }
        let rest = self.raw.read(head[1] as usize - 2, 0.5);
        if rest.len() != head[1] as usize - 2 {
            return err(format!("message cut short after {}{}", hex(&head), hex(&rest)));
        }
        Ok(Some([head, rest].concat()))
    }
}
