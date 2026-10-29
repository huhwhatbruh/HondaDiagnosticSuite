//! Honda DIAG-H trouble codes from READOUT bytes. HDS's interface decodes these codes from tables the PC sends it: each code is
//! one bit of one module address ("bit mapped", vhonda_properties.htm `DTC_PERMANENT`; README, "K-line systems"). The table is
//! `tblDtcPidXref` joined with the PID table (`hds-xref/tools/diagh_dtc_map.py`), loaded into `diagh_dtc_map` by build-db.
//!
//! This module only applies that table to bytes already read: `bytes` maps a module address to the byte read there. How a DIAG-H reply
//! carries those bytes is not known (no reply has been seen), so nothing here parses a reply. A set bit under a single-bit mask means
//! the code is present; a mask with several bits (38 of 1,984 entries) is reported as uncertain, because what HDS does with it is not
//! read. Temporary and history codes, and address-mapped systems, are not handled.
use std::collections::{BTreeMap, BTreeSet};

/// One code of one system: the Honda code `main`-`sub`, and where its bit is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DtcBit {
    pub main: u8,
    pub sub: u8,
    pub symbol: String,
    pub address: u8,
    pub mask: u8,
}

impl DtcBit {
    pub fn single_bit(&self) -> bool {
        self.mask.count_ones() == 1
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Decoded {
    /// (main, sub) of the codes whose single bit is set, sorted.
    pub present: Vec<(u8, u8)>,
    /// (main, sub) of codes with a multi-bit mask where some bit is set and which are not already in `present`, sorted.
    pub uncertain: Vec<(u8, u8)>,
    /// Addresses the table uses that `bytes` has no value for: the codes behind them are unknown, not absent.
    pub unread: BTreeSet<u8>,
}

/// The module addresses a system's table needs read.
pub fn addresses(map: &[DtcBit]) -> BTreeSet<u8> {
    map.iter().map(|e| e.address).collect()
}

pub fn decode(map: &[DtcBit], bytes: &BTreeMap<u8, u8>) -> Decoded {
    let mut present = BTreeSet::new();
    let mut maybe = BTreeSet::new();
    let mut unread = BTreeSet::new();
    for e in map {
        match bytes.get(&e.address) {
            None => {
                unread.insert(e.address);
            }
            Some(b) if b & e.mask == 0 => {}
            Some(_) if e.single_bit() => {
                present.insert((e.main, e.sub));
            }
            Some(_) => {
                maybe.insert((e.main, e.sub));
            }
        }
    }
    Decoded { uncertain: maybe.difference(&present).copied().collect(), present: present.into_iter().collect(), unread }
}
