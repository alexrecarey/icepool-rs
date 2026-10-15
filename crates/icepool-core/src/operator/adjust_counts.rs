//! Port of icepool `operator/adjust_counts.py`: operators that adjust the counts of one expression (multiply,
//! floor divide, modulo, unique, keep_counts) and `map_counts`.

use super::{times, Statelet};
use crate::error::{Error, Result};
use crate::pool::Count;

/// The function of `MultisetMapCounts`: takes `outcome, *counts` and produces a combined count. Python's is any
/// callable (a host callback for the facade), given `int` or tuple counts; a result that is neither an `int` nor
/// a tuple of them is `Unsupported`.
pub trait CountFunction<O> {
    fn call(&self, outcome: &O, counts: &[Count]) -> Result<Count>;
}

/// The comparison of `MultisetKeepCounts` (the constructor raises `ValueError` for any other string).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum KeepCountsComparison {
    Eq,
    Ne,
    Le,
    Lt,
    Ge,
    Gt,
}

impl KeepCountsComparison {
    pub fn parse(comparison: &str) -> Result<Self> {
        Ok(match comparison {
            "==" => KeepCountsComparison::Eq,
            "!=" => KeepCountsComparison::Ne,
            "<=" => KeepCountsComparison::Le,
            "<" => KeepCountsComparison::Lt,
            ">=" => KeepCountsComparison::Ge,
            ">" => KeepCountsComparison::Gt,
            _ => return Err(Error::Value(format!("Invalid comparison {comparison}"))),
        })
    }

    /// The comparison as given to the constructor (`__str__` shows it).
    pub fn symbol(self) -> &'static str {
        match self {
            KeepCountsComparison::Eq => "==",
            KeepCountsComparison::Ne => "!=",
            KeepCountsComparison::Le => "<=",
            KeepCountsComparison::Lt => "<",
            KeepCountsComparison::Ge => ">=",
            KeepCountsComparison::Gt => ">",
        }
    }

    pub(super) fn test(self, count: i64, constant: i64) -> bool {
        match self {
            KeepCountsComparison::Eq => count == constant,
            KeepCountsComparison::Ne => count != constant,
            KeepCountsComparison::Le => count <= constant,
            KeepCountsComparison::Lt => count < constant,
            KeepCountsComparison::Ge => count >= constant,
            KeepCountsComparison::Gt => count > constant,
        }
    }
}

/// `MultisetMultiplyCounts._initial_state`: the child's size times the constant, if known (`tuple * n` repeats
/// a tuple size).
pub(super) fn multiply_initial_state(
    child_size: Option<&Count>,
    constant: i64,
) -> Result<(Statelet, Option<Count>)> {
    let size = child_size.map(|s| times(s, constant)).transpose()?;
    Ok((Statelet::None, size))
}

/// Python's `count // constant` (rounds towards negative infinity).
pub(super) fn floordiv(count: i64, constant: i64) -> Result<i64> {
    if constant == 0 {
        return Err(Error::ZeroDivision(
            "integer division or modulo by zero".into(),
        ));
    }
    let q = count.checked_div(constant).ok_or_else(super::overflow)?;
    Ok(if count % constant != 0 && (count < 0) != (constant < 0) {
        q - 1
    } else {
        q
    })
}

/// Python's `count % constant` (the sign of the constant).
pub(super) fn modulo(count: i64, constant: i64) -> Result<i64> {
    if constant == 0 {
        return Err(Error::ZeroDivision(
            "integer division or modulo by zero".into(),
        ));
    }
    // `i64::MIN % -1` overflows in Rust; Python's answer is 0.
    let r = count.checked_rem(constant).unwrap_or(0);
    Ok(if r != 0 && (r < 0) != (constant < 0) {
        r + constant
    } else {
        r
    })
}
