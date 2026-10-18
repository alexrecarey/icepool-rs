//! Evaluating multiset expressions of the TypeScript API (js/src/multiset.ts) with the core's built-in evaluators
//! (evaluator/basic.py, poker.py, keep.py, comparison.py).
//!
//! The expression tree is the TypeScript's: `MultisetExpression` objects, as in the Python facade, where upstream's
//! Python expression objects are turned into the core's tree when evaluated (crates/icepool-py/src/evaluate.rs).
//! Here the TypeScript walks its tree in postfix order into an [`ExpressionBuilder`], which rebuilds it as the
//! core's operators over sources, prepares it (`_prepare`: the flats of dungeonlets, the sources of their free
//! variables, one way per combination of mixture inners) and runs the evaluator named by the TypeScript.
//!
//! The JS functions of `map_counts` and `keep_outcomes` are host callbacks: the core holds each JS function and calls
//! it through `__icepool_wasm_host.countFunction` / `outcomePredicate`. A callback's own exception goes through the
//! core as `Error::Callback` and the TypeScript rethrows it as itself (js/src/wasm.ts `callback`).
//!
//! JS evaluators and multiset functions are evaluated over the same tree (`evaluator.rs`, `multiset_function.rs`).

use std::sync::Arc;

use icepool_core::evaluator::argsort::{ArgsortEvaluator, ArgsortGroupedEvaluator, Tie};
use icepool_core::evaluator::basic::{
    EmptyEvaluator, ExpandEvaluator, ProductOfCountsEvaluator, SizeEvaluator, SumEvaluator,
};
use icepool_core::evaluator::comparison::{Comparison, ComparisonEvaluator};
use icepool_core::evaluator::keep::KeepEvaluator;
use icepool_core::evaluator::lexi::{compute_lexi_tuple_with_extra, LexiComparisonEvaluator};
use icepool_core::evaluator::poker::{
    AllCountsEvaluator, AllStraightsEvaluator, CountSubsetEvaluator, HighestOutcomeAndCountEvaluator,
    LargestCountAndOutcomeEvaluator, LargestCountEvaluator, LargestStraightAndOutcomeEvaluator,
    LargestStraightEvaluator,
};
use icepool_core::evaluator::{evaluate_expressions, top_level_depth_check, MultisetEvaluator, MultisetSource, Order};
use icepool_core::expression::{count_value, value_count, Dungeonlet, Flat};
use icepool_core::host::HostError;
use icepool_core::operator::{
    BinaryOperator, CountFunction, KeepCountsComparison, MultisetKeepIndex, Operator, OutcomeFilter,
    OutcomePredicate,
};
use icepool_core::evaluator::Count;
use icepool_core::value::evaluators::PyEvaluator;
use icepool_core::value::recursion::level;
use icepool_core::weightless::WeightlessSource;
use icepool_core::{Die, DieArg, Error, Int, Quantity, Value};
use wasm_bindgen::prelude::*;

use super::convert::{int_from_js, value_from_js, value_to_js};
use super::die::WasmDie;
use super::error::{JsResult, OrThrow};
use super::host;
use super::multiset::{bound, WasmGenerator};
use super::multiset_function::Raw;

/// A node of an expression as the core needs it to evaluate.
pub(super) enum Node {
    /// A generator: its source is a free variable of the flat.
    Source(MultisetSource<Value>),
    /// A `MultisetOperator` and its children.
    Operator(Operator<Value>, Vec<Node>),
    /// `MultisetMixture`: inner expressions with their weights.
    Mixture(Vec<(Node, Quantity)>),
    /// `MultisetParameter`: argument `arg_index` of the `@multiset_function` being traced (its `star_index`
    /// element if starred).
    Parameter { arg_index: usize, star_index: Option<usize> },
}

/// One way to prepare an expression: its dungeonlets (postfix order), the sources of its free variables, and the
/// weight.
type Part = (Vec<Dungeonlet<Value>>, Vec<MultisetSource<Value>>, Quantity);

/// One way to prepare all the inputs of an evaluation: one flat per input, the sources of their free variables (in
/// order), and the weight.
pub(super) type Prepared = (Vec<Flat<Value>>, Vec<MultisetSource<Value>>, Quantity);

impl Node {
    /// `_prepare`: each way to prepare this expression (one, unless a mixture is inside).
    fn prepare(&self) -> icepool_core::Result<Vec<Part>> {
        // Expressions nest as deep as JS makes them: a level each (`value::recursion`).
        let _level = level()?;
        match self {
            Node::Source(s) => Ok(vec![(vec![Dungeonlet::FreeVariable], vec![s.clone()], Quantity::ONE)]),
            // A parameter reads the count of an argument of the enclosing call: no source of its own.
            &Node::Parameter { arg_index, star_index } => {
                Ok(vec![(vec![Dungeonlet::Parameter { arg_index, star_index }], Vec::new(), Quantity::ONE)])
            }
            Node::Operator(operator, children) => {
                let children = children.iter().map(Node::prepare).collect::<icepool_core::Result<Vec<_>>>()?;
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
                        flat.push(Dungeonlet::Operator { operator: operator.clone(), children: positions });
                        (flat, sources, weight)
                    })
                    .collect())
            }
            // `MultisetMixture._prepare`: each inner's preparations, weighted by the inner's weight.
            Node::Mixture(inners) => {
                let mut parts = Vec::new();
                for (inner, weight) in inners {
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
        d => d.clone(),
    }
}

/// `itertools.product(*lists)`: every combination of one element from each list, the last varying fastest.
pub(super) fn product<T>(lists: &[Vec<T>]) -> Vec<Vec<&T>> {
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

/// The marker of a JS callback's exception inside the core: the TypeScript holds the exception itself.
pub(super) fn callback_error() -> Error {
    Error::Callback(HostError(Arc::new("raised by a JS callback")))
}

/// A callback's `[true, result]`, or `[false]` when it threw (the TypeScript keeps the exception).
pub(super) fn envelope(parts: Vec<JsValue>) -> icepool_core::Result<JsValue> {
    match parts.first().and_then(JsValue::as_bool) {
        Some(true) => Ok(parts.get(1).cloned().unwrap_or(JsValue::UNDEFINED)),
        _ => Err(callback_error()),
    }
}

/// `map_counts`' JS function: `function(outcome, *counts)`, an int.
struct JsCountFunction(JsValue);

impl CountFunction<Value> for JsCountFunction {
    fn call(&self, outcome: &Value, counts: &[Count]) -> icepool_core::Result<Count> {
        let counts = counts.iter().map(|c| value_to_js(&count_value(c))).collect::<icepool_core::Result<_>>()?;
        let result = envelope(host::count_function(&self.0, value_to_js(outcome)?, counts))?;
        value_count(&value_from_js(&result)?)
    }
}

/// `keep_outcomes`' JS callable: `bool(func(outcome))`, Python's truth taken by the TypeScript.
struct JsOutcomePredicate(JsValue);

impl OutcomePredicate<Value> for JsOutcomePredicate {
    fn test(&self, outcome: &Value) -> icepool_core::Result<bool> {
        let result = envelope(host::outcome_predicate(&self.0, value_to_js(outcome)?))?;
        Ok(result.as_bool().unwrap_or(false))
    }
}

/// An int parameter of an operator (a count constant), exactly.
fn int(x: &JsValue) -> JsResult<Int> {
    Ok(Int::from(int_from_js(x, "a count constant").or_throw()?))
}

/// An `Order` value: 1 (ascending), -1 (descending) or 0 (any).
fn order(x: i32) -> i64 {
    i64::from(x)
}

/// Builds the core's tree of one evaluation's inputs from the TypeScript's expressions, in postfix order (each node
/// after its children), then evaluates them. Tracing a multiset function's body fills one with what the body
/// returned: its raw results, each over nodes pushed before it (`multiset_function.rs`).
#[wasm_bindgen]
#[derive(Default)]
pub struct ExpressionBuilder {
    stack: Vec<Node>,
    pub(super) raws: Vec<Raw>,
}

#[wasm_bindgen]
impl ExpressionBuilder {
    #[wasm_bindgen(constructor)]
    pub fn new() -> ExpressionBuilder {
        ExpressionBuilder::default()
    }

    /// A keep generator (`_make_source`).
    pub fn generator(&mut self, g: &WasmGenerator) -> JsResult<()> {
        let source = g.g.source().or_throw()?;
        self.stack.push(Node::Source(source));
        Ok(())
    }

    /// `WeightlessGenerator(generator)` of the generator just pushed.
    pub fn weightless(&mut self) -> JsResult<()> {
        match self.pop(1)?.pop() {
            Some(Node::Source(s)) => {
                self.stack.push(Node::Source(MultisetSource::Weightless(WeightlessSource::new(s))));
                Ok(())
            }
            _ => Err(Error::Unported("weightless() of an expression".into())).or_throw(),
        }
    }

    /// A binary operator (operator/binary.py) over the last `arity` nodes, by its class's name.
    pub fn binary(&mut self, name: &str, arity: usize) -> JsResult<()> {
        let op = match name {
            "MultisetIntersection" => BinaryOperator::Intersection,
            "MultisetDifferenceDropNegative" => BinaryOperator::DifferenceDropNegative,
            "MultisetDifferenceKeepNegative" => BinaryOperator::DifferenceKeepNegative,
            "MultisetUnion" => BinaryOperator::Union,
            "MultisetAdditiveUnion" => BinaryOperator::AdditiveUnion,
            "MultisetSymmetricDifference" => BinaryOperator::SymmetricDifference,
            _ => return Err(Error::Unported(format!("no binary operator {name}"))).or_throw(),
        };
        self.operator(Operator::Binary(op), arity)
    }

    /// `MultisetMultiplyCounts(constant)`.
    #[wasm_bindgen(js_name = multiplyCounts)]
    pub fn multiply_counts(&mut self, constant: JsValue) -> JsResult<()> {
        self.operator(Operator::MultiplyCounts(int(&constant)?), 1)
    }

    /// `MultisetFloordivCounts(constant)`.
    #[wasm_bindgen(js_name = floordivCounts)]
    pub fn floordiv_counts(&mut self, constant: JsValue) -> JsResult<()> {
        self.operator(Operator::FloordivCounts(int(&constant)?), 1)
    }

    /// `MultisetModuloCounts(constant)`.
    #[wasm_bindgen(js_name = moduloCounts)]
    pub fn modulo_counts(&mut self, constant: JsValue) -> JsResult<()> {
        self.operator(Operator::ModuloCounts(int(&constant)?), 1)
    }

    /// `MultisetUnique(constant)`.
    pub fn unique(&mut self, constant: JsValue) -> JsResult<()> {
        self.operator(Operator::Unique(int(&constant)?), 1)
    }

    /// `MultisetKeepCounts(comparison, constant)` (the comparison already checked).
    #[wasm_bindgen(js_name = keepCounts)]
    pub fn keep_counts(&mut self, comparison: &str, constant: JsValue) -> JsResult<()> {
        let comparison = KeepCountsComparison::parse(comparison).or_throw()?;
        self.operator(Operator::KeepCounts(comparison, int(&constant)?), 1)
    }

    /// `MultisetFilterOutcomes` with a collection of outcomes (Python's `frozenset`).
    #[wasm_bindgen(js_name = filterSet)]
    pub fn filter_set(&mut self, invert: bool, outcomes: Vec<JsValue>) -> JsResult<()> {
        let set = outcomes.iter().map(value_from_js).collect::<icepool_core::Result<Vec<_>>>().or_throw()?;
        self.operator(Operator::FilterOutcomes { filter: OutcomeFilter::Set(set.into()), invert }, 1)
    }

    /// `MultisetFilterOutcomes` with a JS callable.
    #[wasm_bindgen(js_name = filterFunction)]
    pub fn filter_function(&mut self, invert: bool, f: JsValue) -> JsResult<()> {
        let filter = OutcomeFilter::Function(Arc::new(JsOutcomePredicate(f)));
        self.operator(Operator::FilterOutcomes { filter, invert }, 1)
    }

    /// `MultisetFilterOutcomesBinary`: the first node's count where the second's is positive (`!= invert`).
    #[wasm_bindgen(js_name = filterBinary)]
    pub fn filter_binary(&mut self, invert: bool) -> JsResult<()> {
        self.operator(Operator::FilterOutcomesBinary { invert }, 2)
    }

    /// `MultisetMapCounts` over the last `arity` nodes, with this JS function.
    #[wasm_bindgen(js_name = mapCounts)]
    pub fn map_counts(&mut self, f: JsValue, arity: usize) -> JsResult<()> {
        self.operator(Operator::MapCounts(Arc::new(JsCountFunction(f))), arity)
    }

    /// `MultisetKeep` with a slice index (bounds ints or null; `step` whether one was given).
    #[wasm_bindgen(js_name = keepSlice)]
    pub fn keep_slice(&mut self, start: JsValue, stop: JsValue, step: bool) -> JsResult<()> {
        let (start, stop) = (bound(&start)?, bound(&stop)?);
        self.operator(Operator::Keep(MultisetKeepIndex::Slice { start, stop, step }), 1)
    }

    /// `MultisetKeep` with a sequence index: ints, null for `...`.
    #[wasm_bindgen(js_name = keepSequence)]
    pub fn keep_sequence(&mut self, counts: Vec<JsValue>) -> JsResult<()> {
        let counts = counts
            .iter()
            .map(|c| if c.is_null() { Ok(None) } else { int(c).map(Some) })
            .collect::<JsResult<_>>()?;
        self.operator(Operator::Keep(MultisetKeepIndex::Seq(counts)), 1)
    }

    /// `MultisetKeep` with an index of another type (by its Python type name): it raises when evaluated.
    #[wasm_bindgen(js_name = keepInvalid)]
    pub fn keep_invalid(&mut self, type_name: &str) -> JsResult<()> {
        self.operator(Operator::Keep(MultisetKeepIndex::Invalid(type_name.into())), 1)
    }

    /// `MultisetSortPair` (`sort_order` already `Descending` if `Any` was given).
    #[wasm_bindgen(js_name = sortPair)]
    pub fn sort_pair(&mut self, comparison: &str, sort_order: i32, extra: &str) -> JsResult<()> {
        let op = Operator::SortPair { comparison: comparison.into(), sort_order: order(sort_order), extra: extra.into() };
        self.operator(op, 2)
    }

    /// `MultisetSortPairWhile`.
    #[wasm_bindgen(js_name = sortPairWhile)]
    pub fn sort_pair_while(&mut self, keep: bool, comparison: &str, sort_order: i32, extra: &str) -> JsResult<()> {
        let op = Operator::SortPairWhile {
            keep,
            comparison: comparison.into(),
            sort_order: order(sort_order),
            extra: extra.into(),
        };
        self.operator(op, 2)
    }

    /// `MultisetMaxPairEarly` (`early`) or `MultisetMaxPairLate`.
    #[wasm_bindgen(js_name = maxPair)]
    pub fn max_pair(&mut self, early: bool, max_order: i32, pair_equal: bool, keep: bool) -> JsResult<()> {
        let order = order(max_order);
        let op = if early {
            Operator::MaxPairEarly { order, pair_equal, keep }
        } else {
            Operator::MaxPairLate { order, pair_equal, keep }
        };
        self.operator(op, 2)
    }

    /// `MultisetVersus(lexi_tuple, order)`.
    pub fn versus(&mut self, lexi_tuple: Vec<i32>, versus_order: i32) -> JsResult<()> {
        let [a, b, c] = lexi_tuple[..] else {
            return Err(Error::InvalidArgument("a lexi tuple has three elements".into())).or_throw();
        };
        let op = Operator::Versus { lexi_tuple: (a.into(), b.into(), c.into()), order: order(versus_order) };
        self.operator(op, 2)
    }

    /// A `MultisetParameter` of the multiset function being traced: argument `arg_index` (its `star_index` element
    /// if starred).
    pub fn parameter(&mut self, arg_index: usize, star_index: Option<usize>) {
        self.stack.push(Node::Parameter { arg_index, star_index });
    }

    /// `MultisetForceOrder(force_order)`.
    #[wasm_bindgen(js_name = forceOrder)]
    pub fn force_order(&mut self, force_order: i32) -> JsResult<()> {
        self.operator(Operator::ForceOrder(order(force_order)), 1)
    }

    /// `MultisetMixture` of the last `weights.len()` nodes, with these weights.
    pub fn mixture(&mut self, weights: Vec<JsValue>) -> JsResult<()> {
        let inners = self.pop(weights.len())?;
        let weights = weights
            .iter()
            .map(|w| Quantity::try_from(int_from_js(w, "a weight")?).map_err(|_| Error::InvalidArgument("a negative weight".into())))
            .collect::<icepool_core::Result<Vec<_>>>()
            .or_throw()?;
        self.stack.push(Node::Mixture(inners.into_iter().zip(weights).collect()));
        Ok(())
    }

    /// `evaluator.evaluate(*inputs)` of the built-in evaluator `name` (with `params`, as for the Python facade's
    /// `evaluate_builtin`) over the last `inputs` nodes: each prepared evaluation gives
    /// `Die(final_outcomes, final_weights)`, and those are mixed by weight.
    pub fn evaluate(&mut self, name: &str, params: Vec<JsValue>, inputs: usize) -> JsResult<WasmDie> {
        let inputs = self.pop(inputs)?;
        with_builtin(name, &params, Run(&inputs)).map(WasmDie::new)
    }
}

impl ExpressionBuilder {
    pub(super) fn pop(&mut self, n: usize) -> JsResult<Vec<Node>> {
        if n > self.stack.len() {
            return Err(Error::InvalidArgument("an expression with too few children".into())).or_throw();
        }
        Ok(self.stack.split_off(self.stack.len() - n))
    }

    fn operator(&mut self, operator: Operator<Value>, arity: usize) -> JsResult<()> {
        let children = self.pop(arity)?;
        self.stack.push(Node::Operator(operator, children));
        Ok(())
    }
}

/// Something to do with a built-in evaluator of the core, whatever its type.
pub(super) trait BuiltinVisitor {
    type Out;
    fn visit<E>(self, evaluator: E) -> JsResult<Self::Out>
    where
        E: MultisetEvaluator<Value, Final = Value> + 'static;
}

/// The built-in evaluator `name` with its parameters (the Python facade's `builtin_spec` names), handed to
/// `visitor`.
pub(super) fn with_builtin<V: BuiltinVisitor>(name: &str, params: &[JsValue], visitor: V) -> JsResult<V::Out> {
    let param = |i: usize| -> JsResult<Value> {
        let p = params.get(i).ok_or_else(|| Error::InvalidArgument(format!("{name} needs a parameter")));
        value_from_js(p.or_throw()?).or_throw()
    };
    let flag = |i: usize| -> JsResult<bool> { Ok(param(i)? == Value::Bool(true)) };
    let text = |i: usize| -> JsResult<String> {
        match param(i)? {
            Value::Str(s) => Ok(s.to_string()),
            _ => Err(Error::Unported(format!("{name} needs a str"))).or_throw(),
        }
    };
    // `limit`: an int or None.
    let limit = |i: usize| -> JsResult<Option<i64>> {
        match param(i)? {
            Value::None => Ok(None),
            Value::Int(n) => i64::try_from(&n).map(Some).or_else(|_| Err(Error::Unported("a limit beyond 64 bits".into())).or_throw()),
            _ => Err(Error::Unported("a limit that is not an int".into())).or_throw(),
        }
    };
    let v = visitor;
    match name {
        "sum" => v.visit(SumEvaluator),
        "size" => v.visit(SizeEvaluator),
        "product_of_counts" => v.visit(ProductOfCountsEvaluator),
        "empty" => v.visit(EmptyEvaluator { any: flag(0)? }),
        "expand" => v.visit(ExpandEvaluator { ascending: flag(0)? }),
        "keep" => v.visit(KeepEvaluator {
            index: match param(0)? {
                Value::None => None,
                Value::Int(i) => Some(i.into()),
                _ => return Err(Error::Unported("a keep index that is not an int".into())).or_throw(),
            },
        }),
        "highest_outcome_and_count" => v.visit(PyEvaluator(HighestOutcomeAndCountEvaluator)),
        "all_counts" => v.visit(PyEvaluator(AllCountsEvaluator { filter: param(0)? })),
        "largest_count" => v.visit(PyEvaluator(LargestCountEvaluator)),
        "largest_count_and_outcome" => v.visit(PyEvaluator(LargestCountAndOutcomeEvaluator)),
        // `empty_divisor=None` is "not set".
        "count_subset" => v.visit(PyEvaluator(CountSubsetEvaluator {
            empty_divisor: match param(0)? {
                Value::None => None,
                d => Some(d),
            },
        })),
        "largest_straight" => v.visit(PyEvaluator(LargestStraightEvaluator)),
        "largest_straight_and_outcome" => {
            v.visit(PyEvaluator(LargestStraightAndOutcomeEvaluator { prioritize_highest: flag(0)? }))
        }
        "all_straights" => v.visit(PyEvaluator(AllStraightsEvaluator)),
        "is_proper_subset" => v.visit(ComparisonEvaluator(Comparison::IsProperSubset)),
        "is_subset" => v.visit(ComparisonEvaluator(Comparison::IsSubset)),
        "is_proper_superset" => v.visit(ComparisonEvaluator(Comparison::IsProperSuperset)),
        "is_superset" => v.visit(ComparisonEvaluator(Comparison::IsSuperset)),
        "is_equal_set" => v.visit(ComparisonEvaluator(Comparison::IsEqualSet)),
        "is_not_equal_set" => v.visit(ComparisonEvaluator(Comparison::IsNotEqualSet)),
        "is_disjoint_set" => v.visit(ComparisonEvaluator(Comparison::IsDisjointSet)),
        // `(tie, order, limit, limit_pad)`.
        "argsort" => v.visit(ArgsortEvaluator {
            tie: match text(0)?.as_str() {
                "drop" => Tie::Drop,
                "left" => Tie::Left,
                "right" => Tie::Right,
                _ => return Err(Error::InvalidArgument("tie must be \"drop\", \"left\", or \"right\".".into())).or_throw(),
            },
            order: param(1)?,
            limit: limit(2)?,
            limit_pad: match param(3)? {
                Value::None => None,
                pad => Some(pad),
            },
        }),
        // `(order, limit)`.
        "argsort_grouped" => v.visit(ArgsortGroupedEvaluator { order: param(0)?, limit: limit(1)? }),
        // `(max, comparison, extra)`: `leximax` (else `leximin`) with lexi.py's `compute_lexi_tuple_with_extra`.
        "lexi" => {
            let sort_order = if flag(0)? { Order::Descending } else { Order::Ascending };
            let lexi_tuple = compute_lexi_tuple_with_extra(&text(1)?, sort_order.value(), &text(2)?).or_throw()?;
            v.visit(LexiComparisonEvaluator { sort_order, lexi_tuple })
        }
        _ => Err(Error::Unported(format!("no built-in evaluator {name:?}"))).or_throw(),
    }
}

/// Runs a built-in evaluation over the inputs.
struct Run<'a>(&'a [Node]);

impl BuiltinVisitor for Run<'_> {
    type Out = Die<Value>;

    fn visit<E>(self, evaluator: E) -> JsResult<Die<Value>>
    where
        E: MultisetEvaluator<Value, Final = Value> + 'static,
    {
        evaluate(&evaluator, self.0).or_throw()
    }
}

/// `itertools.product(*(exp._prepare() for exp in input_exps))`: each way to prepare the inputs together.
pub(super) fn prepare_inputs(inputs: &[Node]) -> icepool_core::Result<Vec<Prepared>> {
    let parts = inputs.iter().map(Node::prepare).collect::<icepool_core::Result<Vec<_>>>()?;
    Ok(product(&parts)
        .into_iter()
        .map(|combination| {
            let mut flats: Vec<Flat<Value>> = Vec::with_capacity(combination.len());
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

/// The evaluations of `MultisetEvaluatorBase.evaluate`: for each way to prepare the inputs, the final outcomes with
/// their weights, and the preparation's weight.
pub(super) type Evaluations<F> = Vec<(Vec<(F, Quantity)>, Quantity)>;

/// Runs `evaluator` over each way to prepare the inputs (`dungeon.evaluate(quest, sources, kwargs)`).
pub(super) fn evaluations<E>(evaluator: &E, inputs: &[Node]) -> icepool_core::Result<Evaluations<E::Final>>
where
    E: MultisetEvaluator<Value>,
{
    prepare_inputs(inputs)?
        .into_iter()
        .map(|(flats, sources, weight)| {
            Ok((evaluate_expressions(evaluator, &flats, &sources, top_level_depth_check)?, weight))
        })
        .collect()
}

/// `MultisetEvaluatorBase.evaluate`: for each combination of the inputs' preparations, `Die(final_outcomes,
/// final_weights)`; then `final_data[result] += weight` (equal dice merge, the first kept) and `Die(final_data)`.
fn evaluate<E>(evaluator: &E, inputs: &[Node]) -> icepool_core::Result<Die<Value>>
where
    E: MultisetEvaluator<Value, Final = Value>,
{
    let mut data: Vec<(Die<Value>, Quantity)> = Vec::new();
    for (finals, weight) in evaluations(evaluator, inputs)? {
        let (outcomes, weights): (Vec<_>, Vec<_>) = finals.into_iter().map(|(o, w)| (DieArg::Outcome(o), w)).unzip();
        let die = Die::new(outcomes, weights)?;
        match data.iter_mut().find(|(d, _)| *d == die) {
            Some((_, w)) => *w += weight,
            None => data.push((die, weight)),
        }
    }
    let (dice, weights): (Vec<_>, Vec<_>) = data.into_iter().map(|(d, w)| (DieArg::Die(d), w)).unzip();
    Die::new(dice, weights)
}
