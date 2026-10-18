//! Python's operators on plain values (no dice): `ops.add(1, 2.5)`, `ops.lt("a", "b")`, `truth(vector)`, as the
//! core's `Value` does them (value/ops.rs); and the methods of `Vector` and `Symbols`.

use wasm_bindgen::prelude::*;

use super::convert::{value_from_js, value_to_js};
use super::die::{apply, BinaryOp, UnaryOp};
use super::error::{JsResult, OrThrow};
use super::host;
use icepool_core::value::builtins::{getitem, py_len, Index};
use icepool_core::value::pystr::{py_repr, py_str};
use icepool_core::value::{BinOp, Symbols, UnOp, Vector};
use icepool_core::{Error, Value};

/// `a <op> b` for two plain values.
#[wasm_bindgen(js_name = valueBinary)]
pub fn value_binary(op: BinaryOp, a: JsValue, b: JsValue) -> JsResult<JsValue> {
    let (a, b) = (value_from_js(&a).or_throw()?, value_from_js(&b).or_throw()?);
    value_to_js(&apply(op, &a, &b).or_throw()?).or_throw()
}

/// `<op> a` for a plain value.
#[wasm_bindgen(js_name = valueUnary)]
pub fn value_unary(op: UnaryOp, a: JsValue) -> JsResult<JsValue> {
    let a = value_from_js(&a).or_throw()?;
    value_to_js(&a.unary(UnOp::from(op)).or_throw()?).or_throw()
}

/// `round(a, ndigits)` for a plain value.
#[wasm_bindgen(js_name = valueRound)]
pub fn value_round(a: JsValue, ndigits: JsValue) -> JsResult<JsValue> {
    let (a, ndigits) = (value_from_js(&a).or_throw()?, value_from_js(&ndigits).or_throw()?);
    value_to_js(&a.round(&ndigits).or_throw()?).or_throw()
}

/// Python's `bool(a)` for a plain value (a Vector from a comparison has its truth value; Symbols theirs).
#[wasm_bindgen(js_name = valueTruth)]
pub fn value_truth(a: JsValue) -> JsResult<bool> {
    value_from_js(&a).or_throw()?.truthy().or_throw()
}

// --- Vector and Symbols (collection/vector.py, collection/symbols.py) ----------------------------------------------

/// The protocols both collections share: `x[key]`, `x[start:stop:step]` (`arg` the three bounds), `len(x)`,
/// `repr(x)`, `str(x)`, `tuple(x)` (`iter`; a Symbols' `keys()` too: a mapping iterates over its keys). `None` for
/// any other name.
fn protocol(x: &Value, name: &str, arg: &JsValue) -> Option<icepool_core::Result<Value>> {
    let result = || -> icepool_core::Result<Value> {
        Ok(match name {
            "getitem" => getitem(x, &Index::Item(value_from_js(arg)?))?,
            "slice" => {
                let bounds = host::array_from(arg).iter().map(value_from_js).collect::<icepool_core::Result<Vec<_>>>()?;
                let [start, stop, step]: [Value; 3] =
                    bounds.try_into().map_err(|_| Error::Unported("a slice of other than 3 bounds".into()))?;
                getitem(x, &Index::Slice(start, stop, step))?
            }
            "len" => Value::from(py_len(x)? as i64),
            "repr" => Value::str(&py_repr(x)?),
            "str" => Value::str(&py_str(x)?),
            _ => Value::tuple(py_iter(x)?),
        })
    };
    match name {
        "getitem" | "slice" | "len" | "repr" | "str" | "iter" | "keys" => Some(result()),
        _ => None,
    }
}

/// Python's `iter(x)` of a value: a tuple's or Vector's elements, a str's characters, a Symbols' keys (a mapping
/// iterates over its keys).
fn py_iter(x: &Value) -> icepool_core::Result<Vec<Value>> {
    let chars = |s: &str| s.chars().map(|c| Value::str(c.encode_utf8(&mut [0; 4]))).collect();
    Ok(match x {
        Value::Tuple(t) => t.to_vec(),
        Value::Vector(v) => v.data()?.to_vec(),
        Value::Str(s) => chars(s),
        Value::Symbols(s) => s.items().iter().map(|(c, _)| Value::str(c.encode_utf8(&mut [0; 4]))).collect(),
        _ => return Err(Error::WrongType(format!("'{}' object is not iterable", x.type_name()))),
    })
}

/// A `Vector`'s method `name` (`append`, `concatenate`) or protocol (see `protocol`), with one argument.
#[wasm_bindgen(js_name = vectorMethod)]
pub fn vector_method(v: JsValue, name: &str, arg: JsValue) -> JsResult<JsValue> {
    let v = value_from_js(&v).or_throw()?;
    if let Some(result) = protocol(&v, name, &arg) {
        return value_to_js(&result.or_throw()?).or_throw();
    }
    let Value::Vector(vector) = &v else {
        return Err(Error::MissingAttribute(format!("'{}' object has no attribute '{name}'", v.type_name()))).or_throw();
    };
    let mut items = vector.data().or_throw()?.to_vec();
    match name {
        // `Vector(self._data + (other, ))`
        "append" => items.push(value_from_js(&arg).or_throw()?),
        // `Vector(itertools.chain(self, other))`
        "concatenate" => items.extend(py_iter(&value_from_js(&arg).or_throw()?).or_throw()?),
        _ => return Err(Error::MissingAttribute(format!("'Vector' object has no attribute '{name}'"))).or_throw(),
    }
    value_to_js(&Value::Vector(Vector::new(items))).or_throw()
}

/// A `Symbols`' method `name` (snake_case) or protocol (see `protocol`), with one argument (`count_subset`: the
/// array `[divisor, empty_divisor]`; `getattr`: `s.<arg>`, which is `s[arg]` for a public name).
#[wasm_bindgen(js_name = symbolsMethod)]
pub fn symbols_method(s: JsValue, name: &str, arg: JsValue) -> JsResult<JsValue> {
    let s = value_from_js(&s).or_throw()?;
    if let Some(result) = protocol(&s, name, &arg) {
        return value_to_js(&result.or_throw()?).or_throw();
    }
    let Value::Symbols(symbols) = &s else {
        // `getattr(x, 'a')` on anything else, as `x.values()`, `x.size()`...: no such attribute.
        let attr = match (name, arg.as_string()) {
            ("getattr", Some(a)) => a,
            _ => name.to_string(),
        };
        return Err(Error::MissingAttribute(format!("'{}' object has no attribute '{attr}'", s.type_name()))).or_throw();
    };
    let arg = || value_from_js(&arg);
    let result = match name {
        // `self <= other`, `self >= other`, `self ^ other`, `self * other`, `self % other`.
        "issubset" => arg().and_then(|a| s.binary(BinOp::Le, &a)),
        "issuperset" => arg().and_then(|a| s.binary(BinOp::Ge, &a)),
        "symmetric_difference" => arg().and_then(|a| s.binary(BinOp::Xor, &a)),
        "multiply_counts" => arg().and_then(|a| s.binary(BinOp::Mul, &a)),
        "modulo_counts" => arg().and_then(|a| s.binary(BinOp::Mod, &a)),
        "divide_counts" => arg().and_then(|a| symbols.divide_counts(&a)),
        "isdisjoint" => arg().and_then(|a| symbols.isdisjoint(&a)).map(Value::Bool),
        "has_negative_counts" => symbols.has_negative_counts().map(Value::Bool),
        "size" => symbols.size(),
        "count_subset" => match arg() {
            Ok(Value::Tuple(t)) if t.len() == 2 => symbols.count_subset(&t[0], Some(&t[1])),
            Ok(_) => Err(Error::Unported("count_subset arguments".into())),
            Err(e) => Err(e),
        },
        // `functools.reduce(operator.add, args, initial=self)`: `reduce` takes no keyword arguments before Python
        // 3.14, so these always raise there (DIVERGENCES.md).
        "additive_union" | "difference" | "intersection" | "union" => {
            Err(Error::WrongType("reduce() takes no keyword arguments".into()))
        }
        // `s.<name>` for a name that is not an attribute: `__getattr__`, which is `s[name]` unless the name starts
        // with an underscore (and `key[0]` of an empty name raises IndexError).
        "getattr" => match arg() {
            Ok(Value::Str(a)) if a.is_empty() => Err(Error::IndexOutOfRange("string index out of range".into())),
            Ok(Value::Str(a)) if a.starts_with('_') => {
                Err(Error::MissingAttribute(format!("'Symbols' object has no attribute '{a}'")))
            }
            Ok(Value::Str(a)) => Ok(symbols.getitem(&a)),
            Ok(_) => Err(Error::WrongType("attribute name must be string".into())),
            Err(e) => Err(e),
        },
        // The dict's values in its order, zero counts included.
        "values" => Ok(Value::tuple(symbols.items().iter().map(|(_, n)| n.clone()).collect())),
        _ => Err(Error::MissingAttribute(format!("'Symbols' object has no attribute '{name}'"))),
    };
    value_to_js(&result.or_throw()?).or_throw()
}

/// `Symbols(x)` for a string (parsed: the characters after a dash count negatively), an iterable of characters (`x`
/// an array), or a mapping (`mapping`: `x` its `[key, count]` pairs).
#[wasm_bindgen(js_name = symbolsFrom)]
pub fn symbols_from(x: JsValue, mapping: bool) -> JsResult<JsValue> {
    let items = host::array_from(&x);
    let symbols = if let Some(s) = x.as_string() {
        Symbols::parse(&s)
    } else if mapping {
        let pairs = items
            .iter()
            .map(|kv| {
                let kv = host::array_from(kv);
                let part = |i: usize| value_from_js(kv.get(i).unwrap_or(&JsValue::NULL));
                Ok((part(0)?, part(1)?))
            })
            .collect::<icepool_core::Result<Vec<_>>>()
            .or_throw()?;
        Symbols::from_mapping(pairs)
    } else {
        let elements = items.iter().map(value_from_js).collect::<icepool_core::Result<Vec<_>>>().or_throw()?;
        Symbols::from_elements(&elements)
    };
    value_to_js(&Value::Symbols(symbols.or_throw()?)).or_throw()
}
