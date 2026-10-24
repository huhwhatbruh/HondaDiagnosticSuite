//! Per-vehicle history: one SQLite file under the data directory, keyed by VIN. Port of hondascan/store.py.
//!
//! Writes are queued and committed together (`flush`, called by the recorder every few seconds and at the end of a
//! session), so a 2 Hz watch list costs one transaction per flush, not one per row. WAL mode lets the app read history
//! while a session is being written. A session that a crash or power loss left open is closed when the store next opens
//! ('interrupted', at its last sample, with the summary.json written so far).
//!
//! Where: $HONDASCAN_HOME, else the platform's user data directory (XDG_DATA_HOME or ~/.local/share on Linux), in a
//! `hondascan` folder.
use crate::value::{dumps, Json};
use rusqlite::types::Value as Sql;
use rusqlite::{params, params_from_iter, Connection};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS vehicles(vin TEXT PRIMARY KEY, first_seen REAL, last_seen REAL);
CREATE TABLE IF NOT EXISTS sessions(id INTEGER PRIMARY KEY, vin TEXT, started REAL, ended REAL, adapter TEXT,
  end_reason TEXT, dir TEXT, summary TEXT);
CREATE TABLE IF NOT EXISTS ecus(session INTEGER, address INTEGER, system_type TEXT, system TEXT, display TEXT, ident TEXT);
CREATE TABLE IF NOT EXISTS dtc_events(session INTEGER, vin TEXT, t REAL, system TEXT, address INTEGER, code TEXT,
  failure_type INTEGER, status INTEGER, kind TEXT, event TEXT, description TEXT);
CREATE TABLE IF NOT EXISTS samples(session INTEGER, t REAL, system TEXT, did INTEGER, kind TEXT, vals TEXT);
CREATE TABLE IF NOT EXISTS stats(session INTEGER, system TEXT, did INTEGER, idx INTEGER, symbol TEXT, unit TEXT,
  n INTEGER, lo REAL, hi REAL, total REAL, last TEXT, PRIMARY KEY(session, system, did, idx));
CREATE TABLE IF NOT EXISTS baselines(id INTEGER PRIMARY KEY, vin TEXT, created REAL, label TEXT, healthy INTEGER, payload TEXT, sha256 TEXT);
CREATE TABLE IF NOT EXISTS markers(id INTEGER PRIMARY KEY, vin TEXT, at REAL, kind TEXT, note TEXT, mileage_km REAL, created REAL);
CREATE INDEX IF NOT EXISTS dtc_vin ON dtc_events(vin, code);
CREATE INDEX IF NOT EXISTS samples_session ON samples(session, system, did, t);
CREATE INDEX IF NOT EXISTS sessions_vin ON sessions(vin, started);
CREATE INDEX IF NOT EXISTS baselines_vin ON baselines(vin, created);
CREATE INDEX IF NOT EXISTS markers_vin ON markers(vin, at);
";

pub fn data_dir() -> PathBuf {
    if let Ok(h) = std::env::var("HONDASCAN_HOME") {
        if !h.is_empty() {
            return h.into();
        }
    }
    let home = || std::env::var("HOME").map(PathBuf::from).unwrap_or_default();
    let base = if cfg!(target_os = "macos") {
        home().join("Library/Application Support")
    } else if cfg!(windows) {
        std::env::var("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|_| home())
    } else {
        std::env::var("XDG_DATA_HOME").ok().filter(|s| !s.is_empty()).map(PathBuf::from).unwrap_or_else(|| home().join(".local/share"))
    };
    base.join("hondascan")
}

/// The summary.json a session wrote last (every minute), as a compact JSON string, or None if it is missing or cut short.
fn saved_summary(folder: &str) -> Option<String> {
    let text = std::fs::read_to_string(Path::new(folder).join("summary.json")).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    Some(dumps(&v, Json::Spaced))
}

/// One ECU as recorded: address, HDS type, database system and display name.
pub struct EcuRow<'a> {
    pub address: u32,
    pub system_type: &'a str,
    pub system: &'a str,
    pub display: &'a str,
}

/// One stats row: (system, did, idx, symbol, unit, n, lo, hi, total, last).
pub struct StatRow {
    pub system: String,
    pub did: i64,
    pub idx: i64,
    pub symbol: String,
    pub unit: String,
    pub n: i64,
    pub lo: Option<f64>,
    pub hi: Option<f64>,
    pub total: f64,
    pub last: String,
}

pub struct Store {
    pub root: PathBuf,
    con: Mutex<Connection>,
    queue: Mutex<Vec<(&'static str, Vec<Sql>)>>,
}

fn txt(s: &str) -> Sql {
    Sql::Text(s.into())
}

fn f(x: f64) -> Sql {
    Sql::Real(x)
}

fn opt(x: Option<f64>) -> Sql {
    x.map_or(Sql::Null, Sql::Real)
}

fn i(x: i64) -> Sql {
    Sql::Integer(x)
}

impl Store {
    pub fn open(root: Option<&Path>) -> rusqlite::Result<Store> {
        let root = root.map(Path::to_path_buf).unwrap_or_else(data_dir);
        std::fs::create_dir_all(&root).ok(); // VINs and drive logs: the owner only
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).ok();
        }
        let con = Connection::open(root.join("history.db"))?;
        con.query_row("PRAGMA journal_mode=WAL", [], |_| Ok(()))?;
        con.execute_batch("PRAGMA synchronous=NORMAL")?;
        con.execute_batch(SCHEMA)?;
        let s = Store { root, con: Mutex::new(con), queue: Mutex::new(vec![]) };
        s.close_interrupted()?;
        Ok(s)
    }

    fn close_interrupted(&self) -> rusqlite::Result<()> {
        let con = self.con.lock().unwrap();
        let rows: Vec<(i64, f64, Option<String>, Option<f64>)> = {
            let mut st = con.prepare("SELECT id, started, dir, (SELECT MAX(t) FROM samples WHERE session=s.id) FROM sessions s WHERE ended IS NULL")?;
            let r = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<Result<_, _>>()?;
            r
        };
        for (sid, started, folder, last) in rows {
            let summary = folder.as_deref().and_then(saved_summary);
            con.execute("UPDATE sessions SET ended=?, end_reason='interrupted', summary=? WHERE id=?", params![last.unwrap_or(started), summary, sid])?;
        }
        Ok(())
    }

    // -- writes (queued) ----------------------------------------------------------
    fn q(&self, sql: &'static str, row: Vec<Sql>) {
        self.queue.lock().unwrap().push((sql, row));
    }

    pub fn flush(&self) -> rusqlite::Result<()> {
        let queued = std::mem::take(&mut *self.queue.lock().unwrap());
        if queued.is_empty() {
            return Ok(());
        }
        // grouped from &queued (rows cloned) so `queued` survives to be put back when the flush fails
        let mut by: Vec<(&str, Vec<Vec<Sql>>)> = vec![];
        for (sql, row) in &queued {
            match by.iter_mut().find(|(s, _)| *s == *sql) {
                Some((_, rows)) => rows.push(row.clone()),
                None => by.push((*sql, vec![row.clone()])),
            }
        }
        let out = (|| -> rusqlite::Result<()> {
            let mut con = self.con.lock().unwrap();
            let tx = con.transaction()?;
            for (sql, rows) in &by {
                let mut st = tx.prepare_cached(sql)?;
                for r in rows {
                    st.execute(params_from_iter(r))?;
                }
            }
            tx.commit()
        })();
        match out {
            Ok(()) => Ok(()),
            Err(e) => {
                // a failed flush must not lose the session's rows: they go back at the front, ahead of anything queued since
                let mut q = self.queue.lock().unwrap();
                q.splice(..0, queued);
                Err(e)
            }
        }
    }

    pub fn start_session(&self, vin: &str, t: f64, adapter: &str, folder: &str) -> rusqlite::Result<i64> {
        self.flush()?;
        let con = self.con.lock().unwrap();
        con.execute("INSERT INTO vehicles(vin, first_seen, last_seen) VALUES (?,?,?) ON CONFLICT(vin) DO UPDATE SET last_seen=excluded.last_seen", params![vin, t, t])?;
        con.execute("INSERT INTO sessions(vin, started, adapter, dir) VALUES (?,?,?,?)", params![vin, t, adapter, folder])?;
        Ok(con.last_insert_rowid())
    }

    pub fn end_session(&self, sid: i64, t: f64, reason: &str, summary: &Value) -> rusqlite::Result<()> {
        self.q("UPDATE sessions SET ended=?, end_reason=?, summary=? WHERE id=?", vec![f(t), txt(reason), txt(&dumps(summary, Json::Spaced)), i(sid)]);
        self.q("UPDATE vehicles SET last_seen=? WHERE vin=(SELECT vin FROM sessions WHERE id=?)", vec![f(t), i(sid)]);
        self.flush()
    }

    pub fn ecu(&self, sid: i64, e: &EcuRow, ident: &Value) {
        self.q(
            "INSERT INTO ecus VALUES (?,?,?,?,?,?)",
            vec![i(sid), i(e.address as i64), txt(e.system_type), txt(e.system), txt(e.display), txt(&dumps(ident, Json::Spaced))],
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn dtc(&self, sid: i64, vin: &str, t: f64, system: &str, address: u32, code: &str, failure_type: i64, status: i64, kind: &str, event: &str, description: &str) {
        self.q(
            "INSERT INTO dtc_events VALUES (?,?,?,?,?,?,?,?,?,?,?)",
            vec![i(sid), txt(vin), f(t), txt(system), i(address as i64), txt(code), i(failure_type), i(status), txt(kind), txt(event), txt(description)],
        );
    }

    pub fn sample(&self, sid: i64, t: f64, system: &str, did: i64, kind: &str, vals: &Value) {
        self.q("INSERT INTO samples VALUES (?,?,?,?,?,?)", vec![i(sid), f(t), txt(system), i(did), txt(kind), txt(&dumps(vals, Json::Compact))]);
    }

    pub fn stats(&self, sid: i64, rows: &[StatRow]) {
        for r in rows {
            self.q(
                "INSERT OR REPLACE INTO stats VALUES (?,?,?,?,?,?,?,?,?,?,?)",
                vec![i(sid), txt(&r.system), i(r.did), i(r.idx), txt(&r.symbol), txt(&r.unit), i(r.n), opt(r.lo), opt(r.hi), f(r.total), txt(&r.last)],
            );
        }
    }

    // -- reads --------------------------------------------------------------------
    /// [{vin, first_seen, last_seen, sessions}], most recently seen first.
    pub fn vehicles(&self) -> Vec<Value> {
        let con = self.con.lock().unwrap();
        let mut st = con
            .prepare("SELECT v.vin, v.first_seen, v.last_seen, COUNT(s.id) FROM vehicles v LEFT JOIN sessions s ON s.vin=v.vin GROUP BY v.vin ORDER BY v.last_seen DESC")
            .unwrap();
        st.query_map([], |r| Ok(json!({"vin": r.get::<_, String>(0)?, "first_seen": r.get::<_, Option<f64>>(1)?, "last_seen": r.get::<_, Option<f64>>(2)?, "sessions": r.get::<_, i64>(3)?})))
            .unwrap()
            .filter_map(Result::ok)
            .collect()
    }

    /// Newest first, `limit` of them (-1: all).
    pub fn sessions(&self, vin: &str, limit: i64) -> Vec<Value> {
        let con = self.con.lock().unwrap();
        let mut st = con.prepare("SELECT id, started, ended, adapter, end_reason, dir, summary FROM sessions WHERE vin=? ORDER BY started DESC LIMIT ?").unwrap();
        st.query_map(params![vin, limit], |r| {
            let summ: Option<String> = r.get(6)?;
            Ok(json!({"id": r.get::<_, i64>(0)?, "started": r.get::<_, f64>(1)?, "ended": r.get::<_, Option<f64>>(2)?, "adapter": r.get::<_, Option<String>>(3)?,
                      "end_reason": r.get::<_, Option<String>>(4)?, "dir": r.get::<_, Option<String>>(5)?,
                      "summary": summ.filter(|s| !s.is_empty()).and_then(|s| serde_json::from_str::<Value>(&s).ok()).unwrap_or(Value::Null)}))
        })
        .unwrap()
        .filter_map(Result::ok)
        .collect()
    }

    pub fn last_session(&self, vin: &str, before: i64) -> Option<i64> {
        self.con
            .lock()
            .unwrap()
            .query_row("SELECT id FROM sessions WHERE vin=? AND id<? AND ended IS NOT NULL ORDER BY id DESC LIMIT 1", params![vin, before], |r| r.get(0))
            .ok()
    }

    /// The observed maps (`maps.out()`) saved in this car's earlier sessions' summaries, newest first. Several cars never
    /// mix: an unknown VIN has no history.
    pub fn prior_maps(&self, vin: &str, before: i64, limit: i64) -> Vec<Value> {
        if vin.is_empty() || vin == "UNKNOWN" {
            return vec![];
        }
        let con = self.con.lock().unwrap();
        let mut st = con.prepare("SELECT summary FROM sessions WHERE vin=? AND id<? AND summary IS NOT NULL ORDER BY id DESC LIMIT ?").unwrap();
        let rows: Vec<String> = st.query_map(params![vin, before, limit], |r| r.get(0)).unwrap().filter_map(Result::ok).collect();
        rows.iter()
            .filter_map(|s| serde_json::from_str::<Value>(s).ok())
            .filter_map(|v| v.get("maps").cloned())
            .filter(|m| !matches!(m, Value::Null | Value::Bool(false)) && m.as_object().is_none_or(|o| !o.is_empty()))
            .collect()
    }

    /// One row per code: first and last time seen, sessions seen in, last event.
    pub fn dtc_history(&self, vin: &str) -> Vec<Value> {
        let con = self.con.lock().unwrap();
        let mut st = con
            .prepare(
                "SELECT system, code, failure_type, MIN(t), MAX(t), COUNT(DISTINCT session), \
                 (SELECT event FROM dtc_events d2 WHERE d2.vin=d.vin AND d2.code=d.code AND d2.system=d.system \
                  AND d2.failure_type=d.failure_type ORDER BY t DESC LIMIT 1), MAX(description), \
                 (SELECT kind FROM dtc_events d3 WHERE d3.vin=d.vin AND d3.code=d.code AND d3.system=d.system \
                  AND d3.failure_type=d.failure_type ORDER BY t DESC LIMIT 1) \
                 FROM dtc_events d WHERE vin=? GROUP BY system, code, failure_type ORDER BY MAX(t) DESC",
            )
            .unwrap();
        st.query_map([vin], |r| {
            Ok(json!({"system": r.get::<_, Option<String>>(0)?, "code": r.get::<_, Option<String>>(1)?, "failure_type": r.get::<_, Option<i64>>(2)?,
                      "first_seen": r.get::<_, Option<f64>>(3)?, "last_seen": r.get::<_, Option<f64>>(4)?, "sessions": r.get::<_, i64>(5)?,
                      "last_event": r.get::<_, Option<String>>(6)?, "description": r.get::<_, Option<String>>(7)?, "kind": r.get::<_, Option<String>>(8)?}))
        })
        .unwrap()
        .filter_map(Result::ok)
        .collect()
    }

    /// Codes present at the last read of a session: {(system, code, failure_type)}.
    pub fn session_codes(&self, sid: i64) -> std::collections::BTreeSet<(String, String, i64)> {
        let con = self.con.lock().unwrap();
        let mut st = con.prepare("SELECT system, code, failure_type, event FROM dtc_events WHERE session=? ORDER BY t").unwrap();
        let mut out = std::collections::BTreeSet::new();
        for (system, code, ft, ev) in st
            .query_map([sid], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?, r.get::<_, String>(3)?)))
            .unwrap()
            .filter_map(Result::ok)
        {
            match ev.as_str() {
                "present" | "appeared" => {
                    out.insert((system, code, ft));
                }
                "gone" | "cleared" => {
                    out.remove(&(system, code, ft));
                }
                _ => {}
            }
        }
        out
    }

    /// Per-session min / mean / max of one signal, oldest first. A symbol can sit in several data groups; per session take
    /// its best-sampled one (the live source).
    pub fn trend(&self, vin: &str, system: &str, symbol: &str, limit: i64) -> Vec<Value> {
        let con = self.con.lock().unwrap();
        let mut st = con
            .prepare(
                "SELECT s.id, s.started, st.lo, st.total / st.n, st.hi, MAX(st.n), st.unit FROM stats st JOIN sessions s ON s.id=st.session \
                 WHERE s.vin=? AND st.system=? AND st.symbol=? AND st.n>0 GROUP BY s.id ORDER BY s.started DESC LIMIT ?",
            )
            .unwrap();
        let mut rows: Vec<Value> = st
            .query_map(params![vin, system, symbol, limit], |r| {
                Ok(json!({"session": r.get::<_, i64>(0)?, "started": r.get::<_, f64>(1)?, "min": r.get::<_, Option<f64>>(2)?, "mean": r.get::<_, Option<f64>>(3)?,
                          "max": r.get::<_, Option<f64>>(4)?, "n": r.get::<_, i64>(5)?, "unit": r.get::<_, Option<String>>(6)?}))
            })
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        rows.reverse();
        rows
    }

    /// What each module said about itself in this car's sessions, oldest first: {session, started, address, system, ecu_id,
    /// ecu_model, part_number}. Rows that carry neither an ECU ID nor a part number are left out.
    pub fn identities(&self, vin: &str) -> Vec<Value> {
        let con = self.con.lock().unwrap();
        let mut st = con
            .prepare("SELECT e.session, s.started, e.address, e.system, e.ident FROM ecus e JOIN sessions s ON s.id=e.session WHERE s.vin=? ORDER BY s.started, e.session, e.address")
            .unwrap();
        st.query_map([vin], |r| {
            let id: Value = r.get::<_, Option<String>>(4)?.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Value::Null);
            let field = |k: &str| id.get(k).filter(|v| v.is_string()).cloned().unwrap_or(Value::Null);
            let (ecu_id, part_number) = (field("ecu_id"), field("part_number"));
            Ok((!ecu_id.is_null() || !part_number.is_null()).then(|| {
                json!({"session": r.get::<_, i64>(0).ok(), "started": r.get::<_, f64>(1).ok(), "address": r.get::<_, i64>(2).ok(), "system": r.get::<_, Option<String>>(3).ok().flatten(),
                       "ecu_id": ecu_id, "ecu_model": field("ecu_model"), "part_number": part_number})
            }))
        })
        .unwrap()
        .filter_map(Result::ok)
        .flatten()
        .collect()
    }

    /// The folder of a session, if there is one.
    pub fn session_dir(&self, sid: i64) -> Option<String> {
        self.con.lock().unwrap().query_row("SELECT dir FROM sessions WHERE id=?", [sid], |r| r.get(0)).ok().flatten()
    }

    // -- frozen baselines and service markers ------------------------------------------
    /// Store a frozen baseline. A baseline is a record of what the car reported: there is deliberately no update or
    /// delete method, so a stored row keeps its payload and hash for as long as the database lives.
    pub fn add_baseline(&self, vin: &str, created: f64, label: &str, healthy: bool, payload: &str, sha256: &str) -> rusqlite::Result<i64> {
        let con = self.con.lock().unwrap();
        con.execute(
            "INSERT INTO baselines(vin, created, label, healthy, payload, sha256) VALUES (?,?,?,?,?,?)",
            params![vin, created, label, healthy, payload, sha256],
        )?;
        Ok(con.last_insert_rowid())
    }

    /// Baseline metadata for one car, newest first: {id, created, label, healthy, sha256, systems_count}. The system
    /// count is read from the payload, so a damaged row counts as zero systems rather than claiming the old one.
    pub fn baselines(&self, vin: &str) -> Vec<Value> {
        let con = self.con.lock().unwrap();
        let mut st = con
            .prepare("SELECT id, created, label, healthy, payload, sha256 FROM baselines WHERE vin=? ORDER BY created DESC, id DESC")
            .unwrap();
        st.query_map([vin], |r| {
            let payload: Option<String> = r.get(4)?;
            let systems = payload
                .and_then(|p| serde_json::from_str::<Value>(&p).ok())
                .and_then(|v| Some(v.get("systems")?.as_array()?.len()))
                .unwrap_or(0);
            Ok(json!({"id": r.get::<_, i64>(0)?, "created": r.get::<_, f64>(1)?, "label": r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                      "healthy": r.get::<_, bool>(3)?, "sha256": r.get::<_, Option<String>>(5)?.unwrap_or_default(), "systems_count": systems}))
        })
        .unwrap()
        .filter_map(Result::ok)
        .collect()
    }

    /// (id, payload, sha256) of one car's baselines, newest first: what the integrity check and the latest baseline read.
    pub fn baseline_payloads(&self, vin: &str) -> Vec<(i64, String, String)> {
        let con = self.con.lock().unwrap();
        let mut st = con.prepare("SELECT id, payload, sha256 FROM baselines WHERE vin=? ORDER BY created DESC, id DESC").unwrap();
        st.query_map([vin], |r| Ok((r.get(0)?, r.get::<_, Option<String>>(1)?.unwrap_or_default(), r.get::<_, Option<String>>(2)?.unwrap_or_default())))
            .unwrap()
            .filter_map(Result::ok)
            .collect()
    }

    /// Every baseline for the logical export, oldest first, with its car and whole payload.
    pub fn all_baselines(&self) -> Vec<Value> {
        let con = self.con.lock().unwrap();
        let mut st = con.prepare("SELECT id, vin, created, label, healthy, sha256, payload FROM baselines ORDER BY created, id").unwrap();
        st.query_map([], |r| {
            Ok(json!({"id": r.get::<_, i64>(0)?, "vin": r.get::<_, Option<String>>(1)?.unwrap_or_default(), "created": r.get::<_, f64>(2)?,
                      "label": r.get::<_, Option<String>>(3)?.unwrap_or_default(), "healthy": r.get::<_, bool>(4)?,
                      "sha256": r.get::<_, Option<String>>(5)?.unwrap_or_default(), "payload": r.get::<_, Option<String>>(6)?.unwrap_or_default()}))
        })
        .unwrap()
        .filter_map(Result::ok)
        .collect()
    }

    /// Store a service marker (work done between sessions). `at` is when the work was recorded; `created` is when the row
    /// was written.
    pub fn add_marker(&self, vin: &str, at: f64, kind: &str, note: &str, mileage_km: Option<f64>, created: f64) -> rusqlite::Result<i64> {
        let con = self.con.lock().unwrap();
        con.execute(
            "INSERT INTO markers(vin, at, kind, note, mileage_km, created) VALUES (?,?,?,?,?,?)",
            params![vin, at, kind, note, mileage_km, created],
        )?;
        Ok(con.last_insert_rowid())
    }

    /// One car's service markers, newest first: {id, at, kind, note, mileage_km, created}. An unrecorded mileage is null.
    pub fn markers(&self, vin: &str) -> Vec<Value> {
        let con = self.con.lock().unwrap();
        let mut st = con.prepare("SELECT id, at, kind, note, mileage_km, created FROM markers WHERE vin=? ORDER BY at DESC, id DESC").unwrap();
        st.query_map([vin], |r| {
            Ok(json!({"id": r.get::<_, i64>(0)?, "at": r.get::<_, f64>(1)?, "kind": r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                      "note": r.get::<_, Option<String>>(3)?.unwrap_or_default(), "mileage_km": r.get::<_, Option<f64>>(4)?, "created": r.get::<_, f64>(5)?}))
        })
        .unwrap()
        .filter_map(Result::ok)
        .collect()
    }

    /// Delete a marker by id: true when a row was removed. No VIN filter: the caller removes the id it displayed.
    pub fn remove_marker(&self, id: i64) -> rusqlite::Result<bool> {
        let con = self.con.lock().unwrap();
        Ok(con.execute("DELETE FROM markers WHERE id=?", [id])? > 0)
    }

    /// Every marker for the logical export, oldest first, with its car.
    pub fn all_markers(&self) -> Vec<Value> {
        let con = self.con.lock().unwrap();
        let mut st = con.prepare("SELECT id, vin, at, kind, note, mileage_km, created FROM markers ORDER BY at, id").unwrap();
        st.query_map([], |r| {
            Ok(json!({"id": r.get::<_, i64>(0)?, "vin": r.get::<_, Option<String>>(1)?.unwrap_or_default(), "at": r.get::<_, f64>(2)?,
                      "kind": r.get::<_, Option<String>>(3)?.unwrap_or_default(), "note": r.get::<_, Option<String>>(4)?.unwrap_or_default(),
                      "mileage_km": r.get::<_, Option<f64>>(5)?, "created": r.get::<_, f64>(6)?}))
        })
        .unwrap()
        .filter_map(Result::ok)
        .collect()
    }

    /// Write a complete standalone copy of the database to `dst` (`VACUUM INTO`), for the backup archive. An existing
    /// file is removed first, because `VACUUM INTO` refuses to overwrite one; the caller deletes `dst` when it is done
    /// with it. Queued rows are committed first, so the copy holds everything the session has written so far.
    pub fn snapshot(&self, dst: &Path) -> rusqlite::Result<()> {
        self.flush()?;
        if dst.exists() {
            std::fs::remove_file(dst).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        }
        self.con.lock().unwrap().execute("VACUUM INTO ?1", params![dst.to_string_lossy()])?;
        Ok(())
    }

    pub fn close(&self) -> rusqlite::Result<()> {
        self.flush()
    }
}
