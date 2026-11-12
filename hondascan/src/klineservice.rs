//! K-line requests as JSON, for the desktop app (rpc.rs `kline_*` and `obd_*`). Read only: there is no clear here, on purpose
//! (the K-line clear bytes are unconfirmed against the vendor code and the CLI's `kwp clear` stays the only way to send them).
//! The requests are the CLI's (`kwp` and `obd` in cli.rs): kwp.rs for HDS's VKWP systems on ISO 14230, obd9141.rs for generic OBD-II.
//! What both decide (which PIDs to read, which text a code gets) is here once and cli.rs calls it; cli.rs prints lines, this returns JSON.
//! Never run on a car or a real adapter; tests/klineservice.rs drives it through the emulated ELM327.
use crate::db::Database;
use crate::hexfmt::hex_spaced_upper;
use crate::kwp::{real_ecu_id, KwpClient, KwpError, TESTER};
use crate::livedata::LiveData;
use crate::obd9141::{self, Obd};
use crate::service::{Emit, ServiceError};
use crate::transport::elm327::KProto;
use crate::transport::elm_kline::ElmKLine;
use crate::transport::Adapter;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Instant;

/// `"iso14230"` or `"iso9141"` for a K-line adapter, null for any other (the connect reply's `kline`).
pub fn kind(adapter: &dyn Adapter) -> Value {
    match adapter.as_kline().map(ElmKLine::proto) {
        Some(KProto::Iso9141) => json!("iso9141"),
        Some(_) => json!("iso14230"),
        None => Value::Null,
    }
}

/// An identification reply as text: bytes past ASCII become U+FFFD.
pub fn ascii(d: &[u8]) -> String {
    d.iter().map(|b| if *b < 0x80 { *b as char } else { '\u{fffd}' }).collect()
}

/// The text for a code out of the database's candidates (best match first). HDS gives some systems a placeholder for another system's
/// code (1,023 codes carry "Do not diagnose, proceed to troubleshoot DTCs or symptoms", `dtc` table, counted 2026-11-11): it is kept
/// when it is `system`'s own row and passed over otherwise, when another system describes the code. "" when the database has none.
fn description(hits: &[crate::db::DtcInfo], system: &str) -> String {
    let own = hits.first().filter(|h| h.system == system);
    own.or_else(|| hits.iter().find(|h| !h.description.starts_with("Do not diagnose"))).or(hits.first()).map_or(String::new(), |h| h.description.clone())
}

/// HDS's text for a code read over generic OBD-II, where the system is not known.
pub fn obd_description(db: &Database, code: &str, pending: bool) -> String {
    description(&db.lookup_dtc(code, None, None, Some(pending)), "")
}

/// HDS's text for a code a VKWP system reported; `system` is its database name, "" when not given.
pub fn kwp_description(db: &Database, r: &crate::uds::DtcRecord, system: &str) -> String {
    description(&db.lookup_dtc(&r.code, Some(r.failure_type as i64), Some(system).filter(|s| !s.is_empty()), Some(r.kind() == "pending")), system)
}

/// The mode 01 PIDs to read: the ones asked for, or every supported PID this crate can decode.
pub fn wanted_pids(o: &Obd, pids: &[u8]) -> Result<Vec<u8>, obd9141::ObdError> {
    if !pids.is_empty() {
        return Ok(pids.to_vec());
    }
    Ok(o.supported_pids()?.into_values().flatten().collect::<BTreeSet<_>>().into_iter().filter(|p| obd9141::decode(*p, &[0, 0]).is_some()).collect())
}

fn address_byte(address: i64) -> Result<u8, ServiceError> {
    u8::try_from(address).map_err(|_| ServiceError::value(format!("a K-line address is one byte, not {address}")))
}

/// The adapter for the ISO 14230 (VKWP) methods: the CLI's `kwp` check.
fn kwp_adapter(adapter: Option<&Arc<dyn Adapter>>) -> Result<&ElmKLine, ServiceError> {
    let k = adapter.ok_or_else(|| ServiceError::value("not connected to an adapter"))?.as_kline();
    match k {
        None => Err(ServiceError::value("not a K-line adapter: connect elm-kline:PORT (older body systems) or elm-obd:PORT (generic OBD-II)")),
        Some(k) if k.proto() == KProto::Iso9141 => Err(ServiceError::value("this adapter is on ISO 9141-2, which is generic OBD-II only; the VKWP systems need ISO 14230 (elm-kline:PORT)")),
        Some(k) => Ok(k),
    }
}

/// The adapter for the generic OBD-II methods (either K-line protocol): the CLI's `obd` check.
fn obd_adapter(adapter: Option<&Arc<dyn Adapter>>) -> Result<&ElmKLine, ServiceError> {
    let a = adapter.ok_or_else(|| ServiceError::value("not connected to an adapter"))?;
    a.as_kline().ok_or_else(|| ServiceError::value("not a K-line adapter: connect elm-obd:PORT (generic OBD-II) or elm-kline:PORT"))
}

/// (name, display name, K-line address) of every VKWP system in the database, by name.
fn vkwp(db: &Database) -> Vec<(String, String, u8)> {
    db.systems().into_iter().filter(|s| s.family == "VKWP").filter_map(|s| {
        let a = u8::try_from(s.props.get("ADDRESS")?.as_u64()?).ok()?;
        Some((s.name, s.display.trim().to_string(), a))
    }).collect()
}

fn system_json((name, display, address): &(String, String, u8)) -> Value {
    json!({"name": name, "display": display, "address": address})
}

/// The VKWP systems and their addresses. No bus traffic.
pub fn systems(db: &Database) -> Value {
    json!({"systems": vkwp(db).iter().map(system_json).collect::<Vec<_>>()})
}

/// One TesterPresent per distinct VKWP address (the CLI's `kwp present`): the ECU answers, or it does not. A negative response is
/// an answer. Each address is a bus initialisation of its own, so this takes as long as the silent ones' timeouts.
pub fn scan(adapter: Option<&Arc<dyn Adapter>>, db: &Database, emit: &Emit) -> Result<Value, ServiceError> {
    let kline = kwp_adapter(adapter)?;
    let t0 = Instant::now();
    let all = vkwp(db);
    let mut found = vec![];
    for address in all.iter().map(|s| s.2).collect::<BTreeSet<u8>>() {
        emit("scan", json!({"address": address}));
        let mut c = KwpClient::new(kline.kwp_link(address, TESTER));
        c.retries = 0; // a silent address costs one bus initialisation, not two; 57 of them are minutes on a 5-baud adapter
        match c.tester_present() {
            Ok(_) | Err(KwpError::Negative { .. }) => {
                let systems: Vec<Value> = all.iter().filter(|s| s.2 == address).map(system_json).collect();
                found.push(json!({"address": address, "systems": systems}));
            }
            Err(KwpError::NotAllowed(m)) => return Err(ServiceError { kind: "KwpNotAllowed".into(), message: m }),
            Err(_) => {} // silent, or the bus initialisation failed: nothing answers at this address
        }
    }
    Ok(json!({"ecus": found, "seconds": (t0.elapsed().as_secs_f64() * 1000.0).round() / 1000.0}))
}

/// The CLI's `kwp dtc`: every code of one ECU, looked up in the database under `system`.
pub fn dtcs(adapter: Option<&Arc<dyn Adapter>>, db: &Database, address: i64, system: &str) -> Result<Value, ServiceError> {
    let mut c = KwpClient::new(kwp_adapter(adapter)?.kwp_link(address_byte(address)?, TESTER));
    let system = system.trim().to_uppercase();
    let mut out = vec![];
    for r in c.read_dtcs(crate::kwp::ALL_GROUPS).map_err(crate::uds::UdsError::Kwp)? {
        out.push(json!({"code": r.code, "status": r.status, "kind": r.kind(), "flags": r.flags(), "description": kwp_description(db, &r, &system)}));
    }
    Ok(json!({"address": address, "dtcs": out}))
}

/// The CLI's `kwp live`: every local identifier HDS lists for `system`, decoded. A rejected identifier is that entry's `error`.
/// `live` is the service's own (its signal and converter tables are read from the database once, not at every call).
pub fn live(adapter: Option<&Arc<dyn Adapter>>, live: &LiveData, address: i64, system: &str) -> Result<Value, ServiceError> {
    let mut c = KwpClient::new(kwp_adapter(adapter)?.kwp_link(address_byte(address)?, TESTER));
    let system = system.trim().to_uppercase();
    if system.is_empty() {
        return Err(ServiceError::value("live needs the database name of the K-line system, e.g. METER"));
    }
    let mut lids = vec![];
    for (lid, error, readings) in live.read(&mut c, &system, None)? {
        let signals: Vec<Value> = live.signals(&system, lid).iter().zip(readings.iter()).map(|(s, r)| {
            json!({"symbol": s.symbol, "description": s.description, "value": r.display(), "number": r.value, "unit": r.unit()})
        }).collect();
        lids.push(json!({"lid": lid, "error": error, "signals": signals}));
    }
    Ok(json!({"address": address, "system": system, "lids": lids}))
}

/// The CLI's `kwp ecu-id`: ReadECUIdentification (option 0x80) and, with `destination`, HDS's body-ECU identity from LID 0x14.
pub fn ecu_id(adapter: Option<&Arc<dyn Adapter>>, db: &Database, address: i64, system: &str, destination: Option<i64>) -> Result<Value, ServiceError> {
    let mut c = KwpClient::new(kwp_adapter(adapter)?.kwp_link(address_byte(address)?, TESTER));
    let d = c.read_ecu_id(0x80).map_err(crate::uds::UdsError::Kwp)?;
    let mut out = json!({"address": address, "hex": hex_spaced_upper(&d), "text": ascii(&d), "ecu_id": null, "match": null, "error": null});
    if let Some(dest) = destination {
        match c.read_by_lid(0x14).and_then(|raw| real_ecu_id(&raw, dest)) {
            Err(e) => out["error"] = json!(format!("no ECU ID from LID 14: {e}")),
            Ok(id) => {
                let system = system.trim().to_uppercase();
                out["match"] = json!(Some(system.as_str()).filter(|s| !s.is_empty()).and_then(|s| crate::ident::match_ecu(db, s, &id)));
                out["ecu_id"] = json!(id);
            }
        }
    }
    Ok(out)
}

fn obd_error(e: obd9141::ObdError) -> ServiceError {
    ServiceError { kind: "ObdError".into(), message: e.to_string() }
}

/// The CLI's `obd scan`: per ECU the supported mode 01 PIDs and the check-engine-light state, the stored and pending codes, the VIN.
pub fn obd_scan(adapter: Option<&Arc<dyn Adapter>>, db: &Database) -> Result<Value, ServiceError> {
    let o = Obd::new(obd_adapter(adapter)?);
    let status = o.status().map_err(obd_error)?;
    let pids = o.supported_pids().map_err(obd_error)?;
    // every ECU that answered either request. One that did not answer PID 01 has `mil` and `confirmed` null: its light is not known, not off
    let ecus: Vec<Value> = pids.keys().chain(status.keys()).copied().collect::<BTreeSet<u8>>().into_iter().map(|ecu| {
        let light = status.get(&ecu);
        json!({"address": ecu, "pids": pids.get(&ecu).cloned().unwrap_or_default(), "mil": light.map(|l| l.0), "confirmed": light.map(|l| l.1)})
    }).collect();
    let mut codes = vec![];
    for (service, kind) in [(obd9141::STORED_DTC, "stored"), (obd9141::PENDING_DTC, "pending")] {
        for (ecu, list) in o.dtcs(service).map_err(obd_error)? {
            for code in list {
                codes.push(json!({"address": ecu, "kind": kind, "description": obd_description(db, &code, service == obd9141::PENDING_DTC), "code": code}));
            }
        }
    }
    Ok(json!({"ecus": ecus, "codes": codes, "vin": o.vin().map_err(obd_error)?}))
}

/// The CLI's `obd live`: mode 01 values for `pids`, or for every supported PID this crate can decode when none are given.
pub fn obd_live(adapter: Option<&Arc<dyn Adapter>>, pids: &[u8]) -> Result<Value, ServiceError> {
    let o = Obd::new(obd_adapter(adapter)?);
    let mut readings = vec![];
    for pid in wanted_pids(&o, pids).map_err(obd_error)? {
        for (ecu, d) in o.read_pid(pid).map_err(obd_error)? {
            readings.push(match obd9141::decode(pid, &d) {
                Some(r) => json!({"address": ecu, "pid": pid, "name": r.name, "value": r.value, "unit": r.unit}),
                None => json!({"address": ecu, "pid": pid, "name": null, "hex": hex_spaced_upper(&d)}),
            });
        }
    }
    Ok(json!({"readings": readings}))
}
