//! `MultiDeal` / `MultiDealSource` (generator/multi_deal.py) on the core.
//!
//! The facade's `MultiDeal` keeps its hand groups, `hand_sizes()`, `total_cards_dealt()`, `len()` and the
//! total-cards check in Python (they only juggle the arguments) and holds the core multi-deal made here as `_rs`:
//! the denominator and the source are the core's, and the source pops in the core (the cards of the popped outcome
//! dealt to the hands in every way, hands of a group up to reordering), under the core's evaluation driver
//! (evaluate.rs), where its counts and size reach the evaluator as tuples.

use std::sync::Arc;

use dashu_int::IBig;
use icepool_core::multi_deal::{HandGroup, MultiDealSource};
use icepool_core::order::{Order, OrderReason};
use icepool_core::{Int, MultiDeal, Value};
use pyo3::exceptions::PyNotImplementedError;
use pyo3::prelude::*;
use pyo3::types::{PyInt, PyTuple};

use crate::convert::{counts_to_py, from_py, int_from_py, quantity_to_py, to_py_err};
use crate::deck::RsDeck;
use crate::pool::{hash_of, outcome_tuple};

/// A multi-deal of the Rust core, held by the facade's `MultiDeal` as `_rs`.
#[pyclass(frozen, name = "MultiDeal", module = "icepool_rs")]
///
/// It holds the deck's Python object rather than a second clone of its cards, and makes the core multi-deal when
/// asked: the facade's `MultiDeal` keeps its deck too (see `RsDeal`).
pub struct RsMultiDeal {
    deck: Py<RsDeck>,
    hand_groups: Arc<[HandGroup]>,
}

impl RsMultiDeal {
    fn multi_deal(&self) -> MultiDeal<Value> {
        MultiDeal::from_parts(self.deck.get().deck.clone(), self.hand_groups.clone())
    }
}

/// What the evaluator pops a multi-deal from, held by the facade's `MultiDealSource` as `_rs`.
#[pyclass(frozen, name = "MultiDealSource", module = "icepool_rs")]
pub struct RsMultiDealSource {
    pub(crate) source: MultiDealSource<Value>,
}

fn hand_int(x: &Bound<'_, PyAny>) -> PyResult<IBig> {
    if !x.is_instance_of::<PyInt>() {
        return Err(PyNotImplementedError::new_err(
            "icepool_rs: a MultiDeal hand size that is not an int",
        ));
    }
    int_from_py(x)
}

/// `MultiDeal(deck, hand_groups)` once the facade has checked the total (and so unpacked every group): each group
/// a `(hand_size, group_size)` pair of ints.
#[pyfunction]
pub fn multi_deal_new(
    deck: &Bound<'_, RsDeck>,
    hand_groups: Vec<(Bound<'_, PyAny>, Bound<'_, PyAny>)>,
) -> PyResult<RsMultiDeal> {
    let groups = hand_groups
        .iter()
        .map(|(h, g)| Ok((hand_int(h)?, hand_int(g)?)))
        .collect::<PyResult<Vec<_>>>()?;
    let multi_deal = MultiDeal::new(deck.get().deck.clone(), &groups).map_err(to_py_err)?;
    Ok(RsMultiDeal {
        deck: deck.clone().unbind(),
        hand_groups: multi_deal.hand_groups().into(),
    })
}

/// One possibility of `pop`: `(popped source, counts, weight)`.
type Popped<'py> = (RsMultiDealSource, Bound<'py, PyTuple>, Bound<'py, PyAny>);

#[pymethods]
impl RsMultiDeal {
    /// Its deck's Python object (which holds the cards), for the cycle collector.
    fn __traverse__(&self, visit: pyo3::gc::PyVisit<'_>) -> Result<(), pyo3::PyTraverseError> {
        visit.call(&self.deck)
    }

    /// `comb(deck.size(), total) * prod(comb(total, h) for h in hand_sizes()[1:])`.
    fn denominator<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        quantity_to_py(py, &self.multi_deal().denominator())
    }

    /// `_make_source`.
    fn source(&self) -> RsMultiDealSource {
        RsMultiDealSource {
            source: self.multi_deal().source(),
        }
    }

    /// Python's `hash_key` `(MultiDeal, deck, hand_groups)`.
    fn __hash__(&self) -> isize {
        hash_of(&self.multi_deal())
    }

    fn __eq__(&self, other: &Bound<'_, PyAny>) -> bool {
        other
            .cast::<RsMultiDeal>()
            .is_ok_and(|o| self.multi_deal() == o.get().multi_deal())
    }
}

#[pymethods]
impl RsMultiDealSource {
    /// The Python objects it holds, for the cycle collector (`convert::traverse`).
    fn __traverse__(&self, visit: pyo3::gc::PyVisit<'_>) -> Result<(), pyo3::PyTraverseError> {
        crate::convert::traverse(&self.source, &visit)
    }

    fn deck(&self) -> RsDeck {
        RsDeck {
            deck: self.source.deck().clone(),
        }
    }

    /// The hand groups as a tuple of `(hand_size, group_size)` tuples.
    fn hand_groups<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        let groups = self
            .source
            .hand_groups()
            .iter()
            .map(|&(h, g)| PyTuple::new(py, [h, g]))
            .collect::<PyResult<Vec<_>>>()?;
        PyTuple::new(py, groups)
    }

    fn outcomes<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        outcome_tuple(py, self.source.outcomes())
    }

    fn denominator<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        quantity_to_py(py, &self.source.denominator())
    }

    /// `size()`: the hand sizes.
    fn size<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        counts_to_py(py, &self.source.size())
    }

    fn is_resolvable(&self) -> bool {
        self.source.is_resolvable()
    }

    /// `(order, reason)` as the values of the facade's `Order` and `OrderReason`.
    fn order_preference(&self) -> (i64, i64) {
        let (order, reason): (Order, OrderReason) = self.source.order_preference();
        (order.value(), reason as i64)
    }

    /// `MultiDealSource.pop(order, outcome)`: `(popped source, counts, weight)` for each possibility (`order > 0`
    /// pops the min outcome, else the max).
    fn pop<'py>(
        &self,
        py: Python<'py>,
        order: i64,
        outcome: &Bound<'py, PyAny>,
    ) -> PyResult<Vec<Popped<'py>>> {
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
            .map(|(source, counts, weight): (_, Arc<[Int]>, _)| {
                Ok((
                    RsMultiDealSource { source },
                    counts_to_py(py, &counts)?,
                    quantity_to_py(py, &weight)?,
                ))
            })
            .collect()
    }

    /// Python's `hash_key` `(MultiDealSource, deck, hand_groups)`.
    fn __hash__(&self) -> isize {
        hash_of(&self.source)
    }

    fn __eq__(&self, other: &Bound<'_, PyAny>) -> bool {
        other
            .cast::<RsMultiDealSource>()
            .is_ok_and(|o| self.source == o.get().source)
    }
}
