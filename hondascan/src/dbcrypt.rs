//! Keeps the packaged database from being a one-command `sqlite3 hondascan.db` read. The shipped file is
//! `hondascan.db.enc`: `MAGIC | nonce(12) | ChaCha20-Poly1305(plaintext)`, the magic bound as associated data, so a
//! flipped bit or a swapped file fails the tag instead of yielding garbage. The key is built in at compile time
//! (`build.rs`: `HONDASCAN_DB_KEY`, else a public development key) and held as four XOR shares, so the file is safe
//! from anyone without the binary, but a binary that can decrypt can be made to decrypt: whoever runs it, as root or
//! with a kernel-level memory reader, can still dump the memory it decrypts into — no software control on a general
//! purpose OS stops that. `db.rs` loads the plaintext straight into SQLite memory (no file, no temp path),
//! `harden_process` shuts the same-user `ptrace`/`/proc/pid/mem`/core-dump doors first, and `lock_plain` keeps the
//! short-lived plaintext buffer out of swap while it exists. This raises the cost, it is not a boundary.
//! `xor_inplace` below is the older keystream, kept for the older XOR form.
use chacha20poly1305::{aead::{Aead, KeyInit, Payload}, ChaCha20Poly1305, Key, Nonce};

include!(concat!(env!("OUT_DIR"), "/dbkey.rs"));

const MAGIC: &[u8; 4] = b"HDB2";
const NONCE_LEN: usize = 12;

/// The key, assembled at the moment of use from its shares (`black_box` keeps the compiler from folding them back
/// into one constant). Wipe it with `wipe` when done.
fn key() -> [u8; 32] {
    let s = std::hint::black_box(SHARES);
    let mut k = [0u8; 32];
    for i in 0..32 {
        k[i] = s[0][i] ^ s[1][i] ^ s[2][i] ^ s[3][i];
    }
    k
}

/// Overwrite `buf` with zeros in a way the optimiser cannot drop.
pub fn wipe(buf: &mut [u8]) {
    for b in buf.iter_mut() {
        unsafe { std::ptr::write_volatile(b, 0) };
    }
    std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
}

/// Encrypt a database file's bytes into the `.enc` form. `nonce` must be unique per call under one key.
pub fn encrypt(plain: &[u8], nonce: [u8; NONCE_LEN]) -> Vec<u8> {
    let mut k = key();
    let ct = ChaCha20Poly1305::new(&Key::from(k))
        .encrypt(&Nonce::from(nonce), Payload { msg: plain, aad: MAGIC })
        .expect("chacha20poly1305 encrypt");
    wipe(&mut k);
    let mut out = Vec::with_capacity(MAGIC.len() + NONCE_LEN + ct.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    out
}

/// Decrypt an `.enc` file's bytes. Errors do not say which check failed.
pub fn decrypt(enc: &[u8]) -> Result<Vec<u8>, &'static str> {
    const BAD: &str = "not a hondascan database for this build";
    if enc.len() < MAGIC.len() + NONCE_LEN + 16 || &enc[..MAGIC.len()] != MAGIC {
        return Err(BAD);
    }
    let (nonce, ct) = enc[MAGIC.len()..].split_at(NONCE_LEN);
    let mut k = key();
    let nonce = Nonce::try_from(nonce).map_err(|_| BAD)?;
    let out = ChaCha20Poly1305::new(&Key::from(k)).decrypt(&nonce, Payload { msg: ct, aad: MAGIC });
    wipe(&mut k);
    out.map_err(|_| BAD)
}

/// Close the cheap doors before plaintext exists in memory: on Linux, `PR_SET_DUMPABLE=0` stops another process of
/// the same user (gdb, `/proc/<pid>/mem`, `process_vm_readv`) from attaching to or reading this one, and stops core
/// dumps. Root can still read it. A no-op elsewhere.
pub fn harden_process() {
    #[cfg(target_os = "linux")]
    unsafe {
        libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0);
    }
}

/// Pin `buf` so the plaintext it holds never gets written to swap (`mlock`), and exclude it from a core dump even if
/// one happens anyway (`madvise(MADV_DONTDUMP)`; belt-and-suspenders next to `PR_SET_DUMPABLE=0`, which already skips
/// core dumps). Without this, the decrypted bytes could still be paged out and sit readable in the swap file long
/// after the process exits — `PR_SET_DUMPABLE` only stops another process reading *this* process's live memory.
/// Call before the plaintext is written, `unlock_plain` after `wipe` has zeroed it. A no-op elsewhere; root, and
/// anything that can read physical RAM directly, is still outside what either call can stop.
pub fn lock_plain(buf: &mut [u8]) {
    #[cfg(target_os = "linux")]
    unsafe {
        if !buf.is_empty() {
            libc::mlock(buf.as_ptr() as *const libc::c_void, buf.len());
            libc::madvise(buf.as_mut_ptr() as *mut libc::c_void, buf.len(), libc::MADV_DONTDUMP);
        }
    }
}

/// Undo `lock_plain` once the buffer has been wiped, so the mlock'd page count does not grow unbounded over the
/// process's life.
pub fn unlock_plain(buf: &mut [u8]) {
    #[cfg(target_os = "linux")]
    unsafe {
        if !buf.is_empty() {
            libc::munlock(buf.as_ptr() as *const libc::c_void, buf.len());
        }
    }
}

/// Fixed keystream seed for the older XOR form (not a literal repeating key). Changing it invalidates files in that form.
const SEED: u64 = 0x4f_68_31_30_31_5f_47_44;

/// splitmix64: fast, deterministic bit mixing for the older keystream; nothing here resists an attack.
fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E3779B97F4A7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^ (z >> 31)
}

/// XOR `data` with the keystream in place. Symmetric: the same call encrypts and decrypts.
pub fn xor_inplace(data: &mut [u8]) {
    let mut state = SEED;
    let mut chunk = [0u8; 8];
    for block in data.chunks_mut(8) {
        let word = splitmix64(&mut state).to_le_bytes();
        chunk[..block.len()].copy_from_slice(&word[..block.len()]);
        for (b, k) in block.iter_mut().zip(chunk.iter()) {
            *b ^= k;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aead_round_trips_and_rejects_tampering() {
        let plain = b"SQLite format 3\0 and some rows".to_vec();
        let enc = encrypt(&plain, [7; NONCE_LEN]);
        assert_eq!(&enc[..4], MAGIC);
        assert!(!enc.windows(plain.len()).any(|w| w == plain.as_slice()));
        assert_eq!(decrypt(&enc).unwrap(), plain);
        for i in [0, 5, 20, enc.len() - 1] {
            let mut bad = enc.clone();
            bad[i] ^= 1;
            assert!(decrypt(&bad).is_err(), "flip at {i}");
        }
        assert!(decrypt(&enc[..10]).is_err());
        assert!(decrypt(b"").is_err());
    }

    #[test]
    fn xor_round_trips() {
        let original = b"SELECT * FROM dtc WHERE system='ABS'".to_vec();
        let mut data = original.clone();
        xor_inplace(&mut data);
        assert_ne!(data, original, "encrypted form should not equal the plaintext");
        xor_inplace(&mut data);
        assert_eq!(data, original, "xor is its own inverse");
    }

    #[test]
    fn xor_handles_short_and_odd_lengths() {
        for len in [0, 1, 7, 8, 9, 21, 4096] {
            let original: Vec<u8> = (0..len).map(|i| (i % 251) as u8).collect();
            let mut data = original.clone();
            xor_inplace(&mut data);
            xor_inplace(&mut data);
            assert_eq!(data, original, "len={len}");
        }
    }
}
