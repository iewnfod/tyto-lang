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

/// 声明关键字：`let x = v` / `const x = v`（普通赋值为 None）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclKind {
    Let,
    Const,
}

/// 结构化类型标注（渐进类型系统的语法表示）。
///
/// 运行时擦除（解释器只认值不认类型）；编辑器分析与 checker 消费。
/// `Display` 产出规范串（`Map<string, number>`、`number | null`），
/// 与源码书写风格无关。
#[derive(Debug, Clone, PartialEq)]
pub enum TypeAst {
    /// 基础/命名类型：`number`、`Point`、泛型参数 `T`
    Named(String, Span),
    /// 数组后缀：`T[]`
    Array(Box<TypeAst>, Span),
    /// 泛型应用：`Array<T>`、`Map<K, V>`、`Box<T>`
    Generic(String, Vec<TypeAst>, Span),
    /// 联合：`A | B | C`
    Union(Vec<TypeAst>, Span),
    /// 结构化对象类型：`{x: number, y: string}`
    Object(Vec<(String, TypeAst)>, Span),
    /// 函数类型：`(a: number, b: string) -> bool`（ret None = 无返回标注）
    Func {
        params: Vec<(String, TypeAst)>,
        ret: Option<Box<TypeAst>>,
        span: Span,
    },
}

impl TypeAst {
    pub fn span(&self) -> Span {
        match self {
            TypeAst::Named(_, s)
            | TypeAst::Array(_, s)
            | TypeAst::Generic(_, _, s)
            | TypeAst::Union(_, s)
            | TypeAst::Object(_, s)
            | TypeAst::Func { span: s, .. } => *s,
        }
    }
}

impl std::fmt::Display for TypeAst {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TypeAst::Named(n, _) => write!(f, "{}", n),
            TypeAst::Array(t, _) => {
                // 联合/函数类型作元素时补括号，保证回显不变形（`(A | B)[]`）
                if matches!(**t, TypeAst::Union(..) | TypeAst::Func { .. }) {
                    write!(f, "({})[]", t)
                } else {
                    write!(f, "{}[]", t)
                }
            }
            TypeAst::Generic(n, args, _) => {
                write!(f, "{}<", n)?;
                for (i, a) in args.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", a)?;
                }
                write!(f, ">")
            }
            TypeAst::Union(ms, _) => {
                for (i, m) in ms.iter().enumerate() {
                    if i > 0 {
                        write!(f, " | ")?;
                    }
                    write!(f, "{}", m)?;
                }
                Ok(())
            }
            TypeAst::Object(fields, _) => {
                write!(f, "{{")?;
                for (i, (n, t)) in fields.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}: {}", n, t)?;
                }
                write!(f, "}}")
            }
            TypeAst::Func { params, ret, .. } => {
                write!(f, "(")?;
                for (i, (n, t)) in params.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}: {}", n, t)?;
                }
                write!(f, ")")?;
                if let Some(r) = ret {
                    write!(f, " -> {}", r)?;
                }
                Ok(())
            }
        }
    }
}

/// 参数 / struct 字段：名字 + 可选类型标注。
/// 运行时擦除（`Param::names` 只取名字）；类型检查在 checker（编辑器/check 期）。
#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub name: String,
    pub ty: Option<TypeAst>,
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
        /// 泛型参数声明 `function<T>(x: T) -> T`
        type_params: Vec<String>,
        ret: Option<TypeAst>,
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

/// interface 方法签名：名字 + 完整参数/返回类型（运行时 `is` 检查只看方法名）
#[derive(Debug, Clone, PartialEq)]
pub struct InterfaceMethod {
    pub name: String,
    pub params: Vec<Param>,
    pub ret: Option<TypeAst>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    Expr(Expr, Span),
    Assign {
        target: Expr,
        op: AssignOp,
        value: Expr,
        /// `x: T = v` / `let x: T = v` 的类型标注（None = 未标注）
        ann: Option<TypeAst>,
        /// `let` / `const` 声明（None = 普通赋值）
        decl: Option<DeclKind>,
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
        /// 泛型参数声明 `function id<T>(x: T) -> T`
        type_params: Vec<String>,
        /// `-> T` 返回类型标注
        ret: Option<TypeAst>,
        body: Rc<Stmt>,
        span: Span,
    },
    /// `struct Point { x, y }`：字段声明（实例字段默认 null），可带 `: T` 标注
    Struct {
        name: String,
        /// 泛型参数声明 `struct Box<T> { v: T }`
        type_params: Vec<String>,
        fields: Vec<Param>,
        span: Span,
    },
    /// `impl Point { function ... }`：methods 全部为 FuncDecl（parser 保证），
    /// 执行时逐个挂到 struct 的方法表。
    /// type_params：`impl TreeNode<T>` 声明的泛型参数（方法标注可引用；
    /// 名字约定与 struct 声明一致，v1 不做改名映射）
    Impl {
        target: String,
        type_params: Vec<String>,
        methods: Vec<Stmt>,
        span: Span,
    },
    /// `interface Shape { function area() -> number }`：方法签名（无体，纯声明）
    Interface {
        name: String,
        methods: Vec<InterfaceMethod>,
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
