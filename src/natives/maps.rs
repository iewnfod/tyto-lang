use std::{cell::RefCell, rc::Rc};

use super::need_args;
use crate::value::MapKey;
use crate::{Interpreter, RtError, RtResult, Span, Value};

/// Map 方法：get insert contains_key remove len keys values isEmpty
pub fn call(
    receiver: &Value,
    name: &str,
    args: Vec<Value>,
    _interp: &mut Interpreter,
    span: Span,
) -> RtResult<Value> {
    let Value::Map(map) = receiver else { unreachable!() };
    match name {
        "get" => {
            need_args("get", &args, 1, span)?;
            let k = map_key(&args[0], span)?;
            let v = map.borrow().entries.get(&k).cloned();
            Ok(v.unwrap_or(Value::Null))
        }
        "insert" => {
            need_args("insert", &args, 2, span)?;
            let k = map_key(&args[0], span)?;
            map.borrow_mut().entries.insert(k, args[1].clone());
            Ok(receiver.clone()) // 链式
        }
        "contains_key" => {
            need_args("contains_key", &args, 1, span)?;
            let k = map_key(&args[0], span)?;
            Ok(Value::Bool(map.borrow().entries.contains_key(&k)))
        }
        "remove" => {
            need_args("remove", &args, 1, span)?;
            let k = map_key(&args[0], span)?;
            // shift_remove 保持插入顺序
            let removed = map.borrow_mut().entries.shift_remove(&k);
            Ok(Value::Bool(removed.is_some()))
        }
        "len" => {
            need_args("len", &args, 0, span)?;
            Ok(Value::Num(map.borrow().entries.len() as f64))
        }
        "is_empty" => {
            need_args("is_empty", &args, 0, span)?;
            Ok(Value::Bool(map.borrow().entries.is_empty()))
        }
        "keys" => {
            need_args("keys", &args, 0, span)?;
            let ks: Vec<Value> = map.borrow().entries.keys().map(|k| k.to_value()).collect();
            Ok(Value::Array(Rc::new(RefCell::new(ks))))
        }
        "values" => {
            need_args("values", &args, 0, span)?;
            let vs: Vec<Value> = map.borrow().entries.values().cloned().collect();
            Ok(Value::Array(Rc::new(RefCell::new(vs))))
        }
        _ => Err(RtError::runtime(Some(span), format!("`map` has no method `{}`", name))),
    }
}

fn map_key(v: &Value, span: Span) -> RtResult<MapKey> {
    MapKey::from_value(v).ok_or_else(|| {
        RtError::runtime(
            Some(span),
            format!("map keys must be number/string/bool/null, got {}", v.type_name()),
        )
    })
}
