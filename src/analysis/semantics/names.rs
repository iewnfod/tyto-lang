//! 名字定位扫描（token 流单遍）。
//!
//! AST 里 `Member`/`FuncDecl` 等声明的 span 都是语句起点，函数名/参数/字段/
//! 成员名没有独立位置——在 token 流上用上下文（关键字前瞻 + 括号/大括号栈）
//! 把这些名字的位置精确补出来；着色与跳转定义共用这份索引。

use crate::lexer::{Keyword, Token, TokenKind};
use crate::Span;

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
    pub(super) fn token_type(&self) -> Option<u32> {
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
