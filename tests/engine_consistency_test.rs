//! 引擎单源守护：光标路径（walker 镜像链驱动 checker 引擎）与全文件检查
//! （`check_program` 的 TypeTable）对同一表达式必须给出同一类型。
//!
//! 这是镜像链模式的回归防线——两侧走查逻辑（作用域压栈/链上登记/
//! ambient 替换）一旦漂移，绑定类型与表达式类型会在此暴露。
//!
//! 已知的有意分歧不在守护范围：无标注函数的返回类型（光标路径经
//! `infer_func_ret` 推导，诊断路径只用标注——接入诊断属后续工作）。

use std::collections::HashMap;

use tyto_lang::analysis::complete;
use tyto_lang::analysis::ty_view::editor_display;
use tyto_lang::checker::check_program;
use tyto_lang::checker::ty::Type;
use tyto_lang::{Lexer, Parser, Span};

/// `§` 光标标记定位（0-based 行、UTF-16 列，与 analysis_test 的 at 一致）
fn at(src: &str) -> (String, usize, usize) {
    let cut = src.find('§').expect("测试源码需含 § 光标标记");
    let before = &src[..cut];
    let line = before.matches('\n').count();
    let line_start = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
    let col = before[line_start..].chars().map(|c| c.len_utf16()).sum::<usize>();
    (format!("{}{}", before, &src[cut + '§'.len_utf8()..]), line, col)
}

/// 子串首现处回算 1-based Span（表达式起点；Call 的 span = 后缀链起点，
/// 与 AST `expr.span()` 对齐）
fn span_of(src: &str, needle: &str) -> Span {
    let off = src.find(needle).unwrap_or_else(|| panic!("源码缺 {needle}"));
    span_at(src, off)
}

fn span_at(src: &str, off: usize) -> Span {
    let line = src[..off].matches('\n').count() + 1;
    let line_start = src[..off].rfind('\n').map(|i| i + 1).unwrap_or(0);
    Span::new(line, off - line_start + 1)
}

/// 全文件检查的表达式类型表（TypeTable）
fn type_table(src: &str) -> HashMap<Span, Type> {
    let out = Lexer::new(src).tokenize().unwrap();
    let program = Parser::new(out.tokens).parse_program().unwrap();
    check_program(&program).types
}

/// 光标路径：全局补全里某绑定的 detail（`名字: 类型`）
fn binding_detail(src_with_cursor: &str, label: &str) -> String {
    let (text, l, c) = at(src_with_cursor);
    complete(&text, l, c)
        .into_iter()
        .find(|i| i.label == label)
        .unwrap_or_else(|| panic!("补全缺绑定 {label}"))
        .detail
}

/// 断言：编辑器绑定的展示类型 == 全文件检查对 RHS 表达式的推导类型
fn assert_binding_matches_table(src_with_cursor: &str, label: &str, rhs: &str) {
    let (text, ..) = at(src_with_cursor);
    let table = type_table(&text);
    let t = table
        .get(&span_of(&text, rhs))
        .cloned()
        .unwrap_or_else(|| panic!("TypeTable 缺 {rhs} 的类型"));
    let want = format!("{}: {}", label, editor_display(&t));
    let got = binding_detail(src_with_cursor, label);
    assert_eq!(got, want, "光标路径与全文件检查对 `{label}` 的类型不一致");
}

#[test]
fn method_chain_binding_agrees_with_type_table() {
    assert_binding_matches_table(
        "s = \"a-b\"\nparts = s.split(\"-\")\n§",
        "parts",
        "s.split(\"-\")",
    );
}

#[test]
fn array_literal_binding_agrees_with_type_table() {
    assert_binding_matches_table("arr = [1, 2, 3]\n§", "arr", "[1, 2, 3]");
}

#[test]
fn param_member_chain_agrees_with_type_table() {
    // 函数体内：参数镜像登记 + 成员调用推导（镜像链的作用域走查防线）
    assert_binding_matches_table(
        "function work(xs: number[]) {\n    total = xs.len()\n    §\n}\n",
        "total",
        "xs.len()",
    );
}

#[test]
fn annotated_function_call_agrees_with_type_table() {
    let src = "function make() -> number[] {\n    return [1, 2]\n}\ng = make()\n§";
    let (text, ..) = at(src);
    let table = type_table(&text);
    // rfind 取调用点（find 会先命中第 1 行的函数声明）
    let call_off = text.rfind("make()").expect("缺调用点");
    let t = table
        .get(&span_at(&text, call_off))
        .cloned()
        .expect("TypeTable 缺 make() 调用类型");
    // 两侧同源（标注代入）：number[]
    assert_eq!(editor_display(&t), "number[]");
    let detail = binding_detail(src, "g");
    assert_eq!(detail, "g: number[]");
}
