//! 着色走查（全文档 AST + token 游标）。
//!
//! 作用域链 + 类型推导分类每个标识符引用，配 token 游标定位成员名，
//! 输出语义 token。原则：推不出的**不发 token**——VSCode 会保留 TextMate
//! 静态着色，宁可少色不可错色。

use std::collections::HashMap;

use crate::analysis::builtins;
use crate::analysis::ty_view::editor_display;
use crate::analysis::scope::{self, Binding, StructRegistry};
use crate::analysis::ItemKind;
use crate::ast::{Expr, ForIter, Param, Stmt};
use crate::checker::ty::Type;
use crate::checker::{Binding as CkBinding, Checker};
use crate::lexer::{Token, TokenKind};
use crate::Span;

use super::top_stmts;

// ============ 着色走查 ============

pub(super) struct Highlighter<'a> {
    pub(super) structs: &'a StructRegistry,
    pub(super) tokens: &'a [Token],
    /// token 游标：AST 访问序 = token 序，用于定位成员名等
    pub(super) tok_pos: usize,
    pub(super) scopes: Vec<HashMap<String, Binding>>,
    pub(super) emitted: Vec<(Span, u32)>,
    /// 引擎镜像链：与 scopes 成对操作（与 scope::Walker 同一模式），
    /// 绑定类型与表达式推导单源到 checker 引擎
    pub(super) ck: Checker,
}

impl<'a> Highlighter<'a> {
    pub(super) fn walk_stmts(&mut self, stmts: &[Stmt]) {
        for stmt in stmts {
            self.sync_to(stmt.span());
            // 叶子语句登记绑定（容器语句递归时在各自作用域内登记）
            if matches!(
                stmt,
                Stmt::Assign { .. } | Stmt::FuncDecl { .. } | Stmt::Struct { .. } | Stmt::Interface { .. }
            ) {
                scope::register_leaf(&mut self.ck, stmt, &mut self.scopes);
            }
            self.walk_stmt(stmt);
        }
    }

    fn walk_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Assign { target, value, .. } => {
                self.walk_expr(target);
                self.walk_expr(value);
            }
            Stmt::Expr(e, _) => self.walk_expr(e),
            Stmt::Return { value: Some(e), .. } => self.walk_expr(e),
            Stmt::If { cond, then_block, else_block, .. } => {
                self.walk_expr(cond);
                self.scopes.push(HashMap::new());
                self.ck.push_scope();
                self.walk_stmts(top_stmts(then_block));
                // else-if 的 else_block 是 If 语句而非 Block，直接 walk_stmt
                if let Some(e) = else_block {
                    self.walk_stmt(e);
                }
            }
            Stmt::While { cond, body, .. } => {
                self.walk_expr(cond);
                self.scopes.push(HashMap::new());
                self.ck.push_scope();
                self.walk_stmts(top_stmts(body));
            }
            Stmt::ForIn { var, iter, body, .. } => {
                self.scopes.push(HashMap::new());
                self.ck.push_scope();
                // 元素类型：字符串迭代 → string，区间 → number，数组 → 元素类型
                let elem = match iter {
                    ForIter::Range { .. } => Type::Number,
                    ForIter::Expr(Expr::Str(..)) => Type::Str,
                    ForIter::Expr(e) => match self.ck.infer(e) {
                        Type::Str => Type::Str,
                        Type::Array(el) => *el,
                        _ => Type::Any,
                    },
                };
                let vspan = loop_var_span(stmt);
                self.bind_var(var, elem, vspan);
                if let ForIter::Expr(e) = iter {
                    self.walk_expr(e);
                }
                self.sync_to(body.span());
                self.walk_stmts(top_stmts(body));
            }
            Stmt::ForC { var, init, cond, step, body, .. } => {
                self.scopes.push(HashMap::new());
                self.ck.push_scope();
                let t = self.ck.infer(init);
                let vspan = loop_var_span(stmt);
                self.bind_var(var, t, vspan);
                self.walk_expr(init);
                if let Some(c) = cond {
                    self.walk_expr(c);
                }
                if let Some(st) = step {
                    match &**st {
                        Stmt::Assign { target, value, .. } => {
                            self.walk_expr(target);
                            self.walk_expr(value);
                        }
                        Stmt::Expr(e, _) => self.walk_expr(e),
                        _ => {}
                    }
                }
                self.sync_to(body.span());
                self.walk_stmts(top_stmts(body));
            }
            Stmt::FuncDecl { params, ret, body, .. } => {
                // 名字与参数 token 由扫描产出；这里压作用域绑参数，走函数体
                self.scopes.push(HashMap::new());
                self.ck.push_scope();
                self.bind_params(params);
                let _ = ret;
                self.sync_to(body.span());
                self.walk_stmts(top_stmts(body));
            }
            Stmt::Impl { target, methods, .. } => {
                for m in methods {
                    if let Stmt::FuncDecl { name, params, ret, body, .. } = m {
                        self.sync_to(m.span());
                        self.scopes.push(HashMap::new());
                        self.ck.push_scope();
                        self.scopes.last_mut().unwrap().insert(
                            "self".into(),
                            Binding {
                                name: "self".into(),
                                ty: Type::Struct(target.clone(), Vec::new()),
                                kind: ItemKind::Variable,
                                detail: format!("self: {}", target),
                                doc: String::new(),
                                span: Span::default(),
                            },
                        );
                        self.ck.define(
                            "self",
                            CkBinding::plain(Type::Struct(target.clone(), Vec::new())),
                        );
                        self.bind_params(params);
                        let _ = name;
                        let _ = ret;
                        self.sync_to(body.span());
                        self.walk_stmts(top_stmts(body));
                    }
                }
            }
            Stmt::Struct { .. } | Stmt::Interface { .. } => {
                // 声明 token 由扫描产出；体内无可执行内容
            }
            Stmt::Block { .. } => {
                self.scopes.push(HashMap::new());
                self.ck.push_scope();
                self.walk_stmts(top_stmts(stmt));
            }
            _ => {}
        }
    }

    fn bind_var(&mut self, name: &str, ty: Type, span: Span) {
        let detail = format!("{}: {}", name, editor_display(&ty));
        self.ck.define(name, CkBinding::plain(ty.clone()));
        self.scopes.last_mut().unwrap().insert(
            name.to_string(),
            Binding {
                name: name.to_string(),
                ty,
                kind: ItemKind::Variable,
                detail,
                doc: "循环变量".into(),
                span,
            },
        );
    }

    fn bind_params(&mut self, params: &[Param]) {
        for p in params {
            let ty = p
                .ty
                .as_ref()
                .map(|a| crate::analysis::ty_view::ty_from_ast(a, self.structs))
                .unwrap_or(Type::Any);
            let detail = format!("{}: {}", p.name, editor_display(&ty));
            self.ck.define(&p.name, CkBinding::plain(ty.clone()));
            let frame = self.scopes.last_mut().unwrap();
            frame.insert(
                p.name.clone(),
                Binding {
                    name: p.name.clone(),
                    ty,
                    kind: ItemKind::Parameter,
                    detail,
                    doc: String::new(),
                    span: Span::default(),
                },
            );
        }
    }

    fn walk_expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Ident(name, span) => {
                if let Some(t) = self.classify_ident(name) {
                    self.emitted.push((*span, t));
                }
                // 游标自愈：用自己的 span 定位（跳过 = 等不产生 AST 节点的 token）
                self.sync_to(*span);
                if matches!(self.tokens.get(self.tok_pos).map(|t| &t.kind), Some(TokenKind::Ident(n)) if n == name) {
                    self.tok_pos += 1;
                }
            }
            Expr::Member { target, name, .. } | Expr::OptionalMember { target, name, .. } => {
                self.walk_expr(target);
                // 游标应停在 `.` 上；成员名 = 下一个 token
                if let Some(nspan) = self.member_name_span(name)
                    && let Some(t) = self.classify_member(target, name)
                {
                    self.emitted.push((nspan, t));
                }
            }
            Expr::Call { callee, args, .. } => {
                self.walk_expr(callee);
                for a in args {
                    self.walk_expr(a);
                }
            }
            Expr::New { class, args, .. } => {
                if let Expr::Ident(name, span) = &**class {
                    let t = if builtins::is_builtin_class(name) {
                        Some(7) // class
                    } else if self.structs.contains(name) {
                        Some(5) // struct
                    } else {
                        self.classify_ident(name)
                    };
                    if let Some(t) = t {
                        self.emitted.push((*span, t));
                    }
                    self.sync_to(*span);
                    if matches!(self.tokens.get(self.tok_pos).map(|t| &t.kind), Some(TokenKind::Ident(n)) if n == name) {
                        self.tok_pos += 1;
                    }
                } else {
                    self.walk_expr(class);
                }
                for a in args {
                    self.walk_expr(a);
                }
            }
            Expr::Object(fields, _) => {
                self.skip_open(); // `{`
                for (k, v) in fields {
                    // 键：值是函数字面量 → method，否则 property
                    if let Some(kspan) = self.object_key_span(k) {
                        let t = if matches!(v, Expr::Function { .. }) { 3 } else { 4 };
                        self.emitted.push((kspan, t));
                    }
                    self.walk_expr(v);
                }
                self.skip_close(); // `}`（或停在附近）
            }
            Expr::Array(elems, _) => {
                for e in elems {
                    self.walk_expr(e);
                }
            }
            Expr::Binary { left, right, .. } | Expr::Logic { left, right, .. } => {
                self.walk_expr(left);
                self.walk_expr(right);
            }
            Expr::Unary { operand, .. } => self.walk_expr(operand),
            Expr::Ternary { cond, then_expr, else_expr, .. } => {
                self.walk_expr(cond);
                self.walk_expr(then_expr);
                self.walk_expr(else_expr);
            }
            Expr::Index { target, index, .. } => {
                self.walk_expr(target);
                self.walk_expr(index);
            }
            Expr::Slice { target, start, end, .. } => {
                self.walk_expr(target);
                for e in [start, end].into_iter().flatten() {
                    self.walk_expr(e);
                }
            }
            Expr::Is { operand, target, .. } => {
                self.walk_expr(operand);
                self.walk_expr(target);
            }
            Expr::Function { params, body, .. } => {
                // 匿名函数体：独立作用域（参数 token 由扫描产出）
                self.sync_to(body.span());
                self.scopes.push(HashMap::new());
                self.ck.push_scope();
                self.bind_params(params);
                self.walk_stmts(top_stmts(body));
            }
            _ => {}
        }
    }

    /// 标识符引用分类（作用域优先，回退内置表）
    fn classify_ident(&self, name: &str) -> Option<u32> {
        // self / 字面量关键字交回 TextMate
        if matches!(name, "self" | "true" | "false" | "null" | "let") {
            return None;
        }
        if let Some(b) = self.scopes.iter().rev().find_map(|s| s.get(name)) {
            return Some(match b.kind {
                ItemKind::Function => 2,
                ItemKind::Parameter => 1,
                ItemKind::Struct => 5,
                ItemKind::Interface => 6,
                _ => 0, // variable
            });
        }
        match name {
            "EMPTY" | "inf" | "nan" => None, // TextMate 已高亮
            "fs" | "sys" => Some(8),         // namespace
            _ if builtins::global_fn(name).is_some() => Some(2),
            _ if builtins::is_builtin_class(name) => Some(7),
            _ => None,
        }
    }

    /// 成员名分类：按接收者推导（method / property / namespace 函数）。
    /// 推导经引擎镜像链（与 walker 同一模式）；联合接收者去 null 后单臂
    /// 按该臂分类，多臂宁可少色不可错色。
    fn classify_member(&mut self, target: &Expr, name: &str) -> Option<u32> {
        let recv = self.ck.infer(target);
        let recv = crate::analysis::ty_view::member_recv(&recv)?;
        match recv {
            Type::Namespace(_) => Some(2), // fs.read_file / sys.shell → function
            Type::Struct(s, _) => {
                let info = self.structs.get(s)?;
                if info.methods.iter().any(|m| m.name == *name) {
                    Some(3)
                } else if info.fields.iter().any(|f| f.name == *name) {
                    Some(4)
                } else {
                    None
                }
            }
            Type::Object(fields) => fields
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, t)| if matches!(t, Type::Func(_)) { 3 } else { 4 }),
            // 内置容器类型的方法；字段访问（不太可能）不发
            t if builtins::methods_for(t).is_some() => {
                if builtins::methods_for(t)?.iter().any(|m| *m.name == *name) {
                    Some(3)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    // ---- token 游标（自愈式：span 同步 + 有界前搜，短暂失配自动追上） ----

    /// 前进到首个 span ≥ 给定位置的 token（AST 访问与 token 序一致，只前进）
    fn sync_to(&mut self, span: Span) {
        while self.tok_pos < self.tokens.len() && self.tokens[self.tok_pos].span < span {
            self.tok_pos += 1;
        }
    }

    /// 游标附近找 `.` / `?.` + 成员名：返回名字位置并前进；
    /// 窗口内找不到返回 None（放弃该 token，不错色）
    fn member_name_span(&mut self, name: &str) -> Option<Span> {
        // 有界前搜第一个点号（允许中途隔着 token——游标可能落后）
        let mut j = self.tok_pos;
        let bound = (self.tok_pos + 16).min(self.tokens.len());
        while j < bound {
            match &self.tokens[j].kind {
                TokenKind::Dot | TokenKind::QuestionDot => break,
                TokenKind::Eof => return None,
                _ => j += 1,
            }
        }
        if j >= bound {
            return None;
        }
        let nt = self.tokens.get(j + 1)?;
        let (span, matched) = match &nt.kind {
            TokenKind::Ident(n) => (nt.span, *n == name),
            _ => return None,
        };
        self.tok_pos = j + 2;
        matched.then_some(span)
    }

    /// 跳到对象字面量的 `{` 之后（游标可能停在 `=` 等位置）
    fn skip_open(&mut self) {
        let bound = (self.tok_pos + 8).min(self.tokens.len());
        while self.tok_pos < bound {
            match self.tokens[self.tok_pos].kind {
                TokenKind::LBrace => {
                    self.tok_pos += 1;
                    return;
                }
                TokenKind::Eof => return,
                _ => self.tok_pos += 1,
            }
        }
    }

    fn skip_close(&mut self) {
        let bound = (self.tok_pos + 8).min(self.tokens.len());
        while self.tok_pos < bound {
            match self.tokens[self.tok_pos].kind {
                TokenKind::RBrace => {
                    self.tok_pos += 1;
                    return;
                }
                TokenKind::Eof => return,
                _ => self.tok_pos += 1,
            }
        }
    }

    /// 对象字面量键：窗口内找 Ident(键名) + `:`
    fn object_key_span(&mut self, key: &str) -> Option<Span> {
        let mut j = self.tok_pos;
        let bound = (self.tok_pos + 8).min(self.tokens.len());
        while j < bound {
            match &self.tokens[j].kind {
                TokenKind::Ident(n) if n == key => {
                    let nt = self.tokens.get(j + 1)?;
                    if let TokenKind::Colon = nt.kind {
                        let span = self.tokens[j].span;
                        self.tok_pos = j + 2;
                        return Some(span);
                    }
                    return None;
                }
                TokenKind::Str(_) => return None, // 字符串键：不强行着色
                TokenKind::Eof => return None,
                _ => j += 1,
            }
        }
        None
    }
}

/// 循环变量名字位置：`for x` → 语句起点 + 4（len("for ")）
fn loop_var_span(stmt: &Stmt) -> Span {
    let s = stmt.span();
    Span::new(s.line, s.col + 4)
}
