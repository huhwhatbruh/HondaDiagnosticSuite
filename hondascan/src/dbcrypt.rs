//! The packaged database is sealed, which makes a flipped bit or a swapped file fail instead of answering wrongly. The shipped
//! file is `hondascan.db.enc`: `MAGIC | nonce(12) | ChaCha20-Poly1305(plaintext)`, the magic bound as associated data, so a
//! damaged file fails the tag instead of yielding garbage. The key is built in at compile time (`build.rs`:
//! `HONDASCAN_DB_KEY`, else `release-key.txt`, which is public). This is integrity, not secrecy: the key is published, and a
//! source build reads the plain `hondascan.db`. `db.rs` loads the plaintext straight into SQLite memory (no file, no temp path).
use chacha20poly1305::{aead::{Aead, KeyInit, Payload}, ChaCha20Poly1305, Key, Nonce};

include!(concat!(env!("OUT_DIR"), "/dbkey.rs"));

const MAGIC: &[u8; 4] = b"HDB2";
const NONCE_LEN: usize = 12;

/// Encrypt a database file's bytes into the `.enc` form. `nonce` must be unique per call under one key.
pub fn encrypt(plain: &[u8], nonce: [u8; NONCE_LEN]) -> Vec<u8> {
    let ct = ChaCha20Poly1305::new(&Key::from(KEY))
        .encrypt(&Nonce::from(nonce), Payload { msg: plain, aad: MAGIC })
        .expect("chacha20poly1305 encrypt");
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
    let nonce = Nonce::try_from(nonce).map_err(|_| BAD)?;
    ChaCha20Poly1305::new(&Key::from(KEY)).decrypt(&nonce, Payload { msg: ct, aad: MAGIC }).map_err(|_| BAD)
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
}
