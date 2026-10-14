//! Python's operators on outcomes: what `operator.add(a, b)`, `round(a, n)`, `math.floor(a)`... do for the
//! types a `Value` holds. `Die.binary_operator` / `unary_operator` apply these outcome by outcome.
//!
//! The numeric tower follows CPython (`Objects/longobject.c`, `floatobject.c`) and `fractions.py`:
//! - `bool` is an `int` everywhere, except that `&`, `|`, `^` of two bools give a bool;
//! - `int` with `int` stays exact, except `/` (always a float) and `**` with a negative exponent (a float);
//! - anything with a `float` is done in floats, converting the other operand first (which may overflow);
//! - otherwise `Fraction` arithmetic, where `//` gives an `int` and `**` with an integral exponent stays exact.
//!
//! `str` and `tuple` concatenate with `+` and repeat with `* int`; comparisons follow `Outcome::py_lt`'s rules.

use std::cmp::Ordering;

use dashu_base::{BitTest, DivRem, UnsignedAbs};
use dashu_int::IBig;
use dashu_ratio::RBig;

use super::{float_repr, int_to_f64, pow, ratio_to_f64, Value};
use crate::error::{type_error, Error, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    TrueDiv,
    FloorDiv,
    Mod,
    Pow,
    LShift,
    RShift,
    And,
    Or,
    Xor,
    Lt,
    Le,
    Gt,
    Ge,
}

impl BinOp {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Pos,
    Invert,
    Abs,
    Trunc,
    Floor,
    Ceil,
}

/// A number operand, with `bool` already turned into `int`.
enum N {
    Int(IBig),
    Float(f64),
    Frac(RBig),
}

impl N {
    fn of(v: &Value) -> Option<N> {
        Some(match v {
            Value::Bool(b) => N::Int(IBig::from(*b as u8)),
            Value::Int(i) => N::Int(i.clone()),
            Value::Float(f) => N::Float(*f),
            Value::Fraction(r) => N::Frac(r.clone()),
            _ => return None,
        })
    }

    fn to_f64(&self) -> Result<f64> {
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
const MAX_INT_BITS: u64 = 1 << 32;
/// Repetition beyond this many elements raises (Python would raise `MemoryError` or take very long).
const MAX_REPEAT: usize = 1 << 28;

fn unsupported_operands<T>(op: BinOp, a: &Value, b: &Value) -> Result<T> {
    type_error(format!(
        "unsupported operand type(s) for {}: '{}' and '{}'",
        op.symbol(),
        a.type_name(),
        b.type_name()
    ))
}

fn zero_division<T>(msg: &str) -> Result<T> {
    Err(Error::ZeroDivision(msg.into()))
}

fn too_large<T>() -> Result<T> {
    Err(Error::Overflow("result too large".into()))
}

fn is_odd_integer(x: f64) -> bool {
    (x.abs() % 2.0) == 1.0
}

impl Value {
    /// Python's `operator.<op>(self, other)` on two outcomes.
    pub fn binary(&self, op: BinOp, other: &Value) -> Result<Value> {
        use BinOp::*;
        if let Some(h) = super::host::host_of(self, other) {
            return h.binary(op, self, other);
        }
        if self.is_collection() || other.is_collection() {
            return collection_binary(op, self, other);
        }
        if matches!(op, Lt | Le | Gt | Ge) {
            return compare(op, self, other).map(Value::Bool);
        }
        if matches!(self, Value::Complex(..)) || matches!(other, Value::Complex(..)) {
            return Err(Error::Unsupported("arithmetic on complex numbers".into()));
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
            type_error(format!("{what}: '{}'", self.type_name()))
        };
        match self {
            Value::Host(h) => return h.unary(op),
            Value::Complex(..) => {
                return Err(Error::Unsupported("operators on complex numbers".into()))
            }
            Value::Vector(v) => return v.unary(op),
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
    pub fn round(&self, ndigits: &Value) -> Result<Value> {
        let nd = match ndigits {
            Value::None => None,
            Value::Int(i) => Some(i.clone()),
            Value::Bool(b) => Some(IBig::from(*b as u8)),
            other => {
                return type_error(format!(
                    "'{}' object cannot be interpreted as an integer",
                    other.type_name()
                ))
            }
        };
        if let Value::Complex(..) = self {
            return Err(Error::Unsupported("round of a complex number".into()));
        }
        // `Vector.round(ndigits)`: `round(x, ndigits)` elementwise.
        if let Value::Vector(v) = self {
            return v.map(|x| x.round(ndigits));
        }
        let Some(n) = N::of(self) else {
            return type_error(format!(
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

/// `10 ** abs(n)`.
fn pow10(n: &IBig) -> Result<IBig> {
    let k =
        usize::try_from(n.unsigned_abs()).map_err(|_| Error::Overflow("too many digits".into()))?;
    if k as u64 > MAX_INT_BITS / 4 {
        return too_large();
    }
    Ok(IBig::from(10u8).pow(k))
}

/// `n / d` rounded to the nearest integer, ties to even (`d > 0`).
fn round_half_even(n: &IBig, d: &IBig) -> IBig {
    let (q, r) = floor_divmod(n, d);
    let twice: IBig = r * IBig::from(2u8);
    match twice.cmp(d) {
        Ordering::Less => q,
        Ordering::Greater => q + IBig::ONE,
        Ordering::Equal if (&q % IBig::from(2u8)) == IBig::ZERO => q,
        Ordering::Equal => q + IBig::ONE,
    }
}

/// Python's `divmod` on ints: the quotient rounds toward minus infinity, the remainder takes the divisor's sign.
fn floor_divmod(a: &IBig, b: &IBig) -> (IBig, IBig) {
    let (mut q, mut r) = a.clone().div_rem(b.clone());
    if r != IBig::ZERO && ((r < IBig::ZERO) != (*b < IBig::ZERO)) {
        q -= IBig::ONE;
        r += b;
    }
    (q, r)
}

/// `int(f)` for an integral float: `OverflowError` for infinities, `ValueError` for NaN.
pub(crate) fn float_to_int(f: f64) -> Result<IBig> {
    if f.is_nan() {
        return Err(Error::Value("cannot convert float NaN to integer".into()));
    }
    if f.is_infinite() {
        return Err(Error::Overflow(
            "cannot convert float infinity to integer".into(),
        ));
    }
    Ok(RBig::try_from(f).expect("finite").trunc())
}

/// `round(x, ndigits)` for a float (CPython `float___round___impl` and `double_round`): the exact binary value is
/// rounded to `ndigits` decimals, ties to even (dtoa mode 3), and the decimal string is read back as a float.
fn round_float(x: f64, ndigits: &IBig) -> Result<f64> {
    // NDIGITS_MAX and NDIGITS_MIN: beyond them the result is `x`, or a zero, outright. (ndigits saturates.)
    let nd = i64::try_from(ndigits.clone()).unwrap_or(if *ndigits > IBig::ZERO {
        i64::MAX
    } else {
        i64::MIN
    });
    // Nans and infinities round to themselves.
    if !x.is_finite() || nd > 323 {
        return Ok(x);
    }
    if nd < -308 {
        return Ok(0.0 * x);
    }
    let exact = RBig::try_from(x).expect("finite");
    let pow10 = IBig::from(10u8).pow(nd.unsigned_abs() as usize);
    let (num, den) = (exact.numerator(), IBig::from(exact.denominator().clone()));
    let digits = if nd >= 0 {
        round_half_even(&(num * &pow10), &den)
    } else {
        round_half_even(num, &(den * &pow10))
    };
    let sign = if x.is_sign_negative() { "-" } else { "" };
    let rounded: f64 = format!("{sign}{}e{}", digits.unsigned_abs(), -nd)
        .parse()
        .expect("a valid float literal");
    if rounded.is_infinite() {
        return Err(Error::Overflow(
            "rounded value too large to represent".into(),
        ));
    }
    Ok(rounded)
}

/// `a ** b` for ints with `b >= 0`.
fn int_pow(a: &IBig, b: &IBig) -> Result<IBig> {
    // 0, 1 and -1 stay small whatever the exponent.
    if a.unsigned_abs() <= dashu_int::UBig::ONE {
        let odd = (b % IBig::from(2u8)) != IBig::ZERO;
        return Ok(if *a == IBig::ZERO && *b != IBig::ZERO {
            IBig::ZERO
        } else if *a < IBig::ZERO && odd {
            -IBig::ONE
        } else {
            IBig::ONE
        });
    }
    match usize::try_from(b.clone()) {
        Ok(e) if (e as u64).saturating_mul(a.unsigned_abs().bit_len() as u64) <= MAX_INT_BITS => {
            Ok(a.pow(e))
        }
        _ => too_large(),
    }
}

fn int_op(op: BinOp, a: &IBig, b: &IBig) -> Result<Value> {
    use BinOp::*;
    let zero = IBig::ZERO;
    Ok(Value::Int(match op {
        Add => a + b,
        Sub => a - b,
        Mul => a * b,
        TrueDiv => return Ok(Value::Float(ratio_to_f64(a, b)?)),
        FloorDiv | Mod => {
            if *b == zero {
                return zero_division("integer division or modulo by zero");
            }
            let (q, r) = floor_divmod(a, b);
            if op == FloorDiv {
                q
            } else {
                r
            }
        }
        // A negative exponent makes it a float operation (`long_pow` hands it to `float_pow`).
        Pow if *b < zero => return float_pow(int_to_f64(a)?, int_to_f64(b)?),
        Pow => int_pow(a, b)?,
        LShift | RShift if *b < zero => return Err(Error::Value("negative shift count".into())),
        LShift if *a == zero => zero,
        LShift => match usize::try_from(b.clone()) {
            Ok(s)
                if (s as u64).saturating_add(a.unsigned_abs().bit_len() as u64) <= MAX_INT_BITS =>
            {
                a << s
            }
            _ => return too_large(),
        },
        // Shifting right by more than the length leaves 0, or -1 for a negative number (rounds toward -inf).
        RShift => match usize::try_from(b.clone()) {
            Ok(s) if s <= a.unsigned_abs().bit_len() => a >> s,
            _ if *a < zero => -IBig::ONE,
            _ => zero,
        },
        And => a & b,
        Or => a | b,
        Xor => a ^ b,
        Lt | Le | Gt | Ge => unreachable!("comparisons are handled by compare"),
    }))
}

/// Float arithmetic, as CPython's `float_add` ... `float_pow`.
fn float_op(op: BinOp, a: f64, b: f64) -> Result<Value> {
    use BinOp::*;
    Ok(Value::Float(match op {
        Add => a + b,
        Sub => a - b,
        Mul => a * b,
        TrueDiv if b == 0.0 => return zero_division("float division by zero"),
        TrueDiv => a / b,
        FloorDiv | Mod if b == 0.0 => return zero_division("float floor division by zero"),
        FloorDiv => float_divmod(a, b).0,
        Mod => float_divmod(a, b).1,
        Pow => return float_pow(a, b),
        _ => unreachable!("bitwise operators and comparisons never reach float_op"),
    }))
}

/// CPython's `_float_div_mod`: `fmod`, with the remainder moved to the divisor's sign and the quotient snapped to
/// the nearest integer.
fn float_divmod(vx: f64, wx: f64) -> (f64, f64) {
    let mut m = vx % wx;
    // fmod is exact, so vx - m is an exact multiple of wx (up to rounding of the division).
    let mut div = (vx - m) / wx;
    if m != 0.0 {
        // Ensure the remainder has the same sign as the denominator.
        if (wx < 0.0) != (m < 0.0) {
            m += wx;
            div -= 1.0;
        }
    } else {
        // The remainder is zero, and in the presence of signed zeroes fmod returns different results across
        // platforms; ensure it has the same sign as the denominator.
        m = 0.0f64.copysign(wx);
    }
    let floordiv = if div != 0.0 {
        // Snap the quotient to the nearest integral value.
        let mut f = div.floor();
        if div - f > 0.5 {
            f += 1.0;
        }
        f
    } else {
        // div is zero: get the correct sign.
        0.0f64.copysign(vx / wx)
    };
    (floordiv, m)
}

/// CPython's `float_pow`, which sorts out the special cases itself instead of relying on the platform `pow()`.
fn float_pow(iv: f64, iw: f64) -> Result<Value> {
    let float = |x: f64| Ok(Value::Float(x));
    // v**0 is 1, even 0**0.
    if iw == 0.0 {
        return float(1.0);
    }
    // nan**w = nan, unless w == 0.
    if iv.is_nan() {
        return float(iv);
    }
    // v**nan = nan, unless v == 1; 1**nan = 1.
    if iw.is_nan() {
        return float(if iv == 1.0 { 1.0 } else { iw });
    }
    if iw.is_infinite() {
        // v**inf is: 0.0 if abs(v) < 1; 1.0 if abs(v) == 1; inf if abs(v) > 1 (including case where v infinite).
        // v**-inf is: inf if abs(v) < 1; 1.0 if abs(v) == 1; 0.0 if abs(v) > 1 (including case where v infinite).
        let av = iv.abs();
        return float(if av == 1.0 {
            1.0
        } else if (iw > 0.0) == (av > 1.0) {
            iw.abs()
        } else {
            0.0
        });
    }
    if iv.is_infinite() {
        // (+-inf)**w is: inf for w positive, 0 for w negative; in both cases, we need to add the appropriate sign
        // if w is an odd integer.
        let odd = is_odd_integer(iw);
        return float(if iw > 0.0 {
            if odd {
                iv
            } else {
                iv.abs()
            }
        } else if odd {
            0.0f64.copysign(iv)
        } else {
            0.0
        });
    }
    if iv == 0.0 {
        // 0**w is: 0 for w positive, 1 for w zero (already dealt with above), and an error if w is negative.
        if iw < 0.0 {
            return zero_division("zero to a negative power");
        }
        // Use correct sign if iw is odd.
        return float(if is_odd_integer(iw) { iv } else { 0.0 });
    }
    let mut base = iv;
    let mut negate = false;
    if iv < 0.0 {
        // Negative numbers raised to fractional powers become complex.
        if iw != iw.floor() {
            return complex_pow(iv, iw);
        }
        // iw is an exact integer, albeit perhaps a very large one. Replace iv by its absolute value and remember
        // to negate the pow result if iw is odd.
        base = -iv;
        negate = is_odd_integer(iw);
    }
    // 1**w is 1, even 1**inf and 1**nan; (-1) ** large_integer also ends up here.
    if base == 1.0 {
        return float(if negate { -1.0 } else { 1.0 });
    }
    // Now iv and iw are finite, iw is nonzero, and iv is positive and not equal to 1.0. We finally allow the
    // platform pow to step in and do the rest. Overflow raises; underflow to 0 is fine.
    let ix = pow::pow(base, iw);
    if ix.is_infinite() {
        return Err(Error::Overflow("Numerical result out of range".into()));
    }
    float(if negate { -ix } else { ix })
}

/// `complex(v) ** complex(w)` for a negative real `v` and a non-integral real `w` (CPython `_Py_c_pow`). The
/// result is not orderable, so a die that keeps it (with a nonzero quantity) raises when it sorts its outcomes.
fn complex_pow(v: f64, w: f64) -> Result<Value> {
    let vabs = v.hypot(0.0);
    let len = pow::pow(vabs, w);
    let at = 0.0f64.atan2(v);
    let phase = at * w;
    let (re, im) = (len * phase.cos(), len * phase.sin());
    if re.is_infinite() || im.is_infinite() {
        return Err(Error::Overflow("complex exponentiation".into()));
    }
    Ok(Value::Complex(re, im))
}

/// Fraction arithmetic (`fractions.py`), for an `int` or `Fraction` with a `Fraction`.
fn frac_op(op: BinOp, a: N, b: N) -> Result<Value> {
    use BinOp::*;
    if op == Pow {
        return frac_pow(a, b);
    }
    let (x, y) = (a.to_ratio(), b.to_ratio());
    // Numerators and denominators as `fractions.py` names them.
    let (na, da) = (x.numerator(), IBig::from(x.denominator().clone()));
    let (nb, db) = (y.numerator(), IBig::from(y.denominator().clone()));
    Ok(match op {
        Add => Value::Fraction(x + y),
        Sub => Value::Fraction(x - y),
        Mul => Value::Fraction(x * y),
        TrueDiv if *nb == IBig::ZERO => return zero_division("Fraction(n, 0)"),
        TrueDiv => Value::Fraction(x / y),
        FloorDiv | Mod if *nb == IBig::ZERO => {
            return zero_division("integer division or modulo by zero")
        }
        // `(a.numerator * b.denominator) // (a.denominator * b.numerator)`: an int.
        FloorDiv => Value::Int(floor_divmod(&(na * &db), &(&da * nb)).0),
        // `Fraction((a.numerator * db) % (b.numerator * da), da * db)`.
        Mod => Value::Fraction(RBig::from_parts_signed(
            floor_divmod(&(na * &db), &(nb * &da)).1,
            da * db,
        )),
        _ => unreachable!("bitwise operators and comparisons never reach frac_op"),
    })
}

/// `**` where an operand is a `Fraction` (and none is a float): `Fraction.__pow__` and `__rpow__`.
fn frac_pow(a: N, b: N) -> Result<Value> {
    // `int ** Fraction`: "If a is an int, keep it that way if possible."
    if let (N::Int(a), N::Frac(b)) = (&a, &b) {
        if b.denominator().is_one() && *b.numerator() >= IBig::ZERO {
            return Ok(Value::Int(int_pow(a, b.numerator())?));
        }
    }
    let (x, y) = (a.to_ratio(), b.to_ratio());
    if !y.denominator().is_one() {
        // A fractional power will generally produce an irrational number.
        return float_pow(N::Frac(x).to_f64()?, N::Frac(y).to_f64()?);
    }
    let power = y.numerator();
    let (n, d) = (x.numerator(), IBig::from(x.denominator().clone()));
    let k = IBig::from(power.unsigned_abs());
    Ok(Value::Fraction(if *power >= IBig::ZERO {
        RBig::from_parts_signed(int_pow(n, &k)?, int_pow(&d, &k)?)
    } else if *n == IBig::ZERO {
        return zero_division("Fraction(n, 0)");
    } else {
        // The sign moves to the numerator.
        RBig::from_parts_signed(int_pow(&d, &k)?, int_pow(n, &k)?)
    }))
}

/// `+`, `*`, `%` and friends where an operand is not a number.
fn sequence_op(op: BinOp, a: &Value, b: &Value) -> Result<Value> {
    use Value::*;
    match (op, a, b) {
        (BinOp::Add, Str(x), Str(y)) => Ok(Value::str(&format!("{x}{y}"))),
        (BinOp::Add, Tuple(x), Tuple(y)) => {
            Ok(Value::tuple(x.iter().chain(y.iter()).cloned().collect()))
        }
        (BinOp::Mul, Str(_) | Tuple(_), Int(_) | Bool(_)) => repeat(a, b),
        (BinOp::Mul, Int(_) | Bool(_), Str(_) | Tuple(_)) => repeat(b, a),
        (BinOp::Mod, Str(s), _) => str_format(s, b),
        _ => unsupported_operands(op, a, b),
    }
}

/// `seq * n`: `n` must fit in an index-sized integer; `n <= 0` gives an empty sequence.
fn repeat(seq: &Value, n: &Value) -> Result<Value> {
    let n = match n {
        Value::Bool(b) => *b as i64,
        Value::Int(i) => i64::try_from(i.clone())
            .map_err(|_| Error::Overflow("cannot fit 'int' into an index-sized integer".into()))?,
        _ => unreachable!("only called with an int count"),
    };
    let n = usize::try_from(n).unwrap_or(0);
    let len = match seq {
        Value::Str(s) => s.chars().count(),
        Value::Tuple(t) => t.len(),
        _ => unreachable!("only called with a str or tuple"),
    };
    if len.saturating_mul(n) > MAX_REPEAT {
        return too_large();
    }
    Ok(match seq {
        Value::Str(s) => Value::str(&s.repeat(n)),
        Value::Tuple(t) => Value::tuple(t.iter().cycle().take(len * n).cloned().collect()),
        _ => unreachable!(),
    })
}

/// `str % args` (printf-style formatting). Without conversion specifiers it only succeeds for an empty tuple of
/// arguments: anything else is "not all arguments converted". Specifiers themselves are not ported yet.
fn str_format(s: &str, args: &Value) -> Result<Value> {
    if s.contains('%') {
        return Err(Error::Unsupported(
            "str % formatting with conversion specifiers".into(),
        ));
    }
    match args {
        Value::Tuple(t) if t.is_empty() => Ok(Value::str(s)),
        _ => type_error("not all arguments converted during string formatting"),
    }
}

/// `str % mapping`: a mapping argument is never "not all converted", so without conversion specifiers the result
/// is the string itself. (A die is a mapping.)
pub fn str_format_mapping(s: &str) -> Result<Value> {
    if s.contains('%') {
        return Err(Error::Unsupported(
            "str % formatting with conversion specifiers".into(),
        ));
    }
    Ok(Value::str(s))
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
    if let Some(h) = super::host::host_of(a, b) {
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
        (Value::Tuple(x), Value::Tuple(y)) => match x.iter().zip(y.iter()).find(|(p, q)| p != q) {
            Some((p, q)) => compare(op, p, q),
            None => Ok(by_ord(x.len().cmp(&y.len()))),
        },
        _ => match (a.as_num(), b.as_num()) {
            // NaN compares false with everything.
            (Some(x), Some(y)) => Ok(x.partial_cmp(&y).is_some_and(by_ord)),
            _ => type_error(format!(
                "'{}' not supported between instances of '{}' and '{}'",
                op.symbol(),
                a.type_name(),
                b.type_name()
            )),
        },
    }
}

/// Python's dispatch of `a <op> b` where an operand is a `Vector` or `Symbols`: the left operand's method, then
/// the right one's reflected method (`__radd__`, or for a comparison the swapped comparison: `a < b` tries
/// `b > a`), then `TypeError`. The native types always decline these operands, so only the collections' own methods
/// answer, with one exception: `str % x` is printf formatting, and both are mappings to it (they have
/// `__getitem__`), so without conversion specifiers it gives the string.
fn collection_binary(op: BinOp, a: &Value, b: &Value) -> Result<Value> {
    use super::VecOp;
    if let (BinOp::Mod, Value::Str(s)) = (op, a) {
        return str_format_mapping(s);
    }
    // Every method of a `VectorWithTruthOnly`, and every method given one, reads its `_data`, which raises.
    for x in [a, b] {
        if let Value::Vector(v) = x {
            v.data()?;
        }
    }
    match a {
        Value::Vector(v) => return v.binary(VecOp::Bin(op), b),
        Value::Symbols(s) => {
            if let Some(r) = s.binary(op, b) {
                return r;
            }
        }
        _ => {}
    }
    let reflected = op.reflected_comparison();
    match (b, reflected) {
        (Value::Vector(v), Some(r)) => return v.binary(VecOp::Bin(r), a),
        (Value::Vector(v), None) => return v.reverse_binary(op, a),
        (Value::Symbols(s), Some(r)) => {
            if let Some(x) = s.binary(r, a) {
                return x;
            }
        }
        (Value::Symbols(s), None) => {
            if let Some(x) = s.reverse_binary(op, a) {
                return x;
            }
        }
        _ => {}
    }
    if reflected.is_some() {
        type_error(format!(
            "'{}' not supported between instances of '{}' and '{}'",
            op.symbol(),
            a.type_name(),
            b.type_name()
        ))
    } else {
        unsupported_operands(op, a, b)
    }
}

impl Value {
    /// Python's `operator.eq(self, other)` (`operator.ne` if `ne`): a bool, except that a `Vector` compares
    /// elementwise. `Symbols` converts the other operand (so `Symbols('ab') == 'ab'`), and when it cannot, Python
    /// falls back to identity: unequal.
    pub fn eq_op(&self, other: &Value, ne: bool) -> Result<Value> {
        use super::VecOp;
        if super::host::host_of(self, other).is_some()
            || !(self.is_collection() || other.is_collection())
        {
            return Ok(Value::Bool((self == other) != ne));
        }
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

impl crate::outcome::SumOutcome for Value {
    fn add(&self, other: &Self) -> Result<Self> {
        self.binary(BinOp::Add, other)
    }

    fn neg(&self) -> Result<Self> {
        self.unary(UnOp::Neg)
    }

    fn times_zero(&self) -> Result<Self> {
        self.binary(BinOp::Mul, &Value::from(0))
    }
}

#[cfg(test)]
mod tests {
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
