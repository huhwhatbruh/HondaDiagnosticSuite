//! `encrypt-db [IN] [OUT]`: encrypts a file into the `.enc` form `db.rs` reads back (`dbcrypt::encrypt`, ChaCha20-Poly1305
//! with the key built into this binary). Defaults to `data/hondascan.db` -> `data/hondascan.db.enc` in this crate.
//! `package.rs` runs it and ships the `.enc` instead of the plain file. The key is the one this build was compiled
//! with (`HONDASCAN_DB_KEY`, see `build.rs`), so run it from the same build as the binaries it ships with. The decoy
//! video (`assets/fulldatabase.mp4.enc`) is not made by this tool any more; it keeps the older XOR form.
use std::{io::Read, path::PathBuf};

fn nonce() -> [u8; 12] {
    let mut n = [0u8; 12];
    std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut n)).expect("/dev/urandom");
    n
}

fn main() {
    let mut args = std::env::args().skip(1);
    let here = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let in_path: PathBuf = args.next().map(PathBuf::from).unwrap_or_else(|| here.join("data/hondascan.db"));
    let out_path: PathBuf = args.next().map(PathBuf::from).unwrap_or_else(|| in_path.with_extension("db.enc"));
    let plain = std::fs::read(&in_path).unwrap_or_else(|e| {
        eprintln!("{}: {e}", in_path.display());
        std::process::exit(1);
    });
    let enc = hondascan::dbcrypt::encrypt(&plain, nonce());
    std::fs::write(&out_path, &enc).unwrap_or_else(|e| {
        eprintln!("{}: {e}", out_path.display());
        std::process::exit(1);
    });
    println!("{} -> {} ({} bytes)", in_path.display(), out_path.display(), enc.len());
}
