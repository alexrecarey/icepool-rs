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
//!
//! Their arguments and results are core types only. (The JS facade's callables and evaluators, `HostFunction` and
//! `HostEvaluator`, still live with the program runner in icepool-harness, whose values they take: until task C6e
//! gives them a typed home in icepool-wasm.)

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
