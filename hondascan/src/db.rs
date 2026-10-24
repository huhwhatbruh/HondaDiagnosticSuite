//! Read-only access to hondascan.db (built by hondascan/src/bin/build-db.rs from the decoded HDS tables).
//! Port of hondascan/db.py.
use rusqlite::{params, Connection, OpenFlags, Params, Row};
use serde_json::Value;
use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// One row of the `dtc` table as a lookup candidate: (system, sae, subcode, honda_code, description, help_key, temporary).
type DtcCandidateRow = (String, String, i64, String, String, Option<String>, bool);

#[derive(Debug, Clone, PartialEq)]
pub struct DtcInfo {
    pub system: String,
    pub code: String,
    pub subcode: i64,
    pub honda_code: String,
    pub description: String,
    pub help: String,
    pub temporary: bool, // row of HDS's temporary (pending) DTC list
}

#[derive(Debug, Clone, PartialEq)]
pub struct System {
    pub name: String,
    pub display: String,
    pub family: String,
    pub props: Value,
}

pub struct Database {
    pub path: PathBuf,
    con: Mutex<Connection>,
    queries: std::sync::atomic::AtomicU64,
}

/// Where the database is looked for, in order: $HONDASCAN_DB, the crate's data directory (the build-db binary writes
/// it there), /usr/share/hondascan. Each location is tried plain first, then as `.db.enc` (the form `package.rs`
/// ships, decrypted on open — see `dbcrypt` and `open_encrypted` below): a checkout's plain `hondascan.db` wins
/// when both happen to exist, so `cargo test` and everyday dev runs never take the decrypt path at all.
/// `resource()` is called once per literal filename, not once for `data/hondascan.db` with `.db.enc` derived from
/// its result: `resource`'s "beside the executable" check only matches the exact name it is given, and a real
/// package ships `data/hondascan.db.enc`, never the plain name, so deriving the `.enc` path from the plain name's
/// `resource()` call would silently skip straight to resource()'s source-tree fallback for every real install.
pub fn default_paths() -> Vec<PathBuf> {
    let mut v = vec![];
    if let Ok(p) = std::env::var("HONDASCAN_DB") {
        if !p.is_empty() {
            v.push(p.into()); // taken literally: `.enc` iff the caller named it, same as any other path here
        }
    }
    v.push(crate::resource("data/hondascan.db"));
    v.push(crate::resource("data/hondascan.db.enc"));
    v.push("/usr/share/hondascan/hondascan.db".into());
    v.push("/usr/share/hondascan/hondascan.db.enc".into());
    v
}

/// Opens the encrypted `enc_path` as a read-only in-memory sqlite database: no plaintext ever touches a path, a temp
/// file or a directory entry. Before decrypting it closes the same-user memory-read doors (`dbcrypt::harden_process`).
/// The plaintext buffer is `mlock`'d for the short window it exists (`dbcrypt::lock_plain`, so it cannot be paged to
/// swap and read back later) and wiped as soon as sqlite has copied it. A wrong build's file fails the AEAD tag.
fn open_encrypted(enc_path: &Path) -> Result<Connection, String> {
    crate::dbcrypt::harden_process();
    let bytes = std::fs::read(enc_path).map_err(|e| format!("{}: {e}", enc_path.display()))?;
    let mut plain = crate::dbcrypt::decrypt(&bytes).map_err(|e| format!("{}: {e}", enc_path.display()))?;
    crate::dbcrypt::lock_plain(&mut plain);
    let mut con = Connection::open_in_memory().map_err(|e| e.to_string())?;
    let n = plain.len();
    let loaded = con.deserialize_read_exact(rusqlite::MAIN_DB, std::io::Cursor::new(&plain[..]), n, true);
    crate::dbcrypt::wipe(&mut plain);
    crate::dbcrypt::unlock_plain(&mut plain);
    loaded.map_err(|e| format!("{}: {e}", enc_path.display()))?;
    Ok(con)
}

impl Database {
    pub fn open(path: Option<&Path>) -> Result<Database, String> {
        let path = match path {
            Some(p) => p.to_path_buf(),
            None => default_paths().into_iter().find(|p| p.exists()).ok_or(
                "hondascan.db not found; build it with `cargo run --release --bin build-db` (in hondascan/), or set HONDASCAN_DB",
            )?,
        };
        let con = if path.extension().and_then(|e| e.to_str()) == Some("enc") {
            open_encrypted(&path)?
        } else {
            Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)
                .map_err(|e| format!("{}: {e}", path.display()))?
        };
        let has: i64 = con.query_row("SELECT COUNT(*) FROM sqlite_master WHERE name='signal_help'", [], |r| r.get(0)).map_err(|e| format!("{}: {e}", path.display()))?;
        if has == 0 {
            return Err(format!("{} is older than this build (no signal_help table); rebuild it with `cargo run --release --bin build-db` (in hondascan/)", path.display()));
        }
        Ok(Database { path: path.canonicalize().unwrap_or(path), con: Mutex::new(con), queries: Default::default() })
    }

    /// Run a query and collect every row through `f`.
    /// How many queries have been run. Each one takes a file lock and checks for journal files, so what a live read does per call
    /// is worth counting: `tests/db_queries.rs` holds the steady paths to none.
    pub fn query_count(&self) -> u64 { self.queries.load(std::sync::atomic::Ordering::Relaxed) }

    pub fn query<T>(&self, sql: &str, p: impl Params, f: impl FnMut(&Row) -> rusqlite::Result<T>) -> Vec<T> {
        self.queries.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let con = self.con.lock().unwrap();
        let mut st = con.prepare_cached(sql).expect(sql);
        st.query_map(p, f).expect(sql).filter_map(Result::ok).collect()
    }

    pub fn query_one<T>(&self, sql: &str, p: impl Params, f: impl FnMut(&Row) -> rusqlite::Result<T>) -> Option<T> {
        self.query(sql, p, f).into_iter().next()
    }

    pub fn systems(&self) -> Vec<System> {
        self.query("SELECT name, display, family, props FROM systems ORDER BY name", [], system_row)
    }

    pub fn system(&self, name: &str) -> Option<System> {
        self.query_one("SELECT name, display, family, props FROM systems WHERE name=?", [name], system_row)
    }

    /// {address: [(system_type, rule, db_system_hint), ...]}. Each rule is one path through HDS's decision tree
    /// for that address, so at most one matches.
    pub fn can_rules(&self) -> BTreeMap<i64, Vec<(String, String, String)>> {
        let mut out: BTreeMap<i64, Vec<(String, String, String)>> = BTreeMap::new();
        for (a, t, r, h) in self.query(
            "SELECT address, system_type, rule, db_system_hint FROM can_addresses ORDER BY address, rowid",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        ) {
            out.entry(a).or_default().push((t, r, h));
        }
        out
    }

    /// HDS registry type name ('HONDA ABS') -> VCAN database system ('CANABS').
    pub fn vcan_system_for_type(&self, system_type: &str, hint: &str) -> Option<String> {
        if !hint.is_empty() && self.system(hint).is_some() {
            return Some(hint.into());
        }
        self.query_one(
            "SELECT name FROM systems WHERE type=? AND family='VCAN' ORDER BY name LIKE 'CAN%' DESC, name",
            [system_type],
            |r| r.get(0),
        )
    }

    pub fn help_text(&self, key: &str) -> String {
        self.query_one("SELECT text FROM help WHERE key=?", [key], |r| r.get(0)).unwrap_or_default()
    }

    /// Matches for an SAE code (e.g. 'P0420') + optional failure-type byte, best first. HDS (HUTILS.GetDtcDispData)
    /// looks a DTC up by an exact key; that exact match ranks first, then the same code in that system, then any
    /// system, so a record is still named when the table has no exact row.
    pub fn lookup_dtc(&self, code: &str, subcode: Option<i64>, system: Option<&str>, temporary: Option<bool>) -> Vec<DtcInfo> {
        let mut cands: Vec<DtcCandidateRow> = self.query(
            "SELECT system, sae, subcode, honda_code, description, help_key, temporary FROM dtc WHERE sae=?",
            [code.to_uppercase()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get::<_, Option<i64>>(6)?.unwrap_or(0) != 0)),
        );
        let rank = |r: &DtcCandidateRow| {
            (system.is_some_and(|s| r.0 == s), subcode.is_some_and(|s| r.2 == s), temporary.is_some_and(|t| r.6 == t), r.2 == 0)
        };
        cands.sort_by_key(|r| Reverse(rank(r))); // stable, like Python's sort(reverse=True)
        cands
            .into_iter()
            .map(|(system, code, subcode, honda_code, description, hk, temporary)| DtcInfo {
                help: hk.filter(|k| !k.is_empty()).map(|k| self.help_text(&k)).unwrap_or_default(),
                system, code, subcode, honda_code, description, temporary,
            })
            .collect()
    }

    pub fn has_dtc(&self, system: &str, code: &str) -> bool {
        self.query_one("SELECT 1 FROM dtc WHERE system=? AND sae=? LIMIT 1", params![system, code.to_uppercase()], |r| r.get::<_, i64>(0)).is_some()
    }
}

fn system_row(r: &Row) -> rusqlite::Result<System> {
    let props: String = r.get(3)?;
    Ok(System { name: r.get(0)?, display: r.get(1)?, family: r.get(2)?, props: serde_json::from_str(&props).unwrap_or(Value::Null) })
}
