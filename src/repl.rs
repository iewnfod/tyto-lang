use std::io::{self, BufRead, Write};

use crate::{Interpreter, Lexer, Parser, RtError, Stmt};

/// 交互式 REPL：全局作用域跨行保持，裸表达式回显值，未闭合的块自动续行
pub fn run() {
    let mut interp = Interpreter::new();
    let stdin = io::stdin();
    let mut buffer = String::new();

    println!("tyto v{} REPL — Ctrl-D 退出", env!("CARGO_PKG_VERSION"));
    loop {
        print!("{}", if needs_more(&buffer) { ".. " } else { ">> " });
        io::stdout().flush().ok();

        let mut line = String::new();
        match stdin.lock().read_line(&mut line) {
            Ok(0) => break, // EOF
            Ok(_) => {}
            Err(_) => break,
        }
        buffer.push_str(&line);

        if buffer.trim().is_empty() {
            buffer.clear();
            continue;
        }
        if eval_buffer(&mut interp, &buffer) == Outcome::NeedMore {
            continue;
        }
        buffer.clear();
    }
}

#[derive(PartialEq, Eq)]
enum Outcome {
    NeedMore,
    Done,
}

fn eval_buffer(interp: &mut Interpreter, src: &str) -> Outcome {
    let tokens = match Lexer::new(src).tokenize() {
        Ok(out) => {
            if out.unclosed_depth > 0 {
                return Outcome::NeedMore;
            }
            out.tokens
        }
        Err(e) => {
            if is_unterminated(&e) {
                return Outcome::NeedMore;
            }
            eprintln!("{}", e.report(Some(src)));
            return Outcome::Done;
        }
    };

    let program = match Parser::new(tokens).parse_program() {
        Ok(p) => p,
        Err(e) => {
            if matches!(&e, RtError::Parse { message, .. } if message.contains("unclosed")) {
                return Outcome::NeedMore;
            }
            eprintln!("{}", e.report(Some(src)));
            return Outcome::Done;
        }
    };

    // 单个表达式语句：回显值（Node 控制台风格）
    if let Stmt::Block { stmts, .. } = &program {
        if stmts.len() == 1 {
            if let Stmt::Expr(expr, _) = &stmts[0] {
                match interp.evaluate(expr) {
                    Ok(v) => {
                        let mut out = interp.out.borrow_mut();
                        writeln!(out, "{}", v.to_repr()).ok();
                    }
                    Err(e) => eprintln!("{}", e.report(Some(src))),
                }
                return Outcome::Done;
            }
        }
    }

    if let Err(e) = interp.run(&program) {
        eprintln!("{}", e.report(Some(src)));
    }
    Outcome::Done
}

/// 输入是否还需要更多内容（未闭合的块/括号/字符串）
fn needs_more(src: &str) -> bool {
    match Lexer::new(src).tokenize() {
        Ok(out) => out.unclosed_depth > 0,
        Err(e) => is_unterminated(&e),
    }
}

fn is_unterminated(e: &RtError) -> bool {
    matches!(e, RtError::Lex { message, .. } if message.contains("unterminated"))
}
