//! `Value::Host`: an outcome owned by the host language, for which the core has no native variant (in Python:
//! namedtuples, enums, user classes...). See docs/ANALYSIS.md, "Outcome type": the escape hatch
//! that keeps the core static while any Python outcome still works.
//!
//! The core never looks inside a host object. Whenever an operator or comparison has a host object on either side,
//! it hands *both* operands to the host, which applies its own semantics (in Python: the real `operator.add(a, b)`,
//! with `__radd__` and `NotImplemented` dispatch). The native variants keep their ported semantics among themselves.

use std::any::Any;
use std::fmt;
use std::sync::Arc;

use dashu_ratio::RBig;

use crate::error::{Error, Result};
use crate::value::builtins::Index;
use crate::value::{BinOp, UnOp, Value};

/// What the core needs from a host object. `a` and `b` are the full operands in Python's order; at least one of
/// them is this host object.
///
/// ```
/// use std::any::Any;
/// use std::sync::Arc;
/// use icepool_core::host::{HostError, HostObject, HostRef, LenKind};
/// use icepool_core::value::{BinOp, UnOp};
/// use icepool_core::{Error, Result, Value};
/// /// A binding's host object: a number that sorts and adds like an int, but is never equal to a native one.
/// #[derive(Debug)]
/// struct Pip(i64);
/// /// The `Pip` inside a value, if it is one: the binding downcasts back to its own type.
/// fn pip(v: &Value) -> Option<i64> {
///     match v {
///         Value::Host(h) => h.as_any().downcast_ref::<Pip>().map(|p| p.0),
///         _ => None,
///     }
/// }
/// fn host(x: i64) -> Value {
///     Value::Host(HostRef::new(Pip(x)))
/// }
/// impl HostObject for Pip {
///     fn hash_value(&self) -> i64 { self.0 }
///     fn eq(&self, a: &Value, b: &Value) -> bool { pip(a).is_some() && pip(a) == pip(b) }
///     fn compare(&self, op: BinOp, a: &Value, b: &Value) -> Result<bool> {
///         let (Some(x), Some(y)) = (pip(a), pip(b)) else {
///             return Err(Error::WrongType("'<' not supported".into()));
///         };
///         Ok(match op { BinOp::Lt => x < y, BinOp::Le => x <= y, BinOp::Gt => x > y, _ => x >= y })
///     }
///     fn binary(&self, op: BinOp, a: &Value, b: &Value) -> Result<Value> {
///         match (op, pip(a), pip(b)) {
///             (BinOp::Add, Some(x), Some(y)) => Ok(host(x + y)),
///             _ => Err(Error::WrongType("unsupported operand type(s)".into())),
///         }
///     }
///     fn unary(&self, op: UnOp) -> Result<Value> {
///         // Any other operator raises in the host: the core carries the exception through unchanged.
///         match op {
///             UnOp::Neg => Ok(host(-self.0)),
///             _ => Err(Error::Callback(HostError(Arc::new("bad operand type")))),
///         }
///     }
///     fn truthy(&self) -> Result<bool> { Ok(self.0 != 0) }
///     fn repr(&self) -> String { format!("Pip({})", self.0) }
///     fn as_any(&self) -> &dyn Any { self }
///     // Optional methods: the defaults say the case is unported.
///     fn to_float(&self) -> Result<f64> { Ok(self.0 as f64) }
///     fn as_index(&self) -> Result<Option<dashu_int::IBig>> { Ok(Some(self.0.into())) }
///     fn try_str(&self) -> Result<String> { Ok(format!("{} pips", self.0)) }
///     fn len_kind(&self) -> Result<LenKind> { Ok(LenKind::Unsized) }
/// }
/// // The core sorts, merges and operates on host outcomes by calling back: Python's `d = Die([Pip(3), Pip(1)])`,
/// // then `d + d`.
/// use icepool_core::{Die, Quantity};
/// let die = Die::from_dict(vec![(host(3), Quantity::ONE), (host(1), Quantity::ONE)]).unwrap();
/// let outcomes: Vec<String> = die.outcomes().map(|o| o.to_string()).collect();
/// assert_eq!(outcomes, ["Pip(1)", "Pip(3)"]);
/// let sum = die.binary_operator(&die, |a, b| a.binary(BinOp::Add, b)).unwrap();
/// let sum: Vec<String> = sum.items().iter().map(|(o, q)| format!("{o}: {q}")).collect();
/// assert_eq!(sum, ["Pip(2): 1", "Pip(4): 2", "Pip(6): 1"]);
/// ```
pub trait HostObject: Send + Sync + fmt::Debug {
    /// The host's hash, computed once when the object entered the core. Equal values must hash alike, as Python
    /// requires; see `Hash for Value` for how this combines with native values.
    ///
    /// ```
    /// # include!("../../doctest/host_pip.rs");
    /// # fn main() {
    /// use std::hash::BuildHasher;
    /// // `Pip` hashes as Python hashes the int it holds, so it may equal (and must then hash like) that int.
    /// let hasher = rustc_hash::FxBuildHasher;
    /// assert_eq!(hasher.hash_one(&host(3)), hasher.hash_one(&host(3)));
    /// assert_eq!(hasher.hash_one(&host(3)), hasher.hash_one(&Value::from(3)));
    /// # }
    /// ```
    fn hash_value(&self) -> i64;
    /// Python's `a == b`, identity first as dict lookups and tuple comparisons do. An exception counts as unequal
    /// (Rust's `Eq` cannot fail).
    ///
    /// ```
    /// # include!("../../doctest/host_pip.rs");
    /// # fn main() {
    /// assert_eq!(host(2), host(2));
    /// // The host decides for a native operand too: this `Pip` is never equal to an int.
    /// assert_ne!(host(2), Value::from(2));
    /// # }
    /// ```
    fn eq(&self, a: &Value, b: &Value) -> bool;
    /// Python's `a <op> b` for the comparisons `<`, `<=`, `>`, `>=`, as a truth value (`bool(a < b)`).
    ///
    /// ```
    /// # include!("../../doctest/host_pip.rs");
    /// # fn main() {
    /// use icepool_core::OutcomeOrder;
    /// assert_eq!(host(1).try_lt(&host(2)), Ok(true));
    /// assert_eq!(host(2).try_ge(&host(2)), Ok(true));
    /// // An exception is the comparison's error.
    /// assert!(host(1).try_lt(&Value::from(2)).is_err());
    /// # }
    /// ```
    fn compare(&self, op: BinOp, a: &Value, b: &Value) -> Result<bool>;
    /// Python's `operator.<op>(a, b)`, comparisons included (they need not return a bool).
    ///
    /// ```
    /// # include!("../../doctest/host_pip.rs");
    /// # fn main() {
    /// assert_eq!(host(1).binary(BinOp::Add, &host(2)).unwrap(), host(3));
    /// // The host receives both operands, in Python's order, whichever side it is on.
    /// assert!(Value::from(1).binary(BinOp::Add, &host(2)).is_err());
    /// # }
    /// ```
    fn binary(&self, op: BinOp, a: &Value, b: &Value) -> Result<Value>;
    /// Python's unary operator on this object.
    ///
    /// ```
    /// # include!("../../doctest/host_pip.rs");
    /// # fn main() {
    /// assert_eq!(host(2).unary(UnOp::Neg).unwrap(), host(-2));
    /// assert!(matches!(host(2).unary(UnOp::Invert), Err(Error::Callback(_))));
    /// # }
    /// ```
    fn unary(&self, op: UnOp) -> Result<Value>;
    /// Python's `bool(self)`, which may raise.
    ///
    /// ```
    /// # include!("../../doctest/host_pip.rs");
    /// # fn main() {
    /// assert!(host(2).truthy().unwrap());
    /// assert!(!host(0).truthy().unwrap());
    /// # }
    /// ```
    fn truthy(&self) -> Result<bool>;
    /// Python's `repr(self)`.
    ///
    /// ```
    /// # include!("../../doctest/host_pip.rs");
    /// # fn main() {
    /// // `Value`'s `Display` is Python's `repr`.
    /// assert_eq!(host(2).to_string(), "Pip(2)");
    /// assert_eq!(Value::tuple(vec![host(1), Value::from(2)]).to_string(), "(Pip(1), 2)");
    /// # }
    /// ```
    fn repr(&self) -> String;
    /// For the host's own downcast back to its object.
    ///
    /// ```
    /// # include!("../../doctest/host_pip.rs");
    /// # fn main() {
    /// // The binding gets its own object back from the core's value.
    /// let Value::Host(h) = host(5) else { unreachable!() };
    /// assert_eq!(h.as_any().downcast_ref::<Pip>().unwrap().0, 5);
    /// # }
    /// ```
    fn as_any(&self) -> &dyn Any;
    /// Python's `PyFloat_AsDouble(self)`, as `math` functions convert their arguments.
    ///
    /// ```
    /// # include!("../../doctest/host_pip.rs");
    /// # fn main() {
    /// use icepool_core::value::builtins::{math_sqrt, to_float};
    /// assert_eq!(to_float(&host(2)).unwrap(), 2.0);
    /// // `math.sqrt(Pip(4))` converts its argument to a float.
    /// assert_eq!(math_sqrt(&host(4)).unwrap(), Value::Float(2.0));
    /// # }
    /// ```
    fn to_float(&self) -> Result<f64> {
        Err(Error::Unported("float() of a host object".into()))
    }
    /// `Fraction(self.numerator, self.denominator)` if `self` is a `numbers.Rational`, else `None`: what
    /// `Fraction(self, d)` needs.
    ///
    /// ```
    /// # include!("../../doctest/host_pip.rs");
    /// # fn main() {
    /// use dashu_int::IBig;
    /// use icepool_core::value::builtins::try_fraction;
    /// // `Pip` keeps the default, so `Fraction(Pip(1), 2)` is not ported. A host whose object is a `numbers.Rational`
    /// // answers `Some(fraction)` instead.
    /// assert!(matches!(try_fraction(&host(1), &IBig::from(2)), Err(Error::Unported(_))));
    /// # }
    /// ```
    fn as_rational(&self) -> Result<Option<RBig>> {
        Err(Error::Unported("Fraction() of a host object".into()))
    }
    /// `operator.index(self)` if `self` has `__index__` (`PyIndex_Check`), else `None`: how a native sequence
    /// takes it as an index.
    ///
    /// ```
    /// # include!("../../doctest/host_pip.rs");
    /// # fn main() {
    /// use icepool_core::value::one_hot;
    /// // `range(Pip(3))` takes `operator.index(Pip(3))`.
    /// assert_eq!(one_hot(&host(3)).unwrap().len(), 3);
    /// # }
    /// ```
    fn as_index(&self) -> Result<Option<dashu_int::IBig>> {
        Err(Error::Unported("a host object as an index".into()))
    }
    /// Python's `self[key]`.
    ///
    /// ```
    /// # include!("../../doctest/host_pip.rs");
    /// # fn main() {
    /// use icepool_core::value::builtins::{getitem, Index};
    /// // `Pip` keeps the default: `Pip(1)[0]` is not ported.
    /// assert!(matches!(getitem(&host(1), &Index::Item(Value::from(0))), Err(Error::Unported(_))));
    /// # }
    /// ```
    fn getitem(&self, _key: &Index) -> Result<Value> {
        Err(Error::Unported("indexing a host object".into()))
    }
    /// Python's `len(self)`.
    ///
    /// ```
    /// # include!("../../doctest/host_pip.rs");
    /// # fn main() {
    /// use icepool_core::value::builtins::py_len;
    /// // `Pip` keeps the default: `len(Pip(1))` is not ported.
    /// assert!(matches!(py_len(&host(1)), Err(Error::Unported(_))));
    /// # }
    /// ```
    fn try_len(&self) -> Result<usize> {
        Err(Error::Unported("len() of a host object".into()))
    }
    /// Python's `repr(self)`, which may raise (`repr` above is for messages and cannot).
    ///
    /// ```
    /// # include!("../../doctest/host_pip.rs");
    /// # fn main() {
    /// use icepool_core::value::pystr::py_repr;
    /// // The default is `repr`.
    /// assert_eq!(py_repr(&Value::tuple(vec![host(1)])).unwrap(), "(Pip(1),)");
    /// # }
    /// ```
    fn try_repr(&self) -> Result<String> {
        Ok(self.repr())
    }
    /// Python's `str(self)`.
    ///
    /// ```
    /// # include!("../../doctest/host_pip.rs");
    /// # fn main() {
    /// use icepool_core::value::pystr::py_str;
    /// assert_eq!(py_str(&host(3)).unwrap(), "3 pips");
    /// # }
    /// ```
    fn try_str(&self) -> Result<String> {
        Err(Error::Unported("str() of a host object".into()))
    }
    /// What `Population.common_outcome_length` asks of an outcome: `isinstance(self, Mapping)` first, then
    /// `isinstance(self, Sized)` and `len(self)`.
    ///
    /// ```
    /// # include!("../../doctest/host_pip.rs");
    /// # fn main() {
    /// use icepool_core::value::common_outcome_length;
    /// use icepool_core::{Die, Quantity};
    /// // `Pip`s are not `Sized`: a die of them has no common outcome length.
    /// let die = Die::from_dict(vec![(host(1), Quantity::ONE), (host(2), Quantity::ONE)]).unwrap();
    /// assert_eq!(common_outcome_length(&die).unwrap(), None);
    /// # }
    /// ```
    fn len_kind(&self) -> Result<LenKind> {
        Err(Error::Unported("the length of a host object".into()))
    }
}

/// An outcome's kind for `common_outcome_length`.
///
/// ```
/// use icepool_core::value::common_outcome_length;
/// use icepool_core::{Die, Quantity, Value};
/// // A tuple is `Sized`: every outcome of length 2 makes a common length of 2.
/// let pair = |a: i64| Value::tuple(vec![Value::from(a), Value::from(0)]);
/// let die = Die::from_dict(vec![(pair(1), Quantity::ONE), (pair(2), Quantity::ONE)]).unwrap();
/// assert_eq!(common_outcome_length(&die).unwrap(), Some(2));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LenKind {
    /// A `Mapping`: the population has no common outcome length.
    Mapping,
    /// A `Sized` object of this length.
    Len(usize),
    /// Neither: such outcomes are skipped.
    Unsized,
}

/// A shared reference to a host object.
///
/// ```
/// # include!("../../doctest/host_pip.rs");
/// # fn main() {
/// // A clone shares the object.
/// let a = HostRef::new(Pip(1));
/// let b = a.clone();
/// assert!(Arc::ptr_eq(&a.0, &b.0));
/// # }
/// ```
#[derive(Clone, Debug)]
pub struct HostRef(pub Arc<dyn HostObject>);

impl HostRef {
    /// Shares `obj`.
    ///
    /// ```
    /// # include!("../../doctest/host_pip.rs");
    /// # fn main() {
    /// // `HostRef` derefs to the object.
    /// let pip = HostRef::new(Pip(2));
    /// assert_eq!(pip.repr(), "Pip(2)");
    /// assert_eq!(Value::Host(pip), host(2));
    /// # }
    /// ```
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
///
/// ```
/// # include!("../../doctest/host_pip.rs");
/// # fn main() {
/// use icepool_core::{Die, Quantity};
/// // `~Pip(1)` raises in the host; the very same exception object comes back out of the die's operator.
/// let die = Die::from_dict(vec![(host(1), Quantity::ONE)]).unwrap();
/// let Err(Error::Callback(raised)) = die.unary_operator(|o| o.unary(UnOp::Invert)) else { panic!() };
/// assert_eq!(raised.0.downcast_ref::<&str>(), Some(&"bad operand type"));
/// # }
/// ```
#[derive(Clone, Debug)]
pub struct HostError(pub Arc<dyn Any + Send + Sync>);

impl PartialEq for HostError {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for HostError {}

/// The host object among two operands, if any (the left one first).
pub(crate) fn host_of<'a>(a: &'a Value, b: &'a Value) -> Option<&'a HostRef> {
    match (a, b) {
        (Value::Host(h), _) | (_, Value::Host(h)) => Some(h),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::creation_args::DieArg;
    use crate::outcome::OutcomeOrder;
    use crate::{Die, Quantity};

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
        fn hash_value(&self) -> i64 {
            self.0
        }
        fn eq(&self, a: &Value, b: &Value) -> bool {
            rev(a).is_some() && rev(a) == rev(b)
        }
        fn compare(&self, op: BinOp, a: &Value, b: &Value) -> Result<bool> {
            let (Some(x), Some(y)) = (rev(a), rev(b)) else {
                return Err(Error::WrongType("not comparable".into()));
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
                _ => Err(Error::WrongType("unsupported".into())),
            }
        }
        fn unary(&self, _op: UnOp) -> Result<Value> {
            Err(Error::Callback(HostError(Arc::new("raised by the host"))))
        }
        fn truthy(&self) -> Result<bool> {
            Ok(self.0 != 0)
        }
        fn repr(&self) -> String {
            format!("Rev({})", self.0)
        }
        fn as_any(&self) -> &dyn Any {
            self
        }
        fn as_index(&self) -> Result<Option<dashu_int::IBig>> {
            Ok(Some(self.0.into()))
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
        assert!(host(1).try_lt(&Value::from(1)).is_err());
        assert!(host(1).binary(BinOp::Add, &Value::from(1)).is_err());
        assert_ne!(host(1), Value::from(1));
        // Inside a tuple, the native tuple comparison reaches the host for the differing element.
        let t = |x| Value::tuple(vec![Value::from(0), host(x)]);
        assert_eq!(t(5).try_lt(&t(4)), Ok(true));
        // A host exception passes through unchanged.
        let err = d.unary_operator(|o| o.unary(UnOp::Neg)).unwrap_err();
        assert!(matches!(err, Error::Callback(_)));
        assert!(!host(0).truthy().unwrap() && host(2).truthy().unwrap());
    }

    #[test]
    fn host_objects_with_index_are_ints_to_range() {
        // `range(sides)` takes `operator.index(sides)`.
        assert_eq!(crate::value::function::one_hot(&host(3)).unwrap().len(), 3);
    }
}
