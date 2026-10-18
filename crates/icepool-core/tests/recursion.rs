//! Recursion on nested values stays within the stack (`value::recursion`): every recursive operation on a value
//! nested deeper than `MAX_DEPTH` raises `RecursionError` instead of overflowing a thread with a main thread's
//! stack (8 MiB), and shallower ones do not raise.
//!
//! A separate test binary: it sets the process-wide frame budget (as the Python facade does for a raised
//! `sys.setrecursionlimit`), so nested vectors recurse as deep as nested tuples.

use std::collections::HashSet;

use icepool_core::value::pystr::py_repr;
use icepool_core::value::recursion::{set_frame_budget, MAX_DEPTH};
use icepool_core::value::{BinOp, UnOp, Vector};
use icepool_core::{Die, Error, Quantity, Value};
use icepool_core::value::DieArg;

const STACK: usize = 8 << 20;

fn tuple(depth: usize, leaf: i64) -> Value {
    (0..depth).fold(Value::from(leaf), |v, _| Value::tuple(vec![v]))
}

fn vector(depth: usize, leaf: i64) -> Value {
    (0..depth).fold(Value::from(leaf), |v, _| {
        Value::Vector(Vector::new(vec![v]))
    })
}

fn is_recursion_error<T: std::fmt::Debug>(r: icepool_core::Result<T>) -> bool {
    match r {
        Err(Error::RecursionLimit(_)) => true,
        other => panic!("expected RecursionError, got {other:?}"),
    }
}

fn die(v: Value) -> icepool_core::Result<Die<Value>> {
    Die::new(vec![DieArg::Outcome(v)], vec![Quantity::from(1u8)])
}

/// Everything fallible that recurses on a value, applied to `x` and `y`, which differ only at the bottom (so that
/// comparisons recurse all the way down).
fn all_ops(x: &Value, y: &Value) -> Vec<(&'static str, icepool_core::Result<()>)> {
    type Op = fn(&Value, &Value) -> icepool_core::Result<()>;
    let ops: [(&str, Op); 12] = [
        ("check_key", |x, _| x.py_check_key()),
        ("py_lt", |x, y| ordering(x, y, |x, y| x.py_lt(y))),
        ("py_le", |x, y| ordering(x, y, |x, y| x.py_le(y))),
        ("py_eq", |x, y| x.py_eq(y).map(drop)),
        ("binary <", |x, y| {
            ordering(x, y, |x, y| x.binary(BinOp::Lt, y))
        }),
        ("binary +", |x, y| x.binary(BinOp::Add, y).map(drop)),
        ("binary 1 +", |x, _| {
            Value::from(1).binary(BinOp::Add, x).map(drop)
        }),
        ("eq_op", |x, y| x.eq_op(y, false).map(drop)),
        ("unary -", |x, _| x.unary(UnOp::Neg).map(drop)),
        ("round", |x, _| x.round(&Value::None).map(drop)),
        ("repr", |x, _| py_repr(x).map(drop)),
        ("die", |x, _| die(x.clone()).map(drop)),
    ];
    ops.into_iter().map(|(name, op)| (name, op(x, y))).collect()
}

/// An ordering, skipped for vectors not nested too deep (exponential time), which give a `TypeError` instead.
fn ordering<T>(
    x: &Value,
    y: &Value,
    f: fn(&Value, &Value) -> icepool_core::Result<T>,
) -> icepool_core::Result<()> {
    match x {
        Value::Vector(_) if x.py_check_key().is_ok() => Err(Error::WrongType("skipped".into())),
        _ => f(x, y).map(drop),
    }
}

fn on_a_main_thread_stack(f: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(STACK)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn deep_values_raise_and_shallower_ones_do_not_overflow() {
    // As if Python's recursion limit had been raised far beyond the frames nested vectors use.
    set_frame_budget(|| usize::MAX);
    on_a_main_thread_stack(|| {
        for (is_tuple, make) in [(true, tuple as fn(usize, i64) -> Value), (false, vector)] {
            let (x, y) = (make(MAX_DEPTH + 1, 1), make(MAX_DEPTH + 1, 2));
            for (name, r) in all_ops(&x, &y) {
                // Tuples: `+` concatenates without recursing, and there is no `1 + t`, `-t` or `round(t)`.
                if is_tuple
                    && matches!(
                        &r,
                        Ok(()) | Err(Error::WrongType(_) | Error::Incomparable { .. })
                    )
                {
                    continue;
                }
                assert!(is_recursion_error(r), "{name}");
            }
            // The infallible traits recurse too, as deep as the value: equality, hashing, Display, drop.
            assert!(x != y && x == x.clone());
            assert_eq!(HashSet::from([x.clone(), y.clone()]).len(), 2);
            assert!(!x.to_string().is_empty());
            drop((x, y));

            // Well within the limits, everything works. (Ordering two vectors takes time exponential in their
            // depth, in Python too: each level orders the elements for the truth value, then again elementwise.)
            let (x, y) = (make(2000, 1), make(2000, 2));
            for (name, r) in all_ops(&x, &y) {
                assert!(!matches!(r, Err(Error::RecursionLimit(_))), "{name}: {r:?}");
            }
        }
    });
}
