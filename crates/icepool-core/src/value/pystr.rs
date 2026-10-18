//! Python's `str()` and `repr()` of values, exactly (unlike `Display for Value`, which is a debugging repr), and
//! the float presentation types of `format()` (`'f'`, `'e'`, `'%'`) that population/format.py uses.

use dashu_int::IBig;

use super::Value;
use crate::error::{Error, Result};

/// CPython's default `sys.get_int_max_str_digits()`: converting an int with more decimal digits than this to a
/// string raises `ValueError`.
const INT_MAX_STR_DIGITS: usize = 4300;

/// `str(int)`, with CPython's limit on the number of digits (the sign does not count).
fn int_str(i: &IBig) -> Result<String> {
    let s = i.to_string();
    if s.trim_start_matches('-').len() > INT_MAX_STR_DIGITS {
        return Err(Error::InvalidArgument(format!(
            "Exceeds the limit ({INT_MAX_STR_DIGITS} digits) for integer string conversion; use \
             sys.set_int_max_str_digits() to increase the limit"
        )));
    }
    Ok(s)
}

/// `repr(str)` (CPython `unicode_repr`): single quotes unless the string holds a `'` and no `"`; backslash, the
/// quote, `\t` `\n` `\r` and other ASCII control characters escaped. Whether a non-ASCII character is printable
/// needs Unicode's tables, which the core does not carry: those are unsupported.
fn str_repr(s: &str) -> Result<String> {
    let quote = if s.contains('\'') && !s.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::with_capacity(s.len() + 2);
    out.push(quote);
    for c in s.chars() {
        match c {
            _ if c == quote || c == '\\' => {
                out.push('\\');
                out.push(c);
            }
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ if c < ' ' || c == '\x7f' => out.push_str(&format!("\\x{:02x}", c as u32)),
            _ if c.is_ascii() => out.push(c),
            _ => return Err(Error::Unported("repr() of a non-ASCII str".into())),
        }
    }
    out.push(quote);
    Ok(out)
}

/// Python's `repr(x)`. Tuples and vectors nest, so this recurses: a level each (`recursion.rs`), and only through
/// small frames (the other types are `scalar_repr`'s).
pub fn py_repr(x: &Value) -> Result<String> {
    match x {
        Value::Tuple(t) => {
            let _level = super::recursion::level()?;
            tuple_repr(t)
        }
        Value::Vector(v) => {
            let _level = super::recursion::level()?;
            v.repr_with(py_repr, py_str)
        }
        _ => scalar_repr(x),
    }
}

fn tuple_repr(t: &[Value]) -> Result<String> {
    let items = t.iter().map(py_repr).collect::<Result<Vec<_>>>()?;
    Ok(if items.len() == 1 {
        format!("({},)", items[0])
    } else {
        format!("({})", items.join(", "))
    })
}

#[inline(never)]
fn scalar_repr(x: &Value) -> Result<String> {
    Ok(match x {
        Value::Int(i) => int_str(i)?,
        Value::Fraction(r) => format!(
            "Fraction({}, {})",
            int_str(r.numerator())?,
            int_str(&IBig::from(r.denominator().clone()))?
        ),
        Value::Str(s) => str_repr(s)?,
        Value::Host(h) => h.try_repr()?,
        Value::Symbols(s) => s.py_repr()?,
        Value::Tuple(_) | Value::Vector(_) => py_repr(x)?,
        // None, bool, float, complex: `Display` is already Python's repr.
        _ => x.to_string(),
    })
}

/// Python's `str(x)`: the same as `repr` except for `str` itself, `Fraction` (`'n/d'`, or `'n'` for a whole
/// number) and host objects (their own `__str__`).
pub fn py_str(x: &Value) -> Result<String> {
    match x {
        Value::Str(s) => Ok(s.to_string()),
        Value::Fraction(r) => {
            let n = int_str(r.numerator())?;
            if r.denominator().is_one() {
                Ok(n)
            } else {
                Ok(format!(
                    "{n}/{}",
                    int_str(&IBig::from(r.denominator().clone()))?
                ))
            }
        }
        Value::Host(h) => h.try_str(),
        Value::Symbols(s) => s.py_str(),
        // `str(Vector)` is `Vector(str(self._data))`, and `str(tuple)` shows the elements' reprs.
        _ => py_repr(x),
    }
}

/// A float presentation type of Python's format mini-language.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FloatFormat {
    /// `'f'`: fixed point.
    Fixed,
    /// `'e'`: scientific, with at least two exponent digits (`'1.50e+06'`).
    Exponent,
    /// `'%'`: the number times 100 in fixed point, then `'%'`.
    Percent,
}

/// `format(x, f'.{precision}{kind}')` for a float. Rust's exact formatting rounds half to even on the exact binary
/// value, as CPython's dtoa does (checked against Python on 800k values, ties included); only the exponent's
/// spelling and the non-finite values differ.
pub fn format_float(x: f64, kind: FloatFormat, precision: usize) -> String {
    let x = if kind == FloatFormat::Percent {
        x * 100.0
    } else {
        x
    };
    let body = if x.is_nan() {
        "nan".to_string()
    } else if x.is_infinite() {
        if x > 0.0 { "inf" } else { "-inf" }.to_string()
    } else if kind == FloatFormat::Exponent {
        let s = format!("{x:.precision$e}");
        let (mantissa, exp) = s.split_once('e').expect("{:e} has an exponent");
        let exp: i32 = exp.parse().expect("an integer exponent");
        let sign = if exp < 0 { '-' } else { '+' };
        format!("{mantissa}e{sign}{:02}", exp.abs())
    } else {
        format!("{x:.precision$}")
    };
    if kind == FloatFormat::Percent {
        body + "%"
    } else {
        body
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dashu_ratio::RBig;

    #[test]
    fn str_and_repr() {
        let f = Value::Fraction(RBig::from_parts(IBig::from(-3), 2u8.into()));
        assert_eq!(py_str(&f).unwrap(), "-3/2");
        assert_eq!(py_repr(&f).unwrap(), "Fraction(-3, 2)");
        let whole = Value::Fraction(RBig::from(4));
        assert_eq!(py_str(&whole).unwrap(), "4");
        let t = Value::tuple(vec![Value::str("it's"), Value::str("a\"'\n"), f.clone()]);
        assert_eq!(
            py_str(&t).unwrap(),
            r#"("it's", 'a"\'\n', Fraction(-3, 2))"#
        );
        assert_eq!(py_str(&Value::tuple(vec![Value::None])).unwrap(), "(None,)");
        assert_eq!(py_str(&Value::Float(1e16)).unwrap(), "1e+16");
        assert!(py_str(&Value::Int(IBig::from(10).pow(4300))).is_err());
        assert!(py_str(&Value::Int(IBig::ONE - IBig::from(10).pow(4300))).is_ok());
    }

    #[test]
    fn float_formats() {
        use FloatFormat::*;
        assert_eq!(format_float(5e6, Exponent, 2), "5.00e+06");
        assert_eq!(format_float(1.5e-300, Exponent, 0), "2e-300");
        assert_eq!(format_float(0.125, Fixed, 2), "0.12");
        assert_eq!(format_float(-0.0, Fixed, 1), "-0.0");
        assert_eq!(format_float(1.0 / 3.0, Percent, 6), "33.333333%");
        assert_eq!(format_float(f64::INFINITY, Percent, 6), "inf%");
        assert_eq!(format_float(f64::NAN, Exponent, 6), "nan");
    }
}
