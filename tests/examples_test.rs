use tyto_lang::{InRef, Interpreter, Lexer, OutRef, Parser};
use std::{cell::RefCell, fs, io::{BufReader, Cursor}, rc::Rc};

fn run_file(path: &str) -> String {
    let src = fs::read_to_string(path).unwrap();
    let vec: Rc<RefCell<Vec<u8>>> = Rc::new(RefCell::new(Vec::new()));
    let out: OutRef = vec.clone();
    let input: InRef = Rc::new(RefCell::new(BufReader::new(Cursor::new(Vec::<u8>::new()))));
    let mut interp = Interpreter::with_io(out, input);
    let tokens = Lexer::new(&src).tokenize().unwrap().tokens;
    let program = Parser::new(tokens).parse_program().unwrap();
    interp.run(&program).expect("runtime ok");
    String::from_utf8(vec.borrow().clone()).unwrap()
}

/// 用户的 median 伪代码逐字运行（examples/median.tyto 前半部分为原始伪代码，
/// 仅追加底部驱动）。手工推演：插入 5,1,3,2 → median=2.5；delete_median→2；再 median→3。
#[test]
fn median_example_from_user_pseudocode() {
    assert_eq!(run_file("examples/median.tyto"), "2.5\n2\n3\n");
}

#[test]
fn fib_with_memo_map() {
    assert_eq!(
        run_file("examples/fib.tyto"),
        "0 1 1 2 3 5 8 13 21 34 \n12586269025\n"
    );
}

#[test]
fn word_count_example() {
    assert_eq!(
        run_file("examples/word_count.tyto"),
        "the: 3\nquick: 3\nlazy: 1\ndog: 1\n"
    );
}

#[test]
fn objects_example() {
    assert_eq!(run_file("examples/objects.tyto"), "[1, 2, 3]\n6\n2\n");
}

/// 中位数删除序列的更完整推演
#[test]
fn median_stream_sequence() {
    let src = r#"
h_low = null
h_high = null

function init() { h_low = new MaxHeap()
h_high = new MinHeap() }

function balance() {
    if h_low.len() > h_high.len() + 1 { h_high.push(h_low.pop()) }
    if h_high.len() > h_low.len() { h_low.push(h_high.pop()) }
}

function insert(x) {
    if h_low.is_empty() || x <= h_low.peek() { h_low.push(x) } else { h_high.push(x) }
    balance()
}

function median() {
    if h_low.is_empty() && h_high.is_empty() { return EMPTY }
    if h_low.len() == h_high.len() { return (h_low.peek() + h_high.peek()) / 2 }
    return h_low.peek()
}

function delete_median() {
    if h_low.is_empty() && h_high.is_empty() { return EMPTY }
    m = h_low.pop()
    balance()
    return m
}

init()
for x in [7, 3, 9, 1, 5, 5, 11] { insert(x) }
// 排序后: 1 3 5 5 7 9 11，中位数 5
println(median())
println(delete_median())
// 删掉一个 5 后: 1 3 5 7 9 11，中位数 (5+7)/2 = 6
println(median())
println(delete_median())
println(delete_median())
println(delete_median())
println(delete_median())
println(delete_median())
println(delete_median())
// 删空后
println(median())
println(delete_median())
"#;
    let vec: Rc<RefCell<Vec<u8>>> = Rc::new(RefCell::new(Vec::new()));
    let out: OutRef = vec.clone();
    let input: InRef = Rc::new(RefCell::new(BufReader::new(Cursor::new(Vec::<u8>::new()))));
    let mut interp = Interpreter::with_io(out, input);
    let tokens = Lexer::new(src).tokenize().unwrap().tokens;
    let program = Parser::new(tokens).parse_program().unwrap();
    interp.run(&program).expect("runtime ok");
    let output = String::from_utf8(vec.borrow().clone()).unwrap();
    // 逐步推演（删的是当前较小中位数，再平衡会改变后续顺序）：
    // 插入 7,3,9,1,5,5,11 → 排序 1 3 5 5 7 9 11，中位数 5
    // 删 5 → 1 3 5 7 9 11，中位数 6；删 5 → 1 3 7 9 11，中位数 7
    // 删 7 → 1 3 9 11；删 3 → 1 9 11；删 9 → 1 11；删 1 → 11；删 11 → 空
    assert_eq!(output, "5\n5\n6\n5\n7\n3\n9\n1\n11\nEMPTY\nEMPTY\n");
}
