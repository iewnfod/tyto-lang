//! token 游标原语与错误描述。
//!
//! 语法分析各子模块（语句/类型/表达式）都经由这些原语消费 token 流，
//! 错误消息统一用 `describe_peek` 描述当前 token。

use crate::lexer::{Keyword, Token, TokenKind};
use crate::{RtError, RtResult, Span};

use super::Parser;

impl Parser {
    /// impl 方法名：标识符，或关键字 `new`（构造方法约定名）
    pub(super) fn expect_method_name(&mut self) -> RtResult<String> {
        match self.peek_kind().cloned() {
            Some(TokenKind::Ident(name)) => {
                self.advance();
                Ok(name)
            }
            Some(TokenKind::Keyword(Keyword::New)) => {
                self.advance();
                Ok("new".to_string())
            }
            _ => Err(RtError::parse(
                self.cur_span(),
                format!("expected method name, found {}", self.describe_peek()),
            )),
        }
    }

    /// 成员名/字段名：标识符或关键字（允许 `arr.contains` 这类与关键字不冲突的名字，
    /// 以及极少数关键字字段名），字符串也可作为对象字面量的键
    pub(super) fn expect_member_name(&mut self) -> RtResult<String> {
        match self.peek_kind().cloned() {
            Some(TokenKind::Ident(name)) => {
                self.advance();
                Ok(name)
            }
            Some(TokenKind::Keyword(k)) => {
                self.advance();
                Ok(k.as_str().to_string())
            }
            Some(TokenKind::Str(s)) => {
                self.advance();
                Ok(s)
            }
            _ => Err(RtError::parse(
                self.cur_span(),
                format!("expected field name, found {}", self.describe_peek()),
            )),
        }
    }

    pub(super) fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    pub(super) fn peek_kind(&self) -> Option<&TokenKind> {
        self.peek().map(|t| &t.kind)
    }

    pub(super) fn cur_span(&self) -> Span {
        self.peek().map(|t| t.span).unwrap_or_default()
    }

    pub(super) fn at_end(&self) -> bool {
        matches!(self.peek_kind(), Some(TokenKind::Eof) | None)
    }

    pub(super) fn advance(&mut self) -> Token {
        let tok = self.peek().cloned().unwrap_or(Token {
            kind: TokenKind::Eof,
            span: Span::default(),
        });
        if self.pos < self.tokens.len() {
            self.pos += 1;
        }
        tok
    }

    pub(super) fn check(&self, kind: &TokenKind) -> bool {
        self.peek_kind() == Some(kind)
    }

    pub(super) fn expect(&mut self, kind: TokenKind, msg: &str) -> RtResult<Token> {
        if self.check(&kind) {
            Ok(self.advance())
        } else {
            Err(RtError::parse(
                self.cur_span(),
                format!("{} , found {}", msg, self.describe_peek()),
            ))
        }
    }

    pub(super) fn expect_ident(&mut self, msg: &str) -> RtResult<String> {
        match self.peek_kind().cloned() {
            Some(TokenKind::Ident(name)) => {
                self.advance();
                Ok(name)
            }
            _ => Err(RtError::parse(
                self.cur_span(),
                format!("{}, found {}", msg, self.describe_peek()),
            )),
        }
    }

    /// 跳过语句分隔符（换行与分号）
    pub(super) fn skip_separators(&mut self) {
        while matches!(self.peek_kind(), Some(TokenKind::Eol | TokenKind::Semi)) {
            self.pos += 1;
        }
    }

    /// 仅跳过换行（块/参数前的宽松处理）
    pub(super) fn skip_eol(&mut self) {
        while matches!(self.peek_kind(), Some(TokenKind::Eol)) {
            self.pos += 1;
        }
    }

    /// 当前 token 是否可能开始一个表达式
    pub(super) fn expr_can_start(&self) -> bool {
        matches!(
            self.peek_kind(),
            Some(
                TokenKind::Num(_)
                    | TokenKind::Str(_)
                    | TokenKind::Ident(_)
                    | TokenKind::LParen
                    | TokenKind::LBracket
                    | TokenKind::LBrace
                    | TokenKind::Minus
                    | TokenKind::Not
                    | TokenKind::Keyword(
                        Keyword::True | Keyword::False | Keyword::Null | Keyword::New | Keyword::Function
                    )
            )
        )
    }

    pub(super) fn describe_peek(&self) -> String {
        match self.peek_kind() {
            Some(TokenKind::Num(n)) => format!("number {}", n),
            Some(TokenKind::Str(s)) => format!("string {:?}", s),
            Some(TokenKind::Ident(s)) => format!("identifier '{}'", s),
            Some(TokenKind::Keyword(k)) => format!("keyword '{}'", k.as_str()),
            Some(TokenKind::Eol) => "end of line".to_string(),
            Some(TokenKind::Eof) | None => "end of input".to_string(),
            Some(other) => format!("'{}'", token_text(other)),
        }
    }
}

fn token_text(kind: &TokenKind) -> &'static str {
    use TokenKind::*;
    match kind {
        Assign => "=",
        AddAssign => "+=",
        SubAssign => "-=",
        MulAssign => "*=",
        DivAssign => "/=",
        ModAssign => "%=",
        Plus => "+",
        Minus => "-",
        Star => "*",
        Slash => "/",
        Percent => "%",
        Lt => "<",
        Gt => ">",
        Lte => "<=",
        Gte => ">=",
        Eq => "==",
        Neq => "!=",
        AndAnd => "&&",
        OrOr => "||",
        Pipe => "|",
        Not => "!",
        Arrow => "->",
        Dot => ".",
        DotDot => "..",
        DotDotEq => "..=",
        Question => "?",
        QuestionDot => "?.",
        QuestionQuestion => "??",
        QuestionQuestionAssign => "??=",
        Colon => ":",
        Comma => ",",
        Semi => ";",
        LParen => "(",
        RParen => ")",
        LBracket => "[",
        RBracket => "]",
        LBrace => "{",
        RBrace => "}",
        Num(_) | Str(_) | Ident(_) | Keyword(_) | Eol | Eof => "?",
    }
}
