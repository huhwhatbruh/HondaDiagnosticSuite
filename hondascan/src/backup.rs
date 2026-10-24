//! The export a person keeps: the store's history as one readable JSON file, and a `.tar.gz` of the data directory that
//! can be unpacked over it (with the app closed) to get the same data back.
//!
//! The archive is a plain ustar stream written here - 512-byte headers and gzip via `flate2` - because the layout is
//! simple enough that hand-writing the format is less code than a `tar` dependency, and `history.db` inside it is a
//! `VACUUM INTO` snapshot rather than a copy of a database that may have been open while it was taken. Every file entry
//! is hashed into `manifest.json`, and the archive's own hash goes into a `<backup>.sha256` sidecar in `sha256sum` form.
use crate::service::ServiceError;
use crate::store::Store;
use crate::transport::can::now;
use crate::value::{dumps, Json};
use flate2::write::GzEncoder;
use flate2::Compression;
use serde_json::{json, Map, Value};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

/// The export's JSON schema. A new key may be added; changing one needs a new number.
pub const SCHEMA: u64 = 1;

/// The largest size the 11-octal-digit size field of a ustar header can express (just under 8 GiB).
const MAX_SIZE: u64 = 0o77_777_777_777;

/// The JSON export and the backup archive, for a store at `root`. `out` names the JSON file: the archive is the same
/// path with its `.json` replaced by `.tar.gz` (anything else gets `.tar.gz` appended). Without `out` both live under
/// `root`, named after the time. Both paths are absolute and their directories are created.
pub fn export(store: &Store, root: &Path, out: Option<&Path>) -> Result<Value, ServiceError> {
    let created = now();
    let (json_path, backup_path) = paths(root, out, created as i64)?;
    for path in [&json_path, &backup_path] {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).map_err(|e| failed(dir, e))?;
        }
    }

    let vehicles = store.vehicles();
    let mut sessions = Map::new();
    let mut history = Map::new();
    let mut session_count = 0u64;
    for vehicle in &vehicles {
        let vin = vehicle["vin"].as_str().unwrap_or_default();
        let rows = store.sessions(vin, -1); // -1: all of them (-1 because LIMIT 0 would mean none)
        session_count += rows.len() as u64;
        sessions.insert(vin.to_string(), Value::Array(rows));
        history.insert(vin.to_string(), Value::Array(store.dtc_history(vin)));
    }
    let document = json!({
        "schema": SCHEMA,
        "generated": created,
        "version": env!("CARGO_PKG_VERSION"),
        "data_dir": root.display().to_string(),
        "vehicles": vehicles,
        "sessions": sessions,
        "dtc_history": history,
        "baselines": store.all_baselines(),
        "markers": store.all_markers(),
        "audit": crate::audit::read(root, None, 0),
    });
    let text = format!("{}\n", dumps(&document, Json::Indent(2)));
    std::fs::write(&json_path, &text).map_err(|e| failed(&json_path, e))?;

    let items = archive_items(store, root, created)?;
    let file = std::fs::File::create(&backup_path).map_err(|e| failed(&backup_path, e))?;
    let mut gz = GzEncoder::new(BufWriter::new(file), Compression::default());
    write_tar(&mut gz, &items)?;
    gz.finish()?.into_inner().map_err(|e| ServiceError::from(e.into_error()))?.sync_all()?;

    let archive = std::fs::read(&backup_path).map_err(|e| failed(&backup_path, e))?;
    let name = backup_path.file_name().unwrap_or_default().to_string_lossy().into_owned();
    let sha_path = with_suffix(&backup_path, ".sha256");
    std::fs::write(&sha_path, format!("{}  {name}\n", crate::audit::sha256_hex(&archive))).map_err(|e| failed(&sha_path, e))?;

    Ok(json!({
        "json_path": json_path.display().to_string(),
        "backup_path": backup_path.display().to_string(),
        "sessions": session_count,
        "vehicles": vehicles.len(),
        "bytes": text.len() as u64 + archive.len() as u64,
    }))
}

/// Where the two files go.
fn paths(root: &Path, out: Option<&Path>, unix: i64) -> Result<(PathBuf, PathBuf), ServiceError> {
    match out {
        Some(p) => {
            let json = std::path::absolute(p)?;
            let backup = match json.extension().and_then(|e| e.to_str()) {
                Some(e) if e.eq_ignore_ascii_case("json") => json.with_extension("tar.gz"),
                _ => with_suffix(&json, ".tar.gz"),
            };
            Ok((json, backup))
        }
        None => {
            let json = std::path::absolute(root.join(format!("exports/hondascan-export-{unix}.json")))?;
            let backup = std::path::absolute(root.join(format!("backups/hondascan-backup-{unix}.tar.gz")))?;
            Ok((json, backup))
        }
    }
}

/// `p` with `suffix` appended to its name, keeping every byte of the original path.
fn with_suffix(p: &Path, suffix: &str) -> PathBuf {
    let mut name = p.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}

/// An OS error as the caller reports it, naming the path that failed.
fn failed(path: &Path, e: std::io::Error) -> ServiceError {
    ServiceError { kind: "OSError".into(), message: format!("{}: {e}", path.display()) }
}

fn read_file(path: &Path) -> Result<Vec<u8>, ServiceError> {
    std::fs::read(path).map_err(|e| failed(path, e))
}

// -- what goes into the archive -----------------------------------------------------

/// One archive entry, in write order. A name of a directory keeps its trailing `/`.
enum Item {
    Dir(String),
    File { path: String, data: Vec<u8> },
}

/// The archive's contents: a snapshot of `history.db` first, then the recorded cars, the audit trail, the reports and
/// the app's settings when they exist, and the generated `README.txt` and `manifest.json` last.
fn archive_items(store: &Store, root: &Path, created: f64) -> Result<Vec<Item>, ServiceError> {
    let mut items = vec![Item::File { path: "history.db".into(), data: snapshot(store, root)? }];
    // Always present, even from a store that never recorded a drive: a restore should end with the same tree either way.
    items.push(Item::Dir("vehicles/".into()));
    let vehicles = root.join("vehicles");
    if vehicles.is_dir() {
        walk(&vehicles, "vehicles", &mut items)?;
    }
    for name in ["audit/writes.jsonl", "audit/pending.json"] {
        let path = root.join(name);
        if path.is_file() {
            items.push(Item::File { path: name.into(), data: read_file(&path)? });
        }
    }
    let reports = root.join("reports");
    if reports.is_dir() {
        items.push(Item::Dir("reports/".into()));
        walk(&reports, "reports", &mut items)?;
    }
    let gui = root.join("gui.json");
    if gui.is_file() {
        items.push(Item::File { path: "gui.json".into(), data: read_file(&gui)? });
    }

    items.push(Item::File { path: "README.txt".into(), data: readme(root, created).into_bytes() });
    let files: Vec<Value> = items
        .iter()
        .filter_map(|item| match item {
            Item::Dir(_) => None,
            Item::File { path, data } => Some(json!({"path": path, "size": data.len(), "sha256": crate::audit::sha256_hex(data)})),
        })
        .collect();
    let manifest = json!({
        "name": "hondascan",
        "version": env!("CARGO_PKG_VERSION"),
        "created": created,
        "target": {"os": std::env::consts::OS, "arch": std::env::consts::ARCH},
        "files": files,
    });
    items.push(Item::File { path: "manifest.json".into(), data: format!("{}\n", dumps(&manifest, Json::Indent(2))).into_bytes() });
    Ok(items)
}

/// A standalone copy of `history.db`, written into a temp file under `root`, read whole and removed again: the copy in
/// the archive must not be a database that was open while it was taken, and `VACUUM INTO` refuses to overwrite a file.
fn snapshot(store: &Store, root: &Path) -> Result<Vec<u8>, ServiceError> {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let tmp = root.join(format!(".hondascan-snapshot-{}-{}.db", std::process::id(), SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
    store
        .snapshot(&tmp)
        .map_err(|e| ServiceError { kind: "OperationalError".into(), message: format!("history.db could not be snapshotted: {e}") })?;
    let data = std::fs::read(&tmp);
    let _ = std::fs::remove_file(&tmp);
    data.map_err(|e| failed(&tmp, e))
}

/// Every directory and file under `dir` (in name order) as entries under `prefix`. A directory gets an entry even when
/// it is empty, so a restore recreates the same tree.
fn walk(dir: &Path, prefix: &str, items: &mut Vec<Item>) -> Result<(), ServiceError> {
    let mut entries: Vec<(String, PathBuf)> = std::fs::read_dir(dir)
        .map_err(|e| failed(dir, e))?
        .filter_map(Result::ok)
        .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
        .collect();
    entries.sort();
    for (name, path) in entries {
        // `metadata` follows a symlink: recordings are data to keep, not links to preserve.
        let meta = path.metadata().map_err(|e| failed(&path, e))?;
        if meta.is_dir() {
            items.push(Item::Dir(format!("{prefix}/{name}/")));
            walk(&path, &format!("{prefix}/{name}"), items)?;
        } else if meta.is_file() {
            items.push(Item::File { path: format!("{prefix}/{name}"), data: read_file(&path)? });
        } // devices, sockets and the like: nothing the app writes, nothing a restore would want
    }
    Ok(())
}

fn readme(root: &Path, created: f64) -> String {
    format!(
        "hondascan {} backup, {}\n\n\
         Restore it by unpacking this archive over the data directory with the app closed:\n\n    cd {} && tar -xzf <this file>\n\n\
         history.db is a complete SQLite snapshot (VACUUM INTO), not a copy of a database that was open.\n\
         manifest.json lists every file in this archive with its size and SHA-256.\n",
        env!("CARGO_PKG_VERSION"),
        crate::journal::ts(&json!(created)),
        root.display(),
    )
}

// -- the archive itself --------------------------------------------------------------

/// The entries as a ustar stream: a 512-byte header per entry, the data padded to a 512-byte boundary, and two zero
/// blocks at the end.
fn write_tar<W: Write>(out: &mut W, items: &[Item]) -> Result<(), ServiceError> {
    let mtime = now() as i64;
    for item in items {
        match item {
            Item::Dir(path) => out.write_all(&header(path, 0, mtime, b'5')?)?,
            Item::File { path, data } => {
                out.write_all(&header(path, data.len() as u64, mtime, b'0')?)?;
                out.write_all(data)?;
                let pad = (512 - data.len() % 512) % 512;
                out.write_all(&[0u8; 512][..pad])?;
            }
        }
    }
    out.write_all(&[0u8; 1024])?;
    Ok(())
}

/// One 512-byte ustar header. The mode is 0644 - the file is written by the owner and the data directory stays 0700 -
/// and the name goes into the `name`/`prefix` pair when it is longer than the 100-byte name field.
fn header(name: &str, size: u64, mtime: i64, typeflag: u8) -> Result<[u8; 512], ServiceError> {
    let (prefix, name) = split_name(name)?;
    let mut h = [0u8; 512];
    h[..name.len()].copy_from_slice(name.as_bytes());
    h[100..108].copy_from_slice(b"0000644\0");
    h[108..116].copy_from_slice(b"0000000\0"); // uid/gid: 0 is how ustar means "the unpacking user"
    h[116..124].copy_from_slice(b"0000000\0");
    h[124..136].copy_from_slice(wide_field(size)?.as_bytes());
    h[136..148].copy_from_slice(wide_field(mtime as u64)?.as_bytes());
    h[148..156].copy_from_slice(b"        "); // the checksum is computed with this field as spaces, then written into it
    h[156] = typeflag;
    h[257..263].copy_from_slice(b"ustar\0");
    h[263..265].copy_from_slice(b"00");
    h[265..265 + b"hondascan".len()].copy_from_slice(b"hondascan");
    h[329..337].copy_from_slice(b"0000000\0");
    h[337..345].copy_from_slice(b"0000000\0");
    h[345..345 + prefix.len()].copy_from_slice(prefix.as_bytes());
    let sum: u64 = h.iter().map(|b| u64::from(*b)).sum();
    h[148..154].copy_from_slice(octal(sum, 6)?.as_bytes());
    h[154] = 0; // the field is six octal digits, a NUL and a space
    h[155] = b' ';
    Ok(h)
}

/// The ustar name/prefix split: a name of more than 100 bytes is split at a `/` so both halves fit (155 and 100 bytes
/// respectively). A name that does not fit either way is refused rather than written as a wrong header.
fn split_name(name: &str) -> Result<(String, String), ServiceError> {
    if name.len() <= 100 {
        return Ok((String::new(), name.to_string()));
    }
    for (at, _) in name.match_indices('/').rev() {
        let (prefix, rest) = name.split_at(at);
        let suffix = &rest[1..];
        if !prefix.is_empty() && !suffix.is_empty() && prefix.len() <= 155 && suffix.len() <= 100 {
            return Ok((prefix.to_string(), suffix.to_string()));
        }
    }
    Err(ServiceError::value(format!("{name} does not fit the tar name and prefix fields")))
}

/// A fixed-width octal field of `digits` digits; the caller's slice decides what follows it.
fn octal(value: u64, digits: usize) -> Result<String, ServiceError> {
    if value >= 8u64.pow(digits as u32) {
        return Err(ServiceError::value(format!("{value} does not fit the {digits} octal digits of a tar header field")));
    }
    Ok(format!("{value:0width$o}", width = digits))
}

/// The 12-byte size/mtime field: 11 octal digits and a NUL. A file of 8 GiB or more cannot be named here, and says so
/// instead of being written with a size that wraps.
fn wide_field(value: u64) -> Result<String, ServiceError> {
    if value > MAX_SIZE {
        return Err(ServiceError::value(format!("{value} bytes is too large for a tar header (the limit is {MAX_SIZE})")));
    }
    Ok(format!("{}\0", octal(value, 11)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_field_refuses_8_gib_and_holds_the_rest() {
        assert_eq!(wide_field(0).unwrap(), "00000000000\0");
        assert_eq!(wide_field(4096).unwrap(), "00000010000\0");
        assert_eq!(wide_field(MAX_SIZE).unwrap(), "77777777777\0");
        assert!(wide_field(MAX_SIZE + 1).is_err(), "a file over 8 GiB must error rather than wrap the size field");
        assert!(wide_field(u64::MAX).is_err(), "and so must a negative mtime turned into u64");
    }

    #[test]
    fn split_name_uses_the_prefix_field_only_when_it_has_to() {
        assert_eq!(split_name("history.db").unwrap(), (String::new(), "history.db".to_string()));
        assert_eq!(split_name("a".repeat(100).as_str()).unwrap(), (String::new(), "a".repeat(100)));

        let deep = format!("{}/{}", "d".repeat(120), "f".repeat(90));
        let (prefix, name) = split_name(&deep).unwrap();
        assert_eq!(prefix, "d".repeat(120), "the last usable slash is the split point");
        assert_eq!(name, "f".repeat(90));
        assert!(prefix.len() <= 155 && name.len() <= 100);

        // A directory entry keeps its trailing slash, which is not a split point on its own.
        let dir = format!("{}/{}/", "d".repeat(120), "f".repeat(30));
        let (prefix, name) = split_name(&dir).unwrap();
        assert_eq!(prefix, "d".repeat(120));
        assert_eq!(name, format!("{}/", "f".repeat(30)));

        assert!(split_name(&format!("{}/{}", "d".repeat(160), "f".repeat(120))).is_err(), "neither field can hold it");
        assert!(split_name(&"x".repeat(101)).is_err(), "no slash to split at");
    }

    #[test]
    fn header_checksum_and_magic() {
        let h = header("history.db", 4096, 1_700_000_000, b'0').unwrap();
        assert!(h.starts_with(b"history.db"));
        assert_eq!(&h[257..263], b"ustar\0");
        assert_eq!(&h[263..265], b"00");
        assert_eq!(h[156], b'0');
        assert_eq!(&h[124..136], wide_field(4096).unwrap().as_bytes());

        let mut spaces = h;
        spaces[148..156].copy_from_slice(b"        ");
        let sum: u64 = spaces.iter().map(|b| u64::from(*b)).sum();
        assert_eq!(&h[148..154], format!("{sum:06o}").as_bytes());
        assert_eq!(h[154], 0);
        assert_eq!(h[155], b' ');
    }

    #[test]
    fn tar_stream_is_block_aligned_and_ends_in_two_zero_blocks() {
        let items = vec![
            Item::Dir("vehicles/".into()),
            Item::File { path: "manifest.json".into(), data: vec![b'x'; 600] },
        ];
        let mut out = Vec::new();
        write_tar(&mut out, &items).unwrap();
        assert_eq!(out.len(), 3072, "a directory header, a file header, two data blocks and two end blocks");
        assert_eq!(out[156], b'5', "the directory entry says so");
        assert_eq!(out[512 + 156], b'0');
        assert_eq!(&out[1024..1624], vec![b'x'; 600].as_slice());
        assert!(out[1624..2048].iter().all(|b| *b == 0), "the data is padded to a block boundary");
        assert!(out[2048..].iter().all(|b| *b == 0), "the stream ends in two zero blocks");
    }
}
