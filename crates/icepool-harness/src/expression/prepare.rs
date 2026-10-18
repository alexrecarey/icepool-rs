//! Preparing an expression for evaluation (`_prepare`): its flat of dungeonlets, the sources of its free
//! variables and the weight of each way to prepare it.

use super::{MExpr, OpNode};
use crate::PErr;
use icepool_core::evaluator::MultisetSource;
use icepool_core::expression::{Dungeonlet, Flat};
use icepool_core::value::builtins::Index;
use icepool_core::weightless::WeightlessSource;
use icepool_core::{Quantity, Value};

/// One way to prepare an expression: its dungeonlets, the sources of its free variables, and the weight.
type Part = (Vec<Dungeonlet<Value>>, Vec<MultisetSource<Value>>, Quantity);

impl MExpr {
    /// `_prepare`: each way to prepare this expression (one, unless a mixture is inside): its dungeonlets (postfix
    /// order), the sources of its free variables and the weight.
    fn prepare(&self) -> Result<Vec<Part>, PErr> {
        // Expressions nest (from JS, as deep as its calls make them): a level each (`value::recursion`).
        let _level = icepool_core::value::recursion::level()?;
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
                ..
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
}

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
