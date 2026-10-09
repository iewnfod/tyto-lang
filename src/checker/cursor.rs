//! 光标模式：编辑器（analysis）在哨兵容错 AST 上驱动检查器引擎。
//!
//! 与 `check_program` 的全文件诊断遍历不同，光标模式由 analysis 的 Walker
//! 按 span 门控驱动（何时登记绑定、进入哪层作用域由编辑器决定），
//! 这里只提供引擎构造入口与编辑器特有的推导能力（无标注函数的返回
//! 类型推导）。诊断照常收集但调用方忽略——残缺代码上的红线无意义，
//! 引擎对未知名（含哨兵 `__tyto_cx__`）静默落 Any，不会因光标语境报错。

use std::rc::Rc;

use crate::ast::{Expr, Param, Stmt, TypeAst};

use super::registry::StructRegistry;
use super::ty::{self as cty, FuncShape, Type};
use super::{Binding, Checker, FuncCallInfo};

impl Binding {
    /// 光标模式镜像登记用的普通绑定（无约束、无函数签名）
    pub(crate) fn plain(ty: Type) -> Binding {
        Binding { ty, is_const: false, declared: None, let_decl: None, func: None }
    }
}

impl Checker {
    /// 光标模式构造：空作用域链 + 容错解析得到的 struct 注册表。
    pub(crate) fn for_cursor(structs: StructRegistry) -> Checker {
        Checker {
            structs,
            scopes: vec![std::collections::HashMap::new()],
            ret_stack: Vec::new(),
            saw_return: Vec::new(),
            generics: Vec::new(),
            diags: Vec::new(),
            types: std::collections::HashMap::new(),
        }
    }

    /// 光标模式镜像登记：与 analysis::scope::chain_insert 语义一致——
    /// 命中已有绑定的层就更新（无 const/declared 守卫，编辑器视角宽松），
    /// 否则插入最内层。走查器每登记一个编辑器绑定时成对调用。
    pub(crate) fn mirror_define(&mut self, name: &str, ty: Type, func: Option<Rc<FuncCallInfo>>) {
        for scope in self.scopes.iter_mut().rev() {
            if let Some(b) = scope.get_mut(name) {
                b.ty = ty;
                b.func = func;
                return;
            }
        }
        self.scopes.last_mut().unwrap().insert(
            name.to_string(),
            Binding { ty, is_const: false, declared: None, let_decl: None, func },
        );
    }

    /// 链上登记函数绑定：函数形状带参数标注（调用点推导用）+ 原始签名
    /// （泛型求解用）。`ret_ty` 由调用方决定（标注或 [`Self::infer_func_ret`]）。
    ///
    /// `FuncCallInfo` 仅在返回有标注时挂：无标注时 `apply_func_info` 恒回
    /// Any，而形状里的引擎推导返回类型（`ret_ty`）更精确，走形状路径。
    pub(crate) fn mirror_define_func(
        &mut self,
        name: &str,
        params: &[Param],
        type_params: &[String],
        ret: &Option<TypeAst>,
        ret_ty: Option<Type>,
    ) {
        let shape_params: Vec<Option<Type>> = params
            .iter()
            .map(|p| p.ty.as_ref().map(|a| cty::from_ast(a, type_params, &self.structs)))
            .collect();
        let shape = Type::Func(Rc::new(FuncShape { params: shape_params, ret: ret_ty }));
        let func = ret.is_some().then(|| {
            Rc::new(FuncCallInfo {
                params: params.to_vec(),
                type_params: type_params.to_vec(),
                ret: ret.clone(),
            })
        });
        self.mirror_define(name, shape, func);
    }

    /// 无标注函数的返回类型推导（编辑器补全/悬停用）：
    /// 压参数作用域 → 穿透收集 return → 取第一个非 null 类型，冲突弃权。
    /// 引擎的 `Expr::Function` 只看标注不推函数体，无递归环，无需深度守卫。
    pub(crate) fn infer_func_ret(&mut self, params: &[Param], body: &Stmt) -> Option<Type> {
        self.push_scope();
        self.bind_params(params);
        let mut returns: Vec<&Expr> = Vec::new();
        collect_returns(body, &mut returns);
        let mut result: Option<Type> = None;
        for value in returns {
            let t = self.infer(value);
            // null 返回不参与判定（空 return 习惯）
            let t = if matches!(t, Type::Null) { Type::Any } else { t };
            match &result {
                None => result = Some(t),
                // 冲突（两个不同的具体类型）：弃权
                Some(prev) if !prev.is_any() && *prev != t => result = Some(Type::Any),
                Some(prev) if prev.is_any() => result = Some(t),
                _ => {}
            }
        }
        self.pop_scope();
        result.filter(|t| !t.is_any())
    }
}

/// 递归收集 return 语句的值（穿透嵌套块；函数字面量 / impl 方法内的
/// return 属于别的函数，不穿透）
fn collect_returns<'a>(stmt: &'a Stmt, out: &mut Vec<&'a Expr>) {
    match stmt {
        Stmt::Return { value: Some(v), .. } => out.push(v),
        Stmt::Block { stmts, .. } => stmts.iter().for_each(|s| collect_returns(s, out)),
        Stmt::If { then_block, else_block, .. } => {
            collect_returns(then_block, out);
            if let Some(e) = else_block {
                collect_returns(e, out);
            }
        }
        Stmt::While { body, .. } | Stmt::ForIn { body, .. } | Stmt::ForC { body, .. } => {
            collect_returns(body, out)
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::rc::Rc;
    use crate::lexer::Lexer;
    use crate::parser::Parser;

    /// 解析单函数程序，取其参数与函数体
    fn func_parts(src: &str) -> (Vec<Param>, Rc<Stmt>) {
        let out = Lexer::new(src).tokenize().unwrap();
        let program = Parser::new(out.tokens).parse_program().unwrap();
        let stmts = match &program {
            Stmt::Block { stmts, .. } => stmts,
            _ => panic!("程序应为 Block"),
        };
        match &stmts[0] {
            Stmt::FuncDecl { params, body, .. } => (params.clone(), Rc::clone(body)),
            _ => panic!("首条语句应为 FuncDecl"),
        }
    }

    #[test]
    fn infers_single_return() {
        let (params, body) = func_parts("function make() {\n    return \"hello\"\n}\n");
        let mut ck = Checker::for_cursor(StructRegistry::default());
        assert_eq!(ck.infer_func_ret(&params, &body), Some(Type::Str));
    }

    #[test]
    fn conflicting_returns_give_up() {
        let (params, body) = func_parts(
            "function f(c) {\n    if c {\n        return 1\n    }\n    return \"s\"\n}\n",
        );
        let mut ck = Checker::for_cursor(StructRegistry::default());
        assert_eq!(ck.infer_func_ret(&params, &body), None);
    }

    #[test]
    fn no_returns_gives_none() {
        let (params, body) = func_parts("function f() {\n    println(1)\n}\n");
        let mut ck = Checker::for_cursor(StructRegistry::default());
        assert_eq!(ck.infer_func_ret(&params, &body), None);
    }

    #[test]
    fn params_visible_in_returns() {
        let (params, body) = func_parts("function f(x: number) {\n    return x\n}\n");
        let mut ck = Checker::for_cursor(StructRegistry::default());
        assert_eq!(ck.infer_func_ret(&params, &body), Some(Type::Number));
    }
}
