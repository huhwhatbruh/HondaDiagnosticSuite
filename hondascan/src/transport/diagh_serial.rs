//! Honda DIAG-H on a plain serial port: a USB-to-K-line cable at 9600 8N1, no J2534 id and no ELM involved. The baud is the one
//! HDS and the VCM firmware use for DIAG-H (README, "K-line systems": DiagHConnect 0x2580 = 9600; `test.dll` connects the DiagH ids
//! at 0x2580). Wake-up is HDS's stored message per row (`diagh::Profile`), sent as bytes; HDS's interface also sends an H/99 pulse
//! first unless the row turns `ESTABLISH_MODE_H99` off, and this layer does not (README, "vhonda_properties"). The DIAG-H line is DLC
//! pin 14 by default (vendor property `DIAG_H_PIN`: 1, 3, 8, 9, 11, 12, 13 or 14). This layer only moves bytes;
//! `diagh::HalfDuplexLink` checks the echo a one-wire cable returns.
use super::serial::SerialPort;
use crate::diagh::RawLine;
use std::collections::VecDeque;
use std::io;
use std::time::{Duration, Instant};

pub const BAUD: u32 = 9600;

pub struct SerialLine {
    port: SerialPort,
    pending: VecDeque<u8>, // the port may return more than one message's worth at a time
}

impl SerialLine {
    pub fn open(path: &str) -> io::Result<SerialLine> {
        Ok(SerialLine { port: SerialPort::open(path, BAUD)?, pending: VecDeque::new() })
    }

    pub fn lost(&self) -> bool {
        self.port.lost()
    }
}

impl RawLine for SerialLine {
    /// A failed write shows up as a missing echo (HalfDuplexLink) or a missing reply, so it is not reported twice here.
    fn write(&mut self, data: &[u8]) {
        let _ = self.port.write(data, 1.0);
    }

    fn read(&mut self, n: usize, timeout: f64) -> Vec<u8> {
        let end = Instant::now() + Duration::from_secs_f64(timeout.max(0.0));
        while self.pending.len() < n && !self.port.lost() {
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            self.pending.extend(self.port.read(left.as_secs_f64()));
        }
        let k = n.min(self.pending.len());
        self.pending.drain(..k).collect()
    }
}

impl Drop for SerialLine {
    fn drop(&mut self) {
        self.port.close();
    }
}
