use super::{need_args, need_num};
use crate::value::MapKey;
use crate::{Interpreter, RtError, RtResult, Span, Value};

/// 全局函数：input / num / str / len / type / has / floor / ceil / round / abs / sqrt / pow / min / max
pub fn call_global(
    name: &str,
    interp: &mut Interpreter,
    args: Vec<Value>,
    span: Span,
) -> RtResult<Value> {
    match name {
        "input" => {
            need_args("input", &args, 0, span)?;
            let mut line = String::new();
            let n = interp
                .input
                .borrow_mut()
                .read_line(&mut line)
                .map_err(|e| RtError::runtime(Some(span), format!("io error: {}", e)))?;
            if n == 0 {
                Ok(Value::Null) // EOF
            } else {
                if line.ends_with('\n') {
                    line.pop();
                    if line.ends_with('\r') {
                        line.pop();
                    }
                }
                Ok(Value::Str(line))
            }
        }
        "num" => {
            need_args("num", &args, 1, span)?;
            match &args[0] {
                Value::Num(n) => Ok(Value::Num(*n)),
                Value::Str(s) => match s.trim().parse::<f64>() {
                    Ok(n) => Ok(Value::Num(n)),
                    Err(_) => Err(RtError::runtime(
                        Some(span),
                        format!("num(): cannot convert {:?} to number", s),
                    )),
                },
                other => Err(RtError::runtime(
                    Some(span),
                    format!("num(): expects a string or number, got {}", other.type_name()),
                )),
            }
        }
        "str" => {
            need_args("str", &args, 1, span)?;
            Ok(Value::Str(args[0].to_display()))
        }
        "len" => {
            need_args("len", &args, 1, span)?;
            let n = match &args[0] {
                Value::Array(a) => a.borrow().len(),
                Value::Str(s) => s.chars().count(),
                Value::Obj(o) => o.borrow().fields.len(),
                Value::Map(m) => m.borrow().entries.len(),
                Value::MaxHeap(h) => h.borrow().len(),
                Value::MinHeap(h) => h.borrow().len(),
                other => {
                    return Err(RtError::runtime(
                        Some(span),
                        format!("len(): cannot take len of {}", other.type_name()),
                    ))
                }
            };
            Ok(Value::Num(n as f64))
        }
        "type" => {
            need_args("type", &args, 1, span)?;
            Ok(Value::Str(args[0].type_name().to_string()))
        }
        "has" => {
            need_args("has", &args, 2, span)?;
            match (&args[0], &args[1]) {
                (Value::Obj(o), Value::Str(field)) => {
                    Ok(Value::Bool(o.borrow().fields.contains_key(field)))
                }
                (Value::Map(m), key) => {
                    let k = MapKey::from_value(key).ok_or_else(|| {
                        RtError::runtime(
                            Some(span),
                            format!("has(): invalid map key type {}", key.type_name()),
                        )
                    })?;
                    Ok(Value::Bool(m.borrow().entries.contains_key(&k)))
                }
                (other, _) => Err(RtError::runtime(
                    Some(span),
                    format!("has(): expects an object or map, got {}", other.type_name()),
                )),
            }
        }
        "floor" | "ceil" | "round" | "abs" | "sqrt" => {
            need_args(name, &args, 1, span)?;
            let x = need_num(name, &args[0], span)?;
            let v = match name {
                "floor" => x.floor(),
                "ceil" => x.ceil(),
                "round" => x.round(),
                "abs" => x.abs(),
                "sqrt" => x.sqrt(),
                _ => unreachable!(),
            };
            Ok(Value::Num(v))
        }
        "pow" => {
            need_args("pow", &args, 2, span)?;
            let a = need_num("pow", &args[0], span)?;
            let b = need_num("pow", &args[1], span)?;
            Ok(Value::Num(a.powf(b)))
        }
        "min" | "max" => {
            // min(3, 1, 2) 或 min([3, 1, 2])
            let items: Vec<Value> = if args.len() == 1 {
                match &args[0] {
                    Value::Array(a) => a.borrow().clone(),
                    other => {
                        return Err(RtError::runtime(
                            Some(span),
                            format!("`{}()` expects numbers or an array, got {}", name, other.type_name()),
                        ))
                    }
                }
            } else {
                args.clone()
            };
            if items.is_empty() {
                return Err(RtError::runtime(Some(span), format!("`{}()` of empty sequence", name)));
            }
            let mut best = need_num(name, &items[0], span)?;
            for item in &items[1..] {
                let n = need_num(name, item, span)?;
                let better = if name == "min" { n < best } else { n > best };
                if better {
                    best = n;
                }
            }
            Ok(Value::Num(best))
        }
        _ => Err(RtError::runtime(Some(span), format!("unknown function `{}`", name))),
    }
}
