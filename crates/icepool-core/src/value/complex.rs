//! Python's `complex` arithmetic, as CPython 3.13 computes it (Objects/complexobject.c), bit for bit.
//!
//! - A real operand (`bool`, `int`, `float`, `Fraction`) is converted to `complex(float(x), 0.0)` first, both by
//!   `complex`'s own slots (`TO_COMPLEX`) and by `Fraction`'s fallbacks (`complex(a)`), so `1j * inf` is
//!   `(nan+infj)`: 3.13 has no special mixed real/complex arithmetic (3.14 added it).
//! - `+`, `-`, `*` are the textbook formulas (`_Py_c_sum`, `_Py_c_diff`, `_Py_c_prod`), `/` is Smith's algorithm
//!   (`_Py_c_quot`), `**` is `c_powi` (repeated squaring) for small integral exponents, else `_Py_c_pow` (polar
//!   form). No infinity recovery in any of them (that too came in 3.14).
//! - `//`, `%`, the bitwise operators and `<` etc. raise `TypeError`, as do `~`, `math.trunc/floor/ceil`, `round`.
//!
//! CPython's x86_64 build has no FMA contractions, and neither does Rust, so the plain expressions agree.

use super::ops::{BinOp, N};
use super::{pow, Value};
use crate::error::{Error, Result};

/// CPython's `Py_complex`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct C {
    re: f64,
    im: f64,
}

const C1: C = C { re: 1.0, im: 0.0 };

impl From<C> for Value {
    fn from(c: C) -> Value {
        Value::Complex(c.re, c.im)
    }
}

/// The C `errno` the complex routines report through: `EDOM` (a division by zero) or `ERANGE` (an overflow).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Errno {
    Ok,
    Dom,
    Range,
}

/// `TO_COMPLEX` / `complex(x)`: a complex as it is, a real number as `complex(float(x), 0.0)` (which raises
/// `OverflowError` for an int or Fraction too large for a float). `None` for anything else.
fn to_complex(v: &Value) -> Option<Result<C>> {
    if let Value::Complex(re, im) = v {
        return Some(Ok(C { re: *re, im: *im }));
    }
    N::of(v).map(|n| n.to_f64().map(|re| C { re, im: 0.0 }))
}

fn sum(a: C, b: C) -> C {
    C {
        re: a.re + b.re,
        im: a.im + b.im,
    }
}

fn diff(a: C, b: C) -> C {
    C {
        re: a.re - b.re,
        im: a.im - b.im,
    }
}

fn prod(a: C, b: C) -> C {
    C {
        re: a.re * b.re - a.im * b.im,
        im: a.re * b.im + a.im * b.re,
    }
}

/// `_Py_c_quot`. Its comment: the original algorithm "is grossly prone to spurious overflow and underflow errors";
/// this one is better, and is pretty obvious: first divide the numerators and denominator by whichever of
/// {b.real, b.imag} has larger magnitude (CACM Algorithm 116, Complex Division, Robert L. Smith). As usual, though,
/// we're still ignoring all IEEE endcases. A zero divisor gives `0j` and `EDOM`.
fn quot(a: C, b: C) -> (C, Errno) {
    let abs_breal = b.re.abs();
    let abs_bimag = b.im.abs();
    if abs_breal >= abs_bimag {
        // Divide tops and bottom by b.real.
        if abs_breal == 0.0 {
            return (C { re: 0.0, im: 0.0 }, Errno::Dom);
        }
        let ratio = b.im / b.re;
        let denom = b.re + b.im * ratio;
        let c = C {
            re: (a.re + a.im * ratio) / denom,
            im: (a.im - a.re * ratio) / denom,
        };
        (c, Errno::Ok)
    } else if abs_bimag >= abs_breal {
        // Divide tops and bottom by b.imag.
        let ratio = b.re / b.im;
        let denom = b.re * ratio + b.im;
        let c = C {
            re: (a.re * ratio + a.im) / denom,
            im: (a.im * ratio - a.re) / denom,
        };
        (c, Errno::Ok)
    } else {
        // At least one of b.real or b.imag is a NaN.
        (
            C {
                re: f64::NAN,
                im: f64::NAN,
            },
            Errno::Ok,
        )
    }
}

/// `c_powu`: `x ** n` for `n >= 0` by repeated squaring (the last squaring is computed and not used, as in C).
fn powu(x: C, n: i64) -> C {
    let mut r = C1;
    let mut p = x;
    let mut mask: i64 = 1;
    while mask > 0 && n >= mask {
        if n & mask != 0 {
            r = prod(r, p);
        }
        mask <<= 1;
        p = prod(p, p);
    }
    r
}

/// `c_powi`: a negative power is `1 / x ** -n`, which may divide by zero.
fn powi(x: C, n: i64) -> (C, Errno) {
    if n > 0 {
        (powu(x, n), Errno::Ok)
    } else {
        quot(C1, powu(x, -n))
    }
}

/// `_Py_c_pow`, in polar form. The `errno` is the one the C library leaves: `cos` and `sin` set `EDOM` for an
/// infinite phase (and run last); the `ERANGE` of an overflow or underflow in `pow`/`exp`/`hypot` is decided
/// afterwards from the result (`_Py_ADJUST_ERANGE2`), so it is not tracked here.
fn c_pow(a: C, b: C) -> (C, Errno) {
    if b.re == 0.0 && b.im == 0.0 {
        return (C1, Errno::Ok);
    }
    if a.re == 0.0 && a.im == 0.0 {
        let errno = if b.im != 0.0 || b.re < 0.0 {
            Errno::Dom
        } else {
            Errno::Ok
        };
        return (C { re: 0.0, im: 0.0 }, errno);
    }
    let vabs = super::libm::hypot(a.re, a.im);
    let mut len = pow::pow(vabs, b.re);
    let at = a.im.atan2(a.re);
    let mut phase = at * b.re;
    if b.im != 0.0 {
        // `len *= exp(-at * b.imag)`, as the oracle's CPython 3.13 binary computes it (read off the disassembly of
        // `_Py_c_pow`): not `len /= exp(at * b.imag)`, which rounds differently.
        len *= (-at * b.im).exp();
        phase += b.im * super::libm::log(vabs);
    }
    let errno = if phase.is_infinite() {
        Errno::Dom
    } else {
        Errno::Ok
    };
    (
        C {
            re: len * phase.cos(),
            im: len * phase.sin(),
        },
        errno,
    )
}

/// `complex_pow`: `a ** b` with the "faster and more accurate algorithm" for a small integral exponent.
fn complex_pow(a: C, b: C) -> Result<C> {
    let (p, mut errno) = if b.im == 0.0 && b.re == b.re.floor() && b.re.abs() <= 100.0 {
        powi(a, b.re as i64)
    } else {
        c_pow(a, b)
    };
    // `_Py_ADJUST_ERANGE2`: an infinite part is an overflow (unless errno is already set); otherwise any ERANGE was
    // an underflow, which is fine.
    if p.re.is_infinite() || p.im.is_infinite() {
        if errno == Errno::Ok {
            errno = Errno::Range;
        }
    } else if errno == Errno::Range {
        errno = Errno::Ok;
    }
    match errno {
        Errno::Dom => Err(Error::ZeroDivision(
            "0.0 to a negative or complex power".into(),
        )),
        Errno::Range => Err(Error::Overflow("complex exponentiation".into())),
        Errno::Ok => Ok(p),
    }
}

/// `x ** y` for a negative real `x` and a non-integral real `y`: `float.__pow__` hands this case to
/// `complex.__pow__`.
pub(super) fn real_pow(x: f64, y: f64) -> Result<Value> {
    complex_pow(C { re: x, im: 0.0 }, C { re: y, im: 0.0 }).map(Value::from)
}

/// `a <op> b` where `a` or `b` is a complex number. `None` when the other operand is not a number: the caller
/// answers as for any other type (`TypeError`, or the other operand's own method).
pub(super) fn binary(op: BinOp, a: &Value, b: &Value) -> Option<Result<Value>> {
    use BinOp::*;
    if !matches!(op, Add | Sub | Mul | TrueDiv | Pow) {
        // complex has no `//`, `%` (removed in Python 3) or bitwise slots, and int's and Fraction's decline a
        // complex operand: TypeError, before anything is converted.
        return None;
    }
    let (x, y) = match (to_complex(a)?, to_complex(b)?) {
        (Ok(x), Ok(y)) => (x, y),
        (Err(e), _) | (_, Err(e)) => return Some(Err(e)),
    };
    Some(
        match op {
            Add => Ok(sum(x, y)),
            Sub => Ok(diff(x, y)),
            Mul => Ok(prod(x, y)),
            TrueDiv => match quot(x, y) {
                (_, Errno::Dom) => Err(Error::ZeroDivision("complex division by zero".into())),
                (q, _) => Ok(q),
            },
            Pow => complex_pow(x, y),
            _ => unreachable!("filtered above"),
        }
        .map(Value::from),
    )
}

/// `abs(complex)`: `_Py_c_abs`. C99 rules: if either the real or the imaginary part is an infinity, return
/// infinity, even if the other part is a NaN. A finite number whose magnitude overflows raises.
pub(super) fn abs(re: f64, im: f64) -> Result<Value> {
    if !re.is_finite() || !im.is_finite() {
        if re.is_infinite() {
            return Ok(Value::Float(re.abs()));
        }
        if im.is_infinite() {
            return Ok(Value::Float(im.abs()));
        }
        // Either the real or imaginary part is a NaN, and neither is infinite. Result should be NaN.
        return Ok(Value::Float(f64::NAN));
    }
    let result = super::libm::hypot(re, im);
    if result.is_finite() {
        Ok(Value::Float(result))
    } else {
        Err(Error::Overflow("absolute value too large".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(re: f64, im: f64) -> C {
        C { re, im }
    }

    #[test]
    fn cpython_313_endcases() {
        // No infinity recovery in 3.13: (1+1j) / (inf+infj) is nan+nanj.
        let (q, e) = quot(c(1.0, 1.0), c(f64::INFINITY, f64::INFINITY));
        assert!(q.re.is_nan() && q.im.is_nan() && e == Errno::Ok);
        assert!(quot(c(1.0, 1.0), c(0.0, -0.0)).1 == Errno::Dom);
        // (inf+0j) ** 2 is nan+nanj by repeated squaring, not an overflow.
        let p = complex_pow(c(f64::INFINITY, 0.0), c(2.0, 0.0)).unwrap();
        assert!(p.re.is_nan() && p.im.is_nan());
        assert_eq!(
            complex_pow(c(1.0, 1.0), c(-100.0, 0.0)),
            Ok(c(-8.881784197001252e-16, -0.0))
        );
        assert!(matches!(
            complex_pow(c(1e-200, 0.0), c(-2.0, 0.0)),
            Err(Error::ZeroDivision(_))
        ));
        // An infinite phase: cos(inf) sets EDOM.
        assert!(matches!(
            complex_pow(c(-1.0, 1.0), c(1e308, 0.0)),
            Err(Error::ZeroDivision(_))
        ));
        assert_eq!(complex_pow(c(0.0, 0.0), c(0.0, 0.0)), Ok(C1));
    }
}
