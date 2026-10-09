//! 补全条目构造：类型位置 / 成员 / 全局三个来源。

use crate::Span;

use super::builtins;
use super::infer::{self, Ty};
use super::scope::{self, Binding, Ctx, ScopeSnapshot, StructRegistry};
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

pub(super) fn member_items(snap: &ScopeSnapshot) -> Vec<CompleteItem> {
    let Some(recv_expr) = &snap.receiver else {
        return all_method_items();
    };
    let ctx = Ctx { scopes: &snap.scopes, structs: &snap.structs };
    let recv = infer::infer(recv_expr, &ctx);
    let mut items = match &recv {
        Ty::Namespace(ns) => builtins::namespace_fns(ns)
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
        Ty::Struct(name) => struct_member_items(name, &ctx),
        Ty::Object(fields) => fields
            .iter()
            .map(|f| CompleteItem {
                label: f.name.clone(),
                kind: if f.is_fn { ItemKind::Method } else { ItemKind::Field },
                detail: match &f.ty {
                    Ty::Func(Some(ret)) => format!("{}() -> {}", f.name, ret.display()),
                    Ty::Func(None) => format!("{}()", f.name),
                    t => format!("{}: {}", f.name, t.display()),
                },
                doc: if f.is_fn { "方法字段（对象字面量）".into() } else { "字段（对象字面量）".into() },
            })
            .collect(),
        // 基本类型没有成员
        Ty::Number | Ty::Bool | Ty::Null | Ty::Empty | Ty::Func(_) | Ty::StructDef(_)
        | Ty::InterfaceDef(_) => Vec::new(),
        // 内置容器类型
        Ty::Array | Ty::Str | Ty::Map | Ty::MaxHeap | Ty::MinHeap | Ty::Stack | Ty::Queue => {
            match builtins::methods_for(&recv) {
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
        // 未知：全量方法池（与旧行为一致）
        Ty::Unknown => return all_method_items(),
    };
    items.dedup_by(|a, b| a.label == b.label);
    items
}

/// struct 实例成员：字段（声明顺序）+ impl 方法
fn struct_member_items(name: &str, ctx: &Ctx) -> Vec<CompleteItem> {
    let Some(info) = ctx.structs.get(name) else {
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
        // 返回类型：`-> T` 标注优先，否则从方法体 return 推导
        let ret_str = match &m.ret {
            Some(t) => Some(t.to_string()),
            None => infer::function_return(&m.params, &m.body, ctx).map(|t| t.display()),
        };
        let detail = match ret_str {
            Some(r) => format!("{} -> {}", infer::func_sig(&m.name, &m.params, None), r),
            None => infer::func_sig(&m.name, &m.params, None),
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
