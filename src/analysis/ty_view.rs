//! 编辑器类型视图：展示名、标注薄委托与联合解析。
//!
//! 表达式类型推导已**单源**到 checker 引擎（`Checker` 光标模式，由
//! scope::Walker / semantics::Highlighter 驱动镜像链）；本模块只保留
//! 纯编辑器辅助：展示名约定（`editor_display`）、标注转换与复合赋值
//! 结果的薄委托（事实源在 `checker::ty`）、联合接收者的成员语境解析
//! （`member_recv`）与签名串构造。

use crate::ast::{AssignOp, Param, TypeAst};

use crate::checker::ty::{arith_result, from_ast, Type};

use super::scope::StructRegistry;

// ============ 编辑器展示名 ============

/// 编辑器展示名：与语言 `type()` 返回串对齐——Any 形参的容器显示裸名
/// （`array` / `map`），无参函数保持 `function` 习惯，其余同 checker 的
/// `Type::display`（诊断消息面向检查器受众，编辑器面向用户习惯，两套约定）。
pub fn editor_display(t: &Type) -> String {
    match t {
        Type::Array(e) if e.is_any() => "array".into(),
        Type::Map(k, v) if k.is_any() && v.is_any() => "map".into(),
        Type::Func(f) if f.params.is_empty() => match &f.ret {
            Some(r) => format!("function -> {}", editor_display(r)),
            None => "function".into(),
        },
        other => other.display(),
    }
}

// ============ 标注与赋值（薄委托 checker） ============

/// 类型标注 AST → Type（薄委托 checker 事实源；编辑器语境无泛型形参）。
/// 未识别的标识符若在 struct 注册表中则视为其实例，否则 Any。
pub fn ty_from_ast(ann: &TypeAst, structs: &StructRegistry) -> Type {
    from_ast(ann, &[], structs)
}

/// 赋值运算符对变量类型的影响（`x += 1` 数字叠加等；Set 直接换类型）。
/// 算术规则与 checker 同源（`arith_result`），Any 吸收为 Any（诚实优于臆测）。
pub fn assign_result_ty(op: AssignOp, old: &Type, rhs: &Type) -> Type {
    match op {
        AssignOp::Set => rhs.clone(),
        AssignOp::Add => arith_result("+", old, rhs).unwrap_or(Type::Number),
        AssignOp::Sub | AssignOp::Mul | AssignOp::Div | AssignOp::Mod => {
            if old.is_any() {
                Type::Any
            } else {
                Type::Number
            }
        }
        AssignOp::Nullish => {
            if matches!(old, Type::Null) {
                rhs.clone()
            } else {
                old.clone()
            }
        }
    }
}

// ============ 联合解析 ============

/// 成员语境的联合解析：`T | null` 去掉 null 后恰好一臂 → 该臂（编辑器按
/// 该臂给成员）；多臂 / 纯 null → None（调用方决定兜底）。非联合原样返回。
pub fn member_recv(t: &Type) -> Option<&Type> {
    match t {
        Type::Union(ms) => {
            let arms: Vec<&Type> = ms.iter().filter(|m| !matches!(m, Type::Null)).collect();
            match arms.as_slice() {
                [only] => Some(*only),
                _ => None,
            }
        }
        other => Some(other),
    }
}

// ============ 签名串 ============

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
