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

/// 参数 / struct 字段：名字 + 可选类型标注。
/// 标注是纯文档性质（为清晰与将来的类型推导保留），运行时完全不检查。
#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub name: String,
    pub ty: Option<String>,
}

impl Param {
    /// 提取名字列表（丢弃类型标注——运行时只需要名字）
    pub fn names(params: &[Param]) -> Vec<String> {
        params.iter().map(|p| p.name.clone()).collect()
    }
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
    /// 切片 `a[start..end]`（端点可省略，`..=` 含 end）；仅数组/字符串
    Slice {
        target: Box<Expr>,
        start: Option<Box<Expr>>,
        end: Option<Box<Expr>>,
        inclusive: bool,
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
    /// `new MaxHeap(args)`；原生类或用户 struct
    New {
        class: Box<Expr>,
        args: Vec<Expr>,
        span: Span,
    },
    /// `v is T`：T 为 struct → 名义检查；为 interface → 结构化检查
    Is {
        operand: Box<Expr>,
        target: Box<Expr>,
        span: Span,
    },
    /// 匿名函数 `function(x) { ... }`；params 可带 `: T` 标注，`-> T` 标注返回类型
    Function {
        params: Vec<Param>,
        ret: Option<String>,
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
            | Expr::Slice { span, .. }
            | Expr::Member { span, .. }
            | Expr::OptionalMember { span, .. }
            | Expr::Call { span, .. }
            | Expr::New { span, .. }
            | Expr::Is { span, .. }
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
        /// `x: T = v` 的类型标注（仅普通变量的首次标注赋值非 None，运行时忽略）
        ann: Option<String>,
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
        params: Vec<Param>,
        /// `-> T` 返回类型标注（运行时忽略）
        ret: Option<String>,
        body: Rc<Stmt>,
        span: Span,
    },
    /// `struct Point { x, y }`：字段声明（实例字段默认 null），可带 `: T` 标注
    Struct {
        name: String,
        fields: Vec<Param>,
        span: Span,
    },
    /// `impl Point { function ... }`：methods 全部为 FuncDecl（parser 保证），
    /// 执行时逐个挂到 struct 的方法表
    Impl {
        target: String,
        methods: Vec<Stmt>,
        span: Span,
    },
    /// `interface Shape { function area() }`：方法名签名（无体，纯声明）
    Interface {
        name: String,
        methods: Vec<String>,
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
            | Stmt::Struct { span, .. }
            | Stmt::Impl { span, .. }
            | Stmt::Interface { span, .. }
            | Stmt::Return { span, .. }
            | Stmt::Break(span)
            | Stmt::Continue(span)
            | Stmt::Block { span, .. } => *span,
        }
    }
}
