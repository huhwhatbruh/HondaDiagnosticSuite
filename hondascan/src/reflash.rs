//! Offline reflash: the J2534/UDS program-download sequence `J2534Reprogramming.exe` itself uses, read from static
//! disassembly only -- never executed, never sent to a car (README.md "Reflash groundwork", 2026-11-11 notes "The
//! exe's own flash sequence over J2534" and "Transfer helpers decoded"). CLAUDE.md "Writing functions": a reflash
//! changes stored program data, so it follows the writing rule, not the actuator-test one -- the old value read and
//! stored first (not done for this PCM: no stock backup and no confirmed read route exist yet, README.md "Reflash
//! groundwork"), `confirm: true`, the owner's own VIN, HDS's conditions (none confirmed on this path: the exe's
//! string table has no ignition or battery prompt here, so hondascan checks none either -- unconfirmed, not assumed
//! safe) and a fake at least as strict as the ECU (`sim.rs`, `SimEcu::with_reflash`).
//!
//! STATUS: this module and its runner (`reflashrun.rs`) are checked against the strict fake ECU only. Nothing here
//! has been checked by execution against the vendor code -- the disassembly it is built from is "reported, not
//! re-checked line by line" (README.md) -- and **no CLI command exists for this yet** (README.md "Offline flash
//! client built, fake only"). When one is built, it must refuse every adapter but the simulator until that
//! execution check has run and the owner has asked for a car run (CLAUDE.md: "Nothing is sent to a car until the
//! function has been checked by execution against the vendor code and the owner has asked for that run").
//!
//! Sequence (the exe's own driver, 0x413ae0; VAs are in `J2534Reprogramming.exe`, image base 0x400000):
//! SecurityAccess `27 01`/`27 02` (`uds::UdsClient::security_access`, `seedkey::calc_key_pcm`) -> session `10 02`
//! (gated on a valid `27 02` on this ECU, README.md "The `10 02`/`34` question, answered the other way round") ->
//! erase: `2E F1 00 01` -> `31 01 FF 00` -> `2E F1 01 00` -> per block: `34 44 <addr be4><size be4>` ->
//! `74 20 <max block be2>` (capped at `FlashTarget::max_block`), block counter reset to 1, then
//! `36 <counter><data>` -> `76 <counter>` repeated until the block's bytes are all sent (counter wraps
//! `0xFF -> 0x00`, "with no explicit check"), then `37` -> `77` -- then, once, after every block: `31 01 FF 01`.
//! The target ECU's address/header on this route and two callees in it are not determined (README.md); this module
//! does not depend on them. hondascan does not decode a vendor `.rwd` file itself (CLAUDE.md: the decryptors and the
//! decompiler live in `hds-xref/tools`, not the shipped crates): a flash run takes `FlashBlock`s already produced
//! there, not a `.rwd` path.

use std::fmt;

pub const WRITE_DID: u8 = 0x2E;
pub const REQUEST_DOWNLOAD: u8 = 0x34;
pub const TRANSFER_DATA: u8 = 0x36;
pub const REQUEST_TRANSFER_EXIT: u8 = 0x37;
/// Routine ids the exe sends over `31 01 ...` on this path: neither is an HDS write function (`write.rs`'s
/// whitelist is for named menu procedures cited to their own HDS source); reflash has its own fixed pair, start
/// only, never polled or stopped.
pub const ERASE_ROUTINE: u16 = 0xFF00;
pub const FINISH_ROUTINE: u16 = 0xFF01;
/// `2E F1 00 01` / `2E F1 01 00`, the exe's own erase-mode markers around the erase routine.
pub const ERASE_BEGIN: [u8; 4] = [WRITE_DID, 0xF1, 0x00, 0x01];
pub const ERASE_END: [u8; 4] = [WRITE_DID, 0xF1, 0x01, 0x00];

#[derive(Debug, Clone, PartialEq)]
pub struct ReflashRefused(pub String);

impl fmt::Display for ReflashRefused {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for ReflashRefused {}

fn refuse<T>(s: String) -> Result<T, ReflashRefused> {
    Err(ReflashRefused(s))
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// One block of a flash image: a big-endian address and its bytes, the exe's own on-disk layout (README.md "The
/// ECU-file handlers, read 2026-11-11": `[u32 big-endian address][u32 big-endian size][size data bytes]`, the data
/// untransformed). Produced outside this crate (`hds-xref/tools`), not parsed from a `.rwd` here.
#[derive(Debug, Clone, PartialEq)]
pub struct FlashBlock {
    pub address: u32,
    pub data: Vec<u8>,
}

/// One whitelisted reflash target. CLAUDE.md requires a read route, a verified full backup and a check by
/// execution before a real one is added; only a placeholder bound for the car's own A550 PCM is listed here,
/// pending that, and nothing in this build sends it to a real adapter (see the module doc).
#[derive(Debug, Clone, Copy)]
pub struct FlashTarget {
    pub key: &'static str,
    pub system: &'static str,
    pub addr_min: u32,
    pub addr_max: u32, // inclusive
    pub max_block: u16,
    pub source: &'static str,
}

pub const TARGETS: [FlashTarget; 1] = [FlashTarget {
    key: "a550_pcm",
    system: "CANFI",
    addr_min: 0x0000_0000,
    addr_max: 0x001F_0003, // README.md "Reflash groundwork": the decoded `.rwd` covers 0x000000-0x1F0003 only
    // README.md: the exe's own maxNumberOfBlockLength cap is 0x1000 (4096), read back from the `74 20` reply --
    // but hondascan's own ISO-TP layer (transport/isotp.rs) caps one request's payload at 4095 bytes, and
    // TransferData adds 2 of those (SID, counter), so this client never asks for more than 4093 bytes of data
    // per chunk, whatever the ECU answers. hondascan's own limit, not a vendor fact.
    max_block: 4093,
    source: "README.md \"Reflash groundwork\" (2026-11-11 notes): J2534Reprogramming.exe's REPRO_SEQUENCE (0x42d0a0) \
             and its transfer helpers (0x4128f0/0x413980/0x413430), read statically, not executed",
}];

pub fn target(key: &str) -> Option<&'static FlashTarget> {
    TARGETS.iter().find(|t| t.key == key)
}

/// Every block lies inside `t`'s address range and is not empty.
pub fn check_bounds(t: &FlashTarget, address: u32, size: u32) -> Result<(), ReflashRefused> {
    if size == 0 {
        return refuse(format!("block 0x{address:08X} is empty"));
    }
    let Some(end) = address.checked_add(size) else {
        return refuse(format!("block 0x{address:08X}+{size} overflows a 32-bit address"));
    };
    if address < t.addr_min || (end - 1) > t.addr_max {
        return refuse(format!("block 0x{address:08X}..0x{:08X} is outside {}'s range 0x{:08X}-0x{:08X}", end - 1, t.key, t.addr_min, t.addr_max));
    }
    Ok(())
}

/// `34 44 <addr be4><size be4>`, checked against `t`'s bounds (README.md: `dataFormatIdentifier` 0, and
/// `addressAndLengthFormatIdentifier` 4/4, both the exe's own literal `0x44`).
pub fn encode_request_download(t: &FlashTarget, address: u32, size: u32) -> Result<[u8; 10], ReflashRefused> {
    check_bounds(t, address, size)?;
    let mut out = [0u8; 10];
    out[0] = REQUEST_DOWNLOAD;
    out[1] = 0x44;
    out[2..6].copy_from_slice(&address.to_be_bytes());
    out[6..10].copy_from_slice(&size.to_be_bytes());
    Ok(out)
}

/// (address, size) out of a `34 44 ...` request, or refused if it is not exactly that shape.
pub fn decode_request_download(req: &[u8]) -> Result<(u32, u32), ReflashRefused> {
    if req.len() != 10 || req[0] != REQUEST_DOWNLOAD || req[1] != 0x44 {
        return refuse(format!("not a RequestDownload(0x44) request: {}", hex(req)));
    }
    let address = u32::from_be_bytes(req[2..6].try_into().unwrap());
    let size = u32::from_be_bytes(req[6..10].try_into().unwrap());
    Ok((address, size))
}

/// The ECU's maxNumberOfBlockLength out of a `74 20 <u16>` reply (README.md: the exe reads a 2-byte value this
/// way), capped at `t.max_block` -- hondascan's own safety bound, never exceeded even if an ECU answered a larger
/// one, since nothing past that cap has been checked.
/// The shape/range checks for `2E F1 00/01 01/00`, `34`, `36` and `37` (`uds::UdsClient::check`, once it already
/// knows `sid` is one of these, a `reflash_target` is set, and the client is not on a functional id). Pure checks
/// only: whether the ECU is actually at the right step is the fake's job (`sim.rs`) and the runner's
/// (`reflashrun.rs`), the same split `uds.rs` already uses for `memory`/`io`/`routine`.
pub fn check(sid: u8, req: &[u8], target: &FlashTarget) -> Result<(), ReflashRefused> {
    match sid {
        WRITE_DID if req == ERASE_BEGIN.as_slice() || req == ERASE_END.as_slice() => Ok(()),
        WRITE_DID => refuse(format!("hondascan's reflash only sends {} / {} (the exe's erase markers), not {}", hex(&ERASE_BEGIN), hex(&ERASE_END), hex(req))),
        REQUEST_DOWNLOAD => {
            let (address, size) = decode_request_download(req)?;
            check_bounds(target, address, size)
        }
        TRANSFER_DATA if req.len() < 2 => refuse("TransferData needs at least a block counter".into()),
        REQUEST_TRANSFER_EXIT if req.len() != 1 => refuse("hondascan sends a bare 37".into()),
        _ => Ok(()),
    }
}

pub fn decode_download_reply(t: &FlashTarget, rsp: &[u8]) -> Result<u16, ReflashRefused> {
    if rsp.len() != 4 || rsp[0] != 0x74 || rsp[1] != 0x20 {
        return refuse(format!("not a RequestDownload reply `74 20 <u16>`: {}", hex(rsp)));
    }
    let max = u16::from_be_bytes([rsp[2], rsp[3]]);
    if max == 0 {
        return refuse("the ECU answered a zero block length".into());
    }
    Ok(max.min(t.max_block))
}
