//! 语义着色与跳转定义。
//!
//! 两层配合：
//! 1. **名字定位扫描**（[`names`]，token 流单遍）：AST 里 `Member`/`FuncDecl`
//!    等声明的 span 都是语句起点，函数名/参数/字段/成员名没有独立位置——在
//!    token 流上用上下文（关键字前瞻 + 括号/大括号栈）把这些名字的位置精确
//!    补出来；
//! 2. **着色走查**（[`highlight`]，全文档 AST + token 游标）：作用域链 + 类型
//!    推导分类每个标识符引用，配 token 游标定位成员名，输出语义 token。
//!    [`definition`]（跳转定义）复用扫描索引。
//!
//! 原则：推不出的**不发 token**——VSCode 会保留 TextMate 静态着色，宁可少色
//! 不可错色。

mod definition;
mod highlight;
mod names;

pub use definition::definition;

use std::collections::HashMap;

use crate::ast::Stmt;
use crate::lexer::Token;
use crate::parser::Parser;
use crate::Span;

use super::scope;
use super::tolerate::{lex_tolerant, SourceMap};

use self::highlight::Highlighter;
use self::names::scan_names;

// ============ 对外类型 ============

/// 语义 token 类型表（LSP legend；插件端必须一致）
pub const TOKEN_TYPES: &[&str] = &[
    "variable",   // 0
    "parameter",  // 1
    "function",   // 2
    "method",     // 3
    "property",   // 4
    "struct",     // 5
    "interface",  // 6
    "class",      // 7
    "namespace",  // 8
    "type",       // 9：类型标注位置的标识符（VSCode 标准类型，主题缺省回退 entity.name.type）
];

/// 一个语义 token（LSP 位置：0-based 行、UTF-16 列）
#[derive(Debug, Clone, PartialEq)]
pub struct SemTok {
    pub line: usize,
    pub col: usize,
    pub len: usize,
    pub ty: u32,
}

/// 跳转定义结果（LSP 位置与长度）
#[derive(Debug, Clone, PartialEq)]
pub struct DefLoc {
    pub line: usize,
    pub col: usize,
    pub len: usize,
}

/// 语义着色主入口：整个文档的语义 token（按位置排序、去重）
pub fn semantic_tokens(src: &str) -> Vec<SemTok> {
    let Some((program, tokens)) = parse_full(src) else {
        return Vec::new();
    };
    let map = SourceMap::new(src);
    let names = scan_names(&tokens);
    let structs = scope::collect_registry(&program);

    // 扫描产出：声明位置 token（函数名/参数/字段/类型名/循环变量）
    let mut out: Vec<(Span, u32)> = names
        .iter()
        .filter_map(|n| Some((n.span, n.role.token_type()?)))
        .collect();

    // 走查产出：引用分类 token + 成员名分类
    let mut hl = Highlighter {
        structs: &structs,
        tokens: &tokens,
        tok_pos: 0,
        scopes: vec![HashMap::new()],
        emitted: Vec::new(),
        ck: crate::checker::Checker::for_cursor(structs.clone()),
    };
    hl.walk_stmts(top_stmts(&program));
    out.extend(hl.emitted);

    // 转 LSP 位置 + 排序去重
    let mut sems: Vec<SemTok> = out
        .into_iter()
        .map(|(span, ty)| {
            let (line, col) = map.from_span(span);
            SemTok { line, col, ty, len: 0 }
        })
        .collect();
    // 长度事后按名字补（走查只记位置）——重算：从源码该位置读标识符长度
    for t in &mut sems {
        t.len = ident_len_at(&map, t.line, t.col);
    }
    sems.sort_by_key(|t| (t.line, t.col));
    sems.dedup_by(|a, b| a.line == b.line && a.col == b.col);
    sems
}

/// 光标词的源码位置读标识符长度（ASCII 标识符：字符数 = UTF-16 数）
fn ident_len_at(map: &SourceMap, line: usize, col: usize) -> usize {
    let line_str = map.line(line);
    let mut units = 0usize;
    let mut len = 0usize;
    for c in line_str.chars() {
        if units < col {
            units += c.len_utf16();
            continue;
        }
        if c.is_ascii_alphanumeric() || c == '_' {
            len += 1;
        } else {
            break;
        }
    }
    len
}

pub(super) fn top_stmts(program: &Stmt) -> &[Stmt] {
    match program {
        Stmt::Block { stmts, .. } => stmts,
        _ => &[],
    }
}

/// 全文档解析（着色/定义用，无哨兵）：失败时截到出错行前重试一次；
/// 返回 (程序 AST, 与之匹配的 token 流)。彻底失败返回 None（回落 TextMate）。
pub(crate) fn parse_full(src: &str) -> Option<(Stmt, Vec<Token>)> {
    let try_one = |text: &str| -> Option<(Stmt, Vec<Token>)> {
        let out = lex_tolerant(text).ok()?;
        let program = Parser::new(out.tokens.clone()).parse_program().ok()?;
        Some((program, out.tokens))
    };
    if let Some(r) = try_one(src) {
        return Some(r);
    }
    // 截断重试：找到第一个能解析的前缀（逐行回退，最多 64 次）
    let mut cut = src.len();
    for _ in 0..64 {
        let Some(nl) = src[..cut].rfind('\n') else { break };
        cut = nl;
        if cut == 0 {
            return None;
        }
        if let Some(r) = try_one(&src[..cut]) {
            return Some(r);
        }
    }
    None
}
