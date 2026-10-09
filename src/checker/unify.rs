//! 泛型求解（简化合一）：形参 TypeAst 与实参 Type 配对收集 TypeVar 解。

use std::collections::HashMap;
use std::rc::Rc;

use crate::ast::TypeAst;

use super::ty::{normalize_union, FuncShape, Type};

// ============ 泛型求解（简化合一） ============

/// 形参 TypeAst 与实参 Type 配对，收集 TypeVar 解。
/// 冲突（同一 TypeVar 绑到不同类型）→ 回退 Any（绝不误报）。
pub(crate) fn unify(ast: &TypeAst, t: &Type, type_params: &[String], sols: &mut HashMap<String, Type>) {
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
