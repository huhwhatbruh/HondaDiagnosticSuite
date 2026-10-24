//! hondascan command line. Port of hondascan/cli.py.
//!
//!   hondascan -a socketcan:can0 scan              find ECUs (HDS address table + DID rules)
//!   hondascan -a socketcan:can0 dtc [SYSTEM...]   read DTCs (all found systems by default)
//!   hondascan -a socketcan:can0 health            one DTC read per ECU, aggregated (faults/pending/history/silent)
//!   hondascan -a socketcan:can0 clear SYSTEM      clear DTCs of one system (asks for confirmation)
//!   hondascan -a socketcan:can0 read SYSTEM DID   read one data identifier (hex)
//!   hondascan -a socketcan:can0 live SYSTEM       live data with names, values and units (--watch to repeat)
//!   hondascan -a sim scan                         same, against the built-in simulated vehicle
//!   hondascan ports                               list USB serial adapters (GODIAG GD101 = e327:2534)
//!   hondascan -a gd101 adapter                    identify the adapter (ELM version, baud, battery voltage)
//!   hondascan -a gd101 wakeup                     does one functional 3E 00 reach every system? (vs one per address)
//!   hondascan -a elm-kline:/dev/ttyUSB0 kwp ecu-id|read|dtc|clear|present|live ADDR [LID] [--system S]   K-line (ISO 14230); never run on a car
//!   hondascan -a elm-obd:/dev/ttyUSB0 obd scan|dtc|vin|live [PID...]   generic OBD-II on the K-line (cars before ~2008; elm-obd9141: forces ISO 9141-2); never run on a car
//!   hondascan actuator list                       HDS's actuator tests that are whitelisted (never run on a car yet)
//!   hondascan -a ... actuator run TEST VALUE      run one for --seconds (asks first, --yes skips; conditions are checked live)
//!   hondascan write list                          HDS's write functions that are whitelisted (never run on a car yet)
//!   hondascan -a ... --log F --vin VIN write run KEY   run one (asks first, --yes skips; needs a frame log and the car's VIN)
//!   hondascan -a ... memory SYSTEM ADDR LEN       read LEN bytes at hex ADDR (service 0x23; no read has succeeded on a car yet)
//!   hondascan -a ... memory-probe SYSTEM          which 0x23 format/session does the ECU accept? (read of one RAM word)
//!   hondascan -a ... ramwatch SYSTEM              the ignition module's RAM words, only from a 5A0-A510 PCM (--watch S needs --max-seconds N,
//!                                                 --csv FILE, --force-profile; never run on a car)
//!   hondascan -a socketcan:can0 report [--out F]  read the car and write the health report (HTML and JSON under reports/)
//!   hondascan selfcheck                           check this installation (offline; exit 1 when something failed)
//!   hondascan export [--out FILE.json]            write the JSON export and the .tar.gz backup of the data directory (offline)
//!   hondascan --version                           print the version and exit
//!
//! SYSTEM is a database system (CANABS), an HDS type ('HONDA ABS', SRS) or a hex address (0x28, or 28). An address made only of
//! letters needs the 0x (0xEF), since AC and FC are system names. --addresses takes hex addresses (28,53 or 0x28,0x53).
//! --log FILE appends every CAN frame (candump format) and adapter command to FILE.
use crate::actuator::{condition, TESTS};
use crate::actuatorrun::{monotonic_clock, ActuatorRun, Reader, RunError};
use crate::audit;
use crate::db::Database;
use crate::honda::{Ecu, HondaCan, FUNCTIONAL_ID, OBD_FUNCTIONAL_ID, PROBE_ADDRESS};
use crate::ident::{match_ecu, read_ecu_info, read_vin};
use crate::kwp::{real_ecu_id, KwpClient, KwpError, TESTER};
use crate::obd9141::{self, Obd, ObdError};
use crate::transport::elm327::KProto;
use crate::livedata::LiveData;
use crate::ramwatch::{self, check_ecu, profile_system, BLOCK, PROFILE_NAMES};
use crate::service::{Service, ServiceError, ENGINE};
use crate::store::Store;
use crate::transport::can::CaptureLog;
use crate::transport::elm327::str_repr;
use crate::transport::{modem_manager_warning, open_adapter, serial_ports, GD101_USB_ID};
use crate::uds::{DidReader, UdsError};
use crate::write::{self, WRITES};
use crate::writerun::WriteRun;
use std::collections::BTreeSet;
use std::io::{BufRead, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Set by the binary's Ctrl-C handler: Python's KeyboardInterrupt.
pub static INTERRUPTED: AtomicBool = AtomicBool::new(false);

pub struct Io<'a> {
    pub out: &'a mut dyn Write,
    pub err: &'a mut dyn Write,
    pub input: &'a mut dyn BufRead,
    /// stderr is a terminal (the progress line is only for people)
    pub tty: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Cmd {
    Scan,
    Ports,
    Adapter,
    Wakeup,
    SelfCheck,
    Export { out: Option<String> },
    Report { out: Option<String> },
    Dtc { systems: Vec<String>, mask: u8 },
    Health,
    Clear { system: String, yes: bool },
    Read { system: String, did: u16 },
    Memory { system: String, address: u64, length: u64, format: u8, extended: bool },
    MemoryProbe { system: String, address: u64 },
    Ramwatch { system: String, profile: String, watch: Option<f64>, max_seconds: Option<f64>, force_profile: bool, csv: Option<String>, block: u64, format: u8, extended: bool },
    Kwp { action: String, address: u8, lid: Option<u8>, system: Option<String>, destination: Option<i64>, yes: bool },
    Obd { action: String, pids: Vec<u8> },
    Actuator { action: String, test: Option<String>, value: Option<i64>, seconds: f64, yes: bool },
    Write { action: String, key: Option<String>, vin: Option<String>, yes: bool },
    Live { system: String, dids: Vec<u16>, watch: Option<f64> },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Args {
    pub adapter: String,
    pub db: Option<String>,
    pub log: Option<String>,
    pub addresses: Option<Vec<u32>>,
    pub cmd: Cmd,
}

/// What stops a command: a message with exit code 1 (Python's `raise SystemExit(msg)`), or an error (exit code 2).
enum Fail {
    Exit(String),
    Error(String),
}

impl From<UdsError> for Fail {
    fn from(e: UdsError) -> Fail {
        Fail::Error(e.to_string())
    }
}

impl From<ObdError> for Fail {
    fn from(e: ObdError) -> Fail {
        Fail::Exit(e.to_string())
    }
}

impl From<KwpError> for Fail {
    fn from(e: KwpError) -> Fail {
        Fail::Error(e.to_string())
    }
}

impl From<std::io::Error> for Fail {
    fn from(e: std::io::Error) -> Fail {
        Fail::Error(e.to_string())
    }
}

impl From<rusqlite::Error> for Fail {
    fn from(e: rusqlite::Error) -> Fail {
        Fail::Error(e.to_string())
    }
}

impl From<ServiceError> for Fail {
    /// A rejected argument is what the Python CLI raised `SystemExit` for (exit 1); anything else is a real error (2).
    fn from(e: ServiceError) -> Fail {
        if e.kind == "ValueError" { Fail::Exit(e.message) } else { Fail::Error(e.message) }
    }
}

type R<T> = Result<T, Fail>;

/// A SYSTEM argument as an address, or None if it names a system: '0x28', '28', '1E' -> int; 'AC' -> None.
pub fn parse_address(name: &str) -> Option<u32> {
    let n = name.trim();
    let hexd = |s: &str| (1..=2).contains(&s.len()) && s.chars().all(|c| c.is_ascii_hexdigit());
    if let Some(r) = n.strip_prefix("0x").or_else(|| n.strip_prefix("0X")) {
        if hexd(r) {
            return u32::from_str_radix(r, 16).ok();
        }
    }
    (hexd(n) && n.chars().any(|c| c.is_ascii_digit())).then(|| u32::from_str_radix(n, 16).unwrap())
}

fn hex_int(s: &str) -> Result<u64, String> {
    let t = s.trim();
    let t = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")).unwrap_or(t);
    u64::from_str_radix(t, 16).map_err(|_| format!("invalid int value: {s:?}"))
}

fn int0(s: &str) -> Result<u64, String> {
    let t = s.trim();
    let r = if let Some(h) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        u64::from_str_radix(h, 16)
    } else if let Some(b) = t.strip_prefix("0b") {
        u64::from_str_radix(b, 2)
    } else if let Some(o) = t.strip_prefix("0o") {
        u64::from_str_radix(o, 8)
    } else {
        t.parse()
    };
    r.map_err(|_| format!("invalid int value: {s:?}"))
}

fn dec(s: &str) -> Result<i64, String> {
    s.trim().parse().map_err(|_| format!("invalid int value: {s:?}"))
}

fn float(s: &str) -> Result<f64, String> {
    s.trim().parse().map_err(|_| format!("invalid float value: {s:?}"))
}

/// A finite, non-negative number of seconds (`--seconds`).
fn secs(s: &str, name: &str) -> Result<f64, String> {
    let v = float(s)?;
    if !v.is_finite() || v < 0.0 {
        return Err(format!("argument --{name}: {s:?} must be a finite number >= 0"));
    }
    Ok(v)
}

/// A finite, positive number of seconds (a watch interval).
fn interval(s: &str, name: &str) -> Result<f64, String> {
    let v = float(s)?;
    if !v.is_finite() || v <= 0.0 {
        return Err(format!("argument --{name}: {s:?} must be a finite number > 0"));
    }
    Ok(v)
}

/// A user-given integer narrowed to the field the command stores: the error names the option and its range.
fn narrowed<T: TryFrom<u64>>(s: &str, name: &str, what: &str, parse: impl Fn(&str) -> Result<u64, String>) -> Result<T, String> {
    T::try_from(parse(s)?).map_err(|_| format!("argument {name}: {s:?} does not fit a {what}"))
}

/// A user-given count: `dec` takes a signed number, and a plain cast would wrap a negative input.
fn count(s: &str, name: &str) -> Result<u64, String> {
    let n = dec(s)?;
    u64::try_from(n).map_err(|_| format!("argument {name}: {s:?} must be 0 or more"))
}

/// One subcommand's options: names that take a value, and flags.
struct Spec {
    valued: &'static [&'static str],
    flags: &'static [&'static str],
    repeat: &'static [&'static str],
}

struct Parsed {
    pos: Vec<String>,
    opts: Vec<(String, String)>,
    flags: Vec<String>,
}

impl Parsed {
    fn opt(&self, k: &str) -> Option<&str> {
        self.opts.iter().rev().find(|(n, _)| n == k).map(|(_, v)| v.as_str())
    }

    fn all(&self, k: &str) -> Vec<&str> {
        self.opts.iter().filter(|(n, _)| n == k).map(|(_, v)| v.as_str()).collect()
    }

    fn flag(&self, k: &str) -> bool {
        self.flags.iter().any(|f| f == k)
    }
}

fn scan_args(rest: &[String], spec: &Spec) -> Result<Parsed, String> {
    let (mut pos, mut opts, mut flags) = (vec![], vec![], vec![]);
    let mut i = 0;
    while i < rest.len() {
        let t = &rest[i];
        if let Some(name) = t.strip_prefix("--") {
            let (name, inline) = match name.split_once('=') {
                Some((n, v)) => (n, Some(v.to_string())),
                None => (name, None),
            };
            if spec.flags.contains(&name) {
                if let Some(v) = inline {
                    return Err(format!("argument --{name}: ignored explicit argument {v:?}"));
                }
                flags.push(name.to_string());
            } else if spec.valued.contains(&name) {
                let v = match inline {
                    Some(v) => v,
                    None => {
                        i += 1;
                        rest.get(i).cloned().ok_or_else(|| format!("argument --{name}: expected one argument"))?
                    }
                };
                if !spec.repeat.contains(&name) && opts.iter().any(|(n, _): &(String, String)| n == name) {
                    // argparse keeps the last value of a repeated option
                    opts.retain(|(n, _)| n != name);
                }
                opts.push((name.to_string(), v));
            } else {
                return Err(format!("unrecognized arguments: {t}"));
            }
        } else {
            pos.push(t.clone());
        }
        i += 1;
    }
    Ok(Parsed { pos, opts, flags })
}

fn want_pos(p: &Parsed, n: usize, names: &[&str]) -> Result<(), String> {
    if p.pos.len() < n {
        return Err(format!("the following arguments are required: {}", names[p.pos.len()..n].join(", ")));
    }
    Ok(())
}

fn no_extra(p: &Parsed, max: usize) -> Result<(), String> {
    if p.pos.len() > max {
        return Err(format!("unrecognized arguments: {}", p.pos[max..].join(" ")));
    }
    Ok(())
}

/// Parse the command line the way argparse did (global options first, then the subcommand and its own).
pub fn parse_args(argv: &[String]) -> Result<Args, String> {
    let (mut adapter, mut db, mut log, mut addresses) = ("socketcan:can0".to_string(), None, None, None);
    let mut i = 0;
    while i < argv.len() && argv[i].starts_with('-') {
        if argv[i] == "--" {
            i += 1; // a bare -- ends the global options
            break;
        }
        let (name, inline) = match argv[i].split_once('=') {
            Some((n, v)) => (n, Some(v.to_string())),
            None => (argv[i].as_str(), None),
        };
        let val = |i: &mut usize| -> Result<String, String> {
            match &inline {
                Some(v) => Ok(v.clone()),
                None => {
                    *i += 1;
                    argv.get(*i).cloned().ok_or_else(|| format!("argument {name}: expected one argument"))
                }
            }
        };
        match name {
            "-a" | "--adapter" => adapter = val(&mut i)?,
            "--db" => db = Some(val(&mut i)?),
            "--log" => log = Some(val(&mut i)?),
            "--addresses" => {
                let v = val(&mut i)?;
                addresses = Some(v.split(',').map(|x| narrowed(x, "--addresses", "CAN address (0..=0xFFFFFFFF)", hex_int)).collect::<Result<Vec<_>, _>>()?);
            }
            _ => return Err(format!("unrecognized arguments: {}", argv[i])),
        }
        i += 1;
    }
    let Some(sub) = argv.get(i) else { return Err("the following arguments are required: cmd".into()) };
    let rest = &argv[i + 1..];
    let none = Spec { valued: &[], flags: &[], repeat: &[] };
    let cmd = match sub.as_str() {
        "scan" | "ports" | "adapter" | "wakeup" | "selfcheck" => {
            let p = scan_args(rest, &none)?;
            no_extra(&p, 0)?;
            match sub.as_str() {
                "scan" => Cmd::Scan,
                "ports" => Cmd::Ports,
                "adapter" => Cmd::Adapter,
                "selfcheck" => Cmd::SelfCheck,
                _ => Cmd::Wakeup,
            }
        }
        "export" => {
            let p = scan_args(rest, &Spec { valued: &["out"], flags: &[], repeat: &[] })?;
            no_extra(&p, 0)?;
            Cmd::Export { out: p.opt("out").map(String::from) }
        }
        "report" => {
            let p = scan_args(rest, &Spec { valued: &["out"], flags: &[], repeat: &[] })?;
            no_extra(&p, 0)?;
            Cmd::Report { out: p.opt("out").map(String::from) }
        }
        "dtc" => {
            let p = scan_args(rest, &Spec { valued: &["mask"], flags: &[], repeat: &[] })?;
            Cmd::Dtc { systems: p.pos.clone(), mask: p.opt("mask").map(|s| narrowed::<u8>(s, "--mask", "byte (0..=0xFF)", int0)).transpose()?.unwrap_or(0xFF) }
        }
        "health" => {
            let p = scan_args(rest, &none)?;
            no_extra(&p, 0)?;
            Cmd::Health
        }
        "clear" => {
            let p = scan_args(rest, &Spec { valued: &[], flags: &["yes"], repeat: &[] })?;
            want_pos(&p, 1, &["system"])?;
            no_extra(&p, 1)?;
            Cmd::Clear { system: p.pos[0].clone(), yes: p.flag("yes") }
        }
        "read" => {
            let p = scan_args(rest, &none)?;
            want_pos(&p, 2, &["system", "did"])?;
            no_extra(&p, 2)?;
            Cmd::Read { system: p.pos[0].clone(), did: narrowed(&p.pos[1], "did", "DID (0..=0xFFFF)", hex_int)? }
        }
        "memory" => {
            let p = scan_args(rest, &Spec { valued: &["format"], flags: &["extended"], repeat: &[] })?;
            want_pos(&p, 3, &["system", "address", "length"])?;
            no_extra(&p, 3)?;
            Cmd::Memory { system: p.pos[0].clone(), address: hex_int(&p.pos[1])?, length: count(&p.pos[2], "length")?, format: p.opt("format").map(|s| narrowed::<u8>(s, "--format", "byte (0..=0xFF)", int0)).transpose()?.unwrap_or(0x24), extended: p.flag("extended") }
        }
        "memory-probe" => {
            let p = scan_args(rest, &Spec { valued: &["address"], flags: &[], repeat: &[] })?;
            want_pos(&p, 1, &["system"])?;
            no_extra(&p, 1)?;
            Cmd::MemoryProbe { system: p.pos[0].clone(), address: p.opt("address").map(hex_int).transpose()?.unwrap_or(PROBE_ADDRESS) }
        }
        "ramwatch" => {
            let p = scan_args(rest, &Spec { valued: &["profile", "watch", "max-seconds", "csv", "block", "format"], flags: &["force-profile", "extended"], repeat: &[] })?;
            want_pos(&p, 1, &["system"])?;
            no_extra(&p, 1)?;
            let profile = p.opt("profile").unwrap_or("ignition").to_string();
            if !PROFILE_NAMES.contains(&profile.as_str()) {
                return Err(format!("argument --profile: invalid choice: {profile:?} (choose from {})", PROFILE_NAMES.iter().map(|n| format!("{n:?}")).collect::<Vec<_>>().join(", ")));
            }
            let watch = p.opt("watch").map(float).transpose()?;
            if let Some(v) = watch {
                if !(ramwatch::MIN_INTERVAL..=ramwatch::MAX_INTERVAL).contains(&v) {
                    return Err(format!("argument --watch: must be {}..{} seconds (not {v})", ramwatch::MIN_INTERVAL, ramwatch::MAX_INTERVAL));
                }
            }
            Cmd::Ramwatch {
                system: p.pos[0].clone(), profile, watch, max_seconds: p.opt("max-seconds").map(|s| secs(s, "max-seconds")).transpose()?, force_profile: p.flag("force-profile"),
                csv: p.opt("csv").map(String::from), block: p.opt("block").map(|s| count(s, "--block")).transpose()?.unwrap_or(BLOCK),
                format: p.opt("format").map(|s| narrowed::<u8>(s, "--format", "byte (0..=0xFF)", int0)).transpose()?.unwrap_or(0x24), extended: p.flag("extended"),
            }
        }
        "kwp" => {
            let p = scan_args(rest, &Spec { valued: &["system", "destination"], flags: &["yes"], repeat: &[] })?;
            want_pos(&p, 2, &["action", "address"])?;
            no_extra(&p, 3)?;
            let action = p.pos[0].clone();
            if !["ecu-id", "read", "dtc", "clear", "present", "live"].contains(&action.as_str()) {
                return Err(format!("argument action: invalid choice: {action:?} (choose from 'ecu-id', 'read', 'dtc', 'clear', 'present', 'live')"));
            }
            let destination = p.opt("destination").map(dec).transpose()?;
            if destination.is_some_and(|d| !(49..=53).contains(&d)) {
                return Err("argument --destination: invalid choice (choose from 49-53)".into());
            }
            Cmd::Kwp { action, address: narrowed(&p.pos[1], "address", "K-line address (0..=0xFF)", hex_int)?, lid: p.pos.get(2).map(|s| narrowed::<u8>(s, "lid", "LID (0..=0xFF)", hex_int)).transpose()?, system: p.opt("system").map(String::from), destination, yes: p.flag("yes") }
        }
        "obd" => {
            let p = scan_args(rest, &Spec { valued: &[], flags: &[], repeat: &[] })?;
            want_pos(&p, 1, &["action"])?;
            let action = p.pos[0].clone();
            if !["scan", "dtc", "vin", "live"].contains(&action.as_str()) {
                return Err(format!("argument action: invalid choice: {action:?} (choose from 'scan', 'dtc', 'vin', 'live')"));
            }
            if action != "live" {
                no_extra(&p, 1)?;
            }
            Cmd::Obd { action, pids: p.pos[1..].iter().map(|s| narrowed::<u8>(s, "pid", "PID (0..=0xFF)", hex_int)).collect::<Result<_, _>>()? }
        }
        "actuator" => {
            let p = scan_args(rest, &Spec { valued: &["seconds"], flags: &["yes"], repeat: &[] })?;
            want_pos(&p, 1, &["action"])?;
            no_extra(&p, 3)?;
            let action = p.pos[0].clone();
            if action != "list" && action != "run" {
                return Err(format!("argument action: invalid choice: {action:?} (choose from 'list', 'run')"));
            }
            let test = p.pos.get(1).cloned();
            if let Some(t) = &test {
                if !TESTS.iter().any(|x| x.key == t) {
                    return Err(format!("argument test: invalid choice: {t:?}"));
                }
            }
            Cmd::Actuator { action, test, value: p.pos.get(2).map(|s| dec(s)).transpose()?, seconds: p.opt("seconds").map(|s| secs(s, "seconds")).transpose()?.unwrap_or(10.0), yes: p.flag("yes") }
        }
        "write" => {
            let p = scan_args(rest, &Spec { valued: &["vin"], flags: &["yes"], repeat: &[] })?;
            want_pos(&p, 1, &["action"])?;
            no_extra(&p, 2)?;
            let action = p.pos[0].clone();
            if action != "list" && action != "run" {
                return Err(format!("argument action: invalid choice: {action:?} (choose from 'list', 'run')"));
            }
            let key = p.pos.get(1).cloned();
            if let Some(k) = &key {
                if write::write(k).is_none() {
                    return Err(format!("argument write: invalid choice: {k:?}"));
                }
            }
            Cmd::Write { action, key, vin: p.opt("vin").map(String::from), yes: p.flag("yes") }
        }
        "live" => {
            let p = scan_args(rest, &Spec { valued: &["did", "watch"], flags: &[], repeat: &["did"] })?;
            want_pos(&p, 1, &["system"])?;
            no_extra(&p, 1)?;
            Cmd::Live { system: p.pos[0].clone(), dids: p.all("did").into_iter().map(|d| narrowed(d, "--did", "DID (0..=0xFFFF)", hex_int)).collect::<Result<_, _>>()?, watch: p.opt("watch").map(|s| interval(s, "watch")).transpose()? }
        }
        other => return Err(format!("argument cmd: invalid choice: {other:?}")),
    };
    Ok(Args { adapter, db, log, addresses, cmd })
}

fn resolve(ecus: &[Ecu], name: &str) -> R<Ecu> {
    let n = name.trim().to_uppercase();
    if let Some(e) = ecus.iter().find(|e| n == e.system.to_uppercase() || n == e.system_type.to_uppercase()) {
        return Ok(e.clone());
    }
    let Some(addr) = parse_address(name) else {
        let names: Vec<&str> = ecus.iter().map(|e| if e.system.is_empty() { e.system_type.as_str() } else { e.system.as_str() }).collect();
        return Err(Fail::Exit(format!("{name}: not among the found systems ({})", names.join(", "))));
    };
    Ok(ecus.iter().find(|e| e.address == addr).cloned().unwrap_or(Ecu { address: addr, system_type: "?".into(), system: String::new(), rule: "given on command line".into(), probes: Default::default() }))
}

fn addr_hint(name: &str) -> Option<Vec<u32>> {
    parse_address(name).map(|a| vec![a])
}

macro_rules! out {
    ($io:expr, $($a:tt)*) => {{
        let _ = writeln!($io.out, $($a)*);
    }};
}

fn ask(io: &mut Io, prompt: &str) -> bool {
    let _ = write!(io.out, "{prompt}");
    let _ = io.out.flush();
    let mut line = String::new();
    let _ = io.input.read_line(&mut line);
    line.trim().to_lowercase() == "y"
}

fn hex_spaced(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02X}")).collect::<Vec<_>>().join(" ")
}

/// `{:02X}` for a possibly negative number, as Python prints it.
fn hex2(n: i32) -> String {
    if n < 0 { format!("-{:02X}", -n) } else { format!("{n:02X}") }
}

fn sleep_interruptible(s: f64) -> bool {
    // inf/NaN/huge would panic in from_secs_f64; clamp instead (every caller loops or returns anyway)
    let dur = Duration::try_from_secs_f64(s.max(0.0)).unwrap_or(Duration::from_secs(3600));
    let end = Instant::now() + dur;
    while Instant::now() < end {
        if INTERRUPTED.load(Ordering::Relaxed) {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20).min(end.saturating_duration_since(Instant::now())));
    }
    !INTERRUPTED.load(Ordering::Relaxed)
}

/// The `-h`/`--help` text: the usage line plus the module doc's command summary.
const HELP: [&str; 25] = [
    "usage: hondascan [-h] [-a ADAPTER] [--db DB] [--log LOG] [--addresses ADDRESSES] cmd ...",
    "   hondascan -a socketcan:can0 scan              find ECUs (HDS address table + DID rules)",
    "   hondascan -a socketcan:can0 dtc [SYSTEM...]   read DTCs (all found systems by default)",
    "   hondascan -a socketcan:can0 health            one DTC read per ECU, aggregated (faults/pending/history/silent)",
    "   hondascan -a socketcan:can0 clear SYSTEM      clear DTCs of one system (asks for confirmation)",
    "   hondascan -a socketcan:can0 read SYSTEM DID   read one data identifier (hex)",
    "   hondascan -a socketcan:can0 live SYSTEM       live data with names, values and units (--watch to repeat)",
    "   hondascan -a sim scan                         same, against the built-in simulated vehicle",
    "   hondascan ports                               list USB serial adapters (GODIAG GD101 = e327:2534)",
    "   hondascan -a gd101 adapter                    identify the adapter (ELM version, baud, battery voltage)",
    "   hondascan -a gd101 wakeup                     does one functional 3E 00 reach every system? (vs one per address)",
    "   hondascan -a elm-kline:/dev/ttyUSB0 kwp ecu-id|read|dtc|clear|present|live ADDR [LID] [--system S]   K-line (ISO 14230); never run on a car",
    "   hondascan -a elm-obd:/dev/ttyUSB0 obd scan|dtc|vin|live [PID...]   generic OBD-II on the K-line (cars before ~2008; elm-obd9141: forces ISO 9141-2); never run on a car",
    "   hondascan actuator list                       HDS's actuator tests that are whitelisted (never run on a car yet)",
    "   hondascan -a ... actuator run TEST VALUE      run one for --seconds (asks first, --yes skips; conditions are checked live)",
    "   hondascan write list                          HDS's write functions that are whitelisted (never run on a car yet)",
    "   hondascan -a ... --log F --vin VIN write run KEY   run one (asks first, --yes skips; needs a frame log and the car's VIN)",
    "   hondascan -a ... memory SYSTEM ADDR LEN       read LEN bytes at hex ADDR (service 0x23; no read has succeeded on a car yet)",
    "   hondascan -a ... memory-probe SYSTEM          which 0x23 format/session does the ECU accept? (read of one RAM word)",
    "   hondascan -a ... ramwatch SYSTEM              the ignition module's RAM words, only from a 5A0-A510 PCM (--watch S needs --max-seconds N,",
    "                                                 --csv FILE, --force-profile; never run on a car)",
    "   hondascan -a socketcan:can0 report [--out F]  read the car and write the health report (HTML and JSON under reports/)",
    "   hondascan selfcheck                           check this installation: database, data directory, audit trail, adapters (offline)",
    "   hondascan export [--out FILE.json]            write the JSON export and the .tar.gz backup of the data directory (offline)",
    "   hondascan --version                           print the version and exit",
];

/// `hondascan selfcheck`: the installation check (selfcheck.rs), offline. One line per check; exit 1 when something
/// failed, so a packaging script can gate on it.
fn selfcheck(io: &mut Io) -> i32 {
    let root = crate::store::data_dir();
    let db = Database::open(None).ok();
    let report = crate::selfcheck::run(&root, db.as_ref());
    for c in report["checks"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
        out!(io, "{:<4}  {:<15}  {}: {}", c["status"].as_str().unwrap_or("?"), c["id"].as_str().unwrap_or(""), c["title"].as_str().unwrap_or(""), c["detail"].as_str().unwrap_or(""));
    }
    if report["ok"].as_bool().unwrap_or(false) { 0 } else { 1 }
}

/// `hondascan export`: the JSON export and the `.tar.gz` backup of the data directory (backup.rs), offline. Prints both
/// paths; `--out` names the JSON file and the archive sits beside it.
fn export(io: &mut Io, out: Option<&str>) -> i32 {
    let store = match Store::open(None) {
        Ok(s) => s,
        Err(e) => {
            let _ = writeln!(io.err, "error: the history database could not be opened: {e}");
            return 2;
        }
    };
    let root = store.root.clone();
    match crate::backup::export(&store, &root, out.map(std::path::Path::new)) {
        Ok(r) => {
            out!(io, "{}", r["json_path"].as_str().unwrap_or(""));
            out!(io, "{}", r["backup_path"].as_str().unwrap_or(""));
            0
        }
        Err(e) => {
            let _ = writeln!(io.err, "error: {e}");
            2
        }
    }
}

pub fn main(argv: &[String], io: &mut Io) -> i32 {
    if argv.iter().any(|a| a == "-V" || a == "--version") {
        let _ = writeln!(io.out, "hondascan {}", env!("CARGO_PKG_VERSION"));
        return 0;
    }
    if argv.iter().any(|a| a == "-h" || a == "--help") {
        for line in HELP {
            let _ = writeln!(io.out, "{line}");
        }
        return 0;
    }
    let a = match parse_args(argv) {
        Ok(a) => a,
        Err(m) => {
            let _ = writeln!(io.err, "usage: hondascan [-h] [-a ADAPTER] [--db DB] [--log LOG] [--addresses ADDRESSES] cmd ...\nhondascan: error: {m}");
            return 2;
        }
    };
    if let Cmd::Actuator { action, .. } = &a.cmd {
        if action == "list" {
            for t in TESTS {
                out!(io, "{}: {} ({}, id 0x{:04X}, values {}, limit {} s)", t.key, t.name, t.system, t.did, t.values.iter().map(u16::to_string).collect::<Vec<_>>().join("/"), crate::journal::py(&serde_json::json!(t.hold_s)).trim_end_matches(".0"));
                for c in t.conditions {
                    out!(io, "    needs: {}", condition(c).map_or("", |c| c.text));
                }
                out!(io, "    source: {}", t.source);
            }
            return 0;
        }
    }
    if let Cmd::Write { action, .. } = &a.cmd {
        if action == "list" {
            for w in WRITES {
                let stop = w.stop.map_or_else(|| "none (HDS sends no stop)".to_string(), hex_spaced);
                out!(io, "{}: {} ({}, start {}, stop {})", w.key, w.name, w.system, hex_spaced(w.start), stop);
                for c in w.conditions {
                    out!(io, "    needs: {}", condition(c).map_or("", |c| c.text));
                }
                let when = if let Some(m) = w.monitor {
                    format!(
                        "polls {} every {} ms for up to {} s",
                        hex_spaced(m),
                        w.poll_ms,
                        crate::journal::py(&serde_json::json!(w.timeout_s)).trim_end_matches(".0")
                    )
                } else if w.poll_ms > 0 {
                    format!(
                        "polls every {} ms for up to {} s",
                        w.poll_ms,
                        crate::journal::py(&serde_json::json!(w.timeout_s)).trim_end_matches(".0")
                    )
                } else {
                    format!("waits {} s", crate::journal::py(&serde_json::json!(w.wait_s)).trim_end_matches(".0"))
                };
                out!(io, "    {when}, then {}", w.read_back);
                out!(io, "    source: {}", w.source);
            }
            return 0;
        }
    }
    if a.cmd == Cmd::Ports {
        let ports = serial_ports();
        for (dev, vid, pid, prod) in &ports {
            let gd = (vid.as_str(), pid.as_str()) == GD101_USB_ID;
            out!(io, "{dev}  {vid}:{pid}  {prod}{}", if gd { "  <- GODIAG GD101" } else { "" });
            if gd {
                if let Some(w) = modem_manager_warning(dev) {
                    out!(io, "  warning: {w}");
                }
            }
        }
        if ports.is_empty() {
            out!(io, "no USB serial adapters found");
        }
        return 0;
    }
    // Both read this installation only: no adapter is opened.
    if a.cmd == Cmd::SelfCheck {
        return selfcheck(io);
    }
    if let Cmd::Export { out } = &a.cmd {
        return export(io, out.as_deref());
    }
    let db = match Database::open(a.db.as_deref().map(std::path::Path::new)) {
        Ok(d) => Arc::new(d),
        Err(e) => {
            let _ = writeln!(io.err, "error: {e}");
            return 2;
        }
    };
    let log = match &a.log {
        Some(p) => match CaptureLog::at(std::path::Path::new(p)) {
            Ok(l) => Some(Arc::new(l)),
            Err(e) => {
                let _ = writeln!(io.err, "error: {p}: {e}");
                return 2;
            }
        },
        None => None,
    };
    if let Some(l) = &log {
        l.note(&format!("hondascan {}", argv.join(" ")));
    }
    let adapter = match open_adapter(&a.adapter, log.clone()) {
        Ok(ad) => ad,
        Err(e) => {
            let _ = writeln!(io.err, "error: {}: {e}", a.adapter);
            return 2;
        }
    };
    let code = match run(&a, &db, &adapter, &log, io) {
        Ok(c) => c,
        Err(Fail::Exit(m)) => {
            let _ = writeln!(io.err, "{m}");
            1
        }
        Err(Fail::Error(m)) => {
            let _ = writeln!(io.err, "error: {m}");
            2
        }
    };
    adapter.close();
    if let Some(l) = log {
        l.detach();
    }
    code
}

/// `hondascan report`: read the connected car and write the health report (report.rs). Without `--out` the default pair
/// goes under `reports/`; with it, exactly the one file it names. The read is the same one the freeze gate uses
/// (maintenance.rs), so nothing but reads is sent to the car.
fn report(a: &Args, db: &Arc<Database>, adapter: &Arc<dyn crate::transport::Adapter>, log: &Option<Arc<CaptureLog>>, out: Option<&str>, io: &mut Io) -> R<i32> {
    let store = Arc::new(Store::open(None)?);
    let root = store.root.clone();
    let mut svc = Service::new(db.clone(), Arc::new(|_: &str, _: serde_json::Value| {}), Some(store));
    svc.capture = log.clone();
    svc.attach(&a.adapter, adapter.clone());
    let read = (|| -> Result<serde_json::Value, ServiceError> {
        svc.scan(None)?;
        crate::report::build(&mut svc, None)
    })();
    // The adapter main opened is main's to close: closing shuts a file descriptor (SocketCAN, slcan) and is not safe
    // twice. Everything else the service holds still ends here, including the session the scan started.
    svc.adapter.take();
    svc.disconnect("report");
    let report = read?;
    match out {
        Some(path) => out!(io, "{}", crate::report::save_one(&report, std::path::Path::new(path))?.display()),
        None => {
            let (html, json) = crate::report::save(&report, &root)?;
            out!(io, "{}", html.display());
            out!(io, "{}", json.display());
        }
    }
    Ok(0)
}

fn run(a: &Args, db: &Arc<Database>, adapter: &Arc<dyn crate::transport::Adapter>, log: &Option<Arc<CaptureLog>>, io: &mut Io) -> R<i32> {
    if a.cmd == Cmd::Adapter {
        out!(io, "{}", adapter.describe());
        if let Some(w) = adapter.path().and_then(|p| modem_manager_warning(&p)) {
            out!(io, "warning: {w}");
        }
        return Ok(0);
    }
    if let Cmd::Kwp { .. } = &a.cmd {
        return kwp(adapter, db, &a.cmd, io);
    }
    if let Cmd::Obd { .. } = &a.cmd {
        return obd(adapter, db, &a.cmd, io);
    }
    if let Cmd::Report { out } = &a.cmd {
        return report(a, db, adapter, log, out.as_deref(), io);
    }
    let mut hc = HondaCan::new(adapter.clone(), db.clone(), matches!(a.cmd, Cmd::Clear { .. }));
    hc.capture = log.clone();
    let hint = |s: &str| a.addresses.clone().filter(|v| !v.is_empty()).or_else(|| addr_hint(s));
    match &a.cmd {
        Cmd::Wakeup => return wakeup(&mut hc, a.addresses.as_deref(), io),
        Cmd::Memory { system, .. } | Cmd::MemoryProbe { system, .. } | Cmd::Ramwatch { system, .. } => {
            let h = hint(system);
            return memory(&mut hc, &a.cmd, h.as_deref(), io);
        }
        Cmd::Actuator { .. } => return actuator(&mut hc, db, &a.cmd, a.addresses.as_deref(), io),
        Cmd::Write { .. } => return write_cmd(&mut hc, db, &a.cmd, a.addresses.as_deref(), io),
        Cmd::Read { system, did } => {
            let e = resolve(&hc.discover(hint(system).as_deref(), None)?, system)?;
            let d = hc.client(e.address).read_did(*did)?;
            out!(io, "{e}: DID {did:04X} = {}", hex_spaced(&d));
            return Ok(0);
        }
        Cmd::Live { system, dids, watch } => {
            let e = resolve(&hc.discover(hint(system).as_deref(), None)?, system)?;
            if e.system.is_empty() {
                return Err(Fail::Exit(format!("{e}: unknown system, live data needs a database system")));
            }
            return live(&LiveData::new(db.clone()), hc.client(e.address), &e, if dids.is_empty() { None } else { Some(dids) }, *watch, io);
        }
        _ => {}
    }
    let tty = io.tty;
    let progress = |x: u32| eprint!("\rprobing 0x{x:02X}");
    let ecus = hc.discover(a.addresses.as_deref().filter(|v| !v.is_empty()), if tty { Some(&progress) } else { None })?;
    if tty {
        eprint!("\r{}\r", " ".repeat(20));
    }
    match &a.cmd {
        Cmd::Scan => {
            if ecus.is_empty() {
                out!(io, "no ECU answered (ignition ON? the battery reads about 12 V with the key on and the engine off)");
            }
            for e in &ecus {
                out!(io, "0x{:02X}  {:12} {:10} {}", e.address, e.system_type, e.system, e.rule);
            }
            Ok(0)
        }
        Cmd::Dtc { systems, mask } => {
            let targets: Vec<Ecu> = if systems.is_empty() { ecus.clone() } else { systems.iter().map(|s| resolve(&ecus, s)).collect::<R<_>>()? };
            for e in &targets {
                let recs = match hc.read_dtcs(e, *mask) {
                    Ok((_, r)) => r,
                    Err(UdsError::Io(x)) => return Err(x.into()),
                    Err(x) => {
                        out!(io, "{e}: {x}");
                        continue;
                    }
                };
                out!(io, "{e}: {} DTC(s)", recs.len());
                for (r, info) in &recs {
                    let desc = info.as_ref().map_or("(not in database)".to_string(), |i| i.description.clone());
                    let hcode = info.as_ref().filter(|i| !i.honda_code.is_empty() && i.honda_code != "0-0").map_or(String::new(), |i| format!(" [{}]", i.honda_code));
                    out!(io, "  {}-{}{hcode}  {:9}  status 0x{:02X} {}\n      {desc}", r.code, hex2(r.failure_type), r.kind(), r.status, r.flags().join(","));
                }
            }
            Ok(0)
        }
        Cmd::Health => {
            // The health summary: one DTC read per discovered ECU, aggregated like the car page's circles
            // (confirmed faults, pending codes, stored history, no-answer ECUs). Read-only: clear stays
            // behind `clear`. Exit 1 when the read could not establish a clean bill of health.
            if ecus.is_empty() {
                out!(io, "no ECU answered (ignition ON? the battery reads about 12 V with the key on and the engine off)");
                return Ok(1);
            }
            let mut any_fault = false;
            let mut any_pending = false;
            let mut any_stored = false;
            let mut silent = 0;
            for e in &ecus {
                match hc.read_dtcs(e, 0xFF) {
                    Ok((_, recs)) => {
                        let confirmed = recs.iter().filter(|(r, _)| r.kind() == "confirmed").count();
                        let pending = recs.iter().filter(|(r, _)| r.kind() == "pending").count();
                        let stored = recs.iter().filter(|(r, _)| r.kind() == "stored").count();
                        // "faults" means active faults: confirmed codes. Stored codes are history only —
                        // they set the exit code (the docs count them) but not the faults line.
                        if confirmed > 0 {
                            any_fault = true;
                        }
                        if pending > 0 {
                            any_pending = true;
                        }
                        if stored > 0 {
                            any_stored = true;
                        }
                        out!(io, "{e}: {} confirmed, {} pending, {} stored", confirmed, pending, stored);
                    }
                    Err(UdsError::Io(x)) => return Err(x.into()),
                    Err(_) => {
                        silent += 1;
                        out!(io, "{e}: no comms");
                    }
                }
            }
            if silent > 0 {
                out!(io, "{} ECU(s) silent", silent);
            }
            out!(io, "faults: {}; pending: {}; history: {}", if any_fault { "yes" } else { "no" }, if any_pending { "yes" } else { "no" }, if any_stored { "yes" } else { "no" });
            // A silent ECU also means health cannot be confirmed: the bus is not clean, even with no codes anywhere.
            // Stored codes are history, not faults, but the docs still exit 1 on them (the car was faulted).
            Ok(if any_fault || any_pending || any_stored || silent > 0 { 1 } else { 0 })
        }
        Cmd::Clear { system, yes } => {
            let e = resolve(&ecus, system)?;
            if !yes && !ask(io, &format!("Clear all DTCs in {e}? [y/N] ")) {
                out!(io, "not cleared");
                return Ok(1);
            }
            hc.clear_dtcs(&e)?;
            out!(io, "{e}: DTCs cleared");
            Ok(0)
        }
        _ => unreachable!(),
    }
}

fn live(ld: &LiveData, client: &mut dyn DidReader, ecu: &Ecu, dids: Option<&Vec<u16>>, watch: Option<f64>, io: &mut Io) -> R<i32> {
    let list: Option<Vec<i64>> = dids.map(|d| d.iter().map(|x| *x as i64).collect());
    loop {
        out!(io, "{ecu}");
        for (did, err, readings) in ld.read(client, &ecu.system, list.as_deref())? {
            if let Some(e) = err {
                out!(io, "  DID {did:04X}: {e}");
                continue;
            }
            let sigs = ld.signals(&ecu.system, did);
            for (r, s) in readings.iter().zip(sigs.iter()) {
                out!(io, "  {did:04X} {:14} {:>16}  {}", s.symbol, r.display(), s.description);
            }
        }
        match watch {
            Some(w) if w != 0.0 => {
                if !sleep_interruptible(w) {
                    return Ok(0);
                }
            }
            _ => return Ok(0),
        }
    }
}

/// The memory commands (memory.rs): one ECU, found the way `read` finds it. In the extended session (10 03) the ECU is put back in
/// the default one on the way out, whatever happens.
fn memory(hc: &mut HondaCan, cmd: &Cmd, addresses: Option<&[u32]>, io: &mut Io) -> R<i32> {
    let system = match cmd {
        Cmd::Memory { system, .. } | Cmd::MemoryProbe { system, .. } | Cmd::Ramwatch { system, .. } => system,
        _ => unreachable!(),
    };
    if let Cmd::Ramwatch { watch, max_seconds, .. } = cmd {
        // before anything is sent
        if watch.is_none() != max_seconds.is_none() {
            return Err(Fail::Error("--watch and --max-seconds go together: a repeating watch has no default run length".into()));
        }
        if max_seconds.is_some_and(|m| m <= 0.0) {
            return Err(Fail::Error("--max-seconds must be positive".into()));
        }
    }
    let ecus = hc.discover(addresses, None)?;
    let e = resolve(&ecus, system)?;
    if let Cmd::Ramwatch { profile, force_profile, .. } = cmd {
        if !force_profile {
            // the ECU ID is read through the profile's system, whichever row `e` is
            let systems: Vec<String> = ecus.iter().filter(|x| x.address == e.address).map(|x| x.system.clone()).collect();
            let sys = profile_system(profile, &systems).map_err(|x| Fail::Error(x.0))?;
            let db = hc.db.clone();
            let info = read_ecu_info(&db, sys, hc.client(e.address))?;
            check_ecu(profile, info["ecu_model"].as_str(), false).map_err(|x| Fail::Error(x.0))?;
        }
    }
    let extended = matches!(cmd, Cmd::Memory { extended: true, .. } | Cmd::Ramwatch { extended: true, .. });
    if extended {
        hc.client(e.address).session(0x03)?;
    }
    let r = memory_run(hc, cmd, &e, io);
    if extended {
        let _ = hc.client(e.address).session(0x01); // the ECU's S3 timeout drops it back to 10 01 anyway
    }
    r
}

fn memory_run(hc: &mut HondaCan, cmd: &Cmd, e: &Ecu, io: &mut Io) -> R<i32> {
    match cmd {
        Cmd::Memory { address, length, format, .. } => {
            let d = hc.read_memory(e.address, *address, *length, *format)?;
            out!(io, "{e}: 0x{address:08X} +{length} = {}", hex_spaced(&d));
            Ok(0)
        }
        Cmd::MemoryProbe { address, .. } => {
            let rows = hc.probe_memory(e.address, *address, 2)?;
            for r in &rows {
                let fmt = r.format.map_or("--".to_string(), |f| format!("0x{f:02X}"));
                out!(io, "session 0x{:02X} format {fmt}: {}{}", r.session, r.outcome, r.data.as_ref().filter(|d| !d.is_empty()).map_or(String::new(), |d| format!(" {d}")));
            }
            Ok(if rows.last().is_some_and(|r| r.outcome == "ok") { 0 } else { 1 })
        }
        Cmd::Ramwatch { profile, watch, max_seconds, csv, block, format, .. } => {
            let words = ramwatch::profile(profile).unwrap();
            let mut file = match csv {
                Some(p) => Some(std::fs::OpenOptions::new().append(true).create(true).open(p)?),
                None => None,
            };
            if let Some(f) = file.as_mut() {
                if f.metadata()?.len() == 0 {
                    writeln!(f, "time,{}", words.iter().map(|w| w.name).collect::<Vec<_>>().join(","))?;
                }
            }
            let mut emit = |v: Vec<(&'static str, i64)>| {
                let value = |n: &str| v.iter().find(|(k, _)| *k == n).map_or(0, |(_, x)| *x);
                // the console shows the values in the order they were read, the CSV in the profile's word order
                let line: Vec<String> = v.iter().map(|(k, x)| format!("{k}={x}")).collect();
                let _ = writeln!(io.out, "{}", line.join(" "));
                if let Some(f) = file.as_mut() {
                    let _ = writeln!(f, "{:.3},{}", crate::transport::can::now(), words.iter().map(|w| value(w.name).to_string()).collect::<Vec<_>>().join(","));
                }
            };
            let t0 = Instant::now();
            let mut read = |addr: u64, n: u64| -> Result<Vec<u8>, UdsError> { hc.read_memory(e.address, addr, n, *format) };
            let r: Result<u64, UdsError> = ramwatch::watch(
                &mut read,
                words,
                ramwatch::WatchOpts {
                    emit: &mut emit,
                    every: *watch,
                    block: *block,
                    max_seconds: *max_seconds,
                    clock: &|| t0.elapsed().as_secs_f64(),
                    sleep: &|s| {
                        sleep_interruptible(s);
                    },
                    stop: &|| INTERRUPTED.load(Ordering::Relaxed),
                },
            );
            r?;
            Ok(0)
        }
        _ => unreachable!(),
    }
}

/// ISO 14230 requests to one K-line ECU (kwp.rs). The adapter initialises the bus on the first request.
fn kwp(adapter: &Arc<dyn crate::transport::Adapter>, db: &Arc<Database>, cmd: &Cmd, io: &mut Io) -> R<i32> {
    let Cmd::Kwp { action, address, lid, system, destination, yes } = cmd else { unreachable!() };
    let Some(kline) = adapter.as_kline().filter(|k| k.proto() != KProto::Iso9141) else {
        return Err(Fail::Exit("kwp needs a K-line adapter, e.g. -a elm-kline:/dev/ttyUSB0 (elm-kline5: for 5-baud init); a plain OBD-II car on ISO 9141-2 is `obd` with elm-obd9141".into()));
    };
    let mut c = KwpClient::new(kline.kwp_link(*address, TESTER));
    c.allow_clear = action == "clear";
    let who = format!("ECU 0x{address:02X}");
    match action.as_str() {
        "ecu-id" => {
            let d = c.read_ecu_id(0x80)?;
            let text: String = d.iter().map(|b| if *b < 0x80 { *b as char } else { '\u{fffd}' }).collect();
            out!(io, "{who}: {}  {}", hex_spaced(&d), str_repr(&text));
            // HDS's body-ECU identity (BE_UTILS.CreateRealEcuId): LID 0x14 (6 bytes) + the destination byte, read only when asked for
            if let Some(dest) = destination {
                match c.read_by_lid(0x14).and_then(|raw| real_ecu_id(&raw, *dest)) {
                    Err(x) => out!(io, "{who}: no ECU ID from LID 14: {x}"),
                    Ok(eid) => {
                        let name = system.as_ref().and_then(|s| match_ecu(db, &s.to_uppercase(), &eid));
                        out!(io, "{who}: ECU ID {eid}{}", name.map_or(" (no HDS entry; give --system for the match)".to_string(), |n| format!(" = {n}")));
                    }
                }
            }
        }
        "read" => {
            let Some(lid) = lid else { return Err(Fail::Exit("kwp read needs a local identifier (hex)".into())) };
            let d = c.read_by_lid(*lid)?;
            out!(io, "{who}: LID {lid:02X} = {}", hex_spaced(&d));
        }
        "dtc" => {
            let recs = c.read_dtcs(crate::kwp::ALL_GROUPS)?;
            out!(io, "{who}: {} code(s)", recs.len());
            for r in &recs {
                let hit = db.lookup_dtc(&r.code, Some(r.failure_type as i64), system.as_ref().map(|s| s.to_uppercase()).as_deref(), Some(r.kind() == "pending"));
                out!(io, "  {}  {:9} {}  {}", r.code, r.kind(), r.flags().join(" "), hit.first().map_or("", |h| h.description.as_str()));
            }
        }
        "live" => {
            let Some(system) = system else { return Err(Fail::Exit("kwp live needs --system (the database name of the K-line system, e.g. METER)".into())) };
            for (lid, err, readings) in LiveData::new(db.clone()).read(&mut c, &system.to_uppercase(), None)? {
                if let Some(e) = err {
                    out!(io, "LID {lid:02X}: {e}");
                }
                let sigs = LiveData::new(db.clone()).signals(&system.to_uppercase(), lid);
                for (r, s) in readings.iter().zip(sigs.iter()) {
                    out!(io, "LID {lid:02X} {:14} {}", s.symbol, r.display());
                }
            }
        }
        "present" => {
            c.tester_present()?;
            out!(io, "{who}: answers TesterPresent");
        }
        _ => {
            if !yes && !ask(io, &format!("Clear all DTCs in {who}? [y/N] ")) {
                out!(io, "not cleared");
                return Ok(1);
            }
            c.clear_dtcs()?;
            out!(io, "{who}: DTCs cleared");
        }
    }
    Ok(0)
}

/// Generic OBD-II requests over ISO 9141-2 (obd9141.rs): read only, every ECU that answers. The adapter starts the bus on the first request.
fn obd(adapter: &Arc<dyn crate::transport::Adapter>, db: &Arc<Database>, cmd: &Cmd, io: &mut Io) -> R<i32> {
    let Cmd::Obd { action, pids } = cmd else { unreachable!() };
    let Some(kline) = adapter.as_kline() else {
        return Err(Fail::Exit("obd needs a K-line adapter, e.g. -a elm-obd:/dev/ttyUSB0 (tries every K-line protocol), elm-obd9141: or gd101-obd".into()));
    };
    let o = Obd::new(kline);
    let codes = |io: &mut Io, service: u8, label: &str| -> R<()> {
        for (ecu, list) in o.dtcs(service)? {
            out!(io, "ECU 0x{ecu:02X} {label}: {} code(s)", list.len());
            for c in &list {
                let hit = db.lookup_dtc(c, None, None, Some(service == obd9141::PENDING_DTC));
                out!(io, "  {c}  {}", hit.first().map_or("", |h| h.description.as_str()));
            }
        }
        Ok(())
    };
    match action.as_str() {
        "vin" => match o.vin()? {
            Some(v) => out!(io, "VIN {v}"),
            None => out!(io, "no VIN (the car did not answer mode 09 PID 02 with a 17-character VIN)"),
        },
        "dtc" => {
            codes(io, obd9141::STORED_DTC, "stored")?;
            codes(io, obd9141::PENDING_DTC, "pending")?;
        }
        "live" => {
            let mut wanted: Vec<u8> = pids.clone();
            if wanted.is_empty() {
                wanted = o.supported_pids()?.into_values().flatten().collect::<std::collections::BTreeSet<_>>().into_iter().filter(|p| obd9141::decode(*p, &[0, 0]).is_some()).collect();
            }
            for pid in wanted {
                for (ecu, d) in o.read_pid(pid)? {
                    match obd9141::decode(pid, &d) {
                        Some(r) => out!(io, "ECU 0x{ecu:02X} PID {pid:02X} {:38} {:.2} {}", r.name, r.value, r.unit),
                        None => out!(io, "ECU 0x{ecu:02X} PID {pid:02X} {}", hex_spaced(&d)),
                    }
                }
            }
        }
        _ => {
            for (ecu, set) in o.supported_pids()? {
                out!(io, "ECU 0x{ecu:02X} supports {} mode 01 PID(s): {}", set.len(), set.iter().map(|p| format!("{p:02X}")).collect::<Vec<_>>().join(" "));
            }
            for (ecu, (mil, n)) in o.status()? {
                out!(io, "ECU 0x{ecu:02X} check engine light {}, {n} confirmed code(s)", if mil { "ON" } else { "off" });
            }
            codes(io, obd9141::STORED_DTC, "stored")?;
            codes(io, obd9141::PENDING_DTC, "pending")?;
            if let Some(v) = o.vin()? {
                out!(io, "VIN {v}");
            }
        }
    }
    Ok(0)
}

/// The identity a write's audit attempt and result share (audit.rs schema). `result` also clears the pending record
/// and appends the result line, so every exit path of `write_cmd` records what happened.
struct AuditWrite {
    root: std::path::PathBuf,
    vin: String,
    system: &'static str,
    address: u32,
    key: &'static str,
    name: &'static str,
    request: String,
    frame_log: Option<String>,
}

impl AuditWrite {
    fn result(&self, io: &mut Io, ok: bool, outcome: &str, run: Option<&serde_json::Value>, errors: Vec<String>) {
        let field = |k: &str| run.and_then(|r| r.get(k)).cloned().unwrap_or(serde_json::Value::Null);
        let mut record = audit::record("result");
        record["vin"] = serde_json::json!(self.vin);
        record["system"] = serde_json::json!(self.system);
        record["address"] = serde_json::json!(self.address);
        record["key"] = serde_json::json!(self.key);
        record["name"] = serde_json::json!(self.name);
        record["request"] = serde_json::json!(self.request);
        record["frame_log"] = serde_json::json!(self.frame_log);
        record["ok"] = serde_json::json!(ok);
        record["outcome"] = serde_json::json!(outcome);
        record["old"] = field("old");
        record["new"] = field("new");
        record["error_value"] = field("error_value");
        record["errors"] = serde_json::json!(errors);
        if let Err(e) = audit::clear_pending(&self.root) {
            record["detail"] = serde_json::json!(format!("the pending record could not be removed: {e}"));
        }
        if let Err(e) = audit::append(&self.root, &record) {
            let _ = writeln!(io.err, "error: the write result could not be recorded in the audit: {e}");
        }
    }
}

/// One write function (write.rs/writerun.rs): VIN match, HDS's conditions, the old value, the routine and the read-back.
fn write_cmd(hc: &mut HondaCan, db: &Arc<Database>, cmd: &Cmd, addresses: Option<&[u32]>, io: &mut Io) -> R<i32> {
    let Cmd::Write { key, vin, yes, .. } = cmd else { unreachable!() };
    let (Some(key), Some(vin)) = (key, vin) else {
        return Err(Fail::Exit("write run needs KEY and --vin VIN (see: write list; scan prints the VIN)".into()));
    };
    let vin = vin.trim().to_ascii_uppercase();
    if vin.is_empty() {
        return Err(Fail::Exit("a write needs the car's VIN (--vin VIN; scan prints it)".into()));
    }
    let w = *write::write(key).unwrap();
    if !hc.capture.as_ref().is_some_and(|c| c.is_attached() && !c.capped()) {
        return Err(Fail::Exit("a write needs --log FILE: every frame must be recorded, and the log must be attached and not size-capped".into()));
    }
    if !yes {
        let session = match w.session {
            Some(s) => format!(" in session {s}"),
            None => " without a session change".to_string(), // the meter: HDS's sub_65f9 changes no session
        };
        if let Some(stop) = w.stop {
            out!(io, "{}: sends {} and {} to the {} ECU{session}, waits {} s, then reads {}.",
                w.name, hex_spaced(w.start), hex_spaced(stop), w.system,
                crate::journal::py(&serde_json::json!(w.wait_s)).trim_end_matches(".0"), w.read_back);
        } else if let Some(monitor) = w.monitor {
            out!(io, "{}: sends {} to the {} ECU{session}, then polls {} every {} ms for up to {} s (HDS sends no stop): {}.",
                w.name, hex_spaced(w.start), w.system, hex_spaced(monitor), w.poll_ms,
                crate::journal::py(&serde_json::json!(w.timeout_s)).trim_end_matches(".0"), w.read_back);
        } else {
            out!(io, "{}: sends {} to the {} ECU{session}, then polls every {} ms for up to {} s (HDS sends no stop): {}.",
                w.name, hex_spaced(w.start), w.system, w.poll_ms,
                crate::journal::py(&serde_json::json!(w.timeout_s)).trim_end_matches(".0"), w.read_back);
        }
        let conds = w.conditions.iter().map(|c| condition(c).map_or("", |c| c.text)).collect::<Vec<_>>().join("; ");
        // The meter's VIN gate reads the PCM's F190 (vin_system), so the wording names that ECU.
        let vin_text = match w.vin_system {
            Some(sys) => format!("the {sys} ECU's VIN must be {vin}"),
            None => format!("the ECU's VIN must be {vin}"),
        };
        if conds.is_empty() {
            out!(io, "conditions: none (HDS checks none); {vin_text}.");
        } else {
            out!(io, "conditions: {conds}; {vin_text}.");
        }
        if !ask(io, "Run it? [y/N] ") {
            out!(io, "not run");
            return Ok(1);
        }
    }
    let ecus = hc.discover(addresses.filter(|v| !v.is_empty()), None)?;
    let Some(e) = ecus.iter().find(|e| e.system == w.system) else { return Err(Fail::Exit(format!("{} did not answer", w.system))) };
    let reader = Reader::new(&LiveData::new(db.clone()), w.system, &WriteRun::symbols(&w));
    let mut run = WriteRun::new(e.address, w, reader);
    // A write whose ECU does not answer F190 (the meter) verifies the VIN through the system named in `vin_system`.
    if let Some(sys) = w.vin_system {
        run.vin_address = ecus.iter().find(|x| x.system == sys).map(|x| x.address).or(Some(e.address));
    }

    // The audit and pending records go down before the runner may send anything.
    let root = crate::store::data_dir();
    if audit::pending_path(&root).exists() {
        return Err(Fail::Exit("a pending operation is already recorded; recover it first".into()));
    }
    let frame_log = hc.capture.as_ref().and_then(|c| c.path()).map(|p| p.to_string_lossy().into_owned());
    let entry = AuditWrite {
        root: root.clone(),
        vin: vin.clone(),
        system: w.system,
        address: e.address,
        key: w.key,
        name: w.name,
        request: hex_spaced(w.start),
        frame_log: frame_log.clone(),
    };
    let mut attempt = audit::record("attempt");
    attempt["action"] = serde_json::json!("write");
    attempt["vin"] = serde_json::json!(vin);
    attempt["system"] = serde_json::json!(w.system);
    attempt["address"] = serde_json::json!(e.address);
    attempt["key"] = serde_json::json!(w.key);
    attempt["name"] = serde_json::json!(w.name);
    attempt["request"] = serde_json::json!(entry.request);
    attempt["frame_log"] = serde_json::json!(frame_log);
    attempt["old_value"] = serde_json::Value::Null;
    if let Err(e) = audit::append(&root, &attempt) {
        return Err(Fail::Error(format!("the audit record could not be written, so nothing was sent: {e}")));
    }
    let pending = serde_json::json!({
        "schema": audit::SCHEMA,
        "at": crate::transport::can::now(),
        "pid": std::process::id(),
        "kind": "write",
        "vin": vin,
        "vin_address": run.vin_address.unwrap_or(e.address),
        "address": e.address,
        "system": w.system,
        "name": w.name,
        "frame_log": frame_log,
        "key": w.key,
        "session": w.session,
        "stop": w.stop.map(hex_spaced),
        "request": entry.request,
    });
    if let Err(e) = audit::write_pending(&root, &pending) {
        let message = format!("the pending record could not be written, so nothing was sent: {e}");
        // The attempt is already in the trail: close it, or the record would show a write that never ended.
        entry.result(io, false, "refused", None, vec![message.clone()]);
        return Err(Fail::Error(message));
    }

    let r = match run.run(hc, &vin, true, &|| INTERRUPTED.load(Ordering::Relaxed)) {
        Ok(v) => v,
        Err(RunError::Condition(c)) => {
            out!(io, "not run: {c}");
            entry.result(io, false, "refused", None, vec![c.to_string()]);
            return Ok(1);
        }
        Err(RunError::Value(m)) => {
            entry.result(io, false, "refused", None, vec![m.clone()]);
            return Err(Fail::Error(m));
        }
        Err(RunError::Runtime(m)) => {
            let outcome = if m.contains("cancelled") {
                "cancelled"
            } else if m.contains("the start may have reached the ECU") {
                "unconfirmed"
            } else {
                "error"
            };
            entry.result(io, false, outcome, None, vec![m.clone()]);
            return Err(Fail::Error(m));
        }
        Err(RunError::Uds(u)) => {
            entry.result(io, false, "error", None, vec![u.to_string()]);
            return Err(u.into());
        }
    };
    entry.result(io, r["ok"] == true, if r["ok"] == true { "complete" } else { "unconfirmed" }, Some(&r), {
        r["errors"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default()
    });
    let read = read_text(&r, &w);
    out!(io, "{e}: {}", if r["ok"] == true {
        format!("{} complete ({read})", w.name)
    } else if w.stop.is_some() {
        format!("{} did not confirm; the routine was stopped ({read})", w.name)
    } else {
        format!("{} did not confirm ({read})", w.name)
    });
    let errors: Vec<String> = r["errors"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
    if !errors.is_empty() {
        out!(io, "problems: {}", errors.join("; "));
    }
    Ok(if r["ok"] == true && errors.is_empty() { 0 } else { 1 })
}

fn bit_text(v: &serde_json::Value) -> String {
    match v.as_bool() {
        Some(true) => "1".into(),
        Some(false) => "0".into(),
        None => "?".into(),
    }
}

/// The run's old -> new read-back: HDS's status code for a DID-poll write, the routine status byte for a monitored
/// one, the success bit for a single-read one.
fn read_text(r: &serde_json::Value, w: &write::Write) -> String {
    match r["kind"].as_str() {
        // The meter's proof is the routine status itself: 0x7F while the reset runs, 0 when HDS calls it complete.
        Some("routine_status") => format!(
            "routine status 0x{:02X} -> {}",
            w.status().map_or(0, |s| s.pending),
            r["new_value"].as_u64().map_or_else(|| "?".into(), |x| format!("0x{x:02X}"))
        ),
        Some("did") if r["poll_ms"].as_u64().unwrap_or(0) > 0 => {
            let n = |v: &serde_json::Value| v.as_u64().map_or_else(|| "?".into(), |x| x.to_string());
            format!("SLPLMOD {} -> {}", n(&r["old_value"]), n(&r["new_value"]))
        }
        _ => format!("{} -> {}", bit_text(&r["old"]), bit_text(&r["new"])),
    }
}

/// One actuator test (actuator.rs): conditions are read from the car before the command and at every refresh.
fn actuator(hc: &mut HondaCan, db: &Arc<Database>, cmd: &Cmd, addresses: Option<&[u32]>, io: &mut Io) -> R<i32> {
    let Cmd::Actuator { test, value, seconds, yes, .. } = cmd else { unreachable!() };
    let (Some(test), Some(value)) = (test, value) else { return Err(Fail::Exit("actuator run needs TEST and VALUE (see: actuator list)".into())) };
    let t = *TESTS.iter().find(|x| x.key == test).unwrap();
    let value = u16::try_from(*value).map_err(|_| Fail::Exit(format!("actuator value {value} is outside 0-65535")))?;
    let dur = Duration::try_from_secs_f64(*seconds).map_err(|_| Fail::Exit(format!("invalid --seconds {seconds}")))?;
    let ecus = hc.discover(addresses.filter(|v| !v.is_empty()), None)?;
    let Some(e) = ecus.iter().find(|e| e.system == t.system) else { return Err(Fail::Exit(format!("{} did not answer", t.system))) };
    if !yes {
        out!(io, "{}: sends value {value} to {e} for up to {} s, then hands control back.", t.name, crate::journal::py(&serde_json::json!(seconds.min(t.hold_s))).trim_end_matches(".0"));
        if !ask(io, "Run it? [y/N] ") {
            out!(io, "not run");
            return Ok(1);
        }
    }
    let symbols: BTreeSet<String> = ActuatorRun::symbols(&t);
    let reader = Reader::new(&LiveData::new(db.clone()), t.system, &symbols);
    let mut run = ActuatorRun::new(e.address, t, reader, 1.0, monotonic_clock());

    // The pending record goes down before the start may reach the ECU. On this car the VIN gate reads the engine ECU's
    // F190; a car whose VIN was never read is recorded as UNKNOWN, which recovery will refuse to act on.
    let root = crate::store::data_dir();
    if audit::pending_path(&root).exists() {
        return Err(Fail::Exit("a pending operation is already recorded; recover it first".into()));
    }
    let mut vin_addrs: Vec<u32> = ecus.iter().map(|x| x.address).collect();
    vin_addrs.sort_unstable();
    vin_addrs.dedup();
    vin_addrs.sort_by_key(|a| !ecus.iter().any(|x| x.address == *a && ENGINE.contains(&x.system_type.as_str())));
    let vin = read_vin(hc, &vin_addrs)?.unwrap_or_else(|| "UNKNOWN".to_string());
    let vin_address = ecus.iter().find(|x| ENGINE.contains(&x.system_type.as_str())).map(|x| x.address).unwrap_or(e.address);
    let frame_log = hc.capture.as_ref().and_then(|c| c.path()).map(|p| p.to_string_lossy().into_owned());
    let pending = serde_json::json!({
        "schema": audit::SCHEMA,
        "at": crate::transport::can::now(),
        "pid": std::process::id(),
        "kind": "actuator",
        "vin": vin,
        "vin_address": vin_address,
        "address": e.address,
        "system": t.system,
        "name": t.name,
        "frame_log": frame_log,
        "test": t.key,
        "io_did": t.did,
        "value": value,
    });
    if let Err(err) = audit::write_pending(&root, &pending) {
        return Err(Fail::Error(format!("the pending record could not be written, so nothing was sent: {err}")));
    }

    match run.start(hc, value, true) {
        Ok(_) => {}
        Err(RunError::Condition(c)) => {
            let _ = audit::clear_pending(&root);
            out!(io, "not started: {c}");
            return Ok(1);
        }
        Err(RunError::Uds(u)) => {
            let _ = audit::clear_pending(&root);
            return Err(u.into());
        }
        Err(other) => {
            let _ = audit::clear_pending(&root);
            return Err(Fail::Error(other.to_string()));
        }
    }
    out!(io, "{}: value {value} sent; Ctrl-C ends it", t.name);
    let end = Instant::now() + dur;
    while run.active && Instant::now() < end && !INTERRUPTED.load(Ordering::Relaxed) {
        let Some(wait) = run.tick(hc) else { break };
        sleep_interruptible(wait.min(0.25));
    }
    let status = run.stop(hc, "finished");
    if let Err(err) = audit::clear_pending(&root) {
        let _ = writeln!(io.err, "the pending record could not be removed: {err}");
    }
    let errors: Vec<String> = status["errors"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
    out!(io, "ended: {}{}", status["reason"].as_str().unwrap_or(""), if errors.is_empty() { String::new() } else { format!(" (problems: {})", errors.join("; ")) });
    Ok(if errors.is_empty() { 0 } else { 1 })
}

/// Which ways does the car answer TesterPresent: on HDS's functional id, on the OBD-II functional id, or on each address's own id
/// (what a scan tries first)? One line each.
fn wakeup(hc: &mut HondaCan, addresses: Option<&[u32]>, io: &mut Io) -> R<i32> {
    let addresses: Vec<u32> = match addresses.filter(|v| !v.is_empty()) {
        Some(a) => a.to_vec(),
        None => hc.db.can_rules().keys().map(|k| *k as u32).collect(),
    };
    let mut found: Vec<BTreeSet<u32>> = vec![];
    for way in 0..3 {
        let name = match way {
            0 => format!("functional 3E 00 on {FUNCTIONAL_ID:08X}"),
            1 => format!("functional 3E 00 on {OBD_FUNCTIONAL_ID:08X}"),
            _ => format!("3E 00 to each of {} addresses on its own id", addresses.len()),
        };
        let t = Instant::now();
        let got: std::collections::BTreeMap<u32, Option<f64>> = match way {
            0 => hc.wakeup(1.0)?,
            1 => hc.answer_functional(OBD_FUNCTIONAL_ID, &(0..256).collect::<Vec<u32>>(), 1.0)?,
            _ => hc.present_physical(&addresses, crate::uds::P2)?.into_iter().map(|a| (a, None)).collect(),
        };
        out!(io, "{name}: {} answered in {:.2} s", got.len(), t.elapsed().as_secs_f64());
        for (addr, after) in &got {
            out!(io, "  0x{addr:02X}{}", after.map_or(String::new(), |a| format!("  after {:.0} ms", a * 1000.0)));
        }
        found.push(got.keys().filter(|a| addresses.contains(a)).cloned().collect());
    }
    let same = found.iter().all(|f| *f == found[0]);
    out!(io, "{}", if same { "same systems every way".to_string() } else { format!("the ways differ among the {} addresses (see above)", addresses.len()) });
    Ok(0)
}
