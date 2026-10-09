use super::token::{operator_table, Keyword, Token, TokenKind};
use crate::{RtError, RtResult, Span};

/// 词法输出：token 流 + 到达 EOF 时仍未闭合的括号深度（REPL 续行判断用）
pub struct LexOutput {
    pub tokens: Vec<Token>,
    pub unclosed_depth: usize,
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
