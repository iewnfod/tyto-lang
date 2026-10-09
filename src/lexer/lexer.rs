use super::token::{Keyword, Token, TokenKind};
use crate::{RtError, RtResult, Span};

/// 词法输出：token 流 + 到达 EOF 时仍未闭合的括号深度（REPL 续行判断用）
pub struct LexOutput {
    pub tokens: Vec<Token>,
    pub unclosed_depth: usize,
}

/// 运算符表：按长度降序排列，保证最长匹配（`..=` 在 `..` 之前，`?.` 在 `?` 之前）
fn operator_table() -> &'static [(&'static str, TokenKind)] {
    use TokenKind::*;
    &[
        ("..=", DotDotEq),
        ("??=", QuestionQuestionAssign),
        ("?.", QuestionDot),
        ("??", QuestionQuestion),
        ("<=", Lte),
        (">=", Gte),
        ("==", Eq),
        ("!=", Neq),
        ("&&", AndAnd),
        ("||", OrOr),
        ("+=", AddAssign),
        ("-=", SubAssign),
        ("*=", MulAssign),
        ("/=", DivAssign),
        ("%=", ModAssign),
        ("..", DotDot),
        ("->", Arrow),
        ("+", Plus),
        ("-", Minus),
        ("*", Star),
        ("/", Slash),
        ("%", Percent),
        ("<", Lt),
        (">", Gt),
        ("=", Assign),
        ("!", Not),
        ("|", Pipe),
        (".", Dot),
        ("?", Question),
        (":", Colon),
        (",", Comma),
        (";", Semi),
        ("(", LParen),
        (")", RParen),
        ("[", LBracket),
        ("]", RBracket),
        ("{", LBrace),
        ("}", RBrace),
    ]
}

pub struct Lexer {
    input: Vec<char>,
    pos: usize,
    line: usize,
    col: usize,
    /// `(` `[` `{` 的嵌套深度：内部换行不产生 Eol
    depth: usize,
    /// 上一个发出的 token（EOL 抑制与 `.5` 判断用）
    last: Option<TokenKind>,
}

impl Lexer {
    pub fn new(src: &str) -> Self {
        Lexer {
            input: src.chars().collect(),
            pos: 0,
            line: 1,
            col: 1,
            depth: 0,
            last: None,
        }
    }

    pub fn tokenize(mut self) -> RtResult<LexOutput> {
        let mut tokens: Vec<Token> = Vec::new();
        while self.pos < self.input.len() {
            if let Some(tok) = self.next_token()? {
                match &tok.kind {
                    // 仅 ( 和 [ 计入续行深度：块内换行是语句分隔符，不能抑制
                    TokenKind::LParen | TokenKind::LBracket => self.depth += 1,
                    TokenKind::RParen | TokenKind::RBracket => {
                        self.depth = self.depth.saturating_sub(1)
                    }
                    _ => {}
                }
                self.last = Some(tok.kind.clone());
                tokens.push(tok);
            }
        }
        tokens.push(Token { kind: TokenKind::Eof, span: self.span() });
        Ok(LexOutput { tokens, unclosed_depth: self.depth })
    }

    /// 识别下一个 token；返回 None 表示跳过了空白/注释/被抑制的换行
    fn next_token(&mut self) -> RtResult<Option<Token>> {
        // 空白、注释、换行（可能被抑制）
        loop {
            match self.peek() {
                Some(' ') | Some('\t') | Some('\r') => {
                    self.advance();
                }
                Some('\n') => {
                    let span = self.span();
                    if self.suppress_eol() {
                        self.advance();
                        continue;
                    }
                    self.advance();
                    return Ok(Some(Token { kind: TokenKind::Eol, span }));
                }
                Some('/') if self.peek_next() == Some('/') => {
                    while let Some(c) = self.peek() {
                        if c == '\n' {
                            break;
                        }
                        self.advance();
                    }
                }
                Some('/') if self.peek_next() == Some('*') => {
                    self.skip_block_comment()?;
                }
                _ => break,
            }
        }

        let Some(c) = self.peek() else {
            return Ok(None);
        };

        let span = self.span();
        let kind = if c.is_ascii_digit() {
            self.lex_number()?
        } else if c.is_alphabetic() || c == '_' {
            self.lex_ident()
        } else if c == '"' || c == '\'' {
            self.lex_string()?
        } else if c == '.'
            && self.peek_next().map(|n| n.is_ascii_digit()).unwrap_or(false)
            && !self.last_kind().map(|k| k.can_end_expr()).unwrap_or(false)
        {
            // `.5` 形式的小数：仅当 `.` 不可能是成员访问时
            self.lex_number()?
        } else {
            self.lex_operator()?
        };
        Ok(Some(Token { kind, span }))
    }

    fn lex_number(&mut self) -> RtResult<TokenKind> {
        let start = self.pos;
        while self.peek().map(|c| c.is_ascii_digit()).unwrap_or(false) {
            self.advance();
        }
        // 小数部分：必须 `.` 后跟数字（这样 `0..n` 不会被吃掉一个点）
        if self.peek() == Some('.')
            && self.peek_next().map(|c| c.is_ascii_digit()).unwrap_or(false)
        {
            self.advance();
            while self.peek().map(|c| c.is_ascii_digit()).unwrap_or(false) {
                self.advance();
            }
        }
        // 指数部分
        if matches!(self.peek(), Some('e') | Some('E')) {
            self.advance();
            if matches!(self.peek(), Some('+') | Some('-')) {
                self.advance();
            }
            if !self.peek().map(|c| c.is_ascii_digit()).unwrap_or(false) {
                return Err(RtError::lex(self.span(), "invalid number literal: exponent has no digits"));
            }
            while self.peek().map(|c| c.is_ascii_digit()).unwrap_or(false) {
                self.advance();
            }
        }
        let s: String = self.input[start..self.pos].iter().collect();
        match s.parse::<f64>() {
            Ok(n) => Ok(TokenKind::Num(n)),
            Err(_) => Err(RtError::lex(self.span(), format!("invalid number literal: {}", s))),
        }
    }

    fn lex_ident(&mut self) -> TokenKind {
        let start = self.pos;
        while self.peek().map(|c| c.is_alphanumeric() || c == '_').unwrap_or(false) {
            self.advance();
        }
        let s: String = self.input[start..self.pos].iter().collect();
        match Keyword::from_str(&s) {
            Some(k) => TokenKind::Keyword(k),
            None => TokenKind::Ident(s),
        }
    }

    fn lex_string(&mut self) -> RtResult<TokenKind> {
        let quote = self.peek().unwrap();
        self.advance(); // 开头引号
        let mut s = String::new();
        loop {
            match self.peek() {
                None | Some('\n') => {
                    return Err(RtError::lex(
                        self.span(),
                        "unterminated string literal (escape newlines as \\n)",
                    ))
                }
                Some(c) if c == quote => {
                    self.advance();
                    return Ok(TokenKind::Str(s));
                }
                Some('\\') => {
                    self.advance();
                    let esc = match self.peek() {
                        Some(c) => c,
                        None => {
                            return Err(RtError::lex(self.span(), "unterminated escape sequence"))
                        }
                    };
                    self.advance();
                    let ch = match esc {
                        'n' => '\n',
                        't' => '\t',
                        'r' => '\r',
                        '\\' => '\\',
                        '"' => '"',
                        '\'' => '\'',
                        '0' => '\0',
                        _ => {
                            return Err(RtError::lex(
                                self.span(),
                                format!("invalid escape character: \\{}", esc),
                            ))
                        }
                    };
                    s.push(ch);
                }
                Some(c) => {
                    self.advance();
                    s.push(c);
                }
            }
        }
    }

    fn lex_operator(&mut self) -> RtResult<TokenKind> {
        for (text, kind) in operator_table() {
            if self.starts_with(text) {
                self.advance_n(text.chars().count());
                return Ok(kind.clone());
            }
        }
        let bad = self.peek().unwrap_or('?');
        Err(RtError::lex(
            self.span(),
            format!("illegal character: '{}'", bad),
        ))
    }

    fn skip_block_comment(&mut self) -> RtResult<()> {
        self.advance(); // '/'
        self.advance(); // '*'
        loop {
            match self.peek() {
                None => {
                    return Err(RtError::lex(self.span(), "unterminated block comment"));
                }
                Some('*') if self.peek_next() == Some('/') => {
                    self.advance();
                    self.advance();
                    return Ok(());
                }
                Some(_) => self.advance(),
            }
        }
    }

    /// EOL 抑制规则：
    /// 1. 括号（`(` `[` `{`）内部换行；
    /// 2. 上一个 token 不可能结束语句（运算符/逗号/赋值号等之后）；
    /// 3. 下一行以 `.` 或 `?.` 开头（方法链换行续接）。
    fn suppress_eol(&self) -> bool {
        if self.depth > 0 {
            return true;
        }
        let ends_stmt = match self.last_kind() {
            None => false,
            Some(k) => {
                k.can_end_expr()
                    || matches!(
                        k,
                        TokenKind::Keyword(Keyword::Return)
                            | TokenKind::Keyword(Keyword::Break)
                            | TokenKind::Keyword(Keyword::Continue)
                    )
            }
        };
        !ends_stmt || self.next_nonspace_is_dot()
    }

    /// 从当前位置的下一个字符起，跳过空格后是否是 `.`（用于方法链换行）
    fn next_nonspace_is_dot(&self) -> bool {
        let mut i = self.pos + 1;
        while let Some(&c) = self.input.get(i) {
            match c {
                ' ' | '\t' | '\r' => i += 1,
                '.' => return true,
                _ => return false,
            }
        }
        false
    }

    fn last_kind(&self) -> Option<&TokenKind> {
        self.last.as_ref()
    }

    fn starts_with(&self, s: &str) -> bool {
        let chars: Vec<char> = s.chars().collect();
        if self.pos + chars.len() > self.input.len() {
            return false;
        }
        self.input[self.pos..self.pos + chars.len()]
            .iter()
            .zip(chars)
            .all(|(a, b)| *a == b)
    }

    fn advance_n(&mut self, n: usize) {
        for _ in 0..n {
            self.advance();
        }
    }

    fn peek(&self) -> Option<char> {
        self.input.get(self.pos).copied()
    }

    fn peek_next(&self) -> Option<char> {
        self.input.get(self.pos + 1).copied()
    }

    fn advance(&mut self) {
        if let Some(c) = self.peek() {
            self.pos += 1;
            if c == '\n' {
                self.line += 1;
                self.col = 1;
            } else {
                self.col += 1;
            }
        }
    }

    fn span(&self) -> Span {
        Span { line: self.line, col: self.col }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::token::Keyword;
    use TokenKind::*;

    fn kinds(src: &str) -> RtResult<Vec<TokenKind>> {
        Ok(Lexer::new(src).tokenize()?.tokens.into_iter().map(|t| t.kind).collect())
    }

    #[test]
    fn simple_statements_with_eol() {
        let ks = kinds("x = 1\ny = 2").unwrap();
        // 结尾无换行时 EOF 直接作终止符，不再补 Eol
        assert_eq!(
            ks,
            vec![
                Ident("x".into()),
                Assign,
                Num(1.0),
                Eol,
                Ident("y".into()),
                Assign,
                Num(2.0),
                Eof
            ]
        );
    }

    #[test]
    fn multi_char_operators() {
        let ks = kinds("a <= b != c && d || !e").unwrap();
        assert_eq!(
            ks[..8],
            [
                Ident("a".into()),
                Lte,
                Ident("b".into()),
                Neq,
                Ident("c".into()),
                AndAnd,
                Ident("d".into()),
                OrOr
            ]
        );
    }

    #[test]
    fn range_tokens_not_eaten_by_number() {
        let ks = kinds("0..n").unwrap();
        assert_eq!(ks[..3], [Num(0.0), DotDot, Ident("n".into())]);

        let ks = kinds("0..=n").unwrap();
        assert_eq!(ks[..3], [Num(0.0), DotDotEq, Ident("n".into())]);
    }

    #[test]
    fn compound_assignments() {
        let ks = kinds("i += 1\nj -= 2\nk *= 3\nm /= 4\nn %= 5").unwrap();
        let ops: Vec<&TokenKind> = ks.iter().filter(|k| matches!(k, AddAssign | SubAssign | MulAssign | DivAssign | ModAssign)).collect();
        assert_eq!(ops, vec![&AddAssign, &SubAssign, &MulAssign, &DivAssign, &ModAssign]);
    }

    #[test]
    fn string_escapes_both_quotes() {
        let ks = kinds("\"a\\n\\\"b\\\"\"").unwrap();
        assert_eq!(ks[0], Str("a\n\"b\"".into()));

        let ks = kinds("'it\\'s'").unwrap();
        assert_eq!(ks[0], Str("it's".into()));
    }

    #[test]
    fn comments_are_skipped() {
        // 注释后的换行：文件尚未发出任何 token（last=None），Eol 被抑制
        let ks = kinds("// hi\nx = 1").unwrap();
        assert_eq!(ks[..3], [Ident("x".into()), Assign, Num(1.0)]);

        let ks = kinds("y = 2 /* multi\nline */ + 1").unwrap();
        assert_eq!(
            ks[..5],
            [Ident("y".into()), Assign, Num(2.0), Plus, Num(1.0)]
        );
    }

    #[test]
    fn eol_suppressed_inside_parens_and_brackets() {
        // 括号/方括号内换行不产生 Eol
        let ks = kinds("f(1,\n  2)").unwrap();
        assert!(!ks[..].contains(&Eol), "no Eol inside parens: {:?}", ks);

        let ks = kinds("[1,\n2]").unwrap();
        assert!(!ks[..].contains(&Eol));

        // 花括号（对象字面量）是语句块的世界：仅 `{` 后与逗号后的换行被抑制
        let ks = kinds("{\n  x: 1,\n  y: 2\n}").unwrap();
        let eols = ks.iter().filter(|k| **k == Eol).count();
        assert_eq!(eols, 1, "{:?}", ks); // 只在 `2` 之后（值结束时）发 Eol
    }

    #[test]
    fn eol_suppressed_after_operator() {
        let ks = kinds("a +\nb").unwrap();
        assert!(!ks[..].contains(&Eol), "{:?}", ks);

        let ks = kinds("x = a ||\nb").unwrap();
        assert!(!ks[..].contains(&Eol), "{:?}", ks);
    }

    #[test]
    fn eol_suppressed_before_leading_dot_chain() {
        let ks = kinds("arr\n  .map(f)\n  .len()").unwrap();
        assert!(!ks[..].contains(&Eol), "{:?}", ks);
        assert!(ks.contains(&Dot));
    }

    #[test]
    fn leading_dot_float() {
        let ks = kinds("x = .5").unwrap();
        assert_eq!(ks[..3], [Ident("x".into()), Assign, Num(0.5)]);

        // 但值后面的 `.` 仍是成员访问
        let ks = kinds("a.b").unwrap();
        assert_eq!(ks[..3], [Ident("a".into()), Dot, Ident("b".into())]);
    }

    #[test]
    fn optional_chaining_token() {
        let ks = kinds("p?.x").unwrap();
        assert_eq!(ks[..3], [Ident("p".into()), QuestionDot, Ident("x".into())]);
    }

    #[test]
    fn nullish_coalescing_tokens() {
        let ks = kinds("a ?? b").unwrap();
        assert_eq!(ks[..3], [Ident("a".into()), QuestionQuestion, Ident("b".into())]);

        // ??= 优先于 ?? 匹配（最长匹配）
        let ks = kinds("x ??= 1").unwrap();
        assert_eq!(ks[..3], [Ident("x".into()), QuestionQuestionAssign, Num(1.0)]);

        // 与 ?. / 三元互不干扰
        let ks = kinds("p?.x ?? q ??= r").unwrap();
        assert_eq!(ks[1], QuestionDot);
        assert_eq!(ks[3], QuestionQuestion);
        assert_eq!(ks[5], QuestionQuestionAssign);

        // 运算符后的换行被抑制（可跨行书写）
        let ks = kinds("a ??\nb").unwrap();
        assert!(!ks.contains(&Eol));
    }

    #[test]
    fn arrow_token() {
        // `->` 是独立 token（函数返回类型标注用）
        let ks = kinds("function f() -> number {}").unwrap();
        assert_eq!(ks[4], Arrow);
        assert_eq!(ks[5], Ident("number".into()));
        // 与减号 / 大于号不混淆
        let ks = kinds("a - b").unwrap();
        assert_eq!(ks[1], Minus);
        let ks = kinds("a >= b").unwrap();
        assert_eq!(ks[1], Gte);
        let ks = kinds("a > b").unwrap();
        assert_eq!(ks[1], Gt);
    }

    #[test]
    fn ternary_tokens() {
        let ks = kinds("a ? b : c").unwrap();
        assert_eq!(ks[1], Question);
        assert_eq!(ks[3], Colon);
    }

    #[test]
    fn pipe_token() {
        // `|` 单独成 token（类型联合用）；`||` 仍是逻辑或（最长匹配优先）
        let ks = kinds("a | b").unwrap();
        assert_eq!(ks[..3], [Ident("a".into()), Pipe, Ident("b".into())]);
        let ks = kinds("a || b").unwrap();
        assert_eq!(ks[..3], [Ident("a".into()), OrOr, Ident("b".into())]);
        let ks = kinds("a |=").unwrap();
        assert_eq!(ks[1], Pipe); // 不存在 |=，`=` 独立
        assert_eq!(ks[2], Assign);
    }

    #[test]
    fn let_const_keywords() {
        let ks = kinds("let x = 1").unwrap();
        assert_eq!(ks[0], Keyword(Keyword::Let));
        let ks = kinds("const y = 2").unwrap();
        assert_eq!(ks[0], Keyword(Keyword::Const));
        // 不再是普通标识符
        let ks = kinds("let").unwrap();
        assert_eq!(ks[0], Keyword(Keyword::Let));
    }

    #[test]
    fn keywords_vs_identifiers() {
        let ks = kinds("function foo() {}").unwrap();
        assert_eq!(ks[0], TokenKind::Keyword(Keyword::Function));
        assert_eq!(ks[1], Ident("foo".into()));

        let ks = kinds("functionx").unwrap();
        assert_eq!(ks[0], Ident("functionx".into()));
    }

    #[test]
    fn exponent_numbers() {
        let ks = kinds("1e3").unwrap();
        assert_eq!(ks[0], Num(1000.0));

        let ks = kinds("2.5e-2").unwrap();
        assert_eq!(ks[0], Num(0.025));
    }

    #[test]
    fn semicolon_is_separator() {
        let ks = kinds("x = 1; y = 2").unwrap();
        assert_eq!(ks[3], Semi);
    }

    #[test]
    fn error_cases() {
        assert!(kinds("\"abc").is_err()); // unterminated
        assert!(kinds("\"a\nb\"").is_err()); // 换行未转义
        assert!(kinds("#").is_err()); // illegal char
        assert!(kinds("\"a\\q\"").is_err()); // bad escape
        assert!(kinds("1e").is_err()); // exponent 无数字
        assert!(kinds("/* no end").is_err()); // unterminated block comment
    }

    #[test]
    fn unclosed_depth_reported_for_repl() {
        let out = Lexer::new("f(1, [2,").tokenize().unwrap();
        assert_eq!(out.unclosed_depth, 2);

        let out = Lexer::new("x = 1").tokenize().unwrap();
        assert_eq!(out.unclosed_depth, 0);
    }

    #[test]
    fn spans_track_lines() {
        let out = Lexer::new("x = 1\ny = 2").tokenize().unwrap();
        let y_tok = &out.tokens[4];
        assert_eq!(y_tok.span.line, 2);
        assert_eq!(y_tok.span.col, 1);
    }
}
