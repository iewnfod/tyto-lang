use std::{cell::RefCell, rc::Rc};

use super::need_args;
use crate::value::MapKey;
use crate::{Interpreter, RtError, RtResult, Span, Value};

/// Set 方法：add contains remove len is_empty to_array。
/// 元素合法性同 Map 键（number/string/bool/null，规范化一致：1 与 1.0 同元素）。
pub fn call(
    receiver: &Value,
    name: &str,
    args: Vec<Value>,
    _interp: &mut Interpreter,
    span: Span,
) -> RtResult<Value> {
    let Value::Set(set) = receiver else { unreachable!() };
    match name {
        "add" => {
            need_args("add", &args, 1, span)?;
            let k = set_elem(&args[0], span)?;
            set.borrow_mut().items.insert(k);
            Ok(receiver.clone()) // 链式
        }
        "contains" => {
            need_args("contains", &args, 1, span)?;
            let k = set_elem(&args[0], span)?;
            Ok(Value::Bool(set.borrow().items.contains(&k)))
        }
        "remove" => {
            need_args("remove", &args, 1, span)?;
            let k = set_elem(&args[0], span)?;
            // shift_remove 保持插入顺序
            Ok(Value::Bool(set.borrow_mut().items.shift_remove(&k)))
        }
        "len" => {
            need_args("len", &args, 0, span)?;
            Ok(Value::Num(set.borrow().items.len() as f64))
        }
        "is_empty" => {
            need_args("is_empty", &args, 0, span)?;
            Ok(Value::Bool(set.borrow().items.is_empty()))
        }
        "to_array" => {
            need_args("to_array", &args, 0, span)?;
            let vs: Vec<Value> = set.borrow().items.iter().map(|k| k.to_value()).collect();
            Ok(Value::Array(Rc::new(RefCell::new(vs))))
        }
        _ => Err(RtError::runtime(Some(span), format!("`set` has no method `{}`", name))),
    }
}

fn set_elem(v: &Value, span: Span) -> RtResult<MapKey> {
    MapKey::from_value(v).ok_or_else(|| {
        RtError::runtime(
            Some(span),
            format!("set elements must be number/string/bool/null, got {}", v.type_name()),
        )
    })
}
