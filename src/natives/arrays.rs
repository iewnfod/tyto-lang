use std::{cell::RefCell, rc::Rc};

use super::{call_user, need_args, need_num, need_str, resolve_slice_index};
use crate::{Interpreter, RtError, RtResult, Span, Value};

/// 数组方法：push pop len isEmpty contains indexOf join sort reverse slice map filter reduce
pub fn call(
    receiver: &Value,
    name: &str,
    args: Vec<Value>,
    interp: &mut Interpreter,
    span: Span,
) -> RtResult<Value> {
    let Value::Array(arr) = receiver else { unreachable!() };
    match name {
        "push" => {
            let mut a = arr.borrow_mut();
            for v in args {
                a.push(v);
            }
            Ok(Value::Num(a.len() as f64))
        }
        "pop" => {
            need_args("pop", &args, 0, span)?;
            let popped = arr.borrow_mut().pop();
            Ok(popped.unwrap_or(Value::Null))
        }
        "len" => {
            need_args("len", &args, 0, span)?;
            Ok(Value::Num(arr.borrow().len() as f64))
        }
        "isEmpty" => {
            need_args("isEmpty", &args, 0, span)?;
            Ok(Value::Bool(arr.borrow().is_empty()))
        }
        "contains" => {
            need_args("contains", &args, 1, span)?;
            let hit = arr.borrow().iter().any(|v| v == &args[0]);
            Ok(Value::Bool(hit))
        }
        "indexOf" => {
            need_args("indexOf", &args, 1, span)?;
            let a = arr.borrow();
            let idx = a.iter().position(|v| v == &args[0]).map(|i| i as f64).unwrap_or(-1.0);
            Ok(Value::Num(idx))
        }
        "join" => {
            let sep = if args.is_empty() { ",".to_string() } else { need_str("join", &args[0], span)? };
            let a = arr.borrow();
            let parts: Vec<String> = a.iter().map(|v| v.to_display()).collect();
            Ok(Value::Str(parts.join(&sep)))
        }
        "sort" => sort(arr.clone(), args, interp, span),
        "reverse" => {
            need_args("reverse", &args, 0, span)?;
            arr.borrow_mut().reverse();
            Ok(receiver.clone())
        }
        "slice" => slice(arr.clone(), args, span),
        "map" => {
            need_args("map", &args, 1, span)?;
            let snapshot = arr.borrow().clone();
            let mut out = Vec::with_capacity(snapshot.len());
            for item in snapshot {
                out.push(call_user(interp, &args[0], vec![item], span)?);
            }
            Ok(Value::Array(Rc::new(RefCell::new(out))))
        }
        "filter" => {
            need_args("filter", &args, 1, span)?;
            let snapshot = arr.borrow().clone();
            let mut out = Vec::new();
            for item in snapshot {
                let keep = call_user(interp, &args[0], vec![item.clone()], span)?;
                if keep.truthy() {
                    out.push(item);
                }
            }
            Ok(Value::Array(Rc::new(RefCell::new(out))))
        }
        "reduce" => {
            // reduce(f, init)：args[0] 是回调，args[1] 是初值
            need_args("reduce", &args, 2, span)?;
            let snapshot = arr.borrow().clone();
            let mut acc = args[1].clone();
            for item in snapshot {
                acc = call_user(interp, &args[0], vec![acc, item], span)?;
            }
            Ok(acc)
        }
        _ => Err(RtError::runtime(
            Some(span),
            format!("`array` has no method `{}`", name),
        )),
    }
}

/// 就地排序并返回自身（可链式）。无比较器：全数字或全字符串；
/// 有比较器 f(a, b) → number（< 0 表示 a 在前）。
fn sort(
    arr: Rc<RefCell<Vec<Value>>>,
    args: Vec<Value>,
    interp: &mut Interpreter,
    span: Span,
) -> RtResult<Value> {
    // 把数据取出来排序，避免比较器回调里再借用数组导致 RefCell 冲突
    let mut data = std::mem::take(&mut *arr.borrow_mut());
    let result = (|| -> RtResult<()> {
        match args.first() {
            None => {
                let all_num = data.iter().all(|v| matches!(v, Value::Num(_)));
                let all_str = data.iter().all(|v| matches!(v, Value::Str(_)));
                if all_num {
                    data.sort_by(|a, b| num_cmp(a, b));
                } else if all_str {
                    data.sort_by(|a, b| str_cmp(a, b));
                } else {
                    return Err(RtError::runtime(
                        Some(span),
                        "sort() without comparator requires all numbers or all strings",
                    ));
                }
                Ok(())
            }
            Some(f) => {
                // 插入排序：错误可传播（脚本规模下性能足够）
                for i in 1..data.len() {
                    let mut j = i;
                    while j > 0 {
                        let cmp =
                            call_user(interp, f, vec![data[j - 1].clone(), data[j].clone()], span)?;
                        let n = need_num("sort comparator", &cmp, span)?;
                        if n > 0.0 {
                            data.swap(j - 1, j);
                            j -= 1;
                        } else {
                            break;
                        }
                    }
                }
                Ok(())
            }
        }
    })();
    *arr.borrow_mut() = data;
    result?;
    Ok(Value::Array(arr))
}

/// slice(start, end?) → 新数组；负数下标从末尾数，越界截断
fn slice(arr: Rc<RefCell<Vec<Value>>>, args: Vec<Value>, span: Span) -> RtResult<Value> {
    if args.is_empty() || args.len() > 2 {
        return Err(RtError::runtime(
            Some(span),
            format!("`slice()` expects 1 or 2 arguments, got {}", args.len()),
        ));
    }
    let a = arr.borrow();
    let start = resolve_slice_index(need_num("slice", &args[0], span)?, a.len());
    let end = match args.get(1) {
        Some(v) => resolve_slice_index(need_num("slice", v, span)?, a.len()),
        None => a.len(),
    };
    let items = if start >= end { vec![] } else { a[start..end].to_vec() };
    Ok(Value::Array(Rc::new(RefCell::new(items))))
}

fn num_cmp(a: &Value, b: &Value) -> std::cmp::Ordering {
    let (Value::Num(x), Value::Num(y)) = (a, b) else { unreachable!() };
    x.total_cmp(y)
}

fn str_cmp(a: &Value, b: &Value) -> std::cmp::Ordering {
    let (Value::Str(x), Value::Str(y)) = (a, b) else { unreachable!() };
    x.cmp(y)
}
