//! ReadMemoryByAddress (ISO 14229 service 0x23): the policy that decides which reads may be sent.
//! Port of hondascan/memory.py, whose docstring has the status and the authorization: only reads; the client refuses the
//! service unless it carries a MemoryPolicy, never sends it on a functional id, and the policy refuses anything outside its
//! windows or longer than max_length. Writing and programming are not implemented. SecurityAccess seed/key is
//! implemented in `uds::UdsClient::security_access` / `seedkey::calc_key`, tried automatically by `HondaCan::read_memory`
//! on NRC 0x33 -- the level-1 exchange was confirmed live 2026-10-20, but 2026-10-20 also proved the 0x33 itself is
//! permanent on a real car (the lock flag is a ROM calibration bit; README.md "Spark inputs" bullet "The `0x23` memory
//! gate"): no key can open it, so this policy will only ever matter against fakes.
use std::fmt;

pub const READ_MEMORY: u8 = 0x23;
/// Windows measured on the A550 image (README.md "RWD files"): on-chip RAM 0xFFF8xxxx-0xFFF9xxxx and the flash
/// image 0x000000-0x1FFFFF. The peripheral registers above 0xFFFA0000 are not readable.
pub const RAM: (u64, u64) = (0xFFF8_0000, 0xFFFA_0000);
pub const ROM: (u64, u64) = (0x0000_0000, 0x0020_0000);
pub const MAX_LENGTH: u64 = 64; // bytes per request: one ISO-TP single or short multi-frame reply
pub const FORMATS: [u8; 8] = [0x24, 0x44, 0x14, 0x34, 0x22, 0x12, 0x42, 0x13]; // the probe's order: 4-byte address first

#[derive(Debug, Clone, PartialEq)]
pub struct MemoryRefused(pub String);

impl fmt::Display for MemoryRefused {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for MemoryRefused {}

fn refuse<T>(s: String) -> Result<T, MemoryRefused> {
    Err(MemoryRefused(s))
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn encode(address: u64, length: u64, fmt: u8) -> Result<Vec<u8>, MemoryRefused> {
    let (alen, mlen) = ((fmt & 0x0F) as u32, (fmt >> 4) as u32);
    if !((1..=4).contains(&alen) && (1..=4).contains(&mlen)) {
        return refuse(format!("bad address-and-length format 0x{fmt:02X}"));
    }
    if address >> (8 * alen) != 0 || length >> (8 * mlen) != 0 {
        return refuse(format!("address 0x{address:X} or length {length} does not fit format 0x{fmt:02X}"));
    }
    let mut out = vec![READ_MEMORY, fmt];
    out.extend(&address.to_be_bytes()[8 - alen as usize..]);
    out.extend(&length.to_be_bytes()[8 - mlen as usize..]);
    Ok(out)
}

/// (format, address, length) of a 0x23 request, or MemoryRefused if it is malformed.
pub fn decode(req: &[u8]) -> Result<(u8, u64, u64), MemoryRefused> {
    if req.len() < 2 || req[0] != READ_MEMORY {
        return refuse("not a ReadMemoryByAddress request".into());
    }
    let fmt = req[1];
    let (alen, mlen) = ((fmt & 0x0F) as usize, (fmt >> 4) as usize);
    if !((1..=4).contains(&alen) && (1..=4).contains(&mlen)) || req.len() != 2 + alen + mlen {
        return refuse(format!("malformed request {}", hex(req)));
    }
    let be = |b: &[u8]| b.iter().fold(0u64, |v, x| v << 8 | *x as u64);
    Ok((fmt, be(&req[2..2 + alen]), be(&req[2 + alen..])))
}

#[derive(Debug, Clone, PartialEq)]
pub struct MemoryPolicy {
    windows: Vec<(u64, u64)>,
    max_length: u64,
}

impl Default for MemoryPolicy {
    fn default() -> Self {
        MemoryPolicy { windows: vec![RAM, ROM], max_length: MAX_LENGTH }
    }
}

impl MemoryPolicy {
    pub fn check(&self, req: &[u8]) -> Result<(), MemoryRefused> {
        let (_, address, length) = decode(req)?;
        if !(1..=self.max_length).contains(&length) {
            return refuse(format!("length {length} outside 1..{}", self.max_length));
        }
        if !self.windows.iter().any(|(lo, hi)| *lo <= address && address + length <= *hi) {
            let w: Vec<String> = self.windows.iter().map(|(lo, hi)| format!("0x{lo:X}-0x{hi:X}")).collect();
            return refuse(format!("0x{address:X}+{length} is outside the readable windows {}", w.join(", ")));
        }
        Ok(())
    }
}
