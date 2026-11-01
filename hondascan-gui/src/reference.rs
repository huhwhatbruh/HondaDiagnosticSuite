//! Everything a system can report, in HDS's own terms: the data group (DID) each signal lives in, its symbol and HDS
//! description, next to the plain name (ported from hondascan-app/src/components/DataReference.tsx). The search box's
//! text is the lead's textbox state; here the query arrives as a plain `&str`.
//! QML only draws the JSON `reference_json` makes.
use crate::api::{Catalog, SignalDef};
use crate::format::convert;
use crate::names::friendly;
use serde_json::{json, Value};

/// One row of the table, in the egui's column order: the plain name, HDS's own description and help text, the symbol,
/// the data group in `0x%04X` form, and the unit as shown (converted, the egui showed the converted unit's name).
fn row(did: u32, s: &SignalDef) -> Value {
    json!({
        "name": friendly(s).name,
        "description": s.description,
        "about": s.help.clone().unwrap_or_else(|| "—".to_string()).replace('\n', " "),
        "symbol": s.symbol,
        "group": format!("0x{did:04X}"),
        "unit": convert(0.0, &s.unit).1,
    })
}

/// The screen as QML draws it. While the catalog has not arrived the egui early-returned with only the muted
/// "Loading…", so there is nothing to say but the title — the placeholder text itself is QML's drawing. The rows are in
/// catalog order (DID order, then signal order — the egui does not sort), filtered by the search.
pub fn reference_json(cat: Option<&Catalog>, system: &str, query: &str) -> Value {
    let Some(cat) = cat else { return json!({ "loading": true, "title": "Data reference" }); };
    let q = query.trim().to_lowercase();
    let rows: Vec<Value> = cat.dids.iter().flat_map(|d| d.signals.iter().map(move |s| (d.did, s)))
        .filter(|(did, s)| q.is_empty()
            || format!("{} {} {} {:04x}", friendly(s).name, s.symbol, s.description, did).to_lowercase().contains(&q))
        .map(|(did, s)| row(did, s)).collect();
    json!({
        "loading": false,
        "title": "Data reference",
        "hint": "Search name, symbol or data group",
        "counts": format!("{} signals in {} data groups of {system}", rows.len(), cat.dids.len()),
        "note": "The car sends its data in numbered groups (data identifiers, DIDs). Each group holds several signals; the symbol is Honda’s short name for one. Names in plain words are on the Live data tab.",
        "headers": ["Name", "Honda’s description", "What Honda says", "Symbol", "Group", "Unit"],
        "rows": rows,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::DidDef;

    fn did(n: u32, sigs: &[(&str, &str, &str)]) -> DidDef {
        DidDef { did: n, signals: sigs.iter().map(|&(symbol, description, unit)| SignalDef {
            symbol: symbol.into(), description: description.into(), unit: unit.into(), ..Default::default() }).collect() }
    }

    #[test]
    fn rows_are_in_did_order_and_searched_in_plain_words() {
        let cat = Catalog { dids: vec![
            did(0x2601, &[("NE", "revolutions", ""), ("VSP", "road speed", "")]),
            did(0x2602, &[("ECT", "coolant", ""), ("MAP", "manifold", "")]),
        ], ..Default::default() };
        fn symbols(j: &Value) -> Vec<&str> { j["rows"].as_array().unwrap().iter().map(|r| r["symbol"].as_str().unwrap()).collect() }
        assert_eq!(symbols(&reference_json(Some(&cat), "CANFI", "")), ["NE", "VSP", "ECT", "MAP"],
            "catalog order: DID order, then each group's signal order — the egui does not sort");
        // a friendly name finds the row even when neither the symbol nor the description has the word
        assert_eq!(symbols(&reference_json(Some(&cat), "CANFI", "engine speed")), ["NE"]);
        // the data group itself is searchable, in its lowercase four-digit form
        let j = reference_json(Some(&cat), "CANFI", "2602");
        assert_eq!(symbols(&j), ["ECT", "MAP"]);
        assert_eq!(j["rows"][0]["group"], "0x2602");
        // nothing survives a search with no match
        assert!(reference_json(Some(&cat), "CANFI", "nothing matches this")["rows"].as_array().unwrap().is_empty());
    }

    #[test]
    fn a_rows_texts_come_out_hds_verbatim_and_the_count_line_counts() {
        let cat = Catalog { dids: vec![
            DidDef { did: 0x2694, signals: vec![
                SignalDef { symbol: "ABC".into(), description: "AUTO CRUISE MAIN".into(), unit: "kPa".into(),
                    help: Some("Cruise holds\nfrom 40 to 60".into()), ..Default::default() },
                SignalDef { symbol: "XYZ".into(), description: "ENGINE".into(), unit: "V".into(), ..Default::default() },
            ] },
            did(0x0021, &[("ECT", "coolant", "degC")]),
        ], ..Default::default() };
        let j = reference_json(Some(&cat), "CANFI", "");
        let rows = j["rows"].as_array().unwrap();
        assert_eq!(rows[0]["description"], "AUTO CRUISE MAIN", "HDS's own wording, untouched");
        assert_eq!(rows[0]["name"], "Auto cruise main", "the plain name is the tidied description");
        assert_eq!(rows[0]["about"], "Cruise holds from 40 to 60", "the help's newlines become spaces");
        assert_eq!((rows[0]["group"].as_str(), rows[0]["unit"].as_str()), (Some("0x2694"), Some("psi")));
        assert_eq!(rows[1]["about"], "—", "no help, no text: the dash");
        assert_eq!(rows[1]["name"], "Engine");
        assert_eq!(rows[1]["unit"], "V", "a unit that needs no conversion passes through");
        assert_eq!(rows[2]["name"], "Coolant temperature", "the plain-words name from names.rs");
        assert_eq!(rows[2]["group"], "0x0021");
        assert_eq!(rows[2]["unit"], "°F");
        assert_eq!(j["counts"], "3 signals in 2 data groups of CANFI", "the egui's count line: rows.len() then cat.dids.len()");
        assert_eq!(j["headers"].as_array().unwrap().len(), 6);
        let h: Vec<&str> = j["headers"].as_array().unwrap().iter().map(|x| x.as_str().unwrap()).collect();
        assert_eq!(h, ["Name", "Honda’s description", "What Honda says", "Symbol", "Group", "Unit"]);
        assert_eq!(j["hint"], "Search name, symbol or data group");
        assert_eq!(j["note"], "The car sends its data in numbered groups (data identifiers, DIDs). Each group holds several signals; the symbol is Honda’s short name for one. Names in plain words are on the Live data tab.");
        // while the catalog has not arrived there is nothing of the table to draw yet
        let j = reference_json(None, "CANFI", "engine speed");
        assert_eq!(j["loading"], true);
        assert_eq!(j["title"], "Data reference");
        assert!(j.get("rows").is_none(), "the egui's early return drew nothing of the table");
    }
}
