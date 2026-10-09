//! 内置符号表 —— 补全与悬停的单一事实源。
//!
//! 数据迁自 editors/vscode/extension.js 的静态表，并按**接收者类型**分组、
//! 为每个函数/方法补上返回类型（[`Ty`]），供类型推导使用。
//! 修改内置库（src/natives）时应同步维护这里。

use super::infer::Ty;

/// 全局函数：`num` / `len` / `print` ……
pub struct FnSig {
    pub name: &'static str,
    pub sig: &'static str,
    pub doc: &'static str,
    pub ret: Ty,
}

/// 方法：带接收者类型约束与返回类型
pub struct MethodSig {
    pub name: &'static str,
    pub sig: &'static str,
    pub doc: &'static str,
    pub ret: Ty,
}

pub const KEYWORDS: &[&str] = &[
    "if", "else", "while", "for", "in", "break", "continue", "return",
    "function", "new", "true", "false", "null", "self",
    "struct", "impl", "interface", "is",
];

pub const GLOBALS: &[FnSig] = &[
    FnSig { name: "print", sig: "print(...args)", doc: "输出，不换行；多参数以空格分隔", ret: Ty::Null },
    FnSig { name: "println", sig: "println(...args)", doc: "输出并换行；多参数以空格分隔", ret: Ty::Null },
    FnSig { name: "input", sig: "input() → string | null", doc: "读入一行（去行尾换行）；EOF 返回 null", ret: Ty::Str },
    FnSig { name: "num", sig: "num(x) → number", doc: "字符串/数字转数字；非法字符串报错", ret: Ty::Number },
    FnSig { name: "str", sig: "str(x) → string", doc: "任意值转字符串", ret: Ty::Str },
    FnSig { name: "len", sig: "len(x) → number", doc: "长度：数组 / 字符串 / 对象 / Map / 堆 / 栈 / 队列", ret: Ty::Number },
    FnSig { name: "type", sig: "type(x) → string", doc: "类型名：number/string/bool/null/array/object/map/maxheap/minheap/function", ret: Ty::Str },
    FnSig { name: "has", sig: "has(obj, \"field\") → bool", doc: "对象是否有某字段（Map 请用 .contains_key() 方法）", ret: Ty::Bool },
    FnSig { name: "floor", sig: "floor(x) → number", doc: "向下取整", ret: Ty::Number },
    FnSig { name: "ceil", sig: "ceil(x) → number", doc: "向上取整", ret: Ty::Number },
    FnSig { name: "round", sig: "round(x) → number", doc: "四舍五入（half away from zero）", ret: Ty::Number },
    FnSig { name: "abs", sig: "abs(x) → number", doc: "绝对值", ret: Ty::Number },
    FnSig { name: "sqrt", sig: "sqrt(x) → number", doc: "平方根", ret: Ty::Number },
    FnSig { name: "pow", sig: "pow(a, b) → number", doc: "幂", ret: Ty::Number },
    FnSig { name: "min", sig: "min(...) 或 min(arr) → number", doc: "最小值", ret: Ty::Number },
    FnSig { name: "max", sig: "max(...) 或 max(arr) → number", doc: "最大值", ret: Ty::Number },
];

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

const ARRAY_METHODS: &[MethodSig] = &[
    MethodSig { name: "push", sig: ".push(x, ...) → number", doc: "追加所有参数，返回新长度", ret: Ty::Number },
    MethodSig { name: "pop", sig: ".pop() → value", doc: "移除并返回末尾元素；空数组返回 null", ret: Ty::Unknown },
    MethodSig { name: "len", sig: ".len() → number", doc: "元素数", ret: Ty::Number },
    MethodSig { name: "is_empty", sig: ".is_empty() → bool", doc: "是否为空", ret: Ty::Bool },
    MethodSig { name: "contains", sig: ".contains(x) → bool", doc: "是否包含（按语言 == 语义）", ret: Ty::Bool },
    MethodSig { name: "index_of", sig: ".index_of(x) → number", doc: "首次出现的下标；未找到 -1", ret: Ty::Number },
    MethodSig { name: "join", sig: ".join(sep?) → string", doc: "连接为字符串；默认 \",\"", ret: Ty::Str },
    MethodSig { name: "sort", sig: ".sort() / .sort(f) → array", doc: "就地排序并返回自身；比较器 f(a,b)→number，负数在前", ret: Ty::Array },
    MethodSig { name: "reverse", sig: ".reverse() → array", doc: "就地反转，返回自身", ret: Ty::Array },
    MethodSig { name: "slice", sig: ".slice(start, end?) → array", doc: "子数组（新数组）；负下标从末尾数", ret: Ty::Array },
    MethodSig { name: "map", sig: ".map(f) → array", doc: "f(elem)，返回新数组", ret: Ty::Array },
    MethodSig { name: "filter", sig: ".filter(f) → array", doc: "f(elem) 真值保留，返回新数组", ret: Ty::Array },
    MethodSig { name: "fold", sig: ".fold(init, f) → value", doc: "f(acc, x) 依次折叠；init 在前（Rust 参数顺序）", ret: Ty::Unknown },
];

const STRING_METHODS: &[MethodSig] = &[
    MethodSig { name: "len", sig: ".len() → number", doc: "字符数（不是字节数）", ret: Ty::Number },
    MethodSig { name: "is_empty", sig: ".is_empty() → bool", doc: "是否为空", ret: Ty::Bool },
    MethodSig { name: "split", sig: ".split(sep) → array", doc: "按分隔符拆为数组；sep 为空串按字符拆", ret: Ty::Array },
    MethodSig { name: "contains", sig: ".contains(s) → bool", doc: "是否包含子串", ret: Ty::Bool },
    MethodSig { name: "index_of", sig: ".index_of(s) → number", doc: "首次出现的字符下标；未找到 -1", ret: Ty::Number },
    MethodSig { name: "trim", sig: ".trim() → string", doc: "去两端空白", ret: Ty::Str },
    MethodSig { name: "starts_with", sig: ".starts_with(s) → bool", doc: "前缀", ret: Ty::Bool },
    MethodSig { name: "ends_with", sig: ".ends_with(s) → bool", doc: "后缀", ret: Ty::Bool },
    MethodSig { name: "to_uppercase", sig: ".to_uppercase() → string", doc: "转大写", ret: Ty::Str },
    MethodSig { name: "to_lowercase", sig: ".to_lowercase() → string", doc: "转小写", ret: Ty::Str },
    MethodSig { name: "sub", sig: ".sub(start, end?) → string", doc: "子串；负下标从末尾数", ret: Ty::Str },
    MethodSig { name: "replace", sig: ".replace(old, new) → string", doc: "替换全部匹配", ret: Ty::Str },
    MethodSig { name: "chars", sig: ".chars() → array", doc: "拆成单字符数组", ret: Ty::Array },
];

const MAP_METHODS: &[MethodSig] = &[
    MethodSig { name: "get", sig: ".get(k) → value", doc: "取值；缺失返回 null", ret: Ty::Unknown },
    MethodSig { name: "insert", sig: ".insert(k, v) → map", doc: "写入；返回自身可链式", ret: Ty::Map },
    MethodSig { name: "contains_key", sig: ".contains_key(k) → bool", doc: "是否有键", ret: Ty::Bool },
    MethodSig { name: "remove", sig: ".remove(k) → bool", doc: "删除；返回是否删除", ret: Ty::Bool },
    MethodSig { name: "len", sig: ".len() → number", doc: "键数", ret: Ty::Number },
    MethodSig { name: "is_empty", sig: ".is_empty() → bool", doc: "是否为空", ret: Ty::Bool },
    MethodSig { name: "keys", sig: ".keys() → array", doc: "键数组（保序）", ret: Ty::Array },
    MethodSig { name: "values", sig: ".values() → array", doc: "值数组（保序）", ret: Ty::Array },
];

const HEAP_METHODS: &[MethodSig] = &[
    MethodSig { name: "push", sig: ".push(x)", doc: "入堆（仅数字）", ret: Ty::Null },
    MethodSig { name: "pop", sig: ".pop() → number", doc: "移除并返回堆顶；空堆返回 EMPTY", ret: Ty::Number },
    MethodSig { name: "peek", sig: ".peek() → number", doc: "堆顶（不移除）；空堆返回 EMPTY", ret: Ty::Number },
    MethodSig { name: "len", sig: ".len() → number", doc: "元素数", ret: Ty::Number },
    MethodSig { name: "is_empty", sig: ".is_empty() → bool", doc: "是否为空", ret: Ty::Bool },
];

const STACK_METHODS: &[MethodSig] = &[
    MethodSig { name: "push", sig: ".push(x)", doc: "入栈（顶）", ret: Ty::Null },
    MethodSig { name: "pop", sig: ".pop() → value", doc: "移除并返回栈顶；空栈返回 EMPTY", ret: Ty::Unknown },
    MethodSig { name: "peek", sig: ".peek() → value", doc: "栈顶（不移除）；空栈返回 EMPTY", ret: Ty::Unknown },
    MethodSig { name: "len", sig: ".len() → number", doc: "元素数", ret: Ty::Number },
    MethodSig { name: "is_empty", sig: ".is_empty() → bool", doc: "是否为空", ret: Ty::Bool },
];

const QUEUE_METHODS: &[MethodSig] = &[
    MethodSig { name: "push_back", sig: ".push_back(x)", doc: "入队（队尾）", ret: Ty::Null },
    MethodSig { name: "pop_front", sig: ".pop_front() → value", doc: "出队（队头）；空队列返回 EMPTY", ret: Ty::Unknown },
    MethodSig { name: "front", sig: ".front() → value", doc: "队头（不移除）；空队列返回 EMPTY", ret: Ty::Unknown },
    MethodSig { name: "back", sig: ".back() → value", doc: "队尾（不移除）；空队列返回 EMPTY", ret: Ty::Unknown },
    MethodSig { name: "len", sig: ".len() → number", doc: "元素数", ret: Ty::Number },
    MethodSig { name: "is_empty", sig: ".is_empty() → bool", doc: "是否为空", ret: Ty::Bool },
];

pub const FS_FNS: &[FnSig] = &[
    FnSig { name: "read_file", sig: "fs.read_file(path) → string", doc: "整文件读入；不存在/非 UTF-8 报错", ret: Ty::Str },
    FnSig { name: "read_lines", sig: "fs.read_lines(path) → array", doc: "按行读入（去行尾换行）", ret: Ty::Array },
    FnSig { name: "write_file", sig: "fs.write_file(path, content)", doc: "覆盖写；父目录必须存在", ret: Ty::Null },
    FnSig { name: "append_file", sig: "fs.append_file(path, content)", doc: "追加写；不存在则创建", ret: Ty::Null },
    FnSig { name: "exists", sig: "fs.exists(path) → bool", doc: "文件/目录存在性", ret: Ty::Bool },
    FnSig { name: "list_dir", sig: "fs.list_dir(path) → array", doc: "目录条目名（排序）", ret: Ty::Array },
];

pub const SYS_FNS: &[FnSig] = &[
    FnSig { name: "shell", sig: "sys.shell(cmd) → {status, stdout, stderr}", doc: "sh -c 执行；非零退出不报错；输出原样", ret: Ty::Unknown },
    FnSig { name: "get_env", sig: "sys.get_env(name) → string | null", doc: "环境变量；未设置返回 null", ret: Ty::Str },
    FnSig { name: "args", sig: "sys.args() → array", doc: "脚本命令行参数", ret: Ty::Array },
];

/// 兜底方法池：接收者类型未知时给出全部方法（与旧插件行为一致）。
pub fn all_methods() -> Vec<&'static MethodSig> {
    ARRAY_METHODS
        .iter()
        .chain(STRING_METHODS)
        .chain(MAP_METHODS)
        .chain(HEAP_METHODS)
        .chain(STACK_METHODS)
        .chain(QUEUE_METHODS)
        .collect()
}

/// 按接收者类型给出方法表；未知类型返回空（由调用方决定兜底）。
pub fn methods_for(ty: &Ty) -> Option<&'static [MethodSig]> {
    match ty {
        Ty::Array => Some(ARRAY_METHODS),
        Ty::Str => Some(STRING_METHODS),
        Ty::Map => Some(MAP_METHODS),
        Ty::MaxHeap | Ty::MinHeap => Some(HEAP_METHODS),
        Ty::Stack => Some(STACK_METHODS),
        Ty::Queue => Some(QUEUE_METHODS),
        _ => None,
    }
}

/// 方法返回类型查表（内置类型）
pub fn method_return(ty: &Ty, name: &str) -> Option<Ty> {
    methods_for(ty)?.iter().find(|m| m.name == name).map(|m| m.ret.clone())
}

pub fn global_fn(name: &str) -> Option<&'static FnSig> {
    GLOBALS.iter().find(|f| f.name == name)
}

pub fn namespace_fns(ns: &str) -> Option<&'static [FnSig]> {
    match ns {
        "fs" => Some(FS_FNS),
        "sys" => Some(SYS_FNS),
        _ => None,
    }
}

/// 内置类名（`new X()` 的 X）
pub fn is_builtin_class(name: &str) -> bool {
    CLASSES.iter().any(|(n, _, _)| *n == name)
}

/// `new Builtin()` 的实例类型
pub fn class_instance(name: &str) -> Option<Ty> {
    let ty = match name {
        "Map" => Ty::Map,
        "MaxHeap" => Ty::MaxHeap,
        "MinHeap" => Ty::MinHeap,
        "Stack" => Ty::Stack,
        "Queue" => Ty::Queue,
        _ => return None,
    };
    Some(ty)
}
