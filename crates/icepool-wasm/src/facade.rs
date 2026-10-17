//! The bridge under the JS facade (js/icepool.js): program values held in a handle table, nodes called with
//! JSON arguments, and JS callables / evaluators called back through a global the JS side installs.
//!
//! JS sees every value that has a JS counterpart as JSON ("JS-JSON", below) and everything else (dice, pools,
//! decks, expressions, ...) as a handle: an index into this module's table, which the JS object owns and releases
//! when it is garbage-collected. Values cross as JSON text because that is the one structured type wasm-bindgen
//! passes without js-sys, and because the exact integers survive it (JS's `JSON.parse` would round big ones, so
//! those travel as strings).
//!
//! JS-JSON, both ways:
//!
//! | Python | JS-JSON |
//! |---|---|
//! | `None`, `bool`, `str` | `null`, `true` / `false`, `"..."` |
//! | `int` | a JSON integer, or `{"$int": "<digits>"}` beyond 2**53 (JS may also send any JSON integer) |
//! | `float` | `{"float": "<repr>"}` (JS has no int/float distinction of its own) |
//! | `Fraction` | `{"frac": [n, d]}` |
//! | `complex` | `{"complex": ["<repr of re>", "<repr of im>"]}` |
//! | `Vector` | `{"vector": [...] \| null, "truth": x}` (`truth` only if it has one; `null` data: `VectorWithTruthOnly`) |
//! | `Symbols` | `{"$symbols": [[char, count], ...], "str": "<str>"}` in dict order; JS may send `{"symbols": "<str>"}` |
//! | `tuple` | a JSON array |
//! | `list` | `{"$list": [...]}` |
//! | `Reroll`, `Restart`, `...` | `{"$special": "Reroll" \| "Restart" \| "Ellipsis"}` |
//! | `Break(x)` | `{"$break": x}` |
//! | a JS function | `{"$fn": <id>, "n": <its length>}` (JS keeps the function, see `host_call`) |
//! | a JS `Map` (JS -> Rust only) | `{"$map": [[key, value], ...]}`: a Python dict (`program::host::mapping`) |
//! | an evaluator state that is a JS object | `{"$json": "<canonical JSON>"}`: hashed and compared by that key |
//! | anything else | `{"$h": <handle>, "kind": "die" \| "die_truth" \| "pool" \| ...}` (`"truth"` for `die_truth`) |
//!
//! Everything returned to JS is `{"ok": <JS-JSON>}` or an error `{"error": <Python type>, "message": ...}`;
//! `{"host": true}` means the JS callback threw, and the JS side rethrows its own exception.

use std::cell::RefCell;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use icepool_core::evaluator::Final;
use icepool_core::evaluator::Order;
use icepool_core::program::host::{HostEvaluator, HostFunction};
use icepool_core::program::{self, PErr, PValue};
use icepool_core::value::recursion::{level, Level};
use icepool_core::host::{HostError, HostObject, HostRef};
use icepool_core::value::{float_repr, Symbols, Vector};
use icepool_core::Value;
use icepool_core::{Error, Result as CoreResult};
use serde_json::{json, Map, Value as Json};
use wasm_bindgen::prelude::*;

// The callbacks into JS. js/icepool.js installs `globalThis.__icepool_wasm_host` before the first call; nothing
// here calls it unless a JS function or evaluator was passed in.
#[wasm_bindgen]
extern "C" {
    /// `f(*args, **kwargs)` for the JS function registered as `id`: JS-JSON in, a result envelope out.
    #[wasm_bindgen(js_namespace = __icepool_wasm_host, js_name = call)]
    fn host_call(id: u32, args: &str, kwargs: &str) -> String;
    /// Forgets the JS function or evaluator registered as `id`: the core dropped its last reference.
    #[wasm_bindgen(js_namespace = __icepool_wasm_host, js_name = release)]
    fn host_release(id: u32);
    /// A method of the JS evaluator registered as `id`; `kwargs` are `evaluate()`'s keyword arguments, which
    /// Python passes on to `initial_state` and `final_outcome`.
    #[wasm_bindgen(js_namespace = __icepool_wasm_host, js_name = evaluator)]
    fn host_evaluator(id: u32, method: &str, args: &str, kwargs: &str) -> String;
}

// --- The handle table ---------------------------------------------------------------------------------------------

thread_local! {
    /// Slots for the values JS holds; freed slots are reused. WASM here is single-threaded, so one table.
    static HANDLES: RefCell<(Vec<Option<PValue>>, Vec<u32>)> = const { RefCell::new((Vec::new(), Vec::new())) };
}

fn put(v: PValue) -> u32 {
    HANDLES.with(|h| {
        let (slots, free) = &mut *h.borrow_mut();
        match free.pop() {
            Some(i) => {
                slots[i as usize] = Some(v);
                i
            }
            None => {
                slots.push(Some(v));
                (slots.len() - 1) as u32
            }
        }
    })
}

/// A clone of the value: the table is never borrowed while the core runs, since a JS callback may come back in.
fn get(id: u32) -> Result<PValue, PErr> {
    HANDLES
        .with(|h| h.borrow().0.get(id as usize).cloned().flatten())
        .ok_or_else(|| {
            PErr::Raise(Error::InvalidArgument(format!(
                "released or unknown handle {id}"
            )))
        })
}

/// Frees a handle (the JS object holding it was garbage-collected).
#[wasm_bindgen]
pub fn release(id: u32) {
    // The value is dropped after the table is released: dropping it may call back into JS (`host_release`).
    let value = HANDLES.with(|h| {
        let (slots, free) = &mut *h.borrow_mut();
        let value = slots.get_mut(id as usize).and_then(Option::take);
        if value.is_some() {
            free.push(id);
        }
        value
    });
    drop(value);
}

/// After a trap (a Rust panic: WASM has no unwinding), once JS has left every call into the instance: resets the
/// core's per-call state, which the trap skipped the destructors of, and returns the panic's message (empty if
/// none was recorded). The handle table is never borrowed across code that can panic, so it survives as it is.
#[wasm_bindgen]
pub fn recover() -> String {
    program::reset_after_trap();
    crate::take_panic().unwrap_or_default()
}

/// How many handles are live (for the JS tests' leak check).
#[wasm_bindgen]
pub fn live_handles() -> usize {
    HANDLES.with(|h| h.borrow().0.iter().filter(|s| s.is_some()).count())
}

// --- Exports ------------------------------------------------------------------------------------------------------

/// Calls node `op` (harness/grammar/: the JS methods are nodes) with a JSON object of JS-JSON arguments.
/// The lazy nodes (`khepri`, `mf_def`) take their arguments as program JSON instead, and run as a program.
#[wasm_bindgen]
pub fn call(op: &str, args: &str) -> String {
    let result = (|| {
        let Json::Object(args) = parse(args)? else {
            return Err(bad("node arguments that are not an object"));
        };
        if matches!(op, "khepri" | "mf_def") {
            let mut node = args;
            node.insert("op".into(), Json::String(op.into()));
            return program::eval(&Json::Object(node));
        }
        let mut values = Vec::with_capacity(args.len());
        for (k, v) in &args {
            values.push((k.as_str(), from_js(v)?));
        }
        program::call(op, values)
    })();
    envelope(result.and_then(|v| to_js(&v)))
}

/// `evaluator.evaluate(*args, **kwargs)` for the JS evaluator registered as `id` (args: a JSON array of JS-JSON;
/// kwargs: a JSON object, handed back to JS as it is).
#[wasm_bindgen]
pub fn evaluate(id: u32, args: &str, kwargs: &str) -> String {
    // Made first: its drop releases `id` on the JS side, whatever happens next.
    let evaluator: Arc<dyn HostEvaluator> = Arc::new(JsEvaluator {
        id,
        kwargs: kwargs.to_string(),
    });
    let result = (|| {
        let args = from_js(&json!({ "$list": parse(args)? }))?;
        program::host::evaluate(&evaluator, args)
    })();
    envelope(result.and_then(|v| to_js(&v)))
}

/// `icepool.multiset_function(f)` for a JS function: `{"f": {"$fn": id, "n": length}, "params": n, "var": bool,
/// "kwonly": [names]}` (the signature as Python would read it: required positional parameters, a `*args`, the
/// keyword-only names). Calling the result traces `f` with parameter expressions (see `program::host`).
#[wasm_bindgen]
pub fn multiset_function(args: &str) -> String {
    let result = (|| {
        let args = parse(args)?;
        let id = args["f"]["$fn"]
            .as_u64()
            .ok_or_else(|| bad("function id"))? as u32;
        // Registered on the JS side: from here on the core owns it (and releases it when dropped).
        let f = Arc::new(JsFunction {
            id,
            length: args["f"]["n"].as_u64().unwrap_or(0) as usize,
        });
        let params = args["params"]
            .as_u64()
            .ok_or_else(|| bad("a parameter count"))? as usize;
        let var = args["var"].as_bool().ok_or_else(|| bad("var"))?;
        let kwonly = match &args["kwonly"] {
            Json::Array(names) => names
                .iter()
                .map(|n| {
                    n.as_str()
                        .map(String::from)
                        .ok_or_else(|| bad("a keyword name"))
                })
                .collect::<Result<Vec<_>, _>>()?,
            _ => return Err(bad("kwonly")),
        };
        Ok(program::host::multiset_function(f, params, var, kwonly))
    })();
    envelope(result.and_then(|v| to_js(&v)))
}

/// `icepool.from_rv(rv, outcomes, denominator, **kwargs)` for a JS random variable: `{"cdf": {"$fn": id, "n":
/// length}, "pdf": bool, "outcomes": x, "denominator": x, "kwargs": {name: x}}`, where `cdf` is the rv's bound
/// `cdf` method and `pdf` says whether it has one (Python's `hasattr(rv, 'pdf')`: a continuous distribution).
#[wasm_bindgen]
pub fn from_rv(args: &str) -> String {
    let result = (|| {
        let args = parse(args)?;
        let id = args["cdf"]["$fn"]
            .as_u64()
            .ok_or_else(|| bad("function id"))? as u32;
        let cdf = Arc::new(JsFunction {
            id,
            length: args["cdf"]["n"].as_u64().unwrap_or(0) as usize,
        });
        let pdf = args["pdf"].as_bool().ok_or_else(|| bad("pdf"))?;
        let Json::Object(kw) = &args["kwargs"] else {
            return Err(bad("kwargs"));
        };
        let kwargs = kw
            .iter()
            .map(|(k, v)| Ok((k.clone(), from_js(v)?)))
            .collect::<Result<Vec<_>, PErr>>()?;
        program::host::from_rv(
            cdf,
            pdf,
            from_js(&args["outcomes"])?,
            from_js(&args["denominator"])?,
            kwargs,
        )
    })();
    envelope(result.and_then(|v| to_js(&v)))
}

/// A literal of the harness's wire format (encode.py `dec`) as JS-JSON: lets the fixture-driven JS tests read
/// program literals exactly as `icepool-run` does, then pass them back through the JS API.
#[wasm_bindgen]
pub fn from_wire(literal: &str) -> String {
    envelope(
        parse(literal)
            .and_then(|j| program::wire::dec(&j))
            .and_then(|v| to_js(&v)),
    )
}

/// The harness's wire encoding of a handle's value (`encode.result_of`: `{"ok": true, "die" | "value": ...}`),
/// for the fixture-driven JS tests, which compare it with the oracle's.
#[wasm_bindgen]
pub fn wire_result(id: u32) -> String {
    match get(id) {
        Ok(v) => program::wire::result_of(&v).to_string(),
        Err(e) => envelope(Err(e)),
    }
}

fn parse(text: &str) -> Result<Json, PErr> {
    serde_json::from_str(text)
        .map_err(|e| PErr::Raise(Error::InvalidArgument(format!("bad JSON from JS: {e}"))))
}

fn envelope(result: Result<Json, PErr>) -> String {
    match result {
        Ok(v) => json!({ "ok": v }),
        Err(PErr::Raise(Error::Callback(_))) => json!({"host": true}),
        Err(PErr::Raise(e)) => json!({"error": program::wire::python_exception(&e), "message": e.to_string()}),
        Err(PErr::Unimplemented(what)) => {
            json!({"error": "NotImplementedError", "message": format!("not ported yet: {what}"), "unimplemented": true})
        }
    }
    .to_string()
}

// --- Rust -> JS ---------------------------------------------------------------------------------------------------

const MAX_SAFE: u64 = (1 << 53) - 1;

fn int_js(digits: String) -> Json {
    match digits.parse::<i64>() {
        Ok(n) if n.unsigned_abs() <= MAX_SAFE => json!(n),
        _ => json!({ "$int": digits }),
    }
}

/// A level of recursion on nested values (`icepool_core::value::recursion`): `RecursionError` rather than
/// overflowing the WASM stack.
fn nest() -> Result<Level, PErr> {
    level().map_err(PErr::Raise)
}

/// A value as JS-JSON. Fraction parts are always strings (JS makes them BigInts). Tuples and vectors nest, a level
/// each.
fn value_js(v: &Value) -> Result<Json, PErr> {
    let _level = match v {
        Value::Tuple(_) | Value::Vector(_) => Some(nest()?),
        _ => None,
    };
    Ok(match v {
        Value::None => Json::Null,
        Value::Bool(b) => Json::Bool(*b),
        Value::Int(i) => int_js(i.to_string()),
        Value::Float(f) => json!({ "float": float_repr(*f) }),
        Value::Fraction(r) => {
            json!({ "frac": [r.numerator().to_string(), r.denominator().to_string()] })
        }
        Value::Str(s) => Json::String(s.to_string()),
        Value::Tuple(t) => Json::Array(t.iter().map(value_js).collect::<Result<_, _>>()?),
        // The truth value of a comparison's vector, and a `VectorWithTruthOnly` (no data), survive the trip.
        Value::Vector(x) => {
            let data = if x.is_truth_only() {
                Json::Null
            } else {
                Json::Array(x.data()?.iter().map(value_js).collect::<Result<_, _>>()?)
            };
            let mut j = json!({ "vector": data });
            if let Some(truth) = x.truth() {
                j["truth"] = value_js(truth)?;
            }
            j
        }
        // The dict's items in its own order, zero counts included (`str()` shows neither), and `str()` itself.
        Value::Symbols(x) => json!({
            "$symbols": x
                .items()
                .iter()
                .map(|(c, n)| Ok(json!([c.to_string(), value_js(n)?])))
                .collect::<Result<Vec<_>, PErr>>()?,
            "str": x.py_str().ok(),
        }),
        Value::Complex(re, im) => json!({ "complex": [float_repr(*re), float_repr(*im)] }),
        Value::Host(h) => match h.as_any().downcast_ref::<JsonKey>() {
            Some(k) => json!({ "$json": &*k.0 }),
            None => json!({ "repr": h.repr() }),
        },
    })
}

fn handle(v: &PValue, kind: &str) -> Json {
    json!({"$h": put(v.clone()), "kind": kind})
}

/// A program value as JS-JSON: values inline, the rest as new handles. Lists, tuples and breaks nest (chained
/// calls on handles nest them as deep as JS likes), a level each.
fn to_js(v: &PValue) -> Result<Json, PErr> {
    let _level = match v {
        PValue::List(_) | PValue::Tuple(_) | PValue::Break(_) => Some(nest()?),
        // A handle JS may build on, one level deeper each call: never deeper than the core recurses.
        PValue::Again(_) | PValue::Expr(_) | PValue::RawResult(_) => {
            program::check_nesting(v)?;
            None
        }
        _ => None,
    };
    Ok(match v {
        PValue::Value(x) => value_js(x)?,
        PValue::List(xs) => {
            json!({ "$list": xs.iter().map(to_js).collect::<Result<Vec<_>, _>>()? })
        }
        PValue::Tuple(xs) => Json::Array(xs.iter().map(to_js).collect::<Result<_, _>>()?),
        PValue::Reroll => json!({"$special": "Reroll"}),
        PValue::Restart => json!({"$special": "Restart"}),
        PValue::Ellipsis => json!({"$special": "Ellipsis"}),
        PValue::Break(x) => json!({ "$break": to_js(x)? }),
        PValue::Die(_) => handle(v, "die"),
        PValue::DieWithTruth(_, truth) | PValue::DieWithTruthError(_, truth) => {
            let mut h = handle(v, "die_truth");
            h["truth"] = Json::Bool(*truth);
            h
        }
        PValue::Func(_) => handle(v, "fn"),
        PValue::Again(_) => handle(v, "again"),
        PValue::Pool(_) => handle(v, "pool"),
        PValue::Deck(_) => handle(v, "deck"),
        PValue::Deal(_) => handle(v, "deal"),
        PValue::MultiDeal(_) => handle(v, "multi_deal"),
        // A tuple expression (a tuple parameter, a slice of hands) indexes into hands, as a MultiDeal does.
        PValue::Expr(_) if program::host::is_tuple_expression(v) => handle(v, "tuple_expr"),
        PValue::Expr(_) => handle(v, "expr"),
        PValue::MultisetFunction(_) => handle(v, "mfunc"),
        PValue::RawResult(_) => handle(v, "raw"),
    })
}

// --- JS -> Rust ---------------------------------------------------------------------------------------------------

fn bad(what: &str) -> PErr {
    PErr::Raise(Error::WrongType(format!("not a JS-JSON value: {what}")))
}

/// JS-JSON as a program value.
fn from_js(j: &Json) -> Result<PValue, PErr> {
    Ok(match j {
        Json::Null => PValue::Value(Value::None),
        Json::Bool(b) => PValue::Value(Value::Bool(*b)),
        Json::String(s) => PValue::Value(Value::str(s)),
        Json::Number(_) => program::wire::dec(j)?,
        Json::Array(items) => PValue::tuple(items.iter().map(from_js).collect::<Result<_, _>>()?),
        // The two-key forms first (a vector's truth value, a symbols' str, a function's length).
        Json::Object(m) if m.contains_key("vector") => {
            let values = |items: &[Json]| -> Result<Vec<Value>, PErr> {
                items
                    .iter()
                    .map(|x| value_of(from_js(x)?).map_err(PErr::from))
                    .collect()
            };
            let truth = m
                .get("truth")
                .map(|t| value_of(from_js(t)?).map_err(PErr::from))
                .transpose()?;
            PValue::Value(Value::Vector(match (&m["vector"], truth) {
                (Json::Array(items), truth) => Vector::from_parts(values(items)?, truth),
                (Json::Null, Some(truth)) => Vector::truth_only(truth),
                _ => return Err(bad("vector")),
            }))
        }
        Json::Object(m) if m.contains_key("$symbols") => {
            let Some(Json::Array(items)) = m.get("$symbols") else {
                return Err(bad("$symbols"));
            };
            let mut data = Vec::with_capacity(items.len());
            for item in items {
                let (Some(Json::String(c)), Some(n)) = (item.get(0), item.get(1)) else {
                    return Err(bad("$symbols item"));
                };
                let mut chars = c.chars();
                let (Some(c), None) = (chars.next(), chars.next()) else {
                    return Err(bad("$symbols key"));
                };
                data.push((c, value_of(from_js(n)?)?));
            }
            PValue::Value(Value::Symbols(Symbols::raw(data)))
        }
        // A JS Map: Python's `dict(items)` (keys equal by Python's `==`).
        Json::Object(m) if m.contains_key("$map") => {
            let Some(Json::Array(items)) = m.get("$map") else {
                return Err(bad("$map"));
            };
            program::host::mapping(PValue::List(
                items.iter().map(from_js).collect::<Result<_, _>>()?,
            ))?
        }
        Json::Object(m) if m.contains_key("$fn") => {
            let id = m["$fn"].as_u64().ok_or_else(|| bad("function id"))? as u32;
            let length = m.get("n").and_then(Json::as_u64).unwrap_or(0) as usize;
            program::host::function(Arc::new(JsFunction { id, length }))
        }
        Json::Object(m) if m.len() == 1 => {
            let (k, x) = m.iter().next().expect("one entry");
            match (k.as_str(), x) {
                ("$h", Json::Number(n)) => get(n.as_u64().ok_or_else(|| bad("handle"))? as u32)?,
                ("$int", Json::String(s)) => {
                    PValue::Value(Value::Int(s.parse().map_err(|_| bad("$int digits"))?))
                }
                ("$list", Json::Array(items)) => PValue::List(
                    items
                        .iter()
                        .map(|x| match from_js(x)? {
                            // As `program::eval` evaluates a list: only `bool()` may use this die.
                            PValue::DieWithTruthError(e, _) => Err(PErr::from(e)),
                            x => Ok(x),
                        })
                        .collect::<Result<_, _>>()?,
                ),
                ("$break", x) => PValue::Break(Box::new(from_js(x)?)),
                ("$special", Json::String(s)) => match s.as_str() {
                    "Reroll" => PValue::Reroll,
                    "Restart" => PValue::Restart,
                    "Ellipsis" => PValue::Ellipsis,
                    _ => return Err(bad(s)),
                },
                ("$json", Json::String(s)) => PValue::Value(JsonKey::value(s)),
                ("frac", Json::Array(parts)) => {
                    // Parts may come as strings (the way they went out) or JSON integers.
                    let parts = parts
                        .iter()
                        .map(|p| match p {
                            Json::String(s) => {
                                serde_json::from_str::<Json>(s).map_err(|_| bad("frac"))
                            }
                            p => Ok(p.clone()),
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    program::wire::dec(&json!({ "frac": parts }))?
                }
                // Python's `complex`, as it went out: the reprs of its two floats.
                ("complex", Json::Array(parts)) => match parts.as_slice() {
                    [re, im] => {
                        let part = |x: &Json| match program::wire::dec(&json!({ "float": x }))? {
                            PValue::Value(Value::Float(f)) => Ok(f),
                            _ => Err(bad("complex")),
                        };
                        PValue::Value(Value::Complex(part(re)?, part(im)?))
                    }
                    _ => return Err(bad("complex")),
                },
                // A float's repr; a Symbols from a str (`Symbols("aab")`), an iterable or a mapping.
                ("float" | "symbols", _) => program::wire::dec(j)?,
                _ => return Err(bad(k)),
            }
        }
        Json::Object(_) => return Err(bad("an object")),
    })
}

fn value_of(p: PValue) -> CoreResult<Value> {
    match p {
        PValue::Value(v) => Ok(v),
        _ => Err(Error::WrongType(
            "expected a value (an outcome or state), got a die or another object".into(),
        )),
    }
}

// --- Callbacks ----------------------------------------------------------------------------------------------------

/// The token of a JS exception: the JS side keeps the exception itself and rethrows it when the call returns.
fn host_error() -> Error {
    Error::Callback(HostError(Arc::new("a JS exception")))
}

/// A JS callback's result envelope: `{"ok": x}`, `{"throw": true}` (JS keeps the exception) or `{"error":
/// "UnsupportedOrder"}` (an evaluator's `throw new UnsupportedOrder()`, which the driver catches).
fn callback_result(text: &str) -> Result<PValue, PErr> {
    let r = parse(text)?;
    if let Some(x) = r.get("ok") {
        return from_js(x);
    }
    if r.get("error").and_then(Json::as_str) == Some("UnsupportedOrder") {
        return Err(PErr::Raise(Error::UnsupportedOrder(String::new())));
    }
    Err(PErr::Raise(host_error()))
}

fn js_args(args: &[PValue]) -> Result<String, PErr> {
    Ok(Json::Array(args.iter().map(to_js).collect::<Result<_, _>>()?).to_string())
}

/// The core's errors from inside a callback as `CoreResult` (the evaluator traits use it).
fn core_error(e: PErr) -> Error {
    match e {
        PErr::Raise(e) => e,
        PErr::Unimplemented(what) => Error::Unported(what),
    }
}

/// A JS function: `{"$fn": id, "n": f.length}`.
#[derive(Debug)]
struct JsFunction {
    id: u32,
    /// `f.length`: the parameters before the first one with a default (or the rest parameter).
    length: usize,
}

/// Each `$fn` the core decodes registered the function once on the JS side; the core holds it (behind an `Arc`)
/// as long as any value refers to it, e.g. a multiset expression with a `map_counts` function.
impl Drop for JsFunction {
    fn drop(&mut self) {
        host_release(self.id);
    }
}

impl HostFunction for JsFunction {
    /// JS functions accept any number of arguments; `length` is what they require, for `map`'s `star` inference.
    fn positional(&self) -> (usize, Option<usize>) {
        (self.length, None)
    }

    fn call(&self, args: Vec<PValue>, kwargs: Vec<(String, PValue)>) -> Result<PValue, PErr> {
        let mut kw = Map::new();
        for (k, v) in &kwargs {
            kw.insert(k.clone(), to_js(v)?);
        }
        callback_result(&host_call(
            self.id,
            &js_args(&args)?,
            &Json::Object(kw).to_string(),
        ))
    }
}

/// A JS `MultisetEvaluator` (registered as `id` on the JS side). States are values: JS turns its states into
/// JS-JSON, objects into `$json` keys, so the driver can hash and merge them.
#[derive(Debug)]
struct JsEvaluator {
    id: u32,
    kwargs: String,
}

/// The JS side keeps the evaluator registered as long as the core may call it: a `@multiset_function` body
/// records the evaluation, which runs when the function is called.
impl Drop for JsEvaluator {
    fn drop(&mut self) {
        host_release(self.id);
    }
}

impl JsEvaluator {
    fn method(&self, name: &str, args: Vec<PValue>) -> CoreResult<PValue> {
        let args = js_args(&args).map_err(core_error)?;
        callback_result(&host_evaluator(self.id, name, &args, &self.kwargs)).map_err(core_error)
    }
}

fn values(xs: &[Value]) -> PValue {
    PValue::Value(Value::tuple(xs.to_vec()))
}

fn order_value(order: Order) -> PValue {
    PValue::Value(Value::from(order.value()))
}

impl HostEvaluator for JsEvaluator {
    fn initial_state(
        &self,
        order: Order,
        outcomes: &[Value],
        sizes: &[Value],
    ) -> CoreResult<Value> {
        let mut args = vec![order_value(order), values(outcomes)];
        args.extend(sizes.iter().cloned().map(PValue::Value));
        value_of(self.method("initialState", args)?)
    }

    fn next_state(
        &self,
        state: &Value,
        order: Order,
        outcome: &Value,
        counts: &[Value],
    ) -> CoreResult<Option<Value>> {
        let mut args = vec![
            PValue::Value(state.clone()),
            order_value(order),
            PValue::Value(outcome.clone()),
        ];
        args.extend(counts.iter().cloned().map(PValue::Value));
        match self.method("nextState", args)? {
            PValue::Reroll => Ok(None),
            p => value_of(p).map(Some),
        }
    }

    fn extra_outcomes(&self, outcomes: &[Value]) -> CoreResult<Vec<Value>> {
        match self.method("extraOutcomes", vec![values(outcomes)])? {
            PValue::Value(Value::Tuple(xs)) => Ok(xs.to_vec()),
            PValue::List(xs) => xs.into_iter().map(value_of).collect(),
            _ => Err(Error::WrongType(
                "extraOutcomes must return an array of outcomes".into(),
            )),
        }
    }

    fn final_outcome(
        &self,
        final_state: &Value,
        order: Order,
        outcomes: &[Value],
        sizes: &[Value],
    ) -> CoreResult<Final<PValue>> {
        let mut args = vec![
            PValue::Value(final_state.clone()),
            order_value(order),
            values(outcomes),
        ];
        args.extend(sizes.iter().cloned().map(PValue::Value));
        Ok(match self.method("finalOutcome", args)? {
            PValue::Reroll => Final::Reroll,
            PValue::Value(Value::None) => Final::None,
            p => Final::Outcome(p),
        })
    }
}

/// An evaluator state that is a JS object, keyed by its canonical JSON (sorted keys, made by the JS side): equal
/// keys are equal states. It has no order or arithmetic, like a Python dict, and its truth value is a dict's.
#[derive(Debug)]
struct JsonKey(Arc<str>);

impl JsonKey {
    fn value(text: &str) -> Value {
        Value::Host(HostRef::new(JsonKey(text.into())))
    }

    fn of(v: &Value) -> Option<&JsonKey> {
        match v {
            Value::Host(h) => h.as_any().downcast_ref::<JsonKey>(),
            _ => None,
        }
    }
}

impl HostObject for JsonKey {
    fn py_hash(&self) -> i64 {
        // Any fixed hash of the key: equal keys hash alike, and no native value equals a key.
        let mut h = std::hash::DefaultHasher::new();
        self.0.hash(&mut h);
        h.finish() as i64
    }
    fn eq(&self, a: &Value, b: &Value) -> bool {
        matches!((JsonKey::of(a), JsonKey::of(b)), (Some(x), Some(y)) if x.0 == y.0)
    }
    fn compare(&self, _: icepool_core::value::BinOp, _: &Value, _: &Value) -> CoreResult<bool> {
        Err(Error::WrongType("JS object states have no order".into()))
    }
    fn binary(&self, _: icepool_core::value::BinOp, _: &Value, _: &Value) -> CoreResult<Value> {
        Err(Error::WrongType(
            "JS object states have no operators".into(),
        ))
    }
    fn unary(&self, _: icepool_core::value::UnOp) -> CoreResult<Value> {
        Err(Error::WrongType(
            "JS object states have no operators".into(),
        ))
    }
    fn truthy(&self) -> CoreResult<bool> {
        Ok(&*self.0 != "{}")
    }
    fn repr(&self) -> String {
        self.0.to_string()
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(j: Json) -> Json {
        to_js(&from_js(&j).unwrap()).unwrap()
    }

    #[test]
    fn values_round_trip() {
        for j in [
            json!(null),
            json!(true),
            json!("abc"),
            json!(7),
            json!({"$int": "123456789012345678901234567890"}),
            json!({"float": "1.0"}),
            json!({"float": "-inf"}),
            json!({"frac": ["1", "3"]}),
            json!([1, [2, "x"], {"float": "0.5"}]),
            json!({"vector": [1, 2]}),
            // A comparison's vector keeps its truth value; a VectorWithTruthOnly has no data.
            json!({"vector": [true, false], "truth": false}),
            json!({"vector": null, "truth": true}),
            // Dict order and zero counts survive (str() shows neither).
            json!({"$symbols": [["b", 1], ["a", 0]], "str": "b"}),
            json!({"$list": [1, {"$special": "Reroll"}]}),
            json!({"$break": null}),
            json!({"$json": "{\"a\":1}"}),
        ] {
            assert_eq!(roundtrip(j.clone()), j);
        }
        // Safe integers stay JSON numbers; a big one written plainly by JS comes back as `$int`.
        assert_eq!(
            roundtrip(json!(9007199254740991_i64)),
            json!(9007199254740991_i64)
        );
        assert_eq!(
            roundtrip(serde_json::from_str("9007199254740992").unwrap()),
            json!({"$int": "9007199254740992"})
        );
    }

    #[test]
    fn handles_hold_dice_and_are_released() {
        let before = live_handles();
        let r: Json = serde_json::from_str(&call("d", r#"{"sides": 6}"#)).unwrap();
        assert_eq!(r["ok"]["kind"], "die");
        let id = r["ok"]["$h"].as_u64().unwrap() as u32;
        let sum: Json =
            serde_json::from_str(&call("add", &format!(r#"{{"a": {{"$h": {id}}}, "b": 1}}"#)))
                .unwrap();
        let sum_id = sum["ok"]["$h"].as_u64().unwrap() as u32;
        let wire: Json = serde_json::from_str(&wire_result(sum_id)).unwrap();
        assert_eq!(wire["die"][0], json!([2, 1]));
        release(id);
        release(sum_id);
        assert_eq!(live_handles(), before);
        let gone: Json =
            serde_json::from_str(&call("neg", &format!(r#"{{"a": {{"$h": {id}}}}}"#))).unwrap();
        assert_eq!(gone["error"], "ValueError");
    }

    #[test]
    fn errors_are_python_types() {
        let r: Json = serde_json::from_str(&call("d", r#"{"sides": 0}"#)).unwrap();
        assert_eq!(r["error"], "ValueError");
        let r: Json = serde_json::from_str(&call("d", r#"{"sides": {"oops": 1}}"#)).unwrap();
        assert_eq!(r["error"], "TypeError");
    }

    #[test]
    fn json_keys_are_equal_by_text() {
        let (a, b, c) = (
            JsonKey::value("{\"a\":1}"),
            JsonKey::value("{\"a\":1}"),
            JsonKey::value("{}"),
        );
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert!(!c.truthy().unwrap());
    }
}
