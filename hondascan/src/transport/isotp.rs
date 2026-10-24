//! ISO 15765-2 (ISO-TP) over a raw CanBus, classic CAN (8-byte frames), normal/fixed addressing.
//! Port of hondascan/transport/isotp.py.
use super::can::{CanBus, CanFrame};
use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const SF: u8 = 0;
pub const FF: u8 = 1;
pub const CF: u8 = 2;
pub const FC: u8 = 3;
/// What the HDS GNA600 firmware (HCANIF) puts on the wire: every frame DLC 8, unused bytes 0x55, flow control
/// 30 00 00 (CTS, BS 0, STmin 0) -- see README.md "Wire layer".
pub const HDS_PAD: u8 = 0x55;
pub const FC_CTS: u8 = 0;
pub const FC_WAIT: u8 = 1;

#[derive(Debug, Clone, PartialEq)]
pub struct IsoTpError(pub String);

impl fmt::Display for IsoTpError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for IsoTpError {}

/// What a link can fail with: a protocol error (Python's IsoTpError: the UDS client retries it) or an OS error from the
/// bus (Python's OSError: the adapter is gone or its queue stays full; it propagates).
#[derive(Debug)]
pub enum LinkError {
    IsoTp(IsoTpError),
    Io(std::io::Error),
}

impl fmt::Display for LinkError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            LinkError::IsoTp(e) => write!(f, "{e}"),
            LinkError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for LinkError {}

impl From<std::io::Error> for LinkError {
    fn from(e: std::io::Error) -> Self {
        LinkError::Io(e)
    }
}

impl From<IsoTpError> for LinkError {
    fn from(e: IsoTpError) -> Self {
        LinkError::IsoTp(e)
    }
}

fn err<T>(s: impl Into<String>) -> Result<T, LinkError> {
    Err(IsoTpError(s.into()).into())
}

/// A request/response channel to one ECU: payloads in, payloads out (transport framing hidden).
pub trait DiagLink: Send {
    fn send(&mut self, payload: &[u8]) -> Result<(), LinkError>;
    /// Next complete response payload or None.
    fn recv(&mut self, timeout: f64) -> Result<Option<Vec<u8>>, LinkError>;
    fn close(&mut self) {}
}

pub fn stmin_seconds(b: u8) -> f64 {
    match b {
        0..=0x7F => b as f64 / 1000.0,
        0xF1..=0xF9 => (b - 0xF0) as f64 / 10000.0,
        _ => 0.127,
    }
}

/// The physical request id of the ECU that sends from `rx_id`: 0x18DAF10E -> 0x18DA0EF1 (target and source swap).
pub fn reply_to(rx_id: u32) -> u32 {
    (rx_id & 0xFFFF0000) | (rx_id & 0xFF) << 8 | (rx_id >> 8) & 0xFF
}

pub struct IsoTpLink {
    pub bus: Arc<dyn CanBus>,
    pub tx_id: u32,
    pub rx_id: u32,
    pub ext: bool,
    pub fc_id: u32, // an ECU asked functionally gets flow control physically
    pub pad: Option<u8>,
    pub block_size: u8,
    pub stmin: u8,
    pub n_bs: f64,
    pub n_cr: f64,
    pub max_wait: u32,
}

impl IsoTpLink {
    pub fn new(bus: Arc<dyn CanBus>, tx_id: u32, rx_id: u32) -> IsoTpLink {
        IsoTpLink::with(bus, tx_id, rx_id, None)
    }

    /// `fc_id`: where the receiver's flow control arrives, if not `rx_id`.
    pub fn with(bus: Arc<dyn CanBus>, tx_id: u32, rx_id: u32, fc_id: Option<u32>) -> IsoTpLink {
        let fc_id = fc_id.unwrap_or(rx_id);
        bus.listen(rx_id);
        if fc_id != rx_id {
            bus.listen(fc_id);
        }
        IsoTpLink { bus, tx_id, rx_id, ext: true, fc_id, pad: Some(HDS_PAD), block_size: 0, stmin: 0, n_bs: 1.0, n_cr: 1.0, max_wait: 10 }
    }

    fn frame(&self, data: &[u8], can_id: Option<u32>) -> CanFrame {
        let mut d = data.to_vec();
        if let Some(p) = self.pad {
            d.resize(d.len().max(8), p);
        }
        CanFrame { can_id: can_id.unwrap_or(self.tx_id), data: d, extended: self.ext, timestamp: 0.0 }
    }

    /// Flow control goes to the sender of the reply, whatever id the request used (a functional one included).
    fn flow_control(&self) -> CanFrame {
        self.frame(&[0x30, self.block_size, self.stmin], Some(reply_to(self.rx_id)))
    }

    fn await_fc(&self) -> Result<(u8, f64), LinkError> {
        let mut waits = 0;
        let mut end = Instant::now() + super::can::secs(self.n_bs);
        loop {
            let left = end.saturating_duration_since(Instant::now()).as_secs_f64();
            let Some(f) = self.bus.recv(left) else { return err("timeout waiting for flow control (N_Bs)") };
            if f.can_id != self.fc_id || f.data.is_empty() || f.data[0] >> 4 != FC {
                continue;
            }
            match f.data[0] & 0xF {
                FC_CTS => return Ok((f.data.get(1).copied().unwrap_or(0), stmin_seconds(f.data.get(2).copied().unwrap_or(0)))),
                FC_WAIT => {
                    waits += 1;
                    if waits > self.max_wait {
                        return err("too many FC.WAIT");
                    }
                    end = Instant::now() + super::can::secs(self.n_bs);
                }
                _ => return err("receiver overflow (FC.OVFLW)"),
            }
        }
    }

    fn recv_multi(&self, n: usize, mut buf: Vec<u8>) -> Result<Vec<u8>, LinkError> {
        self.bus.send(&self.flow_control())?;
        let (mut seq, mut in_block) = (1u8, 0u32);
        while buf.len() < n {
            let end = Instant::now() + super::can::secs(self.n_cr);
            let f = loop {
                let left = end.saturating_duration_since(Instant::now()).as_secs_f64();
                let Some(f) = self.bus.recv(left) else { return err("timeout waiting for consecutive frame (N_Cr)") };
                if f.can_id == self.rx_id && !f.data.is_empty() && f.data[0] >> 4 == CF {
                    break f;
                }
            };
            if f.data[0] & 0xF != seq {
                return err(format!("consecutive frame out of sequence (got {}, want {seq})", f.data[0] & 0xF));
            }
            buf.extend(&f.data[1..f.data.len().min(8)]);
            seq = (seq + 1) & 0xF;
            in_block += 1;
            if self.block_size != 0 && in_block == self.block_size as u32 && buf.len() < n {
                self.bus.send(&self.flow_control())?;
                in_block = 0;
            }
        }
        buf.truncate(n);
        Ok(buf)
    }
}

impl DiagLink for IsoTpLink {
    fn send(&mut self, payload: &[u8]) -> Result<(), LinkError> {
        let n = payload.len();
        if n == 0 || n > 4095 {
            return err(format!("payload length {n} not supported"));
        }
        if n <= 7 {
            let mut d = vec![n as u8];
            d.extend(payload);
            self.bus.send(&self.frame(&d, None))?;
            return Ok(());
        }
        let mut first = vec![0x10 | (n >> 8) as u8, (n & 0xFF) as u8];
        first.extend(&payload[..6]);
        self.bus.send(&self.frame(&first, None))?;
        let (mut pos, mut seq) = (6usize, 1u8);
        let (mut bs, mut st) = self.await_fc()?;
        let mut sent_in_block = 0u32;
        while pos < n {
            if bs != 0 && sent_in_block == bs as u32 {
                (bs, st) = self.await_fc()?;
                sent_in_block = 0;
            }
            std::thread::sleep(Duration::from_secs_f64(st));
            let mut d = vec![0x20 | seq];
            d.extend(&payload[pos..(pos + 7).min(n)]);
            self.bus.send(&self.frame(&d, None))?;
            pos += 7;
            seq = (seq + 1) & 0xF;
            sent_in_block += 1;
        }
        Ok(())
    }

    fn recv(&mut self, timeout: f64) -> Result<Option<Vec<u8>>, LinkError> {
        let end = Instant::now() + super::can::secs(timeout);
        loop {
            let left = end.saturating_duration_since(Instant::now()).as_secs_f64();
            let Some(f) = self.bus.recv(left) else { return Ok(None) };
            if f.can_id != self.rx_id || f.data.is_empty() {
                continue;
            }
            match f.data[0] >> 4 {
                SF => {
                    let n = (f.data[0] & 0xF) as usize;
                    if 1 <= n && n < f.data.len() {
                        return Ok(Some(f.data[1..1 + n].to_vec()));
                    }
                }
                FF if f.data.len() >= 2 => {
                    let n = (((f.data[0] & 0xF) as usize) << 8) | f.data[1] as usize;
                    return self.recv_multi(n, f.data[2..f.data.len().min(8)].to_vec()).map(Some);
                }
                _ => {}
            }
        }
    }
}
