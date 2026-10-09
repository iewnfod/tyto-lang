//! 类型标注语法（渐进类型系统：运行时擦除，checker 检查）。

use crate::ast::*;
use crate::lexer::{Keyword, TokenKind};
use crate::{RtError, RtResult};

use super::Parser;

impl Parser {
    // ============ 类型标注（渐进类型系统：运行时擦除，checker 检查） ============

    /// 类型语法（联合层）：`atom | atom | ...`
    pub(super) fn parse_type(&mut self) -> RtResult<TypeAst> {
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
    pub(super) fn parse_type_params(&mut self) -> RtResult<Vec<String>> {
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
    pub(super) fn parse_ret_ann(&mut self) -> RtResult<Option<TypeAst>> {
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
}
