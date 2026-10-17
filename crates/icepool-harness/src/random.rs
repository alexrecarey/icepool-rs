//! Python's `random.Random` as far as `Population.sample` uses it: `seed(int)` and `randrange(n)`, consuming the
//! Mersenne Twister exactly as CPython does (Modules/_randommodule.c and Lib/random.py), so a seeded program samples
//! the same outcomes as Python. The Python facade does not use this: it must draw from Python's own (global,
//! user-seeded) generator, and passes the drawn number to `Die::sample_at`.

use dashu_base::{BitTest, UnsignedAbs};
use dashu_int::{IBig, UBig};

use icepool_core::{Error, Result};

const N: usize = 624;
const M: usize = 397;
const MATRIX_A: u32 = 0x9908_b0df;
const UPPER_MASK: u32 = 0x8000_0000;
const LOWER_MASK: u32 = 0x7fff_ffff;

/// MT19937 with CPython's seeding.
pub struct Random {
    state: [u32; N],
    index: usize,
}

impl Random {
    /// `random.Random(a)` / `random.seed(a)` for an int `a`: the absolute value's 32-bit words, least significant
    /// first (one zero word for 0), go to `init_by_array`.
    pub fn new(seed: &IBig) -> Random {
        let mut key = Vec::new();
        let mut rest = seed.unsigned_abs();
        while rest > UBig::ZERO {
            key.push(u32::try_from(&rest & UBig::from(u32::MAX)).expect("masked to 32 bits"));
            rest >>= 32;
        }
        if key.is_empty() {
            key.push(0);
        }
        let mut r = Random {
            state: [0; N],
            index: N,
        };
        r.init_by_array(&key);
        r
    }

    fn init_genrand(&mut self, s: u32) {
        self.state[0] = s;
        for i in 1..N {
            let prev = self.state[i - 1];
            self.state[i] = 1_812_433_253u32
                .wrapping_mul(prev ^ (prev >> 30))
                .wrapping_add(i as u32);
        }
        self.index = N;
    }

    fn init_by_array(&mut self, key: &[u32]) {
        self.init_genrand(19_650_218);
        let mt = &mut self.state;
        let (mut i, mut j) = (1usize, 0usize);
        for _ in 0..N.max(key.len()) {
            let prev = mt[i - 1];
            mt[i] = (mt[i] ^ (prev ^ (prev >> 30)).wrapping_mul(1_664_525))
                .wrapping_add(key[j])
                .wrapping_add(j as u32); // non linear
            i += 1;
            j += 1;
            if i >= N {
                mt[0] = mt[N - 1];
                i = 1;
            }
            if j >= key.len() {
                j = 0;
            }
        }
        for _ in 0..N - 1 {
            let prev = mt[i - 1];
            mt[i] =
                (mt[i] ^ (prev ^ (prev >> 30)).wrapping_mul(1_566_083_941)).wrapping_sub(i as u32); // non linear
            i += 1;
            if i >= N {
                mt[0] = mt[N - 1];
                i = 1;
            }
        }
        mt[0] = 0x8000_0000; // MSB is 1; assuring non-zero initial array
    }

    /// `genrand_uint32`: the next 32 random bits.
    fn next_u32(&mut self) -> u32 {
        let mag01 = |y: u32| if y & 1 == 0 { 0 } else { MATRIX_A };
        if self.index >= N {
            // generate N words at one time
            let mt = &mut self.state;
            for kk in 0..N {
                let y = (mt[kk] & UPPER_MASK) | (mt[(kk + 1) % N] & LOWER_MASK);
                mt[kk] = mt[(kk + M) % N] ^ (y >> 1) ^ mag01(y);
            }
            self.index = 0;
        }
        let mut y = self.state[self.index];
        self.index += 1;
        y ^= y >> 11;
        y ^= (y << 7) & 0x9d2c_5680;
        y ^= (y << 15) & 0xefc6_0000;
        y ^ (y >> 18)
    }

    /// `getrandbits(k)` for `k >= 1`: 32-bit words from least to most significant, the last one keeping only its
    /// top bits.
    fn getrandbits(&mut self, k: usize) -> UBig {
        if k <= 32 {
            return UBig::from(self.next_u32() >> (32 - k));
        }
        let mut result = UBig::ZERO;
        let mut shift = 0;
        let mut left = k;
        while left > 0 {
            let mut r = self.next_u32();
            if left < 32 {
                r >>= 32 - left; // Drop least significant bits
            }
            result |= UBig::from(r) << shift;
            shift += 32;
            left = left.saturating_sub(32);
        }
        result
    }

    /// `randrange(stop)`: `_randbelow_with_getrandbits(stop)`, which draws `stop.bit_length()` bits until the number
    /// is below `stop`. `ValueError` for an empty range.
    pub fn randrange(&mut self, stop: &UBig) -> Result<UBig> {
        if *stop == UBig::ZERO {
            return Err(Error::InvalidArgument("empty range in randrange(0)".into()));
        }
        let k = stop.bit_len();
        loop {
            let r = self.getrandbits(k);
            if r < *stop {
                return Ok(r);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `random.seed(s); [random.randrange(n) for n in ns]`, recorded from CPython 3.13.
    #[test]
    fn matches_python() {
        let draw = |seed: IBig, ns: &[UBig]| {
            let mut r = Random::new(&seed);
            ns.iter()
                .map(|n| r.randrange(n).unwrap().to_string())
                .collect::<Vec<_>>()
        };
        let u = |n: u128| UBig::from(n);
        assert_eq!(
            draw(0.into(), &[u(6), u(6), u(6), u(100)]),
            ["3", "3", "0", "33"]
        );
        assert_eq!(
            draw((-12345).into(), &[u(1 << 40), u(7), u(1)]),
            ["593537256020", "4", "0"]
        );
        assert_eq!(
            draw((IBig::ONE << 70) + 5, &[UBig::from(10u8).pow(30), u(3)]),
            ["829904723954819379215411981483", "1"]
        );
        assert_eq!(
            draw(u64::MAX.into(), &[u((1 << 33) + 1), u(5)]),
            ["93740670", "2"]
        );
        assert!(Random::new(&IBig::ZERO).randrange(&UBig::ZERO).is_err());
    }
}
