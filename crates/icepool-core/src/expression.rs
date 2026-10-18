//! Port of the evaluation side of icepool `expression/multiset_expression_base.py`: the dungeonlets and questlets
//! a multiset expression is prepared into, and how they are driven (`DungeonletCallTree.next_state`,
//! `QuestletCallTree.initial_state`), plus the nodes of `expression/multiset_parameter.py` and
//! `expression/multiset_tuple_expression.py`.
//!
//! Preparing an expression (`_prepare`) flattens its tree in postfix order: each node after its children, the
//! input's own count produced by the last node. A generator leaves a [`Dungeonlet::FreeVariable`] in place of the
//! source it freed; the sources of all inputs, in the same order, go to the driver separately, which pops them and
//! hands their counts to the free variables in traversal order. An operator node computes its count from its
//! children's (`crate::operator`).
//!
//! Inside a `@multiset_function` the evaluation is a [`CallTree`]: the flats of the function's arguments, then
//! one call per evaluation in its body, whose own flats read the arguments' counts through
//! [`Dungeonlet::Parameter`]s. What the tree produces per outcome is a [`CountTree`]: at a call without calls the
//! counts of its flats, else one subtree per call.
//!
//! The local states ("statelets") of all operator nodes of the whole tree form one [`Statelets`] vector (pre-order:
//! a call's flats, then its calls), part of the driver's room key. Python keeps a `None` statelet for every other
//! node too, and nests them like the call tree; neither changes any equality, the tree's shape being fixed.

use std::sync::Arc;

use crate::error::{Error, Result};
pub use crate::operator::{Operator, Statelet};
// For language bindings: the operator nodes a binding builds expressions of (operator/*.py).
pub use crate::operator::{
    BinaryOperator, CountFunction, KeepCountsComparison, MultisetKeepIndex, OutcomeFilter, OutcomePredicate,
};
use crate::order::Order;
use crate::pool::Count;
use crate::value::builtins::{getitem, Index};
use crate::{Int, Value};

/// One node of a prepared expression.
///
/// ```
/// use icepool_core::expression::{BinaryOperator, Dungeonlet, Operator};
///
/// // `a & b` of two generators, flattened in postfix order: both free variables, then the operator over them.
/// let flat: Vec<Dungeonlet<i64>> = vec![
///     Dungeonlet::FreeVariable,
///     Dungeonlet::FreeVariable,
///     Dungeonlet::Operator { operator: Operator::Binary(BinaryOperator::Intersection), children: vec![0, 1] },
/// ];
/// assert!(matches!(flat[2], Dungeonlet::Operator { ref children, .. } if children == &[0, 1]));
/// ```
#[derive(Clone, Debug)]
pub enum Dungeonlet<O> {
    /// `MultisetFreeVariable` (with a `MultisetGeneratorQuestlet` / `MultisetTupleGeneratorQuestlet`): the count
    /// (and size) of the next source.
    FreeVariable,
    /// A `BodyDungeonlet` / `BodyQuestlet` pair: an operator over the counts of earlier nodes of the same flat,
    /// given by position (Python's relative `child_indexes`, made absolute).
    Operator {
        /// The operator.
        operator: Operator<O>,
        /// The positions of its operands.
        children: Vec<usize>,
    },
    /// `MultisetParameterDungeonlet` / `MultisetParameterQuestlet`: the count (size) of argument `arg_index` of
    /// the enclosing `@multiset_function` call, subscripted by `star_index` if the argument was starred.
    Parameter {
        /// Which argument of the call.
        arg_index: usize,
        /// The element of a starred argument, if it was starred.
        star_index: Option<usize>,
    },
    /// `MultisetTupleIndex` (an `int` index) and `MultisetTupleSlice` (a slice): `child[index]` of a tuple count.
    TupleItem {
        /// The position of the tuple's node.
        child: usize,
        /// The index or slice.
        index: Index,
    },
}

/// The prepared nodes of one evaluation input, in postfix order.
///
/// ```
/// use icepool_core::expression::{free_variable, Dungeonlet, Flat};
///
/// let flat: Flat<i64> = free_variable();
/// assert_eq!(flat.len(), 1);
/// assert!(matches!(flat[0], Dungeonlet::FreeVariable));
/// ```
pub type Flat<O> = Arc<[Dungeonlet<O>]>;

/// The statelets of every operator node of every input, in traversal order.
///
/// ```
/// use icepool_core::evaluator::Order;
/// use icepool_core::expression::{free_variable, initial_state, CallTree, Statelets};
///
/// // A plain generator input (of unknown size) has no operator node, hence no statelets.
/// let tree = CallTree::<i64>::leaf(vec![free_variable()]);
/// let sizes = std::iter::once(Ok(None));
/// let (statelets, _): (Statelets, _) = initial_state(&tree, Order::Ascending, &[1, 2], sizes).unwrap();
/// assert!(statelets.is_empty());
/// ```
pub type Statelets = Arc<[Statelet]>;

/// `DungeonletCallTree` and `QuestletCallTree` in one: the flats of one call and the calls inside it.
///
/// ```
/// use icepool_core::expression::{free_variable, CallTree};
///
/// // A plain evaluation of two generators.
/// let tree = CallTree::<i64>::leaf(vec![free_variable(), free_variable()]);
/// assert_eq!(tree.flats.len(), 2);
/// assert!(tree.calls.is_empty());
/// ```
#[derive(Clone, Debug)]
pub struct CallTree<O> {
    /// The flats of this call's inputs.
    pub flats: Vec<Flat<O>>,
    /// The calls inside this one.
    pub calls: Vec<CallTree<O>>,
}

impl<O> CallTree<O> {
    /// The tree of a plain evaluation: its inputs' flats, no calls.
    pub fn leaf(flats: Vec<Flat<O>>) -> Self {
        CallTree {
            flats,
            calls: Vec::new(),
        }
    }
}

/// What a call tree produces: at a call without calls, the count (or size) of each of its flats; otherwise one
/// subtree per call (Python's nested tuples).
///
/// ```
/// use icepool_core::expression::CountTree;
///
/// let leaf = CountTree::Leaf(vec![1, 2]);
/// assert_eq!(leaf.leaf().unwrap(), &[1, 2]);
/// assert!(leaf.calls().is_empty());
/// // A plain evaluator cannot take the counts of calls.
/// assert!(CountTree::Calls(vec![leaf]).leaf().is_err());
/// ```
#[derive(Clone, Debug)]
pub enum CountTree<C> {
    /// One count per flat of a call without calls.
    Leaf(Vec<C>),
    /// One subtree per call.
    Calls(Vec<CountTree<C>>),
}

impl<C> CountTree<C> {
    /// The counts of a call without calls (a plain evaluator's arguments).
    pub fn leaf(&self) -> Result<&[C]> {
        match self {
            CountTree::Leaf(c) => Ok(c),
            CountTree::Calls(_) => Err(Error::Unported(
                "multiset function calls given to a plain evaluator".into(),
            )),
        }
    }

    /// The subtrees of a call with calls.
    pub fn calls(&self) -> &[CountTree<C>] {
        match self {
            CountTree::Calls(c) => c,
            CountTree::Leaf(_) => &[],
        }
    }
}

/// The flat of an input that is a generator: one free variable.
///
/// ```
/// use icepool_core::expression::{free_variable, Dungeonlet};
///
/// assert!(matches!(free_variable::<i64>()[..], [Dungeonlet::FreeVariable]));
/// ```
pub fn free_variable<O>() -> Flat<O> {
    Arc::from(vec![Dungeonlet::FreeVariable])
}

fn child_values<T: Clone>(values: &[T], children: &[usize]) -> Vec<T> {
    children.iter().map(|&i| values[i].clone()).collect()
}

fn no_source() -> Error {
    Error::Unported("an expression with more free variables than sources".into())
}

/// A count as the Python value it is: an `int` or a tuple of them.
///
/// ```
/// use icepool_core::evaluator::Count;
/// use icepool_core::expression::count_value;
/// use icepool_core::{Int, Value};
///
/// assert_eq!(count_value(&Count::Int(Int::from(3))), Value::from(3));
/// let pair = Count::Tuple(vec![Int::from(1), Int::from(2)].into());
/// assert_eq!(count_value(&pair), Value::tuple(vec![Value::from(1), Value::from(2)]));
/// ```
pub fn count_value(c: &Count) -> Value {
    match c {
        Count::Int(c) => Value::from(c),
        Count::Tuple(cs) => Value::tuple(cs.iter().map(Value::from).collect()),
    }
}

/// A Python value as a count, if it is one the port models: an `int` or a tuple of `int`s (not bools).
///
/// ```
/// use icepool_core::evaluator::Count;
/// use icepool_core::expression::value_count;
/// use icepool_core::{Int, Value};
///
/// assert_eq!(value_count(&Value::from(3)).unwrap(), Count::Int(Int::from(3)));
/// // `True` is an int to Python, but not a count the port models.
/// assert!(value_count(&Value::Bool(true)).is_err());
/// assert!(value_count(&Value::str("a")).is_err());
/// ```
pub fn value_count(v: &Value) -> Result<Count> {
    match v {
        Value::Int(i) => Ok(Count::Int(i.into())),
        Value::Tuple(t) => t
            .iter()
            .map(|x| match x {
                Value::Int(i) => Some(Int::from(i)),
                _ => None,
            })
            .collect::<Option<Vec<Int>>>()
            .map(|t| Count::Tuple(t.into()))
            .ok_or_else(|| Error::Unported("a tuple count that is not ints".into())),
        _ => Err(Error::Unported("a count that is not an int".into())),
    }
}

/// `count[index]` with Python's subscript semantics: a tuple count is indexed or sliced, anything else raises
/// `TypeError` ('int' / 'NoneType' object is not subscriptable).
fn subscript(c: Option<&Count>, index: &Index) -> Result<Count> {
    let v = c.map_or(Value::None, count_value);
    value_count(&getitem(&v, index)?)
}

/// `arg_sizes[arg_index]` / `arg_counts[arg_index][star_index]`.
fn parameter<C: Clone>(
    args: &[C],
    arg_index: usize,
    star_index: Option<usize>,
    as_count: impl Fn(&C) -> Option<&Count>,
    from_count: impl Fn(Count) -> C,
) -> Result<C> {
    let arg = args
        .get(arg_index)
        .ok_or_else(|| Error::IndexOutOfRange("tuple index out of range".into()))?;
    match star_index {
        None => Ok(arg.clone()),
        Some(i) => Ok(from_count(subscript(
            as_count(arg),
            &Index::Item(Value::from(i as i64)),
        )?)),
    }
}

/// `QuestletCallTree.initial_state`: the initial statelets and the size tree. `source_sizes` yields the size of
/// each source, consumed in traversal order (Python's generator computes them lazily, so a size is only computed
/// when its free variable is reached); `arg_sizes` are the sizes of the enclosing call's arguments.
fn initial_tree<O: PartialEq>(
    tree: &CallTree<O>,
    order: Order,
    outcomes: &[O],
    source_sizes: &mut impl Iterator<Item = Result<Option<Count>>>,
    arg_sizes: &[Option<Count>],
    statelets: &mut Vec<Statelet>,
) -> Result<CountTree<Option<Count>>> {
    let mut output_sizes = Vec::with_capacity(tree.flats.len());
    for flat in &tree.flats {
        let mut sizes: Vec<Option<Count>> = Vec::with_capacity(flat.len());
        for dungeonlet in flat.iter() {
            let size = match dungeonlet {
                Dungeonlet::FreeVariable => source_sizes.next().ok_or_else(no_source)??,
                Dungeonlet::Operator { operator, children } => {
                    let (statelet, size) =
                        operator.initial_state(order, outcomes, &child_values(&sizes, children))?;
                    statelets.push(statelet);
                    size
                }
                Dungeonlet::Parameter {
                    arg_index,
                    star_index,
                } => parameter(arg_sizes, *arg_index, *star_index, Option::as_ref, Some)?,
                Dungeonlet::TupleItem { child, index } => match &sizes[*child] {
                    // `MultisetTupleIndex._initial_state` returns a bare `None` here, which the caller fails to
                    // unpack.
                    None => {
                        return Err(Error::WrongType(
                            "cannot unpack non-iterable NoneType object".into(),
                        ))
                    }
                    Some(c) => Some(subscript(Some(c), index)?),
                },
            };
            sizes.push(size);
        }
        output_sizes.push(sizes.pop().expect("a flat is never empty"));
    }
    if tree.calls.is_empty() {
        return Ok(CountTree::Leaf(output_sizes));
    }
    let calls = tree
        .calls
        .iter()
        .map(|call| {
            initial_tree(
                call,
                order,
                outcomes,
                source_sizes,
                &output_sizes,
                statelets,
            )
        })
        .collect::<Result<_>>()?;
    Ok(CountTree::Calls(calls))
}

/// The initial statelets and size tree of a call tree (see `initial_tree`). `outcomes` are all the outcomes
/// the evaluation will see, ascending (a host operator's `_initial_state` gets them).
///
/// ```
/// use icepool_core::evaluator::{Count, Order};
/// use icepool_core::expression::{initial_state, CallTree, CountTree, Dungeonlet, Operator, Statelet};
/// use icepool_core::Int;
///
/// // `x * 2` of one generator of size 3.
/// let flat = vec![
///     Dungeonlet::FreeVariable,
///     Dungeonlet::Operator { operator: Operator::MultiplyCounts(Int::from(2)), children: vec![0] },
/// ];
/// let tree = CallTree::<i64>::leaf(vec![flat.into()]);
/// let sizes = std::iter::once(Ok(Some(Count::Int(Int::from(3)))));
/// let (statelets, size) = initial_state(&tree, Order::Ascending, &[1, 2], sizes).unwrap();
/// assert_eq!(&statelets[..], &[Statelet::None]);
/// assert!(matches!(size, CountTree::Leaf(s) if s == [Some(Count::Int(Int::from(6)))]));
/// ```
pub fn initial_state<O: PartialEq>(
    tree: &CallTree<O>,
    order: Order,
    outcomes: &[O],
    mut source_sizes: impl Iterator<Item = Result<Option<Count>>>,
) -> Result<(Statelets, CountTree<Option<Count>>)> {
    let mut statelets = Vec::new();
    let sizes = initial_tree(
        tree,
        order,
        outcomes,
        &mut source_sizes,
        &[],
        &mut statelets,
    )?;
    Ok((Arc::from(statelets), sizes))
}

/// `DungeonletCallTree.next_state` for one call: advances its statelets (read from and written to the shared
/// iterators) and returns its count tree.
fn next_tree<'s, O: PartialEq>(
    tree: &CallTree<O>,
    statelets_in: &mut impl Iterator<Item = &'s Statelet>,
    next_statelets: &mut Vec<Statelet>,
    order: Order,
    outcome: &O,
    source_counts: &mut std::slice::Iter<'_, Count>,
    arg_counts: &[Count],
) -> Result<CountTree<Count>> {
    let mut output_counts = Vec::with_capacity(tree.flats.len());
    for flat in &tree.flats {
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
                Dungeonlet::Parameter {
                    arg_index,
                    star_index,
                } => parameter(arg_counts, *arg_index, *star_index, |c| Some(c), |c| c)?,
                Dungeonlet::TupleItem { child, index } => subscript(Some(&counts[*child]), index)?,
            };
            counts.push(count);
        }
        output_counts.push(counts.pop().expect("a flat is never empty"));
    }
    if tree.calls.is_empty() {
        return Ok(CountTree::Leaf(output_counts));
    }
    let calls = tree
        .calls
        .iter()
        .map(|call| {
            next_tree(
                call,
                statelets_in,
                next_statelets,
                order,
                outcome,
                source_counts,
                &output_counts,
            )
        })
        .collect::<Result<_>>()?;
    Ok(CountTree::Calls(calls))
}

/// `DungeonletCallTree.next_state`: advances the statelets for one outcome. `source_counts` are the counts the
/// sources produced for it, consumed by the free variables in traversal order. Returns the next statelets and the
/// count tree.
///
/// ```
/// use icepool_core::evaluator::{Count, Order};
/// use icepool_core::expression::{next_state, CallTree, Dungeonlet, Operator, Statelet};
/// use icepool_core::Int;
///
/// // `x * 2` of one generator that produced 2 of the outcome 5.
/// let flat = vec![
///     Dungeonlet::FreeVariable,
///     Dungeonlet::Operator { operator: Operator::MultiplyCounts(Int::from(2)), children: vec![0] },
/// ];
/// let tree = CallTree::<i64>::leaf(vec![flat.into()]);
/// let statelets = vec![Statelet::None].into();
/// let (_, counts) = next_state(&tree, &statelets, Order::Ascending, &5, &[Count::Int(Int::from(2))]).unwrap();
/// assert_eq!(counts.leaf().unwrap(), &[Count::Int(Int::from(4))]);
/// ```
pub fn next_state<O: PartialEq>(
    tree: &CallTree<O>,
    statelets: &Statelets,
    order: Order,
    outcome: &O,
    source_counts: &[Count],
) -> Result<(Statelets, CountTree<Count>)> {
    let mut statelets_in = statelets.iter();
    let mut next_statelets = Vec::with_capacity(statelets.len());
    let counts = next_tree(
        tree,
        &mut statelets_in,
        &mut next_statelets,
        order,
        outcome,
        &mut source_counts.iter(),
        &[],
    )?;
    Ok((Arc::from(next_statelets), counts))
}
