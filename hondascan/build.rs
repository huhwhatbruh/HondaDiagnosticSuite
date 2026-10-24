//! Writes `$OUT_DIR/dbkey.rs`: the 32-byte database key as four XOR shares, so no contiguous key sits in the binary
//! (or in the source). The key comes from `HONDASCAN_DB_KEY` (64 hex digits), else from `release-key.txt` next to
//! this file (tracked in the private repo only; the public repo must not carry it), else a fixed development key, so a
//! checkout without either builds, encrypts and decrypts with itself and `cargo test` needs no secret.
//! A `.enc` made by one key does not open under another (the AEAD tag fails), so release packaging must run
//! `encrypt-db` from the same build that ships. The release key stays out of every repo.
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
    // Shares 0..2 are a deterministic mix of the key (splitmix64), not random: the build must be reproducible, and the
    // shares only exist so the key is not a literal. The strength is the AEAD, not this split.
    let mut state = key.iter().fold(0x6a09e667f3bcc908u64, |s, b| s.rotate_left(7) ^ u64::from(*b)).wrapping_mul(0x9E3779B97F4A7C15);
    let mut shares = [[0u8; 32]; 4];
    for s in shares.iter_mut().take(3) {
        for chunk in s.chunks_mut(8) {
            chunk.copy_from_slice(&splitmix64(&mut state).to_le_bytes());
        }
    }
    for i in 0..32 {
        shares[3][i] = key[i] ^ shares[0][i] ^ shares[1][i] ^ shares[2][i];
    }
    let src = format!("pub(crate) const SHARES: [[u8; 32]; 4] = {shares:?};\n");
    fs::write(PathBuf::from(env::var("OUT_DIR").unwrap()).join("dbkey.rs"), src).unwrap();
}

fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E3779B97F4A7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^ (z >> 31)
}

fn parse_hex(h: &str) -> [u8; 32] {
    assert!(h.len() == 64 && h.is_ascii(), "HONDASCAN_DB_KEY must be 64 hex digits");
    let mut k = [0u8; 32];
    for (i, b) in k.iter_mut().enumerate() {
        *b = u8::from_str_radix(&h[2 * i..2 * i + 2], 16).expect("HONDASCAN_DB_KEY must be hex");
    }
    k
}
