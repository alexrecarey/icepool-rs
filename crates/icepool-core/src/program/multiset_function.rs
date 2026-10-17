//! `@multiset_function` (harness/grammar/multiset_function.py): the Python side of evaluator/multiset_function.py,
//! i.e. tracing the function body with parameters and preparing the call tree that the core
//! (`crate::evaluator::multiset_function`) evaluates.
//!
//! A function is node `mf_def` (`params` positional parameters, `var` = a `*args` parameter, `kwonly` the names
//! of keyword-only parameters, `body` a lazy program); inside its body node `mf_var` is its `i`-th argument and
//! `mf_kwarg` a keyword argument (the innermost function's: a nested function's body is traced later, with its own
//! arguments). A host function (the JS facade's `multisetFunction(f)`) is a body too: it is called with the
//! parameters as Python calls the wrapped function. Evaluating an expression that holds a parameter does not
//! evaluate: it returns a [`RawResult`] (Python's `MultisetFunctionRawResult`) recording the evaluator and its
//! inputs. Calling the function (`mf_call`) traces the body and evaluates what it recorded.

use std::cell::RefCell;
use std::sync::Arc;

use dashu_int::IBig;
use serde_json::{Map, Value as Json};

use super::expression::{prepare, product, MExpr};
use super::host::HostFunction;
use super::ops::{new_die, py_iter, type_error};
use super::{eval, raise, PErr, PValue};
use crate::error::Error;
use crate::evaluator::multiset_function::{Call, DynEvaluator, Erased, MultisetFunction};
use crate::evaluator::{evaluate_call_tree, Final, MultisetEvaluator};
use crate::expression::CallTree;
use crate::order::Order;
use crate::pool::MultisetSource;
use crate::Int;
use crate::{Quantity, Value};

/// An evaluator called in a function body, type-erased, with final outcomes as program values.
pub type Leaf = Arc<dyn DynEvaluator<Value, PValue> + Send + Sync>;

/// A function made by `@multiset_function` (`MultisetFunctionEvaluator`): its signature and its body.
#[derive(Debug)]
pub struct FunctionDef {
    /// The number of positional parameters (none has a default).
    params: usize,
    /// Whether it has a `*args` parameter.
    var: bool,
    /// Its keyword-only parameters (none has a default): non-multiset values passed by `f(..., name=value)`.
    kwonly: Vec<String>,
    body: Body,
}

impl FunctionDef {
    /// `_get_positional_name(index)`: the name of the `index`-th multiset variable, shown by `str()` of its
    /// parameter. `mf_def` names its function's parameters `p0, p1, ...` and `*rest` (harness/grammar/
    /// multiset_function.py); a host function's names are not known, and it gets the same ones.
    fn positional_name(&self, index: usize) -> Arc<str> {
        if index < self.params {
            format!("p{index}").into()
        } else if self.var {
            format!("rest[{index}]").into()
        } else {
            // The exception will be raised later.
            "(too many args)".into()
        }
    }
}

/// What tracing the function runs: Python's `self._wrapped(*multiset_variables, **kwargs)`.
#[derive(Debug)]
enum Body {
    /// A lazy program (node `mf_def`), whose `mf_var` / `mf_kwarg` read the arguments.
    Program(Json),
    /// A host callable (the JS facade's `multisetFunction(f)`), called with the arguments: parameter
    /// expressions, then the keyword arguments.
    Host(Arc<dyn HostFunction>),
}

/// What a `MultisetFunctionRawResult` records.
#[derive(Clone)]
pub enum RawEvaluator {
    Leaf(Leaf),
    Function(Arc<FunctionDef>),
}

/// `MultisetFunctionRawResult(evaluator, body_exps, inner_kwargs)`: an evaluation inside a function body.
#[derive(Clone)]
pub struct RawResult {
    evaluator: RawEvaluator,
    exprs: Vec<MExpr>,
    /// The keyword arguments of a nested function call (`star` and the function's own); a leaf evaluator holds
    /// its own.
    kwargs: Vec<(String, PValue)>,
}

impl RawResult {
    /// The expressions evaluated.
    pub(super) fn exprs(&self) -> &[MExpr] {
        &self.exprs
    }
}

impl std::fmt::Debug for RawResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MultisetFunctionRawResult({:?})", self.exprs)
    }
}

/// The positional and keyword arguments of a traced body.
type Arguments = (Vec<PValue>, Vec<(String, PValue)>);

thread_local! {
    /// The arguments of the function bodies being traced, innermost last.
    static VARS: RefCell<Vec<Arguments>> = const { RefCell::new(Vec::new()) };
}

/// Node `mf_def`: `icepool.multiset_function(f)` for a function `f(p0, ..., p{params-1}[, *rest][, *, k...])`
/// whose body is the lazy program `body` (`kwonly`, the keyword-only names, may be left out).
pub fn mf_def(m: &Map<String, Json>) -> Result<PValue, PErr> {
    let keys: Vec<&str> = m
        .keys()
        .map(String::as_str)
        .filter(|k| *k != "op")
        .collect();
    let kwonly = m.get("kwonly");
    let (Some(params), Some(var), Some(body), true) = (
        m.get("params"),
        m.get("var"),
        m.get("body"),
        m.len() == 4 + usize::from(kwonly.is_some()),
    ) else {
        return raise(Error::Type(format!(
            "expected arguments [\"params\", \"var\", \"body\", \"kwonly\"], got {keys:?}"
        )));
    };
    let kwonly = match kwonly {
        None => Vec::new(),
        Some(names) => py_iter(eval(names)?)?
            .into_iter()
            .map(|name| match name {
                PValue::Value(Value::Str(name)) => Ok(name.to_string()),
                _ => Err(PErr::Unimplemented(
                    "a keyword-only parameter name that is not a str".into(),
                )),
            })
            .collect::<Result<_, _>>()?,
    };
    let params = match eval(params)? {
        PValue::Value(Value::Int(n)) => {
            usize::try_from(&n).map_err(|_| PErr::Unimplemented("a parameter count".into()))?
        }
        _ => return Err(PErr::Unimplemented("a parameter count".into())),
    };
    let var = matches!(eval(var)?, PValue::Value(Value::Bool(true)));
    Ok(PValue::MultisetFunction(Arc::new(FunctionDef {
        params,
        var,
        kwonly,
        body: Body::Program(body.clone()),
    })))
}

/// `icepool.multiset_function(f)` for a host function `f` with `params` required positional parameters, a
/// `*args` parameter if `var`, and the keyword-only parameters `kwonly` (the host states the signature: Python
/// reads it with `inspect.signature`).
pub fn host_function(
    f: Arc<dyn HostFunction>,
    params: usize,
    var: bool,
    kwonly: Vec<String>,
) -> PValue {
    PValue::MultisetFunction(Arc::new(FunctionDef {
        params,
        var,
        kwonly,
        body: Body::Host(f),
    }))
}

/// Node `mf_var`: argument `i` of the innermost function being traced (`args[i]`, negative from the end).
pub fn mf_var(i: PValue) -> Result<PValue, PErr> {
    let PValue::Value(Value::Int(i)) = i else {
        return Err(PErr::Unimplemented("a variable index".into()));
    };
    VARS.with(|v| {
        let v = v.borrow();
        let (args, _) = v
            .last()
            .ok_or_else(|| PErr::Unimplemented("mf_var outside a function body".into()))?;
        // Python's `args[i]`: a negative index counts from the end.
        let i = if i < IBig::ZERO {
            i + IBig::from(args.len())
        } else {
            i
        };
        usize::try_from(&i)
            .ok()
            .and_then(|i| args.get(i))
            .cloned()
            .ok_or_else(|| PErr::Raise(Error::Index("tuple index out of range".into())))
    })
}

/// Node `mf_kwarg`: keyword argument `name` of the innermost function being traced (KeyError if it has none of
/// that name).
pub fn mf_kwarg(name: PValue) -> Result<PValue, PErr> {
    let PValue::Value(Value::Str(name)) = name else {
        return Err(PErr::Unimplemented("a keyword argument name".into()));
    };
    VARS.with(|v| {
        let v = v.borrow();
        let (_, kwargs) = v
            .last()
            .ok_or_else(|| PErr::Unimplemented("mf_kwarg outside a function body".into()))?;
        kwargs
            .iter()
            .find(|(k, _)| **k == *name)
            .map(|(_, x)| x.clone())
            .ok_or_else(|| PErr::Raise(Error::Key(format!("{name:?}"))))
    })
}

/// Whether `evaluator.evaluate(*exprs)` is inside a function body: some input holds a parameter.
pub fn in_function(exprs: &[MExpr]) -> bool {
    exprs.iter().any(MExpr::has_parameter)
}

/// `evaluator.evaluate(*exprs)` inside a function body: the raw result that records it.
pub fn raw_result<E>(evaluator: E, exprs: &[MExpr]) -> PValue
where
    E: MultisetEvaluator<Value> + Send + Sync + 'static,
    E::State: 'static,
    E::Final: Into<PValue>,
{
    let leaf: Leaf = Arc::new(Erased {
        evaluator,
        map_final: |f: E::Final| Ok(f.into()),
    });
    PValue::RawResult(Arc::new(RawResult {
        evaluator: RawEvaluator::Leaf(leaf),
        exprs: exprs.to_vec(),
        kwargs: Vec::new(),
    }))
}

/// `ConstantEvaluator`: a function result that is not an evaluation. No state; the final outcome is the value.
struct Constant(PValue);

impl MultisetEvaluator<Value> for Constant {
    type State = ();
    type Final = PValue;

    fn initial_state(&self, _: Order, _: &[Value], _: &[Option<Int>]) -> crate::Result<()> {
        Ok(())
    }

    fn next_state(&self, _: &(), _: Order, _: &Value, _: &[Int]) -> crate::Result<Option<()>> {
        Ok(Some(()))
    }

    fn final_outcome(
        &self,
        _: &(),
        _: Order,
        _: &[Value],
        _: &[Option<Int>],
    ) -> crate::Result<Final<PValue>> {
        Ok(final_of(self.0.clone()))
    }
}

/// A final outcome as `finalize_evaluation` sees it: `None` and the reroll types are special.
fn final_of(x: PValue) -> Final<PValue> {
    match x {
        PValue::Value(Value::None) => Final::None,
        PValue::Reroll | PValue::Restart => Final::Reroll,
        x => Final::Outcome(x),
    }
}

/// `icepool.tupleize(*final_outcomes)` for a joint function: `Reroll` if any is a reroll type, else a tuple, or
/// a die of tuples if any is a die.
fn tupleize(finals: Vec<Final<PValue>>) -> crate::Result<Final<PValue>> {
    let mut items = Vec::with_capacity(finals.len());
    for f in finals {
        items.push(match f {
            Final::Reroll => return Ok(Final::Reroll),
            Final::None => PValue::Value(Value::None),
            Final::Outcome(x) => x,
        });
    }
    if items.iter().any(|x| matches!(x, PValue::Deck(_))) {
        return Err(Error::Unsupported("tupleizing decks".into()));
    }
    let items = items
        .into_iter()
        .map(|x| match x {
            PValue::DieWithTruth(d, _) => PValue::Die(d),
            x => x,
        })
        .collect::<Vec<_>>();
    if items.iter().any(|x| matches!(x, PValue::Die(_))) {
        let die = super::map::tupleize(items).map_err(core_error)?;
        return match die {
            crate::DieArg::Die(d) => Ok(Final::Outcome(PValue::Die(d))),
            _ => Err(Error::Unsupported("tupleize".into())),
        };
    }
    Ok(Final::Outcome(PValue::tuple(items)))
}

fn core_error(e: PErr) -> Error {
    match e {
        PErr::Raise(e) => e,
        PErr::Unimplemented(what) => Error::Unsupported(what),
    }
}

/// The main side of a prepared call ([`Call`], which holds its evaluators uniquely), shared between preparations.
#[derive(Clone)]
enum CallSpec {
    Leaf(Leaf),
    Function(Box<CallSpec>),
    Joint(Vec<CallSpec>),
}

impl CallSpec {
    fn call(&self) -> Call<'static, Value, PValue> {
        match self {
            CallSpec::Leaf(leaf) => Call::Evaluator(Box::new(leaf.clone())),
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
fn prepare_leaf(leaf: &Leaf, exprs: &[MExpr]) -> Result<Vec<Prepared>, PErr> {
    Ok(prepare(exprs)?
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
fn prepare_raw(raw: &RawResult) -> Result<Vec<Prepared>, PErr> {
    // Multiset functions calling multiset functions nest (a JS body may even call its own function): a level each
    // (`value::recursion`), as Python's recursion limit stops them.
    let _level = crate::value::recursion::level()?;
    match &raw.evaluator {
        RawEvaluator::Leaf(leaf) => prepare_leaf(leaf, &raw.exprs),
        RawEvaluator::Function(f) => prepare_function(f, &raw.exprs, &raw.kwargs),
    }
}

/// `convert_die_to_raw_result`: a raw result stays one; an expression without an evaluation raises; anything
/// else is a constant.
fn convert(x: PValue) -> Result<RawResult, PErr> {
    match x {
        PValue::RawResult(r) => Ok((*r).clone()),
        PValue::Pool(_) | PValue::Deal(_) | PValue::MultiDeal(_) | PValue::Expr(_) => type_error(
            "The result of a @multiset_function cannot contain expressions without a terminating evaluation.",
        ),
        x => {
            let leaf: Leaf = Arc::new(Erased {
                evaluator: Constant(x),
                map_final: Ok,
            });
            Ok(RawResult {
                evaluator: RawEvaluator::Leaf(leaf),
                exprs: Vec::new(),
                kwargs: Vec::new(),
            })
        }
    }
}

/// `MultisetExpressionBase._make_param(name, arg_index)`: a tuple expression makes a tuple parameter.
fn make_param(e: &MExpr, name: Arc<str>, arg_index: usize) -> Result<PValue, PErr> {
    Ok(PValue::Expr(match e.tuple_len()? {
        Some(len) => MExpr::TupleParam {
            arg_index,
            len,
            name,
        },
        None => MExpr::Param {
            arg_index,
            star_index: None,
            name,
        },
    }))
}

/// Python's `if star:` on the `star` keyword argument.
fn truthy(x: &PValue) -> Result<bool, PErr> {
    super::catalogue::truthy(x)
}

/// `MultisetFunctionEvaluator._prepare(input_exps, kwargs)`: traces the body with one parameter per input (or per
/// element of the first, starred, input), then prepares what it returned around the inputs' own flats.
fn prepare_function(
    f: &FunctionDef,
    inputs: &[MExpr],
    kwargs: &[(String, PValue)],
) -> Result<Vec<Prepared>, PErr> {
    let star = match kwargs.iter().find(|(k, _)| k == "star") {
        Some((_, star)) => truthy(star)?,
        // `infer_star`: more required positional parameters than inputs.
        None => f.params > inputs.len(),
    };
    // `self._wrapped(*multiset_variables, **kwargs)` without `star`.
    let kwargs: Vec<(String, PValue)> = kwargs
        .iter()
        .filter(|(k, _)| k != "star")
        .cloned()
        .collect();
    let mut vars = Vec::new();
    let rest = if star {
        let Some(first) = inputs.first() else {
            return raise(Error::Index("tuple index out of range".into()));
        };
        let Some(len) = first.tuple_len()? else {
            return type_error("object of this type has no len()");
        };
        for i in 0..len {
            vars.push(PValue::Expr(MExpr::Param {
                arg_index: 0,
                star_index: Some(i),
                name: f.positional_name(vars.len()),
            }));
        }
        1
    } else {
        0
    };
    for (i, e) in inputs.iter().enumerate().skip(rest) {
        vars.push(make_param(e, f.positional_name(vars.len()), i)?);
    }
    // `self._wrapped(*multiset_variables, **kwargs)`: Python binds the arguments to the signature.
    if vars.len() < f.params || (!f.var && vars.len() > f.params) {
        return type_error(format!(
            "wrapped() takes {} positional arguments but {} were given",
            f.params,
            vars.len()
        ));
    }
    if let Some((k, _)) = kwargs.iter().find(|(k, _)| !f.kwonly.contains(k)) {
        return type_error(format!(
            "wrapped() got an unexpected keyword argument '{k}'"
        ));
    }
    if let Some(k) = f
        .kwonly
        .iter()
        .find(|k| kwargs.iter().all(|(n, _)| n != *k))
    {
        return type_error(format!(
            "wrapped() missing 1 required keyword-only argument: '{k}'"
        ));
    }
    let result = match &f.body {
        Body::Program(body) => {
            VARS.with(|v| v.borrow_mut().push((vars, kwargs)));
            let result = eval(body);
            VARS.with(|v| v.borrow_mut().pop());
            result?
        }
        Body::Host(body) => body.call(vars, kwargs)?,
    };
    match result {
        PValue::RawResult(_)
        | PValue::Die(_)
        | PValue::DieWithTruth(..)
        | PValue::Pool(_)
        | PValue::Deal(_)
        | PValue::MultiDeal(_)
        | PValue::Expr(_) => prepare_single(inputs, convert(result)?),
        result => {
            let raws = py_iter(result)?
                .into_iter()
                .map(convert)
                .collect::<Result<Vec<_>, _>>()?;
            prepare_joint(inputs, raws)
        }
    }
}

/// `prepare_multiset_function`: for each preparation of the inputs, their flats and then one call, prepared
/// again for each.
fn prepare_single(outer: &[MExpr], raw: RawResult) -> Result<Vec<Prepared>, PErr> {
    let mut result = Vec::new();
    for (flats, sources, weight) in prepare(outer)? {
        for inner in prepare_raw(&raw)? {
            result.push(Prepared {
                tree: CallTree {
                    flats: flats.clone(),
                    calls: vec![inner.tree],
                },
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
fn prepare_joint(outer: &[MExpr], raws: Vec<RawResult>) -> Result<Vec<Prepared>, PErr> {
    let mut result = Vec::new();
    for (flats, sources, weight) in prepare(outer)? {
        let inners = raws
            .iter()
            .map(prepare_raw)
            .collect::<Result<Vec<_>, _>>()?;
        for combination in product(&inners) {
            if combination.is_empty() {
                // `zip(*inner)` over the one empty product: nothing to unpack into four names.
                return raise(Error::Value(
                    "not enough values to unpack (expected 4, got 0)".into(),
                ));
            }
            let mut prepared = Prepared {
                tree: CallTree {
                    flats: flats.clone(),
                    calls: Vec::new(),
                },
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

/// Node `mf_call`: `f(*args)`, or `f(*args, star=star)` unless `star` is `None`.
pub fn mf_call(f: PValue, args: PValue, star: PValue) -> Result<PValue, PErr> {
    let kwargs = match star {
        PValue::Value(Value::None) => Vec::new(),
        star => vec![("star".to_string(), star)],
    };
    call(f, args, kwargs)
}

/// Node `mf_call_kw`: `f(*args, **dict(kwargs))` (`star` among them or not).
pub fn mf_call_kw(f: PValue, args: PValue, kwargs: PValue) -> Result<PValue, PErr> {
    call(f, args, super::map::kwargs(kwargs)?)
}

/// `f.evaluate(*args, **kwargs)` for a multiset function `f`.
fn call(f: PValue, args: PValue, kwargs: Vec<(String, PValue)>) -> Result<PValue, PErr> {
    let PValue::MultisetFunction(f) = f else {
        return type_error("not a multiset function");
    };
    let exprs = py_iter(args)?
        .into_iter()
        .map(super::evaluate::implicit_convert)
        .collect::<Result<Vec<_>, _>>()?;
    // Inside another function's body: record the call.
    if exprs.iter().any(MExpr::has_parameter) {
        return Ok(PValue::RawResult(Arc::new(RawResult {
            evaluator: RawEvaluator::Function(f),
            exprs,
            kwargs,
        })));
    }
    let mut results = Vec::new();
    for prepared in prepare_function(&f, &exprs, &kwargs)? {
        let evaluator = MultisetFunction {
            call: prepared.call.call(),
            tupleize: Box::new(tupleize),
        };
        let finals = evaluate_call_tree(
            &evaluator,
            &prepared.tree,
            &prepared.sources,
            super::evaluate::check_recursion,
        )?;
        let (outcomes, weights): (Vec<_>, Vec<_>) = finals.into_iter().unzip();
        let weights: Vec<Value> = weights.into_iter().map(|w| Value::Int(w.into())).collect();
        results.push((new_die(outcomes, &weights)?, prepared.weight));
    }
    super::evaluate::mix(results)
}
