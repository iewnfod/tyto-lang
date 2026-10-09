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

    fn parse_params(&mut self) -> RtResult<Vec<Param>> {
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

    // ============ 类型标注（渐进类型系统：运行时擦除，checker 检查） ============

    /// 类型语法（联合层）：`atom | atom | ...`
    fn parse_type(&mut self) -> RtResult<TypeAst> {
        let first = self.parse_type_atom()?;
        if !self.check(&TokenKind::Pipe) {
            return Ok(first);
        }
        let span = first.span();
        let mut members = vec![first];
        while self.check(&TokenKind::Pipe) {
            self.advance();
            members.push(self.parse_type_atom()?);
        }
        Ok(TypeAst::Union(members, span))
    }

    /// 类型原子：`Ident<...>`、`Ident[]`、`{x: T}`、`(a: T) -> R`、`(T)` 分组
    fn parse_type_atom(&mut self) -> RtResult<TypeAst> {
        if self.check(&TokenKind::LBrace) {
            return self.parse_object_type();
        }
        if self.check(&TokenKind::LParen) {
            if self.is_func_type_ahead() {
                return self.parse_func_type();
            }
            // 括号分组：`(T)`（如 `(number | null)[]`），分组后可叠 `[]` 后缀
            self.advance(); // (
            let mut inner = self.parse_type()?;
            self.expect(TokenKind::RParen, "expected ')' to close grouped type")?;
            while self.check(&TokenKind::LBracket) {
                let open = self.cur_span();
                self.advance();
                self.expect(TokenKind::RBracket, "expected ']' after '[' in type")?;
                inner = TypeAst::Array(Box::new(inner), open);
            }
            return Ok(inner);
        }
        let span = self.cur_span();
        // `null` 是关键字但也是合法类型名（联合类型常见：`number | null`）
        let base_name = if self.check(&TokenKind::Keyword(Keyword::Null)) {
            self.advance();
            "null".to_string()
        } else {
            self.expect_ident("expected type name")?
        };
        let mut ty = {
            let name = base_name;
            if self.check(&TokenKind::Lt) {
                self.advance();
                let mut args = Vec::new();
                loop {
                    args.push(self.parse_type()?);
                    if self.check(&TokenKind::Comma) {
                        self.advance();
                        if self.check(&TokenKind::Gt) {
                            break; // 尾逗号
                        }
                    } else {
                        break;
                    }
                }
                self.expect(TokenKind::Gt, "expected '>' to close generic arguments")?;
                TypeAst::Generic(name, args, span)
            } else {
                TypeAst::Named(name, span)
            }
        };
        while self.check(&TokenKind::LBracket) {
            let open = self.cur_span();
            self.advance();
            self.expect(TokenKind::RBracket, "expected ']' after '[' in type")?;
            let inner_span = ty.span();
            ty = TypeAst::Array(Box::new(ty), inner_span.min(open));
        }
        Ok(ty)
    }

    /// 结构化对象类型：`{x: number, y: string}`（字段名 + 类型，逗号/换行分隔）
    fn parse_object_type(&mut self) -> RtResult<TypeAst> {
        let open = self.cur_span();
        self.advance(); // {
        let mut fields = Vec::new();
        self.skip_eol();
        while !self.check(&TokenKind::RBrace) {
            if self.at_end() {
                return Err(RtError::parse(self.cur_span(), "unclosed '{' in object type"));
            }
            let name = self.expect_ident("expected field name in object type")?;
            self.expect(TokenKind::Colon, "expected ':' after field name in object type")?;
            let ty = self.parse_type()?;
            fields.push((name, ty));
            if self.check(&TokenKind::Comma) {
                self.advance();
            }
            self.skip_eol();
        }
        let close = self.cur_span();
        self.advance(); // }
        Ok(TypeAst::Object(fields, open.min(close)))
    }

    /// 函数类型前瞻：`(` 之后是 `name :` 或 `) ->` → 函数类型；否则是括号分组
    fn is_func_type_ahead(&self) -> bool {
        match self.tokens.get(self.pos + 1).map(|t| &t.kind) {
            Some(TokenKind::RParen) => matches!(
                self.tokens.get(self.pos + 2).map(|t| &t.kind),
                Some(TokenKind::Arrow)
            ),
            Some(TokenKind::Ident(_)) => {
                matches!(self.tokens.get(self.pos + 2).map(|t| &t.kind), Some(TokenKind::Colon))
            }
            _ => false,
        }
    }

    /// 函数类型：`(a: number, b: string) -> bool`（返回类型必填，区别于普通括号表达式）
    fn parse_func_type(&mut self) -> RtResult<TypeAst> {
        let open = self.cur_span();
        self.advance(); // (
        let mut params = Vec::new();
        self.skip_eol();
        while !self.check(&TokenKind::RParen) {
            if self.at_end() {
                return Err(RtError::parse(self.cur_span(), "unclosed '(' in function type"));
            }
            let name = self.expect_ident("expected parameter name in function type")?;
            self.expect(TokenKind::Colon, "expected ':' after parameter name in function type")?;
            let ty = self.parse_type()?;
            params.push((name, ty));
            if self.check(&TokenKind::Comma) {
                self.advance();
            }
            self.skip_eol();
        }
        self.advance(); // )
        self.expect(TokenKind::Arrow, "expected '->' after function type parameters")?;
        let ret = Box::new(self.parse_type()?);
        let span = open.min(ret.span());
        Ok(TypeAst::Func { params, ret: Some(ret), span })
    }

    /// 泛型参数声明：`<T, U>`（仅标识符列表；函数/struct 名之后）
    fn parse_type_params(&mut self) -> RtResult<Vec<String>> {
        let mut out = Vec::new();
        if !self.check(&TokenKind::Lt) {
            return Ok(out);
        }
        self.advance();
        loop {
            let name = self.expect_ident("expected type parameter name")?;
            if out.contains(&name) {
                return Err(RtError::parse(
                    self.cur_span(),
                    format!("duplicate type parameter `{}`", name),
                ));
            }
            out.push(name);
            if self.check(&TokenKind::Comma) {
                self.advance();
                if self.check(&TokenKind::Gt) {
                    break; // 尾逗号
                }
            } else {
                break;
            }
        }
        self.expect(TokenKind::Gt, "expected '>' to close type parameters")?;
        Ok(out)
    }

    /// `)` 之后可选的返回类型标注：`-> T`（`->` 允许出现在换行之后）
    fn parse_ret_ann(&mut self) -> RtResult<Option<TypeAst>> {
        self.skip_eol();
        if self.check(&TokenKind::Arrow) {
            self.advance();
            let ty = self.parse_type()?;
            self.skip_eol();
            Ok(Some(ty))
        } else {
            Ok(None)
        }
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

    // ============ 工具方法 ============

    /// impl 方法名：标识符，或关键字 `new`（构造方法约定名）
    fn expect_method_name(&mut self) -> RtResult<String> {
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
    fn expect_member_name(&mut self) -> RtResult<String> {        match self.peek_kind().cloned() {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;
    use crate::Span;

    /// 标注 → 规范串（Display），断言 TypeAst 用
    fn ty_str(t: &Option<TypeAst>) -> Option<String> {
        t.as_ref().map(|x| x.to_string())
    }

    /// 把所有 span 抹平，便于断言 AST 结构
    fn no_span_expr(e: Expr) -> Expr {
        let d = Span::default();        match e {
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
            Expr::Slice { target, start, end, inclusive, .. } => Expr::Slice {
                target: Box::new(no_span_expr(*target)),
                start: start.map(|e| Box::new(no_span_expr(*e))),
                end: end.map(|e| Box::new(no_span_expr(*e))),
                inclusive,
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
            Expr::Is { operand, target, .. } => Expr::Is {
                operand: Box::new(no_span_expr(*operand)),
                target: Box::new(no_span_expr(*target)),
                span: d,
            },
            Expr::Function { params, type_params, ret, body, .. } => Expr::Function {
                params,
                type_params,
                ret,
                body: Rc::new(no_span_stmt((*body).clone())),
                span: d,
            },
        }
    }

    fn no_span_stmt(s: Stmt) -> Stmt {
        let d = Span::default();
        match s {
            Stmt::Expr(e, _) => Stmt::Expr(no_span_expr(e), d),
            Stmt::Assign { target, op, value, ann, decl, .. } => Stmt::Assign {
                target: no_span_expr(target),
                op,
                value: no_span_expr(value),
                ann,
                decl,
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
            Stmt::FuncDecl { name, params, type_params, ret, body, .. } => Stmt::FuncDecl {
                name,
                params,
                type_params,
                ret,
                body: Rc::new(no_span_stmt((*body).clone())),
                span: d,
            },
            Stmt::Struct { name, type_params, fields, .. } => {
                Stmt::Struct { name, type_params, fields, span: d }
            }
            Stmt::Impl { target, type_params, methods, .. } => Stmt::Impl {
                target,
                type_params,
                methods: methods.into_iter().map(no_span_stmt).collect(),
                span: d,
            },
            Stmt::Interface { name, methods, .. } => {
                Stmt::Interface { name, methods, span: d }
            }
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
    fn nullish_precedence_and_assoc() {
        // a ?? b || c → a ?? (b || c)：?? 低于 ||
        let e = expr("a ?? b || c");
        match e {
            Expr::Logic { op: LogicOp::Nullish, right, .. } => {
                assert!(matches!(*right, Expr::Logic { op: LogicOp::Or, .. }));
            }
            other => panic!("expected Logic(Nullish), got {:?}", other),
        }
        // a ?? b ?? c → (a ?? b) ?? c：左结合
        let e = expr("a ?? b ?? c");
        match e {
            Expr::Logic { op: LogicOp::Nullish, left, .. } => {
                assert!(matches!(*left, Expr::Logic { op: LogicOp::Nullish, .. }));
            }
            other => panic!("expected nested Logic(Nullish), got {:?}", other),
        }
        // ?? 高于三元：a ?? b ? c : d → (a ?? b) ? c : d
        assert!(matches!(expr("a ?? b ? c : d"), Expr::Ternary { .. }));
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
    fn slice_syntax_forms() {
        // a[1..3]：双端
        let e = expr("a[1..3]");
        match e {
            Expr::Slice { target, start, end, inclusive, .. } => {
                assert!(matches!(*target, Expr::Ident(ref s, _) if s == "a"));
                assert!(matches!(*start.unwrap(), Expr::Num(1.0, _)));
                assert!(matches!(*end.unwrap(), Expr::Num(3.0, _)));
                assert!(!inclusive);
            }
            other => panic!("expected Slice, got {:?}", other),
        }
        // a[..3]：省略起点
        let e = expr("a[..3]");
        match e {
            Expr::Slice { start, end, .. } => {
                assert!(start.is_none());
                assert!(matches!(*end.unwrap(), Expr::Num(3.0, _)));
            }
            other => panic!("expected Slice, got {:?}", other),
        }
        // a[1..]：省略终点
        let e = expr("a[1..]");
        match e {
            Expr::Slice { start, end, .. } => {
                assert!(matches!(*start.unwrap(), Expr::Num(1.0, _)));
                assert!(end.is_none());
            }
            other => panic!("expected Slice, got {:?}", other),
        }
        // a[..]：两端都省略（全量拷贝）
        assert!(matches!(expr("a[..]"), Expr::Slice { start: None, end: None, .. }));
        // a[1..=3]：含终点
        assert!(matches!(expr("a[1..=3]"), Expr::Slice { inclusive: true, .. }));
        assert!(matches!(expr("a[..=3]"), Expr::Slice { inclusive: true, start: None, .. }));
        // 表达式端点
        assert!(matches!(expr("a[n + 1..m]"), Expr::Slice { .. }));
        assert!(matches!(expr("a[x..-2]"), Expr::Slice { .. }));
        // 切片可继续链式后缀
        assert!(matches!(expr("a[1..][0]"), Expr::Index { .. }));
        assert!(matches!(expr("s[..2].len()"), Expr::Call { .. }));
        // 普通索引不受影响
        assert!(matches!(expr("a[0]"), Expr::Index { .. }));
    }

    #[test]
    fn slice_not_assignable() {
        assert!(program_fails("a[1..2] = [1, 2]"));
        assert!(program_fails("s[..1] += \"x\""));
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
                assert_eq!(
                    params.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
                    vec!["x", "y"]
                );
                assert!(params.iter().all(|p| p.ty.is_none()));
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
    fn nullish_assign_parses() {
        assert!(matches!(&program("x ??= 1"), Stmt::Block { stmts, .. }
            if matches!(&stmts[0], Stmt::Assign { op: AssignOp::Nullish, .. })));
        // 索引 / 成员目标
        assert!(matches!(&program("a[0] ??= 1"), Stmt::Block { stmts, .. }
            if matches!(&stmts[0], Stmt::Assign { op: AssignOp::Nullish, target: Expr::Index { .. }, .. })));
        assert!(matches!(&program("p.x ??= 1"), Stmt::Block { stmts, .. }
            if matches!(&stmts[0], Stmt::Assign { op: AssignOp::Nullish, target: Expr::Member { .. }, .. })));
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

    // ============ 类型标注（纯文档性质，运行时不检查） ============

    #[test]
    fn union_type_annotation() {
        // 用户核心诉求：x: number | null = null
        match program("x: number | null = null") {
            Stmt::Block { stmts, .. } => match &stmts[0] {
                Stmt::Assign { ann, .. } => {
                    assert_eq!(ty_str(&ann).as_deref(), Some("number | null"));
                    assert!(matches!(ann.as_ref(), Some(TypeAst::Union(ms, _)) if ms.len() == 2));
                }
                other => panic!("expected Assign, got {:?}", other),
            },
            other => panic!("expected Block, got {:?}", other),
        }
        // 多成员联合、联合内的泛型与数组
        for (src, ty) in [
            ("x: number | string | bool = x", "number | string | bool"),
            ("x: Array<number> | null = null", "Array<number> | null"),
            ("x: number[] | string[] = a", "number[] | string[]"),
            ("m: Map<string, number | null> = m", "Map<string, number | null>"),
            ("f: (number | null)[] = f", "(number | null)[]"),
        ] {
            match program(src) {
                Stmt::Block { stmts, .. } => match &stmts[0] {
                    Stmt::Assign { ann, .. } => assert_eq!(ty_str(&ann).as_deref(), Some(ty)),
                    other => panic!("{}: expected Assign, got {:?}", src, other),
                },
                other => panic!("{}: expected Block, got {:?}", src, other),
            }
        }
        // 联合解析错误
        assert!(program_fails("x: | number = 1")); // 联合首成员缺失
        assert!(program_fails("x: number | = 1")); // 后续成员缺失
    }

    #[test]
    fn object_and_func_type_annotation() {
        for (src, ty) in [
            ("o: {x: number, y: string} = o", "{x: number, y: string}"),
            ("o: {x: number} = o", "{x: number}"),
            ("f: (a: number) -> string = f", "(a: number) -> string"),
            ("f: (a: number, b: bool) -> number = f", "(a: number, b: bool) -> number"),
            ("cb: (e: string) -> null = cb", "(e: string) -> null"),
        ] {
            match program(src) {
                Stmt::Block { stmts, .. } => match &stmts[0] {
                    Stmt::Assign { ann, .. } => assert_eq!(ty_str(&ann).as_deref(), Some(ty)),
                    other => panic!("{}: expected Assign, got {:?}", src, other),
                },
                other => panic!("{}: expected Block, got {:?}", src, other),
            }
        }
        // 对象类型字段必须带类型；函数类型必须 -> 返回
        assert!(program_fails("o: {x} = o"));
        assert!(program_fails("f: (a: number) = f"));
    }

    #[test]
    fn generic_declarations() {
        // 泛型函数声明
        match program("function id<T>(x: T) -> T { return x }") {
            Stmt::Block { stmts, .. } => match &stmts[0] {
                Stmt::FuncDecl { type_params, ret, params, .. } => {
                    assert_eq!(type_params, &["T".to_string()][..]);
                    assert_eq!(ty_str(&params[0].ty).as_deref(), Some("T"));
                    assert_eq!(ty_str(&ret).as_deref(), Some("T"));
                }
                other => panic!("expected FuncDecl, got {:?}", other),
            },
            other => panic!("expected Block, got {:?}", other),
        }
        // 多泛型参数 + 尾逗号
        match program("function pair<K, V>(k: K, v: V) -> V { return v }") {
            Stmt::Block { stmts, .. } => match &stmts[0] {
                Stmt::FuncDecl { type_params, .. } => {
                    assert_eq!(type_params, &["K".to_string(), "V".to_string()][..]);
                }
                other => panic!("expected FuncDecl, got {:?}", other),
            },
            other => panic!("expected Block, got {:?}", other),
        }
        // 泛型 struct
        match program("struct Box<T> {\n    v: T,\n}") {
            Stmt::Block { stmts, .. } => match &stmts[0] {
                Stmt::Struct { name, type_params, fields, .. } => {
                    assert_eq!(name, "Box");
                    assert_eq!(type_params, &["T".to_string()][..]);
                    assert_eq!(ty_str(&fields[0].ty).as_deref(), Some("T"));
                }
                other => panic!("expected Struct, got {:?}", other),
            },
            other => panic!("expected Block, got {:?}", other),
        }
        // 泛型 impl：块级 + 方法级泛型参数（递归 struct 场景）
        match program("impl TreeNode<T> {\n    function new<T>(val: T) {\n        self.val = val\n    }\n}") {
            Stmt::Block { stmts, .. } => match &stmts[0] {
                Stmt::Impl { target, type_params, methods, .. } => {
                    assert_eq!(target, "TreeNode");
                    assert_eq!(type_params, &["T".to_string()][..]);
                    match &methods[0] {
                        Stmt::FuncDecl { name, type_params: mtp, params, .. } => {
                            assert_eq!(name, "new");
                            assert_eq!(mtp, &["T".to_string()][..]);
                            assert_eq!(ty_str(&params[0].ty).as_deref(), Some("T"));
                        }
                        other => panic!("expected FuncDecl, got {:?}", other),
                    }
                }
                other => panic!("expected Impl, got {:?}", other),
            },
            other => panic!("expected Block, got {:?}", other),
        }
        // impl 不带泛型参数照旧合法（非泛型 struct）
        assert!(matches!(
            program("impl P {\n    function len() { return 1 }\n}"),
            Stmt::Block { stmts, .. } if matches!(&stmts[0], Stmt::Impl { type_params, .. } if type_params.is_empty())
        ));
        // 匿名泛型函数
        match expr("function<T>(x: T) -> T { return x }") {
            Expr::Function { type_params, .. } => {
                assert_eq!(type_params, &["T".to_string()][..]);
            }
            other => panic!("expected Function, got {:?}", other),
        }
        // 错误形态：未闭合 / 重复参数名
        assert!(program_fails("function id<T(x) -> T { return x }"));
        assert!(program_fails("function id<T, T>(x: T) -> T { return x }"));
        assert!(program_fails("impl Box<T { }"));
        // 普通函数不受影响
        assert!(matches!(
            program("function f(x) { return x }"),
            Stmt::Block { stmts, .. } if matches!(&stmts[0], Stmt::FuncDecl { type_params, .. } if type_params.is_empty())
        ));
    }

    #[test]
    fn let_const_declarations() {
        // let：带/不带标注
        for src in ["let x = 1", "let x: number = 1", "let x: number | null = null"] {
            match program(src) {
                Stmt::Block { stmts, .. } => match &stmts[0] {
                    Stmt::Assign { target, decl, ann, .. } => {
                        assert!(matches!(target, Expr::Ident(..)));
                        assert_eq!(*decl, Some(DeclKind::Let));
                        if src.contains("number") {
                            assert!(ann.is_some());
                        } else {
                            assert!(ann.is_none());
                        }
                    }
                    other => panic!("{}: expected Assign, got {:?}", src, other),
                },
                other => panic!("{}: expected Block, got {:?}", src, other),
            }
        }
        // const：必须初始化
        match program("const PI: number = 3.14") {
            Stmt::Block { stmts, .. } => match &stmts[0] {
                Stmt::Assign { decl, .. } => assert_eq!(*decl, Some(DeclKind::Const)),
                other => panic!("expected Assign, got {:?}", other),
            },
            other => panic!("expected Block, got {:?}", other),
        }
        assert!(program_fails("const x")); // 缺初始化
        assert!(program_fails("let x")); // 缺初始化
        assert!(program_fails("let x: number += 1")); // 声明后只允许 =
        assert!(program_fails("let a[0] = 1")); // 仅普通变量
        // let/const 是保留字，不能再作标识符
        assert!(program_fails("let = 1"));
        assert!(program_fails("x = let"));
    }

    #[test]
    fn type_annotation_on_assign() {
        // x: T = v → Assign { ann: Some(T) }
        match program("x: number = 1") {
            Stmt::Block { ref stmts, .. } => match &stmts[0] {
                Stmt::Assign { target, op, value, ann, .. } => {
                    assert!(matches!(target, Expr::Ident(..)));
                    assert!(matches!(op, AssignOp::Set));
                    assert!(matches!(value, Expr::Num(1.0, _)));
                    assert_eq!(ty_str(&ann).as_deref(), Some("number"));
                }
                other => panic!("expected Assign, got {:?}", other),
            },
            other => panic!("expected Block, got {:?}", other),
        }
        // 泛型参数与 [] 后缀
        for (src, ty) in [
            ("m: Map<string, number> = new Map()", "Map<string, number>"),
            ("a: number[] = []", "number[]"),
            ("g: Grid<number[][]> = g", "Grid<number[][]>"),
            ("m: Map<string, number[]> = m", "Map<string, number[]>"),
        ] {
            match program(src) {
                Stmt::Block { stmts, .. } => match &stmts[0] {
                    Stmt::Assign { ann, .. } => assert_eq!(ty_str(&ann).as_deref(), Some(ty)),
                    other => panic!("{}: expected Assign, got {:?}", src, other),
                },
                other => panic!("expected Block, got {:?}", other),
            }
        }
        // 普通赋值不带标注
        assert!(matches!(&program("x = 1"), Stmt::Block { stmts, .. }
            if matches!(&stmts[0], Stmt::Assign { ann: None, .. })));
        // 三元/对象字面量的冒号不受影响
        assert!(matches!(program("y = a ? b : c"), Stmt::Block { stmts, .. }
            if matches!(&stmts[0], Stmt::Assign { value: Expr::Ternary { .. }, ann: None, .. })));
        assert!(matches!(program("o = {x: 1}"), Stmt::Block { stmts, .. }
            if matches!(&stmts[0], Stmt::Assign { ann: None, .. })));
    }

    #[test]
    fn type_annotation_errors() {
        assert!(program_fails("x: number")); // 缺 '='，纯声明不成句
        assert!(program_fails("x: = 1")); // 类型名缺失
        assert!(program_fails("a[0]: number = 1")); // 仅普通变量可标注
        assert!(program_fails("p.x: number = 1"));
        assert!(program_fails("1: number = 1"));
        assert!(program_fails("x: Map<string = 1")); // 泛型未闭合
        assert!(program_fails("x: number += 1")); // 标注后只允许 '='
    }

    #[test]
    fn function_annotations() {
        match program("function add(a: number, b: number) -> number {\n    return a + b\n}") {
            Stmt::Block { stmts, .. } => match &stmts[0] {
                Stmt::FuncDecl { name, params, ret, .. } => {
                    assert_eq!(name, "add");
                    assert_eq!(params.len(), 2);
                    assert_eq!(params[0].name, "a");
                    assert_eq!(ty_str(&params[0].ty).as_deref(), Some("number"));
                    assert_eq!(ty_str(&params[1].ty).as_deref(), Some("number"));
                    assert_eq!(ty_str(&ret).as_deref(), Some("number"));
                }
                other => panic!("expected FuncDecl, got {:?}", other),
            },
            other => panic!("expected Block, got {:?}", other),
        }
        // 部分参数标注
        match program("function f(a, b: bool) {}") {
            Stmt::Block { stmts, .. } => match &stmts[0] {
                Stmt::FuncDecl { params, ret, .. } => {
                    assert_eq!(params[0].name, "a");
                    assert!(params[0].ty.is_none());
                    assert_eq!(ty_str(&params[1].ty).as_deref(), Some("bool"));
                    assert!(ret.is_none());
                }
                other => panic!("expected FuncDecl, got {:?}", other),
            },
            other => panic!("expected Block, got {:?}", other),
        }
        // impl 方法：参数 + 返回类型
        match program("impl P {\n    function len(self: P) -> number { return 1 }\n}") {
            Stmt::Block { stmts, .. } => match &stmts[0] {
                Stmt::Impl { methods, .. } => match &methods[0] {
                    Stmt::FuncDecl { ret, params, .. } => {
                        assert_eq!(ty_str(&ret).as_deref(), Some("number"));
                        assert_eq!(ty_str(&params[0].ty).as_deref(), Some("P"));
                    }
                    other => panic!("expected FuncDecl, got {:?}", other),
                },
                other => panic!("expected Impl, got {:?}", other),
            },
            other => panic!("expected Block, got {:?}", other),
        }
        // 匿名函数
        match expr("function(x: number) -> number { return x }") {
            Expr::Function { params, ret, .. } => {
                assert_eq!(ty_str(&params[0].ty).as_deref(), Some("number"));
                assert_eq!(ty_str(&ret).as_deref(), Some("number"));
            }
            other => panic!("expected Function, got {:?}", other),
        }
        // interface 签名：完整保留（名字 + 标注），仍不允许函数体
        assert!(matches!(
            program("interface Shape {\n    function area() -> number\n}"),
            Stmt::Block { stmts, .. }
                if matches!(&stmts[0], Stmt::Interface { methods, .. }
                    if methods.len() == 1
                        && methods[0].name == "area"
                        && ty_str(&methods[0].ret).as_deref() == Some("number"))
        ));
        assert!(program_fails("interface Shape {\n    function area() -> number { return 1 }\n}"));
    }

    #[test]
    fn struct_field_annotations() {
        match program("struct Point {\n    x: number,\n    y: number,\n    label: string,\n}") {
            Stmt::Block { stmts, .. } => match &stmts[0] {
                Stmt::Struct { name, fields, .. } => {
                    assert_eq!(name, "Point");
                    assert_eq!(fields.len(), 3);
                    assert_eq!(fields[0].name, "x");
                    assert_eq!(ty_str(&fields[0].ty).as_deref(), Some("number"));
                    assert_eq!(ty_str(&fields[1].ty).as_deref(), Some("number"));
                    assert_eq!(ty_str(&fields[2].ty).as_deref(), Some("string"));
                }
                other => panic!("expected Struct, got {:?}", other),
            },
            other => panic!("expected Block, got {:?}", other),
        }
    }

    // ============ struct / impl / interface / is ============

    #[test]
    fn struct_declaration_forms() {
        // 逗号分隔 + 尾逗号
        match program("struct Point {\n    x,\n    y,\n}") {
            Stmt::Block { stmts, .. } => match &stmts[0] {
                Stmt::Struct { name, fields, .. } => {
                    assert_eq!(name, "Point");
                    assert_eq!(
                        fields.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(),
                        vec!["x", "y"]
                    );
                    assert!(fields.iter().all(|f| f.ty.is_none()));
                }
                other => panic!("expected Struct, got {:?}", other),
            },
            other => panic!("expected Block, got {:?}", other),
        }
        // 换行分隔（无逗号）
        assert!(matches!(
            program("struct P {\n    a\n    b\n}"),
            Stmt::Block { stmts, .. } if matches!(&stmts[0], Stmt::Struct { fields, .. } if fields.len() == 2)
        ));
        // 单行 + 空体
        assert!(matches!(
            program("struct P { a, b }"),
            Stmt::Block { stmts, .. } if matches!(&stmts[0], Stmt::Struct { fields, .. } if fields.len() == 2)
        ));
        assert!(matches!(
            program("struct Empty {}"),
            Stmt::Block { stmts, .. } if matches!(&stmts[0], Stmt::Struct { fields, .. } if fields.is_empty())
        ));
    }

    #[test]
    fn impl_block_methods() {
        let src = "impl Point {\n    function new(x, y) {\n        self.x = x\n    }\n    function len() {\n        return 1\n    }\n}";
        match program(src) {
            Stmt::Block { stmts, .. } => match &stmts[0] {
                Stmt::Impl { target, methods, .. } => {
                    assert_eq!(target, "Point");
                    assert_eq!(methods.len(), 2);
                    assert!(matches!(&methods[0], Stmt::FuncDecl { name, params, .. }
                        if name == "new" && params.len() == 2));
                    assert!(matches!(&methods[1], Stmt::FuncDecl { name, .. } if name == "len"));
                }
                other => panic!("expected Impl, got {:?}", other),
            },
            other => panic!("expected Block, got {:?}", other),
        }
    }

    #[test]
    fn interface_signatures() {
        let src = "interface Shape {\n    function area()\n    function name()\n}";
        match program(src) {
            Stmt::Block { stmts, .. } => match &stmts[0] {
                Stmt::Interface { name, methods, .. } => {
                    assert_eq!(name, "Shape");
                    let names: Vec<&str> = methods.iter().map(|m| m.name.as_str()).collect();
                    assert_eq!(names, vec!["area", "name"]);
                }
                other => panic!("expected Interface, got {:?}", other),
            },
            other => panic!("expected Block, got {:?}", other),
        }
        // 空接口、逗号分隔
        assert!(matches!(
            program("interface Nothing {}"),
            Stmt::Block { stmts, .. } if matches!(&stmts[0], Stmt::Interface { methods, .. } if methods.is_empty())
        ));
        assert!(matches!(
            program("interface A { function f(), function g() }"),
            Stmt::Block { stmts, .. } if matches!(&stmts[0], Stmt::Interface { methods, .. } if methods.len() == 2)
        ));
    }

    #[test]
    fn is_operator_precedence() {
        // p is Shape && q → (p is Shape) && q
        let e = expr("p is Shape && q");
        match e {
            Expr::Logic { op: LogicOp::And, left, .. } => {
                assert!(matches!(*left, Expr::Is { .. }));
            }
            other => panic!("expected Logic(And), got {:?}", other),
        }
        // p is T == true → (p is T) == true
        match expr("p is T == true") {
            Expr::Binary { op: BinaryOp::Eq, left, .. } => {
                assert!(matches!(*left, Expr::Is { .. }));
            }
            other => panic!("expected Binary(Eq), got {:?}", other),
        }
        // a is B is C → 左结合链
        match expr("a is B is C") {
            Expr::Is { operand, .. } => {
                assert!(matches!(*operand, Expr::Is { .. }));
            }
            other => panic!("expected nested Is, got {:?}", other),
        }
        // 右侧是普通标识符/成员链
        assert!(matches!(expr("p is Shape"), Expr::Is { .. }));
        assert!(matches!(expr("p is ns.Shape"), Expr::Is { .. }));
        // is 不再是合法变量名（关键字）
        assert!(program_fails("is = 1"));
    }

    #[test]
    fn struct_impl_interface_errors() {
        // interface 方法不能带函数体
        assert!(program_fails("interface A { function f() { return 1 } }"));
        // impl 体只能包含 function 声明
        assert!(program_fails("impl P { x = 1 }"));
        // 缺字段名
        assert!(program_fails("struct P { , }"));
        // 未闭合
        assert!(program_fails("struct P { x"));
        assert!(program_fails("impl P {"));
        assert!(program_fails("interface P {"));
    }
}
