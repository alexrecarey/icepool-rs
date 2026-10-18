//! Expression identity: `hash_key` equality, which a dict of expressions (`MultisetMixture`) relies on.

use super::MExpr;
use crate::pool::KeepGen;
use crate::PErr;
use icepool_core::operator::{Operator, OutcomeFilter};
use icepool_core::Value;
use std::sync::Arc;

/// Whether a dict holding expression `a` finds it equal to `b` (`MultisetMixture` counts its inners in one): their
/// `hash_key`s. Expressions of different classes have different hash keys and are never compared.
pub(crate) fn same_expression(a: &MExpr, b: &MExpr) -> Result<bool, PErr> {
    Ok(match (a, b) {
        (MExpr::Gen(a), MExpr::Gen(b)) => same_generator(a, b),
        // `(WeightlessGenerator, wrapped.hash_key)`.
        (MExpr::Weightless(a), MExpr::Weightless(b)) => same_generator(a, b),
        (MExpr::MultiDeal(a), MExpr::MultiDeal(b)) => a == b,
        (
            MExpr::Param {
                arg_index: a,
                star_index: sa,
                ..
            },
            MExpr::Param {
                arg_index: b,
                star_index: sb,
                ..
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
        // Expressions of different classes have different hash keys (every pair of the same kind is above).
        _ => false,
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
        | (O::Unique(a), O::Unique(b)) => a == b,
        (O::ForceOrder(a), O::ForceOrder(b)) => a == b,
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
pub(super) fn hashable(e: &MExpr) -> bool {
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
