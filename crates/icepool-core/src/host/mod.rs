//! Host callbacks: everything a language binding implements for the core to call back. The core stays static and
//! never names a host language; where Python (or JS) has something the core has no native variant for, the binding
//! hands it over behind one of these traits, and the core calls its methods exactly where icepool would call the
//! Python object's.
//!
//! - [`HostObject`] (held as a [`HostRef`] in [`Value::Host`](crate::Value::Host)): an outcome of a host type
//!   (namedtuples, enums, user classes...). [`HostError`] carries the host's own exception through the core.
//! - [`HostSourceObject`] (held as a [`HostSource`] in
//!   [`MultisetSource::Host`](crate::evaluator::MultisetSource::Host)): a user's `MultisetSource`.
//! - [`HostOperator`] (in a multiset expression's operator nodes): a user's `MultisetOperator`.
//! - [`Traverse`] / [`owned_hosts`]: which host objects a core value alone owns, for the host's cycle collector;
//!   [`set_interrupt_check`]: the host's check (Ctrl-C) that the core's long loops poll.
//!
//! Their arguments and results are core types only. User callables and evaluators are each binding's own (a
//! `MultisetEvaluator<Value>` calling the host object back: icepool-py's `evaluate.rs`, icepool-wasm's
//! `api/evaluator.rs`), as are the program runner's (icepool-harness's `host.rs`).

mod object;
mod operator;
mod source;

pub(crate) use object::host_of;
pub use object::{HostError, HostObject, HostRef, LenKind};
pub use operator::HostOperator;
pub use source::{HostSource, HostSourceObject};

// The host's side of two more core services: its cycle collector, which asks a core value which host objects it
// alone owns, and its interrupt check (Ctrl-C), which the core's long loops poll.
pub use crate::interrupt::set_check as set_interrupt_check;
pub use crate::ownership::{owned_hosts, Owners, Traverse};
