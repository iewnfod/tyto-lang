//! 表达式类型推导与使用点检查（赋值兼容性见 ty::assignable）。
//!
//! 推不出的地方一律 [`Type::Any`]（渐进承诺：绝不因「不知道」报错）；
//! 报错只发生在「明确知道且明确不匹配」的地方。

use std::collections::HashMap;
use std::rc::Rc;

use crate::ast::{BinaryOp, Expr, LogicOp, UnaryOp};
use crate::Span;

use super::builtins;
use super::registry::FuncInfo;
use super::ty::{
    arith_result, from_ast, normalize_union, without_null, FuncShape, Type,
};
use super::unify::subst;
use super::Checker;

impl Checker {
    /// 表达式推导主入口（同时做使用点检查：调用实参 / 成员存在性 / 算术操作数）
    pub(crate) fn infer(&mut self, expr: &Expr) -> Type {
        let t = self.infer_inner(expr);
        self.types.insert(expr.span(), t.clone());
        t
    }

    pub(crate) fn infer_inner(&mut self, expr: &Expr) -> Type {
        match expr {
            Expr::Num(..) => Type::Number,
            Expr::Str(..) => Type::Str,
            Expr::Bool(..) => Type::Bool,
            Expr::Null(..) => Type::Null,
            Expr::Array(elems, _) => {
                // 元素全同型 → 元素类型；否则 Any（语言允许异构）
                let mut elem: Option<Type> = None;
                let mut mixed = false;
                for e in elems {
                    let t = self.infer(e);
                    match &elem {
                        None => elem = Some(t),
                        Some(prev) if *prev != t && !t.is_any() && !prev.is_any() => mixed = true,
                        _ => {}
                    }
                }
                let et = if mixed { Type::Any } else { elem.unwrap_or(Type::Any) };
                Type::Array(Box::new(et))
            }
            Expr::Object(fields, _) => {
                let shape: Vec<(String, Type)> = fields
                    .iter()
                    .map(|(n, v)| (n.clone(), self.infer(v)))
                    .collect();
                Type::Object(Rc::from(shape))
            }
            Expr::Ident(name, _) => self.ident_ty(name),
            Expr::Unary { op, operand, span } => {
                let t = self.infer(operand);
                match op {
                    UnaryOp::Not => Type::Bool,
                    UnaryOp::Neg => {
                        if t.is_any() || matches!(t, Type::TypeVar(_)) {
                            Type::Number
                        } else if let Type::Union(ms) = &t {
                            if ms.iter().all(|m| m.is_any() || matches!(m, Type::Number | Type::TypeVar(_))) {
                                Type::Number
                            } else {
                                self.err(
                                    *span,
                                    format!("取负要求数字，实际 `{}`", t.display()),
                                );
                                Type::Number
                            }
                        } else if t == Type::Number {
                            Type::Number
                        } else {
                            self.err(
                                *span,
                                format!("取负要求数字，实际 `{}`", t.display()),
                            );
                            Type::Number
                        }
                    }
                }
            }
            Expr::Binary { left, op, right, span } => {
                let l = self.infer(left);
                let r = self.infer(right);
                match op {
                    BinaryOp::Add => self.add_result(&l, &r, *span),
                    BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod => {
                        let sym = match op {
                            BinaryOp::Sub => "-",
                            BinaryOp::Mul => "*",
                            BinaryOp::Div => "/",
                            _ => "%",
                        };
                        match arith_result(sym, &l, &r) {
                            Ok(t) => t,
                            Err(()) => {
                                self.err(
                                    *span,
                                    format!("`{}` 不能用于 `{}` 和 `{}`", sym, l.display(), r.display()),
                                );
                                Type::Any
                            }
                        }
                    }
                    // 比较恒为 bool（动态语义宽松，不查操作数）
                    BinaryOp::Lt
                    | BinaryOp::Gt
                    | BinaryOp::Lte
                    | BinaryOp::Gte
                    | BinaryOp::Eq
                    | BinaryOp::Neq => Type::Bool,
                }
            }
            Expr::Logic { left, op, right, .. } => {
                let l = self.infer(left);
                let r = self.infer(right);
                match op {
                    // 返回操作数：保守并集（真值收窄属控制流分析，v1 不做）
                    LogicOp::And | LogicOp::Or => normalize_union(vec![l, r]),
                    // `l ?? r`：左操作数去掉 null 后与右操作数取并集
                    LogicOp::Nullish => normalize_union(vec![without_null(&l), r]),
                }
            }
            Expr::Ternary { then_expr, else_expr, .. } => {
                let t = self.infer(then_expr);
                let e = self.infer(else_expr);
                normalize_union(vec![t, e])
            }
            Expr::Index { target, index, span } => {
                let recv = self.infer(target);
                self.infer(index);
                match recv {
                    Type::Array(elem) => *elem,
                    Type::Str => Type::Str,
                    Type::Map(k, v) => {
                        let _ = k;
                        normalize_union(vec!(*v, Type::Null))
                    }
                    Type::Any | Type::TypeVar(_) => Type::Any,
                    other => {
                        self.err(
                            *span,
                            format!("不能索引 `{}` 类型", other.display()),
                        );
                        Type::Any
                    }
                }
            }
            Expr::Slice { target, .. } => match self.infer(target) {
                Type::Array(elem) => Type::Array(elem),
                Type::Str => Type::Str,
                other => {
                    let _ = other;
                    Type::Any
                }
            },
            Expr::Member { target, name, span } => {
                let recv = self.infer(target);
                self.member_ty(&recv, name, *span, false)
            }
            Expr::OptionalMember { target, name, span } => {
                // p?.x：接收者去掉 null 后取成员
                let recv = self.infer(target);
                let recv = without_null(&recv);
                self.member_ty(&recv, name, *span, true)
            }
            Expr::Call { callee, args, span } => self.check_call(callee, args, *span),
            Expr::New { class, args, span } => self.check_new(class, args, *span),
            Expr::Is { operand, target, .. } => {
                self.infer(operand);
                self.infer(target);
                Type::Bool
            }
            Expr::Function { params, type_params, ret, .. } => {
                let shape = Type::Func(Rc::new(FuncShape {
                    params: params
                        .iter()
                        .map(|p| p.ty.as_ref().map(|a| from_ast(a, type_params, &self.structs)))
                        .collect(),
                    ret: ret.as_ref().map(|a| from_ast(a, type_params, &self.structs)),
                }));
                shape
            }
        }
    }

    fn add_result(&mut self, l: &Type, r: &Type, span: Span) -> Type {
        match arith_result("+", l, r) {
            Ok(t) => t,
            Err(()) => {
                self.err(
                    span,
                    format!("`+` 不能用于 `{}` 和 `{}`", l.display(), r.display()),
                );
                Type::Any
            }
        }
    }

    fn ident_ty(&mut self, name: &str) -> Type {
        if let Some(b) = self.lookup(name) {
            return b.ty.clone();
        }
        match name {
            "EMPTY" => Type::Empty,
            "inf" | "nan" => Type::Number,
            "true" | "false" => Type::Bool,
            "null" => Type::Null,
            "fs" => Type::Namespace("fs"),
            "sys" => Type::Namespace("sys"),
            _ => {
                if let Some(t) = builtins::global_fn_return(name) {
                    return Type::Func(Rc::new(FuncShape { params: Vec::new(), ret: Some(t) }));
                }
                if let Some(t) = builtins::constant_ty(name) {
                    return t;
                }
                if let Some(info) = self.structs.get(name) {
                    return if info.interface {
                        Type::InterfaceDef(name.to_string())
                    } else {
                        Type::StructDef(name.to_string())
                    };
                }
                Type::Any
            }
        }
    }

    /// 成员类型（读路径）；带使用点检查：已知类型上不存在的成员报错。
    /// `optional`：`?.` 访问（接收者已去 null）。
    pub(crate) fn member_ty(&mut self, recv: &Type, name: &str, span: Span, optional: bool) -> Type {
        match recv {
            Type::Any | Type::TypeVar(_) => Type::Any,
            Type::Namespace(ns) => builtins::namespace_fn_return(ns, name)
                .map(|t| Type::Func(Rc::new(FuncShape { params: Vec::new(), ret: Some(t) })))
                .unwrap_or_else(|| {
                    if !optional {
                        self.err(span, format!("命名空间 `{}` 没有 `{}`", ns, name));
                    }
                    Type::Any
                }),
            Type::Object(fields) => fields
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, t)| t.clone())
                .unwrap_or_else(|| {
                    if !optional {
                        self.err(
                            span,
                            format!("对象类型 `{}` 没有字段 `{}`", recv.display(), name),
                        );
                    }
                    Type::Any
                }),
            Type::Struct(sname, args) => self.struct_member(sname, args, name, span, optional),
            // 联合：每个成员都必须有该成员（交集语义）；返回成员类型的并集
            Type::Union(ms) => {
                let mut results = Vec::new();
                for m in ms.iter() {
                    // null 成员在 ?. 下已剔除；普通成员访问 null 直接放行 Any（运行时报）
                    if matches!(m, Type::Null | Type::Any | Type::TypeVar(_)) {
                        return Type::Any;
                    }
                    let t = self.member_ty(m, name, span, optional);
                    if t.is_any() {
                        return Type::Any;
                    }
                    results.push(t);
                }
                normalize_union(results)
            }
            Type::StructDef(_) | Type::InterfaceDef(_) => Type::Any,
            // 内置类型：方法表
            _ => {
                if let Some(t) = builtins::method_return(recv, name) {
                    t
                } else {
                    if !optional {
                        self.err(
                            span,
                            format!("类型 `{}` 没有 `{}` 成员", recv.display(), name),
                        );
                    }
                    Type::Any
                }
            }
        }
    }

    /// struct 成员：字段（泛型实参代入）优先，再 impl 方法
    fn struct_member(
        &mut self,
        sname: &str,
        args: &[Type],
        name: &str,
        span: Span,
        optional: bool,
    ) -> Type {
        let Some(info) = self.structs.get(sname) else { return Type::Any };
        if let Some(f) = info.fields.iter().find(|f| f.name == name) {
            return f
                .ty
                .as_ref()
                .map(|a| {
                    let raw = from_ast(a, &info.type_params, &self.structs);
                    self.subst_struct(&raw, sname, args)
                })
                .unwrap_or(Type::Any);
        }
        if info.methods.iter().any(|m| m.name == name) {
            // 需要可变借用：把方法副本出来再构造形状
            let m_clone = info
                .methods
                .iter()
                .find(|m| m.name == name)
                .cloned()
                .expect("just checked");
            return self.method_shape(&m_clone, sname, args);
        }
        if !optional {
            self.err(
                span,
                format!("struct `{}` 没有字段或方法 `{}`", sname, name),
            );
        }
        Type::Any
    }

    /// impl 方法的函数类型（类型实参代入签名）
    fn method_shape(&mut self, m: &FuncInfo, sname: &str, args: &[Type]) -> Type {
        let struct_tparams =
            self.structs.get(sname).map(|s| s.type_params.clone()).unwrap_or_default();
        let mut all_generics = m.type_params.clone();
        all_generics.extend(struct_tparams.iter().cloned());
        let params: Vec<Option<Type>> = m
            .params
            .iter()
            .map(|p| p.ty.as_ref().map(|a| from_ast(a, &all_generics, &self.structs)))
            .collect();
        let ret = m.ret.as_ref().map(|a| from_ast(a, &all_generics, &self.structs));
        let shape = Type::Func(Rc::new(FuncShape { params, ret }));
        self.subst_struct(&shape, sname, args)
    }

    /// struct 泛型实参代入（Box<T> 实例 Box<number> → T 替换为 number）
    pub(crate) fn subst_struct(&self, t: &Type, sname: &str, args: &[Type]) -> Type {
        let Some(info) = self.structs.get(sname) else { return t.clone() };
        if info.type_params.is_empty() || args.is_empty() {
            // 未实例化的泛型 struct（args 空）：TypeVar → Any，避免消息里出现裸 `T`
            if info.type_params.is_empty() {
                return t.clone();
            }
            let map: HashMap<String, Type> = HashMap::new();
            return subst(t, &map);
        }
        let map: HashMap<String, Type> = info
            .type_params
            .iter()
            .cloned()
            .zip(args.iter().cloned())
            .collect();
        subst(t, &map)
    }

}
