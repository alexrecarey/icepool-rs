//! Python builtins and `math` functions over `Value`, as CPython 3.13 computes them: `sum`, `float()`,
//! `fractions.Fraction(n, d)`, `math.sqrt`, `math.log`, `len` and `x[key]`. population/base.py's statistics are
//! written in terms of these, so the port gets their exact rounding and their exceptions.

use dashu_base::UnsignedAbs;
use dashu_int::IBig;
use dashu_ratio::RBig;

use super::{int_to_f64, ratio_to_f64, BinOp, Value};
use crate::error::{invalid_argument, wrong_type, Error, Result};

/// `sum(items, start)` (CPython 3.13 `builtin_sum_impl`), with its two fast paths, because they round differently
/// from adding one item at a time.
///
/// While the running total is an exact `int` it accumulates in a C `long` as long as the items are `int`s (or
/// `bool`s) and nothing overflows. If the total is a `float`, `float` items are added with Neumaier's compensated
/// summation and small `int`s are added directly *without* compensation; any other item first folds the
/// compensation into the total and then adds normally. Once the total leaves a fast path it never returns to one.
/// `items` is consumed lazily, so an error from computing an item surfaces at the same point as in Python.
pub fn py_sum(items: impl IntoIterator<Item = Result<Value>>, start: Value) -> Result<Value> {
    let mut iter = items.into_iter();
    let mut result = start;
    if let Value::Int(i) = &result {
        if let Ok(mut acc) = i64::try_from(i) {
            result = loop {
                let Some(item) = iter.next() else {
                    return Ok(Value::from(acc));
                };
                let item = item?;
                let small = match &item {
                    Value::Int(x) => i64::try_from(x).ok(),
                    Value::Bool(b) => Some(*b as i64),
                    _ => None,
                };
                if let Some(sum) = small.and_then(|b| acc.checked_add(b)) {
                    acc = sum;
                    continue;
                }
                // Either overflowed or is not an int. Restore real objects and process normally.
                break Value::from(acc).binary(BinOp::Add, &item)?;
            };
        }
    }
    if let Value::Float(f) = result {
        let (mut f_result, mut c) = (f, 0.0f64);
        result = loop {
            let Some(item) = iter.next() else {
                // Avoid losing the sign on a negative result, and don't let adding the compensation convert an
                // infinite or overflowed sum to a NaN.
                if c != 0.0 && c.is_finite() {
                    f_result += c;
                }
                return Ok(Value::Float(f_result));
            };
            let item = item?;
            match &item {
                // Improved Kahan–Babuška algorithm by Arnold Neumaier.
                Value::Float(x) => {
                    let t = f_result + x;
                    if f_result.abs() >= x.abs() {
                        c += (f_result - t) + x;
                    } else {
                        c += (x - t) + f_result;
                    }
                    f_result = t;
                    continue;
                }
                Value::Bool(b) => {
                    f_result += *b as i64 as f64;
                    continue;
                }
                Value::Int(x) => {
                    if let Ok(v) = i64::try_from(x) {
                        f_result += v as f64;
                        continue;
                    }
                }
                _ => {}
            }
            if c != 0.0 && c.is_finite() {
                f_result += c;
            }
            break Value::Float(f_result).binary(BinOp::Add, &item)?;
        };
    }
    for item in iter {
        result = result.binary(BinOp::Add, &item?)?;
    }
    Ok(result)
}

/// Python's `min(iterable)` (`max` false) or `max(iterable)`: the first of the least (greatest) elements, comparing
/// each `item < best` (`item > best`); an empty iterable raises ValueError.
pub fn py_min_max(items: Vec<Value>, max: bool) -> Result<Value> {
    let mut items = items.into_iter();
    let Some(mut best) = items.next() else {
        let f = if max { "max" } else { "min" };
        return invalid_argument(format!("{f}() iterable argument is empty"));
    };
    for item in items {
        best = if max {
            Value::py_max(best, item)?
        } else {
            Value::py_min(best, item)?
        };
    }
    Ok(best)
}

/// `PyFloat_AsDouble(x)`, which `math` functions apply to their arguments: `float(x)` for numbers (a `Fraction`
/// is `numerator / denominator`, correctly rounded), `TypeError` for anything else.
pub fn to_float(x: &Value) -> Result<f64> {
    match x {
        Value::Float(f) => Ok(*f),
        Value::Bool(b) => Ok(*b as u8 as f64),
        Value::Int(i) => int_to_f64(i),
        Value::Fraction(r) => ratio_to_f64(r.numerator(), &IBig::from(r.denominator().clone())),
        Value::Host(h) => h.to_float(),
        _ => wrong_type(format!("must be real number, not {}", x.type_name())),
    }
}

/// `fractions.Fraction(numerator, denominator)` for an `int` denominator, falling back to `numerator /
/// denominator` where Python's constructor raises `TypeError` (icepool's `math.try_fraction`). A rational
/// numerator (`int`, `bool`, `Fraction`, or a host object that is a `numbers.Rational`) makes a `Fraction`, even an
/// integral one; anything else is divided (`float / int` is a `float`).
pub fn try_fraction(numerator: &Value, denominator: &IBig) -> Result<Value> {
    let rational = match numerator {
        Value::Int(i) => Some(RBig::from(i.clone())),
        Value::Bool(b) => Some(RBig::from(*b as u8)),
        Value::Fraction(r) => Some(r.clone()),
        Value::Host(h) => h.as_rational()?,
        _ => None,
    };
    match rational {
        Some(_) if *denominator == IBig::ZERO => {
            Err(Error::DivisionByZero(format!("Fraction({numerator}, 0)")))
        }
        Some(r) => Ok(Value::Fraction(r / RBig::from(denominator.clone()))),
        None => numerator.binary(BinOp::TrueDiv, &Value::Int(denominator.clone())),
    }
}

fn domain_error<T>() -> Result<T> {
    Err(Error::InvalidArgument("math domain error".into()))
}

/// CPython's `math_1(x, func, can_overflow=0)`: a NaN from a non-NaN argument, or an infinity from a finite
/// one, is a `ValueError`.
fn math_1(x: &Value, func: fn(f64) -> f64) -> Result<f64> {
    let x = to_float(x)?;
    let r = func(x);
    if (r.is_nan() && !x.is_nan()) || (r.is_infinite() && x.is_finite()) {
        return domain_error();
    }
    Ok(r)
}

/// `math.sqrt(x)`. `sqrt` is correctly rounded everywhere (IEEE 754), so it is bit-exact even under WASM.
pub fn math_sqrt(x: &Value) -> Result<Value> {
    math_1(x, f64::sqrt).map(Value::Float)
}

/// CPython's `m_log`: `log` for positive finite arguments, with C99's special values made explicit.
fn m_log(x: f64) -> f64 {
    if x.is_finite() {
        if x > 0.0 {
            x.ln()
        } else if x == 0.0 {
            f64::NEG_INFINITY
        } else {
            f64::NAN
        }
    } else if x.is_nan() || x > 0.0 {
        x
    } else {
        f64::NAN
    }
}

/// CPython's `m_log10`: `log10` for positive finite arguments, the special values as `m_log`.
fn m_log10(x: f64) -> f64 {
    if x.is_finite() {
        if x > 0.0 {
            x.log10()
        } else if x == 0.0 {
            f64::NEG_INFINITY
        } else {
            f64::NAN
        }
    } else if x.is_nan() || x > 0.0 {
        x
    } else {
        f64::NAN
    }
}

/// CPython's `loghelper(arg, func)`. An `int` is handled directly (so a huge one still has a logarithm:
/// `func(x) + func(2) * e` from its `frexp`); anything else goes through `math_1`.
fn loghelper(x: &Value, func: fn(f64) -> f64) -> Result<f64> {
    let m_log = func;
    let int = match x {
        Value::Int(i) => i.clone(),
        Value::Bool(b) => IBig::from(*b as u8),
        _ => return math_1(x, m_log),
    };
    // Negative or zero inputs give a ValueError.
    if int <= IBig::ZERO {
        return domain_error();
    }
    match int_to_f64(&int) {
        Ok(f) => Ok(m_log(f)),
        Err(_) => {
            // Here the conversion to double overflowed, but it's possible to compute the log anyway.
            let (m, e) = frexp_int(&int);
            // Value is ~= x * 2**e, so the log ~= log(x) + log(2) * e.
            Ok(m_log(m) + m_log(2.0) * e as f64)
        }
    }
}

/// `_PyLong_Frexp` for a positive int too large for a float: `(m, e)` with `m` in `[0.5, 1)` and `int ~= m *
/// 2**e`, `m` correctly rounded (half to even) to 53 bits.
fn frexp_int(int: &IBig) -> (f64, i64) {
    use dashu_base::{BitTest, UnsignedAbs};
    let bits = int.unsigned_abs().bit_len() as i64;
    // Scale to exactly `bits` binary digits after the point: `ratio_to_f64` rounds as `_PyLong_Frexp` does.
    let shift = bits - 1000;
    let top = int >> shift as usize;
    let rest_nonzero = (top.clone() << shift as usize) != *int;
    // Keep a sticky bit below the kept digits so ties round the same way as the full value.
    let top = (top << 1) + IBig::from(rest_nonzero as u8);
    let m = ratio_to_f64(&top, &(IBig::ONE << 1001)).expect("in range");
    // A carry may round m up to 1.0; frexp then reports 0.5 and one more in the exponent.
    if m == 1.0 {
        (0.5, bits + 1)
    } else {
        (m, bits)
    }
}

/// `math.log(x)` (natural) or `math.log(x, base)`: `log(x) / log(base)`, `ZeroDivisionError` for base 1.
pub fn math_log(x: &Value, base: Option<&Value>) -> Result<Value> {
    let num = loghelper(x, m_log)?;
    let Some(base) = base else {
        return Ok(Value::Float(num));
    };
    let den = loghelper(base, m_log)?;
    Value::Float(num).binary(BinOp::TrueDiv, &Value::Float(den))
}

/// `math.log10(x)`.
pub fn math_log10(x: &Value) -> Result<f64> {
    loghelper(x, m_log10)
}

/// `math.ceil(x)` of a float: an int, `OverflowError` for an infinity, `ValueError` for NaN.
pub fn float_ceil(x: f64) -> Result<IBig> {
    super::ops::float_to_int(x.ceil())
}

/// The key of `x[key]` (`operator.getitem`): any object, or a slice.
#[derive(Clone, Debug)]
pub enum Index {
    /// Any object.
    Item(Value),
    /// `slice(start, stop, step)`; `Value::None` where omitted.
    Slice(Value, Value, Value),
}

/// `PyNumber_AsSsize_t` for an index: `int`s, `bool`s and host objects with `__index__` (others raise
/// `TypeError`); with `clamp`, values beyond the C range saturate, as slice indices do (`_PyEval_SliceIndex`),
/// else they raise `IndexError`.
fn ssize(v: &Value, clamp: bool, what: &str) -> Result<i64> {
    let i = match v {
        Value::Int(i) => i.clone(),
        Value::Bool(b) => IBig::from(*b as u8),
        Value::Host(h) => match h.as_index()? {
            Some(i) => i,
            None => return wrong_type(what.to_string()),
        },
        _ => return wrong_type(what.to_string()),
    };
    match i64::try_from(&i) {
        Ok(x) => Ok(x),
        Err(_) if clamp => Ok(if i < IBig::ZERO { i64::MIN } else { i64::MAX }),
        Err(_) => Err(Error::IndexOutOfRange(
            "cannot fit 'int' into an index-sized integer".into(),
        )),
    }
}

/// `PySlice_Unpack` + `PySlice_AdjustIndices`: the positions a slice selects from a sequence of `len` items.
fn slice_positions(start: &Value, stop: &Value, step: &Value, len: usize) -> Result<Vec<usize>> {
    const MSG: &str = "slice indices must be integers or None or have an __index__ method";
    let step = match step {
        Value::None => 1,
        s => ssize(s, true, MSG)?,
    };
    if step == 0 {
        return Err(Error::InvalidArgument("slice step cannot be zero".into()));
    }
    // Ensure step can be negated without overflow.
    let step = step.max(-i64::MAX) as i128;
    let bound = |v: &Value, if_none: i64| match v {
        Value::None => Ok(if_none),
        x => ssize(x, true, MSG),
    };
    let start = bound(start, if step < 0 { i64::MAX } else { 0 })? as i128;
    let stop = bound(stop, if step < 0 { i64::MIN } else { i64::MAX })? as i128;
    let len = len as i128;
    let adjust = |x: i128| {
        if x < 0 {
            let x = x + len;
            if x < 0 {
                if step < 0 {
                    -1
                } else {
                    0
                }
            } else {
                x
            }
        } else if x >= len {
            if step < 0 {
                len - 1
            } else {
                len
            }
        } else {
            x
        }
    };
    let (start, stop) = (adjust(start), adjust(stop));
    let count = if step < 0 {
        if stop < start {
            (start - stop - 1) / -step + 1
        } else {
            0
        }
    } else if start < stop {
        (stop - start - 1) / step + 1
    } else {
        0
    };
    Ok((0..count).map(|i| (start + i * step) as usize).collect())
}

/// `getattr(x, name)` for the data attributes of Python's numeric tower (`real`, `imag`, `numerator`,
/// `denominator`), which a type either has or lacks (AttributeError), and `Symbols.__getattr__` (`s[name]`, for
/// any name that is not one of its own attributes, unless it starts with `_`). Any other name may be a method
/// (`conjugate`, `count`, ...), a bound method that is no outcome: the port does not model those.
pub fn getattr(x: &Value, name: &str) -> Result<Value> {
    let missing = || {
        Err(Error::MissingAttribute(format!(
            "'{}' object has no attribute '{name}'",
            x.type_name()
        )))
    };
    if !matches!(name, "real" | "imag" | "numerator" | "denominator") {
        return Err(Error::Unported(format!("getattr(x, {name:?})")));
    }
    Ok(match (x, name) {
        // `bool` is an `int`: its attributes are ints (`True.real` is `1`).
        (Value::Bool(b), "real" | "numerator") => Value::from(*b as i64),
        (Value::Int(i), "real" | "numerator") => Value::Int(i.clone()),
        (Value::Bool(_) | Value::Int(_), "imag") => Value::from(0),
        (Value::Bool(_) | Value::Int(_), "denominator") => Value::from(1),
        (Value::Float(f), "real") => Value::Float(*f),
        (Value::Float(_), "imag") => Value::Float(0.0),
        // `Fraction.real` is the fraction itself, its `imag` the int 0.
        (Value::Fraction(r), "real") => Value::Fraction(r.clone()),
        (Value::Fraction(_), "imag") => Value::from(0),
        (Value::Fraction(r), "numerator") => Value::Int(r.numerator().clone()),
        (Value::Fraction(r), "denominator") => Value::Int(IBig::from(r.denominator().clone())),
        (Value::Complex(re, _), "real") => Value::Float(*re),
        (Value::Complex(_, im), "imag") => Value::Float(*im),
        (Value::Symbols(s), _) => s.getitem(name),
        (Value::Host(_), _) => return Err(Error::Unported("getattr of a host object".into())),
        _ => return missing(),
    })
}

/// `x[key]` for the sequences the core models (`tuple`, `str`) and host objects; other values are not
/// subscriptable.
pub fn getitem(x: &Value, key: &Index) -> Result<Value> {
    match (x, key) {
        // `Vector.__getitem__`: an int index gives the element, anything else `Vector(self._data[index])`.
        (Value::Vector(v), Index::Item(Value::Int(_) | Value::Bool(_))) => {
            return getitem(&Value::Tuple(v.data()?.clone()), key)
        }
        (Value::Vector(v), Index::Slice(..)) => {
            let Value::Tuple(t) = getitem(&Value::Tuple(v.data()?.clone()), key)? else {
                unreachable!("a slice of a tuple is a tuple")
            };
            return Ok(Value::Vector(super::Vector::new(t.to_vec())));
        }
        (Value::Vector(v), Index::Item(i)) => {
            // A non-int index: the tuple raises, unless it has `__index__` (a host object).
            return match getitem(&Value::Tuple(v.data()?.clone()), key)? {
                _ if matches!(i, Value::Host(_)) => {
                    Err(Error::Unported("a Vector indexed by a host object".into()))
                }
                _ => unreachable!("the tuple raised"),
            };
        }
        // `Symbols.__getitem__`: `len(key)` first.
        (Value::Symbols(s), Index::Item(Value::Str(k))) => return Ok(s.getitem(k)),
        (
            Value::Symbols(_),
            Index::Item(
                k @ (Value::Tuple(_) | Value::Vector(_) | Value::Symbols(_) | Value::Host(_)),
            ),
        ) => {
            return Err(Error::Unported(format!(
                "Symbols indexed by a {}",
                k.type_name()
            )))
        }
        (Value::Symbols(_), Index::Item(k)) => {
            return wrong_type(format!("object of type '{}' has no len()", k.type_name()))
        }
        (Value::Symbols(_), Index::Slice(..)) => {
            return wrong_type("object of type 'slice' has no len()")
        }
        _ => {}
    }
    let (kind, len) = match x {
        Value::Tuple(t) => ("tuple", t.len()),
        Value::Str(s) => ("string", s.chars().count()),
        Value::Host(h) => return h.getitem(key),
        _ => return wrong_type(format!("'{}' object is not subscriptable", x.type_name())),
    };
    let positions = match key {
        Index::Item(i) => {
            let msg = match x {
                Value::Tuple(_) => format!(
                    "tuple indices must be integers or slices, not {}",
                    i.type_name()
                ),
                _ => format!("string indices must be integers, not '{}'", i.type_name()),
            };
            let i = ssize(i, false, &msg)?;
            let i = if i < 0 { i + len as i64 } else { i };
            if i < 0 || i >= len as i64 {
                return Err(Error::IndexOutOfRange(format!("{kind} index out of range")));
            }
            let i = i as usize;
            return Ok(match x {
                Value::Tuple(t) => t[i].clone(),
                Value::Str(s) => Value::str(&s.chars().nth(i).expect("in range").to_string()),
                _ => unreachable!("a sequence"),
            });
        }
        Index::Slice(start, stop, step) => slice_positions(start, stop, step, len)?,
    };
    Ok(match x {
        Value::Tuple(t) => Value::tuple(positions.iter().map(|&i| t[i].clone()).collect()),
        Value::Str(s) => {
            let chars: Vec<char> = s.chars().collect();
            Value::str(&positions.iter().map(|&i| chars[i]).collect::<String>())
        }
        _ => unreachable!("a sequence"),
    })
}

/// `len(x)`.
pub fn py_len(x: &Value) -> Result<usize> {
    match x {
        Value::Tuple(t) => Ok(t.len()),
        Value::Str(s) => Ok(s.chars().count()),
        Value::Vector(v) => Ok(v.data()?.len()),
        Value::Symbols(s) => Ok(s.len()),
        Value::Host(h) => h.try_len(),
        _ => wrong_type(format!("object of type '{}' has no len()", x.type_name())),
    }
}

/// `fractions.Fraction(x)` for one argument: an `int` / `bool` / `Fraction` exactly, a `float` exactly (its
/// `as_integer_ratio()`; NaN is ValueError, an infinity OverflowError), a host `numbers.Rational`. A `str` is parsed
/// by Python (not ported); anything else is TypeError.
pub fn fraction(x: &Value) -> Result<RBig> {
    match x {
        Value::Int(i) => Ok(RBig::from(i.clone())),
        Value::Bool(b) => Ok(RBig::from(*b as u8)),
        Value::Fraction(r) => Ok(r.clone()),
        Value::Float(f) if f.is_nan() => Err(Error::InvalidArgument(
            "cannot convert NaN to integer ratio".into(),
        )),
        Value::Float(f) if f.is_infinite() => Err(Error::Overflow(
            "cannot convert Infinity to integer ratio".into(),
        )),
        Value::Float(f) => Ok(RBig::try_from(*f).expect("finite")),
        Value::Str(_) => Err(Error::Unported("Fraction(str)".into())),
        Value::Host(h) => match h.as_rational()? {
            Some(r) => Ok(r),
            None => wrong_type("argument should be a string or a Rational instance"),
        },
        _ => wrong_type("argument should be a string or a Rational instance"),
    }
}

/// `Fraction.limit_denominator(max_denominator)` (CPython 3.13 fractions.py) for an `int` `max_denominator`: the
/// closest Fraction to `x` with denominator at most `max_denominator`.
///
/// Algorithm notes: For any real number x, define a *best upper approximation* to x to be a rational number p/q
/// such that: (1) p/q >= x, and (2) if p/q > r/s >= x then s > q, for any rational r/s. Define *best lower
/// approximation* similarly. Then it can be proved that a rational number is a best upper or lower approximation
/// to x if, and only if, it is a convergent or semiconvergent of the (unique shortest) continued fraction
/// associated to x.
///
/// To find a best rational approximation with denominator <= M, we find the best upper and lower approximations
/// with denominator <= M and take whichever of these is closer to x. In the event of a tie, the bound with smaller
/// denominator is chosen. If both denominators are equal (which can happen only when max_denominator == 1 and
/// self is midway between two integers) the lower bound---i.e., the floor of self, is taken.
pub fn limit_denominator(x: &RBig, max_denominator: &IBig) -> Result<RBig> {
    if *max_denominator < IBig::ONE {
        return Err(Error::InvalidArgument(
            "max_denominator should be at least 1".into(),
        ));
    }
    let denominator = IBig::from(x.denominator().clone());
    if denominator <= *max_denominator {
        return Ok(x.clone());
    }
    let (mut p0, mut q0, mut p1, mut q1) = (IBig::ZERO, IBig::ONE, IBig::ONE, IBig::ZERO);
    let (mut n, mut d) = (x.numerator().clone(), denominator.clone());
    loop {
        let a = floor_div(&n, &d);
        let q2 = &q0 + &a * &q1;
        if q2 > *max_denominator {
            break;
        }
        let p2 = &p0 + &a * &p1;
        (p0, q0, p1, q1) = (p1, q1, p2, q2);
        let r = &n - &a * &d;
        (n, d) = (d, r);
    }
    let k = floor_div(&(max_denominator - &q0), &q1);
    // Determine which of the candidates (p0+k*p1)/(q0+k*q1) and p1/q1 is closer to self. The distance between them
    // is 1/(q1*(q0+k*q1)), while the distance from p1/q1 to self is d/(q1*self._denominator). So we need to compare
    // 2*(q0+k*q1) with self._denominator/d.
    let (p, q) = if IBig::from(2) * &d * (&q0 + &k * &q1) <= denominator {
        (p1, q1)
    } else {
        (&p0 + &k * &p1, &q0 + &k * &q1)
    };
    Ok(RBig::from_parts(p, q.unsigned_abs()))
}

/// Python's `a // b` for ints (`b != 0`): rounds towards negative infinity.
fn floor_div(a: &IBig, b: &IBig) -> IBig {
    use dashu_base::DivRemEuclid;
    // Euclidean division rounds down for a positive divisor; the divisors here are positive.
    debug_assert!(*b > IBig::ZERO);
    a.div_rem_euclid(b).0
}

#[cfg(test)]
mod tests {
    #[test]
    fn limit_denominator_matches_fractions_doctests() {
        use dashu_ratio::RBig;
        // Fraction('3.141592653589793').limit_denominator(10) / (100), Fraction(4321, 8765).limit_denominator(10000).
        let pi = RBig::from_parts(3141592653589793i64.into(), 1000000000000000u64.into());
        let r = |n: i64, d: u64| RBig::from_parts(n.into(), d.into());
        assert_eq!(super::limit_denominator(&pi, &10.into()).unwrap(), r(22, 7));
        assert_eq!(
            super::limit_denominator(&pi, &100.into()).unwrap(),
            r(311, 99)
        );
        assert_eq!(
            super::limit_denominator(&r(4321, 8765), &10000.into()).unwrap(),
            r(4321, 8765)
        );
        // Midway between two integers with max_denominator 1: the floor.
        assert_eq!(
            super::limit_denominator(&r(5, 2), &1.into()).unwrap(),
            r(2, 1)
        );
        assert_eq!(
            super::limit_denominator(&r(-5, 2), &1.into()).unwrap(),
            r(-3, 1)
        );
        assert!(super::limit_denominator(&r(1, 3), &0.into()).is_err());
    }

    use super::*;

    fn f(x: f64) -> Value {
        Value::Float(x)
    }

    #[test]
    fn sum_follows_cpython_fast_paths() {
        // Compensated for floats, uncompensated for ints in the float path (Python 3.13).
        let s = py_sum(
            [f(1e16), Value::from(1), Value::from(1)].map(Ok),
            Value::from(0),
        );
        assert_eq!(s, Ok(f(1e16)));
        let s = py_sum([f(1e16), f(1.0), f(1.0)].map(Ok), Value::from(0)).unwrap();
        assert_eq!(s.to_string(), "1.0000000000000002e+16");
        let s = py_sum(vec![Ok(f(0.1)); 10], Value::from(0));
        assert_eq!(s, Ok(f(1.0)));
        // An empty sum is the start; ints overflowing a C long stay exact.
        assert_eq!(py_sum([], Value::from(0)), Ok(Value::from(0)));
        let big = Value::from(i64::MAX);
        let s = py_sum([big.clone(), big].map(Ok), Value::from(0)).unwrap();
        assert_eq!(s.to_string(), "18446744073709551614");
    }

    #[test]
    fn slices_select_like_python() {
        let t = Value::tuple((0..5).map(Value::from).collect());
        let sl =
            |a: Value, b: Value, c: Value| getitem(&t, &Index::Slice(a, b, c)).unwrap().to_string();
        let n = Value::None;
        assert_eq!(sl(n.clone(), n.clone(), Value::from(-1)), "(4, 3, 2, 1, 0)");
        assert_eq!(
            sl(Value::from(1), Value::from(-1), Value::from(2)),
            "(1, 3)"
        );
        assert_eq!(sl(Value::from(10), n.clone(), n.clone()), "()");
        assert_eq!(
            getitem(&t, &Index::Item(Value::from(-1))).unwrap(),
            Value::from(4)
        );
        assert!(getitem(&t, &Index::Item(Value::from(5))).is_err());
        assert_eq!(
            getitem(&Value::str("héllo"), &Index::Item(Value::from(1))).unwrap(),
            Value::str("é")
        );
    }

    #[test]
    fn log_of_huge_int() {
        // As Python prints them: `math.log(2**2000, 2.0)`, `math.log(3**1000)`, `math.log(10**400, 10)`.
        let log = |x: IBig, base: Option<Value>| {
            math_log(&Value::Int(x), base.as_ref()).unwrap().to_string()
        };
        assert_eq!(log(IBig::ONE << 2000, Some(f(2.0))), "2000.0000000000002");
        assert_eq!(log(IBig::from(3u8).pow(1000), None), "1098.6122886681096");
        assert_eq!(
            log(IBig::from(10u8).pow(400), Some(Value::from(10))),
            "399.99999999999994"
        );
    }
}
