//! Writing functions: the whitelist, the wire form and the per-function conditions.
//!
//! STATUS: written against HDS's decompiled code and strict fake ECUs; never sent to a real car. Authorization and
//! conditions: CLAUDE.md "Writing functions": only functions HDS itself defines, whitelisted per system and function,
//! `confirm: true` on every request plus a confirmation the person ticks, only on the owner's car (matched by VIN),
//! HDS's conditions, the old value read and stored first, every frame logged, and a fake at least as strict as the ECU.
//! A write is not an actuator test: the routine itself changes stored adaptation data.
//!
//! Entry 1: E-VTC learning, `EVTCLEARN.LearnSub` (PGMFI_INSP 20307-20350). HDS prompts an ignition off/on cycle
//! (`CM.IgnMonitor`, 20295-20296), enters session 3, sends `31 01 02 18 02` (RoutineControl start, routine 0x0218,
//! option 02), waits `DP.Wait(0x989680)` = 10 s (the same value is labelled `10s` in EAT_app 345-347), then reads
//! `PGM-FI,EVTCZPEND` and treats 1 as success. The signal is DID
//! 0x266C byte 37 bit 0x80 (`decoded/can_signal_parts.csv` row 6331). `LearnSub`'s normal caller passes 0, so HDS does
//! **not** send the stop request after a success; the stop bytes below are HDS's, sent by its `LearnSub(1)` path and by
//! hondascan when a run is aborted or the read-back does not confirm the learn.
//!
//! HDS lists this entry for the car's system: the PGM-FI Adjustment menu (`pgmfi.umf` `sub_13e1`, entry 0x13e1) adds
//! it only when `HUTILS.IsVehicleCan()` and both `find_data('PARAM_REC', ...)` lookups resolve, `PGM-FI,EVTCZPEND` and
//! `PGM-FI,HEVOPSTAT` (database lookups made after `CM.EndComms()`; `README.md`, "First write-function candidates").
//! Both signals are defined for CANFI system 35, the car's system, so the `system: "CANFI"` whitelist is
//! the equivalent gate. The car refuses `22 26 91` (HEVOPSTAT) with `7F 22 31`; the menu check does not read it, and
//! this learn reads only `22 26 6C`.
//!
//! Entry 2: CVT inclination-sensor initial learn (stage one), `CVT_ISL.InitialLearn` (EAT_SOL_TST 4479-4485) through
//! `sub_af69(0)` and `sub_abf4` (4270-4380). The CAN path checks `CVT,ATPP == 1` and `CVT,NEW == 0` (engine speed,
//! DID 0x2301 bytes 7-8), enters session 3, and for the first count sends `31 01 11 48` (`CM.RoutineControl(1,
//! 0x1148, -1)`, CM 523-543: no control bytes). It then polls `CVT,SLPLMOD` (DID 0x2340 byte 29) for up to
//! `0x7a1200` = 8 s; 2 and 18 are success, 3 and 19 are errors and
//! `sub_b3b7` reads `CVT,SLPLERR` (byte 30) for the code (4487-4528). HDS's loop has no explicit delay (it
//! reads the DID as fast as the ECU answers, 4336-4375); hondascan paces it at 100 ms -- a chosen floor, not an
//! HDS interval -- inside HDS's 8 s window. **HDS sends no RoutineControl stop in this
//! procedure**, so `stop` is None and the runner never invents one. HDS then prompts an ignition off/on, increments
//! the count and (on the normal path) verifies the result before the completion screen; hondascan implements the
//! first stage only and does not automate that cycle or the later count-17/34 `0x1149` path (labelled not reproduced
//! in `README.md`, "CVT inclination-sensor learn (CVT_ISL) — the second write"). The menu gate is `ATFT.CANCVTMenu` (ATM.umf entry 0x137d), which adds `CVT_ISL.TestMode`
//! when `find_data('PARAM_REC','CVT,SLOPE0')` resolves; `SLOPE0` is CANCVT DID 0x2340 byte 23 bit 0x10, so HDS lists
//! it for this car.
//!
//! Entry 3: meter US smart maintenance resets — item A and items 1-5 (`METERADJUST.TestMode`, Meterapp 2419-2440, ->
//! `sub_796d` -> `MaintMinder` -> `sub_7231` (2207-2315) -> `Reset` -> `sub_65f9`, 1840-1866). HDS's procedure changes
//! no session, checks no DTCs and no engine/park conditions: it reads the item's old remaining value from DID 0x7068
//! (item A: bytes 10-11, `METER,OUT_USS_ITARDI`; item n: bytes 10+2n-11+2n, `METER,OUT_USS_ITnRDI`), sends `31 01 70 2x`
//! (RoutineControl start, routine 0x7021 for item A and 0x7022-0x7026 for items 1-5, no option bytes), then polls
//! `31 03 70 2x` (`CM.RoutineControlWithStatus(3, ...)`) until the status is not 0x7F; 0 is complete and anything
//! else is a failure. **HDS sends no RoutineControl stop in this procedure**, so `stop` is None and the runner never
//! invents one. The status layout: test.dll `FUN_1001393d` case 0x134 drops the CAN header and two payload bytes
//! (`vcomms_decomp.c` 15630-15635), so HDS's `sub_65f9` checks buffer byte 2; on the wire the positive reply is
//! `71 03 70 2x <status>` and the status is byte 4 — derived and verified by `tools/meterstatus.py`, which executes
//! HDS's own `sub_65f9` with mocked replies (`README.md`, "The meter maintenance reset"). The menu gate is
//! `METERADJUST.TestMode` needing `METER,FNC_USSM == 1` or `FNC_EUSM == 1`; the car's `22 7010` byte 11 = 0x80 gives
//! FNC_USSM = 1 (CST_USS_RDCALC is unsupported, `7F 22 31`, so `sub_796d` binds MaintMinder, not MaintMinder_Days).
//! Item A is always offered; items 1-7 only when their enable bit in 0x7068 byte 7/8 is set. The car's byte 7 = 0xF8
//! enables items 1-5 (IT1EN 0x80, IT2EN 0x40, IT3EN 0x20, IT4EN 0x10, IT5EN 0x08; IT6EN 0x04, IT7EN 0x02, IT8EN 0x01
//! and byte 8's IT9EN are clear), and hondascan implements exactly those: `meter_maint_a` and `meter_maint_1`..
//! `meter_maint_5`, each item's enable bit being one of the write's conditions (`meter_item1_enabled`.., the
//! equivalent of HDS's FastReader menu build, sub_7231 2264-2286). Items 6-7 are in HDS's table but not offered on
//! this car, so they are not whitelisted. The item texts are HDS's help screens (`4R_MET_USM_ITEM_1..5`): 1 rotate
//! tires and check tire inflation/condition, 2 air cleaner element + dust/pollen filter + drive belt, 3 transmission
//! fluid, 4 spark plugs + valve clearance + timing belt/water pump, 5 engine coolant. Item 3's distance displays as
//! "----" while `OUT_USS_IT3JDGMETH` (byte 9 bit 0x02) = 0 and `FNC_MISSION` != 0 (sub_7231 2289-2299) — display
//! only, the reset is the same procedure. The VIN gate reads the PCM's F190 (`vin_system: Some("CANFI")`) because
//! the meter refuses F190 (`7F 22 31`).
//!
//! Not reproduced: HDS's ignition-off/on prompts and the comms re-open after them (`CM.IgnMonitor`/`CM.IgnCheck`);
//! hondascan keeps the link up. Whether either learn needs that cycle is unconfirmed. The meter procedure has no such
//! cycle.
//!
//! Measured on the car (not a write): `22 26 6C` answered 76 times across the two 2026-10 recordings, 54 data bytes,
//! byte 37 bit 0x80 = 0 every time (`README.md`, "Writing functions"), so the read-back path and the old-value
//! gate hold on this car. `22 23 40` answered 3 (ELM) + 5 (J2534) times with 36 data bytes, SLPLMOD = 0 and
//! SLPLERR = 0 every time; `22 23 21` returned ATPP 0 and 1, and `22 23 01` engine speeds 721-1336 rpm in the recorded
//! samples (the engine was running), so the CVT conditions gate correctly. The meter's reads are measured too:
//! `22 7010` byte 11 = 0x80 (FNC_USSM), and `22 7068` answered 54 data bytes (the database's packet size agrees) with
//! byte 7 = 0xF8 (items 1-5 enabled), bytes 10-11 = 0x003C (item A remaining distance 60, in all eight recordings),
//! 12-13 = 0x0C1E (item 1, decreasing 0x0C1E..0x0C13 across the drive), 14-15 = 0x17F9 (item 2), 16-17 = 0x0000
//! (item 3, whose display is "----" while IT3JDGMETH = 0), 18-19 = 0x73F9 (item 4) and 20-21 = 0x7FFF (item 5); the
//! meter refused `22 F1 90` with `7F 22 31`, so the PCM-VIN gate is the one the car accepts. None of these routines
//! has ever been sent to the car.
use crate::actuator::condition;
use std::collections::BTreeSet;
use std::fmt;

pub const ROUTINE_CONTROL: u8 = 0x31;

#[derive(Debug, Clone, PartialEq)]
pub struct WriteRefused(pub String);

impl fmt::Display for WriteRefused {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for WriteRefused {}

fn refuse<T>(s: String) -> Result<T, WriteRefused> {
    Err(WriteRefused(s))
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// The value read and stored before the write, in the DID HDS reads it from. `len` is 1 for a bit/byte field, 2 for a
/// big-endian word (the meter's remaining distance).
#[derive(Debug, Clone, Copy)]
pub struct OldValue {
    pub did: u16,
    pub offset: usize,
    pub len: usize,
    pub label: &'static str,
}

/// How HDS proves a write ran: one field of a DID, and the values it reads as success and as a reported error.
#[derive(Debug, Clone, Copy)]
pub struct ReadBack {
    pub did: u16,
    pub offset: usize, // byte in the DID's data
    pub mask: u8,      // bit(s) in that byte
    pub success: &'static [u8], // values (after the mask) HDS treats as success
    pub error: &'static [u8],   // values HDS treats as a reported error (empty: none)
    pub error_offset: Option<usize>, // a second field in the same DID with the error code
}

impl ReadBack {
    pub fn value(&self, data: &[u8]) -> Option<u8> {
        data.get(self.offset).map(|b| b & self.mask)
    }

    pub fn reads(&self, data: &[u8]) -> Option<bool> {
        self.value(data).map(|v| self.success.contains(&v))
    }

    pub fn is_error(&self, data: &[u8]) -> bool {
        self.value(data).is_some_and(|v| self.error.contains(&v))
    }

    pub fn error_code(&self, data: &[u8]) -> Option<u8> {
        self.error_offset.and_then(|o| data.get(o).copied())
    }
}

/// HDS's routine-result monitor (`CM.RoutineControlWithStatus(3, ...)`): the status byte in the reply. The meter's
/// `sub_65f9` keeps polling while it reads 0x7F, treats 0 as complete and treats anything else as a failure.
#[derive(Debug, Clone, Copy)]
pub struct RoutineStatus {
    pub pending: u8,
    pub success: u8,
}

/// How a write is proved to have run.
#[derive(Debug, Clone, Copy)]
pub enum Proof {
    Did(ReadBack),
    Status(RoutineStatus),
}

/// One HDS write function: the exact requests, the read-back that proves it ran, and HDS's own conditions.
#[derive(Debug, Clone, Copy)]
pub struct Write {
    pub key: &'static str,
    pub system: &'static str,
    pub name: &'static str,
    pub start: &'static [u8],        // the exact RoutineControl start request
    pub stop: Option<&'static [u8]>, // the exact RoutineControl stop request, when HDS sends one
    pub monitor: Option<&'static [u8]>, // HDS's routine-result monitor request, when the proof is a status byte
    pub old: OldValue,               // the value read and stored before anything is sent
    pub proof: Proof,
    /// Values accepted before the write, as HDS's own first check accepts them. Empty: refuse when the read-back
    /// already shows success (HDS's E-VTC path has no such check; the refusal proves the read-back can change).
    pub old_allowed: &'static [u8],
    pub wait_s: f64,   // HDS's fixed wait before a single read-back (0 when HDS polls at once)
    pub poll_ms: u64,  // pacing between reads (0: a single read after wait_s); hondascan's floor, not an HDS interval
    pub timeout_s: f64, // HDS's poll window when poll_ms is non-zero; hondascan's safety bound where HDS has none
    pub conditions: &'static [&'static str], // names in actuator::CONDITIONS, all must hold (empty: HDS checks none)
    pub session: Option<u8>,           // session HDS enters before the routine; None: HDS does not change session
    pub check_dtcs: bool,              // whether HDS checks stored DTCs before this function
    pub vin_system: Option<&'static str>, // read F190 from this system instead (meter does not answer F190)
    pub source: &'static str,          // where HDS defines it
    pub read_back: &'static str,       // what the read-back is, for the person
}

pub const WRITES: [Write; 8] = [
    Write {
        key: "evtc_learn",
        system: "CANFI",
        name: "E-VTC learning",
        start: &[ROUTINE_CONTROL, 0x01, 0x02, 0x18, 0x02],
        stop: Some(&[ROUTINE_CONTROL, 0x02, 0x02, 0x18, 0x02]),
        monitor: None,
        old: OldValue { did: 0x266C, offset: 37, len: 1, label: "PGM-FI,EVTCZPEND" },
        proof: Proof::Did(ReadBack { did: 0x266C, offset: 37, mask: 0x80, success: &[0x80], error: &[], error_offset: None }),
        old_allowed: &[],
        wait_s: 10.0,
        poll_ms: 0,
        timeout_s: 10.0,
        conditions: &["engine_off", "park"],
        session: Some(3),
        check_dtcs: true,
        vin_system: None,
        source: "EVTCLEARN.LearnSub (PGMFI_INSP 20307-20350): CM.IgnMonitor ignition-off/on prompts (20295-20296), CM.DiagSessionControl(3), CM.RoutineControl2WithStatus(1, 0x218, 1, &{2}, 1), DP.Wait(0x989680) = 10 s, then PGM-FI,EVTCZPEND == 1; the stop is only in LearnSub's l8_0 path; evtclearn_8_13.$CODE (20265-20296): CM.ChkDtc (0xa1e5) and PGM_FI_UTILS.ShiftCheck (0x589d): EngNotRun (0x56ec, PGM-FI,NE == 0) + ATNP == 1; menu: pgmfi.umf sub_13e1 (entry 0x13e1, seed-key decompile) adds E-VTC only when IsVehicleCan() and both PARAM_REC lookups PGM-FI,EVTCZPEND and PGM-FI,HEVOPSTAT resolve (database lookups after CM.EndComms); both are defined for CANFI system 35, the car's system",
        read_back: "PGM-FI,EVTCZPEND = 1, DID 0x266C byte 37 bit 0x80 (can_signal_parts.csv row 6331)",
    },
    Write {
        key: "cvt_isl_learn",
        system: "CANCVT",
        name: "CVT inclination-sensor initial learn (stage one)",
        start: &[ROUTINE_CONTROL, 0x01, 0x11, 0x48],
        stop: None,
        monitor: None,
        old: OldValue { did: 0x2340, offset: 29, len: 1, label: "CVT,SLPLMOD" },
        proof: Proof::Did(ReadBack { did: 0x2340, offset: 29, mask: 0xFF, success: &[2, 18], error: &[3, 19], error_offset: Some(30) }),
        old_allowed: &[0, 1, 2, 3],
        wait_s: 0.0,
        poll_ms: 100,
        timeout_s: 8.0,
        conditions: &["cvt_park", "cvt_engine_off"],
        session: Some(3),
        check_dtcs: true,
        vin_system: None,
        source: "CVT_ISL.InitialLearn (EAT_SOL_TST 4479-4485) -> sub_af69(0) (4382-4465) -> sub_abf4 (4270-4380): CM.DisplayAllDtcs == 0, sub_ab41 uses CVT,ATPP == 1 (4235-4268), sub_aa62 uses CVT,NEW == 0 (4198-4233), CM.DiagSessionControl(3), then CM.RoutineControl(1, 0x1148, -1) for count 1 (4308-4310; no control bytes, CM 523-543); no stop anywhere in the procedure; polls CVT,SLPLMOD up to 0x7a1200 = 8 s (4336-4375; the loop itself has no wait, hondascan paces it at 100 ms), success 2/18, error 3/19 then sub_b3b7 reads CVT,SLPLERR (4487-4528); menu: ATFT.CANCVTMenu (ATM.umf entry 0x137d) adds CVT_ISL.TestMode when find_data('PARAM_REC','CVT,SLOPE0') resolves; SLOPE0 is CANCVT DID 0x2340 byte 23 bit 0x10. hondascan implements the first stage only: HDS then prompts ignition off/on and continues the count (4411-4413), which is not reproduced",
        read_back: "CVT,SLPLMOD = 2 or 18 (success), 3 or 19 (error) at DID 0x2340 byte 29; error code CVT,SLPLERR at byte 30 (can_signal_parts.csv rows 3639-3640)",
    },
    Write {
        key: "meter_maint_a",
        system: "CAN_METER",
        name: "US smart maintenance reset (item A)",
        start: &[ROUTINE_CONTROL, 0x01, 0x70, 0x21],
        stop: None,
        monitor: Some(&[ROUTINE_CONTROL, 0x03, 0x70, 0x21]),
        old: OldValue { did: 0x7068, offset: 10, len: 2, label: "METER,OUT_USS_ITARDI (item A remaining distance)" },
        proof: Proof::Status(RoutineStatus { pending: 0x7F, success: 0 }),
        old_allowed: &[],
        wait_s: 0.0,
        poll_ms: 100,
        timeout_s: 10.0,
        conditions: &[],
        session: None,
        check_dtcs: false,
        vin_system: Some("CANFI"),
        source: "METERADJUST.TestMode (Meterapp 0x7ad1, lines 2419-2440) -> sub_796d (2382-2417) -> METERADJUST.MaintMinder (0x77b0, 2341-2358) -> sub_7231 (2207-2315; item A routine 0x7021 at line 2212) -> METERADJUST.Reset (0x6667, 1867-1887) -> sub_65f9 (0x65f9, 1840-1866): CM.RoutineControl(1, g_370, -1) then CM.RoutineControlWithStatus(3, g_370, -1, &l_2, 6) until the status is not 0x7F; 0 is complete and anything else fails. No RoutineControl stop, no session change and no DTC/engine/park check in this procedure. Status layout: test.dll FUN_1001393d case 0x134 drops the CAN header and 2 payload bytes (vcomms_decomp.c 15630-15635), so CM sees 70 21 <status> and sub_65f9 checks byte 2; on the wire the positive reply is 71 03 70 21 <status>. Executed by tools/meterstatus.py. Menu gate: METERADJUST.TestMode needs METER,FNC_USSM == 1 or FNC_EUSM == 1 (2419-2425); the car's 22 7010 byte 11 = 0x80 gives FNC_USSM = 1, FNC_EUSM = 0. CST_USS_RDCALC (0x70A9 byte 11 bit 0x80) is unsupported (7F 22 31), so sub_796d binds MaintMinder, not MaintMinder_Days. Item A is always offered; items 1-7 are offered only when their enable bit in 0x7068 byte 7/8 is set (the car's byte 7 = 0xF8 enables 1-5). This build implements item A and items 1-5 (meter_maint_1..meter_maint_5); items 6-7 are in HDS's table but their bits are clear on this car, so they are not whitelisted.",
        read_back: "routine 0x7021 status: 0x7F while running, 0 when HDS calls it complete (wire 71 03 70 21 <status>); old value METER,OUT_USS_ITARDI in DID 0x7068 bytes 10-11 (item A remaining distance; the car read 0x003C = 60 in all eight recordings)",
    },
    Write {
        key: "meter_maint_1",
        system: "CAN_METER",
        name: "US smart maintenance reset (item 1: tire rotation)",
        start: &[ROUTINE_CONTROL, 0x01, 0x70, 0x22],
        stop: None,
        monitor: Some(&[ROUTINE_CONTROL, 0x03, 0x70, 0x22]),
        old: OldValue { did: 0x7068, offset: 12, len: 2, label: "METER,OUT_USS_IT1RDI (item 1 remaining distance)" },
        proof: Proof::Status(RoutineStatus { pending: 0x7F, success: 0 }),
        old_allowed: &[],
        wait_s: 0.0,
        poll_ms: 100,
        timeout_s: 10.0,
        conditions: &["meter_item1_enabled"],
        session: None,
        check_dtcs: false,
        vin_system: Some("CANFI"),
        source: "USM_Item (Meterapp sub_7231, 2207-2315) binds item 1 to routine 0x7022 with old value METER,OUT_USS_IT1RDI (item 1's record, lines 2215-2219) and offers it only when its enable bit reads 1: the CM.FastReader loop over OUT_USS_IT1EN..IT7EN (2264-2286) sets each record's enable = value == 1, so the write's condition meter_item1_enabled is that gate. The reset is item A's procedure (meter_maint_a's source has the chain, the status layout and the FNC_USSM menu gate): METERADJUST.Reset (0x6667, 1867-1887) -> sub_65f9 (0x65f9, 1840-1866), CM.RoutineControl(1, g_370, -1) then CM.RoutineControlWithStatus(3, g_370, -1, &l_2, 6) until the status is not 0x7F; 0 is complete and anything else fails; no stop, no session change, no DTC/engine/park check. The car's DID 0x7068 byte 7 = 0xF8 enables items 1-5. Item text: HDS help screen 4R_MET_USM_ITEM_1, 'Rotate tires, and check tire inflation and condition.'",
        read_back: "routine 0x7022 status: 0x7F while running, 0 when HDS calls it complete (wire 71 03 70 22 <status>); old value METER,OUT_USS_IT1RDI in DID 0x7068 bytes 12-13 (the car read 0x0C1E = 3102 in the recording)",
    },
    Write {
        key: "meter_maint_2",
        system: "CAN_METER",
        name: "US smart maintenance reset (item 2: air cleaner, pollen filter, drive belt)",
        start: &[ROUTINE_CONTROL, 0x01, 0x70, 0x23],
        stop: None,
        monitor: Some(&[ROUTINE_CONTROL, 0x03, 0x70, 0x23]),
        old: OldValue { did: 0x7068, offset: 14, len: 2, label: "METER,OUT_USS_IT2RDI (item 2 remaining distance)" },
        proof: Proof::Status(RoutineStatus { pending: 0x7F, success: 0 }),
        old_allowed: &[],
        wait_s: 0.0,
        poll_ms: 100,
        timeout_s: 10.0,
        conditions: &["meter_item2_enabled"],
        session: None,
        check_dtcs: false,
        vin_system: Some("CANFI"),
        source: "USM_Item (Meterapp sub_7231, 2207-2315) binds item 2 to routine 0x7023 with old value METER,OUT_USS_IT2RDI (item 2's record, lines 2220-2224) and offers it only when its enable bit reads 1 (the FastReader loop, 2264-2286); the write's condition meter_item2_enabled is that gate. The reset is item A's procedure (see meter_maint_a): Reset -> sub_65f9, RoutineControl(1) then RoutineControlWithStatus(3) until the status is not 0x7F; no stop, no session change, no DTC/engine/park check. The car's 0x7068 byte 7 = 0xF8 enables items 1-5. Item text: HDS help screen 4R_MET_USM_ITEM_2, 'Replace air cleaner element. Replace the dust and pollen filter. Inspect drive belt.'",
        read_back: "routine 0x7023 status: 0x7F while running, 0 when HDS calls it complete (wire 71 03 70 23 <status>); old value METER,OUT_USS_IT2RDI in DID 0x7068 bytes 14-15 (the car read 0x17F9 = 6137 in the recording)",
    },
    Write {
        key: "meter_maint_3",
        system: "CAN_METER",
        name: "US smart maintenance reset (item 3: transmission fluid)",
        start: &[ROUTINE_CONTROL, 0x01, 0x70, 0x24],
        stop: None,
        monitor: Some(&[ROUTINE_CONTROL, 0x03, 0x70, 0x24]),
        old: OldValue { did: 0x7068, offset: 16, len: 2, label: "METER,OUT_USS_IT3RDI (item 3 remaining distance)" },
        proof: Proof::Status(RoutineStatus { pending: 0x7F, success: 0 }),
        old_allowed: &[],
        wait_s: 0.0,
        poll_ms: 100,
        timeout_s: 10.0,
        conditions: &["meter_item3_enabled"],
        session: None,
        check_dtcs: false,
        vin_system: Some("CANFI"),
        source: "USM_Item (Meterapp sub_7231, 2207-2315) binds item 3 to routine 0x7024 with old value METER,OUT_USS_IT3RDI (item 3's record, lines 2225-2229) and offers it only when its enable bit reads 1 (the FastReader loop, 2264-2286); the write's condition meter_item3_enabled is that gate. The reset is item A's procedure (see meter_maint_a): Reset -> sub_65f9, RoutineControl(1) then RoutineControlWithStatus(3) until the status is not 0x7F; no stop, no session change, no DTC/engine/park check. The car's 0x7068 byte 7 = 0xF8 enables items 1-5. Display note: while OUT_USS_IT3JDGMETH (byte 9 bit 0x02) = 0 and FNC_MISSION != 0, sub_7231 displays the item's distance as '----' (2289-2299) — display only, the reset is the same procedure; the car's byte 9 = 0x00 and its mission is the CVT, so HDS shows the dash. Item text: HDS help screen 4R_MET_USM_ITEM_3, 'Replace transmission fluid.'",
        read_back: "routine 0x7024 status: 0x7F while running, 0 when HDS calls it complete (wire 71 03 70 24 <status>); old value METER,OUT_USS_IT3RDI in DID 0x7068 bytes 16-17 (the car read 0x0000 — item 3 is not judged by distance while IT3JDGMETH = 0)",
    },
    Write {
        key: "meter_maint_4",
        system: "CAN_METER",
        name: "US smart maintenance reset (item 4: spark plugs, valve clearance, timing belt)",
        start: &[ROUTINE_CONTROL, 0x01, 0x70, 0x25],
        stop: None,
        monitor: Some(&[ROUTINE_CONTROL, 0x03, 0x70, 0x25]),
        old: OldValue { did: 0x7068, offset: 18, len: 2, label: "METER,OUT_USS_IT4RDI (item 4 remaining distance)" },
        proof: Proof::Status(RoutineStatus { pending: 0x7F, success: 0 }),
        old_allowed: &[],
        wait_s: 0.0,
        poll_ms: 100,
        timeout_s: 10.0,
        conditions: &["meter_item4_enabled"],
        session: None,
        check_dtcs: false,
        vin_system: Some("CANFI"),
        source: "USM_Item (Meterapp sub_7231, 2207-2315) binds item 4 to routine 0x7025 with old value METER,OUT_USS_IT4RDI (item 4's record, lines 2230-2234) and offers it only when its enable bit reads 1 (the FastReader loop, 2264-2286); the write's condition meter_item4_enabled is that gate. The reset is item A's procedure (see meter_maint_a): Reset -> sub_65f9, RoutineControl(1) then RoutineControlWithStatus(3) until the status is not 0x7F; no stop, no session change, no DTC/engine/park check. The car's 0x7068 byte 7 = 0xF8 enables items 1-5. Item text: HDS help screen 4R_MET_USM_ITEM_4, 'Replace spark plugs. Inspect the valve clearance (cold). Replace timing belt and inspect water pump.'",
        read_back: "routine 0x7025 status: 0x7F while running, 0 when HDS calls it complete (wire 71 03 70 25 <status>); old value METER,OUT_USS_IT4RDI in DID 0x7068 bytes 18-19 (the car read 0x73F9 = 29689 in the recording)",
    },
    Write {
        key: "meter_maint_5",
        system: "CAN_METER",
        name: "US smart maintenance reset (item 5: engine coolant)",
        start: &[ROUTINE_CONTROL, 0x01, 0x70, 0x26],
        stop: None,
        monitor: Some(&[ROUTINE_CONTROL, 0x03, 0x70, 0x26]),
        old: OldValue { did: 0x7068, offset: 20, len: 2, label: "METER,OUT_USS_IT5RDI (item 5 remaining distance)" },
        proof: Proof::Status(RoutineStatus { pending: 0x7F, success: 0 }),
        old_allowed: &[],
        wait_s: 0.0,
        poll_ms: 100,
        timeout_s: 10.0,
        conditions: &["meter_item5_enabled"],
        session: None,
        check_dtcs: false,
        vin_system: Some("CANFI"),
        source: "USM_Item (Meterapp sub_7231, 2207-2315) binds item 5 to routine 0x7026 with old value METER,OUT_USS_IT5RDI (item 5's record, lines 2235-2239) and offers it only when its enable bit reads 1 (the FastReader loop, 2264-2286); the write's condition meter_item5_enabled is that gate. The reset is item A's procedure (see meter_maint_a): Reset -> sub_65f9, RoutineControl(1) then RoutineControlWithStatus(3) until the status is not 0x7F; no stop, no session change, no DTC/engine/park check. The car's 0x7068 byte 7 = 0xF8 enables items 1-5. Item text: HDS help screen 4R_MET_USM_ITEM_5, 'Replace engine coolant.'",
        read_back: "routine 0x7026 status: 0x7F while running, 0 when HDS calls it complete (wire 71 03 70 26 <status>); old value METER,OUT_USS_IT5RDI in DID 0x7068 bytes 20-21 (the car read 0x7FFF = 32767 in the recording)",
    },
];

pub fn write(key: &str) -> Option<&'static Write> {
    WRITES.iter().find(|w| w.key == key)
}

impl Write {
    /// Whether this write's DID read-back is satisfied, or None when the proof is a routine status or the data is short.
    pub fn reads(&self, data: &[u8]) -> Option<bool> {
        match self.proof {
            Proof::Did(r) => r.reads(data),
            Proof::Status(_) => None,
        }
    }

    /// The read-back field after the mask, or None when the proof is a routine status or the data is short.
    pub fn value(&self, data: &[u8]) -> Option<u8> {
        match self.proof {
            Proof::Did(r) => r.value(data),
            Proof::Status(_) => None,
        }
    }

    /// True when the DID read-back is one of HDS's error values.
    pub fn is_error(&self, data: &[u8]) -> bool {
        match self.proof {
            Proof::Did(r) => r.is_error(data),
            Proof::Status(_) => false,
        }
    }

    /// The error code in the same DID, when HDS defines one.
    pub fn error_code(&self, data: &[u8]) -> Option<u8> {
        match self.proof {
            Proof::Did(r) => r.error_code(data),
            Proof::Status(_) => None,
        }
    }

    /// The routine status HDS reads, when this write is proved by one.
    pub fn status(&self) -> Option<RoutineStatus> {
        match self.proof {
            Proof::Status(s) => Some(s),
            Proof::Did(_) => None,
        }
    }

    /// "did" for a read-back field, "routine_status" for a monitored routine result.
    pub fn kind(&self) -> &'static str {
        match self.proof {
            Proof::Did(_) => "did",
            Proof::Status(_) => "routine_status",
        }
    }

    /// The read-back mask, or 0xFFFF for a two-byte old value and 0xFF for a routine status.
    pub fn mask(&self) -> u32 {
        match self.proof {
            Proof::Did(r) => r.mask as u32,
            Proof::Status(_) => 0xFF,
        }
    }

    /// The old value as a number: one byte as read, or a two-byte big-endian word. None when the DID is too short.
    pub fn old_value(&self, data: &[u8]) -> Option<u32> {
        let end = self.old.offset.checked_add(self.old.len)?;
        let bytes = data.get(self.old.offset..end)?;
        Some(match bytes.len() {
            1 => bytes[0] as u32,
            2 => u16::from_be_bytes([bytes[0], bytes[1]]) as u32,
            _ => return None,
        })
    }

    /// The live signals the conditions read.
    pub fn symbols(&self) -> BTreeSet<String> {
        self.conditions.iter().filter_map(|n| condition(n)).flat_map(|c| c.symbols.iter().map(|x| x.to_string())).collect()
    }
}

/// (control sub-function, routine id, option record) of a RoutineControl request.
pub fn decode(req: &[u8]) -> Result<(u8, u16, &[u8]), WriteRefused> {
    if req.len() < 4 || req[0] != ROUTINE_CONTROL {
        return refuse(format!("not a RoutineControl request: {}", hex(req)));
    }
    if req[1] & 0x80 != 0 {
        return refuse("suppress-positive-response is not used by HDS's write functions".into());
    }
    Ok((req[1], u16::from_be_bytes([req[2], req[3]]), &req[4..]))
}

/// What a client may send as 0x31: exactly the start and stop requests of the whitelisted writes, nothing else.
#[derive(Debug, Clone)]
pub struct WritePolicy {
    writes: Vec<Write>,
}

impl Default for WritePolicy {
    fn default() -> Self {
        WritePolicy { writes: WRITES.to_vec() }
    }
}

impl WritePolicy {
    /// The policy for exactly one whitelisted write: the runner arms the client with it alone.
    pub fn single(write: Write) -> WritePolicy {
        WritePolicy { writes: vec![write] }
    }

    pub fn check(&self, req: &[u8]) -> Result<(), WriteRefused> {
        let (control, rid, data) = decode(req)?;
        if self.writes.iter().any(|w| req == w.start || w.stop == Some(req) || w.monitor == Some(req)) {
            return Ok(());
        }
        let Some(w) = self.writes.iter().find(|w| w.start.get(2..4) == Some(&req[2..4])) else {
            return refuse(format!("routine 0x{rid:04X} is not an HDS-defined write in the whitelist"));
        };
        let allowed = if control & 0x7F == 0x01 {
            Some(w.start)
        } else if control & 0x7F == 0x02 {
            w.stop
        } else if control & 0x7F == 0x03 {
            w.monitor
        } else {
            None
        };
        let Some(allowed) = allowed else {
            return refuse(format!(
                "routine 0x{rid:04X} control 0x{control:02X} with {} option byte(s) is not HDS's request (HDS sends {})",
                data.len(),
                if control & 0x7F == 0x02 { "no stop for this write" } else { "no such control" }
            ));
        };
        refuse(format!("routine 0x{rid:04X} control 0x{control:02X} with {} option byte(s) is not HDS's request {}", data.len(), hex(allowed)))
    }
}
