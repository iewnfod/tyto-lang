//! 表达式（优先级爬升）：从三元到后缀/主表达式。

use std::rc::Rc;

use crate::ast::*;
use crate::lexer::{Keyword, TokenKind};
use crate::{RtError, RtResult};

use super::Parser;

impl Parser {
    // ============ 表达式（优先级爬升） ============

    pub fn parse_expr(&mut self) -> RtResult<Expr> {
        self.parse_ternary()
    }

    fn parse_ternary(&mut self) -> RtResult<Expr> {
        let cond = self.parse_nullish()?;
        if self.check(&TokenKind::Question) {
            self.advance();
            let then_expr = self.parse_expr()?;
            self.expect(TokenKind::Colon, "expected ':' in ternary expression")?;
            let else_expr = self.parse_ternary()?; // 右结合
            let span = cond.span();
            return Ok(Expr::Ternary {
                cond: Box::new(cond),
                then_expr: Box::new(then_expr),
                else_expr: Box::new(else_expr),
                span,
            });
        }
        Ok(cond)
    }

    /// `??`：优先级介于三元与 `||` 之间（JS 同位），左结合
    fn parse_nullish(&mut self) -> RtResult<Expr> {
        let mut left = self.parse_logic_or()?;
        while self.check(&TokenKind::QuestionQuestion) {
            self.advance();
            let right = self.parse_logic_or()?;
            let span = left.span();
            left = Expr::Logic { left: Box::new(left), op: LogicOp::Nullish, right: Box::new(right), span };
        }
        Ok(left)
    }

    fn parse_logic_or(&mut self) -> RtResult<Expr> {
        let mut left = self.parse_logic_and()?;
        while self.check(&TokenKind::OrOr) {
            self.advance();
            let right = self.parse_logic_and()?;
            let span = left.span();
            left = Expr::Logic { left: Box::new(left), op: LogicOp::Or, right: Box::new(right), span };
        }
        Ok(left)
    }

    fn parse_logic_and(&mut self) -> RtResult<Expr> {
        let mut left = self.parse_equality()?;
        while self.check(&TokenKind::AndAnd) {
            self.advance();
            let right = self.parse_equality()?;
            let span = left.span();
            left = Expr::Logic { left: Box::new(left), op: LogicOp::And, right: Box::new(right), span };
        }
        Ok(left)
    }

    fn parse_equality(&mut self) -> RtResult<Expr> {
        let mut left = self.parse_comparison()?;
        loop {
            match self.peek_kind() {
                Some(TokenKind::Eq) | Some(TokenKind::Neq) => {
                    let op = match self.advance().kind {
                        TokenKind::Eq => BinaryOp::Eq,
                        _ => BinaryOp::Neq,
                    };
                    let right = self.parse_comparison()?;
                    let span = left.span();
                    left = Expr::Binary { left: Box::new(left), op, right: Box::new(right), span };
                }
                // `p is T`：与 == 同级、左结合；右侧是类型名表达式（标识符/成员链）
                Some(TokenKind::Keyword(Keyword::Is)) => {
                    self.advance();
                    let target = self.parse_unary()?;
                    let span = left.span();
                    left = Expr::Is {
                        operand: Box::new(left),
                        target: Box::new(target),
                        span,
                    };
                }
                _ => return Ok(left),
            }
        }
    }

    fn parse_comparison(&mut self) -> RtResult<Expr> {
        let mut left = self.parse_additive()?;
        loop {
            let op = match self.peek_kind() {
                Some(TokenKind::Lt) => BinaryOp::Lt,
                Some(TokenKind::Gt) => BinaryOp::Gt,
                Some(TokenKind::Lte) => BinaryOp::Lte,
                Some(TokenKind::Gte) => BinaryOp::Gte,
                _ => return Ok(left),
            };
            self.advance();
            let right = self.parse_additive()?;
            let span = left.span();
            left = Expr::Binary { left: Box::new(left), op, right: Box::new(right), span };
        }
    }

    fn parse_additive(&mut self) -> RtResult<Expr> {
        let mut left = self.parse_mul()?;
        loop {
            let op = match self.peek_kind() {
                Some(TokenKind::Plus) => BinaryOp::Add,
                Some(TokenKind::Minus) => BinaryOp::Sub,
                _ => return Ok(left),
            };
            self.advance();
            let right = self.parse_mul()?;
            let span = left.span();
            left = Expr::Binary { left: Box::new(left), op, right: Box::new(right), span };
        }
    }

    fn parse_mul(&mut self) -> RtResult<Expr> {
        let mut left = self.parse_unary()?;
        loop {
            let op = match self.peek_kind() {
                Some(TokenKind::Star) => BinaryOp::Mul,
                Some(TokenKind::Slash) => BinaryOp::Div,
                Some(TokenKind::Percent) => BinaryOp::Mod,
                _ => return Ok(left),
            };
            self.advance();
            let right = self.parse_unary()?;
            let span = left.span();
            left = Expr::Binary { left: Box::new(left), op, right: Box::new(right), span };
        }
    }

    fn parse_unary(&mut self) -> RtResult<Expr> {
        let span = self.cur_span();
        match self.peek_kind() {
            Some(TokenKind::Minus) => {
                self.advance();
                let operand = self.parse_unary()?;
                Ok(Expr::Unary { op: UnaryOp::Neg, operand: Box::new(operand), span })
            }
            Some(TokenKind::Not) => {
                self.advance();
                let operand = self.parse_unary()?;
                Ok(Expr::Unary { op: UnaryOp::Not, operand: Box::new(operand), span })
            }
            _ => self.parse_postfix(),
        }
    }

    fn parse_postfix(&mut self) -> RtResult<Expr> {
        let mut expr = self.parse_primary()?;
        loop {
            match self.peek_kind() {
                Some(TokenKind::LParen) => {
                    self.advance();
                    let args = self.parse_call_args()?;
                    let span = expr.span();
                    expr = Expr::Call { callee: Box::new(expr), args, span };
                }
                Some(TokenKind::LBracket) => {
                    self.advance();
                    // 切片语法：[..e] [s..] [s..e]（含 ..= 变体，端点均可省略）
                    let slice = if matches!(self.peek_kind(), Some(TokenKind::DotDot | TokenKind::DotDotEq)) {
                        // 无起点：[..e] 或 [..]
                        let inclusive = matches!(self.advance().kind, TokenKind::DotDotEq);
                        let end = if self.check(&TokenKind::RBracket) {
                            None
                        } else {
                            Some(self.parse_expr()?)
                        };
                        (None, end, inclusive)
                    } else {
                        let first = self.parse_expr()?;
                        match self.peek_kind() {
                            Some(TokenKind::DotDot) | Some(TokenKind::DotDotEq) => {
                                // 有起点：[s..e] / [s..=e] / [s..]
                                let inclusive = matches!(self.advance().kind, TokenKind::DotDotEq);
                                let end = if self.check(&TokenKind::RBracket) {
                                    None
                                } else {
                                    Some(self.parse_expr()?)
                                };
                                (Some(first), end, inclusive)
                            }
                            _ => {
                                self.expect(TokenKind::RBracket, "expected ']' after index")?;
                                let span = expr.span();
                                expr = Expr::Index { target: Box::new(expr), index: Box::new(first), span };
                                continue;
                            }
                        }
                    };
                    self.expect(TokenKind::RBracket, "expected ']' after slice")?;
                    let span = expr.span();
                    expr = Expr::Slice {
                        target: Box::new(expr),
                        start: slice.0.map(Box::new),
                        end: slice.1.map(Box::new),
                        inclusive: slice.2,
                        span,
                    };
                }
                Some(TokenKind::Dot) => {
                    self.advance();
                    let name = self.expect_member_name()?;
                    let span = expr.span();
                    expr = Expr::Member { target: Box::new(expr), name, span };
                }
                Some(TokenKind::QuestionDot) => {
                    self.advance();
                    let name = self.expect_member_name()?;
                    let span = expr.span();
                    expr = Expr::OptionalMember { target: Box::new(expr), name, span };
                }
                _ => return Ok(expr),
            }
        }
    }

    fn parse_primary(&mut self) -> RtResult<Expr> {
        let span = self.cur_span();
        match self.peek_kind().cloned() {
            Some(TokenKind::Num(n)) => {
                self.advance();
                Ok(Expr::Num(n, span))
            }
            Some(TokenKind::Str(s)) => {
                self.advance();
                Ok(Expr::Str(s, span))
            }
            Some(TokenKind::Ident(name)) => {
                self.advance();
                Ok(Expr::Ident(name, span))
            }
            Some(TokenKind::Keyword(Keyword::True)) => {
                self.advance();
                Ok(Expr::Bool(true, span))
            }
            Some(TokenKind::Keyword(Keyword::False)) => {
                self.advance();
                Ok(Expr::Bool(false, span))
            }
            Some(TokenKind::Keyword(Keyword::Null)) => {
                self.advance();
                Ok(Expr::Null(span))
            }
            Some(TokenKind::Keyword(Keyword::New)) => self.parse_new(),
            Some(TokenKind::Keyword(Keyword::Function)) => {
                self.advance();
                let type_params = self.parse_type_params()?;
                self.expect(TokenKind::LParen, "expected '(' after 'function'")?;
                let params = self.parse_params()?;
                let ret = self.parse_ret_ann()?;
                let body = Rc::new(self.parse_block()?);
                Ok(Expr::Function { params, type_params, ret, body, span })
            }
            Some(TokenKind::LParen) => {
                self.advance();
                let expr = self.parse_expr()?;
                self.expect(TokenKind::RParen, "expected ')' after expression")?;
                Ok(expr)
            }
            Some(TokenKind::LBracket) => {
                self.advance();
                let mut elems = Vec::new();
                self.skip_eol();
                if !self.check(&TokenKind::RBracket) {
                    loop {
                        elems.push(self.parse_expr()?);
                        if self.check(&TokenKind::Comma) {
                            self.advance();
                            self.skip_eol();
                            if self.check(&TokenKind::RBracket) {
                                break; // 尾逗号
                            }
                        } else {
                            break;
                        }
                    }
                }
                self.skip_eol();
                self.expect(TokenKind::RBracket, "expected ']' after array elements")?;
                Ok(Expr::Array(elems, span))
            }
            Some(TokenKind::LBrace) => {
                self.advance();
                let mut fields = Vec::new();
                self.skip_eol();
                if !self.check(&TokenKind::RBrace) {
                    loop {
                        let key = self.expect_member_name()?;
                        self.skip_eol();
                        self.expect(TokenKind::Colon, "expected ':' after object field name")?;
                        let value = self.parse_expr()?;
                        fields.push((key, value));
                        if self.check(&TokenKind::Comma) {
                            self.advance();
                            self.skip_eol();
                            if self.check(&TokenKind::RBrace) {
                                break; // 尾逗号
                            }
                        } else {
                            break;
                        }
                    }
                }
                self.skip_eol();
                self.expect(TokenKind::RBrace, "expected '}' after object fields")?;
                Ok(Expr::Object(fields, span))
            }
            _ => Err(RtError::parse(
                span,
                format!("unexpected {}, expected expression", self.describe_peek()),
            )),
        }
    }

    /// `new MaxHeap(args)`：类名（可带成员链）+ 可选构造参数
    fn parse_new(&mut self) -> RtResult<Expr> {
        let span = self.cur_span();
        self.advance(); // new
        let mut class = self.parse_primary()?;
        while self.check(&TokenKind::Dot) {
            self.advance();
            let name = self.expect_member_name()?;
            let cspan = class.span();
            class = Expr::Member { target: Box::new(class), name, span: cspan };
        }
        let mut args = Vec::new();
        if self.check(&TokenKind::LParen) {
            self.advance();
            args = self.parse_call_args()?;
        }
        Ok(Expr::New { class: Box::new(class), args, span })
    }

    fn parse_call_args(&mut self) -> RtResult<Vec<Expr>> {
        // 进入时 '(' 已被消耗
        let mut args = Vec::new();
        if self.check(&TokenKind::RParen) {
            self.advance();
            return Ok(args);
        }
        loop {
            args.push(self.parse_expr()?);
            if self.check(&TokenKind::Comma) {
                self.advance();
                if self.check(&TokenKind::RParen) {
                    break; // 尾逗号
                }
            } else {
                break;
            }
        }
        self.expect(TokenKind::RParen, "expected ')' after arguments")?;
        Ok(args)
    }
}
