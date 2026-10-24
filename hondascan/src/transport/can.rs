//! Raw CAN frame I/O: SocketCAN (Linux), slcan/Lawicel serial adapters, an in-process virtual bus, and a
//! candump-format capture log. Port of hondascan/transport/can.py.
use std::collections::BTreeSet;
use std::io;
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const CAN_EFF_FLAG: u32 = 0x8000_0000;
pub const CAN_EFF_MASK: u32 = 0x1FFF_FFFF;
pub const CAN_SFF_MASK: u32 = 0x7FF;
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
                Err(e) if e.raw_os_error() == Some(libc::ENOBUFS) => std::thread::sleep(Duration::from_millis(2)),
                Err(e) => return Err(e),
            }
        }
        Err(io::Error::from_raw_os_error(libc::ENOBUFS))
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

/// CAN_RAW_FILTER value: one exact-match filter per extended id (the kernel ORs them).
pub fn socketcan_filters(ids: &BTreeSet<u32>) -> Vec<u8> {
    let mut out = vec![];
    for i in ids {
        out.extend(((i & CAN_EFF_MASK) | CAN_EFF_FLAG).to_ne_bytes());
        out.extend((CAN_EFF_MASK | CAN_EFF_FLAG).to_ne_bytes());
    }
    out
}

// -- SocketCAN -----------------------------------------------------------------------------------------------------

/// Linux SocketCAN raw socket (can0, vcan0, slcan0 after slcand, gs_usb/candleLight, PEAK, Kvaser...).
pub struct SocketCanBus {
    core: BusCore,
    fd: libc::c_int,
}

impl SocketCanBus {
    pub fn open(iface: &str, log: Option<Arc<CaptureLog>>) -> io::Result<SocketCanBus> {
        unsafe {
            let fd = libc::socket(libc::AF_CAN, libc::SOCK_RAW, libc::CAN_RAW);
            if fd < 0 {
                return Err(io::Error::last_os_error());
            }
            let mut ifr: libc::ifreq = std::mem::zeroed();
            let name = std::ffi::CString::new(iface).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
            let b = name.as_bytes_with_nul();
            if b.len() > ifr.ifr_name.len() {
                libc::close(fd);
                return Err(io::Error::new(io::ErrorKind::InvalidInput, "interface name too long"));
            }
            for (d, s) in ifr.ifr_name.iter_mut().zip(b) {
                *d = *s as libc::c_char;
            }
            if libc::ioctl(fd, libc::SIOCGIFINDEX, &mut ifr) < 0 {
                let e = io::Error::last_os_error();
                libc::close(fd);
                return Err(e);
            }
            let mut addr: libc::sockaddr_can = std::mem::zeroed();
            addr.can_family = libc::AF_CAN as libc::sa_family_t;
            addr.can_ifindex = ifr.ifr_ifru.ifru_ifindex;
            if libc::bind(fd, &addr as *const _ as *const libc::sockaddr, std::mem::size_of::<libc::sockaddr_can>() as u32) < 0 {
                let e = io::Error::last_os_error();
                libc::close(fd);
                return Err(e);
            }
            Ok(SocketCanBus { core: BusCore::with_log(log), fd })
        }
    }
}

impl CanBus for SocketCanBus {
    fn core(&self) -> &BusCore {
        &self.core
    }

    fn apply_filters(&self) {
        let f = socketcan_filters(&self.ids());
        unsafe {
            libc::setsockopt(self.fd, libc::SOL_CAN_RAW, libc::CAN_RAW_FILTER, f.as_ptr() as *const _, f.len() as u32);
        }
    }

    fn send_raw(&self, frame: &CanFrame) -> io::Result<()> {
        let cid = if frame.extended { (frame.can_id & CAN_EFF_MASK) | CAN_EFF_FLAG } else { frame.can_id & CAN_SFF_MASK };
        let mut raw = [0u8; 16]; // struct can_frame: can_id (u32), can_dlc (u8), 3 pad, data[8]
        raw[..4].copy_from_slice(&cid.to_ne_bytes());
        raw[4] = frame.data.len().min(8) as u8;
        raw[8..8 + frame.data.len().min(8)].copy_from_slice(&frame.data[..frame.data.len().min(8)]);
        let n = unsafe { libc::send(self.fd, raw.as_ptr() as *const _, 16, 0) };
        if n < 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
    }

    fn recv_raw(&self, timeout: f64) -> Option<CanFrame> {
        let end = Instant::now() + secs(timeout);
        loop {
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return None;
            }
            let mut p = libc::pollfd { fd: self.fd, events: libc::POLLIN, revents: 0 };
            let r = unsafe { libc::poll(&mut p, 1, left.as_millis().min(i32::MAX as u128).max(1) as i32) };
            if r <= 0 {
                if r < 0 && io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
                    return None;
                }
                continue;
            }
            let mut raw = [0u8; 16];
            let n = unsafe { libc::recv(self.fd, raw.as_mut_ptr() as *mut _, 16, 0) };
            if n < 16 {
                continue;
            }
            let cid = u32::from_ne_bytes([raw[0], raw[1], raw[2], raw[3]]);
            if cid & 0x6000_0000 != 0 {
                continue; // RTR / error frames
            }
            let ext = cid & CAN_EFF_FLAG != 0;
            let dlc = (raw[4] as usize).min(8);
            return Some(CanFrame { can_id: cid & if ext { CAN_EFF_MASK } else { CAN_SFF_MASK }, data: raw[8..8 + dlc].to_vec(), extended: ext, timestamp: now() });
        }
    }

    fn close(&self) {
        unsafe {
            libc::close(self.fd);
        }
    }
}

// -- serial + slcan ----------------------------------------------------------------------------------------------------

/// Minimal raw termios serial port.
pub struct SerialPort {
    fd: libc::c_int,
    lost: std::sync::atomic::AtomicBool,
}

fn baud_const(baud: u32) -> Option<libc::speed_t> {
    Some(match baud {
        9600 => libc::B9600,
        38400 => libc::B38400,
        57600 => libc::B57600,
        115200 => libc::B115200,
        230400 => libc::B230400,
        460800 => libc::B460800,
        500000 => libc::B500000,
        921600 => libc::B921600,
        1000000 => libc::B1000000,
        2000000 => libc::B2000000,
        _ => return None,
    })
}

impl SerialPort {
    pub fn supported_baud(baud: u32) -> bool {
        baud_const(baud).is_some()
    }

    pub fn bauds() -> Vec<u32> {
        vec![9600, 38400, 57600, 115200, 230400, 460800, 500000, 921600, 1000000, 2000000]
    }

    /// the port is gone: poll says HUP/ERR or a read hit EOF.
    pub fn lost(&self) -> bool {
        self.lost.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn open(path: &str, baud: u32) -> io::Result<SerialPort> {
        let speed = baud_const(baud).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, format!("unsupported baud {baud}")))?;
        let busy = || io::Error::from_raw_os_error(libc::EBUSY);
        let c = std::ffi::CString::new(path).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
        unsafe {
            // one reader only: a second one (another hondascan, ModemManager probing with AT commands) takes replies
            // meant for us and puts the adapter out of step. TIOCEXCL refuses later opens, flock another hondascan.
            let fd = libc::open(c.as_ptr(), libc::O_RDWR | libc::O_NOCTTY | libc::O_NONBLOCK);
            if fd < 0 {
                let e = io::Error::last_os_error();
                return Err(if e.raw_os_error() == Some(libc::EBUSY) { busy() } else { e });
            }
            if libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) < 0 || libc::ioctl(fd, libc::TIOCEXCL) < 0 {
                libc::close(fd);
                return Err(busy());
            }
            let mut t: libc::termios = std::mem::zeroed();
            libc::tcgetattr(fd, &mut t);
            t.c_iflag = 0;
            t.c_oflag = 0;
            t.c_cflag = libc::CS8 | libc::CREAD | libc::CLOCAL;
            t.c_lflag = 0; // raw
            libc::cfsetispeed(&mut t, speed);
            libc::cfsetospeed(&mut t, speed);
            t.c_cc[libc::VMIN] = 0;
            t.c_cc[libc::VTIME] = 0;
            libc::tcsetattr(fd, libc::TCSANOW, &t);
            libc::tcflush(fd, libc::TCIOFLUSH);
            Ok(SerialPort { fd, lost: std::sync::atomic::AtomicBool::new(false) })
        }
    }

    /// All of `data`, or ETIMEDOUT when the device takes nothing for `timeout`.
    pub fn write(&self, data: &[u8], timeout: f64) -> io::Result<()> {
        let end = Instant::now() + secs(timeout);
        let mut rest = data;
        while !rest.is_empty() {
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(io::Error::from_raw_os_error(libc::ETIMEDOUT));
            }
            let mut p = libc::pollfd { fd: self.fd, events: libc::POLLOUT, revents: 0 };
            unsafe { libc::poll(&mut p, 1, left.as_millis().max(1) as i32) };
            let n = unsafe { libc::write(self.fd, rest.as_ptr() as *const _, rest.len()) };
            if n < 0 {
                if io::Error::last_os_error().kind() == io::ErrorKind::WouldBlock {
                    std::thread::sleep(Duration::from_millis(10)); // writable per poll, then EAGAIN: wait again
                    continue;
                }
                return Err(io::Error::last_os_error());
            }
            rest = &rest[n as usize..];
        }
        Ok(())
    }

    /// Bytes that arrived within `timeout`, or empty. USB CDC ports can report readable and then fail with EAGAIN;
    /// that is "nothing yet", not an error, and not lost: only HUP/ERR from the poll or EOF from the read set `lost`.
    pub fn read(&self, timeout: f64) -> Vec<u8> {
        let mut p = libc::pollfd { fd: self.fd, events: libc::POLLIN, revents: 0 };
        let ms = (timeout.max(0.0) * 1000.0).ceil().min(i32::MAX as f64) as i32;
        if unsafe { libc::poll(&mut p, 1, ms) } <= 0 {
            return vec![];
        }
        if p.revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0 {
            self.lost.store(true, std::sync::atomic::Ordering::Relaxed); // unplugged, or the driver gave the port up
            return vec![];
        }
        let mut buf = [0u8; 4096];
        let n = unsafe { libc::read(self.fd, buf.as_mut_ptr() as *mut _, buf.len()) };
        if n > 0 { buf[..n as usize].to_vec() } else if n == 0 {
            self.lost.store(true, std::sync::atomic::Ordering::Relaxed); // EOF: the adapter is gone
            vec![]
        } else if matches!(io::Error::last_os_error().kind(), io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted) {
            vec![] // nothing yet, as before
        } else {
            self.lost.store(true, std::sync::atomic::Ordering::Relaxed);
            vec![]
        }
    }

    pub fn close(&self) {
        unsafe {
            libc::ioctl(self.fd, libc::TIOCNXCL); // the flag outlives us while another fd is open (a pty)
            libc::close(self.fd);
        }
    }
}

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
                return Err(io::Error::from_raw_os_error(libc::ENOBUFS));
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
