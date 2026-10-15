//! `icepool.map` (map_tools/function.py, core_impl.py `map_simple`) and the outcome-selecting `Die` methods:
//! `reroll`, `filter`, `split` (population/base.py `_select_outcomes`), `clip`, `truncate`. Also `tupleize`
//! (expand.py `cartesian_product`), which `expand_arg` applies to tuples holding dice.
//!
//! The core does the arithmetic; this layer does Python's calling conventions: inferring `star`, unpacking
//! outcomes, membership tests, and the order in which Python would raise.

use dashu_int::IBig;

use super::catalogue::{py_eq, truthy, Func};
use super::expression::as_expression;
use super::ops::{die_arg, py_iter, type_error, Key};
use super::{raise, PErr, PValue};
use crate::creation_args::DieArg;
use crate::error::Error;
use crate::expand::iter_cartesian_product;
use crate::{Die, Quantity, Value};

/// What `repl` is: `transition_and_star` accepts a callable or a mapping.
enum Repl<'a> {
    Callable(&'a Func),
    Mapping(&'a Func),
}

fn repl(f: &PValue) -> Result<Repl<'_>, PErr> {
    match f {
        PValue::Func(f) if f.positional().is_some() => Ok(Repl::Callable(f)),
        PValue::Func(f) => Ok(Repl::Mapping(f)),
        // A die is a Mapping too (outcome -> quantity); nobody maps with one.
        PValue::Die(_) => Err(PErr::Unimplemented("a Die as repl".into())),
        _ => type_error("repl must be a callable or a mapping."),
    }
}

/// `star` as given: `None` means infer it; anything else is used for its truth value.
pub(super) fn star_arg(star: &PValue) -> Result<Option<bool>, PErr> {
    match star {
        PValue::Value(Value::None) => Ok(None),
        x => truthy(x).map(Some),
    }
}

/// Calls `f` on one joint outcome, unpacking the first argument if `star` (`transition(*first, *extra,
/// **kwargs)`).
pub(super) fn call(
    f: &Func,
    star: bool,
    mut args: Vec<PValue>,
    kwargs: &[(String, PValue)],
) -> Result<PValue, PErr> {
    if star {
        let rest = args.split_off(1);
        let first = args.pop().expect("at least one argument");
        // `*x` accepts any iterable; a non-iterable raises TypeError, as `py_iter` does.
        let mut unpacked = py_iter(first)?;
        unpacked.extend(rest);
        args = unpacked;
    }
    f.call_kw(args, kwargs.to_vec())
}

/// `map`'s `**kwargs` as the `map_kw` node gives them: a list of `[name, value]` pairs (a dict comprehension, so
/// a repeated name keeps the last value).
pub fn kwargs(pairs: PValue) -> Result<Vec<(String, PValue)>, PErr> {
    let mut out: Vec<(String, PValue)> = Vec::new();
    for pair in py_iter(pairs)? {
        let [name, value] = <[PValue; 2]>::try_from(py_iter(pair)?)
            .map_err(|_| PErr::Unimplemented("a keyword argument that is not a pair".into()))?;
        let PValue::Value(Value::Str(name)) = name else {
            return type_error("keywords must be strings");
        };
        match out.iter_mut().find(|(n, _)| **n == *name) {
            Some(slot) => slot.1 = value,
            None => out.push((name.to_string(), value)),
        }
    }
    Ok(out)
}

/// `iter_cartesian_product`'s view of one argument: a population's items (`_items_for_cartesian_product`: dice
/// and decks), or the argument itself with quantity 1.
pub(super) fn arg_items(arg: PValue) -> Vec<(PValue, Quantity)> {
    let items = |items: &[(Value, Quantity)]| {
        items
            .iter()
            .map(|(o, q)| (PValue::Value(o.clone()), q.clone()))
            .collect()
    };
    match arg {
        PValue::Die(d) | PValue::DieWithTruth(d, _) => items(d.items()),
        PValue::Deck(d) => items(d.items()),
        x => vec![(x, Quantity::ONE)],
    }
}

/// `arg.expand() if isinstance(arg, icepool.MultisetExpression) else arg`: `map` and `map_and_time` turn a
/// multiset expression argument into the die of its sorted tuples before anything else.
pub(super) fn expand_arg(arg: PValue) -> Result<PValue, PErr> {
    if as_expression(&arg).is_some() {
        super::evaluate::expand(arg, PValue::Value(Value::from(1)))
    } else {
        Ok(arg)
    }
}

/// `icepool.map_function(f, star=star, **kwargs)(*args)` or, as a decorator with arguments,
/// `icepool.map_function(star=star, **kwargs)(f)(*args)`: both are `functools.partial(map, f, ...)`.
///
/// Given the function directly, map_function binds only `**kwargs` (`partial(map, function, **kwargs)`): `star`,
/// `repeat` and the `again_*` arguments are dropped, so `star` is inferred as for a plain `map`. The decorator
/// form binds them all.
pub fn map_function(
    f: PValue,
    args: PValue,
    star: PValue,
    kw: PValue,
    decorator: PValue,
) -> Result<PValue, PErr> {
    let star = match decorator {
        PValue::Value(Value::Bool(true)) => star,
        PValue::Value(Value::Bool(false)) => PValue::Value(Value::None),
        _ => return Err(PErr::Unimplemented("map_function decorator flag".into())),
    };
    map(f, args, star, kwargs(kw)?)
}

/// `icepool.map(repl, *args, star=star)` (no `repeat`, no `Again`).
///
/// `repl` is called with every joint outcome of `args` (dice expand into their outcomes, zero quantities
/// included; anything else is passed as-is), and the results become `Die(results, quantities)`: results that are
/// dice are mixed in with `weighted_lcm`, `Reroll` (and `Restart`) drops out, tuples holding dice are `tupleize`d,
/// and a `Break` is unwrapped (`map_simple`). `kwargs` go to `repl` as they are (dice are not expanded).
pub fn map(
    f: PValue,
    args: PValue,
    star: PValue,
    kwargs: Vec<(String, PValue)>,
) -> Result<PValue, PErr> {
    let PValue::List(args) = args else {
        return Err(PErr::Unimplemented(
            "map arguments not given as a list".into(),
        ));
    };
    let star = star_arg(&star)?;
    let repl = repl(&f)?;
    if args.is_empty() {
        let result = match repl {
            Repl::Mapping(_) => {
                return raise(Error::Value(
                    "If no arguments are given, repl must be a callable.".into(),
                ))
            }
            // `Die([repl(**kwargs)])`: not `map_simple`, so a `Break` is not unwrapped.
            Repl::Callable(f) => f.call_kw(vec![], kwargs)?,
        };
        return die_of(vec![result], vec![Quantity::ONE]);
    }
    let args = args
        .into_iter()
        .map(expand_arg)
        .collect::<Result<Vec<_>, _>>()?;
    // transition_and_star.
    let (f, star) = match repl {
        Repl::Callable(f) => {
            let star = match star {
                Some(star) => star,
                None => f.infer_star(args.len())?,
            };
            (f, star)
        }
        Repl::Mapping(_) if args.len() != 1 => {
            return raise(Error::Value(
                "If a mapping is provided for repl, len(args) must be 1.".into(),
            ))
        }
        Repl::Mapping(f) => (f, false),
    };
    let items: Vec<_> = args.into_iter().map(arg_items).collect();
    let mut final_states = Vec::new();
    let mut final_quantities = Vec::new();
    for (outcomes, quantity) in iter_cartesian_product(&items) {
        let first = outcomes[0].clone();
        let final_state = match call(f, star, outcomes, &kwargs)? {
            // `Break()` stands for the first outcome (before unpacking), `Break(x)` for `x`.
            PValue::Break(outcome) => match *outcome {
                PValue::Value(Value::None) => first,
                outcome => outcome,
            },
            state => state,
        };
        final_states.push(final_state);
        final_quantities.push(quantity);
    }
    die_of(final_states, final_quantities)
}

/// `icepool.Die(outcomes, times)` for objects and int times.
pub(super) fn die_of(outcomes: Vec<PValue>, times: Vec<Quantity>) -> Result<PValue, PErr> {
    let args = outcomes
        .into_iter()
        .map(die_arg)
        .collect::<Result<_, _>>()?;
    Ok(PValue::Die(Die::new(args, times)?))
}

/// `cartesian_product(*items, outcome_type=tuple)` for a tuple that holds something other than plain values.
///
/// If any element is a die, the result is a die of tuples, one per joint outcome. (Equal tuples, e.g. `(1,)` and `(True,)`, overwrite each other's quantity in the dict
/// it builds, keeping the first key: `data[outcome] = quantity`, not `+=`.)
pub fn tupleize(items: Vec<PValue>) -> Result<DieArg<Value>, PErr> {
    // `cartesian_product` returns `Reroll` if any element is `Reroll`, but `expand_arg` then hands that on as if
    // it were a mapping, and `merge_weights_lcm` raises AttributeError on `Reroll.values()` (see DIVERGENCES.md).
    if items
        .iter()
        .any(|x| matches!(x, PValue::Reroll | PValue::Restart))
    {
        return type_error("AttributeError: 'RerollType' object has no attribute 'values'");
    }
    if !items.iter().any(|x| matches!(x, PValue::Die(_))) {
        // A tuple holding a list or a nested tuple of dice: an unhashable or die-valued outcome.
        return Err(PErr::Unimplemented(
            "tuple outcomes holding lists or dice".into(),
        ));
    }
    let mut parts = Vec::with_capacity(items.len());
    for x in items {
        parts.push(match x {
            PValue::Die(d) => d.items().to_vec(),
            PValue::Value(v) => vec![(v, Quantity::ONE)],
            _ => {
                return Err(PErr::Unimplemented(
                    "tuple outcomes holding lists or dice".into(),
                ))
            }
        });
    }
    Ok(DieArg::Die(crate::expand::tupleize_dice(&parts)?))
}

pub(super) fn the_die(die: PValue) -> Result<Die<Value>, PErr> {
    match die {
        PValue::Die(d) | PValue::DieWithTruth(d, _) => Ok(d),
        _ => type_error("AttributeError: only dice have this method"),
    }
}

/// `self` of a `Population` method that returns a value (a statistic, a quantity, a probability, a quantile): a die,
/// or a deck as the die of its outcomes and quantities, which gives the same value (`deck_stat` relies on it too).
/// Methods that return a population keep `the_die`: on a deck, Python's give a `Deck`.
pub(super) fn the_population(x: PValue) -> Result<Die<Value>, PErr> {
    match x {
        PValue::Deck(d) => Ok(d.population().clone()),
        x => the_die(x),
    }
}

/// `x in collection`, for the collections a program can give: lists, tuples, strs (substring), dice and mappings
/// (keys).
pub(super) fn contains(collection: &PValue, x: &Value) -> Result<bool, PErr> {
    match collection {
        PValue::DieWithTruthError(e, _) => Err(e.clone().into()),
        PValue::List(items) | PValue::Tuple(items) => {
            let x = PValue::Value(x.clone());
            for item in items {
                if py_eq(item, &x)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        // `x in tuple`: `==` element by element, as for a list (`Symbols('') == ()` counts).
        PValue::Value(Value::Tuple(items)) => {
            for item in items.iter() {
                if item.eq_op(x, false)?.truthy()? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        PValue::Value(Value::Str(s)) => match x {
            Value::Str(sub) => Ok(s.contains(&**sub)),
            _ => type_error("'in <string>' requires string as left operand"),
        },
        PValue::Value(v) => type_error(format!(
            "argument of type '{}' is not iterable",
            v.type_name()
        )),
        PValue::Die(d) | PValue::DieWithTruth(d, _) => Ok(d.outcomes().any(|o| o == x)),
        PValue::Func(f) => match &**f {
            Func::Mapping(m) => Ok(m.contains_key(&Key::Value(x.clone()))),
            _ => type_error("argument of type 'function' is not iterable"),
        },
        PValue::Reroll | PValue::Restart => {
            type_error("argument of type 'RerollType' is not iterable")
        }
        PValue::Break(_) => type_error("argument of type 'Break' is not iterable"),
        PValue::Again(_) => type_error("argument of type 'AgainExpression' is not iterable"),
        PValue::Ellipsis => type_error("argument of type 'ellipsis' is not iterable"),
        // A mapping: its keys.
        PValue::Deck(d) => Ok(d.outcomes().any(|o| o == x)),
        PValue::Pool(_)
        | PValue::Deal(_)
        | PValue::MultiDeal(_)
        | PValue::Expr(_)
        | PValue::MultisetFunction(_)
        | PValue::RawResult(_) => Err(PErr::Unimplemented(
            "membership in a pool or a multiset function".into(),
        )),
    }
}

/// `Population._select_outcomes(which, star)`: one flag per outcome. A callable is called on each outcome (in
/// order, unpacked if `star`, which is inferred for one argument); anything else is a collection to test
/// membership in.
pub(super) fn select(die: &Die<Value>, which: &PValue, star: &PValue) -> Result<Vec<bool>, PErr> {
    let star = star_arg(star)?;
    match which {
        PValue::Func(f) if f.positional().is_some() => {
            let star = match star {
                Some(star) => star,
                None => f.infer_star(1)?,
            };
            die.outcomes()
                .map(|o| truthy(&call(f, star, vec![PValue::Value(o.clone())], &[])?))
                .collect()
        }
        collection => die.outcomes().map(|o| contains(collection, o)).collect(),
    }
}

/// `depth`: an int (not negative) or `'inf'` (`None` here).
fn depth_arg(depth: &PValue) -> Result<Option<u32>, PErr> {
    let int = match depth {
        PValue::Value(Value::Str(s)) if &**s == "inf" => return Ok(None),
        PValue::Value(Value::Int(i)) => i.clone(),
        PValue::Value(Value::Bool(b)) => IBig::from(*b as u8),
        PValue::Value(Value::Float(_) | Value::Fraction(_)) => {
            return Err(PErr::Unimplemented("non-int reroll depth".into()))
        }
        _ => return type_error("'<' not supported between depth and 'int'"),
    };
    if int < IBig::ZERO {
        return raise(Error::Value("reroll depth cannot be negative.".into()));
    }
    // Python would compute for a very long time (or run out of memory) with a depth this large.
    u32::try_from(int)
        .map(Some)
        .map_err(|_| PErr::Unimplemented("huge reroll depth".into()))
}

/// `die.reroll(which, star=star, depth=depth)`, or `die.filter(...)`, which rerolls the outcomes *not* selected.
/// `reroll` with `which=None` rerolls the min outcome.
pub fn reroll(
    filter: bool,
    die: PValue,
    which: PValue,
    star: PValue,
    depth: PValue,
) -> Result<PValue, PErr> {
    let die = the_die(die)?;
    let mask = match (&which, filter) {
        (PValue::Value(Value::None), false) => {
            die.min_outcome()?;
            let mut m = vec![false; die.len()];
            m[0] = true;
            m
        }
        (_, false) => select(&die, &which, &star)?,
        (_, true) => select(&die, &which, &star)?
            .into_iter()
            .map(|s| !s)
            .collect(),
    };
    let depth = depth_arg(&depth)?;
    Ok(PValue::Die(die.reroll(&mask, depth)?))
}

/// `die.split(which, star=star)`: a tuple of the die of selected outcomes and the die of the rest.
pub fn split(die: PValue, which: PValue, star: PValue) -> Result<PValue, PErr> {
    let die = the_die(die)?;
    let mask = select(&die, &which, &star)?;
    let (selected, rest) = die.split(&mask)?;
    Ok(PValue::tuple(vec![
        PValue::Die(selected),
        PValue::Die(rest),
    ]))
}

/// `die.clip(min, max)` or `die.truncate(min, max)`; `None` leaves that side alone.
pub fn clip_or_truncate(clip: bool, die: PValue, min: PValue, max: PValue) -> Result<PValue, PErr> {
    let die = the_die(die)?;
    let bound = |x: PValue| match x {
        PValue::Value(Value::None) => Ok(None),
        PValue::Value(v) => Ok(Some(v)),
        _ => Err(PErr::Unimplemented(
            "clip/truncate bounds that are not values".into(),
        )),
    };
    let (min, max) = (bound(min)?, bound(max)?);
    let result = if clip {
        die.clip(min.as_ref(), max.as_ref())?
    } else {
        die.truncate(min.as_ref(), max.as_ref())?
    };
    Ok(PValue::Die(result))
}
