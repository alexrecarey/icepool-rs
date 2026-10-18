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
use crate::error::{wrong_type, Error, Result};

/// icepool's `Symbols`: a multiset of single-character symbols, with its arithmetic.
///
/// ```
/// use icepool_core::value::{BinOp, Symbols};
/// use icepool_core::Value;
///
/// // `Symbols('aab')`: two `a`s and a `b`; `+` adds counts, converting the other side.
/// let s = Symbols::parse("aab").unwrap();
/// assert_eq!(s.count('a'), Value::from(2));
/// assert_eq!(s.py_str().unwrap(), "aab");
/// let sum = s.binary(BinOp::Add, &Value::str("b")).unwrap().unwrap();
/// assert_eq!(sum.to_string(), "Symbols('aabb')");
/// ```
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
    ///
    /// ```
    /// use icepool_core::value::Symbols;
    /// use icepool_core::Value;
    ///
    /// assert_eq!(Value::Symbols(Symbols::parse("ab - c").unwrap()).to_string(), "Symbols('ab - c')");
    /// // `Symbols('a-b-c')` and `Symbols('a_')` raise ValueError.
    /// assert!(Symbols::parse("a-b-c").is_err());
    /// assert!(Symbols::parse("a_").is_err());
    /// ```
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
            return Err(Error::InvalidArgument(
                "Multiple dashes not allowed.".into(),
            ));
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
                return Err(Error::InvalidArgument(format!(
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
    ///
    /// ```
    /// use icepool_core::value::Symbols;
    /// use icepool_core::Value;
    ///
    /// // `Symbols(('b', 'a', 'b'))` is `Symbols('abb')`; `Symbols(3)` raises TypeError.
    /// let t = Value::tuple(vec![Value::str("b"), Value::str("a"), Value::str("b")]);
    /// assert_eq!(Symbols::convert(&t).unwrap().py_str().unwrap(), "abb");
    /// assert!(Symbols::convert(&Value::from(3)).is_err());
    /// ```
    pub fn convert(x: &Value) -> Result<Symbols> {
        match x {
            Value::Str(s) => Self::parse(s),
            Value::Symbols(s) => Self::validated_sorted(s.data.to_vec()),
            Value::Tuple(items) => Self::from_elements(items),
            Value::Vector(v) => Self::from_elements(v.data()?),
            Value::Host(_) => Err(Error::Unported("Symbols() of a host object".into())),
            _ => wrong_type(format!("'{}' object is not iterable", x.type_name())),
        }
    }

    /// The constructor's iterable branch: `data[s] += 1` for each element, then the keys are checked.
    ///
    /// ```
    /// use icepool_core::value::Symbols;
    /// use icepool_core::Value;
    ///
    /// // The keys are sorted: `Symbols(['b', 'a', 'b'])` holds `a` first.
    /// let s = Symbols::from_elements(&[Value::str("b"), Value::str("a"), Value::str("b")]).unwrap();
    /// assert_eq!(s.items(), [('a', Value::from(1)), ('b', Value::from(2))]);
    /// ```
    pub fn from_elements(items: &[Value]) -> Result<Symbols> {
        Self::from_mapping(counter(items.iter().cloned())?)
    }

    /// The constructor's mapping branch (`defaultdict(int, symbols)`, which keeps the mapping's order): each key is
    /// checked to be a single character, in insertion order, so the first bad key decides the exception.
    ///
    /// ```
    /// use icepool_core::value::Symbols;
    /// use icepool_core::Value;
    ///
    /// // `Symbols({'b': 2, 'a': 1})` is `Symbols('abb')`; `Symbols({'ab': 1})` raises ValueError.
    /// let mapping = vec![(Value::str("b"), Value::from(2)), (Value::str("a"), Value::from(1))];
    /// let s = Symbols::from_mapping(mapping).unwrap();
    /// assert_eq!(s.py_str().unwrap(), "abb");
    /// assert!(Symbols::from_mapping(vec![(Value::str("ab"), Value::from(1))]).is_err());
    /// ```
    pub fn from_mapping(items: Vec<(Value, Value)>) -> Result<Symbols> {
        let mut data = Vec::with_capacity(items.len());
        for (k, n) in items {
            let len = super::builtins::py_len(&k)?;
            let c = match k {
                Value::Str(s) if len == 1 => s.chars().next().expect("one character"),
                _ if len != 1 => {
                    return Err(Error::InvalidArgument(format!(
                        "Symbol {} is not a single character.",
                        super::pystr::py_str(&k).unwrap_or_default()
                    )))
                }
                // `re.match` of a non-string.
                _ => return wrong_type("expected string or bytes-like object"),
            };
            data.push((c, n));
        }
        Self::validated_sorted(data)
    }

    /// `Symbols._new_raw` of these items, in this order.
    ///
    /// ```
    /// use icepool_core::value::Symbols;
    /// use icepool_core::Value;
    ///
    /// // Unlike the constructor, the order is kept as given.
    /// let s = Symbols::raw(vec![('b', Value::from(1)), ('a', Value::from(2))]);
    /// assert_eq!(s.items()[0].0, 'b');
    /// assert_eq!(s.py_str().unwrap(), "aab");
    /// ```
    pub fn raw(items: Vec<(char, Value)>) -> Symbols {
        Symbols {
            data: Arc::from(items),
        }
    }

    /// Every `(symbol, count)` in insertion order.
    ///
    /// ```
    /// use icepool_core::value::Symbols;
    /// use icepool_core::Value;
    ///
    /// let s = Symbols::parse("aab").unwrap();
    /// assert_eq!(s.items(), [('a', Value::from(2)), ('b', Value::from(1))]);
    /// ```
    pub fn items(&self) -> &[(char, Value)] {
        &self.data
    }

    /// `len(self)`: the number of keys, zero counts included.
    ///
    /// ```
    /// use icepool_core::value::{BinOp, Symbols};
    /// use icepool_core::Value;
    ///
    /// // `len(Symbols('aab') - 'aab')` is 2: both keys stay, with zero counts.
    /// let s = Symbols::parse("aab").unwrap();
    /// let Some(Ok(Value::Symbols(diff))) = s.binary(BinOp::Sub, &Value::str("aab")) else { unreachable!() };
    /// assert_eq!((s.len(), diff.len()), (2, 2));
    /// ```
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Whether there are no symbols.
    ///
    /// ```
    /// use icepool_core::value::Symbols;
    /// use icepool_core::Value;
    ///
    /// assert!(Symbols::parse("").unwrap().is_empty());
    /// ```
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// `self[c]` for a single character: `self._data.get(c, 0)`.
    ///
    /// ```
    /// use icepool_core::value::Symbols;
    /// use icepool_core::Value;
    ///
    /// let s = Symbols::parse("aab").unwrap();
    /// assert_eq!(s.count('a'), Value::from(2));
    /// assert_eq!(s.count('z'), Value::from(0));
    /// ```
    pub fn count(&self, c: char) -> Value {
        self.data
            .iter()
            .find(|(k, _)| *k == c)
            .map_or_else(|| int(0), |(_, v)| v.clone())
    }

    /// `self[key]`: the count of a single character as an `int`; for any other string, a `Symbols` with only those
    /// characters (in the key's order, without sorting), dropping the rest.
    ///
    /// ```
    /// use icepool_core::value::Symbols;
    /// use icepool_core::Value;
    ///
    /// let s = Symbols::parse("aab").unwrap();
    /// // `s['a']` is 2; `s['ca']` is `Symbols('aa')`, keeping `c` with count 0.
    /// assert_eq!(s.getitem("a"), Value::from(2));
    /// let Value::Symbols(sub) = s.getitem("ca") else { unreachable!() };
    /// assert_eq!(sub.items(), [('c', Value::from(0)), ('a', Value::from(2))]);
    /// assert_eq!(sub.py_str().unwrap(), "aa");
    /// ```
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
    ///
    /// ```
    /// use icepool_core::value::{BinOp, Symbols};
    /// use icepool_core::Value;
    ///
    /// let s = Symbols::parse("aab").unwrap();
    /// let op = |op, other: &str| s.binary(op, &Value::str(other)).unwrap().unwrap().to_string();
    /// // `s - 'abc'`, `s & 'bc'`, `s | 'bbb'`, `s ^ 'abbb'`, `s // 'ab'`.
    /// assert_eq!(op(BinOp::Sub, "abc"), "Symbols('a - c')");
    /// assert_eq!(op(BinOp::And, "bc"), "Symbols('b')");
    /// assert_eq!(op(BinOp::Or, "bbb"), "Symbols('aabbb')");
    /// assert_eq!(op(BinOp::Xor, "abbb"), "Symbols('abb')");
    /// assert_eq!(op(BinOp::FloorDiv, "ab"), "1");
    /// // `s * 2` repeats; `s * 'a'` is `NotImplemented`.
    /// assert_eq!(s.binary(BinOp::Mul, &Value::from(2)).unwrap().unwrap().to_string(), "Symbols('aaaabb')");
    /// assert!(s.binary(BinOp::Mul, &Value::str("a")).is_none());
    /// ```
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
                Value::Host(_) => Err(Error::Unported("Symbols // a host object".into())),
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
    ///
    /// ```
    /// use icepool_core::value::{BinOp, Symbols};
    /// use icepool_core::Value;
    ///
    /// let s = Symbols::parse("aab").unwrap();
    /// // `'abc' - s` is `Symbols('c - a')`; `'aaaabb' // s` is 2.
    /// assert_eq!(s.reverse_binary(BinOp::Sub, &Value::str("abc")).unwrap().unwrap().to_string(), "Symbols('c - a')");
    /// assert_eq!(s.reverse_binary(BinOp::FloorDiv, &Value::str("aaaabb")).unwrap().unwrap(), Value::from(2));
    /// ```
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
    ///
    /// ```
    /// use icepool_core::value::{BinOp, Symbols};
    /// use icepool_core::Value;
    ///
    /// let s = Symbols::parse("aab").unwrap();
    /// // `s.multiply_counts(3)`.
    /// let tripled = s.map_counts(|n| n.binary(BinOp::Mul, &Value::from(3))).unwrap();
    /// assert_eq!(tripled.to_string(), "Symbols('aaaaaabbb')");
    /// ```
    pub fn map_counts(&self, f: impl Fn(&Value) -> Result<Value>) -> Result<Value> {
        let items = self
            .items()
            .iter()
            .map(|(c, n)| Ok((*c, f(n)?)))
            .collect::<Result<Vec<_>>>()?;
        Ok(Value::Symbols(Symbols::raw(items)))
    }

    /// `__lt__` / `__gt__`: the counts of each character in alphabetical order; the first difference decides.
    ///
    /// ```
    /// use icepool_core::value::{BinOp, Symbols};
    /// use icepool_core::Value;
    ///
    /// // `Symbols('ab') < Symbols('b')` is `False`: `a` comes first, and 1 > 0 decides.
    /// let (ab, b) = (Symbols::parse("ab").unwrap(), Symbols::parse("b").unwrap());
    /// assert!(!ab.lexicographic(BinOp::Lt, &b).unwrap());
    /// assert!(b.lexicographic(BinOp::Lt, &ab).unwrap());
    /// ```
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
    ///
    /// ```
    /// use icepool_core::value::{BinOp, Symbols};
    /// use icepool_core::Value;
    ///
    /// let s = Symbols::parse("aab").unwrap();
    /// // `s <= 'aabc'`: a subset.
    /// assert!(s.all_counts(&Symbols::parse("aabc").unwrap(), BinOp::Le).unwrap());
    /// assert!(!s.all_counts(&Symbols::parse("ab").unwrap(), BinOp::Le).unwrap());
    /// ```
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
    ///
    /// ```
    /// use icepool_core::value::Symbols;
    /// use icepool_core::Value;
    ///
    /// let s = Symbols::parse("aab").unwrap();
    /// // `s == 'aba'` is `True`; `s == 3` is `NotImplemented` (then `False` by identity).
    /// assert!(s.eq_op(&Value::str("aba"), false).unwrap().unwrap());
    /// assert!(s.eq_op(&Value::from(3), false).is_none());
    /// ```
    pub fn eq_op(&self, other: &Value, ne: bool) -> Option<Result<bool>> {
        let o = match Symbols::convert(other) {
            Ok(o) => o,
            Err(e) if e.is_wrong_kind() || matches!(e, Error::InvalidArgument(_)) => return None,
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
    ///
    /// ```
    /// use icepool_core::value::Symbols;
    /// use icepool_core::Value;
    ///
    /// let s = Symbols::parse("aab").unwrap();
    /// // `s.count_subset('ab')` is 1; `s.count_subset('', empty_divisor=7)` is 7; `s.count_subset('')` raises
    /// // ZeroDivisionError.
    /// assert_eq!(s.count_subset(&Value::str("ab"), None).unwrap(), Value::from(1));
    /// assert_eq!(s.count_subset(&Value::str(""), Some(&Value::from(7))).unwrap(), Value::from(7));
    /// assert!(s.count_subset(&Value::str(""), None).is_err());
    /// ```
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
            Value::Host(_) => return Err(Error::Unported("count_subset of a host object".into())),
            _ => return wrong_type(format!("'{}' object is not iterable", divisor.type_name())),
        };
        self.count_subset_items(counted, empty_divisor)
    }

    /// `count_subset` of a mapping divisor given as its items (e.g. a die: `symbols // die` is `count_subset`).
    ///
    /// ```
    /// use icepool_core::value::Symbols;
    /// use icepool_core::Value;
    ///
    /// let s = Symbols::parse("aab").unwrap();
    /// // `s.count_subset({'a': 1})`: two `a`s hold one each.
    /// let counted = vec![(Value::str("a"), Value::from(1))];
    /// assert_eq!(s.count_subset_items(counted, None).unwrap(), Value::from(2));
    /// ```
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
            (None, _) => Err(Error::DivisionByZero("Divisor is empty.".into())),
        }
    }

    /// `divide_counts(other)`: every count `// other`, for any `other` (the method does not check it is an int).
    ///
    /// ```
    /// use icepool_core::value::Symbols;
    /// use icepool_core::Value;
    ///
    /// let s = Symbols::parse("aab").unwrap();
    /// // `s.divide_counts(2)`: `b`'s count becomes 0.
    /// assert_eq!(s.divide_counts(&Value::from(2)).unwrap().to_string(), "Symbols('a')");
    /// ```
    pub fn divide_counts(&self, other: &Value) -> Result<Value> {
        self.map_counts(|n| n.binary(BinOp::FloorDiv, other))
    }

    /// Unary `+` (drop the counts that are not positive) and `-`; other unary operators are not defined.
    ///
    /// ```
    /// use icepool_core::value::{Symbols, UnOp};
    /// use icepool_core::Value;
    ///
    /// // `+Symbols('a - b')` is `Symbols('a')`; `-Symbols('aab')` is `Symbols('-aab')`; `~` is not defined.
    /// let s = Symbols::parse("a - b").unwrap();
    /// assert_eq!(s.unary(UnOp::Pos).unwrap().unwrap().to_string(), "Symbols('a')");
    /// assert_eq!(Symbols::parse("aab").unwrap().unary(UnOp::Neg).unwrap().unwrap().to_string(), "Symbols('-aab')");
    /// assert!(s.unary(UnOp::Invert).is_none());
    /// ```
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
    ///
    /// ```
    /// use icepool_core::value::Symbols;
    /// use icepool_core::Value;
    ///
    /// assert!(Symbols::parse("a-b").unwrap().has_negative_counts().unwrap());
    /// assert!(!Symbols::parse("ab").unwrap().has_negative_counts().unwrap());
    /// ```
    pub fn has_negative_counts(&self) -> Result<bool> {
        for (_, n) in self.items() {
            if n.binary(BinOp::Lt, &int(0))?.truthy()? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// `size()`: the total number of elements, `sum(self._data.values())`.
    ///
    /// ```
    /// use icepool_core::value::Symbols;
    /// use icepool_core::Value;
    ///
    /// // `Symbols('aab').size()` is 3; `Symbols('a-b').size()` is 0.
    /// assert_eq!(Symbols::parse("aab").unwrap().size().unwrap(), Value::from(3));
    /// assert_eq!(Symbols::parse("a-b").unwrap().size().unwrap(), Value::from(0));
    /// ```
    pub fn size(&self) -> Result<Value> {
        super::builtins::py_sum(self.items().iter().map(|(_, n)| Ok(n.clone())), int(0))
    }

    /// `isdisjoint(other)`: whether any positive elements are in common.
    ///
    /// ```
    /// use icepool_core::value::Symbols;
    /// use icepool_core::Value;
    ///
    /// let s = Symbols::parse("aab").unwrap();
    /// // As in icepool: `True` when they share a positive element.
    /// assert!(s.isdisjoint(&Value::str("bc")).unwrap());
    /// assert!(!s.isdisjoint(&Value::str("c")).unwrap());
    /// ```
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
    ///
    /// ```
    /// use icepool_core::value::Symbols;
    /// use icepool_core::Value;
    ///
    /// assert_eq!(Symbols::parse("ab - c").unwrap().py_str().unwrap(), "ab - c");
    /// assert_eq!(Symbols::parse("-c").unwrap().py_str().unwrap(), "-c");
    /// ```
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
    ///
    /// ```
    /// use icepool_core::value::Symbols;
    /// use icepool_core::Value;
    ///
    /// assert_eq!(Symbols::parse("ba").unwrap().py_repr().unwrap(), "Symbols('ab')");
    /// ```
    pub fn py_repr(&self) -> Result<String> {
        Ok(format!("Symbols('{}')", self.py_str()?))
    }

    /// The nonzero counts in key order: what `==` and the hash depend on.
    ///
    /// ```
    /// use icepool_core::value::{BinOp, Symbols};
    /// use icepool_core::Value;
    ///
    /// // `Symbols('aab') - 'b'` keeps `b` with count 0, which `==` and the hash ignore.
    /// let Some(Ok(Value::Symbols(s))) = Symbols::parse("aab").unwrap().binary(BinOp::Sub, &Value::str("b")) else {
    ///     unreachable!()
    /// };
    /// assert_eq!(s.nonzero_sorted(), [('a', &Value::from(2))]);
    /// ```
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
