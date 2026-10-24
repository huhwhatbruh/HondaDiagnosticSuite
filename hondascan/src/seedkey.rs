//! SecurityAccess key calculation is not part of the public source. Both functions answer "no key", so the client never
//! unlocks a protected service; everything that needs no unlock (scan, codes, live data) is unaffected.

/// No key is available in this source tree.
pub fn calc_key(_seed: u16, _type_: u8) -> Option<u16> {
    None
}

/// No key is available in this source tree; 0 is not a valid answer to any seed, so an ECU refuses it.
pub fn calc_key_pcm(_seed: u16) -> u16 {
    0
}
