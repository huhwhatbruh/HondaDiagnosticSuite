//! First contact with a Honda DIAG-H system through a USB-to-K-line cable at 9600 8N1 (transport::diagh_serial): HDS's own wake-up
//! message(s) for a protocol row, one keep-awake (a read request, same row table) and the stop message, every byte printed. It
//! sends nothing else and never a clear. Never run on a car or an ECU: what a real ECU answers is not known (README "K-line
//! systems"), so the transcript this prints is the evidence the project needs. It does NOT send the H/99 wake-up pulse that HDS's
//! interface sends first (the pulse timings are the interface's defaults, not in the database), so an H/99 ECU may stay silent;
//! the DIAG-H line is DLC pin 14 by default.
//!
//!   diagh-session PORT ROW [--log FILE]   (ROW: 1, 2, 3 or 14; row 13 is refused, one command byte there both reads and clears;
//!                                          --log writes every byte with a timestamp)
use crate::diagh::{profile, DiagHClient, DiagHError, DiagHLink, HalfDuplexLink, RawLine, PROFILES};
use crate::transport::diagh_serial::{SerialLine, BAUD};
use std::io::Write;
use std::time::Instant;

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02X}")).collect::<Vec<_>>().join(" ")
}

/// The session on any link: wake, keep-awake if the row has one, stop. Each step is printed with the bytes sent.
pub fn session<L: DiagHLink, W: Write>(c: &mut DiagHClient<L>, out: &mut W) -> Result<(), DiagHError> {
    let p = c.profile;
    let say = |out: &mut W, s: String| {
        let _ = writeln!(out, "{s}");
    };
    say(out, format!("row {}: wake-up {}", p.row, hex(p.wake)));
    let replies = c.wake()?;
    if replies.is_empty() {
        say(out, "  no reply".into());
    }
    for (cmd, data) in &replies {
        say(out, format!("  reply command {cmd:02X} data {}", hex(data)));
    }
    if !p.keep_awake.is_empty() {
        say(out, format!("keep-awake {}", hex(p.keep_awake)));
        match c.keep_awake()? {
            Some((cmd, data)) => say(out, format!("  reply command {cmd:02X} data {}", hex(&data))),
            None => say(out, "  no reply".into()),
        }
    }
    if !p.stop.is_empty() {
        say(out, format!("stop {}", hex(p.stop)));
    }
    c.stop()
}

/// Every byte that crosses the line, with the seconds since the session began: `0.0123 TX FE 04 91 6D`, `0.0460 RX FE 04 91 6D`.
/// A one-wire cable shows each transmitted byte again as an RX (the echo), so the log has the tester's bytes twice.
pub struct LoggedLine<R: RawLine, W: Write> {
    inner: R,
    log: W,
    t0: Instant,
}

impl<R: RawLine, W: Write> LoggedLine<R, W> {
    pub fn new(inner: R, log: W) -> LoggedLine<R, W> {
        LoggedLine { inner, log, t0: Instant::now() }
    }

    fn note(&mut self, dir: &str, bytes: &[u8]) {
        let _ = writeln!(self.log, "{:.4} {dir} {}", self.t0.elapsed().as_secs_f64(), hex(bytes));
        let _ = self.log.flush(); // a hang or a pulled cable must not lose the last bytes
    }
}

impl<R: RawLine, W: Write> RawLine for LoggedLine<R, W> {
    fn write(&mut self, data: &[u8]) {
        self.note("TX", data);
        self.inner.write(data);
    }

    fn read(&mut self, n: usize, timeout: f64) -> Vec<u8> {
        let got = self.inner.read(n, timeout);
        if !got.is_empty() {
            self.note("RX", &got);
        }
        got
    }
}

pub fn run<W: Write>(args: &[String], out: &mut W) -> Result<(), String> {
    let rows: Vec<String> = PROFILES.iter().filter(|p| !p.ambiguous()).map(|p| p.row.to_string()).collect();
    let usage = || format!("usage: diagh-session PORT ROW [--log FILE]   (ROW: {}), {BAUD} baud 8N1", rows.join(", "));
    let (args, log_path) = match args.iter().position(|a| a == "--log") {
        Some(i) => match args.get(i + 1) {
            Some(path) => ([&args[..i], &args[i + 2..]].concat(), Some(path.clone())),
            None => return Err(usage()),
        },
        None => (args.to_vec(), None),
    };
    let [port, row] = &args[..] else {
        return Err(usage());
    };
    let row: u32 = row.parse().map_err(|_| format!("ROW must be a number ({})", rows.join(", ")))?;
    let p = profile(row).filter(|p| !p.ambiguous()).ok_or_else(|| format!("row {row} is not available (choose from {})", rows.join(", ")))?;
    let line = SerialLine::open(port).map_err(|e| format!("cannot open {port}: {e}"))?;
    match log_path {
        None => session(&mut DiagHClient::new(HalfDuplexLink::new(line, true), *p, 0.5, false), out),
        Some(path) => {
            let file = std::fs::File::create(&path).map_err(|e| format!("cannot create {path}: {e}"))?;
            session(&mut DiagHClient::new(HalfDuplexLink::new(LoggedLine::new(line, file), true), *p, 0.5, false), out)
        }
    }
    .map_err(|e| e.to_string())
}
