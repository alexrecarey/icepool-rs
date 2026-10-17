//! Python's recursion limits, for recursion that user data drives: nested tuples and `Vector`s.
//!
//! Python raises `RecursionError` where a native port that simply recursed would overflow its stack (and crash the
//! process: a segfault through the Python facade, a trap in WASM). The core follows Python's two limits:
//!
//! - **Levels** (`level`): the C recursion limit. CPython compares, `repr`s and hashes nested tuples in C, and
//!   3.13 raises `RecursionError` once that C recursion is `Py_C_RECURSION_LIMIT` (10000) deep: `t == u` for two
//!   tuples nested 9998 deep. Every recursive function on `Value`s that can fail takes a level, so no recursion
//!   on user data goes deeper than `MAX_DEPTH` in the core. (CPython hashes without a limit, so it builds a die of
//!   a tuple nested 100000 deep, and crashes further on; the port raises from `MAX_DEPTH`: DIVERGENCES.md.)
//!
//!   Python's C recursion is lean; the port's is not always (converting a Python tuple takes about 500 bytes of
//!   stack a level), so a level also raises once the recursion has used `STACK_LIMIT` bytes of stack since its
//!   outermost level: tuples nested several thousand deep may raise a little before Python would (DIVERGENCES.md).
//!   Recursive functions keep their frames small (heavy branches in separate functions) to push that back.
//!
//!   The infallible traits (`PartialEq`, `Hash`, `Display`, `Drop`) do not count levels: they recurse no deeper
//!   than a value is nested (with small frames), and every outcome of a die passed `check_key`, which does.
//!
//! - **Python frames** (`frames`): icepool's `Vector` operators are Python methods, so each nested vector costs
//!   Python frames, and Python raises once its stack is `sys.getrecursionlimit()` frames deep: `t + t` for a vector
//!   nested 250 deep at top level. The core counts the frames Python would use (`Vector`'s methods say how many)
//!   and raises when they and the frames already on the host's stack exceed the limit. A host (the Python facade)
//!   says how many frames its stack has left with `set_frame_budget`; without one (the native runner) the stack is
//!   taken as empty with Python's default limit, 1000.

use std::cell::Cell;
use std::sync::OnceLock;

use crate::error::{Error, Result};

/// The deepest the core recurses on nested values. In WASM the stack is 1 MiB, so less.
pub const MAX_DEPTH: usize = if cfg!(target_family = "wasm") {
    1000
} else {
    10_000
};

/// The most stack a recursion on nested values may use: a process's main thread has 8 MiB (Linux, macOS), and the
/// host needs some too. In WASM the whole stack is 1 MiB.
pub const STACK_LIMIT: usize = if cfg!(target_family = "wasm") {
    512 << 10
} else {
    6 << 20
};

/// Python's default `sys.getrecursionlimit()`.
pub const DEFAULT_RECURSION_LIMIT: usize = 1000;

/// Emulated frames that never ask the host for its budget: asking walks the host's stack, too slow for every
/// vector operator. Python would raise for these only on a stack already within a few frames of its limit.
const FREE_FRAMES: usize = 16;

thread_local! {
    static LEVELS: Cell<usize> = const { Cell::new(0) };
    /// The stack address of the outermost level.
    static STACK_BASE: Cell<usize> = const { Cell::new(0) };
    static FRAMES: Cell<usize> = const { Cell::new(0) };
}

static FRAME_BUDGET: OnceLock<fn() -> usize> = OnceLock::new();

/// Python's `RecursionError`.
pub fn recursion_error() -> Error {
    Error::Recursion("maximum recursion depth exceeded".into())
}

/// One level of recursion on a nested value, given back when dropped.
#[must_use]
pub struct Level(());

/// Take a level of recursion, or raise `RecursionError` past `MAX_DEPTH` levels or `STACK_LIMIT` bytes of stack.
pub fn level() -> Result<Level> {
    // The address of a local is where the stack is now. Stacks grow down (native targets and WASM's shadow stack).
    let here = 0u8;
    let sp = std::ptr::addr_of!(here) as usize;
    let depth = LEVELS.with(Cell::get);
    if depth == 0 {
        STACK_BASE.with(|b| b.set(sp));
    } else if depth >= MAX_DEPTH || STACK_BASE.with(Cell::get).saturating_sub(sp) > STACK_LIMIT {
        return Err(recursion_error());
    }
    LEVELS.with(|l| l.set(depth + 1));
    Ok(Level(()))
}

impl Drop for Level {
    fn drop(&mut self) {
        LEVELS.with(|l| l.set(l.get() - 1));
    }
}

/// Python frames in use by the core's emulation of a Python method, given back when dropped.
#[must_use]
pub struct Frames(usize);

/// Take the `n` Python frames a method of icepool's would push, or raise `RecursionError` if Python's stack would
/// overflow.
pub fn frames(n: usize) -> Result<Frames> {
    let used = FRAMES.with(Cell::get) + n;
    if used > FREE_FRAMES {
        let budget = FRAME_BUDGET.get().map_or(DEFAULT_RECURSION_LIMIT, |f| f());
        if used > budget {
            return Err(recursion_error());
        }
    }
    FRAMES.with(|f| f.set(used));
    Ok(Frames(n))
}

impl Drop for Frames {
    fn drop(&mut self) {
        FRAMES.with(|f| f.set(f.get() - self.0));
    }
}

/// The host's frame budget: how many more frames its stack holds right now (`sys.getrecursionlimit()` minus the
/// frames on it). Frames the core emulates for an outer call that then called back into the host are not on the
/// host's stack, and the core adds them itself. Set once, by the Python facade.
pub fn set_frame_budget(budget: fn() -> usize) {
    let _ = FRAME_BUDGET.set(budget);
}

/// Gives back every level and frame. Only for the WASM facade after a trap: a trap skips the `Drop`s that would
/// have given them back, so the next call would start out deep.
pub fn reset() {
    LEVELS.with(|l| l.set(0));
    FRAMES.with(|f| f.set(0));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_stop_at_the_limit_and_come_back() {
        let held: Vec<Level> = (0..MAX_DEPTH).map(|_| level().unwrap()).collect();
        assert_eq!(level().err(), Some(recursion_error()));
        drop(held);
        assert!(level().is_ok());
    }

    #[test]
    fn frames_stop_at_the_default_limit() {
        let held: Vec<Frames> = (0..DEFAULT_RECURSION_LIMIT / 4)
            .map(|_| frames(4).unwrap())
            .collect();
        assert!(frames(1).is_err());
        drop(held);
        assert!(frames(4).is_ok());
    }
}
