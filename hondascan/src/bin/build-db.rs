//! `build-db [DECODED_DIR] [OUT_DB]`: build hondascan.db (SQLite) from the decoded HDS tables in ../hds-xref/decoded.
//!
//! The default output is data/hondascan.db in this crate, where `hondascan::db` finds it. The database is generated,
//! not tracked: the build is deterministic, so the CSVs are the source of truth. (Ported from the Python
//! tools/build_db.py; the two produced identical tables, indexes and meta, checked row by row.)
use rusqlite::{params, Connection, Transaction};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

type Res<T> = Result<T, Box<dyn std::error::Error>>;

struct Row(HashMap<String, String>);

impl Row {
    fn s(&self, col: &str) -> &str {
        self.0.get(col).map(String::as_str).unwrap_or_else(|| panic!("column {col} missing"))
    }
    fn i(&self, col: &str) -> Res<i64> {
        Ok(self.s(col).trim().parse().map_err(|e| format!("{col}={:?}: {e}", self.s(col)))?)
    }
    /// An integer column that is empty for "0".
    fn i_or0(&self, col: &str) -> Res<i64> {
        if self.s(col).is_empty() { Ok(0) } else { self.i(col) }
    }
    fn hex(&self, col: &str) -> Res<i64> {
        let t = self.s(col).trim();
        let t = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")).unwrap_or(t);
        Ok(i64::from_str_radix(t, 16).map_err(|e| format!("{col}={:?}: {e}", self.s(col)))?)
    }
    fn f(&self, col: &str) -> Res<f64> {
        Ok(self.s(col).trim().parse().map_err(|e| format!("{col}={:?}: {e}", self.s(col)))?)
    }
}

fn rows(src: &Path, name: &str) -> Res<Vec<Row>> {
    let mut rd = csv::ReaderBuilder::new().flexible(true).from_path(src.join(name)).map_err(|e| format!("{name}: {e}"))?;
    let headers = rd.headers()?.clone();
    let mut out = Vec::new();
    for rec in rd.records() {
        let rec = rec?;
        out.push(Row(headers.iter().zip(rec.iter()).map(|(h, v)| (h.to_string(), v.to_string())).collect()));
    }
    Ok(out)
}

/// A property value as json.dumps writes it (ensure_ascii, ", " and ": " separators).
fn json_str(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 || (c as u32) > 0x7f => {
                let mut b = [0u16; 2];
                for u in c.encode_utf16(&mut b) {
                    out.push_str(&format!("\\u{u:04x}"));
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

enum Prop {
    Int(i64),
    Text(String),
}

fn props_json(p: &[(String, Prop)]) -> String {
    let mut out = String::from("{");
    for (n, (k, v)) in p.iter().enumerate() {
        if n > 0 {
            out.push_str(", ");
        }
        json_str(k, &mut out);
        out.push_str(": ");
        match v {
            Prop::Int(i) => out.push_str(&i.to_string()),
            Prop::Text(t) => json_str(t, &mut out),
        }
    }
    out.push('}');
    out
}

const SCHEMA: &str = "
    CREATE TABLE systems(name TEXT PRIMARY KEY, display TEXT, family TEXT, props TEXT, type TEXT);
    CREATE TABLE can_addresses(address INTEGER, system_type TEXT, rule TEXT, db_system_hint TEXT);
    CREATE TABLE dtc(system TEXT, sae TEXT, subcode INTEGER, honda_code TEXT,
                     description TEXT, help_key TEXT, temporary INTEGER, priority INTEGER);
    CREATE TABLE help(key TEXT PRIMARY KEY, text TEXT);
    -- the help pages that carry a parameter specification, cleaned once here (signalhelp::analyse) instead of at run time
    CREATE TABLE signal_help(key TEXT PRIMARY KEY, body TEXT, stopped INTEGER);
    CREATE TABLE ecus(system TEXT, id_pattern TEXT, name TEXT);
    CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT);
    -- live data (CAN): which DIDs a system serves, the bytes each signal uses, and how to scale them
    CREATE TABLE can_packets(system TEXT, did INTEGER, size INTEGER, kind INTEGER);
    CREATE TABLE can_packet_usage(system TEXT, usage TEXT, did INTEGER);
    CREATE TABLE can_signal_parts(system TEXT, did INTEGER, signal INTEGER, symbol TEXT, description TEXT,
                                  unit TEXT, part INTEGER, offset INTEGER, mask INTEGER, converters TEXT);
    CREATE TABLE converters(id INTEGER PRIMARY KEY, equation INTEGER, name TEXT, out_unit INTEGER);
    CREATE TABLE units(id INTEGER PRIMARY KEY, name TEXT);
    CREATE TABLE converter_coefficients(converter INTEGER, idx INTEGER, value REAL);
    CREATE TABLE converter_map(converter INTEGER, idx INTEGER, x REAL, y REAL);
    CREATE TABLE converter_states(converter INTEGER, idx INTEGER, low REAL, high REAL, text TEXT, led INTEGER);
";

const INDEXES: &str = "
    CREATE INDEX csp_sys ON can_signal_parts(system, did);
    CREATE INDEX cc_conv ON converter_coefficients(converter);
    CREATE INDEX cm_conv ON converter_map(converter);
    CREATE INDEX cs_conv ON converter_states(converter);
    CREATE INDEX dtc_code ON dtc(sae, subcode);
    CREATE INDEX dtc_sys ON dtc(system);
    CREATE INDEX ecu_sys ON ecus(system);
";

fn signal_part(tx: &Transaction, r: &Row) -> Res<()> {
    tx.prepare_cached("INSERT INTO can_signal_parts VALUES(?,?,?,?,?,?,?,?,?,?)")?.execute(params![
        r.s("system"), r.hex("did")?, r.i("signal_row")?, r.s("symbol"), r.s("description"), r.s("unit"),
        r.i("part")?, r.i("offset")?, r.hex("mask")?, r.s("converters")
    ])?;
    Ok(())
}

fn build(src: &Path, out: &Path) -> Res<()> {
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir)?;
    }
    if out.exists() {
        std::fs::remove_file(out)?;
    }
    let mut db = Connection::open(out)?;
    db.execute_batch(SCHEMA)?;
    let tx = db.transaction()?;

    // system properties: slot 0 is the plain name, others "name#slot"; numbers become JSON numbers
    let mut props: HashMap<String, Vec<(String, Prop)>> = HashMap::new();
    for r in rows(src, "system_properties.csv")? {
        let key = if r.s("slot") == "0" { r.s("property").to_string() } else { format!("{}#{}", r.s("property"), r.s("slot")) };
        let v = r.s("value");
        let digits = v.strip_prefix('-').unwrap_or(v);
        let val = if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
            Prop::Int(v.parse()?)
        } else {
            Prop::Text(v.to_string())
        };
        let list = props.entry(r.s("system").to_string()).or_default();
        match list.iter_mut().find(|(k, _)| *k == key) {
            Some(e) => e.1 = val, // a dict keeps the first position, takes the last value
            None => list.push((key, val)),
        }
    }
    let types: HashMap<String, String> = rows(src, "system_types.csv")?.iter().map(|r| (r.s("row").to_string(), r.s("name").to_string())).collect();
    for r in rows(src, "systems.csv")? {
        let ty = types.get(r.s("type_idx?")).map(String::as_str);
        let display = if r.s("display_en?").is_empty() { ty.unwrap_or(r.s("name")) } else { r.s("display_en?") };
        let p = props.get(r.s("name")).map(|p| props_json(p)).unwrap_or_else(|| "{}".into());
        tx.execute("INSERT INTO systems VALUES(?,?,?,?,?)", params![r.s("name"), display, r.s("vendor_protocol?"), p, ty.unwrap_or("")])?;
    }
    for r in rows(src, "can_address_map.csv")? {
        tx.execute("INSERT INTO can_addresses VALUES(?,?,?,?)", params![r.i("address")?, r.s("system_type"), r.s("rule"), r.s("db_system_hint")])?;
    }

    let base = rows(src, "dtc.csv")?;
    let helpcols = rows(src, "dtc_with_help.csv")?;
    if base.len() != helpcols.len() || base.iter().zip(&helpcols).any(|(a, b)| a.s("row") != b.s("row")) {
        return Err("dtc.csv and dtc_with_help.csv do not line up".into());
    }
    for (a, b) in base.iter().zip(&helpcols) {
        tx.execute("INSERT INTO dtc VALUES(?,?,?,?,?,?,?,?)", params![
            a.s("system"), a.s("sae").trim().to_uppercase(), a.i_or0("sae_subcode")?, b.s("honda_code"),
            a.s("description_en"), b.s("help_key"), a.i_or0("temporary")?, a.i_or0("priority")?
        ])?;
    }
    for r in rows(src, "help_en/Help.csv")? {
        tx.execute("INSERT OR REPLACE INTO help VALUES(?,?)", params![r.s("key"), r.s("text")])?;
    }
    {
        let pages: Vec<(String, String)> = {
            let mut st = tx.prepare("SELECT key, text FROM help WHERE text LIKE '%PARAMETER SPECIFICATION%'")?;
            let r = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
            r
        };
        for (key, text) in pages {
            let (body, stopped) = hondascan::signalhelp::analyse(&text);
            tx.execute("INSERT INTO signal_help VALUES(?,?,?)", params![key, body, stopped as i64])?;
        }
    }
    for r in rows(src, "ecus.csv")? {
        tx.execute("INSERT INTO ecus VALUES(?,?,?)", params![r.s("system"), r.s("ecu_id_string"), r.s("ecu_name")])?;
    }
    for r in rows(src, "can_packets.csv")? {
        tx.execute("INSERT INTO can_packets VALUES(?,?,?,?)", params![r.s("system"), r.hex("did")?, r.i("size")?, r.i("kind")?])?;
    }
    for r in rows(src, "can_packet_usage.csv")? {
        tx.execute("INSERT INTO can_packet_usage VALUES(?,?,?)", params![r.s("system"), r.s("usage"), r.hex("did")?])?;
    }
    // offset 255 and DID FFFF are HDS placeholders for values that are not in a live DID (snapshot data,
    // OBD readiness); those are left out
    for r in rows(src, "can_signal_parts.csv")? {
        if r.s("did_in_packets") == "1" && r.s("offset") != "255" && r.s("did") != "FFFF" {
            signal_part(&tx, &r)?;
        }
    }
    // The K-line (VKWP) systems go into the same table, the local identifier in the `did` column.
    for r in rows(src, "kwp_signal_parts.csv")? {
        signal_part(&tx, &r)?;
    }
    for r in rows(src, "converters.csv")? {
        tx.execute("INSERT INTO converters VALUES(?,?,?,?)", params![r.i("row")?, r.i("equation_idx")?, r.s("formula"), r.i("out_unit")?])?;
    }
    for r in rows(src, "units.csv")? {
        tx.execute("INSERT OR REPLACE INTO units VALUES(?,?)", params![r.i("unit_id")?, r.s("unit")])?;
    }
    for r in rows(src, "converter_coefficients.csv")? {
        tx.execute("INSERT INTO converter_coefficients VALUES(?,?,?)", params![r.i("converter")?, r.i("index")?, r.f("value")?])?;
    }
    for r in rows(src, "converter_map.csv")? {
        tx.execute("INSERT INTO converter_map VALUES(?,?,?,?)", params![r.i("converter")?, r.i("index")?, r.f("x")?, r.f("y")?])?;
    }
    for r in rows(src, "converter_states.csv")? {
        tx.execute("INSERT INTO converter_states VALUES(?,?,?,?,?,?)", params![
            r.i("converter")?, r.i("index")?, r.f("low")?, r.f("high")?, r.s("text"), r.i("led")?
        ])?;
    }
    tx.execute_batch(INDEXES)?;
    tx.execute("INSERT INTO meta VALUES('source','HDS 3.105.12 decoded tables (hds-xref)')", [])?;
    tx.commit()?;

    let mut counts = Vec::new();
    for t in ["systems", "dtc", "help", "signal_help", "ecus", "can_signal_parts", "converters"] {
        let n: i64 = db.query_row(&format!("SELECT COUNT(*) FROM {t}"), [], |r| r.get(0))?;
        counts.push(format!("{t}: {n}"));
    }
    db.execute_batch("VACUUM")?;
    drop(db);
    println!("{} {{{}}} {:.1} MB", out.display(), counts.join(", "), std::fs::metadata(out)?.len() as f64 / 1e6);
    Ok(())
}

fn main() {
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let args: Vec<String> = std::env::args().skip(1).collect();
    let src = args.first().map(PathBuf::from).unwrap_or_else(|| crate_dir.join("../hds-xref/decoded"));
    let out = args.get(1).map(PathBuf::from).unwrap_or_else(|| crate_dir.join("data/hondascan.db"));
    if let Err(e) = build(&src, &out) {
        eprintln!("build-db: {e}");
        std::process::exit(1);
    }
}
