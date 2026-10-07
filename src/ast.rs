use std::rc::Rc;

use crate::Span;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Not,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Lt,
    Gt,
    Lte,
    Gte,
    Eq,
    Neq,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogicOp {
    And,
    Or,
    /// `??`：仅 null 触发回退
    Nullish,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignOp {
    Set,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    /// `??=`：旧值为 null 才赋值
    Nullish,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Num(f64, Span),
    Str(String, Span),
    Bool(bool, Span),
    Null(Span),
    Ident(String, Span),
    Array(Vec<Expr>, Span),
    Object(Vec<(String, Expr)>, Span),
    Unary {
        op: UnaryOp,
        operand: Box<Expr>,
        span: Span,
    },
    Binary {
        left: Box<Expr>,
        op: BinaryOp,
        right: Box<Expr>,
        span: Span,
    },
    /// `&&` / `||` / `??`（短路，返回操作数）
    Logic {
        left: Box<Expr>,
        op: LogicOp,
        right: Box<Expr>,
        span: Span,
    },
    /// `cond ? a : b`（TS 习惯，右结合）
    Ternary {
        cond: Box<Expr>,
        then_expr: Box<Expr>,
        else_expr: Box<Expr>,
        span: Span,
    },
    Index {
        target: Box<Expr>,
        index: Box<Expr>,
        span: Span,
    },
    Member {
        target: Box<Expr>,
        name: String,
        span: Span,
    },
    /// `p?.x`
    OptionalMember {
        target: Box<Expr>,
        name: String,
        span: Span,
    },
    Call {
        callee: Box<Expr>,
        args: Vec<Expr>,
        span: Span,
    },
    /// `new MaxHeap(args)`；v1 仅原生类
    New {
        class: Box<Expr>,
        args: Vec<Expr>,
        span: Span,
    },
    /// 匿名函数 `function(x) { ... }`
    Function {
        params: Vec<String>,
        body: Rc<Stmt>,
        span: Span,
    },
}

impl Expr {
    pub fn span(&self) -> Span {
        match self {
            Expr::Num(_, span)
            | Expr::Str(_, span)
            | Expr::Bool(_, span)
            | Expr::Null(span)
            | Expr::Ident(_, span)
            | Expr::Array(_, span)
            | Expr::Object(_, span)
            | Expr::Unary { span, .. }
            | Expr::Binary { span, .. }
            | Expr::Logic { span, .. }
            | Expr::Ternary { span, .. }
            | Expr::Index { span, .. }
            | Expr::Member { span, .. }
            | Expr::OptionalMember { span, .. }
            | Expr::Call { span, .. }
            | Expr::New { span, .. }
            | Expr::Function { span, .. } => *span,
        }
    }
}

/// for-in 的迭代目标：数组/字符串，或 `a..b` / `a..=b` 区间
#[derive(Debug, Clone, PartialEq)]
pub enum ForIter {
    Range {
        start: Expr,
        end: Expr,
        inclusive: bool,
    },
    Expr(Expr),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    Expr(Expr, Span),
    Assign {
        target: Expr,
        op: AssignOp,
        value: Expr,
        span: Span,
    },
    If {
        cond: Expr,
        then_block: Box<Stmt>,
        else_block: Option<Box<Stmt>>,
        span: Span,
    },
    While {
        cond: Expr,
        body: Box<Stmt>,
        span: Span,
    },
    ForIn {
        var: String,
        iter: ForIter,
        body: Box<Stmt>,
        span: Span,
    },
    /// C 风格：`for i = 0; i < n; i += 1 { ... }`
    ForC {
        var: String,
        init: Expr,
        cond: Option<Expr>,
        step: Option<Box<Stmt>>,
        body: Box<Stmt>,
        span: Span,
    },
    FuncDecl {
        name: String,
        params: Vec<String>,
        body: Rc<Stmt>,
        span: Span,
    },
    Return {
        value: Option<Expr>,
        span: Span,
    },
    Break(Span),
    Continue(Span),
    Block {
        stmts: Vec<Stmt>,
        span: Span,
    },
}

impl Stmt {
    pub fn span(&self) -> Span {
        match self {
            Stmt::Expr(_, span)
            | Stmt::Assign { span, .. }
            | Stmt::If { span, .. }
            | Stmt::While { span, .. }
            | Stmt::ForIn { span, .. }
            | Stmt::ForC { span, .. }
            | Stmt::FuncDecl { span, .. }
            | Stmt::Return { span, .. }
            | Stmt::Break(span)
            | Stmt::Continue(span)
            | Stmt::Block { span, .. } => *span,
        }
    }
}
