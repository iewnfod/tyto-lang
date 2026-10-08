//! 光标处可见绑定的收集：AST 遍历 + 作用域链。
//!
//! 核心思路：在光标处插入哨兵标识符后解析（见 [`super::tolerate`]），
//! 然后沿 AST **走到哨兵的路径**收集作用域——语句按「span ≤ 哨兵」顺序登记，
//! 遇到容器语句（函数体 / if / while / for / impl / 含函数字面量的表达式）
//! 则下钻一层。链上赋值语义与运行时一致：命中已有绑定就更新那一层的类型。
//!
//! self 不做特判：impl 方法与对象字面量方法在进入时把 `self` 作为
//! 普通绑定插入作用域，推导沿链自然查到。
//!
//! 函数体内编辑时的全局启发式：语言惯用形是「顶层 `h = null` + init() 内
//! `h = new MaxHeap()`」——运行期函数在顶层跑完后才被调用，故光标在函数体内
//! 时，全局层改用「挖掉光标语句后的完整文件」（ambient）的顶层绑定。

use std::collections::HashMap;
use std::rc::Rc;

use crate::ast::{AssignOp, Expr, ForIter, Param, Stmt};
use crate::Span;

use super::infer::{self, assign_result_ty, func_sig, infer as infer_expr, ty_from_annotation};
use super::infer::Ty;
use super::ItemKind;

pub const SENTINEL: &str = "__tyto_cx__";

/// 一个可见绑定（变量 / 函数 / struct / interface）
#[derive(Debug, Clone)]
pub struct Binding {
    pub name: String,
    pub ty: Ty,
    pub kind: ItemKind,
    /// 展示签名：变量 `x: T`、函数完整签名、struct 字段列表
    pub detail: String,
    pub doc: String,
    /// 声明处名字的位置（跳转定义用；未知为 default）
    pub span: Span,
}

/// struct / interface 注册表项
#[derive(Debug, Clone)]
pub struct StructInfo {
    pub name: String,
    pub interface: bool,
    pub fields: Vec<Param>,
    pub methods: Vec<FuncInfo>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct FuncInfo {
    pub name: String,
    pub params: Vec<Param>,
    pub ret: Option<String>,
    pub body: Rc<Stmt>,
}

impl StructInfo {
    pub fn detail(&self) -> String {
        if self.interface {
            format!(
                "interface {} {{ {} }}",
                self.name,
                self.methods.iter().map(|m| m.name.as_str()).collect::<Vec<_>>().join(", ")
            )
        } else {
            format!(
                "struct {} {{ {} }}",
                self.name,
                self.fields.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", ")
            )
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct StructRegistry {
    map: HashMap<String, StructInfo>,
}

impl StructRegistry {
    pub fn get(&self, name: &str) -> Option<&StructInfo> {
        self.map.get(name)
    }
    pub fn contains(&self, name: &str) -> bool {
        self.map.contains_key(name)
    }
    pub fn iter(&self) -> impl Iterator<Item = &StructInfo> {
        self.map.values()
    }
    fn entry(&mut self, info: StructInfo) {
        self.map.insert(info.name.clone(), info);
    }
}

/// 推导上下文：作用域链（外→内）+ struct 注册表
pub struct Ctx<'a> {
    pub scopes: &'a [HashMap<String, Binding>],
    pub structs: &'a StructRegistry,
}

impl<'a> Ctx<'a> {
    /// 沿链（内→外）查绑定
    pub fn lookup(&self, name: &str) -> Option<&Binding> {
        self.scopes.iter().rev().find_map(|s| s.get(name))
    }
}

/// 走到哨兵后的作用域快照
#[derive(Debug)]
pub struct ScopeSnapshot {
    /// 外→内的作用域链（最内层可能为空——光标所在块还没有语句）
    pub scopes: Vec<HashMap<String, Binding>>,
    /// 光标是否在（至少一层）函数体内
    pub inside_func: bool,
    /// 成员补全：哨兵 Member 的接收者表达式（调用方负责用最终作用域推导）
    pub receiver: Option<Expr>,
    pub structs: StructRegistry,
}

/// 全文件收集 struct / impl / interface（对编辑中的代码宽容：不看光标位置）
pub fn collect_registry(program: &Stmt) -> StructRegistry {
    let mut reg = StructRegistry::default();
    walk_types(program, &mut reg);
    reg
}

fn walk_types(stmt: &Stmt, reg: &mut StructRegistry) {
    match stmt {
        Stmt::Struct { name, fields, .. } => {
            reg.entry(StructInfo {
                name: name.clone(),
                interface: false,
                fields: fields.clone(),
                methods: Vec::new(),
                span: stmt.span(),
            });
        }
        Stmt::Interface { name, methods, .. } => {
            reg.entry(StructInfo {
                name: name.clone(),
                interface: true,
                fields: Vec::new(),
                methods: methods
                    .iter()
                    .map(|m| FuncInfo {
                        name: m.clone(),
                        params: Vec::new(),
                        ret: None,
                        body: Rc::new(Stmt::Block { stmts: Vec::new(), span: Span::default() }),
                    })
                    .collect(),
                span: stmt.span(),
            });
        }
        Stmt::Impl { target, methods, .. } => {
            // struct 可能尚未声明（编辑中）：先造空壳，后续声明补字段
            let entry = reg.map.entry(target.clone()).or_insert_with(|| StructInfo {
                name: target.clone(),
                interface: false,
                fields: Vec::new(),
                methods: Vec::new(),
                span: stmt.span(),
            });
            for m in methods {
                if let Stmt::FuncDecl { name, params, ret, body, .. } = m {
                    // 同名方法覆盖（与运行时一致）
                    entry.methods.retain(|old| old.name != *name);
                    entry.methods.push(FuncInfo {
                        name: name.clone(),
                        params: params.clone(),
                        ret: ret.clone(),
                        body: body.clone(),
                    });
                }
            }
        }
        Stmt::Block { stmts, .. } => stmts.iter().for_each(|s| walk_types(s, reg)),
        Stmt::If { then_block, else_block, .. } => {
            walk_types(then_block, reg);
            if let Some(e) = else_block {
                walk_types(e, reg);
            }
        }
        Stmt::While { body, .. } | Stmt::ForIn { body, .. } | Stmt::ForC { body, .. } => {
            walk_types(body, reg)
        }
        _ => {}
    }
}

/// 走到哨兵（`s`），收集作用域链与接收者。
///
/// `program`：插好哨兵并修复闭括号后解析出的 AST；
/// `ambient`：挖掉光标语句后的完整文件 AST（函数体内全局启发式用），可为 None。
pub fn collect_at_cursor(
    program: &Stmt,
    s: Span,
    ambient: Option<&Stmt>,
    structs: &StructRegistry,
) -> ScopeSnapshot {
    let mut walker = Walker {
        structs,
        inside_func: false,
        receiver: None,
        s,
    };
    let mut scopes: Vec<HashMap<String, Binding>> = vec![HashMap::new()];
    walker.walk_stmts(top_stmts(program), &mut scopes);

    // 函数体内编辑：全局层换成 ambient（整个文件减去光标语句）的顶层绑定
    if let Some(amb) = ambient.filter(|_| walker.inside_func) {
        let mut whole: Vec<HashMap<String, Binding>> = vec![HashMap::new()];
        walker.register_globals_ambient(top_stmts(amb), &mut whole);
        if let Some(g) = whole.first() {
            scopes[0] = g.clone();
        }
    }

    ScopeSnapshot {
        scopes,
        inside_func: walker.inside_func,
        receiver: walker.receiver.clone(),
        structs: structs.clone(),
    }
}

fn top_stmts(program: &Stmt) -> &[Stmt] {
    match program {
        Stmt::Block { stmts, .. } => stmts,
        _ => &[],
    }
}

struct Walker<'a> {
    structs: &'a StructRegistry,
    inside_func: bool,
    /// 哨兵 Member 的接收者
    receiver: Option<Expr>,
    s: Span,
}

impl<'a> Walker<'a> {
    /// 遍历一层语句：span ≤ 哨兵的登记绑定；对**最后一条**这样的语句下钻
    /// （容器走块路径，普通语句走表达式路径——哨兵在语句尾部，必在最后一条里）。
    fn walk_stmts(&mut self, stmts: &[Stmt], scopes: &mut Vec<HashMap<String, Binding>>) {
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
        ret: &Option<String>,
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
                .map(|a| ty_from_annotation(a, self.structs))
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
    fn register_globals_ambient(&mut self, stmts: &[Stmt], scopes: &mut Vec<HashMap<String, Binding>>) {
        for s in stmts {
            self.register(s, scopes);
        }
    }
}

/// 链上语义：命中已有绑定 → 更新那一层；否则插入最内层
pub(crate) fn chain_insert(scopes: &mut [HashMap<String, Binding>], name: &str, binding: Binding) {
    for scope in scopes.iter_mut().rev() {
        if scope.contains_key(name) {
            scope.insert(name.to_string(), binding);
            return;
        }
    }
    scopes.last_mut().unwrap().insert(name.to_string(), binding);
}

fn chain_find<'b>(scopes: &'b [HashMap<String, Binding>], name: &str) -> Option<&'b Binding> {
    scopes.iter().rev().find_map(|s| s.get(name))
}

/// 叶子语句（赋值 / 函数声明 / struct / interface）的绑定登记 + 声明位置记录。
/// 光标走查（scope::Walker）与全文档着色走查（semantics::Highlighter）共用。
pub(crate) fn register_leaf(
    structs: &StructRegistry,
    stmt: &Stmt,
    scopes: &mut Vec<HashMap<String, Binding>>,
) {
    match stmt {
        Stmt::Assign { target, op, value, ann, span } => {
            let Expr::Ident(name, tspan) = target else {
                // 对象字段 / 索引赋值不引入新绑定
                return;
            };
            let ctx = Ctx { scopes, structs };
            let rhs = infer_expr(value, &ctx);
            let new_ty = if let Some(a) = ann {
                ty_from_annotation(a, structs)
            } else if *op != AssignOp::Set {
                match chain_find(scopes, name) {
                    Some(old) => assign_result_ty(*op, &old.ty, &rhs),
                    None => rhs,
                }
            } else {
                rhs
            };
            // 跳转定义指向**首个**声明处：重赋值保留旧 span
            let dspan = chain_find(scopes, name)
                .map(|old| old.span)
                .filter(|s| *s != Span::default())
                .unwrap_or(*tspan);
            let b = Binding {
                name: name.clone(),
                ty: new_ty.clone(),
                kind: ItemKind::Variable,
                detail: format!("{}: {}", name, new_ty.display()),
                doc: String::new(),
                // 赋值目标的名字位置：AST 精确
                span: dspan,
            };
            let _ = span;
            chain_insert(scopes, name, b);
        }
        Stmt::FuncDecl { name, params, ret, body, span } => {
            // 返回类型：标注优先，否则从 body 的 return 推导
            let ret_ty = ret
                .as_ref()
                .map(|a| ty_from_annotation(a, structs))
                .or_else(|| infer::function_return(params, body, &Ctx { scopes, structs }));
            chain_insert(
                scopes,
                name,
                Binding {
                    name: name.clone(),
                    ty: Ty::Func(ret_ty.map(Box::new)),
                    kind: ItemKind::Function,
                    detail: func_sig(name, params, ret.as_ref()),
                    doc: String::new(),
                    // `function name`：名字在关键字 + 空格后（len("function ") == 9）
                    span: Span::new(span.line, span.col + 9),
                },
            );
        }
        Stmt::Struct { name, fields, span } => {
            chain_insert(
                scopes,
                name,
                Binding {
                    name: name.clone(),
                    ty: Ty::StructDef(name.clone()),
                    kind: ItemKind::Struct,
                    detail: format!(
                        "struct {} {{ {} }}",
                        name,
                        fields.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", ")
                    ),
                    doc: String::new(),
                    span: Span::new(span.line, span.col + 7),
                },
            );
        }
        Stmt::Interface { name, methods, span } => {
            chain_insert(
                scopes,
                name,
                Binding {
                    name: name.clone(),
                    ty: Ty::InterfaceDef(name.clone()),
                    kind: ItemKind::Interface,
                    detail: format!("interface {} {{ {} }}", name, methods.join(", ")),
                    doc: String::new(),
                    span: Span::new(span.line, span.col + 10),
                },
            );
        }
        _ => {}
    }
}

