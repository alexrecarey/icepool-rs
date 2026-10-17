//! JS values <-> the core's `Value`, directly (no JSON in between).
//!
//! | Python | JS |
//! |---|---|
//! | `None` | `null` (`undefined` goes in as `None` too) |
//! | `bool`, `str` | `boolean`, `string` |
//! | `int` | in: an integral `number` or a `bigint`; out: a `number` when it is a safe integer, else a `bigint` |
//! | `float` | in: a non-integral `number` (also `Infinity`, `NaN`) or a `Float`; out: a `number`, or a `Float` when it is integral (`2.0`, `-0.0`), so that it goes back in as a float |
//! | `Fraction`, `complex` | `Fraction` (bigint parts), `Complex` |
//! | `tuple` | an array |
//! | `Vector`, `Symbols` | `Vector`, `Symbols` |
//!
//! Quantities are always `bigint`s.

use dashu_int::IBig;
use icepool_core::value::recursion::level;
use icepool_core::value::{Symbols, Vector};
use icepool_core::{Error, Quantity, Value};
use wasm_bindgen::JsValue;

use super::host;

/// JS's `Number.MAX_SAFE_INTEGER`: ints up to it in absolute value go out as numbers.
const MAX_SAFE: u64 = (1 << 53) - 1;

fn wrong_type(msg: String) -> Error {
    Error::WrongType(msg)
}

/// A JS number as Python sees it: integral ones are ints (as a JS integer literal means one), the rest floats.
fn number(x: f64) -> Value {
    if x.is_finite() && x.fract() == 0.0 {
        // `{x}` prints an integral f64 with all its digits and no exponent (`-0` for negative zero).
        Value::Int(format!("{x}").parse().expect("an integral float's digits"))
    } else {
        Value::Float(x)
    }
}

/// A BigInt's value.
fn bigint(x: &JsValue) -> Result<IBig, Error> {
    host::js_string(x)
        .parse()
        .map_err(|_| wrong_type("a malformed BigInt".into()))
}

/// A JS value as an outcome (or any other Python value the core takes).
pub fn value_from_js(x: &JsValue) -> Result<Value, Error> {
    if x.is_null_or_undefined() {
        return Ok(Value::None);
    }
    if let Some(b) = x.as_bool() {
        return Ok(Value::Bool(b));
    }
    if let Some(f) = x.as_f64() {
        return Ok(number(f));
    }
    if x.is_bigint() {
        return Ok(Value::Int(bigint(x)?));
    }
    if let Some(s) = x.as_string() {
        return Ok(Value::str(&s));
    }
    // Nested values recurse a level each: deep nesting is a RecursionError, not a stack overflow.
    let _level = level()?;
    if x.is_function() {
        return Err(wrong_type("a function is not an outcome".into()));
    }
    let parts = host::describe(x);
    let tag = parts.first().and_then(JsValue::as_string).unwrap_or_default();
    let part = |i: usize| parts.get(i).cloned().unwrap_or(JsValue::UNDEFINED);
    let float = |i: usize| part(i).as_f64().ok_or_else(|| wrong_type(format!("a malformed {tag}")));
    Ok(match tag.as_str() {
        "float" => Value::Float(float(1)?),
        "fraction" => {
            let (n, d) = (bigint(&part(1))?, bigint(&part(2))?);
            if d == IBig::ZERO {
                return Err(Error::DivisionByZero(format!("Fraction({n}, 0)")));
            }
            // Normalised as Python's `Fraction(n, d)` is: lowest terms, the sign on the numerator.
            let (n, d) = if d < IBig::ZERO { (-n, -d) } else { (n, d) };
            Value::Fraction(dashu_ratio::RBig::from_parts(
                n,
                d.try_into().expect("a positive denominator"),
            ))
        }
        "complex" => Value::Complex(float(1)?, float(2)?),
        "vector" => {
            let truth = match part(2).as_bool() {
                Some(true) => Some(value_from_js(&part(3))?),
                _ => None,
            };
            let items = part(1);
            Value::Vector(match (items.is_null(), truth) {
                (true, Some(truth)) => Vector::truth_only(truth),
                (true, None) => return Err(wrong_type("a Vector without items or truth".into())),
                (false, truth) => Vector::from_parts(
                    host::array_from(&items)
                        .iter()
                        .map(value_from_js)
                        .collect::<Result<_, _>>()?,
                    truth,
                ),
            })
        }
        "symbols" => {
            let items = part(1);
            if items.is_null() {
                // Made in JS from a str: parsed as `Symbols(str)` parses it.
                let s = part(2).as_string().unwrap_or_default();
                Value::Symbols(Symbols::parse(&s)?)
            } else {
                let mut data = Vec::new();
                for item in host::array_from(&items) {
                    let pair = host::array_from(&item);
                    let c = pair.first().and_then(JsValue::as_string).unwrap_or_default();
                    let mut chars = c.chars();
                    let (Some(c), None) = (chars.next(), chars.next()) else {
                        return Err(wrong_type("a Symbols key that is not one character".into()));
                    };
                    data.push((c, value_from_js(&pair.get(1).cloned().unwrap_or(JsValue::NULL))?));
                }
                Value::Symbols(Symbols::raw(data))
            }
        }
        "array" => Value::tuple(
            host::array_from(x)
                .iter()
                .map(value_from_js)
                .collect::<Result<_, _>>()?,
        ),
        // A `List` is a Python list: unhashable, so never an outcome.
        "list" => return Err(Error::Unhashable { type_name: "list" }),
        _ => {
            let name = part(1).as_string().unwrap_or_else(|| "object".into());
            return Err(wrong_type(format!("cannot pass {name} to icepool as a value")));
        }
    })
}

/// An int as JS: a number when safe, else a BigInt.
pub fn int_to_js(i: &IBig) -> JsValue {
    match i64::try_from(i) {
        Ok(n) if n.unsigned_abs() <= MAX_SAFE => JsValue::from_f64(n as f64),
        _ => JsValue::bigint_from_str(&i.to_string()),
    }
}

/// A quantity as JS: always a BigInt.
pub fn quantity_to_js(q: &Quantity) -> JsValue {
    JsValue::bigint_from_str(&q.to_string())
}

/// A value as JS.
pub fn value_to_js(v: &Value) -> Result<JsValue, Error> {
    let _level = match v {
        Value::Tuple(_) | Value::Vector(_) | Value::Symbols(_) => Some(level()?),
        _ => None,
    };
    Ok(match v {
        Value::None => JsValue::NULL,
        Value::Bool(b) => JsValue::from_bool(*b),
        Value::Int(i) => int_to_js(i),
        Value::Float(f) if f.is_finite() && f.fract() == 0.0 => host::make_float(*f),
        Value::Float(f) => JsValue::from_f64(*f),
        Value::Fraction(r) => host::make_fraction(
            JsValue::bigint_from_str(&r.numerator().to_string()),
            JsValue::bigint_from_str(&r.denominator().to_string()),
        ),
        Value::Str(s) => JsValue::from_str(s),
        Value::Tuple(t) => values_to_js(t)?,
        Value::Complex(re, im) => host::make_complex(*re, *im),
        // The truth value of a comparison's vector, and a `VectorWithTruthOnly` (no data), survive the trip.
        Value::Vector(x) => {
            let items = if x.is_truth_only() {
                JsValue::NULL
            } else {
                values_to_js(x.data()?)?
            };
            let truth = x.truth().map(value_to_js).transpose()?;
            host::make_vector(items, truth.is_some(), truth.unwrap_or(JsValue::NULL))
        }
        // The dict's items in its own order, zero counts included (`str()` shows neither), and `str()` itself.
        Value::Symbols(x) => {
            let items = x
                .items()
                .iter()
                .map(|(c, n)| {
                    Ok(JsValue::from(vec![
                        JsValue::from_str(c.encode_utf8(&mut [0; 4])),
                        value_to_js(n)?,
                    ]))
                })
                .collect::<Result<Vec<_>, Error>>()?;
            host::make_symbols(items, x.py_str().ok())
        }
        Value::Host(h) => host::make_opaque(&h.repr()),
    })
}

/// Values as a JS array.
pub fn values_to_js(values: &[Value]) -> Result<JsValue, Error> {
    Ok(JsValue::from(
        values.iter().map(value_to_js).collect::<Result<Vec<_>, _>>()?,
    ))
}

/// A JS integer (a number or a BigInt) as an int; anything else is Python's TypeError for a non-int.
pub fn int_from_js(x: &JsValue, what: &str) -> Result<IBig, Error> {
    match value_from_js(x)? {
        Value::Int(i) => Ok(i),
        Value::Bool(b) => Ok(IBig::from(b as u8)),
        v => Err(wrong_type(format!(
            "{what} must be an int, not '{}'",
            v.type_name()
        ))),
    }
}
