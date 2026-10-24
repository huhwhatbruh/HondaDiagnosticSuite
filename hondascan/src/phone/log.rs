//! The gateway's two log files (README, "Phone app").
use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

const LOG_CAP: u64 = 50 * 1024 * 1024;

/// An append-only JSON-lines file, capped in size. The gateway writes `phone-gateway.jsonl` (every request, how long the
/// backend takes to answer a snapshot) and the app's measurements land in `phone-client.jsonl` (POST /v1/log), so that
/// both ends of the link are in one place when comparing Wi-Fi with another transport. A log that cannot be opened is
/// silent: logging must never stop the gateway.
pub(crate) struct Log {
    file: Option<Mutex<std::fs::File>>,
    written: AtomicU64,
}

impl Log {
    fn open(dir: Option<&Path>, name: &str) -> Log {
        let file = dir.and_then(|d| {
            std::fs::create_dir_all(d).ok()?;
            std::fs::OpenOptions::new().create(true).append(true).open(d.join(name)).ok()
        });
        let written = file.as_ref().and_then(|f| f.metadata().ok()).map_or(0, |m| m.len());
        Log { file: file.map(Mutex::new), written: AtomicU64::new(written) }
    }

    /// One line, with the time added. False when there is no file or the cap is reached.
    pub(crate) fn write(&self, mut v: Value) -> bool {
        let Some(f) = &self.file else { return false };
        if self.written.load(Ordering::Relaxed) >= LOG_CAP {
            return false;
        }
        v["t"] = json!(SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64()));
        let line = format!("{v}\n");
        self.written.fetch_add(line.len() as u64, Ordering::Relaxed);
        f.lock().unwrap().write_all(line.as_bytes()).is_ok()
    }
}

pub(crate) struct Logs {
    pub(crate) gateway: Log,
    pub(super) client: Log,
}

impl Logs {
    pub(super) fn open(dir: Option<&Path>) -> Logs {
        Logs { gateway: Log::open(dir, "phone-gateway.jsonl"), client: Log::open(dir, "phone-client.jsonl") }
    }
}

/// Where the logs go: the directory in HONDASCAN_PHONE_LOG_DIR, and nowhere when it is not set (or is `off`). Logging is for
/// measuring the link (README, "Phone app"); left on it would add a line per request and grow without end.
pub fn log_dir() -> Option<PathBuf> {
    match std::env::var("HONDASCAN_PHONE_LOG_DIR") {
        Ok(d) if d.is_empty() || d == "off" => None,
        Ok(d) => Some(PathBuf::from(d)),
        Err(_) => None,
    }
}
