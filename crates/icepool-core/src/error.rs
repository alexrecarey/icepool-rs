//! Why an operation failed, in the core's own terms.
//!
//! Each variant is one domain condition, and each condition is raised by exactly one Python exception type at
//! the places icepool raises it: that is what lets the Python facade (`icepool-py`) and the program runner map
//! variants to exception names with a plain `match`, never by inspecting messages. The mapping lives with them,
//! not here: the core knows nothing of Python's exception names.
//!
//! Messages are Python's own text wherever Python has one (upstream tests and the facade may match on them);
//! [`Display`](fmt::Display) prints the message alone.

use std::fmt;

use crate::host::HostError;

/// Why an operation failed.
///
/// ```
/// use icepool_core::Error;
/// let e = Error::Incomparable { op: "<", left: "int", right: "str" };
/// assert_eq!(e.to_string(), "'<' not supported between instances of 'int' and 'str'");
/// let e = Error::InvalidArgument("sides must be at least 1.".into());
/// assert_eq!(e.to_string(), "sides must be at least 1.");
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    // --- Arguments ---
    /// An argument has the right kind but a value outside what is allowed: a negative quantity, `d(0)`, a
    /// keep index past the pool's size. (Python's `ValueError`.)
    InvalidArgument(String),
    /// An argument or operand of the wrong kind: `None` as an outcome, a float where an int is needed, an
    /// operator between outcomes that do not support it. (Python's `TypeError`.)
    WrongType(String),
    /// The object has no such attribute or method, e.g. a deck method called on a die. (Python's
    /// `AttributeError`.)
    MissingAttribute(String),
    /// An index outside a sequence, or a minimum or maximum of nothing: `min_outcome()` of an empty die,
    /// `()[0]`. (Python's `IndexError`.)
    IndexOutOfRange(String),
    /// A lookup of a key that is not there, e.g. a deck's quantity of a card it lacks. (Python's `KeyError`.)
    MissingKey(String),

    // --- Outcomes (raised from the `Value` layer: Python's operators) ---
    /// An order comparison between outcomes that have none, e.g. `1 < "a"`. (Python's `TypeError`.)
    Incomparable {
        /// The comparison operator, e.g. `"<"`.
        op: &'static str,
        /// The Python type name of the left operand.
        left: &'static str,
        /// The Python type name of the right operand.
        right: &'static str,
    },
    /// An outcome that cannot be hashed (a list), used where Python hashes: a dict key, an outcome. (Python's
    /// `TypeError`.)
    Unhashable {
        /// The Python type name of the unhashable value.
        type_name: &'static str,
    },
    /// A division or modulo by zero. (Python's `ZeroDivisionError`.)
    DivisionByZero(String),

    // --- Evaluation order ---
    /// No pop order is compatible with every part of an evaluation. (icepool's `ConflictingOrderError`.)
    ConflictingOrder(String),
    /// An operator needs the size of its right operand to choose an order, and the size cannot be inferred
    /// (`sort_pair` with `extra` against an operand of unknown size). (icepool's base `OrderError`.)
    UninferrableSize(String),
    /// An evaluator does not support the order it was given. The evaluation driver catches it and retries in the
    /// opposite order; it only escapes from places the driver does not guard. (icepool's `UnsupportedOrder`.)
    UnsupportedOrder(String),

    // --- Limits ---
    /// Python's stack would overflow: nested values beyond `value::recursion`'s limit, a sum of very many dice.
    /// (Python's `RecursionError`.)
    RecursionLimit(String),
    /// A result no machine integer holds (an `i64` shift, a float conversion), or a size the port cannot
    /// represent where Python would run out of memory or time. (Python's `OverflowError`.)
    Overflow(String),
    /// A size given by user input that no allocation can hold (`memory`), e.g. `d(2**40)`. (Python's
    /// `MemoryError`.)
    TooLarge(String),

    // --- Coverage ---
    /// icepool itself does not implement this, e.g. `mean_time_to_absorb` with `Restart`. Not to be confused
    /// with [`Error::Unported`], where Python answers and the port does not. (Python's `NotImplementedError`.)
    NotImplementedUpstream(String),
    /// Not an exception: Python would answer, but the port does not cover this case yet (e.g. arithmetic on
    /// complex numbers). The program runner reports it as unimplemented, never as a raise.
    Unported(String),

    // --- Callbacks ---
    /// An exception the host raised in a callback (a host object's operator, a user function), passed through
    /// so the host re-raises it unchanged.
    Callback(HostError),
}

/// A comparison of an `Ord` outcome cannot fail (`OutcomeOrder::Incomparable` is `Infallible`).
impl From<std::convert::Infallible> for Error {
    fn from(never: std::convert::Infallible) -> Self {
        match never {}
    }
}

impl Error {
    /// Whether this is an operand or argument of the wrong kind ([`Error::WrongType`], [`Error::Incomparable`],
    /// [`Error::Unhashable`]): what Python raises as one exception type (`TypeError`), so what icepool's
    /// `except TypeError` catches where the port copies one (returning `NotImplemented`, trying a fallback).
    ///
    /// ```
    /// use icepool_core::Error;
    /// assert!(Error::WrongType("bad operand".into()).is_wrong_kind());
    /// assert!(Error::Unhashable { type_name: "list" }.is_wrong_kind());
    /// assert!(!Error::InvalidArgument("sides must be at least 1.".into()).is_wrong_kind());
    /// ```
    pub fn is_wrong_kind(&self) -> bool {
        matches!(
            self,
            Error::WrongType(_) | Error::Incomparable { .. } | Error::Unhashable { .. }
        )
    }
}

impl fmt::Display for Error {
    /// The message alone, without an exception name: Python's own text where Python has one.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::InvalidArgument(m)
            | Error::WrongType(m)
            | Error::MissingAttribute(m)
            | Error::IndexOutOfRange(m)
            | Error::MissingKey(m)
            | Error::DivisionByZero(m)
            | Error::ConflictingOrder(m)
            | Error::UninferrableSize(m)
            | Error::UnsupportedOrder(m)
            | Error::RecursionLimit(m)
            | Error::Overflow(m)
            | Error::TooLarge(m)
            | Error::NotImplementedUpstream(m)
            | Error::Unported(m) => f.write_str(m),
            Error::Incomparable { op, left, right } => {
                write!(
                    f,
                    "'{op}' not supported between instances of '{left}' and '{right}'"
                )
            }
            Error::Unhashable { type_name } => write!(f, "unhashable type: '{type_name}'"),
            Error::Callback(_) => f.write_str("raised by a host callback"),
        }
    }
}

impl std::error::Error for Error {}

/// The result of a fallible operation of the core.
///
/// ```
/// use icepool_core::{try_d, Error, Result};
/// fn sides(n: i64) -> Result<usize> {
///     Ok(try_d::<i64>(n)?.len())
/// }
/// assert_eq!(sides(6), Ok(6));
/// assert!(matches!(sides(0), Err(Error::InvalidArgument(_))));
/// ```
pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn wrong_type<T>(msg: impl Into<String>) -> Result<T> {
    Err(Error::WrongType(msg.into()))
}

pub(crate) fn invalid_argument<T>(msg: impl Into<String>) -> Result<T> {
    Err(Error::InvalidArgument(msg.into()))
}
