//! 渐进类型系统 · 核心检查器（纯库，无 LSP 依赖）。
//!
//! 设计原则（与 docs/language-reference 的「渐进类型」章节一致）：
//! - **标注才检查**：未标注代码回退 [`Type::Any`]，零误报（渐进承诺）；
//! - **运行时擦除**：这里产出的是诊断（编辑器红线 / `tyto check`），永不阻塞执行；
//! - **尽力推导**：字面量 / new / 方法表 / 标注 / return 语句；推不出 = Any。
//!
//! 消费方：CLI（`tyto check`、`tyto run` 前置警告）与 LSP（publishDiagnostics）。
//!
//! 结构：本模块持检查器状态与入口；语句检查在 [`stmts`]，赋值检查在 [`assign`]，
//! 表达式推导在 [`infer`]，调用/构造检查在 [`call`]，泛型求解在 [`unify`]，
//! 类型表示与兼容性在 [`ty`]，struct 注册表在 [`registry`]，内置知识在 [`builtins`]。

pub mod assign;
pub mod builtins;
pub mod call;
pub mod diag;
pub mod infer;
pub mod registry;
pub mod stmts;
pub mod ty;
pub mod unify;

use std::collections::HashMap;
use std::rc::Rc;

use crate::ast::{AssignOp, Param, Stmt, TypeAst};
use crate::Span;

use diag::Diagnostic;
use registry::StructRegistry;
use ty::{from_ast, Type};

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
    pub(crate) fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }
    pub(crate) fn pop_scope(&mut self) {
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
    pub(crate) fn compound_result_ty(&self, op: AssignOp, value_ty: &Type, _span: Span) -> Type {
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

    pub(crate) fn ann_ty(&self, ann: &TypeAst) -> Type {
        from_ast(ann, &self.generics, &self.structs)
    }

    /// 「期望/实际」消息；declared 提供时附加声明位置
    pub(crate) fn mismatch(&self, expected: &Type, actual: &Type, declared: Option<Span>) -> String {
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
}
