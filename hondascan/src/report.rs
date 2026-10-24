//! The health report: what the car reports now, compared with the newest frozen healthy baseline, as one document a
//! person can read (HTML for printing, Markdown for notes, JSON for the app).
//!
//! The report is a read-only record. `build` reads the car through `maintenance::read_car` (the same live read the freeze
//! gate uses), adds the store's markers and the write audit, and compares the codes and self-tests against the newest
//! baseline that still parses. The verdict is deliberately conservative: `unknown` when a system did not answer its DTC
//! read or the VIN was never read; `attention` for a confirmed or pending code, a self-test not run, a new code or a
//! readiness change; `healthy` otherwise. Stored codes are history and never make a car `attention` on their own.
//!
//! Every piece of car or user text in the HTML goes through `escape`: descriptions, marker notes, audit details and the
//! VIN are data, not markup.
use crate::service::{Service, ServiceError};
use crate::transport::can::now;
use crate::value::{dumps, Json};
use serde_json::{json, Value};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// The report's JSON schema. A new field may be added; changing one needs a new number.
pub const SCHEMA: u64 = 1;

/// The sentence every report ends with; the same text goes into the JSON so a reader of either form sees it.
pub const DISCLAIMER: &str = "This report is a read-only record of what the car reported. It changes nothing on the car.";

fn text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// A system's name for people: the display name, else the catalog system, else its address.
fn name(system: &Value) -> String {
    for key in ["display", "system", "system_type"] {
        if let Some(s) = system[key].as_str().filter(|s| !s.is_empty()) {
            return s.to_string();
        }
    }
    match system["address"].as_i64() {
        Some(a) => format!("0x{a:02X}"),
        None => "?".into(),
    }
}

/// A system's DTC read answered. A system that did not answer is never compared: its baseline codes are not resolved.
fn answered(system: &Value) -> bool {
    system["answered"].as_bool().unwrap_or(false)
}

/// The identity of a trouble code in a comparison: the code and its failure type (the system is the loop's context).
fn code_id(dtc: &Value) -> (String, Option<i64>) {
    (dtc["code"].as_str().unwrap_or("").to_string(), dtc["failure_type"].as_i64())
}

fn has_code(list: &[Value], dtc: &Value) -> bool {
    let id = code_id(dtc);
    list.iter().any(|d| code_id(d) == id)
}

/// One comparison entry, in the shape the app and the HTML both read.
fn code_item(system: &Value, dtc: &Value) -> Value {
    json!({
        "system": name(system),
        "code": dtc["code"].clone(),
        "failure_type": dtc["failure_type"].clone(),
        "kind": dtc["kind"].clone(),
        "description": dtc["description"].clone(),
    })
}

/// The self-tests an ECU still has to run, as a sorted list of (code, failure type): the set the comparison uses.
/// The session's `self_tests.incomplete` is the contract's source; a payload without one falls back to the DTC list.
fn incomplete_set(system: &Value) -> Vec<(String, Option<i64>)> {
    let list = system["self_tests"]["incomplete"].as_array().or_else(|| system["incomplete"].as_array());
    let mut out: Vec<(String, Option<i64>)> = list
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .map(|d| (d["code"].as_str().unwrap_or("").to_string(), d["failure_type"].as_i64()))
        .collect();
    out.sort();
    out
}

/// The baseline's newest payload that still parses, with its metadata and integrity verdict merged in. A damaged newest
/// row does not hide the one before it: only a payload this reader can compare against is useful.
fn newest_baseline(store: &crate::store::Store, vin: &str) -> Option<Value> {
    let metas = store.baselines(vin);
    for (id, payload, sha) in store.baseline_payloads(vin) {
        let Ok(mut v) = serde_json::from_str::<Value>(&payload) else { continue };
        let Some(meta) = metas.iter().find(|m| m["id"].as_i64() == Some(id)) else { continue };
        let Some(object) = v.as_object_mut() else { continue };
        object.insert("id".into(), json!(id));
        object.insert("created".into(), meta["created"].clone());
        object.insert("label".into(), meta["label"].clone());
        object.insert("sha256".into(), json!(sha));
        object.insert("integrity".into(), json!(crate::maintenance::integrity(&payload, &sha)));
        return Some(v);
    }
    None
}

/// The report object `health_report` and `report` share. `vin` is optional: the report is about the connected car, and a
/// different VIN is refused rather than read (the bus is one car at a time).
pub fn build(svc: &mut Service, vin: Option<&str>) -> Result<Value, ServiceError> {
    let car = crate::maintenance::read_car(svc)?;
    let scanned = svc.vin.clone().unwrap_or_default();
    let given = vin.map(str::trim).filter(|v| !v.is_empty());
    let scanned_known = !scanned.is_empty() && !scanned.eq_ignore_ascii_case("UNKNOWN");
    if let Some(v) = given {
        if scanned_known && !v.eq_ignore_ascii_case(&scanned) {
            return Err(ServiceError::value(format!("the report cannot be about another car: the connected car is {scanned}, not {v}")));
        }
    }
    let vin = given.filter(|_| scanned_known).map(String::from).unwrap_or_else(|| scanned.clone());
    let known = !vin.is_empty() && !vin.eq_ignore_ascii_case("UNKNOWN");

    let store = svc.store()?;
    let root = store.root.clone();
    let markers = if known { store.markers(&vin) } else { vec![] };
    let writes = if known { crate::audit::read(&root, Some(&vin), 50) } else { vec![] };
    let baseline = if known { newest_baseline(&store, &vin) } else { None };

    let comparison = compare(&car, baseline.as_ref());
    let verdict = verdict(&car, &vin, &comparison);
    let summary = summary(&car, &comparison, verdict);

    let systems: Vec<Value> = car["systems"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .map(|s| {
            let mut out = json!({
                "address": s["address"].clone(),
                "system": s["system"].clone(),
                "display": s["display"].clone(),
                "answered": s["answered"].clone(),
                "dtcs": s["dtcs"].clone(),
                "incomplete": s["incomplete"].clone(),
                "self_tests": s["self_tests"].clone(),
                "readiness": s["readiness"].clone(),
            });
            if s.get("error").is_some() {
                out["error"] = s["error"].clone();
            }
            out
        })
        .collect();

    Ok(json!({
        "schema": SCHEMA,
        "vin": vin,
        "generated": now(),
        "adapter": car["adapter"].clone(),
        "session": car["session"].clone(),
        "verdict": verdict,
        "summary": summary,
        "totals": car["totals"].clone(),
        "systems": systems,
        "comparison": comparison,
        "markers": markers,
        "writes": writes,
        "disclaimer": DISCLAIMER,
    }))
}

/// What changed against a frozen baseline: new, still present and resolved codes, systems the baseline had and this read
/// did not, and readiness changes (the set of self-tests not run per system). Without a baseline the comparison is empty.
pub fn compare(car: &Value, baseline: Option<&Value>) -> Value {
    let mut new = vec![];
    let mut still = vec![];
    let mut resolved = vec![];
    let mut systems_not_read: Vec<String> = vec![];
    let mut readiness_changes: Vec<String> = vec![];

    let Some(baseline) = baseline else {
        return json!({"baseline": Value::Null, "new": new, "still": still, "resolved": resolved,
                      "systems_not_read": systems_not_read, "readiness_changes": readiness_changes});
    };
    let current = car["systems"].as_array().map_or(&[][..], Vec::as_slice);
    let base_systems = baseline["systems"].as_array().map_or(&[][..], Vec::as_slice);

    for s in current {
        let key = (s["address"].as_i64(), s["system"].as_str().unwrap_or(""));
        let found = base_systems.iter().find(|b| (b["address"].as_i64(), b["system"].as_str().unwrap_or("")) == key);
        let Some(b) = found else {
            // A system the baseline did not have: its codes are new, and there is no readiness set to compare.
            if answered(s) {
                for c in s["dtcs"].as_array().map_or(&[][..], Vec::as_slice) {
                    new.push(code_item(s, c));
                }
            } else {
                systems_not_read.push(name(s));
            }
            continue;
        };
        if !answered(s) {
            systems_not_read.push(name(s));
            continue;
        }
        let b_dtcs = b["dtcs"].as_array().map_or(&[][..], Vec::as_slice);
        let c_dtcs = s["dtcs"].as_array().map_or(&[][..], Vec::as_slice);
        for c in c_dtcs {
            if has_code(b_dtcs, c) {
                still.push(code_item(s, c));
            } else {
                new.push(code_item(s, c));
            }
        }
        for c in b_dtcs {
            if !has_code(c_dtcs, c) {
                resolved.push(code_item(s, c));
            }
        }
        let (was, is) = (incomplete_set(b), incomplete_set(s));
        if was != is {
            readiness_changes.push(format!(
                "{}: {} self-test{} not run (baseline {})",
                name(s),
                is.len(),
                if is.len() == 1 { "" } else { "s" },
                was.len()
            ));
        }
    }

    for b in base_systems {
        let key = (b["address"].as_i64(), b["system"].as_str().unwrap_or(""));
        if !current.iter().any(|s| (s["address"].as_i64(), s["system"].as_str().unwrap_or("")) == key) && !systems_not_read.contains(&name(b)) {
            systems_not_read.push(name(b));
        }
    }

    let meta = json!({
        "id": baseline["id"].clone(),
        "created": baseline["created"].clone(),
        "label": baseline["label"].clone(),
        "sha256": baseline["sha256"].clone(),
        "integrity": baseline["integrity"].clone(),
    });
    json!({"baseline": meta, "new": new, "still": still, "resolved": resolved,
           "systems_not_read": systems_not_read, "readiness_changes": readiness_changes})
}

/// The binding verdict rule: `unknown` when the VIN was not read or a system's trouble codes were not read; `attention`
/// for a confirmed or pending code, a self-test not run, a new code (stored-only history does not count) or a readiness
/// change; `healthy` otherwise.
pub fn verdict(car: &Value, vin: &str, comparison: &Value) -> &'static str {
    let car_vin = car["vin"].as_str().unwrap_or("");
    let known = |v: &str| !v.trim().is_empty() && !v.eq_ignore_ascii_case("UNKNOWN");
    let unread = car["systems"].as_array().is_some_and(|a| a.iter().any(|s| !answered(s)));
    if !known(vin) || !known(car_vin) || unread {
        return "unknown";
    }
    let totals = &car["totals"];
    let count = |k: &str| totals[k].as_u64().unwrap_or(0);
    let new_attention = comparison["new"].as_array().is_some_and(|a| a.iter().any(|c| c["kind"].as_str() != Some("stored")));
    let changed = comparison["readiness_changes"].as_array().is_some_and(|a| !a.is_empty());
    if count("faults") > 0 || count("pending") > 0 || count("incomplete") > 0 || new_attention || changed {
        "attention"
    } else {
        "healthy"
    }
}

/// The baseline's creation time as a local date (YYYY-MM-DD), or `-`.
fn baseline_date(comparison: &Value) -> String {
    let created = &comparison["baseline"]["created"];
    let ts = crate::journal::ts(created);
    if ts == "-" { ts } else { ts.chars().take(10).collect() }
}

fn plural(n: u64, one: &str, many: &str) -> String {
    if n == 1 { one.to_string() } else { many.to_string() }
}

/// One plain sentence saying what the verdict means, with the counts a person can act on.
pub fn summary(car: &Value, comparison: &Value, verdict: &str) -> String {
    let totals = &car["totals"];
    let count = |k: &str| totals[k].as_u64().unwrap_or(0);
    if verdict == "unknown" {
        let unread: Vec<String> = car["systems"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[])
            .iter()
            .filter(|s| !answered(s))
            .map(name)
            .collect();
        if !unread.is_empty() {
            return format!(
                "The car's health is unknown: {} did not answer the trouble-code read.",
                unread.join(", ")
            );
        }
        return "The car's health is unknown: the VIN was never read, so this report cannot identify the car.".into();
    }
    if verdict == "attention" {
        let mut parts: Vec<String> = vec![];
        let (faults, pending, incomplete) = (count("faults"), count("pending"), count("incomplete"));
        if faults > 0 {
            parts.push(format!("{faults} confirmed trouble {}", plural(faults, "code", "codes")));
        }
        if pending > 0 {
            parts.push(format!("{pending} pending trouble {}", plural(pending, "code", "codes")));
        }
        if incomplete > 0 {
            parts.push(format!("{incomplete} self-test{} not run", if incomplete == 1 { "" } else { "s" }));
        }
        let new = comparison["new"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[])
            .iter()
            .filter(|c| c["kind"].as_str() != Some("stored"))
            .count() as u64;
        if new > 0 {
            parts.push(format!("{new} new trouble {} since the baseline", plural(new, "code", "codes")));
        }
        let changed = comparison["readiness_changes"].as_array().map_or(0, Vec::len) as u64;
        if changed > 0 {
            parts.push(format!("{changed} readiness {}", plural(changed, "change", "changes")));
        }
        if parts.is_empty() {
            return "This report needs attention.".into();
        }
        return format!("{}.", parts.join(", "));
    }
    if comparison["baseline"].is_null() {
        return "No confirmed or pending trouble codes and no self-test left to run; no reference snapshot to compare against.".into();
    }
    format!("Nothing new since the baseline of {}: no confirmed or pending trouble codes and no self-test left to run.", baseline_date(comparison))
}

/// HTML-escape one piece of text: `&`, `<`, `>`, `"` and `'`. The only way car and user text reaches the HTML.
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// A table whose cells are raw text: `table` escapes every cell, so a description can never become markup.
fn table(headers: &[&str], rows: &[Vec<String>]) -> String {
    let mut out = String::new();
    out.push_str("<table><thead><tr>");
    for h in headers {
        let _ = write!(out, "<th>{}</th>", escape(h));
    }
    out.push_str("</tr></thead><tbody>");
    for row in rows {
        out.push_str("<tr>");
        for cell in row {
            let _ = write!(out, "<td>{}</td>", escape(cell));
        }
        out.push_str("</tr>");
    }
    out.push_str("</tbody></table>");
    out
}

fn number(v: &Value) -> String {
    match v {
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn mileage(v: &Value) -> String {
    v.as_f64().map_or_else(|| "not recorded".to_string(), |km| format!("{km} km"))
}

/// The whole report as one self-contained printable page: inline CSS only, no scripts, no external references.
pub fn html(report: &Value) -> String {
    let vin = text(&report["vin"]);
    let verdict = report["verdict"].as_str().unwrap_or("unknown");
    let mut w = String::new();
    w.push_str("<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n");
    w.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
    w.push_str("<title>hondascan health report</title>\n<style>\n");
    w.push_str("body{font:15px/1.45 system-ui,-apple-system,\"Segoe UI\",sans-serif;color:#1d2329;max-width:980px;margin:2rem auto;padding:0 1.2rem}\n");
    w.push_str("h1{font-size:1.6rem;margin:0 0 .2rem}h2{font-size:1.2rem;margin:2rem 0 .6rem;border-bottom:1px solid #d7dde3;padding-bottom:.25rem}\n");
    w.push_str("h3{font-size:1rem;margin:1.2rem 0 .4rem}.facts td{padding:.15rem 1rem .15rem 0;vertical-align:top}\n");
    w.push_str(".verdict{font-weight:700;text-transform:uppercase;letter-spacing:.04em;margin:.8rem 0 .2rem}\n");
    w.push_str(".healthy{color:#1b6b3a}.attention{color:#9a5b00}.unknown{color:#6b7280}.muted{color:#5b6570}\n");
    w.push_str(".warn{background:#fff4e5;border-left:4px solid #d08b18;padding:.5rem .8rem;margin:.6rem 0}\n");
    w.push_str("table{border-collapse:collapse;width:100%;margin:.4rem 0 1rem}th,td{text-align:left;padding:.3rem .5rem;border-bottom:1px solid #e3e8ed;vertical-align:top}\n");
    w.push_str("th{font-size:.85rem;text-transform:uppercase;letter-spacing:.03em;color:#5b6570}code{font-family:ui-monospace,monospace}\n");
    w.push_str("footer{margin-top:2.5rem;border-top:1px solid #d7dde3;padding-top:.6rem;color:#5b6570;font-size:.85rem}\n");
    w.push_str("@media print{body{margin:0;max-width:none;padding:0}h2{break-after:avoid}table{break-inside:auto}tr{break-inside:avoid}}\n");
    w.push_str("</style>\n</head>\n<body>\n");

    w.push_str("<h1>hondascan health report</h1>\n");
    let _ = writeln!(
        w,
        "<p class=\"muted\">hondascan {} &middot; generated {}</p>",
        escape(env!("CARGO_PKG_VERSION")),
        escape(&crate::journal::ts(&report["generated"]))
    );
    w.push_str("<table class=\"facts\"><tbody>");
    for (label, value) in [
        ("VIN", vin.clone()),
        ("Adapter", text(&report["adapter"])),
        ("Session", number(&report["session"])),
    ] {
        let _ = writeln!(w, "<tr><th>{}</th><td>{}</td></tr>", escape(label), escape(&value));
    }
    w.push_str("</tbody></table>\n");

    let _ = writeln!(w, "<p class=\"verdict {}\">{}</p>", escape(verdict), escape(&verdict.to_uppercase()));
    let _ = writeln!(w, "<p>{}</p>", escape(report["summary"].as_str().unwrap_or("")));

    // Trouble codes and self-tests per system.
    w.push_str("<h2>Trouble codes</h2>\n");
    let systems = report["systems"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    if systems.is_empty() {
        w.push_str("<p class=\"muted\">The scan found no systems to read.</p>\n");
    }
    let mut readiness_any = false;
    for s in systems {
        let address = s["address"].as_i64().unwrap_or(0);
        let catalog = s["system"].as_str().unwrap_or("");
        let _ = writeln!(w, "<h3>{} ({} 0x{:02X})</h3>", escape(&name(s)), escape(catalog), address);
        if !answered(s) {
            let _ = writeln!(
                w,
                "<p class=\"warn\">No answer to the trouble-code read{}. Nothing about this system is known.</p>",
                s["error"].as_str().map_or(String::new(), |e| format!(": {}", escape(e)))
            );
            continue;
        }
        let dtcs = s["dtcs"].as_array().map(Vec::as_slice).unwrap_or(&[]);
        if dtcs.is_empty() {
            w.push_str("<p>No trouble codes reported.</p>\n");
        } else {
            let rows: Vec<Vec<String>> = dtcs
                .iter()
                .map(|d| {
                    vec![
                        text(&d["code"]),
                        number(&d["failure_type"]),
                        text(&d["kind"]),
                        text(&d["description"]),
                        text(&d["honda_code"]),
                    ]
                })
                .collect();
            w.push_str(&table(&["Code", "Failure type", "Status", "Description", "Honda code"], &rows));
        }
        let incomplete = s["incomplete"].as_array().map(Vec::as_slice).unwrap_or(&[]);
        if !incomplete.is_empty() {
            w.push_str("<p><strong>Self-tests not run</strong></p>");
            let rows: Vec<Vec<String>> = incomplete
                .iter()
                .map(|d| vec![text(&d["code"]), number(&d["failure_type"]), text(&d["description"])])
                .collect();
            w.push_str(&table(&["Code", "Failure type", "Description"], &rows));
        }
        let readiness = s["readiness"].as_array().map(Vec::as_slice).unwrap_or(&[]);
        for group in readiness {
            let signals = group["signals"].as_array().map(Vec::as_slice).unwrap_or(&[]);
            if signals.is_empty() {
                continue;
            }
            readiness_any = true;
            let _ = writeln!(w, "<p><strong>Readiness group 0x{:04X}</strong></p>", group["did"].as_u64().unwrap_or(0));
            let rows: Vec<Vec<String>> = signals
                .iter()
                .map(|sig| {
                    let value = if !sig["state"].is_null() { text(&sig["state"]) } else { number(&sig["value"]) };
                    vec![text(&sig["symbol"]), value, text(&sig["unit"])]
                })
                .collect();
            w.push_str(&table(&["Signal", "Value or state", "Unit"], &rows));
        }
    }
    if !readiness_any {
        w.push_str("<p class=\"muted\">The catalog defines no OBD readiness groups for the systems found.</p>\n");
    }

    // Comparison with the newest frozen baseline.
    w.push_str("<h2>Comparison with the newest reference snapshot</h2>\n");
    let comparison = &report["comparison"];
    if comparison["baseline"].is_null() {
        w.push_str("<p>No reference snapshot yet. Save one on a good day to compare later reports against it.</p>\n");
    } else {
        let b = &comparison["baseline"];
        let rows = vec![vec![
            crate::journal::ts(&b["created"]),
            text(&b["label"]),
            text(&b["integrity"]),
        ]];
        w.push_str(&table(&["Frozen", "Label", "Integrity"], &rows));
        if b["integrity"].as_str() != Some("ok") {
            w.push_str("<p class=\"warn\">The stored baseline no longer matches its SHA-256. It is shown as it is, but its comparison may be unreliable.</p>\n");
        }
        let section = |title: &str, key: &str, empty: &str, out: &mut String| {
            let list = comparison[key].as_array().map(Vec::as_slice).unwrap_or(&[]);
            let _ = write!(out, "<h3>{}</h3>", escape(title));
            if list.is_empty() {
                let _ = writeln!(out, "<p class=\"muted\">{}</p>", escape(empty));
                return;
            }
            let rows: Vec<Vec<String>> = list
                .iter()
                .map(|c| vec![text(&c["system"]), text(&c["code"]), number(&c["failure_type"]), text(&c["kind"]), text(&c["description"])])
                .collect();
            out.push_str(&table(&["System", "Code", "Failure type", "Status", "Description"], &rows));
        };
        section("New since the baseline", "new", "No new trouble codes.", &mut w);
        section("Still present", "still", "No codes are still present.", &mut w);
        section("Resolved since the baseline", "resolved", "No codes were resolved.", &mut w);
        let not_read = comparison["systems_not_read"].as_array().map(Vec::as_slice).unwrap_or(&[]);
        w.push_str("<h3>Systems not read</h3>");
        if not_read.is_empty() {
            w.push_str("<p class=\"muted\">Every system the baseline had was read again.</p>\n");
        } else {
            w.push_str("<ul>");
            for s in not_read {
                let _ = write!(w, "<li>{}</li>", escape(s.as_str().unwrap_or("")));
            }
            w.push_str("</ul>\n");
        }
        let changes = comparison["readiness_changes"].as_array().map(Vec::as_slice).unwrap_or(&[]);
        w.push_str("<h3>Readiness changes</h3>");
        if changes.is_empty() {
            w.push_str("<p class=\"muted\">The set of self-tests not run is the same as the baseline's.</p>\n");
        } else {
            w.push_str("<ul>");
            for c in changes {
                let _ = write!(w, "<li>{}</li>", escape(c.as_str().unwrap_or("")));
            }
            w.push_str("</ul>\n");
        }
    }

    // Service markers.
    w.push_str("<h2>Service markers</h2>\n");
    let markers = report["markers"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    if markers.is_empty() {
        w.push_str("<p class=\"muted\">No service markers recorded for this car.</p>\n");
    } else {
        let rows: Vec<Vec<String>> = markers
            .iter()
            .map(|m| vec![crate::journal::ts(&m["at"]), text(&m["kind"]), text(&m["note"]), mileage(&m["mileage_km"])])
            .collect();
        w.push_str(&table(&["Recorded", "Kind", "Note", "Mileage"], &rows));
    }

    // The write audit trail.
    w.push_str("<h2>Recent writes</h2>\n");
    let writes = report["writes"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    if writes.is_empty() {
        w.push_str("<p class=\"muted\">No writes recorded for this car.</p>\n");
    } else {
        let rows: Vec<Vec<String>> = writes
            .iter()
            .map(|r| {
                let outcome = r["outcome"].as_str().or_else(|| r["event"].as_str()).unwrap_or("");
                let ok = match r["ok"].as_bool() {
                    Some(true) => "ok",
                    Some(false) => "failed",
                    None => "",
                };
                let label = format!("{outcome} {ok}");
                vec![crate::journal::ts(&r["at"]), text(&r["system"]), text(&r["name"]), label.trim().to_string()]
            })
            .collect();
        w.push_str(&table(&["Time", "System", "Function", "Outcome"], &rows));
    }

    let _ = writeln!(w, "<p class=\"muted\">{}</p>", escape(report["disclaimer"].as_str().unwrap_or(DISCLAIMER)));
    let _ = writeln!(w, "<footer>hondascan {} &middot; this page is self-contained; print it or keep the JSON beside it.</footer>", escape(env!("CARGO_PKG_VERSION")));
    w.push_str("</body>\n</html>\n");
    w
}

/// The same report as Markdown, line by line (`report --out x.md`). Nothing is escaped: Markdown is text for a person.
pub fn markdown(report: &Value) -> String {
    let mut w = String::new();
    let _ = writeln!(w, "# hondascan health report");
    let _ = writeln!(w);
    let _ = writeln!(w, "- VIN: {}", text(&report["vin"]));
    let _ = writeln!(w, "- Generated: {}", crate::journal::ts(&report["generated"]));
    let _ = writeln!(w, "- Adapter: {}", text(&report["adapter"]));
    let _ = writeln!(w, "- Session: {}", number(&report["session"]));
    let _ = writeln!(w);
    let _ = writeln!(w, "**Verdict: {}**", report["verdict"].as_str().unwrap_or("unknown"));
    let _ = writeln!(w);
    let _ = writeln!(w, "{}", report["summary"].as_str().unwrap_or(""));
    let _ = writeln!(w);

    let _ = writeln!(w, "## Trouble codes");
    let _ = writeln!(w);
    for s in report["systems"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
        let _ = writeln!(w, "### {} ({}, 0x{:02X})", name(s), text(&s["system"]), s["address"].as_u64().unwrap_or(0));
        if !answered(s) {
            let _ = writeln!(w, "No answer to the trouble-code read{}.", s["error"].as_str().map_or(String::new(), |e| format!(": {e}")));
            let _ = writeln!(w);
            continue;
        }
        let dtcs = s["dtcs"].as_array().map(Vec::as_slice).unwrap_or(&[]);
        if dtcs.is_empty() {
            let _ = writeln!(w, "- No trouble codes reported.");
        }
        for d in dtcs {
            let _ = writeln!(
                w,
                "- `{}`-{} {} ({}): {}",
                text(&d["code"]),
                number(&d["failure_type"]),
                text(&d["kind"]),
                text(&d["honda_code"]),
                text(&d["description"])
            );
        }
        for d in s["incomplete"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
            let _ = writeln!(w, "- self-test not run: `{}`-{} {}", text(&d["code"]), number(&d["failure_type"]), text(&d["description"]));
        }
        for group in s["readiness"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
            let signals = group["signals"].as_array().map(Vec::as_slice).unwrap_or(&[]);
            if signals.is_empty() {
                continue;
            }
            let parts: Vec<String> = signals
                .iter()
                .map(|sig| {
                    let value = if !sig["state"].is_null() { text(&sig["state"]) } else { number(&sig["value"]) };
                    format!("{} {}{}", text(&sig["symbol"]), value, text(&sig["unit"]))
                })
                .collect();
            let _ = writeln!(w, "- readiness 0x{:04X}: {}", group["did"].as_u64().unwrap_or(0), parts.join(", "));
        }
        let _ = writeln!(w);
    }

    let _ = writeln!(w, "## Comparison with the newest reference snapshot");
    let _ = writeln!(w);
    let comparison = &report["comparison"];
    if comparison["baseline"].is_null() {
        let _ = writeln!(w, "No reference snapshot yet.");
    } else {
        let b = &comparison["baseline"];
        let _ = writeln!(
            w,
            "Baseline: {} ({}), integrity {}.",
            crate::journal::ts(&b["created"]),
            text(&b["label"]),
            text(&b["integrity"])
        );
        for (key, title) in [("new", "New"), ("still", "Still present"), ("resolved", "Resolved")] {
            let list = comparison[key].as_array().map(Vec::as_slice).unwrap_or(&[]);
            let _ = writeln!(w);
            let _ = writeln!(w, "### {title} ({})", list.len());
            for c in list {
                let _ = writeln!(w, "- {} `{}`-{} {}: {}", text(&c["system"]), text(&c["code"]), number(&c["failure_type"]), text(&c["kind"]), text(&c["description"]));
            }
        }
        let _ = writeln!(w);
        let _ = writeln!(w, "### Systems not read");
        let not_read = comparison["systems_not_read"].as_array().map(Vec::as_slice).unwrap_or(&[]);
        if not_read.is_empty() {
            let _ = writeln!(w, "Every system the baseline had was read again.");
        }
        for s in not_read {
            let _ = writeln!(w, "- {}", s.as_str().unwrap_or(""));
        }
        let _ = writeln!(w);
        let _ = writeln!(w, "### Readiness changes");
        let changes = comparison["readiness_changes"].as_array().map(Vec::as_slice).unwrap_or(&[]);
        if changes.is_empty() {
            let _ = writeln!(w, "The set of self-tests not run is the same as the baseline's.");
        }
        for c in changes {
            let _ = writeln!(w, "- {}", c.as_str().unwrap_or(""));
        }
    }
    let _ = writeln!(w);

    let _ = writeln!(w, "## Service markers");
    let _ = writeln!(w);
    let markers = report["markers"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    if markers.is_empty() {
        let _ = writeln!(w, "No service markers recorded for this car.");
    }
    for m in markers {
        let _ = writeln!(w, "- {} {} ({}): {}", crate::journal::ts(&m["at"]), text(&m["kind"]), mileage(&m["mileage_km"]), text(&m["note"]));
    }
    let _ = writeln!(w);

    let _ = writeln!(w, "## Recent writes");
    let _ = writeln!(w);
    let writes = report["writes"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    if writes.is_empty() {
        let _ = writeln!(w, "No writes recorded for this car.");
    }
    for r in writes {
        let outcome = r["outcome"].as_str().or_else(|| r["event"].as_str()).unwrap_or("");
        let _ = writeln!(w, "- {} {} {}: {}", crate::journal::ts(&r["at"]), text(&r["system"]), text(&r["name"]), outcome);
    }
    let _ = writeln!(w);
    let _ = writeln!(w, "{}", report["disclaimer"].as_str().unwrap_or(DISCLAIMER));
    w
}

/// A VIN as a file name: ASCII letters and digits only; an unread VIN is `unknown`.
fn sanitize_vin(vin: &str) -> String {
    let s: String = vin.chars().filter(char::is_ascii_alphanumeric).collect();
    if s.is_empty() { "unknown".to_string() } else { s }
}

/// Write the default pair under `<root>/reports/`: `hondascan-health-<VIN>-<unix>.html` and `.json`.
pub fn save(report: &Value, root: &Path) -> Result<(PathBuf, PathBuf), ServiceError> {
    let dir = root.join("reports");
    std::fs::create_dir_all(&dir)?;
    let vin = sanitize_vin(report["vin"].as_str().unwrap_or(""));
    let unix = report["generated"].as_f64().filter(|t| t.is_finite()).unwrap_or_else(now) as i64;
    let stem = format!("hondascan-health-{vin}-{unix}");
    let html_path = dir.join(format!("{stem}.html"));
    let json_path = dir.join(format!("{stem}.json"));
    std::fs::write(&html_path, html(report))?;
    std::fs::write(&json_path, format!("{}\n", dumps(report, Json::Indent(2))))?;
    Ok((html_path, json_path))
}

/// The CLI's `--out`: `.md` writes Markdown, `.json` pretty JSON, anything else HTML. Parent directories are created.
pub fn save_one(report: &Value, out: &Path) -> Result<PathBuf, ServiceError> {
    if let Some(dir) = out.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    let ext = out.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let body = match ext.as_str() {
        "md" | "markdown" => markdown(report),
        "json" => format!("{}\n", dumps(report, Json::Indent(2))),
        _ => html(report),
    };
    std::fs::write(out, body)?;
    Ok(out.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clean_car() -> Value {
        json!({
            "schema": 1, "vin": "1HGCR2F3XFA000000", "session": 7, "adapter": "sim",
            "systems": [{"address": 0x10, "system": "CANFI", "display": "PGM-FI", "answered": true,
                "dtcs": [], "incomplete": [], "self_tests": {"at_start": 0, "now": 0, "completed": [], "incomplete": []},
                "readiness": []}],
            "totals": {"systems": 1, "faults": 0, "pending": 0, "stored": 0, "incomplete": 0},
            "healthy": true
        })
    }

    fn baseline(dtcs: Value, incomplete: Value) -> Value {
        json!({
            "id": 3, "created": 1_700_000_000.0, "label": "After service", "sha256": "ab", "integrity": "ok",
            "systems": [{"address": 0x10, "system": "CANFI", "display": "PGM-FI", "answered": true,
                "dtcs": dtcs, "incomplete": incomplete,
                "self_tests": {"at_start": 0, "now": 0, "completed": [], "incomplete": incomplete}}]
        })
    }

    #[test]
    fn escape_covers_the_five_html_characters() {
        assert_eq!(escape("a & b < c > d \"e\" 'f'"), "a &amp; b &lt; c &gt; d &quot;e&quot; &#39;f&#39;");
        assert_eq!(escape("plain VIN 1HGCR2F3XFA000000"), "plain VIN 1HGCR2F3XFA000000");
    }

    #[test]
    fn verdict_unknown_attention_and_healthy() {
        let car = clean_car();
        let none = compare(&car, None);
        assert_eq!(verdict(&car, "1HGCR2F3XFA000000", &none), "healthy");
        assert_eq!(verdict(&car, "UNKNOWN", &none), "unknown", "a car whose VIN was not read is unknown");

        let mut unread = clean_car();
        unread["systems"][0]["answered"] = json!(false);
        unread["systems"][0]["error"] = json!("no response");
        assert_eq!(verdict(&unread, "1HGCR2F3XFA000000", &none), "unknown");

        let mut confirmed = clean_car();
        confirmed["totals"]["faults"] = json!(1);
        confirmed["systems"][0]["dtcs"] = json!([{"code": "P0420", "failure_type": 0, "kind": "confirmed", "description": "cat"}]);
        assert_eq!(verdict(&confirmed, "1HGCR2F3XFA000000", &none), "attention");

        // A stored-only new code is reported but does not by itself need attention.
        let mut stored = clean_car();
        stored["systems"][0]["dtcs"] = json!([{"code": "P0420", "failure_type": 0, "kind": "stored", "description": "cat"}]);
        stored["totals"]["stored"] = json!(1);
        let base = baseline(json!([]), json!([]));
        let comparison = compare(&stored, Some(&base));
        assert_eq!(comparison["new"].as_array().unwrap().len(), 1);
        assert_eq!(verdict(&stored, "1HGCR2F3XFA000000", &comparison), "healthy");
    }

    #[test]
    fn comparison_lists_new_still_resolved_and_readiness() {
        let mut car = clean_car();
        car["systems"][0]["dtcs"] = json!([
            {"code": "P0300", "failure_type": 0, "kind": "confirmed", "description": "misfire"},
            {"code": "P0420", "failure_type": 0, "kind": "stored", "description": "cat"}
        ]);
        car["systems"][0]["incomplete"] = json!([{"code": "P1000", "failure_type": 0, "description": "not ready"}]);
        car["systems"][0]["self_tests"]["incomplete"] = json!([{"code": "P1000", "failure_type": 0, "description": "not ready"}]);
        let mut base = baseline(
            json!([{"code": "P0420", "failure_type": 0, "kind": "stored", "description": "cat"},
                   {"code": "P0455", "failure_type": 0, "kind": "stored", "description": "evap"}]),
            json!([]),
        );
        base["systems"].as_array_mut().unwrap().push(json!({
            "address": 0x53, "system": "CANSRS", "display": "SRS", "answered": true, "dtcs": [], "incomplete": [],
            "self_tests": {"incomplete": []}
        }));
        let c = compare(&car, Some(&base));
        assert_eq!(c["baseline"]["label"], "After service");
        assert_eq!(c["baseline"]["integrity"], "ok");
        let codes = |key: &str| -> Vec<String> { c[key].as_array().unwrap().iter().map(|d| d["code"].as_str().unwrap().to_string()).collect() };
        assert_eq!(codes("new"), vec!["P0300"]);
        assert_eq!(codes("still"), vec!["P0420"]);
        assert_eq!(codes("resolved"), vec!["P0455"]);
        assert_eq!(c["new"][0]["system"], "PGM-FI");
        assert_eq!(c["systems_not_read"], json!(["SRS"]));
        assert_eq!(c["readiness_changes"].as_array().unwrap().len(), 1);
        assert!(c["readiness_changes"][0].as_str().unwrap().contains("1 self-test not run (baseline 0)"), "{c}");
    }

    #[test]
    fn an_unanswered_system_resolves_nothing_and_is_named_not_read() {
        let mut car = clean_car();
        car["systems"][0]["answered"] = json!(false);
        car["systems"][0]["dtcs"] = json!([]);
        let base = baseline(json!([{"code": "P0420", "failure_type": 0, "kind": "stored", "description": "cat"}]), json!([]));
        let c = compare(&car, Some(&base));
        assert_eq!(c["resolved"], json!([]), "a read that did not answer cannot prove a code gone");
        assert_eq!(c["systems_not_read"], json!(["PGM-FI"]));
    }

    #[test]
    fn summary_names_the_counts_a_person_can_act_on() {
        let mut car = clean_car();
        car["totals"]["faults"] = json!(1);
        car["totals"]["incomplete"] = json!(2);
        car["systems"][0]["dtcs"] = json!([{"code": "P0300", "failure_type": 0, "kind": "confirmed", "description": "misfire"}]);
        let base = baseline(json!([]), json!([{"code": "P1000"}, {"code": "P1010"}]));
        let c = compare(&car, Some(&base));
        let s = summary(&car, &c, "attention");
        assert!(s.contains("1 confirmed trouble code") && s.contains("2 self-tests not run"), "{s}");
        assert!(s.contains("1 readiness change"), "{s}");

        let healthy = summary(&clean_car(), &compare(&clean_car(), Some(&baseline(json!([]), json!([])))), "healthy");
        assert!(healthy.contains("Nothing new since the baseline of 2023-11-14"), "{healthy}");

        let unknown = summary(&clean_car(), &compare(&clean_car(), None), "unknown");
        assert!(unknown.contains("identity") || unknown.contains("unknown"), "{unknown}");
    }

    #[test]
    fn html_escapes_every_piece_of_car_and_user_text() {
        let mut report = json!({
            "schema": 1, "vin": "1HGCR2F3XFA000000", "generated": 1_700_000_000.0, "adapter": "sim",
            "session": 7, "verdict": "attention", "summary": "1 confirmed code",
            "totals": {"systems": 1, "faults": 1, "pending": 0, "stored": 0, "incomplete": 0},
            "systems": [{"address": 0x10, "system": "CANFI", "display": "PGM-FI", "answered": true,
                "dtcs": [{"code": "P0420", "failure_type": 0, "kind": "confirmed", "description": "cat <script>alert(\"x\")</script> & 'quoted'"}],
                "incomplete": [], "self_tests": {"incomplete": []}, "readiness": []}],
            "comparison": {"baseline": null, "new": [], "still": [], "resolved": [], "systems_not_read": [], "readiness_changes": []},
            "markers": [{"id": 1, "at": 1_700_000_000.0, "kind": "oil", "note": "Oil & <filter> changed", "mileage_km": 100.0, "created": 1_700_000_000.0}],
            "writes": [],
            "disclaimer": DISCLAIMER,
        });
        let page = html(&report);
        assert!(!page.contains("<script>"), "raw script tag in {}", page);
        assert!(page.contains("&lt;script&gt;alert(&quot;x&quot;)&lt;/script&gt; &amp; &#39;quoted&#39;"), "{page}");
        assert!(page.contains("Oil &amp; &lt;filter&gt; changed"), "{page}");
        assert!(page.contains("P0420") && page.contains("Trouble codes") && page.contains("Comparison"));

        // A VIN is data too.
        report["vin"] = json!("<not-a-vin>");
        assert!(html(&report).contains("&lt;not-a-vin&gt;"));
    }

    #[test]
    fn markdown_has_the_same_sections() {
        let car = clean_car();
        let mut report = json!({
            "schema": 1, "vin": "1HGCR2F3XFA000000", "generated": 1_700_000_000.0, "adapter": "sim", "session": 7,
            "verdict": "healthy", "summary": "Nothing new.", "totals": car["totals"].clone(), "systems": car["systems"].clone(),
            "comparison": compare(&car, Some(&baseline(json!([]), json!([])))),
            "markers": [], "writes": [], "disclaimer": DISCLAIMER,
        });
        let md = markdown(&report);
        for part in ["# hondascan health report", "## Trouble codes", "## Comparison with the newest reference snapshot", "## Recent writes", DISCLAIMER] {
            assert!(md.contains(part), "missing {part} in {md}");
        }
        // Markdown does not escape the text: it is a person's document.
        report["markers"] = json!([{"at": 1.0, "kind": "oil", "note": "<b>note</b>", "mileage_km": null}]);
        assert!(markdown(&report).contains("<b>note</b>"));
    }

    #[test]
    fn save_writes_both_files_and_save_one_picks_the_extension() {
        let dir = std::env::temp_dir().join(format!("hs-rs-{}-report-save", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let report = json!({
            "schema": 1, "vin": "1HGCR2F3XFA000000", "generated": 1_700_000_000.0, "adapter": "sim", "session": 7,
            "verdict": "healthy", "summary": "Nothing new.", "totals": {}, "systems": [],
            "comparison": {"baseline": null, "new": [], "still": [], "resolved": [], "systems_not_read": [], "readiness_changes": []},
            "markers": [], "writes": [], "disclaimer": DISCLAIMER,
        });
        let (html_path, json_path) = save(&report, &dir).unwrap();
        assert_eq!(html_path.file_name().unwrap(), "hondascan-health-1HGCR2F3XFA000000-1700000000.html");
        assert!(html_path.starts_with(dir.join("reports")) && json_path.with_extension("json").exists());
        assert!(std::fs::read_to_string(&html_path).unwrap().contains("<!DOCTYPE html>"));
        assert_eq!(serde_json::from_str::<Value>(&std::fs::read_to_string(&json_path).unwrap()).unwrap()["vin"], "1HGCR2F3XFA000000");

        for (name, needle) in [("out.md", "# hondascan health report"), ("out.json", "\"vin\""), ("out.html", "<!DOCTYPE html>")] {
            let p = dir.join("nested").join(name);
            let written = save_one(&report, &p).unwrap();
            assert_eq!(written, p);
            assert!(std::fs::read_to_string(&p).unwrap().contains(needle), "{name}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
