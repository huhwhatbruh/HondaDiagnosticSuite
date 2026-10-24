//! Replay of recorded drives is not part of the public source: the recordings are not published. `load` always fails, so the
//! named recorded-car simulations (`-a sim:accord2015-gd101` and friends) are unavailable; the built-in simulated car is not.
use std::path::PathBuf;

pub fn profiles_dir() -> PathBuf {
    crate::resource("profiles")
}

pub struct CaptureEcu {
    pub addr: u32,
    pub system: String,
    pub also: Vec<String>,
    pub listens: Vec<u32>,
    pub requests: Vec<Vec<u8>>,
    pub offline: bool,
}

impl CaptureEcu {
    pub fn handle(&mut self, _req: &[u8]) -> Option<Vec<u8>> {
        None
    }
}

pub fn load(name: &str, _systems: &[(u32, &[&str])], _physical: bool) -> Result<Vec<CaptureEcu>, String> {
    Err(format!("{name}: recorded drives are not part of the public source"))
}
