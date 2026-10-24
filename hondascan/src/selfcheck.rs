//! The installation self-check, shared by `self_check` (rpc.rs) and `selfcheck` (cli.rs): what this backend needs to do
//! its job, checked where it runs.
//!
//! A check is `ok`, `warn` or `fail`. A `warn` is something missing that one way of using the tool still works without
//! (no serial adapter when SocketCAN and the simulated car work, no udev rule, no C compiler for the J2534 driver, a
//! report left in `audit/pending.json` from a killed process); a `fail` is something broken (the database will not open
//! or is damaged, the data directory cannot be written). The overall `ok` is false only for a `fail`.
//!
//! Read-only apart from the two write probes (`data-dir`, `audit`): each creates, fsyncs and removes one temporary file,
//! because a data directory the tool cannot write is the one thing every other part of it depends on.
use crate::db::Database;
use crate::transport::serial_ports;
use serde_json::{json, Value};
use std::io::Write;
use std::path::Path;

/// The checks in the order the app and the CLI show them.
const CHECKS: [(&str, &str); 9] = [
    ("backend/version", "Backend version"),
    ("database", "Database"),
    ("data-dir", "Data directory"),
    ("audit", "Audit directory"),
    ("resources", "Bundled resources"),
    ("serial", "Serial adapters"),
    ("udev", "udev rule"),
    ("compiler", "C compiler"),
    ("pending", "Unfinished operation"),
];

/// The installation check as the app's `SelfCheck` and the CLI's `selfcheck` read it:
/// `{ok, version, data_dir, checks: [{id, title, status, detail}]}` in `CHECKS` order. `root` is the data directory
/// (`store::data_dir()`), `db` the catalog database when it could be opened.
pub fn run(root: &Path, db: Option<&Database>) -> Value {
    let checks: Vec<Value> = CHECKS
        .iter()
        .map(|(id, title)| {
            let (status, detail) = match *id {
                "backend/version" => ("ok", env!("CARGO_PKG_VERSION").to_string()),
                "database" => database(db),
                "data-dir" => writable(root),
                "audit" => writable(&root.join("audit")),
                "resources" => resources(),
                "serial" => serial(),
                "udev" => udev(),
                "compiler" => compiler(),
                _ => pending(root),
            };
            json!({"id": id, "title": title, "status": status, "detail": detail})
        })
        .collect();
    let ok = !checks.iter().any(|c| c["status"] == "fail");
    json!({"ok": ok, "version": env!("CARGO_PKG_VERSION"), "data_dir": root.to_string_lossy(), "checks": checks})
}

/// The catalog database every scan reads: `quick_check` must say the file is sound, and a real query must answer.
///
/// The count is of the `dtc` table: `dtc_events`, the maintenance contract's wording, lives in the store's `history.db`,
/// not in this file, and a fresh data directory has no `history.db` yet — checking one here would fail the self-check
/// the packaging runs against an empty `HONDASCAN_HOME` (hondascan-gui/src/bin/package.rs).
fn database(db: Option<&Database>) -> (&'static str, String) {
    let Some(db) = db else {
        return ("fail", "hondascan.db could not be opened (build it with `cargo run --release --bin build-db`)".into());
    };
    match db.query("PRAGMA quick_check", [], |r| r.get::<_, String>(0)).first().map(String::as_str) {
        Some("ok") => {}
        Some(other) => return ("fail", format!("hondascan.db quick_check: {other}")),
        None => return ("fail", "hondascan.db quick_check returned nothing".into()),
    }
    match db.query_one("SELECT COUNT(*) FROM dtc", [], |r| r.get::<_, i64>(0)) {
        Some(n) => ("ok", format!("hondascan.db ok, {n} trouble codes defined")),
        None => ("fail", "hondascan.db: the dtc table could not be read".into()),
    }
}

/// Create `dir` and prove it writable: one temporary file, written, fsynced and removed. The name carries the pid so
/// two checks (or two processes) never collide.
fn writable(dir: &Path) -> (&'static str, String) {
    let probe = || -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let tmp = dir.join(format!(".hondascan-write-test-{}", std::process::id()));
        let mut f = std::fs::File::create(&tmp)?;
        let r = f.write_all(b"hondascan\n").and_then(|_| f.sync_all());
        drop(f);
        let _ = std::fs::remove_file(&tmp);
        r
    };
    match probe() {
        Ok(()) => ("ok", format!("{} is writable", dir.display())),
        Err(e) => ("fail", format!("{}: {e}", dir.display())),
    }
}

/// The file the tool needs beside it (packaged) or in the crate (from a checkout): the GD101 J2534 driver's source. Missing is
/// a warn: the simulated car and SocketCAN do not need it. (Recorded-car profiles are optional: you make your own.)
fn resources() -> (&'static str, String) {
    if crate::resource("drivers/gd101_j2534.c").exists() {
        ("ok", "drivers/gd101_j2534.c found".into())
    } else {
        ("warn", "drivers/gd101_j2534.c not found (the simulated car and SocketCAN still work)".into())
    }
}

/// Serial adapters (ELM327, the GODIAG GD101): none is a warn, since SocketCAN and the simulated car work without one.
fn serial() -> (&'static str, String) {
    let ports = serial_ports();
    if ports.is_empty() {
        return ("warn", "no serial adapter found; SocketCAN and the simulated car still work".into());
    }
    let names: Vec<String> = ports.iter().map(|p| p.0.clone()).collect();
    ("ok", names.join(", "))
}

/// The udev rule (packaging/60-hondascan.rules) that tells ModemManager to leave the adapter alone. Missing is a warn:
/// the adapter still works, until ModemManager probes it.
fn udev() -> (&'static str, String) {
    #[cfg(target_os = "linux")]
    {
        for dir in ["/etc/udev/rules.d", "/usr/lib/udev/rules.d", "/lib/udev/rules.d"] {
            let Ok(entries) = std::fs::read_dir(dir) else { continue };
            for e in entries.filter_map(Result::ok) {
                let name = e.file_name().to_string_lossy().into_owned();
                if !name.ends_with(".rules") {
                    continue;
                }
                if std::fs::read_to_string(e.path()).is_ok_and(|t| t.to_ascii_lowercase().contains("hondascan")) {
                    return ("ok", format!("{dir}/{name}"));
                }
            }
        }
        ("warn", "no hondascan rule under /etc/udev/rules.d or /usr/lib/udev/rules.d; ModemManager may grab the adapter".into())
    }
    #[cfg(not(target_os = "linux"))]
    {
        ("ok", "no udev on this platform".into())
    }
}

/// A C compiler for the J2534 driver (built once on the target): `cc`, `gcc` or `clang` on PATH. Missing is a warn.
fn compiler() -> (&'static str, String) {
    let Some(path) = std::env::var_os("PATH") else { return ("warn", "no cc, gcc or clang on PATH (the GD101's J2534 driver cannot be built)".into()) };
    for name in ["cc", "gcc", "clang"] {
        if std::env::split_paths(&path).any(|dir| executable(&dir.join(name))) {
            return ("ok", format!("{name} found"));
        }
    }
    ("warn", "no cc, gcc or clang on PATH (the GD101's J2534 driver cannot be built)".into())
}

/// A regular file with an execute bit (on Unix), or any regular file elsewhere.
fn executable(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else { return false };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// A write or actuator test the last process left in flight (`audit/pending.json`). A warn either way: nothing is
/// checked on the car here, and the next connection hands it back (service::connect).
fn pending(root: &Path) -> (&'static str, String) {
    let Some(p) = crate::audit::read_pending(root) else {
        return ("ok", "no write or actuator was left in flight".into());
    };
    if let Some(error) = p["error"].as_str() {
        return ("warn", format!("the pending record could not be read: {error}"));
    }
    let kind = p["kind"].as_str().unwrap_or("operation");
    let what = match kind {
        "write" => format!("write '{}' (key {})", p["name"].as_str().unwrap_or("?"), p["key"].as_str().unwrap_or("?")),
        "actuator" => format!("actuator test '{}'", p["test"].as_str().or(p["name"].as_str()).unwrap_or("?")),
        other => format!("{other} '{}'", p["name"].as_str().unwrap_or("?")),
    };
    let vin = p["vin"].as_str().unwrap_or("");
    ("warn", format!("a {what} on {vin} was left in flight; the next connection hands it back"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_probe_creates_writes_syncs_and_removes_its_file() {
        let dir = std::env::temp_dir().join(format!("hs-rs-{}-selfcheck-unit", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (status, detail) = writable(&dir.join("fresh/nested"));
        assert_eq!(status, "ok", "{detail}");
        let left: Vec<_> = std::fs::read_dir(dir.join("fresh/nested")).unwrap().filter_map(Result::ok).collect();
        assert!(left.is_empty(), "the probe left {} entries behind", left.len());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_in_place_of_the_directory_is_a_failed_check() {
        let dir = std::env::temp_dir().join(format!("hs-rs-{}-selfcheck-file", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("audit"), b"not a directory").unwrap();
        let (status, detail) = writable(&dir.join("audit"));
        assert_eq!(status, "fail");
        assert!(!detail.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_pending_record_names_the_operation_that_was_left_in_flight() {
        let dir = std::env::temp_dir().join(format!("hs-rs-{}-selfcheck-pending", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(pending(&dir).0, "ok");
        crate::audit::write_pending(&dir, &json!({"schema": 1, "kind": "write", "vin": "V", "name": "E-VTC learn", "key": "evtc_learn"})).unwrap();
        let (status, detail) = pending(&dir);
        assert_eq!(status, "warn");
        assert!(detail.contains("write") && detail.contains("E-VTC learn") && detail.contains("evtc_learn") && detail.contains("hands it back"), "{detail}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
