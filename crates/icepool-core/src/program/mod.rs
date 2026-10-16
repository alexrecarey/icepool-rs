//! The JSON program runner: one program in, one exact result out (wire format: harness/core/encode.py).
//!
//! This is the Python-dynamic layer of the port in miniature: it evaluates the program tree the way
//! harness/core/grammar_api.py `evaluate` does, unpacks Python-style arguments (lists, typed literals, dice) and
//! calls the static core. Node implementations live in `ops.rs`, one per grammar node in harness/grammar/.

mod again;
mod catalogue;
mod catalogue_evaluators;
mod collection;
mod deck;
mod die_methods;
mod evaluate;
mod expression;
mod format;
mod function;
pub mod host;
mod map;
mod map_tools;
mod multi_deal;
mod multiset_function;
mod ops;
mod pool;
mod population;
mod statistics;
mod to_pool;
pub mod wire;

pub use ops::matmul_dice;

use std::sync::Arc;

use serde_json::{json, Value as Json};

use crate::error::Error;
use crate::{Die, Value};

/// What a program expression evaluates to: the Python objects the grammar nodes pass around.
#[derive(Clone, Debug)]
pub enum PValue {
    Value(Value),
    Die(Die<Value>),
    /// A JSON list argument, evaluated element by element.
    List(Vec<PValue>),
    /// `icepool.Reroll`.
    Reroll,
    /// `icepool.Restart`. Outside `map(repeat)` and `again_count` it drops out of a die like `Reroll`.
    Restart,
    /// `icepool.Break(outcome)` returned by a `map` callable; `Value::None` is `Break()`.
    Break(Box<PValue>),
    /// A Python tuple holding something that is not a plain value (a die, `Reroll`, ...). Tuples of plain values
    /// are `Value::Tuple`; build tuples with `PValue::tuple`, which picks the right one.
    Tuple(Vec<PValue>),
    /// A callable or mapping from harness/catalogue.
    Func(Arc<catalogue::Func>),
    /// An `AgainExpression`: `icepool.Again`, or an operator applied to one.
    Again(Arc<again::AgainExpr>),
    /// An `icepool.Pool` (a `KeepGenerator`).
    Pool(crate::Pool<Value>),
    /// An `icepool.Deck`.
    Deck(crate::Deck<Value>),
    /// An `icepool.Deal` (a `KeepGenerator`).
    Deal(crate::Deal<Value>),
    /// An `icepool.MultiDeal` (a `MultisetTupleGenerator`).
    MultiDeal(crate::MultiDeal<Value>),
    /// Any other `MultisetExpression`: a `CompoundKeepGenerator` or an operator over expressions (never a pool,
    /// deal or multi-deal, which have their own variants).
    Expr(expression::MExpr),
    /// `...`, for keep indices.
    Ellipsis,
    /// A `DieWithTruth`: what `die == x` and `die != x` return. A die everywhere (`the_die`, `to_die`), except
    /// that it has a truth value and its type name is `DieWithTruth`. Python computes both lazily; the port
    /// eagerly, which only differs if computing one raises, and neither does.
    DieWithTruth(Die<Value>, bool),
    /// A `DieWithTruth` whose data raises when computed (`==` of vectors of different lengths gives
    /// `VectorWithTruthOnly` outcomes, which cannot be hashed). Python computes the data lazily, so only its truth
    /// value can be used: anything else raises this error (see `eval`).
    DieWithTruthError(Error, bool),
    /// A `@multiset_function` (node `mf_def`).
    MultisetFunction(Arc<multiset_function::FunctionDef>),
    /// A `MultisetFunctionRawResult`: an evaluation inside a `@multiset_function` body.
    RawResult(Arc<multiset_function::RawResult>),
}

impl From<Value> for PValue {
    fn from(v: Value) -> Self {
        PValue::Value(v)
    }
}

impl PValue {
    /// Python's `tuple(items)`.
    pub fn tuple(items: Vec<PValue>) -> PValue {
        if items.iter().all(|x| matches!(x, PValue::Value(_))) {
            let values = items
                .into_iter()
                .map(|x| match x {
                    PValue::Value(v) => v,
                    _ => unreachable!("checked above"),
                })
                .collect();
            PValue::Value(Value::tuple(values))
        } else {
            PValue::Tuple(items)
        }
    }
}

/// Why evaluation stopped: Python raised (a valid answer, matched by any oracle raise), or the port lacks something.
#[derive(Debug)]
pub enum PErr {
    Raise(Error),
    Unimplemented(String),
}

impl From<Error> for PErr {
    fn from(e: Error) -> Self {
        match e {
            Error::Unsupported(what) => PErr::Unimplemented(what),
            e => PErr::Raise(e),
        }
    }
}

pub(crate) fn raise<T>(e: Error) -> Result<T, PErr> {
    Err(e.into())
}

/// Run one program. Never panics on bad input: errors are results too (`{"ok": false, "error": ...}`).
pub fn run(program: &Json) -> Json {
    match eval(program) {
        Ok(v) => wire::result_of(&v),
        Err(PErr::Raise(e)) => {
            json!({"ok": false, "error": e.python_type(), "message": e.message()})
        }
        Err(PErr::Unimplemented(what)) => {
            json!({"ok": false, "error": format!("unimplemented: {what}")})
        }
    }
}

/// `grammar_api.evaluate`: nodes are evaluated with their arguments evaluated first (in order), lists element by
/// element, and anything else is a typed literal.
pub fn eval(x: &Json) -> Result<PValue, PErr> {
    match x {
        Json::Object(m) if m.contains_key("op") => {
            let op = m["op"]
                .as_str()
                .ok_or_else(|| PErr::Unimplemented("non-string op".into()))?;
            if op == "khepri" {
                return khepri(m);
            }
            if op == "mf_def" {
                // Its `body` is lazy: traced when the function is called.
                return multiset_function::mf_def(m);
            }
            let mut values = Vec::new();
            for (k, v) in m.iter().filter(|(k, _)| *k != "op") {
                // A node's `lazy` arguments (grammar_api.Node.lazy) arrive unevaluated: here, the catalogue name.
                let v = match (op, k.as_str(), v) {
                    ("fn", "name", Json::String(name)) => PValue::Value(Value::str(name)),
                    _ => eval(v)?,
                };
                // Raised as soon as it is evaluated (see `call`), before any later argument runs.
                if let PValue::DieWithTruthError(e, _) = &v {
                    if !matches!(op, "truth" | "coll_truth" | "equals_self" | "equals") {
                        return Err(e.clone().into());
                    }
                }
                values.push((k.as_str(), v));
            }
            call(op, values)
        }
        Json::Array(a) => a
            .iter()
            .map(|x| match eval(x)? {
                PValue::DieWithTruthError(e, _) => Err(e.into()),
                x => Ok(x),
            })
            .collect::<Result<_, _>>()
            .map(PValue::List),
        _ => wire::dec(x),
    }
}

/// One node applied to its evaluated arguments: what the facades call (the JS facade's methods are nodes). The
/// lazy nodes (`khepri`, `mf_def`) take raw JSON and only run through [`eval`].
pub fn call(op: &str, args: Vec<(&str, PValue)>) -> Result<PValue, PErr> {
    // Only `bool(x)`, `x.equals(x)` (which checks identity first) and `x.equals(y)` (which checks `y` is a `Die`
    // first) can use a `DieWithTruth` whose data raises.
    if !matches!(op, "truth" | "coll_truth" | "equals_self" | "equals") {
        for (_, v) in &args {
            if let PValue::DieWithTruthError(e, _) = v {
                return Err(e.clone().into());
            }
        }
    }
    ops::call(op, Args(args))
}

/// The `khepri` node: khepri-f2f's whole engine (harness/grammar/khepri.py). Its `input` is lazy: the engine
/// input object itself, never evaluated as a program.
fn khepri(m: &serde_json::Map<String, Json>) -> Result<PValue, PErr> {
    match (m.len(), m.get("input")) {
        (2, Some(input)) => Ok(PValue::Die(crate::khepri::engine_values(
            &crate::khepri::Input::from_json(input)?,
        )?)),
        _ => raise(Error::Type("expected arguments [\"input\"]".into())),
    }
}

/// A node's evaluated keyword arguments.
pub struct Args<'a>(Vec<(&'a str, PValue)>);

impl Args<'_> {
    /// Python calls `run(**args)`: exactly the expected names, or `TypeError`.
    fn exactly<const N: usize>(self, names: [&str; N]) -> Result<[PValue; N], PErr> {
        if self.0.len() != N || self.0.iter().any(|(k, _)| !names.contains(k)) {
            return raise(Error::Type(format!("expected arguments {names:?}")));
        }
        let mut vals = self.0;
        Ok(names.map(|n| {
            let i = vals
                .iter()
                .position(|(k, _)| *k == n)
                .expect("checked above");
            vals.swap_remove(i).1
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exception type and message a program raises. Fixtures only check *that* both sides raised, so the
    /// Python messages and types the port copies are checked here.
    fn raised(program: &str) -> (String, String) {
        let r = run(&serde_json::from_str(program).unwrap());
        assert_eq!(r["ok"], json!(false), "{program} should raise, got {r}");
        let field = |k: &str| r[k].as_str().unwrap_or_default().to_string();
        (field("error"), field("message"))
    }

    #[test]
    fn attribute_errors_name_the_type_and_the_attribute() {
        // `getattr(3, 'values')`: the attribute asked for, not "getattr".
        let (_, msg) = raised(
            r#"{"op":"symbols_method","s":3,"name":{"str":"getattr"},"arg":{"str":"values"}}"#,
        );
        assert_eq!(
            msg,
            "AttributeError: 'int' object has no attribute 'values'"
        );
        let (_, msg) = raised(
            r#"{"op":"symbols_method","s":{"float":"1.5"},"name":{"str":"size"},"arg":null}"#,
        );
        assert_eq!(
            msg,
            "AttributeError: 'float' object has no attribute 'size'"
        );
    }

    #[test]
    fn missing_die_key_is_a_key_error_naming_the_key() {
        let (ty, msg) = raised(
            r#"{"op":"die_method","die":{"op":"d","sides":4},"name":{"str":"getitem"},"arg":0}"#,
        );
        assert_eq!((ty.as_str(), msg.as_str()), ("KeyError", "0"));
    }

    #[test]
    fn again_count_rejects_a_die_with_negative_outcomes_matmul_again() {
        // `__rmatmul__`: `d @ Again` is additive only if `d.min_outcome() >= 0`, so count mode raises the
        // ValueError up front (an additive one would fail later, with TypeError, at `die @ int`).
        let again = |die: &str| {
            format!(
                r#"{{"op":"again_count","outcomes":[1,2,{{"op":"matmul","n":{die},"die":{{"op":"Again"}}}}],"count":1,"depth":null,"end":null}}"#
            )
        };
        let (ty, msg) = raised(&again(r#"{"op":"die","outcomes":[-1,2]}"#));
        assert_eq!(ty, "ValueError");
        assert!(msg.contains("non-additive"), "{msg}");
        assert_eq!(
            raised(&again(r#"{"op":"die","outcomes":[0,2]}"#)).0,
            "TypeError"
        );
        // `min_outcome()` of an empty die raises first.
        assert_eq!(
            raised(&again(r#"{"op":"die","outcomes":[]}"#)).0,
            "IndexError"
        );
    }

    #[test]
    fn mean_time_to_sum_of_a_huge_target_is_unimplemented() {
        // Python loops over every target up to this one (for hours); a huge int is not a TypeError.
        let program = r#"{"op":"mean_time_to_sum","die":{"op":"d","sides":3},"targets":[100000000000000000000]}"#;
        let r = run(&serde_json::from_str(program).unwrap());
        assert_eq!(r["ok"], json!(false));
        assert_eq!(r["error"], json!("unimplemented: mean_time_to_sum of a huge target"));
    }

    #[test]
    fn again_hash_keys_with_a_nan_on_both_sides_are_unimplemented() {
        // Tuple `==` tries identity first, and the port cannot tell one NaN object from another; with a NaN on
        // one side only, `==` decides (fixtures/m202610160934/mutant-killers).
        let same_both_sides = |x: &str| {
            let a = format!(r#"{{"op":"add","a":{{"op":"Again"}},"b":{x}}}"#);
            let program = format!(r#"{{"op":"again_truth","x":{{"op":"eq","a":{a},"b":{a}}}}}"#);
            let (error, _) = raised(&program);
            assert!(error.starts_with("unimplemented: NaN"), "{x}: {error}");
        };
        same_both_sides(r#"{"float":"nan"}"#);
        same_both_sides(r#"{"op":"complex","re":{"float":"1.0"},"im":{"float":"nan"}}"#);
        same_both_sides(r#"{"op":"complex","re":{"float":"nan"},"im":{"float":"1.0"}}"#);
        same_both_sides(r#"{"tuple":[1,{"float":"nan"}]}"#);
        same_both_sides(r#"{"op":"die","outcomes":[1,{"float":"nan"}]}"#);
        same_both_sides(r#"[1,{"float":"nan"}]"#);
    }

    #[test]
    fn type_error_messages_of_die_sequence_and_multiset_counts() {
        let (ty, msg) = raised(
            r#"{"op":"die_sequence","die":{"op":"d","sides":2},"rolls":{"float":"1.5"}}"#,
        );
        assert_eq!(ty, "TypeError");
        assert_eq!(msg, "'float' object cannot be interpreted as an integer");
        // `expr * die` is `Die.__rmul__`, which makes `Die([expr])` first.
        let (ty, msg) = raised(
            r#"{"op":"mx_str","expr":{"op":"mx_counts","expr":{"op":"die_pool","die":{"op":"d","sides":2},"rolls":2},"o":{"str":"mul"},"comparison":null,"n":{"op":"d","sides":2}}}"#,
        );
        assert_eq!(ty, "TypeError");
        assert_eq!(msg, "Items do not appear to be sortable.");
    }
}
