//! Errors mirror the Python exception that icepool raises, so the Python facade can re-raise the same type.
//! The harness only checks *that* both sides raised; the kind and message are informational.

use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// Python `TypeError`: e.g. comparing an `int` with a `str`, an unhashable outcome, `None` as an outcome.
    Type(String),
    /// Python `ValueError`: e.g. negative quantities, `d(0)`.
    Value(String),
    /// Python `IndexError`: e.g. `min_outcome()` of an empty die, `()[0]`.
    Index(String),
    /// Python `KeyError`.
    Key(String),
    /// Python `ZeroDivisionError`.
    ZeroDivision(String),
    /// Python `OverflowError`, or a size the port cannot represent (Python would run out of memory or time).
    Overflow(String),
    /// icepool `OrderError` (`ConflictingOrderError`): no pop order is compatible with the evaluation.
    Order(String),
    /// icepool `UnsupportedOrder`: an evaluator does not support the order it was given. The evaluation driver
    /// catches it and retries in the opposite order; it only escapes from places the driver does not guard.
    UnsupportedOrder(String),
    /// Not an exception: Python would answer, but the port does not cover this case yet (e.g. arithmetic on
    /// complex numbers). The program runner reports it as unimplemented, never as a raise.
    Unsupported(String),
}

impl Error {
    /// The Python exception type name, used in wire results.
    pub fn python_type(&self) -> &'static str {
        match self {
            Error::Type(_) => "TypeError",
            Error::Value(_) => "ValueError",
            Error::Index(_) => "IndexError",
            Error::Key(_) => "KeyError",
            Error::ZeroDivision(_) => "ZeroDivisionError",
            Error::Overflow(_) => "OverflowError",
            Error::Order(_) => "ConflictingOrderError",
            Error::UnsupportedOrder(_) => "UnsupportedOrder",
            Error::Unsupported(_) => "unimplemented",
        }
    }

    pub fn message(&self) -> &str {
        match self {
            Error::Type(m)
            | Error::Value(m)
            | Error::Index(m)
            | Error::Key(m)
            | Error::ZeroDivision(m)
            | Error::Overflow(m)
            | Error::Order(m)
            | Error::UnsupportedOrder(m)
            | Error::Unsupported(m) => m,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.python_type(), self.message())
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn type_error<T>(msg: impl Into<String>) -> Result<T> {
    Err(Error::Type(msg.into()))
}

pub(crate) fn value_error<T>(msg: impl Into<String>) -> Result<T> {
    Err(Error::Value(msg.into()))
}
