//! Who may talk to the gateway: the pairing code, and the lockout that makes a four-letter code safe to use.
use std::collections::{HashMap, HashSet};
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
/// Wrong codes allowed from all peers together in LOCK_WINDOW. Past it, only a peer that has already given the right code is
/// still heard: guessing from many addresses is held to MAX_WRONG_ALL a minute in total (24^4 / 20 = 16,588 minutes, 11.5
/// days, for every code), and the paired phone keeps its link meanwhile.
const MAX_WRONG_ALL: usize = 20;

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
/// has aged out: a short code cannot be guessed faster than MAX_WRONG a minute from one address, nor faster than MAX_WRONG_ALL
/// a minute from any number of addresses, and a device that guesses cannot lock a phone that has paired out of the Wi-Fi link.
#[derive(Default)]
pub(super) struct Throttle {
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    wrong: HashMap<String, Vec<Instant>>,
    /// Peers that have given the right code: the total limit does not shut them out.
    paired: HashSet<String>,
}

/// Seconds until the wrong code made at `oldest` has aged out, at least 1.
fn wait(oldest: Instant) -> u64 {
    LOCK_WINDOW.saturating_sub(oldest.elapsed()).as_secs() + 1
}

impl Throttle {
    /// Check one request's code (`right`) and count it if wrong, under one lock, so concurrent guesses cannot all slip past
    /// the check before any of them is counted.
    pub(super) fn attempt(&self, peer: &str, right: bool) -> Result<(), Refusal> {
        let mut guard = self.state.lock().unwrap();
        let State { wrong, paired } = &mut *guard;
        wrong.retain(|_, w| {
            w.retain(|t| t.elapsed() < LOCK_WINDOW);
            !w.is_empty()
        });
        if let Some(w) = wrong.get(peer).filter(|w| w.len() >= MAX_WRONG) {
            return Err(Refusal::Locked(wait(w[0])));
        }
        if !paired.contains(peer) && wrong.values().map(Vec::len).sum::<usize>() >= MAX_WRONG_ALL {
            let oldest = wrong.values().flatten().min().copied().unwrap_or_else(Instant::now);
            return Err(Refusal::Locked(wait(oldest)));
        }
        if right {
            paired.insert(peer.to_string());
            return Ok(());
        }
        wrong.entry(peer.to_string()).or_default().push(Instant::now());
        Err(Refusal::Wrong)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn many_addresses_together_are_held_to_the_total_and_a_paired_phone_is_not_shut_out() {
        let t = Throttle::default();
        assert!(t.attempt("phone", true).is_ok());
        for peer in 0..MAX_WRONG_ALL / (MAX_WRONG - 1) {
            for _ in 0..MAX_WRONG - 1 {
                assert!(matches!(t.attempt(&format!("guesser{peer}"), false), Err(Refusal::Wrong)));
            }
        }
        // no single address reached its own limit, yet a new address is refused, the right code included
        assert!(matches!(t.attempt("newcomer", true), Err(Refusal::Locked(s)) if (1..=60).contains(&s)));
        assert!(matches!(t.attempt("guesser0", false), Err(Refusal::Locked(_))));
        assert!(t.attempt("phone", true).is_ok());
    }

    #[test]
    fn the_wait_never_underflows_when_the_oldest_wrong_code_has_aged_out() {
        assert_eq!(wait(Instant::now() - LOCK_WINDOW - Duration::from_secs(5)), 1);
    }
}
