//! The decoding models, as a JSON-lines service for hds-xref's vendor-differential tools (convverify, rulesverify, kwpverify),
//! which run HDS's own code under emulation and compare it with these models. One request per line, one answer per line:
//!   {"op":"convert","equation":0,"coefficients":[..],"points":[[x,y],..],"xs":[..]}      -> {"value": number|null}
//!   {"op":"rule","text":"...","comms_ok":true,"answers":[dids],"word":4}                  -> {"match":bool,"dids":[..]}
//!   {"op":"ecu_id","raw":"hex","dest":53}                                                 -> {"id":"..."} | {"error":"..."}
//!   {"op":"dtc","hex":"58 01 hh ll ss"}                                                   -> {"code":"P0123"} | {"error":"..."}
use hondascan::honda::Rule;
use hondascan::kwp::{dtc_records, real_ecu_id};
use hondascan::livedata::Converter;
use serde_json::{json, Value};
use std::io::{BufRead, Write};

fn unhex(s: &str) -> Vec<u8> {
    let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    (0..s.len() / 2).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap_or(0)).collect()
}

fn answer(req: &Value) -> Value {
    match req["op"].as_str().unwrap_or("") {
        "convert" => {
            let c = Converter {
                id: 0,
                equation: req["equation"].as_i64().unwrap_or(0),
                name: String::new(),
                unit: String::new(),
                coefficients: req["coefficients"].as_array().map(|a| a.iter().filter_map(Value::as_f64).collect()).unwrap_or_default(),
                points: req["points"].as_array().map(|a| a.iter().map(|p| (p[0].as_f64().unwrap_or(0.0), p[1].as_f64().unwrap_or(0.0))).collect()).unwrap_or_default(),
                states: vec![],
            };
            let xs: Vec<f64> = req["xs"].as_array().map(|a| a.iter().filter_map(Value::as_f64).collect()).unwrap_or_default();
            json!({"value": c.convert(&xs)})
        }
        "rule" => {
            let r = Rule::new(req["text"].as_str().unwrap_or(""));
            let answers: Vec<u16> = req["answers"].as_array().map(|a| a.iter().filter_map(Value::as_u64).map(|x| x as u16).collect()).unwrap_or_default();
            let word = req["word"].as_u64().unwrap_or(0) as u32;
            // a flagged probe returns the low byte alone, an unflagged one the whole big-endian word (as rulesverify models the car)
            let val: Vec<u8> = if r.flagged() { vec![(word & 0xFF) as u8] } else { word.to_be_bytes().to_vec() };
            let m = r.matches(req["comms_ok"].as_bool().unwrap_or(true), &mut |d| answers.contains(&d).then(|| val.clone()));
            json!({"match": m, "dids": r.dids()})
        }
        "ecu_id" => match real_ecu_id(&unhex(req["raw"].as_str().unwrap_or("")), req["dest"].as_i64().unwrap_or(0)) {
            Ok(id) => json!({"id": id}),
            Err(e) => json!({"error": e.to_string()}),
        },
        "dtc" => match dtc_records(&unhex(req["hex"].as_str().unwrap_or(""))) {
            Ok(r) => json!({"code": r.first().map(|x| x.code.clone())}),
            Err(e) => json!({"error": e.to_string()}),
        },
        other => json!({"error": format!("unknown op {other:?}")}),
    }
}

fn main() {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for line in std::io::stdin().lock().lines().map_while(Result::ok) {
        if line.trim().is_empty() {
            continue;
        }
        let a = match serde_json::from_str::<Value>(&line) {
            Ok(r) => answer(&r),
            Err(e) => json!({"error": e.to_string()}),
        };
        let _ = writeln!(out, "{a}");
        let _ = out.flush();
    }
}
