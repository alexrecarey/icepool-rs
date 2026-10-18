//! Weightless generators and mixtures of expressions (`MultisetMixture`).

use super::identity::{hashable, same_expression};
use super::{as_expression, as_tuple_expression, into_pvalue, MExpr};
use crate::ops::{missing_attribute, py_iter, unhashable_list};
use crate::{PErr, PValue};
use icepool_core::Quantity;
use std::sync::Arc;

/// `g.weightless()` (node `weightless`): only multiset generators have the method, and a `WeightlessGenerator`
/// returns itself.
pub fn weightless(g: PValue) -> Result<PValue, PErr> {
    match as_expression(&g) {
        Some(MExpr::Gen(g)) => Ok(PValue::Expr(MExpr::Weightless(g))),
        Some(e @ MExpr::Weightless(_)) => Ok(PValue::Expr(e)),
        _ => missing_attribute("object has no attribute 'weightless'"),
    }
}

/// `data[inner] += weight` for a mixture's `defaultdict(int)` of inners.
pub(crate) fn add_inner(
    data: &mut Vec<(MExpr, Quantity)>,
    inner: MExpr,
    weight: Quantity,
) -> Result<(), PErr> {
    if !hashable(&inner) {
        return unhashable_list();
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
