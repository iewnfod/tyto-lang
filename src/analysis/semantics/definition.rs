//! 跳转定义：光标处的词 → 声明处位置（复用名字定位扫描索引）。

use crate::analysis::ty_view;
use crate::analysis::scope;
use crate::analysis::tolerate;
use crate::analysis::{before_word, word_at, ItemKind};
use crate::checker::ty::Type;
use crate::Span;

use super::names::{scan_names, NameRole, NameTok};
use super::{parse_full, DefLoc, SourceMap};

// ============ 跳转定义 ============

/// 跳转定义主入口：光标（LSP 位置）处的词 → 声明处位置。
/// 内置符号无源码位置，返回 None。
pub fn definition(src: &str, line0: usize, char_utf16: usize) -> Option<DefLoc> {
    let map = SourceMap::new(src);
    let cursor = map.to_span(line0, char_utf16);
    let word = word_at(&map, line0, char_utf16)?;
    if word.is_empty() || matches!(word.as_str(), "true" | "false" | "null" | "let") {
        return None;
    }

    // 全文档解析（词法/解析失败时尽力截断），供名字扫描
    let (_program, tokens) = parse_full(src)?;
    let names = scan_names(&tokens);

    let (mode, ..) = tolerate::cursor_mode_and_insert(src, map.offset(cursor));
    let before = before_word(&map, line0, char_utf16);
    let is_member = mode == tolerate::CursorMode::Member && before.ends_with(['.', '?']);

    if is_member {
        // 接收者推导 → struct 实例查扫描索引（字段/方法声明位置）
        let patched = tolerate::parse_at_cursor(src, cursor)?;
        let ambient = tolerate::parse_ambient(src, cursor);
        let registry_src = ambient.as_ref().unwrap_or(&patched.program);
        let reg = scope::collect_registry(registry_src);
        let snap = scope::collect_at_cursor(&patched.program, patched.sentinel, ambient.as_ref(), &reg);
        // 接收者类型已在快照期由引擎推出（联合解析见 member_recv）
        let recv = snap.receiver_ty.clone()?;
        let recv = ty_view::member_recv(&recv).unwrap_or(&Type::Any);
        if let Type::Struct(sname, _) = recv {
            // impl 方法优先查 MethodName（owner 匹配），字段查 StructField
            if let Some(n) = names.iter().find(|n| {
                n.role == NameRole::MethodName && n.owner.as_deref() == Some(sname.as_str()) && n.name == word
            }) {
                return to_defloc(&map, n.span, &n.name);
            }
            if let Some(n) = names.iter().find(|n| {
                n.role == NameRole::StructField && n.owner.as_deref() == Some(sname.as_str()) && n.name == word
            }) {
                return to_defloc(&map, n.span, &n.name);
            }
        }
        if let Type::Object(_) = &recv {
            // 对象字面量字段：就近向上的同名键（owner 匹配赋值目标可消歧）
            let cur_line = cursor.line;
            let cand = |n: &NameTok| {
                n.role == NameRole::ObjectKey && n.name == word && n.span.line <= cur_line
            };
            if let Some(n) = names.iter().rfind(|n| cand(n)) {
                return to_defloc(&map, n.span, &n.name);
            }
            return names.iter().find(|n| n.role == NameRole::ObjectKey && n.name == word)
                .and_then(|n| to_defloc(&map, n.span, &n.name));
        }
        if word == "self" {
            // self → struct 声明名（接收者的 struct 或绑定链上的 self 类型）
            let sname = match &recv {
                Type::Struct(s, _) => Some(s.clone()),
                _ => snap.scopes.iter().rev().find_map(|s| s.get("self")).and_then(|b| match &b.ty {
                    Type::Struct(s, _) => Some(s.clone()),
                    _ => None,
                }),
            }?;
            let n = names.iter().find(|n| n.role == NameRole::StructName && n.name == sname)?;
            return to_defloc(&map, n.span, &n.name);
        }
        return None;
    }

    // 全局/参数：作用域快照查绑定
    let patched = tolerate::parse_at_cursor(src, cursor);
    let Some(patched) = patched else {
        // 解析失败：在扫描索引里按名字兜底——就近的参数/泛型参数声明，
        // 再退到函数名/类型名（impl 体内操作数位置等补丁解析不了的场景）
        if let Some(n) = names.iter().rev().find(|n| {
            n.role == NameRole::Param && n.name == word && n.span.line <= cursor.line
        }) {
            return to_defloc(&map, n.span, &n.name);
        }
        if let Some(n) = names.iter().rev().find(|n| {
            n.role == NameRole::TypeParam && n.name == word && n.span.line <= cursor.line
        }) {
            return to_defloc(&map, n.span, &n.name);
        }
        return names
            .iter()
            .find(|n| n.name == word && matches!(n.role, NameRole::FuncName | NameRole::StructName | NameRole::InterfaceName))
            .and_then(|n| to_defloc(&map, n.span, &n.name));
    };
    let ambient = tolerate::parse_ambient(src, cursor);
    let registry_src = ambient.as_ref().unwrap_or(&patched.program);
    let reg = scope::collect_registry(registry_src);
    let snap = scope::collect_at_cursor(&patched.program, patched.sentinel, ambient.as_ref(), &reg);

    if let Some(b) = snap.scopes.iter().rev().find_map(|s| s.get(word.as_str())) {
        // 参数没有 AST span：扫描索引按名字就近兜底
        if b.span != Span::default() {
            return to_defloc(&map, b.span, &word);
        }
        if let Type::Struct(sname, _) = &b.ty {
            let n = names.iter().find(|n| n.role == NameRole::StructName && n.name == *sname)?;
            return to_defloc(&map, n.span, &n.name);
        }
        // 参数：光标上方最近的同名 Param token
        let cur_line = cursor.line;
        return names.iter().rfind(|n| {
            n.role == NameRole::Param && n.name == word && n.span.line <= cur_line
        })
            .and_then(|n| to_defloc(&map, n.span, &n.name));
    }

    // 光标在定义点上：词被哨兵替换。位置分两种：
    // - 赋值目标（Variable）：哨兵本体带前置换行，AST span 下移一行 → 减一；
    // - function/struct/interface 名：登记用「关键字位 + 偏移」近似，关键字在原行 → 原位。
    if let Some(b) = snap.scopes.iter().rev().find_map(|s| s.get(super::scope::SENTINEL))
        && b.span != Span::default()
    {
        let span = if mode == tolerate::CursorMode::Global
            && b.kind == ItemKind::Variable
        {
            Span::new(b.span.line.saturating_sub(1), b.span.col)
        } else {
            b.span
        };
        return to_defloc(&map, span, &word);
    }

    // 泛型参数引用：`val: T` / `-> T` / `TreeNode<T>` 里的 T → 就近向上的
    // 同名 TypeParam 声明（函数自带的 `<K>` 比 impl 的 `<T>` 近，天然遮蔽）
    if let Some(n) = names.iter().rev().find(|n| {
        n.role == NameRole::TypeParam && n.name == word && n.span.line <= cursor.line
    }) {
        return to_defloc(&map, n.span, &n.name);
    }

    // 无绑定：函数名/类型名兜底（用户函数在定义前被引用等场景）
    names
        .iter()
        .find(|n| n.name == word && matches!(n.role, NameRole::FuncName | NameRole::StructName | NameRole::InterfaceName))
        .and_then(|n| to_defloc(&map, n.span, &n.name))
}

fn to_defloc(map: &SourceMap, span: Span, name: &str) -> Option<DefLoc> {
    let (line, col) = map.from_span(span);
    Some(DefLoc { line, col, len: name.chars().count() })
}
