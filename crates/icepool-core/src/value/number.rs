//! Python's numeric tower for comparisons and hashing: `bool`/`int`, `float` and `Fraction` compare exactly.

use std::borrow::Cow;
use std::cmp::Ordering;

use dashu_base::UnsignedAbs;
use dashu_int::{IBig, UBig};
use dashu_ratio::RBig;

/// A borrowed number. `bool` is viewed as the `int` 0 or 1.
///
/// ```
/// use icepool_core::value::Num;
/// use icepool_core::Value;
///
/// // `2**53 + 1 > 2.0**53`: Python compares exactly, never rounding the int to a float.
/// let big = Value::from((1i64 << 53) + 1);
/// assert!(big.as_num().unwrap() > Num::Float((1u64 << 53) as f64));
/// // Equal numbers hash alike: `hash(True) == hash(1.0) == 1`; `hash(0.5)` is `2**60`.
/// assert_eq!(Value::Bool(true).as_num().unwrap().py_hash(), Num::Float(1.0).py_hash());
/// assert_eq!(Num::Float(0.5).py_hash(), 1 << 60);
/// assert!(Value::str("a").as_num().is_none());
/// ```
#[derive(Clone, Debug)]
pub enum Num<'a> {
    /// An `int` (or a `bool`).
    Int(Cow<'a, IBig>),
    /// A `float`.
    Float(f64),
    /// A `Fraction`.
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
    /// NaN hashes by object identity in Python. It is never equal to anything, so any hash is consistent with
    /// `==`; a fresh one per call keeps NaN keys from all landing in one bucket, as identity hashing does in
    /// Python (sums of dice with NaN outcomes make many NaN outcomes, which a constant made quadratic).
    ///
    /// ```
    /// use icepool_core::Value;
    /// use icepool_core::value::builtins::fraction;
    /// // `hash(1.5) == hash(Fraction(3, 2)) == 1152921504606846977`; `hash(-1)` is -2 (-1 is CPython's error code).
    /// let three_halves = Value::Fraction(fraction(&Value::from(3)).unwrap() / fraction(&Value::from(2)).unwrap());
    /// assert_eq!(three_halves.as_num().unwrap().py_hash(), 1152921504606846977);
    /// assert_eq!(Value::Float(1.5).as_num().unwrap().py_hash(), 1152921504606846977);
    /// assert_eq!(Value::from(-1).as_num().unwrap().py_hash(), -2);
    /// ```
    pub fn py_hash(&self) -> i64 {
        let h = match self {
            // Fast path for machine-sized ints: `hash(n)` is `n` reduced modulo P, keeping the sign.
            Num::Int(i) if i64::try_from(&**i).is_ok() => {
                let n = i64::try_from(&**i).expect("checked");
                let r = (n.unsigned_abs() % P) as i64;
                if n < 0 {
                    -r
                } else {
                    r
                }
            }
            Num::Int(i) => hash_rational(i, &UBig::ONE),
            Num::Float(f) if f.is_nan() => {
                static NAN_IDENTITY: std::sync::atomic::AtomicI64 =
                    std::sync::atomic::AtomicI64::new(0);
                NAN_IDENTITY
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                    .wrapping_mul(0x9E37_79B9_7F4A_7C15_u64 as i64)
            }
            Num::Float(f) if f.is_infinite() => {
                if *f > 0.0 {
                    HASH_INF
                } else {
                    -HASH_INF
                }
            }
            Num::Float(f) => hash_finite_float(*f),
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

/// `_Py_HashDouble` for a finite float, from its bits rather than through an exact `RBig` (which made hashing
/// float outcomes the bulk of `n @ (float die)`: P1). A finite float is `mant * 2**e` with `mant < 2**53 < P`, and
/// `2**61 == 1 (mod P)`, so its hash is `mant` rotated left by `e mod 61` within 61 bits, negated for a negative
/// float: the same value CPython's frexp loop and `hash_rational` compute.
fn hash_finite_float(f: f64) -> i64 {
    let bits = f.to_bits();
    let exponent_bits = ((bits >> 52) & 0x7ff) as i64;
    let fraction = bits & ((1 << 52) - 1);
    // Subnormals have no implicit leading bit and the exponent of the least normal.
    let (mant, e) = if exponent_bits == 0 {
        (fraction, -1074)
    } else {
        (fraction | (1 << 52), exponent_bits - 1075)
    };
    let e = e.rem_euclid(61) as u32;
    let x = ((mant << e) & P) | (mant >> (61 - e));
    if f < 0.0 {
        -(x as i64)
    } else {
        x as i64
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

/// Python's `n / d` for two ints: the exact quotient correctly rounded to the nearest float (ties to even), as
/// CPython's `long_true_divide` does. Also `float(int)` (with `d = 1`) and `float(Fraction)` (which is
/// `numerator / denominator`). Raises `ZeroDivisionError` for `d == 0` and `OverflowError` beyond the float range.
///
/// The sign of a zero result is negative iff exactly one operand is negative: `0 / -3` is `-0.0`.
///
/// ```
/// use dashu_int::IBig;
/// use icepool_core::value::ratio_to_f64;
///
/// assert_eq!(ratio_to_f64(&IBig::from(1), &IBig::from(3)).unwrap(), 1.0 / 3.0);
/// // `0 / -3` is `-0.0`; `1 / 0` raises ZeroDivisionError.
/// assert!(ratio_to_f64(&IBig::from(0), &IBig::from(-3)).unwrap().is_sign_negative());
/// assert!(ratio_to_f64(&IBig::from(1), &IBig::from(0)).is_err());
/// ```
pub fn ratio_to_f64(n: &IBig, d: &IBig) -> crate::Result<f64> {
    if *d == IBig::ZERO {
        return Err(crate::Error::DivisionByZero("division by zero".into()));
    }
    let negative = (*n < IBig::ZERO) != (*d < IBig::ZERO);
    let (a, b) = (n.unsigned_abs(), d.unsigned_abs());
    let magnitude = if a.is_zero() {
        0.0
    } else {
        positive_ratio_to_f64(&a, &b)?
    };
    Ok(if negative { -magnitude } else { magnitude })
}

/// `float(i)` for an int: correctly rounded, `OverflowError` if too large.
///
/// ```
/// use dashu_int::IBig;
/// use icepool_core::value::int_to_f64;
///
/// // `float(2**53 + 1)` is `9007199254740992.0`: the tie rounds to even.
/// assert_eq!(int_to_f64(&IBig::from((1i64 << 53) + 1)).unwrap(), 9007199254740992.0);
/// // `float(10**400)` raises OverflowError.
/// assert!(int_to_f64(&IBig::from(10).pow(400)).is_err());
/// ```
pub fn int_to_f64(i: &IBig) -> crate::Result<f64> {
    ratio_to_f64(i, &IBig::ONE)
        .map_err(|_| crate::Error::Overflow("int too large to convert to float".into()))
}

/// `a / b` for positive `a` and `b`, correctly rounded (ties to even), subnormals included.
fn positive_ratio_to_f64(a: &UBig, b: &UBig) -> crate::Result<f64> {
    use dashu_base::{BitTest, DivRem};
    // Scale so the integer quotient has 55 or 56 bits: the 53 mantissa bits, a rounding bit and one more. The
    // remainder then only matters as a "sticky" bit (is the value exactly on the quotient, or above it?).
    let shift = 55 + b.bit_len() as i64 - a.bit_len() as i64;
    let (q, r) = if shift >= 0 {
        (a << shift as usize).div_rem(b)
    } else {
        a.div_rem(b << (-shift) as usize)
    };
    let q = u64::try_from(q).expect("the quotient has at most 56 bits");
    let sticky = !r.is_zero();
    let nbits = 64 - q.leading_zeros() as i64;
    // The value is (q + something in [0, 1)) * 2**-shift; its leading bit has weight 2**e.
    let e = nbits - 1 - shift;
    if e > 1023 {
        return Err(crate::Error::Overflow(
            "integer division result too large for a float".into(),
        ));
    }
    // Normal floats keep 53 bits; subnormals fewer, down to none (the value then rounds to 0 or the smallest one).
    let keep = if e >= -1022 { 53 } else { 53 - (-1022 - e) };
    if keep < 0 {
        return Ok(0.0);
    }
    let drop = (nbits - keep) as u32;
    let mut m = q >> drop;
    let rem = q & ((1u64 << drop) - 1);
    let half = 1u64 << (drop - 1);
    if rem > half || (rem == half && (sticky || m & 1 == 1)) {
        m += 1;
    }
    // m * 2**(weight of its lowest bit), exact unless it overflows (a carry out of 2**1024).
    let exp = e - keep + 1;
    let pow2 = if exp >= -1022 {
        f64::from_bits(((exp + 1023) as u64) << 52)
    } else {
        f64::from_bits(1u64 << (exp + 1074))
    };
    let x = m as f64 * pow2;
    if x.is_infinite() {
        return Err(crate::Error::Overflow(
            "integer division result too large for a float".into(),
        ));
    }
    Ok(x)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bit-level float hash agrees with the exact rational one on floats of every magnitude, subnormals included.
    #[test]
    fn float_hash_matches_rational_hash() {
        let mut x: u64 = 0x2545_F491_4F6C_DD1D;
        let mut floats = vec![
            0.0,
            -0.0,
            1.0,
            -1.0,
            0.5,
            f64::MIN_POSITIVE,
            5e-324,
            f64::MAX,
            -f64::MAX,
            1e100,
        ];
        for _ in 0..100_000 {
            // xorshift64: random bit patterns cover every exponent.
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            floats.push(f64::from_bits(x));
        }
        for f in floats.into_iter().filter(|f| f.is_finite()) {
            let r = RBig::try_from(f).unwrap();
            assert_eq!(
                hash_finite_float(f),
                hash_rational(r.numerator(), r.denominator()),
                "{f:e}"
            );
        }
        // CPython 3.13: `[hash(x) for x in [-1.0, 0.1, -2.5, 1e300, 5e-324]]`.
        let python = [
            -2,
            230584300921369408,
            -1152921504606846978,
            1224995262755759164,
            16777216,
        ];
        for (f, h) in [-1.0, 0.1, -2.5, 1e300, 5e-324].into_iter().zip(python) {
            assert_eq!(Num::Float(f).py_hash(), h, "{f:e}");
        }
    }

    fn r(n: i64, d: i64) -> f64 {
        ratio_to_f64(&IBig::from(n), &IBig::from(d)).unwrap()
    }

    #[test]
    fn true_division_is_correctly_rounded() {
        assert_eq!(r(1, 3), 1.0 / 3.0);
        assert_eq!(r(-7, 2), -3.5);
        assert_eq!(r(2, 3), 2.0 / 3.0);
        assert!(r(0, -3).is_sign_negative());
        // 2**53 + 1 is a tie between 2**53 and 2**53 + 2: to even.
        assert_eq!(r((1 << 53) + 1, 1), (1u64 << 53) as f64);
        assert_eq!(r((1 << 53) + 3, 1), ((1u64 << 53) + 4) as f64);
        let big = IBig::from(1) << 1100;
        assert_eq!(ratio_to_f64(&IBig::ONE, &big).unwrap(), 0.0);
        assert_eq!(
            ratio_to_f64(&IBig::ONE, &(IBig::from(1) << 1074)).unwrap(),
            f64::from_bits(1)
        );
        assert!(ratio_to_f64(&big, &IBig::ONE).is_err());
    }
}
