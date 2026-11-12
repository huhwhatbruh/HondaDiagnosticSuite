//! The last errors the app's requests returned (`rpc::Session::call`), kept on this computer in `errors.json` in the data
//! directory, so the opt-in car report (share.rs) can say what went wrong on machines where nothing worked: an adapter that
//! never connected leaves no session to report. Bounded: the newest `KEEP` errors, each message cut to `MAX_MESSAGE` characters.
//! Errors with the simulator and errors in a request's own parameters (`TypeError`) are not kept.
use regex::Regex;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub const KEEP: usize = 50;
pub const MAX_MESSAGE: usize = 200;
/// Distinct errors a report carries (worker.js LIMITS.errors).
pub const MAX_IN_REPORT: usize = 20;

fn path(root: &Path) -> PathBuf {
    root.join("errors.json")
}

/// The kept errors, oldest first: `{t, method, adapter, kind, message}`.
pub fn read(root: &Path) -> Vec<Value> {
    std::fs::read_to_string(path(root)).ok().and_then(|t| serde_json::from_str::<Vec<Value>>(&t).ok()).unwrap_or_default()
}

/// Keep one error. `adapter` is the adapter's kind (`elm`, `gd101-j2534`), never its port. Best effort: a store that cannot be
/// written loses the entry, never the request.
pub fn record(root: &Path, method: &str, adapter: &str, kind: &str, message: &str) {
    if kind == "TypeError" || crate::preflight::simulated(adapter) {
        return;
    }
    let mut all = read(root);
    all.push(json!({"t": crate::transport::can::now(), "method": method, "adapter": adapter, "kind": kind,
                    "message": message.chars().take(MAX_MESSAGE).collect::<String>()}));
    let drop = all.len().saturating_sub(KEEP);
    let all: Vec<Value> = all.into_iter().skip(drop).collect();
    if std::fs::create_dir_all(root).is_ok() {
        let tmp = path(root).with_extension("json.tmp");
        if std::fs::write(&tmp, serde_json::to_string(&all).unwrap_or_default()).is_ok() {
            let _ = std::fs::rename(&tmp, path(root));
        }
    }
}

/// The kept errors as a report has them: each distinct `{method, adapter, kind, message}` once with its `count`, newest first,
/// at most `MAX_IN_REPORT`, no times, and port names (`/dev/ttyUSB0`, `COM3`) replaced by `<port>`. The report's own scrub
/// takes out VINs and the data and home directories afterwards.
pub fn summary(root: &Path) -> Vec<Value> {
    let port = Regex::new(r"/dev/[A-Za-z0-9_./-]+|\\\\\.\\COM[0-9]+|\bCOM[0-9]+\b").unwrap();
    let mut out: Vec<Value> = vec![];
    for e in read(root).iter().rev() {
        let message = port.replace_all(e["message"].as_str().unwrap_or(""), "<port>").into_owned();
        let key = json!({"method": e["method"], "adapter": e["adapter"], "kind": e["kind"], "message": message});
        let same = |o: &Value| ["method", "adapter", "kind", "message"].iter().all(|f| o[*f] == key[*f]);
        match out.iter().position(same) {
            Some(i) => out[i]["count"] = json!(out[i]["count"].as_u64().unwrap_or(0) + 1),
            None if out.len() < MAX_IN_REPORT => {
                let mut k = key;
                k["count"] = json!(1);
                out.push(k);
            }
            None => {}
        }
    }
    out
}
