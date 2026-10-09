//! 核心检查器行为测试：渐进承诺（无标注零误报）+ 各检查项正反例。

use tyto_lang::checker::check_program;
use tyto_lang::checker::diag::Severity;
use tyto_lang::{Lexer, Parser};

fn diags(src: &str) -> Vec<(Severity, String)> {
    let out = Lexer::new(src).tokenize().unwrap();
    let program = Parser::new(out.tokens).parse_program().unwrap();
    check_program(&program)
        .diagnostics
        .into_iter()
        .map(|d| (d.severity, d.message))
        .collect()
}

fn errors(src: &str) -> Vec<String> {
    diags(src)
        .into_iter()
        .filter(|(s, _)| *s == Severity::Error)
        .map(|(_, m)| m)
        .collect()
}

#[test]
fn gradual_promise_unannotated_code_is_silent() {
    // 渐进承诺：无标注代码零诊断（any 兜底，绝不误报）
    let src = "x = 1\nx = \"str\"\ny = [1, \"mix\", null]\nfunction f(a, b) { return a }\nz = f(x, y)\nq = z.foo().bar()";
    assert!(diags(src).is_empty(), "{:?}", diags(src));
    // 现存示例风格：顶层 null + init() 惯用形也零诊断
    let src = "h = null\nfunction init() {\n    h = new Map()\n}\ninit()\nh.insert(1, 2)";
    assert!(diags(src).is_empty(), "{:?}", diags(src));
}

#[test]
fn annotation_mismatch_is_error() {
    let e = errors("x: number = \"abc\"");
    assert_eq!(e.len(), 1);
    assert!(e[0].contains("期望 `number`，实际 `string`"), "{:?}", e);
    assert!(e[0].contains("第 1 行"), "{:?}", e);
    // 二次赋值不匹配
    let e = errors("x: number = 1\nx = \"s\"");
    assert_eq!(e.len(), 1, "{:?}", e);
    // 未标注 let 不约束（渐进）
    assert!(errors("let x = 1\nx = \"s\"").is_empty());
}

#[test]
fn union_types() {
    // 用户核心用例：number | null = null ✓
    assert!(errors("x: number | null = null").is_empty());
    assert!(errors("x: number | null = 1").is_empty());
    assert!(errors("x: number | null = \"s\"").is_empty() == false);
    // 联合上直接算术 → 报（TS strictNullChecks 同款）
    let e = errors("n: number | null = null\nm = n + 1");
    assert_eq!(e.len(), 1, "{:?}", e);
    // ?? 收窄掉 null → 不报
    assert!(errors("n: number | null = null\nm = (n ?? 0) + 1").is_empty());
    // ?. 访问允许
    assert!(errors("n: number | null = null\ns = \"x\"\nm = n?.foo").is_empty());
}

#[test]
fn generic_function_inference() {
    // 泛型函数：调用点实参推导
    let src = "function id<T>(x: T) -> T { return x }\na = id(42)\nb: number = a";
    assert!(errors(src).is_empty(), "{:?}", errors(src));
    // id("s") → string；赋给 number 报错
    let src = "function id<T>(x: T) -> T { return x }\na: number = id(\"s\")";
    let e = errors(src);
    assert_eq!(e.len(), 1);
    assert!(e[0].contains("实际 `string`"), "{:?}", e);
    // 泛型函数体内的 T 参与检查
    let src = "function bad<T>(x: T) -> T { return x + 1 }";
    assert!(errors(src).is_empty(), "{:?}", errors(src)); // T=any 放行（渐进）
    // 实参个数
    let e = errors("function pair<K, V>(k: K, v: V) -> V { return v }\nx = pair(1)");
    assert_eq!(e.len(), 1, "{:?}", e);
    assert!(e[0].contains("期望 2 个参数"), "{:?}", e);
}

#[test]
fn generic_struct_inference() {
    // new Box(1) → Box<number>：字段 v 推导为 number
    let src = "struct Box<T> {\n    v: T,\n}\nb = new Box(1)\nx: number = b.v";
    assert!(errors(src).is_empty(), "{:?}", errors(src));
    // 类型不匹配：Box("s") 的 v 是 string
    let src = "struct Box<T> {\n    v: T,\n}\nb = new Box(\"s\")\nx: number = b.v";
    let e = errors(src);
    assert_eq!(e.len(), 1, "{:?}", e);
    assert!(e[0].contains("实际 `string`"), "{:?}", e);
    // 构造实参与字段标注不符
    let e = errors("struct P {\n    x: number,\n}\np = new P(\"s\")");
    assert_eq!(e.len(), 1, "{:?}", e);
}

#[test]
fn builtin_generics_are_element_aware() {
    // Array<number> 元素类型
    assert!(errors("a: Array<number> = [1, 2]\nx: number = a[0]").is_empty());
    let e = errors("a: Array<number> = [1, 2]\nx: string = a[0]");
    assert_eq!(e.len(), 1, "{:?}", e);
    // number[] 后缀同样
    assert!(errors("a: number[] = [1]\nx: number = a[0]").is_empty());
    // Map<K, V>：get → V | null
    let e = errors("m: Map<string, number> = new Map()\nx: number = m.get(\"k\")");
    assert_eq!(e.len(), 1, "{:?}", e);
    assert!(e[0].contains("实际 `number | null`"), "{:?}", e);
    // ?? 收窄后 ok
    assert!(
        errors("m: Map<string, number> = new Map()\nx: number = m.get(\"k\") ?? 0").is_empty()
    );
    // keys() → Array<K>
    assert!(errors("m: Map<string, number> = new Map()\nk: string[] = m.keys()").is_empty());
}

#[test]
fn call_argument_checks() {
    // 用户函数实参类型与个数
    let e = errors("function add(a: number, b: number) -> number { return a + b }\nx = add(1, \"s\")");
    assert_eq!(e.len(), 1, "{:?}", e);
    assert!(e[0].contains("参数 `b`"), "{:?}", e);
    let e = errors("function add(a: number, b: number) -> number { return a + b }\nx = add(1)");
    assert!(e[0].contains("期望 2 个参数"), "{:?}", e);
    // 内置：len 1 参
    let e = errors("len(1, 2)");
    assert!(e[0].contains("`len` 期望 1 个参数"), "{:?}", e);
    // 实参个数与运行时一致（用户函数严格匹配），未标注参数仍做个数检查
    let e = errors("function f(a) { return a }\nf(1, 2, 3)");
    assert_eq!(e.len(), 1, "{:?}", e);
    assert!(e[0].contains("期望 1 个参数"), "{:?}", e);
    // 未标注参数不做类型检查
    assert!(errors("function f(a) { return a }\nf(\"s\")").is_empty());
}

#[test]
fn return_type_checks() {
    let e = errors("function f() -> number { return \"s\" }");
    assert_eq!(e.len(), 1, "{:?}", e);
    assert!(e[0].contains("返回类型不匹配"), "{:?}", e);
    // 标注返回但无 return → warning（不是 error）
    let d = diags("function f() -> number {\n    println(1)\n}");
    assert_eq!(d.len(), 1);
    assert!(d[0].0 == Severity::Warning, "{:?}", d);
    assert!(d[0].1.contains("没有 return"), "{:?}", d);
    // 正常
    assert!(errors("function f() -> number { return 1 }").is_empty());
}

#[test]
fn const_and_let_rules() {
    // const 重赋值（静态层；运行时另有强制）
    let e = errors("const x = 1\nx = 2");
    assert_eq!(e.len(), 1, "{:?}", e);
    assert!(e[0].contains("不能给常量 `x` 赋值"), "{:?}", e);
    // const 无标注：初始值类型固定
    let e = errors("const s = \"hi\"\nn: number = s");
    assert_eq!(e.len(), 1, "{:?}", e);
    // let 同层重复声明 → warning
    let d = diags("let a = 1\nlet a = 2");
    assert!(d.iter().any(|(s, m)| *s == Severity::Warning && m.contains("重复声明")), "{:?}", d);
    // let 遮蔽外层不警告
    let d = diags("a = 1\nfunction f() {\n    let a = 2\n    return a\n}");
    assert!(d.is_empty(), "{:?}", d);
}

#[test]
fn member_existence_checks() {
    // struct 未知字段
    let e = errors("struct P {\n    x: number,\n}\np = new P(1)\np.z");
    assert_eq!(e.len(), 1, "{:?}", e);
    assert!(e[0].contains("struct `P` 没有字段或方法 `z`"), "{:?}", e);
    // number 没有 len 成员
    let e = errors("x: number = 1\nx.len()");
    assert_eq!(e.len(), 1, "{:?}", e);
    // 数组方法 ok
    assert!(errors("a: number[] = [1]\na.push(2)\na.len()").is_empty());
    // 数组元素类型传导到方法实参
    let e = errors("a: number[] = [1]\na.push(\"s\")");
    assert_eq!(e.len(), 1, "{:?}", e);
    // 命名空间未知成员
    let e = errors("fs.nope()");
    assert!(e[0].contains("命名空间 `fs` 没有 `nope`"), "{:?}", e);
}

#[test]
fn arithmetic_operand_checks() {
    // bool + bool
    let e = errors("x = true + false");
    assert_eq!(e.len(), 1, "{:?}", e);
    assert!(e[0].contains("`+` 不能用于 `bool` 和 `bool`"), "{:?}", e);
    // string + any → string（语言语义）
    assert!(errors("x = \"a\" + 1").is_empty());
    // number[] + number[] → number[]
    assert!(errors("a: number[] = [1]\nb: number[] = a + [2]").is_empty());
    // null 参与算术
    let e = errors("n: number | null = null\nm = n * 2");
    assert_eq!(e.len(), 1, "{:?}", e);
}

#[test]
fn function_type_annotations() {
    // 函数类型标注
    let src = "function make(): (a: number) -> number {\n    return function(a: number) -> number { return a }\n}";
    let _ = src; // 冒号后类型语法（声明位）暂以 -> 形式为准，这里测值位标注
    let src = "f: (a: number) -> number = function(a: number) -> number { return a }\nx: number = f(1)";
    assert!(errors(src).is_empty(), "{:?}", errors(src));
    // 实参不符
    let e = errors("f: (a: number) -> number = function(a: number) -> number { return a }\nf(\"s\")");
    assert_eq!(e.len(), 1, "{:?}", e);
    assert!(e[0].contains("参数类型不匹配"), "{:?}", e);
}

#[test]
fn object_type_annotations() {
    let src = "o: {x: number, y: string} = {x: 1, y: \"a\"}";
    assert!(errors(src).is_empty(), "{:?}", errors(src));
    // 字段类型不符
    let e = errors("o: {x: number} = {x: \"s\"}");
    assert_eq!(e.len(), 1, "{:?}", e);
    // 结构化：缺字段
    let e = errors("o: {x: number, y: string} = {x: 1}");
    assert_eq!(e.len(), 1, "{:?}", e);
    // 对象成员检查
    let e = errors("o: {x: number} = {x: 1}\no.y");
    assert_eq!(e.len(), 1, "{:?}", e);
    assert!(e[0].contains("没有字段 `y`"), "{:?}", e);
}

#[test]
fn for_in_loop_var_types() {
    // 数组元素类型 → 循环变量
    assert!(errors("a: number[] = [1, 2]\nfor x in a {\n    n: number = x\n}").is_empty());
    let e = errors("a: number[] = [1, 2]\nfor x in a {\n    s: string = x\n}");
    assert_eq!(e.len(), 1, "{:?}", e);
    // 字符串迭代 → string
    assert!(errors("for c in \"abc\" {\n    s: string = c\n}").is_empty());
    // 区间 → number
    assert!(errors("for i in 0..3 {\n    n: number = i\n}").is_empty());
}

#[test]
fn builtin_method_arg_checks() {
    // 数组 push 元素类型
    let e = errors("a: number[] = [1]\na.push(\"s\")");
    assert_eq!(e.len(), 1, "{:?}", e);
    assert!(e[0].contains("第 1 个参数"), "{:?}", e);
    // Map.insert 键值类型
    let e = errors("m: Map<string, number> = new Map()\nm.insert(1, 2)");
    assert_eq!(e.len(), 1, "{:?}", e);
    // Map.get 键类型 + 定长
    let e = errors("m: Map<string, number> = new Map()\nm.get(1)");
    assert_eq!(e.len(), 1, "{:?}", e);
    // 堆 push 非数字
    let e = errors("h = new MaxHeap()\nh.push(\"s\")");
    assert_eq!(e.len(), 1, "{:?}", e);
    // 哨兵宽容：number | empty 可入堆（median 惯用形）
    assert!(errors("a = new MaxHeap()\nb = a.pop()\na.push(b)").is_empty());
}

#[test]
fn generic_impl_and_recursive_struct() {
    // 用户场景：递归泛型 struct + impl 声明泛型参数（方法直接引用 T）
    let src = "struct TreeNode<T> {\n\
               \x20   val: T,\n\
               \x20   left: TreeNode<T> | null,\n\
               \x20   right: TreeNode<T> | null,\n\
               }\n\
               impl TreeNode<T> {\n\
               \x20   function new(val: T) {\n\
               \x20       self.val = val\n\
               \x20       self.left = null\n\
               \x20       self.right = null\n\
               \x20   }\n\
               \x20   function get_val() -> T {\n\
               \x20       return self.val\n\
               \x20   }\n\
               }\n\
               root = new TreeNode(1)\n\
               n: number = root.get_val()";
    assert!(errors(src).is_empty(), "{:?}", errors(src));

    // 方法自带 <T> 的写法（function new<T>(val: T)）同样支持，类型实参照样求解
    let src = "impl2_marker = 0\nstruct B<T> {\n    v: T,\n}\nimpl B {\n    function new<T>(v: T) {\n        self.v = v\n    }\n}\nb = new B(\"s\")\nx: string = b.v";
    assert!(errors(src).is_empty(), "{:?}", errors(src));

    // 类型实参求解到字段：new TreeNode(1) → val 为 number
    let e = errors("struct B<T> {\n    v: T,\n}\nimpl B {\n    function new<T>(v: T) {\n        self.v = v\n    }\n}\nb = new B(\"s\")\nx: number = b.v");
    assert_eq!(e.len(), 1, "{:?}", e);
    assert!(e[0].contains("实际 `string`"), "{:?}", e);

    // 跨节点类型一致性：TreeNode<string> 挂 TreeNode<number> 子节点 → 真错误
    let src = "struct N<T> {\n\
               \x20   v: T,\n\
               \x20   kid: N<T> | null,\n\
               }\n\
               impl N<T> {\n\
               \x20   function new(v: T) {\n\
               \x20       self.v = v\n\
               \x20       self.kid = null\n\
               \x20   }\n\
               \x20   function set_kid(k: N<T> | null) {\n\
               \x20       self.kid = k\n\
               \x20   }\n\
               }\n\
               a = new N(\"s\")\n\
               b = new N(1)\n\
               a.set_kid(b)";
    let e = errors(src);
    assert_eq!(e.len(), 1, "{:?}", e);
    assert!(e[0].contains("TreeNode") || e[0].contains("N<"), "{}", e[0]);
}

#[test]
fn interface_signatures_kept() {
    // interface 完整签名保留（is 检查运行时仍只看方法名）
    let src = "interface Shape {\n    function area() -> number\n}\nfunction f(s: Shape) -> number {\n    return s.area()\n}";
    assert!(errors(src).is_empty(), "{:?}", errors(src));
}

#[test]
fn let_shadowing_and_closures() {
    // let 在函数内遮蔽全局
    let src = "x = \"global\"\nfunction f() {\n    let x = 1\n    return x + 1\n}";
    assert!(errors(src).is_empty(), "{:?}", errors(src));
    // 闭包捕获（运行时链上语义，静态按最新类型）
    let src = "function counter() {\n    c = 0\n    return function() {\n        c += 1\n        return c\n    }\n}";
    assert!(errors(src).is_empty(), "{:?}", errors(src));
}

#[test]
fn parse_and_lex_diags_short_circuit() {
    // 语法错误时类型检查不跑（LSP/CLI 同一规则：lex → parse → check）
    // 这里只验证复杂语法混排不崩
    let src = "let cb: (e: string) -> null = function(e: string) -> null {\n    println(e)\n}\ncb(\"x\")\n";
    assert!(errors(src).is_empty(), "{:?}", errors(src));
}
