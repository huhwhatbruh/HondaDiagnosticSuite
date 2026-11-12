//! The K-line screen's state and the words on it (backend: hondascan/src/klineservice.rs; replies are read as JSON, so a field the
//! backend adds later does not break the app). Read only: the screen has no clear.
use crate::format::tidy;
use serde_json::{json, Value};

/// The view for a connect: `kline` is the connect reply's field, `spec` the adapter the person picked. "" (the CAN screens), "kwp"
/// (the VKWP systems, ISO 14230) or "obd" (generic OBD-II, for an `elm-obd` / `gd101-obd` adapter whichever protocol the car answered).
pub fn mode_for(spec: &str, kline: Option<&str>) -> &'static str {
    match kline {
        None => "",
        Some(_) if spec.starts_with("elm-obd") || spec.starts_with("gd101-obd") => "obd",
        Some(_) => "kwp",
    }
}

#[derive(Default)]
pub struct Kline {
    pub mode: &'static str,
    /// What is being read ("" when idle): the screen shows it and holds its buttons.
    pub busy: String,
    pub error: String,
    pub scan: Option<Value>,
    /// (address, system) of the open system, with its codes and live values.
    pub open: Option<(i64, String)>,
    pub dtcs: Option<Value>,
    pub live: Option<Value>,
    pub obd: Option<Value>,
    pub obd_live: Option<Value>,
}

fn num(v: &Value) -> i64 { v.as_i64().unwrap_or(0) }
fn text<'a>(v: &'a Value, k: &str) -> &'a str { v[k].as_str().unwrap_or("") }
fn arr(v: &Value, k: &str) -> Vec<Value> { v[k].as_array().cloned().unwrap_or_default() }

/// A reading to at most two decimals, without trailing zeros: 1726, 0.5, 14.07.
fn plain(v: f64) -> String {
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn hex_addr(a: i64) -> String { format!("0x{a:02X}") }

fn describe(d: &str) -> String { if d.is_empty() { "Not in the HDS database".into() } else { tidy(d) } }

/// A trouble-code row: [code, state, description]; tone 2 fault (confirmed), 1 warn (the rest).
fn code_row(code: &str, kind: &str, desc: &str) -> Value {
    let tone = if kind == "confirmed" || kind == "stored" { 2 } else { 1 };
    json!([{ "text": code }, { "text": kind, "tone": tone }, { "text": describe(desc) }])
}

fn found_json(scan: &Value) -> Vec<Value> {
    arr(scan, "ecus").iter().flat_map(|e| {
        let a = num(&e["address"]);
        arr(e, "systems").into_iter().map(move |s| json!({ "address": a, "system": text(&s, "name"), "label": format!("{} · {}", text(&s, "display"), hex_addr(a)) }))
    }).collect()
}

fn live_rows(live: &Value) -> Vec<Value> {
    let mut rows = vec![];
    for l in arr(live, "lids") {
        if let Some(e) = l["error"].as_str() {
            rows.push(json!([{ "text": format!("Local identifier {:02X}", num(&l["lid"])) }, { "text": e, "tone": 1 }]));
        }
        for s in arr(&l, "signals") {
            let name = if text(&s, "description").is_empty() { text(&s, "symbol") } else { text(&s, "description") };
            rows.push(json!([{ "text": name }, { "text": text(&s, "value") }]));
        }
    }
    rows
}

fn obd_json(obd: &Value, live: Option<&Value>) -> Value {
    // an ECU that did not answer mode 01 PID 01 has no light state (null): that is said, never shown as "off"
    let lights: Vec<String> = arr(obd, "ecus").iter().map(|e| match e["mil"].as_bool() {
        Some(on) => format!("ECU {}: check engine light {}, {} confirmed code{}", hex_addr(num(&e["address"])),
            if on { "on" } else { "off" }, num(&e["confirmed"]), if num(&e["confirmed"]) == 1 { "" } else { "s" }),
        None => format!("ECU {}: check engine light not read", hex_addr(num(&e["address"]))),
    }).collect();
    let codes: Vec<Value> = arr(obd, "codes").iter().map(|c| code_row(text(c, "code"), text(c, "kind"), text(c, "description"))).collect();
    let readings: Vec<Value> = live.map(|l| arr(l, "readings")).unwrap_or_default().iter().map(|r| match r["name"].as_str() {
        Some(n) => json!([{ "text": n }, { "text": format!("{} {}", plain(r["value"].as_f64().unwrap_or(0.0)), text(r, "unit")) }]),
        None => json!([{ "text": format!("Mode 01 PID {:02X}", num(&r["pid"])) }, { "text": text(r, "hex") }]),
    }).collect();
    json!({ "lights": lights, "codes": codes, "vin": obd["vin"].as_str().unwrap_or(""), "readings": readings, "read": live.is_some() })
}

impl Kline {
    pub fn view(&self) -> Value {
        let (address, system) = self.open.clone().unwrap_or((-1, String::new()));
        json!({
            "mode": self.mode, "busy": self.busy, "error": self.error,
            "scanned": self.scan.is_some(),
            "found": self.scan.as_ref().map(found_json).unwrap_or_default(),
            "open": if self.open.is_some() { json!({ "address": address, "system": system,
                "codes": self.dtcs.as_ref().map(|d| arr(d, "dtcs").iter().map(|c| code_row(text(c, "code"), text(c, "kind"), text(c, "description"))).collect::<Vec<_>>()),
                "codesRead": self.dtcs.is_some(),
                "live": self.live.as_ref().map(live_rows), "liveRead": self.live.is_some() }) } else { Value::Null },
            "obd": self.obd.as_ref().map(|o| obd_json(o, self.obd_live.as_ref())),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_adapter_decides_the_view() {
        assert_eq!(mode_for("sim", None), "");
        assert_eq!(mode_for("elm-kline:/dev/ttyUSB0", Some("iso14230")), "kwp");
        assert_eq!(mode_for("elm-obd:/dev/ttyUSB0", Some("iso14230")), "obd");
        assert_eq!(mode_for("elm-obd9141:/dev/ttyUSB0", Some("iso9141")), "obd");
    }

    #[test]
    fn answering_systems_are_listed_one_row_per_system_at_each_address() {
        let k = Kline { mode: "kwp", scan: Some(json!({ "ecus": [{ "address": 16, "systems": [{ "name": "MICU", "display": "Multiplex Integrated Control Unit" }] },
            { "address": 80, "systems": [{ "name": "METER", "display": "Gauge Assembly" }, { "name": "METER2", "display": "Other" }] }] })), ..Default::default() };
        let v = k.view();
        assert_eq!(v["found"].as_array().unwrap().len(), 3);
        assert_eq!(v["found"][0]["label"], "Multiplex Integrated Control Unit · 0x10");
        assert_eq!(v["found"][2]["system"], "METER2");
        assert!(v["open"].is_null() && v["scanned"] == true);
    }

    #[test]
    fn codes_and_live_values_of_the_open_system() {
        let k = Kline { mode: "kwp", open: Some((16, "MICU".into())),
            dtcs: Some(json!({ "dtcs": [{ "code": "P0123", "kind": "confirmed", "description": "THROTTLE POSITION SENSOR LOW" }, { "code": "B1000", "kind": "pending", "description": "" }] })),
            live: Some(json!({ "lids": [{ "lid": 1, "error": null, "signals": [{ "symbol": "VSP", "description": "Vehicle speed", "value": "42 km/h" }] },
                { "lid": 127, "error": "service 0x21: NRC 0x31 requestOutOfRange", "signals": [] }] })), ..Default::default() };
        let o = &k.view()["open"];
        assert_eq!(o["codes"][0][0]["text"], "P0123");
        assert_eq!(o["codes"][0][1]["tone"], 2);
        assert_eq!(o["codes"][0][2]["text"], "Throttle position sensor LOW");
        assert_eq!(o["codes"][1][1]["tone"], 1);
        assert_eq!(o["codes"][1][2]["text"], "Not in the HDS database");
        assert_eq!(o["live"][0][0]["text"], "Vehicle speed");
        assert_eq!(o["live"][0][1]["text"], "42 km/h");
        assert_eq!(o["live"][1][0]["text"], "Local identifier 7F");
        assert_eq!(o["codesRead"], true);
    }

    #[test]
    fn nothing_read_is_not_shown_as_no_codes() {
        let k = Kline { mode: "kwp", open: Some((16, "MICU".into())), ..Default::default() };
        let o = &k.view()["open"];
        assert!(o["codes"].is_null() && o["codesRead"] == false && o["liveRead"] == false);
    }

    #[test]
    fn the_obd_screen() {
        let k = Kline { mode: "obd",
            obd: Some(json!({ "ecus": [{ "address": 17, "mil": true, "confirmed": 1, "pids": [1, 12] }, { "address": 24, "mil": null, "confirmed": null, "pids": [13] }],
                "codes": [{ "address": 17, "kind": "stored", "code": "P0133", "description": "O2 SENSOR CIRCUIT SLOW RESPONSE" }], "vin": "JHLRD78823C000123" })),
            obd_live: Some(json!({ "readings": [{ "address": 17, "pid": 12, "name": "Engine speed", "value": 1726.0, "unit": "rpm" }, { "address": 17, "pid": 99, "name": null, "hex": "01 02" }] })),
            ..Default::default() };
        let o = &k.view()["obd"];
        assert_eq!(o["lights"][0], "ECU 0x11: check engine light on, 1 confirmed code");
        assert_eq!(o["lights"][1], "ECU 0x18: check engine light not read", "no answer to PID 01 is not a light that is off");
        assert_eq!(o["codes"][0][0]["text"], "P0133");
        assert_eq!(o["vin"], "JHLRD78823C000123");
        assert_eq!(o["readings"][0][1]["text"], "1726 rpm");
        assert_eq!(o["readings"][1][0]["text"], "Mode 01 PID 63");
        assert_eq!(o["read"], true);
    }
}
