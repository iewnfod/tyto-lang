use tyto_lang::{InRef, Interpreter, Lexer, OutRef, Parser, RtError};
use std::{cell::RefCell, fs, io::{BufReader, Cursor}, rc::Rc};

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
    let tokens = Lexer::new(src).tokenize().unwrap().tokens;
    let program = Parser::new(tokens).parse_program().unwrap();
    interp.run(&program).expect_err("expected error")
}

#[test]
fn fs_write_read_roundtrip() {
    let dir = "/tmp/opencode/tyto-test-a";
    let path = format!("{}/data.txt", dir);
    fs::create_dir_all(dir).unwrap();

    let src = format!(
        "fs.write_file(\"{p}\", \"line1\\nline2\\n\")\n\
         println(fs.read_file(\"{p}\"))\n\
         println(fs.exists(\"{p}\"), fs.exists(\"{p}/nope\"))",
        p = path
    );
    assert_eq!(run(&src), "line1\nline2\n\ntrue false\n");
}

#[test]
fn fs_read_lines_semantics() {
    let dir = "/tmp/opencode/tyto-test-b";
    let path = format!("{}/lines.txt", dir);
    fs::create_dir_all(dir).unwrap();
    fs::write(&path, "a\nb\r\nc\n").unwrap(); // 末尾换行不产生空尾元素

    let src = format!("println(fs.read_lines(\"{}\"))", path);
    assert_eq!(run(&src), "[\"a\", \"b\", \"c\"]\n");
}

#[test]
fn fs_append_creates_and_appends() {
    let dir = "/tmp/opencode/tyto-test-c";
    let path = format!("{}/log.txt", dir);
    let _ = fs::remove_file(&path);
    fs::create_dir_all(dir).unwrap();

    let src = format!(
        "fs.append_file(\"{p}\", \"one\\n\")\n\
         fs.append_file(\"{p}\", \"two\\n\")\n\
         println(fs.read_lines(\"{p}\"))",
        p = path
    );
    assert_eq!(run(&src), "[\"one\", \"two\"]\n");
}

#[test]
fn fs_list_dir_sorted() {
    let dir = "/tmp/opencode/tyto-test-d";
    let _ = fs::remove_dir_all(dir);
    fs::create_dir_all(dir).unwrap();
    fs::write(format!("{}/b.txt", dir), "x").unwrap();
    fs::write(format!("{}/a.txt", dir), "x").unwrap();
    fs::create_dir(format!("{}/sub", dir)).unwrap();

    let src = format!("println(fs.list_dir(\"{}\"))", dir);
    assert_eq!(run(&src), "[\"a.txt\", \"b.txt\", \"sub\"]\n");
}

#[test]
fn fs_errors_are_strict() {
    assert!(matches!(run_err("fs.read_file(\"/tmp/opencode/__nope__.txt\")"), RtError::Runtime { .. }));
    // 父目录不存在时写文件报错
    assert!(matches!(
        run_err("fs.write_file(\"/tmp/opencode/__nope_dir__/x.txt\", \"hi\")"),
        RtError::Runtime { .. }
    ));
    // 参数类型检查
    assert!(matches!(run_err("fs.read_file(42)"), RtError::Runtime { .. }));
}

#[test]
fn sys_shell_captures_status_stdout_stderr() {
    let src = "r = sys.shell(\"echo hi\")\nprintln(r.status, r.stdout, r.stderr)";
    assert_eq!(run(src), "0 hi\n \n");

    // 非零退出不是错误：正常返回对象
    assert_eq!(run("println(sys.shell(\"false\").status)"), "1\n");

    // stderr 捕获
    assert_eq!(run("println(sys.shell(\"echo err 1>&2\").stderr)"), "err\n\n");

    // 输出原样保留，trim 自己做
    assert_eq!(run("println(sys.shell(\"echo x\").stdout.trim())"), "x\n");
}

#[test]
fn sys_get_env() {
    assert_eq!(run("println(sys.get_env(\"PATH\") != null)"), "true\n");
    assert_eq!(run("println(sys.get_env(\"__TYTO_NOPE__\"))"), "null\n");
    assert!(matches!(run_err("sys.get_env(1)"), RtError::Runtime { .. }));
}

#[test]
fn sys_args_default_empty() {
    // 测试环境下未注入参数 → 空数组
    assert_eq!(run("println(sys.args(), sys.args().is_empty())"), "[] true\n");
}

#[test]
fn fs_and_sys_are_namespaced_objects() {
    assert_eq!(
        run("println(type(fs), type(sys), has(fs, \"read_file\"), has(sys, \"shell\"))"),
        "object object true true\n"
    );
}
