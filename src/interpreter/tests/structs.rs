//! struct / impl / interface / is：位置构造、self 方法、接口的结构检查。

use super::*;

// ============ struct / impl / interface / is ============

#[test]
fn struct_positional_init() {
    // 无 new 方法：参数按字段声明顺序赋值；无参则全 null
    let src = "struct Rect { w, h }\n\
               a = new Rect(3, 4)\n\
               println(a)\n\
               b = new Rect()\n\
               println(b)\n\
               println(a.w * a.h)";
    assert_eq!(run(src), "Rect {w: 3, h: 4}\nRect {w: null, h: null}\n12\n");
    // 参数多于字段数报错
    assert!(matches!(run_err("struct P { x }\nnew P(1, 2)"), RtError::Runtime { .. }));
}

#[test]
fn struct_new_method_and_self() {
    let src = "struct Point {\n\
               \x20   x,\n\
               \x20   y,\n\
               }\n\
               impl Point {\n\
               \x20   function new(x, y) {\n\
               \x20       self.x = x\n\
               \x20       self.y = y\n\
               \x20   }\n\
               \x20   function len() {\n\
               \x20       return sqrt(self.x * self.x + self.y * self.y)\n\
               \x20   }\n\
               }\n\
               p = new Point(3, 4)\n\
               println(p)\n\
               println(p.len())";
    assert_eq!(run(src), "Point {x: 3, y: 4}\n5\n");
    // new 的返回值被忽略：始终返回实例
    let src2 = "struct P { x }\nimpl P { function new() { return 999 } }\nprintln(new P())";
    assert_eq!(run(src2), "P {x: null}\n");
    // new 的 arity 错误照常报告（全名 Point::new）
    let err = run_err("struct P { x }\nimpl P { function new(a) { } }\nnew P(1, 2)");
    assert!(matches!(err, RtError::Runtime { ref message, .. } if message.contains("P::new")));
}

#[test]
fn struct_methods_call_each_other_via_self() {
    let src = "struct Counter {\n\
               \x20   n,\n\
               }\n\
               impl Counter {\n\
               \x20   function inc() {\n\
               \x20       self.n += 1\n\
               \x20       return self\n\
               \x20   }\n\
               \x20   function value() {\n\
               \x20       return self.n\n\
               \x20   }\n\
               }\n\
               c = new Counter(0)\n\
               c.inc().inc()\n\
               println(c.value())";
    assert_eq!(run(src), "2\n");
}

#[test]
fn struct_method_extracted_has_no_self() {
    // 方法取出后单独调用：self 未定义（与对象方法一致）
    let src = "struct P { x }\n\
               impl P { function get() { return self.x } }\n\
               p = new P(7)\n\
               f = p.get\n\
               println(f())";
    let err = run_err(src);
    assert!(matches!(err, RtError::Runtime { ref message, .. } if message.contains("undefined variable `self`")));
}

#[test]
fn struct_field_access_strict() {
    // 读未声明字段报错
    assert!(matches!(
        run_err("struct P { x }\np = new P(1)\nprintln(p.zz)"),
        RtError::Runtime { .. }
    ));
    // 写未声明字段报错（不自动创建，与对象的关键区别）
    let err = run_err("struct P { x }\np = new P(1)\np.zz = 5");
    assert!(
        matches!(err, RtError::Runtime { ref message, .. } if message.contains("struct P has no field `zz`")),
        "{:?}",
        err
    );
    // 复合赋值可用（字段存在时）
    assert_eq!(run("struct P { x }\np = new P(5)\np.x *= 2\nprintln(p.x)"), "10\n");
    // 重复字段声明报错
    assert!(matches!(run_err("struct P { x, x }"), RtError::Runtime { .. }));
}

#[test]
fn struct_field_shadows_method_lookup() {
    // 字段优先于方法：字段存在则返回字段值
    let src = "struct P { name }\n\
               impl P { function name() { return \"method\" } }\n\
               p = new P(\"field\")\n\
               println(p.name)";
    assert_eq!(run(src), "field\n");
}

#[test]
fn impl_target_must_be_struct() {
    let err = run_err("impl Nope { function f() { return 1 } }");
    assert!(matches!(err, RtError::Runtime { .. })); // Nope 未定义
    let err2 = run_err("x = 1\nimpl x { function f() { return 1 } }");
    assert!(
        matches!(err2, RtError::Runtime { ref message, .. } if message.contains("not a struct")),
        "{:?}",
        err2
    );
}

#[test]
fn impl_after_instances_takes_effect_and_overrides() {
    // 共享 Rc：后挂的 impl 对已有实例生效
    let src = "struct P { x }\n\
               p = new P(1)\n\
               impl P { function f() { return \"first\" } }\n\
               println(p.f())\n\
               impl P { function f() { return \"second\" } }\n\
               println(p.f())";
    assert_eq!(run(src), "first\nsecond\n");
}

#[test]
fn struct_call_without_new_errors() {
    let err = run_err("struct P { x }\nP(1)");
    assert!(matches!(err, RtError::Runtime { ref message, .. } if message.contains("use `new P(")));
}

#[test]
fn struct_instance_builtins() {
    let src = "struct P { x, y }\n\
               p = new P(1, 2)\n\
               println(type(p), len(p), has(p, \"x\"), has(p, \"z\"))\n\
               println(type(P))\n\
               println(p == p, p == new P(1, 2))";
    assert_eq!(run(src), "P 2 true false\nstruct\ntrue false\n");
}

#[test]
fn struct_display_and_optional_member() {
    assert_eq!(run("struct E {}\nprintln(new E())"), "E {}\n");
    // 可选链同对象语义：null 短路，非 null 严格读取
    assert_eq!(run("struct P { x }\np = new P(3)\nn = null\nprintln(p?.x, n?.x)"), "3 null\n");
    assert!(matches!(run_err("struct P { x }\np = new P(3)\nprintln(p?.zz)"), RtError::Runtime { .. }));
}

#[test]
fn is_nominal_struct_check() {
    let src = "struct A { x }\n\
               struct B { x }\n\
               a = new A(1)\n\
               b = new B(1)\n\
               println(a is A, b is A, a is B)\n\
               println(1 is A, null is A, {x: 1} is A)";
    assert_eq!(run(src), "true false false\nfalse false false\n");
}

#[test]
fn is_structural_interface_check() {
    let src = "interface Shape { function area() }\n\
               struct Rect { w, h }\n\
               impl Rect { function area() { return self.w * self.h } }\n\
               struct Point { x, y }\n\
               impl Point { function len() { return 0 } }\n\
               r = new Rect(2, 3)\n\
               p = new Point(1, 1)\n\
               println(r is Shape, p is Shape)\n\
               // 对象也能结构化满足接口\n\
               o = {area: function() { return 9 }}\n\
               bad = {area: 1}\n\
               println(o is Shape, bad is Shape, 5 is Shape)";
    assert_eq!(run(src), "true false\ntrue false false\n");
}

#[test]
fn is_requires_struct_or_interface() {
    // 右侧必须求值为 struct 或 interface
    assert!(matches!(run_err("1 is 2"), RtError::Runtime { .. }));
    assert!(matches!(run_err("x = null\n1 is x"), RtError::Runtime { .. }));
    // 右侧是 struct 时：非实例恒 false（不报错）
    assert_eq!(run("struct P { x }\nprintln(1 is P, null is P, {x: 1} is P)"), "false false false\n");
}

#[test]
fn is_precedence_in_expressions() {
    let src = "interface F { function f() }\n\
               struct P { x }\n\
               impl P { function f() { return 1 } }\n\
               p = new P()\n\
               if p is F && p.x == null { println(\"both\") }\n\
               println(p is F == true)";
    assert_eq!(run(src), "both\ntrue\n");
}

#[test]
fn struct_methods_close_over_scope() {
    // impl 执行时的作用域被闭包捕获
    let src = "factor = 10\n\
               struct P { x }\n\
               impl P { function scaled() { return self.x * factor } }\n\
               println(new P(4).scaled())";
    assert_eq!(run(src), "40\n");
}

