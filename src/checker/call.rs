//! 调用检查与构造检查（`new`）：实参兼容 + 泛型求解。

use std::collections::HashMap;

use crate::ast::{Expr, Param, TypeAst};
use crate::Span;

use super::builtins;
use super::ty::{assignable, from_ast, FuncShape, Type};
use super::unify::{subst, unify};
use super::Checker;

impl Checker {
    // ============ 调用检查 ============

    pub(crate) fn check_call(&mut self, callee: &Expr, args: &[Expr], span: Span) -> Type {
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
                // 内置全局函数：实参个数由事实源表的 arity 声明（None = 可变参）
                if let Some(ret) = builtins::global_fn_return(name) {
                    if let Some(expected) = builtins::global_fn_info(name)
                        .and_then(|f| f.arity)
                        .filter(|_| !builtins::is_variadic_global(name))
                    {
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

    pub(crate) fn check_new(&mut self, class: &Expr, args: &[Expr], span: Span) -> Type {
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
