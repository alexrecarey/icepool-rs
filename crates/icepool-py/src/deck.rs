//! `Deck` (population/deck.py) and `Deal` / `DealSource` (generator/deal.py) on the core.
//!
//! The facade's `Deck.__new__` keeps `contains_again`, `itemize` and `expand_arg` (each subdata is passed tagged,
//! as for dice) and holds the core deck made here as `_rs`; its `Population` methods use the deck's cards as a
//! core die (`population`). The operators take `Counter(other)` from Python (any iterable or mapping, as
//! Python counts it); the counts are combined here and the facade builds `Deck(+data)` from the result, so the
//! keys are expanded as any `Deck(...)` argument is. A deal holds the core deal, keeps and multiplies its counts in
//! the core, and its source pops in the core (hypergeometric), under the core's evaluation driver (evaluate.rs).

use std::sync::Arc;

use dashu_int::IBig;
use icepool_core::creation_args::DieArg;
use icepool_core::deal::DealSource;
use icepool_core::deck::{Counter, CounterOp};
use icepool_core::order::{Order, OrderReason};
use icepool_core::{Deal, Deck, Int, Quantity, Value};
use pyo3::exceptions::{PyNotImplementedError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyInt, PyList, PyTuple};

use crate::convert::{
    count_to_py, counts_to_py, from_py, int_from_py, quantity_to_py, to_py, to_py_err,
};
use crate::pool::{exact_count, hash_of, keep_index, outcome_tuple};
use crate::{die_arg, quantity, RsDie};

/// A deck of the Rust core, held by the facade's `Deck` as `_rs`.
#[pyclass(frozen, name = "Deck", module = "icepool_rs")]
pub struct RsDeck {
    pub(crate) deck: Deck<Value>,
}

/// A deal of the Rust core, held by the facade's `Deal` as `_rs`.
#[pyclass(frozen, name = "Deal", module = "icepool_rs")]
///
/// It holds the deck's Python object rather than a second clone of its cards, and makes the core deal when asked:
/// the facade's `Deal` keeps its deck too, and Python's cycle collector can only be told about Python objects held
/// by one object (`convert::traverse`), so the cards stay the deck's alone.
pub struct RsDeal {
    deck: Py<RsDeck>,
    keep_tuple: Arc<[Int]>,
}

/// What the evaluator pops a deal from, held by the facade's `DealSource` as `_rs`.
#[pyclass(frozen, name = "DealSource", module = "icepool_rs")]
pub struct RsDealSource {
    pub(crate) source: DealSource<Value>,
}

fn items<'py>(py: Python<'py>, items: &[(Value, Quantity)]) -> PyResult<Bound<'py, PyList>> {
    let items = items
        .iter()
        .map(|(o, q)| PyTuple::new(py, [to_py(py, o)?, quantity_to_py(py, q)?]))
        .collect::<PyResult<Vec<_>>>()?;
    PyList::new(py, items)
}

/// One element of `times` as `merge_duplicates` uses it (`itemize` has checked `x < 0`). An `int` is the dup. Any
/// other number is skipped if it equals 0 (`subdup == 0`); otherwise it multiplies the argument's dups into a
/// non-int, which `Counts` rejects with `ValueError`, unless the argument contributes no cards at all.
fn duplicate(arg: &DieArg<Value>, t: &Bound<'_, PyAny>) -> PyResult<Quantity> {
    if t.is_instance_of::<PyInt>() {
        return quantity(t);
    }
    let contributes = match arg {
        DieArg::Outcome(_) => true,
        DieArg::Die(d) => d.items().iter().any(|(_, q)| !q.is_zero()),
        DieArg::Reroll => false,
    };
    if t.eq(0)? || !contributes {
        return Ok(Quantity::ZERO);
    }
    Err(PyValueError::new_err(format!(
        "Values must be ints, got {}",
        t.get_type().name()?
    )))
}

/// `creation_args.expand_args_for_deck(args, times)`: `merge_duplicates`, then sorted into a deck. `args` are
/// tagged as for `expand_args_for_die` (a deck argument comes as `(1, deck._rust_die())`).
#[pyfunction]
pub fn expand_args_for_deck(
    args: Vec<Bound<'_, PyAny>>,
    times: Vec<Bound<'_, PyAny>>,
) -> PyResult<RsDeck> {
    let args = args.iter().map(die_arg).collect::<PyResult<Vec<_>>>()?;
    if args.len() != times.len() {
        return Err(PyValueError::new_err("one duplicate count per argument"));
    }
    let duplicates = args
        .iter()
        .zip(&times)
        .map(|(a, t)| duplicate(a, t))
        .collect::<PyResult<Vec<_>>>()?;
    Ok(RsDeck {
        deck: Deck::new(&args, &duplicates).map_err(to_py_err)?,
    })
}

/// A deck from the items of a facade `Counts` (a deck made by upstream's Python code).
#[pyfunction]
pub fn deck_from_counts_items(
    items: Vec<(Bound<'_, PyAny>, Bound<'_, PyAny>)>,
) -> PyResult<RsDeck> {
    let items = items
        .iter()
        .map(|(o, q)| Ok((from_py(o)?, quantity(q)?)))
        .collect::<PyResult<Vec<_>>>()?;
    Ok(RsDeck {
        deck: Deck::from_counts(icepool_core::counts::Counts::from_sorted(items)),
    })
}

/// The binary operators of `Deck`: `op` names one (`'add'`, `'sub'`, `'rsub'`, `'and'`, `'or'`, `'xor'`) and
/// `other` is `Counter(other).items()`. Returns the items of `+data`, for the facade's `Deck(...)`.
#[pyfunction]
pub fn deck_counter_op<'py>(
    py: Python<'py>,
    deck: &Bound<'py, RsDeck>,
    op: &str,
    other: Vec<(Bound<'py, PyAny>, Bound<'py, PyAny>)>,
) -> PyResult<Bound<'py, PyList>> {
    let op = match op {
        "add" => CounterOp::Add,
        "sub" => CounterOp::Sub,
        "rsub" => CounterOp::RSub,
        "and" => CounterOp::And,
        "or" => CounterOp::Or,
        "xor" => CounterOp::Xor,
        _ => {
            return Err(PyValueError::new_err(format!(
                "icepool_rs: no deck operator {op}"
            )))
        }
    };
    let other = other
        .iter()
        .map(|(o, c)| {
            if !c.is_instance_of::<PyInt>() {
                // A mapping's values may be anything in Python's Counter; only int counts are ported.
                return Err(PyNotImplementedError::new_err(format!(
                    "icepool_rs: a Counter value of type '{}' in a deck operation",
                    c.get_type().name()?
                )));
            }
            Ok((from_py(o)?, int_from_py(c)?))
        })
        .collect::<PyResult<Vec<_>>>()?;
    let result = deck
        .get()
        .deck
        .counter_items(op, &Counter::from_mapping(other));
    items(py, &result)
}

#[pymethods]
impl RsDeck {
    /// The Python objects it holds, for the cycle collector (`convert::traverse`).
    fn __traverse__(&self, visit: pyo3::gc::PyVisit<'_>) -> Result<(), pyo3::PyTraverseError> {
        crate::convert::traverse(&self.deck, &visit)
    }

    /// The `(outcome, quantity)` pairs in sorted order: the facade's `Counts` items.
    fn items<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyList>> {
        items(py, self.deck.items())
    }

    /// The cards as a core die, for the `Population` methods a deck shares with a die.
    fn population(&self) -> RsDie {
        RsDie {
            die: self.deck.population().clone(),
        }
    }

    /// `Deck._pop_min`: `(deck without its min outcome, its quantity)`.
    fn pop_min<'py>(&self, py: Python<'py>) -> PyResult<(RsDeck, Bound<'py, PyAny>)> {
        let (deck, q) = self.deck.pop_min().map_err(to_py_err)?;
        Ok((RsDeck { deck }, quantity_to_py(py, &q)?))
    }

    /// `Deck._pop_max`.
    fn pop_max<'py>(&self, py: Python<'py>) -> PyResult<(RsDeck, Bound<'py, PyAny>)> {
        let (deck, q) = self.deck.pop_max().map_err(to_py_err)?;
        Ok((RsDeck { deck }, quantity_to_py(py, &q)?))
    }

    /// As the second element of the facade deck's `hash_key` `(Deck, rs)`.
    fn __hash__(&self) -> isize {
        hash_of(&self.deck)
    }

    fn __eq__(&self, other: &Bound<'_, PyAny>) -> bool {
        other
            .cast::<RsDeck>()
            .is_ok_and(|o| self.deck == o.get().deck)
    }
}

/// `Deal(deck, hand_size)`: `hand_size` must not be negative nor exceed the deck's size. Python then builds
/// `(1, ) * hand_size`, which raises `TypeError` for a non-int.
#[pyfunction]
pub fn deal_new(rs_deck: &Bound<'_, RsDeck>, hand_size: &Bound<'_, PyAny>) -> PyResult<RsDeal> {
    let deck = rs_deck.get().deck.clone();
    if !hand_size.is_instance_of::<PyInt>() {
        if hand_size.lt(0)? {
            return Err(PyValueError::new_err("hand_size cannot be negative."));
        }
        let size = quantity_to_py(hand_size.py(), &deck.size())?;
        if hand_size.gt(size)? {
            return Err(PyValueError::new_err(
                "The number of cards dealt cannot exceed the size of the deck.",
            ));
        }
        return Err(PyTypeError::new_err(format!(
            "can't multiply sequence by non-int of type '{}'",
            hand_size.get_type().name()?
        )));
    }
    let hand_size: IBig = int_from_py(hand_size)?;
    let deal = Deal::new(deck, &hand_size).map_err(to_py_err)?;
    Ok(RsDeal::with_keep_tuple_of(rs_deck.clone().unbind(), &deal))
}

impl RsDeal {
    fn deal(&self) -> Deal<Value> {
        Deal::from_parts(self.deck.get().deck.clone(), self.keep_tuple.clone())
    }

    /// A deal of `deck` (the deck `deal` is dealt from) with `deal`'s keep tuple.
    fn with_keep_tuple_of(deck: Py<RsDeck>, deal: &Deal<Value>) -> RsDeal {
        RsDeal {
            deck,
            keep_tuple: deal.keep_tuple().into(),
        }
    }

    /// Another deal of the same deck.
    fn derived(&self, py: Python<'_>, deal: &Deal<Value>) -> RsDeal {
        RsDeal::with_keep_tuple_of(self.deck.clone_ref(py), deal)
    }
}

#[pymethods]
impl RsDeal {
    /// Its deck's Python object (which holds the cards), for the cycle collector.
    fn __traverse__(&self, visit: pyo3::gc::PyVisit<'_>) -> Result<(), pyo3::PyTraverseError> {
        visit.call(&self.deck)
    }

    fn keep_tuple<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        counts_to_py(py, &self.keep_tuple)
    }

    /// `comb(deck.size(), hand_size)`.
    fn denominator<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        quantity_to_py(py, &self.deal().denominator())
    }

    /// `KeepGenerator.keep` for a keep tuple without negative counts (the facade checks), short of evaluating
    /// the result for an `int` index (the facade does that).
    fn keep(&self, index: &Bound<'_, PyAny>) -> PyResult<RsDeal> {
        let deal = self.deal().keep(&keep_index(index)?).map_err(to_py_err)?;
        Ok(self.derived(index.py(), &deal))
    }

    /// `_set_keep_tuple`: this deal with another keep tuple.
    fn with_keep_tuple(
        &self,
        py: Python<'_>,
        keep_tuple: Vec<Bound<'_, PyAny>>,
    ) -> PyResult<RsDeal> {
        let keep_tuple = keep_tuple
            .iter()
            .map(exact_count)
            .collect::<PyResult<Vec<_>>>()?;
        Ok(self.derived(py, &self.deal().with_keep_tuple(keep_tuple)))
    }

    /// `KeepGenerator.multiply_counts`.
    fn multiply_counts(&self, constant: &Bound<'_, PyAny>) -> PyResult<RsDeal> {
        let deal = self.deal().multiply_counts(&exact_count(constant)?);
        Ok(self.derived(constant.py(), &deal))
    }

    /// `_make_source`.
    fn source(&self) -> RsDealSource {
        RsDealSource {
            source: self.deal().source(),
        }
    }

    /// Python's `hash_key` `(Deal, deck, keep_tuple)`.
    fn __hash__(&self) -> isize {
        hash_of(&self.deal())
    }

    fn __eq__(&self, other: &Bound<'_, PyAny>) -> bool {
        other
            .cast::<RsDeal>()
            .is_ok_and(|o| self.deal() == o.get().deal())
    }
}

#[pymethods]
impl RsDealSource {
    /// The Python objects it holds, for the cycle collector (`convert::traverse`).
    fn __traverse__(&self, visit: pyo3::gc::PyVisit<'_>) -> Result<(), pyo3::PyTraverseError> {
        crate::convert::traverse(&self.source, &visit)
    }

    fn deck(&self) -> RsDeck {
        RsDeck {
            deck: self.source.deck().clone(),
        }
    }

    fn outcomes<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        outcome_tuple(py, self.source.outcomes())
    }

    fn keep_tuple<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        counts_to_py(py, self.source.keep_tuple())
    }

    fn denominator<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        quantity_to_py(py, &self.source.denominator())
    }

    /// `KeepSource.size`: `None` with negative counts.
    fn size<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyAny>>> {
        self.source.size().map(|s| count_to_py(py, &s)).transpose()
    }

    fn is_resolvable(&self) -> bool {
        self.source.is_resolvable()
    }

    /// `(order, reason)` as the values of the facade's `Order` and `OrderReason`.
    fn order_preference(&self) -> (i64, i64) {
        let (order, reason): (Order, OrderReason) = self.source.order_preference();
        (order.value(), reason as i64)
    }

    /// `DealSource.pop(order, outcome)`: `(popped source, count, weight)` for each possibility (`order > 0` pops
    /// the min outcome, else the max).
    fn pop<'py>(
        &self,
        py: Python<'py>,
        order: i64,
        outcome: &Bound<'py, PyAny>,
    ) -> PyResult<Vec<crate::pool::IntPopped<'py, RsDealSource>>> {
        let order = if order > 0 {
            Order::Ascending
        } else {
            Order::Descending
        };
        let popped = self
            .source
            .pop(order, &from_py(outcome)?)
            .map_err(to_py_err)?;
        popped
            .into_iter()
            .map(|(source, count, weight)| {
                Ok((
                    RsDealSource { source },
                    count_to_py(py, &count)?,
                    quantity_to_py(py, &weight)?,
                ))
            })
            .collect()
    }

    /// Python's `hash_key` `(DealSource, deck, keep_tuple)`.
    fn __hash__(&self) -> isize {
        hash_of(&self.source)
    }

    fn __eq__(&self, other: &Bound<'_, PyAny>) -> bool {
        other
            .cast::<RsDealSource>()
            .is_ok_and(|o| self.source == o.get().source)
    }
}
