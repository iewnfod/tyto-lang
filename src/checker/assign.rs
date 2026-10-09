//! 赋值检查：let/const/标注声明、链上更新、复合赋值结果类型、成员写入。

use crate::ast::{AssignOp, DeclKind, Expr, TypeAst};
use crate::Span;

use super::ty;
use super::ty::{assignable, from_ast, normalize_union, Type};
use super::{Binding, Checker};

impl Checker {
    // ============ 赋值检查 ============

    pub(crate) fn check_assign(
        &mut self,
        target: &Expr,
        op: AssignOp,
        value: &Expr,
        ann: Option<&TypeAst>,
        decl: Option<DeclKind>,
        span: Span,
    ) {
        match target {
            Expr::Ident(name, _) => {
                let value_ty = self.infer(value);
                match decl {
                    Some(kind) => self.check_declared_assign(name, kind, ann, value_ty, value.span(), span),
                    None => {
                        if let Some(ann) = ann {
                            // `x: T = v`（无 let）：等价声明
                            self.check_declared_assign(
                                name,
                                DeclKind::Let,
                                Some(ann),
                                value_ty,
                                value.span(),
                                span,
                            );
                            return;
                        }
                        // 普通赋值：写路径只约束「有显式类型的绑定」（渐进承诺）；
                        // 无约束绑定跟踪最新推导类型（读路径：成员访问/索引检查用）
                        let existing = self.lookup(name).cloned();
                        match existing {
                            Some(b) if b.is_const => {
                                let where_ = b
                                    .declared
                                    .map(|s| format!("（声明于第 {} 行）", s.line))
                                    .unwrap_or_default();
                                self.err(
                                    span,
                                    format!("不能给常量 `{}` 赋值{}", name, where_),
                                );
                            }
                            Some(b) if b.declared.is_some() => {
                                self.check_write_to_typed(&b, name, op, &value_ty, span);
                            }
                            Some(_) => {
                                // 无约束：更新类型（最新胜出，与运行时链上一致），不报错
                                let new_ty = self.compound_result_ty(op, &value_ty, span);
                                self.update_binding_ty(name, new_ty);
                            }
                            None => {
                                let new_ty = self.compound_result_ty(op, &value_ty, span);
                                self.define(name, Binding {
                                    ty: new_ty,
                                    is_const: false,
                                    declared: None,
                                    let_decl: None,
                                    func: None,
                                });
                            }
                        }
                    }
                }
            }
            Expr::Index { target, .. } => {
                let recv = self.infer(target);
                let v = self.infer(value);
                if op == AssignOp::Set {
                    if let Type::Array(elem) = &recv {
                        if !assignable(&v, elem) {
                            self.err(
                                span,
                                format!(
                                    "数组元素类型不匹配：期望 `{}`，实际 `{}`",
                                    elem.display(),
                                    v.display()
                                ),
                            );
                        }
                    }
                }
            }
            Expr::Member { target, name, .. } => {
                let recv = self.infer(target);
                let v = self.infer(value);
                if let Some(field_ty) = self.member_type_for_write(&recv, name) {
                    if op == AssignOp::Set && !assignable(&v, &field_ty) {
                        self.err(
                            span,
                            format!(
                                "字段类型不匹配：`{}` 期望 `{}`，实际 `{}`",
                                name,
                                field_ty.display(),
                                v.display()
                            ),
                        );
                    }
                }
            }
            _ => {}
        }
    }

    /// let / const / 标注声明：定义绑定 + 初始值检查
    fn check_declared_assign(
        &mut self,
        name: &str,
        kind: DeclKind,
        ann: Option<&TypeAst>,
        value_ty: Type,
        value_span: Span,
        span: Span,
    ) {
        // 同层重复声明提示（此前已有 let/const 声明）
        if let Some(old) = self.scopes.last().unwrap().get(name) {
            if let Some(first) = old.let_decl.or(old.declared) {
                self.warn(
                    span,
                    format!("`{}` 在同一作用域重复声明（首次声明于第 {} 行）", name, first.line),
                );
            }
        }
        match ann {
            Some(ann) => {
                let ann_ty = self.ann_ty(ann);
                if !assignable(&value_ty, &ann_ty) {
                    let msg = self.mismatch(&ann_ty, &value_ty, Some(ann.span()));
                    let _ = value_span;
                    self.err(span, msg);
                }
                self.define(name, Binding {
                    ty: ann_ty,
                    is_const: kind == DeclKind::Const,
                    declared: Some(ann.span()),
                    let_decl: Some(span),
                    func: None,
                });
            }
            None => {
                // 无标注：let 跟踪初始值类型（读路径用，写入不约束）；const 固定初始值类型
                let ty = value_ty;
                self.define(name, Binding {
                    ty,
                    is_const: kind == DeclKind::Const,
                    declared: if kind == DeclKind::Const { Some(span) } else { None },
                    let_decl: Some(span),
                    func: None,
                });
            }
        }
    }

    /// 对已有类型约束的绑定做写入检查（含复合赋值的结果类型）
    fn check_write_to_typed(&mut self, b: &Binding, name: &str, op: AssignOp, value_ty: &Type, span: Span) {
        let expected = b.ty.clone();
        let ok = if op == AssignOp::Set {
            assignable(value_ty, &expected)
        } else {
            // 复合赋值：先按运算规则算结果，再看能否写回
            let result = match op {
                AssignOp::Add => ty::arith_result("+", &expected, value_ty),
                AssignOp::Sub => ty::arith_result("-", &expected, value_ty),
                AssignOp::Mul => ty::arith_result("*", &expected, value_ty),
                AssignOp::Div => ty::arith_result("/", &expected, value_ty),
                AssignOp::Mod => ty::arith_result("%", &expected, value_ty),
                AssignOp::Nullish => {
                    // x ??= v：结果 = without_null(expected) ∪ v
                    Ok(normalize_union(vec![ty::without_null(&expected), value_ty.clone()]))
                }
                AssignOp::Set => unreachable!(),
            };
            match result {
                Ok(res) => assignable(&res, &expected),
                Err(()) => false,
            }
        };
        if !ok {
            let msg = self.mismatch(&expected, value_ty, b.declared);
            let _ = name;
            self.err(span, msg);
        }
    }

    /// 写入路径的成员类型（struct 字段 / 对象形状；方法不可写）
    fn member_type_for_write(&mut self, recv: &Type, name: &str) -> Option<Type> {
        match recv {
            Type::Struct(sname, args) => {
                let info = self.structs.get(sname)?;
                let field = info.fields.iter().find(|f| f.name == name)?;
                let generics = self.structs.get(sname).map(|s| s.type_params.clone())?;
                let raw = field.ty.as_ref().map(|a| from_ast(a, &generics, &self.structs))?;
                Some(self.subst_struct(&raw, sname, args))
            }
            Type::Object(fields) => {
                fields.iter().find(|(n, _)| n == name).map(|(_, t)| t.clone())
            }
            _ => None,
        }
    }
}
