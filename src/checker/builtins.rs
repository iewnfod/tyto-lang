//! checker 侧内置类型知识：方法集与返回类型（元素/键值类型感知）。
//!
//! **单一事实来源**：内置方法表 / 全局函数 / fs·sys 命名空间函数 / 内置类的
//! 名字、签名串、文档与返回类型都以本表为准——analysis::builtins（补全/悬停）
//! 直接转发消费（类型表示已统一为 checker::ty::Type，无转换层），
//! editor 侧不得另建副本。
//! 修改内置库（src/natives）时只改这里（另有 editors/vscode 的 JS 静态降级表
//! 需同步，见 AGENTS.md 的同步点清单；tests/builtins_consistency_test.rs 兜底）。
//! 依赖方向纪律：checker 不依赖 analysis，本表使用 checker::ty::Type。

use super::ty::{normalize_union, FuncShape, Type};
use std::rc::Rc;

/// 方法条目：名字 + 展示签名 + 文档 + 返回类型构造器（接收者类型 → 返回类型）
pub struct MethodRet {
    pub name: &'static str,
    /// 展示签名（补全 detail / 悬停用，如 `.push(x, ...) → number`）
    pub sig: &'static str,
    /// 中文文档
    pub doc: &'static str,
    pub ret: fn(&Type) -> Type,
}

/// 全局函数条目：名字 + 签名 + 文档 + 返回类型构造器 + 实参个数（None = 可变参）
/// （返回类型用零参构造器：静态表无法调用非常量函数）
pub struct GlobalFnInfo {
    pub name: &'static str,
    pub sig: &'static str,
    pub doc: &'static str,
    pub ret: fn() -> Type,
    pub arity: Option<usize>,
}

/// 命名空间函数条目（fs.* / sys.*）
pub struct NsFnInfo {
    pub name: &'static str,
    pub sig: &'static str,
    pub doc: &'static str,
    pub ret: fn() -> Type,
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

// 零参返回类型构造器（全局/命名空间函数静态表用）
fn g_null() -> Type {
    Type::Null
}
fn g_number() -> Type {
    Type::Number
}
fn g_str() -> Type {
    Type::Str
}
fn g_bool() -> Type {
    Type::Bool
}
fn g_str_array() -> Type {
    Type::Array(Box::new(Type::Str))
}
fn g_str_or_null() -> Type {
    normalize_union(vec![Type::Str, Type::Null])
}
fn g_shell_obj() -> Type {
    Type::Object(Rc::from(vec![
        ("status".into(), Type::Number),
        ("stdout".into(), Type::Str),
        ("stderr".into(), Type::Str),
    ]))
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
    MethodRet { name: "push", sig: ".push(x, ...) → number", doc: "追加所有参数，返回新长度", ret: t_number },
    MethodRet { name: "pop", sig: ".pop() → value", doc: "移除并返回末尾元素；空数组返回 null", ret: array_pop },
    MethodRet { name: "len", sig: ".len() → number", doc: "元素数", ret: t_number },
    MethodRet { name: "is_empty", sig: ".is_empty() → bool", doc: "是否为空", ret: t_bool },
    MethodRet { name: "contains", sig: ".contains(x) → bool", doc: "是否包含（按语言 == 语义）", ret: t_bool },
    MethodRet { name: "index_of", sig: ".index_of(x) → number", doc: "首次出现的下标；未找到 -1", ret: t_number },
    MethodRet { name: "join", sig: ".join(sep?) → string", doc: "连接为字符串；默认 \",\"", ret: t_str },
    MethodRet { name: "sort", sig: ".sort() / .sort(f) → array", doc: "就地排序并返回自身；比较器 f(a,b)→number，负数在前", ret: t_self },
    MethodRet { name: "reverse", sig: ".reverse() → array", doc: "就地反转，返回自身", ret: t_self },
    MethodRet { name: "slice", sig: ".slice(start, end?) → array", doc: "子数组（新数组）；负下标从末尾数", ret: t_self },
    MethodRet { name: "map", sig: ".map(f) → array", doc: "f(elem)，返回新数组", ret: t_self },
    MethodRet { name: "filter", sig: ".filter(f) → array", doc: "f(elem) 真值保留，返回新数组", ret: t_self },
    MethodRet { name: "fold", sig: ".fold(init, f) → value", doc: "f(acc, x) 依次折叠；init 在前（Rust 参数顺序）", ret: t_any },
];

const STRING_METHODS: &[MethodRet] = &[
    MethodRet { name: "len", sig: ".len() → number", doc: "字符数（不是字节数）", ret: t_number },
    MethodRet { name: "is_empty", sig: ".is_empty() → bool", doc: "是否为空", ret: t_bool },
    MethodRet { name: "split", sig: ".split(sep) → array", doc: "按分隔符拆为数组；sep 为空串按字符拆", ret: |_| Type::Array(Box::new(Type::Str)) },
    MethodRet { name: "contains", sig: ".contains(s) → bool", doc: "是否包含子串", ret: t_bool },
    MethodRet { name: "index_of", sig: ".index_of(s) → number", doc: "首次出现的字符下标；未找到 -1", ret: t_number },
    MethodRet { name: "trim", sig: ".trim() → string", doc: "去两端空白", ret: t_str },
    MethodRet { name: "starts_with", sig: ".starts_with(s) → bool", doc: "前缀", ret: t_bool },
    MethodRet { name: "ends_with", sig: ".ends_with(s) → bool", doc: "后缀", ret: t_bool },
    MethodRet { name: "to_uppercase", sig: ".to_uppercase() → string", doc: "转大写", ret: t_str },
    MethodRet { name: "to_lowercase", sig: ".to_lowercase() → string", doc: "转小写", ret: t_str },
    MethodRet { name: "sub", sig: ".sub(start, end?) → string", doc: "子串；负下标从末尾数", ret: t_str },
    MethodRet { name: "replace", sig: ".replace(old, new) → string", doc: "替换全部匹配", ret: t_str },
    MethodRet { name: "chars", sig: ".chars() → array", doc: "拆成单字符数组", ret: |_| Type::Array(Box::new(Type::Str)) },
];

const MAP_METHODS: &[MethodRet] = &[
    MethodRet { name: "get", sig: ".get(k) → value", doc: "取值；缺失返回 null", ret: |t| map_value_or_null(t) },
    MethodRet { name: "insert", sig: ".insert(k, v) → map", doc: "写入；返回自身可链式", ret: t_self },
    MethodRet { name: "contains_key", sig: ".contains_key(k) → bool", doc: "是否有键", ret: t_bool },
    MethodRet { name: "remove", sig: ".remove(k) → bool", doc: "删除；返回是否删除", ret: t_bool },
    MethodRet { name: "len", sig: ".len() → number", doc: "键数", ret: t_number },
    MethodRet { name: "is_empty", sig: ".is_empty() → bool", doc: "是否为空", ret: t_bool },
    MethodRet { name: "keys", sig: ".keys() → array", doc: "键数组（保序）", ret: |t| Type::Array(Box::new(map_key_of(t))) },
    MethodRet { name: "values", sig: ".values() → array", doc: "值数组（保序）", ret: |t| Type::Array(Box::new(map_value_of(t))) },
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
    MethodRet { name: "push", sig: ".push(x)", doc: "入堆（仅数字）", ret: t_null },
    MethodRet { name: "pop", sig: ".pop() → number", doc: "移除并返回堆顶；空堆返回 EMPTY", ret: |_| normalize_union(vec![Type::Number, Type::Empty]) },
    MethodRet { name: "peek", sig: ".peek() → number", doc: "堆顶（不移除）；空堆返回 EMPTY", ret: |_| normalize_union(vec![Type::Number, Type::Empty]) },
    MethodRet { name: "len", sig: ".len() → number", doc: "元素数", ret: t_number },
    MethodRet { name: "is_empty", sig: ".is_empty() → bool", doc: "是否为空", ret: t_bool },
];

const STACK_METHODS: &[MethodRet] = &[
    MethodRet { name: "push", sig: ".push(x)", doc: "入栈（顶）", ret: t_null },
    MethodRet { name: "pop", sig: ".pop() → value", doc: "移除并返回栈顶；空栈返回 EMPTY", ret: |_| normalize_union(vec![Type::Any, Type::Empty]) },
    MethodRet { name: "peek", sig: ".peek() → value", doc: "栈顶（不移除）；空栈返回 EMPTY", ret: |_| normalize_union(vec![Type::Any, Type::Empty]) },
    MethodRet { name: "len", sig: ".len() → number", doc: "元素数", ret: t_number },
    MethodRet { name: "is_empty", sig: ".is_empty() → bool", doc: "是否为空", ret: t_bool },
];

const QUEUE_METHODS: &[MethodRet] = &[
    MethodRet { name: "push_back", sig: ".push_back(x)", doc: "入队（队尾）", ret: t_null },
    MethodRet { name: "pop_front", sig: ".pop_front() → value", doc: "出队（队头）；空队列返回 EMPTY", ret: |_| normalize_union(vec![Type::Any, Type::Empty]) },
    MethodRet { name: "front", sig: ".front() → value", doc: "队头（不移除）；空队列返回 EMPTY", ret: |_| normalize_union(vec![Type::Any, Type::Empty]) },
    MethodRet { name: "back", sig: ".back() → value", doc: "队尾（不移除）；空队列返回 EMPTY", ret: |_| normalize_union(vec![Type::Any, Type::Empty]) },
    MethodRet { name: "len", sig: ".len() → number", doc: "元素数", ret: t_number },
    MethodRet { name: "is_empty", sig: ".is_empty() → bool", doc: "是否为空", ret: t_bool },
];

pub const GLOBAL_FNS: &[GlobalFnInfo] = &[
    GlobalFnInfo { name: "print", sig: "print(...args)", doc: "输出，不换行；多参数以空格分隔", ret: g_null, arity: None },
    GlobalFnInfo { name: "println", sig: "println(...args)", doc: "输出并换行；多参数以空格分隔", ret: g_null, arity: None },
    GlobalFnInfo { name: "input", sig: "input() → string | null", doc: "读入一行（去行尾换行）；EOF 返回 null", ret: g_str_or_null, arity: Some(0) },
    GlobalFnInfo { name: "num", sig: "num(x) → number", doc: "字符串/数字转数字；非法字符串报错", ret: g_number, arity: Some(1) },
    GlobalFnInfo { name: "str", sig: "str(x) → string", doc: "任意值转字符串", ret: g_str, arity: Some(1) },
    GlobalFnInfo { name: "len", sig: "len(x) → number", doc: "长度：数组 / 字符串 / 对象 / Map / 堆 / 栈 / 队列", ret: g_number, arity: Some(1) },
    GlobalFnInfo { name: "type", sig: "type(x) → string", doc: "类型名：number/string/bool/null/array/object/map/maxheap/minheap/function", ret: g_str, arity: Some(1) },
    GlobalFnInfo { name: "has", sig: "has(obj, \"field\") → bool", doc: "对象是否有某字段（Map 请用 .contains_key() 方法）", ret: g_bool, arity: Some(2) },
    GlobalFnInfo { name: "floor", sig: "floor(x) → number", doc: "向下取整", ret: g_number, arity: Some(1) },
    GlobalFnInfo { name: "ceil", sig: "ceil(x) → number", doc: "向上取整", ret: g_number, arity: Some(1) },
    GlobalFnInfo { name: "round", sig: "round(x) → number", doc: "四舍五入（half away from zero）", ret: g_number, arity: Some(1) },
    GlobalFnInfo { name: "abs", sig: "abs(x) → number", doc: "绝对值", ret: g_number, arity: Some(1) },
    GlobalFnInfo { name: "sqrt", sig: "sqrt(x) → number", doc: "平方根", ret: g_number, arity: Some(1) },
    GlobalFnInfo { name: "pow", sig: "pow(a, b) → number", doc: "幂", ret: g_number, arity: Some(2) },
    GlobalFnInfo { name: "min", sig: "min(...) 或 min(arr) → number", doc: "最小值", ret: g_number, arity: None },
    GlobalFnInfo { name: "max", sig: "max(...) 或 max(arr) → number", doc: "最大值", ret: g_number, arity: None },
];

pub const FS_FNS: &[NsFnInfo] = &[
    NsFnInfo { name: "read_file", sig: "fs.read_file(path) → string", doc: "整文件读入；不存在/非 UTF-8 报错", ret: g_str },
    NsFnInfo { name: "read_lines", sig: "fs.read_lines(path) → array", doc: "按行读入（去行尾换行）", ret: g_str_array },
    NsFnInfo { name: "write_file", sig: "fs.write_file(path, content)", doc: "覆盖写；父目录必须存在", ret: g_null },
    NsFnInfo { name: "append_file", sig: "fs.append_file(path, content)", doc: "追加写；不存在则创建", ret: g_null },
    NsFnInfo { name: "exists", sig: "fs.exists(path) → bool", doc: "文件/目录存在性", ret: g_bool },
    NsFnInfo { name: "list_dir", sig: "fs.list_dir(path) → array", doc: "目录条目名（排序）", ret: g_str_array },
];

pub const SYS_FNS: &[NsFnInfo] = &[
    NsFnInfo { name: "shell", sig: "sys.shell(cmd) → {status, stdout, stderr}", doc: "sh -c 执行；非零退出不报错；输出原样", ret: g_shell_obj },
    NsFnInfo { name: "get_env", sig: "sys.get_env(name) → string | null", doc: "环境变量；未设置返回 null", ret: g_str_or_null },
    NsFnInfo { name: "args", sig: "sys.args() → array", doc: "脚本命令行参数", ret: g_str_array },
];

/// 按接收者类型给出内置方法表；未知类型返回 None
pub fn methods_for(recv: &Type) -> Option<&'static [MethodRet]> {
    table_for(recv)
}

/// 全部内置方法（编辑器兜底池）
pub fn all_methods() -> Vec<&'static MethodRet> {
    ARRAY_METHODS
        .iter()
        .chain(STRING_METHODS)
        .chain(MAP_METHODS)
        .chain(HEAP_METHODS)
        .chain(STACK_METHODS)
        .chain(QUEUE_METHODS)
        .collect()
}

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

/// 全局函数条目（名字/签名/文档/返回类型/实参个数）
pub fn global_fn_info(name: &str) -> Option<&'static GlobalFnInfo> {
    GLOBAL_FNS.iter().find(|f| f.name == name)
}

/// 全局函数返回类型（println/len/num/...）
pub fn global_fn_return(name: &str) -> Option<Type> {
    global_fn_info(name).map(|f| (f.ret)())
}

/// 命名空间函数返回类型：fs.* / sys.*
pub fn namespace_fn_return(ns: &str, name: &str) -> Option<Type> {
    let table: &[NsFnInfo] = match ns {
        "fs" => FS_FNS,
        "sys" => SYS_FNS,
        _ => return None,
    };
    table.iter().find(|f| f.name == name).map(|f| (f.ret)())
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
    global_fn_info(name).map(|f| f.arity.is_none()).unwrap_or(false)
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

    /// 表数据完整性：名字/签名/文档齐全，arity 与可变参一致
    #[test]
    fn builtin_entries_are_complete() {
        for m in ARRAY_METHODS
            .iter()
            .chain(STRING_METHODS)
            .chain(MAP_METHODS)
            .chain(HEAP_METHODS)
            .chain(STACK_METHODS)
            .chain(QUEUE_METHODS)
        {
            assert!(!m.name.is_empty());
            assert!(m.sig.starts_with('.'), "方法签名应以 . 开头：{}", m.sig);
            assert!(!m.doc.is_empty(), "方法缺文档：{}", m.name);
        }
        for f in GLOBAL_FNS {
            assert!(!f.name.is_empty());
            assert!(!f.sig.is_empty());
            assert!(!f.doc.is_empty());
            assert_eq!(is_variadic_global(f.name), f.arity.is_none());
        }
        for f in FS_FNS.iter().chain(SYS_FNS) {
            assert!(!f.sig.is_empty());
            assert!(!f.doc.is_empty());
        }
    }
}
