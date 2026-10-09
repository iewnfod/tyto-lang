//! 对象与 self：对象字面量读写、方法字段的 self 绑定、可选链与空值合并。

use super::*;

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

