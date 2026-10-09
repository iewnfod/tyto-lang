//! 语义着色与跳转定义测试。

use tyto_lang::analysis::semantics::{semantic_tokens, DefLoc, TOKEN_TYPES};

/// (line, col) → 类型名；方便断言
fn token_map(src: &str) -> std::collections::HashMap<(usize, usize), &'static str> {
    semantic_tokens(src)
        .into_iter()
        .map(|t| ((t.line, t.col), TOKEN_TYPES[t.ty as usize]))
        .collect()
}

fn tok(src: &str, line: usize, col: usize) -> Option<&'static str> {
    token_map(src).get(&(line, col)).copied()
}

fn def(src: &str, line: usize, col: usize) -> Option<DefLoc> {
    tyto_lang::analysis::semantics::definition(src, line, col)
}

#[test]
fn colors_variable_and_function_references() {
    let src = "\
n = 42
function inc(x) {
    return x + 1
}
m = inc(n)
";
    let m = token_map(src);
    // n 声明（行0）与引用（行4：m = inc(n) → n 在 col 8）
    assert_eq!(m.get(&(0, 0)), Some(&"variable"), "n 声明");
    assert_eq!(m.get(&(4, 8)), Some(&"variable"), "n 引用");
    // inc 声明名（行1）与调用（行4）
    assert_eq!(m.get(&(1, 9)), Some(&"function"), "inc 声明");
    assert_eq!(m.get(&(4, 4)), Some(&"function"), "inc 调用");
    // 参数 x：声明（行1）与引用（行2）
    assert_eq!(m.get(&(1, 13)), Some(&"parameter"), "x 参数声明");
    assert_eq!(m.get(&(2, 11)), Some(&"parameter"), "x 引用");
}

#[test]
fn colors_struct_interface_and_members() {
    let src = "\
struct Point {
    x,
    y,
}
impl Point {
    function len() -> number {
        return 1
    }
}
p = new Point(3, 4)
a = p.x
b = p.len()
";
    let m = token_map(src);
    assert_eq!(m.get(&(0, 7)), Some(&"struct"), "struct 名");
    assert_eq!(m.get(&(1, 4)), Some(&"property"), "字段 x 声明");
    assert_eq!(m.get(&(4, 5)), Some(&"struct"), "impl 目标名");
    assert_eq!(m.get(&(5, 13)), Some(&"method"), "方法 len 声明");
    assert_eq!(m.get(&(9, 0)), Some(&"variable"), "p 声明");
    assert_eq!(m.get(&(9, 8)), Some(&"struct"), "new Point 构造");
    assert_eq!(m.get(&(10, 4)), Some(&"variable"), "p.x 的 p");
    assert_eq!(m.get(&(10, 6)), Some(&"property"), "p.x 的 x");
    assert_eq!(m.get(&(11, 6)), Some(&"method"), "p.len() 的 len");
}

#[test]
fn colors_builtin_class_and_namespace() {
    let src = "\
m = new Map()
t = fs.read_file(\"x\")
u = sys.shell(\"ls\")
";
    let m = token_map(src);
    assert_eq!(m.get(&(0, 8)), Some(&"class"), "new Map 内置类");
    assert_eq!(m.get(&(1, 4)), Some(&"namespace"), "fs 命名空间");
    assert_eq!(m.get(&(1, 7)), Some(&"function"), "fs.read_file 函数");
    assert_eq!(m.get(&(2, 4)), Some(&"namespace"), "sys");
    assert_eq!(m.get(&(2, 8)), Some(&"function"), "sys.shell");
}

#[test]
fn colors_string_and_array_builtin_methods() {
    let src = "\
s = \"abc\"
u = s.to_uppercase()
a = [1, 2]
l = a.len()
";
    let m = token_map(src);
    assert_eq!(m.get(&(1, 6)), Some(&"method"), "字符串方法");
    assert_eq!(m.get(&(3, 6)), Some(&"method"), "数组方法");
}

#[test]
fn colors_object_literal_keys_by_value_kind() {
    let src = "\
counter = {
    n: 0,
    inc: function() {
        return self.n
    },
}
v = counter.n
counter.inc()
";
    let m = token_map(src);
    assert_eq!(m.get(&(1, 4)), Some(&"property"), "对象键 n（普通值）");
    assert_eq!(m.get(&(2, 4)), Some(&"method"), "对象键 inc（函数值）");
    assert_eq!(m.get(&(6, 12)), Some(&"property"), "counter.n 的 n");
    assert_eq!(m.get(&(7, 8)), Some(&"method"), "counter.inc 的 inc");
}

#[test]
fn colors_loop_vars() {
    let src = "\
for i in 0..n {
    println(i)
}
for c in \"ab\" {
    println(c)
}
";
    let m = token_map(src);
    assert_eq!(m.get(&(0, 4)), Some(&"variable"), "for-in 循环变量声明");
    assert_eq!(m.get(&(1, 12)), Some(&"variable"), "循环变量引用");
    assert_eq!(m.get(&(3, 4)), Some(&"variable"), "字符串循环变量");
}

#[test]
fn unknown_member_not_colored() {
    let src = "\
function f(x) {
    return x.mystery
}
";
    let m = token_map(src);
    // x 是未标注参数（unknown）→ 成员名不着色（保留 TextMate）
    assert!(!m.contains_key(&(1, 13)), "未知接收者的成员不发 token");
    // 但 x 引用本身按参数着色
    assert_eq!(m.get(&(1, 11)), Some(&"parameter"));
}

#[test]
fn builtin_globals_colored_as_function() {
    let src = "println(len([1]))\n";
    let m = token_map(src);
    assert_eq!(m.get(&(0, 0)), Some(&"function"), "println");
    assert_eq!(m.get(&(0, 8)), Some(&"function"), "len");
}

#[test]
fn literals_and_self_left_to_textmate() {
    let src = "x = true\ny = null\n";
    let m = token_map(src);
    assert!(!m.contains_key(&(0, 4)), "true 不发语义 token");
    assert!(!m.contains_key(&(1, 4)), "null 不发语义 token");
}

#[test]
fn broken_document_returns_partial_tokens() {
    // 尾部坏语句：前缀照常着色，不 panic
    let src = "n = 1\nprintln(n)\nx = ?? ?";
    let m = token_map(src);
    assert_eq!(m.get(&(0, 0)), Some(&"variable"));
    assert_eq!(m.get(&(1, 0)), Some(&"function"));
    assert_eq!(m.get(&(1, 8)), Some(&"variable"));
}

// ============ 跳转定义 ============

#[test]
fn definition_jumps_to_variable_assignment() {
    let src = "n = 42\nprintln(n)\n";
    let d = def(src, 1, 8).unwrap();
    assert_eq!((d.line, d.col, d.len), (0, 0, 1), "n → 声明处");
}

#[test]
fn definition_jumps_to_function_decl() {
    let src = "function add(a, b) {\n    return a + b\n}\nprintln(add(1, 2))\n";
    let d = def(src, 3, 8).unwrap();
    assert_eq!((d.line, d.col, d.len), (0, 9, 3), "add → function 名字处");
}

#[test]
fn definition_jumps_to_param() {
    let src = "function add(a, b) {\n    return a + b\n}\n";
    let d = def(src, 1, 11).unwrap();
    assert_eq!((d.line, d.col, d.len), (0, 13, 1), "a → 参数声明处");
}

#[test]
fn definition_jumps_to_struct_field_and_method() {
    let src = "\
struct Point {
    x,
    y,
}
impl Point {
    function len() {
        return 1
    }
}
p = new Point(1, 2)
a = p.x
b = p.len()
";
    let d = def(src, 10, 6).unwrap();
    assert_eq!((d.line, d.col, d.len), (1, 4, 1), "p.x → 字段 x 声明");
    let d = def(src, 11, 6).unwrap();
    assert_eq!((d.line, d.col, d.len), (5, 13, 3), "p.len → 方法声明");
}

#[test]
fn definition_self_jumps_to_struct() {
    let src = "\
struct Rect {
    w,
}
impl Rect {
    function area() {
        return self.w
    }
}
";
    let d = def(src, 5, 15).unwrap();
    assert_eq!((d.line, d.col, d.len), (0, 7, 4), "self → struct Rect 名字");
}

#[test]
fn definition_builtin_returns_none() {
    assert!(def("println(1)\n", 0, 3).is_none(), "内置函数无源码位置");
    assert!(def("m = new Map()\nm.", 1, 2).map(|d| d.len).is_none());
}

#[test]
fn colors_if_while_for_conditions() {
    let src = "\
x = 1
if x > 0 {
    y = 2
    println(y)
} else if x < 0 {
    println(x)
}
while x > 0 {
    x -= 1
}
for i = 0; i < x; i += 1 {
    println(i)
}
for c in \"ab\" {
    println(c)
}
";
    let m = token_map(src);
    // 条件里的变量要上色
    assert_eq!(m.get(&(1, 3)), Some(&"variable"), "if 条件的 x");
    assert_eq!(m.get(&(4, 10)), Some(&"variable"), "else-if 条件的 x");
    assert_eq!(m.get(&(7, 6)), Some(&"variable"), "while 条件的 x");
    assert_eq!(m.get(&(10, 15)), Some(&"variable"), "for-C 条件的 x");
    assert_eq!(m.get(&(10, 18)), Some(&"variable"), "for-C 步进的 i");
    // 体内照常
    assert_eq!(m.get(&(2, 4)), Some(&"variable"), "if 体内的 y");
    assert_eq!(m.get(&(8, 4)), Some(&"variable"), "while 体内的 x");
}

#[test]
fn definition_object_literal_field_via_self() {
    let src = "\
counter = {
    n: 0,
    inc: function() {
        self.n += 1
        return self.n
    },
}
";
    // self.n（行3 col 9 的 n）→ 对象键 n（行1 col 4）
    let d = def(src, 3, 13).unwrap();
    assert_eq!((d.line, d.col, d.len), (1, 4, 1), "self.n → 对象键声明");
}

#[test]
fn definition_var_inside_function_via_heuristic() {
    let src = "h = null\nfunction push() {\n    h.push(1)\n}\nh = new MaxHeap()\n";
    // 光标在 h.push 的 h 上（行2 列4）
    let d = def(src, 2, 4).unwrap();
    assert_eq!(d.line, 0, "h → 顶层声明（启发式下仍是首个声明处）");
}

// ============ 泛型参数跳转 ============

#[test]
fn definition_type_param_jumps_to_impl_decl() {
    let src = "\
impl TreeNode<T> {
    function new(val: T) {
        self.val = val
    }
}
";
    // val: T 的 T（行1 col 22）→ impl 头部 <T>（行0 col 14）
    let d = def(src, 1, 22).unwrap();
    assert_eq!((d.line, d.col, d.len), (0, 14, 1), "T → impl 头部 <T> 声明处");
    // 方法体内引用同名参数 val → 参数声明处（泛型函数此前扫描不到参数）
    let d = def(src, 2, 19).unwrap();
    assert_eq!((d.line, d.col, d.len), (1, 17, 3), "val → 泛型方法参数声明");
}

#[test]
fn definition_type_param_prefers_nearest_decl() {
    let src = "\
impl Box<T> {
    function pair<K>(a: K) {
        return a
    }
    function get(v: T) {
        return v
    }
}
";
    // 方法自带 <K>：a: K 的 K（行1 col 24）→ 方法头 <K>（行1 col 18）
    let d = def(src, 1, 24).unwrap();
    assert_eq!((d.line, d.col, d.len), (1, 18, 1), "K → 方法自己的 <K>（就近遮蔽）");
    // 无自带泛型的方法：v: T 的 T（行4 col 20）→ impl 头 <T>（行0 col 9）
    let d = def(src, 4, 20).unwrap();
    assert_eq!((d.line, d.col, d.len), (0, 9, 1), "T → impl 的 <T>");
}

#[test]
fn definition_type_param_in_struct_field_and_args() {
    let src = "\
struct TreeNode<T> {
    val: T,
    left: TreeNode<T> | null,
}
";
    // 字段标注 val: T 的 T → struct 头 <T>
    let d = def(src, 1, 9).unwrap();
    assert_eq!((d.line, d.col, d.len), (0, 16, 1), "字段类型 T → struct <T>");
    // 泛型实参 TreeNode<T> 里的 T → 同一个声明
    let d = def(src, 2, 19).unwrap();
    assert_eq!((d.line, d.col, d.len), (0, 16, 1), "泛型实参 T → struct <T>");
}

#[test]
fn definition_type_param_in_generic_function() {
    let src = "function id<T>(x: T) -> T {\n    return x\n}\n";
    // 参数标注 x: T 的 T（行0 col 18）→ 头部 <T>（行0 col 12）
    let d = def(src, 0, 18).unwrap();
    assert_eq!((d.line, d.col, d.len), (0, 12, 1), "T → 函数 <T>");
    // 体内 x → 参数声明处（泛型函数此前不扫描参数）
    let d = def(src, 1, 11).unwrap();
    assert_eq!((d.line, d.col, d.len), (0, 15, 1), "x → 泛型函数参数声明");
    // 匿名泛型 function<T>(x: T)
    let src2 = "f = function<T>(x: T) -> T {\n    return x\n}\n";
    let d = def(src2, 0, 19).unwrap();
    assert_eq!((d.line, d.col, d.len), (0, 13, 1), "匿名函数 T → <T>");
}

#[test]
fn definition_type_param_undeclared_returns_none() {
    // 没有任何泛型声明：非法的裸 T 不跳转，但不影响其他符号照常解析
    let src = "function g(v: T) {\n    return v\n}\n";
    assert!(def(src, 0, 14).is_none(), "未声明的 T 不跳转");
    assert!(def(src, 1, 11).is_some(), "参数 v 照常跳");
}

// ============ 类型标注着色 ============

#[test]
fn colors_type_annotations_everywhere() {
    let src = "\
x: number = 1
function add(a: number, b: number) -> number {
    return a + b
}
struct ListNode {
    val,
    next: ListNode
}
m: Map<string, number> = new Map()
";
    let m = token_map(src);
    // 行0：变量标注
    assert_eq!(m.get(&(0, 3)), Some(&"type"), "变量标注 number");
    // 行1：参数标注 ×2 + 返回类型
    assert_eq!(m.get(&(1, 16)), Some(&"type"), "参数 a 的类型");
    assert_eq!(m.get(&(1, 27)), Some(&"type"), "参数 b 的类型");
    assert_eq!(m.get(&(1, 38)), Some(&"type"), "返回类型");
    // 行6：struct 字段标注
    assert_eq!(m.get(&(6, 10)), Some(&"type"), "字段类型 ListNode");
    // 行8：泛型参数逐个标识符着色
    assert_eq!(m.get(&(8, 3)), Some(&"type"), "Map");
    assert_eq!(m.get(&(8, 7)), Some(&"type"), "string");
    assert_eq!(m.get(&(8, 15)), Some(&"type"), "number");
    // 行8：`new Map()` 的 Map 仍是 class，不被标注干扰
    assert_eq!(m.get(&(8, 29)), Some(&"class"), "new Map 保持 class");
}

#[test]
fn colors_types_in_interface_and_anonymous_function() {
    let src = "\
interface Shape {
    function area() -> number
}
f = function(a: number) -> number {
    return a
}
";
    let m = token_map(src);
    assert_eq!(m.get(&(1, 13)), Some(&"method"), "接口方法 area");
    assert_eq!(m.get(&(1, 23)), Some(&"type"), "接口方法返回类型");
    assert_eq!(m.get(&(3, 13)), Some(&"parameter"), "匿名函数参数 a");
    assert_eq!(m.get(&(3, 16)), Some(&"type"), "匿名函数参数类型");
    assert_eq!(m.get(&(3, 27)), Some(&"type"), "匿名函数返回类型");
}

#[test]
fn no_type_tokens_in_value_positions() {
    // 对象字面量值 / 三元分支：`:` 后是值不是类型，一个 type 都不该发
    let src = "\
o = { a: 1, b: \"s\" }
q = { c: o }
c = true ? x : y
";
    let m = token_map(src);
    assert!(
        !m.values().any(|&v| v == "type"),
        "值位置不应有 type token：{m:?}"
    );
}
