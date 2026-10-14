//! `@multiset_function` (harness/grammar/multiset_function.py): the Python side of evaluator/multiset_function.py,
//! i.e. tracing the function body with parameters and preparing the call tree that the core
//! (`crate::evaluator::multiset_function`) evaluates.
//!
//! A function is node `mf_def` (`params` positional parameters, `var` = a `*args` parameter, `body` a lazy
//! program); inside its body node `mf_var` is its `i`-th argument (the innermost function's: a nested function's
//! body is traced later, with its own arguments). Evaluating an expression that holds a parameter does not
//! evaluate: it returns a [`RawResult`] (Python's `MultisetFunctionRawResult`) recording the evaluator and its
//! inputs. Calling the function (`mf_call`) traces the body and evaluates what it recorded.

use std::cell::RefCell;
use std::sync::Arc;

use serde_json::{Map, Value as Json};

use super::expression::{prepare, MExpr};
use super::ops::{new_die, py_iter, type_error};
use super::{eval, raise, PErr, PValue};
use crate::error::Error;
use crate::evaluator::multiset_function::{
    Call, DynEvaluator, Erased, MultisetFunction,
};
use crate::evaluator::{evaluate_call_tree, Final, MultisetEvaluator};
use crate::expression::CallTree;
use crate::order::Order;
use crate::pool::MultisetSource;
use crate::Value;

/// An evaluator called in a function body, type-erased, with final outcomes as program values.
pub type Leaf = Arc<dyn DynEvaluator<Value, PValue> + Send + Sync>;

/// A function made by `@multiset_function` (`MultisetFunctionEvaluator`): its signature and its body.
#[derive(Debug)]
pub struct FunctionDef {
    /// The number of positional parameters (none has a default).
    params: usize,
    /// Whether it has a `*args` parameter.
    var: bool,
    body: Json,
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
    /// The keyword arguments of a nested function call (`star`); a leaf evaluator holds its own.
    kwargs: Vec<(String, PValue)>,
}

impl std::fmt::Debug for RawResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MultisetFunctionRawResult({:?})", self.exprs)
    }
}

thread_local! {
    /// The arguments of the function bodies being traced, innermost last.
    static VARS: RefCell<Vec<Vec<PValue>>> = const { RefCell::new(Vec::new()) };
}

/// Node `mf_def`: `icepool.multiset_function(f)` for a function `f(p0, ..., p{params-1}[, *rest])` whose body is
/// the lazy program `body`.
pub fn mf_def(m: &Map<String, Json>) -> Result<PValue, PErr> {
    let keys: Vec<&str> = m.keys().map(String::as_str).filter(|k| *k != "op").collect();
    let (Some(params), Some(var), Some(body), 4) =
        (m.get("params"), m.get("var"), m.get("body"), m.len())
    else {
        return raise(Error::Type(format!(
            "expected arguments [\"params\", \"var\", \"body\"], got {keys:?}"
        )));
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
        body: body.clone(),
    })))
}

/// Node `mf_var`: argument `i` of the innermost function being traced.
pub fn mf_var(i: PValue) -> Result<PValue, PErr> {
    let PValue::Value(Value::Int(i)) = i else {
        return Err(PErr::Unimplemented("a variable index".into()));
    };
    VARS.with(|v| {
        let v = v.borrow();
        let args = v
            .last()
            .ok_or_else(|| PErr::Unimplemented("mf_var outside a function body".into()))?;
        usize::try_from(&i)
            .ok()
            .and_then(|i| args.get(i))
            .cloned()
            .ok_or_else(|| PErr::Raise(Error::Index("tuple index out of range".into())))
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

    fn initial_state(&self, _: Order, _: &[Value], _: &[Option<i64>]) -> crate::Result<()> {
        Ok(())
    }

    fn next_state(&self, _: &(), _: Order, _: &Value, _: &[i64]) -> crate::Result<Option<()>> {
        Ok(Some(()))
    }

    fn final_outcome(
        &self,
        _: &(),
        _: Order,
        _: &[Value],
        _: &[Option<i64>],
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

/// One prepared evaluation: the call tree, its main side, and the sources of its free variables.
struct Prepared {
    tree: CallTree<Value>,
    call: Call<'static, Value, PValue>,
    sources: Vec<MultisetSource<Value>>,
}

/// `MultisetEvaluator._prepare(input_exps)` of a leaf evaluator in a body.
fn prepare_leaf(leaf: &Leaf, exprs: &[MExpr]) -> Result<Prepared, PErr> {
    let (flats, sources) = prepare(exprs)?;
    Ok(Prepared {
        tree: CallTree::leaf(flats),
        call: Call::Evaluator(Box::new(leaf.clone())),
        sources,
    })
}

/// `MultisetFunctionRawResult.prepare_inner`: the recorded evaluator's `_prepare`.
fn prepare_raw(raw: &RawResult) -> Result<Prepared, PErr> {
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
fn make_param(e: &MExpr, arg_index: usize) -> Result<PValue, PErr> {
    Ok(PValue::Expr(match e.tuple_len()? {
        Some(len) => MExpr::TupleParam { arg_index, len },
        None => MExpr::Param {
            arg_index,
            star_index: None,
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
) -> Result<Prepared, PErr> {
    let star = match kwargs.iter().find(|(k, _)| k == "star") {
        Some((_, star)) => truthy(star)?,
        // `infer_star`: more required parameters than inputs.
        None => f.params > inputs.len(),
    };
    if kwargs.iter().any(|(k, _)| k != "star") {
        return Err(PErr::Unimplemented("keyword arguments to a multiset function".into()));
    }
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
            }));
        }
        1
    } else {
        0
    };
    for (i, e) in inputs.iter().enumerate().skip(rest) {
        vars.push(make_param(e, i)?);
    }
    // `self._wrapped(*multiset_variables)`: Python checks the arity.
    if vars.len() < f.params || (!f.var && vars.len() > f.params) {
        return type_error(format!(
            "wrapped() takes {} positional arguments but {} were given",
            f.params,
            vars.len()
        ));
    }
    VARS.with(|v| v.borrow_mut().push(vars));
    let result = eval(&f.body);
    VARS.with(|v| v.borrow_mut().pop());
    let result = result?;
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

/// `prepare_multiset_function`: the inputs' flats, then one call.
fn prepare_single(outer: &[MExpr], raw: RawResult) -> Result<Prepared, PErr> {
    let (flats, mut sources) = prepare(outer)?;
    let inner = prepare_raw(&raw)?;
    sources.extend(inner.sources);
    Ok(Prepared {
        tree: CallTree {
            flats,
            calls: vec![inner.tree],
        },
        call: Call::Function(Box::new(inner.call)),
        sources,
    })
}

/// `prepare_multiset_joint_function`: the inputs' flats, then one call per result.
fn prepare_joint(outer: &[MExpr], raws: Vec<RawResult>) -> Result<Prepared, PErr> {
    let (flats, mut sources) = prepare(outer)?;
    if raws.is_empty() {
        // `zip(*inner)` over the one empty product: nothing to unpack into four names.
        return raise(Error::Value(
            "not enough values to unpack (expected 4, got 0)".into(),
        ));
    }
    let mut calls = Vec::with_capacity(raws.len());
    let mut trees = Vec::with_capacity(raws.len());
    for raw in &raws {
        let inner = prepare_raw(raw)?;
        sources.extend(inner.sources);
        calls.push(inner.call);
        trees.push(inner.tree);
    }
    Ok(Prepared {
        tree: CallTree {
            flats,
            calls: trees,
        },
        call: Call::Joint(calls),
        sources,
    })
}

/// Node `mf_call`: `f(*args)`, or `f(*args, star=star)` unless `star` is `None`.
pub fn mf_call(f: PValue, args: PValue, star: PValue) -> Result<PValue, PErr> {
    let PValue::MultisetFunction(f) = f else {
        return type_error("not a multiset function");
    };
    let exprs = py_iter(args)?
        .into_iter()
        .map(super::evaluate::implicit_convert)
        .collect::<Result<Vec<_>, _>>()?;
    let kwargs = match star {
        PValue::Value(Value::None) => Vec::new(),
        star => vec![("star".to_string(), star)],
    };
    // Inside another function's body: record the call.
    if exprs.iter().any(MExpr::has_parameter) {
        return Ok(PValue::RawResult(Arc::new(RawResult {
            evaluator: RawEvaluator::Function(f),
            exprs,
            kwargs,
        })));
    }
    let Prepared {
        tree,
        call,
        sources,
    } = prepare_function(&f, &exprs, &kwargs)?;
    let evaluator = MultisetFunction {
        call,
        tupleize: Box::new(tupleize),
    };
    let finals = evaluate_call_tree(
        &evaluator,
        &tree,
        &sources,
        super::evaluate::check_recursion,
    )?;
    let (outcomes, weights): (Vec<_>, Vec<_>) = finals.into_iter().unzip();
    let weights: Vec<Value> = weights.into_iter().map(|w| Value::Int(w.into())).collect();
    new_die(outcomes, &weights)
}
