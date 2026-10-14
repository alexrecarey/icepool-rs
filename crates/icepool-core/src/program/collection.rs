//! Nodes of harness/grammar/collection.py: `Vector`, `vectorize` and `Symbols` as values, and their methods.

use super::format;
use super::ops::{binary_op, py_iter, type_error, unary_op};
use super::{raise, PErr, PValue};
use crate::error::Error;
use crate::value::builtins::{getitem, py_len, Index};
use crate::value::pystr::{py_repr, py_str};
use crate::value::{BinOp, Symbols, Vector};
use crate::{Quantity, Value};

fn value(x: PValue) -> Result<Value, PErr> {
    match x {
        PValue::Value(v) => Ok(v),
        _ => Err(PErr::Unimplemented(
            "a collection holding dice, lists or Reroll".into(),
        )),
    }
}

fn name(x: &PValue) -> Result<&str, PErr> {
    match x {
        PValue::Value(Value::Str(s)) => Ok(s),
        _ => Err(PErr::Unimplemented(
            "a method name that is not a str".into(),
        )),
    }
}

fn values(items: Vec<PValue>) -> Result<Vec<Value>, PErr> {
    items.into_iter().map(value).collect()
}

fn attribute_error<T>(receiver: &PValue, attr: &str) -> Result<T, PErr> {
    let type_name = match receiver {
        PValue::Value(v) => v.type_name(),
        _ => "object",
    };
    type_error(format!(
        "AttributeError: '{type_name}' object has no attribute '{attr}'"
    ))
}

/// `icepool.vectorize(*args)`: `Reroll` among the arguments is the result; with no die, a `Vector` of the
/// arguments; otherwise the Cartesian product as a die of vectors.
pub fn vectorize(args: PValue) -> Result<PValue, PErr> {
    let args = py_iter(args)?;
    // `if arg in icepool.REROLL_TYPES: return arg`, checked argument by argument.
    if let Some(r) = args
        .iter()
        .find(|x| matches!(x, PValue::Reroll | PValue::Restart))
    {
        return Ok(r.clone());
    }
    if !args
        .iter()
        .any(|x| matches!(x, PValue::Die(_) | PValue::DieWithTruth(..)))
    {
        return Ok(PValue::Value(Value::Vector(Vector::new(values(args)?))));
    }
    let parts = args
        .into_iter()
        .map(|x| match x {
            PValue::Die(d) | PValue::DieWithTruth(d, _) => Ok(d.items().to_vec()),
            x => Ok(vec![(value(x)?, Quantity::ONE)]),
        })
        .collect::<Result<Vec<_>, PErr>>()?;
    Ok(PValue::Die(crate::expand::vectorize_dice(&parts)?))
}

/// `icepool.Vector(items)`.
pub fn vector(items: PValue) -> Result<PValue, PErr> {
    Ok(PValue::Value(Value::Vector(Vector::new(values(py_iter(
        items,
    )?)?))))
}

/// `icepool.Symbols(x)`.
pub fn symbols(x: PValue) -> Result<PValue, PErr> {
    let s = match x {
        PValue::Value(v) => Symbols::convert(&v)?,
        x @ PValue::List(_) => Symbols::from_elements(&values(py_iter(x)?)?)?,
        _ => {
            return Err(PErr::Unimplemented(
                "Symbols() of a die or a function".into(),
            ))
        }
    };
    Ok(PValue::Value(Value::Symbols(s)))
}

/// `operator.<name>(a, b)`.
pub fn binary(a: PValue, b: PValue, op: PValue) -> Result<PValue, PErr> {
    match name(&op)? {
        n @ ("eq" | "ne") => format::eq(a, b, n == "ne"),
        n => match binary_op(n) {
            Some(op) => super::ops::binary(op, a, b),
            None => Err(PErr::Unimplemented(format!("operator {n}"))),
        },
    }
}

/// `-a`, `+a`, `~a`, `abs(a)`, `math.trunc/floor/ceil(a)`, or `round(a, ndigits)`.
pub fn unary(a: PValue, op: PValue, ndigits: PValue) -> Result<PValue, PErr> {
    match name(&op)? {
        "round" => {
            let nd = value(ndigits)?;
            match a {
                PValue::Value(v) => Ok(PValue::Value(v.round(&nd)?)),
                PValue::Die(d) => Ok(PValue::Die(d.unary_operator(|x| x.round(&nd))?)),
                _ => Err(PErr::Unimplemented("round of this kind of value".into())),
            }
        }
        n => match unary_op(n) {
            Some(op) => super::ops::unary(op, a),
            None => Err(PErr::Unimplemented(format!("unary operator {n}"))),
        },
    }
}

/// The protocols shared by both method nodes: `x[key]`, `len(x)`, `repr(x)`, `str(x)`, `tuple(x)`.
fn protocol(x: &PValue, name: &str, arg: &PValue) -> Option<Result<PValue, PErr>> {
    let v = match x {
        PValue::Value(v) => v,
        _ => return Some(Err(PErr::Unimplemented("methods of non-values".into()))),
    };
    let result = || -> Result<PValue, PErr> {
        Ok(PValue::Value(match name {
            "getitem" => getitem(v, &Index::Item(value(arg.clone())?))?,
            "slice" => {
                let bounds = values(py_iter(arg.clone())?)?;
                let [start, stop, step]: [Value; 3] = bounds
                    .try_into()
                    .map_err(|_| PErr::Unimplemented("a slice of other than 3 bounds".into()))?;
                getitem(v, &Index::Slice(start, stop, step))?
            }
            "len" => Value::from(py_len(v)? as i64),
            "repr" => Value::str(&py_repr(v)?),
            "str" => Value::str(&py_str(v)?),
            "iter" | "keys" => {
                return Ok(PValue::tuple(py_iter(x.clone())?));
            }
            _ => unreachable!("only called with these names"),
        }))
    };
    match name {
        "getitem" | "slice" | "len" | "repr" | "str" | "iter" | "keys" => Some(result()),
        _ => None,
    }
}

/// A `Vector`'s methods (`append`, `concatenate`) and protocols.
pub fn vector_method(v: PValue, method: PValue, arg: PValue) -> Result<PValue, PErr> {
    let method = name(&method)?;
    if let Some(r) = protocol(&v, method, &arg) {
        return r;
    }
    let PValue::Value(Value::Vector(vec)) = &v else {
        return attribute_error(&v, method);
    };
    let mut items = vec.data()?.to_vec();
    match method {
        // `Vector(self._data + (other, ))`
        "append" => items.push(value(arg)?),
        // `Vector(itertools.chain(self, other))`
        "concatenate" => items.extend(values(py_iter(arg)?)?),
        _ => return Err(PErr::Unimplemented(format!("Vector.{method}"))),
    }
    Ok(PValue::Value(Value::Vector(Vector::new(items))))
}

/// A `Symbols`' methods and protocols.
pub fn symbols_method(s: PValue, method: PValue, arg: PValue) -> Result<PValue, PErr> {
    let method = name(&method)?;
    if let Some(r) = protocol(&s, method, &arg) {
        return r;
    }
    let PValue::Value(Value::Symbols(sym)) = &s else {
        // `getattr(x, 'a')` on anything else, as `x.values()`, `x.size()`...: no such attribute.
        let attr = match (method, &arg) {
            ("getattr", PValue::Value(Value::Str(a))) => a.to_string(),
            _ => method.to_string(),
        };
        return attribute_error(&s, &attr);
    };
    let sv = Value::Symbols(sym.clone());
    let op = |op: BinOp, arg: PValue| -> Result<PValue, PErr> {
        Ok(PValue::Value(sv.binary(op, &value(arg)?)?))
    };
    let bool_value = |b: bool| Ok(PValue::Value(Value::Bool(b)));
    match method {
        // `self <= other`, `self >= other`, `self ^ other`, `self * other`, `self % other`.
        "issubset" => op(BinOp::Le, arg),
        "issuperset" => op(BinOp::Ge, arg),
        "symmetric_difference" => op(BinOp::Xor, arg),
        "multiply_counts" => op(BinOp::Mul, arg),
        "modulo_counts" => op(BinOp::Mod, arg),
        "divide_counts" => Ok(PValue::Value(sym.divide_counts(&value(arg)?)?)),
        "isdisjoint" => bool_value(sym.isdisjoint(&value(arg)?)?),
        "has_negative_counts" => bool_value(sym.has_negative_counts()?),
        "size" => Ok(PValue::Value(sym.size()?)),
        "count_subset" => {
            let [divisor, empty]: [Value; 2] = values(py_iter(arg)?)?
                .try_into()
                .map_err(|_| PErr::Unimplemented("count_subset arguments".into()))?;
            Ok(PValue::Value(sym.count_subset(&divisor, Some(&empty))?))
        }
        // `functools.reduce(operator.add, args, initial=self)`: `reduce` takes no keyword arguments before
        // Python 3.14, so these always raise there (DIVERGENCES.md).
        "additive_union" | "difference" | "intersection" | "union" => {
            raise(Error::Type("reduce() takes no keyword arguments".into()))
        }
        // `s.<name>` for a name that is not an attribute: `__getattr__`, which is `s[name]` unless the name starts
        // with an underscore (and `key[0]` of an empty name raises IndexError).
        "getattr" => match value(arg)? {
            Value::Str(a) if a.is_empty() => {
                raise(Error::Index("string index out of range".into()))
            }
            Value::Str(a) if a.starts_with('_') => attribute_error(&s, &a),
            Value::Str(a) => Ok(PValue::Value(sym.getitem(&a))),
            _ => type_error("attribute name must be string"),
        },
        "values" => Ok(PValue::tuple(
            sym.items()
                .iter()
                .map(|(_, n)| PValue::Value(n.clone()))
                .collect(),
        )),
        _ => Err(PErr::Unimplemented(format!("Symbols.{method}"))),
    }
}
