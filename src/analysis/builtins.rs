//! 内置符号表 —— 补全与悬停的编辑器查询层。
//!
//! **事实来源纪律**：内置方法 / 全局函数 / fs·sys 命名空间函数 / 内置类的
//! 名字、签名、文档与返回类型都以 [`crate::checker::builtins`] 为单一事实源，
//! 本模块经 [`super::bridge`] 把返回类型换算为编辑器 [`Ty`] 后对外提供查询；
//! 不再维护独立副本（漂移由此不可能发生）。
//! 仍留在本文件的只有纯编辑器展示数据：关键字、类型标注名、常量与类的
//! 展示签名（checker 不消费它们）。
//! 修改内置库（src/natives）时只改 checker::builtins。

use super::bridge::{checker_to_ty, ty_to_checker};
use super::infer::Ty;
use crate::checker::builtins as cb;

/// 全局函数：`num` / `len` / `print` ……（数据来自 checker 事实源）
#[derive(Clone)]
pub struct FnSig {
    pub name: &'static str,
    pub sig: &'static str,
    pub doc: &'static str,
    pub ret: Ty,
}

/// 方法条目（名字/签名/文档；返回类型由 [`method_return`] 按接收者查询）
pub type MethodSig = cb::MethodRet;

pub const KEYWORDS: &[&str] = &[
    "if", "else", "while", "for", "in", "break", "continue", "return",
    "function", "new", "true", "false", "null", "self",
    "struct", "impl", "interface", "is",
];

/// 内置类展示签名（名字集合与 checker::builtins::class_instance 同源，
/// 由 tests/builtins_consistency_test.rs 守护）
pub const CLASSES: &[(&str, &str, &str)] = &[
    ("MaxHeap", "new MaxHeap() / new MaxHeap(arr)", "最大堆（Rust BinaryHeap 原生实现）；push/pop/peek/len/is_empty，空堆 pop/peek 返回 EMPTY"),
    ("MinHeap", "new MinHeap() / new MinHeap(arr)", "最小堆；push/pop/peek/len/is_empty，空堆 pop/peek 返回 EMPTY"),
    ("Stack", "new Stack() / new Stack(arr)", "栈（Vec，LIFO）；push/pop/peek/len/is_empty，空栈 pop/peek 返回 EMPTY；数组顺序 = 底→顶"),
    ("Queue", "new Queue() / new Queue(arr)", "队列（VecDeque，FIFO）；push_back/pop_front/front/back/len/is_empty，空队列取值返回 EMPTY；数组顺序 = 队头→队尾"),
    ("Map", "new Map()", "保序哈希表；键限 number/string/bool/null；get/insert/contains_key/remove/len/keys/values"),
];

pub const CONSTANTS: &[(&str, &str, &str)] = &[
    ("EMPTY", "EMPTY", "哨兵值：空堆/栈/队列 pop()/peek() 的返回"),
    ("inf", "inf", "正无穷（1 / 0）"),
    ("nan", "nan", "非数（0 / 0）"),
    ("fs", "fs.*", "文件命名空间：read_file / read_lines / write_file / append_file / exists / list_dir"),
    ("sys", "sys.*", "系统命名空间：shell / get_env / args"),
];

/// 类型标注可用名（与 infer::ty_from_annotation / Ty::from_type_name 对齐）。
/// 用户 struct / interface 由分析层追加（见 analysis::type_items）。
pub const TYPES: &[(&str, &str, &str)] = &[
    ("number", "number", "数字类型"),
    ("string", "string", "字符串类型"),
    ("bool", "bool", "布尔类型（也接受 boolean 写法）"),
    ("array", "array", "数组类型；元素类型可加 `[]` 后缀标注，如 number[]"),
    ("map", "map", "映射类型"),
    ("object", "object", "对象字面量类型"),
    ("function", "function", "函数类型"),
    ("any", "any", "任意类型（标注缺省值）"),
    ("Array", "Array", "array 的别名写法"),
    ("Map", "Map<K, V>", "保序哈希表类型，如 Map<string, number>"),
];

/// 全部内置全局函数（补全展示用）
pub fn all_globals() -> Vec<FnSig> {
    cb::GLOBAL_FNS
        .iter()
        .map(|f| FnSig {
            name: f.name,
            sig: f.sig,
            doc: f.doc,
            ret: checker_to_ty(&(f.ret)()),
        })
        .collect()
}

/// 兜底方法池：接收者类型未知时给出全部方法（与旧插件行为一致）。
pub fn all_methods() -> Vec<&'static MethodSig> {
    cb::all_methods()
}

/// 按接收者类型给出方法表；未知类型返回 None（由调用方决定兜底）。
pub fn methods_for(ty: &Ty) -> Option<&'static [MethodSig]> {
    cb::methods_for(&ty_to_checker(ty))
}

/// 方法返回类型查表（内置类型）
pub fn method_return(ty: &Ty, name: &str) -> Option<Ty> {
    cb::method_return(&ty_to_checker(ty), name).map(|t| checker_to_ty(&t))
}

pub fn global_fn(name: &str) -> Option<FnSig> {
    cb::global_fn_info(name).map(|f| FnSig {
        name: f.name,
        sig: f.sig,
        doc: f.doc,
        ret: checker_to_ty(&(f.ret)()),
    })
}

pub fn namespace_fns(ns: &str) -> Option<Vec<FnSig>> {
    let table = match ns {
        "fs" => cb::FS_FNS,
        "sys" => cb::SYS_FNS,
        _ => return None,
    };
    Some(
        table
            .iter()
            .map(|f| FnSig {
                name: f.name,
                sig: f.sig,
                doc: f.doc,
                ret: checker_to_ty(&(f.ret)()),
            })
            .collect(),
    )
}

/// 内置类名（`new X()` 的 X）
pub fn is_builtin_class(name: &str) -> bool {
    cb::is_builtin_class(name)
}

/// `new Builtin()` 的实例类型
pub fn class_instance(name: &str) -> Option<Ty> {
    cb::class_instance(name).map(|t| checker_to_ty(&t))
}
