//! ECU identity: the ISO 14229-1 identification DIDs (ReadDataByIdentifier, read-only), and Honda's own ways in
//! (read_ecu_info). Port of hondascan/ident.py. On the 2015 Accord every F18x DID is refused by every module (2026-10-12),
//! except the VIN F190 on the engine computer; the ECU ID (0x2600) and the meter's part number (F110) answer.
use crate::db::Database;
use crate::honda::HondaCan;
use crate::uds::{UdsClient, UdsError};
use serde_json::{json, Map, Value};

pub const IDENT_DIDS: [(u16, &str); 12] = [
    (0xF187, "part number"), (0xF188, "software number"), (0xF189, "software version"), (0xF18A, "supplier"),
    (0xF18B, "manufacture date"), (0xF18C, "serial number"), (0xF191, "hardware number"), (0xF192, "supplier hardware"),
    (0xF193, "supplier hardware version"), (0xF194, "supplier software"), (0xF195, "supplier software version"), (0xF197, "system name"),
];
pub const VIN_DID: u16 = 0xF190;

fn strip(data: &[u8]) -> &[u8] {
    let end = data.iter().rposition(|b| !matches!(b, 0x00 | 0xFF | b' ')).map_or(0, |p| p + 1);
    &data[..end]
}

/// `bytes.decode('ascii', 'replace')`
fn ascii_replace(b: &[u8]) -> String {
    b.iter().map(|x| if *x < 0x80 { *x as char } else { '\u{fffd}' }).collect()
}

pub fn text(data: &[u8]) -> String {
    let s = ascii_replace(strip(data));
    if !s.is_empty() && s.chars().all(|c| (' '..'\u{7f}').contains(&c)) {
        s
    } else {
        data.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" ")
    }
}

pub fn vin_of(data: &[u8]) -> Option<String> {
    let v: String = ascii_replace(data).chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_uppercase();
    (v.chars().count() == 17 && !v.chars().any(|c| "IOQ".contains(c))).then_some(v)
}

/// Err only for an OS error from the bus; a refusal or silence skips the DID.
pub fn read_identity(client: &mut UdsClient) -> Result<Value, std::io::Error> {
    let mut out = Map::new();
    for (did, name) in IDENT_DIDS {
        match client.read_did(did) {
            Ok(d) => {
                out.insert(name.into(), json!(text(&d)));
            }
            Err(UdsError::Io(e)) => return Err(e),
            Err(_) => {}
        }
    }
    Ok(Value::Object(out))
}

/// The first valid VIN any of the ECUs at `addresses` reports.
pub fn read_vin(hc: &mut HondaCan, addresses: &[u32]) -> Result<Option<String>, std::io::Error> {
    for a in addresses {
        match hc.client(*a).read_did(VIN_DID) {
            Ok(d) => {
                if let Some(v) = vin_of(&d) {
                    return Ok(Some(v));
                }
            }
            Err(UdsError::Io(e)) => return Err(e),
            Err(_) => {}
        }
    }
    Ok(None)
}

/// HDS's ECU ID: the system's ECUID1..n signals (one byte each) read from the car as hex, in order, or None.
pub fn hex_id(db: &Database, system: &str, client: &mut UdsClient) -> Result<Option<String>, std::io::Error> {
    let rows: Vec<(i64, i64)> = db.query(
        "SELECT did, offset FROM can_signal_parts WHERE system=? AND symbol LIKE 'ECUID%' ORDER BY symbol, part",
        [system],
        |r| Ok((r.get(0)?, r.get(1)?)),
    );
    let Some(first) = rows.first() else { return Ok(None) };
    let data = match client.read_did(first.0 as u16) {
        Ok(d) => d,
        Err(UdsError::Io(e)) => return Err(e),
        Err(_) => return Ok(None),
    };
    let s: String = rows.iter().filter(|(_, o)| (*o as usize) < data.len()).map(|(_, o)| format!("{:02X}", data[*o as usize])).collect();
    Ok((!s.is_empty()).then_some(s))
}

/// The most specific HDS ECU entry (`ecus`, '*' = any one character) whose pattern fits `ecu_id`, as its name.
pub fn match_ecu(db: &Database, system: &str, ecu_id: &str) -> Option<String> {
    let id: Vec<char> = ecu_id.chars().collect();
    let mut best: Option<(usize, String)> = None;
    for (pattern, name) in db.query("SELECT id_pattern, name FROM ecus WHERE system=?", [system], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))) {
        let p: Vec<char> = pattern.to_uppercase().chars().collect();
        if p.len() == id.len() && p.iter().zip(&id).all(|(p, c)| *p == '*' || p == c) {
            let wild = pattern.chars().filter(|c| *c == '*').count();
            if wild * 2 < pattern.chars().count() && best.as_ref().is_none_or(|b| wild < b.0) {
                best = Some((wild, name));
            }
        }
    }
    best.map(|b| b.1)
}

/// '78100T2FA710M1' -> '78100-T2F-A71 0M1': Honda's 5-3-3 layout, whatever follows kept after a space.
pub fn honda_part_number(text: &str) -> String {
    let c: Vec<char> = text.chars().collect();
    let alnum_up = |x: &char| x.is_ascii_digit() || x.is_ascii_uppercase();
    if c.len() >= 11 && c[..5].iter().all(|x| x.is_ascii_digit()) && c[5..].iter().all(alnum_up) {
        let (a, b, d, rest): (String, String, String, String) = (c[..5].iter().collect(), c[5..8].iter().collect(), c[8..11].iter().collect(), c[11..].iter().collect());
        format!("{a}-{b}-{d}{}", if rest.is_empty() { String::new() } else { format!(" {rest}") })
    } else {
        text.to_string()
    }
}

/// The text of a UNIT_PARTSNUM signal (the meter's F110), or None.
pub fn unit_part_number(db: &Database, system: &str, client: &mut UdsClient) -> Result<Option<String>, std::io::Error> {
    let rows: Vec<(i64, i64)> = db.query("SELECT did, offset FROM can_signal_parts WHERE system=? AND symbol='UNIT_PARTSNUM' ORDER BY part", [system], |r| Ok((r.get(0)?, r.get(1)?)));
    let Some(first) = rows.first() else { return Ok(None) };
    let data = match client.read_did(first.0 as u16) {
        Ok(d) => d,
        Err(UdsError::Io(e)) => return Err(e),
        Err(_) => return Ok(None),
    };
    let bytes: Vec<u8> = rows.iter().filter(|(_, o)| (*o as usize) < data.len()).map(|(_, o)| data[*o as usize]).collect();
    let s = ascii_replace(strip(&bytes));
    Ok((!s.is_empty() && s.chars().all(|c| !c.is_control())).then_some(s))
}

/// What the module says about itself, in the ways HDS reads it (the standard F18x identification DIDs are refused by this
/// car's modules): the ECU ID and the HDS ECU entry it matches, and a unit part number.
pub fn read_ecu_info(db: &Database, system: &str, client: &mut UdsClient) -> Result<Value, std::io::Error> {
    let ecu_id = hex_id(db, system, client)?;
    let part = unit_part_number(db, system, client)?;
    Ok(json!({
        "ecu_id": ecu_id,
        "ecu_model": ecu_id.as_deref().and_then(|i| match_ecu(db, system, i)),
        "part_number": part.as_deref().map(honda_part_number),
        "part_number_raw": part,
    }))
}
