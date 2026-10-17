//! The statistics nodes (harness/grammar/statistics.py): population/base.py's quantities, probabilities,
//! statistics, marginals, `group_by` and quantity management, over the core's `population.rs`.
//!
//! This layer does Python's argument juggling (optional arguments, `getattr` names, `star` inference); the core
//! does the rest.

use super::catalogue::{truthy, Func};
use super::map::{call, select, star_arg, the_die, the_population};
use super::ops::{missing_attribute, py_iter, unhashable_list, weights_die, wrong_type, Key};
use super::{PErr, PValue};
use icepool_core::population::{check_public_attr, Comparison, Data};
use icepool_core::value::builtins::{getattr, getitem, Index};
use icepool_core::{Die, Value};

/// An argument that must be a plain value (an outcome, a number): the generators never pass dice here.
fn value(x: PValue) -> Result<Value, PErr> {
    match x {
        PValue::Value(v) => Ok(v),
        _ => Err(PErr::Unimplemented(
            "a non-value statistics argument".into(),
        )),
    }
}

/// `quantities(comparison)` / `probabilities(comparison)`: `None` means `'=='`.
fn comparison_or_eq(c: PValue) -> Result<Comparison, PErr> {
    match value(c)? {
        Value::None => Ok(Comparison::Eq),
        c => Ok(Comparison::parse(&c)?),
    }
}

fn int(i: impl Into<dashu_int::IBig>) -> PValue {
    PValue::Value(Value::Int(i.into()))
}

fn tuple_of(values: Vec<Value>) -> PValue {
    PValue::Value(Value::tuple(values))
}

/// `tuple(die.quantities(comparison))`.
pub fn quantities(die: PValue, comparison: PValue) -> Result<PValue, PErr> {
    let die = the_population(die)?;
    let qs = die.quantities_by(comparison_or_eq(comparison)?);
    Ok(tuple_of(
        qs.into_iter().map(|q| Value::Int(q.into())).collect(),
    ))
}

/// `die.probabilities(comparison, percent=percent)`.
pub fn probabilities(die: PValue, comparison: PValue, percent: PValue) -> Result<PValue, PErr> {
    let die = the_population(die)?;
    let c = comparison_or_eq(comparison)?;
    Ok(tuple_of(die.probabilities(c, truthy(&percent)?)?))
}

/// The positional arguments of `quantity` / `probability`: `(outcome)` or `(comparison, outcome)`, where an
/// outcome of `None` means the one-argument form (`outcome = comparison; comparison = '=='`).
fn query(args: PValue) -> Result<(Comparison, Value), PErr> {
    let mut args = py_iter(args)?.into_iter().map(value);
    let (first, second) = match (args.next(), args.next(), args.next()) {
        (Some(a), b, None) => (a?, b.transpose()?),
        _ => return wrong_type("quantity() takes 1 or 2 positional arguments"),
    };
    Ok(match second {
        None | Some(Value::None) => (Comparison::Eq, first),
        Some(outcome) => (Comparison::parse(&first)?, outcome),
    })
}

/// `die.quantity(*args)`.
pub fn quantity(die: PValue, args: PValue) -> Result<PValue, PErr> {
    let die = the_population(die)?;
    let (c, outcome) = query(args)?;
    Ok(int(die.try_quantity(c, &outcome)?))
}

/// `die.probability(*args, percent=percent)`.
pub fn probability(die: PValue, args: PValue, percent: PValue) -> Result<PValue, PErr> {
    let die = the_population(die)?;
    let (c, outcome) = query(args)?;
    Ok(PValue::Value(die.probability(
        c,
        &outcome,
        truthy(&percent)?,
    )?))
}

/// `die.nearest(comparison, outcome)`: `None` if no outcome fits.
pub fn nearest(die: PValue, comparison: PValue, outcome: PValue) -> Result<PValue, PErr> {
    let die = the_population(die)?;
    let c = Comparison::parse(&value(comparison)?)?;
    let found = die.try_nearest(c, &value(outcome)?)?;
    Ok(PValue::Value(found.unwrap_or(Value::None)))
}

/// `getattr(die, stat)()` for the methods without arguments.
pub fn die_stat(die: PValue, stat: PValue) -> Result<PValue, PErr> {
    let die = the_population(die)?;
    let Value::Str(stat) = value(stat)? else {
        return wrong_type("attribute name must be string");
    };
    let half = (Value::from(1), Value::from(2));
    Ok(PValue::Value(match &*stat {
        "mean" => die.mean()?,
        "variance" => die.variance()?,
        "standard_deviation" | "sd" => die.standard_deviation()?,
        "skewness" => die.skewness()?,
        "excess_kurtosis" => die.excess_kurtosis()?,
        "mode" => Value::tuple(die.mode()),
        "modal_quantity" => Value::Int(die.modal_quantity()?.into()),
        "median" => die.quantile(&half.0, &half.1)?,
        "median_low" => die.quantile_low(&half.0, &half.1)?,
        "median_high" => die.quantile_high(&half.0, &half.1)?,
        "min_outcome" => die.min_outcome()?.clone(),
        "max_outcome" => die.max_outcome()?.clone(),
        "denominator" => Value::Int(die.denominator().into()),
        "is_empty" => Value::Bool(die.is_empty()),
        other => return Err(PErr::Unimplemented(format!("die statistic {other}"))),
    }))
}

/// `die.standardized_moment(k)`.
pub fn standardized_moment(die: PValue, k: PValue) -> Result<PValue, PErr> {
    let die = the_population(die)?;
    Ok(PValue::Value(die.standardized_moment(&value(k)?)?))
}

/// `die.entropy(*base)`; the base defaults to `2.0`.
pub fn entropy(die: PValue, base: PValue) -> Result<PValue, PErr> {
    let die = the_population(die)?;
    let base = match <[PValue; 1]>::try_from(py_iter(base)?) {
        Ok([b]) => value(b)?,
        Err(v) if v.is_empty() => Value::Float(2.0),
        Err(_) => return wrong_type("entropy() takes at most 1 argument"),
    };
    Ok(PValue::Value(die.entropy(&base)?))
}

/// `getattr(die, method)(*args)` for `quantile`, `quantile_low`, `quantile_high`: `(n)` or `(n, d)`, `d`
/// defaulting to 100.
pub fn quantile(die: PValue, method: PValue, args: PValue) -> Result<PValue, PErr> {
    let die = the_population(die)?;
    let mut args = py_iter(args)?.into_iter().map(value);
    let (n, d) = match (args.next(), args.next(), args.next()) {
        (Some(n), d, None) => (n?, d.transpose()?.unwrap_or(Value::from(100))),
        _ => return wrong_type("quantile() takes 1 or 2 positional arguments"),
    };
    Ok(PValue::Value(match &value(method)? {
        Value::Str(m) if &**m == "quantile" => die.quantile(&n, &d)?,
        Value::Str(m) if &**m == "quantile_low" => die.quantile_low(&n, &d)?,
        Value::Str(m) if &**m == "quantile_high" => die.quantile_high(&n, &d)?,
        _ => return Err(PErr::Unimplemented("this quantile method".into())),
    }))
}

/// `getattr(a, stat)(b)` for `kolmogorov_smirnov` and `cramer_von_mises`.
pub fn cdf_distance(a: PValue, b: PValue, stat: PValue) -> Result<PValue, PErr> {
    let (a, b) = (the_die(a)?, the_die(b)?);
    Ok(PValue::Value(match &value(stat)? {
        Value::Str(s) if &**s == "kolmogorov_smirnov" => a.kolmogorov_smirnov(&b)?,
        Value::Str(s) if &**s == "cramer_von_mises" => a.cramer_von_mises(&b)?,
        _ => return Err(PErr::Unimplemented("this distance".into())),
    }))
}

/// `die.marginals[index]`.
pub fn marginal(die: PValue, index: PValue) -> Result<PValue, PErr> {
    Ok(PValue::Die(
        the_die(die)?.marginal(&Index::Item(value(index)?))?,
    ))
}

/// `die.marginals[start:stop:step]`.
pub fn marginal_slice(
    die: PValue,
    start: PValue,
    stop: PValue,
    step: PValue,
) -> Result<PValue, PErr> {
    let index = Index::Slice(value(start)?, value(stop)?, value(step)?);
    Ok(PValue::Die(the_die(die)?.marginal(&index)?))
}

/// `getattr(die.marginals, name)`: `getattr(outcome, name)` applied to each outcome.
pub fn marginal_attr(die: PValue, name: PValue) -> Result<PValue, PErr> {
    let die = the_die(die)?;
    Ok(PValue::Die(die.marginal_attr(&attr_name(name)?)?))
}

/// The name passed to `getattr(obj, name)`, which must be a `str`.
fn attr_name(name: PValue) -> Result<String, PErr> {
    match value(name)? {
        Value::Str(s) => Ok(s.to_string()),
        _ => wrong_type("attribute name must be string"),
    }
}

/// `tuple(die.marginals)`: `marginals[i]` for `i` in `range(len(marginals))`.
pub fn marginals(die: PValue) -> Result<PValue, PErr> {
    let die = the_die(die)?;
    let n = die.marginals_len()?;
    let dice = (0..n)
        .map(|i| {
            Ok(PValue::Die(
                die.marginal(&Index::Item(Value::from(i as i64)))?,
            ))
        })
        .collect::<Result<_, PErr>>()?;
    Ok(PValue::tuple(dice))
}

/// A `group_by` key: any dict key a catalogue callable or mapping can return.
#[derive(Clone, PartialEq, Eq, Hash)]
enum GroupKey {
    Key(Key),
    Reroll,
}

impl GroupKey {
    fn new(x: PValue) -> Result<Self, PErr> {
        Ok(match x {
            PValue::Value(v) => GroupKey::Key(Key::Value(v)),
            PValue::Die(d) => GroupKey::Key(Key::Die(d)),
            PValue::Reroll => GroupKey::Reroll,
            PValue::List(_) => return unhashable_list(),
            _ => return Err(PErr::Unimplemented("this group_by key".into())),
        })
    }

    fn into_pvalue(self) -> PValue {
        match self {
            GroupKey::Key(Key::Value(v)) => PValue::Value(v),
            GroupKey::Key(Key::Die(d)) => PValue::Die(d),
            GroupKey::Reroll => PValue::Reroll,
        }
    }
}

/// `tuple(result.items())` for a `group_by` result.
fn groups(die: &Die<Value>, keys: Vec<GroupKey>) -> Result<PValue, PErr> {
    let pairs = die
        .group_by(keys)?
        .into_iter()
        .map(|(k, d)| PValue::tuple(vec![k.into_pvalue(), PValue::Die(d)]))
        .collect();
    Ok(PValue::tuple(pairs))
}

/// `tuple(die.group_by(key_map, star=star).items())`: a callable is called on each outcome (unpacked if `star`,
/// inferred from its signature), a mapping looked up with `key_map.get(o, o)`.
pub fn group_by(die: PValue, key_map: PValue, star: PValue) -> Result<PValue, PErr> {
    let die = the_die(die)?;
    let PValue::Func(f) = &key_map else {
        return missing_attribute("key_map has no attribute 'get'");
    };
    let star = match f.positional() {
        Some(_) => match star_arg(&star)? {
            Some(star) => star,
            None => f.infer_star(1)?,
        },
        None => false,
    };
    let keys = die
        .outcomes()
        .map(|o| {
            let o = PValue::Value(o.clone());
            GroupKey::new(match &**f {
                Func::Mapping(_) => f.call(vec![o])?,
                _ => call(f, star, vec![o], &[])?,
            })
        })
        .collect::<Result<_, _>>()?;
    groups(&die, keys)
}

/// `tuple(die.group_by[index].items())`: grouped by `outcome[index]`.
pub fn group_by_index(die: PValue, index: PValue) -> Result<PValue, PErr> {
    let die = the_die(die)?;
    let index = Index::Item(value(index)?);
    let keys = die
        .outcomes()
        .map(|o| Ok(GroupKey::Key(Key::Value(getitem(o, &index)?))))
        .collect::<Result<_, PErr>>()?;
    groups(&die, keys)
}

/// `tuple(getattr(die.group_by, name).items())`: `_GroupBy.__getattr__`, i.e. `die.group_by(lambda x:
/// getattr(x, name))` (one parameter: `star` is inferred false), private names refused first.
pub fn group_by_attr(die: PValue, name: PValue) -> Result<PValue, PErr> {
    let die = the_die(die)?;
    let name = attr_name(name)?;
    check_public_attr(&name)?;
    let keys = die
        .outcomes()
        .map(|o| Ok(GroupKey::Key(Key::Value(getattr(o, &name)?))))
        .collect::<Result<_, PErr>>()?;
    groups(&die, keys)
}

/// `getattr(die, stat)(i, j)` for `covariance` and `correlation`.
pub fn joint_stat(die: PValue, stat: PValue, i: PValue, j: PValue) -> Result<PValue, PErr> {
    let die = the_die(die)?;
    let (i, j) = (Index::Item(value(i)?), Index::Item(value(j)?));
    Ok(PValue::Value(match &value(stat)? {
        Value::Str(s) if &**s == "covariance" => die.covariance(&i, &j)?,
        Value::Str(s) if &**s == "correlation" => die.correlation(&i, &j)?,
        _ => return Err(PErr::Unimplemented("this joint statistic".into())),
    }))
}

/// `Die(data)`, or the die itself where Python returns `self`.
pub(super) fn new_or_self(die: Die<Value>, data: Option<Data>) -> Result<PValue, PErr> {
    match data {
        None => Ok(PValue::Die(die)),
        Some(data) => {
            let (keys, values) = data
                .into_iter()
                .map(|(o, q)| (PValue::Value(o), PValue::Value(q)))
                .unzip();
            weights_die(keys, values)
        }
    }
}

/// `getattr(die, method)(x)` for `multiply_quantities`, `divide_quantities` and `modulo_quantities`.
pub fn quantities_op(die: PValue, method: PValue, x: PValue) -> Result<PValue, PErr> {
    let die = the_die(die)?;
    let x = value(x)?;
    let data = match &value(method)? {
        Value::Str(m) if &**m == "multiply_quantities" => die.multiply_quantities(&x)?,
        Value::Str(m) if &**m == "divide_quantities" => die.divide_quantities(&x)?,
        Value::Str(m) if &**m == "modulo_quantities" => Some(die.modulo_quantities(&x)?),
        _ => return Err(PErr::Unimplemented("this quantity operation".into())),
    };
    new_or_self(die, data)
}

/// `die.pad_to_denominator(denominator, outcome=outcome)`.
pub fn pad_to_denominator(
    die: PValue,
    denominator: PValue,
    outcome: PValue,
) -> Result<PValue, PErr> {
    let die = the_die(die)?;
    let data = die.pad_to_denominator(&value(denominator)?, &value(outcome)?)?;
    new_or_self(die, Some(data))
}

/// `die.multiply_to_denominator(denominator)`.
pub fn multiply_to_denominator(die: PValue, denominator: PValue) -> Result<PValue, PErr> {
    let die = the_die(die)?;
    let data = die.multiply_to_denominator(&value(denominator)?)?;
    new_or_self(die, data)
}

/// The outcomes a `quantity_where` / `probability_where` callable selects (called on each outcome in order).
fn selected(die: &Die<Value>, which: &PValue, star: &PValue) -> Result<Vec<bool>, PErr> {
    match which {
        PValue::Func(f) if f.positional().is_some() => select(die, which, star),
        _ => wrong_type("'which' object is not callable"),
    }
}

/// `die.quantity_where(which, star=star)`.
pub fn quantity_where(die: PValue, which: PValue, star: PValue) -> Result<PValue, PErr> {
    let die = the_die(die)?;
    let mask = selected(&die, &which, &star)?;
    Ok(int(die.quantity_where(&mask)))
}

/// `die.probability_where(which, star=star, percent=percent)`.
pub fn probability_where(
    die: PValue,
    which: PValue,
    star: PValue,
    percent: PValue,
) -> Result<PValue, PErr> {
    let die = the_die(die)?;
    let mask = selected(&die, &which, &star)?;
    Ok(PValue::Value(
        die.probability_where(&mask, truthy(&percent)?)?,
    ))
}
