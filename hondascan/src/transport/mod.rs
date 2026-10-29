//! Adapters: what the diagnostic layers talk to. An `Adapter` gives `link(tx_id, rx_id)` (an ISO-TP-or-equivalent
//! channel to one ECU), optionally a `probe` (many single-frame requests, one collection window) and a `broadcast` (one
//! functional request, collect the answers), plus `describe` and `close`. Port of hondascan/transport/__init__.py;
//! `open_adapter` follows as the back ends are ported.
pub mod can;
pub mod diagh_serial;
pub mod elm327;
pub mod elm_kline;
pub mod elm_link;
pub mod isotp;
pub mod j2534;
pub mod ports;
pub mod replay;
pub mod serial;
#[cfg(target_os = "linux")]
pub mod socketcan;

use can::{secs, CanBus, CanFrame};
use isotp::{DiagLink, IsoTpError, IsoTpLink, LinkError, HDS_PAD};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Instant;

pub trait Adapter: Send + Sync {
    fn link(&self, tx_id: u32, rx_id: u32) -> Box<dyn DiagLink>;
    fn describe(&self) -> String;
    fn close(&self);

    /// The simulated vehicle behind this adapter, if it is one (tests reach its ECUs through it).
    fn sim(&self) -> Option<&crate::sim::SimulatedVehicle> {
        None
    }

    /// The raw CAN bus behind this adapter, if it is one (the probes listen on it directly).
    fn bus(&self) -> Option<Arc<dyn CanBus>> {
        None
    }

    /// The K-line adapter behind this one, if it is one (the `kwp` commands need it).
    fn as_kline(&self) -> Option<&elm_kline::ElmKLine> {
        None
    }

    /// The serial device a serial adapter was opened on (for the ModemManager warning), if it has one.
    fn path(&self) -> Option<String> {
        None
    }

    /// Python's `hasattr(adapter, 'probe')`.
    fn has_probe(&self) -> bool {
        false
    }

    /// Send the single-frame `payload` on every (tx_id, rx_id) pair back to back and return the rx ids that answered
    /// with a single frame within `timeout` of the last request.
    fn probe(&self, _pairs: &[(u32, u32)], _payload: &[u8], _timeout: f64) -> Result<BTreeSet<u32>, LinkError> {
        Err(IsoTpError("this adapter cannot probe".into()).into())
    }

    /// Python's `hasattr(adapter, 'broadcast')`.
    fn has_broadcast(&self) -> bool {
        false
    }

    /// Send the single-frame `payload` once on `tx_id` (a functional id) and collect the single-frame answers on `rx_ids`
    /// for `timeout` seconds: {rx id: seconds after the request, or None where the adapter does not time each answer}.
    fn broadcast(&self, _tx_id: u32, _rx_ids: &[u32], _payload: &[u8], _timeout: f64) -> Result<BTreeMap<u32, Option<f64>>, LinkError> {
        Err(IsoTpError("this adapter cannot broadcast".into()).into())
    }
}

/// Any raw CanBus + hondascan's own ISO-TP.
pub struct RawCanAdapter {
    pub bus: Arc<dyn CanBus>,
    pub pad: Option<u8>,
    pub kind: &'static str, // the bus's type name, for describe()
}

impl RawCanAdapter {
    pub fn new(bus: Arc<dyn CanBus>, kind: &'static str) -> RawCanAdapter {
        RawCanAdapter { bus, pad: Some(HDS_PAD), kind }
    }

    /// A link whose flow control arrives on `fc_id` (the simulated ECUs answer a functional request physically).
    pub fn link_with_fc(&self, tx_id: u32, rx_id: u32, fc_id: Option<u32>) -> IsoTpLink {
        let mut l = IsoTpLink::with(self.bus.clone(), tx_id, rx_id, fc_id);
        l.pad = self.pad;
        l
    }

    fn ask(&self, txs: &[u32], want: &BTreeSet<u32>, payload: &[u8], timeout: f64) -> Result<BTreeMap<u32, f64>, LinkError> {
        if payload.len() > 7 {
            return Err(IsoTpError("probe payload must fit a single frame".into()).into());
        }
        for rx in want {
            self.bus.listen(*rx);
        }
        let mut data = vec![payload.len() as u8];
        data.extend(payload);
        if let Some(p) = self.pad {
            data.resize(data.len().max(8), p);
        }
        let t0 = Instant::now();
        for tx in txs {
            self.bus.send(&CanFrame::new(*tx, data.clone()))?;
        }
        let mut found = BTreeMap::new();
        let end = Instant::now() + secs(timeout);
        loop {
            let Some(f) = self.bus.recv(end.saturating_duration_since(Instant::now()).as_secs_f64()) else { return Ok(found) };
            if want.contains(&f.can_id) && !f.data.is_empty() && f.data[0] >> 4 == 0 && (1..f.data.len() as u8).contains(&(f.data[0] & 0xF)) {
                found.entry(f.can_id).or_insert_with(|| t0.elapsed().as_secs_f64());
            }
        }
    }
}

impl Adapter for RawCanAdapter {
    fn bus(&self) -> Option<Arc<dyn CanBus>> {
        Some(self.bus.clone())
    }

    fn link(&self, tx_id: u32, rx_id: u32) -> Box<dyn DiagLink> {
        Box::new(self.link_with_fc(tx_id, rx_id, None))
    }

    /// Raw CAN delivers every reply id, so probing many addresses costs one timeout, not one each (HDS's VCI reassembles
    /// up to 10 answers at once the same way, see README.md "Wire layer").
    fn has_probe(&self) -> bool {
        true
    }

    fn probe(&self, pairs: &[(u32, u32)], payload: &[u8], timeout: f64) -> Result<BTreeSet<u32>, LinkError> {
        let txs: Vec<u32> = pairs.iter().map(|p| p.0).collect();
        let want: BTreeSet<u32> = pairs.iter().map(|p| p.1).collect();
        Ok(self.ask(&txs, &want, payload, timeout)?.into_keys().collect())
    }

    fn has_broadcast(&self) -> bool {
        true
    }

    fn broadcast(&self, tx_id: u32, rx_ids: &[u32], payload: &[u8], timeout: f64) -> Result<BTreeMap<u32, Option<f64>>, LinkError> {
        let want: BTreeSet<u32> = rx_ids.iter().cloned().collect();
        Ok(self.ask(&[tx_id], &want, payload, timeout)?.into_iter().map(|(k, v)| (k, Some(v))).collect())
    }

    fn describe(&self) -> String {
        format!("{} (raw CAN, hondascan ISO-TP)", self.kind)
    }

    fn close(&self) {
        self.bus.close();
    }
}

// -- finding and opening adapters ----------------------------------------------------------------------------------

/// What `open_adapter` fails with: the Python exception class name and its message.
#[derive(Debug, Clone)]
pub struct OpenError {
    pub kind: &'static str,
    pub message: String,
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

pub use ports::{modem_manager_warning, serial_ports};

pub const GD101_USB_ID: (&str, &str) = ("e327", "2534");

pub fn find_port(usb_id: Option<(&str, &str)>) -> Result<String, OpenError> {
    let mut ports = serial_ports();
    match usb_id {
        Some((v, p)) => ports.retain(|x| x.1 == v && x.2 == p),
        None => ports.sort_by_key(|x| (x.1.as_str(), x.2.as_str()) != GD101_USB_ID),
    }
    ports.into_iter().next().map(|p| p.0).ok_or_else(|| OpenError {
        kind: "FileNotFoundError",
        message: format!(
            "no {} found; plug it in, or give the port, e.g. elm:{}",
            if usb_id.is_some() { "GODIAG GD101 (USB e327:2534)" } else { "USB serial adapter" },
            if cfg!(windows) { "COM5" } else { "/dev/ttyACM0" }
        ),
    })
}

/// `socketcan:IFACE`, `slcan:PORT[@BITRATE]`, `sim[:PROFILE]` and (as they are ported) `elm`, `gd101`, `j2534`, `gd101-j2534`,
/// `elm-kline`, `elm-obd9141`, `elm-obd` (tries each K-line protocol), `replay:FILE`.
pub fn open_adapter(spec: &str, log: Option<Arc<can::CaptureLog>>) -> Result<Arc<dyn Adapter>, OpenError> {
    let (kind, arg) = spec.split_once(':').unwrap_or((spec, ""));
    let (path, num) = arg.split_once('@').unwrap_or((arg, ""));
    let io = |e: std::io::Error| OpenError { kind: "OSError", message: e.to_string() };
    match kind {
        #[cfg(target_os = "linux")]
        "socketcan" => {
            let bus = socketcan::SocketCanBus::open(if path.is_empty() { "can0" } else { path }, log).map_err(io)?;
            Ok(Arc::new(RawCanAdapter::new(Arc::new(bus), "SocketCanBus")))
        }
        #[cfg(not(target_os = "linux"))]
        "socketcan" => Err(OpenError { kind: "ValueError", message: "socketcan is Linux only; use slcan:, j2534: or an ELM327 adapter".into() }),
        "slcan" => {
            let rate = if num.is_empty() { 500000 } else { num.parse().map_err(|_| OpenError { kind: "ValueError", message: format!("bad bitrate {num:?}") })? };
            let bus = can::SlcanBus::open(path, rate, 115200, log).map_err(io)?;
            Ok(Arc::new(RawCanAdapter::new(Arc::new(bus), "SlcanBus")))
        }
        "sim" => {
            let profile = if arg.is_empty() { "accord2015" } else { arg };
            crate::sim::SimulatedVehicle::new(profile, log, None).map(|v| Arc::new(v) as Arc<dyn Adapter>).map_err(|m| OpenError { kind: "ValueError", message: m })
        }
        "elm" | "gd101" => {
            let port = if path.is_empty() { find_port(if kind == "gd101" { Some(GD101_USB_ID) } else { None })? } else { path.to_string() };
            if let (Some(w), Some(l)) = (modem_manager_warning(&port), &log) {
                l.note(&format!("ELM! {w}"));
            }
            let baud = if num.is_empty() { None } else { Some(num.parse().map_err(|_| OpenError { kind: "ValueError", message: format!("bad baud {num:?}") })?) };
            elm327::open_elm(&port, baud, log).map_err(|e| OpenError { kind: "ElmError", message: e.to_string() })
        }
        "elm-kline" | "elm-kline5" | "elm-obd9141" | "gd101-obd9141" | "elm-obd" | "gd101-obd" => {
            let port = if path.is_empty() { find_port(if kind.starts_with("gd101") { Some(GD101_USB_ID) } else { None })? } else { path.to_string() };
            let baud = if num.is_empty() { None } else { Some(num.parse().map_err(|_| OpenError { kind: "ValueError", message: format!("bad baud {num:?}") })?) };
            if kind.ends_with("-obd") {
                return elm_kline::ElmKLine::open_obd_auto(&port, baud, log).map(|a| Arc::new(a) as Arc<dyn Adapter>).map_err(|e| OpenError { kind: "ElmError", message: e.to_string() });
            }
            let proto = match kind {
                "elm-kline" => elm327::KProto::Fast,
                "elm-kline5" => elm327::KProto::Slow,
                _ => elm327::KProto::Iso9141,
            };
            elm_kline::ElmKLine::open(&port, baud, log, proto).map(|a| Arc::new(a) as Arc<dyn Adapter>).map_err(|e| OpenError { kind: "ElmError", message: e.to_string() })
        }
        "j2534" => {
            let lib = if arg.is_empty() { std::env::var("HONDASCAN_J2534").unwrap_or_default() } else { arg.to_string() };
            if lib.is_empty() {
                let installed: Vec<String> = j2534::registered_libraries().into_iter().map(|(n, _)| n).collect();
                let message = if installed.is_empty() {
                    r"j2534 needs the library path: j2534:/path/to/libj2534.so (j2534:C:\path	o\device.dll on Windows), or set HONDASCAN_J2534".into()
                } else {
                    format!("j2534 needs the library path or the name of an installed device: j2534:NAME, one of {}", installed.join(", "))
                };
                return Err(OpenError { kind: "ValueError", message });
            }
            let bus = j2534::J2534CanBus::open(&j2534::resolve_library(&lib), 500000, log, None).map_err(|e| OpenError { kind: "J2534Error", message: e.to_string() })?;
            Ok(Arc::new(RawCanAdapter::new(Arc::new(bus), "J2534CanBus")))
        }
        "gd101-j2534" => {
            let lib = j2534::gd101_library().map_err(|e| OpenError { kind: "J2534Error", message: e.to_string() })?;
            // the Linux driver is given the serial port; the maker's Windows DLL finds the device itself
            let name = if cfg!(windows) { None } else if !arg.is_empty() { Some(arg.to_string()) } else { Some(find_port(Some(GD101_USB_ID))?) };
            let bus = j2534::J2534CanBus::open(&lib.to_string_lossy(), 500000, log, name.as_deref()).map_err(|e| OpenError { kind: "J2534Error", message: e.to_string() })?;
            Ok(Arc::new(RawCanAdapter::new(Arc::new(bus), "J2534CanBus")))
        }
        "replay" => {
            let bus = replay::ReplayBus::open(arg, log).map_err(io)?;
            Ok(Arc::new(RawCanAdapter::new(Arc::new(bus), "ReplayBus")))
        }
        _ => Err(OpenError {
            kind: "ValueError",
            message: format!("unknown adapter {spec:?} (socketcan:, slcan:, j2534:, elm:, elm-kline:, elm-kline5:, elm-obd9141:, gd101-obd9141, elm-obd:, gd101-obd, gd101, gd101-j2534, sim, replay:)"),
        }),
    }
}
