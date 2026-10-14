//! The JSON program runner: one program in, one exact result out (wire format: harness/core/encode.py).
//!
//! This is the Python-dynamic layer of the port in miniature: it evaluates the program tree the way
//! harness/core/grammar_api.py `evaluate` does, unpacks Python-style arguments (lists, typed literals, dice) and
//! calls the static core. Node implementations live in `ops.rs`, one per grammar node in harness/grammar/.

mod ops;
mod wire;

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
}

/// Why evaluation stopped: Python raised (a valid answer, matched by any oracle raise), or the port lacks something.
#[derive(Debug)]
pub enum PErr {
    Raise(Error),
    Unimplemented(String),
}

impl From<Error> for PErr {
    fn from(e: Error) -> Self {
        PErr::Raise(e)
    }
}

pub(crate) fn raise<T>(e: Error) -> Result<T, PErr> {
    Err(PErr::Raise(e))
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
            let mut values = Vec::new();
            for (k, v) in m.iter().filter(|(k, _)| *k != "op") {
                values.push((k.as_str(), eval(v)?));
            }
            ops::call(op, Args(values))
        }
        Json::Array(a) => a
            .iter()
            .map(eval)
            .collect::<Result<_, _>>()
            .map(PValue::List),
        _ => wire::dec(x),
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
