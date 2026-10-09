//! 语义着色与跳转定义。
//!
//! 两层配合：
//! 1. **名字定位扫描**（token 流单遍）：AST 里 `Member`/`FuncDecl` 等声明的
//!    span 都是语句起点，函数名/参数/字段/成员名没有独立位置——在 token 流上
//!    用上下文（关键字前瞻 + 括号/大括号栈）把这些名字的位置精确补出来；
//! 2. **着色走查**（全文档 AST + token 游标）：作用域链 + 类型推导分类每个
//!    标识符引用，配 token 游标定位成员名，输出语义 token。
//!
//! 原则：推不出的**不发 token**——VSCode 会保留 TextMate 静态着色，宁可少色
//! 不可错色。

use std::collections::HashMap;

use crate::ast::{Expr, ForIter, Param, Stmt};
use crate::lexer::{Keyword, Token, TokenKind};
use crate::parser::Parser;
use crate::Span;

use super::builtins;
use super::infer::{self, Ty};
use super::scope::{self, Binding, Ctx, StructRegistry};
use super::tolerate::{lex_tolerant, SourceMap};

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

fn top_stmts(program: &Stmt) -> &[Stmt] {
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

// ============ 名字定位扫描 ============

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum NameRole {
    FuncName,
    /// impl 内的方法名（owner = struct 名）
    MethodName,
    Param,
    StructName,
    InterfaceName,
    StructField,
    InterfaceMethod,
    LoopVar,
    /// `impl X` 的目标名（着色同 struct；跳转定义不认它）
    ImplTarget,
    /// 泛型参数声明：`struct Box<T>` / `impl X<T>` / `function id<T>` 头部
    /// `<...>` 里的名字。着色交回 TextMate（语法文件已覆盖）；位置供
    /// 跳转定义按「就近向上」解析引用（方法自带 `<K>` 遮蔽 impl 的 `<T>`）
    TypeParam,
    /// 类型标注表达式中的标识符（`number`、`Map<string, number>` 里的每个名字）
    Type,
    /// 对象字面量键（着色由 AST 走查负责；此处只为跳转定义记录位置）
    ObjectKey,
}

impl NameRole {
    fn token_type(&self) -> Option<u32> {
        Some(match self {
            NameRole::FuncName => 2,
            NameRole::MethodName | NameRole::InterfaceMethod => 3,
            NameRole::Param => 1,
            NameRole::StructName | NameRole::ImplTarget => 5,
            NameRole::InterfaceName => 6,
            NameRole::StructField => 4,
            NameRole::LoopVar => 0,
            NameRole::Type => 9,
            // 着色由 TextMate 语法文件的 typeparameters 规则负责，扫描不上色
            NameRole::TypeParam => return None,
            // 着色由 AST 走查的 Object 分支发出，扫描不重复上色
            NameRole::ObjectKey => return None,
        })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct NameTok {
    pub span: Span,
    pub name: String,
    pub role: NameRole,
    /// 方法所属 struct / 字段所属 struct
    pub owner: Option<String>,
}

// ============ 类型表达式扫描 ============

/// 静默走一个类型表达式（`number`、`Map<string, number>`、`number[]`，
/// 支持嵌套泛型）：返回结束下标。首 token 不是标识符时原样返回 start
///（标注写到一半等残缺形态不发射任何 token）。
pub(crate) fn type_end(tokens: &[Token], start: usize) -> usize {
    let mut j = start;
    if !matches!(tokens.get(j).map(|t| &t.kind), Some(TokenKind::Ident(_))) {
        return j;
    }
    j += 1;
    if matches!(tokens.get(j).map(|t| &t.kind), Some(TokenKind::Lt)) {
        j += 1;
        loop {
            match tokens.get(j).map(|t| &t.kind) {
                Some(TokenKind::Ident(_)) => {
                    j = type_end(tokens, j); // 递归：泛型参数还可以是泛型
                }
                Some(TokenKind::Gt) => {
                    j += 1;
                    break;
                }
                Some(TokenKind::Comma) => j += 1,
                _ => return j, // 结构意外：就地停下，避免吞掉后续 token
            }
        }
    }
    while matches!(tokens.get(j).map(|t| &t.kind), Some(TokenKind::LBracket))
        && matches!(tokens.get(j + 1).map(|t| &t.kind), Some(TokenKind::RBracket))
    {
        j += 2;
    }
    j
}

/// 把 `[start..end)` 区间内的标识符全部按类型发射（区间来自 [`type_end`]）
fn emit_type_idents(tokens: &[Token], start: usize, end: usize, out: &mut Vec<NameTok>) {
    for t in tokens.iter().take(end).skip(start) {
        if let TokenKind::Ident(name) = &t.kind {
            out.push(NameTok {
                span: t.span,
                name: name.clone(),
                role: NameRole::Type,
                owner: None,
            });
        }
    }
}

/// 扫描泛型参数声明 `<T, U>`：`<` 处开始，返回匹配 `>` 之后的下标，
/// 每个标识符记为 [`NameRole::TypeParam`]（跳转定义的声明位置）。
/// 首 token 不是 `<` 时原样返回 start（写到一半的残缺形态不记录）。
fn scan_type_params(tokens: &[Token], start: usize, out: &mut Vec<NameTok>) -> usize {
    if !matches!(tokens.get(start).map(|t| &t.kind), Some(TokenKind::Lt)) {
        return start;
    }
    let mut j = start + 1;
    loop {
        match tokens.get(j).map(|t| &t.kind) {
            Some(TokenKind::Ident(name)) => {
                out.push(NameTok {
                    span: tokens[j].span,
                    name: name.clone(),
                    role: NameRole::TypeParam,
                    owner: None,
                });
                j += 1;
            }
            Some(TokenKind::Comma) => j += 1,
            Some(TokenKind::Gt) => return j + 1,
            // 结构意外（写到一半 / 语法错误）：就地停下，不吞后续 token
            _ => return j,
        }
    }
}

/// 大括号上下文
#[derive(Debug, Clone, PartialEq)]
enum BraceCtx {
    Block,
    Impl(String),
    StructBody(String),
    InterfaceBody,
    /// 对象字面量（owner = `x = {...}` 的目标名，供跳转定义消歧）
    ObjectLit(Option<String>),
}

/// token 流单遍扫描：定位 AST 中没有独立 span 的名字
pub(crate) fn scan_names(tokens: &[Token]) -> Vec<NameTok> {
    let mut out = Vec::new();
    let mut stack: Vec<BraceCtx> = Vec::new();
    let len = tokens.len();
    let mut i = 0usize;

    while i < len {
        match &tokens[i].kind {
            TokenKind::Keyword(Keyword::Function) => {
                // 具名函数 / 方法；匿名函数直接进参数。
                // `new` 词法上是关键字（`new Point()`），但它是惯用构造器方法名——
                // 与 parser 的 expect_method_name 对齐：Ident 或 Keyword(New) 都认
                let (name, nspan) = match tokens.get(i + 1) {
                    Some(Token { kind: TokenKind::Ident(n), span }) => (Some(n.clone()), *span),
                    Some(Token { kind: TokenKind::Keyword(Keyword::New), span }) => {
                        (Some("new".to_string()), *span)
                    }
                    _ => (None, Span::default()),
                };
                if let Some(name) = name {
                    let (role, owner) = match stack.last() {
                        Some(BraceCtx::Impl(t)) => (NameRole::MethodName, Some(t.clone())),
                        Some(BraceCtx::InterfaceBody) => (NameRole::InterfaceMethod, None),
                        _ => (NameRole::FuncName, None),
                    };
                    out.push(NameTok {
                        span: nspan,
                        name,
                        role,
                        owner,
                    });
                    i += 2;
                } else {
                    i += 1;
                }
                // 泛型参数声明：`function id<T>(...)` / 匿名 `function<T>(...)`
                i = scan_type_params(tokens, i, &mut out);
                // 参数列表
                if matches!(tokens.get(i).map(|t| &t.kind), Some(TokenKind::LParen)) {
                    i = scan_params(tokens, i, &mut out);
                    // 返回类型标注：`) -> T`（`->` 允许出现在换行之后，与 parser 的 skip_eol 一致）
                    let mut j = i;
                    while matches!(tokens.get(j).map(|t| &t.kind), Some(TokenKind::Eol)) {
                        j += 1;
                    }
                    if matches!(tokens.get(j).map(|t| &t.kind), Some(TokenKind::Arrow)) {
                        let end = type_end(tokens, j + 1);
                        emit_type_idents(tokens, j + 1, end, &mut out);
                    }
                }
            }
            TokenKind::Keyword(Keyword::Struct) | TokenKind::Keyword(Keyword::Interface) => {
                let is_struct = matches!(tokens[i].kind, TokenKind::Keyword(Keyword::Struct));
                if let Some(TokenKind::Ident(name)) = tokens.get(i + 1).map(|t| &t.kind) {
                    out.push(NameTok {
                        span: tokens[i + 1].span,
                        name: name.clone(),
                        role: if is_struct { NameRole::StructName } else { NameRole::InterfaceName },
                        owner: None,
                    });
                    // 泛型参数声明 `struct Box<T>`，再跳到体 `{` 压入上下文
                    let mut j = scan_type_params(tokens, i + 2, &mut out);
                    while j < len && !matches!(tokens[j].kind, TokenKind::LBrace | TokenKind::Eol | TokenKind::Eof) {
                        j += 1;
                    }
                    if matches!(tokens.get(j).map(|t| &t.kind), Some(TokenKind::LBrace)) {
                        stack.push(if is_struct {
                            BraceCtx::StructBody(name.clone())
                        } else {
                            BraceCtx::InterfaceBody
                        });
                        j += 1;
                    }
                    i = j;
                } else {
                    i += 1;
                }
            }
            TokenKind::Keyword(Keyword::Impl) => {
                if let Some(TokenKind::Ident(name)) = tokens.get(i + 1).map(|t| &t.kind) {
                    out.push(NameTok {
                        span: tokens[i + 1].span,
                        name: name.clone(),
                        role: NameRole::ImplTarget,
                        owner: None,
                    });
                    // 泛型参数声明 `impl X<T>`，再跳到体 `{` 压入上下文
                    let mut j = scan_type_params(tokens, i + 2, &mut out);
                    while j < len && !matches!(tokens[j].kind, TokenKind::LBrace | TokenKind::Eol | TokenKind::Eof) {
                        j += 1;
                    }
                    if matches!(tokens.get(j).map(|t| &t.kind), Some(TokenKind::LBrace)) {
                        stack.push(BraceCtx::Impl(name.clone()));
                        j += 1;
                    }
                    i = j;
                } else {
                    i += 1;
                }
            }
            TokenKind::Keyword(Keyword::For) => {
                // for x in ... / for x = ...
                let ident = matches!(tokens.get(i + 1).map(|t| &t.kind), Some(TokenKind::Ident(_)));
                let sep = matches!(
                    tokens.get(i + 2).map(|t| &t.kind),
                    Some(TokenKind::Keyword(Keyword::In)) | Some(TokenKind::Assign)
                );
                if ident && sep {
                    if let Some(TokenKind::Ident(name)) = tokens.get(i + 1).map(|t| &t.kind) {
                        out.push(NameTok {
                            span: tokens[i + 1].span,
                            name: name.clone(),
                            role: NameRole::LoopVar,
                            owner: None,
                        });
                    }
                    i += 3;
                } else {
                    i += 1;
                }
            }
            TokenKind::LBrace => {
                stack.push(classify_brace(tokens, i));
                i += 1;
            }
            TokenKind::RBrace => {
                stack.pop();
                i += 1;
            }
            TokenKind::Ident(name) => {
                // struct 字段声明：struct 体内、非 `:` 之后、后随 , : } 或行尾
                if let Some(BraceCtx::StructBody(owner)) = stack.last() {
                    let prev_colon = matches!(tokens[i.saturating_sub(1)].kind, TokenKind::Colon);
                    let next_ok = matches!(
                        tokens.get(i + 1).map(|t| &t.kind),
                        Some(TokenKind::Colon | TokenKind::Comma | TokenKind::RBrace | TokenKind::Eol)
                    );
                    let prev_dot = matches!(
                        tokens[i.saturating_sub(1)].kind,
                        TokenKind::Dot | TokenKind::QuestionDot
                    );
                    if next_ok && !prev_colon && !prev_dot {
                        out.push(NameTok {
                            span: tokens[i].span,
                            name: name.clone(),
                            role: NameRole::StructField,
                            owner: Some(owner.clone()),
                        });
                        // 字段类型标注：`field: T`
                        if matches!(tokens.get(i + 1).map(|t| &t.kind), Some(TokenKind::Colon)) {
                            let end = type_end(tokens, i + 2);
                            emit_type_idents(tokens, i + 2, end, &mut out);
                            i = end;
                            continue;
                        }
                    }
                }
                // 语句级变量标注：`x: T = v`——标识符在语句起点（前一 token 是
                // 行尾/分号/`{`/文件开头），`:` 后是类型表达式且以 `=` 收尾。
                // 三元的 `?:`（前置是 `?`/运算符）与对象字面量键值（ObjectLit
                // 上下文）都到不了这里，`=` 前瞻再兜一道底。
                if stack
                    .last()
                    .is_none_or(|c| matches!(c, BraceCtx::Block))
                    && matches!(tokens.get(i + 1).map(|t| &t.kind), Some(TokenKind::Colon))
                    && (i == 0
                        || matches!(
                            tokens[i - 1].kind,
                            TokenKind::Eol | TokenKind::Semi | TokenKind::LBrace
                        ))
                {
                    let end = type_end(tokens, i + 2);
                    if matches!(tokens.get(end).map(|t| &t.kind), Some(TokenKind::Assign)) {
                        emit_type_idents(tokens, i + 2, end, &mut out);
                    }
                }
                // 对象字面量键：`{`/`,`/行首 之后、`:` 之前（跳转定义记录用）
                if let Some(BraceCtx::ObjectLit(owner)) = stack.last() {
                    let prev_key_pos = matches!(
                        tokens[i.saturating_sub(1)].kind,
                        TokenKind::LBrace | TokenKind::Comma | TokenKind::Eol
                    );
                    if prev_key_pos
                        && matches!(tokens.get(i + 1).map(|t| &t.kind), Some(TokenKind::Colon))
                    {
                        out.push(NameTok {
                            span: tokens[i].span,
                            name: name.clone(),
                            role: NameRole::ObjectKey,
                            owner: owner.clone(),
                        });
                    }
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    out
}

/// `{` 的上下文：前驱 token 决定（`= ( [ , : ? return` → 对象字面量；其余 → 块）
fn classify_brace(tokens: &[Token], i: usize) -> BraceCtx {
    let prev = tokens.get(i.saturating_sub(1)).map(|t| &t.kind);
    let is_object = matches!(
        prev,
        Some(TokenKind::Assign | TokenKind::LParen | TokenKind::LBracket | TokenKind::Comma
         | TokenKind::Colon | TokenKind::Question | TokenKind::Keyword(Keyword::Return))
    );
    if !is_object {
        return BraceCtx::Block;
    }
    // `x = {`：owner = 赋值目标名（跳转定义消歧用）
    let owner = if let Some(TokenKind::Assign) = prev {
        match tokens.get(i.saturating_sub(2)).map(|t| &t.kind) {
            Some(TokenKind::Ident(n)) => Some(n.clone()),
            _ => None,
        }
    } else {
        None
    };
    BraceCtx::ObjectLit(owner)
}

/// 扫描函数参数：`(` 处开始，返回匹配 `)` 之后的下标
fn scan_params(tokens: &[Token], start: usize, out: &mut Vec<NameTok>) -> usize {
    let mut depth = 0i32;
    let mut j = start;
    while j < tokens.len() {
        match &tokens[j].kind {
            TokenKind::LParen => depth += 1,
            TokenKind::RParen => {
                depth -= 1;
                if depth == 0 {
                    return j + 1;
                }
            }
            TokenKind::Eof => return j,
            TokenKind::Ident(name) if depth == 1 => {
                let prev_colon = matches!(tokens[j.saturating_sub(1)].kind, TokenKind::Colon);
                let next_ok = matches!(
                    tokens.get(j + 1).map(|t| &t.kind),
                    Some(TokenKind::Colon | TokenKind::Comma | TokenKind::RParen)
                );
                if next_ok && !prev_colon {
                    out.push(NameTok {
                        span: tokens[j].span,
                        name: name.clone(),
                        role: NameRole::Param,
                        owner: None,
                    });
                    // 参数类型标注：`param: T`（T 可带泛型与 [] 后缀）
                    if matches!(tokens.get(j + 1).map(|t| &t.kind), Some(TokenKind::Colon)) {
                        let end = type_end(tokens, j + 2);
                        emit_type_idents(tokens, j + 2, end, out);
                        j = end;
                        continue;
                    }
                }
            }
            _ => {}
        }
        j += 1;
    }
    j
}

// ============ 着色走查 ============

struct Highlighter<'a> {
    structs: &'a StructRegistry,
    tokens: &'a [Token],
    /// token 游标：AST 访问序 = token 序，用于定位成员名等
    tok_pos: usize,
    scopes: Vec<HashMap<String, Binding>>,
    emitted: Vec<(Span, u32)>,
}

impl<'a> Highlighter<'a> {
    fn walk_stmts(&mut self, stmts: &[Stmt]) {
        for stmt in stmts {
            self.sync_to(stmt.span());
            // 叶子语句登记绑定（容器语句递归时在各自作用域内登记）
            if matches!(
                stmt,
                Stmt::Assign { .. } | Stmt::FuncDecl { .. } | Stmt::Struct { .. } | Stmt::Interface { .. }
            ) {
                scope::register_leaf(self.structs, stmt, &mut self.scopes);
            }
            self.walk_stmt(stmt);
        }
    }

    fn walk_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Assign { target, value, .. } => {
                self.walk_expr(target);
                self.walk_expr(value);
            }
            Stmt::Expr(e, _) => self.walk_expr(e),
            Stmt::Return { value: Some(e), .. } => self.walk_expr(e),
            Stmt::If { cond, then_block, else_block, .. } => {
                self.walk_expr(cond);
                self.scopes.push(HashMap::new());
                self.walk_stmts(top_stmts(then_block));
                // else-if 的 else_block 是 If 语句而非 Block，直接 walk_stmt
                if let Some(e) = else_block {
                    self.walk_stmt(e);
                }
            }
            Stmt::While { cond, body, .. } => {
                self.walk_expr(cond);
                self.scopes.push(HashMap::new());
                self.walk_stmts(top_stmts(body));
            }
            Stmt::ForIn { var, iter, body, .. } => {
                self.scopes.push(HashMap::new());
                // 元素类型：字符串迭代 → string，区间 → number，数组 → unknown（异构）
                let elem = match iter {
                    ForIter::Range { .. } => Ty::Number,
                    ForIter::Expr(Expr::Str(..)) => Ty::Str,
                    ForIter::Expr(e) => {
                        let ctx = Ctx { scopes: &self.scopes, structs: self.structs };
                        match infer::infer(e, &ctx) {
                            Ty::Str => Ty::Str,
                            _ => Ty::Unknown,
                        }
                    }
                };
                let vspan = loop_var_span(stmt);
                self.bind_var(var, elem, vspan);
                if let ForIter::Expr(e) = iter {
                    self.walk_expr(e);
                }
                self.sync_to(body.span());
                self.walk_stmts(top_stmts(body));
            }
            Stmt::ForC { var, init, cond, step, body, .. } => {
                self.scopes.push(HashMap::new());
                let ctx = Ctx { scopes: &self.scopes, structs: self.structs };
                let t = infer::infer(init, &ctx);
                let vspan = loop_var_span(stmt);
                self.bind_var(var, t, vspan);
                self.walk_expr(init);
                if let Some(c) = cond {
                    self.walk_expr(c);
                }
                if let Some(st) = step {
                    match &**st {
                        Stmt::Assign { target, value, .. } => {
                            self.walk_expr(target);
                            self.walk_expr(value);
                        }
                        Stmt::Expr(e, _) => self.walk_expr(e),
                        _ => {}
                    }
                }
                self.sync_to(body.span());
                self.walk_stmts(top_stmts(body));
            }
            Stmt::FuncDecl { params, ret, body, .. } => {
                // 名字与参数 token 由扫描产出；这里压作用域绑参数，走函数体
                self.scopes.push(HashMap::new());
                self.bind_params(params);
                let _ = ret;
                self.sync_to(body.span());
                self.walk_stmts(top_stmts(body));
            }
            Stmt::Impl { target, methods, .. } => {
                for m in methods {
                    if let Stmt::FuncDecl { name, params, ret, body, .. } = m {
                        self.sync_to(m.span());
                        self.scopes.push(HashMap::new());
                        self.scopes.last_mut().unwrap().insert(
                            "self".into(),
                            Binding {
                                name: "self".into(),
                                ty: Ty::Struct(target.clone()),
                                kind: super::ItemKind::Variable,
                                detail: format!("self: {}", target),
                                doc: String::new(),
                                span: Span::default(),
                            },
                        );
                        self.bind_params(params);
                        let _ = name;
                        let _ = ret;
                        self.sync_to(body.span());
                        self.walk_stmts(top_stmts(body));
                    }
                }
            }
            Stmt::Struct { .. } | Stmt::Interface { .. } => {
                // 声明 token 由扫描产出；体内无可执行内容
            }
            Stmt::Block { .. } => {
                self.scopes.push(HashMap::new());
                self.walk_stmts(top_stmts(stmt));
            }
            _ => {}
        }
    }

    fn bind_var(&mut self, name: &str, ty: Ty, span: Span) {
        let detail = format!("{}: {}", name, ty.display());
        self.scopes.last_mut().unwrap().insert(
            name.to_string(),
            Binding {
                name: name.to_string(),
                ty,
                kind: super::ItemKind::Variable,
                detail,
                doc: "循环变量".into(),
                span,
            },
        );
    }

    fn bind_params(&mut self, params: &[Param]) {
        let frame = self.scopes.last_mut().unwrap();
        for p in params {
            let ty = p
                .ty
                .as_ref()
                .map(|a| infer::ty_from_ast(a, self.structs))
                .unwrap_or(Ty::Unknown);
            let detail = format!("{}: {}", p.name, ty.display());
            frame.insert(
                p.name.clone(),
                Binding {
                    name: p.name.clone(),
                    ty,
                    kind: super::ItemKind::Parameter,
                    detail,
                    doc: String::new(),
                    span: Span::default(),
                },
            );
        }
    }

    fn walk_expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Ident(name, span) => {
                if let Some(t) = self.classify_ident(name) {
                    self.emitted.push((*span, t));
                }
                // 游标自愈：用自己的 span 定位（跳过 = 等不产生 AST 节点的 token）
                self.sync_to(*span);
                if matches!(self.tokens.get(self.tok_pos).map(|t| &t.kind), Some(TokenKind::Ident(n)) if n == name) {
                    self.tok_pos += 1;
                }
            }
            Expr::Member { target, name, .. } | Expr::OptionalMember { target, name, .. } => {
                self.walk_expr(target);
                // 游标应停在 `.` 上；成员名 = 下一个 token
                if let Some(nspan) = self.member_name_span(name)
                    && let Some(t) = self.classify_member(target, name)
                {
                    self.emitted.push((nspan, t));
                }
            }
            Expr::Call { callee, args, .. } => {
                self.walk_expr(callee);
                for a in args {
                    self.walk_expr(a);
                }
            }
            Expr::New { class, args, .. } => {
                if let Expr::Ident(name, span) = &**class {
                    let t = if builtins::is_builtin_class(name) {
                        Some(7) // class
                    } else if self.structs.contains(name) {
                        Some(5) // struct
                    } else {
                        self.classify_ident(name)
                    };
                    if let Some(t) = t {
                        self.emitted.push((*span, t));
                    }
                    self.sync_to(*span);
                    if matches!(self.tokens.get(self.tok_pos).map(|t| &t.kind), Some(TokenKind::Ident(n)) if n == name) {
                        self.tok_pos += 1;
                    }
                } else {
                    self.walk_expr(class);
                }
                for a in args {
                    self.walk_expr(a);
                }
            }
            Expr::Object(fields, _) => {
                self.skip_open(); // `{`
                for (k, v) in fields {
                    // 键：值是函数字面量 → method，否则 property
                    if let Some(kspan) = self.object_key_span(k) {
                        let t = if matches!(v, Expr::Function { .. }) { 3 } else { 4 };
                        self.emitted.push((kspan, t));
                    }
                    self.walk_expr(v);
                }
                self.skip_close(); // `}`（或停在附近）
            }
            Expr::Array(elems, _) => {
                for e in elems {
                    self.walk_expr(e);
                }
            }
            Expr::Binary { left, right, .. } | Expr::Logic { left, right, .. } => {
                self.walk_expr(left);
                self.walk_expr(right);
            }
            Expr::Unary { operand, .. } => self.walk_expr(operand),
            Expr::Ternary { cond, then_expr, else_expr, .. } => {
                self.walk_expr(cond);
                self.walk_expr(then_expr);
                self.walk_expr(else_expr);
            }
            Expr::Index { target, index, .. } => {
                self.walk_expr(target);
                self.walk_expr(index);
            }
            Expr::Slice { target, start, end, .. } => {
                self.walk_expr(target);
                for e in [start, end].into_iter().flatten() {
                    self.walk_expr(e);
                }
            }
            Expr::Is { operand, target, .. } => {
                self.walk_expr(operand);
                self.walk_expr(target);
            }
            Expr::Function { params, body, .. } => {
                // 匿名函数体：独立作用域（参数 token 由扫描产出）
                self.sync_to(body.span());
                self.scopes.push(HashMap::new());
                self.bind_params(params);
                self.walk_stmts(top_stmts(body));
            }
            _ => {}
        }
    }

    /// 标识符引用分类（作用域优先，回退内置表）
    fn classify_ident(&self, name: &str) -> Option<u32> {
        // self / 字面量关键字交回 TextMate
        if matches!(name, "self" | "true" | "false" | "null" | "let") {
            return None;
        }
        if let Some(b) = self.scopes.iter().rev().find_map(|s| s.get(name)) {
            return Some(match b.kind {
                super::ItemKind::Function => 2,
                super::ItemKind::Parameter => 1,
                super::ItemKind::Struct => 5,
                super::ItemKind::Interface => 6,
                _ => 0, // variable
            });
        }
        match name {
            "EMPTY" | "inf" | "nan" => None, // TextMate 已高亮
            "fs" | "sys" => Some(8),         // namespace
            _ if builtins::global_fn(name).is_some() => Some(2),
            _ if builtins::is_builtin_class(name) => Some(7),
            _ => None,
        }
    }

    /// 成员名分类：按接收者推导（method / property / namespace 函数）
    fn classify_member(&self, target: &Expr, name: &str) -> Option<u32> {
        let ctx = Ctx { scopes: &self.scopes, structs: self.structs };
        let recv = infer::infer(target, &ctx);
        match &recv {
            Ty::Namespace(_) => Some(2), // fs.read_file / sys.shell → function
            Ty::Struct(s) => {
                let info = self.structs.get(s)?;
                if info.methods.iter().any(|m| m.name == *name) {
                    Some(3)
                } else if info.fields.iter().any(|f| f.name == *name) {
                    Some(4)
                } else {
                    None
                }
            }
            Ty::Object(fields) => fields
                .iter()
                .find(|f| f.name == *name)
                .map(|f| if f.is_fn { 3 } else { 4 }),
            // 内置容器类型的方法；字段访问（不太可能）不发
            t if builtins::methods_for(t).is_some() => {
                if builtins::methods_for(t)?.iter().any(|m| *m.name == *name) {
                    Some(3)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    // ---- token 游标（自愈式：span 同步 + 有界前搜，短暂失配自动追上） ----

    /// 前进到首个 span ≥ 给定位置的 token（AST 访问与 token 序一致，只前进）
    fn sync_to(&mut self, span: Span) {
        while self.tok_pos < self.tokens.len() && self.tokens[self.tok_pos].span < span {
            self.tok_pos += 1;
        }
    }

    /// 游标附近找 `.` / `?.` + 成员名：返回名字位置并前进；
    /// 窗口内找不到返回 None（放弃该 token，不错色）
    fn member_name_span(&mut self, name: &str) -> Option<Span> {
        // 有界前搜第一个点号（允许中途隔着 token——游标可能落后）
        let mut j = self.tok_pos;
        let bound = (self.tok_pos + 16).min(self.tokens.len());
        while j < bound {
            match &self.tokens[j].kind {
                TokenKind::Dot | TokenKind::QuestionDot => break,
                TokenKind::Eof => return None,
                _ => j += 1,
            }
        }
        if j >= bound {
            return None;
        }
        let nt = self.tokens.get(j + 1)?;
        let (span, matched) = match &nt.kind {
            TokenKind::Ident(n) => (nt.span, *n == name),
            _ => return None,
        };
        self.tok_pos = j + 2;
        matched.then_some(span)
    }

    /// 跳到对象字面量的 `{` 之后（游标可能停在 `=` 等位置）
    fn skip_open(&mut self) {
        let bound = (self.tok_pos + 8).min(self.tokens.len());
        while self.tok_pos < bound {
            match self.tokens[self.tok_pos].kind {
                TokenKind::LBrace => {
                    self.tok_pos += 1;
                    return;
                }
                TokenKind::Eof => return,
                _ => self.tok_pos += 1,
            }
        }
    }

    fn skip_close(&mut self) {
        let bound = (self.tok_pos + 8).min(self.tokens.len());
        while self.tok_pos < bound {
            match self.tokens[self.tok_pos].kind {
                TokenKind::RBrace => {
                    self.tok_pos += 1;
                    return;
                }
                TokenKind::Eof => return,
                _ => self.tok_pos += 1,
            }
        }
    }

    /// 对象字面量键：窗口内找 Ident(键名) + `:`
    fn object_key_span(&mut self, key: &str) -> Option<Span> {
        let mut j = self.tok_pos;
        let bound = (self.tok_pos + 8).min(self.tokens.len());
        while j < bound {
            match &self.tokens[j].kind {
                TokenKind::Ident(n) if n == key => {
                    let nt = self.tokens.get(j + 1)?;
                    if let TokenKind::Colon = nt.kind {
                        let span = self.tokens[j].span;
                        self.tok_pos = j + 2;
                        return Some(span);
                    }
                    return None;
                }
                TokenKind::Str(_) => return None, // 字符串键：不强行着色
                TokenKind::Eof => return None,
                _ => j += 1,
            }
        }
        None
    }
}

/// 循环变量名字位置：`for x` → 语句起点 + 4（len("for ")）
fn loop_var_span(stmt: &Stmt) -> Span {
    let s = stmt.span();
    Span::new(s.line, s.col + 4)
}

// ============ 跳转定义 ============

/// 跳转定义主入口：光标（LSP 位置）处的词 → 声明处位置。
/// 内置符号无源码位置，返回 None。
pub fn definition(src: &str, line0: usize, char_utf16: usize) -> Option<DefLoc> {
    let map = SourceMap::new(src);
    let cursor = map.to_span(line0, char_utf16);
    let word = super::word_at(&map, line0, char_utf16)?;
    if word.is_empty() || matches!(word.as_str(), "true" | "false" | "null" | "let") {
        return None;
    }

    // 全文档解析（词法/解析失败时尽力截断），供名字扫描
    let (_program, tokens) = parse_full(src)?;
    let names = scan_names(&tokens);

    let (mode, ..) = super::tolerate::cursor_mode_and_insert(src, map.offset(cursor));
    let before = super::before_word(&map, line0, char_utf16);
    let is_member = mode == super::tolerate::CursorMode::Member && before.ends_with(['.', '?']);

    if is_member {
        // 接收者推导 → struct 实例查扫描索引（字段/方法声明位置）
        let patched = super::tolerate::parse_at_cursor(src, cursor)?;
        let ambient = super::tolerate::parse_ambient(src, cursor);
        let registry_src = ambient.as_ref().unwrap_or(&patched.program);
        let reg = scope::collect_registry(registry_src);
        let snap = scope::collect_at_cursor(&patched.program, patched.sentinel, ambient.as_ref(), &reg);
        let recv_expr = snap.receiver?;
        let ctx = Ctx { scopes: &snap.scopes, structs: &snap.structs };
        let recv = infer::infer(&recv_expr, &ctx);
        if let Ty::Struct(sname) = &recv {
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
        if let Ty::Object(_) = &recv {
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
                Ty::Struct(s) => Some(s.clone()),
                _ => snap.scopes.iter().rev().find_map(|s| s.get("self")).and_then(|b| match &b.ty {
                    Ty::Struct(s) => Some(s.clone()),
                    _ => None,
                }),
            }?;
            let n = names.iter().find(|n| n.role == NameRole::StructName && n.name == sname)?;
            return to_defloc(&map, n.span, &n.name);
        }
        return None;
    }

    // 全局/参数：作用域快照查绑定
    let patched = super::tolerate::parse_at_cursor(src, cursor);
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
    let ambient = super::tolerate::parse_ambient(src, cursor);
    let registry_src = ambient.as_ref().unwrap_or(&patched.program);
    let reg = scope::collect_registry(registry_src);
    let snap = scope::collect_at_cursor(&patched.program, patched.sentinel, ambient.as_ref(), &reg);

    if let Some(b) = snap.scopes.iter().rev().find_map(|s| s.get(word.as_str())) {
        // 参数没有 AST span：扫描索引按名字就近兜底
        if b.span != Span::default() {
            return to_defloc(&map, b.span, &word);
        }
        if let Ty::Struct(sname) = &b.ty {
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
        let span = if mode == super::tolerate::CursorMode::Global
            && b.kind == super::ItemKind::Variable
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
