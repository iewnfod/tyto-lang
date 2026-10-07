use std::rc::Rc;

use crate::ast::*;
use crate::lexer::{Keyword, Token, TokenKind};
use crate::{RtError, RtResult, Span};

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

    // ============ 语句 ============

    fn parse_stmt(&mut self) -> RtResult<Stmt> {
        let span = self.cur_span();
        match self.peek_kind().cloned() {
            Some(TokenKind::Keyword(Keyword::Function)) => self.parse_func_decl(span),
            Some(TokenKind::Keyword(Keyword::If)) => self.parse_if(span),
            Some(TokenKind::Keyword(Keyword::While)) => self.parse_while(span),
            Some(TokenKind::Keyword(Keyword::For)) => self.parse_for(span),
            Some(TokenKind::Keyword(Keyword::Return)) => {
                self.advance();
                let value = if self.expr_can_start() {
                    Some(self.parse_expr()?)
                } else {
                    None
                };
                Ok(Stmt::Return { value, span })
            }
            Some(TokenKind::Keyword(Keyword::Break)) => {
                self.advance();
                Ok(Stmt::Break(span))
            }
            Some(TokenKind::Keyword(Keyword::Continue)) => {
                self.advance();
                Ok(Stmt::Continue(span))
            }
            _ => self.parse_assign_or_expr(span),
        }
    }

    /// 表达式 / 赋值语句（`x = 1`、`a[0] += 1`、`p.x = f()`）
    fn parse_assign_or_expr(&mut self, span: Span) -> RtResult<Stmt> {
        let expr = self.parse_expr()?;
        let op = match self.peek_kind() {
            Some(TokenKind::Assign) => AssignOp::Set,
            Some(TokenKind::AddAssign) => AssignOp::Add,
            Some(TokenKind::SubAssign) => AssignOp::Sub,
            Some(TokenKind::MulAssign) => AssignOp::Mul,
            Some(TokenKind::DivAssign) => AssignOp::Div,
            Some(TokenKind::ModAssign) => AssignOp::Mod,
            _ => return Ok(Stmt::Expr(expr, span)),
        };
        self.advance();
        match &expr {
            Expr::Ident(..) | Expr::Index { .. } | Expr::Member { .. } => {}
            Expr::OptionalMember { span: s, .. } => {
                return Err(RtError::parse(*s, "cannot assign through optional chain `?.`"))
            }
            _ => {
                return Err(RtError::parse(
                    expr.span(),
                    "invalid assignment target",
                ))
            }
        }
        let value = self.parse_expr()?;
        Ok(Stmt::Assign { target: expr, op, value, span })
    }

    fn parse_func_decl(&mut self, span: Span) -> RtResult<Stmt> {
        self.advance(); // function
        let name = self.expect_ident("expected function name after 'function'")?;
        self.expect(TokenKind::LParen, "expected '(' after function name")?;
        let params = self.parse_params()?;
        self.skip_eol();
        let body = Rc::new(self.parse_block()?);
        Ok(Stmt::FuncDecl { name, params, body, span })
    }

    fn parse_params(&mut self) -> RtResult<Vec<String>> {
        let mut params = Vec::new();
        if self.check(&TokenKind::RParen) {
            self.advance();
            return Ok(params);
        }
        loop {
            params.push(self.expect_ident("expected parameter name")?);
            if self.check(&TokenKind::Comma) {
                self.advance();
                if self.check(&TokenKind::RParen) {
                    break; // 尾逗号
                }
            } else {
                break;
            }
        }
        self.expect(TokenKind::RParen, "expected ')' after parameters")?;
        Ok(params)
    }

    fn parse_if(&mut self, span: Span) -> RtResult<Stmt> {
        self.advance(); // if
        let cond = self.parse_expr()?;
        self.skip_eol();
        let then_block = Box::new(self.parse_block()?);

        let mut else_block = None;
        // 允许 else 换行：先试探，若不是 else 则回退（分隔符留给外层处理）
        let save = self.pos;
        self.skip_separators();
        if matches!(self.peek_kind(), Some(TokenKind::Keyword(Keyword::Else))) {
            self.advance();
            if matches!(self.peek_kind(), Some(TokenKind::Keyword(Keyword::If))) {
                let inner_span = self.cur_span();
                else_block = Some(Box::new(self.parse_if(inner_span)?));
            } else {
                self.skip_eol();
                else_block = Some(Box::new(self.parse_block()?));
            }
        } else {
            self.pos = save;
        }
        Ok(Stmt::If { cond, then_block, else_block, span })
    }

    fn parse_while(&mut self, span: Span) -> RtResult<Stmt> {
        self.advance(); // while
        let cond = self.parse_expr()?;
        self.skip_eol();
        let body = Box::new(self.parse_block()?);
        Ok(Stmt::While { cond, body, span })
    }

    fn parse_for(&mut self, span: Span) -> RtResult<Stmt> {
        self.advance(); // for
        let var = self.expect_ident("expected loop variable after 'for'")?;
        match self.peek_kind() {
            Some(TokenKind::Keyword(Keyword::In)) => {
                self.advance();
                let start = self.parse_expr()?;
                let iter = match self.peek_kind() {
                    Some(TokenKind::DotDot) => {
                        self.advance();
                        let end = self.parse_expr()?;
                        ForIter::Range { start, end, inclusive: false }
                    }
                    Some(TokenKind::DotDotEq) => {
                        self.advance();
                        let end = self.parse_expr()?;
                        ForIter::Range { start, end, inclusive: true }
                    }
                    _ => ForIter::Expr(start),
                };
                self.skip_eol();
                let body = Box::new(self.parse_block()?);
                Ok(Stmt::ForIn { var, iter, body, span })
            }
            Some(TokenKind::Assign) => {
                self.advance();
                let init = self.parse_expr()?;
                self.expect(TokenKind::Semi, "expected ';' in for head")?;
                self.skip_eol();
                let cond = if self.expr_can_start() {
                    Some(self.parse_expr()?)
                } else {
                    None
                };
                self.expect(TokenKind::Semi, "expected ';' in for head")?;
                self.skip_eol();
                let step = if self.expr_can_start() {
                    Some(Box::new(self.parse_step()?))
                } else {
                    None
                };
                self.skip_eol();
                let body = Box::new(self.parse_block()?);
                Ok(Stmt::ForC { var, init, cond, step, body, span })
            }
            _ => Err(RtError::parse(
                self.cur_span(),
                format!(
                    "expected 'in' or '=' after loop variable, found {}",
                    self.describe_peek()
                ),
            )),
        }
    }

    /// for 头的第三段：`i += 1` 这类赋值或普通表达式
    fn parse_step(&mut self) -> RtResult<Stmt> {
        let span = self.cur_span();
        self.parse_assign_or_expr(span)
    }

    fn parse_block(&mut self) -> RtResult<Stmt> {
        let span = self.cur_span();
        self.expect(TokenKind::LBrace, "expected '{'")?;
        let mut stmts = Vec::new();
        self.skip_separators();
        loop {
            match self.peek_kind() {
                Some(TokenKind::RBrace) => {
                    self.advance();
                    break;
                }
                Some(TokenKind::Eof) | None => {
                    return Err(RtError::parse(self.cur_span(), "unclosed '{': unexpected end of input"))
                }
                _ => {}
            }
            stmts.push(self.parse_stmt()?);
            if !matches!(
                self.peek_kind(),
                Some(TokenKind::Eol | TokenKind::Semi | TokenKind::RBrace)
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

    // ============ 表达式（优先级爬升） ============

    pub fn parse_expr(&mut self) -> RtResult<Expr> {
        self.parse_ternary()
    }

    fn parse_ternary(&mut self) -> RtResult<Expr> {
        let cond = self.parse_logic_or()?;
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
            let op = match self.peek_kind() {
                Some(TokenKind::Eq) => BinaryOp::Eq,
                Some(TokenKind::Neq) => BinaryOp::Neq,
                _ => return Ok(left),
            };
            self.advance();
            let right = self.parse_comparison()?;
            let span = left.span();
            left = Expr::Binary { left: Box::new(left), op, right: Box::new(right), span };
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
                    let index = self.parse_expr()?;
                    self.expect(TokenKind::RBracket, "expected ']' after index")?;
                    let span = expr.span();
                    expr = Expr::Index { target: Box::new(expr), index: Box::new(index), span };
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
                self.expect(TokenKind::LParen, "expected '(' after 'function'")?;
                let params = self.parse_params()?;
                self.skip_eol();
                let body = Rc::new(self.parse_block()?);
                Ok(Expr::Function { params, body, span })
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

    // ============ 工具方法 ============

    /// 成员名/字段名：标识符或关键字（允许 `arr.contains` 这类与关键字不冲突的名字，
    /// 以及极少数关键字字段名），字符串也可作为对象字面量的键
    fn expect_member_name(&mut self) -> RtResult<String> {
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

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn peek_kind(&self) -> Option<&TokenKind> {
        self.peek().map(|t| &t.kind)
    }

    fn cur_span(&self) -> Span {
        self.peek().map(|t| t.span).unwrap_or_default()
    }

    fn at_end(&self) -> bool {
        matches!(self.peek_kind(), Some(TokenKind::Eof) | None)
    }

    fn advance(&mut self) -> Token {
        let tok = self.peek().cloned().unwrap_or(Token {
            kind: TokenKind::Eof,
            span: Span::default(),
        });
        if self.pos < self.tokens.len() {
            self.pos += 1;
        }
        tok
    }

    fn check(&self, kind: &TokenKind) -> bool {
        self.peek_kind() == Some(kind)
    }

    fn expect(&mut self, kind: TokenKind, msg: &str) -> RtResult<Token> {
        if self.check(&kind) {
            Ok(self.advance())
        } else {
            Err(RtError::parse(
                self.cur_span(),
                format!("{} , found {}", msg, self.describe_peek()),
            ))
        }
    }

    fn expect_ident(&mut self, msg: &str) -> RtResult<String> {
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
    fn skip_separators(&mut self) {
        while matches!(self.peek_kind(), Some(TokenKind::Eol | TokenKind::Semi)) {
            self.pos += 1;
        }
    }

    /// 仅跳过换行（块/参数前的宽松处理）
    fn skip_eol(&mut self) {
        while matches!(self.peek_kind(), Some(TokenKind::Eol)) {
            self.pos += 1;
        }
    }

    /// 当前 token 是否可能开始一个表达式
    fn expr_can_start(&self) -> bool {
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

    fn describe_peek(&self) -> String {
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
        Not => "!",
        Dot => ".",
        DotDot => "..",
        DotDotEq => "..=",
        Question => "?",
        QuestionDot => "?.",
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;
    use crate::Span;

    /// 把所有 span 抹平，便于断言 AST 结构
    fn no_span_expr(e: Expr) -> Expr {
        let d = Span::default();
        match e {
            Expr::Num(n, _) => Expr::Num(n, d),
            Expr::Str(s, _) => Expr::Str(s, d),
            Expr::Bool(b, _) => Expr::Bool(b, d),
            Expr::Null(_) => Expr::Null(d),
            Expr::Ident(s, _) => Expr::Ident(s, d),
            Expr::Array(items, _) => Expr::Array(items.into_iter().map(no_span_expr).collect(), d),
            Expr::Object(fields, _) => Expr::Object(
                fields.into_iter().map(|(k, v)| (k, no_span_expr(v))).collect(),
                d,
            ),
            Expr::Unary { op, operand, .. } => {
                Expr::Unary { op, operand: Box::new(no_span_expr(*operand)), span: d }
            }
            Expr::Binary { left, op, right, .. } => Expr::Binary {
                left: Box::new(no_span_expr(*left)),
                op,
                right: Box::new(no_span_expr(*right)),
                span: d,
            },
            Expr::Logic { left, op, right, .. } => Expr::Logic {
                left: Box::new(no_span_expr(*left)),
                op,
                right: Box::new(no_span_expr(*right)),
                span: d,
            },
            Expr::Ternary { cond, then_expr, else_expr, .. } => Expr::Ternary {
                cond: Box::new(no_span_expr(*cond)),
                then_expr: Box::new(no_span_expr(*then_expr)),
                else_expr: Box::new(no_span_expr(*else_expr)),
                span: d,
            },
            Expr::Index { target, index, .. } => Expr::Index {
                target: Box::new(no_span_expr(*target)),
                index: Box::new(no_span_expr(*index)),
                span: d,
            },
            Expr::Member { target, name, .. } => {
                Expr::Member { target: Box::new(no_span_expr(*target)), name, span: d }
            }
            Expr::OptionalMember { target, name, .. } => {
                Expr::OptionalMember { target: Box::new(no_span_expr(*target)), name, span: d }
            }
            Expr::Call { callee, args, .. } => Expr::Call {
                callee: Box::new(no_span_expr(*callee)),
                args: args.into_iter().map(no_span_expr).collect(),
                span: d,
            },
            Expr::New { class, args, .. } => Expr::New {
                class: Box::new(no_span_expr(*class)),
                args: args.into_iter().map(no_span_expr).collect(),
                span: d,
            },
            Expr::Function { params, body, .. } => Expr::Function {
                params,
                body: Rc::new(no_span_stmt((*body).clone())),
                span: d,
            },
        }
    }

    fn no_span_stmt(s: Stmt) -> Stmt {
        let d = Span::default();
        match s {
            Stmt::Expr(e, _) => Stmt::Expr(no_span_expr(e), d),
            Stmt::Assign { target, op, value, .. } => Stmt::Assign {
                target: no_span_expr(target),
                op,
                value: no_span_expr(value),
                span: d,
            },
            Stmt::If { cond, then_block, else_block, .. } => Stmt::If {
                cond: no_span_expr(cond),
                then_block: Box::new(no_span_stmt(*then_block)),
                else_block: else_block.map(|b| Box::new(no_span_stmt(*b))),
                span: d,
            },
            Stmt::While { cond, body, .. } => Stmt::While {
                cond: no_span_expr(cond),
                body: Box::new(no_span_stmt(*body)),
                span: d,
            },
            Stmt::ForIn { var, iter, body, .. } => Stmt::ForIn {
                var,
                iter: match iter {
                    ForIter::Range { start, end, inclusive } => ForIter::Range {
                        start: no_span_expr(start),
                        end: no_span_expr(end),
                        inclusive,
                    },
                    ForIter::Expr(e) => ForIter::Expr(no_span_expr(e)),
                },
                body: Box::new(no_span_stmt(*body)),
                span: d,
            },
            Stmt::ForC { var, init, cond, step, body, .. } => Stmt::ForC {
                var,
                init: no_span_expr(init),
                cond: cond.map(no_span_expr),
                step: step.map(|b| Box::new(no_span_stmt(*b))),
                body: Box::new(no_span_stmt(*body)),
                span: d,
            },
            Stmt::FuncDecl { name, params, body, .. } => Stmt::FuncDecl {
                name,
                params,
                body: Rc::new(no_span_stmt((*body).clone())),
                span: d,
            },
            Stmt::Return { value, .. } => Stmt::Return { value: value.map(no_span_expr), span: d },
            Stmt::Break(_) => Stmt::Break(d),
            Stmt::Continue(_) => Stmt::Continue(d),
            Stmt::Block { stmts, .. } => {
                Stmt::Block { stmts: stmts.into_iter().map(no_span_stmt).collect(), span: d }
            }
        }
    }

    fn expr(src: &str) -> Expr {
        let tokens = Lexer::new(src).tokenize().expect("lex ok").tokens;
        let mut p = Parser::new(tokens);
        no_span_expr(p.parse_expr().expect("parse ok"))
    }

    fn program(src: &str) -> Stmt {
        let tokens = Lexer::new(src).tokenize().expect("lex ok").tokens;
        let mut p = Parser::new(tokens);
        no_span_stmt(p.parse_program().expect("parse ok"))
    }

    fn num(n: f64) -> Expr {
        Expr::Num(n, Span::default())
    }

    fn ident(s: &str) -> Expr {
        Expr::Ident(s.into(), Span::default())
    }

    fn binary(left: Expr, op: BinaryOp, right: Expr) -> Expr {
        Expr::Binary { left: Box::new(left), op, right: Box::new(right), span: Span::default() }
    }

    // ============ 表达式 ============

    #[test]
    fn precedence_mul_over_add() {
        // 1 + 2 * 3 → 1 + (2 * 3)
        assert_eq!(
            expr("1 + 2 * 3"),
            binary(num(1.0), BinaryOp::Add, binary(num(2.0), BinaryOp::Mul, num(3.0)))
        );
    }

    #[test]
    fn parens_group() {
        assert_eq!(
            expr("(1 + 2) * 3"),
            binary(binary(num(1.0), BinaryOp::Add, num(2.0)), BinaryOp::Mul, num(3.0))
        );
    }

    #[test]
    fn comparison_and_equality_levels() {
        // a < b == c → (a < b) == c
        assert_eq!(
            expr("a < b == c"),
            binary(binary(ident("a"), BinaryOp::Lt, ident("b")), BinaryOp::Eq, ident("c"))
        );
    }

    #[test]
    fn logic_precedence() {
        // a || b && c → a || (b && c)
        let e = expr("a || b && c");
        match e {
            Expr::Logic { op: LogicOp::Or, right, .. } => {
                assert!(matches!(*right, Expr::Logic { op: LogicOp::And, .. }));
            }
            other => panic!("expected Logic(Or), got {:?}", other),
        }
    }

    #[test]
    fn unary_chain() {
        let e = expr("-x");
        assert!(matches!(
            e,
            Expr::Unary { op: UnaryOp::Neg, .. }
        ));
        let e = expr("!f()");
        assert!(matches!(e, Expr::Unary { op: UnaryOp::Not, operand, .. } if matches!(*operand, Expr::Call { .. })));
        // - -x
        assert!(matches!(expr("- -x"), Expr::Unary { op: UnaryOp::Neg, operand, .. } if matches!(*operand, Expr::Unary { op: UnaryOp::Neg, .. })));
    }

    #[test]
    fn ternary_right_associative() {
        // a ? b : c ? d : e → a ? b : (c ? d : e)
        let e = expr("a ? b : c ? d : e");
        match e {
            Expr::Ternary { else_expr, .. } => {
                assert!(matches!(*else_expr, Expr::Ternary { .. }));
            }
            other => panic!("expected Ternary, got {:?}", other),
        }
    }

    #[test]
    fn postfix_chains() {
        // obj.method(x).field[0]
        let e = expr("obj.method(x).field[0]");
        assert!(matches!(
            e,
            Expr::Index { index, .. } if matches!(*index, Expr::Num(0.0, _))
        ));
        let e2 = expr("f(1)(2)");
        assert!(matches!(e2, Expr::Call { .. }));
    }

    #[test]
    fn call_args_and_trailing_comma() {
        let e = expr("f(1, 2,)");
        match e {
            Expr::Call { args, .. } => assert_eq!(args.len(), 2),
            other => panic!("expected Call, got {:?}", other),
        }
    }

    #[test]
    fn array_literal() {
        assert_eq!(
            expr("[1, [2, 3],]"),
            Expr::Array(
                vec![num(1.0), Expr::Array(vec![num(2.0), num(3.0)], Span::default())],
                Span::default()
            )
        );
    }

    #[test]
    fn object_literal() {
        let e = expr("{x: 1, y: {z: 2},}");
        match e {
            Expr::Object(fields, _) => {
                assert_eq!(fields.len(), 2);
                assert_eq!(fields[0].0, "x");
                assert!(matches!(fields[1].1, Expr::Object(..)));
            }
            other => panic!("expected Object, got {:?}", other),
        }
        assert!(matches!(expr("{}"), Expr::Object(fields, _) if fields.is_empty()));
    }

    #[test]
    fn optional_member_chain() {
        let e = expr("p?.a?.b");
        assert!(matches!(
            e,
            Expr::OptionalMember { name, .. } if name == "b"
        ));
    }

    #[test]
    fn new_expression() {
        let e = expr("new MaxHeap()");
        match e {
            Expr::New { class, args, .. } => {
                assert!(matches!(*class, Expr::Ident(ref s, _) if s == "MaxHeap"));
                assert!(args.is_empty());
            }
            other => panic!("expected New, got {:?}", other),
        }
        // new MaxHeap([1,2]).push(3) → Call(Member(New))
        let e = expr("new MaxHeap([1, 2]).push(3)");
        assert!(matches!(
            e,
            Expr::Call { callee, .. } if matches!(*callee, Expr::Member { .. })
        ));
    }

    #[test]
    fn anonymous_function() {
        let e = expr("function(x, y) { x }");
        match e {
            Expr::Function { params, body, .. } => {
                assert_eq!(params, vec!["x".to_string(), "y".to_string()]);
                assert!(matches!(&*body, Stmt::Block { stmts, .. } if stmts.len() == 1));
            }
            other => panic!("expected Function, got {:?}", other),
        }
    }

    #[test]
    fn literals() {
        assert!(matches!(expr("3.14"), Expr::Num(n, _) if n == 3.14));
        assert!(matches!(expr("\"hi\""), Expr::Str(ref s, _) if s == "hi"));
        assert!(matches!(expr("true"), Expr::Bool(true, _)));
        assert!(matches!(expr("null"), Expr::Null(_)));
    }

    #[test]
    fn expression_errors() {
        assert!(expr_src_fails("1 +"));
        assert!(expr_src_fails("a ?"));
        assert!(expr_src_fails("f("));
        assert!(expr_src_fails("[1, 2"));
    }

    fn expr_src_fails(src: &str) -> bool {
        let tokens = match Lexer::new(src).tokenize() {
            Ok(o) => o.tokens,
            Err(_) => return true,
        };
        let mut p = Parser::new(tokens);
        p.parse_expr().is_err()
    }

    // ============ 语句 ============

    #[test]
    fn simple_assign_program() {
        let p = program("x = 1\ny = x + 2");
        match p {
            Stmt::Block { stmts, .. } => {
                assert_eq!(stmts.len(), 2);
                assert!(matches!(&stmts[0], Stmt::Assign { op: AssignOp::Set, .. }));
                assert!(matches!(&stmts[1], Stmt::Assign { .. }));
            }
            other => panic!("expected Block, got {:?}", other),
        }
    }

    #[test]
    fn compound_and_member_assign() {
        assert!(matches!(&program("i += 1").clone(), Stmt::Block { stmts, .. }
            if matches!(&stmts[0], Stmt::Assign { op: AssignOp::Add, .. })));
        assert!(matches!(&program("a[0] *= 2"), Stmt::Block { stmts, .. }
            if matches!(&stmts[0], Stmt::Assign { op: AssignOp::Mul, target: Expr::Index { .. }, .. })));
        assert!(matches!(&program("p.x -= 1"), Stmt::Block { stmts, .. }
            if matches!(&stmts[0], Stmt::Assign { op: AssignOp::Sub, target: Expr::Member { .. }, .. })));
        // 可选链不能作为赋值目标
        assert!(program_fails("p?.x = 1"));
    }

    #[test]
    fn if_else_chain() {
        let src = "if a { f() } else if b { g() } else { h() }";
        match program(src) {
            Stmt::Block { stmts, .. } => {
                let first = &stmts[0];
                match first {
                    Stmt::If { else_block: Some(elif), .. } => match &**elif {
                        Stmt::If { else_block: Some(els), .. } => {
                            assert!(matches!(&**els, Stmt::Block { .. }));
                        }
                        other => panic!("nested if expected, got {:?}", other),
                    },
                    other => panic!("expected If, got {:?}", other),
                }
            }
            other => panic!("expected Block, got {:?}", other),
        }
    }

    #[test]
    fn while_and_for_in() {
        assert!(matches!(program("while x < 3 { x += 1 }"), Stmt::Block { ref stmts, .. }
            if matches!(&stmts[0], Stmt::While { .. })));

        assert!(matches!(program("for x in arr { f(x) }"), Stmt::Block { ref stmts, .. }
            if matches!(&stmts[0], Stmt::ForIn { iter: ForIter::Expr(_), .. })));

        assert!(matches!(program("for i in 0..n { f(i) }"), Stmt::Block { ref stmts, .. }
            if matches!(&stmts[0], Stmt::ForIn { iter: ForIter::Range { inclusive: false, .. }, .. })));

        assert!(matches!(program("for i in 0..=n { f(i) }"), Stmt::Block { ref stmts, .. }
            if matches!(&stmts[0], Stmt::ForIn { iter: ForIter::Range { inclusive: true, .. }, .. })));
    }

    #[test]
    fn c_style_for() {
        let p = program("for i = 0; i < n; i += 1 { f(i) }");
        match p {
            Stmt::Block { stmts, .. } => match &stmts[0] {
                Stmt::ForC { var, init, cond, step, .. } => {
                    assert_eq!(var, "i");
                    assert!(matches!(init, Expr::Num(0.0, _)));
                    assert!(cond.is_some());
                    assert!(matches!(&**step.as_ref().unwrap(), Stmt::Assign { op: AssignOp::Add, .. }));
                }
                other => panic!("expected ForC, got {:?}", other),
            },
            other => panic!("expected Block, got {:?}", other),
        }
        // 空条件段允许
        assert!(matches!(program("for i = 0; ; i += 1 { }"), Stmt::Block { ref stmts, .. }
            if matches!(&stmts[0], Stmt::ForC { cond: None, .. })));
    }

    #[test]
    fn func_decl_and_return() {
        let p = program("function add(a, b) {\n    return a + b\n}");
        match p {
            Stmt::Block { stmts, .. } => match &stmts[0] {
                Stmt::FuncDecl { name, params, body, .. } => {
                    assert_eq!(name, "add");
                    assert_eq!(params.len(), 2);
                    assert!(matches!(&**body, Stmt::Block { stmts, .. } if stmts.len() == 1));
                }
                other => panic!("expected FuncDecl, got {:?}", other),
            },
            other => panic!("expected Block, got {:?}", other),
        }
        // 无值 return
        assert!(matches!(program("function f() { return }"), Stmt::Block { ref stmts, .. }
            if matches!(&stmts[0], Stmt::FuncDecl { .. })));
    }

    #[test]
    fn break_continue() {
        assert!(matches!(program("break"), Stmt::Block { ref stmts, .. } if matches!(&stmts[0], Stmt::Break(_))));
        assert!(matches!(program("continue"), Stmt::Block { ref stmts, .. } if matches!(&stmts[0], Stmt::Continue(_))));
    }

    #[test]
    fn semicolon_separators() {
        let p = program("x = 1; y = 2;");
        match p {
            Stmt::Block { stmts, .. } => assert_eq!(stmts.len(), 2),
            other => panic!("expected Block, got {:?}", other),
        }
    }

    #[test]
    fn missing_separator_is_error() {
        assert!(program_fails("x = 1 y = 2"));
    }

    #[test]
    fn comments_and_blank_lines_between_stmts() {
        let p = program("// leading comment\n\nx = 1\n\n// mid\ny = 2\n");
        match p {
            Stmt::Block { stmts, .. } => assert_eq!(stmts.len(), 2),
            other => panic!("expected Block, got {:?}", other),
        }
    }

    #[test]
    fn block_allows_newline_before_brace() {
        assert!(matches!(
            program("if cond\n{\n    x = 1\n}"),
            Stmt::Block { ref stmts, .. } if matches!(&stmts[0], Stmt::If { .. })
        ));
    }

    #[test]
    fn else_on_next_line() {
        let p = program("if a { x = 1 }\nelse { x = 2 }");
        match p {
            Stmt::Block { stmts, .. } => {
                assert!(matches!(&stmts[0], Stmt::If { else_block: Some(_), .. }));
            }
            other => panic!("expected Block, got {:?}", other),
        }
    }

    fn program_fails(src: &str) -> bool {
        let tokens = match Lexer::new(src).tokenize() {
            Ok(o) => o.tokens,
            Err(_) => return true,
        };
        let mut p = Parser::new(tokens);
        p.parse_program().is_err()
    }
}
