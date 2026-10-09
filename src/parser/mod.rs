//! 递归下降 + 优先级爬升语法分析器。
//!
//! [`Parser`] 的 `impl` 块按职责分散在子模块：
//! - [`cursor`]：token 游标原语（peek/advance/expect…）与错误描述
//! - [`types`]：类型标注语法（联合 / 泛型 / 对象 / 函数类型）
//! - [`stmts`]：语句与声明（含 struct/impl/interface）
//! - [`exprs`]：表达式优先级爬升
//!
//! 私有字段对子模块可见；跨子模块调用的方法用 `pub(super)` 收窄暴露面。

mod cursor;
mod exprs;
mod stmts;
mod types;

#[cfg(test)]
mod tests;

use crate::ast::Stmt;
use crate::lexer::{Token, TokenKind};
use crate::{RtError, RtResult};

/// 递归下降 + 优先级爬升
pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Parser { tokens, pos: 0 }
    }

    /// 解析整个程序，返回顶层语句块（顶层即全局作用域，由解释器直接执行）
    pub fn parse_program(&mut self) -> RtResult<Stmt> {
        let span = self.cur_span();
        let mut stmts = Vec::new();
        self.skip_separators();
        while !self.at_end() {
            stmts.push(self.parse_stmt()?);
            if !matches!(
                self.peek_kind(),
                Some(TokenKind::Eol | TokenKind::Semi | TokenKind::Eof)
            ) {
                return Err(RtError::parse(
                    self.cur_span(),
                    format!(
                        "expected end of statement (newline or ';'), found {}",
                        self.describe_peek()
                    ),
                ));
            }
            self.skip_separators();
        }
        Ok(Stmt::Block { stmts, span })
    }
}
