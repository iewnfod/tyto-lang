//! 表达式类型推导与使用点检查（赋值兼容性见 ty::assignable）。
//!
//! 推不出的地方一律 [`Type::Any`]（渐进承诺：绝不因「不知道」报错）；
//! 报错只发生在「明确知道且明确不匹配」的地方。

use std::collections::HashMap;
use std::rc::Rc;

use crate::ast::{BinaryOp, Expr, LogicOp, Param, TypeAst, UnaryOp};
use crate::Span;

use super::builtins;
use super::registry::FuncInfo;
use super::ty::{
    arith_result, assignable, from_ast, normalize_union, without_null, FuncShape, Type,
};
use super::Checker;

impl Checker {
    /// 表达式推导主入口（同时做使用点检查：调用实参 / 成员存在性 / 算术操作数）
    pub(crate) fn infer(&mut self, expr: &Expr) -> Type {
        let t = self.infer_inner(expr);
        self.types.insert(expr.span(), t.clone());
        t
    }

    fn infer_inner(&mut self, expr: &Expr) -> Type {
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
    fn member_ty(&mut self, recv: &Type, name: &str, span: Span, optional: bool) -> Type {
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

    // ============ 调用检查 ============

    fn check_call(&mut self, callee: &Expr, args: &[Expr], span: Span) -> Type {
        // 实参先推导（调用前有副作用的表达式也要查）
        let arg_tys: Vec<Type> = args.iter().map(|a| self.infer(a)).collect();
        match callee {
            Expr::Ident(name, _) => {
                // 克隆绑定数据，避免不可变借用横跨后续可变借用
                let found = self.lookup(name).cloned();
                if let Some(b) = found {
                    if let Some(info) = &b.func {
                        return self.apply_func_info(
                            &info.params,
                            &info.type_params,
                            &info.ret,
                            &arg_tys,
                            span,
                        );
                    }
                    if let Type::Func(shape) = &b.ty {
                        let shape = shape.clone();
                        return self.apply_shape(&shape, &arg_tys, span, false);
                    }
                    // 绑定存在但不是函数：动态语言允许（运行时报错），编辑器不重复报
                    return Type::Any;
                }
                // 内置全局函数
                if let Some(ret) = builtins::global_fn_return(name) {
                    let fixed_arity: Option<usize> = match name.as_str() {
                        "pow" | "has" => Some(2),
                        "len" | "num" | "str" | "type" | "floor" | "ceil" | "round" | "abs"
                        | "sqrt" | "input" => Some(1),
                        _ => None, // print/println/min/max 可变参
                    };
                    if let Some(expected) = fixed_arity {
                        if args.len() != expected {
                            self.err(
                                span,
                                format!(
                                    "`{}` 期望 {} 个参数，实际传入 {} 个",
                                    name,
                                    expected,
                                    args.len()
                                ),
                            );
                        }
                    }
                    return ret;
                }
                Type::Any
            }
            Expr::Member { target, name, .. } | Expr::OptionalMember { target, name, .. } => {
                let recv = self.infer(target);
                match &recv {
                    Type::Struct(..) => {
                        // 成员存在性（字段/方法）经 member_ty 检查；方法是代入后的函数形状
                        let mt = self.member_ty(&recv, name, span, false);
                        match mt {
                            Type::Func(shape) => self.apply_shape(&shape, &arg_tys, span, false),
                            _ => Type::Any,
                        }
                    }
                    Type::Namespace(_) => {
                        // 存在性 + 返回类型统一由 member_ty 处理（未知成员报错）
                        match self.member_ty(&recv, name, span, false) {
                            Type::Func(shape) => shape.ret.clone().unwrap_or(Type::Any),
                            _ => Type::Any,
                        }
                    }
                    Type::Object(fields) => match fields.iter().find(|(n, _)| n == name) {
                        Some((_, Type::Func(shape))) => {
                            let shape = shape.clone();
                            self.apply_shape(&shape, &arg_tys, span, false)
                        }
                        _ => Type::Any,
                    },
                    _ => {
                        // 内置类型方法：存在性 + 返回类型由 member_ty 处理；实参查 method_params 表
                        let ret = self.member_ty(&recv, name, span, false);
                        if let Some((params, variadic)) = builtins::method_params(&recv, name) {
                            if !variadic && arg_tys.len() != params.len() {
                                self.err(
                                    span,
                                    format!(
                                        "`.{}` 期望 {} 个参数，实际传入 {} 个",
                                        name,
                                        params.len(),
                                        arg_tys.len()
                                    ),
                                );
                            }
                            for (i, (pt, at)) in params.iter().zip(arg_tys.iter()).enumerate() {
                                if !assignable(at, pt) {
                                    self.err(
                                        span,
                                        format!(
                                            "第 {} 个参数类型不匹配：期望 `{}`，实际 `{}`",
                                            i + 1,
                                            pt.display(),
                                            at.display()
                                        ),
                                    );
                                }
                            }
                        }
                        ret
                    }
                }
            }
            f @ Expr::Function { params, type_params, ret, .. } => {
                let shape = self.infer_inner(f);
                let _ = (params, type_params, ret);
                if let Type::Func(shape) = shape {
                    return self.apply_shape(&shape, &arg_tys, span, false);
                }
                Type::Any
            }
            _ => Type::Any,
        }
    }

    /// 用户函数调用：泛型求解 + 实参逐个兼容检查
    fn apply_func_info(
        &mut self,
        params: &[Param],
        type_params: &[String],
        ret: &Option<TypeAst>,
        arg_tys: &[Type],
        span: Span,
    ) -> Type {
        let mut sols = HashMap::new();
        self.apply_params_ast(params, type_params, ret.as_ref(), arg_tys, span, &mut sols)
    }

    /// 参数标注驱动：求解泛型 → 检查实参 → 代入返回类型
    fn apply_params_ast(
        &mut self,
        params: &[Param],
        type_params: &[String],
        ret: Option<&TypeAst>,
        arg_tys: &[Type],
        span: Span,
        sols: &mut HashMap<String, Type>,
    ) -> Type {
        // 形参个数（运行时严格匹配用户函数实参个数）
        if arg_tys.len() != params.len() {
            let names: Vec<&str> = params.iter().map(|p| p.name.as_str()).collect();
            self.err(
                span,
                format!(
                    "函数期望 {} 个参数（{}），实际传入 {} 个",
                    params.len(),
                    names.join(", "),
                    arg_tys.len()
                ),
            );
        }
        // 泛型求解：形参 TypeAst vs 实参 Type 位置配对
        if !type_params.is_empty() {
            for (p, at) in params.iter().zip(arg_tys.iter()) {
                if let Some(a) = &p.ty {
                    unify(a, at, type_params, sols);
                }
            }
        }
        // 实参兼容检查（TypeVar 已代入）
        for (p, at) in params.iter().zip(arg_tys.iter()) {
            if let Some(a) = &p.ty {
                let pt = from_ast(a, type_params, &self.structs);
                let pt = subst(&pt, sols);
                if !assignable(at, &pt) {
                    self.err(
                        span,
                        format!(
                            "参数 `{}` 类型不匹配：期望 `{}`，实际 `{}`",
                            p.name,
                            pt.display(),
                            at.display()
                        ),
                    );
                }
            }
        }
        match ret {
            Some(r) => {
                let rt = from_ast(r, type_params, &self.structs);
                subst(&rt, sols)
            }
            None => Type::Any,
        }
    }

    /// 无标注形状（函数字面量 / 对象字段函数）：只查有标注的参数
    fn apply_shape(
        &mut self,
        shape: &FuncShape,
        arg_tys: &[Type],
        span: Span,
        skip_arity: bool,
    ) -> Type {
        if !skip_arity && shape.params.len() != arg_tys.len() {
            // 函数字面量的参数个数运行时同样严格；但作为值传递的回调宽松
            self.err(
                span,
                format!(
                    "函数期望 {} 个参数，实际传入 {} 个",
                    shape.params.len(),
                    arg_tys.len()
                ),
            );
        }
        for (pt, at) in shape.params.iter().zip(arg_tys.iter()) {
            if let (Some(p), a) = (pt, at) {
                if !assignable(a, p) {
                    self.err(
                        span,
                        format!(
                            "参数类型不匹配：期望 `{}`，实际 `{}`",
                            p.display(),
                            a.display()
                        ),
                    );
                }
            }
        }
        shape.ret.clone().unwrap_or(Type::Any)
    }

    // ============ 构造检查 ============

    fn check_new(&mut self, class: &Expr, args: &[Expr], span: Span) -> Type {
        let arg_tys: Vec<Type> = args.iter().map(|a| self.infer(a)).collect();
        let Expr::Ident(name, _) = class else { return Type::Any };
        if let Some(t) = builtins::class_instance(name) {
            return t;
        }
        // 克隆注册表项，避免不可变借用横跨 self.err 的可变借用
        let Some(info) = self.structs.get(name).cloned() else {
            return Type::Any;
        };
        let has_custom_new = info.methods.iter().any(|m| m.name == "new");
        if !has_custom_new {
            // 位置构造：实参与字段逐位对应，泛型实参从「字段标注 vs 实参类型」求解
            if arg_tys.len() > info.fields.len() {
                self.err(
                    span,
                    format!(
                        "new {}() 最多接受 {} 个参数（字段数），实际传入 {} 个",
                        name,
                        info.fields.len(),
                        arg_tys.len()
                    ),
                );
            }
            let mut sols: HashMap<String, Type> = HashMap::new();
            for (f, at) in info.fields.iter().zip(arg_tys.iter()) {
                if let Some(a) = &f.ty {
                    unify(a, at, &info.type_params, &mut sols);
                }
            }
            for (f, at) in info.fields.iter().zip(arg_tys.iter()) {
                if let Some(a) = &f.ty {
                    let ft = from_ast(a, &info.type_params, &self.structs);
                    let ft = subst(&ft, &sols);
                    if !assignable(at, &ft) {
                        self.err(
                            span,
                            format!(
                                "字段 `{}` 类型不匹配：期望 `{}`，实际 `{}`",
                                f.name,
                                ft.display(),
                                at.display()
                            ),
                        );
                    }
                }
            }
            let type_args: Vec<Type> = info
                .type_params
                .iter()
                .map(|t| sols.get(t).cloned().unwrap_or(Type::Any))
                .collect();
            return Type::Struct(name.to_string(), type_args);
        }
        // 自定义 new：按方法签名检查实参（返回值运行时固定为实例，忽略方法返回类型）。
        // 同时求解 struct 类型实参：`function new<T>(val: T)` 与 `function new(val: T)`
        // （T 来自 impl/struct 声明）两种惯用形都支持——按名匹配 struct 的泛型参数。
        if let Some(m) = info.methods.iter().find(|m| m.name == "new") {
            let mut sols: HashMap<String, Type> = HashMap::new();
            let _ = self.apply_params_ast(
                &m.params,
                &m.type_params,
                m.ret.as_ref(),
                &arg_tys,
                span,
                &mut sols,
            );
            // 方法自身未声明同名参数时，用 struct 的泛型参数名再解一轮
            for (p, at) in m.params.iter().zip(arg_tys.iter()) {
                if let Some(a) = &p.ty {
                    unify(a, at, &info.type_params, &mut sols);
                }
            }
            let type_args: Vec<Type> = info
                .type_params
                .iter()
                .map(|t| sols.get(t).cloned().unwrap_or(Type::Any))
                .collect();
            return Type::Struct(name.to_string(), type_args);
        }
        Type::Struct(name.to_string(), Vec::new())
    }
}

// ============ 泛型求解（简化合一） ============

/// 形参 TypeAst 与实参 Type 配对，收集 TypeVar 解。
/// 冲突（同一 TypeVar 绑到不同类型）→ 回退 Any（绝不误报）。
fn unify(ast: &TypeAst, t: &Type, type_params: &[String], sols: &mut HashMap<String, Type>) {
    match ast {
        TypeAst::Named(n, _) if type_params.contains(n) => {
            let entry = sols.remove(n).unwrap_or_else(|| t.clone());
            let merged = if entry == *t || t.is_any() || entry.is_any() {
                if t.is_any() { entry } else { t.clone() }
            } else {
                Type::Any // 冲突：放弃该参数的精确性
            };
            sols.insert(n.clone(), merged);
        }
        TypeAst::Array(inner, _) => {
            if let Type::Array(e) = t {
                unify(inner, e, type_params, sols);
            }
        }
        TypeAst::Generic(n, args, _) if n == "Array" => {
            if let (Some(inner), Type::Array(e)) = (args.first(), t) {
                unify(inner, e, type_params, sols);
            }
        }
        TypeAst::Generic(n, args, _) => {
            if let Type::Struct(n2, targs) = t {
                if n == n2 {
                    for (a, ta) in args.iter().zip(targs.iter()) {
                        unify(a, ta, type_params, sols);
                    }
                }
            }
        }
        TypeAst::Union(ms, _) => {
            if let Type::Union(ts) = t {
                for (m, tt) in ms.iter().zip(ts.iter()) {
                    unify(m, tt, type_params, sols);
                }
            }
        }
        _ => {}
    }
}

/// TypeVar 代入（未解出的 → Any）
pub(crate) fn subst(t: &Type, sols: &HashMap<String, Type>) -> Type {
    match t {
        Type::TypeVar(n) => sols.get(n).cloned().unwrap_or(Type::Any),
        Type::Array(e) => Type::Array(Box::new(subst(e, sols))),
        Type::Map(k, v) => {
            Type::Map(Box::new(subst(k, sols)), Box::new(subst(v, sols)))
        }
        Type::Struct(n, args) => {
            Type::Struct(n.clone(), args.iter().map(|a| subst(a, sols)).collect())
        }
        Type::Object(fs) => Type::Object(Rc::from(
            fs.iter().map(|(n, t)| (n.clone(), subst(t, sols))).collect::<Vec<_>>(),
        )),
        Type::Func(f) => Type::Func(Rc::new(FuncShape {
            params: f.params.iter().map(|p| p.as_ref().map(|t| subst(t, sols))).collect(),
            ret: f.ret.as_ref().map(|t| subst(t, sols)),
        })),
        Type::Union(ms) => {
            normalize_union(ms.iter().map(|m| subst(m, sols)).collect())
        }
        other => other.clone(),
    }
}
