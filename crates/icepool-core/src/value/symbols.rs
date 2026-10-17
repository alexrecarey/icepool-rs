//! `Value::Symbols`: icepool's `Symbols` (collection/symbols.py), an immutable multiset of single characters.
//!
//! Python keeps the counts in a `defaultdict(int)`. The constructor sorts the keys; the operators build the result
//! with `_new_raw` from a copy of one operand's dict, so their keys stay in insertion order (unsorted), and a
//! count that reaches zero stays as a key (`len()` and `bool()` count it, `==`, `str()` and the hash do not).
//! Counts are `Value`s: they are ints unless someone builds a `Symbols` from a mapping of other numbers, and the
//! arithmetic on them is Python's.
//!
//! | Operation                   | Count / notes                      |
//! |:----------------------------|:-----------------------------------|
//! | `additive_union`, `+`       | `l + r`                            |
//! | `difference`, `-`           | `l - r`                            |
//! | `intersection`, `&`         | `min(l, r)`                        |
//! | `union`, `\|`               | `max(l, r)`                        |
//! | `symmetric_difference`, `^` | `abs(l - r)`                       |
//! | `multiply_counts`, `*`      | `count * n`                        |
//! | `divide_counts`, `//`       | `count // n`                       |
//! | `issubset`, `<=`            | all counts l <= r                  |
//! | `issuperset`, `>=`          | all counts l >= r                  |
//! | `==`                        | all counts l == r                  |
//! | `!=`                        | any count l != r                   |
//! | unary `+`                   | drop all negative counts           |
//! | unary `-`                   | reverses the sign of all counts    |
//!
//! `<` and `>` are lexicographic orderings rather than subset relations: they compare the count of each character
//! in alphabetical order. Binary operators other than `*` and `//` implicitly convert the other argument to
//! `Symbols` using the constructor.

use std::sync::Arc;

use super::{BinOp, UnOp, Value};
use crate::error::{type_error, Error, Result};

#[derive(Clone, Debug)]
pub struct Symbols {
    /// `self._data`, in the dict's insertion order.
    data: Arc<[(char, Value)]>,
}

fn int(i: i64) -> Value {
    Value::from(i)
}

/// Python's `str.isspace()` for one character, which is what `\s` matches in a `str` pattern: Unicode
/// whitespace, plus the information separators U+001C..U+001F (bidirectional class B/S).
fn py_isspace(c: char) -> bool {
    c.is_whitespace() || ('\x1c'..='\x1f').contains(&c)
}

/// A `defaultdict(int)` keyed by character, in insertion order.
#[derive(Default)]
struct Data(Vec<(char, Value)>);

impl Data {
    /// `data[c]`: the count, inserting `0` first if missing (defaultdict).
    fn entry(&mut self, c: char) -> &mut Value {
        let i = match self.0.iter().position(|(k, _)| *k == c) {
            Some(i) => i,
            None => {
                self.0.push((c, int(0)));
                self.0.len() - 1
            }
        };
        &mut self.0[i].1
    }

    /// `data[c] = f(data[c])`.
    fn update(&mut self, c: char, f: impl FnOnce(&Value) -> Result<Value>) -> Result<()> {
        let slot = self.entry(c);
        *slot = f(slot)?;
        Ok(())
    }

    fn done(self) -> Symbols {
        Symbols {
            data: Arc::from(self.0),
        }
    }
}

impl Symbols {
    /// `Symbols(symbols)` for a string. Negative symbols follow a minus sign, optionally surrounded by whitespace:
    /// `a - b` has one positive a and one negative b.
    pub fn parse(s: &str) -> Result<Symbols> {
        // re.split(r'\s*-\s*', symbols): whitespace next to a dash goes with it.
        let mut parts = s.split('-').collect::<Vec<_>>();
        let n = parts.len();
        for (i, p) in parts.iter_mut().enumerate() {
            if i > 0 {
                *p = p.trim_start_matches(py_isspace);
            }
            if i + 1 < n {
                *p = p.trim_end_matches(py_isspace);
            }
        }
        let mut data = Data::default();
        for c in parts[0].chars() {
            data.update(c, |x| x.binary(BinOp::Add, &int(1)))?;
        }
        if n > 2 {
            return Err(Error::Value("Multiple dashes not allowed.".into()));
        }
        if n == 2 {
            for c in parts[1].chars() {
                data.update(c, |x| x.binary(BinOp::Sub, &int(1)))?;
            }
        }
        Self::validated_sorted(data.0)
    }

    /// The end of the constructor: every key must be a legal symbol, and the keys are sorted.
    fn validated_sorted(mut items: Vec<(char, Value)>) -> Result<Symbols> {
        for (c, _) in &items {
            if py_isspace(*c) || *c == '_' || *c == '-' {
                return Err(Error::Value(format!(
                    "{c} (U+{:04X}) is not a legal symbol.",
                    *c as u32
                )));
            }
        }
        // Python sorts str keys by code point; the keys are distinct, so stability does not matter.
        items.sort_by_key(|(c, _)| *c);
        Ok(Symbols {
            data: Arc::from(items),
        })
    }

    /// `Symbols(x)` for any value: a string, a mapping (another `Symbols`), or an iterable of characters.
    pub fn convert(x: &Value) -> Result<Symbols> {
        match x {
            Value::Str(s) => Self::parse(s),
            Value::Symbols(s) => Self::validated_sorted(s.data.to_vec()),
            Value::Tuple(items) => Self::from_elements(items),
            Value::Vector(v) => Self::from_elements(v.data()?),
            Value::Host(_) => Err(Error::Unsupported("Symbols() of a host object".into())),
            _ => type_error(format!("'{}' object is not iterable", x.type_name())),
        }
    }

    /// The constructor's iterable branch: `data[s] += 1` for each element, then the keys are checked.
    pub fn from_elements(items: &[Value]) -> Result<Symbols> {
        Self::from_mapping(counter(items.iter().cloned())?)
    }

    /// The constructor's mapping branch (`defaultdict(int, symbols)`, which keeps the mapping's order): each key is
    /// checked to be a single character, in insertion order, so the first bad key decides the exception.
    pub fn from_mapping(items: Vec<(Value, Value)>) -> Result<Symbols> {
        let mut data = Vec::with_capacity(items.len());
        for (k, n) in items {
            let len = super::builtins::py_len(&k)?;
            let c = match k {
                Value::Str(s) if len == 1 => s.chars().next().expect("one character"),
                _ if len != 1 => {
                    return Err(Error::Value(format!(
                        "Symbol {} is not a single character.",
                        super::pystr::py_str(&k).unwrap_or_default()
                    )))
                }
                // `re.match` of a non-string.
                _ => return type_error("expected string or bytes-like object"),
            };
            data.push((c, n));
        }
        Self::validated_sorted(data)
    }

    /// `Symbols._new_raw` of these items, in this order.
    pub fn raw(items: Vec<(char, Value)>) -> Symbols {
        Symbols {
            data: Arc::from(items),
        }
    }

    pub fn items(&self) -> &[(char, Value)] {
        &self.data
    }

    /// `len(self)`: the number of keys, zero counts included.
    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// `self[c]` for a single character: `self._data.get(c, 0)`.
    pub fn count(&self, c: char) -> Value {
        self.data
            .iter()
            .find(|(k, _)| *k == c)
            .map_or_else(|| int(0), |(_, v)| v.clone())
    }

    /// `self[key]`: the count of a single character as an `int`; for any other string, a `Symbols` with only those
    /// characters (in the key's order, without sorting), dropping the rest.
    pub fn getitem(&self, key: &str) -> Value {
        let mut chars = key.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) => self.count(c),
            _ => {
                let mut data = Data::default();
                for c in key.chars() {
                    *data.entry(c) = self.count(c);
                }
                Value::Symbols(data.done())
            }
        }
    }

    fn copy(&self) -> Data {
        Data(self.data.to_vec())
    }

    /// `self + other`, `self - other`, `self & other`, `self | other`, `self ^ other` (the reflected forms of `+`,
    /// `&`, `|`, `^` are the same methods). `None` is `NotImplemented`.
    pub fn binary(&self, op: BinOp, other: &Value) -> Option<Result<Value>> {
        let elementwise = |init: Data, f: &dyn Fn(&Value, &Value) -> Result<Value>| {
            let mut data = init;
            for (c, count) in Symbols::convert(other)?.items() {
                data.update(*c, |x| f(x, count))?;
            }
            Ok(Value::Symbols(data.done()))
        };
        Some(match op {
            BinOp::Add => elementwise(self.copy(), &|x, n| x.binary(BinOp::Add, n)),
            BinOp::Sub => elementwise(self.copy(), &|x, n| x.binary(BinOp::Sub, n)),
            // `data[s] = min(self.get(s, 0), count)`: only the other's keys are kept.
            BinOp::And => (|| {
                let mut data = Data::default();
                for (c, count) in Symbols::convert(other)?.items() {
                    *data.entry(*c) = Value::py_min(self.count(*c), count.clone())?;
                }
                Ok(Value::Symbols(data.done()))
            })(),
            BinOp::Or => elementwise(self.copy(), &|x, n| Value::py_max(x.clone(), n.clone())),
            BinOp::Xor => elementwise(self.copy(), &|x, n| {
                x.binary(BinOp::Sub, n)?.unary(UnOp::Abs)
            }),
            BinOp::Mul | BinOp::Mod => {
                if !is_int(other) {
                    return None;
                }
                self.map_counts(|n| n.binary(op, other))
            }
            BinOp::FloorDiv => match other {
                _ if is_int(other) => self.map_counts(|n| n.binary(BinOp::FloorDiv, other)),
                // Iterables: `count_subset`.
                Value::Str(_) | Value::Tuple(_) | Value::Vector(_) | Value::Symbols(_) => {
                    self.count_subset(other, None)
                }
                Value::Host(_) => Err(Error::Unsupported("Symbols // a host object".into())),
                _ => return None,
            },
            BinOp::Lt | BinOp::Gt => {
                let Value::Symbols(o) = other else {
                    return None;
                };
                self.lexicographic(op, o).map(Value::Bool)
            }
            BinOp::Le | BinOp::Ge => (|| {
                let o = Symbols::convert(other)?;
                Ok(Value::Bool(self.all_counts(&o, op)?))
            })(),
            BinOp::TrueDiv | BinOp::Pow | BinOp::LShift | BinOp::RShift => return None,
        })
    }

    /// `other <op> self` where `other` is not a `Symbols` and declined. `None` is `NotImplemented`.
    pub fn reverse_binary(&self, op: BinOp, other: &Value) -> Option<Result<Value>> {
        match op {
            // __radd__ = __add__, __rand__ = __and__, ... (the result starts from self's keys).
            BinOp::Add | BinOp::And | BinOp::Or | BinOp::Xor | BinOp::Mul => self.binary(op, other),
            BinOp::Sub => Some((|| {
                let mut data = Symbols::convert(other)?.copy();
                for (c, count) in self.items() {
                    data.update(*c, |x| x.binary(BinOp::Sub, count))?;
                }
                Ok(Value::Symbols(data.done()))
            })()),
            BinOp::FloorDiv => Some(
                Symbols::convert(other)
                    .and_then(|o| o.count_subset(&Value::Symbols(self.clone()), None)),
            ),
            // The comparisons reflect to the swapped operator, which `ops` asks for directly.
            _ => None,
        }
    }

    /// `Symbols(...)` of every count mapped.
    pub fn map_counts(&self, f: impl Fn(&Value) -> Result<Value>) -> Result<Value> {
        let items = self
            .items()
            .iter()
            .map(|(c, n)| Ok((*c, f(n)?)))
            .collect::<Result<Vec<_>>>()?;
        Ok(Value::Symbols(Symbols::raw(items)))
    }

    /// `__lt__` / `__gt__`: the counts of each character in alphabetical order; the first difference decides.
    pub fn lexicographic(&self, op: BinOp, other: &Symbols) -> Result<bool> {
        let mut keys: Vec<char> = self
            .items()
            .iter()
            .chain(other.items())
            .map(|(c, _)| *c)
            .collect();
        keys.sort_unstable();
        keys.dedup();
        let rev = if op == BinOp::Lt {
            BinOp::Gt
        } else {
            BinOp::Lt
        };
        for k in keys {
            let (a, b) = (self.count(k), other.count(k));
            if a.binary(op, &b)?.truthy()? {
                return Ok(true);
            }
            if a.binary(rev, &b)?.truthy()? {
                return Ok(false);
            }
        }
        Ok(false)
    }

    /// `all(self[s] <op> other[s] for s in itertools.chain(self, other))`.
    pub fn all_counts(&self, other: &Symbols, op: BinOp) -> Result<bool> {
        for (k, _) in self.items().iter().chain(other.items()) {
            if !self.count(*k).binary(op, &other.count(*k))?.truthy()? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// `self == other` (`!=` if `ne`); `None` is `NotImplemented`, which Python returns when `Symbols(other)` raises
    /// `TypeError` or `ValueError`.
    pub fn eq_op(&self, other: &Value, ne: bool) -> Option<Result<bool>> {
        let o = match Symbols::convert(other) {
            Ok(o) => o,
            Err(Error::Type(_) | Error::Value(_)) => return None,
            Err(e) => return Some(Err(e)),
        };
        // `all(self[s] == other[s] ...)` for `==`, `any(self[s] != other[s] ...)` for `!=`.
        let differ = self
            .items()
            .iter()
            .chain(o.items())
            .any(|(k, _)| self.count(*k) != o.count(*k));
        Some(Ok(differ == ne))
    }

    /// `count_subset(divisor, empty_divisor=...)`: the number of times the divisor is contained in this multiset.
    pub fn count_subset(&self, divisor: &Value, empty_divisor: Option<&Value>) -> Result<Value> {
        // A non-mapping divisor is counted with `Counter`, which takes any hashable elements.
        let counted: Vec<(Value, Value)> = match divisor {
            Value::Symbols(s) => s
                .items()
                .iter()
                .map(|(c, n)| (Value::str(&c.to_string()), n.clone()))
                .collect(),
            Value::Str(s) => counter(s.chars().map(|c| Value::str(&c.to_string())))?,
            Value::Tuple(t) => counter(t.iter().cloned())?,
            Value::Vector(v) => counter(v.data()?.iter().cloned())?,
            Value::Host(_) => {
                return Err(Error::Unsupported("count_subset of a host object".into()))
            }
            _ => return type_error(format!("'{}' object is not iterable", divisor.type_name())),
        };
        self.count_subset_items(counted, empty_divisor)
    }

    /// `count_subset` of a mapping divisor given as its items (e.g. a die: `symbols // die` is `count_subset`).
    pub fn count_subset_items(
        &self,
        counted: Vec<(Value, Value)>,
        empty_divisor: Option<&Value>,
    ) -> Result<Value> {
        let mut result: Option<Value> = None;
        for (s, count) in counted {
            // `self._data.get(s, 0)`: only a one-character str can be a key.
            let mine = match &s {
                Value::Str(k) if k.chars().count() == 1 => {
                    self.count(k.chars().next().expect("one character"))
                }
                _ => int(0),
            };
            let current = mine.binary(BinOp::FloorDiv, &count)?;
            result = match result {
                Some(r) if !current.binary(BinOp::Lt, &r)?.truthy()? => Some(r),
                _ => Some(current),
            };
        }
        match (result, empty_divisor) {
            (Some(r), _) => Ok(r),
            (None, Some(e)) if !matches!(e, Value::None) => Ok(e.clone()),
            (None, _) => Err(Error::ZeroDivision("Divisor is empty.".into())),
        }
    }

    /// `divide_counts(other)`: every count `// other`, for any `other` (the method does not check it is an int).
    pub fn divide_counts(&self, other: &Value) -> Result<Value> {
        self.map_counts(|n| n.binary(BinOp::FloorDiv, other))
    }

    /// Unary `+` (drop the counts that are not positive) and `-`; other unary operators are not defined.
    pub fn unary(&self, op: UnOp) -> Option<Result<Value>> {
        Some(match op {
            UnOp::Pos => (|| {
                let mut items = Vec::new();
                for (c, n) in self.items() {
                    if n.binary(BinOp::Gt, &int(0))?.truthy()? {
                        items.push((*c, n.clone()));
                    }
                }
                Ok(Value::Symbols(Symbols::raw(items)))
            })(),
            UnOp::Neg => self.map_counts(|n| n.unary(UnOp::Neg)),
            _ => return None,
        })
    }

    /// `has_negative_counts()`.
    pub fn has_negative_counts(&self) -> Result<bool> {
        for (_, n) in self.items() {
            if n.binary(BinOp::Lt, &int(0))?.truthy()? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// `size()`: the total number of elements, `sum(self._data.values())`.
    pub fn size(&self) -> Result<Value> {
        super::builtins::py_sum(self.items().iter().map(|(_, n)| Ok(n.clone())), int(0))
    }

    /// `isdisjoint(other)`: whether any positive elements are in common.
    pub fn isdisjoint(&self, other: &Value) -> Result<bool> {
        let o = Symbols::convert(other)?;
        for (c, n) in self.items() {
            if n.binary(BinOp::Gt, &int(0))?.truthy()?
                && o.count(*c).binary(BinOp::Gt, &int(0))?.truthy()?
            {
                // Python's docstring says this tests disjointness; the code returns whether they *intersect*.
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// `str(self)`: all symbols in unary form (i.e. including duplicates) in ascending order; negative elements are
    /// listed following a ` - ` sign.
    pub fn py_str(&self) -> Result<String> {
        let mut keys: Vec<&(char, Value)> = self.items().iter().collect();
        keys.sort_by_key(|(c, _)| *c);
        let (mut positive, mut negative) = (String::new(), String::new());
        for (c, n) in &keys {
            if n.binary(BinOp::Gt, &int(0))?.truthy()? {
                positive.push_str(&repeat(*c, n)?);
            }
        }
        for (c, n) in &keys {
            if n.binary(BinOp::Lt, &int(0))?.truthy()? {
                negative.push_str(&repeat(*c, &n.unary(UnOp::Neg)?)?);
            }
        }
        Ok(match (positive.is_empty(), negative.is_empty()) {
            (false, false) => format!("{positive} - {negative}"),
            (false, true) => positive,
            (true, false) => format!("-{negative}"),
            (true, true) => String::new(),
        })
    }

    /// `repr(self)`: `Symbols('<str(self)>')`, the quotes not escaped.
    pub fn py_repr(&self) -> Result<String> {
        Ok(format!("Symbols('{}')", self.py_str()?))
    }

    /// The nonzero counts in key order: what `==` and the hash depend on.
    pub fn nonzero_sorted(&self) -> Vec<(char, &Value)> {
        let mut v: Vec<(char, &Value)> = self
            .items()
            .iter()
            .filter(|(_, n)| *n != int(0))
            .map(|(c, n)| (*c, n))
            .collect();
        v.sort_by_key(|(c, _)| *c);
        v
    }
}

/// `s * n` for a count.
fn repeat(c: char, n: &Value) -> Result<String> {
    match Value::str(&c.to_string()).binary(BinOp::Mul, n)? {
        Value::Str(s) => Ok(s.to_string()),
        _ => unreachable!("str * int is a str"),
    }
}

/// `isinstance(x, int)` (bool included). A host object might be an int subclass: not ported.
fn is_int(x: &Value) -> bool {
    matches!(x, Value::Int(_) | Value::Bool(_))
}

/// `Counter(iterable).items()`: first-occurrence order.
fn counter(items: impl Iterator<Item = Value>) -> Result<Vec<(Value, Value)>> {
    let mut out: Vec<(Value, i64)> = Vec::new();
    for x in items {
        x.check_hashable()?;
        match out.iter_mut().find(|(k, _)| *k == x) {
            Some((_, n)) => *n += 1,
            None => out.push((x, 1)),
        }
    }
    Ok(out.into_iter().map(|(k, n)| (k, int(n))).collect())
}

/// `==` as a dict sees it: the same counts for every character (missing counts are 0).
impl crate::ownership::Traverse for Symbols {
    fn traverse<'a>(&'a self, owners: &mut crate::ownership::Owners<'a>) {
        owners.arc_with(&self.data, |owners, items| {
            for (_, n) in items {
                n.traverse(owners);
            }
        });
    }
}

impl PartialEq for Symbols {
    fn eq(&self, other: &Self) -> bool {
        self.items()
            .iter()
            .chain(other.items())
            .all(|(c, _)| self.count(*c) == other.count(*c))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(x: &str) -> Value {
        Value::Symbols(Symbols::parse(x).unwrap())
    }

    fn keys(v: &Value) -> String {
        match v {
            Value::Symbols(s) => s.items().iter().map(|(c, _)| *c).collect(),
            _ => panic!("not a Symbols"),
        }
    }

    #[test]
    fn parses_sorts_and_keeps_operator_order() {
        assert_eq!(s("ba - c").to_string(), "Symbols('ab - c')");
        assert!(Symbols::parse("a-b-c").is_err() && Symbols::parse("a b").is_err());
        // The constructor sorts; `+` keeps the left operand's keys first.
        let sum = s("b").binary(BinOp::Add, &Value::str("a")).unwrap();
        assert_eq!(keys(&sum), "ba");
        // A count reaching zero stays a key: truthy, but equal to the empty Symbols.
        let zero = s("a").binary(BinOp::Sub, &Value::str("a")).unwrap();
        assert!(zero.truthy().unwrap());
        assert_eq!(zero, s(""));
        assert!(!s("").truthy().unwrap());
    }

    #[test]
    fn comparisons() {
        // Lexicographic `<`, subset `<=`, and `==` with a convertible str.
        assert_eq!(s("a").binary(BinOp::Gt, &s("bb")), Ok(Value::Bool(true)));
        assert_eq!(
            s("a").binary(BinOp::Le, &Value::str("ab")),
            Ok(Value::Bool(true))
        );
        assert!(s("a").binary(BinOp::Lt, &Value::str("a")).is_err());
        assert_eq!(
            s("ab").eq_op(&Value::str("ba"), false),
            Ok(Value::Bool(true))
        );
        assert_eq!(
            s("ab").eq_op(&Value::from(1), false),
            Ok(Value::Bool(false))
        );
        assert_ne!(s("ab"), Value::str("ab"));
    }
}
