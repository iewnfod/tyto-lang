mod calls;
mod exprs;
mod stmts;

#[cfg(test)]
mod tests;

use std::{cell::RefCell, io, rc::Rc};

use crate::ast::Stmt;
use crate::scope::{self, Scope, ScopeRef};
use crate::{NativeClass, RtError, RtResult, Value};

/// 输出/输入抽象：stdout 可捕获，便于测试
pub type OutRef = Rc<RefCell<dyn io::Write>>;
pub type InRef = Rc<RefCell<dyn io::BufRead>>;

/// 控制流信号
#[derive(Debug, Clone)]
pub enum Flow {
    Normal,
    Return(Value),
    Break,
    Continue,
}

pub struct Interpreter {
    pub global: ScopeRef,
    pub scope: ScopeRef,
    pub out: OutRef,
    pub input: InRef,
    /// 传给脚本的命令行参数（`tyto script.tyto a b` → ["a", "b"]），sys.args() 读取
    pub args: Vec<String>,
}

impl Default for Interpreter {
    fn default() -> Self {
        Self::new()
    }
}

impl Interpreter {
    pub fn new() -> Self {
        let out: OutRef = Rc::new(RefCell::new(io::stdout()));
        let input: InRef = Rc::new(RefCell::new(io::BufReader::new(io::stdin())));
        Self::with_io(out, input)
    }

    pub fn with_io(out: OutRef, input: InRef) -> Self {
        let global = Scope::root();
        let mut interp = Interpreter {
            global: global.clone(),
            scope: global,
            out,
            input,
            args: Vec::new(),
        };
        interp.install_globals();
        interp
    }

    fn install_globals(&mut self) {
        let g = &self.global;
        scope::define(g, "EMPTY", Value::Empty);
        scope::define(g, "inf", Value::Num(f64::INFINITY));
        scope::define(g, "nan", Value::Num(f64::NAN));
        scope::define(g, "MaxHeap", Value::NativeClass(NativeClass::MaxHeap));
        scope::define(g, "MinHeap", Value::NativeClass(NativeClass::MinHeap));
        scope::define(g, "Stack", Value::NativeClass(NativeClass::Stack));
        scope::define(g, "Queue", Value::NativeClass(NativeClass::Queue));
        scope::define(g, "Map", Value::NativeClass(NativeClass::Map));
        scope::define(g, "print", Value::NativeFn("print"));
        scope::define(g, "println", Value::NativeFn("println"));
        for name in [
            "input", "num", "str", "len", "type", "has", //
            "floor", "ceil", "round", "abs", "sqrt", "pow", "min", "max",
        ] {
            scope::define(g, name, Value::NativeFn(name));
        }

        // 命名空间对象：fs.*（文件）/ sys.*（系统）
        // 字段是原生函数，obj.f() 调用路径自然分派；v2 模块系统落地后可无缝升级
        let mut fs_obj = crate::value::ObjObj::default();
        for name in ["read_file", "read_lines", "write_file", "append_file", "exists", "list_dir"] {
            let full = format!("fs.{}", name);
            let full: &'static str = Box::leak(full.into_boxed_str());
            fs_obj.fields.insert(name.to_string(), Value::NativeFn(full));
        }
        scope::define(g, "fs", Value::Obj(Rc::new(RefCell::new(fs_obj))));

        let mut sys_obj = crate::value::ObjObj::default();
        for name in ["shell", "get_env", "args"] {
            let full = format!("sys.{}", name);
            let full: &'static str = Box::leak(full.into_boxed_str());
            sys_obj.fields.insert(name.to_string(), Value::NativeFn(full));
        }
        scope::define(g, "sys", Value::Obj(Rc::new(RefCell::new(sys_obj))));
    }

    /// 执行整段程序：顶层语句直接在当前（全局）作用域中运行
    pub fn run(&mut self, program: &Stmt) -> RtResult<()> {
        let stmts = match program {
            Stmt::Block { stmts, .. } => stmts,
            other => {
                return self.execute(other).map(|_| ());
            }
        };
        for stmt in stmts {
            match self.execute(stmt)? {
                Flow::Return(_) => {
                    return Err(RtError::runtime(Some(program.span()), "`return` outside function"))
                }
                _ => {}
            }
        }
        Ok(())
    }
}
