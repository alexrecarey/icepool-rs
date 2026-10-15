//! `icepool.map_to_pool` (map_tools/function.py) and the `Die` methods built on it (population/die.py):
//! `map_to_pool`, `explode_to_pool`, `reroll_to_pool`. Each makes a `MultisetMixture` of pools, one per joint
//! outcome of the arguments, weighted by its quantity (equal pools counted together).
//!
//! The arithmetic of the pool compositions is the core's (`crate::to_pool`); here are Python's calling
//! conventions and the pools each composition stands for.

use std::sync::Arc;

use super::catalogue::Func;
use super::expression::{add_inner, as_expression, binary_op, MExpr};
use super::map::{arg_items, call, select, star_arg, the_die};
use super::ops::{matmul_dice, type_error};
use super::pool::{die_pool, pool, pool_highest_lowest, KeepGen};
use super::{raise, PErr, PValue};
use crate::error::Error;
use crate::expand::iter_cartesian_product;
use crate::{Die, DieArg, Pool, Quantity, Value};

/// `n @ die` with a Python int (or bool) `n`: `Die([n]) @ die` (`Die.__rmatmul__`).
fn rmatmul(n: &Value, die: &Die<Value>) -> Result<Die<Value>, PErr> {
    let n = Die::new(vec![DieArg::Outcome(n.clone())], vec![Quantity::ONE])?;
    matmul_dice(&n, die)
}

/// `icepool.map_to_pool(repl, *args, star=star)` (node `map_to_pool`).
pub fn map_to_pool(f: PValue, args: PValue, star: PValue) -> Result<PValue, PErr> {
    let PValue::List(args) = args else {
        return Err(PErr::Unimplemented(
            "map_to_pool arguments not given as a list".into(),
        ));
    };
    let star = star_arg(&star)?;
    // transition_and_star: a mapping becomes `lambda o: mapping.get(o, o)`, never starred.
    let (f, star) = match &f {
        PValue::Func(func) if func.positional().is_some() => {
            let star = match star {
                Some(star) => star,
                None => func.infer_star(args.len())?,
            };
            (func, star)
        }
        PValue::Func(_) if args.len() != 1 => {
            return raise(Error::Value(
                "If a mapping is provided for repl, len(args) must be 1.".into(),
            ))
        }
        PValue::Func(mapping) => (mapping, false),
        _ => return type_error("repl must be a callable or a mapping."),
    };
    // Unlike `map`, multiset expressions among the args are not expanded: `iter_cartesian_product` passes them
    // as they are.
    let items: Vec<_> = args.into_iter().map(arg_items).collect();
    mixture_of_pools(f, star, &items)
}

/// `die.map_to_pool(repl, *extra_args, star=star)` (node `die_map_to_pool`): `icepool.map_to_pool(repl, die,
/// ...)`, where a missing `repl` is `lambda x: x` (the outcomes are the pools' contents).
pub fn die_map_to_pool(die: PValue, f: PValue, args: PValue, star: PValue) -> Result<PValue, PErr> {
    let PValue::List(extra) = args else {
        return Err(PErr::Unimplemented(
            "map_to_pool arguments not given as a list".into(),
        ));
    };
    let f = match f {
        PValue::Value(Value::None) => PValue::Func(Arc::new(Func::Identity)),
        f => f,
    };
    let die = PValue::Die(the_die(die)?);
    let args = std::iter::once(die).chain(extra).collect();
    map_to_pool(f, PValue::List(args), star)
}

/// The loop of `map_to_pool`: `repl` called with every joint outcome (the first unpacked if `star`); `Reroll`
/// (and `Restart`) drop the outcome, an expression is a pool as it is, anything else goes through `Pool(...)`.
fn mixture_of_pools(
    f: &Func,
    star: bool,
    items: &[Vec<(PValue, Quantity)>],
) -> Result<PValue, PErr> {
    let mut data = Vec::new();
    for (outcomes, quantity) in iter_cartesian_product(items) {
        if star && outcomes.is_empty() {
            // `transition_function(*outcomes[0], ...)` with no args at all.
            return raise(Error::Index("tuple index out of range".into()));
        }
        if let Some(pool) = as_pool(call(f, star, outcomes, &[])?)? {
            add_inner(&mut data, pool, quantity)?;
        }
    }
    Ok(PValue::Expr(MExpr::Mixture(Arc::new(data))))
}

/// One result of `repl`: `None` for `Reroll`/`Restart` (`pool in icepool.REROLL_TYPES`), else the expression,
/// or `icepool.Pool(result)`.
fn as_pool(result: PValue) -> Result<Option<MExpr>, PErr> {
    match result {
        PValue::Reroll | PValue::Restart => Ok(None),
        PValue::Value(Value::Vector(_)) => Err(PErr::Unimplemented(
            "a Vector as a map_to_pool result (Vector == Reroll)".into(),
        )),
        result => match as_expression(&result) {
            Some(e) => Ok(Some(e)),
            None => {
                let p = pool(result, PValue::Value(Value::from(1)))?;
                Ok(Some(as_expression(&p).expect("Pool(...) is a pool")))
            }
        },
    }
}

/// A count argument (`rolls`, `max_rerolls`, `depth`): any value, used with Python's arithmetic (`@`, `==`,
/// `-`, `**`), which raises where Python does.
fn value_arg(x: &PValue, what: &str) -> Result<Value, PErr> {
    match x {
        PValue::Value(v) => Ok(v.clone()),
        _ => Err(PErr::Unimplemented(format!("a {what} that is not a value"))),
    }
}

/// The outcomes `which` selects (`_select_outcomes`, then `split` on that set, which selects the same ones), or
/// the single `extreme` one (`{self.min_outcome()}` / `{self.max_outcome()}`) for `None`.
fn selection(
    die: &Die<Value>,
    which: &PValue,
    star: &PValue,
    max: bool,
) -> Result<Vec<bool>, PErr> {
    if let PValue::Value(Value::None) = which {
        // Raises IndexError for an empty die.
        if max {
            die.max_outcome()?;
        } else {
            die.min_outcome()?;
        }
        let mut mask = vec![false; die.len()];
        mask[if max { die.len() - 1 } else { 0 }] = true;
        return Ok(mask);
    }
    select(die, which, star)
}

/// The pool of `dice`, each `(die, count)` put in `count` times (`[die] * count + ...`: a negative count is
/// none), as `Pool(list_of_dice)` makes it.
fn pool_of(dice: Vec<(Die<Value>, &Value)>) -> Result<MExpr, PErr> {
    let mut counts = Vec::with_capacity(dice.len());
    for (die, count) in dice {
        let Value::Int(n) = count else {
            return type_error("can't multiply sequence by non-int");
        };
        // A die put in zero times would still add its outcomes to the pool's: `[die] * 0` leaves it out.
        if *n > dashu_int::IBig::ZERO {
            let n = usize::try_from(n)
                .map_err(|_| PErr::Raise(Error::Overflow("too many dice".into())))?;
            counts.push((die, n));
        }
    }
    Ok(MExpr::Gen(KeepGen::Pool(Pool::new(counts)?)))
}

/// `die.explode_to_pool(rolls, outcomes, star=star, depth=depth)` (node `explode_to_pool`): each of `rolls` dice
/// explodes up to `depth` times, each roll a separate die in the pool.
pub fn explode_to_pool(
    die: PValue,
    rolls: PValue,
    outcomes: PValue,
    star: PValue,
    depth: PValue,
) -> Result<PValue, PErr> {
    let d = the_die(die.clone())?;
    let depth = value_arg(&depth, "explode_to_pool depth")?;
    if depth.eq_op(&Value::from(0), false)?.truthy()? {
        return die_pool(die, rolls);
    }
    let mask = selection(&d, &outcomes, &star, true)?;
    if !mask.contains(&true) {
        return die_pool(die, rolls);
    }
    let (explode, not_explode) = d.split(&mask)?;
    let rolls = value_arg(&rolls, "explode_to_pool rolls")?;
    // `range(depth + 1)`.
    let depth = match &depth {
        Value::Bool(b) => *b as i64,
        Value::Int(i) => i64::try_from(i)
            .map_err(|_| PErr::Unimplemented("a huge explode_to_pool depth".into()))?,
        v => {
            return type_error(format!(
                "'{}' object cannot be interpreted as an integer",
                v.type_name()
            ))
        }
    };
    let count_die = crate::to_pool::explode_counts(
        &d.denominator(),
        &explode.denominator(),
        &not_explode.denominator(),
        &rolls,
        depth,
        rmatmul,
    )?;
    // `count_die.map_to_pool(lambda x, nx: [explode] * x + [not_explode] * nx)`.
    let mut data = Vec::new();
    for (counts, quantity) in count_die.items() {
        let Value::Vector(v) = counts else {
            unreachable!("explode_counts makes vectors")
        };
        let [x, nx] = &v.data()?[..] else {
            unreachable!("explode_counts makes pairs")
        };
        let pool = pool_of(vec![(explode.clone(), x), (not_explode.clone(), nx)])?;
        add_inner(&mut data, pool, quantity.clone())?;
    }
    Ok(PValue::Expr(MExpr::Mixture(Arc::new(data))))
}

/// `die.reroll_to_pool(rolls, outcomes, max_rerolls=max_rerolls, star=star, depth=depth, mode=mode)` (node
/// `reroll_to_pool`): a limited number of rerolls shared across a pool.
#[allow(clippy::too_many_arguments)]
pub fn reroll_to_pool(
    die: PValue,
    rolls: PValue,
    outcomes: PValue,
    max_rerolls: PValue,
    star: PValue,
    depth: PValue,
    mode: PValue,
) -> Result<PValue, PErr> {
    let inf = |x: &PValue| matches!(x, PValue::Value(Value::Str(s)) if &**s == "inf");
    if inf(&max_rerolls) {
        // `self.reroll(outcomes, star=star, depth=depth).pool(rolls)`.
        let rerolled = super::map::reroll(false, die, outcomes, star, depth)?;
        return die_pool(rerolled, rolls);
    }
    let d = the_die(die.clone())?;
    let mask = selection(&d, &outcomes, &star, false)?;
    if !mask.contains(&true) {
        return die_pool(die, rolls);
    }
    let (rerollable_die, not_rerollable_die) = d.split(&mask)?;
    let rolls = value_arg(&rolls, "reroll_to_pool rolls")?;
    let max_rerolls = value_arg(&max_rerolls, "max_rerolls")?;
    let depth = if inf(&depth) {
        max_rerolls.clone()
    } else {
        value_arg(&depth, "reroll_to_pool depth")?
    };
    let pool_composition = crate::to_pool::reroll_composition(
        &d.denominator(),
        &rerollable_die.denominator(),
        &rolls,
        &max_rerolls,
        &depth,
        rmatmul,
    )?;
    // `match mode: case 'random': ...`: a value that is not a str matches none of the cases (the collections
    // whose `==` could say otherwise are not modelled), and the error message shows its `str` (which is never
    // one of the names: only a str prints without quotes or brackets).
    let mode = match &mode {
        PValue::Value(Value::Str(m)) => m.to_string(),
        PValue::Value(v) if !v.is_collection() => crate::value::pystr::py_str(v)?,
        _ => return Err(PErr::Unimplemented("this reroll_to_pool mode".into())),
    };
    let sub = |a: &Value, b: &Value| a.binary(crate::value::BinOp::Sub, b);
    let die_pool_of =
        |die: &Die<Value>, n: &Value| die_pool(PValue::Die(die.clone()), PValue::Value(n.clone()));
    // `pool_composition.map_to_pool(make_pool, star=True)`.
    let mut data = Vec::new();
    for (composition, quantity) in pool_composition.items() {
        let Value::Tuple(t) = composition else {
            unreachable!("reroll_composition makes tuples")
        };
        let [rerollable, rerolled, rerolled_to_rerollable] = &t[..] else {
            unreachable!("reroll_composition makes triples")
        };
        let rerolls_ran_out = sub(rerollable, rerolled)?;
        let not_rerollable = sub(&sub(&rolls, &rerolls_ran_out)?, rerolled_to_rerollable)?;
        let common = binary_op(
            "+",
            die_pool_of(&rerollable_die, rerolled_to_rerollable)?,
            die_pool_of(&not_rerollable_die, &not_rerollable)?,
        )?;
        let ran_out = PValue::Value(rerolls_ran_out.clone());
        let pool = match mode.as_str() {
            "random" => binary_op("+", common, die_pool_of(&rerollable_die, &rerolls_ran_out)?)?,
            "low" | "high" => {
                let kept = pool_highest_lowest(
                    mode == "low",
                    die_pool_of(&rerollable_die, rerollable)?,
                    ran_out,
                    PValue::Value(Value::None),
                )?;
                binary_op("+", common, kept)?
            }
            "drop" => die_pool_of(&not_rerollable_die, &not_rerollable)?,
            _ => {
                return raise(Error::Value(format!(
                    "Invalid reroll_priority '{mode}'. Allowed values are 'random', 'low', 'high', 'drop'."
                )))
            }
        };
        let pool = as_expression(&pool).expect("pools and their sums are expressions");
        add_inner(&mut data, pool, quantity.clone())?;
    }
    Ok(PValue::Expr(MExpr::Mixture(Arc::new(data))))
}
