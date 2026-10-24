//! Who may talk to the gateway: the pairing code, and the lockout that makes a four-letter code safe to use.
use std::collections::HashMap;
use std::io::Read;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Letters of a generated pairing code: no I or O, which read as 1 and 0.
const CODE_LETTERS: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ";
const CODE_LEN: usize = 4;
pub(super) const MIN_TOKEN: usize = 4;
/// Wrong codes allowed from one peer in LOCK_WINDOW before that peer is refused until the window has passed.
const MAX_WRONG: usize = 5;
const LOCK_WINDOW: Duration = Duration::from_secs(60);

/// A fresh random pairing code: CODE_LEN letters (24^4 = 331,776 possibilities, so the lockout below is what protects it).
pub fn new_token() -> Result<String, String> {
    let mut f = std::fs::File::open("/dev/urandom").map_err(|e| format!("no random pairing code: {e}"))?;
    let mut code = String::new();
    let mut b = [0u8; 1];
    while code.len() < CODE_LEN {
        f.read_exact(&mut b).map_err(|e| format!("no random pairing code: {e}"))?;
        if (b[0] as usize) < CODE_LETTERS.len() * 10 {
            code.push(CODE_LETTERS[b[0] as usize % CODE_LETTERS.len()] as char); // 240 of 256 values, so every letter is equally likely
        }
    }
    Ok(code)
}

/// Case does not matter: a phone keyboard capitalises the first letter.
pub(super) fn same_token(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x.to_ascii_lowercase() ^ y.to_ascii_lowercase())) == 0
}

pub(super) enum Refusal {
    /// The peer is locked out; seconds left.
    Locked(u64),
    Wrong,
}

/// Counts wrong codes per peer (an IP address for Wi-Fi, `ble` for every Bluetooth device, which cannot be told apart). After
/// MAX_WRONG within LOCK_WINDOW every request from that peer is refused, the right code included, until the oldest wrong one
/// has aged out: a short code cannot be guessed faster than MAX_WRONG a minute from one address, and a device that guesses
/// cannot lock the phone out of the Wi-Fi link (nor another address out of anything).
#[derive(Default)]
pub(super) struct Throttle {
    wrong: Mutex<HashMap<String, Vec<Instant>>>,
}

impl Throttle {
    /// Check one request's code (`right`) and count it if wrong, under one lock, so concurrent guesses cannot all slip past
    /// the check before any of them is counted.
    pub(super) fn attempt(&self, peer: &str, right: bool) -> Result<(), Refusal> {
        let mut all = self.wrong.lock().unwrap();
        all.retain(|_, w| {
            w.retain(|t| t.elapsed() < LOCK_WINDOW);
            !w.is_empty()
        });
        let w = all.entry(peer.to_string()).or_default();
        if w.len() >= MAX_WRONG {
            return Err(Refusal::Locked((LOCK_WINDOW - w[0].elapsed()).as_secs() + 1));
        }
        if right {
            if w.is_empty() {
                all.remove(peer);
            }
            return Ok(());
        }
        w.push(Instant::now());
        Err(Refusal::Wrong)
    }
}
