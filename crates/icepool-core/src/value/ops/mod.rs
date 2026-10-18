//! Python's operators on outcomes: what `operator.add(a, b)`, `round(a, n)`, `math.floor(a)`... do for the
//! types a `Value` holds. `Die.binary_operator` / `unary_operator` apply these outcome by outcome.
//!
//! The numeric tower follows CPython (`Objects/longobject.c`, `floatobject.c`) and `fractions.py`:
//! - `bool` is an `int` everywhere, except that `&`, `|`, `^` of two bools give a bool;
//! - `int` with `int` stays exact, except `/` (always a float) and `**` with a negative exponent (a float);
//! - anything with a `float` is done in floats, converting the other operand first (which may overflow);
//! - otherwise `Fraction` arithmetic, where `//` gives an `int` and `**` with an integral exponent stays exact.
//!
//! `str` and `tuple` concatenate with `+` and repeat with `* int`; comparisons follow `Value::py_lt`'s rules.
//!
//! This file dispatches on the operand types; the arithmetic of each is in `integer`, `float`, `fraction` and
//! `sequence` (`str`, `tuple`, `Vector`, `Symbols`).

use std::cmp::Ordering;

use dashu_base::UnsignedAbs;
use dashu_int::IBig;
use dashu_ratio::RBig;

use super::recursion::level;
use super::{float_repr, int_to_f64, ratio_to_f64, Value};
use crate::error::{wrong_type, Error, Result};

// One file per operand type; this one dispatches to them.
mod float;
mod fraction;
mod integer;
mod sequence;

use float::{float_op, round_float};
use fraction::frac_op;
use integer::{int_op, pow10, round_half_even};
use sequence::{collection_binary, sequence_op};

pub(crate) use integer::{float_to_int, floor_divmod};
pub use sequence::str_format_mapping;

/// Python's binary operators and order comparisons.
///
/// ```
/// use icepool_core::value::BinOp;
/// use icepool_core::Value;
///
/// // `7 // -2` rounds down, `1 / 3` is a float, `'a' < 1` raises TypeError.
/// assert_eq!(Value::from(7).binary(BinOp::FloorDiv, &Value::from(-2)).unwrap(), Value::from(-4));
/// assert_eq!(Value::from(1).binary(BinOp::TrueDiv, &Value::from(3)).unwrap().to_string(), "0.3333333333333333");
/// assert!(Value::str("a").binary(BinOp::Lt, &Value::from(1)).is_err());
/// assert_eq!(BinOp::FloorDiv.symbol(), "//");
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    /// `+`.
    Add,
    /// `-`.
    Sub,
    /// `*`.
    Mul,
    /// `/`.
    TrueDiv,
    /// `//`.
    FloorDiv,
    /// `%`.
    Mod,
    /// `**`.
    Pow,
    /// `<<`.
    LShift,
    /// `>>`.
    RShift,
    /// `&`.
    And,
    /// `|`.
    Or,
    /// `^`.
    Xor,
    /// `<`.
    Lt,
    /// `<=`.
    Le,
    /// `>`.
    Gt,
    /// `>=`.
    Ge,
}

impl BinOp {
    /// The operator as Python writes it (`+`, `//`, `<=`, ...).
    ///
    /// ```
    /// use icepool_core::value::BinOp;
    /// assert_eq!(BinOp::FloorDiv.symbol(), "//");
    /// ```
    pub fn symbol(self) -> &'static str {
        use BinOp::*;
        match self {
            Add => "+",
            Sub => "-",
            Mul => "*",
            TrueDiv => "/",
            FloorDiv => "//",
            Mod => "%",
            Pow => "**",
            LShift => "<<",
            RShift => ">>",
            And => "&",
            Or => "|",
            Xor => "^",
            Lt => "<",
            Le => "<=",
            Gt => ">",
            Ge => ">=",
        }
    }

    /// For a comparison, the reflected one Python falls back to when the left operand returns `NotImplemented`:
    /// `a < b` becomes `b > a`. Arithmetic operators reflect to `__radd__` etc., which keep the operand order.
    ///
    /// ```
    /// use icepool_core::value::BinOp;
    /// assert_eq!(BinOp::Lt.reflected_comparison(), Some(BinOp::Gt));
    /// assert_eq!(BinOp::Add.reflected_comparison(), None);
    /// ```
    pub fn reflected_comparison(self) -> Option<BinOp> {
        use BinOp::*;
        match self {
            Lt => Some(Gt),
            Le => Some(Ge),
            Gt => Some(Lt),
            Ge => Some(Le),
            _ => None,
        }
    }

    fn is_bitwise(self) -> bool {
        matches!(
            self,
            BinOp::LShift | BinOp::RShift | BinOp::And | BinOp::Or | BinOp::Xor
        )
    }
}

/// Python's unary operators and the `math` / built-in functions that take one number.
///
/// ```
/// use icepool_core::value::UnOp;
/// use icepool_core::Value;
///
/// // `~5` is `-6`; `math.ceil(2.5)` is the int `3`; `-'a'` raises TypeError.
/// assert_eq!(Value::from(5).unary(UnOp::Invert).unwrap(), Value::from(-6));
/// assert_eq!(Value::Float(2.5).unary(UnOp::Ceil).unwrap().to_string(), "3");
/// assert!(Value::str("a").unary(UnOp::Neg).is_err());
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    /// `-x`.
    Neg,
    /// `+x`.
    Pos,
    /// `~x`.
    Invert,
    /// `abs(x)`.
    Abs,
    /// `math.trunc(x)`.
    Trunc,
    /// `math.floor(x)`.
    Floor,
    /// `math.ceil(x)`.
    Ceil,
}

/// A number operand, with `bool` already turned into `int`.
pub(super) enum N {
    Int(IBig),
    Float(f64),
    Frac(RBig),
}

impl N {
    pub(super) fn of(v: &Value) -> Option<N> {
        Some(match v {
            Value::Bool(b) => N::Int(IBig::from(*b as u8)),
            Value::Int(i) => N::Int(i.clone()),
            Value::Float(f) => N::Float(*f),
            Value::Fraction(r) => N::Frac(r.clone()),
            _ => return None,
        })
    }

    pub(super) fn to_f64(&self) -> Result<f64> {
        match self {
            N::Int(i) => int_to_f64(i),
            N::Float(f) => Ok(*f),
            N::Frac(r) => ratio_to_f64(r.numerator(), &IBig::from(r.denominator().clone())),
        }
    }

    fn to_ratio(&self) -> RBig {
        match self {
            N::Int(i) => RBig::from(i.clone()),
            N::Frac(r) => r.clone(),
            N::Float(_) => unreachable!("floats are handled before Fraction arithmetic"),
        }
    }
}

/// Python refuses to build integers this large (or would run out of memory or time doing it): the port raises
/// instead of trying. (A `u64`: it does not fit a 32-bit `usize`, e.g. on wasm32.)
/// On wasm32 the whole instance has 4 GiB, and a product needs both operands and its result at once: 128 MiB ints.
const MAX_INT_BITS: u64 = if cfg!(target_pointer_width = "32") {
    1 << 30
} else {
    1 << 32
};
/// Repetition beyond this many elements raises (Python would raise `MemoryError` or take very long).
const MAX_REPEAT: usize = 1 << 28;

fn unsupported_operands<T>(op: BinOp, a: &Value, b: &Value) -> Result<T> {
    wrong_type(format!(
        "unsupported operand type(s) for {}: '{}' and '{}'",
        op.symbol(),
        a.type_name(),
        b.type_name()
    ))
}

fn zero_division<T>(msg: &str) -> Result<T> {
    Err(Error::DivisionByZero(msg.into()))
}

fn too_large<T>() -> Result<T> {
    Err(Error::Overflow("result too large".into()))
}

impl Value {
    /// Python's `operator.<op>(self, other)` on two outcomes.
    ///
    /// ```
    /// use icepool_core::value::BinOp;
    /// use icepool_core::{Error, Value};
    /// // `7 // 2` is `3`, `7 / 2` is `3.5`, `True & True` is `True`.
    /// assert_eq!(Value::from(7).binary(BinOp::FloorDiv, &Value::from(2)).unwrap().to_string(), "3");
    /// assert_eq!(Value::from(7).binary(BinOp::TrueDiv, &Value::from(2)).unwrap().to_string(), "3.5");
    /// assert_eq!(Value::Bool(true).binary(BinOp::And, &Value::Bool(true)).unwrap().to_string(), "True");
    /// // `1 / 0` raises ZeroDivisionError.
    /// let err = Value::from(1).binary(BinOp::TrueDiv, &Value::from(0)).unwrap_err();
    /// assert!(matches!(err, Error::DivisionByZero(_)));
    /// ```
    pub fn binary(&self, op: BinOp, other: &Value) -> Result<Value> {
        use BinOp::*;
        if let Some(h) = crate::host::host_of(self, other) {
            return h.binary(op, self, other);
        }
        if self.is_collection() || other.is_collection() {
            return collection_binary(op, self, other);
        }
        if matches!(op, Lt | Le | Gt | Ge) {
            // Python's tuple comparison returns the comparison of the first elements that differ (by `==`) as is,
            // which for vectors is not a bool; equal prefixes compare the lengths.
            if let (Value::Tuple(x), Value::Tuple(y)) = (self, other) {
                let _level = level()?;
                return match Value::first_difference(x, y)? {
                    Some((p, q)) => p.binary(op, q),
                    None => compare(
                        op,
                        &Value::from(x.len() as i64),
                        &Value::from(y.len() as i64),
                    )
                    .map(Value::Bool),
                };
            }
            return compare(op, self, other).map(Value::Bool);
        }
        if matches!(self, Value::Complex(..)) || matches!(other, Value::Complex(..)) {
            // With a str or tuple (or for `//`, `%`, bitwise operators), the TypeError of any other type.
            return super::complex::binary(op, self, other)
                .unwrap_or_else(|| sequence_op(op, self, other));
        }
        // bool's own `&`, `|`, `^` keep the bool type; everything else treats it as an int.
        if let (Value::Bool(a), Value::Bool(b)) = (self, other) {
            match op {
                And => return Ok(Value::Bool(a & b)),
                Or => return Ok(Value::Bool(a | b)),
                Xor => return Ok(Value::Bool(a ^ b)),
                _ => {}
            }
        }
        match (N::of(self), N::of(other)) {
            (Some(N::Int(a)), Some(N::Int(b))) => int_op(op, &a, &b),
            (Some(_), Some(_)) if op.is_bitwise() => unsupported_operands(op, self, other),
            (Some(a), Some(b)) if matches!(a, N::Float(_)) || matches!(b, N::Float(_)) => {
                float_op(op, a.to_f64()?, b.to_f64()?)
            }
            (Some(a), Some(b)) => frac_op(op, a, b),
            _ => sequence_op(op, self, other),
        }
    }

    /// Python's `-x`, `+x`, `~x`, `abs(x)`, `math.trunc(x)`, `math.floor(x)`, `math.ceil(x)`.
    ///
    /// ```
    /// use icepool_core::value::UnOp;
    /// use icepool_core::Value;
    /// // `-True` is `-1` and `~True` is `-2`: ints; `math.floor(-3.5)` is `-4`.
    /// assert_eq!(Value::Bool(true).unary(UnOp::Neg).unwrap().to_string(), "-1");
    /// assert_eq!(Value::Bool(true).unary(UnOp::Invert).unwrap().to_string(), "-2");
    /// assert_eq!(Value::Float(-3.5).unary(UnOp::Floor).unwrap().to_string(), "-4");
    /// assert!(Value::str("a").unary(UnOp::Neg).is_err()); // TypeError
    /// ```
    pub fn unary(&self, op: UnOp) -> Result<Value> {
        use UnOp::*;
        let bad = || {
            let what = match op {
                Neg => "bad operand type for unary -",
                Pos => "bad operand type for unary +",
                Invert => "bad operand type for unary ~",
                Abs => "bad operand type for abs()",
                Trunc => "type doesn't define __trunc__ method",
                Floor | Ceil => "must be real number",
            };
            wrong_type(format!("{what}: '{}'", self.type_name()))
        };
        match self {
            Value::Host(h) => return h.unary(op),
            Value::Complex(re, im) => {
                return match op {
                    Neg => Ok(Value::Complex(-re, -im)),
                    Pos => Ok(self.clone()),
                    Abs => super::complex::abs(*re, *im),
                    Invert | Trunc | Floor | Ceil => bad(),
                }
            }
            Value::Vector(v) => {
                let _level = level()?;
                return v.unary(op);
            }
            Value::Symbols(s) => return s.unary(op).unwrap_or_else(bad),
            _ => {}
        }
        let Some(n) = N::of(self) else { return bad() };
        Ok(match (op, n) {
            // `bool` gives an `int` for all of these (`-True == -1`, `~True == -2`, `math.floor(True) == 1`).
            (Neg, N::Int(i)) => Value::Int(-i),
            (Pos | Trunc | Floor | Ceil, N::Int(i)) => Value::Int(i),
            (Invert, N::Int(i)) => Value::Int(-i - IBig::ONE),
            (Abs, N::Int(i)) => Value::Int(IBig::from(i.unsigned_abs())),
            (Neg, N::Float(f)) => Value::Float(-f),
            (Pos, N::Float(f)) => Value::Float(f),
            (Abs, N::Float(f)) => Value::Float(f.abs()),
            (Trunc, N::Float(f)) => Value::Int(float_to_int(f.trunc())?),
            (Floor, N::Float(f)) => Value::Int(float_to_int(f.floor())?),
            (Ceil, N::Float(f)) => Value::Int(float_to_int(f.ceil())?),
            (Neg, N::Frac(r)) => Value::Fraction(-r),
            (Pos, N::Frac(r)) => Value::Fraction(r),
            (Abs, N::Frac(r)) => Value::Fraction(RBig::from_parts(
                IBig::from(r.numerator().unsigned_abs()),
                r.denominator().clone(),
            )),
            // `Fraction.__trunc__`, `__floor__`, `__ceil__` are integer divisions of numerator by denominator.
            (Trunc, N::Frac(r)) => Value::Int(r.trunc()),
            (Floor, N::Frac(r)) => Value::Int(r.floor()),
            (Ceil, N::Frac(r)) => Value::Int(r.ceil()),
            (Invert, _) => return bad(),
        })
    }

    /// Python's `round(self, ndigits)`; `ndigits` is `None` or an int.
    ///
    /// ```
    /// use icepool_core::Value;
    /// // `round(2.5)` is `2` (half to even), `round(2.675, 2)` is `2.67` (the float is just below 2.675).
    /// assert_eq!(Value::Float(2.5).round(&Value::None).unwrap().to_string(), "2");
    /// assert_eq!(Value::Float(2.675).round(&Value::from(2)).unwrap().to_string(), "2.67");
    /// assert_eq!(Value::from(1234).round(&Value::from(-2)).unwrap().to_string(), "1200");
    /// ```
    pub fn round(&self, ndigits: &Value) -> Result<Value> {
        let nd = match ndigits {
            Value::None => None,
            Value::Int(i) => Some(i.clone()),
            Value::Bool(b) => Some(IBig::from(*b as u8)),
            other => {
                return wrong_type(format!(
                    "'{}' object cannot be interpreted as an integer",
                    other.type_name()
                ))
            }
        };
        // `Vector.round(ndigits)`: `round(x, ndigits)` elementwise. Python calls `__round__` (which is `round`), then
        // `unary_operator`.
        if let Value::Vector(v) = self {
            let _level = level()?;
            let _frames = super::recursion::frames(2)?;
            return v.map(|x| x.round(ndigits));
        }
        let Some(n) = N::of(self) else {
            return wrong_type(format!(
                "type {} doesn't define __round__ method",
                self.type_name()
            ));
        };
        Ok(match (n, nd) {
            (N::Int(i), None) => Value::Int(i),
            // `int.__round__`: unchanged for ndigits >= 0, else `self - r` where `(q, r)` is `divmod` rounded
            // half to even (`_PyLong_DivmodNear`), i.e. the nearest multiple of 10**-ndigits, ties to even.
            (N::Int(i), Some(nd)) if nd >= IBig::ZERO => Value::Int(i),
            (N::Int(i), Some(nd)) => {
                let pow10 = pow10(&nd)?;
                Value::Int(round_half_even(&i, &pow10) * pow10)
            }
            // `float.__round__()`: `round()` (half away from zero) corrected to half-even, then `int`.
            (N::Float(f), None) => Value::Int(float_to_int(f.round_ties_even())?),
            (N::Float(f), Some(nd)) => Value::Float(round_float(f, &nd)?),
            (N::Frac(r), None) => Value::Int(round_half_even(
                r.numerator(),
                &IBig::from(r.denominator().clone()),
            )),
            // `Fraction.__round__`: `Fraction(round(self * shift), shift)` for ndigits > 0, else
            // `Fraction(round(self / shift) * shift)`, with `shift = 10**abs(ndigits)`.
            (N::Frac(r), Some(nd)) => {
                let shift = pow10(&nd)?;
                let (num, den) = (r.numerator(), IBig::from(r.denominator().clone()));
                if nd > IBig::ZERO {
                    Value::Fraction(RBig::from_parts_signed(
                        round_half_even(&(num * &shift), &den),
                        shift,
                    ))
                } else {
                    Value::Fraction(RBig::from(round_half_even(num, &(den * &shift)) * shift))
                }
            }
        })
    }
}

/// Python's rich comparison `<`, `<=`, `>`, `>=` between two outcomes.
pub(super) fn compare(op: BinOp, a: &Value, b: &Value) -> Result<bool> {
    let by_ord = |o: Ordering| match op {
        BinOp::Lt => o == Ordering::Less,
        BinOp::Le => o != Ordering::Greater,
        BinOp::Gt => o == Ordering::Greater,
        BinOp::Ge => o != Ordering::Less,
        _ => unreachable!("not a comparison"),
    };
    if let Some(h) = crate::host::host_of(a, b) {
        return h.compare(op, a, b);
    }
    // A vector's comparison is a vector, whose truth value may raise.
    if a.is_collection() || b.is_collection() {
        return collection_binary(op, a, b)?.truthy();
    }
    match (a, b) {
        (Value::Str(x), Value::Str(y)) => Ok(by_ord(x.cmp(y))),
        // Sequence comparison: the first position where the elements differ (by `==`) decides with the same
        // operator; if one is a prefix of the other, the lengths decide.
        (Value::Tuple(x), Value::Tuple(y)) => {
            let _level = level()?;
            match Value::first_difference(x, y)? {
                Some((p, q)) => compare(op, p, q),
                None => Ok(by_ord(x.len().cmp(&y.len()))),
            }
        }
        _ => match (a.as_num(), b.as_num()) {
            // NaN compares false with everything.
            (Some(x), Some(y)) => Ok(x.partial_cmp(&y).is_some_and(by_ord)),
            _ => Err(Error::Incomparable {
                op: op.symbol(),
                left: a.type_name(),
                right: b.type_name(),
            }),
        },
    }
}

impl Value {
    /// Python's `operator.eq(self, other)` (`operator.ne` if `ne`): a bool, except that a `Vector` compares
    /// elementwise. `Symbols` converts the other operand (so `Symbols('ab') == 'ab'`), and when it cannot, Python
    /// falls back to identity: unequal.
    ///
    /// ```
    /// use icepool_core::value::{Symbols, Vector};
    /// use icepool_core::Value;
    /// // `Vector((1, 2)) != 1` is `Vector((False, True))`; `Symbols('ab') == 'ab'` is `True`.
    /// let v = Value::Vector(Vector::new(vec![Value::from(1), Value::from(2)]));
    /// assert_eq!(v.eq_op(&Value::from(1), true).unwrap().to_string(), "Vector((False, True))");
    /// let s = Value::Symbols(Symbols::parse("ab").unwrap());
    /// assert_eq!(s.eq_op(&Value::str("ab"), false).unwrap().to_string(), "True");
    /// ```
    pub fn eq_op(&self, other: &Value, ne: bool) -> Result<Value> {
        use super::VecOp;
        if crate::host::host_of(self, other).is_some()
            || !(self.is_collection() || other.is_collection())
        {
            return Ok(Value::Bool((self == other) != ne));
        }
        let _level = level()?;
        for x in [self, other] {
            if let Value::Vector(v) = x {
                v.data()?;
            }
        }
        if let Value::Vector(v) = self {
            return v.binary(VecOp::Eq(ne), other);
        }
        if let Value::Symbols(s) = self {
            if let Some(r) = s.eq_op(other, ne) {
                return r.map(Value::Bool);
            }
        }
        match other {
            Value::Vector(v) => v.binary(VecOp::Eq(ne), self),
            Value::Symbols(s) => match s.eq_op(self, ne) {
                Some(r) => r.map(Value::Bool),
                None => Ok(Value::Bool(ne)),
            },
            _ => Ok(Value::Bool(ne)),
        }
    }
}

/// Python's `repr(complex)`: `(re+imj)`, or just `imj` when the real part is +0.0; floats without a trailing `.0`.
pub(super) fn complex_repr(re: f64, im: f64) -> String {
    let short = |x: f64| {
        let s = float_repr(x);
        match s.strip_suffix(".0") {
            Some(t) => t.to_string(),
            None => s,
        }
    };
    if re == 0.0 && re.is_sign_positive() {
        format!("{}j", short(im))
    } else {
        let im_s = short(im);
        let sign = if im_s.starts_with('-') { "" } else { "+" };
        format!("({}{sign}{im_s}j)", short(re))
    }
}

#[cfg(test)]
mod tests {
    use super::float::float_divmod;
    use super::*;

    fn i(x: i64) -> Value {
        Value::from(x)
    }

    #[test]
    fn python_integer_semantics() {
        assert_eq!(i(-7).binary(BinOp::FloorDiv, &i(2)), Ok(i(-4)));
        assert_eq!(i(-7).binary(BinOp::Mod, &i(2)), Ok(i(1)));
        assert_eq!(i(7).binary(BinOp::Mod, &i(-2)), Ok(i(-1)));
        assert_eq!(i(-6).binary(BinOp::And, &i(3)), Ok(i(2)));
        assert_eq!(i(-6).binary(BinOp::Or, &i(3)), Ok(i(-5)));
        assert_eq!(i(-6).binary(BinOp::Xor, &i(3)), Ok(i(-7)));
        assert_eq!(i(-7).binary(BinOp::RShift, &i(1)), Ok(i(-4)));
        assert_eq!(i(2).binary(BinOp::Pow, &i(-1)), Ok(Value::Float(0.5)));
        assert!(i(1).binary(BinOp::TrueDiv, &i(0)).is_err());
        assert_eq!(
            Value::Bool(true).binary(BinOp::And, &Value::Bool(true)),
            Ok(Value::Bool(true))
        );
        assert!(matches!(
            Value::Bool(true).binary(BinOp::Add, &Value::Bool(true)),
            Ok(Value::Int(_))
        ));
    }

    #[test]
    fn float_floor_division_and_rounding() {
        assert_eq!(float_divmod(-7.5, 2.0), (-4.0, 0.5));
        assert_eq!(float_divmod(7.5, -2.0), (-4.0, -0.5));
        assert_eq!(round_float(2.675, &IBig::from(2)), Ok(2.67));
        assert_eq!(round_float(0.125, &IBig::from(2)), Ok(0.12));
        assert_eq!(round_float(2.5, &IBig::ZERO), Ok(2.0));
        assert!(round_float(-0.4, &IBig::ZERO).unwrap().is_sign_negative());
        assert_eq!(complex_repr(1.0, -2.5), "(1-2.5j)");
        assert_eq!(complex_repr(0.0, 2.0), "2j");
    }
}
