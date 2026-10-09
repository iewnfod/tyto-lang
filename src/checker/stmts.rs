//! 语句检查：遍历分派、函数体/方法体检查、参数绑定。

use crate::ast::{ForIter, Param, Stmt, TypeAst};
use crate::Span;

use super::ty::{assignable, Type};
use super::{Binding, Checker};

impl Checker {
    // ============ 语句检查 ============

    pub(crate) fn check_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Block { stmts, .. } => {
                self.push_scope();
                for s in stmts {
                    self.check_stmt(s);
                }
                self.pop_scope();
            }
            Stmt::Expr(e, _) => {
                self.infer(e);
            }
            Stmt::Assign { target, op, value, ann, decl, span } => {
                self.check_assign(target, *op, value, ann.as_ref(), *decl, *span);
            }
            Stmt::If { cond, then_block, else_block, .. } => {
                self.infer(cond);
                self.check_stmt(then_block);
                if let Some(e) = else_block {
                    self.check_stmt(e);
                }
            }
            Stmt::While { cond, body, .. } => {
                self.infer(cond);
                self.check_stmt(body);
            }
            Stmt::ForIn { var, iter, body, .. } => {
                let elem = match iter {
                    ForIter::Range { .. } => Type::Number,
                    ForIter::Expr(e) => {
                        let t = self.infer(e);
                        match t {
                            Type::Array(el) => *el,
                            Type::Str => Type::Str,
                            Type::Any => Type::Any,
                            Type::Map(..) => {
                                // Map 迭代未在语言参考中定义，回退 Any
                                Type::Any
                            }
                            other => {
                                // 数字迭代：`for i in 5` 是常见形态吗？语言只定义数组/字符串/区间
                                let _ = other;
                                Type::Any
                            }
                        }
                    }
                };
                self.push_scope();
                self.define(var, Binding { ty: elem, is_const: false, declared: None, let_decl: None, func: None });
                self.check_stmt(body);
                self.pop_scope();
            }
            Stmt::ForC { var, init, cond, step, body, .. } => {
                self.push_scope();
                let init_ty = self.infer(init);
                let _ = init_ty;
                self.define(var, Binding { ty: Type::Number, is_const: false, declared: None, let_decl: None, func: None });
                if let Some(c) = cond {
                    self.infer(c);
                }
                if let Some(s) = step {
                    self.check_stmt(s);
                }
                self.check_stmt(body);
                self.pop_scope();
            }
            Stmt::FuncDecl { name, params, type_params, ret, body, span } => {
                // 绑定已在预注册完成；这里检查函数体
                self.check_func_body(name, params, type_params, ret, body, None, *span);
            }
            Stmt::Struct { name, .. } => {
                self.define(name, Binding {
                    ty: Type::StructDef(name.clone()),
                    is_const: false,
                    declared: Some(stmt.span()),
                    let_decl: None,
                    func: None,
                });
            }
            Stmt::Impl { target, type_params, methods, .. } => {
                // 方法体可见的泛型参数：impl 声明优先，未声明则沿用 struct 声明
                let struct_tparams =
                    self.structs.get(target).map(|s| s.type_params.clone()).unwrap_or_default();
                let effective =
                    if !type_params.is_empty() { type_params.clone() } else { struct_tparams };
                for m in methods {
                    if let Stmt::FuncDecl { name, params, type_params, ret, body, span } = m {
                        self.check_method_body(
                            target,
                            &effective,
                            name,
                            params,
                            type_params,
                            ret,
                            body,
                            *span,
                        );
                    }
                }
            }
            Stmt::Interface { name, .. } => {
                self.define(name, Binding {
                    ty: Type::InterfaceDef(name.clone()),
                    is_const: false,
                    declared: Some(stmt.span()),
                    let_decl: None,
                    func: None,
                });
            }
            Stmt::Return { value, span } => {
                let actual = value.as_ref().map(|v| self.infer(v)).unwrap_or(Type::Null);
                match self.ret_stack.last() {
                    Some(Some(expected)) => {
                        if !assignable(&actual, expected) {
                            self.err(
                                *span,
                                format!(
                                    "返回类型不匹配：期望 `{}`，实际 `{}`",
                                    expected.display(),
                                    actual.display()
                                ),
                            );
                        }
                    }
                    _ => {}
                }
                if let Some(flag) = self.saw_return.last_mut() {
                    *flag = true;
                }
            }
            Stmt::Break(_) | Stmt::Continue(_) => {}
        }
    }

    /// 函数体检查：压栈（参数/泛型/返回类型）后走体
    #[allow(clippy::too_many_arguments)]
    fn check_func_body(
        &mut self,
        _name: &str,
        params: &[Param],
        type_params: &[String],
        ret: &Option<TypeAst>,
        body: &Stmt,
        self_struct: Option<(String, Vec<Type>)>,
        span: Span,
    ) {
        let saved_generics = self.generics.clone();
        self.generics = type_params.to_vec();
        if let Some((sname, _)) = &self_struct {
            let stparams =
                self.structs.get(sname).map(|s| s.type_params.clone()).unwrap_or_default();
            self.generics.extend(stparams);
        }
        self.push_scope();
        if let Some((sname, args)) = &self_struct {
            self.define("self", Binding {
                ty: Type::Struct(sname.clone(), args.clone()),
                is_const: false,
                declared: None,
                let_decl: None,
                func: None,
            });
        }
        self.bind_params(params);
        let ret_ty = ret.as_ref().map(|a| self.ann_ty(a));
        self.ret_stack.push(ret_ty.clone());
        self.saw_return.push(false);
        self.check_stmt(body);
        let saw = self.saw_return.pop().unwrap_or(false);
        self.ret_stack.pop();
        // 有返回标注但函数体没有任何 return → 提示（null 返回标注除外）
        if let Some(rt) = &ret_ty {
            if !saw && !matches!(rt, Type::Null) && !rt.is_any() {
                self.warn(
                    span,
                    format!("函数标注了返回 `{}`，但函数体中没有 return", rt.display()),
                );
            }
        }
        self.pop_scope();
        self.generics = saved_generics;
    }

    /// impl 方法体：self 绑定为带泛型形参的 struct 实例
    #[allow(clippy::too_many_arguments)]
    fn check_method_body(
        &mut self,
        target: &str,
        struct_tparams: &[String],
        name: &str,
        params: &[Param],
        type_params: &[String],
        ret: &Option<TypeAst>,
        body: &Stmt,
        span: Span,
    ) {
        let args: Vec<Type> =
            struct_tparams.iter().map(|t| Type::TypeVar(t.clone())).collect();
        let _ = (name, type_params);
        self.check_func_body(name, params, type_params, ret, body, Some((target.to_string(), args)), span);
    }

    pub(crate) fn bind_params(&mut self, params: &[Param]) {
        for p in params {
            let ty = p.ty.as_ref().map(|a| self.ann_ty(a)).unwrap_or(Type::Any);
            self.define(&p.name, Binding { ty, is_const: false, declared: None, let_decl: None, func: None });
        }
    }
}
