//! 补全条目构造：类型位置 / 成员 / 全局三个来源。

use crate::Span;

use super::builtins;
use super::ty_view::{self, editor_display};
use crate::checker::ty::Type;
use super::scope::{self, Binding, ScopeSnapshot, StructRegistry};
use super::semantics;
use super::tolerate;
use super::{CompleteItem, ItemKind};

// ============ 类型补全 ============

/// 类型标注位置的补全项：内置类型表 + 用户 struct / interface。
/// 注册表优先用 ambient（挖掉光标语句、尽量保留全文），退而全文档解析（截断容错）。
pub(super) fn type_items(src: &str, cursor: Span) -> Vec<CompleteItem> {
    let mut items: Vec<CompleteItem> = builtins::TYPES
        .iter()
        .map(|(name, sig, doc)| CompleteItem {
            label: (*name).into(),
            // Map 是可 new 的容器类，其余按关键字展示（与 TS 内置类型的图标习惯一致）
            kind: if *name == "Map" { ItemKind::Class } else { ItemKind::Keyword },
            detail: (*sig).into(),
            doc: (*doc).into(),
        })
        .collect();

    let program = tolerate::parse_ambient(src, cursor)
        .or_else(|| semantics::parse_full(src).map(|(p, _)| p));
    if let Some(program) = program {
        let structs = scope::collect_registry(&program);
        for s in structs.iter() {
            items.push(CompleteItem {
                label: s.name.clone(),
                kind: if s.interface { ItemKind::Interface } else { ItemKind::Struct },
                detail: s.detail(),
                doc: if s.interface { "接口（作为类型标注）".into() } else { "struct（作为类型标注）".into() },
            });
        }
    }
    items
}

// ============ 成员补全 ============

pub(super) fn member_items(snap: &mut ScopeSnapshot) -> Vec<CompleteItem> {
    let Some(recv) = snap.receiver_ty.clone() else {
        return all_method_items();
    };
    let mut items = items_for_recv(&recv, snap);
    items.dedup_by(|a, b| a.label == b.label);
    items
}

/// 接收者类型 → 成员补全条目。
/// 语义：Any / 泛型参数 → 全量方法池（不可知不收窄）；联合 → 去 null 后
/// 单臂按该臂、多臂取各臂成员的**交集**（每个臂都得有才合法）；带元素/
/// 键值形参的容器与裸容器同表（方法表按类别分发）。
fn items_for_recv(recv: &Type, snap: &mut ScopeSnapshot) -> Vec<CompleteItem> {
    match recv {
        Type::Union(ms) => {
            let arms: Vec<&Type> = ms.iter().filter(|m| !matches!(m, Type::Null)).collect();
            // 任一臂不可知：无法收窄，全池
            if arms.iter().any(|m| matches!(m, Type::Any | Type::TypeVar(_))) {
                return all_method_items();
            }
            match arms.as_slice() {
                [] => Vec::new(),
                [only] => items_for_recv(only, snap),
                _ => {
                    let sets: Vec<std::collections::HashSet<String>> = arms
                        .iter()
                        .map(|a| member_labels(a, &snap.structs))
                        .collect();
                    let mut common = sets[0].clone();
                    for s in &sets[1..] {
                        common.retain(|l| s.contains(l));
                    }
                    // 条目取第一臂（交集 ⊆ 每臂标签集，过滤后即共有成员）
                    items_for_recv(arms[0], snap)
                        .into_iter()
                        .filter(|i| common.contains(&i.label))
                        .collect()
                }
            }
        }
        // 未知 / 泛型参数：全量方法池（与旧行为一致）
        Type::Any | Type::TypeVar(_) => all_method_items(),
        Type::Namespace(ns) => builtins::namespace_fns(ns)
            .map(|fns| {
                fns.iter()
                    .map(|f| CompleteItem {
                        label: f.name.into(),
                        kind: ItemKind::Function,
                        detail: f.sig.into(),
                        doc: f.doc.into(),
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
        Type::Struct(name, _) => struct_member_items(name, snap),
        Type::Object(fields) => fields
            .iter()
            .map(|(name, ty)| {
                let is_fn = matches!(ty, Type::Func(_));
                CompleteItem {
                    label: name.clone(),
                    kind: if is_fn { ItemKind::Method } else { ItemKind::Field },
                    detail: match ty {
                        Type::Func(f) => match &f.ret {
                            Some(r) => format!("{}() -> {}", name, editor_display(r)),
                            None => format!("{}()", name),
                        },
                        t => format!("{}: {}", name, editor_display(t)),
                    },
                    doc: if is_fn { "方法字段（对象字面量）".into() } else { "字段（对象字面量）".into() },
                }
            })
            .collect(),
        // 基本类型没有成员
        Type::Number | Type::Bool | Type::Null | Type::Empty | Type::Func(_)
        | Type::StructDef(_) | Type::InterfaceDef(_) => Vec::new(),
        // 内置容器类型（元素/键值形参不影响方法表分发）
        Type::Array(_) | Type::Str | Type::Map(..) | Type::MaxHeap | Type::MinHeap
        | Type::Stack | Type::Queue => {
            match builtins::methods_for(recv) {
                Some(ms) => ms
                    .iter()
                    .map(|m| CompleteItem {
                        label: m.name.into(),
                        kind: ItemKind::Method,
                        detail: m.sig.into(),
                        doc: m.doc.into(),
                    })
                    .collect(),
                None => Vec::new(),
            }
        }
    }
}

/// 一个接收者类型的成员标签集（联合交集与条目过滤用；owned 避免跨借用纠缠）
fn member_labels(recv: &Type, structs: &StructRegistry) -> std::collections::HashSet<String> {
    match recv {
        Type::Namespace(ns) => builtins::namespace_fns(ns)
            .map(|fns| fns.iter().map(|f| f.name.to_string()).collect())
            .unwrap_or_default(),
        Type::Struct(name, _) => structs
            .get(name)
            .map(|info| {
                info.fields
                    .iter()
                    .map(|f| f.name.clone())
                    .chain(info.methods.iter().map(|m| m.name.clone()))
                    .collect()
            })
            .unwrap_or_default(),
        Type::Object(fields) => fields.iter().map(|(n, _)| n.clone()).collect(),
        t => builtins::methods_for(t)
            .map(|ms| ms.iter().map(|m| m.name.to_string()).collect())
            .unwrap_or_default(),
    }
}

/// struct 实例成员：字段（声明顺序）+ impl 方法。
/// 无标注方法的返回类型由引擎按需从方法体推导（`snap.ck`）。
fn struct_member_items(name: &str, snap: &mut ScopeSnapshot) -> Vec<CompleteItem> {
    let Some(info) = snap.structs.get(name) else {
        return Vec::new();
    };
    let mut items: Vec<CompleteItem> = info
        .fields
        .iter()
        .map(|f| CompleteItem {
            label: f.name.clone(),
            kind: ItemKind::Field,
            detail: match &f.ty {
                Some(t) => format!("{}: {}", f.name, t),
                None => format!("{}: unknown", f.name),
            },
            doc: "字段".into(),
        })
        .collect();
    for m in &info.methods {
        // 返回类型：`-> T` 标注优先，否则引擎从方法体 return 推导
        let ret_str = match &m.ret {
            Some(t) => Some(t.to_string()),
            None => snap.ck.infer_func_ret(&m.params, &m.body).map(|t| editor_display(&t)),
        };
        let detail = match ret_str {
            Some(r) => format!("{} -> {}", ty_view::func_sig(&m.name, &m.params, None), r),
            None => ty_view::func_sig(&m.name, &m.params, None),
        };
        items.push(CompleteItem {
            label: m.name.clone(),
            kind: ItemKind::Method,
            detail,
            doc: format!("方法（impl {}）", name),
        });
    }
    items
}

pub(super) fn all_method_items() -> Vec<CompleteItem> {
    builtins::all_methods()
        .iter()
        .map(|m| CompleteItem {
            label: m.name.into(),
            kind: ItemKind::Method,
            detail: m.sig.into(),
            doc: m.doc.into(),
        })
        .collect()
}

// ============ 全局补全 ============

pub(super) fn global_items(snap: &ScopeSnapshot) -> Vec<CompleteItem> {
    let mut items = global_static_items(&snap.structs);
    // 作用域绑定：内层优先（去重保留更内层的）；哨兵是内部标记，不外泄
    let mut seen = std::collections::HashSet::new();
    for scope in snap.scopes.iter().rev() {
        for b in scope.values() {
            if b.name == scope::SENTINEL {
                continue;
            }
            if seen.insert(b.name.clone()) {
                items.push(binding_item(b));
            }
        }
    }
    items
}

pub(super) fn global_static_items(structs: &StructRegistry) -> Vec<CompleteItem> {
    let mut items: Vec<CompleteItem> = builtins::KEYWORDS
        .iter()
        .map(|k| CompleteItem {
            label: (*k).into(),
            kind: ItemKind::Keyword,
            detail: "keyword".into(),
            doc: String::new(),
        })
        .collect();
    for f in builtins::all_globals() {
        items.push(CompleteItem {
            label: f.name.into(),
            kind: ItemKind::Function,
            detail: f.sig.into(),
            doc: f.doc.into(),
        });
    }
    for (name, sig, doc) in builtins::CLASSES {
        items.push(CompleteItem {
            label: (*name).into(),
            kind: ItemKind::Class,
            detail: (*sig).into(),
            doc: (*doc).into(),
        });
    }
    for (name, sig, doc) in builtins::CONSTANTS {
        items.push(CompleteItem {
            label: (*name).into(),
            kind: ItemKind::Constant,
            detail: (*sig).into(),
            doc: (*doc).into(),
        });
    }
    for s in structs.iter() {
        if !s.interface {
            items.push(CompleteItem {
                label: s.name.clone(),
                kind: ItemKind::Struct,
                detail: s.detail(),
                doc: String::new(),
            });
        }
    }
    for s in structs.iter() {
        if s.interface {
            items.push(CompleteItem {
                label: s.name.clone(),
                kind: ItemKind::Interface,
                detail: s.detail(),
                doc: String::new(),
            });
        }
    }
    items
}

fn binding_item(b: &Binding) -> CompleteItem {
    CompleteItem {
        label: b.name.clone(),
        kind: b.kind,
        detail: b.detail.clone(),
        doc: b.doc.clone(),
    }
}
