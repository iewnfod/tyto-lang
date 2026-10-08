//! 中途编辑容错：补全请求到达时文档常是语法不完整的。
//!
//! 策略（配合 scope 的哨兵走查）：
//! 1. 在光标处插入哨兵标识符——`.` / `?.` 后直接接（成员模式，接收者可解析），
//!    否则换行后独立成句（全局模式，避免误入前一个表达式的函数字面量作用域）；
//! 2. 统计未闭合的 `(`/`[`/`{`，按开括号逆序补全闭括号，使前缀可解析；
//! 3. 首次解析失败则挖掉光标所在语句片段（回退到上一个边界）重试；
//! 4. ambient（全局启发式用）＝ 全文挖掉光标所在语句再修复，尽量保留完整文件。
//!
//! 位置换算：LSP 是 0-based 行 + UTF-16 码元列；词法器是 1-based 行 + 字符列。

use crate::ast::Stmt;
use crate::lexer::{LexOutput, Lexer, Token, TokenKind};
use crate::parser::Parser;
use crate::{RtResult, Span};

use super::scope::SENTINEL;

/// 源码的行起始偏移缓存（UTF-8 字节）
pub struct SourceMap<'s> {
    src: &'s str,
    line_starts: Vec<usize>,
}

impl<'s> SourceMap<'s> {
    pub fn new(src: &'s str) -> Self {
        let mut line_starts = vec![0usize];
        for (i, b) in src.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i + 1);
            }
        }
        SourceMap { src, line_starts }
    }

    /// 0-based 行 → 该行的 &str（不含换行；越界给空）
    pub fn line(&self, line0: usize) -> &'s str {
        let start = *self.line_starts.get(line0).unwrap_or(&self.src.len());
        let end = self
            .line_starts
            .get(line0 + 1)
            .map(|e| e - 1)
            .unwrap_or(self.src.len());
        self.src.get(start..end).unwrap_or("")
    }

    /// LSP 位置（0-based 行、UTF-16 列）→ 1-based（行、字符列）
    pub fn to_span(&self, line0: usize, char_utf16: usize) -> Span {
        let line = self.line(line0);
        let mut col = 1usize;
        let mut units = 0usize;
        for c in line.chars() {
            if units >= char_utf16 {
                break;
            }
            units += c.len_utf16();
            col += 1;
        }
        Span::new(line0 + 1, col)
    }

    /// Span（1-based）→ 源码字节偏移（越界钳到末尾）
    pub fn offset(&self, span: Span) -> usize {
        let start = *self
            .line_starts
            .get(span.line - 1)
            .unwrap_or(&self.src.len());
        let line_end = self
            .line_starts
            .get(span.line)
            .map(|e| e - 1)
            .unwrap_or(self.src.len());
        let line = self.src.get(start..line_end).unwrap_or("");
        let mut off = start;
        let mut chars = line.char_indices();
        for _ in 1..span.col {
            match chars.next() {
                Some((i, c)) => off = start + i + c.len_utf8(),
                None => break,
            }
        }
        off.min(self.src.len())
    }
}

/// 补全模式：成员访问（`.` / `?.` 后）或全局
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CursorMode {
    Member,
    Global,
}

/// 光标前文本的模式判定 + 哨兵插入点 + 光标词末尾（字节偏移）。
/// `insert_at`：哨兵插入处（成员 = `.` 后；全局 = 光标词首，避免劫持后半句）；
/// `rest_at`：光标词末尾——补丁保留其后原文，词整体被哨兵替换、语句保持完整。
pub fn cursor_mode_and_insert(src: &str, cursor_offset: usize) -> (CursorMode, usize, usize) {
    let prefix = &src[..cursor_offset];
    let bytes = prefix.as_bytes();
    let mut i = bytes.len();
    // `p.ab|` → 剥掉 `ab`（客户端按已输入前缀过滤）
    while i > 0 && (bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_') {
        i -= 1;
    }
    let trimmed = prefix[..i].trim_end();
    let mode = if trimmed.ends_with("?.") || (trimmed.ends_with('.') && !trimmed.ends_with("..")) {
        CursorMode::Member
    } else {
        CursorMode::Global
    };
    // 光标词末尾（`he|llo` → `llo` 也属于光标词）
    let rest = src.as_bytes();
    let mut rest_at = cursor_offset;
    while rest_at < rest.len() && (rest[rest_at].is_ascii_alphanumeric() || rest[rest_at] == b'_') {
        rest_at += 1;
    }
    match mode {
        CursorMode::Member => (mode, trimmed.len(), rest_at),
        CursorMode::Global => (mode, i, rest_at),
    }
}

/// 哨兵文本：成员模式直接拼接；全局模式换行独立成句
pub fn sentinel_text(mode: CursorMode) -> String {
    match mode {
        CursorMode::Member => SENTINEL.to_string(),
        CursorMode::Global => format!("\n{SENTINEL}"),
    }
}

/// token 是否为语句分隔符（片段边界）
fn is_separator(kind: &TokenKind) -> bool {
    matches!(kind, TokenKind::Eol | TokenKind::Semi | TokenKind::LBrace | TokenKind::RBrace)
}

/// 词法 + 容错：裸换行字符串等词法错误时，截到出错行前重试一次
fn lex_tolerant(src: &str) -> RtResult<LexOutput> {
    match Lexer::new(src).tokenize() {
        Ok(out) => Ok(out),
        Err(e) => {
            let err_line = match &e {
                crate::RtError::Lex { span, .. } | crate::RtError::Parse { span, .. } => span.line,
                crate::RtError::Runtime { span, .. } => span.map(|s| s.line).unwrap_or(1),
            };
            let cut = line_start(src, err_line.saturating_sub(1));
            if cut == 0 {
                Err(e)
            } else {
                Lexer::new(&src[..cut]).tokenize()
            }
        }
    }
}

/// 0-based 行号 → 行起始字节偏移
fn line_start(src: &str, line0: usize) -> usize {
    if line0 == 0 {
        return 0;
    }
    let mut n = 0usize;
    for (i, b) in src.bytes().enumerate() {
        if b == b'\n' {
            n += 1;
            if n == line0 {
                return i + 1;
            }
        }
    }
    src.len()
}

/// 未闭合深度 → 需补全的闭括号序列（与开括号逆序配对）
pub fn unclosed_closers(tokens: &[Token]) -> String {
    let mut stack = Vec::new();
    for t in tokens {
        match t.kind {
            TokenKind::LParen => stack.push(')'),
            TokenKind::LBracket => stack.push(']'),
            TokenKind::LBrace => stack.push('}'),
            TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace => {
                stack.pop();
            }
            _ => {}
        }
    }
    stack.truncate(64); // 异常输入防御
    stack.into_iter().rev().collect()
}

/// 哨兵 Ident 的 token 下标
fn sentinel_token_index(tokens: &[Token]) -> Option<usize> {
    tokens
        .iter()
        .position(|t| matches!(&t.kind, TokenKind::Ident(n) if n == SENTINEL))
}

/// 解析结果：AST + 哨兵起始位置
pub struct Patched {
    pub program: Stmt,
    pub sentinel: Span,
}

/// 文本 + 闭括号修复后尽力解析；成功要求哨兵在结果 AST 的 token 流中
fn try_parse(text: &str) -> Option<Patched> {
    let out = lex_tolerant(text).ok()?;
    let closers = unclosed_closers(&out.tokens);
    let full = if closers.is_empty() { text.to_string() } else { format!("{text}{closers}") };
    let out = Lexer::new(&full).tokenize().ok()?;
    let sentinel = sentinel_token_index(&out.tokens).map(|i| out.tokens[i].span)?;
    let program = Parser::new(out.tokens).parse_program().ok()?;
    Some(Patched { program, sentinel })
}

/// 主入口：`src` + 光标（1-based Span）→ 补好哨兵、修复闭括号、尽力解析的 AST。
/// 解析彻底失败返回 None（调用方退回静态补全）。
///
/// 补丁 = `src[..insert_at]` + 哨兵 + **光标后的原文**（词被哨兵替换、语句保持完整，
/// 悬停定义点也能工作）；失败则回退为「截到光标语句边界 + 哨兵」。
pub fn parse_at_cursor(src: &str, cursor: Span) -> Option<Patched> {
    let map = SourceMap::new(src);
    let cursor_off = map.offset(cursor);
    let (mode, insert_at, rest_at) = cursor_mode_and_insert(src, cursor_off);
    let sentinel = sentinel_text(mode);

    let mut patched = String::with_capacity(src.len() + sentinel.len() + 64);
    patched.push_str(&src[..insert_at]);
    patched.push_str(&sentinel);
    patched.push_str(&src[rest_at..]);
    if let Some(p) = try_parse(&patched) {
        return Some(p);
    }

    // 回退：挖掉光标所在语句片段（到上一个边界）再拼哨兵
    let frag_start = fragment_start(src, cursor_off);
    if frag_start < insert_at {
        let mut retry = String::with_capacity(frag_start + sentinel.len() + 64);
        retry.push_str(&src[..frag_start]);
        retry.push_str(&sentinel);
        if let Some(p) = try_parse(&retry) {
            return Some(p);
        }
    }
    None
}

/// 光标前缀里当前语句片段的起点（上一个分隔符 token 之后；无则 0）
fn fragment_start(src: &str, cursor_off: usize) -> usize {
    let out = match lex_tolerant(&src[..cursor_off]) {
        Ok(o) => o,
        Err(_) => return 0,
    };
    let back = match out.tokens.iter().rposition(|t| is_separator(&t.kind)) {
        Some(i) => i,
        None => return 0,
    };
    // 片段起点 = 分隔符字符之后（分隔符都是单字符）
    let map = SourceMap::new(&src[..cursor_off]);
    (map.offset(out.tokens[back].span) + 1).min(cursor_off)
}

/// ambient：全文挖掉光标所在语句（按分隔符 token 定位边界），修复后解析。
/// 用于「函数体内编辑看全局最终态」的启发式；失败返回 None。
pub fn parse_ambient(src: &str, cursor: Span) -> Option<Stmt> {
    let out = lex_tolerant(src).ok()?;
    // 光标 → 首个起始位置 ≥ 光标的 token
    let cursor_tok = out
        .tokens
        .iter()
        .position(|t| t.span >= cursor)
        .unwrap_or(out.tokens.len().saturating_sub(1));
    // 光标前最后一个分隔符（保留其字符）
    let back = out.tokens[..cursor_tok].iter().rposition(|t| is_separator(&t.kind))? + 1;
    // 光标后（含光标 token）第一个语句结束符——只认 Eol/Semi：
    // 表达式里的 `{`/`}`（对象字面量）不是语句边界，误停会把赋值拦腰切断
    let fwd = out.tokens[cursor_tok..]
        .iter()
        .position(|t| matches!(t.kind, TokenKind::Eol | TokenKind::Semi))
        .map(|i| cursor_tok + i)?;

    let map = SourceMap::new(src);
    // 前段：保留分隔符字符本身（`{`/`}` 不能丢）
    let head_end = if back == 0 { 0 } else { map.offset(out.tokens[back - 1].span) + 1 };
    // 片段终点：扫描中遇到**不匹配的闭括号**就停——它属于外围结构，必须留给 tail
    //（否则函数体的 `}` 被挖走，后面的顶层语句掉进函数体里）
    let mut frag_end = fwd;
    let mut depth = 0i32;
    for (i, t) in out.tokens[back..fwd].iter().enumerate() {
        match t.kind {
            TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace => depth += 1,
            TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace => {
                if depth == 0 {
                    frag_end = back + i;
                    break;
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    let tail_start = if frag_end < fwd {
        // 不匹配的闭括号处：tail 从它本身开始（保留）
        map.offset(out.tokens[frag_end].span)
    } else {
        // Eol/Semi 后：丢弃该分隔符
        map.offset(out.tokens[fwd].span) + 1
    };
    if tail_start > src.len() || head_end > tail_start {
        return None;
    }

    let mut spliced = String::with_capacity(src.len());
    spliced.push_str(&src[..head_end]);
    spliced.push_str(&src[tail_start..]);
    let out2 = lex_tolerant(&spliced).ok()?;
    let closers = unclosed_closers(&out2.tokens);
    if !closers.is_empty() {
        spliced.push_str(&closers);
    }
    let out2 = Lexer::new(&spliced).tokenize().ok()?;
    Parser::new(out2.tokens).parse_program().ok()
}
