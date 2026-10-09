//! 解释器端到端单元测试（按主题拆分；辅助 run/run_err 在此，用例在子模块）。

mod basics;
mod objects;
mod structs;

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

