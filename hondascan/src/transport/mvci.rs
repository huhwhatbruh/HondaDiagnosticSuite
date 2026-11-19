//! XHorse M-VCI (Mini-VCI, FTDI, firmware "J2534 MINI V1.03") as a raw CAN bus.
//! The cable's own protocol is [rmvci](https://github.com/FletcherD/rmvci); hondascan keeps its ISO-TP on top, the same
//! way it does for a J2534 raw-CAN channel. One acceptance filter, because the firmware keeps one range per id width
//! and stays silent until that filter is installed. The range is Honda's response mailbox `0x18DAF100`–`0x18DAF1FF`.
use super::can::{now, secs, BusCore, CanBus, CanFrame, CaptureLog, CAN_EFF_MASK, CAN_SFF_MASK};
use rmvci_core::{Can, CanConfig, CanFilter, CanId, Channel, Device, Error, RxMsg, RxStatus};
use std::io;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// A poll shorter than this can time out before the cable's reply (the FTDI link is about 16 ms). The reply then sits in
/// the serial buffer and the next write reads it — a ReadMsg "buffer empty" — as its own status.
const POLL_FLOOR: Duration = Duration::from_millis(50);

/// 29-bit connect flag, the same bit J2534 calls `CAN_29BIT_ID`.
const CAN_29BIT_ID: u32 = 0x100;
const BITRATE: u32 = 500_000;
/// Firmware range is `[pattern & mask, pattern | !mask]`, so this is exactly `0x18DAF100`..=`0x18DAF1FF`.
const MAILBOX_MASK: u32 = 0xFFFF_FF00;
const MAILBOX_PATTERN: u32 = 0x18DA_F100;

pub struct MvciBus {
    core: BusCore,
    /// Dropped after `chan`: the channel disconnects, then the port is released.
    chan: Mutex<Option<Channel<Can>>>,
    device: Mutex<Option<Device>>,
    version: String,
}

impl MvciBus {
    pub fn open(port: &str, log: Option<Arc<CaptureLog>>) -> Result<MvciBus, Error> {
        let device = Device::open(port)?;
        let version = device.firmware_version().unwrap_or_default();
        let mut chan = device.connect::<Can>(CanConfig { bitrate: BITRATE, flags: CAN_29BIT_ID })?;
        chan.set_filter(CanFilter { mask: MAILBOX_MASK, pattern: CanId::Ext(MAILBOX_PATTERN) })?;
        Ok(MvciBus { core: BusCore::with_log(log), chan: Mutex::new(Some(chan)), device: Mutex::new(Some(device)), version })
    }
}

fn frame_of(msg: RxMsg) -> Option<CanFrame> {
    if msg.data.len() < 4 {
        return None;
    }
    let raw = u32::from_be_bytes(msg.data[..4].try_into().ok()?);
    let extended = msg.rx_status.contains(RxStatus::CAN_29BIT) || raw > CAN_SFF_MASK;
    let can_id = raw & if extended { CAN_EFF_MASK } else { CAN_SFF_MASK };
    Some(CanFrame { can_id, data: msg.data[4..].to_vec(), extended, timestamp: now() })
}

impl CanBus for MvciBus {
    fn core(&self) -> &BusCore {
        &self.core
    }

    fn describe(&self) -> Option<String> {
        Some(if self.version.is_empty() { "XHorse M-VCI".into() } else { format!("XHorse M-VCI {}", self.version) })
    }

    fn send_raw(&self, frame: &CanFrame) -> io::Result<()> {
        let id = if frame.extended { CanId::Ext(frame.can_id & CAN_EFF_MASK) } else { CanId::Std((frame.can_id & CAN_SFF_MASK) as u16) };
        let mut guard = self.chan.lock().unwrap();
        let chan = guard.as_mut().ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "M-VCI closed"))?;
        chan.send(id, &frame.data).map_err(|e| io::Error::other(e.to_string()))
    }

    fn recv_raw(&self, timeout: f64) -> Option<CanFrame> {
        let deadline = Instant::now() + secs(timeout);
        let mut guard = self.chan.lock().unwrap();
        let chan = guard.as_mut()?;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining < POLL_FLOOR {
                return None;
            }
            // `poll` returns one queued message. Transmit echoes carry no data; an empty buffer means keep waiting.
            match chan.poll(remaining.min(Duration::from_millis(500))) {
                Ok(Some(msg)) if !msg.is_indication() => return frame_of(msg),
                Ok(_) => {}
                Err(Error::Timeout(_)) => return None,
                Err(_) => return None,
            }
        }
    }

    fn close(&self) {
        self.chan.lock().unwrap().take();
        if let Some(dev) = self.device.lock().unwrap().take() {
            dev.close();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{MAILBOX_MASK, MAILBOX_PATTERN};

    #[test]
    fn the_mailbox_filter_is_exactly_the_honda_response_ids() {
        let (lo, hi) = (MAILBOX_PATTERN & MAILBOX_MASK, MAILBOX_PATTERN | !MAILBOX_MASK);
        assert_eq!((lo, hi), (0x18DA_F100, 0x18DA_F1FF));
    }
}
