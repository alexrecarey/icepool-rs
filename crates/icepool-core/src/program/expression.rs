//! Multiset expression nodes (harness/grammar/multiset.py): the operators of expression/multiset_expression.py
//! (`+ - & | ^`, `additive_union` ..., `* // %`, `unique`, `keep_counts`, `keep_outcomes`, `map_counts`, keep of an
//! expression, `sort_pair`, `max_pair_*`, `versus_*`, `force_order`) and `CompoundKeepGenerator`, plus preparing an
//! expression for evaluation (`_prepare`: its flat of dungeonlets and its sources).
//!
//! Python's argument juggling and dispatch (which class's `__add__` runs, implicit conversion of lists into
//! pools, what raises at construction and what only when evaluated) happens here; the operators themselves are
//! the core's (`crate::operator`).

use std::fmt;
use std::sync::Arc;

use super::catalogue::{truthy, Func};
use super::evaluate::implicit_convert;
use super::ops::{py_iter, type_error};
use super::pool::KeepGen;
use super::{raise, PErr, PValue};
use crate::compound_keep::CompoundKeep;
use crate::error::Error;
use crate::expression::{count_value, value_count, Dungeonlet, Flat};
use crate::lexi::{compute_lexi_tuple, compute_lexi_tuple_with_zero_right_first};
use crate::operator::{
    BinaryOperator, CountFunction, KeepCountsComparison, MultisetKeepIndex, Operator,
    OutcomeFilter, OutcomePredicate,
};
use crate::pool::{Count, MultisetSource, Pool};
use crate::value::builtins::{getitem, Index};
use crate::weightless::WeightlessSource;
use crate::{MultiDeal, Quantity, Value};

/// A `MultisetExpression` (or a multiset tuple generator, which only evaluations take).
#[derive(Clone)]
pub enum MExpr {
    /// A keep generator: `Pool`, `Deal`, `CompoundKeepGenerator`.
    Gen(KeepGen),
    /// A `MultiDeal`.
    MultiDeal(MultiDeal<Value>),
    /// `WeightlessGenerator`: `g.weightless()` of a keep generator `g` (generator/weightless.py). Not a keep
    /// generator itself, so keep-like operations and `+` on it build operators.
    Weightless(KeepGen),
    /// A `MultisetOperator` and its children.
    Op(Arc<OpNode>),
    /// `MultisetParameter`: argument `arg_index` of a `@multiset_function` (its `star_index` element if starred).
    Param {
        arg_index: usize,
        star_index: Option<usize>,
    },
    /// `MultisetTupleParameter`: a multiset tuple argument (a `MultiDeal`) of a `@multiset_function`, not starred.
    TupleParam { arg_index: usize, len: usize },
    /// `MultisetTupleIndex`: `child[index]` of a multiset tuple expression, a multiset expression.
    TupleIndex(Arc<(MExpr, i64)>),
    /// `MultisetTupleSlice`: `child[start:stop:step]`, a multiset tuple expression.
    TupleSlice(Arc<(MExpr, [Value; 3])>),
    /// `MultisetMixture`: inner expressions with their weights (equal inners merged, the first kept).
    Mixture(Arc<Vec<(MExpr, Quantity)>>),
}

/// A `MultisetOperator` node of an expression.
pub struct OpNode {
    pub operator: Operator<Value>,
    pub children: Vec<MExpr>,
    /// Whether the operator's `_expression_key` holds a list (a `MultisetKeep` given a list index): its
    /// `hash_key` cannot be hashed, so it cannot be an inner of a mixture.
    pub list_index: bool,
}

impl fmt::Debug for MExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MExpr::Gen(g) => write!(f, "{g:?}"),
            MExpr::MultiDeal(_) => write!(f, "MultiDeal"),
            MExpr::Weightless(g) => write!(f, "WeightlessGenerator({g:?})"),
            MExpr::Op(node) => write!(f, "MultisetOperator({:?})", node.children),
            MExpr::Param { arg_index, .. } | MExpr::TupleParam { arg_index, .. } => {
                write!(f, "MultisetParameter({arg_index})")
            }
            MExpr::TupleIndex(node) => write!(f, "MultisetTupleIndex({:?})", node.0),
            MExpr::TupleSlice(node) => write!(f, "MultisetTupleSlice({:?})", node.0),
            MExpr::Mixture(inners) => write!(f, "MultisetMixture({inners:?})"),
        }
    }
}

impl MExpr {
    fn op(operator: Operator<Value>, children: Vec<MExpr>) -> MExpr {
        MExpr::Op(Arc::new(OpNode {
            operator,
            children,
            list_index: false,
        }))
    }

    /// `_prepare`: each way to prepare this expression (one, unless a mixture is inside): its dungeonlets (postfix
    /// order), the sources of its free variables and the weight.
    fn prepare(&self) -> Result<Vec<Part>, PErr> {
        let one = |dungeonlet, sources| Ok(vec![(vec![dungeonlet], sources, Quantity::ONE)]);
        match self {
            MExpr::Gen(g) => one(Dungeonlet::FreeVariable, vec![g.source()?]),
            // `WeightlessSource(self._wrapped._make_source())`.
            MExpr::Weightless(g) => one(
                Dungeonlet::FreeVariable,
                vec![MultisetSource::Weightless(WeightlessSource::new(
                    g.source()?,
                ))],
            ),
            MExpr::MultiDeal(md) => one(
                Dungeonlet::FreeVariable,
                vec![MultisetSource::MultiDeal(md.source())],
            ),
            MExpr::Op(node) => {
                let OpNode {
                    operator, children, ..
                } = &**node;
                let children = children
                    .iter()
                    .map(MExpr::prepare)
                    .collect::<Result<Vec<_>, _>>()?;
                // `itertools.product` over the children's preparations.
                Ok(product(&children)
                    .into_iter()
                    .map(|parts| {
                        let mut flat = Vec::new();
                        let mut sources = Vec::new();
                        let mut weight = Quantity::ONE;
                        let mut positions = Vec::with_capacity(parts.len());
                        for (child_flat, child_sources, child_weight) in parts {
                            let offset = flat.len();
                            flat.extend(child_flat.iter().map(|d| shifted(d, offset)));
                            sources.extend(child_sources.iter().cloned());
                            weight *= child_weight;
                            positions.push(flat.len() - 1);
                        }
                        // With no children `positions[-1]` is never evaluated (it is inside a generator
                        // expression): an operator without children is fine until `next_state` reduces nothing.
                        flat.push(Dungeonlet::Operator {
                            operator: operator.clone(),
                            children: positions,
                        });
                        (flat, sources, weight)
                    })
                    .collect())
            }
            MExpr::Param {
                arg_index,
                star_index,
            } => one(
                Dungeonlet::Parameter {
                    arg_index: *arg_index,
                    star_index: *star_index,
                },
                Vec::new(),
            ),
            MExpr::TupleParam { arg_index, .. } => one(
                Dungeonlet::Parameter {
                    arg_index: *arg_index,
                    star_index: None,
                },
                Vec::new(),
            ),
            MExpr::TupleIndex(node) => tuple_item(&node.0, Index::Item(Value::from(node.1))),
            MExpr::TupleSlice(node) => {
                let [start, stop, step] = node.1.clone();
                tuple_item(&node.0, Index::Slice(start, stop, step))
            }
            // `MultisetMixture._prepare`: each inner's preparations, weighted by the inner's weight.
            MExpr::Mixture(inners) => {
                let mut parts = Vec::new();
                for (inner, weight) in inners.iter() {
                    for (flat, sources, inner_weight) in inner.prepare()? {
                        parts.push((flat, sources, inner_weight * weight));
                    }
                }
                Ok(parts)
            }
        }
    }

    /// `_has_parameter`: whether this expression contains a `@multiset_function` parameter (then evaluating it
    /// only records the evaluation for the function).
    pub fn has_parameter(&self) -> bool {
        match self {
            MExpr::Gen(_) | MExpr::MultiDeal(_) | MExpr::Weightless(_) => false,
            MExpr::Param { .. } | MExpr::TupleParam { .. } => true,
            MExpr::Op(node) => node.children.iter().any(MExpr::has_parameter),
            MExpr::TupleIndex(node) => node.0.has_parameter(),
            MExpr::TupleSlice(node) => node.0.has_parameter(),
            MExpr::Mixture(inners) => inners.iter().any(|(inner, _)| inner.has_parameter()),
        }
    }

    /// `len()` of a multiset tuple expression (a `MultiDeal`, a tuple parameter or a slice of one); `None` for a
    /// multiset expression, which has no `len`.
    pub fn tuple_len(&self) -> Result<Option<usize>, PErr> {
        Ok(match self {
            MExpr::MultiDeal(md) => Some(md.len()),
            MExpr::TupleParam { len, .. } => Some(*len),
            // `len([None] * len(child)[index])`
            MExpr::TupleSlice(node) => {
                let n = node.0.tuple_len()?.expect("a slice of a tuple expression");
                let [start, stop, step] = node.1.clone();
                match getitem(
                    &Value::tuple(vec![Value::None; n]),
                    &Index::Slice(start, stop, step),
                )? {
                    Value::Tuple(t) => Some(t.len()),
                    _ => unreachable!("a slice of a tuple is a tuple"),
                }
            }
            _ => None,
        })
    }

    /// Whether this is a keep generator without negative keeps (closed under keep-like operations).
    fn keepable(&self) -> Option<&KeepGen> {
        match self {
            MExpr::Gen(g) if !g.has_negative_keeps() => Some(g),
            _ => None,
        }
    }
}

/// One way to prepare an expression: its dungeonlets, the sources of its free variables, and the weight.
type Part = (Vec<Dungeonlet<Value>>, Vec<MultisetSource<Value>>, Quantity);

/// A dungeonlet moved `offset` places later in a flat: its children's positions move with it (Python's relative
/// child indexes need no change).
fn shifted(d: &Dungeonlet<Value>, offset: usize) -> Dungeonlet<Value> {
    match d {
        Dungeonlet::Operator { operator, children } => Dungeonlet::Operator {
            operator: operator.clone(),
            children: children.iter().map(|c| c + offset).collect(),
        },
        Dungeonlet::TupleItem { child, index } => Dungeonlet::TupleItem {
            child: child + offset,
            index: index.clone(),
        },
        d => d.clone(),
    }
}

/// `MultisetTupleIndex._prepare` / `MultisetTupleSlice._prepare`: the child's, then the item node.
fn tuple_item(child: &MExpr, index: Index) -> Result<Vec<Part>, PErr> {
    let mut parts = child.prepare()?;
    for (flat, _, _) in &mut parts {
        flat.push(Dungeonlet::TupleItem {
            child: flat.len() - 1,
            index: index.clone(),
        });
    }
    Ok(parts)
}

/// `itertools.product(*lists)`: every combination of one element from each list, the last varying fastest.
pub fn product<T>(lists: &[Vec<T>]) -> Vec<Vec<&T>> {
    let mut result: Vec<Vec<&T>> = vec![Vec::new()];
    for list in lists {
        result = result
            .into_iter()
            .flat_map(|prefix| {
                list.iter().map(move |x| {
                    let mut combination = prefix.clone();
                    combination.push(x);
                    combination
                })
            })
            .collect();
    }
    result
}

/// One prepared evaluation's inputs: one flat per expression, the sources of all their free variables, and the
/// weight of this preparation.
pub type Prepared = (Vec<Flat<Value>>, Vec<MultisetSource<Value>>, Quantity);

/// `MultisetEvaluator._prepare`'s inputs: for each combination of the expressions' preparations (one, unless a
/// mixture is among them), one flat per expression, their sources and the product of their weights.
pub fn prepare(exprs: &[MExpr]) -> Result<Vec<Prepared>, PErr> {
    let parts = exprs
        .iter()
        .map(MExpr::prepare)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(product(&parts)
        .into_iter()
        .map(|combination| {
            let mut flats = Vec::with_capacity(combination.len());
            let mut sources = Vec::new();
            let mut weight = Quantity::ONE;
            for (flat, s, w) in combination {
                flats.push(Flat::from(flat.clone()));
                sources.extend(s.iter().cloned());
                weight *= w;
            }
            (flats, sources, weight)
        })
        .collect())
}

/// A program value that is a multiset expression, as one (a multiset tuple expression is not one).
pub fn as_expression(x: &PValue) -> Option<MExpr> {
    match x {
        PValue::Pool(p) => Some(MExpr::Gen(KeepGen::Pool(p.clone()))),
        PValue::Deal(d) => Some(MExpr::Gen(KeepGen::Deal(d.clone()))),
        PValue::Expr(MExpr::TupleParam { .. } | MExpr::TupleSlice(_)) => None,
        PValue::Expr(e) => Some(e.clone()),
        _ => None,
    }
}

/// A program value that is a multiset tuple expression (`MultisetTupleExpression`), as one.
pub fn as_tuple_expression(x: &PValue) -> Option<MExpr> {
    match x {
        PValue::MultiDeal(md) => Some(MExpr::MultiDeal(md.clone())),
        PValue::Expr(e @ (MExpr::TupleParam { .. } | MExpr::TupleSlice(_))) => Some(e.clone()),
        _ => None,
    }
}

/// `x[index]` (node `mx_tuple_index`). For a multiset tuple expression an `int` (or bool) index makes a
/// `MultisetTupleIndex`, anything else makes `__getitem__` fall off its end, returning `None`; for a multiset
/// expression it is `x.keep(index)`.
pub fn tuple_index(x: PValue, index: PValue) -> Result<PValue, PErr> {
    let Some(e) = as_tuple_expression(&x) else {
        return match &x {
            // `MultisetExpression.__getitem__` is `keep`.
            x if as_expression(x).is_some() => super::pool::pool_keep(x.clone(), index),
            PValue::Value(Value::None) => type_error("'NoneType' object is not subscriptable"),
            _ => Err(PErr::Unimplemented("indexing this object".into())),
        };
    };
    Ok(match int_arg(&index) {
        Some(i) => PValue::Expr(MExpr::TupleIndex(Arc::new((e, i?)))),
        None => PValue::Value(Value::None),
    })
}

/// `x[start:stop:step]` (node `mx_tuple_slice`): for a multiset tuple expression a `MultisetTupleSlice`, whose
/// bounds are only looked at when it is evaluated or its `len()` taken; for a multiset expression a keep.
pub fn tuple_slice(x: PValue, start: PValue, stop: PValue, step: PValue) -> Result<PValue, PErr> {
    let Some(e) = as_tuple_expression(&x) else {
        return match &x {
            x if as_expression(x).is_some() => {
                super::pool::pool_slice(x.clone(), start, stop, step)
            }
            PValue::Value(Value::None) => type_error("'NoneType' object is not subscriptable"),
            _ => Err(PErr::Unimplemented("slicing this object".into())),
        };
    };
    let bound = |b: PValue| match b {
        PValue::Value(v) => Ok(v),
        _ => Err(PErr::Unimplemented(
            "a slice bound that is not a value".into(),
        )),
    };
    let index = [bound(start)?, bound(stop)?, bound(step)?];
    Ok(PValue::Expr(MExpr::TupleSlice(Arc::new((e, index)))))
}

/// A multiset expression method's `self`: anything else lacks the method (AttributeError).
pub fn the_expression(x: PValue) -> Result<MExpr, PErr> {
    match as_expression(&x) {
        Some(e) => Ok(e),
        None => match x {
            // `MultisetTupleExpression` has none of them (and no `*`, `//`, `-x`...).
            PValue::MultiDeal(_) | PValue::Expr(_) => type_error(
                "AttributeError: a multiset tuple expression has no multiset expression methods",
            ),
            _ => type_error("AttributeError: not a multiset expression"),
        },
    }
}

/// The program value of an expression: pools and deals are their own kinds.
pub fn into_pvalue(e: MExpr) -> PValue {
    match e {
        MExpr::Gen(KeepGen::Pool(p)) => PValue::Pool(p),
        MExpr::Gen(KeepGen::Deal(d)) => PValue::Deal(d),
        MExpr::MultiDeal(md) => PValue::MultiDeal(md),
        e => PValue::Expr(e),
    }
}

fn binary(op: BinaryOperator, children: Vec<MExpr>) -> MExpr {
    MExpr::op(Operator::Binary(op), children)
}

/// `KeepGenerator.additive_union(*args)`: a `CompoundKeepGenerator` if all are keep generators without negative
/// keeps (counting each element once), else `MultisetExpression.additive_union`.
fn keep_generator_additive_union(args: Vec<MExpr>) -> Result<MExpr, PErr> {
    let gens: Option<Vec<&KeepGen>> = args.iter().map(MExpr::keepable).collect();
    if let Some(gens) = gens {
        let mut size = 0i64;
        for g in &gens {
            size = size
                .checked_add(g.keep_size()?)
                .ok_or_else(|| PErr::Unimplemented("keep sizes beyond 64 bits".into()))?;
        }
        let sources = gens
            .iter()
            .map(|g| g.source())
            .collect::<Result<Vec<_>, _>>()?;
        let len = usize::try_from(size).expect("no negative keeps");
        return Ok(MExpr::Gen(KeepGen::Compound(CompoundKeep::new(
            sources,
            vec![1; len],
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
            if !matches!(
                a,
                PValue::List(_) | PValue::Value(Value::Tuple(_) | Value::Str(_))
            ) && as_tuple_expression(&a).is_none()
            {
                return Err(PErr::Unimplemented(
                    "a reflected multiset operator with this left operand".into(),
                ));
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
            return type_error(format!("unsupported operand type(s) for {op}"))
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
fn int_arg(x: &PValue) -> Option<Result<i64, PErr>> {
    let i = match x {
        PValue::Value(Value::Int(i)) => i.clone(),
        PValue::Value(Value::Bool(b)) => (*b as i64).into(),
        _ => return None,
    };
    Some(i64::try_from(&i).map_err(|_| PErr::Unimplemented("a count beyond 64 bits".into())))
}

/// A constant stored by an operator and used in count arithmetic: the port takes `int`s only.
fn constant(x: &PValue) -> Result<i64, PErr> {
    match int_arg(x) {
        Some(n) => n,
        None => Err(PErr::Unimplemented(
            "a multiset count constant that is not an int".into(),
        )),
    }
}

/// `unique(True)` is `min(count, True)`, which keeps a bool count: not modelled.
fn int_constant(x: &PValue) -> Result<i64, PErr> {
    match x {
        PValue::Value(Value::Bool(_)) => Err(PErr::Unimplemented("a bool count constant".into())),
        x => constant(x),
    }
}

/// `expr * n` and `-expr` on a keep generator change its keep tuple; on any other expression they build
/// `MultisetMultiplyCounts`.
fn multiply_counts(e: MExpr, n: i64) -> Result<MExpr, PErr> {
    if let Some(inners) = keepable_mixture(&e) {
        let mixture = map_mixture(inners, |inner| {
            Ok(into_pvalue(multiply_counts(the_expression(inner)?, n)?))
        })?;
        return the_expression(mixture);
    }
    Ok(match e {
        MExpr::Gen(g) => MExpr::Gen(g.multiply_counts(n)?),
        e => MExpr::op(Operator::MultiplyCounts(n), vec![e]),
    })
}

/// The count-adjusting operations, by `op`: `mul` (`expr * n`), `neg` (`-expr`), `pos` (`+expr`), `floordiv`
/// (`expr // n`: with a non-int, `count_subset`), `mod` (`expr % n`), `unique` (`expr.unique(n)`), `keep_counts`
/// (`expr.keep_counts(comparison, n)`), and the methods `multiply_counts`, `divide_counts`, `modulo_counts`.
pub fn counts(e: PValue, op: &str, comparison: PValue, n: PValue) -> Result<PValue, PErr> {
    let e = the_expression(e)?;
    let not_int = |op: &str| -> Result<PValue, PErr> {
        match n {
            PValue::Die(_) => Err(PErr::Unimplemented(format!(
                "a die as the right operand of {op}"
            ))),
            _ => type_error(format!("unsupported operand type(s) for {op}")),
        }
    };
    let result = match op {
        // `KeepGenerator.__mul__` and `MultisetExpression.__mul__` both return NotImplemented for a non-int.
        "mul" => match int_arg(&n) {
            Some(n) => multiply_counts(e, n?)?,
            None => return not_int("*"),
        },
        "neg" => multiply_counts(e, -1)?,
        "pos" => MExpr::op(Operator::KeepCounts(KeepCountsComparison::Ge, 0), vec![e]),
        "multiply_counts" => multiply_counts(e, constant(&n)?)?,
        "floordiv" => match int_arg(&n) {
            Some(n) => MExpr::op(Operator::FloordivCounts(n?), vec![e]),
            None => {
                return super::evaluate::count_subset(into_pvalue(e), n, PValue::Value(Value::None))
            }
        },
        "divide_counts" => MExpr::op(Operator::FloordivCounts(int_constant(&n)?), vec![e]),
        "mod" | "modulo_counts" => match int_arg(&n) {
            Some(n) => MExpr::op(Operator::ModuloCounts(n?), vec![e]),
            None => return not_int("%"),
        },
        "unique" => MExpr::op(Operator::Unique(int_constant(&n)?), vec![e]),
        "keep_counts" => {
            let PValue::Value(Value::Str(c)) = &comparison else {
                return raise(Error::Value("Invalid comparison".into()));
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
        PErr::Unimplemented(what) => Error::Unsupported(what),
    }
}

impl CountFunction<Value> for CatalogueCount {
    fn call(&self, outcome: &Value, counts: &[Count]) -> crate::Result<Count> {
        let mut args = vec![PValue::Value(outcome.clone())];
        args.extend(counts.iter().map(|c| PValue::Value(count_value(c))));
        match self.0.call(args).map_err(core_error)? {
            PValue::Value(v) => value_count(&v),
            _ => Err(Error::Unsupported(
                "a map_counts result that is not an int".into(),
            )),
        }
    }
}

/// A catalogue function as `keep_outcomes` / `drop_outcomes`' callable: `bool(func(outcome))`.
struct CataloguePredicate(Arc<Func>);

impl OutcomePredicate<Value> for CataloguePredicate {
    fn test(&self, outcome: &Value) -> crate::Result<bool> {
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
            PValue::Func(_) | PValue::MultiDeal(_) => {
                return Err(PErr::Unimplemented("this keep_outcomes target".into()))
            }
            // `frozenset(outcomes)`: iterate, every element hashable.
            target => {
                let items = py_iter(target)?
                    .into_iter()
                    .map(|x| match x {
                        PValue::Value(v) => Ok(v),
                        PValue::List(_) => type_error("unhashable type: 'list'"),
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

/// Whether a dict holding expression `a` finds it equal to `b` (`MultisetMixture` counts its inners in one): their
/// `hash_key`s. Expressions of different classes have different hash keys and are never compared; the port only
/// compares generators and parameters.
pub(super) fn same_expression(a: &MExpr, b: &MExpr) -> Result<bool, PErr> {
    Ok(match (a, b) {
        (MExpr::Gen(a), MExpr::Gen(b)) => same_generator(a, b),
        // `(WeightlessGenerator, wrapped.hash_key)`.
        (MExpr::Weightless(a), MExpr::Weightless(b)) => same_generator(a, b),
        (MExpr::MultiDeal(a), MExpr::MultiDeal(b)) => a == b,
        (
            MExpr::Param {
                arg_index: a,
                star_index: sa,
            },
            MExpr::Param {
                arg_index: b,
                star_index: sb,
            },
        ) => (a, sa) == (b, sb),
        (MExpr::TupleParam { arg_index: a, .. }, MExpr::TupleParam { arg_index: b, .. }) => a == b,
        // Their `hash_key` is `(type, index)`: the child is left out, so hands of different deals compare equal
        // (DIVERGENCES.md).
        (MExpr::TupleIndex(a), MExpr::TupleIndex(b)) => a.1 == b.1,
        (MExpr::TupleSlice(a), MExpr::TupleSlice(b)) => a.1 == b.1,
        // `(MultisetMixture, tuple(inners.items()))`.
        (MExpr::Mixture(a), MExpr::Mixture(b)) => {
            if a.len() != b.len() {
                return Ok(false);
            }
            for ((a, wa), (b, wb)) in a.iter().zip(b.iter()) {
                if !same_expression(a, b)? || wa != wb {
                    return Ok(false);
                }
            }
            true
        }
        // `a is b`: the dict finds the very same object before comparing.
        (MExpr::Op(a), MExpr::Op(b)) if Arc::ptr_eq(a, b) => true,
        // `(_expression_key, tuple(child.hash_key ...))`.
        (MExpr::Op(a), MExpr::Op(b)) => {
            if !same_operator(&a.operator, &b.operator) || a.children.len() != b.children.len() {
                return Ok(false);
            }
            for (a, b) in a.children.iter().zip(&b.children) {
                if !same_expression(a, b)? {
                    return Ok(false);
                }
            }
            true
        }
        (a, b) if std::mem::discriminant(a) != std::mem::discriminant(b) => false,
        _ => {
            return Err(PErr::Unimplemented(
                "comparing multiset expressions other than generators".into(),
            ))
        }
    })
}

/// Whether a function is the same object as another (a function equals only itself).
fn same_object<T: ?Sized>(a: &Arc<T>, b: &Arc<T>) -> bool {
    std::ptr::addr_eq(Arc::as_ptr(a), Arc::as_ptr(b))
}

/// Whether two operators have equal `_expression_key`s: the class and its parameters. Functions (a `map_counts`
/// function, a `keep_outcomes` callable) are compared by identity; a `keep_outcomes` collection becomes a new
/// function per operator, so such operators never compare equal.
fn same_operator(a: &Operator<Value>, b: &Operator<Value>) -> bool {
    use Operator as O;
    match (a, b) {
        // `type(self)`
        (O::Binary(a), O::Binary(b)) => a == b,
        // `(type(self), self._function)`
        (O::MapCounts(a), O::MapCounts(b)) => same_object(a, b),
        // `(type(self), self._constant)`
        (O::MultiplyCounts(a), O::MultiplyCounts(b))
        | (O::FloordivCounts(a), O::FloordivCounts(b))
        | (O::ModuloCounts(a), O::ModuloCounts(b))
        | (O::Unique(a), O::Unique(b))
        | (O::ForceOrder(a), O::ForceOrder(b)) => a == b,
        // `(type(self), self._comparison, self._constant)`
        (O::KeepCounts(ca, a), O::KeepCounts(cb, b)) => (ca, a) == (cb, b),
        // `(type(self), self._func, self._invert)`
        (
            O::FilterOutcomes {
                filter: fa,
                invert: ia,
            },
            O::FilterOutcomes {
                filter: fb,
                invert: ib,
            },
        ) => {
            ia == ib
                && match (fa, fb) {
                    (OutcomeFilter::Function(a), OutcomeFilter::Function(b)) => same_object(a, b),
                    _ => false,
                }
        }
        // `(type(self), self._invert)`
        (O::FilterOutcomesBinary { invert: a }, O::FilterOutcomesBinary { invert: b }) => a == b,
        // `(MultisetKeep, index)`.
        (O::Keep(a), O::Keep(b)) => a == b,
        // `(MultisetSortPair, comparison, sort_order, extra)`
        (
            O::SortPair {
                comparison: ca,
                sort_order: oa,
                extra: ea,
            },
            O::SortPair {
                comparison: cb,
                sort_order: ob,
                extra: eb,
            },
        ) => (ca, oa, ea) == (cb, ob, eb),
        // `(MultisetSortPairWhile, keep, comparison, sort_order, extra)`
        (
            O::SortPairWhile {
                keep: ka,
                comparison: ca,
                sort_order: oa,
                extra: ea,
            },
            O::SortPairWhile {
                keep: kb,
                comparison: cb,
                sort_order: ob,
                extra: eb,
            },
        ) => (ka, ca, oa, ea) == (kb, cb, ob, eb),
        // `(class, order, pair_equal, keep)`
        (
            O::MaxPairLate {
                order: oa,
                pair_equal: pa,
                keep: ka,
            },
            O::MaxPairLate {
                order: ob,
                pair_equal: pb,
                keep: kb,
            },
        )
        | (
            O::MaxPairEarly {
                order: oa,
                pair_equal: pa,
                keep: ka,
            },
            O::MaxPairEarly {
                order: ob,
                pair_equal: pb,
                keep: kb,
            },
        ) => (oa, pa, ka) == (ob, pb, kb),
        // `(MultisetVersus, lexi_tuple, order)`
        (
            O::Versus {
                lexi_tuple: la,
                order: oa,
            },
            O::Versus {
                lexi_tuple: lb,
                order: ob,
            },
        ) => (la, oa) == (lb, ob),
        _ => false,
    }
}

/// Whether `hash(expression)` works: its `hash_key` holds nothing unhashable. `MultisetTupleIndex` and
/// `MultisetTupleSlice` leave their child out of their `hash_key`; a mixture's inners were hashed when it was made.
fn hashable(e: &MExpr) -> bool {
    match e {
        MExpr::Op(node) => !node.list_index && node.children.iter().all(hashable),
        _ => true,
    }
}

/// Whether two keep generators have the same `hash_key`.
fn same_generator(a: &KeepGen, b: &KeepGen) -> bool {
    match (a, b) {
        (KeepGen::Pool(a), KeepGen::Pool(b)) => {
            a.dice() == b.dice() && a.keep_tuple() == b.keep_tuple()
        }
        (KeepGen::Deal(a), KeepGen::Deal(b)) => a == b,
        (KeepGen::Compound(a), KeepGen::Compound(b)) => a == b,
        _ => false,
    }
}

/// `g.weightless()` (node `weightless`): only multiset generators have the method, and a `WeightlessGenerator`
/// returns itself.
pub fn weightless(g: PValue) -> Result<PValue, PErr> {
    match as_expression(&g) {
        Some(MExpr::Gen(g)) => Ok(PValue::Expr(MExpr::Weightless(g))),
        Some(e @ MExpr::Weightless(_)) => Ok(PValue::Expr(e)),
        _ => type_error("AttributeError: object has no attribute 'weightless'"),
    }
}

/// `data[inner] += weight` for a mixture's `defaultdict(int)` of inners.
pub(super) fn add_inner(
    data: &mut Vec<(MExpr, Quantity)>,
    inner: MExpr,
    weight: Quantity,
) -> Result<(), PErr> {
    if !hashable(&inner) {
        return type_error("unhashable type: 'list'");
    }
    for (key, w) in data.iter_mut() {
        if same_expression(key, &inner)? {
            *w += weight;
            return Ok(());
        }
    }
    data.push((inner, weight));
    Ok(())
}

/// `icepool.MultisetMixture(inners)` for a sequence of expressions (node `mx_mixture`): each counted once, equal
/// ones together.
pub fn mixture(inners: PValue) -> Result<PValue, PErr> {
    let mut data = Vec::new();
    for x in py_iter(inners)? {
        let Some(inner) = as_expression(&x).or_else(|| as_tuple_expression(&x)) else {
            return Err(PErr::Unimplemented(
                "a mixture of something other than expressions".into(),
            ));
        };
        add_inner(&mut data, inner, Quantity::ONE)?;
    }
    Ok(PValue::Expr(MExpr::Mixture(Arc::new(data))))
}

/// `_static_keepable`: keep generators are, and mixtures of them.
fn static_keepable(e: &MExpr) -> bool {
    match e {
        MExpr::Gen(_) => true,
        MExpr::Mixture(inners) => inners.iter().all(|(inner, _)| static_keepable(inner)),
        _ => false,
    }
}

/// A `MultisetMixture`'s inners if it is `_static_keepable`: then `keep` and `multiply_counts` apply to each inner
/// instead.
pub fn keepable_mixture(e: &MExpr) -> Option<&[(MExpr, Quantity)]> {
    match e {
        MExpr::Mixture(inners) if static_keepable(e) => Some(inners),
        _ => None,
    }
}

/// `MultisetMixture._unary_operation(op)`: `op` applied to each inner (in order), the results counted with the
/// inners' weights into a new mixture.
pub fn map_mixture(
    inners: &[(MExpr, Quantity)],
    op: impl Fn(PValue) -> Result<PValue, PErr>,
) -> Result<PValue, PErr> {
    let mut data = Vec::new();
    for (inner, weight) in inners {
        let result = op(into_pvalue(inner.clone()))?;
        let Some(result) = as_expression(&result) else {
            return Err(PErr::Unimplemented("a mixture of non-expressions".into()));
        };
        add_inner(&mut data, result, weight.clone())?;
    }
    Ok(PValue::Expr(MExpr::Mixture(Arc::new(data))))
}

/// `MultisetKeep(expr, index=index)` for a slice or a sequence of counts and `...` (`list_index`: given as a
/// list, which makes the expression unhashable).
pub fn multiset_keep(e: MExpr, index: MultisetKeepIndex, list_index: bool) -> MExpr {
    MExpr::Op(Arc::new(OpNode {
        operator: Operator::Keep(index),
        children: vec![e],
        list_index,
    }))
}

/// A sequence index for `MultisetKeep`: its elements are only looked at when evaluated; `int`s and `...` here.
pub fn multiset_keep_seq(index: PValue) -> Result<MultisetKeepIndex, PErr> {
    let items = py_iter(index)?
        .into_iter()
        .map(|x| match x {
            PValue::Ellipsis => Ok(None),
            x => match int_arg(&x) {
                Some(n) => n.map(Some),
                None => Err(PErr::Unimplemented(
                    "a MultisetKeep index element that is not an int".into(),
                )),
            },
        })
        .collect::<Result<_, _>>()?;
    Ok(MultisetKeepIndex::Seq(items))
}

/// A `str` argument of a pairing method; any other value can never match (ValueError when evaluated).
fn name(x: &PValue) -> Arc<str> {
    match x {
        PValue::Value(Value::Str(s)) => s.clone(),
        _ => Arc::from("\0not a str"),
    }
}

/// An `Order` argument (`Order(order)` on the Python side): -1, 0 or 1.
fn order_arg(x: &PValue) -> Result<i64, PErr> {
    match int_arg(x) {
        Some(Ok(o)) if (-1..=1).contains(&o) => Ok(o),
        _ => Err(PErr::Unimplemented("an order that is not an Order".into())),
    }
}

/// `expr.sort_pair(comparison, other, order, extra)`, `sort_pair_keep_while` and `sort_pair_drop_while`.
/// Nothing is checked until evaluated.
pub fn sort_pair(
    e: PValue,
    method: &str,
    comparison: PValue,
    other: PValue,
    order: PValue,
    extra: PValue,
) -> Result<PValue, PErr> {
    let e = the_expression(e)?;
    let other = implicit_convert(other)?;
    let mut sort_order = order_arg(&order)?;
    if sort_order == 0 {
        sort_order = -1;
    }
    let (comparison, extra) = (name(&comparison), name(&extra));
    let operator = match method {
        "sort_pair" => Operator::SortPair {
            comparison,
            sort_order,
            extra,
        },
        "sort_pair_keep_while" | "sort_pair_drop_while" => Operator::SortPairWhile {
            keep: method == "sort_pair_keep_while",
            comparison,
            sort_order,
            extra,
        },
        _ => return Err(PErr::Unimplemented(format!("pairing method {method}"))),
    };
    Ok(into_pvalue(MExpr::op(operator, vec![e, other])))
}

/// `expr.max_pair_keep(comparison, other, priority)` / `max_pair_drop`.
pub fn max_pair(
    e: PValue,
    keep: bool,
    comparison: PValue,
    other: PValue,
    priority: PValue,
) -> Result<PValue, PErr> {
    let e = the_expression(e)?;
    let other = implicit_convert(other)?;
    let comparison = name(&comparison);
    if &*comparison == "==" {
        let result = if keep {
            // `+self & +other`
            let pos = |x| MExpr::op(Operator::KeepCounts(KeepCountsComparison::Ge, 0), vec![x]);
            binary(BinaryOperator::Intersection, vec![pos(e), pos(other)])
        } else {
            // `self - other`
            binary(BinaryOperator::DifferenceDropNegative, vec![e, other])
        };
        return Ok(into_pvalue(result));
    }
    let (order, early) = match &priority {
        PValue::Value(Value::None) => {
            let (left_first, _, _) = compute_lexi_tuple(&comparison, 1)?;
            (if left_first != 0 { -1 } else { 1 }, false)
        }
        PValue::Value(Value::Str(p)) if &**p == "low" || &**p == "high" => {
            let order = if &**p == "low" { 1 } else { -1 };
            let (left_first, _, _) = compute_lexi_tuple(&comparison, order)?;
            (order, left_first != 0)
        }
        _ => return raise(Error::Value("priority must be 'low' or 'high'.".into())),
    };
    let (_, tie, _) = compute_lexi_tuple(&comparison, order)?;
    let pair_equal = tie != 0;
    let operator = if early {
        Operator::MaxPairEarly {
            order,
            pair_equal,
            keep,
        }
    } else {
        Operator::MaxPairLate {
            order,
            pair_equal,
            keep,
        }
    };
    Ok(into_pvalue(MExpr::op(operator, vec![e, other])))
}

/// `expr.versus_all(comparison, other)` / `versus_any`.
pub fn versus(e: PValue, all: bool, comparison: PValue, other: PValue) -> Result<PValue, PErr> {
    let e = the_expression(e)?;
    let other = implicit_convert(other)?;
    let (mut lexi_tuple, mut order) = compute_lexi_tuple_with_zero_right_first(&name(&comparison))?;
    if !all {
        lexi_tuple = (lexi_tuple.2, lexi_tuple.1, lexi_tuple.0);
        order = -order;
    }
    Ok(into_pvalue(MExpr::op(
        Operator::Versus { lexi_tuple, order },
        vec![e, other],
    )))
}

/// `expr.force_order(order)`: `Order.Any` returns the expression itself.
pub fn force_order(e: PValue, order: PValue) -> Result<PValue, PErr> {
    let e = the_expression(e)?;
    let order = order_arg(&order)?;
    if order == 0 {
        return Ok(into_pvalue(e));
    }
    Ok(into_pvalue(MExpr::op(Operator::ForceOrder(order), vec![e])))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_names_the_python_class() {
        let p = MExpr::Param {
            arg_index: 1,
            star_index: None,
        };
        assert_eq!(format!("{p:?}"), "MultisetParameter(1)");
        let index = MExpr::TupleIndex(Arc::new((p, 0)));
        assert_eq!(
            format!("{index:?}"),
            "MultisetTupleIndex(MultisetParameter(1))"
        );
    }
}
