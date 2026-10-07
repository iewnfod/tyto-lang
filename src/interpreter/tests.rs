use super::*;
use crate::lexer::Lexer;
use crate::parser::Parser;
use std::io::{BufReader, Cursor};

/// 运行源码并捕获 stdout
fn run(src: &str) -> String {
    let vec: Rc<RefCell<Vec<u8>>> = Rc::new(RefCell::new(Vec::new()));
    let out: OutRef = vec.clone();
    let input: InRef = Rc::new(RefCell::new(BufReader::new(Cursor::new(Vec::<u8>::new()))));
    let mut interp = Interpreter::with_io(out, input);
    let tokens = Lexer::new(src).tokenize().unwrap().tokens;
    let program = Parser::new(tokens).parse_program().unwrap();
    interp.run(&program).expect("runtime ok");
    String::from_utf8(vec.borrow().clone()).unwrap()
}

fn run_err(src: &str) -> RtError {
    let vec: Rc<RefCell<Vec<u8>>> = Rc::new(RefCell::new(Vec::new()));
    let out: OutRef = vec.clone();
    let input: InRef = Rc::new(RefCell::new(BufReader::new(Cursor::new(Vec::<u8>::new()))));
    let mut interp = Interpreter::with_io(out, input);
    let tokens = match Lexer::new(src).tokenize() {
        Ok(o) => o.tokens,
        Err(e) => return e,
    };
    let program = match Parser::new(tokens).parse_program() {
        Ok(p) => p,
        Err(e) => return e,
    };
    interp.run(&program).expect_err("expected error")
}

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

// ============ 对象与 self ============

#[test]
fn object_literal_read_write_autocreate() {
    let src = "p = {x: 1, y: 2}\n\
               println(p.x)\n\
               p.y = p.y + 10\n\
               println(p.y)\n\
               p.z = 3\n\
               println(p.z)\n\
               println(p)";
    assert_eq!(run(src), "1\n12\n3\n{x: 1, y: 12, z: 3}\n");
    assert_eq!(run("println({})"), "{}\n");
}

#[test]
fn object_compound_assign() {
    assert_eq!(run("p = {n: 5}\np.n += 2\nprintln(p.n)"), "7\n");
}

#[test]
fn nested_objects_share_references() {
    let src = "inner = {v: 1}\n\
               a = {c: inner}\n\
               b = {c: inner}\n\
               a.c.v = 42\n\
               println(b.c.v)\n\
               println(a.c == b.c)";
    assert_eq!(run(src), "42\ntrue\n");
}

#[test]
fn self_binding_in_method_call() {
    let src = "counter = {\n\
               \x20   n: 0,\n\
               \x20   inc: function() {\n\
               \x20       self.n += 1\n\
               \x20       return self.n\n\
               \x20   }\n\
               }\n\
               println(counter.inc())\n\
               println(counter.inc())\n\
               println(counter.n)";
    assert_eq!(run(src), "1\n2\n2\n");
}

#[test]
fn self_calls_sibling_method() {
    let src = "p = {\n\
               \x20   a: function() { return 1 },\n\
               \x20   b: function() { return self.a() + 1 }\n\
               }\n\
               println(p.b())";
    assert_eq!(run(src), "2\n");
}

#[test]
fn extracted_function_has_no_self() {
    // 把方法取出来单独调用：self 未定义（与 JS 动态 this 相反，更可预测）
    let src = "p = {n: 1, f: function() { return self.n }}\n\
               g = p.f\n\
               println(g())";
    let err = run_err(src);
    assert!(matches!(err, RtError::Runtime { ref message, .. } if message.contains("undefined variable `self`")));
}

#[test]
fn missing_field_read_errors() {
    let err = run_err("p = {x: 1}\nprintln(p.y)");
    assert!(
        matches!(err, RtError::Runtime { ref message, .. } if message.contains("object has no field `y`")),
        "{:?}",
        err
    );
}

#[test]
fn optional_chaining() {
    // null 短路
    assert_eq!(run("p = null\nprintln(p?.x)"), "null\n");
    assert_eq!(run("p = null\nprintln(p?.x?.y)"), "null\n");
    // 非空正常读取
    assert_eq!(run("q = {a: {b: 7}}\nprintln(q?.a?.b)"), "7\n");
    // 非空对象缺字段：仍然报错（严格读取）
    assert!(matches!(run_err("q = {a: 1}\nprintln(q?.z)"), RtError::Runtime { .. }));
    // 可选方法调用
    assert_eq!(run("r = {f: function() { return 42 }}\nprintln(r?.f())"), "42\n");
    assert_eq!(run("n = null\nprintln(n?.f())"), "null\n");
}

#[test]
fn nullish_coalescing() {
    // 仅 null 触发回退
    assert_eq!(run("println(null ?? 1)"), "1\n");
    assert_eq!(run("println(null ?? null ?? 3)"), "3\n");
    // 假值不触发（这是与 || 的核心区别）
    assert_eq!(run("println(0 ?? 1)"), "0\n");
    assert_eq!(run("println(\"\" ?? \"x\")"), "\n");
    assert_eq!(run("println(false ?? true)"), "false\n");
    // EMPTY 是容器哨兵，不参与 nullish
    assert_eq!(run("println(EMPTY ?? 1)"), "EMPTY\n");
    // 对比 ||
    assert_eq!(run("println(0 ?? \"x\")\nprintln(0 || \"x\")"), "0\nx\n");
    // 短路：非 null 时右侧不求值
    assert_eq!(run("println(9 ?? undefined_var)"), "9\n");
    assert!(matches!(run_err("println(null ?? undefined_var)"), RtError::Runtime { .. }));
    // 与可选链配合
    assert_eq!(run("p = null\nprintln(p?.x ?? 0)"), "0\n");
}

#[test]
fn nullish_assign() {
    assert_eq!(run("x = null\nx ??= 5\nprintln(x)"), "5\n");
    assert_eq!(run("x = 3\nx ??= 5\nprintln(x)"), "3\n");
    // 旧值非 null：右侧不求值
    assert_eq!(run("x = 3\nx ??= undefined_var\nprintln(x)"), "3\n");
    // 假值不触发
    assert_eq!(run("x = 0\nx ??= 9\nprintln(x)"), "0\n");
    // 索引 / 成员目标
    assert_eq!(run("a = [null, 2]\na[0] ??= 7\nprintln(a)"), "[7, 2]\n");
    assert_eq!(run("p = {x: null}\np.x ??= 2\nprintln(p)"), "{x: 2}\n");
    assert_eq!(run("p = {x: 1}\np.x ??= 2\nprintln(p)"), "{x: 1}\n");
}

#[test]
fn object_equality_by_reference() {
    let src = "a = {x: 1}\n\
               b = a\n\
               c = {x: 1}\n\
               println(a == b)\n\
               println(a == c)\n\
               println(a == 1)";
    assert_eq!(run(src), "true\nfalse\nfalse\n");
}

#[test]
fn non_function_field_not_callable() {
    let err = run_err("p = {n: 5}\np.n()");
    assert!(matches!(err, RtError::Runtime { ref message, .. } if message.contains("not callable")));
}

#[test]
fn member_assign_on_non_object_errors() {
    // 原生类型没有可赋值字段
    assert!(matches!(run_err("a = [1]\na.x = 2"), RtError::Runtime { .. }));
    assert!(matches!(run_err("s = \"a\"\ns.x = 2"), RtError::Runtime { .. }));
}
