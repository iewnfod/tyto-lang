//! 命令行参数解析 —— 参考 caie-code（CAIE_Code）的手动处理设计：
//! 选项注册表 [`OPTIONS`] + 回调写入 [`Settings`]，帮助信息由注册表自动生成。
//!
//! 扫描规则（对应 caie-code `main()` 里的循环）：
//! 命中选项 → 执行回调并消耗 `value_num` 个后续参数；
//! `-` 开头的未知参数 → 报错；`--` 之后不再解析选项；
//! 其余按位置参数处理：第一个是脚本文件，后面的经 `sys.args()` 传给脚本。

use std::process;

use colored::Colorize;

/// 选项回调写入的运行配置（对应 caie-code 的 options_dict）
#[derive(Default)]
pub struct Settings {
    dump_tokens: bool,
    dump_ast: bool,
    lsp: bool,
}

/// 解析 argv 得到的执行指令
#[derive(Debug, PartialEq)]
pub enum Command {
    Repl,
    RunScript { path: String, script_args: Vec<String> },
    DumpTokens { path: String },
    DumpAst { path: String },
    /// `tyto lsp`：stdio 语言服务器（编辑器插件用）
    Lsp,
}

/// 单个选项（对应 caie-code 的 Opt 类）：
/// 短参、长参、描述、消耗的后续参数个数、回调、执行后是否直接退出
struct Opt {
    short: &'static str,
    long: &'static str,
    description: &'static str,
    value_num: usize,
    run: fn(&mut Settings, &[String]),
    exit_after: bool,
}

impl Opt {
    fn matches(&self, arg: &str) -> bool {
        arg == self.short || arg == self.long
    }
}

fn set_dump_tokens(s: &mut Settings, _: &[String]) {
    s.dump_tokens = true;
}

fn set_dump_ast(s: &mut Settings, _: &[String]) {
    s.dump_ast = true;
}

fn show_help(_: &mut Settings, _: &[String]) {
    print_help();
}

fn show_version(_: &mut Settings, _: &[String]) {
    println!("tyto v{}", env!("CARGO_PKG_VERSION"));
}

/// 选项注册表：新增选项只需在这里加一项
const OPTIONS: &[Opt] = &[
    Opt {
        short: "-t",
        long: "--tokens",
        description: "Lex only: print the token stream",
        value_num: 0,
        run: set_dump_tokens,
        exit_after: false,
    },
    Opt {
        short: "-a",
        long: "--ast",
        description: "Parse only: print the AST",
        value_num: 0,
        run: set_dump_ast,
        exit_after: false,
    },
    Opt {
        short: "-h",
        long: "--help",
        description: "Show this help message",
        value_num: 0,
        run: show_help,
        exit_after: true,
    },
    Opt {
        short: "-v",
        long: "--version",
        description: "Show version",
        value_num: 0,
        run: show_version,
        exit_after: true,
    },
];

pub fn parse(argv: &[String]) -> Command {
    let mut settings = Settings::default();
    let mut file: Option<String> = None;
    let mut script_args: Vec<String> = Vec::new();
    let mut no_more_options = false;

    let mut i = 1;
    while i < argv.len() {
        let arg = argv[i].as_str();
        if no_more_options {
            push_positional(arg, &mut file, &mut script_args);
        } else if arg == "--" {
            no_more_options = true;
        } else if let Some(opt) = OPTIONS.iter().find(|o| o.matches(arg)) {
            let values = match argv.get(i + 1..i + 1 + opt.value_num) {
                Some(v) => v,
                None => missing_value(opt),
            };
            (opt.run)(&mut settings, values);
            if opt.exit_after {
                process::exit(0);
            }
            i += opt.value_num;
        } else if arg.starts_with('-') && arg.len() > 1 {
            wrong_argument(arg);
        } else if arg == "lsp" && file.is_none() && script_args.is_empty() {
            // 子命令风格：首个位置参数为 `lsp` → 语言服务器。
            // 真有脚本叫 lsp 时用 `./lsp` 或放在 `--` 之后即可区分。
            settings.lsp = true;
        } else {
            push_positional(arg, &mut file, &mut script_args);
        }
        i += 1;
    }

    match file {
        // 两个调试开关都给时 --tokens 优先
        Some(path) if settings.dump_tokens => Command::DumpTokens { path },
        Some(path) if settings.dump_ast => Command::DumpAst { path },
        Some(path) => Command::RunScript { path, script_args },
        None if settings.dump_tokens => missing_file("--tokens"),
        None if settings.dump_ast => missing_file("--ast"),
        None if settings.lsp => Command::Lsp,
        None => Command::Repl,
    }
}

/// 位置参数：第一个是脚本文件，其余传给脚本
fn push_positional(arg: &str, file: &mut Option<String>, script_args: &mut Vec<String>) {
    if file.is_none() {
        *file = Some(arg.to_owned());
    } else {
        script_args.push(arg.to_owned());
    }
}

/// 帮助信息：选项列表由 [`OPTIONS`] 自动生成
fn print_help() {
    println!("tyto — Tyto language interpreter v{}", env!("CARGO_PKG_VERSION"));
    println!();
    println!("Usage: tyto [file.tyto] [options] [-- script_args...]");
    println!("       tyto lsp                (stdio language server for editors)");
    println!();
    println!("  With no arguments, starts the REPL; arguments after <file> are passed to the script via sys.args()");
    println!("  Arguments after `--` are never parsed as options and are passed to the script as-is");
    println!();
    println!("Options:");
    let width = OPTIONS.iter().map(|o| o.long.len()).max().unwrap_or(0);
    for opt in OPTIONS {
        println!(
            "  {}, {:<width$}  {}",
            opt.short.bold(),
            opt.long,
            opt.description,
            width = width,
        );
    }
}

fn wrong_argument(arg: &str) -> ! {
    eprintln!("unknown argument: {arg}");
    eprintln!("use `tyto -h` for help");
    process::exit(2);
}

fn missing_value(opt: &Opt) -> ! {
    eprintln!("{} requires a value", opt.long);
    eprintln!("use `tyto -h` for help");
    process::exit(2);
}

fn missing_file(flag: &str) -> ! {
    eprintln!("{flag} requires a file argument");
    eprintln!("use `tyto -h` for help");
    process::exit(2);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_args(args: &[&str]) -> Command {
        let argv: Vec<String> = std::iter::once("tyto")
            .chain(args.iter().copied())
            .map(String::from)
            .collect();
        parse(&argv)
    }

    #[test]
    fn no_args_enters_repl() {
        assert_eq!(parse_args(&[]), Command::Repl);
    }

    #[test]
    fn first_positional_is_script_rest_are_args() {
        assert_eq!(
            parse_args(&["s.tyto"]),
            Command::RunScript { path: "s.tyto".into(), script_args: vec![] }
        );
        assert_eq!(
            parse_args(&["s.tyto", "a", "b"]),
            Command::RunScript { path: "s.tyto".into(), script_args: vec!["a".into(), "b".into()] }
        );
    }

    #[test]
    fn dump_flags_long_and_short_anywhere() {
        assert_eq!(parse_args(&["--tokens", "f.tyto"]), Command::DumpTokens { path: "f.tyto".into() });
        assert_eq!(parse_args(&["-t", "f.tyto"]), Command::DumpTokens { path: "f.tyto".into() });
        assert_eq!(parse_args(&["f.tyto", "--ast"]), Command::DumpAst { path: "f.tyto".into() });
        assert_eq!(parse_args(&["f.tyto", "-a"]), Command::DumpAst { path: "f.tyto".into() });
    }

    #[test]
    fn tokens_takes_precedence_over_ast() {
        assert_eq!(
            parse_args(&["-t", "-a", "f.tyto"]),
            Command::DumpTokens { path: "f.tyto".into() }
        );
    }

    #[test]
    fn separator_passes_dash_args_to_script() {
        assert_eq!(
            parse_args(&["s.tyto", "--", "-t", "b"]),
            Command::RunScript {
                path: "s.tyto".into(),
                script_args: vec!["-t".into(), "b".into()],
            }
        );
    }

    #[test]
    fn lsp_subcommand() {
        assert_eq!(parse_args(&["lsp"]), Command::Lsp);
    }

    #[test]
    fn script_named_lsp_still_runs_after_flag() {
        // 文件先出现时 `lsp` 只是普通脚本参数
        assert_eq!(
            parse_args(&["s.tyto", "lsp"]),
            Command::RunScript { path: "s.tyto".into(), script_args: vec!["lsp".into()] }
        );
        // `--` 之后不解析选项，`lsp` 原样传给脚本
        assert_eq!(
            parse_args(&["s.tyto", "--", "lsp"]),
            Command::RunScript {
                path: "s.tyto".into(),
                script_args: vec!["lsp".into()],
            }
        );
    }
}
