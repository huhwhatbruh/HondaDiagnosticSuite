//! Writes `$OUT_DIR/dbkey.rs`: the 32-byte database key. It comes from `HONDASCAN_DB_KEY` (64 hex digits), else from
//! `release-key.txt` next to this file (tracked, public), else a fixed development key, so a
//! checkout without either builds, encrypts and decrypts with itself and `cargo test` needs no secret.
//! A `.enc` made by one key does not open under another (the AEAD tag fails), so release packaging must run
//! `encrypt-db` from the same build that ships.
use std::{env, fs, path::PathBuf};

const DEV_KEY: [u8; 32] = *b"hondascan-dev-key--not-a-secret!";

fn main() {
    // `package.rs` passes `--cfg hondascan_packaged` via RUSTFLAGS for a release build; this just declares the cfg so
    // `-D warnings` does not flag it as unexpected. See lib.rs's `resource()` for what the cfg turns off.
    println!("cargo::rustc-check-cfg=cfg(hondascan_packaged)");
    println!("cargo:rerun-if-env-changed=HONDASCAN_DB_KEY");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=release-key.txt");
    let file = fs::read_to_string(PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("release-key.txt")).ok();
    let key = match (env::var("HONDASCAN_DB_KEY"), file) {
        (Ok(h), _) if !h.trim().is_empty() => parse_hex(h.trim()),
        (_, Some(h)) if !h.trim().is_empty() => parse_hex(h.trim()),
        _ => DEV_KEY,
    };
    let src = format!("pub(crate) const KEY: [u8; 32] = {key:?};\n");
    fs::write(PathBuf::from(env::var("OUT_DIR").unwrap()).join("dbkey.rs"), src).unwrap();
}

fn parse_hex(h: &str) -> [u8; 32] {
    assert!(h.len() == 64 && h.is_ascii(), "HONDASCAN_DB_KEY must be 64 hex digits");
    let mut k = [0u8; 32];
    for (i, b) in k.iter_mut().enumerate() {
        *b = u8::from_str_radix(&h[2 * i..2 * i + 2], 16).expect("HONDASCAN_DB_KEY must be hex");
    }
    k
}
