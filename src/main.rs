use std::{env, fs, process};

use tyto_lang::repl;
use tyto_lang::{Interpreter, Lexer, Parser};

fn main() {
    let args: Vec<String> = env::args().collect();
    match args.get(1).map(String::as_str) {
        None => repl::run(),
        Some("-h") | Some("--help") => print_usage(),
        Some("--tokens") => match args.get(2) {
            Some(path) => dump_tokens(path),
            None => {
                eprintln!("--tokens 需要一个文件参数");
                process::exit(2);
            }
        },
        Some("--ast") => match args.get(2) {
            Some(path) => dump_ast(path),
            None => {
                eprintln!("--ast 需要一个文件参数");
                process::exit(2);
            }
        },
        Some(path) => run_file(path),
    }
}

fn print_usage() {
    println!(
        "tyto — Tyto 语言解释器 v{}\n\n\
         用法:\n\
         \x20 tyto              进入 REPL\n\
         \x20 tyto <file.tyto>   运行脚本\n\
         \x20 tyto --tokens <file>  仅词法分析\n\
         \x20 tyto --ast <file>     仅语法分析",
        env!("CARGO_PKG_VERSION")
    );
}

fn read_source(path: &str) -> String {
    match fs::read_to_string(path) {
        Ok(src) => src,
        Err(e) => {
            eprintln!("无法读取文件 {}: {}", path, e);
            process::exit(2);
        }
    }
}

fn run_file(path: &str) {
    let src = read_source(path);
    let tokens = match Lexer::new(&src).tokenize() {
        Ok(out) => out.tokens,
        Err(e) => {
            eprintln!("{}", e.report(Some(&src)));
            process::exit(1);
        }
    };
    let program = match Parser::new(tokens).parse_program() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{}", e.report(Some(&src)));
            process::exit(1);
        }
    };
    let mut interp = Interpreter::new();
    // `tyto script.tyto a b` → sys.args() == ["a", "b"]
    interp.args = std::env::args().skip(2).collect();
    if let Err(e) = interp.run(&program) {
        eprintln!("{}", e.report(Some(&src)));
        process::exit(1);
    }
}

fn dump_tokens(path: &str) {
    let src = read_source(path);
    match Lexer::new(&src).tokenize() {
        Ok(out) => {
            for t in &out.tokens {
                println!("{:>4}:{:<3} {:?}", t.span.line, t.span.col, t.kind);
            }
        }
        Err(e) => {
            eprintln!("{}", e.report(Some(&src)));
            process::exit(1);
        }
    }
}

fn dump_ast(path: &str) {
    let src = read_source(path);
    let tokens = match Lexer::new(&src).tokenize() {
        Ok(out) => out.tokens,
        Err(e) => {
            eprintln!("{}", e.report(Some(&src)));
            process::exit(1);
        }
    };
    match Parser::new(tokens).parse_program() {
        Ok(program) => println!("{:#?}", program),
        Err(e) => {
            eprintln!("{}", e.report(Some(&src)));
            process::exit(1);
        }
    }
}
