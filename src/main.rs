mod cli;

use std::{env, fs, process};

use tyto_lang::{Interpreter, Lexer, Parser};
use tyto_lang::repl;

use cli::Command;

fn main() {
    let args: Vec<String> = env::args().collect();
    match cli::parse(&args) {
        Command::Repl => repl::run(),
        Command::RunScript { path, script_args } => run_file(&path, script_args),
        Command::DumpTokens { path } => dump_tokens(&path),
        Command::DumpAst { path } => dump_ast(&path),
        Command::Lsp => tyto_lang::lsp::run(),
    }
}

fn read_source(path: &str) -> String {
    match fs::read_to_string(path) {
        Ok(src) => src,
        Err(e) => {
            eprintln!("cannot read file {}: {}", path, e);
            process::exit(2);
        }
    }
}

fn run_file(path: &str, script_args: Vec<String>) {
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
    interp.args = script_args;
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
