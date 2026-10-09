mod cli;

use std::{env, fs, process, time::Instant};

use tyto_lang::{Interpreter, Lexer, Parser};
use tyto_lang::checker::{check_program, diag::Severity};
use tyto_lang::repl;

use cli::Command;

fn main() {
    let args: Vec<String> = env::args().collect();
    match cli::parse(&args) {
        Command::Repl => repl::run(),
        Command::RunScript { path, script_args, time, skip_check } => {
            run_file(&path, script_args, time, skip_check)
        }
        Command::DumpTokens { path } => dump_tokens(&path),
        Command::DumpAst { path } => dump_ast(&path),
        Command::Check { path } => check_file(&path),
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

/// 检查器诊断的 stderr 渲染：`path:line:col: error: message`（编译器风格）
fn print_diags(path: &str, diags: &[tyto_lang::checker::diag::Diagnostic]) -> usize {
    use colored::Colorize;
    let mut errors = 0usize;
    for d in diags {
        let (label, msg) = match d.severity {
            Severity::Error => {
                errors += 1;
                ("error".red().bold(), d.message.red())
            }
            Severity::Warning => ("warning".yellow().bold(), d.message.yellow()),
        };
        eprintln!("{}:{}:{}: {}: {}", path, d.span.line, d.span.col, label, msg);
    }
    errors
}

/// `tyto check file.tyto`：词法/语法 + 类型检查，不执行
fn check_file(path: &str) {
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
    let out = check_program(&program);
    let errors = print_diags(path, &out.diagnostics);
    if errors > 0 {
        process::exit(1);
    }
    if out.diagnostics.is_empty() {
        use colored::Colorize;
        eprintln!("{}: no issues found", path.dimmed());
    }
}

fn run_file(path: &str, script_args: Vec<String>, time: bool, skip_check: bool) {
    let start = Instant::now();
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
    // 渐进类型：运行前打印类型诊断（擦除语义——只提示，不阻塞执行）
    if !skip_check {
        let out = check_program(&program);
        if !out.diagnostics.is_empty() {
            print_diags(path, &out.diagnostics);
        }
    }
    let mut interp = Interpreter::new();
    // `tyto script.tyto a b` → sys.args() == ["a", "b"]
    interp.args = script_args;
    if let Err(e) = interp.run(&program) {
        eprintln!("{}", e.report(Some(&src)));
        process::exit(1);
    }
    if time {
        // Duration 的 Debug 格式自带单位（如 1.234s、5.678ms），stderr 不污染脚本输出
        eprintln!("elapsed: {:.3?}", start.elapsed());
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
