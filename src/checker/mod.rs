//! 渐进类型系统 · 核心检查器（纯库，无 LSP 依赖）。
//!
//! 设计原则（与 docs/language-reference 的「渐进类型」章节一致）：
//! - **标注才检查**：未标注代码回退 [`Type::Any`]，零误报（渐进承诺）；
//! - **运行时擦除**：这里产出的是诊断（编辑器红线 / `tyto check`），永不阻塞执行；
//! - **尽力推导**：字面量 / new / 方法表 / 标注 / return 语句；推不出 = Any。
//!
//! 消费方：CLI（`tyto check`、`tyto run` 前置警告）与 LSP（publishDiagnostics）。

pub mod builtins;
pub mod diag;
pub mod infer;
pub mod registry;
pub mod ty;

use std::collections::HashMap;
use std::rc::Rc;

use crate::ast::{AssignOp, DeclKind, Expr, ForIter, Param, Stmt, TypeAst};
use crate::Span;

use diag::Diagnostic;
use registry::StructRegistry;
use ty::{assignable, from_ast, normalize_union, Type};

/// 表达式类型表：span → 推导类型（补全/hover 等 P3 消费；本期只写入）
pub type TypeTable = HashMap<Span, Type>;

/// 检查输出
pub struct CheckOutput {
    pub diagnostics: Vec<Diagnostic>,
    pub types: TypeTable,
    pub structs: StructRegistry,
}

/// 一个变量/函数绑定
#[derive(Debug, Clone)]
pub(crate) struct Binding {
    pub ty: Type,
    pub is_const: bool,
    /// 有显式类型约束的绑定（标注 / const 推导）记录声明位置，诊断消息引用
    pub declared: Option<Span>,
    /// let/const 声明位置（重复声明警告用；与类型约束分开）
    pub let_decl: Option<Span>,
    /// 函数绑定：调用点检查所需的原始签名（泛型求解用）
    pub func: Option<Rc<FuncCallInfo>>,
}

/// 函数调用检查信息（泛型求解需要 TypeAst 原文）
#[derive(Debug, Clone)]
pub(crate) struct FuncCallInfo {
    pub params: Vec<Param>,
    pub type_params: Vec<String>,
    pub ret: Option<TypeAst>,
}

pub(crate) struct Checker {
    pub structs: StructRegistry,
    /// 词法作用域链（外 → 内）
    pub scopes: Vec<HashMap<String, Binding>>,
    /// 当前函数的返回类型（检查 return；None = 未标注）
    pub ret_stack: Vec<Option<Type>>,
    /// 当前函数体是否出现过 return
    pub saw_return: Vec<bool>,
    /// 当前可见的泛型形参名（函数自身 + impl 所属 struct 的）
    pub generics: Vec<String>,
    pub diags: Vec<Diagnostic>,
    pub types: TypeTable,
}

/// 入口：检查整个程序（program 为解析产物）
pub fn check_program(program: &Stmt) -> CheckOutput {
    let structs = registry::collect_registry(program);
    let mut c = Checker {
        structs,
        scopes: vec![HashMap::new()],
        ret_stack: Vec::new(),
        saw_return: Vec::new(),
        generics: Vec::new(),
        diags: Vec::new(),
        types: HashMap::new(),
    };
    // 预注册：全文件函数签名（先声明后检查——调用可先于文本序出现）
    register_funcs(program, &mut c);
    c.check_stmt(program);
    CheckOutput { diagnostics: c.diags, types: c.types, structs: c.structs }
}

/// 递归注册所有 FuncDecl 绑定（含嵌套）
fn register_funcs(stmt: &Stmt, c: &mut Checker) {
    let register_one = |decl: &crate::ast::Stmt, c: &mut Checker| {
        if let crate::ast::Stmt::FuncDecl { name, params, type_params, ret, .. } = decl {
            let shape = Type::Func(Rc::new(ty::FuncShape {
                params: params
                    .iter()
                    .map(|p| p.ty.as_ref().map(|a| from_ast(a, type_params, &c.structs)))
                    .collect(),
                ret: ret.as_ref().map(|a| from_ast(a, type_params, &c.structs)),
            }));
            c.scopes[0]
                .entry(name.clone())
                .and_modify(|b| {
                    b.ty = shape.clone();
                    b.func = Some(Rc::new(FuncCallInfo {
                        params: params.clone(),
                        type_params: type_params.clone(),
                        ret: ret.clone(),
                    }));
                })
                .or_insert_with(|| Binding {
                    ty: shape.clone(),
                    is_const: false,
                    // 函数绑定不约束写入（动态语言允许覆盖函数名），declared=None
                    declared: None,
                    let_decl: None,
                    func: Some(Rc::new(FuncCallInfo {
                        params: params.clone(),
                        type_params: type_params.clone(),
                        ret: ret.clone(),
                    })),
                });
        }
    };
    match stmt {
        Stmt::Block { stmts, .. } => {
            for s in stmts {
                register_one(s, c);
                register_funcs(s, c);
            }
        }
        Stmt::If { then_block, else_block, .. } => {
            register_funcs(then_block, c);
            if let Some(e) = else_block {
                register_funcs(e, c);
            }
        }
        Stmt::While { body, .. } | Stmt::ForIn { body, .. } | Stmt::ForC { body, .. } => {
            register_funcs(body, c)
        }
        Stmt::Impl { methods, .. } => {
            for m in methods {
                register_one(m, c);
                register_funcs(m, c);
            }
        }
        _ => {}
    }
}

impl Checker {
    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }
    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    pub(crate) fn lookup(&self, name: &str) -> Option<&Binding> {
        self.scopes.iter().rev().find_map(|s| s.get(name))
    }

    /// 定义绑定到最内层作用域（let/const/标注声明）
    pub(crate) fn define(&mut self, name: &str, b: Binding) {
        self.scopes.last_mut().unwrap().insert(name.to_string(), b);
    }

    /// Lua 式链上更新：命中已存在绑定的那一层更新类型（读路径跟踪用）
    pub(crate) fn update_binding_ty(&mut self, name: &str, ty: Type) {
        for scope in self.scopes.iter_mut().rev() {
            if let Some(b) = scope.get_mut(name) {
                if !b.is_const && b.declared.is_none() {
                    b.ty = ty;
                    // 旧值的函数签名不再有效
                    b.func = None;
                }
                return;
            }
        }
        self.scopes.last_mut().unwrap().insert(
            name.to_string(),
            Binding { ty, is_const: false, declared: None, let_decl: None, func: None },
        );
    }

    /// 复合赋值在无约束绑定上的结果类型（无旧值约束，尽力算；算不出 Any）
    fn compound_result_ty(&self, op: AssignOp, value_ty: &Type, _span: Span) -> Type {
        match op {
            AssignOp::Set => value_ty.clone(),
            AssignOp::Add => ty::arith_result("+", &Type::Any, value_ty)
                .unwrap_or(Type::Any),
            AssignOp::Nullish => value_ty.clone(),
            // - * / % 恒为 number
            _ => Type::Number,
        }
    }

    pub(crate) fn err(&mut self, span: Span, msg: String) {
        self.diags.push(Diagnostic::error(span, msg));
    }
    pub(crate) fn warn(&mut self, span: Span, msg: String) {
        self.diags.push(Diagnostic::warning(span, msg));
    }

    fn ann_ty(&self, ann: &TypeAst) -> Type {
        from_ast(ann, &self.generics, &self.structs)
    }

    /// 「期望/实际」消息；declared 提供时附加声明位置
    fn mismatch(&self, expected: &Type, actual: &Type, declared: Option<Span>) -> String {
        match declared {
            Some(s) => format!(
                "类型不匹配：期望 `{}`，实际 `{}`（声明于第 {} 行）",
                expected.display(),
                actual.display(),
                s.line
            ),
            None => format!(
                "类型不匹配：期望 `{}`，实际 `{}`",
                expected.display(),
                actual.display()
            ),
        }
    }

    // ============ 语句检查 ============

    fn check_stmt(&mut self, stmt: &Stmt) {
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

    fn bind_params(&mut self, params: &[Param]) {
        for p in params {
            let ty = p.ty.as_ref().map(|a| self.ann_ty(a)).unwrap_or(Type::Any);
            self.define(&p.name, Binding { ty, is_const: false, declared: None, let_decl: None, func: None });
        }
    }

    // ============ 赋值检查 ============

    fn check_assign(
        &mut self,
        target: &Expr,
        op: AssignOp,
        value: &Expr,
        ann: Option<&TypeAst>,
        decl: Option<DeclKind>,
        span: Span,
    ) {
        match target {
            Expr::Ident(name, _) => {
                let value_ty = self.infer(value);
                match decl {
                    Some(kind) => self.check_declared_assign(name, kind, ann, value_ty, value.span(), span),
                    None => {
                        if let Some(ann) = ann {
                            // `x: T = v`（无 let）：等价声明
                            self.check_declared_assign(
                                name,
                                DeclKind::Let,
                                Some(ann),
                                value_ty,
                                value.span(),
                                span,
                            );
                            return;
                        }
                        // 普通赋值：写路径只约束「有显式类型的绑定」（渐进承诺）；
                        // 无约束绑定跟踪最新推导类型（读路径：成员访问/索引检查用）
                        let existing = self.lookup(name).cloned();
                        match existing {
                            Some(b) if b.is_const => {
                                let where_ = b
                                    .declared
                                    .map(|s| format!("（声明于第 {} 行）", s.line))
                                    .unwrap_or_default();
                                self.err(
                                    span,
                                    format!("不能给常量 `{}` 赋值{}", name, where_),
                                );
                            }
                            Some(b) if b.declared.is_some() => {
                                self.check_write_to_typed(&b, name, op, &value_ty, span);
                            }
                            Some(_) => {
                                // 无约束：更新类型（最新胜出，与运行时链上一致），不报错
                                let new_ty = self.compound_result_ty(op, &value_ty, span);
                                self.update_binding_ty(name, new_ty);
                            }
                            None => {
                                let new_ty = self.compound_result_ty(op, &value_ty, span);
                                self.define(name, Binding {
                                    ty: new_ty,
                                    is_const: false,
                                    declared: None,
                                    let_decl: None,
                                    func: None,
                                });
                            }
                        }
                    }
                }
            }
            Expr::Index { target, .. } => {
                let recv = self.infer(target);
                let v = self.infer(value);
                if op == AssignOp::Set {
                    if let Type::Array(elem) = &recv {
                        if !assignable(&v, elem) {
                            self.err(
                                span,
                                format!(
                                    "数组元素类型不匹配：期望 `{}`，实际 `{}`",
                                    elem.display(),
                                    v.display()
                                ),
                            );
                        }
                    }
                }
            }
            Expr::Member { target, name, .. } => {
                let recv = self.infer(target);
                let v = self.infer(value);
                if let Some(field_ty) = self.member_type_for_write(&recv, name) {
                    if op == AssignOp::Set && !assignable(&v, &field_ty) {
                        self.err(
                            span,
                            format!(
                                "字段类型不匹配：`{}` 期望 `{}`，实际 `{}`",
                                name,
                                field_ty.display(),
                                v.display()
                            ),
                        );
                    }
                }
            }
            _ => {}
        }
    }

    /// let / const / 标注声明：定义绑定 + 初始值检查
    fn check_declared_assign(
        &mut self,
        name: &str,
        kind: DeclKind,
        ann: Option<&TypeAst>,
        value_ty: Type,
        value_span: Span,
        span: Span,
    ) {
        // 同层重复声明提示（此前已有 let/const 声明）
        if let Some(old) = self.scopes.last().unwrap().get(name) {
            if let Some(first) = old.let_decl.or(old.declared) {
                self.warn(
                    span,
                    format!("`{}` 在同一作用域重复声明（首次声明于第 {} 行）", name, first.line),
                );
            }
        }
        match ann {
            Some(ann) => {
                let ann_ty = self.ann_ty(ann);
                if !assignable(&value_ty, &ann_ty) {
                    let msg = self.mismatch(&ann_ty, &value_ty, Some(ann.span()));
                    let _ = value_span;
                    self.err(span, msg);
                }
                self.define(name, Binding {
                    ty: ann_ty,
                    is_const: kind == DeclKind::Const,
                    declared: Some(ann.span()),
                    let_decl: Some(span),
                    func: None,
                });
            }
            None => {
                // 无标注：let 跟踪初始值类型（读路径用，写入不约束）；const 固定初始值类型
                let ty = value_ty;
                self.define(name, Binding {
                    ty,
                    is_const: kind == DeclKind::Const,
                    declared: if kind == DeclKind::Const { Some(span) } else { None },
                    let_decl: Some(span),
                    func: None,
                });
            }
        }
    }

    /// 对已有类型约束的绑定做写入检查（含复合赋值的结果类型）
    fn check_write_to_typed(&mut self, b: &Binding, name: &str, op: AssignOp, value_ty: &Type, span: Span) {
        let expected = b.ty.clone();
        let ok = if op == AssignOp::Set {
            assignable(value_ty, &expected)
        } else {
            // 复合赋值：先按运算规则算结果，再看能否写回
            let result = match op {
                AssignOp::Add => ty::arith_result("+", &expected, value_ty),
                AssignOp::Sub => ty::arith_result("-", &expected, value_ty),
                AssignOp::Mul => ty::arith_result("*", &expected, value_ty),
                AssignOp::Div => ty::arith_result("/", &expected, value_ty),
                AssignOp::Mod => ty::arith_result("%", &expected, value_ty),
                AssignOp::Nullish => {
                    // x ??= v：结果 = without_null(expected) ∪ v
                    Ok(normalize_union(vec![ty::without_null(&expected), value_ty.clone()]))
                }
                AssignOp::Set => unreachable!(),
            };
            match result {
                Ok(res) => assignable(&res, &expected),
                Err(()) => false,
            }
        };
        if !ok {
            let msg = self.mismatch(&expected, value_ty, b.declared);
            let _ = name;
            self.err(span, msg);
        }
    }

    /// 写入路径的成员类型（struct 字段 / 对象形状；方法不可写）
    fn member_type_for_write(&mut self, recv: &Type, name: &str) -> Option<Type> {
        match recv {
            Type::Struct(sname, args) => {
                let info = self.structs.get(sname)?;
                let field = info.fields.iter().find(|f| f.name == name)?;
                let generics = self.structs.get(sname).map(|s| s.type_params.clone())?;
                let raw = field.ty.as_ref().map(|a| from_ast(a, &generics, &self.structs))?;
                Some(self.subst_struct(&raw, sname, args))
            }
            Type::Object(fields) => {
                fields.iter().find(|(n, _)| n == name).map(|(_, t)| t.clone())
            }
            _ => None,
        }
    }
}
