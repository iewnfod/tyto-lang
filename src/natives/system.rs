use std::env;
use std::process::Command;

use super::{need_args, need_str};
use crate::{Interpreter, RtError, RtResult, Span, Value};

/// `sys` 命名空间：shell / get_env / args
pub fn call(name: &str, interp: &mut Interpreter, args: Vec<Value>, span: Span) -> RtResult<Value> {
    match name {
        "shell" => {
            need_args("sys.shell", &args, 1, span)?;
            let cmd = need_str("sys.shell", &args[0], span)?;
            let mut command = if cfg!(windows) {
                let mut c = Command::new("cmd");
                c.args(["/C", &cmd]);
                c
            } else {
                let mut c = Command::new("sh");
                c.arg("-c").arg(&cmd);
                c
            };
            let output = command
                .output()
                .map_err(|e| RtError::runtime(Some(span), format!("sys.shell({:?}): {}", cmd, e)))?;
            let status = output.status.code().unwrap_or(-1); // 被信号终止 → -1
            let mut obj = crate::value::ObjObj::default();
            obj.fields.insert("status".into(), Value::Num(status as f64));
            obj.fields
                .insert("stdout".into(), Value::Str(String::from_utf8_lossy(&output.stdout).into_owned()));
            obj.fields
                .insert("stderr".into(), Value::Str(String::from_utf8_lossy(&output.stderr).into_owned()));
            Ok(Value::Obj(std::rc::Rc::new(std::cell::RefCell::new(obj))))
        }
        "get_env" => {
            need_args("sys.get_env", &args, 1, span)?;
            let name = need_str("sys.get_env", &args[0], span)?;
            match env::var(&name) {
                Ok(v) => Ok(Value::Str(v)),
                Err(env::VarError::NotPresent) => Ok(Value::Null),
                Err(env::VarError::NotUnicode(v)) => Err(RtError::runtime(
                    Some(span),
                    format!("sys.get_env({:?}): environment variable is not valid UTF-8: {:?}", name, v),
                )),
            }
        }
        "args" => {
            need_args("sys.args", &args, 0, span)?;
            let items: Vec<Value> = interp.args.iter().cloned().map(Value::Str).collect();
            Ok(Value::Array(std::rc::Rc::new(std::cell::RefCell::new(items))))
        }
        _ => Err(RtError::runtime(Some(span), format!("`sys` has no function `{}`", name))),
    }
}
