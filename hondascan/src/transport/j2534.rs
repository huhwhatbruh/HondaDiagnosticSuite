//! SAE J2534-1 (PassThru) devices through a native shared library. Port of hondascan/transport/j2534.py.
//!
//! The device is used as a raw CAN interface: a CAN channel (protocol 5, 29-bit ids, 500 kbit/s) with a pass filter on HDS's
//! receive mailbox 0x18DAF100 / 0x1FFFFF00, and hondascan's own ISO-TP on top. The frames are then byte-identical to the other raw
//! adapters (0x55 padding, flow control 30 00 00), and the capture log and replay work unchanged. The device's own ISO 15765 layer
//! is not used: it would hide the frames and pad them with its own byte.
//!
//! The library must export the J2534 C API with the platform's C calling convention (a Linux build; the Windows DLLs are stdcall
//! and cannot be loaded here). For the GODIAG GD101 that is hondascan's own driver, gd101_j2534.c (C: it is a C API; `gd101_library()`
//! builds it once with the system C compiler); other devices with a Linux J2534 library work as `j2534:/path/to/lib.so`.
use super::can::{now, BusCore, CanBus, CanFrame, CaptureLog, CAN_EFF_MASK};
use std::ffi::{c_char, c_long, c_ulong, c_void};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const PROTOCOL_CAN: c_ulong = 5;
pub const CAN_29BIT_ID: c_ulong = 0x100; // connect flag, TxFlags and RxStatus bit
pub const TX_MSG_TYPE: c_ulong = 0x01; // RxStatus: our own transmission echoed back
pub const PASS_FILTER: c_ulong = 1;
pub const IOCTL_READ_VBATT: c_ulong = 3;
pub const STATUS_NOERROR: c_long = 0;
pub const ERR_TIMEOUT: u32 = 0x09;
pub const ERR_BUFFER_EMPTY: u32 = 0x10;

pub fn error_name(code: u32) -> Option<&'static str> {
    Some(match code {
        0x01 => "ERR_NOT_SUPPORTED", 0x02 => "ERR_INVALID_CHANNEL_ID", 0x03 => "ERR_INVALID_PROTOCOL_ID", 0x04 => "ERR_NULL_PARAMETER",
        0x05 => "ERR_INVALID_IOCTL_VALUE", 0x06 => "ERR_INVALID_FLAGS", 0x07 => "ERR_FAILED", 0x08 => "ERR_DEVICE_NOT_CONNECTED",
        0x09 => "ERR_TIMEOUT", 0x0A => "ERR_INVALID_MSG", 0x0B => "ERR_INVALID_TIME_INTERVAL", 0x0C => "ERR_EXCEEDED_LIMIT",
        0x0D => "ERR_INVALID_MSG_ID", 0x0E => "ERR_DEVICE_IN_USE", 0x0F => "ERR_INVALID_IOCTL_ID", 0x10 => "ERR_BUFFER_EMPTY",
        0x11 => "ERR_BUFFER_FULL", 0x12 => "ERR_BUFFER_OVERFLOW", 0x13 => "ERR_PIN_INVALID", 0x14 => "ERR_CHANNEL_IN_USE",
        0x15 => "ERR_MSG_PROTOCOL_ID", 0x16 => "ERR_INVALID_FILTER_ID", 0x17 => "ERR_NO_FLOW_CONTROL", 0x18 => "ERR_NOT_UNIQUE",
        0x19 => "ERR_INVALID_BAUDRATE", 0x1A => "ERR_INVALID_DEVICE_ID",
        _ => return None,
    })
}

/// A failed PassThru call (Python's J2534Error, an OSError).
#[derive(Debug, Clone)]
pub struct J2534Error {
    pub code: u32,
    pub message: String,
}

impl J2534Error {
    pub fn new(call: &str, code: u32, text: &str) -> J2534Error {
        let name = error_name(code).map_or_else(|| format!("error 0x{code:02X}"), String::from);
        J2534Error { code, message: format!("{call}: {name}{}", if text.is_empty() { String::new() } else { format!(" {text}") }) }
    }

    pub fn other(message: impl Into<String>) -> J2534Error {
        J2534Error { code: 0, message: message.into() }
    }
}

impl std::fmt::Display for J2534Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for J2534Error {}

impl From<J2534Error> for io::Error {
    fn from(e: J2534Error) -> Self {
        io::Error::other(e)
    }
}

#[repr(C)]
pub struct PassThruMsg {
    pub protocol_id: c_ulong,
    pub rx_status: c_ulong,
    pub tx_flags: c_ulong,
    pub timestamp: c_ulong,
    pub data_size: c_ulong,
    pub extra_data_index: c_ulong,
    pub data: [u8; 4128],
}

impl PassThruMsg {
    pub fn empty() -> Box<PassThruMsg> {
        Box::new(PassThruMsg { protocol_id: 0, rx_status: 0, tx_flags: 0, timestamp: 0, data_size: 0, extra_data_index: 0, data: [0; 4128] })
    }

    pub fn can(can_id: u32, data: &[u8], flags: c_ulong) -> Box<PassThruMsg> {
        let mut m = PassThruMsg::empty();
        m.protocol_id = PROTOCOL_CAN;
        m.tx_flags = flags;
        let mut raw = can_id.to_be_bytes().to_vec();
        raw.extend(data);
        m.data[..raw.len()].copy_from_slice(&raw);
        m.data_size = raw.len() as c_ulong;
        m
    }
}

type Open = unsafe extern "C" fn(*const c_void, *mut c_ulong) -> c_long;
type Close = unsafe extern "C" fn(c_ulong) -> c_long;
type Connect = unsafe extern "C" fn(c_ulong, c_ulong, c_ulong, c_ulong, *mut c_ulong) -> c_long;
type Rw = unsafe extern "C" fn(c_ulong, *mut PassThruMsg, *mut c_ulong, c_ulong) -> c_long;
type StartFilter = unsafe extern "C" fn(c_ulong, c_ulong, *mut PassThruMsg, *mut PassThruMsg, *mut PassThruMsg, *mut c_ulong) -> c_long;
type StopFilter = unsafe extern "C" fn(c_ulong, c_ulong) -> c_long;
type Ioctl = unsafe extern "C" fn(c_ulong, c_ulong, *mut c_void, *mut c_void) -> c_long;
type ReadVersion = unsafe extern "C" fn(c_ulong, *mut c_char, *mut c_char, *mut c_char) -> c_long;
type LastError = unsafe extern "C" fn(*mut c_char) -> c_long;

/// The PassThru functions of one library; every call returns J2534Error on a non-zero status.
pub struct J2534Library {
    pub path: String,
    _lib: libloading::Library, // keeps the code mapped for the function pointers below
    open: Open,
    close: Close,
    connect: Connect,
    disconnect: Close,
    read: Rw,
    write: Rw,
    start_filter: StartFilter,
    stop_filter: StopFilter,
    ioctl: Ioctl,
    read_version: ReadVersion,
    last_error: LastError,
}

fn buf_text(b: &[c_char]) -> String {
    let bytes: Vec<u8> = b.iter().take_while(|c| **c != 0).map(|c| *c as u8).collect();
    bytes.iter().map(|x| if *x < 0x80 { *x as char } else { '\u{fffd}' }).collect()
}

impl J2534Library {
    pub fn load(path: &str) -> Result<J2534Library, J2534Error> {
        unsafe {
            let lib = libloading::Library::new(path).map_err(|e| J2534Error::other(format!("{path}: {e}")))?;
            macro_rules! sym {
                ($name:literal, $t:ty) => {
                    *lib.get::<$t>(concat!($name, "\0").as_bytes()).map_err(|e| J2534Error::other(format!("{path}: {e}")))?
                };
            }
            Ok(J2534Library {
                path: path.into(),
                open: sym!("PassThruOpen", Open),
                close: sym!("PassThruClose", Close),
                connect: sym!("PassThruConnect", Connect),
                disconnect: sym!("PassThruDisconnect", Close),
                read: sym!("PassThruReadMsgs", Rw),
                write: sym!("PassThruWriteMsgs", Rw),
                start_filter: sym!("PassThruStartMsgFilter", StartFilter),
                stop_filter: sym!("PassThruStopMsgFilter", StopFilter),
                ioctl: sym!("PassThruIoctl", Ioctl),
                read_version: sym!("PassThruReadVersion", ReadVersion),
                last_error: sym!("PassThruGetLastError", LastError),
                _lib: lib,
            })
        }
    }

    fn last_error(&self) -> String {
        let mut buf = [0 as c_char; 80];
        if unsafe { (self.last_error)(buf.as_mut_ptr()) } == STATUS_NOERROR { buf_text(&buf) } else { String::new() }
    }

    fn check(&self, name: &str, code: c_long) -> Result<(), J2534Error> {
        if code == STATUS_NOERROR { Ok(()) } else { Err(J2534Error::new(name, code as u32, &self.last_error())) }
    }

    pub fn open(&self, name: Option<&str>) -> Result<c_ulong, J2534Error> {
        let cname = name.map(|n| std::ffi::CString::new(n).unwrap_or_default());
        let mut dev: c_ulong = 0;
        let p = cname.as_ref().map_or(std::ptr::null(), |c| c.as_ptr() as *const c_void);
        self.check("PassThruOpen", unsafe { (self.open)(p, &mut dev) })?;
        Ok(dev)
    }

    pub fn close(&self, dev: c_ulong) -> Result<(), J2534Error> {
        self.check("PassThruClose", unsafe { (self.close)(dev) })
    }

    pub fn connect(&self, dev: c_ulong, protocol: c_ulong, flags: c_ulong, baud: c_ulong) -> Result<c_ulong, J2534Error> {
        let mut ch: c_ulong = 0;
        self.check("PassThruConnect", unsafe { (self.connect)(dev, protocol, flags, baud, &mut ch) })?;
        Ok(ch)
    }

    pub fn disconnect(&self, ch: c_ulong) -> Result<(), J2534Error> {
        self.check("PassThruDisconnect", unsafe { (self.disconnect)(ch) })
    }

    /// Reads up to one message; returns how many arrived (0 or 1).
    pub fn read_msgs(&self, ch: c_ulong, msg: &mut PassThruMsg, timeout_ms: c_ulong) -> Result<c_ulong, J2534Error> {
        let mut n: c_ulong = 1;
        self.check("PassThruReadMsgs", unsafe { (self.read)(ch, msg, &mut n, timeout_ms) })?;
        Ok(n)
    }

    pub fn write_msgs(&self, ch: c_ulong, msg: &mut PassThruMsg, timeout_ms: c_ulong) -> Result<(), J2534Error> {
        let mut n: c_ulong = 1;
        self.check("PassThruWriteMsgs", unsafe { (self.write)(ch, msg, &mut n, timeout_ms) })
    }

    pub fn start_msg_filter(&self, ch: c_ulong, kind: c_ulong, mask: &mut PassThruMsg, pattern: &mut PassThruMsg) -> Result<c_ulong, J2534Error> {
        let mut id: c_ulong = 0;
        self.check("PassThruStartMsgFilter", unsafe { (self.start_filter)(ch, kind, mask, pattern, std::ptr::null_mut(), &mut id) })?;
        Ok(id)
    }

    pub fn stop_msg_filter(&self, ch: c_ulong, id: c_ulong) -> Result<(), J2534Error> {
        self.check("PassThruStopMsgFilter", unsafe { (self.stop_filter)(ch, id) })
    }

    pub fn read_version(&self, dev: c_ulong) -> Result<(String, String, String), J2534Error> {
        let (mut fw, mut dll, mut api) = ([0 as c_char; 80], [0 as c_char; 80], [0 as c_char; 80]);
        self.check("PassThruReadVersion", unsafe { (self.read_version)(dev, fw.as_mut_ptr(), dll.as_mut_ptr(), api.as_mut_ptr()) })?;
        Ok((buf_text(&fw), buf_text(&dll), buf_text(&api)))
    }

    pub fn read_vbatt(&self, ch: c_ulong) -> Result<c_ulong, J2534Error> {
        let mut mv: c_ulong = 0;
        self.check("PassThruIoctl", unsafe { (self.ioctl)(ch, IOCTL_READ_VBATT, std::ptr::null_mut(), &mut mv as *mut c_ulong as *mut c_void) })?;
        Ok(mv)
    }
}

/// Where the GD101 driver's C source is.
pub fn gd101_source() -> PathBuf {
    if let Ok(p) = std::env::var("HONDASCAN_GD101_SRC") {
        return p.into();
    }
    crate::resource("drivers/gd101_j2534.c")
}

/// The GD101 driver built from gd101_j2534.c with the system C compiler, cached until the source changes.
pub fn gd101_library() -> Result<PathBuf, J2534Error> {
    let src = gd101_source();
    let cache = std::env::var("XDG_CACHE_HOME").ok().filter(|s| !s.is_empty()).map(PathBuf::from).unwrap_or_else(|| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".cache")).join("hondascan");
    let lib = cache.join("libgd101_j2534.so");
    let mtime = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    if !lib.exists() || mtime(&lib) < mtime(&src) {
        let cc = ["cc", "gcc", "clang"].iter().find(|c| which(c).is_some()).ok_or_else(|| J2534Error::other("the GD101 J2534 driver needs a C compiler (cc, gcc or clang) to build once"))?;
        std::fs::create_dir_all(&cache).map_err(|e| J2534Error::other(e.to_string()))?;
        let tmp = cache.join(format!("libgd101_j2534.so.{}.tmp", std::process::id()));
        let st = std::process::Command::new(cc)
            .args(["-shared", "-fPIC", "-O2", "-fstack-protector-strong", "-D_FORTIFY_SOURCE=2", "-Wl,-z,relro,-z,now", "-o"])
            .arg(&tmp)
            .arg(&src)
            .status()
            .map_err(|e| J2534Error::other(e.to_string()))?;
        if !st.success() {
            return Err(J2534Error::other(format!("building the GD101 driver failed ({st})")));
        }
        std::fs::rename(&tmp, &lib).map_err(|e| J2534Error::other(e.to_string()))?;
    }
    Ok(lib)
}

fn which(cmd: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|p| std::env::split_paths(&p).map(|d| d.join(cmd)).find(|f| f.is_file()))
}

/// One raw CAN channel on a J2534 device. Receive filtering is the device's pass filter on 0x18DAF1xx (HDS's own mailbox) plus
/// the per-id selection every CanBus does.
pub struct J2534CanBus {
    core: BusCore,
    pub api: J2534Library,
    device: c_ulong,
    channel: c_ulong,
    /// A hard read error (not a timeout): Python raises it from recv; here the next send reports it.
    pending_error: Mutex<Option<J2534Error>>,
}

impl J2534CanBus {
    pub fn open(library: &str, bitrate: u32, log: Option<Arc<CaptureLog>>, name: Option<&str>) -> Result<J2534CanBus, J2534Error> {
        let api = J2534Library::load(library)?;
        let device = api.open(name)?;
        let r = (|| -> Result<c_ulong, J2534Error> {
            let channel = api.connect(device, PROTOCOL_CAN, CAN_29BIT_ID, bitrate as c_ulong)?;
            let (mut mask, mut pattern) = (PassThruMsg::can(0x1FFFFF00, &[], CAN_29BIT_ID), PassThruMsg::can(0x18DAF100, &[], CAN_29BIT_ID));
            api.start_msg_filter(channel, PASS_FILTER, &mut mask, &mut pattern)?;
            Ok(channel)
        })();
        match r {
            Ok(channel) => Ok(J2534CanBus { core: BusCore::with_log(log), api, device, channel, pending_error: Mutex::new(None) }),
            Err(e) => {
                let _ = api.close(device);
                Err(e)
            }
        }
    }

    pub fn version(&self) -> Result<(String, String, String), J2534Error> {
        self.api.read_version(self.device)
    }

    pub fn voltage(&self) -> Result<f64, J2534Error> {
        Ok(self.api.read_vbatt(self.channel)? as f64 / 1000.0)
    }
}

impl CanBus for J2534CanBus {
    fn core(&self) -> &BusCore {
        &self.core
    }

    fn send_raw(&self, frame: &CanFrame) -> io::Result<()> {
        if let Some(e) = self.pending_error.lock().unwrap().take() {
            return Err(e.into());
        }
        let mut msg = PassThruMsg::can(frame.can_id & CAN_EFF_MASK, &frame.data, if frame.extended { CAN_29BIT_ID } else { 0 });
        self.api.write_msgs(self.channel, &mut msg, 100).map_err(Into::into)
    }

    fn recv_raw(&self, timeout: f64) -> Option<CanFrame> {
        let end = Instant::now() + Duration::from_secs_f64(timeout.max(0.0));
        let mut msg = PassThruMsg::empty();
        loop {
            let left = end.saturating_duration_since(Instant::now()).as_secs_f64();
            if left <= 0.0 {
                return None;
            }
            let n = match self.api.read_msgs(self.channel, &mut msg, ((left * 1000.0) as c_ulong).max(1)) {
                Ok(n) => n,
                Err(e) if e.code == ERR_TIMEOUT || e.code == ERR_BUFFER_EMPTY => continue,
                Err(e) => {
                    *self.pending_error.lock().unwrap() = Some(e);
                    return None;
                }
            };
            if n == 0 || msg.rx_status & TX_MSG_TYPE != 0 || msg.data_size < 4 {
                continue;
            }
            let size = (msg.data_size as usize).min(msg.data.len());
            let raw = &msg.data[..size];
            let can_id = u32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]]);
            if self.ids().is_empty() || self.ids().contains(&can_id) {
                return Some(CanFrame { can_id, data: raw[4..size.min(12)].to_vec(), extended: msg.rx_status & CAN_29BIT_ID != 0, timestamp: now() });
            }
        }
    }

    fn describe(&self) -> Option<String> {
        let (fw, dll, api) = self.version().ok()?;
        Some(format!("J2534 {}: firmware {fw}, library {dll}, API {api}; battery {} V", self.api.path, crate::journal::py(&serde_json::json!(self.voltage().ok()?))))
    }

    fn close(&self) {
        let _ = self.api.disconnect(self.channel);
        let _ = self.api.close(self.device);
    }
}
