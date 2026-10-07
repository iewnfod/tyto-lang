use super::need_args;
use crate::{Interpreter, RtError, RtResult, Span, Value};

/// Stack / Queue 方法。
/// 空容器取值（pop / peek / pop_front / front / back）返回 EMPTY 哨兵，
/// 与堆一致（数组 pop 空才返回 null）。
pub fn call(
    receiver: &Value,
    name: &str,
    args: Vec<Value>,
    _interp: &mut Interpreter,
    span: Span,
) -> RtResult<Value> {
    match (receiver, name) {
        // ============ Stack（Vec，LIFO，顶在 Vec 尾部） ============
        (Value::Stack(s), "push") => {
            need_args("push", &args, 1, span)?;
            s.borrow_mut().push(args[0].clone());
            Ok(Value::Null)
        }
        (Value::Stack(s), "pop") => {
            need_args("pop", &args, 0, span)?;
            let popped = s.borrow_mut().pop();
            Ok(popped.unwrap_or(Value::Empty))
        }
        (Value::Stack(s), "peek") => {
            need_args("peek", &args, 0, span)?;
            let top = s.borrow().last().cloned();
            Ok(top.unwrap_or(Value::Empty))
        }
        // ============ Queue（VecDeque，FIFO，front 在队头） ============
        (Value::Queue(q), "push_back") => {
            need_args("push_back", &args, 1, span)?;
            q.borrow_mut().push_back(args[0].clone());
            Ok(Value::Null)
        }
        (Value::Queue(q), "pop_front") => {
            need_args("pop_front", &args, 0, span)?;
            let popped = q.borrow_mut().pop_front();
            Ok(popped.unwrap_or(Value::Empty))
        }
        (Value::Queue(q), "front") => {
            need_args("front", &args, 0, span)?;
            let v = q.borrow().front().cloned();
            Ok(v.unwrap_or(Value::Empty))
        }
        (Value::Queue(q), "back") => {
            need_args("back", &args, 0, span)?;
            let v = q.borrow().back().cloned();
            Ok(v.unwrap_or(Value::Empty))
        }
        // ============ 共通：len / is_empty ============
        (Value::Stack(s), "len") => {
            need_args("len", &args, 0, span)?;
            Ok(Value::Num(s.borrow().len() as f64))
        }
        (Value::Queue(q), "len") => {
            need_args("len", &args, 0, span)?;
            Ok(Value::Num(q.borrow().len() as f64))
        }
        (Value::Stack(s), "is_empty") => {
            need_args("is_empty", &args, 0, span)?;
            Ok(Value::Bool(s.borrow().is_empty()))
        }
        (Value::Queue(q), "is_empty") => {
            need_args("is_empty", &args, 0, span)?;
            Ok(Value::Bool(q.borrow().is_empty()))
        }
        (other, _) => Err(RtError::runtime(
            Some(span),
            format!("`{}` has no method `{}`", other.type_name(), name),
        )),
    }
}
