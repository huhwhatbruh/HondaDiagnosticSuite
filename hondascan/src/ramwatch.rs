//! Named RAM words to read from the PCM, and the block reads that fetch them. Port of hondascan/ramwatch.py.
//!
//! STATUS: never run on a car, and since 2026-10-20 known to be impossible on it: the `0x23` lock flag 0xFFF944ED is
//! written only by the diagnostic task's ROM-table initializer (`ROM[0x45812] != 0`, and that byte is 0x01 on this
//! calibration), so no key, routine or session change ever opens the gate (README.md, "Spark inputs" bullet "The `0x23`
//! memory gate is a permanent calibration bit"). The addresses still come from the SH-2A code of the
//! 37805-5A0-A550 image (README.md "RWD files", "Spark-advance module"): the ignition function keeps GBR = 0xFFF918B0
//! and reaches its terms as 16-bit words at GBR + 2n. A different calibration may place the module elsewhere, so a profile is
//! only read from an ECU it was measured on (`check_ecu`: the model must be one of PROFILE_ECUS[profile]), unless the person
//! says `--force-profile`.
//!
//! The watch loop is bounded on purpose: one read at a time, no faster than MIN_INTERVAL and no slower than MAX_INTERVAL (the
//! extended session lapses after the ECU's S3 time, which is unknown for this PCM), for a run length the person must give
//! (`max_seconds`, no default), and the first failure ends it (nothing is retried here: UdsClient retries a timeout once).
use std::fmt;

pub const GBR: u64 = 0xFFF9_18B0;

#[derive(Debug, Clone, Copy)]
pub struct Word {
    pub name: &'static str,
    pub address: u64,
    pub size: usize,
    pub signed: bool,
    pub note: &'static str,
}

const fn w(name: &'static str, address: u64, size: usize, signed: bool, note: &'static str) -> Word {
    Word { name, address, size, signed, note }
}

const fn gbr(n: u64, name: &'static str, note: &'static str) -> Word {
    w(name, GBR + 2 * n, 2, true, note)
}

/// 37805-5A0-A550 (ECU 37805-5A0-A510 is in its list). Signed 16-bit, big endian, unless the size is 1.
pub const IGNITION: [Word; 41] = [
    w("engine_speed", 0xFFF9098E, 2, true, "x input of the base-advance lookup: engine speed in rpm"),
    w("row", 0xFFF906D2, 2, true, "row input before the speed-dependent clamp (load?)"),
    w("blend", 0xFFF90B0C, 2, true, "z input that blends 5 maps against breakpoints 0..550"),
    w("flag_mode", 0xFFF94520, 1, false, "non-zero: blended sets, zero: direct maps"),
    w("flag_ec2", 0xFFF94EC2, 1, false, "selects within a branch"),
    w("flag_f9a", 0xFFF94F9A, 1, false, "selects within a branch"),
    gbr(0xD0, "row_clamped", "gbr[0xD0] = min(limit(x), row): the lookup's y"),
    gbr(5, "base_map", "gbr[5]: the base-advance lookup result"),
    gbr(6, "term6", "gbr[6]: zero in this build"),
    gbr(8, "term8", "gbr[8]: rate-limited adjustment"),
    gbr(0x33, "term33", "gbr[0x33]"),
    gbr(4, "base", "gbr[4] = gbr[5] + gbr[6] + gbr[8] + gbr[0x33]"),
    gbr(0xA0, "corr_map", "gbr[0xA0]: blended 5x5 correction maps"),
    gbr(0xA1, "sub_a1", "subtracted from the base"),
    gbr(0xA6, "sub_a6", "subtracted from the base"),
    gbr(0xA7, "sub_a7", "subtracted from the base"),
    gbr(0xAB, "sub_ab", "subtracted from the base"),
    gbr(0x91, "sub_91", "subtracted from the base"),
    gbr(0x99, "sub_99", "subtracted from the base"),
    gbr(0x38, "advance", "gbr[0x38]: the assembled advance"),
    gbr(0x9D, "igenan", "IGENAN, DID 0x2610 bytes 50-51 (0.1 deg): the reported advance"),
    w("pbaenan", 0xFFF906DC, 2, true, "PBAENAN, DID 0x2610 bytes 48-49 (mmHg x 10): next to the row word"),
    w("vtc_abs_src", 0xFFF90B08, 2, true, "copied into the blend word (code 0xF9606); DID 0x2662 bytes 32-33, VTCABSAENAN"),
    // The three words named in README.md "Spark inputs" ("Not done") as settling what IGENAN is made
    // of, once read alongside engine_speed, pbaenan and igenan above. None of the three is in a DID under its own
    // address (but 0xFFF912B6's value is the served RELTHW, 0x2612 bytes 12-13, since 2026-10-20), and since 2026-10-20
    // a car ramwatch run is known to be impossible (README.md, "The `0x23` memory gate"), so these stay unread.
    // Added (as words) 2026-10-16.
    w("relthw_for_limit", 0xFFF912B6, 2, true, "x of the row-clamp limit table at 0x56D3C (0,1000,2000,4000 -> 1200,4200,6500,7500, logical 0x10F45A); identified 2026-10-20 as the RELTHW throttle value, a one-step history of 0xFFF912B4, which 0x10A776 copies from [0xFFF90906] -- the same value DID 0x2612 bytes 12-13 serves (README \"Spark inputs\", \"The clamp x is the throttle\")"),
    gbr(0x9E, "igenan_term", "gbr[0x9E] = clips.w(gbr[0x9F] + gbr[2] + gbr[1]) (0x11F6A0-0x11F6B4; gbr[2] = 0 at 0x11F660): feeds the reported IGENAN alongside gbr[0x38]"),
    gbr(0x9F, "second_map", "gbr[0x9F]: the second spark-map family's lookup result (x = engine_speed, y = row_clamped, or blended by `blend`; descriptors 0x46700/0x46764/0x467C8 direct, 0x4682C/0x46838/0x46844 blended)"),
    // The mode flags of gbr[0x9A] (README.md "Spark inputs", "gbr[0x9A] is mode-switched"; static read of 0x122CE8-0x122E20,
    // added 2026-10-19): the function picks one of six lookups for gbr[0x9A] by these, and none is in any DID, so the capture of
    // the 2026-10-12 drive cannot say which branch ran. Never readable on a car either (2026-10-20: README
    // "The `0x23` memory gate"), so the branch has to be inferred per sample by testing the ROM maps against IGENAN.
    gbr(0x9A, "term_9a", "gbr[0x9A]: the mode-switched lookup result (stores at 0x122D16, 0x122DA2, 0x122DD4, 0x122DDE, 0x122DFA, 0x122E0A)"),
    gbr(0x9B, "lookup_9b", "gbr[0x9B]: the lookup result in the 0x54DB4 / 0x54DC8 branches (stored at 0x122D8E, 0x122DC0), then compared with gbr[0x9C]"),
    gbr(0x9C, "limit_9c", "gbr[0x9C]: compared with gbr[0x9B] at 0x122D94 / 0x122DC6 (its role as a bound is not confirmed)"),
    gbr(0x2C, "term_2c", "gbr[0x2C]: stored to IGENAN directly in one branch (0x123110-0x123114)"),
    gbr(0x87, "flag_87", "gbr[0x87]: tested at 0x122D38; subtracted from the sum at 0x122E1A"),
    gbr(0x43, "flag_43", "gbr[0x43]: tested at 0x122D2C"),
    gbr(0x22, "flag_22", "gbr[0x22]: tested at 0x122D32"),
    gbr(0x4C, "flag_4c", "gbr[0x4C]: tested at 0x122D3E"),
    gbr(0x2F, "flag_2f", "gbr[0x2F]: tested at 0x122D5C"),
    w("flag_fa9", 0xFFF94FA9, 1, false, "tested at 0x122D44-0x122D4A"),
    w("flag_a84", 0xFFF93A84, 1, false, "OR-ed with the next byte, tested at 0x122D4E-0x122D58"),
    w("flag_a85", 0xFFF93A85, 1, false, "OR-ed with the previous byte, tested at 0x122D4E-0x122D58"),
    w("sel_dc9", 0xFFF94DC9, 1, false, "source of r6, the index into the byte table at 0xFFF8CDE4 (0x122D62-0x122D76; not fully decoded, r6 is 3 on one path)"),
    w("tab_cde7", 0xFFF8CDE7, 1, false, "byte table 0xFFF8CDE4 at index 3: tested at 0x122DAA-0x122DAE, one outcome loads the constant at 0x54A44, the other looks up 0x54DC8 (compare operand and index range not decoded)"),
    w("tab_cde8", 0xFFF8CDE8, 1, false, "byte table 0xFFF8CDE4 at index 4 (the neighbouring entry; the index actually used is not confirmed)"),
];

pub const PROFILE_NAMES: [&str; 1] = ["ignition"];

pub fn profile(name: &str) -> Option<&'static [Word]> {
    (name == "ignition").then_some(&IGNITION[..])
}

/// The ECU models (HDS's names) whose image the profile was read from.
pub fn profile_ecus(name: &str) -> &'static [&'static str] {
    if name == "ignition" { &["5A0-A510"] } else { &[] }
}

/// The database system whose ECU ID identifies the unit.
pub fn profile_system_name(name: &str) -> &'static str {
    if name == "ignition" { "CANFI" } else { "" }
}

/// Limits on the watch rate. They are chosen safety bounds, not measurements of the ECU.
pub const MIN_INTERVAL: f64 = 0.1;
pub const MAX_INTERVAL: f64 = 2.0;
/// Conservative bytes per read; the ECU's own limit is unknown.
pub const BLOCK: u64 = 32;

#[derive(Debug, Clone, PartialEq)]
pub struct ProfileRefused(pub String);

impl fmt::Display for ProfileRefused {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for ProfileRefused {}

/// The system to read the ECU ID from, whichever system row the address was reached by: `systems` are the database systems
/// the scan found at this address. Refuses (fails closed) if the profile's system is not among them.
pub fn profile_system(profile: &str, systems: &[String]) -> Result<&'static str, ProfileRefused> {
    let system = profile_system_name(profile);
    if !systems.iter().any(|s| s == system) {
        let mut found: Vec<String> = systems.iter().map(|s| if s.is_empty() { "?".to_string() } else { s.clone() }).collect();
        found.sort();
        return Err(ProfileRefused(format!(
            "the {profile:?} RAM profile identifies its ECU through {system}, which the scan did not find at this address (found: {}); --force-profile reads anyway",
            if found.is_empty() { "nothing".to_string() } else { found.join(", ") }
        )));
    }
    Ok(system)
}

/// Refuse to read `profile` from an ECU it was not measured on. `model`: ident.read_ecu_info's ecu_model, or None.
pub fn check_ecu(profile: &str, model: Option<&str>, force: bool) -> Result<(), ProfileRefused> {
    let want = profile_ecus(profile);
    if force || model.is_some_and(|m| want.contains(&m)) {
        return Ok(());
    }
    Err(ProfileRefused(format!(
        "the {profile:?} RAM profile was read from the image of ECUs {}; this one is {}. Its RAM layout has not been read (--force-profile reads anyway)",
        want.join(", "),
        model.unwrap_or("not identified")
    )))
}

/// [(start, length, [Word])]: reads that cover `words`, merging words closer than `gap` bytes while one read stays within `max_length`.
pub fn plan(words: &[Word], gap: u64, max_length: u64) -> Vec<(u64, u64, Vec<Word>)> {
    let mut sorted = words.to_vec();
    sorted.sort_by_key(|w| w.address);
    let mut out: Vec<(u64, u64, Vec<Word>)> = vec![];
    for w in sorted {
        if let Some((start, length, ws)) = out.last_mut() {
            if w.address + w.size as u64 - *start <= max_length && w.address.saturating_sub(*start + *length) <= gap && w.address + w.size as u64 >= *start {
                *length = (*length).max(w.address + w.size as u64 - *start);
                ws.push(w);
                continue;
            }
        }
        out.push((w.address, w.size as u64, vec![w]));
    }
    out
}

pub fn decode(word: &Word, data: &[u8], start: u64) -> i64 {
    let o = (word.address - start) as usize;
    let b = &data[o..o + word.size];
    let u = b.iter().fold(0u64, |v, x| v << 8 | *x as u64);
    if word.signed && b[0] & 0x80 != 0 { (u as i64) - (1i64 << (8 * word.size)) } else { u as i64 }
}

/// [(name, value)] for `words`, in the order they were read (Python's dict order); `read(address, length)` returns bytes
/// (HondaCan.read_memory bound to an ECU).
pub fn read_profile<E>(read: &mut dyn FnMut(u64, u64) -> Result<Vec<u8>, E>, words: &[Word], block: u64) -> Result<Vec<(&'static str, i64)>, E> {
    let mut values: Vec<(&'static str, i64)> = vec![];
    for (start, length, ws) in plan(words, 24, block) {
        let data = read(start, length)?;
        for w in ws {
            values.push((w.name, decode(&w, &data, start)));
        }
    }
    Ok(values)
}

/// The watch loop's options: what each read produces (`emit`), the run length (`every`, `block`, `max_seconds`) and the
/// environment (`clock`, `sleep`, and `stop` — Ctrl-C ends the watch quietly, like Python's KeyboardInterrupt).
pub struct WatchOpts<'a> {
    pub emit: &'a mut dyn FnMut(Vec<(&'static str, i64)>),
    pub every: Option<f64>,
    pub block: u64,
    pub max_seconds: Option<f64>,
    pub clock: &'a dyn Fn() -> f64,
    pub sleep: &'a dyn Fn(f64),
    pub stop: &'a dyn Fn() -> bool,
}

/// Read the profile once, or every `opts.every` seconds for at most `opts.max_seconds` (required with `every`: there is no
/// default run length); `opts.emit` gets each result. Returns the number of reads. Any failure returns at once.
pub fn watch<E: From<String>>(read: &mut dyn FnMut(u64, u64) -> Result<Vec<u8>, E>, words: &[Word], opts: WatchOpts<'_>) -> Result<u64, E> {
    if let Some(e) = opts.every {
        if !(MIN_INTERVAL..=MAX_INTERVAL).contains(&e) {
            return Err(format!("interval {e} s is outside {MIN_INTERVAL}..{MAX_INTERVAL}").into());
        }
    }
    if (opts.max_seconds.is_none() && opts.every.is_some()) || opts.max_seconds.is_some_and(|m| m <= 0.0) {
        return Err("a repeating watch needs a positive max_seconds".to_string().into());
    }
    let (t0, mut n) = ((opts.clock)(), 0u64);
    loop {
        (opts.emit)(read_profile(read, words, opts.block)?);
        n += 1;
        match opts.every {
            Some(e) if !(opts.stop)() && (opts.clock)() - t0 + e <= opts.max_seconds.unwrap_or(0.0) => (opts.sleep)(e),
            _ => return Ok(n),
        }
    }
}
