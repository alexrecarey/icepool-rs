//! `Value::Host`: an outcome owned by the host language, for which the core has no native variant (in Python:
//! `Vector`, `Symbols`, namedtuples, enums, user classes...). See docs/ANALYSIS.md, "Outcome type": the escape hatch
//! that keeps the core static while any Python outcome still works.
//!
//! The core never looks inside a host object. Whenever an operator or comparison has a host object on either side,
//! it hands *both* operands to the host, which applies its own semantics (in Python: the real `operator.add(a, b)`,
//! with `__radd__` and `NotImplemented` dispatch). The native variants keep their ported semantics among themselves.

use std::any::Any;
use std::fmt;
use std::sync::Arc;

use super::{BinOp, UnOp, Value};
use crate::error::Result;

/// What the core needs from a host object. `a` and `b` are the full operands in Python's order; at least one of
/// them is this host object.
pub trait HostObject: Send + Sync + fmt::Debug {
    /// The host's hash, computed once when the object entered the core. Equal values must hash alike, as Python
    /// requires; see `Hash for Value` for how this combines with native values.
    fn py_hash(&self) -> i64;
    /// Python's `a == b`, identity first as dict lookups and tuple comparisons do. An exception counts as unequal
    /// (Rust's `Eq` cannot fail).
    fn eq(&self, a: &Value, b: &Value) -> bool;
    /// Python's `a <op> b` for the comparisons `<`, `<=`, `>`, `>=`, as a truth value (`bool(a < b)`).
    fn compare(&self, op: BinOp, a: &Value, b: &Value) -> Result<bool>;
    /// Python's `operator.<op>(a, b)`, comparisons included (they need not return a bool).
    fn binary(&self, op: BinOp, a: &Value, b: &Value) -> Result<Value>;
    /// Python's unary operator on this object.
    fn unary(&self, op: UnOp) -> Result<Value>;
    /// Python's `bool(self)`.
    fn truthy(&self) -> bool;
    /// Python's `repr(self)`.
    fn repr(&self) -> String;
    /// For the host's own downcast back to its object.
    fn as_any(&self) -> &dyn Any;
}

/// A shared reference to a host object.
#[derive(Clone, Debug)]
pub struct HostRef(pub Arc<dyn HostObject>);

impl HostRef {
    pub fn new(obj: impl HostObject + 'static) -> Self {
        HostRef(Arc::new(obj))
    }
}

impl std::ops::Deref for HostRef {
    type Target = dyn HostObject;
    fn deref(&self) -> &Self::Target {
        &*self.0
    }
}

/// An exception raised by the host inside a callback, carried through the core unchanged so the host can re-raise
/// the very same exception (its type and message) once the core returns.
#[derive(Clone, Debug)]
pub struct HostError(pub Arc<dyn Any + Send + Sync>);

impl PartialEq for HostError {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for HostError {}

/// The host object among two operands, if any (the left one first).
pub(super) fn host_of<'a>(a: &'a Value, b: &'a Value) -> Option<&'a HostRef> {
    match (a, b) {
        (Value::Host(h), _) | (_, Value::Host(h)) => Some(h),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Die, DieArg, Error, Outcome, Quantity};

    /// A host "number" that sorts in reverse and adds like an int, and whose `-` raises (a host exception).
    #[derive(Debug)]
    struct Rev(i64);

    fn rev(v: &Value) -> Option<i64> {
        match v {
            Value::Host(h) => h.as_any().downcast_ref::<Rev>().map(|r| r.0),
            _ => None,
        }
    }

    impl HostObject for Rev {
        fn py_hash(&self) -> i64 {
            self.0
        }
        fn eq(&self, a: &Value, b: &Value) -> bool {
            rev(a).is_some() && rev(a) == rev(b)
        }
        fn compare(&self, op: BinOp, a: &Value, b: &Value) -> Result<bool> {
            let (Some(x), Some(y)) = (rev(a), rev(b)) else {
                return Err(Error::Type("not comparable".into()));
            };
            Ok(match op {
                BinOp::Lt => x > y,
                BinOp::Gt => x < y,
                BinOp::Le => x >= y,
                _ => x <= y,
            })
        }
        fn binary(&self, op: BinOp, a: &Value, b: &Value) -> Result<Value> {
            match (op, rev(a), rev(b)) {
                (BinOp::Add, Some(x), Some(y)) => Ok(host(x + y)),
                _ => Err(Error::Type("unsupported".into())),
            }
        }
        fn unary(&self, _op: UnOp) -> Result<Value> {
            Err(Error::Host(HostError(Arc::new("raised by the host"))))
        }
        fn truthy(&self) -> bool {
            self.0 != 0
        }
        fn repr(&self) -> String {
            format!("Rev({})", self.0)
        }
        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    fn host(x: i64) -> Value {
        Value::Host(HostRef::new(Rev(x)))
    }

    fn die(xs: &[i64]) -> Die<Value> {
        let args = xs.iter().map(|&x| DieArg::Outcome(host(x))).collect();
        Die::new(args, vec![Quantity::ONE; xs.len()]).unwrap()
    }

    #[test]
    fn host_objects_sort_merge_and_operate_by_the_host() {
        let d = die(&[1, 3, 2, 3]);
        let shown: Vec<String> = d.items().iter().map(|(o, q)| format!("{o}:{q}")).collect();
        assert_eq!(shown, ["Rev(3):2", "Rev(2):1", "Rev(1):1"]);
        let sum = d
            .binary_operator(&die(&[0, 10]), |a, b| a.binary(BinOp::Add, b))
            .unwrap();
        assert_eq!(sum.len(), 6);
        assert_eq!(sum.items()[0].0.to_string(), "Rev(13)");
        // Mixed with a native value, the host decides (and here refuses).
        assert!(host(1).py_lt(&Value::from(1)).is_err());
        assert!(host(1).binary(BinOp::Add, &Value::from(1)).is_err());
        assert_ne!(host(1), Value::from(1));
        // Inside a tuple, the native tuple comparison reaches the host for the differing element.
        let t = |x| Value::tuple(vec![Value::from(0), host(x)]);
        assert_eq!(t(5).py_lt(&t(4)), Ok(true));
        // A host exception passes through unchanged.
        let err = d.unary_operator(|o| o.unary(UnOp::Neg)).unwrap_err();
        assert!(matches!(err, Error::Host(_)));
        assert!(!host(0).truthy() && host(2).truthy());
    }
}
