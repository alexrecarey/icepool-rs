//! `icepool_rs`: the Rust core as a Python extension, for the facade in python/icepool (see its `backend.py`).
//!
//! The facade keeps Python's argument juggling and every dunder trick (`itemize`, `Again`, `DieWithTruth`...) and
//! calls in here for the work: a `Die` built in Rust is held by the facade's `Die` as `_rs`, and its items become
//! the facade's `Counts`.

mod again;
mod collection;
mod convert;
mod deck;
mod evaluate;
mod expression;
mod format;
mod function;
mod map;
mod map_tools;
mod multi_deal;
mod ops;
mod pool;
mod stats;

use dashu_int::{IBig, UBig};
use icepool_core::program::PErr;
use icepool_core::{Die as CoreDie, DieArg, Error, Quantity, Value};
use pyo3::exceptions::{
    PyNotImplementedError, PyOverflowError, PyTypeError, PyValueError, PyZeroDivisionError,
};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyFloat, PyInt, PyList, PyTuple};

use convert::{from_py, int_from_py, quantity_to_py, to_py, to_py_err};

/// A die of the Rust core.
#[pyclass(frozen, name = "Die", module = "icepool_rs")]
pub struct RsDie {
    die: CoreDie<Value>,
}

#[pymethods]
impl RsDie {
    /// The `(outcome, quantity)` pairs in sorted order, zero quantities included: the facade's `Counts` items.
    fn items<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyList>> {
        let items = self
            .die
            .items()
            .iter()
            .map(|(o, q)| PyTuple::new(py, [to_py(py, o)?, quantity_to_py(py, q)?]))
            .collect::<PyResult<Vec<_>>>()?;
        PyList::new(py, items)
    }

    fn __len__(&self) -> usize {
        self.die.len()
    }

    /// As the second element of the facade die's `hash_key` `(Die, rs)`: hashed as the core hashes dice.
    fn __hash__(&self) -> isize {
        pool::hash_of(&self.die)
    }

    /// `==` is the core's die equality, `<` etc. compare as `tuple(items)` would (`format::richcmp`).
    fn __richcmp__(
        &self,
        other: &Bound<'_, PyAny>,
        op: pyo3::pyclass::CompareOp,
    ) -> PyResult<Py<PyAny>> {
        format::richcmp(self, other, op)
    }
}

impl RsDie {
    /// Wraps a core result for Python. `Counts` rejects `Reroll` and `Restart` as keys (they are host objects to
    /// the core, which only rejects `None`): so does this.
    fn wrap(py: Python<'_>, die: icepool_core::Result<CoreDie<Value>>) -> PyResult<RsDie> {
        let die = die.map_err(to_py_err)?;
        let reroll_types = py.import("icepool")?.getattr("REROLL_TYPES")?;
        for (o, _) in die.items() {
            if let Value::Host(_) = o {
                let obj = to_py(py, o)?;
                if reroll_types.contains(&obj)? {
                    return Err(PyTypeError::new_err(format!(
                        "{} is not a valid key.",
                        obj.str()?
                    )));
                }
            }
        }
        Ok(RsDie { die })
    }
}

/// One element of `Die(outcomes)` after `creation_args.expand_arg`, as the facade passes it:
/// `(0, outcome)`, `(1, rs_die)`, `(2, None)` for `Reroll`, or `(3, items)` for another population (a `Deck`).
fn die_arg(arg: &Bound<'_, PyAny>) -> PyResult<DieArg<Value>> {
    let (kind, payload): (u8, Bound<'_, PyAny>) = arg.extract()?;
    Ok(match kind {
        0 => DieArg::Outcome(from_py(&payload)?),
        1 => DieArg::Die(payload.cast::<RsDie>()?.get().die.clone()),
        2 => DieArg::Reroll,
        _ => {
            let items = payload
                .try_iter()?
                .map(|item| {
                    let (o, q): (Bound<'_, PyAny>, Bound<'_, PyAny>) = item?.extract()?;
                    Ok((from_py(&o)?, quantity(&q)?))
                })
                .collect::<PyResult<Vec<_>>>()?;
            DieArg::Die(CoreDie::from_counts(
                icepool_core::counts::Counts::new(items).map_err(to_py_err)?,
            ))
        }
    })
}

/// A non-negative Python `int` as a quantity.
fn quantity(q: &Bound<'_, PyAny>) -> PyResult<Quantity> {
    UBig::try_from(int_from_py(q)?)
        .map_err(|_| pyo3::exceptions::PyValueError::new_err("negative quantity"))
}

/// One element of `times` as the weight `merge_weights_lcm` gives it. `itemize` has already rejected negatives.
/// An `int` (or `bool`) is the weight. Another number only survives if Python never passes it to `math.gcd`:
/// when it is zero, or the argument's denominator is 0 (an empty die, `Reroll`); either way its scale factor is
/// 0 and it contributes nothing. Otherwise `math.gcd` raises `TypeError`.
fn weight(arg: &DieArg<Value>, t: &Bound<'_, PyAny>) -> PyResult<Quantity> {
    if t.is_instance_of::<pyo3::types::PyInt>() {
        return quantity(t);
    }
    let empty = match arg {
        DieArg::Die(d) => d.denominator() == Quantity::ZERO,
        DieArg::Reroll => true,
        DieArg::Outcome(_) => false,
    };
    if empty || t.eq(0)? {
        return Ok(Quantity::ZERO);
    }
    Err(PyTypeError::new_err(format!(
        "'{}' object cannot be interpreted as an integer",
        t.get_type().name()?
    )))
}

/// `creation_args.expand_args_for_die(args, times)`: mixes the arguments with `merge_weights_lcm` and sorts the
/// result into a die. `args` as described at `die_arg`.
#[pyfunction]
fn expand_args_for_die(
    py: Python<'_>,
    args: Vec<Bound<'_, PyAny>>,
    times: Vec<Bound<'_, PyAny>>,
) -> PyResult<RsDie> {
    let args = args.iter().map(die_arg).collect::<PyResult<Vec<_>>>()?;
    let weights = args
        .iter()
        .zip(&times)
        .map(|(a, t)| weight(a, t))
        .collect::<PyResult<Vec<_>>>()?;
    if args.len() != weights.len() {
        return Err(to_py_err(Error::Value("one weight per argument".into())));
    }
    // `Die::new` returns a single die with weight 1 as-is; `Die.__new__` handled that case already (comparing
    // `times[0] == 1` as Python does), so only the merge is left.
    let counts = icepool_core::creation_args::expand_args_for_die(&args, &weights);
    RsDie::wrap(py, counts.map(CoreDie::from_counts))
}

/// A die from the items of a facade `Counts` (already sorted and merged, by Python or by Rust).
#[pyfunction]
fn from_counts_items(items: Vec<(Bound<'_, PyAny>, Bound<'_, PyAny>)>) -> PyResult<RsDie> {
    let items = items
        .iter()
        .map(|(o, q)| Ok((from_py(o)?, quantity(q)?)))
        .collect::<PyResult<Vec<_>>>()?;
    Ok(RsDie {
        die: CoreDie::from_counts(icepool_core::counts::Counts::from_sorted(items)),
    })
}

/// `Population._unary_operator(op, *args, **kwargs)` for a die.
#[pyfunction]
#[pyo3(signature = (die, op, *args, **kwargs))]
fn unary_operator(
    py: Python<'_>,
    die: &Bound<'_, RsDie>,
    op: &Bound<'_, PyAny>,
    args: &Bound<'_, PyTuple>,
    kwargs: Option<&Bound<'_, PyDict>>,
) -> PyResult<RsDie> {
    let kwargs = kwargs.filter(|k| !k.is_empty());
    let op = ops::unary(
        op,
        args,
        if kwargs.is_some() {
            usize::MAX
        } else {
            args.len()
        },
    )?;
    RsDie::wrap(
        py,
        die.get().die.unary_operator(|a| op.apply(a, args, kwargs)),
    )
}

/// `Die.binary_operator(other, op, *args, **kwargs)`.
#[pyfunction]
#[pyo3(signature = (die, other, op, *args, **kwargs))]
fn binary_operator(
    py: Python<'_>,
    die: &Bound<'_, RsDie>,
    other: &Bound<'_, RsDie>,
    op: &Bound<'_, PyAny>,
    args: &Bound<'_, PyTuple>,
    kwargs: Option<&Bound<'_, PyDict>>,
) -> PyResult<RsDie> {
    let kwargs = kwargs.filter(|k| !k.is_empty());
    let op = ops::binary(op, args.is_empty() && kwargs.is_none())?;
    let result = die
        .get()
        .die
        .binary_operator(&other.get().die, |a, b| op.apply(a, b, args, kwargs));
    RsDie::wrap(py, result)
}

/// A program-layer error (the Python-dynamic layer the facade shares with icepool-run) as a Python exception.
fn perr_to_py(e: PErr) -> PyErr {
    match e {
        PErr::Raise(e) => to_py_err(e),
        PErr::Unimplemented(m) => PyNotImplementedError::new_err(format!("icepool_rs: {m}")),
    }
}

fn rs(die: &Bound<'_, RsDie>) -> CoreDie<Value> {
    die.get().die.clone()
}

/// A number used as an exponent `e` of an `int` base, by what Python's `int ** e` makes of it: an int result, or
/// (a float, or a Fraction that is not a non-negative integer) a non-int one, whose value is given as a float.
enum Exponent {
    Int(IBig),
    NonInt(f64),
}

fn exponent(x: &Bound<'_, PyAny>) -> PyResult<Option<Exponent>> {
    // `int` and its subclasses (`bool`, `IntEnum`).
    if x.is_instance_of::<PyInt>() {
        return Ok(Some(Exponent::Int(int_from_py(x)?)));
    }
    if x.is_exact_instance_of::<PyFloat>() {
        return Ok(Some(Exponent::NonInt(x.extract()?)));
    }
    if let Value::Fraction(r) = from_py(x)? {
        // `int ** Fraction` is `int ** numerator` for a non-negative integer, otherwise `Fraction ** Fraction`
        // (a float, or a Fraction for a negative integer).
        if r.denominator().is_one() && r.numerator() >= &IBig::ZERO {
            return Ok(Some(Exponent::Int(r.numerator().clone())));
        }
        return Ok(Some(Exponent::NonInt(
            x.call_method0("__float__")?.extract()?,
        )));
    }
    Ok(None)
}

/// `float(i) ** e` as Python checks it: `OverflowError` for an int too large for a float, or a finite power whose
/// result is not.
fn float_pow(i: &Quantity, e: f64) -> PyResult<f64> {
    let base = icepool_core::value::int_to_f64(&IBig::from(i.clone())).map_err(to_py_err)?;
    let r = base.powf(e);
    if r.is_infinite() && e.is_finite() {
        return Err(PyOverflowError::new_err(
            "(34, 'Numerical result out of range')",
        ));
    }
    Ok(r)
}

/// `Die.reroll` once the facade has selected the outcomes (`_select_outcomes`, or the min outcome) as one flag
/// per outcome. `depth`: `None` is Python's `'inf'`; the facade has already rejected a negative depth.
#[pyfunction]
fn reroll(
    py: Python<'_>,
    die: &Bound<'_, RsDie>,
    mask: Vec<bool>,
    depth: Option<&Bound<'_, PyAny>>,
) -> PyResult<RsDie> {
    let die = &die.get().die;
    let depth = match depth.map(exponent).transpose()? {
        None => None,
        Some(Some(Exponent::Int(d))) => Some(u32::try_from(d).map_err(|_| {
            // Python would compute for a very long time (or run out of memory) with a depth this large.
            PyNotImplementedError::new_err("icepool_rs: huge reroll depth")
        })?),
        Some(Some(Exponent::NonInt(depth))) => {
            // `total_reroll_quantity**depth` and `denominator**(depth + 1)` are floats, and so is the stop factor
            // (unless `// total_stop_quantity` divides by zero) and every quantity it scales: `Die(data)` then
            // raises, as every stopping outcome has a non-zero quantity.
            let total: Quantity = die
                .items()
                .iter()
                .zip(&mask)
                .filter(|(_, &r)| r)
                .map(|((_, q), _)| q)
                .sum();
            let denominator = die.denominator();
            float_pow(&total, depth)?;
            float_pow(&denominator, depth + 1.0)?;
            if denominator == total {
                return Err(PyZeroDivisionError::new_err("float floor division by zero"));
            }
            return Err(PyTypeError::new_err(
                "'float' object cannot be interpreted as an integer",
            ));
        }
        Some(None) => {
            return Err(PyNotImplementedError::new_err(
                "icepool_rs: a reroll depth that is not a number",
            ))
        }
    };
    RsDie::wrap(py, die.reroll(&mask, depth))
}

/// `Population.split` for a die, with the selection as one flag per outcome: `(selected, the rest)`.
#[pyfunction]
fn split(py: Python<'_>, die: &Bound<'_, RsDie>, mask: Vec<bool>) -> PyResult<(RsDie, RsDie)> {
    let (a, b) = die.get().die.split(&mask).map_err(to_py_err)?;
    Ok((RsDie::wrap(py, Ok(a))?, RsDie::wrap(py, Ok(b))?))
}

/// A bound of `truncate` / `clip`: `None` leaves that side alone.
fn bound(x: Option<&Bound<'_, PyAny>>) -> PyResult<Option<Value>> {
    x.map(from_py).transpose()
}

/// `Die.truncate(min_outcome, max_outcome)`.
#[pyfunction]
#[pyo3(signature = (die, min_outcome=None, max_outcome=None))]
fn truncate(
    py: Python<'_>,
    die: &Bound<'_, RsDie>,
    min_outcome: Option<&Bound<'_, PyAny>>,
    max_outcome: Option<&Bound<'_, PyAny>>,
) -> PyResult<RsDie> {
    let (min, max) = (bound(min_outcome)?, bound(max_outcome)?);
    RsDie::wrap(py, die.get().die.truncate(min.as_ref(), max.as_ref()))
}

/// `Die.clip(min_outcome, max_outcome)`.
#[pyfunction]
#[pyo3(signature = (die, min_outcome=None, max_outcome=None))]
fn clip(
    py: Python<'_>,
    die: &Bound<'_, RsDie>,
    min_outcome: Option<&Bound<'_, PyAny>>,
    max_outcome: Option<&Bound<'_, PyAny>>,
) -> PyResult<RsDie> {
    let (min, max) = (bound(min_outcome)?, bound(max_outcome)?);
    RsDie::wrap(py, die.get().die.clip(min.as_ref(), max.as_ref()))
}

/// `n.__matmul__(die)` once both are dice: roll `n`, then roll `die` that many times and sum. The counts'
/// Python types matter (a float count makes float quantities), as in icepool-run.
#[pyfunction]
fn matmul(py: Python<'_>, n: &Bound<'_, RsDie>, die: &Bound<'_, RsDie>) -> PyResult<RsDie> {
    let result = matmul_core(py, n.get().die.clone(), &die.get().die)?;
    RsDie::wrap(py, Ok(result))
}

/// `matmul` on core dice (also count mode's `n @ die`).
fn matmul_core(
    py: Python<'_>,
    mut n: CoreDie<Value>,
    die: &CoreDie<Value>,
) -> PyResult<CoreDie<Value>> {
    // A count of an `int` subclass (an `IntEnum`) is a host object to the core, but `_sum_all` and the scaling
    // only do int arithmetic and comparisons with it, as with the equal int; counts never appear in the result.
    if n.outcomes().any(|o| matches!(o, Value::Host(_))) {
        let items = n
            .items()
            .iter()
            .map(|(o, q)| {
                let obj = to_py(py, o)?;
                let o = if matches!(o, Value::Host(_)) && obj.is_instance_of::<PyInt>() {
                    Value::Int(int_from_py(&obj)?)
                } else {
                    o.clone()
                };
                Ok((o, q.clone()))
            })
            .collect::<PyResult<Vec<_>>>()?;
        n = CoreDie::from_counts(icepool_core::counts::Counts::from_sorted(items));
    }
    icepool_core::program::matmul_dice(&n, die).map_err(perr_to_py)
}

/// `Die._popped_min`: the die without its min outcome (zero quantities kept) and that outcome's quantity.
#[pyfunction]
fn pop_min<'py>(py: Python<'py>, die: &Bound<'py, RsDie>) -> PyResult<(RsDie, Bound<'py, PyAny>)> {
    let (popped, q) = die.get().die.pop_min().map_err(to_py_err)?;
    Ok((RsDie { die: popped }, quantity_to_py(py, &q)?))
}

/// `Die._popped_max`.
#[pyfunction]
fn pop_max<'py>(py: Python<'py>, die: &Bound<'py, RsDie>) -> PyResult<(RsDie, Bound<'py, PyAny>)> {
    let (popped, q) = die.get().die.pop_max().map_err(to_py_err)?;
    Ok((RsDie { die: popped }, quantity_to_py(py, &q)?))
}

/// `Die._lowest_single(rolls)` (`highest` false) or `_highest_single(rolls)` for `rolls` other than 0 (the facade
/// makes that `zero().simplify()`): `from_cumulative` over each cumulative quantity `** rolls`.
#[pyfunction]
fn die_keep_single(
    py: Python<'_>,
    die: &Bound<'_, RsDie>,
    rolls: &Bound<'_, PyAny>,
    highest: bool,
) -> PyResult<RsDie> {
    let die = &die.get().die;
    let rolls = match exponent(rolls)? {
        Some(Exponent::Int(r)) if r >= IBig::ZERO => {
            // Python would run out of memory with x**rolls this large.
            let r = usize::try_from(r).map_err(|_| PyOverflowError::new_err("rolls too large"))?;
            let result = if highest {
                die.highest_single(r)
            } else {
                die.lowest_single(r)
            };
            return RsDie::wrap(py, result);
        }
        // Only its sign matters below.
        Some(Exponent::Int(_)) => -1.0,
        Some(Exponent::NonInt(r)) => r,
        None => {
            return Err(PyNotImplementedError::new_err(
                "icepool_rs: keeping one of a number of rolls that is not a number",
            ))
        }
    };
    // `x**rolls` is not an int here, which `from_cumulative` rejects, unless there are no quantities at all; but
    // computing the powers may raise first. The cumulative quantities run from the quantity of the outcome at one
    // end (the max for `>=`, the min for `<=`), zero or not, up to the denominator, the largest.
    let end = if highest {
        die.items().first()
    } else {
        die.items().last()
    };
    let Some((_, end_quantity)) = end else {
        return RsDie::wrap(py, CoreDie::from_dict(vec![]));
    };
    if rolls < 0.0 && end_quantity.is_zero() {
        return Err(PyZeroDivisionError::new_err(
            "0.0 cannot be raised to a negative power",
        ));
    }
    if rolls > 0.0 {
        float_pow(&die.denominator(), rolls)?;
    }
    Err(PyTypeError::new_err(
        "Unsupported type <class 'list'> for cumulative values.",
    ))
}

/// keep.py `_lowest_single(*dice)` (`highest` false) or `_highest_single(*dice)`: roll all the dice and take the
/// lowest (highest) single one.
#[pyfunction]
fn keep_single(py: Python<'_>, dice: Vec<Bound<'_, RsDie>>, highest: bool) -> PyResult<RsDie> {
    let dice: Vec<_> = dice.iter().map(rs).collect();
    if dice.is_empty() {
        let f = if highest { "max" } else { "min" };
        return Err(PyValueError::new_err(format!(
            "{f}() iterable argument is empty"
        )));
    }
    let result = if highest {
        icepool_core::keep::highest_single(&dice)
    } else {
        icepool_core::keep::lowest_single(&dice)
    };
    RsDie::wrap(py, result)
}

/// `Population.zero` for a die: every outcome times `0`, which must resolve to a single outcome.
#[pyfunction]
fn zero(py: Python<'_>, die: &Bound<'_, RsDie>) -> PyResult<RsDie> {
    RsDie::wrap(py, die.get().die.zero())
}

/// `Die.simplify`: all quantities divided by their greatest common denominator (zero quantities drop out).
#[pyfunction]
fn simplify(py: Python<'_>, die: &Bound<'_, RsDie>) -> PyResult<RsDie> {
    RsDie::wrap(py, die.get().die.simplify())
}

/// `Die.cmp` once `other` is a die: -1, 0, 1 weighted by the `True` quantities of `<`, `==`, `>`.
#[pyfunction]
fn cmp(py: Python<'_>, die: &Bound<'_, RsDie>, other: &Bound<'_, RsDie>) -> PyResult<RsDie> {
    RsDie::wrap(py, die.get().die.cmp_die(&other.get().die))
}

/// `Die.sign`: 1, -1 or 0 by comparing each outcome with its own `x * 0`.
#[pyfunction]
fn sign(py: Python<'_>, die: &Bound<'_, RsDie>) -> PyResult<RsDie> {
    RsDie::wrap(py, die.get().die.sign())
}

#[pymodule]
fn icepool_rs(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<RsDie>()?;
    m.add_class::<pool::RsPool>()?;
    m.add_class::<pool::RsPoolSource>()?;
    m.add_class::<deck::RsDeck>()?;
    m.add_class::<deck::RsDeal>()?;
    m.add_class::<deck::RsDealSource>()?;
    m.add_class::<multi_deal::RsMultiDeal>()?;
    m.add_class::<multi_deal::RsMultiDealSource>()?;
    m.add_class::<expression::RsCompoundKeep>()?;
    m.add_class::<expression::RsCompoundKeepSource>()?;
    m.add_function(wrap_pyfunction!(expand_args_for_die, m)?)?;
    m.add_function(wrap_pyfunction!(from_counts_items, m)?)?;
    m.add_function(wrap_pyfunction!(unary_operator, m)?)?;
    m.add_function(wrap_pyfunction!(binary_operator, m)?)?;
    for f in [
        wrap_pyfunction!(reroll, m)?,
        wrap_pyfunction!(split, m)?,
        wrap_pyfunction!(truncate, m)?,
        wrap_pyfunction!(clip, m)?,
        wrap_pyfunction!(matmul, m)?,
        wrap_pyfunction!(pop_min, m)?,
        wrap_pyfunction!(pop_max, m)?,
        wrap_pyfunction!(die_keep_single, m)?,
        wrap_pyfunction!(keep_single, m)?,
        wrap_pyfunction!(zero, m)?,
        wrap_pyfunction!(cmp, m)?,
        wrap_pyfunction!(sign, m)?,
        wrap_pyfunction!(simplify, m)?,
        wrap_pyfunction!(map::map_simple, m)?,
        wrap_pyfunction!(again::evaluate_agains_using_depth, m)?,
        wrap_pyfunction!(again::evaluate_agains_using_count, m)?,
        wrap_pyfunction!(map::tupleize, m)?,
        wrap_pyfunction!(map::vectorize, m)?,
        wrap_pyfunction!(map_tools::map_repeat, m)?,
        wrap_pyfunction!(map_tools::map_and_time, m)?,
        wrap_pyfunction!(map_tools::absorbing_markov_chain, m)?,
        wrap_pyfunction!(pool::pool_new, m)?,
        wrap_pyfunction!(evaluate::evaluate_builtin, m)?,
        wrap_pyfunction!(evaluate::evaluate_host, m)?,
        wrap_pyfunction!(evaluate::evaluate_tree, m)?,
        wrap_pyfunction!(pool::pool_additive_union, m)?,
        wrap_pyfunction!(deck::expand_args_for_deck, m)?,
        wrap_pyfunction!(deck::deck_from_counts_items, m)?,
        wrap_pyfunction!(deck::deck_counter_op, m)?,
        wrap_pyfunction!(deck::deal_new, m)?,
        wrap_pyfunction!(multi_deal::multi_deal_new, m)?,
        wrap_pyfunction!(expression::compound_keep_new, m)?,
        wrap_pyfunction!(stats::quantities, m)?,
        wrap_pyfunction!(stats::quantity, m)?,
        wrap_pyfunction!(stats::probability, m)?,
        wrap_pyfunction!(stats::probabilities, m)?,
        wrap_pyfunction!(stats::nearest, m)?,
        wrap_pyfunction!(stats::quantity_where, m)?,
        wrap_pyfunction!(stats::probability_where, m)?,
        wrap_pyfunction!(stats::mode, m)?,
        wrap_pyfunction!(stats::modal_quantity, m)?,
        wrap_pyfunction!(stats::denominator, m)?,
        wrap_pyfunction!(stats::is_empty, m)?,
        wrap_pyfunction!(stats::min_outcome, m)?,
        wrap_pyfunction!(stats::max_outcome, m)?,
        wrap_pyfunction!(stats::cdf_distance, m)?,
        wrap_pyfunction!(stats::quantile, m)?,
        wrap_pyfunction!(stats::statistic, m)?,
        wrap_pyfunction!(stats::standardized_moment, m)?,
        wrap_pyfunction!(stats::entropy, m)?,
        wrap_pyfunction!(stats::marginals_len, m)?,
        wrap_pyfunction!(stats::joint, m)?,
        wrap_pyfunction!(stats::group_by, m)?,
        wrap_pyfunction!(stats::quantities_op, m)?,
        wrap_pyfunction!(stats::pad_to_denominator, m)?,
        wrap_pyfunction!(stats::multiply_to_denominator, m)?,
        wrap_pyfunction!(format::format, m)?,
        wrap_pyfunction!(function::weighted_lcm, m)?,
        wrap_pyfunction!(function::pointwise, m)?,
        wrap_pyfunction!(function::coin_data, m)?,
        wrap_pyfunction!(function::min_max, m)?,
        wrap_pyfunction!(function::consecutive, m)?,
        wrap_pyfunction!(function::sorted_union, m)?,
        wrap_pyfunction!(function::one_hot, m)?,
        wrap_pyfunction!(format::format_probability_inverse, m)?,
        wrap_pyfunction!(format::common_outcome_length, m)?,
        wrap_pyfunction!(format::sample_at, m)?,
        wrap_pyfunction!(format::die_equals, m)?,
        wrap_pyfunction!(collection::vector_binary_operator, m)?,
        wrap_pyfunction!(collection::vector_reverse_binary_operator, m)?,
        wrap_pyfunction!(collection::vector_unary_operator, m)?,
        wrap_pyfunction!(collection::symbols_from_str, m)?,
        wrap_pyfunction!(collection::symbols_from_iter, m)?,
        wrap_pyfunction!(collection::symbols_from_mapping, m)?,
        wrap_pyfunction!(collection::symbols_op, m)?,
        wrap_pyfunction!(collection::symbols_unary, m)?,
        wrap_pyfunction!(collection::symbols_count_subset, m)?,
        wrap_pyfunction!(collection::symbols_getitem, m)?,
    ] {
        m.add_function(f)?;
    }
    Ok(())
}
