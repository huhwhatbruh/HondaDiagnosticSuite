//! The write/actuator audit trail: one append-only JSON-lines file and one in-flight pending record, both under
//! `<data dir>/audit/`.
//!
//! The audit is the record of what was sent to a car, written before anything goes out and again after the run; the
//! pending record is what lets the next connection (or the app after a crash) hand control back when the process that
//! started a write or actuator test did not get to finish. Both are durability points, not logs: a caller that cannot
//! write them must not start the operation (`append` and `write_pending` return the error; callers turn it into a
//! refusal).
//!
//! `writes.jsonl` gets one compact JSON object per line, `sync_data`d before the call returns. `pending.json` is
//! written to a temporary file in the same directory, synced, renamed over the old one and the directory fsynced, so a
//! power loss leaves either the previous record or the new one, never half of one. A malformed pending file is read as
//! an object with an `error` string and its raw text: whoever finds it must report it and must not act on it.
//!
//! The records use `crate::transport::can::now()` (Unix seconds as f64) so they line up with the capture logs.
use crate::transport::can::now;
use crate::value::{dumps, Json};
use serde_json::{json, Value};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// The audit schema every record carries. A new field may be added; a change to an existing one needs a new number.
pub const SCHEMA: u64 = 1;

/// A new audit record with the fields every line carries: `schema`, `at`, `pid`, `version`, `event`. Callers add the
/// event's own fields (see the shapes in `README.md` and the maintenance contract).
pub fn record(event: &str) -> Value {
    json!({
        "schema": SCHEMA,
        "at": now(),
        "pid": std::process::id(),
        "version": env!("CARGO_PKG_VERSION"),
        "event": event,
    })
}

fn audit_dir(root: &Path) -> PathBuf {
    root.join("audit")
}

/// `<root>/audit/writes.jsonl`.
pub fn writes_path(root: &Path) -> PathBuf {
    audit_dir(root).join("writes.jsonl")
}

/// `<root>/audit/pending.json`.
pub fn pending_path(root: &Path) -> PathBuf {
    audit_dir(root).join("pending.json")
}

/// Append one compact JSON line to `audit/writes.jsonl` and sync it before returning. Creates `audit/` when needed.
/// An error here means the line is not durably stored: the caller must refuse to send anything.
pub fn append(root: &Path, record: &Value) -> io::Result<()> {
    let dir = audit_dir(root);
    std::fs::create_dir_all(&dir)?;
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(writes_path(root))?;
    // One write for the whole line: with O_APPEND another process's line cannot land between the JSON and its newline.
    let mut line = dumps(record, Json::Compact);
    line.push('\n');
    f.write_all(line.as_bytes())?;
    f.sync_data()
}

/// The audit records, newest first. Lines that are not a JSON object are skipped (a torn last line must not hide the
/// rest); `vin` filters on the record's `vin` field and `limit == 0` means all.
pub fn read(root: &Path, vin: Option<&str>, limit: usize) -> Vec<Value> {
    let text = std::fs::read_to_string(writes_path(root)).unwrap_or_default();
    let mut out: Vec<Value> = text
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|v| v.is_object())
        .filter(|v| vin.is_none_or(|w| v.get("vin").and_then(Value::as_str).is_some_and(|v| v.eq_ignore_ascii_case(w))))
        .collect();
    out.reverse();
    if limit > 0 {
        out.truncate(limit);
    }
    out
}

fn sync_dir(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        std::fs::File::open(dir)?.sync_all()
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
        Ok(())
    }
}

/// Read the pending record. `None` only when there is no file: an unreadable or malformed file is returned as an
/// object with `error` and the raw text, so the caller can report it and refuse to act on it.
pub fn read_pending(root: &Path) -> Option<Value> {
    let path = pending_path(root);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return None,
        Err(e) => {
            return Some(json!({"error": format!("the pending record could not be read: {e}"), "raw": ""}));
        }
    };
    match serde_json::from_str::<Value>(&text) {
        Ok(v) if v.is_object() => Some(v),
        Ok(_) => Some(json!({"error": "the pending record is not a JSON object", "raw": text})),
        Err(e) => Some(json!({"error": format!("the pending record is not valid JSON: {e}"), "raw": text})),
    }
}

/// Durably replace `audit/pending.json`: a temporary file in the same directory, `sync_all`, rename, then the
/// directory fsynced on Unix. On an error the old record (if any) is left in place and the temporary file removed.
pub fn write_pending(root: &Path, pending: &Value) -> io::Result<()> {
    let dir = audit_dir(root);
    std::fs::create_dir_all(&dir)?;
    let dst = pending_path(root);
    let tmp = dir.join(format!(".pending-{}.tmp", std::process::id()));
    let write = (|| -> io::Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(dumps(pending, Json::Compact).as_bytes())?;
        f.write_all(b"\n")?;
        f.sync_all()
    })();
    if let Err(e) = write {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    #[cfg(windows)]
    {
        // std::fs::rename does not replace an existing file on Windows.
        if dst.exists() {
            std::fs::remove_file(&dst)?;
        }
    }
    if let Err(e) = std::fs::rename(&tmp, &dst) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    sync_dir(&dir)
}

/// Remove `audit/pending.json` and fsync the directory. Missing is success (the record is gone either way).
pub fn clear_pending(root: &Path) -> io::Result<()> {
    let dir = audit_dir(root);
    match std::fs::remove_file(pending_path(root)) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    if dir.is_dir() {
        sync_dir(&dir)?;
    }
    Ok(())
}

// -- SHA-256 -----------------------------------------------------------------------------------------------------------
//
// FIPS 180-4, written out here because the crate keeps its dependency list small (maintenance contract: no new
// dependencies). Used by backups and the health report's baseline integrity check.

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// SHA-256 of `data` as lowercase hex.
pub fn sha256_hex(data: &[u8]) -> String {
    let mut h: [u32; 8] = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    let bits = (data.len() as u64).wrapping_mul(8);
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bits.to_be_bytes());
    for chunk in msg.as_chunks::<64>().0 {
        let mut w = [0u32; 64];
        for (i, word) in chunk.as_chunks::<4>().0.iter().enumerate() {
            w[i] = u32::from_be_bytes(*word);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) = (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);
        for (i, k) in K.iter().enumerate() {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(*k).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            (hh, g, f, e, d, c, b, a) = (g, f, e, d.wrapping_add(t1), c, b, a, t1.wrapping_add(t2));
        }
        for (dst, add) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *dst = dst.wrapping_add(add);
        }
    }
    h.iter().map(|x| format!("{x:08x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("hs-rs-{}-audit-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn sha256_matches_the_known_vectors() {
        assert_eq!(sha256_hex(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        // Two blocks: the 56-byte FIPS example, so the padding lands in a second block.
        assert_eq!(
            sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    #[test]
    fn append_read_filter_and_limit() {
        let root = tmp("read");
        let mut a = record("attempt");
        a["vin"] = json!("VIN-A");
        a["n"] = json!(1);
        let mut b = record("result");
        b["vin"] = json!("VIN-B");
        b["n"] = json!(2);
        append(&root, &a).unwrap();
        append(&root, &b).unwrap();
        // A torn line is skipped, not fatal.
        let mut f = std::fs::OpenOptions::new().append(true).open(writes_path(&root)).unwrap();
        f.write_all(b"{not json\n").unwrap();
        drop(f);

        let all = read(&root, None, 0);
        assert_eq!(all.len(), 2);
        assert_eq!(all[0]["n"], 2, "newest first");
        assert_eq!(all[1]["n"], 1);
        assert_eq!(all[0]["schema"], 1);
        assert_eq!(all[0]["version"], env!("CARGO_PKG_VERSION"));
        assert!(all[0]["at"].as_f64().unwrap() > 0.0);
        assert_eq!(read(&root, Some("vin-a"), 0).len(), 1);
        assert_eq!(read(&root, Some("VIN-A"), 4)[0]["n"], 1);
    }

    #[test]
    fn pending_round_trip_and_malformed() {
        let root = tmp("pending");
        assert_eq!(read_pending(&root), None);
        let pending = json!({"schema":1, "kind":"write", "vin":"VIN-A", "address":16});
        write_pending(&root, &pending).unwrap();
        assert_eq!(read_pending(&root).unwrap(), pending);
        clear_pending(&root).unwrap();
        assert_eq!(read_pending(&root), None);
        clear_pending(&root).unwrap(); // already gone: still success

        std::fs::create_dir_all(audit_dir(&root)).unwrap();
        std::fs::write(pending_path(&root), b"{cut short").unwrap();
        let bad = read_pending(&root).unwrap();
        assert!(bad["error"].as_str().unwrap().contains("JSON"), "{bad}");
        assert_eq!(bad["raw"], "{cut short");
    }

    #[test]
    fn a_file_in_place_of_the_audit_directory_refuses_the_writes() {
        let root = tmp("unwritable");
        std::fs::write(root.join("audit"), b"not a directory").unwrap();
        assert!(append(&root, &record("attempt")).is_err());
        assert!(write_pending(&root, &json!({"schema":1})).is_err());
    }
}
