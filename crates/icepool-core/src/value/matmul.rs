//! `n @ die` on dice of `Value`s: `Die.__matmul__` with Python's count types (an integral float count makes its
//! factor a float; a non-integral one recurses until Python raises `RecursionError`). The typed core's
//! `Die::matmul` does the summing; this layer decides what Python would do with the counts. Both bindings use it
//! (the Python facade's `@`, the program runner's `matmul` node).

use dashu_int::IBig;
use rustc_hash::FxHashMap;

use crate::error::{Error, Result};
use crate::{Die, Quantity, Value};

/// `Die._sum_all(k)` recurses once per roll down to the largest sum already in the die's `_sum_cache`, so a big
/// enough step raises RecursionError (Python's limit is 1000 frames; at top level `Die([n]) @ d1` fails from about
/// n = 985). Recursing this deep always works, whatever the caller's stack...
const SAFE_RECURSION: u64 = 800;
/// ...and this deep never does.
const SURE_RECURSION_ERROR: u64 = 1000;

/// Whether Python's `_sum_all` would hit the recursion limit computing these counts in order, given the cache it
/// fills as it goes. A step between the two limits depends on the caller's stack, which the port cannot know, so
/// it is unimplemented. (The port assumes the right die's `_sum_cache` starts empty, but it lives on the die
/// object, and `icepool.d` caches its dice, so a Python process that summed `d(6)` before has it warm. Only a
/// recursion past the limit can tell the difference.)
fn check_sum_all_recursion(counts: &[(i64, Quantity)]) -> Result<()> {
    // The largest positive roll count cached so far (1 is a base case, cached for free).
    let mut cached = 1;
    for (count, _) in counts {
        let k = count.unsigned_abs();
        if k > cached {
            let depth = k - cached + u64::from(*count < 0);
            if depth >= SURE_RECURSION_ERROR {
                return Err(Error::RecursionLimit(
                    "maximum recursion depth exceeded in _sum_all".into(),
                ));
            }
            if depth > SAFE_RECURSION {
                return Err(Error::Unported(
                    "n @ die close to Python's recursion limit".into(),
                ));
            }
            cached = k;
        }
    }
    Ok(())
}

/// `n.__matmul__(die)` once both operands are dice (`Die.__matmul__` after `implicit_convert_to_die`). The
/// counts' Python types (float counts, Fractions) matter.
///
/// ```
/// use icepool_core::{Die, Quantity, Value};
/// let die = |outcomes: &[i64]| {
///     Die::from_dict(outcomes.iter().map(|&o| (Value::from(o), Quantity::ONE)).collect()).unwrap()
/// };
/// let sum = icepool_core::value::matmul_dice(&die(&[2]), &die(&[1, 2, 3, 4, 5, 6])).unwrap();
/// assert_eq!(sum.len(), 11); // 2d6: 2..=12
/// ```
pub fn matmul_dice(n: &Die<Value>, die: &Die<Value>) -> Result<Die<Value>> {
    // `max(abs(self.min_outcome()), abs(self.max_outcome()))`, then `_sum_all(die_count)` for each outcome.
    let mut counts = Vec::with_capacity(n.len());
    let mut kinds = Vec::with_capacity(n.len());
    for (outcome, quantity) in n.items() {
        let (count, kind) = die_count(outcome)?;
        counts.push((count, quantity.clone()));
        kinds.push(kind);
    }
    check_sum_all_recursion(&counts)?;
    if kinds.iter().all(|k| *k == CountKind::Int) {
        return Ok(die.matmul(&counts)?);
    }
    // Some count is a float (with an integral value: others raised above). `_sum_all` treats it as the equal
    // int, but the factor `den**(max_abs_die_count - abs(die_count))` is then a float, and so is every quantity it
    // scales. (A Fraction exponent keeps the factor an int: `int ** Fraction` with a non-negative integral
    // exponent is `int ** int`.) `Die(data)` drops a non-int quantity that is zero and raises for any
    // other, so the result is the int terms alone, unless a non-int term adds anything non-zero to an outcome.
    // `max` returns the first of two equal values: abs(min_outcome) unless abs(max_outcome) is greater.
    let (first, last) = (counts[0].0, counts[counts.len() - 1].0);
    let max_float = if last.unsigned_abs() > first.unsigned_abs() {
        kinds[kinds.len() - 1]
    } else {
        kinds[0]
    } == CountKind::Float;
    let max_abs = first.unsigned_abs().max(last.unsigned_abs());
    let denominator = die.denominator();
    let mut data: Vec<(Value, Quantity, bool)> = Vec::new();
    let mut index: FxHashMap<Value, usize> = FxHashMap::default();
    for ((term, kind), (count, _)) in die
        .matmul_terms(&counts)?
        .into_iter()
        .zip(&kinds)
        .zip(&counts)
    {
        let float_factor = max_float || *kind == CountKind::Float;
        if float_factor {
            // `int ** float` converts the int first, and overflows rather than giving inf.
            let exponent = (max_abs - count.unsigned_abs()) as i32;
            let base = crate::value::int_to_f64(&IBig::from(denominator.clone()))?;
            if !base.powi(exponent).is_finite() {
                return Err(Error::Overflow("int ** float: result too large".into()));
            }
        }
        for (outcome, quantity) in term {
            match index.get(&outcome) {
                Some(&i) => {
                    data[i].1 += quantity;
                    data[i].2 |= float_factor;
                }
                None => {
                    index.insert(outcome.clone(), data.len());
                    data.push((outcome, quantity, float_factor));
                }
            }
        }
    }
    let mut items = Vec::with_capacity(data.len());
    for (outcome, quantity, float) in data {
        if float && !quantity.is_zero() {
            return Err(Error::WrongType(
                "'float' object cannot be interpreted as an integer".into(),
            ));
        }
        items.push((outcome, quantity));
    }
    Ok(Die::from_dict(items)?)
}

/// Whether a count in `n @ die` is a float in Python, which makes its factor a float.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CountKind {
    Int,
    Float,
}

/// One outcome of the left die of `@` as a count of rolls.
fn die_count(outcome: &Value) -> Result<(i64, CountKind)> {
    let (int, kind) = match outcome {
        Value::Int(i) => (i.clone(), CountKind::Int),
        Value::Bool(b) => (IBig::from(*b as u8), CountKind::Int),
        // `_sum_all` subtracts 1 until it reaches 0 or 1, negating on the way below 0, so a count that is not
        // integral (or not finite) recurses until Python raises RecursionError.
        Value::Float(f) if f.is_finite() && f.fract() == 0.0 => (
            IBig::from_str_radix(&format!("{f:.0}"), 10).expect("an integral float"),
            CountKind::Float,
        ),
        Value::Fraction(r) if r.denominator().is_one() => (r.numerator().clone(), CountKind::Int),
        Value::Float(_) | Value::Fraction(_) => {
            return Err(Error::RecursionLimit(
                "maximum recursion depth exceeded in _sum_all".into(),
            ))
        }
        // Only the Python facade has these (an `IntEnum` count, a numpy int): Python's answer depends on the type.
        Value::Host(_) => {
            return Err(Error::Unported(
                "a count of a host type in n @ die".into(),
            ))
        }
        v => {
            return Err(Error::WrongType(format!(
                "bad operand type for abs(): '{}'",
                v.type_name()
            )))
        }
    };
    // Beyond i64, `_sum_all` recurses past Python's limit.
    let count = i64::try_from(int).map_err(|_| {
        Error::RecursionLimit("maximum recursion depth exceeded in _sum_all".into())
    })?;
    Ok((count, kind))
}