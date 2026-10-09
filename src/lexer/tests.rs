//! 词法分析器单元测试（从 lexer.rs 原样迁出）。

use super::*;
use crate::RtResult;
use crate::lexer::token::Keyword;
use TokenKind::*;

fn kinds(src: &str) -> RtResult<Vec<TokenKind>> {
    Ok(Lexer::new(src).tokenize()?.tokens.into_iter().map(|t| t.kind).collect())
}

#[test]
fn simple_statements_with_eol() {
    let ks = kinds("x = 1\ny = 2").unwrap();
    // 结尾无换行时 EOF 直接作终止符，不再补 Eol
    assert_eq!(
        ks,
        vec![
            Ident("x".into()),
            Assign,
            Num(1.0),
            Eol,
            Ident("y".into()),
            Assign,
            Num(2.0),
            Eof
        ]
    );
}

#[test]
fn multi_char_operators() {
    let ks = kinds("a <= b != c && d || !e").unwrap();
    assert_eq!(
        ks[..8],
        [
            Ident("a".into()),
            Lte,
            Ident("b".into()),
            Neq,
            Ident("c".into()),
            AndAnd,
            Ident("d".into()),
            OrOr
        ]
    );
}

#[test]
fn range_tokens_not_eaten_by_number() {
    let ks = kinds("0..n").unwrap();
    assert_eq!(ks[..3], [Num(0.0), DotDot, Ident("n".into())]);

    let ks = kinds("0..=n").unwrap();
    assert_eq!(ks[..3], [Num(0.0), DotDotEq, Ident("n".into())]);
}

#[test]
fn compound_assignments() {
    let ks = kinds("i += 1\nj -= 2\nk *= 3\nm /= 4\nn %= 5").unwrap();
    let ops: Vec<&TokenKind> = ks.iter().filter(|k| matches!(k, AddAssign | SubAssign | MulAssign | DivAssign | ModAssign)).collect();
    assert_eq!(ops, vec![&AddAssign, &SubAssign, &MulAssign, &DivAssign, &ModAssign]);
}

#[test]
fn string_escapes_both_quotes() {
    let ks = kinds("\"a\\n\\\"b\\\"\"").unwrap();
    assert_eq!(ks[0], Str("a\n\"b\"".into()));

    let ks = kinds("'it\\'s'").unwrap();
    assert_eq!(ks[0], Str("it's".into()));
}

#[test]
fn comments_are_skipped() {
    // 注释后的换行：文件尚未发出任何 token（last=None），Eol 被抑制
    let ks = kinds("// hi\nx = 1").unwrap();
    assert_eq!(ks[..3], [Ident("x".into()), Assign, Num(1.0)]);

    let ks = kinds("y = 2 /* multi\nline */ + 1").unwrap();
    assert_eq!(
        ks[..5],
        [Ident("y".into()), Assign, Num(2.0), Plus, Num(1.0)]
    );
}

#[test]
fn eol_suppressed_inside_parens_and_brackets() {
    // 括号/方括号内换行不产生 Eol
    let ks = kinds("f(1,\n  2)").unwrap();
    assert!(!ks[..].contains(&Eol), "no Eol inside parens: {:?}", ks);

    let ks = kinds("[1,\n2]").unwrap();
    assert!(!ks[..].contains(&Eol));

    // 花括号（对象字面量）是语句块的世界：仅 `{` 后与逗号后的换行被抑制
    let ks = kinds("{\n  x: 1,\n  y: 2\n}").unwrap();
    let eols = ks.iter().filter(|k| **k == Eol).count();
    assert_eq!(eols, 1, "{:?}", ks); // 只在 `2` 之后（值结束时）发 Eol
}

#[test]
fn eol_suppressed_after_operator() {
    let ks = kinds("a +\nb").unwrap();
    assert!(!ks[..].contains(&Eol), "{:?}", ks);

    let ks = kinds("x = a ||\nb").unwrap();
    assert!(!ks[..].contains(&Eol), "{:?}", ks);
}

#[test]
fn eol_suppressed_before_leading_dot_chain() {
    let ks = kinds("arr\n  .map(f)\n  .len()").unwrap();
    assert!(!ks[..].contains(&Eol), "{:?}", ks);
    assert!(ks.contains(&Dot));
}

#[test]
fn leading_dot_float() {
    let ks = kinds("x = .5").unwrap();
    assert_eq!(ks[..3], [Ident("x".into()), Assign, Num(0.5)]);

    // 但值后面的 `.` 仍是成员访问
    let ks = kinds("a.b").unwrap();
    assert_eq!(ks[..3], [Ident("a".into()), Dot, Ident("b".into())]);
}

#[test]
fn optional_chaining_token() {
    let ks = kinds("p?.x").unwrap();
    assert_eq!(ks[..3], [Ident("p".into()), QuestionDot, Ident("x".into())]);
}

#[test]
fn nullish_coalescing_tokens() {
    let ks = kinds("a ?? b").unwrap();
    assert_eq!(ks[..3], [Ident("a".into()), QuestionQuestion, Ident("b".into())]);

    // ??= 优先于 ?? 匹配（最长匹配）
    let ks = kinds("x ??= 1").unwrap();
    assert_eq!(ks[..3], [Ident("x".into()), QuestionQuestionAssign, Num(1.0)]);

    // 与 ?. / 三元互不干扰
    let ks = kinds("p?.x ?? q ??= r").unwrap();
    assert_eq!(ks[1], QuestionDot);
    assert_eq!(ks[3], QuestionQuestion);
    assert_eq!(ks[5], QuestionQuestionAssign);

    // 运算符后的换行被抑制（可跨行书写）
    let ks = kinds("a ??\nb").unwrap();
    assert!(!ks.contains(&Eol));
}

#[test]
fn arrow_token() {
    // `->` 是独立 token（函数返回类型标注用）
    let ks = kinds("function f() -> number {}").unwrap();
    assert_eq!(ks[4], Arrow);
    assert_eq!(ks[5], Ident("number".into()));
    // 与减号 / 大于号不混淆
    let ks = kinds("a - b").unwrap();
    assert_eq!(ks[1], Minus);
    let ks = kinds("a >= b").unwrap();
    assert_eq!(ks[1], Gte);
    let ks = kinds("a > b").unwrap();
    assert_eq!(ks[1], Gt);
}

#[test]
fn ternary_tokens() {
    let ks = kinds("a ? b : c").unwrap();
    assert_eq!(ks[1], Question);
    assert_eq!(ks[3], Colon);
}

#[test]
fn pipe_token() {
    // `|` 单独成 token（类型联合用）；`||` 仍是逻辑或（最长匹配优先）
    let ks = kinds("a | b").unwrap();
    assert_eq!(ks[..3], [Ident("a".into()), Pipe, Ident("b".into())]);
    let ks = kinds("a || b").unwrap();
    assert_eq!(ks[..3], [Ident("a".into()), OrOr, Ident("b".into())]);
    let ks = kinds("a |=").unwrap();
    assert_eq!(ks[1], Pipe); // 不存在 |=，`=` 独立
    assert_eq!(ks[2], Assign);
}

#[test]
fn let_const_keywords() {
    let ks = kinds("let x = 1").unwrap();
    assert_eq!(ks[0], Keyword(Keyword::Let));
    let ks = kinds("const y = 2").unwrap();
    assert_eq!(ks[0], Keyword(Keyword::Const));
    // 不再是普通标识符
    let ks = kinds("let").unwrap();
    assert_eq!(ks[0], Keyword(Keyword::Let));
}

#[test]
fn keywords_vs_identifiers() {
    let ks = kinds("function foo() {}").unwrap();
    assert_eq!(ks[0], TokenKind::Keyword(Keyword::Function));
    assert_eq!(ks[1], Ident("foo".into()));

    let ks = kinds("functionx").unwrap();
    assert_eq!(ks[0], Ident("functionx".into()));
}

#[test]
fn exponent_numbers() {
    let ks = kinds("1e3").unwrap();
    assert_eq!(ks[0], Num(1000.0));

    let ks = kinds("2.5e-2").unwrap();
    assert_eq!(ks[0], Num(0.025));
}

#[test]
fn semicolon_is_separator() {
    let ks = kinds("x = 1; y = 2").unwrap();
    assert_eq!(ks[3], Semi);
}

#[test]
fn error_cases() {
    assert!(kinds("\"abc").is_err()); // unterminated
    assert!(kinds("\"a\nb\"").is_err()); // 换行未转义
    assert!(kinds("#").is_err()); // illegal char
    assert!(kinds("\"a\\q\"").is_err()); // bad escape
    assert!(kinds("1e").is_err()); // exponent 无数字
    assert!(kinds("/* no end").is_err()); // unterminated block comment
}

#[test]
fn unclosed_depth_reported_for_repl() {
    let out = Lexer::new("f(1, [2,").tokenize().unwrap();
    assert_eq!(out.unclosed_depth, 2);

    let out = Lexer::new("x = 1").tokenize().unwrap();
    assert_eq!(out.unclosed_depth, 0);
}

#[test]
fn spans_track_lines() {
    let out = Lexer::new("x = 1\ny = 2").tokenize().unwrap();
    let y_tok = &out.tokens[4];
    assert_eq!(y_tok.span.line, 2);
    assert_eq!(y_tok.span.col, 1);
}
