//! The formatting, sampling and equality nodes (harness/grammar/format.py): `format(die, spec)` / `str(die)`,
//! `format_probability_inverse`, `common_outcome_length`, seeded `sample`, `==` / `!=` (`DieWithTruth`) and their
//! truth values, `equals`, and `hash_key` comparisons.

use super::catalogue::truthy;
use super::collection::attribute_error;
use super::ops::{missing_attribute, py_iter, to_die, wrong_type};
use super::{PErr, PValue};
use icepool_core::random::Random;
use icepool_core::value::{PyDie, common_outcome_length as outcome_length, format_population, Formatted, BinOp};
use icepool_core::{Die, Error, Quantity, Value};

/// A die argument with Python's `type(die).__name__`.
fn named_die(die: &PValue) -> Result<(&Die<Value>, &'static str), PErr> {
    match die {
        PValue::Die(d) => Ok((d, "Die")),
        PValue::DieWithTruth(d, _) => Ok((d, "DieWithTruth")),
        _ => Err(PErr::Unimplemented(
            "format() of something not a die".into(),
        )),
    }
}

/// `format(die, spec)`; `None` for `str(die)` (which is `format(die, '')`). For a plain value only `str()` is
/// ported (its own format mini-language is not).
pub fn format(die: PValue, spec: PValue) -> Result<PValue, PErr> {
    let spec = match spec {
        PValue::Value(Value::None) => String::new(),
        PValue::Value(Value::Str(s)) => s.to_string(),
        _ => return wrong_type("format() argument 2 must be str"),
    };
    if let (PValue::Value(v), "") = (&die, spec.as_str()) {
        return Ok(PValue::Value(Value::str(
            &icepool_core::value::pystr::py_str(v)?,
        )));
    }
    let (d, type_name) = named_die(&die)?;
    match format_population(d, type_name, &spec, false)? {
        Formatted::Text(s) => Ok(PValue::Value(Value::str(&s))),
        Formatted::CsvTable(..) => unreachable!("only asked for with csv writer arguments"),
    }
}

/// `icepool.format_probability_inverse(probability, *args)`: `args` is `[]` or `[int_start]`.
pub fn format_probability_inverse(probability: PValue, args: PValue) -> Result<PValue, PErr> {
    let PValue::Value(p) = probability else {
        return Err(PErr::Unimplemented(
            "format_probability_inverse of a non-value".into(),
        ));
    };
    let int_start = match &py_iter(args)?[..] {
        [] => Value::from(20),
        [PValue::Value(v)] => v.clone(),
        [_] => return Err(PErr::Unimplemented("a non-value int_start".into())),
        _ => {
            return wrong_type(
                "format_probability_inverse() takes from 1 to 2 positional arguments",
            )
        }
    };
    Ok(PValue::Value(Value::str(
        &icepool_core::value::format_probability_inverse(&p, &int_start)?,
    )))
}

/// `die.common_outcome_length()`: an int or `None`.
pub fn common_outcome_length(die: PValue) -> Result<PValue, PErr> {
    // A plain value (e.g. the `()` that `die.sequence(0)` is) has no such method.
    if let PValue::Value(_) = die {
        return attribute_error(&die, "common_outcome_length");
    }
    let (d, _) = named_die(&die)?;
    Ok(PValue::Value(match outcome_length(d)? {
        Some(n) => Value::from(n as i64),
        None => Value::None,
    }))
}

/// `random.seed(seed); tuple(die.sample() for _ in range(n))`.
pub fn sample(die: PValue, seed: PValue, n: PValue) -> Result<PValue, PErr> {
    // `random.seed` takes a bool as the int it is; other seed types hash or convert differently (not ported).
    let seed = match seed {
        PValue::Value(Value::Int(i)) => i,
        PValue::Value(Value::Bool(b)) => (b as u8).into(),
        _ => return Err(PErr::Unimplemented("a non-int random seed".into())),
    };
    let n = match n {
        PValue::Value(Value::Int(n)) => i64::try_from(&n).unwrap_or(i64::MAX).max(0),
        _ => return Err(PErr::Unimplemented("a non-int sample count".into())),
    };
    // `die.sample()` is looked up lazily in the generator: on a plain value it is an AttributeError, but only
    // once the generator runs, so with `n <= 0` the result is `()`. `Symbols.__getattr__` answers any public name
    // with `self['sample']`, the sub-`Symbols` of those letters, which is then not callable.
    if let PValue::Value(v) = &die {
        if n == 0 {
            return Ok(PValue::Value(Value::tuple(Vec::new())));
        }
        if let Value::Symbols(_) = v {
            return wrong_type("'Symbols' object is not callable");
        }
        return attribute_error(&die, "sample");
    }
    let (d, _) = named_die(&die)?;
    let mut random = Random::new(&seed);
    let den: Quantity = d.denominator();
    let mut out = Vec::new();
    for _ in 0..n {
        out.push(d.sample_at(&random.randrange(&den)?)?);
    }
    Ok(PValue::Value(Value::tuple(out)))
}

/// `a == b` (`ne`: `a != b`). With a die on either side, a `DieWithTruth`: its data is the outcome-wise `==` of
/// the die and the other operand converted to a die (Python reflects `x == die` to `die == x`), its truth value
/// whether the die `equals()` the *unconverted* operand (so a die is never equal to a plain outcome). Between plain
/// values, Python's `==`.
pub fn eq(a: PValue, b: PValue, ne: bool) -> Result<PValue, PErr> {
    let is_die = |x: &PValue| matches!(x, PValue::Die(_) | PValue::DieWithTruth(..));
    let (die, other) = match (a, b) {
        (a @ PValue::Again(_), b) | (a, b @ PValue::Again(_)) => return super::again::eq(a, b, ne),
        (a, b) if is_die(&a) => (a, b),
        (a, b) if is_die(&b) => (b, a),
        (PValue::Value(a), PValue::Value(b)) => return Ok(PValue::Value(a.eq_op(&b, ne)?)),
        // `Reroll` is an enum member: equal only to itself.
        (a @ (PValue::Reroll | PValue::Restart | PValue::Value(_)), b)
            if matches!(b, PValue::Reroll | PValue::Restart | PValue::Value(_)) =>
        {
            let same = super::catalogue::py_eq(&a, &b)?;
            return Ok(PValue::Value(Value::Bool(same != ne)));
        }
        _ => return Err(PErr::Unimplemented("== between non-values".into())),
    };
    let die = to_die(die)?;
    let equals = match &other {
        PValue::Die(o) | PValue::DieWithTruth(o, _) => die.equals(o, false)?,
        _ => false,
    };
    let other = to_die(other)?;
    Ok(match die.binary_operator(&other, |x, y| x.eq_op(y, ne)) {
        Ok(data) => PValue::DieWithTruth(data, equals != ne),
        Err(Error::Unported(what)) => return Err(PErr::Unimplemented(what)),
        Err(e) => PValue::DieWithTruthError(e, equals != ne),
    })
}

/// `bool(x)`: a `DieWithTruth`'s truth value; a plain die raises.
pub fn truth(x: PValue) -> Result<PValue, PErr> {
    Ok(PValue::Value(Value::Bool(truthy(&x)?)))
}

/// `a.equals(b, simplify=simplify)`: `False` if `b` is not a die.
pub fn equals(a: PValue, b: PValue, simplify: PValue) -> Result<PValue, PErr> {
    // `if not isinstance(other, Die): return False` comes before any data is computed: a `DieWithTruth` whose data
    // raises only raises once both are dice (their `hash_key`s).
    let is_die = |x: &PValue| {
        matches!(
            x,
            PValue::Die(_) | PValue::DieWithTruth(..) | PValue::DieWithTruthError(..)
        )
    };
    if let PValue::DieWithTruthError(e, _) = &a {
        return if is_die(&b) {
            Err(e.clone().into())
        } else {
            Ok(PValue::Value(Value::Bool(false)))
        };
    }
    if let (true, PValue::DieWithTruthError(e, _)) = (is_die(&a), &b) {
        return Err(e.clone().into());
    }
    let Ok((a, _)) = named_die(&a) else {
        return missing_attribute("'equals'");
    };
    let simplify = truthy(&simplify)?;
    Ok(PValue::Value(Value::Bool(match &b {
        PValue::Die(b) | PValue::DieWithTruth(b, _) => a.equals(b, simplify)?,
        _ => false,
    })))
}

/// `x.equals(x)` on the same object. `Die.equals` and `MaybeHashKeyed.equals` (decks, multiset expressions and
/// generators, Again expressions) both start with `if self is other: return True`, before looking at any data
/// (so even a `DieWithTruth` whose data raises answers). Other values have no `equals`.
pub fn equals_self(x: PValue) -> Result<PValue, PErr> {
    match x {
        PValue::Die(_)
        | PValue::DieWithTruth(..)
        | PValue::DieWithTruthError(..)
        | PValue::Deck(_)
        | PValue::Pool(_)
        | PValue::Deal(_)
        | PValue::MultiDeal(_)
        | PValue::Expr(_)
        | PValue::Again(_) => Ok(PValue::Value(Value::Bool(true))),
        PValue::MultisetFunction(_) | PValue::RawResult(_) => {
            Err(PErr::Unimplemented("equals of this object".into()))
        }
        x => attribute_error(&x, "equals"),
    }
}

/// `a.hash_key <cmp> b.hash_key` for two dice.
pub fn hash_key_compare(a: PValue, b: PValue, cmp: PValue) -> Result<PValue, PErr> {
    let (Ok((a, _)), Ok((b, _))) = (named_die(&a), named_die(&b)) else {
        return missing_attribute("'hash_key'");
    };
    let PValue::Value(Value::Str(cmp)) = cmp else {
        return Err(PErr::Unimplemented("a hash_key comparison by name".into()));
    };
    let result = match &*cmp {
        "==" => Value::Bool(a == b),
        "!=" => Value::Bool(a != b),
        "<" => a.hash_key_compare(b, BinOp::Lt)?,
        "<=" => a.hash_key_compare(b, BinOp::Le)?,
        ">" => a.hash_key_compare(b, BinOp::Gt)?,
        ">=" => a.hash_key_compare(b, BinOp::Ge)?,
        _ => return Err(PErr::Unimplemented(format!("hash_key comparison {cmp}"))),
    };
    Ok(PValue::Value(result))
}
