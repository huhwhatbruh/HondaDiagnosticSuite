//! The phone gateway: a read-only HTTP view of the running backend, for the iPhone app (ios/).
//!
//! It runs inside the backend process that owns the car (`hondascan-rpc` with HONDASCAN_PHONE=addr:port), so the phone shares
//! the one bus owner instead of fighting for it. The gateway never forwards anything a client sends: it has three fixed
//! routes (below) and no way to name a method, so a write, a clear or an actuator test cannot be reached from the phone. It
//! asks the backend for one thing itself, four times a second: `snapshot` (service.rs), which reads this process's memory
//! and puts no traffic on the bus.
//!
//!   GET /v1/snapshot?watch=SYSTEM.SYMBOL,...   the live document, with values for the watched signals only
//!   GET /v1/signals                            every signal read so far, once: {"signals": [{system, symbol, unit, description, value}]}
//!   POST /v1/log                               the app's own measurements, JSON lines (at most 64 KiB), appended to phone-client.jsonl
//! All need `Authorization: Bearer <token>`. `/v1/log` writes a file on the laptop and nothing else: no method of the backend
//! is reachable from any route. Logs (see `log.rs`): `phone-gateway.jsonl` has one `req` line per request (peer, route,
//! status, bytes, handling ms) and one `backend` line per snapshot (ms the backend took to answer, bytes, signals). The live
//! document is {"t", "age_s", "snapshot", "events"}: `snapshot` is the backend's answer (null until the first one) with
//! `signals` cut down to the watched ones (first reading of a symbol wins, each with its low and high), `events` the last
//! `car`, `session` and `error` event it emitted and, under `codes`, the last `codes` event of each system. Signals are
//! listed apart because all of them are about 300 KB, too much to send every second over a hotspot or cell link; a screen
//! watches the few it shows (at most WATCH_MAX).
//!
//! Plumbing: `attach` puts a merged reader in front of the backend (the app's own lines plus the gateway's `snapshot`
//! requests, ids "phone:...") and a tap behind it (answers to those ids stay here, every other line goes on to the app).
//! Submodules: `http` (one request in, one reply out), `guard` (the pairing code and the lockout), `log` (the log files).
mod guard;
mod http;
mod log;

pub use guard::new_token;
pub use http::Conn;
pub use log::log_dir;
pub(crate) use log::Logs;

use guard::{same_token, Refusal, Throttle, MIN_TOKEN};
use http::{compress, read_request, respond, Next, Request, COMPRESS_OVER};
use serde_json::{json, Map, Value};
use std::collections::HashSet;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const PHONE_ID: &str = "phone:snapshot";
const MAX_CLIENTS: usize = 8;
const WATCH_MAX: usize = 64;
const ASK_AGAIN: Duration = Duration::from_secs(10);
/// How often the gateway refreshes its snapshot: four times a second, so a reading is at most a quarter second old when the
/// phone asks (the backend reads the engine's watch list about every 0.5 s; a dial refreshed once a second looked like steps,
/// and one refreshed twice a second still ran visibly behind the engine).
const ASK_EVERY: Duration = Duration::from_millis(250);
const EVENTS: [&str; 4] = ["codes", "car", "session", "error"];
/// A kept-alive connection waits this long for the next request before it is closed.
const IDLE: Duration = Duration::from_secs(15);
const MAX_PER_CONNECTION: usize = 5000;

pub type Out = Arc<Mutex<dyn Write + Send>>;
pub type In = Box<dyn BufRead + Send>;

#[derive(Default)]
struct Cache {
    /// The backend's last answer; shared, so a request takes a handle instead of a copy of ~300 KB under the lock.
    snapshot: Latest,
    asked: Option<Instant>, // when the gateway asked the backend for a snapshot it has not answered yet
    events: Map<String, Value>,
}

type Shared = Arc<Mutex<Cache>>;

/// What every connection handler needs.
struct Ctx {
    cache: Shared,
    token: Vec<u8>,
    logs: Arc<Logs>,
    throttle: Throttle,
}

fn signal_id(s: &Value) -> String {
    format!("{}.{}", s["system"].as_str().unwrap_or(""), s["symbol"].as_str().unwrap_or(""))
}

/// The first reading of each symbol, as the backend counts them.
fn first_readings(all: &[Value]) -> Vec<&Value> {
    let mut seen = HashSet::new();
    all.iter().filter(|s| seen.insert(signal_id(s))).collect()
}

type Latest = Option<(Arc<Value>, Instant)>;

fn snapshot_of(cache: &Shared) -> (Latest, Map<String, Value>) {
    let c = cache.lock().unwrap();
    (c.snapshot.clone(), c.events.clone())
}

fn document(cache: &Shared, watch: &[String]) -> Value {
    let (snapshot, events) = snapshot_of(cache);
    let t = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64());
    let (snap, age) = match &snapshot {
        Some((v, at)) => {
            // everything but the signals is copied; the signals are picked by reference
            let mut o: Map<String, Value> = v.as_object().into_iter().flatten().filter(|(k, _)| *k != "signals").map(|(k, v)| (k.clone(), v.clone())).collect();
            let all = v["signals"].as_array().map_or(&[][..], Vec::as_slice);
            let picked: Vec<&Value> = first_readings(all).into_iter().filter(|s| watch.contains(&signal_id(s))).collect();
            o.insert("signals".into(), json!(picked));
            (Value::Object(o), json!(at.elapsed().as_secs_f64()))
        }
        None => (Value::Null, Value::Null),
    };
    json!({"t": t, "age_s": age, "snapshot": snap, "events": events})
}

/// Every signal read so far, one entry each, without the per-read fields.
fn signal_index(cache: &Shared) -> Value {
    let (snapshot, _) = snapshot_of(cache);
    let all = snapshot.as_ref().and_then(|(v, _)| v["signals"].as_array()).map_or(&[][..], Vec::as_slice);
    let entries: Vec<Value> = first_readings(all)
        .into_iter()
        .map(|s| json!({"system": s["system"], "symbol": s["symbol"], "unit": s["unit"], "description": s["description"], "value": s["value"]}))
        .collect();
    json!({"signals": entries})
}

/// `watch=A.B,C.D` of a query string: ids made of letters, digits and `_ . -` only, at most WATCH_MAX of them.
fn watch_list(query: &str) -> Vec<String> {
    let ok = |id: &&str| !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c));
    query
        .split('&')
        .filter_map(|kv| kv.strip_prefix("watch="))
        .flat_map(|v| v.split(','))
        .filter(ok)
        .take(WATCH_MAX)
        .map(String::from)
        .collect()
}

/// Lines from two senders (the app's stdin and the gateway's own requests) as one byte stream.
struct ChanReader {
    rx: Receiver<String>,
    cur: Vec<u8>,
    pos: usize,
}

impl Read for ChanReader {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if self.pos >= self.cur.len() {
            match self.rx.recv() {
                Ok(mut line) => {
                    line.push('\n');
                    self.cur = line.into_bytes();
                    self.pos = 0;
                }
                Err(_) => return Ok(0),
            }
        }
        let n = out.len().min(self.cur.len() - self.pos);
        out[..n].copy_from_slice(&self.cur[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

/// Everything the backend writes, line by line: the gateway's own answers stay here, the rest goes on to the app. Events are
/// remembered on the way past.
struct Tap {
    inner: Out,
    cache: Shared,
    logs: Arc<Logs>,
    line: Vec<u8>,
}

impl Tap {
    fn handle(&mut self, line: &[u8]) -> std::io::Result<()> {
        if let Ok(mut msg) = serde_json::from_slice::<Value>(line) {
            if msg["id"].as_str() == Some(PHONE_ID) {
                let ms = self.cache.lock().unwrap().asked.take().map(|a| a.elapsed().as_secs_f64() * 1000.0);
                match msg.get_mut("result").map(Value::take) {
                    Some(r) => {
                        let signals = r["signals"].as_array().map(Vec::len);
                        self.cache.lock().unwrap().snapshot = Some((Arc::new(r), Instant::now()));
                        // how long the backend's single thread took to get to the request, and how big the answer is
                        self.logs.gateway.write(json!({"ev": "backend", "ms": ms, "bytes": line.len(), "signals": signals}));
                    }
                    // an error answer ends the wait too, or the next request would be held back for ASK_AGAIN
                    None => {
                        self.logs.gateway.write(json!({"ev": "backend", "ms": ms, "error": msg["error"]}));
                    }
                }
                return Ok(());
            }
            if let Some(ev) = msg["event"].as_str().filter(|e| EVENTS.contains(e)) {
                let mut c = self.cache.lock().unwrap();
                if ev == "codes" {
                    // one event per system: keep the latest of each
                    let system = msg["data"]["system"].as_str().unwrap_or("").to_string();
                    let by_system = c.events.entry("codes").or_insert_with(|| json!({}));
                    by_system[system] = msg["data"].clone();
                } else {
                    c.events.insert(ev.into(), msg["data"].clone());
                }
            }
        }
        let mut o = self.inner.lock().unwrap();
        o.write_all(line)?;
        o.write_all(b"\n")?;
        o.flush()
    }
}

impl Write for Tap {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        for &b in buf {
            if b == b'\n' {
                let line = std::mem::take(&mut self.line);
                self.handle(&line)?;
            } else {
                self.line.push(b);
            }
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// `attach` when HONDASCAN_PHONE=addr:port is set (the token from HONDASCAN_PHONE_TOKEN, else a fresh one printed on stderr),
/// otherwise the pair unchanged.
pub fn from_env(inp: In, out: Out) -> Result<(In, Out), String> {
    let Ok(addr) = std::env::var("HONDASCAN_PHONE") else {
        return Ok((inp, out));
    };
    let token = match std::env::var("HONDASCAN_PHONE_TOKEN") {
        Ok(t) => t,
        Err(_) => new_token()?,
    };
    let dir = log_dir();
    // Bluetooth goes on with the gateway unless HONDASCAN_PHONE_BLE=off: it needs the laptop's adapter, and works without
    // it (the gateway then serves Wi-Fi only and says so in the log)
    let bluetooth = std::env::var("HONDASCAN_PHONE_BLE").map_or(true, |v| v != "off");
    let wired = attach_with(inp, out, &addr, &token, dir.as_deref(), bluetooth)?;
    // so the phone can find the laptop without a typed address; the gateway works without it
    #[cfg(unix)]
    match addr.rsplit(':').next().and_then(|p| p.parse::<u16>().ok()) {
        Some(port) => {
            if let Err(e) = crate::bonjour::advertise(crate::bonjour::Ad::for_this_host(port)) {
                eprintln!("hondascan-rpc: {e}");
            }
        }
        None => eprintln!("hondascan-rpc: bonjour needs a port in HONDASCAN_PHONE"),
    }
    eprintln!("hondascan-rpc: phone gateway on {addr}, token {token}, logs in {}", dir.map_or("nowhere".into(), |d| d.display().to_string()));
    Ok(wired)
}

/// Put the gateway around a backend: serve `addr` over HTTP and return the reader and writer to hand to `rpc::serve`. Logs
/// go to `log_dir` (see `Log`). Wi-Fi only: `from_env` also turns Bluetooth on.
pub fn attach(inp: In, out: Out, addr: &str, token: &str, log_dir: Option<&Path>) -> Result<(In, Out), String> {
    attach_with(inp, out, addr, token, log_dir, false)
}

/// `attach`, and with `bluetooth` the same gateway on a Bluetooth stream channel (ble.rs).
pub fn attach_with(inp: In, out: Out, addr: &str, token: &str, log_dir: Option<&Path>, bluetooth: bool) -> Result<(In, Out), String> {
    if token.len() < MIN_TOKEN {
        return Err(format!("the pairing code must be at least {MIN_TOKEN} characters"));
    }
    let listener = TcpListener::bind(addr).map_err(|e| format!("phone gateway cannot listen on {addr}: {e}"))?;
    let logs = Arc::new(Logs::open(log_dir));
    logs.gateway.write(json!({"ev": "start", "addr": addr}));
    let cache: Shared = Arc::new(Mutex::new(Cache::default()));
    let (tx, rx) = channel::<String>();
    let closed = Arc::new(AtomicBool::new(false));
    {
        let (tx, closed) = (tx.clone(), closed.clone());
        std::thread::spawn(move || {
            for line in inp.lines() {
                if !line.is_ok_and(|l| tx.send(l).is_ok()) {
                    break;
                }
            }
            closed.store(true, Ordering::Relaxed);
        });
    }
    let asker = cache.clone();
    std::thread::spawn(move || {
        let req = json!({"id": PHONE_ID, "method": "snapshot", "params": {}}).to_string();
        let mut last = Instant::now() - ASK_EVERY;
        while !closed.load(Ordering::Relaxed) {
            // One request at a time, at most one every ASK_EVERY: the backend has a single thread, and while it is busy (a
            // long bus read) a steady stream of requests would queue up behind it and every answer would be older than the
            // last. Ask again after ASK_AGAIN if an answer never came.
            let due = match asker.lock().unwrap().asked {
                None => last.elapsed() >= ASK_EVERY,
                Some(a) => a.elapsed() > ASK_AGAIN,
            };
            if due {
                asker.lock().unwrap().asked = Some(Instant::now());
                last = Instant::now();
                if tx.send(req.clone()).is_err() {
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    });
    // one handler for every transport: a connection from the Wi-Fi listener below or from the Bluetooth channel (ble.rs)
    let serve: Arc<dyn Fn(Box<dyn Conn>) + Send + Sync> = {
        let (ctx, clients) = (Arc::new(Ctx { cache: cache.clone(), token: token.as_bytes().to_vec(), logs: logs.clone(), throttle: Throttle::default() }), Arc::new(AtomicUsize::new(0)));
        Arc::new(move |mut conn: Box<dyn Conn>| {
            if clients.fetch_add(1, Ordering::Relaxed) >= MAX_CLIENTS {
                clients.fetch_sub(1, Ordering::Relaxed);
                ctx.logs.gateway.write(json!({"ev": "busy"}));
                let _ = respond(conn.as_mut(), "503 Service Unavailable", "text/plain", b"busy\n", false, None);
                return;
            }
            let (ctx, clients) = (ctx.clone(), clients.clone());
            std::thread::spawn(move || {
                let _ = client(conn, &ctx);
                clients.fetch_sub(1, Ordering::Relaxed);
            });
        })
    };
    #[cfg(not(target_os = "linux"))]
    let _ = bluetooth; // the Bluetooth channel is a BlueZ L2CAP socket
    #[cfg(target_os = "linux")]
    if bluetooth {
        crate::ble::start(serve.clone(), logs.clone());
    }
    std::thread::spawn(move || {
        for conn in listener.incoming().flatten() {
            serve(Box::new(conn));
        }
    });
    let tap: Out = Arc::new(Mutex::new(Tap { inner: out, cache, logs, line: vec![] }));
    Ok((Box::new(BufReader::new(ChanReader { rx, cur: vec![], pos: 0 })), tap))
}

/// (status, content type, body, route) for one request.
fn answer(req: &Request, peer: &str, ctx: &Ctx) -> (&'static str, &'static str, String, String) {
    const TEXT: &str = "text/plain";
    let (route, query) = req.path.split_once('?').unwrap_or((req.path.as_str(), ""));
    let route_name = route.chars().take(40).collect::<String>();
    match ctx.throttle.attempt(peer, same_token(req.bearer.as_bytes(), &ctx.token)) {
        Err(Refusal::Locked(wait)) => return ("429 Too Many Requests", TEXT, format!("too many wrong codes; wait {wait} s\n"), route_name),
        Err(Refusal::Wrong) => return ("401 Unauthorized", TEXT, "pairing code required\n".into(), route_name),
        Ok(()) => {}
    }
    let (status, kind, body) = match (route, req.method.as_str()) {
        ("/v1/snapshot", "GET") => ("200 OK", "application/json", document(&ctx.cache, &watch_list(query)).to_string()),
        ("/v1/signals", "GET") => ("200 OK", "application/json", signal_index(&ctx.cache).to_string()),
        ("/v1/log", "POST") => client_log(&req.body, peer, &ctx.logs),
        ("/v1/snapshot" | "/v1/signals" | "/v1/log", _) => ("405 Method Not Allowed", TEXT, "not allowed\n".into()),
        _ => ("404 Not Found", TEXT, "not found\n".into()),
    };
    (status, kind, body, route_name)
}

/// The app's own measurements, JSON lines, appended to `phone-client.jsonl` with the sender's address. This is a log on the
/// laptop only; it reaches nothing of the car's.
fn client_log(body: &[u8], peer: &str, logs: &Logs) -> (&'static str, &'static str, String) {
    let mut stored = true;
    for line in body.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
        match serde_json::from_slice::<Value>(line) {
            Ok(Value::Object(mut o)) => {
                o.insert("peer".into(), json!(peer));
                stored &= logs.client.write(Value::Object(o));
            }
            _ => return ("400 Bad Request", "text/plain", "each line must be a JSON object\n".into()),
        }
    }
    if stored {
        ("204 No Content", "text/plain", String::new())
    } else {
        ("507 Insufficient Storage", "text/plain", "log full or unavailable\n".into())
    }
}

/// One connection: requests are answered in turn until the client closes it, asks for it to be closed, sends something
/// refused, or is idle for IDLE. The log line of each request says how long the request took to arrive (`wait_ms`: from the
/// connection opening, or from the last reply, to the request's first byte) apart from how long the laptop took to answer
/// (`ms`), which is what tells a slow laptop from a slow phone or network.
fn client(mut conn: Box<dyn Conn>, ctx: &Ctx) -> std::io::Result<()> {
    conn.set_nodelay();
    let peer = conn.peer();
    let logs = &ctx.logs;
    let mut waiting_since = Instant::now();
    let mut pending = Vec::new();
    for served in 0..MAX_PER_CONNECTION {
        let first = served == 0;
        let (req, bad) = match read_request(conn.as_mut(), if first { Duration::from_secs(5) } else { IDLE }, &mut pending) {
            Next::Request(r) => (Some(r), false),
            Next::Closed => {
                if first {
                    // a connection that sent nothing (the phone checking that the laptop is there)
                    logs.gateway.write(json!({"ev": "probe", "peer": peer, "wait_ms": waiting_since.elapsed().as_secs_f64() * 1000.0}));
                }
                return Ok(());
            }
            Next::Bad => (None, true),
        };
        let wait_ms = waiting_since.elapsed().as_secs_f64() * 1000.0;
        let started = Instant::now();
        let (status, kind, body, route) = match &req {
            Some(r) => answer(r, &peer, ctx),
            None => ("400 Bad Request", "text/plain", "bad request\n".into(), "?".into()),
        };
        let keep = !bad && status.starts_with('2') && req.as_ref().is_some_and(|r| !r.close);
        let compressed = req.as_ref().is_some_and(|r| r.deflate) && body.len() > COMPRESS_OVER;
        let payload = if compressed { compress(body.as_bytes()) } else { body.clone().into_bytes() };
        let sent = respond(conn.as_mut(), status, kind, &payload, keep, compressed.then_some("raw-deflate"));
        logs.gateway.write(json!({"ev": "req", "peer": peer, "route": route, "status": status[..3].parse::<u16>().ok(), "bytes": payload.len(), "raw_bytes": body.len(),
                                  "wait_ms": wait_ms, "ms": started.elapsed().as_secs_f64() * 1000.0, "sent": sent.is_ok(), "n": served}));
        if sent.is_err() || !keep {
            return sent;
        }
        waiting_since = Instant::now();
    }
    Ok(())
}
