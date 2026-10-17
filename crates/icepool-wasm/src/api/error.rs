//! Core errors as JS exceptions: an `IcepoolError` (js/src/errors.ts) with the core's variant as `kind` and the
//! exception Python raises as `pythonType`, so that JS code can test either.

use icepool_core::Error;
use wasm_bindgen::JsValue;

use super::host;

/// The name of an error's variant (`IcepoolError.kind` in JS).
pub fn kind(e: &Error) -> &'static str {
    match e {
        Error::InvalidArgument(_) => "InvalidArgument",
        Error::WrongType(_) => "WrongType",
        Error::MissingAttribute(_) => "MissingAttribute",
        Error::IndexOutOfRange(_) => "IndexOutOfRange",
        Error::MissingKey(_) => "MissingKey",
        Error::Incomparable { .. } => "Incomparable",
        Error::Unhashable { .. } => "Unhashable",
        Error::DivisionByZero(_) => "DivisionByZero",
        Error::ConflictingOrder(_) => "ConflictingOrder",
        Error::UninferrableSize(_) => "UninferrableSize",
        Error::UnsupportedOrder(_) => "UnsupportedOrder",
        Error::RecursionLimit(_) => "RecursionLimit",
        Error::Overflow(_) => "Overflow",
        Error::TooLarge(_) => "TooLarge",
        Error::NotImplementedUpstream(_) => "NotImplementedUpstream",
        Error::Unported(_) => "Unported",
        Error::Callback(_) => "Callback",
    }
}

/// The exception Python raises for an error (`IcepoolError.pythonType` in JS), as the Python facade maps them
/// (crates/icepool-py `to_py_err`).
pub fn python_type(e: &Error) -> &'static str {
    match e {
        Error::InvalidArgument(_) => "ValueError",
        Error::WrongType(_) | Error::Incomparable { .. } | Error::Unhashable { .. } => "TypeError",
        Error::MissingAttribute(_) => "AttributeError",
        Error::IndexOutOfRange(_) => "IndexError",
        Error::MissingKey(_) => "KeyError",
        Error::DivisionByZero(_) => "ZeroDivisionError",
        Error::ConflictingOrder(_) => "ConflictingOrderError",
        Error::UninferrableSize(_) => "OrderError",
        Error::UnsupportedOrder(_) => "UnsupportedOrder",
        Error::RecursionLimit(_) => "RecursionError",
        Error::Overflow(_) => "OverflowError",
        Error::TooLarge(_) => "MemoryError",
        Error::NotImplementedUpstream(_) | Error::Unported(_) => "NotImplementedError",
        // A JS callback's own exception is rethrown as itself (see `callback`); this names the rest.
        Error::Callback(_) => "Exception",
    }
}

/// The JS exception for a core error.
pub fn to_js(e: Error) -> JsValue {
    host::make_error(kind(&e), python_type(&e), &e.to_string())
}

/// A core result as a wasm-bindgen one (an `Err` is thrown in JS).
pub type JsResult<T> = Result<T, JsValue>;

/// `?` on a core result inside an export.
pub trait OrThrow<T> {
    fn or_throw(self) -> JsResult<T>;
}

impl<T> OrThrow<T> for icepool_core::Result<T> {
    fn or_throw(self) -> JsResult<T> {
        self.map_err(to_js)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_and_python_types() {
        let e = Error::InvalidArgument("x".into());
        assert_eq!((kind(&e), python_type(&e)), ("InvalidArgument", "ValueError"));
        let e = Error::Unhashable { type_name: "list" };
        assert_eq!((kind(&e), python_type(&e)), ("Unhashable", "TypeError"));
        let e = Error::Unported("x".into());
        assert_eq!((kind(&e), python_type(&e)), ("Unported", "NotImplementedError"));
    }
}
