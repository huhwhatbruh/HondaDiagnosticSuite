//! Bi-directional (actuator) tests: the whitelist, the conditions and the UDS IOControlByIdentifier (0x2F) encoding.
//! Port of hondascan/actuator.py.
//!
//! STATUS: written against HDS's decompiled code and a strict fake ECU; never sent to a real car. Authorization and
//! conditions: CLAUDE.md "Bi-directional (actuator) tests": only tests HDS itself defines, whitelisted per system and test,
//! `confirm: true` on every request, a time limit, control handed back at the end, on any error and on disconnect, only with
//! the car stationary and the conditions HDS checks, and a fake at least as strict as the ECU. Wire form
//! (hds-xref/tools/actuators.py): `2F <id hi> <id lo> <option> [<value hi> <value lo>]`; option 3 = shortTermAdjustment with a
//! 16-bit value, option 0 = returnControlToECU. Tests that change stored data are not actuator tests and are not listed.
use crate::livedata::{Raw, Reading};
use std::collections::HashMap;
use std::fmt;

pub const IO_CONTROL: u8 = 0x2F;
pub const RETURN_CONTROL: u8 = 0x00;
pub const SHORT_TERM: u8 = 0x03;

#[derive(Debug, Clone, PartialEq)]
pub struct IoRefused(pub String);

impl fmt::Display for IoRefused {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for IoRefused {}

fn refuse<T>(s: String) -> Result<T, IoRefused> {
    Err(IoRefused(s))
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// What a condition reads from the live signals: raw equals a number, or the value lies in a range.
#[derive(Debug, Clone, Copy)]
pub enum Check {
    Eq(&'static str, u64),
    Between(&'static str, f64, f64),
}

impl Check {
    /// readings: {symbol: Reading}
    pub fn holds(&self, rs: &HashMap<String, Reading>) -> bool {
        match *self {
            Check::Eq(sym, want) => rs.get(sym).is_some_and(|r| matches!(&r.raw, Some(Raw::One(b)) if b.as_u64() == Some(want))),
            Check::Between(sym, lo, hi) => rs.get(sym).and_then(|r| r.value).is_some_and(|v| lo <= v && v <= hi),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Condition {
    pub name: &'static str,
    pub symbols: &'static [&'static str], // live signals (database symbols of the test's system) the check reads
    pub text: &'static str,                // what HDS checks, for the person
    pub check: Check,
}

// HDS's shared checks (PGM_FI_UTILS in the decompiled apps). EngNotRun: engine speed zero with the ignition on; ShiftCheck: the
// ATNP signal reads 1 (park/neutral), else HDS asks the person to put the car in park; BattVltgChk(lo, hi): battery in millivolts.
// The CVT checks are CVT_ISL's own (`sub_ab41`/`sub_aa62`, EAT_SOL_TST 4198-4268): on CAN it reads CVT,ATPP (not ATNP) and
// CVT,NEW (engine speed) instead of PGM-FI,NE.
pub const CONDITIONS: [Condition; 13] = [
    Condition { name: "engine_off", symbols: &["NE"], text: "engine not running (HDS EngNotRun: engine speed 0, ignition on)", check: Check::Between("NE", 0.0, 0.0) },
    Condition { name: "stationary", symbols: &["VSP"], text: "vehicle speed 0", check: Check::Between("VSP", 0.0, 0.0) },
    Condition { name: "park", symbols: &["ATNP"], text: "transmission in park (HDS ShiftCheck: ATNP = 1)", check: Check::Eq("ATNP", 1) },
    Condition { name: "ac_off", symbols: &["ACS"], text: "air conditioner switch off (HDS checks PGM-FI,ACS = 0)", check: Check::Eq("ACS", 0) },
    Condition { name: "battery_10_5", symbols: &["VB"], text: "battery 10.5-20 V (HDS BattVltgChk 0x2904-0x4E20 mV)", check: Check::Between("VB", 10.5, 20.0) },
    Condition { name: "battery_8", symbols: &["VB"], text: "battery 8-20 V (HDS BattVltgChk 0x1F40-0x4E20 mV)", check: Check::Between("VB", 8.0, 20.0) },
    Condition { name: "cvt_park", symbols: &["ATPP"], text: "transmission in park (HDS CVT_ISL sub_ab41: CVT,ATPP = 1)", check: Check::Eq("ATPP", 1) },
    Condition { name: "cvt_engine_off", symbols: &["NEW"], text: "engine not running (HDS CVT_ISL sub_aa62: CVT,NEW = 0)", check: Check::Between("NEW", 0.0, 0.0) },
    // HDS's USM_Item menu (sub_7231, Meterapp 2264-2286) offers items 1-7 only when their enable bit reads 1; item A
    // is always offered and needs no condition. The car's DID 0x7068 byte 7 = 0xF8 sets items 1-5 (IT6EN 0x04,
    // IT7EN 0x02, IT8EN 0x01 and byte 8's IT9EN are clear).
    Condition { name: "meter_item1_enabled", symbols: &["OUT_USS_IT1EN"], text: "US smart-maintenance item 1 offered (HDS's USM_Item menu: METER,OUT_USS_IT1EN = 1)", check: Check::Eq("OUT_USS_IT1EN", 1) },
    Condition { name: "meter_item2_enabled", symbols: &["OUT_USS_IT2EN"], text: "US smart-maintenance item 2 offered (HDS's USM_Item menu: METER,OUT_USS_IT2EN = 1)", check: Check::Eq("OUT_USS_IT2EN", 1) },
    Condition { name: "meter_item3_enabled", symbols: &["OUT_USS_IT3EN"], text: "US smart-maintenance item 3 offered (HDS's USM_Item menu: METER,OUT_USS_IT3EN = 1)", check: Check::Eq("OUT_USS_IT3EN", 1) },
    Condition { name: "meter_item4_enabled", symbols: &["OUT_USS_IT4EN"], text: "US smart-maintenance item 4 offered (HDS's USM_Item menu: METER,OUT_USS_IT4EN = 1)", check: Check::Eq("OUT_USS_IT4EN", 1) },
    Condition { name: "meter_item5_enabled", symbols: &["OUT_USS_IT5EN"], text: "US smart-maintenance item 5 offered (HDS's USM_Item menu: METER,OUT_USS_IT5EN = 1)", check: Check::Eq("OUT_USS_IT5EN", 1) },
];

pub fn condition(name: &str) -> Option<&'static Condition> {
    CONDITIONS.iter().find(|c| c.name == name)
}

#[derive(Debug, Clone, Copy)]
pub struct Test {
    pub key: &'static str,
    pub system: &'static str, // database system whose ECU is driven
    pub name: &'static str,
    pub did: u16,
    pub values: &'static [u16],        // the values HDS sends with option 3; nothing else is allowed
    pub conditions: &'static [&'static str], // names in CONDITIONS, all must hold at the start and at every refresh
    pub hold_s: f64,                   // time limit: control is handed back after this long
    pub source: &'static str,          // where HDS defines it
    pub state: &'static [&'static str], // live signals that show the effect (for the person; not a condition)
}

pub const TESTS: [Test; 3] = [
    Test {
        key: "radiator_fan", system: "CANFI", name: "Radiator fan", did: 0x013C, values: &[0, 1],
        conditions: &["engine_off", "park", "battery_10_5", "ac_off", "stationary"], hold_s: 30.0,
        source: "PGMFI_INSP rad_fan_act (sub_290 loop, sub_616 checks): CM.IOControlByIdentifier(0x13C, 3, 0|1), release (0x13C, 0, 0)",
        state: &["FANH", "FANC", "DRFC"],
    },
    Test {
        key: "ac_clutch", system: "CANFI", name: "A/C compressor clutch", did: 0x014A, values: &[0, 1],
        conditions: &["engine_off", "park", "battery_8", "stationary"], hold_s: 30.0,
        source: "PGMFI_INSP ac_comp_sw_act (sub_5925f loop, sub_59561 checks): CM.IOControlByIdentifier(0x14A, 3, 0|1), release (0x14A, 0, 0)",
        state: &["ACC"],
    },
    Test {
        key: "battery_fan", system: "CANFI", name: "12 V battery fan", did: 0x0163, values: &[0, 1],
        conditions: &["engine_off", "park", "battery_8", "stationary"], hold_s: 30.0,
        source: "PGMFI_INSP batt_fan_test (batt_fan_test.$CODE loop, BATT_FAN_TEST.TestMode checks): CM.IOControlByIdentifier(0x163, 3, 1 - BFAN), release (0x163, 0, 0)",
        state: &["BFAN", "BFANR"],
    },
];

pub fn test(key: &str) -> Option<&'static Test> {
    TESTS.iter().find(|t| t.key == key)
}

pub fn encode(did: u16, option: u8, value: Option<u32>) -> Result<Vec<u8>, IoRefused> {
    match option {
        SHORT_TERM => match value {
            Some(v) if v <= 0xFFFF => Ok(vec![IO_CONTROL, (did >> 8) as u8, (did & 0xFF) as u8, option, (v >> 8) as u8, (v & 0xFF) as u8]),
            _ => refuse("shortTermAdjustment needs a 16-bit value".into()),
        },
        RETURN_CONTROL => Ok(vec![IO_CONTROL, (did >> 8) as u8, (did & 0xFF) as u8, option]),
        _ => refuse(format!("IO control option {option} is not used by HDS for these tests")),
    }
}

/// (did, option, value or None) of a 0x2F request, or IoRefused.
pub fn decode(req: &[u8]) -> Result<(u16, u8, Option<u16>), IoRefused> {
    if req.len() < 4 || req[0] != IO_CONTROL {
        return refuse("not an IOControlByIdentifier request".into());
    }
    let (did, option) = ((req[1] as u16) << 8 | req[2] as u16, req[3]);
    if option == RETURN_CONTROL && req.len() == 4 {
        return Ok((did, option, None));
    }
    if option == SHORT_TERM && req.len() == 6 {
        return Ok((did, option, Some((req[4] as u16) << 8 | req[5] as u16)));
    }
    refuse(format!("malformed IO control request {}", hex(req)))
}

/// What a client may send as 0x2F: the whitelisted data identifiers, option 3 with a whitelisted value, option 0.
#[derive(Debug, Clone)]
pub struct IoPolicy {
    tests: Vec<Test>,
}

impl Default for IoPolicy {
    fn default() -> Self {
        IoPolicy { tests: TESTS.to_vec() }
    }
}

impl IoPolicy {
    /// The policy for exactly one whitelisted test: the run arms the client with it alone.
    pub fn single(test: Test) -> IoPolicy {
        IoPolicy { tests: vec![test] }
    }

    pub fn check(&self, req: &[u8]) -> Result<(), IoRefused> {
        let (did, option, value) = decode(req)?;
        let Some(test) = self.tests.iter().find(|t| t.did == did) else {
            return refuse(format!("0x{did:04X} is not an HDS-defined actuator test in the whitelist"));
        };
        if option == SHORT_TERM && !value.is_some_and(|v| test.values.contains(&v)) {
            let vals: Vec<String> = test.values.iter().map(u16::to_string).collect();
            let shown = value.map_or("None".to_string(), |v| v.to_string());
            return refuse(format!("{}: value {shown} is not one HDS sends ({})", test.key, vals.join(", ")));
        }
        Ok(())
    }
}
