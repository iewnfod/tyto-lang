use std::cmp::Reverse;

use super::{need_args, need_num};
use crate::value::HeapVal;
use crate::{Interpreter, RtError, RtResult, Span, Value};

/// 堆方法：push pop peek len isEmpty。
/// 空堆 pop/peek 返回 EMPTY（哨兵）。
pub fn call(
    receiver: &Value,
    name: &str,
    args: Vec<Value>,
    _interp: &mut Interpreter,
    span: Span,
) -> RtResult<Value> {
    match (receiver, name) {
        (Value::MaxHeap(h), "push") => {
            need_args("push", &args, 1, span)?;
            let n = need_num("push", &args[0], span)?;
            h.borrow_mut().push(HeapVal(n));
            Ok(Value::Null)
        }
        (Value::MinHeap(h), "push") => {
            need_args("push", &args, 1, span)?;
            let n = need_num("push", &args[0], span)?;
            h.borrow_mut().push(Reverse(HeapVal(n)));
            Ok(Value::Null)
        }
        (Value::MaxHeap(h), "pop") => {
            need_args("pop", &args, 0, span)?;
            let popped = h.borrow_mut().pop().map(|v| Value::Num(v.0));
            Ok(popped.unwrap_or(Value::Empty))
        }
        (Value::MinHeap(h), "pop") => {
            need_args("pop", &args, 0, span)?;
            let popped = h.borrow_mut().pop().map(|v| Value::Num(v.0 .0));
            Ok(popped.unwrap_or(Value::Empty))
        }
        (Value::MaxHeap(h), "peek") => {
            need_args("peek", &args, 0, span)?;
            let top = h.borrow().peek().map(|v| Value::Num(v.0));
            Ok(top.unwrap_or(Value::Empty))
        }
        (Value::MinHeap(h), "peek") => {
            need_args("peek", &args, 0, span)?;
            let top = h.borrow().peek().map(|v| Value::Num(v.0 .0));
            Ok(top.unwrap_or(Value::Empty))
        }
        (Value::MaxHeap(h), "len") => {
            need_args("len", &args, 0, span)?;
            Ok(Value::Num(h.borrow().len() as f64))
        }
        (Value::MinHeap(h), "len") => {
            need_args("len", &args, 0, span)?;
            Ok(Value::Num(h.borrow().len() as f64))
        }
        (Value::MaxHeap(h), "isEmpty") => {
            need_args("isEmpty", &args, 0, span)?;
            Ok(Value::Bool(h.borrow().is_empty()))
        }
        (Value::MinHeap(h), "isEmpty") => {
            need_args("isEmpty", &args, 0, span)?;
            Ok(Value::Bool(h.borrow().is_empty()))
        }
        (other, _) => Err(RtError::runtime(
            Some(span),
            format!("`{}` has no method `{}`", other.type_name(), name),
        )),
    }
}
