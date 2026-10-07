use tyto_lang::{InRef, Interpreter, Lexer, OutRef, Parser, RtError};
use std::{cell::RefCell, io::{BufReader, Cursor}, rc::Rc};

fn run_with_input(src: &str, stdin: &str) -> String {
    let vec: Rc<RefCell<Vec<u8>>> = Rc::new(RefCell::new(Vec::new()));
    let out: OutRef = vec.clone();
    let input: InRef =
        Rc::new(RefCell::new(BufReader::new(Cursor::new(stdin.as_bytes().to_vec()))));
    let mut interp = Interpreter::with_io(out, input);
    let tokens = Lexer::new(src).tokenize().unwrap().tokens;
    let program = Parser::new(tokens).parse_program().unwrap();
    interp.run(&program).expect("runtime ok");
    String::from_utf8(vec.borrow().clone()).unwrap()
}

fn run(src: &str) -> String {
    run_with_input(src, "")
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

// ============ 全局函数 ============

#[test]
fn input_reads_lines_and_eof_null() {
    let src = "a = input()\nb = input()\nc = input()\nprintln(a + b)\nprintln(c)";
    assert_eq!(run_with_input(src, "42\nabc\n"), "42abc\nnull\n");
}

#[test]
fn num_conversions() {
    assert_eq!(run("println(num(\"3.5\") + 1)"), "4.5\n");
    assert_eq!(run("println(num(\"  7 \"))"), "7\n");
    assert_eq!(run("println(num(3))"), "3\n");
    assert!(matches!(run_err("println(num(\"abc\"))"), RtError::Runtime { .. }));
    // 经典组合：解析输入
    assert_eq!(run_with_input("n = num(input())\nprintln(n * 2)", "21\n"), "42\n");
}

#[test]
fn str_len_type_has() {
    assert_eq!(run("println(str(1.5) + \"x\")"), "1.5x\n");
    assert_eq!(run("println(str(null))"), "null\n");
    assert_eq!(run("println(str([1, 2]))"), "[1, 2]\n");
    assert_eq!(run("println(len(\"abc\"), len([1, 2]), len({a: 1, b: 2}), len(new Map()))"), "3 2 2 0\n");
    assert_eq!(
        run("println(type(1), type(\"a\"), type([]), type({}), type(null), type(print), type(function() {}))"),
        "number string array object null native function function\n"
    );
    assert_eq!(
        run("println(has({a: 1}, \"a\"), has({a: 1}, \"b\"))"),
        "true false\n"
    );
}

#[test]
fn math_globals() {
    assert_eq!(
        run("println(floor(1.7), ceil(1.2), round(2.5), abs(-3), sqrt(9), pow(2, 10))"),
        "1 2 3 3 3 1024\n"
    );
    assert_eq!(run("println(min(3, 1, 2), max(3, 1, 2))"), "1 3\n");
    assert_eq!(run("println(min([5, 2, 8]), max([5, 2, 8]))"), "2 8\n");
    assert!(matches!(run_err("min()"), RtError::Runtime { .. }));
}

// ============ 数组 ============

#[test]
fn array_push_pop_len_isEmpty() {
    let src = "a = [1]\n\
               println(a.push(2, 3))\n\
               println(a)\n\
               println(a.len(), a.is_empty())\n\
               println(a.pop())\n\
               println(a.pop(), a.pop())\n\
               println(a.is_empty())";
    // pop 空数组返回 null（不是 EMPTY）
    assert_eq!(run(src), "3\n[1, 2, 3]\n3 false\n3\n2 1\ntrue\n");
}

#[test]
fn array_contains_indexOf_join() {
    let src = "a = [1, 2, 3]\n\
               println(a.contains(2), a.contains(9))\n\
               println(a.index_of(3), a.index_of(9))\n\
               println(a.join(\"-\"))\n\
               println(a.join())";
    assert_eq!(run(src), "true false\n2 -1\n1-2-3\n1,2,3\n");
}

#[test]
fn array_sort_default_and_comparator() {
    assert_eq!(run("a = [3, 1, 2]\na.sort()\nprintln(a)"), "[1, 2, 3]\n");
    assert_eq!(run("a = [\"c\", \"a\", \"b\"]\na.sort()\nprintln(a)"), "[\"a\", \"b\", \"c\"]\n");
    // 比较器降序
    let src = "a = [3, 1, 2]\na.sort(function(x, y) { return y - x })\nprintln(a)";
    assert_eq!(run(src), "[3, 2, 1]\n");
    // 混合类型无比较器报错
    assert!(matches!(run_err("a = [1, \"a\"]\na.sort()"), RtError::Runtime { .. }));
    // 返回自身可链式
    assert_eq!(run("a = [2, 1]\nprintln(a.sort().reverse())"), "[2, 1]\n");
}

#[test]
fn array_reverse_slice() {
    assert_eq!(run("println([1, 2, 3].reverse())"), "[3, 2, 1]\n");
    assert_eq!(run("println([1, 2, 3, 4].slice(1, 3))"), "[2, 3]\n");
    assert_eq!(run("println([1, 2, 3, 4].slice(-2))"), "[3, 4]\n");
    assert_eq!(run("println([1, 2, 3].slice(5, 9))"), "[]\n");
}

#[test]
fn array_map_filter_fold() {
    assert_eq!(
        run("println([1, 2, 3].map(function(x) { return x * 2 }))"),
        "[2, 4, 6]\n"
    );
    assert_eq!(
        run("println([1, 2, 3, 4].filter(function(x) { return x % 2 == 0 }))"),
        "[2, 4]\n"
    );
    assert_eq!(
        run("println([1, 2, 3].fold(0, function(acc, x) { return acc + x }))"),
        "6\n"
    );
    // 结合对象/字符串
    assert_eq!(
        run("println([\"a\", \"bb\"].map(function(s) { return s.len() }))"),
        "[1, 2]\n"
    );
}

// ============ 字符串 ============

#[test]
fn string_methods() {
    let src = "s = \"  Hello, World  \"\n\
               println(s.len())\n\
               println(s.trim())\n\
               println(s.trim().to_lowercase())\n\
               println(s.trim().to_uppercase())\n\
               println(s.contains(\"lo,\") , s.starts_with(\"  H\"), s.ends_with(\"  \"))\n\
               println(s.trim().sub(0, 5))\n\
               println(s.trim().sub(-5))\n\
               println(\"a-b-c\".split(\"-\"))\n\
               println(\"abc\".split(\"\"))\n\
               println(\"abc\".chars())\n\
               println(\"hello world\".index_of(\"world\"))\n\
               println(\"a-b\".replace(\"-\", \"+\"))\n\
               println(\"\".is_empty())";
    assert_eq!(
        run(src),
        "16\nHello, World\nhello, world\nHELLO, WORLD\ntrue true true\nHello\nWorld\n[\"a\", \"b\", \"c\"]\n[\"a\", \"b\", \"c\"]\n[\"a\", \"b\", \"c\"]\n6\na+b\ntrue\n"
    );
}

#[test]
fn string_sub_slice_semantics() {
    assert_eq!(run("println(\"hello\".sub(1, 3))"), "el\n");
    assert_eq!(run("println(\"hello\".sub(-3, -1))"), "ll\n");
    assert_eq!(run("println(\"hello\".sub(2, 1))"), "\n");
}

// ============ Map ============

#[test]
fn map_basics() {
    let src = "m = new Map()\n\
               println(m.is_empty())\n\
               m.insert(\"a\", 1)\n\
               m.insert(2, \"two\")\n\
               m.insert(true, 3)\n\
               println(m.len())\n\
               println(m.get(\"a\"), m.get(2), m.get(true))\n\
               println(m.get(\"missing\"))\n\
               println(m.contains_key(\"a\"), m.contains_key(\"missing\"))\n\
               println(m.keys())\n\
               println(m.values())\n\
               println(m.remove(2), m.contains_key(2), m.len())\n\
               println(m)";
    assert_eq!(
        run(src),
        "true\n3\n1 two 3\nnull\ntrue false\n[\"a\", 2, true]\n[1, \"two\", 3]\ntrue false 2\nMap {\"a\": 1, true: 3}\n"
    );
}

#[test]
fn map_key_rules() {
    // 整数键与浮点键同键（1 == 1.0）
    assert_eq!(run("m = new Map()\nm.insert(1, \"a\")\nm.insert(1.0, \"b\")\nprintln(m.len(), m.get(1))"), "1 b\n");
    // null 可做键
    assert_eq!(run("m = new Map()\nm.insert(null, 0)\nprintln(m.get(null))"), "0\n");
    // 数组不能做键
    assert!(matches!(run_err("new Map().insert([], 1)"), RtError::Runtime { .. }));
}

#[test]
fn map_set_is_chainable() {
    assert_eq!(
        run("m = new Map().insert(\"a\", 1).insert(\"b\", 2)\nprintln(m.len())"),
        "2\n"
    );
}

// ============ 堆 ============

#[test]
fn heap_order_and_empty_sentinel() {
    let src = "h = new MaxHeap()\n\
               println(h.is_empty())\n\
               println(h.pop())\n\
               println(h.peek())\n\
               h.push(3)\n\
               h.push(1)\n\
               h.push(4)\n\
               println(h.peek())\n\
               println(h.len())\n\
               println(h.pop(), h.pop(), h.pop(), h.pop())";
    assert_eq!(run(src), "true\nEMPTY\nEMPTY\n4\n3\n4 3 1 EMPTY\n");
}

#[test]
fn min_heap_sorted_output() {
    let src = "h = new MinHeap()\n\
               for x in [5, 3, 8, 1] { h.push(x) }\n\
               out = []\n\
               while !h.is_empty() { out.push(h.pop()) }\n\
               println(out)";
    assert_eq!(run(src), "[1, 3, 5, 8]\n");
}

#[test]
fn heap_heapify_and_type_rules() {
    assert_eq!(run("println(new MaxHeap([2, 9, 4]).peek())"), "9\n");
    assert_eq!(run("println(new MinHeap([2, 9, 4]).peek())"), "2\n");
    assert_eq!(run("println(new MaxHeap([3, 1]).len())"), "2\n");
    // 堆只收数字
    assert!(matches!(run_err("new MaxHeap().push(\"x\")"), RtError::Runtime { .. }));
    assert!(matches!(run_err("new MaxHeap([\"a\"])"), RtError::Runtime { .. }));
}

#[test]
fn heap_median_two_heaps_pattern() {
    // 双堆求中位数的骨架（不依赖全局变量名）
    let src = "low = new MaxHeap()\n\
               high = new MinHeap()\n\
               function insert(x) {\n\
               \x20   if low.is_empty() || x <= low.peek() { low.push(x) } else { high.push(x) }\n\
               \x20   if low.len() > high.len() + 1 { high.push(low.pop()) }\n\
               \x20   if high.len() > low.len() { low.push(high.pop()) }\n\
               }\n\
               function median() {\n\
               \x20   if low.len() == high.len() { return (low.peek() + high.peek()) / 2 }\n\
               \x20   return low.peek()\n\
               }\n\
               for x in [5, 1, 3, 2] { insert(x) }\n\
               println(median())";
    assert_eq!(run(src), "2.5\n");
}
