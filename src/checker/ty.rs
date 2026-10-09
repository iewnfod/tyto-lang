//! 渐进类型系统的语义类型（检查器内部表示）。
//!
//! 与 TypeAst（语法）的分工：TypeAst 保留源码书写形态；
//! Type 是检查用的规范化类型——联合去重排序、泛型参数代入、Any 擦除边界。
//! 未标注/推不出 → [`Type::Any`]（渐进承诺：动态代码零误报）。

use std::rc::Rc;

use crate::ast::TypeAst;

use super::registry::StructRegistry;

#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    /// 渐变迁换站：与任何类型互相兼容（未标注代码的兜底）
    Any,
    Number,
    Str,
    Bool,
    Null,
    Empty,
    /// 数组（元素类型；未细化时为 Any）
    Array(Box<Type>),
    /// Map<K, V>
    Map(Box<Type>, Box<Type>),
    MaxHeap,
    MinHeap,
    Stack,
    Queue,
    /// struct 实例：名义名 + 泛型实参（`Box<number>`）
    Struct(String, Vec<Type>),
    /// struct 定义本身（`struct P {...}` 绑定值）
    StructDef(String),
    /// interface 定义本身
    InterfaceDef(String),
    /// 结构化对象：已知字段
    Object(Rc<[(String, Type)]>),
    /// 函数值：参数（None = 该参数未标注）+ 返回
    Func(Rc<FuncShape>),
    /// 联合：成员已规范化（去重、非空、不嵌套联合）
    Union(Rc<[Type]>),
    /// 泛型形参（检查泛型函数体时出现；对赋值兼容按 Any 处理）
    TypeVar(String),
    /// 命名空间 fs / sys
    Namespace(&'static str),
}

#[derive(Debug, Clone, PartialEq)]
pub struct FuncShape {
    pub params: Vec<Option<Type>>,
    pub ret: Option<Type>,
}

impl Type {
    /// 展示名（诊断消息用）
    pub fn display(&self) -> String {
        match self {
            Type::Any => "any".into(),
            Type::Number => "number".into(),
            Type::Str => "string".into(),
            Type::Bool => "bool".into(),
            Type::Null => "null".into(),
            Type::Empty => "empty".into(),
            Type::Array(t) => {
                if matches!(**t, Type::Union(..) | Type::Func(..)) {
                    format!("({})[]", t.display())
                } else {
                    format!("{}[]", t.display())
                }
            }
            Type::Map(k, v) => format!("Map<{}, {}>", k.display(), v.display()),
            Type::MaxHeap => "maxheap".into(),
            Type::MinHeap => "minheap".into(),
            Type::Stack => "stack".into(),
            Type::Queue => "queue".into(),
            Type::Struct(n, args) => {
                if args.is_empty() {
                    n.clone()
                } else {
                    format!(
                        "{}<{}>",
                        n,
                        args.iter().map(|a| a.display()).collect::<Vec<_>>().join(", ")
                    )
                }
            }
            Type::StructDef(n) | Type::InterfaceDef(n) => n.clone(),
            Type::Object(fs) => format!(
                "{{{}}}",
                fs.iter().map(|(n, t)| format!("{}: {}", n, t.display())).collect::<Vec<_>>().join(", ")
            ),
            Type::Func(f) => {
                let ps = f
                    .params
                    .iter()
                    .map(|p| p.as_ref().map(|t| t.display()).unwrap_or_else(|| "any".into()))
                    .collect::<Vec<_>>()
                    .join(", ");
                match &f.ret {
                    Some(r) => format!("({}) -> {}", ps, r.display()),
                    None => format!("({})", ps),
                }
            }
            Type::Union(ms) => {
                ms.iter().map(|m| m.display()).collect::<Vec<_>>().join(" | ")
            }
            Type::TypeVar(n) => n.clone(),
            Type::Namespace(n) => (*n).into(),
        }
    }

    pub fn is_any(&self) -> bool {
        matches!(self, Type::Any)
    }
}

/// 算术操作数的投影类别（决定 `+ - * / %` 的合法性与结果）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArithKind {
    Num,
    Str,
    Arr,
    /// any / 类型参数 / 含 any 的联合：放行但结果未知
    AnyKind,
}

/// 操作数 → 算术类别；None = 不能参与算术（null / bool / map / object / func…）。
///
/// 语义约定：
/// - **any 吸收一切**：联合中含 any 即按 any 处理（渐进承诺——`any | null` 常来自
///   未细化 Map 的 `get()`，空值由 `contains_key` 等控制流保证，编辑器不掺和）；
/// - **null 严格**：具体类型与 null 的联合（`number | null`）算术直接拒绝，
///   需 `??` 收窄（strictNullChecks 体验）；
/// - **empty 哨兵放行**：堆/栈 `pop()/peek()` 的 `number | empty` 按数字习惯放行
///   （median 流等既有惯用形依赖算法不变量，检查器是建议性的）。
fn project(t: &Type) -> Option<ArithKind> {
    match t {
        Type::Any | Type::TypeVar(_) => Some(ArithKind::AnyKind),
        Type::Number | Type::Empty => Some(ArithKind::Num),
        Type::Str => Some(ArithKind::Str),
        Type::Array(_) => Some(ArithKind::Arr),
        Type::Union(ms) => {
            // 先看是否有 any 成员（吸收一切，null 也被其吸收）
            if ms.iter().any(|m| matches!(m, Type::Any | Type::TypeVar(_))) {
                return Some(ArithKind::AnyKind);
            }            let mut kinds: Vec<ArithKind> = Vec::new();
            for m in ms.iter() {
                let k = project(m)?;
                if !kinds.contains(&k) {
                    kinds.push(k);
                }
            }
            match kinds.len() {
                0 => Some(ArithKind::AnyKind),
                1 => Some(kinds[0]),
                _ => Some(ArithKind::AnyKind), // 异构联合：放行，结果未知
            }
        }
        _ => None,
    }
}

/// 算术（+ - * / %）操作数检查：返回 Ok(结果类型) 或 Err(不合法的操作数类型)
pub fn arith_result(op: &str, l: &Type, r: &Type) -> Result<Type, ()> {
    let lp = project(l).ok_or(())?;
    let rp = project(r).ok_or(())?;
    if op == "+" {
        // 语言语义：string + 任意 → string；array + array → array；number + number → number
        match (lp, rp) {
            (ArithKind::AnyKind, _) | (_, ArithKind::AnyKind) => Ok(Type::Any),
            (ArithKind::Str, _) | (_, ArithKind::Str) => Ok(Type::Str),
            (ArithKind::Arr, ArithKind::Arr) => match (l, r) {
                (Type::Array(a), Type::Array(b)) if a == b => Ok(Type::Array(a.clone())),
                _ => Ok(Type::Array(Box::new(Type::Any))),
            },
            (ArithKind::Num, ArithKind::Num) => Ok(Type::Number),
            // 数字 + 数组：运行时必然报错（cannot add）
            (ArithKind::Num, ArithKind::Arr) | (ArithKind::Arr, ArithKind::Num) => Err(()),
        }
    } else {
        match (lp, rp) {
            (ArithKind::AnyKind, _) | (_, ArithKind::AnyKind) | (ArithKind::Num, ArithKind::Num) => {
                Ok(Type::Number)
            }
            // - * / % 只接受数字（any 放行）；字符串/数组参与即运行时错误
            (ArithKind::Str, _) | (_, ArithKind::Str) | (ArithKind::Arr, _) | (_, ArithKind::Arr) => {
                Err(())
            }
        }
    }
}

/// TypeAst（语法）→ Type（语义）。
/// `generics`：当前作用域可见的泛型形参名（`T`），命中 → TypeVar。
/// 未识别的标识符：struct 注册表 → 其实例；否则 Any（宽松，不误报）。
pub fn from_ast(ast: &TypeAst, generics: &[String], structs: &StructRegistry) -> Type {
    match ast {
        TypeAst::Named(n, _) => named(n, generics, structs),
        TypeAst::Generic(n, args, _) => {
            let conv_args: Vec<Type> =
                args.iter().map(|a| from_ast(a, generics, structs)).collect();
            match n.as_str() {
                "Array" => Type::Array(Box::new(conv_args.first().cloned().unwrap_or(Type::Any))),
                "Map" => Type::Map(
                    Box::new(conv_args.first().cloned().unwrap_or(Type::Any)),
                    Box::new(conv_args.get(1).cloned().unwrap_or(Type::Any)),
                ),
                _ => {
                    if structs.contains(n) {
                        Type::Struct(n.clone(), conv_args)
                    } else {
                        Type::Any
                    }
                }
            }
        }
        TypeAst::Array(elem, _) => Type::Array(Box::new(from_ast(elem, generics, structs))),
        TypeAst::Union(ms, _) => normalize_union(
            ms.iter().map(|m| from_ast(m, generics, structs)).collect(),
        ),
        TypeAst::Object(fields, _) => Type::Object(Rc::from(
            fields
                .iter()
                .map(|(n, t)| (n.clone(), from_ast(t, generics, structs)))
                .collect::<Vec<_>>(),
        )),
        TypeAst::Func { params, ret, .. } => Type::Func(Rc::new(FuncShape {
            params: params.iter().map(|(_, t)| Some(from_ast(t, generics, structs))).collect(),
            ret: ret.as_ref().map(|r| Box::from(from_ast(r, generics, structs))).map(|b| *b),
        })),
    }
}

fn named(n: &str, generics: &[String], structs: &StructRegistry) -> Type {
    if generics.iter().any(|g| g == n) {
        return Type::TypeVar(n.to_string());
    }
    match n {
        "any" | "unknown" => Type::Any,
        "number" => Type::Number,
        "string" => Type::Str,
        "bool" | "boolean" => Type::Bool,
        "null" => Type::Null,
        "empty" => Type::Empty,
        "array" | "Array" => Type::Array(Box::new(Type::Any)),
        "object" => Type::Object(Rc::from(Vec::new())),
        "map" | "Map" => Type::Map(Box::new(Type::Any), Box::new(Type::Any)),
        "maxheap" | "MaxHeap" => Type::MaxHeap,
        "minheap" | "MinHeap" => Type::MinHeap,
        "stack" | "Stack" => Type::Stack,
        "queue" | "Queue" => Type::Queue,
        "function" => Type::Func(Rc::new(FuncShape { params: Vec::new(), ret: None })),
        _ => {
            if structs.contains(n) {
                Type::Struct(n.to_string(), Vec::new())
            } else {
                Type::Any
            }
        }
    }
}

/// 联合规范化：展平嵌套、去重（保首现顺序）；单成员直接折叠
pub fn normalize_union(ms: Vec<Type>) -> Type {
    use std::collections::VecDeque;
    let mut flat: Vec<Type> = Vec::new();
    let mut queue: VecDeque<Type> = ms.into_iter().collect();
    while let Some(t) = queue.pop_front() {
        match t {
            Type::Union(inner) => {
                for i in inner.iter().rev() {
                    queue.push_front(i.clone());
                }
            }
            other => {
                if !flat.contains(&other) {
                    flat.push(other);
                }
            }
        }
    }
    match flat.len() {
        0 => Type::Any,
        1 => flat.pop().unwrap(),
        _ => Type::Union(Rc::from(flat)),
    }
}

/// 联合中移除 null（`??` / `?.` 的表达式级收窄）
pub fn without_null(t: &Type) -> Type {
    match t {
        Type::Union(ms) => normalize_union(
            ms.iter().filter(|m| !matches!(m, Type::Null)).cloned().collect(),
        ),
        Type::Null => Type::Any,
        other => other.clone(),
    }
}

/// `src` 能否赋给 `dst`（渐进：Any 双向放行；Unknown 不存在——由 Any 承担）
pub fn assignable(src: &Type, dst: &Type) -> bool {
    match (src, dst) {
        (Type::Any, _) | (_, Type::Any) => true,
        (Type::TypeVar(_), _) | (_, Type::TypeVar(_)) => true,
        (Type::Union(ms), _) => ms.iter().all(|m| assignable(m, dst)),
        (_, Type::Union(ms)) => ms.iter().any(|m| assignable(src, m)),
        (Type::Array(a), Type::Array(b)) => assignable(a, b), // 协变（TS 风格）
        (Type::Map(k1, v1), Type::Map(k2, v2)) => {
            // v1 双参不变（简化）：逆变太难讲清楚，先严格同型 + Any 放行
            assignable(k1, k2) && assignable(v1, v2)
        }
        (Type::Struct(n1, a1), Type::Struct(n2, a2)) => {
            // 名义：同名 + 类型实参逐个兼容
            n1 == n2 && a1.len() == a2.len() && a1.iter().zip(a2.iter()).all(|(x, y)| assignable(x, y))
        }
        (Type::Object(fs1), Type::Object(fs2)) => {
            // 结构化：dst 的每个字段在 src 中存在且类型兼容
            fs2.iter().all(|(n, dt)| {
                fs1.iter()
                    .find(|(n2, _)| n2 == n)
                    .map(|(_, st)| assignable(st, dt))
                    .unwrap_or(false)
            })
        }
        (Type::Object(fs), Type::InterfaceDef(n)) | (Type::InterfaceDef(n), Type::Object(fs)) => {
            let _ = (fs, n);
            true // interface 结构化契约在 v1 检查器中放宽（is 检查在运行时）
        }
        (Type::Func(f1), Type::Func(f2)) => {
            // 参数：双变（TS 方法风格）；返回：协变
            let params_ok = f1.params.len() <= f2.params.len()
                || f1.params.iter().zip(f2.params.iter()).all(|(a, b)| match (a, b) {
                    (Some(x), Some(y)) => assignable(y, x) || assignable(x, y),
                    _ => true,
                });
            let ret_ok = match (&f1.ret, &f2.ret) {
                (Some(a), Some(b)) => assignable(a, b),
                _ => true,
            };
            params_ok && ret_ok
        }
        (a, b) => a == b,
    }
}

// 旧的 arith_result 实现已由上方基于投影的版本取代（ArithKind/project）
