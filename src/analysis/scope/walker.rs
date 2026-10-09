//! 光标走查器：沿「走到哨兵的路径」收集作用域链与接收者。

use std::collections::HashMap;
use std::rc::Rc;

use crate::ast::{Expr, ForIter, Param, Stmt, TypeAst};
use crate::Span;

use super::super::infer::{infer as infer_expr, ty_from_ast, Ty};
use super::super::ItemKind;
use super::{register_leaf, top_stmts, Binding, Ctx, SENTINEL};
use crate::checker::registry::StructRegistry;

pub(super) struct Walker<'a> {
    pub(super) structs: &'a StructRegistry,
    pub(super) inside_func: bool,
    /// 哨兵 Member 的接收者
    pub(super) receiver: Option<Expr>,
    pub(super) s: Span,
}

impl<'a> Walker<'a> {
    /// 遍历一层语句：span ≤ 哨兵的登记绑定；对**最后一条**这样的语句下钻
    /// （容器走块路径，普通语句走表达式路径——哨兵在语句尾部，必在最后一条里）。
    pub(super) fn walk_stmts(&mut self, stmts: &[Stmt], scopes: &mut Vec<HashMap<String, Binding>>) {
        let mut descend: Option<usize> = None;
        for (i, stmt) in stmts.iter().enumerate() {
            if stmt.span() > self.s {
                break;
            }
            descend = Some(i);
            self.register(stmt, scopes);
        }
        if let Some(i) = descend {
            self.descend(&stmts[i], scopes);
        }
    }

    /// 进入容器语句：新开作用域（函数绑参数与 self、循环绑循环变量），递归体
    fn descend(&mut self, stmt: &Stmt, scopes: &mut Vec<HashMap<String, Binding>>) {
        match stmt {
            Stmt::FuncDecl { name, params, ret, body, .. } => {
                self.enter_function(name, params, ret, body, None, scopes);
            }
            Stmt::Impl { target, methods, .. } => {
                // 光标在某个方法体内：只进最后一个 span ≤ 哨兵的方法
                //（前面的方法体是独立作用域，不该把它们的参数漏进链上）
                let last = methods
                    .iter()
                    .rposition(|m| m.span() <= self.s);
                if let Some(Stmt::FuncDecl { name, params, ret, body, .. }) = last.map(|i| &methods[i]) {
                    self.enter_function(
                        name,
                        params,
                        ret,
                        body,
                        Some(Ty::Struct(target.clone())),
                        scopes,
                    );
                }
            }
            Stmt::If { cond, then_block, else_block, .. } => {
                // 条件表达式里可能有哨兵（补全/悬停在条件中）：先走条件
                if cond.span() <= self.s {
                    self.walk_expr(cond, scopes);
                }
                scopes.push(HashMap::new());
                self.walk_stmts(top_stmts(then_block), scopes);
                // else 链：else-if 的 else_block 是 If 语句而非 Block，走 descend
                //（Block 分支自己会压作用域；span 门控自然失效的一侧无副作用）
                if let Some(e) = else_block {
                    self.descend(e, scopes);
                }
            }
            Stmt::While { cond, body, .. } => {
                if cond.span() <= self.s {
                    self.walk_expr(cond, scopes);
                }
                scopes.push(HashMap::new());
                self.walk_stmts(top_stmts(body), scopes);
            }
            Stmt::ForIn { var, iter, body, .. } => {
                scopes.push(HashMap::new());
                let elem = match iter {
                    ForIter::Range { .. } => Ty::Number,
                    ForIter::Expr(Expr::Str(..)) => Ty::Str,
                    ForIter::Expr(e) => {
                        // 迭代目标里可能有哨兵（`for c in s.|`）
                        if e.span() <= self.s {
                            self.walk_expr(e, scopes);
                        }
                        let ctx = Ctx { scopes, structs: self.structs };
                        match infer_expr(e, &ctx) {
                            Ty::Str => Ty::Str,
                            _ => Ty::Unknown, // 数组异构
                        }
                    }
                };
                scopes.last_mut().unwrap().insert(
                    var.clone(),
                    Binding {
                        name: var.clone(),
                        ty: elem.clone(),
                        kind: ItemKind::Variable,
                        detail: format!("{}: {}", var, elem.display()),
                        doc: "循环变量".into(),
                        // `for x in` / `for x =`：名字在关键字 + 空格之后
                        span: Span::new(stmt.span().line, stmt.span().col + 4),
                    },
                );
                self.walk_stmts(top_stmts(body), scopes);
            }
            Stmt::ForC { var, init, cond, step, body, .. } => {
                scopes.push(HashMap::new());
                if init.span() <= self.s {
                    self.walk_expr(init, scopes);
                }
                if let Some(c) = cond.as_ref().filter(|c| c.span() <= self.s) {
                    self.walk_expr(c, scopes);
                }
                if let Some(st) = step {
                    self.walk_stmt_step(st, scopes);
                }
                let ctx = Ctx { scopes, structs: self.structs };
                let t = infer_expr(init, &ctx);
                scopes.last_mut().unwrap().insert(
                    var.clone(),
                    Binding {
                        name: var.clone(),
                        ty: t.clone(),
                        kind: ItemKind::Variable,
                        detail: format!("{}: {}", var, t.display()),
                        doc: "循环变量".into(),
                        span: Span::new(stmt.span().line, stmt.span().col + 4),
                    },
                );
                self.walk_stmts(top_stmts(body), scopes);
            }
            Stmt::Block { .. } => {
                scopes.push(HashMap::new());
                self.walk_stmts(top_stmts(stmt), scopes);
            }
            // 含函数字面量/哨兵的普通语句：走表达式路径（赋值目标与值都走——
            // `self.n += 1` 的哨兵可能在目标里）
            Stmt::Assign { target, value, .. } => {
                if target.span() <= self.s {
                    self.walk_expr(target, scopes);
                }
                self.walk_expr(value, scopes);
            }
            Stmt::Expr(e, _) => self.walk_expr(e, scopes),
            Stmt::Return { value: Some(e), .. } => self.walk_expr(e, scopes),
            _ => {}
        }
    }

    /// for-C 步进段是语句（`i += 1`）：其中也可能有哨兵
    fn walk_stmt_step(&mut self, step: &Stmt, scopes: &mut Vec<HashMap<String, Binding>>) {
        match step {
            Stmt::Assign { target, value, .. } => {
                if target.span() <= self.s {
                    self.walk_expr(target, scopes);
                }
                self.walk_expr(value, scopes);
            }
            Stmt::Expr(e, _) => self.walk_expr(e, scopes),
            _ => {}
        }
    }

    /// 进入函数：压栈作用域 + self（方法语境）+ 参数，再走函数体
    fn enter_function(
        &mut self,
        name: &str,
        params: &[Param],
        ret: &Option<TypeAst>,
        body: &Rc<Stmt>,
        self_ty: Option<Ty>,
        scopes: &mut Vec<HashMap<String, Binding>>,
    ) {
        self.inside_func = true;
        scopes.push(HashMap::new());
        let frame = scopes.last_mut().unwrap();
        if let Some(t) = &self_ty {
            let sname = t.display();
            frame.insert(
                "self".into(),
                Binding {
                    name: "self".into(),
                    ty: t.clone(),
                    kind: ItemKind::Variable,
                    detail: format!("self: {}", sname),
                    doc: format!("方法 self（impl {}）", sname),
                    span: Span::default(),
                },
            );
        }
        for p in params {
            let ty = p
                .ty
                .as_ref()
                .map(|a| ty_from_ast(a, self.structs))
                .unwrap_or(Ty::Unknown);
            frame.insert(
                p.name.clone(),
                Binding {
                    detail: format!("{}: {}", p.name, ty.display()),
                    name: p.name.clone(),
                    ty,
                    kind: ItemKind::Parameter,
                    doc: format!("参数（function {}）", name),
                    span: Span::default(),
                },
            );
        }
        let _ = ret;
        self.walk_stmts(top_stmts(body), scopes);
    }

    /// 表达式路径：只关心函数字面量（它们开新作用域）与哨兵 Member。
    /// 在有序子表达式里选最后一个 span ≤ 哨兵的继续下钻。
    fn walk_expr(&mut self, expr: &Expr, scopes: &mut Vec<HashMap<String, Binding>>) {
        match expr {
            Expr::Member { target, name, .. } | Expr::OptionalMember { target, name, .. } => {
                if name == SENTINEL {
                    // 哨兵在此：记录接收者，不再下钻
                    self.receiver = Some((**target).clone());
                    return;
                }
                self.walk_expr(target, scopes);
            }
            Expr::Function { params, ret, body, .. } => {
                // 对象字面量方法字段由 Object 分支带 self 进入；其余无 self
                self.enter_function("<anonymous>", params, ret, body, None, scopes);
            }
            Expr::Object(fields, _) => {
                // 光标可能落在某个字段值（尤其是函数字段：self = 该对象）内
                for (field_name, value) in fields {
                    let vspan = value.span();
                    if vspan > self.s {
                        break;
                    }
                    if let Expr::Function { params, ret, body, .. } = value {
                        // self = 整个对象的已知形状（字段递归推导）
                        let ctx = Ctx { scopes, structs: self.structs };
                        let shape = match infer_expr(expr, &ctx) {
                            Ty::Object(fs) => fs,
                            _ => Rc::new(Vec::new()),
                        };
                        let self_ty = Some(Ty::Object(shape));
                        self.enter_function(field_name, params, ret, body, self_ty, scopes);
                    } else {
                        self.walk_expr(value, scopes);
                    }
                }
            }
            Expr::Call { callee, args, .. } => {
                self.walk_expr(callee, scopes);
                for a in args {
                    if a.span() > self.s {
                        break;
                    }
                    self.walk_expr(a, scopes);
                }
            }
            Expr::New { class, args, .. } => {
                if class.span() <= self.s {
                    self.walk_expr(class, scopes);
                }
                for a in args {
                    if a.span() > self.s {
                        break;
                    }
                    self.walk_expr(a, scopes);
                }
            }
            Expr::Array(elems, _) => {
                for e in elems {
                    if e.span() > self.s {
                        break;
                    }
                    self.walk_expr(e, scopes);
                }
            }
            Expr::Binary { left, right, .. } | Expr::Logic { left, right, .. } => {
                self.walk_expr(left, scopes);
                if right.span() <= self.s {
                    self.walk_expr(right, scopes);
                }
            }
            Expr::Unary { operand, .. } => self.walk_expr(operand, scopes),
            Expr::Ternary { cond, then_expr, else_expr, .. } => {
                self.walk_expr(cond, scopes);
                if then_expr.span() <= self.s {
                    self.walk_expr(then_expr, scopes);
                }
                if else_expr.span() <= self.s {
                    self.walk_expr(else_expr, scopes);
                }
            }
            Expr::Index { target, index, .. } => {
                self.walk_expr(target, scopes);
                if index.span() <= self.s {
                    self.walk_expr(index, scopes);
                }
            }
            Expr::Slice { target, start, end, .. } => {
                self.walk_expr(target, scopes);
                for e in [start, end].into_iter().flatten() {
                    if e.span() <= self.s {
                        self.walk_expr(e, scopes);
                    }
                }
            }
            Expr::Is { operand, target, .. } => {
                self.walk_expr(operand, scopes);
                if target.span() <= self.s {
                    self.walk_expr(target, scopes);
                }
            }
            _ => {}
        }
    }

    /// 登记语句在当前层产生的绑定（不进函数体——那是独立作用域，
    /// 由 descend 在光标位于其中时进入）
    fn register(&mut self, stmt: &Stmt, scopes: &mut Vec<HashMap<String, Binding>>) {
        match stmt {
            // 控制流体内对链上绑定的写入：浅层登记（不进函数体）
            Stmt::If { then_block, else_block, .. } => {
                self.register_block_shallow(then_block, scopes);
                if let Some(e) = else_block {
                    self.register_block_shallow(e, scopes);
                }
            }
            Stmt::While { body, .. } | Stmt::ForIn { body, .. } | Stmt::ForC { body, .. } => {
                self.register_block_shallow(body, scopes);
            }
            Stmt::Block { .. } => self.register_block_shallow(stmt, scopes),
            leaf => register_leaf(self.structs, leaf, scopes),
        }
    }

    /// 浅层登记块内语句（链上写入对后续可见；函数体内的声明不外泄的近似——
    /// 直接登记到当前层，编辑器视角够用）
    fn register_block_shallow(&mut self, block: &Stmt, scopes: &mut Vec<HashMap<String, Binding>>) {
        for s in top_stmts(block) {
            // 只登记赋值与声明；控制流递归浅走
            if matches!(
                s,
                Stmt::Assign { .. }
                    | Stmt::FuncDecl { .. }
                    | Stmt::Struct { .. }
                    | Stmt::Interface { .. }
                    | Stmt::If { .. }
                    | Stmt::While { .. }
                    | Stmt::ForIn { .. }
                    | Stmt::ForC { .. }
                    | Stmt::Block { .. }
            ) {
                self.register(s, scopes);
            }
        }
    }

    /// ambient（整个文件减光标语句）的顶层绑定：只走顶层与控制流体，
    /// 不进函数体；控制流内的赋值沿链登记（对齐运行期的全局可见性）
    pub(super) fn register_globals_ambient(&mut self, stmts: &[Stmt], scopes: &mut Vec<HashMap<String, Binding>>) {
        for s in stmts {
            self.register(s, scopes);
        }
    }
}
