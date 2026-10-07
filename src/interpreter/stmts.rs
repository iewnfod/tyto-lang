//! 语句执行：execute 及各类语句的落地（赋值 / 分支 / 循环 / 块）

use std::rc::Rc;

use crate::ast::{AssignOp, BinaryOp, Expr, ForIter, Stmt};
use crate::scope::{self, Scope};
use crate::value::FuncObj;
use crate::{Flow, Interpreter, RtError, RtResult, Span, Value};

use super::exprs::{fmt_index, index_int};

impl Interpreter {
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
}
