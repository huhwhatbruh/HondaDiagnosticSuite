//! Sharing a car report: what a person may choose to send the project so it can see which cars, adapters and control units
//! work, and the one place that sends it.
//!
//! Nothing leaves the machine unless the person answered Share (the first-launch card, `hondascan share on`): the answer and a
//! random install id live in `sharing.json` in the data directory. After that a report goes out by itself when a recorded
//! session has ended and the report differs from the last one sent (at most once a day); `bundle` is the text `preview` shows, so what is
//! reviewed is what goes out.
//!
//! The report is built from an allow-list, not by scrubbing the history: per car the model part of the VIN (make, descriptor,
//! model year, plant; never the check digit or the serial), which control units answered with their ids and part numbers,
//! the trouble codes seen, and per session only how the adapter and the car behaved (which adapter type, how long the scan
//! took, reads that worked, were refused or went unanswered). Not in it: the VIN, timestamps, driving data (speed, fuel,
//! maps, drive statistics), notes, the write audit trail, recordings, paths, the Honda database. Sessions on a simulated car
//! are left out entirely. Every string is still scrubbed of VIN-shaped words, the data directory and the home directory, as a
//! second line behind the allow-list.
//!
//! Sending shells out to the system `curl` (the packaging already relies on the system `tar`), so the crate gains no TLS
//! dependency. The endpoint must be https, or http on a loopback address (the tests' mock server). The collector
//! (`collector/`) keeps what it is sent and nothing about who sent it.
use crate::audit::sha256_hex;
use crate::service::ServiceError;
use crate::store::Store;
use crate::transport::can::now;
use crate::value::{dumps, Json};
use regex::Regex;
use serde_json::{json, Map, Value};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The report's JSON schema. A new key may be added; changing one needs a new number.
pub const SCHEMA: u64 = 1;

/// Where `send` posts. Set when a release is built (`HONDASCAN_SHARE_ENDPOINT` in the build's environment, read here at compile
/// time, the way the database key is), so the public source holds no address and a build from source has none: it asks nobody
/// anything and cannot send. The address is never printed, logged or returned by anything in this program, and the exporter
/// refuses to publish one (public-export/export.py). It is in the release binary, as it has to be for the program to use it:
/// what protects the collector is on its side (collector/worker.js), not any hiding of the address.
pub const DEFAULT_ENDPOINT: &str = match option_env!("HONDASCAN_SHARE_ENDPOINT") {
    Some(url) => url,
    None => "",
};

/// The largest report `send` will post; the collector's cap is the same number.
pub const MAX_BYTES: usize = 32 * 1024;

/// Newest sessions per car a report carries.
const MAX_SESSIONS: i64 = 5;

/// Seconds between two sends from one install, however often the report changes: the collector runs on a free plan, and a
/// report is a summary, not a feed.
pub const MIN_GAP: f64 = 86400.0;

fn consent_path(root: &Path) -> PathBuf {
    root.join("sharing.json")
}

fn saved(root: &Path) -> Value {
    std::fs::read_to_string(consent_path(root)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null)
}

fn write_saved(root: &Path, doc: &Value) -> Result<(), ServiceError> {
    std::fs::create_dir_all(root)?;
    let tmp = consent_path(root).with_extension("json.tmp");
    std::fs::write(&tmp, format!("{}\n", dumps(doc, Json::Indent(2))))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?; // the install id is for this person's eyes
    }
    std::fs::rename(&tmp, consent_path(root))?;
    Ok(())
}

fn endpoint() -> Option<String> {
    let url = std::env::var("HONDASCAN_SHARE_URL").ok().filter(|u| !u.is_empty()).unwrap_or_else(|| DEFAULT_ENDPOINT.to_string());
    (!url.is_empty()).then_some(url)
}

/// `{asked, enabled, install_id, since, last_sent, can_send}`. `asked`: the person has answered the first-launch question
/// either way (so it is never asked again). `can_send`: this build has somewhere to send to (without one nothing is asked or
/// sent); the address itself is not reported. A damaged file counts as not asked and off.
pub fn status(root: &Path) -> Value {
    let s = saved(root);
    let on = s["enabled"].as_bool() == Some(true) && s["install_id"].as_str().is_some_and(|i| !i.is_empty());
    json!({
        "asked": s["asked"].as_bool() == Some(true),
        "enabled": on,
        "install_id": if on { s["install_id"].clone() } else { Value::Null },
        "since": if on { s["since"].clone() } else { Value::Null },
        "last_sent": if on { s["last_sent"].clone() } else { Value::Null },
        "can_send": endpoint().is_some(),
    })
}

/// The person's answer: Share (`true`, a fresh random install id kept if already on) or No thanks (`false`, the id is
/// forgotten). Either way the question is answered for good; a later `answer` changes it.
pub fn answer(root: &Path, enabled: bool) -> Result<Value, ServiceError> {
    let before = saved(root);
    let doc = if enabled {
        let keep = before["enabled"].as_bool() == Some(true) && before["install_id"].as_str().is_some_and(|i| !i.is_empty());
        if keep {
            let mut d = before.clone();
            d["asked"] = json!(true);
            d
        } else {
            json!({"asked": true, "enabled": true, "install_id": random_id(), "since": now()})
        }
    } else {
        json!({"asked": true, "enabled": false})
    };
    write_saved(root, &doc)?;
    Ok(status(root))
}

/// 128 random bits as hex: the kernel's source where there is one, else the clock, process id and the std hasher's
/// per-process random keys, hashed. It names an install; it protects nothing.
fn random_id() -> String {
    let mut bytes = [0u8; 16];
    if std::fs::File::open("/dev/urandom").and_then(|mut f| std::io::Read::read_exact(&mut f, &mut bytes)).is_ok() {
        return bytes.iter().map(|b| format!("{b:02x}")).collect();
    }
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u64(now().to_bits());
    h.write_u32(std::process::id());
    sha256_hex(&h.finish().to_le_bytes())[..32].to_string()
}

fn simulated(adapter: &str) -> bool {
    crate::preflight::simulated(adapter)
}

/// The adapter's type from the `-a` spec the session recorded (`gd101-j2534:/dev/ttyACM0` -> `gd101-j2534`): never a device
/// path, a port name or a DLL path.
fn adapter_kind(spec: &str) -> String {
    match spec.split_once(':') {
        Some((head, _)) if head.len() > 1 && head.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') => head.to_string(),
        None if !spec.is_empty() && spec.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') && !spec.to_ascii_uppercase().starts_with("COM") => spec.to_string(),
        _ => "other".to_string(),
    }
}

/// The car as the report names it: everything but the check digit and the serial. A VIN that is not 17 characters (the car did
/// not give it) is `{"vin_read": false}`.
fn car_of(vin: &str) -> Value {
    let chars: Vec<char> = vin.chars().collect();
    if chars.len() != 17 {
        return json!({"vin_read": false});
    }
    let part = |a: usize, b: usize| chars[a..b].iter().collect::<String>();
    json!({"vin_read": true, "wmi": part(0, 3), "vds": part(3, 8), "model_year": part(9, 10), "plant": part(10, 11)})
}

/// `s` with each known VIN, the data directory and the home directory replaced, then any 17-character VIN-shaped word.
fn scrub_text(s: &str, secrets: &[(String, &'static str)], vin_shape: &Regex) -> String {
    let mut t = s.to_string();
    for (needle, with) in secrets {
        if !needle.is_empty() {
            t = t.replace(needle.as_str(), with);
        }
    }
    vin_shape.replace_all(&t, "<vin>").into_owned()
}

/// Replaces what identifies a person or a car in every string of `v`, object keys included (a read that went unanswered is
/// keyed by text).
fn scrub(v: &mut Value, secrets: &[(String, &'static str)], vin_shape: &Regex) {
    match v {
        Value::String(s) => *s = scrub_text(s, secrets, vin_shape),
        Value::Array(a) => a.iter_mut().for_each(|x| scrub(x, secrets, vin_shape)),
        Value::Object(o) => {
            let old = std::mem::take(o);
            for (k, mut x) in old {
                scrub(&mut x, secrets, vin_shape);
                o.insert(scrub_text(&k, secrets, vin_shape), x);
            }
        }
        _ => {}
    }
}

/// The keys of a session summary the report keeps: how the adapter and the car behaved, nothing about the drive.
const SESSION_KEYS: [&str; 8] = ["duration_s", "scan_s", "reads", "rejected", "unanswered", "latency_ms", "missing", "wakeup"];

fn session_entry(s: &Value) -> Value {
    let summary = &s["summary"];
    let mut m = Map::new();
    m.insert("adapter".into(), json!(adapter_kind(s["adapter"].as_str().unwrap_or(""))));
    m.insert("end_reason".into(), s["end_reason"].clone());
    for k in SESSION_KEYS {
        if !summary[k].is_null() {
            m.insert(k.into(), summary[k].clone());
        }
    }
    Value::Object(m)
}

/// The control units of the car as the newest real session found them, with their ids and part numbers (never the serial
/// numbers the identification services also return).
fn ecus_of(store: &Store, vin: &str, newest: &Value) -> Vec<Value> {
    let ids = store.identities(vin);
    newest["summary"]["ecus"]
        .as_array()
        .map(|a| a.as_slice())
        .unwrap_or(&[])
        .iter()
        .map(|e| {
            let mut m = Map::new();
            for k in ["address", "system_type", "system", "display", "rule"] {
                m.insert(k.into(), e[k].clone());
            }
            if let Some(i) = ids.iter().rev().find(|i| i["address"] == e["address"] && i["system"] == e["system"]) {
                for k in ["ecu_id", "ecu_model", "part_number"] {
                    m.insert(k.into(), i[k].clone());
                }
            }
            Value::Object(m)
        })
        .collect()
}

/// The report for the store at `root`, signed with `install_id`, or None when no session on a real adapter has ended (there is
/// nothing to say about a car yet).
pub fn bundle(store: &Store, root: &Path, install_id: &str) -> Option<Value> {
    let vins: Vec<String> = store.vehicles().iter().filter_map(|v| v["vin"].as_str().map(String::from)).collect();
    let mut cars = vec![];
    for vin in &vins {
        let sessions: Vec<Value> =
            store.sessions(vin, MAX_SESSIONS * 3).into_iter().filter(|s| !s["ended"].is_null() && !simulated(s["adapter"].as_str().unwrap_or(""))).take(MAX_SESSIONS as usize).collect();
        let Some(newest) = sessions.first() else { continue };
        let codes: Vec<Value> = store
            .dtc_history(vin)
            .into_iter()
            .map(|c| json!({"system": c["system"], "code": c["code"], "failure_type": c["failure_type"], "kind": c["kind"], "description": c["description"], "sessions": c["sessions"]}))
            .collect();
        cars.push(json!({"car": car_of(vin), "ecus": ecus_of(store, vin, newest), "codes": codes, "sessions": sessions.iter().map(session_entry).collect::<Vec<_>>()}));
    }
    if cars.is_empty() {
        return None;
    }
    let mut doc = json!({
        "schema": SCHEMA,
        "kind": "hondascan-car-report",
        "version": env!("CARGO_PKG_VERSION"),
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "install_id": install_id,
        "cars": cars,
    });
    let mut secrets: Vec<(String, &'static str)> = vins.into_iter().map(|v| (v, "<vin>")).collect();
    secrets.push((root.display().to_string(), "<data>"));
    for var in ["HOME", "USERPROFILE"] {
        if let Ok(h) = std::env::var(var) {
            if h.len() > 1 {
                secrets.push((h, "~"));
            }
        }
    }
    // Longest first, so the data directory is not half-replaced by the home directory that contains it.
    secrets.sort_by_key(|(s, _)| std::cmp::Reverse(s.len()));
    scrub(&mut doc, &secrets, &Regex::new(r"[A-HJ-NPR-Z0-9]{17}").unwrap());
    Some(doc)
}

/// The text `send` posts and `preview` shows.
pub fn render(doc: &Value) -> String {
    format!("{}\n", dumps(doc, Json::Indent(2)))
}

/// What a send would post right now, for `preview` and `export`: the report with the person's install id, or a one-off id
/// when sharing is off. None when there is no car to report.
pub fn preview(store: &Store, root: &Path) -> Option<String> {
    let st = status(root);
    let id = st["install_id"].as_str().map(String::from).unwrap_or_else(|| "preview".to_string());
    bundle(store, root, &id).map(|d| render(&d))
}

/// Where a report may be posted: https, or http to a loopback host.
fn check_endpoint(url: &str) -> Result<(), ServiceError> {
    if url.starts_with("https://") {
        return Ok(());
    }
    let local = ["http://127.0.0.1:", "http://127.0.0.1/", "http://localhost:", "http://localhost/", "http://[::1]:"];
    if local.iter().any(|p| url.starts_with(p)) {
        return Ok(());
    }
    Err(ServiceError::value("the sharing endpoint must be https (or http on this machine)"))
}

/// The report to send now, when one is due: sharing is on, an endpoint exists, a car has a finished real session, the
/// report differs from the last one sent, and the last send was at least MIN_GAP ago. Reads only.
pub fn due(store: &Store, root: &Path) -> Option<String> {
    let st = status(root);
    if st["enabled"] != json!(true) || st["can_send"] != json!(true) {
        return None;
    }
    let before = saved(root);
    if before["last_sent"].as_f64().is_some_and(|t| now() - t < MIN_GAP) {
        return None;
    }
    let text = render(&bundle(store, root, st["install_id"].as_str()?)?);
    (before["sent_digest"].as_str() != Some(sha256_hex(text.as_bytes()).as_str())).then_some(text)
}

/// Post `body` to the configured endpoint, if sharing is on, and remember that this exact report went. Returns
/// `{bytes, sent: true}`. Refuses without consent, without an endpoint, over `MAX_BYTES`, or when `curl` fails or the
/// server answers outside 2xx.
pub fn send(root: &Path, body: &str) -> Result<Value, ServiceError> {
    if status(root)["enabled"] != json!(true) {
        return Err(ServiceError::value("sharing is off: turn it on first (hondascan share on)"));
    }
    let Some(url) = endpoint() else {
        return Err(ServiceError::value("no sharing endpoint is configured in this build (a build from source has none; `hondascan share export` writes the report to a file you can attach to an issue)"));
    };
    check_endpoint(&url)?;
    if body.len() > MAX_BYTES {
        return Err(ServiceError::value(format!("the report is {} bytes, over the {MAX_BYTES} limit: use `hondascan share export`", body.len())));
    }
    let mut child = Command::new("curl")
        // TLS 1.2 or newer, no redirects followed (a redirect could send the report somewhere else), no reply body kept
        .args(["--silent", "--show-error", "--fail", "--max-time", "30", "--tlsv1.2", "--max-redirs", "0", "--proto", if url.starts_with("https://") { "=https" } else { "=http" }, "-X", "POST", "-H", "Content-Type: application/json", "--data-binary", "@-", &url])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| ServiceError { kind: "OSError".into(), message: format!("curl could not be started: {e}") })?;
    child.stdin.take().expect("piped").write_all(body.as_bytes())?;
    let done = child.wait_with_output()?;
    if !done.status.success() {
        let why = String::from_utf8_lossy(&done.stderr).trim().to_string();
        return Err(ServiceError { kind: "TransportError".into(), message: format!("the upload failed: {why}") });
    }
    let mut doc = saved(root);
    doc["sent_digest"] = json!(sha256_hex(body.as_bytes()));
    doc["last_sent"] = json!(now());
    write_saved(root, &doc)?;
    Ok(json!({"sent": true, "bytes": body.len()}))
}
