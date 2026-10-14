//! Python's numeric tower for comparisons and hashing: `bool`/`int`, `float` and `Fraction` compare exactly.

use std::borrow::Cow;
use std::cmp::Ordering;

use dashu_base::UnsignedAbs;
use dashu_int::{IBig, UBig};
use dashu_ratio::RBig;

/// A borrowed number. `bool` is viewed as the `int` 0 or 1.
#[derive(Clone, Debug)]
pub enum Num<'a> {
    Int(Cow<'a, IBig>),
    Float(f64),
    Fraction(&'a RBig),
}

/// CPython's `_PyHASH_MODULUS` (2**61 - 1) and `_PyHASH_INF`.
const P: u64 = (1 << 61) - 1;
const HASH_INF: i64 = 314159;

impl Num<'_> {
    /// The exact rational value, or `None` for inf and NaN.
    fn exact(&self) -> Option<RBig> {
        match self {
            Num::Int(i) => Some(RBig::from(i.as_ref().clone())),
            Num::Float(f) => RBig::try_from(*f).ok(),
            Num::Fraction(r) => Some((*r).clone()),
        }
    }

    /// Python's `hash()` for numbers: equal numbers hash alike whatever their type, because each is its exact
    /// value reduced modulo 2**61 - 1 (CPython `Objects/object.c` `_Py_HashDouble`, `fractions.Fraction.__hash__`).
    /// NaN hashes by object identity in Python; it is never equal to anything, so any constant will do.
    pub fn py_hash(&self) -> i64 {
        let h = match self {
            Num::Int(i) => hash_rational(i, &UBig::ONE),
            Num::Float(f) if f.is_nan() => 0,
            Num::Float(f) if f.is_infinite() => {
                if *f > 0.0 {
                    HASH_INF
                } else {
                    -HASH_INF
                }
            }
            _ => {
                let r = self.exact().expect("finite");
                hash_rational(r.numerator(), r.denominator())
            }
        };
        if h == -1 {
            -2
        } else {
            h
        }
    }
}

fn mod_p(x: &UBig) -> u64 {
    u64::try_from(x % UBig::from(P)).expect("reduced modulo P")
}

fn mul_mod(a: u64, b: u64) -> u64 {
    ((a as u128 * b as u128) % P as u128) as u64
}

/// `hash(Fraction(num, den))`. The denominator of a float or a reduced Fraction met here is never a multiple of P
/// unless it is huge; Python then gives `_PyHASH_INF`, and so do we.
fn hash_rational(num: &IBig, den: &UBig) -> i64 {
    let d = mod_p(den);
    let h = if d == 0 {
        HASH_INF as u64
    } else {
        // Inverse by Fermat: d ** (P - 2) mod P.
        let (mut base, mut exp, mut inv) = (d, P - 2, 1u64);
        while exp > 0 {
            if exp & 1 == 1 {
                inv = mul_mod(inv, base);
            }
            base = mul_mod(base, base);
            exp >>= 1;
        }
        mul_mod(mod_p(&num.unsigned_abs()), inv)
    };
    if *num < IBig::ZERO {
        -(h as i64)
    } else {
        h as i64
    }
}

/// Exact comparison, `None` when a NaN is involved (then every Python comparison is `False`).
impl PartialEq for Num<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.partial_cmp(other) == Some(Ordering::Equal)
    }
}

impl PartialOrd for Num<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        match (self, other) {
            (Num::Int(a), Num::Int(b)) => Some(a.cmp(b)),
            (Num::Float(a), Num::Float(b)) => a.partial_cmp(b),
            (Num::Fraction(a), Num::Fraction(b)) => Some(a.cmp(b)),
            // A float against an exact number: inf and NaN first, then exactly (Python never rounds the int).
            (Num::Float(f), _) if !f.is_finite() => f.partial_cmp(&0.0),
            (_, Num::Float(f)) if !f.is_finite() => 0.0f64.partial_cmp(f),
            _ => Some(self.exact()?.cmp(&other.exact()?)),
        }
    }
}
