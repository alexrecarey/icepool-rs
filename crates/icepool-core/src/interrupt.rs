//! Interrupts: letting the user stop a long computation (Ctrl-C).
//!
//! Python checks for signals between bytecodes, so Ctrl-C stops pure-Python icepool within moments wherever it is.
//! The core is native code: a signal only sets a flag that the host must look at. Every loop that can run for a
//! long time (the evaluation driver's rooms, convolutions, `map(repeat)` steps, Markov chain elimination, big
//! constructions) calls [`poll`], which asks the host's check (`set_check`; the Python facade's raises
//! `KeyboardInterrupt` through `PyErr_CheckSignals`). The error it returns propagates like any other, so a
//! computation stops with no state left behind.
//!
//! Asking the host is not free (the facade reacquires the GIL, which may mean waiting for another thread), so
//! `poll` asks at most once every `INTERVAL`. Without a check (the native runner, WASM) `poll` does nothing; in
//! particular it never reads the clock, which WASM does not have.

use std::cell::Cell;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use crate::error::Result;

/// The longest a computation runs between two checks: well under the 0.2 s that Python takes to react.
const INTERVAL: Duration = Duration::from_millis(50);

static CHECK: OnceLock<fn() -> Result<()>> = OnceLock::new();

thread_local! {
    /// When this thread next asks the host (`None`: at the next poll).
    static NEXT_CHECK: Cell<Option<Instant>> = const { Cell::new(None) };
}

/// The host's check for an interrupt: the error it returns (e.g. `Error::Host` holding a `KeyboardInterrupt`) stops
/// the computation. Set once, by the Python facade.
pub fn set_check(check: fn() -> Result<()>) {
    let _ = CHECK.set(check);
}

/// Called in long loops: the host's check, if it has one and has not been asked in the last `INTERVAL`.
pub fn poll() -> Result<()> {
    let Some(check) = CHECK.get() else {
        return Ok(());
    };
    let now = Instant::now();
    if NEXT_CHECK.with(Cell::get).is_some_and(|next| now < next) {
        return Ok(());
    }
    NEXT_CHECK.with(|n| n.set(Some(now + INTERVAL)));
    check()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Error;

    thread_local! {
        static INTERRUPTED: Cell<bool> = const { Cell::new(false) };
    }

    /// Interrupts only the test thread that asked for it (tests run in parallel and the check is global).
    fn check() -> Result<()> {
        if INTERRUPTED.with(Cell::get) {
            Err(Error::Value("interrupted".into()))
        } else {
            Ok(())
        }
    }

    #[test]
    fn an_interrupt_stops_an_evaluation() {
        set_check(check);
        let pool = crate::Pool::new(vec![(crate::function::try_d::<crate::Value>(20).unwrap(), 30)]).unwrap();
        let sum = || {
            let sources = [crate::pool::MultisetSource::Pool(pool.source())];
            crate::evaluator::evaluate(&crate::evaluator::basic::SumEvaluator, &sources)
        };
        // Works without an interrupt...
        assert!(sum().is_ok());
        // ...and stops with the check's error once there is one.
        INTERRUPTED.with(|i| i.set(true));
        NEXT_CHECK.with(|n| n.set(None));
        assert_eq!(sum().err(), Some(Error::Value("interrupted".into())));
        INTERRUPTED.with(|i| i.set(false));
    }
}
