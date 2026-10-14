//! Port of the evaluation side of icepool `expression/multiset_expression_base.py`: the dungeonlets and questlets
//! a multiset expression is prepared into, and how one evaluation input's flat list of them is driven
//! (`DungeonletCallTree.next_state`, `QuestletCallTree.initial_state` without multiset function calls).
//!
//! Preparing an expression (`_prepare`) flattens its tree in postfix order: each node after its children, the
//! input's own count produced by the last node. A generator leaves a [`Dungeonlet::FreeVariable`] in place of the
//! source it freed; the sources of all inputs, in the same order, go to the driver separately, which pops them and
//! hands their counts to the free variables in traversal order. An operator node computes its count from its
//! children's (`crate::operator`).
//!
//! The local states ("statelets") of all operator nodes of all inputs form one [`Statelets`] vector, part of the
//! driver's room key. Python keeps a `None` statelet for every free variable too; leaving them out changes no
//! equality.

use std::sync::Arc;

use crate::error::{Error, Result};
use crate::operator::{Operator, Statelet};
use crate::order::Order;
use crate::pool::Count;

/// One node of a prepared expression.
#[derive(Clone)]
pub enum Dungeonlet<O> {
    /// `MultisetFreeVariable` (with a `MultisetGeneratorQuestlet` / `MultisetTupleGeneratorQuestlet`): the count
    /// (and size) of the next source.
    FreeVariable,
    /// A `BodyDungeonlet` / `BodyQuestlet` pair: an operator over the counts of earlier nodes of the same flat,
    /// given by position (Python's relative `child_indexes`, made absolute).
    Operator {
        operator: Operator<O>,
        children: Vec<usize>,
    },
}

/// The prepared nodes of one evaluation input, in postfix order.
pub type Flat<O> = Arc<[Dungeonlet<O>]>;

/// The statelets of every operator node of every input, in traversal order.
pub type Statelets = Arc<[Statelet]>;

/// The flat of an input that is a generator: one free variable.
pub fn free_variable<O>() -> Flat<O> {
    Arc::from(vec![Dungeonlet::FreeVariable])
}

fn child_values<T: Clone>(values: &[T], children: &[usize]) -> Vec<T> {
    children.iter().map(|&i| values[i].clone()).collect()
}

fn no_source() -> Error {
    Error::Unsupported("an expression with more free variables than sources".into())
}

/// `QuestletCallTree.initial_state`: the initial statelets and the size of each input (the size its last node
/// produces). `source_sizes` yields the size of each source, consumed in traversal order (Python's generator
/// computes them lazily, so a size is only computed when its free variable is reached).
pub fn initial_state<O: PartialEq>(
    flats: &[Flat<O>],
    order: Order,
    mut source_sizes: impl Iterator<Item = Result<Option<Count>>>,
) -> Result<(Statelets, Vec<Option<Count>>)> {
    let mut statelets = Vec::new();
    let mut output_sizes = Vec::with_capacity(flats.len());
    for flat in flats {
        let mut sizes: Vec<Option<Count>> = Vec::with_capacity(flat.len());
        for dungeonlet in flat.iter() {
            let size = match dungeonlet {
                Dungeonlet::FreeVariable => source_sizes.next().ok_or_else(no_source)??,
                Dungeonlet::Operator { operator, children } => {
                    let (statelet, size) =
                        operator.initial_state(order, &child_values(&sizes, children))?;
                    statelets.push(statelet);
                    size
                }
            };
            sizes.push(size);
        }
        output_sizes.push(sizes.pop().flatten());
    }
    Ok((Arc::from(statelets), output_sizes))
}

/// `DungeonletCallTree.next_state`: advances the statelets for one outcome. `source_counts` are the counts the
/// sources produced for it, consumed by the free variables in traversal order. Returns the next statelets and
/// each input's count.
pub fn next_state<O: PartialEq>(
    flats: &[Flat<O>],
    statelets: &Statelets,
    order: Order,
    outcome: &O,
    source_counts: &[Count],
) -> Result<(Statelets, Vec<Count>)> {
    let mut source_counts = source_counts.iter();
    let mut statelets_in = statelets.iter();
    let mut next_statelets = Vec::with_capacity(statelets.len());
    let mut output_counts = Vec::with_capacity(flats.len());
    for flat in flats {
        let mut counts: Vec<Count> = Vec::with_capacity(flat.len());
        for dungeonlet in flat.iter() {
            let count = match dungeonlet {
                Dungeonlet::FreeVariable => source_counts.next().ok_or_else(no_source)?.clone(),
                Dungeonlet::Operator { operator, children } => {
                    let statelet = statelets_in.next().expect("one statelet per operator node");
                    let (next, count) = operator.next_state(
                        statelet,
                        order,
                        outcome,
                        &child_values(&counts, children),
                    )?;
                    next_statelets.push(next);
                    count
                }
            };
            counts.push(count);
        }
        output_counts.push(counts.pop().expect("a flat is never empty"));
    }
    Ok((Arc::from(next_statelets), output_counts))
}
