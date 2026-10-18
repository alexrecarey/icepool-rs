//! `Value` as a state of the repeated maps (`map_tools::MapState`): transition outcomes are Python's tuples
//! `(transition_type, state)` and `(transition_type, state, time)`, with the `TransitionType` as its int.

use dashu_int::IBig;

use crate::error::{Error, Result};
use crate::map_tools::{MapState, Transition};
use crate::Value;

/// `TransitionType` is an `IntEnum`, so its members sort and hash as these ints.
const DEFAULT: i64 = 0;
const BREAK: i64 = 1;
const RESTART: i64 = 2;

fn int(i: i64) -> Value {
    Value::from(IBig::from(i))
}

/// `(transition_type, state)` as a list of tuple items; a `Restart` has the state `None`.
fn items(transition: Transition<Value>) -> [Value; 2] {
    match transition {
        Transition::Default(s) => [int(DEFAULT), s],
        Transition::Break(s) => [int(BREAK), s],
        Transition::Restart => [int(RESTART), Value::None],
    }
}

/// The transition of the tuple items `(transition_type, state, ...)`.
fn read(items: &[Value]) -> Transition<&Value> {
    match &items[0] {
        t if *t == int(DEFAULT) => Transition::Default(&items[1]),
        t if *t == int(BREAK) => Transition::Break(&items[1]),
        _ => Transition::Restart,
    }
}

fn tuple_items(outcome: &Value) -> &[Value] {
    match outcome {
        Value::Tuple(items) => items,
        _ => unreachable!("transition outcomes are (transition_type, state, ...) tuples"),
    }
}

impl MapState for Value {
    type Transition = Value;
    type Timed = Value;
    type WithTime = Value;

    fn transition(transition: Transition<Value>) -> Value {
        Value::tuple(items(transition).to_vec())
    }

    fn read_transition(outcome: &Value) -> Transition<&Value> {
        read(tuple_items(outcome))
    }

    fn timed(transition: Transition<Value>, time: u64) -> Value {
        let [t, s] = items(transition);
        Value::tuple(vec![t, s, int(time as i64)])
    }

    fn read_timed(outcome: &Value) -> (Transition<&Value>, u64) {
        let items = tuple_items(outcome);
        let time = match &items[2] {
            Value::Int(t) => u64::try_from(t).expect("times count up from 0"),
            _ => unreachable!("times are ints"),
        };
        (read(items), time)
    }

    /// `transition_die.marginals[1:]`: `(state, time)`, `(None, time)` for a `Restart`.
    fn with_time(transition: Transition<Value>, time: u64) -> Result<Value> {
        let [_, s] = items(transition);
        Ok(Value::tuple(vec![s, int(time as i64)]))
    }

    /// Whether a NaN is in the state. Python's dicts and tuple `==` short-circuit on object *identity*, so the
    /// same NaN object found again (a state carried over a step, `Break()` to the current state) is the same key
    /// there, while a new NaN object is not: the port cannot tell which is which, so it does not guess.
    fn check_state(&self) -> Result<()> {
        fn has_nan(v: &Value) -> bool {
            match v {
                Value::Float(f) => f.is_nan(),
                Value::Complex(re, im) => re.is_nan() || im.is_nan(),
                Value::Tuple(items) => items.iter().any(has_nan),
                _ => false,
            }
        }
        if has_nan(self) {
            return Err(Error::Unported(
                "NaN in map(repeat) states (Python finds the same NaN object again by identity)"
                    .into(),
            ));
        }
        Ok(())
    }

    /// Whether two equal values are spelled the same, i.e. are the same object as far as Python's results can
    /// tell (`1` and `True` and `1.0` are equal but not interchangeable; neither are `0.0` and `-0.0`).
    fn same_spelling(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Float(x), Value::Float(y)) => x.to_bits() == y.to_bits(),
            (Value::Complex(xr, xi), Value::Complex(yr, yi)) => {
                xr.to_bits() == yr.to_bits() && xi.to_bits() == yi.to_bits()
            }
            (Value::Tuple(x), Value::Tuple(y)) => {
                x.len() == y.len() && x.iter().zip(y.iter()).all(|(a, b)| a.same_spelling(b))
            }
            // Host objects: the host knows; equal ones of the same kind are taken to be interchangeable.
            _ => std::mem::discriminant(self) == std::mem::discriminant(other),
        }
    }
}
