// Shared setup of the doctests in src/host/object.rs, pulled into their hidden lines by `include!` (a doctest
// cannot import another's items). A copy of the toy that `HostObject`'s own example shows in full: keep the two
// in step. Not part of the crate.

use icepool_core::host::{HostError, HostObject, HostRef, LenKind};
use icepool_core::value::{BinOp, UnOp};
use icepool_core::{Error, Result, Value};
use std::any::Any;
use std::sync::Arc;
#[derive(Debug)]
struct Pip(i64);
fn pip(v: &Value) -> Option<i64> {
    match v {
        Value::Host(h) => h.as_any().downcast_ref::<Pip>().map(|p| p.0),
        _ => None,
    }
}
fn host(x: i64) -> Value {
    Value::Host(HostRef::new(Pip(x)))
}
impl HostObject for Pip {
    fn hash_value(&self) -> i64 {
        self.0
    }
    fn eq(&self, a: &Value, b: &Value) -> bool {
        pip(a).is_some() && pip(a) == pip(b)
    }
    fn compare(&self, op: BinOp, a: &Value, b: &Value) -> Result<bool> {
        let (Some(x), Some(y)) = (pip(a), pip(b)) else {
            return Err(Error::WrongType("'<' not supported".into()));
        };
        Ok(match op {
            BinOp::Lt => x < y,
            BinOp::Le => x <= y,
            BinOp::Gt => x > y,
            _ => x >= y,
        })
    }
    fn binary(&self, op: BinOp, a: &Value, b: &Value) -> Result<Value> {
        match (op, pip(a), pip(b)) {
            (BinOp::Add, Some(x), Some(y)) => Ok(host(x + y)),
            _ => Err(Error::WrongType("unsupported operand type(s)".into())),
        }
    }
    fn unary(&self, op: UnOp) -> Result<Value> {
        match op {
            UnOp::Neg => Ok(host(-self.0)),
            _ => Err(Error::Callback(HostError(Arc::new("bad operand type")))),
        }
    }
    fn truthy(&self) -> Result<bool> {
        Ok(self.0 != 0)
    }
    fn repr(&self) -> String {
        format!("Pip({})", self.0)
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn to_float(&self) -> Result<f64> {
        Ok(self.0 as f64)
    }
    fn as_index(&self) -> Result<Option<dashu_int::IBig>> {
        Ok(Some(self.0.into()))
    }
    fn try_str(&self) -> Result<String> {
        Ok(format!("{} pips", self.0))
    }
    fn len_kind(&self) -> Result<LenKind> {
        Ok(LenKind::Unsized)
    }
}
