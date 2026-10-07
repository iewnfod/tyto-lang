use super::{need_args, need_str, resolve_slice_index};
use crate::{Interpreter, RtError, RtResult, Span, Value};

/// 字符串方法：len split contains trim startsWith endsWith toUpper toLower sub replace chars indexOf isEmpty
pub fn call(
    receiver: &Value,
    name: &str,
    args: Vec<Value>,
    _interp: &mut Interpreter,
    span: Span,
) -> RtResult<Value> {
    let Value::Str(s) = receiver else { unreachable!() };
    match name {
        "len" => {
            need_args("len", &args, 0, span)?;
            Ok(Value::Num(s.chars().count() as f64))
        }
        "isEmpty" => {
            need_args("isEmpty", &args, 0, span)?;
            Ok(Value::Bool(s.is_empty()))
        }
        "split" => {
            need_args("split", &args, 1, span)?;
            let sep = need_str("split", &args[0], span)?;
            let parts: Vec<Value> = if sep.is_empty() {
                s.chars().map(|c| Value::Str(c.to_string())).collect()
            } else {
                s.split(&sep).map(|p| Value::Str(p.to_string())).collect()
            };
            Ok(Value::Array(std::rc::Rc::new(std::cell::RefCell::new(parts))))
        }
        "contains" => {
            need_args("contains", &args, 1, span)?;
            let sub = need_str("contains", &args[0], span)?;
            Ok(Value::Bool(s.contains(&sub)))
        }
        "indexOf" => {
            need_args("indexOf", &args, 1, span)?;
            let sub = need_str("indexOf", &args[0], span)?;
            let idx = s.find(&sub).map(|byte| s[..byte].chars().count()).map(|i| i as f64).unwrap_or(-1.0);
            Ok(Value::Num(idx))
        }
        "trim" => {
            need_args("trim", &args, 0, span)?;
            Ok(Value::Str(s.trim().to_string()))
        }
        "startsWith" => {
            need_args("startsWith", &args, 1, span)?;
            let sub = need_str("startsWith", &args[0], span)?;
            Ok(Value::Bool(s.starts_with(&sub)))
        }
        "endsWith" => {
            need_args("endsWith", &args, 1, span)?;
            let sub = need_str("endsWith", &args[0], span)?;
            Ok(Value::Bool(s.ends_with(&sub)))
        }
        "toUpper" => {
            need_args("toUpper", &args, 0, span)?;
            Ok(Value::Str(s.to_uppercase()))
        }
        "toLower" => {
            need_args("toLower", &args, 0, span)?;
            Ok(Value::Str(s.to_lowercase()))
        }
        "sub" => {
            if args.is_empty() || args.len() > 2 {
                return Err(RtError::runtime(
                    Some(span),
                    format!("`sub()` expects 1 or 2 arguments, got {}", args.len()),
                ));
            }
            let chars: Vec<char> = s.chars().collect();
            let start = resolve_slice_index(super::need_num("sub", &args[0], span)?, chars.len());
            let end = match args.get(1) {
                Some(v) => resolve_slice_index(super::need_num("sub", v, span)?, chars.len()),
                None => chars.len(),
            };
            let out: String =
                if start >= end { String::new() } else { chars[start..end].iter().collect() };
            Ok(Value::Str(out))
        }
        "replace" => {
            need_args("replace", &args, 2, span)?;
            let old = need_str("replace", &args[0], span)?;
            let new = need_str("replace", &args[1], span)?;
            Ok(Value::Str(s.replace(&old, &new)))
        }
        "chars" => {
            need_args("chars", &args, 0, span)?;
            let parts: Vec<Value> = s.chars().map(|c| Value::Str(c.to_string())).collect();
            Ok(Value::Array(std::rc::Rc::new(std::cell::RefCell::new(parts))))
        }
        _ => Err(RtError::runtime(
            Some(span),
            format!("`string` has no method `{}`", name),
        )),
    }
}
