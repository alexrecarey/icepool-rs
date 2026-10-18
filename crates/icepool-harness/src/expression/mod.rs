//! Multiset expression nodes (harness/grammar/multiset.py): the operators of expression/multiset_expression.py
//! (`+ - & | ^`, `additive_union` ..., `* // %`, `unique`, `keep_counts`, `keep_outcomes`, `map_counts`, keep of an
//! expression, `sort_pair`, `max_pair_*`, `versus_*`, `force_order`) and `CompoundKeepGenerator`, plus preparing an
//! expression for evaluation (`_prepare`: its flat of dungeonlets and its sources).
//!
//! Python's argument juggling and dispatch (which class's `__add__` runs, implicit conversion of lists into
//! pools, what raises at construction and what only when evaluated) happens here; the operators themselves are
//! the core's (`icepool_core::operator`).
//!
//! This file holds the expression tree ([`MExpr`]) and its conversions from program values; the rest is by area:
//! `operators` (building expressions), `keep` (keeping and ordering), `mixture`, `identity` (`hash_key`
//! equality), `prepare` (for evaluation) and `format` (`str`/`repr`).

use std::fmt;
use std::sync::Arc;

use super::ops::{missing_attribute, wrong_type};
use super::pool::KeepGen;
use super::{PErr, PValue};
use icepool_core::operator::Operator;
use icepool_core::value::builtins::{getitem, Index};
use icepool_core::{MultiDeal, Quantity, Value};

mod format;
mod identity;
mod keep;
mod mixture;
mod operators;
mod prepare;

pub(crate) use format::{mx_str, pvalue_repr, pvalue_str};
pub(crate) use identity::same_expression;
pub(crate) use keep::{force_order, max_pair, multiset_keep, multiset_keep_seq, sort_pair, versus};
pub(crate) use mixture::{add_inner, keepable_mixture, map_mixture, mixture, weightless};
use operators::int_arg;
pub(crate) use operators::{binary_op, counts, filter_outcomes, map_counts, method};
pub(crate) use prepare::{prepare, product};

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
    /// `name` is what `str()` shows: the function's parameter name.
    Param {
        arg_index: usize,
        star_index: Option<usize>,
        name: Arc<str>,
    },
    /// `MultisetTupleParameter`: a multiset tuple argument (a `MultiDeal`) of a `@multiset_function`, not starred.
    TupleParam {
        arg_index: usize,
        len: usize,
        name: Arc<str>,
    },
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
    /// A `MultisetKeep`'s index as it was given, which only `str()` shows.
    pub keep_index: Option<KeepIndexArg>,
}

/// The index of a `MultisetKeep` as Python keeps it (`self._index`): a slice's `start, stop, step`, or the sequence
/// (or invalid index) itself.
#[derive(Clone)]
pub enum KeepIndexArg {
    Slice(Box<[PValue; 3]>),
    Given(PValue),
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
            keep_index: None,
        }))
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
            PValue::Value(Value::None) => wrong_type("'NoneType' object is not subscriptable"),
            _ => Err(PErr::Unimplemented("indexing this object".into())),
        };
    };
    Ok(match int_arg(&index) {
        // An index beyond 64 bits is out of range anyway (IndexError when evaluated): saturating keeps that.
        Some(i) => {
            let i = i
                .to_i64()
                .unwrap_or(if i.is_negative() { i64::MIN } else { i64::MAX });
            PValue::Expr(MExpr::TupleIndex(Arc::new((e, i))))
        }
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
            PValue::Value(Value::None) => wrong_type("'NoneType' object is not subscriptable"),
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
            PValue::MultiDeal(_) | PValue::Expr(_) => {
                missing_attribute("a multiset tuple expression has no multiset expression methods")
            }
            _ => missing_attribute("not a multiset expression"),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_names_the_python_class() {
        let p = MExpr::Param {
            arg_index: 1,
            star_index: None,
            name: "p1".into(),
        };
        assert_eq!(format!("{p:?}"), "MultisetParameter(1)");
        let index = MExpr::TupleIndex(Arc::new((p, 0)));
        assert_eq!(
            format!("{index:?}"),
            "MultisetTupleIndex(MultisetParameter(1))"
        );
    }
}
