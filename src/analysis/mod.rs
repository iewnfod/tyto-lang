//! 编辑器分析核心：类型推导补全与悬停。
//!
//! 输入是源码 + LSP 位置（0-based 行、UTF-16 列），
//! 输出 [`CompleteItem`] / [`HoverInfo`]，由 LSP 层（src/lsp.rs）翻译成协议 JSON。
//!
//! 一切尽力而为：解析失败、类型推不出时回退静态表，绝不阻塞补全。

pub mod builtins;
mod bridge;
pub mod infer;
pub mod scope;
pub mod semantics;
pub mod tolerate;

mod complete_items;
mod hover_info;

use scope::{Ctx, ScopeSnapshot, StructRegistry};
use tolerate::{CursorMode, SourceMap};

use complete_items::{
    all_method_items, global_items, global_static_items, member_items, type_items,
};
use hover_info::{builtin_global_info, builtin_member_info, member_info, scope_hover};

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
