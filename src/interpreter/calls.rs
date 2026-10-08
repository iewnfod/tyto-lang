//! 调用与构造：函数调用（用户函数 / 原生函数）、方法分派、new 原生类

use std::{cell::RefCell, cmp::Reverse, collections::BinaryHeap, rc::Rc};

use indexmap::IndexMap;

use crate::ast::Expr;
use crate::natives;
use crate::scope::{self, Scope};
use crate::value::{FuncObj, HeapVal, InstanceObj, StructObj};
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
            // struct 实例：字段优先（可调用则以 self 调用），否则查 impl 方法表
            Value::Instance(inst) => {
                let (struct_name, field, method) = {
                    let inst = inst.borrow();
                    let def = inst.def.borrow();
                    (
                        def.name.clone(),
                        inst.fields.get(name).cloned(),
                        def.methods.get(name).cloned(),
                    )
                };
                if let Some(v) = field {
                    return self.call_value(&v, args, Some(receiver), span);
                }
                if let Some(f) = method {
                    return self.call_value(&Value::Func(f), args, Some(receiver), span);
                }
                Err(RtError::runtime(
                    Some(span),
                    format!("struct {} has no method `{}`", struct_name, name),
                ))
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
            Value::Struct(s) => Err(RtError::runtime(
                Some(span),
                format!(
                    "`{}` is a struct, use `new {}(...)`",
                    s.borrow().name,
                    s.borrow().name
                ),
            )),
            Value::Interface(i) => Err(RtError::runtime(
                Some(span),
                format!("interface `{}` cannot be called or constructed", i.name),
            )),
            Value::Instance(inst) => Err(RtError::runtime(
                Some(span),
                format!("{} instance is not callable", inst.borrow().def.borrow().name),
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
        // 用户 struct：字段全 null 起步，有 `new` 方法则调用之，否则按位置初始化
        if let Value::Struct(def) = class {
            return construct_struct(self, def.clone(), args, span);
        }
        let Value::NativeClass(kind) = class else {
            return Err(RtError::runtime(
                Some(span),
                "only native classes (Map / MaxHeap / MinHeap / Stack / Queue) and structs can be constructed",
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

/// 用户 struct 构造：`new Point(1, 2)`
/// - impl 定义了 `new` → 调用 `Point::new(1, 2)`（self 注入为新实例，返回值忽略）
/// - 未定义 `new` → 参数按字段声明顺序赋值（无 new 方法时的便捷构造）
fn construct_struct(
    interp: &mut Interpreter,
    def: Rc<RefCell<StructObj>>,
    args: &[Expr],
    span: Span,
) -> RtResult<Value> {
    let mut argv = Vec::with_capacity(args.len());
    for a in args {
        argv.push(interp.evaluate(a)?);
    }
    let new_method = def.borrow().methods.get("new").cloned();

    // 全 null 字段起步
    let mut fields = IndexMap::new();
    for f in &def.borrow().fields {
        fields.insert(f.clone(), Value::Null);
    }
    let instance = Value::Instance(Rc::new(RefCell::new(InstanceObj { def, fields })));

    if let Some(new_fn) = new_method {
        interp.call_value(&Value::Func(new_fn), argv, Some(instance.clone()), span)?;
        return Ok(instance);
    }

    // 位置初始化：参数按声明顺序赋给字段
    let Value::Instance(inst) = &instance else { unreachable!() };
    let field_count = inst.borrow().fields.len();
    if argv.len() > field_count {
        let name = inst.borrow().def.borrow().name.clone();
        return Err(RtError::runtime(
            Some(span),
            format!(
                "new {}() takes at most {} argument(s) (field count), got {}",
                name,
                field_count,
                argv.len()
            ),
        ));
    }
    {
        let mut inst = inst.borrow_mut();
        for (i, v) in argv.into_iter().enumerate() {
            if let Some((_, slot)) = inst.fields.get_index_mut(i) {
                *slot = v;
            }
        }
    }
    Ok(instance)
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
