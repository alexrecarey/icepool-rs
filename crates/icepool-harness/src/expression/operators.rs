//! The operators of expression/multiset_expression.py: `+ - & | ^`, `additive_union` and the other methods,
//! `* // %` with a count, `keep_counts`, `map_counts` and the outcome filters.

use super::mixture::{keepable_mixture, map_mixture};
use super::{as_expression, as_tuple_expression, into_pvalue, the_expression, MExpr};
use crate::catalogue::{truthy, Func};
use crate::evaluate::implicit_convert;
use crate::ops::{py_iter, unhashable_list, wrong_type};
use crate::pool::KeepGen;
use crate::{raise, PErr, PValue};
use icepool_core::evaluator::{CompoundKeep, Count};
use icepool_core::expression::{count_value, value_count};
use icepool_core::expression::{BinaryOperator, CountFunction, KeepCountsComparison, Operator, OutcomeFilter, OutcomePredicate};
use icepool_core::value::BinOp;
use icepool_core::{Error, Pool};
use icepool_core::{Int, Value};
use std::sync::Arc;

pub(super) fn binary(op: BinaryOperator, children: Vec<MExpr>) -> MExpr {
    MExpr::op(Operator::Binary(op), children)
}

/// `KeepGenerator.additive_union(*args)`: a `CompoundKeepGenerator` if all are keep generators without negative
/// keeps (counting each element once), else `MultisetExpression.additive_union`.
fn keep_generator_additive_union(args: Vec<MExpr>) -> Result<MExpr, PErr> {
    let gens: Option<Vec<&KeepGen>> = args.iter().map(MExpr::keepable).collect();
    if let Some(gens) = gens {
        let size: Int = gens.iter().map(|g| g.keep_size()).sum();
        let sources = gens
            .iter()
            .map(|g| g.source())
            .collect::<Result<Vec<_>, _>>()?;
        // `(1, ) * sum(generator.keep_size() for generator in generators)`
        let len = icepool_core::evaluator::list_len(&size)?;
        return Ok(MExpr::Gen(KeepGen::Compound(CompoundKeep::new(
            sources,
            vec![Int::ONE; len],
        ))));
    }
    Ok(binary(BinaryOperator::AdditiveUnion, args))
}

/// `self.additive_union(*args)` as the method resolves on `args[0]`: `Pool.additive_union` merges pools whose
/// sorted positions all count the same, other keep generators make a compound keep.
fn method_additive_union(args: Vec<MExpr>) -> Result<MExpr, PErr> {
    match &args[0] {
        MExpr::Gen(KeepGen::Pool(_)) => {
            let pools: Option<Vec<Pool<Value>>> = args
                .iter()
                .map(|a| match a {
                    MExpr::Gen(KeepGen::Pool(p)) => Some(p.clone()),
                    _ => None,
                })
                .collect();
            if let Some(pools) = pools {
                if let Some(merged) = Pool::additive_union(&pools)? {
                    return Ok(MExpr::Gen(KeepGen::Pool(merged)));
                }
            }
            keep_generator_additive_union(args)
        }
        MExpr::Gen(_) => keep_generator_additive_union(args),
        _ => Ok(binary(BinaryOperator::AdditiveUnion, args)),
    }
}

/// `a op b` for `op` in `+ - & | ^` with a multiset expression on at least one side. A non-expression operand
/// is implicitly converted to a pool (a list, tuple, str or mapping; anything else raises TypeError).
pub fn binary_op(op: &str, a: PValue, b: PValue) -> Result<PValue, PErr> {
    // An `Again` expression on either side: the expression's `__op__` (or its reflected `__rop__`, after the
    // multiset expression's own returned `NotImplemented`: `Again` cannot be implicitly converted to a pool)
    // builds a bigger `AgainExpression`, as with any other operand.
    if matches!(a, PValue::Again(_)) || matches!(b, PValue::Again(_)) {
        let op = match op {
            "+" => BinOp::Add,
            "-" => BinOp::Sub,
            "&" => BinOp::And,
            "|" => BinOp::Or,
            "^" => BinOp::Xor,
            _ => return Err(PErr::Unimplemented(format!("multiset operator {op}"))),
        };
        return crate::ops::binary(op, a, b);
    }
    let (left, right) = (as_expression(&a), as_expression(&b));
    let result = match (left, right) {
        (Some(a), _) => {
            let b = implicit_convert(b)?;
            match op {
                // `KeepGenerator.__add__` is `self.additive_union(other)`.
                "+" => method_additive_union(vec![a, b])?,
                "-" => binary(BinaryOperator::DifferenceDropNegative, vec![a, b]),
                "&" => binary(BinaryOperator::Intersection, vec![a, b]),
                "|" => binary(BinaryOperator::Union, vec![a, b]),
                "^" => binary(BinaryOperator::SymmetricDifference, vec![a, b]),
                _ => return Err(PErr::Unimplemented(format!("multiset operator {op}"))),
            }
        }
        (None, Some(b)) => {
            // The reflected operators: the left operand is a sequence without the numeric slot, or a multiset
            // tuple expression, which has no operators (it stays as it is: an operator over it gets tuple counts).
            match &a {
                PValue::List(_) | PValue::Value(Value::Tuple(_) | Value::Str(_)) => {}
                _ if as_tuple_expression(&a).is_some() => {}
                // `Deck.__op__` takes any iterable: it counts the expression's items (dice).
                PValue::Deck(_) => {
                    return crate::deck::deck_op(
                        PValue::Value(Value::str(op)),
                        a.clone(),
                        into_pvalue(b),
                    )
                }
                // `Die.__op__` converts the other operand first: `Die([expr])`, whose outcome cannot be sorted
                // (an expression's `<` makes a die without a truth value).
                PValue::Die(_) | PValue::DieWithTruth(..) => {
                    return wrong_type("Items do not appear to be sortable.")
                }
                // A number's own `__op__` returns `NotImplemented`, and so does the expression's `__rop__`
                // (`ImplicitConversionError` from `implicit_convert_to_expression`): TypeError.
                PValue::Value(
                    Value::Int(_)
                    | Value::Bool(_)
                    | Value::Float(_)
                    | Value::Fraction(_)
                    | Value::Complex(..)
                    | Value::None,
                ) => return wrong_type(format!("unsupported operand type(s) for {op}")),
                _ => {
                    return Err(PErr::Unimplemented(
                        "a reflected multiset operator with this left operand".into(),
                    ))
                }
            }
            let a = implicit_convert(a)?;
            match op {
                // `KeepGenerator.__radd__` is `self.additive_union(other)`; `MultisetExpression.__radd__` is
                // `additive_union(other, self)`.
                "+" => match b {
                    MExpr::Gen(_) => method_additive_union(vec![b, a])?,
                    _ => binary(BinaryOperator::AdditiveUnion, vec![a, b]),
                },
                "-" => binary(BinaryOperator::DifferenceDropNegative, vec![a, b]),
                "&" => binary(BinaryOperator::Intersection, vec![a, b]),
                "|" => binary(BinaryOperator::Union, vec![a, b]),
                // Symmetric: `symmetric_difference(self, other)`.
                "^" => binary(BinaryOperator::SymmetricDifference, vec![b, a]),
                _ => return Err(PErr::Unimplemented(format!("multiset operator {op}"))),
            }
        }
        // A multiset tuple expression has no operators, and the other operand none for it.
        (None, None) if as_tuple_expression(&a).is_some() || as_tuple_expression(&b).is_some() => {
            return wrong_type(format!("unsupported operand type(s) for {op}"))
        }
        (None, None) => return Err(PErr::Unimplemented("no multiset operand".into())),
    };
    Ok(into_pvalue(result))
}

/// `MultisetExpression.<name>(*args, keep_negative_counts=...)` called on the class, for `additive_union`,
/// `difference`, `intersection` and `union`.
pub fn method(name: &str, args: PValue, keep_negative_counts: PValue) -> Result<PValue, PErr> {
    let args = py_iter(args)?
        .into_iter()
        .map(implicit_convert)
        .collect::<Result<Vec<_>, _>>()?;
    let op = match name {
        "additive_union" => BinaryOperator::AdditiveUnion,
        "difference" if truthy(&keep_negative_counts)? => BinaryOperator::DifferenceKeepNegative,
        "difference" => BinaryOperator::DifferenceDropNegative,
        "intersection" => BinaryOperator::Intersection,
        "union" => BinaryOperator::Union,
        _ => return Err(PErr::Unimplemented(format!("MultisetExpression.{name}"))),
    };
    Ok(into_pvalue(binary(op, args)))
}

/// An `int` argument (Python's `isinstance(n, int)`, so also a bool), if it is one.
pub(super) fn int_arg(x: &PValue) -> Option<Int> {
    match x {
        PValue::Value(Value::Int(i)) => Some(i.into()),
        PValue::Value(Value::Bool(b)) => Some((*b as i64).into()),
        _ => None,
    }
}

/// A constant stored by an operator and used in count arithmetic: the port takes `int`s only.
fn constant(x: &PValue) -> Result<Int, PErr> {
    match int_arg(x) {
        Some(n) => Ok(n),
        None => Err(PErr::Unimplemented(
            "a multiset count constant that is not an int".into(),
        )),
    }
}

/// `unique(True)` is `min(count, True)`, which keeps a bool count: not modelled.
fn int_constant(x: &PValue) -> Result<Int, PErr> {
    match x {
        PValue::Value(Value::Bool(_)) => Err(PErr::Unimplemented("a bool count constant".into())),
        x => constant(x),
    }
}

/// `expr * n` and `-expr` on a keep generator change its keep tuple; on any other expression they build
/// `MultisetMultiplyCounts`.
fn multiply_counts(e: MExpr, n: Int) -> Result<MExpr, PErr> {
    if let Some(inners) = keepable_mixture(&e) {
        let mixture = map_mixture(inners, |inner| {
            Ok(into_pvalue(multiply_counts(
                the_expression(inner)?,
                n.clone(),
            )?))
        })?;
        return the_expression(mixture);
    }
    Ok(match e {
        MExpr::Gen(g) => MExpr::Gen(g.multiply_counts(&n)),
        e => MExpr::op(Operator::MultiplyCounts(n), vec![e]),
    })
}

/// `expr <op> n` (`reflected`: `n <op> expr`) for `*` and `%` with a non-int `n`: `KeepGenerator.__mul__` /
/// `__rmul__` and `MultisetExpression.__mul__` / `__rmul__` / `__mod__` all return NotImplemented for it, so
/// Python asks the other operand.
fn not_int(e: MExpr, op: BinOp, n: PValue, reflected: bool) -> Result<PValue, PErr> {
    match n {
        // `AgainExpression` defines them all: a bigger expression.
        PValue::Again(_) if reflected => crate::ops::binary(op, n, into_pvalue(e)),
        PValue::Again(_) => crate::ops::binary(op, into_pvalue(e), n),
        // `Die.__op__` / `__rop__` convert the expression first: `Die([expr])` cannot sort its outcome (an
        // expression's `<` makes a die without a truth value).
        PValue::Die(_) | PValue::DieWithTruth(..) => {
            wrong_type("Items do not appear to be sortable.")
        }
        // Numbers return NotImplemented too; a sequence's repeat needs an int.
        _ => wrong_type(format!("unsupported operand type(s) for {}", op.symbol())),
    }
}

/// The count-adjusting operations, by `op`: `mul` (`expr * n`), `rmul` (`n * expr`), `neg` (`-expr`), `pos`
/// (`+expr`), `floordiv` (`expr // n`: with a non-int, `count_subset`), `mod` (`expr % n`), `unique`
/// (`expr.unique(n)`), `keep_counts` (`expr.keep_counts(comparison, n)`), and the methods `multiply_counts`,
/// `divide_counts`, `modulo_counts`.
pub fn counts(e: PValue, op: &str, comparison: PValue, n: PValue) -> Result<PValue, PErr> {
    let e = the_expression(e)?;
    let result = match op {
        "mul" | "rmul" => match int_arg(&n) {
            // `int.__mul__` returns NotImplemented for an expression: `n * expr` is `expr.__rmul__(n)`, which is
            // `multiply_counts(n)` like `__mul__` ("commutable in this case").
            Some(n) => multiply_counts(e, n)?,
            None => return not_int(e, BinOp::Mul, n, op == "rmul"),
        },
        "neg" => multiply_counts(e, Int::from(-1))?,
        "pos" => MExpr::op(
            Operator::KeepCounts(KeepCountsComparison::Ge, Int::ZERO),
            vec![e],
        ),
        "multiply_counts" => multiply_counts(e, constant(&n)?)?,
        "floordiv" => match int_arg(&n) {
            Some(n) => MExpr::op(Operator::FloordivCounts(n), vec![e]),
            None => {
                return crate::evaluate::count_subset(into_pvalue(e), n, PValue::Value(Value::None))
            }
        },
        "divide_counts" => MExpr::op(Operator::FloordivCounts(int_constant(&n)?), vec![e]),
        "mod" | "modulo_counts" => match int_arg(&n) {
            Some(n) => MExpr::op(Operator::ModuloCounts(n), vec![e]),
            None => return not_int(e, BinOp::Mod, n, false),
        },
        "unique" => MExpr::op(Operator::Unique(int_constant(&n)?), vec![e]),
        "keep_counts" => {
            let PValue::Value(Value::Str(c)) = &comparison else {
                return raise(Error::InvalidArgument("Invalid comparison".into()));
            };
            let comparison = KeepCountsComparison::parse(c)?;
            MExpr::op(Operator::KeepCounts(comparison, constant(&n)?), vec![e])
        }
        _ => return Err(PErr::Unimplemented(format!("multiset count op {op}"))),
    };
    Ok(into_pvalue(result))
}

/// A catalogue function as `map_counts`' function: `function(outcome, *counts)`, which must return an `int` (or a
/// tuple of them: counts over a multiset tuple expression are tuples).
struct CatalogueCount(Arc<Func>);

/// A `PErr` inside the core: an unimplemented case is `Unsupported` there.
fn core_error(e: PErr) -> Error {
    match e {
        PErr::Raise(e) => e,
        PErr::Unimplemented(what) => Error::Unported(what),
    }
}

impl CountFunction<Value> for CatalogueCount {
    fn call(&self, outcome: &Value, counts: &[Count]) -> icepool_core::Result<Count> {
        let mut args = vec![PValue::Value(outcome.clone())];
        args.extend(counts.iter().map(|c| PValue::Value(count_value(c))));
        match self.0.call(args).map_err(core_error)? {
            PValue::Value(v) => value_count(&v),
            _ => Err(Error::Unported(
                "a map_counts result that is not an int".into(),
            )),
        }
    }
}

/// A catalogue function as `keep_outcomes` / `drop_outcomes`' callable: `bool(func(outcome))`.
struct CataloguePredicate(Arc<Func>);

impl OutcomePredicate<Value> for CataloguePredicate {
    fn test(&self, outcome: &Value) -> icepool_core::Result<bool> {
        let r = self
            .0
            .call(vec![PValue::Value(outcome.clone())])
            .map_err(core_error)?;
        truthy(&r).map_err(core_error)
    }
}

/// `MultisetExpression.map_counts(*args, function=function)`.
pub fn map_counts(args: PValue, function: PValue) -> Result<PValue, PErr> {
    let args = py_iter(args)?
        .into_iter()
        .map(implicit_convert)
        .collect::<Result<Vec<_>, _>>()?;
    let PValue::Func(f) = function else {
        return Err(PErr::Unimplemented(
            "a map_counts function that is not from the catalogue".into(),
        ));
    };
    if f.positional().is_none() {
        return Err(PErr::Unimplemented(
            "a mapping as map_counts' function".into(),
        ));
    }
    Ok(into_pvalue(MExpr::op(
        Operator::MapCounts(Arc::new(CatalogueCount(f))),
        args,
    )))
}

/// `expr.keep_outcomes(target)` / `expr.drop_outcomes(target)` (`invert`): an expression target counts while
/// positive (`MultisetFilterOutcomesBinary`); a callable is called per outcome; a collection becomes a
/// `frozenset`.
pub fn filter_outcomes(e: PValue, invert: bool, target: PValue) -> Result<PValue, PErr> {
    let e = the_expression(e)?;
    let operator = if let Some(target) = as_expression(&target) {
        let node = MExpr::op(Operator::FilterOutcomesBinary { invert }, vec![e, target]);
        return Ok(into_pvalue(node));
    } else {
        let filter = match target {
            PValue::Func(f) if f.positional().is_some() => {
                OutcomeFilter::Function(Arc::new(CataloguePredicate(f)))
            }
            PValue::Func(_) => return Err(PErr::Unimplemented("this keep_outcomes target".into())),
            // A multiset tuple expression is not a `MultisetExpression`: `frozenset(target)` iterates it into its
            // `MultisetTupleIndex`es, whose hashes (of `(type, index)`) no outcome's matches, so nothing is in it.
            target if as_tuple_expression(&target).is_some() => {
                let e = as_tuple_expression(&target).expect("checked");
                e.tuple_len()?;
                OutcomeFilter::Set(Arc::from(Vec::new()))
            }
            // `frozenset(outcomes)`: iterate, every element hashable.
            target => {
                let items = py_iter(target)?
                    .into_iter()
                    .map(|x| match x {
                        PValue::Value(v) => Ok(v),
                        PValue::List(_) => unhashable_list(),
                        _ => Err(PErr::Unimplemented(
                            "a non-value in keep_outcomes' collection".into(),
                        )),
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                OutcomeFilter::Set(items.into())
            }
        };
        Operator::FilterOutcomes { filter, invert }
    };
    Ok(into_pvalue(MExpr::op(operator, vec![e])))
}
