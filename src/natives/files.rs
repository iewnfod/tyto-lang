use std::fs as std_fs;
use std::io::Write;
use std::path::Path;

use super::{need_args, need_str};
use crate::{Interpreter, RtError, RtResult, Span, Value};

/// `fs` 命名空间：read_file / read_lines / write_file / append_file / exists / list_dir
/// 调用形式 `fs.read_file(path)`，经 Obj 字段中的 NativeFn 分派到这里。
pub fn call(name: &str, _interp: &mut Interpreter, args: Vec<Value>, span: Span) -> RtResult<Value> {
    match name {
        "read_file" => {
            need_args("fs.read_file", &args, 1, span)?;
            let path = need_str("fs.read_file", &args[0], span)?;
            match std_fs::read_to_string(&path) {
                Ok(s) => Ok(Value::Str(s)),
                Err(e) => Err(RtError::runtime(Some(span), format!("fs.read_file({:?}): {}", path, e))),
            }
        }
        "read_lines" => {
            need_args("fs.read_lines", &args, 1, span)?;
            let path = need_str("fs.read_lines", &args[0], span)?;
            let content = std_fs::read_to_string(&path)
                .map_err(|e| RtError::runtime(Some(span), format!("fs.read_lines({:?}): {}", path, e)))?;
            // Rust lines() 语义：去掉行尾 \n / \r\n，末尾换行不产生空尾元素
            let lines: Vec<Value> = content.lines().map(|l| Value::Str(l.to_string())).collect();
            Ok(Value::Array(std::rc::Rc::new(std::cell::RefCell::new(lines))))
        }
        "write_file" => {
            need_args("fs.write_file", &args, 2, span)?;
            let path = need_str("fs.write_file", &args[0], span)?;
            let content = need_str("fs.write_file", &args[1], span)?;
            std_fs::write(&path, content)
                .map_err(|e| RtError::runtime(Some(span), format!("fs.write_file({:?}): {}", path, e)))?;
            Ok(Value::Null)
        }
        "append_file" => {
            need_args("fs.append_file", &args, 2, span)?;
            let path = need_str("fs.append_file", &args[0], span)?;
            let content = need_str("fs.append_file", &args[1], span)?;
            let mut f = std_fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .map_err(|e| RtError::runtime(Some(span), format!("fs.append_file({:?}): {}", path, e)))?;
            f.write_all(content.as_bytes())
                .map_err(|e| RtError::runtime(Some(span), format!("fs.append_file({:?}): {}", path, e)))?;
            Ok(Value::Null)
        }
        "exists" => {
            need_args("fs.exists", &args, 1, span)?;
            let path = need_str("fs.exists", &args[0], span)?;
            Ok(Value::Bool(Path::new(&path).exists()))
        }
        "list_dir" => {
            need_args("fs.list_dir", &args, 1, span)?;
            let path = need_str("fs.list_dir", &args[0], span)?;
            let entries = std_fs::read_dir(&path)
                .map_err(|e| RtError::runtime(Some(span), format!("fs.list_dir({:?}): {}", path, e)))?;
            let mut names: Vec<String> = entries
                .filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            let items: Vec<Value> = names.into_iter().map(Value::Str).collect();
            Ok(Value::Array(std::rc::Rc::new(std::cell::RefCell::new(items))))
        }
        _ => Err(RtError::runtime(Some(span), format!("`fs` has no function `{}`", name))),
    }
}
