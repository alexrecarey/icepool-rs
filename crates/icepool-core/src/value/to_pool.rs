//! The arithmetic of `Die.explode_to_pool` and `Die.reroll_to_pool` (population/die.py): the die of pool
//! compositions each of them hands to `map_to_pool`.
//!
//! Both methods describe a mixture of pools by a die whose outcomes say how many dice of each kind the pool holds
//! (`Vector((exploded, not_exploded))`, or `(rerollable, rerolled, rerolled_to_rerollable)` tuples). The caller
//! keeps the argument juggling (`_select_outcomes`, `split`, the early `self.pool(rolls)` returns) and the
//! `map_to_pool` that turns each composition into a pool. `matmul` is the caller's `n @ die` (`Die.__rmatmul__`
//! with Python's types of `n`: its recursion limit and float counts belong to the facade), as in
//! `again::evaluate_agains_using_count`.

use dashu_int::IBig;

use crate::error::Error;
use crate::expand::tupleize_dice;
use crate::map_tools::{map_repeat, TransitionCache};
use crate::Mapped;
use crate::value::{BinOp, Vector};
use crate::{Die, DieArg, Quantity, Value};

fn int(i: impl Into<IBig>) -> Value {
    Value::Int(i.into())
}

fn quantity_value(q: &Quantity) -> Value {
    Value::Int(IBig::from(q.clone()))
}

/// `explode_to_pool`'s `count_die = rolls @ Die(single_data)` for `depth != 0`, where each die explodes up to
/// `depth` times: `Vector((i, 1))` is `i` explosions followed by a roll that does not explode, and `Vector((depth
/// + 1, 0))` the chain that explodes every time. `die_den`, `explode_den` and `not_explode_den` are the
/// denominators of the die and of its two halves after `split`.
pub fn explode_counts<E: From<Error>>(
    die_den: &Quantity,
    explode_den: &Quantity,
    not_explode_den: &Quantity,
    rolls: &Value,
    depth: i64,
    matmul: impl Fn(&Value, &Die<Value>) -> Result<Die<Value>, E>,
) -> Result<Die<Value>, E> {
    // Below -1, `range(depth + 1)` is empty and `explode.denominator()**(depth + 1)` a float, never zero (the caller
    // only gets here if some outcome explodes), which `Die(single_data)` refuses.
    let top = match depth.checked_add(1).map(u32::try_from) {
        Some(Ok(top)) => top,
        Some(Err(_)) if depth < -1 => {
            return Err(Error::WrongType(
                "'float' object cannot be interpreted as an integer".into(),
            )
            .into())
        }
        _ => return Err(Error::Unported("a huge explode_to_pool depth".into()).into()),
    };
    let vector = |i: u32, n: i64| Value::Vector(Vector::new(vec![int(i), int(n)]));
    let mut single_data =
        crate::memory::try_with_capacity(Some(top as u64 + 1), "explode_to_pool(depth)")?;
    // Each weight is a product of `top` denominators.
    crate::memory::check_pow(
        &(explode_den * die_den),
        top as usize,
        "explode_to_pool(depth)",
    )?;
    for i in 0..top {
        let weight =
            explode_den.pow(i as usize) * die_den.pow((top - 1 - i) as usize) * not_explode_den;
        single_data.push((vector(i, 1), weight));
    }
    single_data.push((vector(top, 0), explode_den.pow(top as usize)));
    // `Die(single_data)` drops the zero weights (an empty half has denominator 0).
    let single_count_die = Die::from_dict(single_data)?;
    matmul(rolls, &single_count_die)
}

/// `reroll_to_pool`'s `pool_composition` for a finite `max_rerolls` (`depth` already an int: `'inf'` is
/// `max_rerolls`): a die of `(rerollable, rerolled, rerolled_to_rerollable)` tuples, multiplied to the
/// denominator `die_den**(rolls + max_rerolls)`. `rerollable_den` is the denominator of the rerollable half of
/// the die. The counts are Python ints (or bools, which `rolls @ coin` keeps for one roll), so all arithmetic on
/// them is `Value`'s.
pub fn reroll_composition<E: From<Error>>(
    die_den: &Quantity,
    rerollable_den: &Quantity,
    rolls: &Value,
    max_rerolls: &Value,
    depth: &Value,
    matmul: impl Fn(&Value, &Die<Value>) -> Result<Die<Value>, E>,
) -> Result<Die<Value>, E> {
    let (n, d) = (quantity_value(rerollable_den), quantity_value(die_den));
    // `icepool.coin(rerollable_die.denominator(), self.denominator())`.
    let coin = crate::value::function::coin_data(&n, &d)?
        .into_iter()
        .map(|(o, q)| match q {
            Value::Int(q) => Ok((o, Quantity::try_from(q).map_err(|_| negative())?)),
            _ => unreachable!("coin of ints has int quantities"),
        })
        .collect::<Result<Vec<_>, Error>>()?;
    let single_is_rerollable = Die::from_dict(coin)?;
    let tupleize = |items: Vec<Vec<(Value, Quantity)>>| tupleize_dice(&items);
    let one = |v: &Value| vec![(v.clone(), Quantity::ONE)];

    // Advances one step of rerolling if there are enough rerolls left to cover all rerollable dice. Returns the
    // number of dice showing rerollable outcomes and the number of remaining rerolls.
    let step = |state: &Value, _: usize| -> Result<Mapped<Value>, E> {
        let [rerollable, rerolls_left] = pair(state);
        if rerollable.eq_op(&int(0), false)?.truthy()? {
            return Ok(Mapped::Outcome(Value::tuple(vec![int(0), int(0)])));
        }
        if rerolls_left.binary(BinOp::Lt, rerollable)?.truthy()? {
            return Ok(Mapped::Outcome(Value::tuple(vec![
                rerollable.clone(),
                rerolls_left.clone(),
            ])));
        }
        let rerolled = matmul(rerollable, &single_is_rerollable)?;
        let left = rerolls_left.binary(BinOp::Sub, rerollable)?;
        Ok(Mapped::Die(tupleize(vec![
            rerolled.items().to_vec(),
            one(&left),
        ])?))
    };

    // `initial_state = icepool.tupleize(rolls @ single_is_rerollable, max_rerolls)`.
    let initial_state = tupleize(vec![
        matmul(rolls, &single_is_rerollable)?.items().to_vec(),
        one(max_rerolls),
    ])?;
    // `icepool.map(step, initial_state, star=True, repeat=depth - 1)`.
    let repeat = depth.binary(BinOp::Sub, &int(1))?;
    let mid_pool_composition = if repeat.binary(BinOp::Lt, &int(0))?.truthy()? {
        return Err(Error::InvalidArgument("repeat cannot be negative.".into()).into());
    } else if repeat.eq_op(&int(0), false)?.truthy()? {
        // `Die([first_arg])` of a die is the die.
        initial_state
    } else {
        // `range(repeat)` raises TypeError for a non-int (after the initial self-loop die, in `map_repeat`). A
        // repeat beyond u64 saturates: the loop stops once nothing is left to step anyway.
        let repeat = match repeat {
            Value::Int(r) => Some(u64::try_from(r).unwrap_or(u64::MAX)),
            _ => None,
        };
        let mut cache = TransitionCache::new(step, vec![Quantity::ONE]);
        map_repeat(&mut cache, &initial_state, repeat)?
    };

    // Performs the final reroll, which might not have enough rerolls to cover all rerollable dice. Returns the
    // number of dice that had a rerollable outcome, the number of dice that were rerolled due to max_rerolls,
    // the number of rerolled dice that landed on a rerollable outcome again.
    // (`mid_pool_composition.map(final_step, star=True)`: one die of tuples per state, mixed by `Die`.)
    let mut final_states = Vec::with_capacity(mid_pool_composition.len());
    let mut quantities = Vec::with_capacity(mid_pool_composition.len());
    for (state, quantity) in mid_pool_composition.items() {
        let [rerollable, rerolls_left] = pair(state);
        let rerolled = Value::py_min(rerollable.clone(), rerolls_left.clone())?;
        let again = matmul(&rerolled, &single_is_rerollable)?;
        final_states.push(DieArg::Die(tupleize(vec![
            one(rerollable),
            one(&rerolled),
            again.items().to_vec(),
        ])?));
        quantities.push(quantity.clone());
    }
    let pool_composition = Die::new(final_states, quantities)?;

    let denominator = d.binary(BinOp::Pow, &rolls.binary(BinOp::Add, max_rerolls)?)?;
    let Some(data) = pool_composition.multiply_to_denominator(&denominator)? else {
        return Ok(pool_composition);
    };
    // `self._new_type(data)`: the quantities are the old ones times a positive factor, which is a float if the
    // denominator is (a non-int `rolls + max_rerolls`): `Die` refuses those.
    let items = data
        .into_iter()
        .map(|(o, q)| match q {
            Value::Int(q) => Ok((o, Quantity::try_from(q).map_err(|_| negative())?)),
            q => Err(Error::WrongType(format!(
                "'{}' object cannot be interpreted as an integer",
                q.type_name()
            ))),
        })
        .collect::<Result<Vec<_>, Error>>()?;
    Ok(Die::from_dict(items)?)
}

fn negative() -> Error {
    Error::InvalidArgument("times cannot have negative values.".into())
}

/// The two parts of a `(rerollable, rerolls_left)` state (`step(*state)`).
fn pair(state: &Value) -> [&Value; 2] {
    match state {
        Value::Tuple(t) if t.len() == 2 => [&t[0], &t[1]],
        _ => unreachable!("reroll_to_pool states are pairs"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explode_counts_depth_limits() {
        let q = Quantity::from(2u8);
        let count = |depth: i64| {
            explode_counts(
                &q,
                &Quantity::ONE,
                &Quantity::ONE,
                &Value::from(1),
                depth,
                |_, d| Ok::<_, Error>(d.clone()),
            )
        };
        // Below -1 Python's own TypeError; a depth past `u32` is one the port does not cover (Python would
        // build that many outcomes).
        assert!(matches!(count(-2), Err(Error::WrongType(_))));
        assert!(matches!(count(1 << 32), Err(Error::Unported(_))));
        assert!(matches!(count(i64::MAX), Err(Error::Unported(_))));
        assert_eq!(count(1).unwrap().len(), 3);
    }

    #[test]
    #[should_panic(expected = "reroll_to_pool states are pairs")]
    fn pair_requires_a_pair() {
        let three = Value::tuple(vec![Value::from(1), Value::from(2), Value::from(3)]);
        pair(&three);
    }
}
