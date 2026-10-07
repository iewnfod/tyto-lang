use std::{cell::RefCell, collections::BinaryHeap, cmp::Reverse, io, rc::Rc};

use crate::ast::*;
use crate::natives;
use crate::scope::{self, Scope, ScopeRef};
use crate::value::{eq_value, FuncObj, HeapVal};
use crate::{NativeClass, RtError, RtResult, Span, Value};

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
        let mut interp = Interpreter { global: global.clone(), scope: global, out, input };
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
        scope::define(g, "Map", Value::NativeClass(NativeClass::Map));
        scope::define(g, "print", Value::NativeFn("print"));
        scope::define(g, "println", Value::NativeFn("println"));
        for name in [
            "input", "num", "str", "len", "type", "has", //
            "floor", "ceil", "round", "abs", "sqrt", "pow", "min", "max",
        ] {
            scope::define(g, name, Value::NativeFn(name));
        }
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

    // ============ 语句执行 ============

    pub fn execute(&mut self, stmt: &Stmt) -> RtResult<Flow> {
        match stmt {
            Stmt::Expr(expr, _) => {
                self.evaluate(expr)?;
                Ok(Flow::Normal)
            }
            Stmt::Assign { target, op, value, span } => self.exec_assign(target, *op, value, *span),
            Stmt::If { cond, then_block, else_block, .. } => {
                let c = self.evaluate(cond)?;
                if c.truthy() {
                    self.execute(then_block)
                } else if let Some(else_block) = else_block {
                    self.execute(else_block)
                } else {
                    Ok(Flow::Normal)
                }
            }
            Stmt::While { cond, body, .. } => self.exec_while(cond, body),
            Stmt::ForIn { var, iter, body, .. } => self.exec_for_in(var, iter, body),
            Stmt::ForC { var, init, cond, step, body, .. } => {
                self.exec_for_c(var, init, cond, step.as_deref(), body)
            }
            Stmt::FuncDecl { name, params, body, .. } => {
                let func = Value::Func(Rc::new(FuncObj {
                    name: name.clone(),
                    params: params.clone(),
                    body: body.clone(),
                    closure: self.scope.clone(),
                }));
                scope::define(&self.scope, name, func);
                Ok(Flow::Normal)
            }
            Stmt::Return { value, span } => {
                let v = match value {
                    Some(e) => self.evaluate(e)?,
                    None => Value::Null,
                };
                let _ = span;
                Ok(Flow::Return(v))
            }
            Stmt::Break(_) => Ok(Flow::Break),
            Stmt::Continue(_) => Ok(Flow::Continue),
            Stmt::Block { stmts, .. } => self.exec_block(stmts),
        }
    }

    fn exec_block(&mut self, stmts: &[Stmt]) -> RtResult<Flow> {
        let child = Scope::child(self.scope.clone());
        let saved = std::mem::replace(&mut self.scope, child);
        let mut result = Ok(Flow::Normal);
        for stmt in stmts {
            match self.execute(stmt) {
                Ok(Flow::Normal) => {}
                Ok(flow) => {
                    result = Ok(flow);
                    break;
                }
                Err(e) => {
                    result = Err(e);
                    break;
                }
            }
        }
        self.scope = saved;
        result
    }

    fn exec_assign(&mut self, target: &Expr, op: AssignOp, value: &Expr, span: Span) -> RtResult<Flow> {
        match target {
            Expr::Ident(name, _) => {
                let old = if op != AssignOp::Set {
                    Some(scope::get(&self.scope, name).map_err(|e| e.with_span(span))?)
                } else {
                    None
                };
                let v = self.assigned_value(op, old, value, span)?;
                scope::assign(&self.scope, name, v);
                Ok(Flow::Normal)
            }
            Expr::Index { target, index, .. } => {
                let container = self.evaluate(target)?;
                let idx = self.evaluate(index)?;
                let old = if op != AssignOp::Set {
                    Some(self.index_read(&container, &idx, span)?)
                } else {
                    None
                };
                let v = self.assigned_value(op, old, value, span)?;
                match &container {
                    Value::Array(arr) => {
                        let i = index_int(&idx, span)?;
                        let mut a = arr.borrow_mut();
                        if i >= a.len() {
                            return Err(RtError::runtime(
                                Some(span),
                                format!("index {} out of bounds (len {})", fmt_index(&idx), a.len()),
                            ));
                        }
                        a[i] = v;
                        Ok(Flow::Normal)
                    }
                    Value::Str(_) => Err(RtError::runtime(Some(span), "strings are immutable")),
                    other => Err(RtError::runtime(
                        Some(span),
                        format!("cannot index-assign into {}", other.type_name()),
                    )),
                }
            }
            Expr::Member { target, name, .. } => {
                let recv = self.evaluate(target)?;
                let old = if op != AssignOp::Set {
                    Some(self.get_member(&recv, name, span)?)
                } else {
                    None
                };
                let v = self.assigned_value(op, old, value, span)?;
                match &recv {
                    Value::Obj(obj) => {
                        obj.borrow_mut().fields.insert(name.clone(), v);
                        Ok(Flow::Normal)
                    }
                    other => Err(RtError::runtime(
                        Some(span),
                        format!("`{}` has no assignable field `{}`", other.type_name(), name),
                    )),
                }
            }
            // parser 已保证 target 只能是以上三种
            _ => Err(RtError::runtime(Some(span), "invalid assignment target")),
        }
    }

    /// 计算赋的值：普通赋值直接求值；复合赋值用预先读好的旧值做二元运算
    fn assigned_value(
        &mut self,
        op: AssignOp,
        old: Option<Value>,
        value: &Expr,
        span: Span,
    ) -> RtResult<Value> {
        if op == AssignOp::Set {
            return self.evaluate(value);
        }
        let old = old.unwrap_or_else(|| {
            // 复合赋值的目标不存在：视为运行时错误，由上层 span 报告
            Value::Null
        });
        let rhs = self.evaluate(value)?;
        let binop = match op {
            AssignOp::Add => BinaryOp::Add,
            AssignOp::Sub => BinaryOp::Sub,
            AssignOp::Mul => BinaryOp::Mul,
            AssignOp::Div => BinaryOp::Div,
            AssignOp::Mod => BinaryOp::Mod,
            AssignOp::Set => unreachable!(),
        };
        self.binary_op(old, binop, rhs, span)
    }

    fn exec_while(&mut self, cond: &Expr, body: &Stmt) -> RtResult<Flow> {
        loop {
            let c = self.evaluate(cond)?;
            if !c.truthy() {
                return Ok(Flow::Normal);
            }
            match self.execute(body)? {
                Flow::Break => return Ok(Flow::Normal),
                Flow::Return(v) => return Ok(Flow::Return(v)),
                _ => {}
            }
        }
    }

    fn exec_for_in(&mut self, var: &str, iter: &ForIter, body: &Stmt) -> RtResult<Flow> {
        // 循环变量定义在循环自己的作用域
        let loop_scope = Scope::child(self.scope.clone());
        let saved = std::mem::replace(&mut self.scope, loop_scope);
        let result = self.exec_for_in_inner(var, iter, body);
        self.scope = saved;
        result
    }

    fn exec_for_in_inner(&mut self, var: &str, iter: &ForIter, body: &Stmt) -> RtResult<Flow> {
        scope::define(&self.scope, var, Value::Null);
        match iter {
            ForIter::Range { start, end, inclusive } => {
                let s = self.evaluate(start)?;
                let e = self.evaluate(end)?;
                let (Value::Num(mut i), Value::Num(e)) = (s, e) else {
                    return Err(RtError::runtime(
                        Some(body.span()),
                        "range bounds must be numbers",
                    ));
                };
                loop {
                    let cont = if *inclusive { i <= e } else { i < e };
                    if !cont {
                        break;
                    }
                    scope::assign(&self.scope, var, Value::Num(i));
                    match self.execute(body)? {
                        Flow::Break => break,
                        Flow::Return(v) => return Ok(Flow::Return(v)),
                        _ => {}
                    }
                    i += 1.0;
                }
            }
            ForIter::Expr(expr) => {
                let val = self.evaluate(expr)?;
                match val {
                    Value::Array(arr) => {
                        let mut idx = 0usize;
                        loop {
                            let next = {
                                let a = arr.borrow();
                                if idx < a.len() {
                                    Some(a[idx].clone())
                                } else {
                                    None
                                }
                            };
                            let Some(item) = next else { break };
                            scope::assign(&self.scope, var, item);
                            match self.execute(body)? {
                                Flow::Break => break,
                                Flow::Return(v) => return Ok(Flow::Return(v)),
                                _ => {}
                            }
                            idx += 1;
                        }
                    }
                    Value::Str(s) => {
                        for c in s.chars() {
                            scope::assign(&self.scope, var, Value::Str(c.to_string()));
                            match self.execute(body)? {
                                Flow::Break => break,
                                Flow::Return(v) => return Ok(Flow::Return(v)),
                                _ => {}
                            }
                        }
                    }
                    other => {
                        return Err(RtError::runtime(
                            Some(expr.span()),
                            format!("cannot iterate over {}", other.type_name()),
                        ))
                    }
                }
            }
        }
        Ok(Flow::Normal)
    }

    fn exec_for_c(
        &mut self,
        var: &str,
        init: &Expr,
        cond: &Option<Expr>,
        step: Option<&Stmt>,
        body: &Stmt,
    ) -> RtResult<Flow> {
        let loop_scope = Scope::child(self.scope.clone());
        let saved = std::mem::replace(&mut self.scope, loop_scope);
        let result = self.exec_for_c_inner(var, init, cond, step, body);
        self.scope = saved;
        result
    }

    fn exec_for_c_inner(
        &mut self,
        var: &str,
        init: &Expr,
        cond: &Option<Expr>,
        step: Option<&Stmt>,
        body: &Stmt,
    ) -> RtResult<Flow> {
        let init_val = self.evaluate(init)?;
        scope::define(&self.scope, var, init_val);
        loop {
            if let Some(c) = cond {
                let v = self.evaluate(c)?;
                if !v.truthy() {
                    break;
                }
            }
            match self.execute(body)? {
                Flow::Break => break,
                Flow::Return(v) => return Ok(Flow::Return(v)),
                Flow::Continue | Flow::Normal => {}
            }
            if let Some(s) = step {
                self.execute(s)?;
            }
        }
        Ok(Flow::Normal)
    }

    // ============ 表达式求值 ============

    pub fn evaluate(&mut self, expr: &Expr) -> RtResult<Value> {
        match expr {
            Expr::Num(n, _) => Ok(Value::Num(*n)),
            Expr::Str(s, _) => Ok(Value::Str(s.clone())),
            Expr::Bool(b, _) => Ok(Value::Bool(*b)),
            Expr::Null(_) => Ok(Value::Null),
            Expr::Ident(name, span) => {
                scope::get(&self.scope, name).map_err(|e| e.with_span(*span))
            }
            Expr::Array(elements, _) => {
                let mut v = Vec::with_capacity(elements.len());
                for e in elements {
                    v.push(self.evaluate(e)?);
                }
                Ok(Value::Array(Rc::new(RefCell::new(v))))
            }
            Expr::Object(fields, _) => {
                let mut obj = crate::value::ObjObj::default();
                for (k, e) in fields {
                    let v = self.evaluate(e)?;
                    obj.fields.insert(k.clone(), v);
                }
                Ok(Value::Obj(Rc::new(RefCell::new(obj))))
            }
            Expr::Unary { op, operand, span } => {
                let v = self.evaluate(operand)?;
                match op {
                    UnaryOp::Neg => match v {
                        Value::Num(n) => Ok(Value::Num(-n)),
                        other => Err(RtError::runtime(
                            Some(*span),
                            format!("cannot negate {}", other.type_name()),
                        )),
                    },
                    UnaryOp::Not => Ok(Value::Bool(!v.truthy())),
                }
            }
            Expr::Binary { left, op, right, span } => {
                let l = self.evaluate(left)?;
                let r = self.evaluate(right)?;
                self.binary_op(l, *op, r, *span)
            }
            Expr::Logic { left, op, right, .. } => {
                // 短路，返回操作数（JS 式）
                let l = self.evaluate(left)?;
                let take_left = match op {
                    LogicOp::Or => l.truthy(),
                    LogicOp::And => !l.truthy(),
                };
                if take_left {
                    Ok(l)
                } else {
                    self.evaluate(right)
                }
            }
            Expr::Ternary { cond, then_expr, else_expr, .. } => {
                let c = self.evaluate(cond)?;
                if c.truthy() {
                    self.evaluate(then_expr)
                } else {
                    self.evaluate(else_expr)
                }
            }
            Expr::Index { target, index, span } => {
                let t = self.evaluate(target)?;
                let i = self.evaluate(index)?;
                self.index_read(&t, &i, *span)
            }
            Expr::Member { target, name, span } => {
                let recv = self.evaluate(target)?;
                self.get_member(&recv, name, *span)
            }
            Expr::OptionalMember { target, name, span } => {
                let recv = self.evaluate(target)?;
                if matches!(recv, Value::Null) {
                    Ok(Value::Null)
                } else {
                    self.get_member(&recv, name, *span)
                }
            }
            Expr::Call { callee, args, span } => {
                let mut argv = Vec::with_capacity(args.len());
                for a in args {
                    argv.push(self.evaluate(a)?);
                }
                match &**callee {
                    Expr::Member { target, name, .. } => {
                        let receiver = self.evaluate(target)?;
                        self.call_member(receiver, name, argv, *span)
                    }
                    Expr::OptionalMember { target, name, .. } => {
                        let receiver = self.evaluate(target)?;
                        if matches!(receiver, Value::Null) {
                            Ok(Value::Null)
                        } else {
                            self.call_member(receiver, name, argv, *span)
                        }
                    }
                    _ => {
                        let callee_val = self.evaluate(callee)?;
                        self.call_value(&callee_val, argv, None, *span)
                    }
                }
            }
            Expr::New { class, args, span } => {
                let cv = self.evaluate(class)?;
                self.construct(&cv, args, *span)
            }
            Expr::Function { params, body, .. } => {
                Ok(Value::Func(Rc::new(FuncObj {
                    name: String::new(),
                    params: params.clone(),
                    body: body.clone(),
                    closure: self.scope.clone(),
                })))
            }
        }
    }

    fn binary_op(&self, l: Value, op: BinaryOp, r: Value, span: Span) -> RtResult<Value> {
        use BinaryOp::*;
        match op {
            Add => match (&l, &r) {
                (Value::Num(a), Value::Num(b)) => Ok(Value::Num(a + b)),
                (Value::Str(_), _) | (_, Value::Str(_)) => {
                    Ok(Value::Str(format!("{}{}", l.to_display(), r.to_display())))
                }
                (Value::Array(a), Value::Array(b)) => {
                    let mut v = a.borrow().clone();
                    v.extend(b.borrow().iter().cloned());
                    Ok(Value::Array(Rc::new(RefCell::new(v))))
                }
                _ => Err(RtError::runtime(
                    Some(span),
                    format!("cannot add {} and {}", l.type_name(), r.type_name()),
                )),
            },
            Sub | Mul | Div | Mod => {
                let (Value::Num(a), Value::Num(b)) = (&l, &r) else {
                    return Err(RtError::runtime(
                        Some(span),
                        format!(
                            "`{}` requires numbers, got {} and {}",
                            op_symbol(op),
                            l.type_name(),
                            r.type_name()
                        ),
                    ));
                };
                let v = match op {
                    Sub => a - b,
                    Mul => a * b,
                    Div => a / b, // JS 式：1/0 = inf，0/0 = nan，不报错
                    Mod => a % b,
                    _ => unreachable!(),
                };
                Ok(Value::Num(v))
            }
            Lt | Gt | Lte | Gte => {
                let ord = match (&l, &r) {
                    (Value::Num(a), Value::Num(b)) => a.partial_cmp(b),
                    (Value::Str(a), Value::Str(b)) => Some(a.cmp(b)),
                    _ => {
                        return Err(RtError::runtime(
                            Some(span),
                            format!(
                                "`{}` requires two numbers or two strings, got {} and {}",
                                op_symbol(op),
                                l.type_name(),
                                r.type_name()
                            ),
                        ))
                    }
                };
                let Some(ord) = ord else {
                    return Ok(Value::Bool(false)); // 含 nan 的比较一律 false（JS 式）
                };
                use std::cmp::Ordering::*;
                let b = match op {
                    Lt => ord == Less,
                    Gt => ord == Greater,
                    Lte => ord != Greater,
                    Gte => ord != Less,
                    _ => unreachable!(),
                };
                Ok(Value::Bool(b))
            }
            Eq => Ok(Value::Bool(eq_value(&l, &r))),
            Neq => Ok(Value::Bool(!eq_value(&l, &r))),
        }
    }

    fn index_read(&self, container: &Value, index: &Value, span: Span) -> RtResult<Value> {
        match container {
            Value::Array(arr) => {
                let i = index_int(index, span)?;
                let a = arr.borrow();
                if i >= a.len() {
                    Err(RtError::runtime(
                        Some(span),
                        format!("index {} out of bounds (len {})", fmt_index(index), a.len()),
                    ))
                } else {
                    Ok(a[i].clone())
                }
            }
            Value::Str(s) => {
                let i = index_int(index, span)?;
                s.chars()
                    .nth(i)
                    .map(|c| Value::Str(c.to_string()))
                    .ok_or_else(|| {
                        RtError::runtime(
                            Some(span),
                            format!("index {} out of bounds (len {})", fmt_index(index), s.chars().count()),
                        )
                    })
            }
            other => Err(RtError::runtime(
                Some(span),
                format!("cannot index {} with {}", other.type_name(), index.type_name()),
            )),
        }
    }

    /// 成员读取（非调用）：只有对象有字段；原生类型的方法必须以调用形式出现
    fn get_member(&self, recv: &Value, name: &str, span: Span) -> RtResult<Value> {
        match recv {
            Value::Obj(obj) => obj.borrow().fields.get(name).cloned().ok_or_else(|| {
                RtError::runtime(Some(span), format!("object has no field `{}`", name))
            }),
            other => Err(RtError::runtime(
                Some(span),
                format!(
                    "`{}` has no field `{}`; methods must be called, e.g. `{}.{}()`",
                    other.type_name(),
                    name,
                    other.type_name(),
                    name
                ),
            )),
        }
    }

    // ============ 调用与构造 ============

    fn call_member(
        &mut self,
        receiver: Value,
        name: &str,
        args: Vec<Value>,
        span: Span,
    ) -> RtResult<Value> {
        match &receiver {
            // 对象：字段里的函数 → 注入 self 调用
            Value::Obj(obj) => {
                let field = obj.borrow().fields.get(name).cloned().ok_or_else(|| {
                    RtError::runtime(Some(span), format!("object has no method `{}`", name))
                })?;
                self.call_value(&field, args, Some(receiver), span)
            }
            // 原生类型方法分派
            _ => natives::call_method(&receiver, name, args, self, span),
        }
    }

    /// 供 natives 回调用户函数（map/filter/sort 比较器等）
    pub(crate) fn call_value_pub(
        &mut self,
        callee: &Value,
        args: Vec<Value>,
        span: Span,
    ) -> RtResult<Value> {
        self.call_value(callee, args, None, span)
    }

    fn call_value(
        &mut self,
        callee: &Value,
        args: Vec<Value>,
        receiver: Option<Value>,
        span: Span,
    ) -> RtResult<Value> {
        match callee {
            Value::Func(f) => self.call_function(f, args, receiver, span),
            Value::NativeFn(name) => natives::call_native(name, self, args, span),
            Value::NativeClass(c) => Err(RtError::runtime(
                Some(span),
                format!("`{}` is a class, use `new {}(...)`", c.name(), c.name()),
            )),
            other => Err(RtError::runtime(
                Some(span),
                format!("{} is not callable", other.type_name()),
            )),
        }
    }

    fn call_function(
        &mut self,
        f: &Rc<FuncObj>,
        args: Vec<Value>,
        receiver: Option<Value>,
        span: Span,
    ) -> RtResult<Value> {
        if args.len() != f.params.len() {
            let name = if f.name.is_empty() { "<anonymous>" } else { &f.name };
            return Err(RtError::runtime(
                Some(span),
                format!(
                    "function `{}` expects {} argument(s), got {}",
                    name,
                    f.params.len(),
                    args.len()
                ),
            ));
        }
        let call_scope = Scope::child(f.closure.clone());
        for (p, v) in f.params.iter().zip(args) {
            scope::define(&call_scope, p.clone(), v);
        }
        // 仅 `obj.f()` 形式的调用注入 self
        if let Some(recv) = receiver {
            scope::define(&call_scope, "self", recv);
        }
        let saved = std::mem::replace(&mut self.scope, call_scope);
        let result = self.execute(&f.body);
        self.scope = saved;
        match result? {
            Flow::Return(v) => Ok(v),
            _ => Ok(Value::Null),
        }
    }

    fn construct(&mut self, class: &Value, args: &[Expr], span: Span) -> RtResult<Value> {
        let Value::NativeClass(kind) = class else {
            return Err(RtError::runtime(
                Some(span),
                "only native classes (Map / MaxHeap / MinHeap) can be constructed in v1",
            ));
        };
        match kind {
            NativeClass::Map => {
                if !args.is_empty() {
                    return Err(RtError::runtime(
                        Some(span),
                        "Map() takes no arguments (v1)",
                    ));
                }
                Ok(Value::Map(Rc::new(RefCell::new(crate::value::MapObj::default()))))
            }
            NativeClass::MaxHeap | NativeClass::MinHeap => {
                let items: Vec<Value> = match args {
                    [] => vec![],
                    [one] => {
                        let v = self.evaluate(one)?;
                        match v {
                            Value::Array(a) => a.borrow().clone(),
                            other => {
                                return Err(RtError::runtime(
                                    Some(span),
                                    format!(
                                        "{}() takes no arguments or an array, got {}",
                                        kind.name(),
                                        other.type_name()
                                    ),
                                ))
                            }
                        }
                    }
                    _ => {
                        return Err(RtError::runtime(
                            Some(span),
                            format!("{}() takes 0 or 1 argument", kind.name()),
                        ))
                    }
                };
                let mut heap: BinaryHeap<HeapVal> = BinaryHeap::new();
                for item in items {
                    let Value::Num(n) = item else {
                        return Err(RtError::runtime(
                            Some(span),
                            format!("heaps hold numbers, got {}", item.type_name()),
                        ));
                    };
                    heap.push(HeapVal(n));
                }
                Ok(match kind {
                    NativeClass::MaxHeap => Value::MaxHeap(Rc::new(RefCell::new(heap))),
                    _ => Value::MinHeap(Rc::new(RefCell::new(heap.into_iter().map(Reverse).collect()))),
                })
            }
        }
    }
}

/// 索引必须是整数值的 number
fn index_int(index: &Value, span: Span) -> RtResult<usize> {
    match index {
        Value::Num(n) if n.fract() == 0.0 && *n >= 0.0 && *n < 9.007_199_254_740_992e15 => {
            Ok(*n as usize)
        }
        other => Err(RtError::runtime(
            Some(span),
            format!("index must be a non-negative integer, got {}", fmt_index(other)),
        )),
    }
}

fn fmt_index(v: &Value) -> String {
    match v {
        Value::Num(n) => crate::value::fmt_num(*n),
        other => format!("{} {}", other.type_name(), other.to_repr()),
    }
}

fn op_symbol(op: BinaryOp) -> &'static str {
    use BinaryOp::*;
    match op {
        Add => "+",
        Sub => "-",
        Mul => "*",
        Div => "/",
        Mod => "%",
        Lt => "<",
        Gt => ">",
        Lte => "<=",
        Gte => ">=",
        Eq => "==",
        Neq => "!=",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;
    use crate::parser::Parser;
    use std::io::{BufReader, Cursor};

    /// 运行源码并捕获 stdout
    fn run(src: &str) -> String {
        let vec: Rc<RefCell<Vec<u8>>> = Rc::new(RefCell::new(Vec::new()));
        let out: OutRef = vec.clone();
        let input: InRef = Rc::new(RefCell::new(BufReader::new(Cursor::new(Vec::<u8>::new()))));
        let mut interp = Interpreter::with_io(out, input);
        let tokens = Lexer::new(src).tokenize().unwrap().tokens;
        let program = Parser::new(tokens).parse_program().unwrap();
        interp.run(&program).expect("runtime ok");
        String::from_utf8(vec.borrow().clone()).unwrap()
    }

    fn run_err(src: &str) -> RtError {
        let vec: Rc<RefCell<Vec<u8>>> = Rc::new(RefCell::new(Vec::new()));
        let out: OutRef = vec.clone();
        let input: InRef = Rc::new(RefCell::new(BufReader::new(Cursor::new(Vec::<u8>::new()))));
        let mut interp = Interpreter::with_io(out, input);
        let tokens = match Lexer::new(src).tokenize() {
            Ok(o) => o.tokens,
            Err(e) => return e,
        };
        let program = match Parser::new(tokens).parse_program() {
            Ok(p) => p,
            Err(e) => return e,
        };
        interp.run(&program).expect_err("expected error")
    }

    #[test]
    fn print_and_println() {
        assert_eq!(run("print(\"hello\")"), "hello");
        assert_eq!(run("println(\"hello\")"), "hello\n");
        assert_eq!(run("println(1, \"a\", true)"), "1 a true\n");
        assert_eq!(run("print()"), "");
    }

    #[test]
    fn arithmetic_and_js_number() {
        assert_eq!(run("println(1 + 2 * 3)"), "7\n");
        assert_eq!(run("println((1 + 2) / 2)"), "1.5\n");
        assert_eq!(run("println(7 % 3)"), "1\n");
        assert_eq!(run("println(1 / 0)"), "inf\n");
        assert_eq!(run("println(0 / 0)"), "nan\n");
        assert_eq!(run("println(2.0)"), "2\n");
        assert_eq!(run("println(-3)"), "-3\n");
    }

    #[test]
    fn string_concat_with_anything() {
        assert_eq!(run("println(\"a\" + 1 + true)"), "a1true\n");
        assert_eq!(run("println(1 + \"a\")"), "1a\n");
        assert_eq!("a" < "b", true);
        assert_eq!(run("println(\"abc\" < \"abd\")"), "true\n");
    }

    #[test]
    fn equality_no_coercion() {
        assert_eq!(run("println(1 == \"1\")"), "false\n");
        assert_eq!(run("println(1 == 1.0)"), "true\n");
        assert_eq!(run("println(null == null)"), "true\n");
        assert_eq!(run("println(null == 0)"), "false\n");
        assert_eq!(run("println(EMPTY == EMPTY)"), "true\n");
    }

    #[test]
    fn scoping_penetrates_to_global() {
        // 伪代码式用法：全局 h_low 在函数内被赋值
        let src = "h_low = null\n\
                   function Init() {\n\
                   \x20   h_low = 42\n\
                   }\n\
                   Init()\n\
                   println(h_low)";
        assert_eq!(run(src), "42\n");
    }

    #[test]
    fn local_creation_inside_function() {
        let src = "x = 1\n\
                   function F() {\n\
                   \x20   y = 2\n\
                   \x20   println(y)\n\
                   }\n\
                   F()\n\
                   println(x)";
        assert_eq!(run(src), "2\n1\n");
        // 函数内新建的 y 不泄漏到全局
        let src2 = "function F() { y = 2 }\nF()\nprintln(y)";
        assert!(matches!(run_err(src2), RtError::Runtime { .. }));
    }

    #[test]
    fn if_body_has_own_scope() {
        let src = "x = 1\nif true { y = 2\nprintln(y) }\nprintln(x)";
        assert_eq!(run(src), "2\n1\n");
        // y 不存在于 if 外
        let src2 = "if true { y = 2 }\nprintln(y)";
        assert!(matches!(run_err(src2), RtError::Runtime { .. }));
    }

    #[test]
    fn closures_share_state_by_reference() {
        let src = "function make_counter() {\n\
                   \x20   n = 0\n\
                   \x20   return function() { n += 1\nreturn n }\n\
                   }\n\
                   c = make_counter()\n\
                   println(c())\n\
                   println(c())";
        assert_eq!(run(src), "1\n2\n");
    }

    #[test]
    fn recursion_fib() {
        let src = "function fib(n) {\n\
                   \x20   if n < 2 { return n }\n\
                   \x20   return fib(n - 1) + fib(n - 2)\n\
                   }\n\
                   println(fib(10))";
        assert_eq!(run(src), "55\n");
    }

    #[test]
    fn anonymous_function_value() {
        assert_eq!(run("f = function(x) { return x * 2 }\nprintln(f(3))"), "6\n");
        // 高阶：返回闭包
        assert_eq!(
            run("function add(a) { return function(b) { return a + b } }\nprintln(add(2)(3))"),
            "5\n"
        );
    }

    #[test]
    fn if_else_if_else_chain() {
        let src = "x = 5\n\
                   if x > 10 { println(\"a\") } else if x > 3 { println(\"b\") } else { println(\"c\") }";
        assert_eq!(run(src), "b\n");
        let src2 = "x = 1\nif x > 10 { println(\"a\") } else if x > 3 { println(\"b\") } else { println(\"c\") }";
        assert_eq!(run(src2), "c\n");
    }

    #[test]
    fn while_loop() {
        let src = "i = 0\ns = 0\nwhile i < 5 { s += i\ni += 1 }\nprintln(s)";
        assert_eq!(run(src), "10\n");
    }

    #[test]
    fn for_in_over_array_range_string() {
        let src = "total = 0\n\
                   for x in [1, 2, 3] { total += x }\n\
                   println(total)\n\
                   for i in 0..5 { total += i }\n\
                   println(total)\n\
                   for c in \"ab\" { println(c) }";
        assert_eq!(run(src), "6\n16\na\nb\n");
        // ..= 包含端点
        assert_eq!(run("s = 0\nfor i in 1..=3 { s += i }\nprintln(s)"), "6\n");
    }

    #[test]
    fn c_style_for_with_break_continue() {
        // 0+1+2+4+5 = 12（跳过 3，遇 6 中断）
        let src = "s = 0\nfor i = 0; i < 10; i += 1 {\n\
                   \x20   if i == 3 { continue }\n\
                   \x20   if i == 6 { break }\n\
                   \x20   s += i\n\
                   }\n\
                   println(s)";
        assert_eq!(run(src), "12\n");
    }

    #[test]
    fn ternary_and_short_circuit() {
        assert_eq!(run("println(true ? 1 : 2)"), "1\n");
        assert_eq!(run("println(0 || \"x\")"), "x\n");
        assert_eq!(run("println(1 && 2)"), "2\n");
        // 短路：右侧不求值，不会报 undefined
        assert_eq!(run("println(false && undefined_var == 1)"), "false\n");
        assert_eq!(run("println(true || undefined_var == 1)"), "true\n");
    }

    #[test]
    fn arrays_read_write_concat() {
        let src = "a = [1, 2, 3]\na[1] = 9\nprintln(a)\nprintln(a[0])\nprintln(a + [4])";
        assert_eq!(run(src), "[1, 9, 3]\n1\n[1, 9, 3, 4]\n");
        // 越界读/写都报错
        assert!(matches!(run_err("a = [1]\nprintln(a[5])"), RtError::Runtime { .. }));
        assert!(matches!(run_err("a = [1]\na[5] = 0"), RtError::Runtime { .. }));
        // 字符串不可变
        assert!(matches!(run_err("s = \"ab\"\ns[0] = \"c\""), RtError::Runtime { .. }));
        // 字符串索引
        assert_eq!(run("println(\"abc\"[1])"), "b\n");
    }

    #[test]
    fn undefined_variable_has_span() {
        let err = run_err("println(z)");
        match err {
            RtError::Runtime { span: Some(s), message } => {
                assert!(message.contains("undefined variable `z`"), "{}", message);
                assert_eq!(s.line, 1);
            }
            other => panic!("expected runtime error, got {:?}", other),
        }
    }

    #[test]
    fn return_outside_function_is_error() {
        assert!(matches!(run_err("return 1"), RtError::Runtime { .. }));
    }

    #[test]
    fn arity_mismatch_is_error() {
        let err = run_err("function f(a) { return a }\nf(1, 2)");
        assert!(matches!(err, RtError::Runtime { ref message, .. } if message.contains("expects 1 argument")));
    }

    #[test]
    fn native_class_globals() {
        assert_eq!(run("println(EMPTY)"), "EMPTY\n");
        assert_eq!(run("println(inf)"), "inf\n");
        // 类不能直接调用
        let err = run_err("MaxHeap()");
        assert!(matches!(err, RtError::Runtime { ref message, .. } if message.contains("use `new MaxHeap")));
    }

    #[test]
    fn new_heap_from_array() {
        // 堆方法在任务 7 实现，这里只验证构造不炸
        assert_eq!(run("h = new MaxHeap()\nprintln(h)"), "MaxHeap[]\n");
        assert_eq!(run("h = new MaxHeap([3, 1])\nprintln(h)"), "MaxHeap[3, 1]\n");
        assert_eq!(run("h = new MinHeap([3, 1])\nprintln(h)"), "MinHeap[1, 3]\n");
        // 非数组参数报错
        assert!(matches!(run_err("new MaxHeap(3)"), RtError::Runtime { .. }));
        // new 非类报错
        assert!(matches!(run_err("new f()"), RtError::Runtime { .. }));
    }

    #[test]
    fn nested_break_inside_function() {
        // return 从循环内穿越
        let src = "function first_even(xs) {\n\
                   \x20   for x in xs {\n\
                   \x20       if x % 2 == 0 { return x }\n\
                   \x20   }\n\
                   \x20   return EMPTY\n\
                   }\n\
                   println(first_even([1, 3, 4, 6]))\n\
                   println(first_even([1, 3]))";
        assert_eq!(run(src), "4\nEMPTY\n");
    }

    // ============ 对象与 self ============

    #[test]
    fn object_literal_read_write_autocreate() {
        let src = "p = {x: 1, y: 2}\n\
                   println(p.x)\n\
                   p.y = p.y + 10\n\
                   println(p.y)\n\
                   p.z = 3\n\
                   println(p.z)\n\
                   println(p)";
        assert_eq!(run(src), "1\n12\n3\n{x: 1, y: 12, z: 3}\n");
        assert_eq!(run("println({})"), "{}\n");
    }

    #[test]
    fn object_compound_assign() {
        assert_eq!(run("p = {n: 5}\np.n += 2\nprintln(p.n)"), "7\n");
    }

    #[test]
    fn nested_objects_share_references() {
        let src = "inner = {v: 1}\n\
                   a = {c: inner}\n\
                   b = {c: inner}\n\
                   a.c.v = 42\n\
                   println(b.c.v)\n\
                   println(a.c == b.c)";
        assert_eq!(run(src), "42\ntrue\n");
    }

    #[test]
    fn self_binding_in_method_call() {
        let src = "counter = {\n\
                   \x20   n: 0,\n\
                   \x20   inc: function() {\n\
                   \x20       self.n += 1\n\
                   \x20       return self.n\n\
                   \x20   }\n\
                   }\n\
                   println(counter.inc())\n\
                   println(counter.inc())\n\
                   println(counter.n)";
        assert_eq!(run(src), "1\n2\n2\n");
    }

    #[test]
    fn self_calls_sibling_method() {
        let src = "p = {\n\
                   \x20   a: function() { return 1 },\n\
                   \x20   b: function() { return self.a() + 1 }\n\
                   }\n\
                   println(p.b())";
        assert_eq!(run(src), "2\n");
    }

    #[test]
    fn extracted_function_has_no_self() {
        // 把方法取出来单独调用：self 未定义（与 JS 动态 this 相反，更可预测）
        let src = "p = {n: 1, f: function() { return self.n }}\n\
                   g = p.f\n\
                   println(g())";
        let err = run_err(src);
        assert!(matches!(err, RtError::Runtime { ref message, .. } if message.contains("undefined variable `self`")));
    }

    #[test]
    fn missing_field_read_errors() {
        let err = run_err("p = {x: 1}\nprintln(p.y)");
        assert!(
            matches!(err, RtError::Runtime { ref message, .. } if message.contains("object has no field `y`")),
            "{:?}",
            err
        );
    }

    #[test]
    fn optional_chaining() {
        // null 短路
        assert_eq!(run("p = null\nprintln(p?.x)"), "null\n");
        assert_eq!(run("p = null\nprintln(p?.x?.y)"), "null\n");
        // 非空正常读取
        assert_eq!(run("q = {a: {b: 7}}\nprintln(q?.a?.b)"), "7\n");
        // 非空对象缺字段：仍然报错（严格读取）
        assert!(matches!(run_err("q = {a: 1}\nprintln(q?.z)"), RtError::Runtime { .. }));
        // 可选方法调用
        assert_eq!(run("r = {f: function() { return 42 }}\nprintln(r?.f())"), "42\n");
        assert_eq!(run("n = null\nprintln(n?.f())"), "null\n");
    }

    #[test]
    fn object_equality_by_reference() {
        let src = "a = {x: 1}\n\
                   b = a\n\
                   c = {x: 1}\n\
                   println(a == b)\n\
                   println(a == c)\n\
                   println(a == 1)";
        assert_eq!(run(src), "true\nfalse\nfalse\n");
    }

    #[test]
    fn non_function_field_not_callable() {
        let err = run_err("p = {n: 5}\np.n()");
        assert!(matches!(err, RtError::Runtime { ref message, .. } if message.contains("not callable")));
    }

    #[test]
    fn member_assign_on_non_object_errors() {
        // 原生类型没有可赋值字段
        assert!(matches!(run_err("a = [1]\na.x = 2"), RtError::Runtime { .. }));
        assert!(matches!(run_err("s = \"a\"\ns.x = 2"), RtError::Runtime { .. }));
    }
}
