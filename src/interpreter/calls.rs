//! 调用与构造：函数调用（用户函数 / 原生函数）、方法分派、new 原生类

use std::{cell::RefCell, cmp::Reverse, collections::BinaryHeap, rc::Rc};

use crate::ast::Expr;
use crate::natives;
use crate::scope::{self, Scope};
use crate::value::{FuncObj, HeapVal};
use crate::{Flow, Interpreter, NativeClass, RtError, RtResult, Span, Value};

impl Interpreter {
    // ============ 调用与构造 ============

    pub(crate) fn call_member(
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

    pub(crate) fn call_value(
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

    pub(crate) fn construct(&mut self, class: &Value, args: &[Expr], span: Span) -> RtResult<Value> {
        let Value::NativeClass(kind) = class else {
            return Err(RtError::runtime(
                Some(span),
                "only native classes (Map / MaxHeap / MinHeap / Stack / Queue) can be constructed in v1",
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
            NativeClass::Stack => {
                let items = eval_init_items(self, args, *kind, span)?;
                Ok(Value::Stack(Rc::new(RefCell::new(items))))
            }
            NativeClass::Queue => {
                let items = eval_init_items(self, args, *kind, span)?;
                Ok(Value::Queue(Rc::new(RefCell::new(items.into_iter().collect()))))
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

/// Stack()/Queue() 的初始化参数：无参或单个数组（按序装入，可异构）
fn eval_init_items(
    interp: &mut Interpreter,
    args: &[Expr],
    kind: NativeClass,
    span: Span,
) -> RtResult<Vec<Value>> {
    match args {
        [] => Ok(vec![]),
        [one] => {
            let v = interp.evaluate(one)?;
            match v {
                Value::Array(a) => Ok(a.borrow().clone()),
                other => Err(RtError::runtime(
                    Some(span),
                    format!(
                        "{}() takes no arguments or an array, got {}",
                        kind.name(),
                        other.type_name()
                    ),
                )),
            }
        }
        _ => Err(RtError::runtime(
            Some(span),
            format!("{}() takes 0 or 1 argument", kind.name()),
        )),
    }
}
