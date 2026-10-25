//! A session on disk, for people and agents: <data dir>/vehicles/<VIN>/<start time>/ with journal.jsonl (every event),
//! frames.log, summary.json and summary.md, and <data dir>/vehicles/<VIN>/vehicle.md (the car across sessions).
//! Port of hondascan/journal.py; the Markdown is built with Python's formatting so the files are the same text.
use crate::value::{dumps, Json};
use serde_json::{json, Map, Value};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const RECENT: usize = 20; // sessions listed one by one in vehicle.md; older ones are summed up in one line

pub struct Journal {
    pub folder: PathBuf,
    fh: Mutex<std::io::BufWriter<std::fs::File>>,
}

impl Journal {
    pub fn new(folder: &Path) -> std::io::Result<Journal> {
        std::fs::create_dir_all(folder)?;
        let f = std::fs::OpenOptions::new().append(true).create(true).open(folder.join("journal.jsonl"))?;
        Ok(Journal { folder: folder.to_path_buf(), fh: Mutex::new(std::io::BufWriter::with_capacity(1 << 16, f)) })
    }

    /// One event: {"t": unix time, "ev": kind, ...data}, one line.
    pub fn event(&self, ev: &str, data: Value) {
        let mut m = Map::new();
        m.insert("t".into(), json!(crate::value::round_to(crate::transport::can::now(), 3)));
        m.insert("ev".into(), json!(ev));
        if let Value::Object(d) = data {
            m.extend(d);
        }
        let line = dumps(&Value::Object(m), Json::Compact);
        let _ = writeln!(self.fh.lock().unwrap(), "{line}");
    }

    pub fn flush(&self) {
        let _ = self.fh.lock().unwrap().flush();
    }

    pub fn write_summary(&self, summary: &Value) {
        let _ = std::fs::write(self.folder.join("summary.json"), dumps(summary, Json::Indent(1)));
        let _ = std::fs::write(self.folder.join("summary.md"), render_summary(summary));
    }

    pub fn close(&self) {
        self.flush();
    }
}

// -- Python-flavoured formatting -------------------------------------------------------------------------------------------

/// Python's `repr(float)`: shortest round-trip digits, exponent form below 1e-4 and from 1e16.
fn py_float(x: f64) -> String {
    if x.is_nan() {
        return "nan".into();
    }
    if x.is_infinite() {
        return if x > 0.0 { "inf" } else { "-inf" }.into();
    }
    let a = x.abs();
    if a != 0.0 && !(1e-4..1e16).contains(&a) {
        let e = format!("{x:e}"); // 1e-5, 1.5e16
        let (m, ex) = e.split_once('e').unwrap();
        let n: i32 = ex.parse().unwrap();
        return format!("{m}e{}{:02}", if n < 0 { '-' } else { '+' }, n.abs());
    }
    let s = format!("{x}");
    if s.contains('.') { s } else { format!("{s}.0") }
}

/// Python's `str()` of a JSON value, as an f-string `{v}` shows it.
pub fn py(v: &Value) -> String {
    match v {
        Value::Null => "None".into(),
        Value::Bool(b) => if *b { "True" } else { "False" }.into(),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                i.to_string()
            } else if let Some(u) = n.as_u64() {
                u.to_string()
            } else {
                py_float(n.as_f64().unwrap_or(0.0))
            }
        }
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Python truthiness of a JSON value.
pub fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|x| x != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

fn get<'a>(v: &'a Value, k: &str) -> &'a Value {
    v.get(k).unwrap_or(&Value::Null)
}

fn num(v: &Value) -> f64 {
    v.as_f64().unwrap_or(0.0)
}

fn arr(v: &Value) -> &[Value] {
    v.as_array().map_or(&[][..], |a| a.as_slice())
}

/// `time.strftime('%Y-%m-%d %H:%M:%S', time.localtime(t))`, or '-' for 0 / None.
pub fn ts(t: &Value) -> String {
    let secs = num(t);
    if !truthy(t) {
        return "-".into();
    }
    crate::localtime::stamp(secs)
}

fn mm(d: &Value, unit: &str) -> String {
    if !truthy(d) {
        return "n/a".into();
    }
    format!("{}/{}/{} {unit}", py(get(d, "min")), py(get(d, "mean")), py(get(d, "max"))).trim().to_string()
}

fn secs_(s: &Value) -> String {
    if truthy(s) { format!("{} s", py(s)) } else { "n/a".into() }
}

fn kv(d: &Value, unit: &str) -> String {
    match d.as_object() {
        Some(o) if !o.is_empty() => o.iter().map(|(k, v)| format!("{} {}{unit}", k.replace('_', " "), py(v))).collect::<Vec<_>>().join(", "),
        _ => "n/a".into(),
    }
}

pub fn code(d: &Value) -> String {
    let ft = get(d, "failure_type");
    let suffix = if truthy(ft) { format!("-{:02X}", ft.as_i64().unwrap_or(0)) } else { String::new() };
    format!("{}{suffix}", py(get(d, "code")))
}

/// A code with its kind, system and description. Codes carried over from an earlier session have no description field;
/// they are not called "not in the database" for that.
pub fn dtc_line(d: &Value) -> String {
    let desc = match d.get("description") {
        Some(x) => format!(": {}", if truthy(x) { py(x) } else { "(not in the HDS database)".into() }),
        None => String::new(),
    };
    let kind = d.get("kind").map_or(String::new(), py);
    let system = d.get("system").map_or("?".into(), py);
    format!("`{}` {kind} in {system}{desc}", code(d))
}

/// Markdown for an agent or a person: facts first, then how the drive went, then what to look at.
pub fn render_summary(s: &Value) -> String {
    let mut l: Vec<String> = vec![];
    let mut p = |x: String| l.push(x);
    p(format!("# Session {} · {}", py(get(s, "id")), py(get(s, "vin"))));
    p(String::new());
    let dur = s.get("duration_s").map_or(0.0, num);
    p(format!(
        "- {} to {} ({dur:.0} s), adapter `{}`, ended: {}",
        ts(get(s, "started")),
        ts(get(s, "ended")),
        py(get(s, "adapter")),
        if truthy(get(s, "end_reason")) { py(get(s, "end_reason")) } else { "still running".into() }
    ));
    p(format!("- {} systems; scan {} s; {} reads, {} failed", arr(get(s, "ecus")).len(), py(get(s, "scan_s")), py(get(get(s, "reads"), "ok")), py(get(get(s, "reads"), "failed"))));
    p(String::new());
    for x in ["## Systems", "| address | type | database | identity |", "|---|---|---|---|"] {
        p(x.into());
    }
    for e in arr(get(s, "ecus")) {
        let ident = match get(e, "ident").as_object() {
            Some(o) if !o.is_empty() => o.iter().map(|(k, v)| format!("{k} {}", py(v))).collect::<Vec<_>>().join(", "),
            _ => "-".into(),
        };
        let system = if truthy(get(e, "system")) { py(get(e, "system")) } else { "-".into() };
        p(format!("| 0x{:02X} | {} | {system} | {ident} |", get(e, "address").as_i64().unwrap_or(0), py(get(e, "system_type"))));
    }
    let c = get(s, "codes");
    let n = |k: &str| arr(get(c, k)).len();
    p(String::new());
    p("## Trouble codes".into());
    p(format!("- at the end: {}; appeared during the session: {}; went away: {}; cleared by the user: {}", n("present"), n("appeared"), n("gone"), n("cleared")));
    p(if truthy(get(s, "first_session")) {
        "- first recorded session of this car".into()
    } else {
        format!("- since the previous session: new {}, resolved {}", n("new_since_last"), n("resolved_since_last"))
    });
    for (title, key) in [("Present", "present"), ("New since the previous session", "new_since_last"), ("Resolved since the previous session", "resolved_since_last"), ("Appeared during this session", "appeared")] {
        if !arr(get(c, key)).is_empty() {
            p(String::new());
            p(format!("### {title}"));
            for d in arr(get(c, key)) {
                p(format!("- {}", dtc_line(d)));
            }
        }
    }
    if let Some(tests) = get(s, "self_tests").as_object() {
        for (system, st) in tests {
            p(String::new());
            p(format!("## Self-tests not completed: {system}"));
            p(format!("- {} at the first read, {} now; completed during the session: {}", py(get(st, "at_start")), py(get(st, "now")), arr(get(st, "completed")).len()));
            if !arr(get(st, "completed")).is_empty() {
                p(String::new());
                p("### Completed during the session".into());
                for d in arr(get(st, "completed")) {
                    p(format!("- `{}`: {}", code(d), py(get(d, "description"))));
                }
            }
            if !arr(get(st, "incomplete")).is_empty() {
                p(String::new());
                p("### Still to run".into());
                for d in arr(get(st, "incomplete")) {
                    p(format!("- `{}`: {}", code(d), py(get(d, "description"))));
                }
            }
        }
    }
    let tr = get(s, "trip");
    if truthy(get(tr, "duration_s")) {
        let f = get(tr, "fuel");
        let eco = get(f, "economy");
        p(String::new());
        p("## Drive".into());
        p(format!("- distance {} km (speed integral); odometer change {}; data gaps {}", py(get(tr, "distance_km")), kv(get(tr, "odometer_km"), " km"), py(get(tr, "gaps"))));
        let run = if !get(tr, "engine_run_s").is_null() { format!("; engine running since its start (PCM): {} s", py(get(tr, "engine_run_s"))) } else { String::new() };
        p(format!("- time (recorded): {}{run}", kv(get(tr, "times_s"), " s")));
        let economy = if truthy(eco) {
            format!("{} mpg (US), {} L/100 km from the {}", py(get(eco, "mpg_us")), py(get(eco, "l_per_100km")), py(get(f, "economy_source")))
        } else {
            "n/a".into()
        };
        p(format!("- fuel: engine counter {} L, MAF estimate {} L; economy {economy}", py(get(f, "engine_counter_l")), py(get(f, "maf_estimate_l"))));
        let b = get(tr, "battery_v");
        p(format!(
            "- min/mean/max: speed {}, rpm {}, coolant {} (warm-up {}), battery running {}, battery off {}, brake pressure {}, lateral {}",
            mm(get(tr, "speed_kmh"), "km/h"), mm(get(tr, "rpm"), ""), mm(get(tr, "coolant_c"), "degC"), secs_(get(tr, "warmup_s")),
            mm(get(b, "engine_running"), "V"), mm(get(b, "engine_off"), "V"), mm(get(tr, "brake_pressure_bar"), "bar"), mm(get(tr, "lateral_g"), "G")
        ));
    }
    if !arr(get(s, "notes")).is_empty() {
        p(String::new());
        p("## Worth a look".into());
        for n in arr(get(s, "notes")) {
            p(format!("- {}", py(n)));
        }
    }
    let lat = get(s, "latency_ms");
    if truthy(lat) || truthy(get(s, "rejected")) || truthy(get(s, "unanswered")) {
        p(String::new());
        p("## Bus".into());
        let items = lat.as_object().map_or(String::new(), |o| o.iter().map(|(k, v)| format!("{k} {}", py(v))).collect::<Vec<_>>().join(", "));
        p(format!("- response time per system (ms, min/mean/max): {items}"));
        if truthy(get(s, "rejected")) {
            p(format!("- data groups the car refused: {}", get(s, "rejected").as_object().unwrap().iter().map(|(k, v)| format!("{k}: {}", py(v))).collect::<Vec<_>>().join(", ")));
        }
        if truthy(get(s, "unanswered")) {
            p(format!("- data groups without an answer (reads): {}", get(s, "unanswered").as_object().unwrap().iter().map(|(k, v)| format!("{k}: {}", py(v))).collect::<Vec<_>>().join(", ")));
        }
    }
    p(String::new());
    p("Files: `journal.jsonl` (every event), `frames.log` (every frame), `summary.json` (this, as data).".into());
    p(String::new());
    l.join("\n")
}

/// A note with its numbers masked (`\d+(?:\.\d+)?` -> '#'), so the same finding in two sessions compares equal.
fn pattern(note: &str) -> String {
    let c: Vec<char> = note.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < c.len() {
        if c[i].is_ascii_digit() {
            while i < c.len() && c[i].is_ascii_digit() {
                i += 1;
            }
            if i + 1 < c.len() && c[i] == '.' && c[i + 1].is_ascii_digit() {
                i += 1;
                while i < c.len() && c[i].is_ascii_digit() {
                    i += 1;
                }
            }
            out.push('#');
        } else {
            out.push(c[i]);
            i += 1;
        }
    }
    out
}

/// The car across sessions, for an agent to keep in context: the state now, what keeps coming back, the code history, and
/// one row per recent session pointing at its folder. `sessions` is newest first, as Store::sessions returns them;
/// `trends` is {name: [per-session rows]}.
pub fn render_vehicle(vin: &str, sessions: &[Value], codes: &[Value], trends: &Map<String, Value>) -> String {
    let null = Value::Null;
    let sm: Vec<&Value> = sessions.iter().map(|s| s.get("summary").filter(|x| truthy(x)).unwrap_or(&null)).collect();
    let km: Vec<f64> = sm.iter().map(|x| num(get(get(x, "trip"), "distance_km"))).collect();
    let mut l: Vec<String> = vec![format!("# {vin}"), String::new()];
    if sessions.is_empty() {
        l.push("No sessions recorded.".into());
        l.push(String::new());
        return l.join("\n");
    }
    l.push(format!(
        "{} session{} from {} to {}, {:.1} km recorded. Each session's folder (next to this file) has its summary.md with the detail.",
        sessions.len(),
        if sessions.len() > 1 { "s" } else { "" },
        ts(get(&sessions[sessions.len() - 1], "started")),
        ts(get(&sessions[0], "started")),
        crate::value::pysum(&km)
    ));
    l.push(String::new());
    let (last, now) = (&sessions[0], sm[0]);
    let c = get(now, "codes");
    l.push(format!("## Now (session {}, {}, ended: {})", py(get(last, "id")), ts(get(last, "started")), if truthy(get(last, "end_reason")) { py(get(last, "end_reason")) } else { "still running".into() }));
    if arr(get(c, "present")).is_empty() {
        l.push("- no trouble codes at the last read".into());
    } else {
        l.extend(arr(get(c, "present")).iter().map(|d| format!("- {}", dtc_line(d))));
    }
    if !arr(get(c, "new_since_last")).is_empty() || !arr(get(c, "resolved_since_last")).is_empty() {
        let list = |k: &str| {
            let v: Vec<String> = arr(get(c, k)).iter().map(code).collect();
            if v.is_empty() { "none".to_string() } else { v.join(", ") }
        };
        l.push(format!("- since the session before: new {}; gone {}", list("new_since_last"), list("resolved_since_last")));
    }
    l.extend(arr(get(now, "notes")).iter().map(|n| format!("- worth a look: {}", py(n))));

    let recent = &sm[..sm.len().min(RECENT)];
    let mut seen: Vec<(String, usize, String)> = vec![]; // pattern -> (count, newest wording)
    for x in recent {
        let mut once: Vec<String> = vec![];
        for n in arr(get(x, "notes")) {
            let n = py(n);
            if !once.contains(&n) {
                once.push(n);
            }
        }
        for n in once {
            let pt = pattern(&n);
            match seen.iter_mut().find(|(k, _, _)| *k == pt) {
                Some(e) => e.1 += 1,
                None => seen.push((pt, 1, n)),
            }
        }
    }
    let mut again: Vec<&(String, usize, String)> = seen.iter().filter(|e| e.1 > 1).collect();
    again.sort_by_key(|e| std::cmp::Reverse(e.1));
    if !again.is_empty() {
        l.push(String::new());
        l.push(format!("## Recurring findings (last {} sessions)", recent.len()));
        l.extend(again.iter().map(|(_, k, n)| format!("- in {k} sessions: {n}")));
    }
    if !codes.is_empty() {
        l.push(String::new());
        l.push("## Trouble code history".into());
        l.push("| code | system | first seen | last seen | sessions | last event |".into());
        l.push("|---|---|---|---|---|---|".into());
        for c in codes {
            l.push(format!("| {} | {} | {} | {} | {} | {} |", code(c), py(get(c, "system")), ts(get(c, "first_seen")), ts(get(c, "last_seen")), py(get(c, "sessions")), py(get(c, "last_event"))));
        }
    }
    l.push(String::new());
    l.push(format!("## Sessions (newest first, last {})", recent.len()));
    l.push("| start | folder | duration | km | economy | codes | notes | ended |".into());
    l.push("|---|---|---|---|---|---|---|---|".into());
    for (s, x) in sessions.iter().zip(recent) {
        let tr = get(x, "trip");
        let eco = get(get(tr, "fuel"), "economy");
        let or_dash = |v: &Value| if v.is_null() { "-".to_string() } else { py(v) };
        let dir = std::path::Path::new(&py(get(s, "dir"))).file_name().map_or(String::new(), |f| f.to_string_lossy().into_owned());
        l.push(format!(
            "| {} | `{dir}` | {:.0} s | {} | {} mpg | {} | {} | {} |",
            ts(get(s, "started")), num(get(x, "duration_s")), or_dash(get(tr, "distance_km")), or_dash(get(eco, "mpg_us")),
            arr(get(get(x, "codes"), "present")).len(), arr(get(x, "notes")).len(), if truthy(get(s, "end_reason")) { py(get(s, "end_reason")) } else { "-".into() }
        ));
    }
    if sessions.len() > RECENT {
        let older = &sessions[RECENT..];
        l.push(format!(
            "\n{} older sessions ({} to {}, {:.1} km) are in history.db and their folders.",
            older.len(), ts(get(&older[older.len() - 1], "started")), ts(get(&older[0], "started")), crate::value::pysum(&km[RECENT..])
        ));
    }
    if !trends.is_empty() {
        l.push(String::new());
        l.push("## Trends (per session mean, oldest first)".into());
        for (name, rows) in trends {
            let rows = arr(rows);
            if rows.is_empty() {
                continue; // an empty row list has no last row to take the unit from
            }
            let unit = if truthy(get(&rows[rows.len() - 1], "unit")) { format!(" ({})", py(get(&rows[rows.len() - 1], "unit"))) } else { String::new() };
            let tail = &rows[rows.len().saturating_sub(12)..];
            l.push(format!("- {name}{unit}: {}", tail.iter().map(|r| format!("{:.2}", num(get(r, "mean")))).collect::<Vec<_>>().join(", ")));
        }
    }
    l.join("\n") + "\n"
}
