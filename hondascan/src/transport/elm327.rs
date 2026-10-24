//! ELM327-compatible adapters: ELM327 and clones, STN11xx/STN21xx (OBDLink), and adapters with an ELM mode such as the GODIAG
//! GD101 (USB 0xe327:0x2534, appears as /dev/ttyACM*). Port of hondascan/transport/elm327.py, whose docstring explains the
//! protocol (protocol 7, ISO 15765-4, 29-bit ids, 500 kbit/s; the adapter does the ISO-TP segmentation; flow control set to
//! HDS's `30 00 00`) and the faults seen live on the GD101 (a reply that belongs to an earlier command, a banner in a reply, a
//! command with no prompt) that are handled here: the stale reply is dropped and the command asked again, a reboot sets the
//! adapter up again and repeats the addressing and the request. Both leave a `# ELM!` line in the log.
use super::can::{secs, CaptureLog, SerialPort};
use super::elm_link::{exchange, frames, request, Elm327Link, ElmErr};
use super::isotp::{reply_to, DiagLink, IsoTpError, LinkError};
use super::Adapter;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const BAUDS: [u32; 8] = [38400, 115200, 500000, 230400, 57600, 9600, 2000000, 1000000];
pub const RETRIES: usize = 3; // asks / setups per request before giving up
pub const DRAIN_S: f64 = 0.5; // unsolicited output for longer than this: still monitoring
pub const MAX_RESETS: u32 = 12; // per session: an adapter that keeps rebooting is a power fault
const BANNERS: [&str; 2] = ["ELM", "STN"];
pub const DEFAULT_PRIORITY: u32 = 0x18; // the ELM327's 29-bit priority after ATZ / ATD (ATCP default)
pub const POWER_UP_HEADER: u32 = 0x18DB33F1; // ATSP7's header after ATZ: the OBD-II functional id
const SETUP: [&str; 7] = ["ATE0", "ATL0", "ATS0", "ATH1", "ATSP7", "ATCAF1", "ATAT1"];

/// Python's `repr()` of a str, for the error texts.
pub fn str_repr(s: &str) -> String {
    let q = if s.contains('\'') && !s.contains('"') { '"' } else { '\'' };
    let mut o = String::from(q);
    for c in s.chars() {
        match c {
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if c == q => {
                o.push('\\');
                o.push(c)
            }
            c if (c as u32) < 0x20 || c as u32 == 0x7f => o.push_str(&format!("\\x{:02x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push(q);
    o
}

/// Python's `repr()` of bytes.
pub fn bytes_repr(b: &[u8]) -> String {
    let q = if b.contains(&b'\'') && !b.contains(&b'"') { '"' } else { '\'' };
    let mut o = format!("b{q}");
    for &x in b {
        match x {
            b'\\' => o.push_str("\\\\"),
            b'\n' => o.push_str("\\n"),
            b'\r' => o.push_str("\\r"),
            b'\t' => o.push_str("\\t"),
            x if x as char == q => {
                o.push('\\');
                o.push(x as char)
            }
            0x20..=0x7e => o.push(x as char),
            x => o.push_str(&format!("\\x{x:02x}")),
        }
    }
    o.push(q);
    o
}

/// Which K-line protocol the adapter is set to: ISO 14230-4 with fast init (ATSP5) or 5-baud init (ATSP4), or ISO 9141-2 (ATSP3),
/// the generic OBD-II K-line of cars before about 2008.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum KProto {
    Fast,
    Slow,
    Iso9141,
}

impl KProto {
    pub fn atsp(self) -> &'static str {
        match self {
            KProto::Fast => "ATSP5",
            KProto::Slow => "ATSP4",
            KProto::Iso9141 => "ATSP3",
        }
    }
}

/// The K-line side of an ELM327 (transport/elm_kline.rs).
pub struct KLineState {
    pub proto: KProto,
    pub addressed: Option<(u32, u32, bool)>,
    pub started: bool,
}

/// All the adapter's state; every operation runs under the Elm327's lock.
pub struct Inner {
    pub path: String,
    pub log: Option<Arc<CaptureLog>>,
    want_baud: Option<u32>,
    pub timeout: f64,
    resets: u32,
    ready: bool,
    port: Option<SerialPort>,
    pub baud: u32,
    header: Option<(u32, u32)>,
    fc_header: Option<u32>,
    custom_header: bool, // an ATSH since the last ATZ
    priority: u32,
    pub ident: String,
    pub hds_flow_control: bool,
    /// Some: the K-line instead of CAN
    pub kline: Option<KLineState>,
}

pub struct Elm327 {
    inner: Mutex<Inner>,
}

fn sleep(s: f64) {
    std::thread::sleep(Duration::from_secs_f64(s));
}

impl Elm327 {
    pub fn open(port: &str, baud: Option<u32>, timeout_ms: u32, log: Option<Arc<CaptureLog>>) -> Result<Arc<Elm327>, ElmErr> {
        Elm327::open_mode(port, baud, timeout_ms, log, None)
    }

    /// `kline`: Some(protocol) for a K-line adapter instead of a CAN one.
    pub fn open_mode(port: &str, baud: Option<u32>, timeout_ms: u32, log: Option<Arc<CaptureLog>>, kline: Option<KProto>) -> Result<Arc<Elm327>, ElmErr> {
        if let Some(b) = baud {
            if !SerialPort::supported_baud(b) {
                return Err(ElmErr::error(format!("unsupported baud {b} (one of {})", SerialPort::bauds().iter().map(u32::to_string).collect::<Vec<_>>().join(", "))));
            }
        }
        let mut inn = Inner {
            path: port.into(), log, want_baud: baud, timeout: timeout_ms as f64 / 1000.0, resets: 0, ready: false, port: None, baud: 0, header: None, fc_header: None,
            custom_header: false, priority: DEFAULT_PRIORITY, ident: String::new(), hds_flow_control: false,
            kline: kline.map(|proto| KLineState { proto, addressed: None, started: false }),
        };
        inn.connect(false)?;
        Ok(Arc::new(Elm327 { inner: Mutex::new(inn) }))
    }

    pub fn with<T>(&self, f: impl FnOnce(&mut Inner) -> T) -> T {
        f(&mut self.inner.lock().unwrap())
    }

    pub fn voltage(&self) -> Option<f64> {
        self.with(|i| i.voltage())
    }

    pub fn timeout(&self) -> f64 {
        self.with(|i| i.timeout)
    }
}

impl Inner {
    /// Open the port and set the adapter up; again if it does not answer (just powered, re-enumerating, rebooting). The first
    /// open of a missing device fails at once; after a fault we wait for it to return.
    pub fn connect(&mut self, patient: bool) -> Result<(), ElmErr> {
        for attempt in 0..RETRIES {
            let mut opened = false;
            let r = (|| -> Result<(), ElmErr> {
                if let Some(p) = self.port.take() {
                    p.close();
                }
                let (p, b) = self.open_port()?;
                self.port = Some(p);
                self.baud = b;
                opened = true;
                self.init()
            })();
            match r {
                Ok(()) => return Ok(()),
                Err(e) => {
                    if let Some(p) = self.port.take() {
                        p.close();
                    }
                    if attempt == RETRIES - 1 || !(opened || patient) {
                        return Err(e);
                    }
                    self.note(&format!("setup failed ({e}); trying again"));
                    sleep(1.0);
                }
            }
        }
        Ok(())
    }

    pub fn init(&mut self) -> Result<(), ElmErr> {
        if self.kline.is_some() {
            return self.init_kline();
        }
        self.header = None;
        self.fc_header = None;
        self.custom_header = false; // an ATSH since the last ATZ
        self.priority = DEFAULT_PRIORITY; // ATZ sets it: no ATCP for Honda ids (see send_header)
        self.ready = false; // during setup a stray banner is stale, not a reset
        let reset = self.cmd("ATZ", 3.0)?;
        self.ident = reset.lines().last().unwrap_or("").to_string();
        sleep(0.3); // the GD101 can print a second banner after the prompt
        self.drain()?;
        for c in SETUP {
            self.must(c)?;
        }
        self.set_timeout((self.timeout * 1000.0) as u32)?;
        let ati = self.cmd("ATI", 2.0)?;
        if !ati.is_empty() {
            self.ident = ati;
        }
        self.hds_flow_control = self.setup_hds_flow_control()?;
        self.ready = true;
        Ok(())
    }

    /// As init, with the K-line setup instead of the CAN one: no flow control, no ids.
    fn init_kline(&mut self) -> Result<(), ElmErr> {
        self.ready = false;
        if let Some(k) = self.kline.as_mut() {
            (k.addressed, k.started) = (None, false);
        }
        let proto = self.kline.as_ref().map_or(KProto::Fast, |k| k.proto);
        let reset = self.cmd("ATZ", 3.0)?;
        self.ident = reset.lines().last().unwrap_or("").to_string();
        sleep(0.3);
        self.drain()?;
        for c in ["ATE0", "ATL0", "ATS0", "ATH1", "ATAT1", proto.atsp()] {
            self.must(c)?;
        }
        self.set_timeout((self.timeout * 1000.0) as u32)?;
        let ati = self.cmd("ATI", 2.0)?;
        if !ati.is_empty() {
            self.ident = ati;
        }
        self.ready = true;
        Ok(())
    }

    pub fn note(&self, text: &str) {
        if let Some(l) = &self.log {
            l.note(&format!("ELM! {text}"));
        }
    }

    /// The adapter rebooted or vanished: set it up again (all addressing state is gone).
    pub fn recover(&mut self, fault: &ElmErr) -> Result<(), ElmErr> {
        self.resets += 1;
        if self.resets > MAX_RESETS {
            return Err(ElmErr::error(format!("adapter reset {} times (check the supply voltage and the cable): {fault}", self.resets)));
        }
        self.note(&format!("{fault}; setting the adapter up again"));
        self.connect(true)
    }

    /// Run `f`, a whole addressing + request sequence; if the adapter resets meanwhile, set it up again and run it again from
    /// the start.
    pub fn retrying<T>(&mut self, f: &mut dyn FnMut(&mut Inner) -> Result<T, ElmErr>) -> Result<T, ElmErr> {
        for _ in 0..RETRIES {
            match f(self) {
                Err(e @ ElmErr::Reset { .. }) => self.recover(&e)?,
                other => return other,
            }
        }
        Err(ElmErr::error(format!("adapter reset {RETRIES} times in a row")))
    }

    /// FCSM1 is refused until FCSH and FCSD are defined, so define them first (FCSH is a placeholder here, set per target in
    /// select()).
    fn setup_hds_flow_control(&mut self) -> Result<bool, ElmErr> {
        for c in ["ATFCSH18DA00F1", "ATFCSD300000", "ATFCSM1"] {
            if self.cmd(c, 2.0)?.contains('?') {
                self.cmd("ATFCSM0", 2.0)?;
                return Ok(false);
            }
        }
        Ok(true)
    }

    // -- serial ------------------------------------------------------------------------------------------------------------
    /// Open at `baud`, or find the speed the adapter answers at. USB CDC adapters (GD101, most STN devices) answer at any setting.
    fn open_port(&self) -> Result<(SerialPort, u32), ElmErr> {
        let mut last: Option<String> = None;
        let speeds: Vec<u32> = match self.want_baud {
            Some(b) => vec![b],
            None => BAUDS.to_vec(),
        };
        for b in speeds {
            let p = match SerialPort::open(&self.path, b) {
                Ok(p) => p,
                Err(e) => {
                    if e.raw_os_error() == Some(libc::EBUSY) {
                        return Err(ElmErr::error(format!("{} is in use by another program", self.path))); // not a matter of speed
                    }
                    last = Some(e.to_string());
                    continue;
                }
            };
            let probe = (|| -> std::io::Result<Vec<u8>> {
                p.write(b"\r", super::can::WRITE_TIMEOUT_S)?;
                sleep(0.05);
                p.read(0.1);
                p.write(b"ATI\r", super::can::WRITE_TIMEOUT_S)?;
                let mut got: Vec<u8> = vec![];
                let end = Instant::now() + secs(0.6);
                while !got.contains(&b'>') && Instant::now() < end {
                    got.extend(p.read(end.saturating_duration_since(Instant::now()).as_secs_f64()));
                    if p.lost() {
                        break; // the port is gone: no prompt is coming, and spinning to the deadline wastes the probe
                    }
                }
                Ok(got)
            })();
            let got = match probe {
                Ok(g) => g,
                Err(e) => {
                    p.close();
                    return Err(ElmErr::error(format!("{}: {e}", self.path))); // wedged: takes nothing (write timeout)
                }
            };
            let up = got.to_ascii_uppercase();
            let has = |n: &[u8]| up.windows(n.len()).any(|w| w == n);
            if got.contains(&b'>') && (has(b"ELM") || has(b"STN") || got.windows(2).any(|w| w == b"OK") || got.contains(&b'?')) {
                return Ok((p, b));
            }
            p.close();
            last = Some(format!("no ELM327 prompt at {b} baud (got {})", bytes_repr(&got)));
        }
        Err(ElmErr::error(format!("{}: no ELM327-compatible adapter found ({})", self.path, last.unwrap_or_else(|| "None".into()))))
    }

    /// Raw access to the serial port, for the addressing experiments (ATMA monitoring has no prompt).
    pub fn raw_write(&self, data: &[u8]) {
        let _ = self.port().write(data, super::can::WRITE_TIMEOUT_S);
    }

    pub fn raw_read(&self, timeout: f64) -> Vec<u8> {
        self.port().read(timeout)
    }

    fn port(&self) -> &SerialPort {
        self.port.as_ref().expect("the port is open")
    }

    /// Throw away what the adapter sent that nobody asked for. An adapter that keeps sending is still monitoring (ATMA): any
    /// character stops that, so send one, a few times, rather than read forever.
    fn drain(&mut self) -> Result<(), ElmErr> {
        let mut stale: Vec<u8> = vec![];
        for _ in 0..RETRIES {
            let end = Instant::now() + secs(DRAIN_S);
            while Instant::now() < end {
                let more = self.port().read(0.0);
                if more.is_empty() {
                    if !stale.is_empty() {
                        let shown = &stale[..stale.len().min(200)];
                        self.note(&format!("dropped unsolicited {} ({} bytes)", bytes_repr(shown), stale.len()));
                    }
                    return Ok(());
                }
                stale.extend(more);
            }
            self.note("adapter keeps sending (still monitoring?): stopping it");
            let _ = self.port().write(b"\r", super::can::WRITE_TIMEOUT_S);
            sleep(0.2);
        }
        Err(ElmErr::error(format!("{}: adapter does not stop sending; unplug it and plug it in again", self.path)))
    }

    /// One line to the adapter, its reply back. A reply that cannot belong to `text` is from an earlier command (the stream is
    /// one behind): drop what is pending and ask again. A banner or a returned echo means the adapter rebooted: ElmReset.
    pub fn cmd(&mut self, text: &str, timeout: f64) -> Result<String, ElmErr> {
        let mut out = String::new();
        for _ in 0..RETRIES {
            self.drain()?;
            let (o, echoed) = self.line(text, timeout)?;
            out = o;
            match self.mismatch(text, &out, echoed) {
                None => return Ok(out),
                Some("reset") if self.ready => {
                    return Err(ElmErr::Reset { msg: format!("adapter rebooted (answered {} with {})", str_repr(text), str_repr(&out)), lost: false });
                }
                Some(_) => {
                    self.note(&format!("{} answered with {}: asking again", str_repr(text), str_repr(&out)));
                    sleep(self.timeout + 0.2); // the late real reply is dropped by the next drain
                }
            }
        }
        Err(ElmErr::error(format!("adapter out of step at {} (got {})", str_repr(text), str_repr(&out))))
    }

    fn line(&mut self, text: &str, timeout: f64) -> Result<(String, bool), ElmErr> {
        if let Some(l) = &self.log {
            l.note(&format!("ELM> {text}"));
        }
        let lost = |e: std::io::Error| ElmErr::Reset { msg: format!("adapter port lost at {}: {e}", str_repr(text)), lost: true };
        self.port().write(format!("{text}\r").as_bytes(), super::can::WRITE_TIMEOUT_S).map_err(lost)?;
        let mut buf: Vec<u8> = vec![];
        let end = Instant::now() + secs(timeout);
        while !buf.contains(&b'>') {
            let left = end.saturating_duration_since(Instant::now()).as_secs_f64();
            if left <= 0.0 {
                let msg = format!("no prompt after {} (got {})", str_repr(text), bytes_repr(&buf));
                return Err(if self.ready { ElmErr::Reset { msg: format!("{msg}: adapter hung or gone"), lost: true } } else { ElmErr::Error(msg) });
            }
            buf.extend(self.port().read(left));
            if self.port().lost() {
                return Err(ElmErr::Reset { msg: format!("adapter port lost while waiting for the prompt after {}", str_repr(text)), lost: true });
            }
        }
        let head: &[u8] = buf.split(|b| *b == b'>').next().unwrap_or(&[]);
        let out: String = head.iter().map(|x| if *x < 0x80 { *x as char } else { '\u{fffd}' }).collect::<String>().replace('\r', "\n");
        let mut lines: Vec<String> = out.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
        let echoed = lines.first().is_some_and(|l| l.replace(' ', "") == text.replace(' ', ""));
        if echoed {
            lines.remove(0); // echo, before ATE0 took effect
        }
        if let Some(l) = &self.log {
            for ln in &lines {
                l.note(&format!("ELM< {ln}"));
            }
        }
        Ok((lines.join("\n"), echoed))
    }

    /// None if `out` can be the reply to `text`, else 'reset' (banner, or echo after ATE0) or 'stale'.
    fn mismatch(&self, text: &str, out: &str, echoed: bool) -> Option<&'static str> {
        let (up, upout) = (text.to_uppercase(), out.to_uppercase());
        let banner = BANNERS.iter().any(|b| upout.starts_with(b));
        if ["ATZ", "ATI", "AT@1"].contains(&up.as_str()) {
            return if upout == "OK" { Some("stale") } else { None };
        }
        if banner || (echoed && self.ready) {
            return Some("reset");
        }
        if up == "ATRV" {
            return if upout.ends_with('V') { None } else { Some("stale") };
        }
        if up.starts_with("AT") {
            return if upout.contains("OK") || out.contains('?') { None } else { Some("stale") };
        }
        if upout == "OK" { Some("stale") } else { None }
    }

    pub fn must_cmd(&mut self, c: &str) -> Result<String, ElmErr> {
        self.must(c)
    }

    pub fn close_port(&mut self) {
        if let Some(p) = self.port.take() {
            p.close();
        }
    }

    fn must(&mut self, c: &str) -> Result<String, ElmErr> {
        let r = self.cmd(c, 2.0)?;
        if r.contains('?') {
            return Err(ElmErr::error(format!("adapter rejected {c}: {}", str_repr(&r))));
        }
        Ok(r)
    }

    // -- settings ------------------------------------------------------------------------------------------------------------
    /// Receive timeout after the last frame, in 4 ms steps (ATST).
    pub fn set_timeout(&mut self, ms: u32) -> Result<(), ElmErr> {
        let steps = ms.div_ceil(4).clamp(1, 255);
        self.retrying(&mut |i| i.cmd(&format!("ATST{steps:02X}"), 2.0))?;
        self.timeout = steps as f64 * 4.0 / 1000.0;
        Ok(())
    }

    pub fn voltage(&mut self) -> Option<f64> {
        let r = self.retrying(&mut |i| i.cmd("ATRV", 2.0)).ok()?;
        r.to_uppercase().trim_end_matches('V').parse().ok()
    }

    /// Address the next request. `flow_control=false` skips the flow-control header (a single-frame answer needs none), which
    /// saves a round trip per address while probing.
    pub fn select(&mut self, tx_id: u32, rx_id: u32, flow_control: bool) -> Result<(), ElmErr> {
        if self.header != Some((tx_id, rx_id)) {
            self.send_header(tx_id)?;
            if self.cmd(&format!("ATCRA{rx_id:08X}"), 2.0)?.contains('?') {
                // receive filter; older clones: filter + mask
                self.must(&format!("ATCF{rx_id:08X}"))?;
                self.must("ATCM1FFFFFFF")?;
            }
            self.header = Some((tx_id, rx_id));
        }
        let fc = reply_to(rx_id); // the ECU's own id, also after a functional request
        if flow_control && self.hds_flow_control && self.fc_header != Some(fc) {
            if self.cmd(&format!("ATFCSH{fc:08X}"), 2.0)?.contains('?') {
                self.cmd("ATFCSM0", 2.0)?;
                self.hds_flow_control = false;
            } else {
                self.fc_header = Some(fc);
            }
        }
        Ok(())
    }

    fn send_header(&mut self, tx_id: u32) -> Result<(), ElmErr> {
        if tx_id == POWER_UP_HEADER {
            // the GD101 answers nothing after any ATSH, even ATSHDB33F1 (README.md, "Fourth capture"); only the power-up header works
            if self.custom_header {
                self.init()?;
            }
            return Ok(());
        }
        self.custom_header = true;
        let priority = (tx_id >> 24) & 0x1F;
        if priority != self.priority {
            // never for Honda ids: all have the default 0x18
            self.must(&format!("ATCP{priority:02X}"))?;
            self.priority = priority;
        }
        self.must(&format!("ATSH{:06X}", tx_id & 0xFFFFFF))?;
        Ok(())
    }

    pub fn describe(&mut self) -> String {
        let fc = if self.hds_flow_control { "HDS 30 00 00" } else { "adapter default" };
        let v = self.voltage();
        format!("{} @ {} baud: {}; battery {} V; flow control {fc}", self.path, self.baud, self.ident, v.map_or("None".to_string(), |v| crate::journal::py(&serde_json::json!(v))))
    }
}

impl Adapter for Elm327Wrap {
    fn link(&self, tx_id: u32, rx_id: u32) -> Box<dyn DiagLink> {
        Box::new(Elm327Link::new(self.0.clone(), tx_id, Some(rx_id)))
    }

    fn describe(&self) -> String {
        self.0.with(|i| i.describe())
    }

    fn close(&self) {
        self.0.with(|i| {
            let _ = i.cmd("ATPC", 1.0); // protocol close
            if let Some(p) = i.port.take() {
                p.close();
            }
        });
    }

    fn path(&self) -> Option<String> {
        Some(self.0.with(|i| i.path.clone()))
    }

    fn has_probe(&self) -> bool {
        true
    }

    /// One request per pair (the ELM327 serialises everything), with the adapter's receive timeout set to `timeout` and no
    /// flow-control header. Returns the rx ids that answered.
    fn probe(&self, pairs: &[(u32, u32)], payload: &[u8], timeout: f64) -> Result<BTreeSet<u32>, LinkError> {
        self.0.with(|inn| -> Result<BTreeSet<u32>, ElmErr> {
            let saved = inn.timeout;
            inn.set_timeout((timeout * 1000.0) as u32)?;
            let mut found = BTreeSet::new();
            let mut result = Ok(());
            for &(tx, rx) in pairs {
                let r = inn.retrying(&mut |i| {
                    i.select(tx, rx, false)?;
                    let t = i.timeout + 2.0;
                    exchange(i, tx, Some(rx), payload, t)
                });
                match r {
                    Ok(v) if !v.is_empty() => {
                        found.insert(rx);
                    }
                    Ok(_) => {}
                    Err(ElmErr::Bus(_)) => {} // a bus error on this address: not an answer
                    Err(e) => {
                        result = Err(e);
                        break;
                    }
                }
            }
            inn.set_timeout((saved * 1000.0) as u32)?;
            result.map(|_| found)
        })
        .map_err(Into::into)
    }

    fn has_broadcast(&self) -> bool {
        true
    }

    /// Send the single-frame `payload` once on `tx_id` (a functional id) and collect every single-frame answer on `rx_ids` that
    /// arrives within `timeout` (the adapter's receive timeout, ATST). The receive filter is opened to all of 0x18DAF1xx (ATCF +
    /// ATCM) for this one request. The adapter prints the answers together when its timeout ends, so they are not timed one by
    /// one: {rx id: None}.
    fn broadcast(&self, tx_id: u32, rx_ids: &[u32], payload: &[u8], timeout: f64) -> Result<BTreeMap<u32, Option<f64>>, LinkError> {
        let want: BTreeSet<u32> = rx_ids.iter().cloned().collect();
        self.0
            .with(|inn| -> Result<BTreeMap<u32, Option<f64>>, ElmErr> {
                let saved = inn.timeout;
                inn.set_timeout((timeout * 1000.0) as u32)?;
                let r = inn.retrying(&mut |i| {
                    i.send_header(tx_id)?;
                    i.must("ATCF18DAF100")?;
                    i.must("ATCM1FFFFF00")?;
                    i.header = None; // the next select sets its own filter again
                    let t = i.timeout + 2.0;
                    let text = request(i, tx_id, payload, t)?;
                    Ok(frames(i, &text)?.into_iter().filter(|(cid, d)| want.contains(cid) && !d.is_empty() && d[0] >> 4 == 0).map(|(cid, _)| (cid, None)).collect())
                });
                inn.set_timeout((saved * 1000.0) as u32)?;
                r
            })
            .map_err(Into::into)
    }
}

/// An open ELM327-compatible adapter as an `Adapter`.
pub struct Elm327Wrap(pub Arc<Elm327>);

pub fn open_elm(port: &str, baud: Option<u32>, log: Option<Arc<CaptureLog>>) -> Result<Arc<dyn Adapter>, ElmErr> {
    Ok(Arc::new(Elm327Wrap(Elm327::open(port, baud, 400, log)?)))
}

impl From<IsoTpError> for ElmErr {
    fn from(e: IsoTpError) -> Self {
        ElmErr::Error(e.0)
    }
}
