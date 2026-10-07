pub mod arrays;
pub mod collections;
pub mod fns;
pub mod heaps;
pub mod maps;
pub mod strings;

use crate::{Interpreter, RtError, RtResult, Span, Value};

/// 全局原生函数入口
pub fn call_native(
    name: &str,
    interp: &mut Interpreter,
    args: Vec<Value>,
    span: Span,
) -> RtResult<Value> {
    match name {
        "print" | "println" => print_impl(name, interp, args, span),
        _ => fns::call_global(name, interp, args, span),
    }
}

/// 原生类型方法分派：`arr.push(1)`、`s.len()`、`heap.pop()`、`m.set(k, v)` 等。
/// 对象（Obj）的方法在解释器层处理（注入 self），不经过这里。
pub fn call_method(
    receiver: &Value,
    name: &str,
    args: Vec<Value>,
    interp: &mut Interpreter,
    span: Span,
) -> RtResult<Value> {
    match receiver {
        Value::Array(_) => arrays::call(receiver, name, args, interp, span),
        Value::Str(_) => strings::call(receiver, name, args, interp, span),
        Value::Map(_) => maps::call(receiver, name, args, interp, span),
        Value::MaxHeap(_) | Value::MinHeap(_) => heaps::call(receiver, name, args, interp, span),
        Value::Stack(_) | Value::Queue(_) => collections::call(receiver, name, args, interp, span),
        other => Err(RtError::runtime(
            Some(span),
            format!("`{}` has no method `{}`", other.type_name(), name),
        )),
    }
}

/// 调用用户函数值（map/filter/sort 比较器等回调）
pub(crate) fn call_user(
    interp: &mut Interpreter,
    f: &Value,
    args: Vec<Value>,
    span: Span,
) -> RtResult<Value> {
    interp.call_value_pub(f, args, span)
}

fn print_impl(
    name: &str,
    interp: &mut Interpreter,
    args: Vec<Value>,
    span: Span,
) -> RtResult<Value> {
    let mut out = interp.out.borrow_mut();
    for (i, a) in args.iter().enumerate() {
        if i > 0 {
            out.write_all(b" ").map_err(|e| RtError::runtime(Some(span), format!("io error: {}", e)))?;
        }
        out.write_all(a.to_display().as_bytes())
            .map_err(|e| RtError::runtime(Some(span), format!("io error: {}", e)))?;
    }
    if name == "println" {
        out.write_all(b"\n")
            .map_err(|e| RtError::runtime(Some(span), format!("io error: {}", e)))?;
    }
    Ok(Value::Null)
}

// ============ 参数检查工具 ============

pub(crate) fn need_args(name: &str, args: &[Value], n: usize, span: Span) -> RtResult<()> {
    if args.len() != n {
        Err(RtError::runtime(
            Some(span),
            format!("`{}()` expects {} argument(s), got {}", name, n, args.len()),
        ))
    } else {
        Ok(())
    }
}

pub(crate) fn need_num(name: &str, v: &Value, span: Span) -> RtResult<f64> {
    match v {
        Value::Num(n) => Ok(*n),
        other => Err(RtError::runtime(
            Some(span),
            format!("`{}()` expects a number, got {}", name, other.type_name()),
        )),
    }
}

pub(crate) fn need_str(name: &str, v: &Value, span: Span) -> RtResult<String> {
    match v {
        Value::Str(s) => Ok(s.clone()),
        other => Err(RtError::runtime(
            Some(span),
            format!("`{}()` expects a string, got {}", name, other.type_name()),
        )),
    }
}

/// 解析切片下标：支持负数（从末尾数），越界截断（JS slice 语义），小数截断
pub(crate) fn resolve_slice_index(i: f64, len: usize) -> usize {
    let len_f = len as f64;
    let i = if i < 0.0 { (len_f + i).max(0.0) } else { i.min(len_f) };
    i.trunc().max(0.0) as usize
}
