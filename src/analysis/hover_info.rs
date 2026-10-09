//! 悬停信息构造：作用域绑定 / 内置全局 / 成员三个来源。

use super::builtins;
use super::ty_view;
use crate::checker::ty::Type;
use super::scope::{self, ScopeSnapshot};
use super::{Ctx, HoverInfo};

// ============ 悬停 ============

/// 作用域内绑定的悬停；查不到时尝试哨兵绑定（定义点：光标词被哨兵替换）
pub(super) fn scope_hover(word: &str, snap: &ScopeSnapshot) -> Option<HoverInfo> {
    let lookup = |name: &str| snap.scopes.iter().rev().find_map(|s| s.get(name));
    if let Some(b) = lookup(word) {
        return Some(HoverInfo {
            signature: format!("**{}**", b.detail),
            doc: b.doc.clone(),
        });
    }
    // `n = 42` 的 `n` 上悬停：n 被哨兵替换成 `__tyto_cx__ = 42`。
    // 哨兵绑定存在 ⟺ 光标词就是该语句的赋值目标，直接换名展示
    let b = lookup(scope::SENTINEL)?;
    if !b.detail.contains(scope::SENTINEL) {
        return None;
    }
    let detail = b.detail.replace(scope::SENTINEL, word);
    Some(HoverInfo {
        signature: format!("**{}**", detail),
        doc: b.doc.clone(),
    })
}

pub(super) fn builtin_global_info(word: &str) -> Option<HoverInfo> {
    if let Some(f) = builtins::global_fn(word) {
        return Some(HoverInfo {
            signature: format!("**{}**", f.sig),
            doc: f.doc.into(),
        });
    }
    if let Some((_, sig, doc)) = builtins::CLASSES.iter().find(|(n, _, _)| *n == word) {
        return Some(HoverInfo {
            signature: format!("**{}**", sig),
            doc: (*doc).into(),
        });
    }
    if let Some((_, sig, doc)) = builtins::CONSTANTS.iter().find(|(n, _, _)| *n == word) {
        return Some(HoverInfo {
            signature: format!("**{}**", sig),
            doc: (*doc).into(),
        });
    }
    None
}

/// 已知接收者类型的成员悬停。联合接收者去 null 后单臂按该臂解析，
/// 多臂无精确信息（返回 None 由调用方全池兜底）。
pub(super) fn member_info(recv: &Type, word: &str, ctx: &Ctx) -> Option<HoverInfo> {
    let recv = ty_view::member_recv(recv)?;
    match recv {
        Type::Namespace(ns) => {
            let fns = builtins::namespace_fns(ns)?;
            let f = fns.iter().find(|f| f.name == word)?;
            Some(HoverInfo {
                signature: format!("**{}**", f.sig),
                doc: f.doc.into(),
            })
        }
        Type::Struct(name, _) => {
            let info = ctx.structs.get(name)?;
            if let Some(f) = info.fields.iter().find(|p| p.name == word) {
                let ty_str = match &f.ty {
                    Some(t) => t.to_string(),
                    None => "unknown".into(),
                };
                return Some(HoverInfo {
                    signature: format!("**{}.{}**: {}", name, word, ty_str),
                    doc: format!("字段（struct {}）", name),
                });
            }
            let m = info.methods.iter().find(|m| m.name == word)?;
            let sig = ty_view::func_sig(&m.name, &m.params, m.ret.as_ref());
            Some(HoverInfo {
                signature: format!("**{}**", sig),
                doc: format!("方法（impl {}）", name),
            })
        }
        Type::Object(fields) => {
            let f = fields.iter().find(|(n, _)| n == word)?;
            let is_fn = matches!(f.1, Type::Func(_));
            Some(HoverInfo {
                signature: format!("**{}.{}**", "对象", word),
                doc: if is_fn { "方法字段（对象字面量）".into() } else { "字段（对象字面量）".into() },
            })
        }
        t => {
            let m = builtins::methods_for(t)?.iter().find(|m| m.name == word)?;
            Some(HoverInfo {
                signature: format!("**{}**", m.sig),
                doc: m.doc.into(),
            })
        }
    }
}

/// 接收者未知时按方法名查全池（可能与类型不匹配，注明适用类型）
pub(super) fn builtin_member_info(word: &str) -> Option<HoverInfo> {
    let m = builtins::all_methods().into_iter().find(|m| m.name == word)?;
    Some(HoverInfo {
        signature: format!("**{}**", m.sig),
        doc: m.doc.into(),
    })
}
