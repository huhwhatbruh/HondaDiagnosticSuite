//! Raw CAN frame I/O: SocketCAN (Linux), slcan/Lawicel serial adapters, an in-process virtual bus, and a
//! candump-format capture log. Port of hondascan/transport/can.py.
use super::serial::SerialPort;
use std::collections::BTreeSet;
use std::io;
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const CAN_EFF_FLAG: u32 = 0x8000_0000;
pub const CAN_EFF_MASK: u32 = 0x1FFF_FFFF;
pub const CAN_SFF_MASK: u32 = 0x7FF;
/// "No buffer space available": what a full transmit queue reports (SocketCAN's txqueuelen, a J2534 device's buffer).
#[cfg(unix)]
pub const ENOBUFS: i32 = libc::ENOBUFS;
#[cfg(windows)]
pub const ENOBUFS: i32 = 10055; // WSAENOBUFS
pub const WRITE_TIMEOUT_S: f64 = 2.0; // a serial device that takes nothing for this long is wedged

pub fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

pub fn secs(x: f64) -> Duration {
    Duration::from_secs_f64(x.max(0.0))
}

#[derive(Clone, Debug, PartialEq)]
pub struct CanFrame {
    pub can_id: u32,
    pub data: Vec<u8>,
    pub extended: bool,
    pub timestamp: f64,
}

impl CanFrame {
    pub fn new(can_id: u32, data: impl Into<Vec<u8>>) -> CanFrame {
        CanFrame { can_id, data: data.into(), extended: true, timestamp: 0.0 }
    }

    pub fn std(can_id: u32, data: impl Into<Vec<u8>>) -> CanFrame {
        CanFrame { can_id, data: data.into(), extended: false, timestamp: 0.0 }
    }
}

impl std::fmt::Display for CanFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let hex: String = self.data.iter().map(|b| format!("{b:02X}")).collect();
        if self.extended { write!(f, "{:08X}#{hex}", self.can_id) } else { write!(f, "{:03X}#{hex}", self.can_id) }
    }
}

// -- capture log -----------------------------------------------------------------------------------------------

pub const FRAME_CAP: usize = 200 * 1024 * 1024; // bytes per file; past it the capture stops and says so
const UNATTACHED_LINES: usize = 50_000; // lines kept in memory until a file is attached

/// Python's `errors='backslashreplace'` for an ASCII file.
fn ascii_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c as u32 {
            0..=0x7F => out.push(c),
            n @ 0x80..=0xFF => out.push_str(&format!("\\x{n:02x}")),
            n @ 0x100..=0xFFFF => out.push_str(&format!("\\u{n:04x}")),
            n => out.push_str(&format!("\\U{n:08x}")),
        }
    }
    out
}

#[derive(Default)]
struct LogInner {
    file: Option<std::fs::File>,
    path: Option<std::path::PathBuf>,
    buf: Vec<String>,
    size: usize,
    capped: bool,
}

/// candump -L style log: `(timestamp) hondascan ID#DATA ; TX|RX`, plus `# ` notes. Replayable by can-utils and by
/// `-a replay:`. Without a path, lines wait in memory (the last 50,000) until `attach`; a file stops at `cap` bytes.
/// Every line is written through at once, so a killed process loses no frame.
pub struct CaptureLog {
    inner: Mutex<LogInner>,
    cap: usize,
}

impl CaptureLog {
    pub fn new() -> CaptureLog {
        CaptureLog::with_cap(FRAME_CAP)
    }

    pub fn with_cap(cap: usize) -> CaptureLog {
        CaptureLog { inner: Mutex::new(LogInner::default()), cap }
    }

    pub fn at(path: &std::path::Path) -> io::Result<CaptureLog> {
        let l = CaptureLog::new();
        l.attach(path)?;
        Ok(l)
    }

    fn line(&self, g: &mut LogInner, s: String) {
        use std::io::Write;
        if g.file.is_none() {
            g.buf.push(s);
            if g.buf.len() > UNATTACHED_LINES {
                let extra = g.buf.len() - UNATTACHED_LINES;
                g.buf.drain(..extra);
            }
            return;
        }
        if g.capped {
            return;
        }
        g.size += s.chars().count();
        let s = if g.size > self.cap {
            g.capped = true;
            format!("# capture stopped at {} bytes\n", self.cap)
        } else {
            s
        };
        let failed = if let Some(f) = g.file.as_mut() {
            f.write_all(ascii_escape(&s).as_bytes()).is_err()
        } else {
            false
        };
        // A frame that failed to persist must not look like a kept one: detaching makes is_attached() false,
        // so the write gate refuses while the disk (or the file) is failing.
        if failed {
            g.file = None;
            g.path = None;
        }
    }

    pub fn write(&self, frame: &CanFrame, direction: &str) {
        let s = format!("({:.6}) hondascan {frame} ; {direction}\n", now());
        let mut g = self.inner.lock().unwrap();
        self.line(&mut g, s);
    }

    pub fn note(&self, text: &str) {
        let mut g = self.inner.lock().unwrap();
        self.line(&mut g, format!("# {text}\n"));
    }

    pub fn attach(&self, path: &std::path::Path) -> io::Result<()> {
        self.detach();
        let f = std::fs::OpenOptions::new().append(true).create(true).open(path)?;
        let mut g = self.inner.lock().unwrap();
        g.file = Some(f);
        g.path = Some(path.to_path_buf());
        g.size = 0;
        g.capped = false;
        for s in std::mem::take(&mut g.buf) {
            self.line(&mut g, s);
        }
        Ok(())
    }

    pub fn detach(&self) {
        let mut g = self.inner.lock().unwrap();
        g.file = None;
        g.path = None;
    }

    /// The file `attach` opened, while it is attached: the audit trail names it for a write (`frame_log`).
    pub fn path(&self) -> Option<std::path::PathBuf> {
        self.inner.lock().unwrap().path.clone()
    }

    /// Whether frames are going to a file (a write refuses to run without one).
    pub fn is_attached(&self) -> bool {
        self.inner.lock().unwrap().file.is_some()
    }

    pub fn capped(&self) -> bool {
        self.inner.lock().unwrap().capped
    }
}

impl Default for CaptureLog {
    fn default() -> Self {
        CaptureLog::new()
    }
}

// -- bus ---------------------------------------------------------------------------------------------------------

/// State every bus shares: the capture log and the ids of interest.
#[derive(Default)]
pub struct BusCore {
    pub log: Mutex<Option<Arc<CaptureLog>>>,
    pub ids: Mutex<BTreeSet<u32>>,
}

impl BusCore {
    pub fn with_log(log: Option<Arc<CaptureLog>>) -> BusCore {
        BusCore { log: Mutex::new(log), ids: Mutex::new(BTreeSet::new()) }
    }
}

/// Send/receive raw CAN frames. Implementations provide `core`, `send_raw` and `recv_raw`.
pub trait CanBus: Send + Sync {
    fn core(&self) -> &BusCore;
    fn send_raw(&self, frame: &CanFrame) -> io::Result<()>;
    fn recv_raw(&self, timeout: f64) -> Option<CanFrame>;
    /// Backends with hardware/kernel filtering install the ids here; the default receives everything.
    fn apply_filters(&self) {}
    fn close(&self) {}

    /// A device description, for buses that have one (J2534).
    fn describe(&self) -> Option<String> {
        None
    }

    fn log(&self) -> Option<Arc<CaptureLog>> {
        self.core().log.lock().unwrap().clone()
    }

    /// Queue `frame`. A full transmit queue (SocketCAN: txqueuelen 10, ENOBUFS on a burst) is waited out.
    fn send(&self, frame: &CanFrame) -> io::Result<()> {
        if let Some(l) = self.log() {
            l.write(frame, "TX");
        }
        for _ in 0..500 {
            match self.send_raw(frame) {
                Ok(()) => return Ok(()),
                Err(e) if e.raw_os_error() == Some(ENOBUFS) => std::thread::sleep(Duration::from_millis(2)),
                Err(e) => return Err(e),
            }
        }
        Err(io::Error::from_raw_os_error(ENOBUFS))
    }

    /// Next frame, or None after `timeout` seconds.
    fn recv(&self, timeout: f64) -> Option<CanFrame> {
        let f = self.recv_raw(timeout)?;
        if let Some(l) = self.log() {
            l.write(&f, "RX");
        }
        Some(f)
    }

    /// Add an (extended) id to the ones of interest. The bus delivers the union of all ids it was asked for (every link
    /// on a shared bus keeps receiving); before the first call it delivers everything.
    fn listen(&self, can_id: u32) {
        self.core().ids.lock().unwrap().insert(can_id);
        self.apply_filters();
    }

    fn ids(&self) -> BTreeSet<u32> {
        self.core().ids.lock().unwrap().clone()
    }
}

// -- serial + slcan ----------------------------------------------------------------------------------------------------

/// Lawicel/slcan ASCII protocol over a serial port (CANable slcan firmware, USBtin, many clones).
pub struct SlcanBus {
    core: BusCore,
    port: SerialPort,
    state: Mutex<(Vec<u8>, std::collections::VecDeque<CanFrame>)>, // (partial line, parsed frames)
}

fn bitrate_cmd(bitrate: u32) -> Option<&'static str> {
    Some(match bitrate {
        10000 => "S0",
        20000 => "S1",
        50000 => "S2",
        100000 => "S3",
        125000 => "S4",
        250000 => "S5",
        500000 => "S6",
        800000 => "S7",
        1000000 => "S8",
        _ => return None,
    })
}

impl SlcanBus {
    pub fn open(port: &str, bitrate: u32, baud: u32, log: Option<Arc<CaptureLog>>) -> io::Result<SlcanBus> {
        let rate = bitrate_cmd(bitrate).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, format!("unsupported bitrate {bitrate}")))?;
        let port = SerialPort::open(port, baud)?;
        for cmd in ["C", rate, "O"] {
            port.write(format!("{cmd}\r").as_bytes(), WRITE_TIMEOUT_S)?;
            std::thread::sleep(Duration::from_millis(50));
        }
        port.read(0.1);
        Ok(SlcanBus { core: BusCore::with_log(log), port, state: Mutex::new((vec![], Default::default())) })
    }

    pub fn parse(s: &str) -> Option<CanFrame> {
        let hex = |a: usize, b: usize| s.get(a..b).and_then(|h| u32::from_str_radix(h, 16).ok());
        let bytes = |a: usize, n: usize| -> Option<Vec<u8>> {
            let h = s.get(a..a + 2 * n)?;
            (0..n).map(|i| u8::from_str_radix(&h[2 * i..2 * i + 2], 16).ok()).collect()
        };
        match s.chars().next()? {
            'T' => {
                let n = hex(9, 10)? as usize;
                Some(CanFrame { can_id: hex(1, 9)?, data: bytes(10, n)?, extended: true, timestamp: now() })
            }
            't' => {
                let n = hex(4, 5)? as usize;
                Some(CanFrame { can_id: hex(1, 4)?, data: bytes(5, n)?, extended: false, timestamp: now() })
            }
            _ => None,
        }
    }
}

impl CanBus for SlcanBus {
    fn core(&self) -> &BusCore {
        &self.core
    }

    fn send_raw(&self, frame: &CanFrame) -> io::Result<()> {
        let hex: String = frame.data.iter().map(|b| format!("{b:02X}")).collect();
        let s = if frame.extended { format!("T{:08X}{}{hex}\r", frame.can_id, frame.data.len()) } else { format!("t{:03X}{}{hex}\r", frame.can_id, frame.data.len()) };
        self.port.write(s.as_bytes(), WRITE_TIMEOUT_S)
    }

    fn recv_raw(&self, timeout: f64) -> Option<CanFrame> {
        let end = Instant::now() + secs(timeout);
        loop {
            {
                let mut st = self.state.lock().unwrap();
                if let Some(f) = st.1.pop_front() {
                    return Some(f);
                }
            }
            let left = end.saturating_duration_since(Instant::now()).as_secs_f64();
            if left <= 0.0 {
                return None;
            }
            let got = self.port.read(left);
            if self.port.lost() {
                return None; // the port is gone: a timeout, not a spin until the caller's deadline
            }
            let mut st = self.state.lock().unwrap();
            st.0.extend(got);
            while let Some(i) = st.0.iter().position(|b| *b == b'\r') {
                let line: Vec<u8> = st.0.drain(..=i).collect();
                let text: String = String::from_utf8_lossy(&line[..line.len() - 1]).trim_matches(|c| c == '\u{7}' || c == '\n').to_string();
                if let Some(f) = SlcanBus::parse(&text) {
                    st.1.push_back(f);
                }
            }
        }
    }

    fn close(&self) {
        let _ = self.port.write(b"C\r", WRITE_TIMEOUT_S);
        self.port.close();
    }
}

// -- virtual bus -------------------------------------------------------------------------------------------------------

type Queues = Arc<Mutex<Vec<(usize, Sender<CanFrame>)>>>;

/// In-process broadcast bus. `VirtualBus::new().endpoint()` gives CanBus objects that see each other's frames.
#[derive(Default, Clone)]
pub struct VirtualBus {
    queues: Queues,
}

impl VirtualBus {
    pub fn new() -> VirtualBus {
        VirtualBus::default()
    }

    pub fn endpoint(&self, log: Option<Arc<CaptureLog>>) -> VirtualEndpoint {
        let (tx, rx) = channel();
        let mut q = self.queues.lock().unwrap();
        let id = q.len();
        q.push((id, tx));
        VirtualEndpoint { core: BusCore::with_log(log), bus: self.queues.clone(), id, rx: Mutex::new(rx), txq: Mutex::new(vec![]) }
    }
}

/// A SocketCAN interface queues 10 frames (txqueuelen) and drains them at the bit rate: an extended 8-byte frame takes
/// about 0.26 ms at 500 kbit/s. A burst past that gets ENOBUFS, here as on the hardware.
pub struct VirtualEndpoint {
    core: BusCore,
    bus: Queues,
    id: usize,
    rx: Mutex<Receiver<CanFrame>>,
    txq: Mutex<Vec<Instant>>, // times at which queued frames leave the interface
}

pub const TXQUEUELEN: usize = 10;
pub const FRAME_TIME: f64 = 128.0 / 500000.0;

impl CanBus for VirtualEndpoint {
    fn core(&self) -> &BusCore {
        &self.core
    }

    fn send_raw(&self, frame: &CanFrame) -> io::Result<()> {
        let now_i = Instant::now();
        {
            let mut q = self.txq.lock().unwrap();
            q.retain(|t| *t > now_i);
            if q.len() >= TXQUEUELEN {
                return Err(io::Error::from_raw_os_error(ENOBUFS));
            }
            let start = q.last().copied().unwrap_or(now_i).max(now_i);
            q.push(start + secs(FRAME_TIME));
        }
        let f = CanFrame { can_id: frame.can_id, data: frame.data.clone(), extended: frame.extended, timestamp: now() };
        for (id, tx) in self.bus.lock().unwrap().iter() {
            if *id != self.id {
                let _ = tx.send(f.clone());
            }
        }
        Ok(())
    }

    /// Filters like SocketCAN does, so tests catch a link that loses another link's frames.
    fn recv_raw(&self, timeout: f64) -> Option<CanFrame> {
        let end = Instant::now() + secs(timeout);
        let rx = self.rx.lock().unwrap();
        loop {
            let f = match rx.recv_timeout(end.saturating_duration_since(Instant::now())) {
                Ok(f) => f,
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => return None,
            };
            let ids = self.core.ids.lock().unwrap();
            if ids.is_empty() || ids.contains(&f.can_id) {
                return Some(f);
            }
        }
    }
}
