//! 表达式求值：evaluate、二元运算、索引/成员读取及共享工具

use std::{cell::RefCell, rc::Rc};

use crate::ast::{BinaryOp, Expr, LogicOp, UnaryOp};
use crate::scope;
use crate::value::{eq_value, FuncObj};
use crate::{Interpreter, RtError, RtResult, Span, Value};

impl Interpreter {
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
                    // 仅 null 触发回退：非 null 一律取左（EMPTY/0/"" 不回退）
                    LogicOp::Nullish => !matches!(l, Value::Null),
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

    pub(crate) fn binary_op(&self, l: Value, op: BinaryOp, r: Value, span: Span) -> RtResult<Value> {
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

    pub(crate) fn index_read(&self, container: &Value, index: &Value, span: Span) -> RtResult<Value> {
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
    pub(crate) fn get_member(&self, recv: &Value, name: &str, span: Span) -> RtResult<Value> {
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
}

/// 索引必须是整数值的 number
pub(crate) fn index_int(index: &Value, span: Span) -> RtResult<usize> {
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

pub(crate) fn fmt_index(v: &Value) -> String {
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
