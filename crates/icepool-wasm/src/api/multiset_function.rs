//! `@multiset_function` (evaluator/multiset_function.py) for JS functions: preparing a traced function into the call
//! tree the core evaluates (`icepool_core::evaluator::multiset_function`), as the Python facade does for Python
//! functions (crates/icepool-py/src/evaluate.rs `evaluate_tree`) and the program runner for its own
//! (crates/icepool-harness/src/multiset_function.rs, whose structure this follows).
//!
//! The tracing is the TypeScript's (js/src/evaluator.ts): Python's `MultisetFunctionEvaluator._prepare` binds the
//! arguments and calls the wrapped function with parameter expressions, and an evaluation whose inputs hold a
//! parameter returns a `MultisetFunctionRawResult` instead of a die. The core asks for a trace where Python makes
//! one (`host::trace_function`): when the function is called, and again each time a call of a multiset function
//! recorded in a body is prepared (`prepare_inner`), so nested functions are traced lazily and once per
//! preparation of the outer inputs, as in Python, a recursion level each. The TypeScript hands the trace back as
//! an [`ExpressionBuilder`] holding the body's raw results ([`Raw`]), each pushed after the nodes of its inputs.

use std::rc::Rc;

use icepool_core::evaluator::multiset_function::{AnyState, Call, DynEvaluator, Erased, MultisetFunction};
use icepool_core::evaluator::{
    evaluate_call_tree, top_level_depth_check, Count, Final, MultisetEvaluator, MultisetSource, Order,
};
use icepool_core::expression::CallTree;
use icepool_core::value::recursion::level;
use icepool_core::{Error, Int, Quantity, Value};
use wasm_bindgen::convert::TryFromJsValue;
use wasm_bindgen::prelude::*;

use super::convert::value_to_js;
use super::error::{JsResult, OrThrow};
use super::evaluate::{envelope, prepare_inputs, product, with_builtin, BuiltinVisitor, ExpressionBuilder, Node};
use super::evaluator::{evaluations_to_js, HostEvaluator};
use super::host::{self, JsEvaluator};

/// An evaluator called in a function body (or a constant result), type-erased, its final outcomes JS values;
/// shared by every preparation of the body.
type Leaf = Rc<dyn DynEvaluator<Value, JsValue>>;

/// What a `MultisetFunctionRawResult` records: an evaluator and the expressions it evaluates.
pub(super) enum Raw {
    /// A built-in or JS evaluator, or `ConstantEvaluator` (no inputs).
    Leaf { leaf: Leaf, inputs: Vec<Node> },
    /// A call of a multiset function (the TypeScript `MultisetFunction`) with these keyword arguments (`star` and
    /// the function's own, a JS object): traced when it is prepared.
    Function { function: JsValue, inputs: Vec<Node>, kwargs: JsValue },
}

/// `ConstantEvaluator(final_outcome)`: a function result that is not an evaluation. No state; the final outcome
/// is the value.
struct Constant(Option<JsValue>);

impl MultisetEvaluator<Value> for Constant {
    type State = ();
    type Final = JsValue;

    fn initial_state(&self, _: Order, _: &[Value], _: &[Option<Int>]) -> icepool_core::Result<()> {
        Ok(())
    }

    fn next_state(&self, _: &(), _: Order, _: &Value, _: &[Int]) -> icepool_core::Result<Option<()>> {
        Ok(Some(()))
    }

    fn final_outcome(&self, _: &(), _: Order, _: &[Value], _: &[Option<Int>]) -> icepool_core::Result<Final<JsValue>> {
        // As `finalize_evaluation` sees it: `None` and the reroll types (`None` here) are special.
        Ok(match &self.0 {
            None => Final::Reroll,
            Some(x) if x.is_null_or_undefined() => Final::None,
            Some(x) => Final::Outcome(x.clone()),
        })
    }
}

/// A shared [`Leaf`] as the core's call takes it (an owned evaluator).
struct Shared(Leaf);

impl DynEvaluator<Value, JsValue> for Shared {
    fn initial_state(
        &self,
        order: Order,
        outcomes: &[Value],
        sizes: &[Option<Count>],
    ) -> icepool_core::Result<AnyState> {
        self.0.initial_state(order, outcomes, sizes)
    }

    fn next_state(
        &self,
        state: &AnyState,
        order: Order,
        outcome: &Value,
        counts: &[Count],
    ) -> icepool_core::Result<Option<AnyState>> {
        self.0.next_state(state, order, outcome, counts)
    }

    fn extra_outcomes(&self, outcomes: &[Value]) -> icepool_core::Result<Vec<Value>> {
        self.0.extra_outcomes(outcomes)
    }

    fn final_outcome(
        &self,
        state: &AnyState,
        order: Order,
        outcomes: &[Value],
        sizes: &[Option<Count>],
    ) -> icepool_core::Result<Final<JsValue>> {
        self.0.final_outcome(state, order, outcomes, sizes)
    }
}

/// A built-in evaluator as a [`Leaf`], its final outcomes made JS values.
struct BuiltinLeaf;

impl BuiltinVisitor for BuiltinLeaf {
    type Out = Leaf;

    fn visit<E>(self, evaluator: E) -> JsResult<Leaf>
    where
        E: MultisetEvaluator<Value, Final = Value> + 'static,
    {
        Ok(Rc::new(Erased { evaluator, map_final: |v: Value| value_to_js(&v) }))
    }
}

/// The main side of a prepared call ([`Call`] holds its evaluators uniquely), shared between preparations.
#[derive(Clone)]
enum CallSpec {
    Leaf(Leaf),
    Function(Box<CallSpec>),
    Joint(Vec<CallSpec>),
}

impl CallSpec {
    fn call(&self) -> Call<'static, Value, JsValue> {
        match self {
            CallSpec::Leaf(leaf) => Call::Evaluator(Box::new(Shared(leaf.clone()))),
            CallSpec::Function(inner) => Call::Function(Box::new(inner.call())),
            CallSpec::Joint(inners) => Call::Joint(inners.iter().map(CallSpec::call).collect()),
        }
    }
}

/// One prepared evaluation (`_prepare` yields several with a mixture): the call tree, its main side, the sources
/// of its free variables, and its weight.
#[derive(Clone)]
struct Prepared {
    tree: CallTree<Value>,
    call: CallSpec,
    sources: Vec<MultisetSource<Value>>,
    weight: Quantity,
}

/// `MultisetEvaluator._prepare(input_exps)` of a leaf evaluator in a body.
fn prepare_leaf(leaf: &Leaf, inputs: &[Node]) -> icepool_core::Result<Vec<Prepared>> {
    Ok(prepare_inputs(inputs)?
        .into_iter()
        .map(|(flats, sources, weight)| Prepared {
            tree: CallTree::leaf(flats),
            call: CallSpec::Leaf(leaf.clone()),
            sources,
            weight,
        })
        .collect())
}

/// `MultisetFunctionRawResult.prepare_inner`: the recorded evaluator's `_prepare`.
fn prepare_raw(raw: &Raw) -> icepool_core::Result<Vec<Prepared>> {
    // Multiset functions calling multiset functions nest (a body may even call its own function): a level each
    // (`value::recursion`), as Python's recursion limit stops them.
    let _level = level()?;
    match raw {
        Raw::Leaf { leaf, inputs } => prepare_leaf(leaf, inputs),
        Raw::Function { function, inputs, kwargs } => prepare_function(function, inputs, kwargs),
    }
}

/// What tracing a function body gave: its raw results (one unless `joint`).
fn trace(function: &JsValue, inputs: usize, kwargs: &JsValue) -> icepool_core::Result<(Vec<Raw>, bool)> {
    let inputs = u32::try_from(inputs).map_err(|_| Error::TooLarge("too many inputs".into()))?;
    let parts = host::trace_function(function, inputs, kwargs);
    let joint = parts.get(2).and_then(JsValue::as_bool) == Some(true);
    let builder = envelope(parts)?;
    let builder = ExpressionBuilder::try_from_js_value(builder)
        .map_err(|_| Error::WrongType("a trace that is not an ExpressionBuilder".into()))?;
    Ok((builder.raws, joint))
}

/// `MultisetFunctionEvaluator._prepare(input_exps, kwargs)`: traces the body with one parameter per input, then
/// prepares what it returned around the inputs' own flats.
fn prepare_function(function: &JsValue, inputs: &[Node], kwargs: &JsValue) -> icepool_core::Result<Vec<Prepared>> {
    let (mut raws, joint) = trace(function, inputs.len(), kwargs)?;
    if joint {
        return prepare_joint(inputs, &raws);
    }
    match (raws.pop(), raws.is_empty()) {
        (Some(raw), true) => prepare_single(inputs, &raw),
        _ => Err(Error::InvalidArgument("a trace with one result holds one raw result".into())),
    }
}

/// `prepare_multiset_function`: for each preparation of the inputs, their flats and then one call, prepared
/// again for each.
fn prepare_single(outer: &[Node], raw: &Raw) -> icepool_core::Result<Vec<Prepared>> {
    let mut result = Vec::new();
    for (flats, sources, weight) in prepare_inputs(outer)? {
        for inner in prepare_raw(raw)? {
            result.push(Prepared {
                tree: CallTree { flats: flats.clone(), calls: vec![inner.tree] },
                call: CallSpec::Function(Box::new(inner.call)),
                sources: sources.iter().cloned().chain(inner.sources).collect(),
                weight: &weight * inner.weight,
            });
        }
    }
    Ok(result)
}

/// `prepare_multiset_joint_function`: for each preparation of the inputs, their flats and then one call per
/// result (each combination of the results' preparations).
fn prepare_joint(outer: &[Node], raws: &[Raw]) -> icepool_core::Result<Vec<Prepared>> {
    let mut result = Vec::new();
    for (flats, sources, weight) in prepare_inputs(outer)? {
        let inners = raws.iter().map(prepare_raw).collect::<icepool_core::Result<Vec<_>>>()?;
        for combination in product(&inners) {
            if combination.is_empty() {
                // `zip(*inner)` over the one empty product: nothing to unpack into four names.
                return Err(Error::InvalidArgument("not enough values to unpack (expected 4, got 0)".into()));
            }
            let mut prepared = Prepared {
                tree: CallTree { flats: flats.clone(), calls: Vec::new() },
                call: CallSpec::Joint(Vec::new()),
                sources: sources.clone(),
                weight: weight.clone(),
            };
            let mut calls = Vec::new();
            for inner in combination {
                prepared.tree.calls.push(inner.tree.clone());
                calls.push(inner.call.clone());
                prepared.sources.extend(inner.sources.iter().cloned());
                prepared.weight *= &inner.weight;
            }
            prepared.call = CallSpec::Joint(calls);
            result.push(prepared);
        }
    }
    Ok(result)
}

/// `icepool.tupleize(*final_outcomes)` for a joint function: `Reroll` if any is a reroll type, else the
/// TypeScript's `tupleize` (a tuple, or a die of tuples if any is a die).
fn tupleize(finals: Vec<Final<JsValue>>) -> icepool_core::Result<Final<JsValue>> {
    let mut items = Vec::with_capacity(finals.len());
    for f in finals {
        items.push(match f {
            Final::Reroll => return Ok(Final::Reroll),
            Final::None => JsValue::NULL,
            Final::Outcome(x) => x,
        });
    }
    envelope(host::tupleize(items)).map(Final::Outcome)
}

#[wasm_bindgen]
impl ExpressionBuilder {
    /// A raw result of the built-in evaluator `name` (with `params`, as for `evaluate`) over the last `inputs`
    /// nodes.
    #[wasm_bindgen(js_name = rawBuiltin)]
    pub fn raw_builtin(&mut self, name: &str, params: Vec<JsValue>, inputs: usize) -> JsResult<()> {
        let leaf = with_builtin(name, &params, BuiltinLeaf)?;
        let inputs = self.pop(inputs)?;
        self.raws.push(Raw::Leaf { leaf, inputs });
        Ok(())
    }

    /// A raw result of a JS evaluator over the last `inputs` nodes.
    #[wasm_bindgen(js_name = rawHost)]
    pub fn raw_host(&mut self, evaluator: JsEvaluator, inputs: usize) -> JsResult<()> {
        let leaf: Leaf = Rc::new(Erased { evaluator: HostEvaluator(evaluator), map_final: Ok });
        let inputs = self.pop(inputs)?;
        self.raws.push(Raw::Leaf { leaf, inputs });
        Ok(())
    }

    /// A raw result of a multiset function called on the last `inputs` nodes with these keyword arguments.
    #[wasm_bindgen(js_name = rawFunction)]
    pub fn raw_function(&mut self, function: JsValue, inputs: usize, kwargs: JsValue) -> JsResult<()> {
        let inputs = self.pop(inputs)?;
        self.raws.push(Raw::Function { function, inputs, kwargs });
        Ok(())
    }

    /// `convert_die_to_raw_result` of a result that is not an evaluation: a constant (`reroll` if it is a reroll
    /// type).
    #[wasm_bindgen(js_name = rawConstant)]
    pub fn raw_constant(&mut self, value: JsValue, reroll: bool) {
        let leaf: Leaf = Rc::new(Erased { evaluator: Constant((!reroll).then_some(value)), map_final: Ok });
        self.raws.push(Raw::Leaf { leaf, inputs: Vec::new() });
    }

    /// `f.evaluate(*inputs, **kwargs)` of a multiset function over the last `inputs` nodes: its evaluations (see
    /// `evaluator::evaluations_to_js`). All preparations are made before any is evaluated, as the program runner
    /// does.
    #[wasm_bindgen(js_name = evaluateFunction)]
    pub fn evaluate_function(&mut self, function: JsValue, inputs: usize, kwargs: JsValue) -> JsResult<JsValue> {
        let inputs = self.pop(inputs)?;
        let mut evaluations = Vec::new();
        for prepared in prepare_function(&function, &inputs, &kwargs).or_throw()? {
            let evaluator = MultisetFunction { call: prepared.call.call(), tupleize: Box::new(tupleize) };
            let finals = evaluate_call_tree(&evaluator, &prepared.tree, &prepared.sources, top_level_depth_check)
                .or_throw()?;
            evaluations.push((finals, prepared.weight));
        }
        Ok(evaluations_to_js(evaluations))
    }
}
