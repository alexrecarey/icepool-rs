//! Sources the core has no variant for, on the core driver: a user's `MultisetGenerator` subclass makes a
//! `MultisetSource` (or `MultisetTupleSource`) of its own, whose methods are called back here
//! (`icepool_core::host_source`). Python's driver calls the same methods: `is_resolvable()`,
//! `order_preference()` and `outcomes()` before evaluating (outside its `try`, so an `UnsupportedOrder` there is
//! not the retry signal), `size()` and `pop(order, outcome)` while evaluating (inside it).
//!
//! What a user source yields must be what the core models: counts and sizes that are `int`s or tuples of `int`s
//! within 64 bits (else `NotImplementedError`, a facade gap: Python would compute with anything), weights that
//! are non-negative `int`s. A popped source is a core one when the facade object holds one (`_rs`), else another
//! host source.

use std::sync::Arc;

use icepool_core::expression::value_count;
use icepool_core::host_source::{HostSource, HostSourceObject};
use icepool_core::order::{Order, OrderReason};
use icepool_core::pool::{Count, MultisetSource, SourcePopped};
use icepool_core::{Error, Value};
use pyo3::prelude::*;
use pyo3::sync::PyOnceLock;
use pyo3::types::PyType;

use crate::convert::{from_py, host_error};
use crate::expression::{callback_error, core_source};

static ORDER: PyOnceLock<Py<PyType>> = PyOnceLock::new();

/// A facade source: the core source it holds (`_rs`), or itself as a host source.
pub(crate) fn facade_source(s: &Bound<'_, PyAny>) -> PyResult<MultisetSource<Value>> {
    match s.getattr("__dict__").and_then(|d| d.get_item("_rs")) {
        Ok(rs) => core_source(&rs),
        Err(_) => host_source(s),
    }
}

/// A Python source object as a host source (its hash taken now: an unhashable source raises here).
pub(crate) fn host_source(s: &Bound<'_, PyAny>) -> PyResult<MultisetSource<Value>> {
    let hash = s.hash()? as u64;
    Ok(MultisetSource::Host(HostSource::new(Arc::new(PyHostSource {
        object: s.clone().unbind(),
        hash,
    }))))
}

/// The Python object of a host source, for `source_object`.
pub(crate) fn host_object<'py>(py: Python<'py>, s: &HostSource<Value>) -> Option<Bound<'py, PyAny>> {
    s.object()
        .as_any()
        .downcast_ref::<PyHostSource>()
        .map(|h| h.object.bind(py).clone())
}

struct PyHostSource {
    object: Py<PyAny>,
    hash: u64,
}

/// A count or size as Python gave it: `int` or a tuple of `int`s (a `bool` is not modelled).
fn count(x: &Bound<'_, PyAny>) -> icepool_core::Result<Count> {
    value_count(&from_py(x).map_err(host_error)?).map_err(|e| match e {
        Error::Unsupported(m) => Error::Unsupported(format!("a user source: {m}")),
        e => e,
    })
}

impl PyHostSource {
    fn call<'py>(&self, py: Python<'py>, name: &str) -> PyResult<Bound<'py, PyAny>> {
        self.object.bind(py).call_method0(name)
    }
}

impl HostSourceObject<Value> for PyHostSource {
    fn outcomes(&self) -> icepool_core::Result<Vec<Value>> {
        Python::attach(|py| {
            self.call(py, "outcomes")
                .and_then(|o| o.try_iter()?.map(|x| from_py(&x?)).collect())
                .map_err(host_error)
        })
    }

    fn size(&self) -> icepool_core::Result<Option<Count>> {
        Python::attach(|py| {
            let size = self.call(py, "size").map_err(|e| callback_error(py, e))?;
            if size.is_none() {
                return Ok(None);
            }
            count(&size).map(Some)
        })
    }

    fn is_resolvable(&self) -> icepool_core::Result<bool> {
        Python::attach(|py| {
            self.call(py, "is_resolvable")
                .and_then(|r| r.is_truthy())
                .map_err(host_error)
        })
    }

    fn order_preference(&self) -> icepool_core::Result<(Order, OrderReason)> {
        Python::attach(|py| {
            let (order, reason): (Bound<'_, PyAny>, Bound<'_, PyAny>) = self
                .call(py, "order_preference")
                .and_then(|p| p.extract())
                .map_err(host_error)?;
            let order = order.extract::<i64>().ok().and_then(Order::from_int);
            let reason = reason
                .getattr("value")
                .and_then(|v| v.extract::<i64>())
                .ok()
                .and_then(OrderReason::from_int);
            match (order, reason) {
                (Some(o), Some(r)) => Ok((o, r)),
                _ => Err(Error::Unsupported(
                    "a user source's order_preference that is not (Order, OrderReason)".into(),
                )),
            }
        })
    }

    fn pop(&self, order: Order, outcome: &Value) -> icepool_core::Result<Vec<SourcePopped<Value>>> {
        Python::attach(|py| {
            let order = ORDER
                .import(py, "icepool", "Order")
                .and_then(|o| o.call1((order.value(),)))
                .map_err(host_error)?;
            let outcome = crate::convert::to_py(py, outcome).map_err(host_error)?;
            let popped = self
                .object
                .bind(py)
                .call_method1("pop", (order, outcome))
                .and_then(|p| p.try_iter())
                .map_err(|e| callback_error(py, e))?;
            let mut result = Vec::new();
            for item in popped {
                let (source, c, weight): (Bound<'_, PyAny>, Bound<'_, PyAny>, Bound<'_, PyAny>) = item
                    .and_then(|i| i.extract())
                    .map_err(|e| callback_error(py, e))?;
                result.push((
                    facade_source(&source).map_err(host_error)?,
                    count(&c)?,
                    crate::quantity(&weight).map_err(host_error)?,
                ));
            }
            Ok(result)
        })
    }

    fn host_eq(&self, other: &dyn HostSourceObject<Value>) -> bool {
        let Some(other) = other.as_any().downcast_ref::<PyHostSource>() else {
            return false;
        };
        // The memo of rooms is a dict: equal hashes first, then `==` (identity counts as equal, as for any key).
        self.hash == other.hash
            && Python::attach(|py| {
                let (a, b) = (self.object.bind(py), other.object.bind(py));
                a.is(b) || a.eq(b).unwrap_or(false)
            })
    }

    fn host_hash(&self) -> u64 {
        self.hash
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
