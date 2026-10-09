//! 编辑器分析核心：类型推导补全与悬停。
//!
//! 输入是源码 + LSP 位置（0-based 行、UTF-16 列），
//! 输出 [`CompleteItem`] / [`HoverInfo`]，由 LSP 层（src/lsp.rs）翻译成协议 JSON。
//!
//! 一切尽力而为：解析失败、类型推不出时回退静态表，绝不阻塞补全。

pub mod builtins;
pub mod infer;
pub mod scope;
pub mod semantics;
pub mod tolerate;

use crate::Span;
use infer::Ty;
use scope::{Binding, Ctx, ScopeSnapshot, StructRegistry};
use tolerate::{CursorMode, SourceMap};

/// 补全项类型（LSP 层负责映射到 CompletionItemKind 数值）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    Keyword,
    Function,
    Class,
    Constant,
    Variable,
    /// 函数参数（补全按 Variable 展示，语义着色单列）
    Parameter,
    Field,
    Method,
    Struct,
    Interface,
}

#[derive(Debug, Clone)]
pub struct CompleteItem {
    pub label: String,
    pub kind: ItemKind,
    /// 签名 / 推断类型（VSCode detail）
    pub detail: String,
    /// 中文文档（Markdown）
    pub doc: String,
}

#[derive(Debug, Clone)]
pub struct HoverInfo {
    /// 加粗首行（签名）
    pub signature: String,
    /// 其余文档
    pub doc: String,
}

/// 补全主入口
pub fn complete(src: &str, line0: usize, char_utf16: usize) -> Vec<CompleteItem> {
    let map = SourceMap::new(src);
    let cursor = map.to_span(line0, char_utf16);
    let cursor_off = map.offset(cursor);
    let (mode, _, _) = tolerate::cursor_mode_and_insert(src, cursor_off);

    // 类型标注位置（`:` / `->` 之后）：类型名补全，不经哨兵解析
    if mode == CursorMode::Type && tolerate::type_context(src, cursor_off) {
        return type_items(src, cursor);
    }

    let patched = tolerate::parse_at_cursor(src, cursor);
    let Some(patched) = patched else {
        // 完全解析失败：静态兜底（Type 已在上方提前返回，此处防御性归入 Global）
        return match mode {
            CursorMode::Member => all_method_items(),
            CursorMode::Type | CursorMode::Global => {
                global_static_items(&StructRegistry::default())
            }
        };
    };

    // struct 注册表：优先 ambient（更完整），退而求其次光标前缀
    let ambient = tolerate::parse_ambient(src, cursor);
    let registry_src = ambient.as_ref().unwrap_or(&patched.program);
    let structs = scope::collect_registry(registry_src);

    let snap: ScopeSnapshot = scope::collect_at_cursor(
        &patched.program,
        patched.sentinel,
        ambient.as_ref(),
        &structs,
    );

    match mode {
        CursorMode::Member => member_items(&snap),
        CursorMode::Type | CursorMode::Global => global_items(&snap),
    }
}

/// 悬停主入口
pub fn hover(src: &str, line0: usize, char_utf16: usize) -> Option<HoverInfo> {
    let map = SourceMap::new(src);
    let cursor = map.to_span(line0, char_utf16);
    let word = word_at(&map, line0, char_utf16)?;
    if word.is_empty() {
        return None;
    }

    let (mode, _, _) = tolerate::cursor_mode_and_insert(src, map.offset(cursor));
    let before = before_word(&map, line0, char_utf16);
    let is_member = mode == CursorMode::Member && before.ends_with(['.', '?']);

    let patched = tolerate::parse_at_cursor(src, cursor);
    let Some(patched) = patched else {
        // 解析失败：只查内置表
        return if is_member { builtin_member_info(&word) } else { builtin_global_info(&word) };
    };
    let ambient = tolerate::parse_ambient(src, cursor);
    let registry_src = ambient.as_ref().unwrap_or(&patched.program);
    let structs = scope::collect_registry(registry_src);
    let snap = scope::collect_at_cursor(&patched.program, patched.sentinel, ambient.as_ref(), &structs);

    if is_member {
        // 接收者 = 哨兵 Member 的 target；解析不到接收者时回退查全池
        if let Some(recv_expr) = &snap.receiver {
            let ctx = Ctx { scopes: &snap.scopes, structs: &snap.structs };
            let recv = infer::infer(recv_expr, &ctx);
            if let Some(info) = member_info(&recv, &word, &ctx) {
                return Some(info);
            }
        }
        builtin_member_info(&word)
    } else {
        scope_hover(&word, &snap).or_else(|| builtin_global_info(&word))
    }
}

// ============ 类型补全 ============

/// 类型标注位置的补全项：内置类型表 + 用户 struct / interface。
/// 注册表优先用 ambient（挖掉光标语句、尽量保留全文），退而全文档解析（截断容错）。
fn type_items(src: &str, cursor: Span) -> Vec<CompleteItem> {
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

fn member_items(snap: &ScopeSnapshot) -> Vec<CompleteItem> {
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

fn all_method_items() -> Vec<CompleteItem> {
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

fn global_items(snap: &ScopeSnapshot) -> Vec<CompleteItem> {
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

fn global_static_items(structs: &StructRegistry) -> Vec<CompleteItem> {
    let mut items: Vec<CompleteItem> = builtins::KEYWORDS
        .iter()
        .map(|k| CompleteItem {
            label: (*k).into(),
            kind: ItemKind::Keyword,
            detail: "keyword".into(),
            doc: String::new(),
        })
        .collect();
    for f in builtins::GLOBALS {
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

// ============ 悬停 ============

/// 作用域内绑定的悬停；查不到时尝试哨兵绑定（定义点：光标词被哨兵替换）
fn scope_hover(word: &str, snap: &ScopeSnapshot) -> Option<HoverInfo> {
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

fn builtin_global_info(word: &str) -> Option<HoverInfo> {
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

/// 已知接收者类型的成员悬停
fn member_info(recv: &Ty, word: &str, ctx: &Ctx) -> Option<HoverInfo> {
    match recv {
        Ty::Namespace(ns) => {
            let f = builtins::namespace_fns(ns)?.iter().find(|f| f.name == word)?;
            Some(HoverInfo {
                signature: format!("**{}**", f.sig),
                doc: f.doc.into(),
            })
        }
        Ty::Struct(name) => {
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
            let sig = infer::func_sig(&m.name, &m.params, m.ret.as_ref());
            Some(HoverInfo {
                signature: format!("**{}**", sig),
                doc: format!("方法（impl {}）", name),
            })
        }
        Ty::Object(fields) => {
            let f = fields.iter().find(|f| f.name == word)?;
            Some(HoverInfo {
                signature: format!("**{}.{}**", "对象", word),
                doc: if f.is_fn { "方法字段（对象字面量）".into() } else { "字段（对象字面量）".into() },
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
fn builtin_member_info(word: &str) -> Option<HoverInfo> {
    let m = builtins::all_methods().into_iter().find(|m| m.name == word)?;
    Some(HoverInfo {
        signature: format!("**{}**", m.sig),
        doc: m.doc.into(),
    })
}

// ============ 光标词提取 ============

/// 光标处标识符（光标可在词内或词尾）
pub(crate) fn word_at(map: &SourceMap, line0: usize, char_utf16: usize) -> Option<String> {
    let line = map.line(line0);
    let mut start = char_utf16.min(line.len());
    let mut end = start;
    let bytes = line.as_bytes();
    let is_word = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    while start > 0 && is_word(bytes[start - 1]) {
        start -= 1;
    }
    while end < bytes.len() && is_word(bytes[end]) {
        end += 1;
    }
    if start == end {
        return None;
    }
    line.get(start..end).map(|s| s.to_string())
}

/// 光标词之前的同行文本（判成员访问用；UTF-16 索引近似——`.`/`?` 均 ASCII）
pub(crate) fn before_word(map: &SourceMap, line0: usize, char_utf16: usize) -> String {
    let line = map.line(line0);
    let mut i = char_utf16.min(line.len());
    let bytes = line.as_bytes();
    let is_word = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    while i > 0 && is_word(bytes[i - 1]) {
        i -= 1;
    }
    line[..i].trim_end().to_string()
}
