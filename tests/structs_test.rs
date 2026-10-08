//! struct / impl / interface / is 集成测试：跨模块走完整 lexer → parser → interpreter 链路

use std::{
    cell::RefCell,
    fs,
    io::{BufReader, Cursor},
    rc::Rc,
};
use tyto_lang::{InRef, Interpreter, Lexer, OutRef, Parser, RtError};

fn run_with_io(src: &str) -> (Interpreter, String) {
    let vec: Rc<RefCell<Vec<u8>>> = Rc::new(RefCell::new(Vec::new()));
    let out: OutRef = vec.clone();
    let input: InRef = Rc::new(RefCell::new(BufReader::new(Cursor::new(Vec::<u8>::new()))));
    let mut interp = Interpreter::with_io(out, input);
    let tokens = Lexer::new(src).tokenize().unwrap().tokens;
    let program = Parser::new(tokens).parse_program().unwrap();
    interp.run(&program).expect("runtime ok");
    let output = String::from_utf8(vec.borrow().clone()).unwrap();
    (interp, output)
}

fn run(src: &str) -> String {
    run_with_io(src).1
}

fn run_err(src: &str) -> RtError {
    let vec: Rc<RefCell<Vec<u8>>> = Rc::new(RefCell::new(Vec::new()));
    let out: OutRef = vec.clone();
    let input: InRef = Rc::new(RefCell::new(BufReader::new(Cursor::new(Vec::<u8>::new()))));
    let mut interp = Interpreter::with_io(out, input);
    let tokens = Lexer::new(src).tokenize().unwrap().tokens;
    let program = Parser::new(tokens).parse_program().unwrap();
    interp.run(&program).expect_err("expected error")
}

/// struct 版链表（对照 examples/objects.tyto 的对象版）：
/// 字段声明 + new 构造 + self 方法互调
#[test]
fn linked_list_with_struct() {
    let src = r#"
struct Node {
    value,
    next,
}

impl Node {
    function new(value, next) {
        self.value = value
        self.next = next
    }
    function to_list() {
        out = [self.value]
        cur = self.next
        while cur != null {
            out.push(cur.value)
            cur = cur.next
        }
        return out
    }
    function sum() {
        total = 0
        for v in self.to_list() { total += v }
        return total
    }
}

list = new Node(1, new Node(2, new Node(3, null)))
println(list.to_list())
println(list.sum())
println(list.value, list.next.value)
"#;
    assert_eq!(run(src), "[1, 2, 3]\n6\n1 2\n");
}

/// 排序回调里用 struct 方法；闭包捕获外层变量
#[test]
fn struct_with_higher_order_functions() {
    let src = r#"
struct Item {
    name,
    price,
}

items = [new Item("b", 3), new Item("a", 1), new Item("c", 2)]
names = items
    .map(function(it) { return it.name })
    .sort()
println(names.join(","))

prices = items
    .map(function(it) { return it.price })
    .sort(function(x, y) { return y - x })
println(prices)
"#;
    assert_eq!(run(src), "a,b,c\n[3, 2, 1]\n");
}

/// REPL 式两段执行：struct/impl 定义与使用分离（全局作用域持久）
#[test]
fn definitions_persist_across_runs() {
    let (mut interp, _) = run_with_io("struct P { x }\nimpl P { function f() { return self.x } }");
    let vec: Rc<RefCell<Vec<u8>>> = Rc::new(RefCell::new(Vec::new()));
    let tokens = Lexer::new("p = new P(42)\nprintln(p.f())").tokenize().unwrap().tokens;
    let program = Parser::new(tokens).parse_program().unwrap();
    let saved = std::mem::replace(&mut interp.out, vec.clone());
    interp.run(&program).expect("runtime ok");
    interp.out = saved;
    assert_eq!(String::from_utf8(vec.borrow().clone()).unwrap(), "42\n");
}

/// 错误信息带行号与摘录
#[test]
fn struct_errors_have_spans() {
    let err = run_err("struct P { x }\np = new P(1)\np.zz = 2");
    match err {
        RtError::Runtime { span: Some(s), message } => {
            assert!(message.contains("struct P has no field `zz`"), "{}", message);
            assert_eq!(s.line, 3);
        }
        other => panic!("expected runtime error, got {:?}", other),
    }

    // is 求值不满足时返回 false，不报错
    assert_eq!(
        run("interface I { function f() }\nstruct P { x }\nprintln(new P() is I)"),
        "false\n"
    );
}

/// 多 interface、空 interface、is 链式组合
#[test]
fn interface_variants() {
    let src = r#"
interface Walker { function walk() }
interface Swimmer { function swim() }

struct Duck { name }
impl Duck {
    function walk() { return self.name + " walks" }
    function swim() { return self.name + " swims" }
}

struct Rock { }

d = new Duck("duck")
r = new Rock()
println(d is Walker, d is Swimmer, r is Walker, d is Duck)
println(r is Walker || d is Walker)
"#;
    assert_eq!(run(src), "true true false true\ntrue\n");
}

/// struct 定义可以出现在函数内（局部作用域绑定）；实例可逃逸，类型名不可
#[test]
fn struct_inside_function_scope() {
    let make = "function make() {\n    struct T { v }\n    return new T(7)\n}\nt = make()\n";
    assert_eq!(run(&format!("{}println(t.v)", make)), "7\n");
    assert!(matches!(
        run_err(&format!("{}println(T)", make)),
        RtError::Runtime { ref message, .. } if message.contains("undefined variable `T`")
    ));
}

/// fs 命名空间读取 struct 脚本并执行（确保与文件 IO 互不干扰的冒烟）
#[test]
fn struct_example_file_runs() {
    let src = fs::read_to_string("examples/shapes.tyto").unwrap();
    let (_, out) = run_with_io(&src);
    assert_eq!(
        out,
        "Rect 3x4 -> 12\nCircle r=2 -> 12.56636\nobject shape -> 1\ntotal: 25.56636\n5\nfalse true\n"
    );
}
