//! The wire format (harness/core/encode.py): typed JSON literals in, typed JSON results out.

use std::str::FromStr;

use dashu_int::IBig;
use dashu_ratio::RBig;
use serde_json::{json, Map, Number, Value as Json};

use super::{raise, PErr, PValue};
use crate::error::Error;
use crate::value::{float_repr, Value};
use crate::Die;

/// A JSON number: Python's `json` makes it an `int` unless it has a fraction or exponent. Returns `None` for floats.
pub fn json_int(n: &Number) -> Option<IBig> {
    let s = n.to_string();
    if s.contains(['.', 'e', 'E']) {
        None
    } else {
        IBig::from_str(&s).ok()
    }
}

fn value_error(msg: &str) -> PErr {
    PErr::Raise(Error::Value(msg.into()))
}

/// `encode.dec`: typed JSON -> value, for literals inside programs. Lists are handled by the evaluator; inside a
/// literal (e.g. a tuple's elements) they are an error, as in Python.
pub fn dec(j: &Json) -> Result<PValue, PErr> {
    match j {
        Json::Null => Ok(PValue::Value(Value::None)),
        Json::Number(n) => match json_int(n) {
            Some(i) => Ok(PValue::Value(Value::Int(i))),
            None => Err(value_error("bare JSON floats are not allowed")),
        },
        Json::Bool(_) => Err(value_error("bare JSON booleans are not allowed")),
        Json::String(_) => Err(value_error("bare JSON strings are not allowed")),
        Json::Array(_) => Err(value_error("bare JSON lists are not allowed as values")),
        Json::Object(m) if m.len() == 1 => {
            let (k, v) = m.iter().next().expect("one entry");
            match k.as_str() {
                "bool" => Ok(PValue::Value(Value::Bool(truthy(v)))),
                "str" => py_str(v).map(|s| PValue::Value(Value::str(&s))),
                "float" => py_float(v).map(|f| PValue::Value(Value::Float(f))),
                "frac" => fraction(v).map(|r| PValue::Value(Value::Fraction(r))),
                "tuple" => {
                    // tuple(dec(e) for e in v): any iterable. An empty str or dict iterates to nothing; a non-empty
                    // one yields bare strings, which dec rejects.
                    let elems: &[Json] = match v {
                        Json::Array(a) => a,
                        Json::String(s) if s.is_empty() => &[],
                        Json::Object(o) if o.is_empty() => &[],
                        Json::String(_) | Json::Object(_) => {
                            return Err(value_error("not a typed value"))
                        }
                        _ => return raise(Error::Type("object is not iterable".into())),
                    };
                    let mut out = Vec::with_capacity(elems.len());
                    for e in elems {
                        match dec(e)? {
                            PValue::Value(x) => out.push(x),
                            _ => return Err(value_error("not a typed value")),
                        }
                    }
                    Ok(PValue::Value(Value::tuple(out)))
                }
                "vector" | "symbols" => Err(PErr::Unimplemented(format!("{k} literals"))),
                _ => Err(value_error("not a typed value")),
            }
        }
        Json::Object(_) => Err(value_error("not a typed value")),
    }
}

/// Python's `bool(x)` of a raw JSON value.
fn truthy(v: &Json) -> bool {
    match v {
        Json::Null => false,
        Json::Bool(b) => *b,
        Json::Number(n) => match json_int(n) {
            Some(i) => i != IBig::ZERO,
            None => n.as_f64().is_some_and(|f| f != 0.0),
        },
        Json::String(s) => !s.is_empty(),
        Json::Array(a) => !a.is_empty(),
        Json::Object(o) => !o.is_empty(),
    }
}

/// Python's `str(x)` of a raw JSON value (the forms a program could sensibly use).
fn py_str(v: &Json) -> Result<String, PErr> {
    match v {
        Json::String(s) => Ok(s.clone()),
        Json::Null => Ok("None".into()),
        Json::Bool(b) => Ok(if *b { "True" } else { "False" }.into()),
        Json::Number(n) => match json_int(n) {
            Some(i) => Ok(i.to_string()),
            None => Ok(float_repr(parse_float(&n.to_string())?)),
        },
        _ => Err(PErr::Unimplemented("str() of a JSON container".into())),
    }
}

/// Python's `float(x)` of a raw JSON value.
fn py_float(v: &Json) -> Result<f64, PErr> {
    match v {
        Json::String(s) => parse_float(s),
        Json::Bool(b) => Ok(*b as u8 as f64),
        Json::Number(n) => {
            // float(int) rounds correctly (half to even) and overflows rather than giving inf; so does parsing.
            let f = parse_float(&n.to_string())?;
            if f.is_infinite() {
                raise(Error::Overflow("int too large to convert to float".into()))
            } else {
                Ok(f)
            }
        }
        _ => raise(Error::Type(
            "float() argument must be a string or a real number".into(),
        )),
    }
}

/// `float(str)`: surrounding whitespace is ignored, and an underscore may sit between two digits.
fn parse_float(s: &str) -> Result<f64, PErr> {
    let s = s.trim();
    let b = s.as_bytes();
    let underscores_ok = b.iter().enumerate().all(|(i, &c)| {
        c != b'_'
            || (i > 0 && i + 1 < b.len() && b[i - 1].is_ascii_digit() && b[i + 1].is_ascii_digit())
    });
    let cleaned = s.replace('_', "");
    match cleaned.parse::<f64>() {
        Ok(f) if underscores_ok => Ok(f),
        _ => raise(Error::Value(format!(
            "could not convert string to float: {s:?}"
        ))),
    }
}

/// `Fraction(v[0], v[1])`: two ints (bools count), normalised; a zero denominator raises.
fn fraction(v: &Json) -> Result<RBig, PErr> {
    let int = |j: &Json| match j {
        Json::Number(n) => json_int(n),
        Json::Bool(b) => Some(IBig::from(*b as u8)),
        _ => None,
    };
    let (Some(n), Some(d)) = (v.get(0).and_then(int), v.get(1).and_then(int)) else {
        return raise(Error::Type(
            "both arguments should be Rational instances".into(),
        ));
    };
    if d == IBig::ZERO {
        return raise(Error::ZeroDivision("Fraction(n, 0)".into()));
    }
    Ok(RBig::from_parts_signed(n, d))
}

fn int_json(s: String) -> Json {
    Json::Number(Number::from_str(&s).expect("an integer is a JSON number"))
}

/// `encode.enc` for a value.
pub fn enc(v: &Value) -> Json {
    let tagged = |k: &str, x: Json| {
        let mut m = Map::new();
        m.insert(k.into(), x);
        Json::Object(m)
    };
    match v {
        Value::None => Json::Null,
        Value::Bool(b) => tagged("bool", Json::Bool(*b)),
        Value::Int(i) => int_json(i.to_string()),
        Value::Float(f) => tagged("float", Json::String(float_repr(*f))),
        Value::Fraction(r) => tagged(
            "frac",
            Json::Array(vec![
                int_json(r.numerator().to_string()),
                int_json(r.denominator().to_string()),
            ]),
        ),
        Value::Str(s) => tagged("str", Json::String(s.to_string())),
        Value::Tuple(t) => tagged("tuple", Json::Array(t.iter().map(enc).collect())),
        // encode.py's fallback for types it does not know.
        Value::Complex(..) => tagged("repr", Json::String(format!("complex: {v}"))),
    }
}

fn die_items(d: &Die<Value>) -> Vec<Json> {
    d.items()
        .iter()
        .map(|(o, q)| Json::Array(vec![enc(o), int_json(q.to_string())]))
        .collect()
}

/// `encode.enc` for any program value: `None` for the ones the port cannot spell as Python does (lists, functions).
fn enc_object(v: &PValue) -> Option<Json> {
    Some(match v {
        PValue::Value(x) => enc(x),
        PValue::Die(d) => json!({"die": die_items(d)}),
        PValue::Tuple(items) => {
            json!({"tuple": items.iter().map(enc_object).collect::<Option<Vec<_>>>()?})
        }
        PValue::Reroll => json!({"repr": "RerollType: <RerollType.Reroll: 'Reroll'>"}),
        // An `AgainExpression` has the default object repr, with an address in it.
        PValue::List(_) | PValue::Func(_) | PValue::Again(_) => return None,
    })
}

/// `encode.result_of`.
pub fn result_of(v: &PValue) -> Json {
    match v {
        PValue::Die(d) => json!({"ok": true, "die": die_items(d)}),
        other => match enc_object(other) {
            Some(value) => json!({"ok": true, "value": value}),
            None => json!({"ok": false, "error": "unimplemented: this kind of program result"}),
        },
    }
}
