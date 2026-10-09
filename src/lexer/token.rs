use crate::Span;

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    Num(f64),
    Str(String),
    Ident(String),
    Keyword(Keyword),

    // 赋值
    Assign,
    AddAssign,
    SubAssign,
    MulAssign,
    DivAssign,
    ModAssign,
    // 算术/比较/逻辑
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Lt,
    Gt,
    Lte,
    Gte,
    Eq,
    Neq,
    AndAnd,
    OrOr,
    Not,
    /// `|`：仅用于类型联合（`A | B`），不是位或运算符
    Pipe,
    // 其它符号
    Arrow,
    Dot,
    DotDot,
    DotDotEq,
    Question,
    QuestionDot,
    QuestionQuestion,
    QuestionQuestionAssign,
    Colon,
    Comma,
    Semi,
    // 括号
    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,
    // 语句分隔
    Eol,
    Eof,
}

impl TokenKind {
    /// 该类 token 是否可能是一个表达式/语句的结尾。
    /// EOL 抑制规则的核心：只有能结尾的 token 之后才发 Eol。
    pub fn can_end_expr(&self) -> bool {
        matches!(
            self,
            TokenKind::Num(_)
                | TokenKind::Str(_)
                | TokenKind::Ident(_)
                | TokenKind::Keyword(Keyword::True)
                | TokenKind::Keyword(Keyword::False)
                | TokenKind::Keyword(Keyword::Null)
                | TokenKind::RParen
                | TokenKind::RBracket
                | TokenKind::RBrace
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Keyword {
    Function,
    Return,
    If,
    Else,
    While,
    For,
    In,
    Break,
    Continue,
    True,
    False,
    Null,
    New,
    Struct,
    Impl,
    Interface,
    Is,
    Let,
    Const,
}

impl Keyword {
    pub fn from_str(s: &str) -> Option<Keyword> {
        Some(match s {
            "function" => Keyword::Function,
            "return" => Keyword::Return,
            "if" => Keyword::If,
            "else" => Keyword::Else,
            "while" => Keyword::While,
            "for" => Keyword::For,
            "in" => Keyword::In,
            "break" => Keyword::Break,
            "continue" => Keyword::Continue,
            "true" => Keyword::True,
            "false" => Keyword::False,
            "null" => Keyword::Null,
            "new" => Keyword::New,
            "struct" => Keyword::Struct,
            "impl" => Keyword::Impl,
            "interface" => Keyword::Interface,
            "is" => Keyword::Is,
            "let" => Keyword::Let,
            "const" => Keyword::Const,
            _ => return None,
        })
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Keyword::Function => "function",
            Keyword::Return => "return",
            Keyword::If => "if",
            Keyword::Else => "else",
            Keyword::While => "while",
            Keyword::For => "for",
            Keyword::In => "in",
            Keyword::Break => "break",
            Keyword::Continue => "continue",
            Keyword::True => "true",
            Keyword::False => "false",
            Keyword::Null => "null",
            Keyword::New => "new",
            Keyword::Struct => "struct",
            Keyword::Impl => "impl",
            Keyword::Interface => "interface",
            Keyword::Is => "is",
            Keyword::Let => "let",
            Keyword::Const => "const",
        }
    }
}
