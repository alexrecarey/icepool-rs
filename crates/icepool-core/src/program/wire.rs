//! The wire format (harness/core/encode.py): typed JSON literals in, typed JSON results out.

use std::str::FromStr;

use dashu_int::IBig;
use dashu_ratio::RBig;
use serde_json::{json, Map, Number, Value as Json};

use super::{raise, PErr, PValue};
use crate::error::Error;
use crate::value::{float_repr, Symbols, Value, Vector};
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

/// The Python exception icepool raises where the core returns `e`, by name: the `error` field of a wire result.
/// Fixtures only check that both sides raised, so the name is informational; it is total over the variants, so a
/// new variant must be named here. (The Python facade maps variants to exception types itself, in icepool-py.)
pub fn python_exception(e: &Error) -> &'static str {
    match e {
        Error::InvalidArgument(_) => "ValueError",
        Error::WrongType(_) | Error::Incomparable { .. } | Error::Unhashable { .. } => "TypeError",
        Error::MissingAttribute(_) => "AttributeError",
        Error::IndexOutOfRange(_) => "IndexError",
        Error::MissingKey(_) => "KeyError",
        Error::DivisionByZero(_) => "ZeroDivisionError",
        Error::ConflictingOrder(_) => "ConflictingOrderError",
        Error::UninferrableSize(_) => "OrderError",
        Error::UnsupportedOrder(_) => "UnsupportedOrder",
        Error::RecursionLimit(_) => "RecursionError",
        Error::Overflow(_) => "OverflowError",
        Error::TooLarge(_) => "MemoryError",
        Error::NotImplementedUpstream(_) => "NotImplementedError",
        // Not raises: the runner reports these before asking (`PErr::Unimplemented`, the host's own exception).
        Error::Unported(_) => "unimplemented",
        Error::Callback(_) => "host exception",
    }
}

/// The wire result of a raise: `{"ok": false, "error": <Python exception>, "message": <its message>}`.
pub fn raised(e: &Error) -> Json {
    json!({"ok": false, "error": python_exception(e), "message": e.to_string()})
}

fn invalid_argument(msg: &str) -> PErr {
    PErr::Raise(Error::InvalidArgument(msg.into()))
}

/// `encode.dec`: typed JSON -> value, for literals inside programs. Lists are handled by the evaluator; inside a
/// literal (e.g. a tuple's elements) they are an error, as in Python.
pub fn dec(j: &Json) -> Result<PValue, PErr> {
    match j {
        Json::Null => Ok(PValue::Value(Value::None)),
        Json::Number(n) => match json_int(n) {
            Some(i) => Ok(PValue::Value(Value::Int(i))),
            None => Err(invalid_argument("bare JSON floats are not allowed")),
        },
        Json::Bool(_) => Err(invalid_argument("bare JSON booleans are not allowed")),
        Json::String(_) => Err(invalid_argument("bare JSON strings are not allowed")),
        Json::Array(_) => Err(invalid_argument(
            "bare JSON lists are not allowed as values",
        )),
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
                            return Err(invalid_argument("not a typed value"))
                        }
                        _ => return raise(Error::WrongType("object is not iterable".into())),
                    };
                    let mut out = Vec::with_capacity(elems.len());
                    for e in elems {
                        match dec(e)? {
                            PValue::Value(x) => out.push(x),
                            _ => return Err(invalid_argument("not a typed value")),
                        }
                    }
                    Ok(PValue::Value(Value::tuple(out)))
                }
                // icepool.Vector(dec(e) for e in v): as a tuple.
                "vector" => match dec(&json!({ "tuple": v }))? {
                    PValue::Value(Value::Tuple(t)) => {
                        Ok(PValue::Value(Value::Vector(Vector::new(t.to_vec()))))
                    }
                    _ => unreachable!("a tuple literal decodes to a tuple"),
                },
                "symbols" => symbols(v).map(|s| PValue::Value(Value::Symbols(s))),
                _ => Err(invalid_argument("not a typed value")),
            }
        }
        Json::Object(_) => Err(invalid_argument("not a typed value")),
    }
}

/// `icepool.Symbols(v)` of a raw JSON value (not decoded first): a string, a list (an iterable of characters), or
/// an object (a mapping of characters to counts).
fn symbols(v: &Json) -> Result<Symbols, PErr> {
    // A raw JSON scalar as the Python object `json.loads` makes.
    let scalar = |j: &Json| -> Result<Value, PErr> {
        Ok(match j {
            Json::Null => Value::None,
            Json::Bool(b) => Value::Bool(*b),
            Json::String(s) => Value::str(s),
            Json::Number(n) => match json_int(n) {
                Some(i) => Value::Int(i),
                None => Value::Float(parse_float(&n.to_string())?),
            },
            // A list or dict is unhashable: `data[s] += 1` raises.
            Json::Array(_) => return raise(Error::Unhashable { type_name: "list" }),
            Json::Object(_) => return raise(Error::Unhashable { type_name: "dict" }),
        })
    };
    match v {
        Json::String(s) => Symbols::parse(s),
        Json::Array(a) => {
            Symbols::from_elements(&a.iter().map(scalar).collect::<Result<Vec<_>, _>>()?)
        }
        Json::Object(o) => {
            let mut items = Vec::with_capacity(o.len());
            for (k, n) in o {
                let n = match n {
                    Json::Number(_) | Json::Bool(_) => scalar(n)?,
                    _ => {
                        return Err(PErr::Unimplemented(
                            "Symbols with non-numeric counts".into(),
                        ))
                    }
                };
                items.push((Value::str(k), n));
            }
            Symbols::from_mapping(items)
        }
        _ => {
            let name = match v {
                Json::Null => "NoneType",
                Json::Bool(_) => "bool",
                Json::Number(n) if json_int(n).is_some() => "int",
                _ => "float",
            };
            return raise(Error::WrongType(format!("'{name}' object is not iterable")));
        }
    }
    .map_err(PErr::Raise)
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
        _ => raise(Error::WrongType(
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
        _ => raise(Error::InvalidArgument(format!(
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
        return raise(Error::WrongType(
            "both arguments should be Rational instances".into(),
        ));
    };
    if d == IBig::ZERO {
        return raise(Error::DivisionByZero("Fraction(n, 0)".into()));
    }
    Ok(RBig::from_parts_signed(n, d))
}

fn int_json(s: String) -> Json {
    Json::Number(Number::from_str(&s).expect("an integer is a JSON number"))
}

/// `encode.enc` for a value. It raises where Python's does: iterating a `VectorWithTruthOnly`, or `str()` of a
/// `Symbols` whose counts are not ints.
///
/// Tuples and vectors nest: a level each (`value::recursion`).
pub fn enc(v: &Value) -> crate::Result<Json> {
    let _level = match v {
        Value::Tuple(_) | Value::Vector(_) => Some(crate::value::recursion::level()?),
        _ => None,
    };
    let tagged = |k: &str, x: Json| {
        let mut m = Map::new();
        m.insert(k.into(), x);
        Json::Object(m)
    };
    Ok(match v {
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
        Value::Tuple(t) => tagged(
            "tuple",
            Json::Array(t.iter().map(enc).collect::<crate::Result<_>>()?),
        ),
        Value::Vector(x) => tagged(
            "vector",
            Json::Array(x.data()?.iter().map(enc).collect::<crate::Result<_>>()?),
        ),
        Value::Symbols(x) => tagged("symbols", Json::String(x.py_str()?)),
        // encode.py's fallback for types it does not know.
        Value::Complex(..) => tagged("repr", Json::String(format!("complex: {v}"))),
        // Programs never make host objects (only the Python facade does); spelled like encode.py's fallback.
        Value::Host(_) => tagged("repr", Json::String(format!("object: {v}"))),
    })
}

fn die_items(d: &Die<Value>) -> crate::Result<Vec<Json>> {
    d.items()
        .iter()
        .map(|(o, q)| Ok(Json::Array(vec![enc(o)?, int_json(q.to_string())])))
        .collect()
}

/// `encode.enc` for any program value: `Ok(None)` for the ones the port cannot spell as Python does (lists,
/// functions).
fn enc_object(v: &PValue) -> crate::Result<Option<Json>> {
    let _level = crate::value::recursion::level()?;
    Ok(Some(match v {
        // Encoding reads the data, which raises.
        PValue::DieWithTruthError(e, _) => return Err(e.clone()),
        PValue::Value(x) => enc(x)?,
        PValue::Die(d) | PValue::DieWithTruth(d, _) => json!({"die": die_items(d)?}),
        PValue::Tuple(items) => {
            let mut out = Vec::with_capacity(items.len());
            for x in items {
                match enc_object(x)? {
                    Some(j) => out.push(j),
                    None => return Ok(None),
                }
            }
            json!({ "tuple": out })
        }
        PValue::Reroll => json!({"repr": "RerollType: <RerollType.Reroll: 'Reroll'>"}),
        PValue::Restart => json!({"repr": "RerollType: <RerollType.Restart: 'Restart'>"}),
        // An `AgainExpression` has the default object repr, with an address in it.
        PValue::Ellipsis => json!({"repr": "ellipsis: Ellipsis"}),
        PValue::Deck(d) => match deck_repr(d) {
            Ok(r) => json!({ "repr": format!("Deck: {r}") }),
            Err(_) => return Ok(None),
        },
        PValue::Deal(d) => match deck_repr(d.deck()) {
            Ok(r) => json!({"repr": format!("Deal: Deal({r}, hand_size={})", d.hand_size())}),
            Err(_) => return Ok(None),
        },
        PValue::MultiDeal(d) => {
            match deck_repr(d.deck()).and_then(|r| super::multi_deal::repr(d, &r)) {
                Ok(r) => json!({ "repr": format!("MultiDeal: {r}") }),
                Err(_) => return Ok(None),
            }
        }
        // A pool's repr, like an `AgainExpression`'s or a multiset expression's, holds an address.
        PValue::List(_)
        | PValue::Func(_)
        | PValue::Again(_)
        | PValue::Pool(_)
        | PValue::Expr(_)
        | PValue::Break(_)
        | PValue::MultisetFunction(_)
        | PValue::RawResult(_) => return Ok(None),
    }))
}

/// `Deck.__repr__`: `Deck({outcome!r}: quantity, ...)`.
pub(super) fn deck_repr(d: &crate::Deck<Value>) -> crate::Result<String> {
    let items = d
        .items()
        .iter()
        .map(|(o, q)| Ok(format!("{}: {q}", crate::value::pystr::py_repr(o)?)))
        .collect::<crate::Result<Vec<_>>>()?;
    Ok(format!("Deck({{{}}})", items.join(", ")))
}

/// `encode.result_of`.
pub fn result_of(v: &PValue) -> Json {
    let encoded = match v {
        PValue::Die(d) | PValue::DieWithTruth(d, _) => {
            die_items(d).map(|items| Some(json!({"ok": true, "die": items})))
        }
        other => enc_object(other).map(|j| j.map(|value| json!({"ok": true, "value": value}))),
    };
    match encoded {
        Ok(Some(j)) => j,
        Ok(None) => json!({"ok": false, "error": "unimplemented: this kind of program result"}),
        Err(Error::Unported(what)) => {
            json!({"ok": false, "error": format!("unimplemented: {what}")})
        }
        Err(e) => raised(&e),
    }
}
