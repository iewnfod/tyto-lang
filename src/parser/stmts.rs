//! 语句与声明：let/const、赋值、函数、struct/impl/interface、控制流、块。

use std::rc::Rc;

use crate::ast::*;
use crate::lexer::{Keyword, TokenKind};
use crate::{RtError, RtResult, Span};

use super::Parser;

impl Parser {
    // ============ 语句 ============

    pub(super) fn parse_stmt(&mut self) -> RtResult<Stmt> {
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
            Some(TokenKind::Keyword(Keyword::Struct)) => self.parse_struct(span),
            Some(TokenKind::Keyword(Keyword::Impl)) => self.parse_impl(span),
            Some(TokenKind::Keyword(Keyword::Interface)) => self.parse_interface(span),
            Some(TokenKind::Keyword(Keyword::Let)) => {
                self.parse_let_const(span, DeclKind::Let)
            }
            Some(TokenKind::Keyword(Keyword::Const)) => {
                self.parse_let_const(span, DeclKind::Const)
            }
            _ => self.parse_assign_or_expr(span),
        }
    }

    /// `let x = v` / `let x: T = v` / `const ...`：声明（必须初始化）
    fn parse_let_const(&mut self, span: Span, kind: DeclKind) -> RtResult<Stmt> {
        let kw = match kind {
            DeclKind::Let => "let",
            DeclKind::Const => "const",
        };
        self.advance(); // let / const
        let name = self
            .expect_ident(&format!("expected variable name after `{}`", kw))?;
        let ann = if self.check(&TokenKind::Colon) {
            self.advance();
            Some(self.parse_type()?)
        } else {
            None
        };
        let msg = if kind == DeclKind::Const {
            "expected '=' after const declaration (const requires an initializer)"
        } else {
            "expected '=' after let declaration (initializers are required)"
        };
        self.expect(TokenKind::Assign, msg)?;
        let value = self.parse_expr()?;
        Ok(Stmt::Assign {
            target: Expr::Ident(name, span),
            op: AssignOp::Set,
            value,
            ann,
            decl: Some(kind),
            span,
        })
    }

    /// 表达式 / 赋值语句（`x = 1`、`a[0] += 1`、`p.x = f()`）
    fn parse_assign_or_expr(&mut self, span: Span) -> RtResult<Stmt> {
        let expr = self.parse_expr()?;
        // `x: T = v`：类型标注赋值（仅普通变量，纯文档性质，运行时不检查）
        if self.check(&TokenKind::Colon) {
            let name = match &expr {
                Expr::Ident(name, _) => name.clone(),
                _ => {
                    return Err(RtError::parse(
                        expr.span(),
                        "type annotations are only allowed on plain variables",
                    ))
                }
            };
            self.advance(); // :
            let ann = self.parse_type()?;
            self.expect(TokenKind::Assign, "expected '=' after type annotation")?;
            let value = self.parse_expr()?;
            return Ok(Stmt::Assign {
                target: Expr::Ident(name, span),
                op: AssignOp::Set,
                value,
                ann: Some(ann),
                decl: None,
                span,
            });
        }
        let op = match self.peek_kind() {
            Some(TokenKind::Assign) => AssignOp::Set,
            Some(TokenKind::AddAssign) => AssignOp::Add,
            Some(TokenKind::SubAssign) => AssignOp::Sub,
            Some(TokenKind::MulAssign) => AssignOp::Mul,
            Some(TokenKind::DivAssign) => AssignOp::Div,
            Some(TokenKind::ModAssign) => AssignOp::Mod,
            Some(TokenKind::QuestionQuestionAssign) => AssignOp::Nullish,
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
        Ok(Stmt::Assign { target: expr, op, value, ann: None, decl: None, span })
    }

    fn parse_func_decl(&mut self, span: Span) -> RtResult<Stmt> {
        self.advance(); // function
        let name = self.expect_ident("expected function name after 'function'")?;
        let type_params = self.parse_type_params()?;
        self.expect(TokenKind::LParen, "expected '(' after function name")?;
        let params = self.parse_params()?;
        let ret = self.parse_ret_ann()?;
        let body = Rc::new(self.parse_block()?);
        Ok(Stmt::FuncDecl { name, params, type_params, ret, body, span })
    }

    pub(super) fn parse_params(&mut self) -> RtResult<Vec<Param>> {
        let mut params = Vec::new();
        if self.check(&TokenKind::RParen) {
            self.advance();
            return Ok(params);
        }
        loop {
            let name = self.expect_ident("expected parameter name")?;
            let ty = if self.check(&TokenKind::Colon) {
                self.advance();
                Some(self.parse_type()?)
            } else {
                None
            };
            params.push(Param { name, ty });
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

    /// `struct Point { x, y, }`：字段名列表，逗号/换行均可作分隔，允许尾逗号
    fn parse_struct(&mut self, span: Span) -> RtResult<Stmt> {
        self.advance(); // struct
        let name = self.expect_ident("expected struct name after 'struct'")?;
        let type_params = self.parse_type_params()?;
        self.skip_eol();
        self.expect(TokenKind::LBrace, "expected '{' after struct name")?;
        let mut fields = Vec::new();
        loop {
            self.skip_separators();
            if self.check(&TokenKind::RBrace) {
                self.advance();
                break;
            }
            if self.at_end() {
                return Err(RtError::parse(self.cur_span(), "unclosed '{': unexpected end of input"));
            }
            let msg = format!("expected field name in struct `{}`", name);
            let field = self.expect_ident(&msg)?;
            // 可选类型标注 `x: number`（纯文档性质）
            let ty = if self.check(&TokenKind::Colon) {
                self.advance();
                Some(self.parse_type()?)
            } else {
                None
            };
            fields.push(Param { name: field, ty });
            // 逗号可选：换行分隔（`x\ny`）同样合法，分隔符在循环顶部跳过
            if self.check(&TokenKind::Comma) {
                self.advance();
            }
        }
        Ok(Stmt::Struct { name, type_params, fields, span })
    }

    /// `impl Point { function new(...) { } ... }`：方法序列，复用函数声明语法。
    /// 目标名后可声明泛型参数（`impl TreeNode<T>`），方法名后亦可（`function id<T>`）。
    fn parse_impl(&mut self, span: Span) -> RtResult<Stmt> {
        self.advance(); // impl
        let target = self.expect_ident("expected struct name after 'impl'")?;
        let type_params = self.parse_type_params()?;
        self.skip_eol();
        self.expect(TokenKind::LBrace, "expected '{' after struct name")?;
        let mut methods = Vec::new();
        loop {
            self.skip_separators();
            if self.check(&TokenKind::RBrace) {
                self.advance();
                break;
            }
            if self.at_end() {
                return Err(RtError::parse(self.cur_span(), "unclosed '{': unexpected end of input"));
            }
            if !self.check(&TokenKind::Keyword(Keyword::Function)) {
                return Err(RtError::parse(
                    self.cur_span(),
                    format!(
                        "impl body may only contain `function` declarations, found {}",
                        self.describe_peek()
                    ),
                ));
            }
            let mspan = self.cur_span();
            self.advance(); // function
            let name = self.expect_method_name()?;
            let m_type_params = self.parse_type_params()?;
            self.expect(TokenKind::LParen, "expected '(' after method name")?;
            let params = self.parse_params()?;
            let ret = self.parse_ret_ann()?;
            let body = Rc::new(self.parse_block()?);
            methods.push(Stmt::FuncDecl {
                name,
                params,
                type_params: m_type_params,
                ret,
                body,
                span: mspan,
            });
        }
        Ok(Stmt::Impl { target, type_params, methods, span })
    }

    /// `interface Shape { function area() ... }`：方法签名（无函数体），逗号/换行分隔
    fn parse_interface(&mut self, span: Span) -> RtResult<Stmt> {
        self.advance(); // interface
        let name = self.expect_ident("expected interface name after 'interface'")?;
        self.skip_eol();
        self.expect(TokenKind::LBrace, "expected '{' after interface name")?;
        let mut methods = Vec::new();
        loop {
            self.skip_separators();
            if self.check(&TokenKind::RBrace) {
                self.advance();
                break;
            }
            if self.at_end() {
                return Err(RtError::parse(self.cur_span(), "unclosed '{': unexpected end of input"));
            }
            self.expect(
                TokenKind::Keyword(Keyword::Function),
                "expected 'function' in interface body",
            )?;
            let msg = format!("expected method name in interface `{}`", name);
            let mname = self.expect_ident(&msg)?;
            self.expect(TokenKind::LParen, "expected '(' after interface method name")?;
            // 完整签名保留（checker 用）；运行时 `is` 检查只看方法名
            let params = self.parse_params()?;
            let ret = self.parse_ret_ann()?;
            if self.check(&TokenKind::LBrace) {
                return Err(RtError::parse(
                    self.cur_span(),
                    "interface methods cannot have bodies",
                ));
            }
            methods.push(InterfaceMethod { name: mname, params, ret });
            if self.check(&TokenKind::Comma) {
                self.advance();
            }
        }
        Ok(Stmt::Interface { name, methods, span })
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

    pub(super) fn parse_block(&mut self) -> RtResult<Stmt> {
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
}
