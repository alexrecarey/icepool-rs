//! The operators of `str` and `tuple` (concatenation, repetition, printf-style `%`) and of `Vector` and
//! `Symbols`, which follow their own classes.

use super::{too_large, unsupported_operands, BinOp, MAX_REPEAT};
use crate::error::{wrong_type, Error, Result};
use crate::value::recursion::level;
use crate::value::Value;

/// `+`, `*`, `%` and friends where an operand is not a number.
pub(super) fn sequence_op(op: BinOp, a: &Value, b: &Value) -> Result<Value> {
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
    // Within `MAX_REPEAT` only wasm32's lower allocation cap can refuse (`MemoryError`).
    match seq {
        Value::Str(s) => crate::memory::check_len::<u8, _>(s.len().checked_mul(n), "str * n")?,
        _ => crate::memory::check_len::<Value, _>(len.checked_mul(n), "tuple * n")?,
    };
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
        return Err(Error::Unported(
            "str % formatting with conversion specifiers".into(),
        ));
    }
    match args {
        Value::Tuple(t) if t.is_empty() => Ok(Value::str(s)),
        _ => wrong_type("not all arguments converted during string formatting"),
    }
}

/// `str % mapping`: a mapping argument is never "not all converted", so without conversion specifiers the result
/// is the string itself. (A die is a mapping.)
pub fn str_format_mapping(s: &str) -> Result<Value> {
    if s.contains('%') {
        return Err(Error::Unported(
            "str % formatting with conversion specifiers".into(),
        ));
    }
    Ok(Value::str(s))
}

/// Python's dispatch of `a <op> b` where an operand is a `Vector` or `Symbols`: the left operand's method, then
/// the right one's reflected method (`__radd__`, or for a comparison the swapped comparison: `a < b` tries
/// `b > a`), then `TypeError`. The native types always decline these operands, so only the collections' own methods
/// answer, with one exception: `str % x` is printf formatting, and both are mappings to it (they have
/// `__getitem__`), so without conversion specifiers it gives the string.
pub(super) fn collection_binary(op: BinOp, a: &Value, b: &Value) -> Result<Value> {
    use crate::value::VecOp;
    let _level = level()?;
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
        Err(Error::Incomparable {
            op: op.symbol(),
            left: a.type_name(),
            right: b.type_name(),
        })
    } else {
        unsupported_operands(op, a, b)
    }
}
