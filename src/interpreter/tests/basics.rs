//! 基础语义：值与运算、作用域与闭包、控制流、数组/字符串切片、错误路径。

use super::*;

#[test]
fn print_and_println() {
    assert_eq!(run("print(\"hello\")"), "hello");
    assert_eq!(run("println(\"hello\")"), "hello\n");
    assert_eq!(run("println(1, \"a\", true)"), "1 a true\n");
    assert_eq!(run("print()"), "");
}

#[test]
fn arithmetic_and_js_number() {
    assert_eq!(run("println(1 + 2 * 3)"), "7\n");
    assert_eq!(run("println((1 + 2) / 2)"), "1.5\n");
    assert_eq!(run("println(7 % 3)"), "1\n");
    assert_eq!(run("println(1 / 0)"), "inf\n");
    assert_eq!(run("println(0 / 0)"), "nan\n");
    assert_eq!(run("println(2.0)"), "2\n");
    assert_eq!(run("println(-3)"), "-3\n");
}

#[test]
fn string_concat_with_anything() {
    assert_eq!(run("println(\"a\" + 1 + true)"), "a1true\n");
    assert_eq!(run("println(1 + \"a\")"), "1a\n");
    assert_eq!("a" < "b", true);
    assert_eq!(run("println(\"abc\" < \"abd\")"), "true\n");
}

#[test]
fn equality_no_coercion() {
    assert_eq!(run("println(1 == \"1\")"), "false\n");
    assert_eq!(run("println(1 == 1.0)"), "true\n");
    assert_eq!(run("println(null == null)"), "true\n");
    assert_eq!(run("println(null == 0)"), "false\n");
    assert_eq!(run("println(EMPTY == EMPTY)"), "true\n");
}

#[test]
fn scoping_penetrates_to_global() {
    // 伪代码式用法：全局 h_low 在函数内被赋值
    let src = "h_low = null\n\
               function Init() {\n\
               \x20   h_low = 42\n\
               }\n\
               Init()\n\
               println(h_low)";
    assert_eq!(run(src), "42\n");
}

#[test]
fn local_creation_inside_function() {
    let src = "x = 1\n\
               function F() {\n\
               \x20   y = 2\n\
               \x20   println(y)\n\
               }\n\
               F()\n\
               println(x)";
    assert_eq!(run(src), "2\n1\n");
    // 函数内新建的 y 不泄漏到全局
    let src2 = "function F() { y = 2 }\nF()\nprintln(y)";
    assert!(matches!(run_err(src2), RtError::Runtime { .. }));
}

#[test]
fn if_body_has_own_scope() {
    let src = "x = 1\nif true { y = 2\nprintln(y) }\nprintln(x)";
    assert_eq!(run(src), "2\n1\n");
    // y 不存在于 if 外
    let src2 = "if true { y = 2 }\nprintln(y)";
    assert!(matches!(run_err(src2), RtError::Runtime { .. }));
}

#[test]
fn closures_share_state_by_reference() {
    let src = "function make_counter() {\n\
               \x20   n = 0\n\
               \x20   return function() { n += 1\nreturn n }\n\
               }\n\
               c = make_counter()\n\
               println(c())\n\
               println(c())";
    assert_eq!(run(src), "1\n2\n");
}

#[test]
fn recursion_fib() {
    let src = "function fib(n) {\n\
               \x20   if n < 2 { return n }\n\
               \x20   return fib(n - 1) + fib(n - 2)\n\
               }\n\
               println(fib(10))";
    assert_eq!(run(src), "55\n");
}

#[test]
fn anonymous_function_value() {
    assert_eq!(run("f = function(x) { return x * 2 }\nprintln(f(3))"), "6\n");
    // 高阶：返回闭包
    assert_eq!(
        run("function add(a) { return function(b) { return a + b } }\nprintln(add(2)(3))"),
        "5\n"
    );
}

#[test]
fn if_else_if_else_chain() {
    let src = "x = 5\n\
               if x > 10 { println(\"a\") } else if x > 3 { println(\"b\") } else { println(\"c\") }";
    assert_eq!(run(src), "b\n");
    let src2 = "x = 1\nif x > 10 { println(\"a\") } else if x > 3 { println(\"b\") } else { println(\"c\") }";
    assert_eq!(run(src2), "c\n");
}

#[test]
fn while_loop() {
    let src = "i = 0\ns = 0\nwhile i < 5 { s += i\ni += 1 }\nprintln(s)";
    assert_eq!(run(src), "10\n");
}

#[test]
fn for_in_over_array_range_string() {
    let src = "total = 0\n\
               for x in [1, 2, 3] { total += x }\n\
               println(total)\n\
               for i in 0..5 { total += i }\n\
               println(total)\n\
               for c in \"ab\" { println(c) }";
    assert_eq!(run(src), "6\n16\na\nb\n");
    // ..= 包含端点
    assert_eq!(run("s = 0\nfor i in 1..=3 { s += i }\nprintln(s)"), "6\n");
}

#[test]
fn c_style_for_with_break_continue() {
    // 0+1+2+4+5 = 12（跳过 3，遇 6 中断）
    let src = "s = 0\nfor i = 0; i < 10; i += 1 {\n\
               \x20   if i == 3 { continue }\n\
               \x20   if i == 6 { break }\n\
               \x20   s += i\n\
               }\n\
               println(s)";
    assert_eq!(run(src), "12\n");
}

#[test]
fn ternary_and_short_circuit() {
    assert_eq!(run("println(true ? 1 : 2)"), "1\n");
    assert_eq!(run("println(0 || \"x\")"), "x\n");
    assert_eq!(run("println(1 && 2)"), "2\n");
    // 短路：右侧不求值，不会报 undefined
    assert_eq!(run("println(false && undefined_var == 1)"), "false\n");
    assert_eq!(run("println(true || undefined_var == 1)"), "true\n");
}

#[test]
fn arrays_read_write_concat() {
    let src = "a = [1, 2, 3]\na[1] = 9\nprintln(a)\nprintln(a[0])\nprintln(a + [4])";
    assert_eq!(run(src), "[1, 9, 3]\n1\n[1, 9, 3, 4]\n");
    // 越界读/写都报错
    assert!(matches!(run_err("a = [1]\nprintln(a[5])"), RtError::Runtime { .. }));
    assert!(matches!(run_err("a = [1]\na[5] = 0"), RtError::Runtime { .. }));
    // 字符串不可变
    assert!(matches!(run_err("s = \"ab\"\ns[0] = \"c\""), RtError::Runtime { .. }));
    // 字符串索引
    assert_eq!(run("println(\"abc\"[1])"), "b\n");
}

#[test]
fn array_slice_syntax() {
    let prelude = "a = [1, 2, 3, 4]\n";
    assert_eq!(run(&format!("{}println(a[1..3])", prelude)), "[2, 3]\n");
    assert_eq!(run(&format!("{}println(a[..2])", prelude)), "[1, 2]\n");
    assert_eq!(run(&format!("{}println(a[2..])", prelude)), "[3, 4]\n");
    assert_eq!(run(&format!("{}println(a[..])", prelude)), "[1, 2, 3, 4]\n");
    // 含终点（..=）
    assert_eq!(run(&format!("{}println(a[1..=2])", prelude)), "[2, 3]\n");
    assert_eq!(run(&format!("{}println(a[..=1])", prelude)), "[1, 2]\n");
    // 负下标从末尾数
    assert_eq!(run(&format!("{}println(a[-2..])", prelude)), "[3, 4]\n");
    assert_eq!(run(&format!("{}println(a[..-1])", prelude)), "[1, 2, 3]\n");
    assert_eq!(run(&format!("{}println(a[-3..-1])", prelude)), "[2, 3]\n");
    // 越界截断、起点 >= 终点得空（同 slice() 语义，不报错）
    assert_eq!(run(&format!("{}println(a[1..100])", prelude)), "[2, 3, 4]\n");
    assert_eq!(run(&format!("{}println(a[2..1])", prelude)), "[]\n");
    assert_eq!(run(&format!("{}println(a[10..20])", prelude)), "[]\n");
    // 表达式端点
    assert_eq!(run(&format!("{}i = 1\nprintln(a[i..i + 2])", prelude)), "[2, 3]\n");
    // 切片是新数组（引用语义隔离）
    assert_eq!(
        run(&format!("{}b = a[..]\nb[0] = 9\nprintln(a[0])\nprintln(b)", prelude)),
        "1\n[9, 2, 3, 4]\n"
    );
    // 可继续链式后缀 / 用于 for-in
    assert_eq!(run(&format!("{}println(a[1..][0])", prelude)), "2\n");
    assert_eq!(run(&format!("{}t = 0\nfor x in a[1..3] {{ t += x }}\nprintln(t)", prelude)), "5\n");
    // 与 slice() 方法结果一致（数组按引用比较，故用 join 对比内容）
    assert_eq!(
        run(&format!("{}println(a[1..3].join(\",\") == a.slice(1, 3).join(\",\"))", prelude)),
        "true\n"
    );
}

#[test]
fn string_slice_syntax() {
    let prelude = "s = \"hello\"\n";
    assert_eq!(run(&format!("{}println(s[1..3])", prelude)), "el\n");
    assert_eq!(run(&format!("{}println(s[..2])", prelude)), "he\n");
    assert_eq!(run(&format!("{}println(s[2..])", prelude)), "llo\n");
    assert_eq!(run(&format!("{}println(s[..])", prelude)), "hello\n");
    assert_eq!(run(&format!("{}println(s[1..=3])", prelude)), "ell\n");
    // 负下标、越界截断
    assert_eq!(run(&format!("{}println(s[-3..])", prelude)), "llo\n");
    assert_eq!(run(&format!("{}println(s[..-2])", prelude)), "hel\n");
    assert_eq!(run(&format!("{}println(s[2..100])", prelude)), "llo\n");
    assert_eq!(run(&format!("{}println(s[3..1])", prelude)), "\n");
    // 按字符（Unicode 码点），不是字节
    assert_eq!(run("println(\"héllo\"[1..3])"), "él\n");
    assert_eq!(run("println(\"你好世界\"[..2])"), "你好\n");
    // 与 sub() 方法等价
    assert_eq!(run(&format!("{}println(s[1..3] == s.sub(1, 3))", prelude)), "true\n");
}

#[test]
fn slice_errors() {
    // 只能切数组/字符串
    assert!(matches!(run_err("x = 1\nprintln(x[..])"), RtError::Runtime { .. }));
    assert!(matches!(run_err("m = new Map()\nprintln(m[..])"), RtError::Runtime { .. }));
    // 端点必须是 number
    assert!(matches!(run_err("a = [1]\nprintln(a[\"x\"..])"), RtError::Runtime { .. }));
    assert!(matches!(run_err("a = [1]\nprintln(a[..null])"), RtError::Runtime { .. }));
    // 切片不能作为赋值目标（解析期报错）
    assert!(matches!(run_err("a = [1]\na[..] = []"), RtError::Parse { .. }));
}

#[test]
fn undefined_variable_has_span() {
    let err = run_err("println(z)");
    match err {
        RtError::Runtime { span: Some(s), message } => {
            assert!(message.contains("undefined variable `z`"), "{}", message);
            assert_eq!(s.line, 1);
        }
        other => panic!("expected runtime error, got {:?}", other),
    }
}

#[test]
fn return_outside_function_is_error() {
    assert!(matches!(run_err("return 1"), RtError::Runtime { .. }));
}

#[test]
fn arity_mismatch_is_error() {
    let err = run_err("function f(a) { return a }\nf(1, 2)");
    assert!(matches!(err, RtError::Runtime { ref message, .. } if message.contains("expects 1 argument")));
}

#[test]
fn native_class_globals() {
    assert_eq!(run("println(EMPTY)"), "EMPTY\n");
    assert_eq!(run("println(inf)"), "inf\n");
    // 类不能直接调用
    let err = run_err("MaxHeap()");
    assert!(matches!(err, RtError::Runtime { ref message, .. } if message.contains("use `new MaxHeap")));
}

#[test]
fn new_heap_from_array() {
    // 堆方法在任务 7 实现，这里只验证构造不炸
    assert_eq!(run("h = new MaxHeap()\nprintln(h)"), "MaxHeap[]\n");
    assert_eq!(run("h = new MaxHeap([3, 1])\nprintln(h)"), "MaxHeap[3, 1]\n");
    assert_eq!(run("h = new MinHeap([3, 1])\nprintln(h)"), "MinHeap[1, 3]\n");
    // 非数组参数报错
    assert!(matches!(run_err("new MaxHeap(3)"), RtError::Runtime { .. }));
    // new 非类报错
    assert!(matches!(run_err("new f()"), RtError::Runtime { .. }));
}

#[test]
fn nested_break_inside_function() {
    // return 从循环内穿越
    let src = "function first_even(xs) {\n\
               \x20   for x in xs {\n\
               \x20       if x % 2 == 0 { return x }\n\
               \x20   }\n\
               \x20   return EMPTY\n\
               }\n\
               println(first_even([1, 3, 4, 6]))\n\
               println(first_even([1, 3]))";
    assert_eq!(run(src), "4\nEMPTY\n");
}

#[test]
fn type_annotations_are_documentation_only() {
    // 标注四处出现（变量/参数/返回值/struct 字段），行为与不写完全一致
    let src = "x: number = 1\n\
               function add(a: number, b: number) -> number {\n\
               \x20   return a + b\n\
               }\n\
               struct Point {\n\
               \x20   x: number,\n\
               \x20   y: number,\n\
               }\n\
               p = new Point(1, 2)\n\
               println(x + add(p.x, p.y))";
    assert_eq!(run(src), "4\n");
    // 泛型标注 + 原生类
    let src = "m: Map<string, number> = new Map()\n\
               m.insert(\"a\", 2)\n\
               println(m.get(\"a\"))";
    assert_eq!(run(src), "2\n");
    // 运行时不做任何类型检查：标注与实际值不符也照常运行
    assert_eq!(run("x: number = \"str\"\nprintln(x)"), "str\n");
    assert_eq!(
        run("function f(a: number) -> string { return a }\nprintln(f(true))"),
        "true\n"
    );
    // 参数个数检查照旧生效
    let e = run_err("function f(a: number) {}\nf(1, 2)");
    assert!(matches!(e, RtError::Runtime { ref message, .. } if message.contains("expects 1 argument")));
}

#[test]
fn let_and_const_declarations_run() {
    // let：正常声明与读、可重新赋值
    assert_eq!(run("let x = 1\nx = x + 1\nprintln(x)"), "2\n");
    assert_eq!(run("let x: number = 3\nprintln(x * 2)"), "6\n");
    // let 在函数内遮蔽外层同名（当前作用域新建）
    assert_eq!(
        run("x = 1\nfunction f() {\n    let x = 2\n    return x\n}\nprintln(f() + x)"),
        "3\n"
    );
    // const：可读不可写
    assert_eq!(run("const PI = 3.14\nprintln(PI)"), "3.14\n");
    assert_eq!(run("const s: string = \"hi\"\nprintln(s)"), "hi\n");
    // 对 const 赋值 → 运行时错误（含复合赋值与内层作用域穿透）
    let e = run_err("const x = 1\nx = 2");
    assert!(matches!(e, RtError::Runtime { ref message, .. } if message.contains("cannot assign to constant `x`")));
    let e = run_err("const x = 1\nx += 1");
    assert!(matches!(e, RtError::Runtime { ref message, .. } if message.contains("cannot assign to constant `x`")));
    let e = run_err("const h = 1\nfunction f() {\n    h = 2\n}\nf()");
    assert!(matches!(e, RtError::Runtime { ref message, .. } if message.contains("cannot assign to constant `h`")));
    // const 绑定本身可变内容（引用值不加锁：数组内容可改）
    assert_eq!(run("const a = [1, 2]\na.push(3)\nprintln(a.len())"), "3\n");
}
