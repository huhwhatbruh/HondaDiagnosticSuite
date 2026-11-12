//! Byte strings as hex, the three ways the logs, replies and CLI print them.

/// `0aff`: lower case, no separator.
pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// `0a ff`: lower case, one space between bytes.
pub fn hex_spaced(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect::<Vec<_>>().join(" ")
}

/// `0A FF`: upper case, one space between bytes.
pub fn hex_spaced_upper(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02X}")).collect::<Vec<_>>().join(" ")
}
