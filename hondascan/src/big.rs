//! A minimal unsigned big integer: the raw value of a signal is the big-endian number its bytes form, and some HDS
//! signals (texts) have up to 62 one-byte parts, which Python's ints hold and a u128 does not.
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Big {
    limbs: Vec<u64>, // little-endian, no high zero limbs
}

impl Big {
    pub fn from_u64(x: u64) -> Big {
        Big { limbs: if x == 0 { vec![] } else { vec![x] } }
    }

    /// (self << bits) | low, for bits <= 8 and low < 2^bits.
    pub fn shl_or(&mut self, bits: u32, low: u64) {
        let mut carry = low;
        for l in self.limbs.iter_mut() {
            let next = if bits == 0 { 0 } else { *l >> (64 - bits) };
            *l = (*l << bits) | carry;
            carry = next;
        }
        if carry != 0 {
            self.limbs.push(carry);
        }
    }

    pub fn bit_len(&self) -> u32 {
        self.limbs.last().map_or(0, |l| (self.limbs.len() as u32 - 1) * 64 + (64 - l.leading_zeros()))
    }

    pub fn as_u64(&self) -> Option<u64> {
        match self.limbs.len() {
            0 => Some(0),
            1 => Some(self.limbs[0]),
            _ => None,
        }
    }

    /// Python's `float(int)`: the nearest double, ties to even.
    pub fn to_f64(&self) -> f64 {
        let len = self.bit_len();
        if len <= 64 {
            return self.as_u64().unwrap_or(0) as f64;
        }
        let shift = len - 64; // keep the top 64 bits; a nonzero remainder goes into the lowest kept bit as a sticky bit
        let (word, bit) = ((shift / 64) as usize, shift % 64);
        let mut top = self.limbs[word] >> bit;
        if bit != 0 {
            top |= self.limbs.get(word + 1).map_or(0, |h| h << (64 - bit));
        }
        let sticky = self.limbs[..word].iter().any(|l| *l != 0) || (bit != 0 && self.limbs[word] & ((1u64 << bit) - 1) != 0);
        if sticky {
            top |= 1;
        }
        (top as f64) * 2f64.powi(shift as i32)
    }
}

impl fmt::Display for Big {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.limbs.is_empty() {
            return write!(f, "0");
        }
        let mut limbs = self.limbs.clone();
        let mut chunks: Vec<u64> = vec![];
        while !limbs.is_empty() {
            let mut rem: u128 = 0;
            for l in limbs.iter_mut().rev() {
                let cur = (rem << 64) | *l as u128;
                *l = (cur / 10_000_000_000_000_000_000) as u64;
                rem = cur % 10_000_000_000_000_000_000;
            }
            chunks.push(rem as u64);
            while limbs.last() == Some(&0) {
                limbs.pop();
            }
        }
        let mut s = chunks.pop().unwrap().to_string();
        for c in chunks.iter().rev() {
            s.push_str(&format!("{c:019}"));
        }
        write!(f, "{s}")
    }
}
