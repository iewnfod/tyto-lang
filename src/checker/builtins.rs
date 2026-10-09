//! checker 侧内置类型知识：方法集与返回类型（元素/键值类型感知）。
//!
//! 与 analysis::builtins（补全/悬停用）数据同源——修改内置库（src/natives）时
//! 两边同步维护；tests 中有方法名一致性测试兜底。
//! 依赖方向纪律：checker 不依赖 analysis，本表使用 checker::ty::Type。

use super::ty::{normalize_union, FuncShape, Type};
use std::rc::Rc;

/// 方法条目：名字 + 返回类型构造器（接收者类型 → 返回类型）
struct MethodRet {
    name: &'static str,
    ret: fn(&Type) -> Type,
}

fn t_number(_: &Type) -> Type {
    Type::Number
}
fn t_bool(_: &Type) -> Type {
    Type::Bool
}
fn t_str(_: &Type) -> Type {
    Type::Str
}
fn t_null(_: &Type) -> Type {
    Type::Null
}
fn t_self(t: &Type) -> Type {
    t.clone()
}
fn t_any(_: &Type) -> Type {
    Type::Any
}
/// 数组元素类型（接收者是 Array<T> 时取 T，否则 Any）
fn elem_of(t: &Type) -> Type {
    match t {
        Type::Array(e) => (**e).clone(),
        _ => Type::Any,
    }
}
fn array_pop(t: &Type) -> Type {
    normalize_union(vec![elem_of(t), Type::Null])
}

const ARRAY_METHODS: &[MethodRet] = &[
    MethodRet { name: "push", ret: t_number },
    MethodRet { name: "pop", ret: array_pop },
    MethodRet { name: "len", ret: t_number },
    MethodRet { name: "is_empty", ret: t_bool },
    MethodRet { name: "contains", ret: t_bool },
    MethodRet { name: "index_of", ret: t_number },
    MethodRet { name: "join", ret: t_str },
    MethodRet { name: "sort", ret: t_self },
    MethodRet { name: "reverse", ret: t_self },
    MethodRet { name: "slice", ret: t_self },
    MethodRet { name: "map", ret: t_self },
    MethodRet { name: "filter", ret: t_self },
    MethodRet { name: "fold", ret: t_any },
];

const STRING_METHODS: &[MethodRet] = &[
    MethodRet { name: "len", ret: t_number },
    MethodRet { name: "is_empty", ret: t_bool },
    MethodRet { name: "split", ret: |_| Type::Array(Box::new(Type::Str)) },
    MethodRet { name: "contains", ret: t_bool },
    MethodRet { name: "index_of", ret: t_number },
    MethodRet { name: "trim", ret: t_str },
    MethodRet { name: "starts_with", ret: t_bool },
    MethodRet { name: "ends_with", ret: t_bool },
    MethodRet { name: "to_uppercase", ret: t_str },
    MethodRet { name: "to_lowercase", ret: t_str },
    MethodRet { name: "sub", ret: t_str },
    MethodRet { name: "replace", ret: t_str },
    MethodRet { name: "chars", ret: |_| Type::Array(Box::new(Type::Str)) },
];

const MAP_METHODS: &[MethodRet] = &[
    MethodRet { name: "get", ret: |t| map_value_or_null(t) },
    MethodRet { name: "insert", ret: t_self },
    MethodRet { name: "contains_key", ret: t_bool },
    MethodRet { name: "remove", ret: t_bool },
    MethodRet { name: "len", ret: t_number },
    MethodRet { name: "is_empty", ret: t_bool },
    MethodRet { name: "keys", ret: |t| Type::Array(Box::new(map_key_of(t))) },
    MethodRet { name: "values", ret: |t| Type::Array(Box::new(map_value_of(t))) },
];

fn map_key_of(t: &Type) -> Type {
    match t {
        Type::Map(k, _) => (**k).clone(),
        _ => Type::Any,
    }
}
fn map_value_of(t: &Type) -> Type {
    match t {
        Type::Map(_, v) => (**v).clone(),
        _ => Type::Any,
    }
}
fn map_value_or_null(t: &Type) -> Type {
    normalize_union(vec![map_value_of(t), Type::Null])
}

const HEAP_METHODS: &[MethodRet] = &[
    MethodRet { name: "push", ret: t_null },
    MethodRet { name: "pop", ret: |_| normalize_union(vec![Type::Number, Type::Empty]) },
    MethodRet { name: "peek", ret: |_| normalize_union(vec![Type::Number, Type::Empty]) },
    MethodRet { name: "len", ret: t_number },
    MethodRet { name: "is_empty", ret: t_bool },
];

const STACK_METHODS: &[MethodRet] = &[
    MethodRet { name: "push", ret: t_null },
    MethodRet { name: "pop", ret: |_| normalize_union(vec![Type::Any, Type::Empty]) },
    MethodRet { name: "peek", ret: |_| normalize_union(vec![Type::Any, Type::Empty]) },
    MethodRet { name: "len", ret: t_number },
    MethodRet { name: "is_empty", ret: t_bool },
];

const QUEUE_METHODS: &[MethodRet] = &[
    MethodRet { name: "push_back", ret: t_null },
    MethodRet { name: "pop_front", ret: |_| normalize_union(vec![Type::Any, Type::Empty]) },
    MethodRet { name: "front", ret: |_| normalize_union(vec![Type::Any, Type::Empty]) },
    MethodRet { name: "back", ret: |_| normalize_union(vec![Type::Any, Type::Empty]) },
    MethodRet { name: "len", ret: t_number },
    MethodRet { name: "is_empty", ret: t_bool },
];

/// 接收者是内置类型时：方法是否存在于该类型
pub fn has_method(recv: &Type, name: &str) -> bool {
    table_for(recv).map(|t| t.iter().any(|m| m.name == name)).unwrap_or(false)
}

/// 内置方法的返回类型（接收者必须已是内置类型）
pub fn method_return(recv: &Type, name: &str) -> Option<Type> {
    let table = table_for(recv)?;
    table.iter().find(|m| m.name == name).map(|m| (m.ret)(recv))
}

fn table_for(recv: &Type) -> Option<&'static [MethodRet]> {
    match recv {
        Type::Array(_) => Some(ARRAY_METHODS),
        Type::Str => Some(STRING_METHODS),
        Type::Map(..) => Some(MAP_METHODS),
        Type::MaxHeap | Type::MinHeap => Some(HEAP_METHODS),
        Type::Stack => Some(STACK_METHODS),
        Type::Queue => Some(QUEUE_METHODS),
        _ => None,
    }
}

/// 内置方法的形参期望（只编码运行时严格或语义明确的少数方法）。
/// 返回 (期望形参类型, 是否变长)；None = 不做实参检查。
/// 变长：至少出现过的实参与期望逐个比对（如 push(x, ...)）。
pub fn method_params(recv: &Type, name: &str) -> Option<(Vec<Type>, bool)> {
    match (recv, name) {
        // 数组元素类型约束 push 的每个实参（标注语义）
        (Type::Array(e), "push") => Some((vec![(**e).clone()], true)),
        // Map 键值类型约束
        (Type::Map(k, v), "insert") => Some((vec![(**k).clone(), (**v).clone()], false)),
        (Type::Map(k, _), "get" | "contains_key" | "remove") => {
            Some((vec![(**k).clone()], false))
        }
        // 堆只存数字（运行时强制）；empty 哨兵按惯用形放行（median 流等靠
        // len 不变量保证非空——与算术投影的 empty 宽容一致）
        (Type::MaxHeap, "push") | (Type::MinHeap, "push") => {
            Some((vec![normalize_union(vec![Type::Number, Type::Empty])], false))
        }
        // 字符串方法的定长字符串实参
        (
            Type::Str,
            "split" | "contains" | "index_of" | "starts_with" | "ends_with",
        ) => Some((vec![Type::Str], false)),
        (Type::Str, "replace") => Some((vec![Type::Str, Type::Str], false)),
        _ => None,
    }
}

/// 全局函数返回类型（println/len/num/...）
pub fn global_fn_return(name: &str) -> Option<Type> {
    Some(match name {
        "print" | "println" => Type::Null,
        "input" => normalize_union(vec![Type::Str, Type::Null]),
        "num" | "floor" | "ceil" | "round" | "abs" | "sqrt" | "pow" | "min" | "max" => Type::Number,
        "str" | "type" => Type::Str,
        "len" => Type::Number,
        "has" => Type::Bool,
        _ => return None,
    })
}

/// 命名空间函数返回类型：fs.* / sys.*
pub fn namespace_fn_return(ns: &str, name: &str) -> Option<Type> {
    Some(match (ns, name) {
        ("fs", "read_file") => Type::Str,
        ("fs", "read_lines") => Type::Array(Box::new(Type::Str)),
        ("fs", "write_file") | ("fs", "append_file") => Type::Null,
        ("fs", "exists") => Type::Bool,
        ("fs", "list_dir") => Type::Array(Box::new(Type::Str)),
        ("sys", "shell") => Type::Object(Rc::from(vec![
            ("status".into(), Type::Number),
            ("stdout".into(), Type::Str),
            ("stderr".into(), Type::Str),
        ])),
        ("sys", "get_env") => normalize_union(vec![Type::Str, Type::Null]),
        ("sys", "args") => Type::Array(Box::new(Type::Str)),
        _ => return None,
    })
}

/// `new X(...)` 的实例类型（X 为内置类）
pub fn class_instance(name: &str) -> Option<Type> {
    Some(match name {
        "Map" => Type::Map(Box::new(Type::Any), Box::new(Type::Any)),
        "MaxHeap" => Type::MaxHeap,
        "MinHeap" => Type::MinHeap,
        "Stack" => Type::Stack,
        "Queue" => Type::Queue,
        _ => return None,
    })
}

pub fn is_builtin_class(name: &str) -> bool {
    matches!(name, "Map" | "MaxHeap" | "MinHeap" | "Stack" | "Queue")
}

/// 全局常量类型
pub fn constant_ty(name: &str) -> Option<Type> {
    Some(match name {
        "EMPTY" => Type::Empty,
        "inf" | "nan" => Type::Number,
        _ => return None,
    })
}

/// 全局函数是否可变参（print/println/min/max）：跳过实参个数检查
pub fn is_variadic_global(name: &str) -> bool {
    matches!(name, "print" | "println" | "min" | "max")
}

/// 无参函数形状（辅助构造）
pub fn func_shape(params: Vec<Option<Type>>, ret: Option<Type>) -> Type {
    Type::Func(Rc::new(FuncShape { params, ret }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn array_methods_are_element_aware() {
        let arr = Type::Array(Box::new(Type::Number));
        assert_eq!(method_return(&arr, "pop").unwrap().display(), "number | null");
        assert_eq!(method_return(&arr, "map").unwrap().display(), "number[]");
    }

    #[test]
    fn map_get_is_nullable_value() {
        let m = Type::Map(Box::new(Type::Str), Box::new(Type::Number));
        assert_eq!(method_return(&m, "get").unwrap().display(), "number | null");
        assert_eq!(method_return(&m, "keys").unwrap().display(), "string[]");
    }
}
