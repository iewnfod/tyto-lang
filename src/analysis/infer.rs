//! 类型表示与表达式类型推导。
//!
//! Tyto 是动态类型语言，这里是**编辑器视角的尽力推导**：
//! 从字面量、`new`、方法返回类型表、类型标注（`x: T = v`、`-> T`）
//! 与用户函数的 return 语句推断；推不出的就是 [`Ty::Unknown`]，
//! 补全时回退到全量方法池，绝不阻塞。

use std::collections::HashMap;
use std::rc::Rc;

use crate::ast::{AssignOp, BinaryOp, Expr, Param, Stmt, TypeAst, UnaryOp};

use super::builtins;
use super::scope::{Binding, Ctx, StructRegistry};

/// 推导出的值类型（编辑器近似，不做完整类型检查）
#[derive(Debug, Clone, PartialEq)]
pub enum Ty {
    Number,
    Str,
    Bool,
    Null,
    Empty,
    Array,
    Map,
    MaxHeap,
    MinHeap,
    Stack,
    Queue,
    /// 用户 struct 实例（含名字，可查注册表得到字段与方法）
    Struct(String),
    /// struct 定义本身（`struct P {...}` 执行后的绑定值）
    StructDef(String),
    /// interface 定义本身
    InterfaceDef(String),
    /// 函数值；payload 为返回类型（标注或推导）
    Func(Option<Box<Ty>>),
    /// 对象字面量：已知字段（名字、类型、是否函数字段）
    Object(Rc<Vec<FieldInfo>>),
    /// 命名空间 `fs` / `sys`
    Namespace(&'static str),
    Unknown,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FieldInfo {
    pub name: String,
    pub ty: Ty,
    pub is_fn: bool,
}

impl Ty {
    /// 展示名（与语言 `type()` 的返回对齐，实例显示 struct 名）
    pub fn display(&self) -> String {
        match self {
            Ty::Number => "number".into(),
            Ty::Str => "string".into(),
            Ty::Bool => "bool".into(),
            Ty::Null => "null".into(),
            Ty::Empty => "empty".into(),
            Ty::Array => "array".into(),
            Ty::Map => "map".into(),
            Ty::MaxHeap => "maxheap".into(),
            Ty::MinHeap => "minheap".into(),
            Ty::Stack => "stack".into(),
            Ty::Queue => "queue".into(),
            Ty::Struct(n) | Ty::StructDef(n) | Ty::InterfaceDef(n) => n.clone(),
            Ty::Func(ret) => match ret {
                Some(t) => format!("function -> {}", t.display()),
                None => "function".into(),
            },
            Ty::Object(_) => "object".into(),
            Ty::Namespace(n) => (*n).into(),
            Ty::Unknown => "unknown".into(),
        }
    }

    fn from_type_name(name: &str) -> Option<Ty> {
        // 与 type() 返回串对齐；struct 实例由 Struct(n) 单独表示
        Some(match name {
            "number" => Ty::Number,
            "string" => Ty::Str,
            "bool" | "boolean" => Ty::Bool,
            "null" => Ty::Null,
            "empty" => Ty::Empty,
            "array" => Ty::Array,
            "map" => Ty::Map,
            "maxheap" => Ty::MaxHeap,
            "minheap" => Ty::MinHeap,
            "stack" => Ty::Stack,
            "queue" => Ty::Queue,
            "object" => Ty::Object(Rc::new(Vec::new())),
            "function" => Ty::Func(None),
            _ => return None,
        })
    }
}

/// 类型标注 AST → 编辑器近似 Ty（桥接期实现）。
/// 语言不强制写法：联合/函数类型保守回退（完整语义在核心 checker），
/// 识别不了的标识符若在 struct 注册表中则视为其实例，否则 Unknown。
pub fn ty_from_ast(ann: &TypeAst, structs: &StructRegistry) -> Ty {
    match ann {
        TypeAst::Union(..) => Ty::Unknown,
        TypeAst::Object(..) => Ty::Object(Rc::new(Vec::new())),
        TypeAst::Func { ret, .. } => Ty::Func(
            ret.as_ref().map(|r| Box::new(ty_from_ast(r, structs))),
        ),
        TypeAst::Array(..) => Ty::Array,
        TypeAst::Named(n, _) => named_ty(n, structs),
        TypeAst::Generic(n, _, _) => named_ty(n, structs),
    }
}

fn named_ty(s: &str, structs: &StructRegistry) -> Ty {
    match s {
        "any" | "unknown" => Ty::Unknown,
        "Array" => Ty::Array,
        _ => Ty::from_type_name(s).unwrap_or_else(|| {
            if structs.contains(s) {
                Ty::Struct(s.into())
            } else {
                Ty::Unknown
            }
        }),
    }
}

/// 表达式类型推导主入口。`depth` 防止用户函数 return 推断的递归环。
pub fn infer(expr: &Expr, ctx: &Ctx) -> Ty {
    infer_depth(expr, ctx, 0)
}

const MAX_INFER_DEPTH: usize = 8;

fn infer_depth(expr: &Expr, ctx: &Ctx, depth: usize) -> Ty {
    if depth > MAX_INFER_DEPTH {
        return Ty::Unknown;
    }
    match expr {
        Expr::Num(..) => Ty::Number,
        Expr::Str(..) => Ty::Str,
        Expr::Bool(..) => Ty::Bool,
        Expr::Null(..) => Ty::Null,
        Expr::Array(..) => Ty::Array,
        Expr::Object(fields, _) => Ty::Object(Rc::new(
            fields
                .iter()
                .map(|(name, v)| {
                    let ty = infer_depth(v, ctx, depth);
                    let is_fn = matches!(ty, Ty::Func(_));
                    FieldInfo { name: name.clone(), ty, is_fn }
                })
                .collect(),
        )),
        Expr::Ident(name, _) => ident_ty(name, ctx),
        Expr::Unary { op, operand, .. } => {
            let t = infer_depth(operand, ctx, depth);
            match op {
                UnaryOp::Not => Ty::Bool,
                UnaryOp::Neg => match t {
                    Ty::Unknown => Ty::Unknown,
                    _ => Ty::Number,
                },
            }
        }
        Expr::Binary { left, op, right, .. } => {
            let l = infer_depth(left, ctx, depth);
            let r = infer_depth(right, ctx, depth);
            match op {
                BinaryOp::Add => {
                    if l == Ty::Str || r == Ty::Str {
                        Ty::Str
                    } else if l == Ty::Array && r == Ty::Array {
                        Ty::Array
                    } else {
                        Ty::Number
                    }
                }
                BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod => Ty::Number,
                BinaryOp::Lt | BinaryOp::Gt | BinaryOp::Lte | BinaryOp::Gte | BinaryOp::Eq
                | BinaryOp::Neq => Ty::Bool,
            }
        }
        // && / || / ?? 返回操作数：取左侧（右侧可能不同，取保守的左类型）
        Expr::Logic { left, .. } => infer_depth(left, ctx, depth),
        Expr::Ternary { then_expr, else_expr, .. } => {
            let t = infer_depth(then_expr, ctx, depth);
            if t == Ty::Null || t == Ty::Unknown {
                infer_depth(else_expr, ctx, depth)
            } else {
                t
            }
        }
        Expr::Index { target, .. } => match infer_depth(target, ctx, depth) {
            Ty::Str => Ty::Str,
            Ty::Array => Ty::Unknown, // 异构，元素类型未知
            _ => Ty::Unknown,
        },
        Expr::Slice { target, .. } => match infer_depth(target, ctx, depth) {
            Ty::Array => Ty::Array,
            Ty::Str => Ty::Str,
            _ => Ty::Unknown,
        },
        Expr::Member { target, name, .. } | Expr::OptionalMember { target, name, .. } => {
            member_ty(&infer_depth(target, ctx, depth), name, ctx, depth)
        }
        Expr::Call { callee, .. } => call_return(callee, ctx, depth),
        Expr::New { class, .. } => match &**class {
            Expr::Ident(name, _) => {
                builtins::class_instance(name).unwrap_or_else(|| {
                    if ctx.structs.contains(name) {
                        Ty::Struct(name.clone())
                    } else {
                        Ty::Unknown
                    }
                })
            }
            _ => Ty::Unknown,
        },
        Expr::Is { .. } => Ty::Bool,
        Expr::Function { params: _, ret, body, .. } => {
            let ret_ty = ret
                .as_ref()
                .map(|a| ty_from_ast(a, ctx.structs))
                .or_else(|| body_return(body, ctx, depth + 1));
            Ty::Func(ret_ty.map(Box::new))
        }
    }
}

/// 标识符类型：沿作用域链查绑定 + 常量表
fn ident_ty(name: &str, ctx: &Ctx) -> Ty {
    if let Some(b) = ctx.lookup(name) {
        return b.ty.clone();
    }
    match name {
        "EMPTY" => Ty::Empty,
        "inf" | "nan" => Ty::Number,
        "fs" => Ty::Namespace("fs"),
        "sys" => Ty::Namespace("sys"),
        "true" | "false" => Ty::Bool,
        "null" => Ty::Null,
        _ => Ty::Unknown,
    }
}

/// `a.name` 的成员类型
fn member_ty(recv: &Ty, name: &str, ctx: &Ctx, depth: usize) -> Ty {
    match recv {
        Ty::Namespace(ns) => {
            // fs.x / sys.x 是函数；返回类型查表
            builtins::namespace_fns(ns)
                .and_then(|fns| fns.iter().find(|f| f.name == name))
                .map(|f| f.ret.clone())
                .unwrap_or(Ty::Unknown)
        }
        Ty::Object(fields) => fields
            .iter()
            .find(|f| f.name == name)
            .map(|f| f.ty.clone())
            .unwrap_or(Ty::Unknown),
        Ty::Struct(s) => {
            if let Some(info) = ctx.structs.get(s) {
                // 字段优先（与运行时一致），再查 impl 方法
                if let Some(p) = info.fields.iter().find(|p| p.name == name) {
                    return p
                        .ty
                        .as_ref()
                        .map(|a| ty_from_ast(a, ctx.structs))
                        .unwrap_or(Ty::Unknown);
                }
                if let Some(m) = info.methods.iter().find(|m| m.name == name) {
                    return method_ret_ty(m, ctx, depth);
                }
            }
            Ty::Unknown
        }
        // 内置类型方法链：`.insert(k, v).get(k)` 这类返回自身的方法
        _ => builtins::method_return(recv, name).unwrap_or(Ty::Unknown),
    }
}

/// struct 方法返回类型：`-> T` 标注优先，否则从 return 语句推导
fn method_ret_ty(m: &super::scope::FuncInfo, ctx: &Ctx, depth: usize) -> Ty {
    if let Some(ann) = &m.ret {
        return ty_from_ast(ann, ctx.structs);
    }
    body_return(&m.body, ctx, depth + 1).unwrap_or(Ty::Unknown)
}

/// 调用表达式的返回类型
fn call_return(callee: &Expr, ctx: &Ctx, depth: usize) -> Ty {
    match callee {
        Expr::Ident(name, _) => {
            // 用户函数优先（可覆盖内置）
            if let Some(b) = ctx.lookup(name) {
                if let Ty::Func(ret) = &b.ty {
                    return ret.as_deref().cloned().unwrap_or(Ty::Unknown);
                }
                // 绑定存在但不是函数：仍按函数调用，返回 Unknown
                return Ty::Unknown;
            }
            if let Some(f) = builtins::global_fn(name) {
                return f.ret.clone();
            }
            Ty::Unknown
        }
        // 方法调用 `recv.m()`：查表
        Expr::Member { target, name, .. } | Expr::OptionalMember { target, name, .. } => {
            let recv = infer_depth(target, ctx, depth);
            member_ty(&recv, name, ctx, depth)
        }
        // `(function(x){...})(...)` 立即调用
        f @ Expr::Function { .. } => {
            let t = infer_depth(f, ctx, depth);
            if let Ty::Func(ret) = t {
                ret.map(|b| *b).unwrap_or(Ty::Unknown)
            } else {
                Ty::Unknown
            }
        }
        _ => Ty::Unknown,
    }
}

/// 从函数体 return 语句推导返回类型：取第一个非 null 的类型，冲突则放弃
fn body_return(body: &Stmt, ctx: &Ctx, depth: usize) -> Option<Ty> {
    if depth > MAX_INFER_DEPTH {
        return None;
    }
    let mut result: Option<Ty> = None;
    walk_returns(body, &mut |value| {
        if let Some(v) = value {
            let t = infer_depth(v, ctx, depth);
            let t = if t == Ty::Null { Ty::Unknown } else { t };
            match &result {
                None => result = Some(t),
                Some(prev) if *prev != Ty::Unknown && *prev != t => {
                    // 冲突：放弃
                    result = Some(Ty::Unknown);
                }
                Some(Ty::Unknown) => result = Some(t),
                _ => {}
            }
        }
    });
    result.filter(|t| *t != Ty::Unknown)
}

/// FuncDecl 绑定的返回类型推导：参数压入临时帧后走 body_return
pub fn function_return(params: &[Param], body: &Stmt, ctx: &Ctx) -> Option<Ty> {
    let mut frames: Vec<HashMap<String, Binding>> = ctx.scopes.to_vec();
    let mut frame = HashMap::new();
    for p in params {
        let ty = p
            .ty
            .as_ref()
            .map(|a| ty_from_ast(a, ctx.structs))
            .unwrap_or(Ty::Unknown);
        frame.insert(
            p.name.clone(),
            Binding {
                detail: format!("{}: {}", p.name, ty.display()),
                name: p.name.clone(),
                ty,
                kind: super::ItemKind::Parameter,
                doc: "参数".into(),
                span: crate::Span::default(),
            },
        );
    }
    frames.push(frame);
    let inner = Ctx { scopes: &frames, structs: ctx.structs };
    body_return(body, &inner, 0)
}

/// 递归收集 return 语句的值（穿透所有嵌套块）
fn walk_returns(stmt: &Stmt, f: &mut impl FnMut(Option<&Expr>)) {
    match stmt {
        Stmt::Return { value, .. } => f(value.as_ref()),
        Stmt::Block { stmts, .. } => stmts.iter().for_each(|s| walk_returns(s, f)),
        Stmt::If { then_block, else_block, .. } => {
            walk_returns(then_block, f);
            if let Some(e) = else_block {
                walk_returns(e, f);
            }
        }
        Stmt::While { body, .. } | Stmt::ForIn { body, .. } | Stmt::ForC { body, .. } => {
            walk_returns(body, f)
        }
        // FuncDecl / impl 内的 return 属于别的函数，不穿透
        _ => {}
    }
}

/// 参数列表 → 签名串 `a: number, b`（标注存在时带上）
pub fn params_sig(params: &[Param]) -> String {
    params
        .iter()
        .map(|p| match &p.ty {
            Some(t) => format!("{}: {}", p.name, t),
            None => p.name.clone(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// 函数签名串：`function name(a: number, b) -> number`
pub fn func_sig(name: &str, params: &[Param], ret: Option<&TypeAst>) -> String {
    let mut s = format!("function {}({})", name, params_sig(params));
    if let Some(r) = ret {
        s.push_str(&format!(" -> {}", r));
    }
    s
}

/// 赋值运算符对变量类型的影响（`x += 1` 数字叠加等；Set 直接换类型）
pub fn assign_result_ty(op: AssignOp, old: &Ty, rhs: &Ty) -> Ty {
    match op {
        AssignOp::Set => rhs.clone(),
        AssignOp::Add => {
            if *old == Ty::Str || *rhs == Ty::Str {
                Ty::Str
            } else {
                Ty::Number
            }
        }
        AssignOp::Sub | AssignOp::Mul | AssignOp::Div | AssignOp::Mod => {
            if *old == Ty::Unknown {
                Ty::Unknown
            } else {
                Ty::Number
            }
        }
        AssignOp::Nullish => {
            if *old == Ty::Null {
                rhs.clone()
            } else {
                old.clone()
            }
        }
    }
}
