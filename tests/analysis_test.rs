//! 分析核心（补全/悬停/类型推导）集成测试。
//!
//! 位置一律用 LSP 习惯：0-based 行、UTF-16 列（与 complete/hover 入参一致）。

use tyto_lang::analysis::{complete, hover, ItemKind};

fn labels(items: &[tyto_lang::analysis::CompleteItem]) -> Vec<String> {
    items.iter().map(|i| i.label.clone()).collect()
}

fn find<'a>(
    items: &'a [tyto_lang::analysis::CompleteItem],
    label: &str,
) -> Option<&'a tyto_lang::analysis::CompleteItem> {
    items.iter().find(|i| i.label == label)
}

/// 光标位置：`§` 标记处（utf-16 列 = 标记在行内的 UTF-16 偏移）
fn at(src: &str) -> (String, usize, usize) {
    let mut cut = None;
    for (i, c) in src.char_indices() {
        if c == '§' {
            cut = Some(i);
            break;
        }
    }
    let cut = cut.expect("测试源码需含 § 光标标记");
    let before = &src[..cut];
    let line = before.matches('\n').count();
    let line_start = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
    let col = before[line_start..].chars().map(|c| c.len_utf16()).sum::<usize>();
    let rest = &src[cut + '§'.len_utf8()..];
    (format!("{}{}", before, rest), line, col)
}

fn complete_at(src: &str) -> Vec<tyto_lang::analysis::CompleteItem> {
    let (text, l, c) = at(src);
    complete(&text, l, c)
}

fn hover_at(src: &str) -> Option<tyto_lang::analysis::HoverInfo> {
    let (text, l, c) = at(src);
    hover(&text, l, c)
}

// ============ 全局补全：变量与函数可见性 ============

#[test]
fn global_completion_includes_variables_with_types() {
    let items = complete_at("n = 42\ns = \"hi\"\narr = [1, 2]\n§");
    for (name, detail) in [("n", "n: number"), ("s", "s: string"), ("arr", "arr: number[]")] {
        let item = find(&items, name).unwrap_or_else(|| panic!("缺 {name}"));
        assert_eq!(item.detail, detail);
        assert_eq!(item.kind, ItemKind::Variable);
    }
}

#[test]
fn global_completion_includes_user_functions_with_signature() {
    let items = complete_at(
        "function add(a: number, b: number) -> number {\n    return a + b\n}\nfunction plain(x) { return x }\n§",
    );
    let add = find(&items, "add").expect("缺 add");
    assert_eq!(add.detail, "function add(a: number, b: number) -> number");
    assert_eq!(add.kind, ItemKind::Function);
    let plain = find(&items, "plain").expect("缺 plain");
    assert_eq!(plain.detail, "function plain(x)");
}

#[test]
fn function_return_type_inferred_from_body() {
    // 无标注：return 表达式推导
    let items = complete_at("function make_greeting() {\n    return \"hello\"\n}\ng = make_greeting()\n§");
    let g = find(&items, "g").expect("缺 g");
    assert_eq!(g.detail, "g: string");
}

#[test]
fn completion_inside_function_sees_params_and_locals() {
    let src = "outside = 1\nfunction work(limit: number) {\n    total = 0\n    §";
    let items = complete_at(src);
    assert!(find(&items, "outside").is_some(), "全局变量应可见");
    let limit = find(&items, "limit").expect("缺参数 limit");
    assert_eq!(limit.detail, "limit: number");
    let total = find(&items, "total").expect("缺局部变量 total");
    assert_eq!(total.detail, "total: number");
}

#[test]
fn bindings_after_cursor_not_visible_at_top_level() {
    let items = complete_at("a = 1\n§\nb = 2\n");
    assert!(find(&items, "a").is_some());
    assert!(find(&items, "b").is_none(), "光标后的顶层绑定不应出现");
}

#[test]
fn struct_and_interface_in_global_completion() {
    let src = "struct Point {\n    x,\n    y,\n}\ninterface Shape {\n    function area()\n}\n§";
    let items = complete_at(src);
    let p = find(&items, "Point").expect("缺 struct");
    assert_eq!(p.kind, ItemKind::Struct);
    assert_eq!(p.detail, "struct Point { x, y }");
    let s = find(&items, "Shape").expect("缺 interface");
    assert_eq!(s.kind, ItemKind::Interface);
}

// ============ 函数体内编辑：全局最终态启发式 ============

#[test]
fn global_heuristic_sees_later_assignment_inside_function() {
    // 语言惯用形：顶层先 null，init 里赋值——函数体内编辑应看到最终类型
    let src = "h = null\n\nfunction push_val(v) {\n    h.§\n}\n\nh = new MaxHeap()\npush_val(1)\n";
    let items = complete_at(src);
    // MaxHeap 方法：push / pop / peek / len / is_empty —— 不含数组/字符串方法
    assert!(find(&items, "peek").is_some(), "应推出 h: maxheap");
    assert!(find(&items, "push").is_some());
    assert!(find(&items, "trim").is_none(), "不应出现 string 方法");
    assert!(find(&items, "join").is_none(), "不应出现 array 方法");
}

// ============ 成员补全：类型过滤 ============

#[test]
fn member_completion_string_methods_only() {
    let items = complete_at("s = \"hello\"\ns.§");
    assert!(find(&items, "to_uppercase").is_some());
    assert!(find(&items, "split").is_some());
    assert!(find(&items, "push").is_none(), "不应出现 array/堆方法");
    assert!(find(&items, "insert").is_none(), "不应出现 map 方法");
}

#[test]
fn member_completion_array_methods_only() {
    let items = complete_at("a = [1, 2, 3]\na.§");
    assert!(find(&items, "map").is_some());
    assert!(find(&items, "fold").is_some());
    assert!(find(&items, "to_uppercase").is_none());
}

#[test]
fn member_completion_map_methods() {
    let items = complete_at("m = new Map()\nm.§");
    assert!(find(&items, "contains_key").is_some());
    assert!(find(&items, "keys").is_some());
    assert!(find(&items, "pop").is_none());
}

#[test]
fn member_completion_namespace_fs() {
    let items = complete_at("fs.§");
    assert!(find(&items, "read_file").is_some());
    assert!(find(&items, "write_file").is_some());
    assert!(find(&items, "shell").is_none(), "sys 的方法不应出现");
    let items = complete_at("sys.§");
    assert!(find(&items, "shell").is_some());
    assert!(find(&items, "read_file").is_none());
}

#[test]
fn member_completion_through_method_return_chain() {
    // "a-b".split("-") → array，继续 . 给数组方法
    let items = complete_at("parts = \"a-b\".split(\"-\")\nparts.§");
    assert!(find(&items, "join").is_some());
    assert!(find(&items, "map").is_some());
    assert!(find(&items, "to_uppercase").is_none());
}

#[test]
fn member_completion_local_inference_inside_function() {
    let items = complete_at("function f() {\n    q = new Queue()\n    q.§\n}\n");
    assert!(find(&items, "push_back").is_some());
    assert!(find(&items, "push").is_none());
}

#[test]
fn member_completion_unknown_receiver_falls_back_to_all() {
    let items = complete_at("function f(x) {\n    return x.§\n}\n");
    // 未标注参数 x：未知类型 → 全方法池（含 array 与 string 方法）
    assert!(find(&items, "map").is_some());
    assert!(find(&items, "to_uppercase").is_some());
}

#[test]
fn member_completion_partial_word_after_dot() {
    // `p.ab|` 中途输入：剥掉残留词，接收者仍可解析
    let items = complete_at("struct P {\n    x,\n}\np = new P()\np.x = 1\np.ab§");
    assert!(find(&items, "x").is_some(), "struct 字段应补全");
}

// ============ 赋值目标 / 条件头里的成员补全 ============

#[test]
fn member_completion_in_assignment_target() {
    // `self.n += 1` 正在输入：光标在 self. 后（目标表达式里）
    let items = complete_at(
        "counter = {\n    n: 0,\n    inc: function() {\n        self.§ += 1\n    },\n}\n",
    );
    assert!(find(&items, "n").is_some(), "赋值目标 self. 应补出字段");
    assert!(find(&items, "inc").is_some(), "赋值目标 self. 应补出方法字段");
}

#[test]
fn member_completion_in_condition_headers() {
    // if / while 条件头、for-in 迭代目标（块还没写）
    let items = complete_at("s = \"abc\"\nif s.§\n");
    assert!(find(&items, "to_uppercase").is_some(), "if 条件头 string 方法");

    let items = complete_at("s = \"abc\"\nwhile s.§\n");
    assert!(find(&items, "len").is_some(), "while 条件头 string 方法");

    let items = complete_at("s = \"abc\"\nfor c in s.§\n");
    assert!(find(&items, "split").is_some(), "for 迭代目标 string 方法");
}
// ============ 联合标注与泛型容器的成员补全 ============

#[test]
fn union_annotation_strips_null_single_arm() {
    // number | null 去 null 后按 number：number 无方法 → 精确空集
    let items = complete_at("x: number | null = null\nx.§");
    assert!(find(&items, "map").is_none(), "number 臂不应给数组方法");
    assert!(find(&items, "to_uppercase").is_none(), "number 臂不应给字符串方法");
}

#[test]
fn union_annotation_member_intersection() {
    // array | string：两臂共有 len；map / to_uppercase 非共有 → 不出现
    let items = complete_at("x: array | string = null\nx.§");
    assert!(find(&items, "len").is_some(), "两臂共有 len 应保留");
    assert!(find(&items, "map").is_none(), "map 非两臂共有");
    assert!(find(&items, "to_uppercase").is_none(), "to_uppercase 非两臂共有");
}

#[test]
fn union_with_any_falls_back_to_all() {
    // 含 any 臂的联合无法收窄：全池（与未知接收者行为一致）
    let items = complete_at("x: any | null = null\nx.§");
    assert!(find(&items, "map").is_some());
    assert!(find(&items, "to_uppercase").is_some());
}

#[test]
fn generic_map_annotation_members() {
    // Map<string, number>：泛型实参不影响方法表分发，仍按 map 表给
    let items = complete_at("m: Map<string, number> = new Map()\nm.§");
    assert!(find(&items, "contains_key").is_some());
    assert!(find(&items, "push").is_none(), "不应出现数组方法");
}

#[test]
fn hover_map_annotated_member() {
    let h = hover_at("m: Map<string, number> = new Map()\nm.get§(\"k\")").unwrap();
    assert!(h.signature.contains("get"), "got {}", h.signature);
}

// ============ struct / 对象字面量成员 ============

#[test]
fn struct_instance_members_fields_and_methods() {
    let src = "\
struct Point {
    x: number,
    y: number,
}
impl Point {
    function new(x, y) {
        self.x = x
        self.y = y
    }
    function len() -> number {
        return sqrt(self.x * self.x + self.y * self.y)
    }
}
p = new Point(3, 4)
p.§
";
    let items = complete_at(src);
    let x = find(&items, "x").expect("缺字段 x");
    assert_eq!(x.kind, ItemKind::Field);
    assert_eq!(x.detail, "x: number");
    let len = find(&items, "len").expect("缺方法 len");
    assert_eq!(len.kind, ItemKind::Method);
    assert_eq!(len.detail, "function len() -> number");
    assert!(find(&items, "push").is_none(), "不应出现内置方法");
}

#[test]
fn self_in_impl_method_sees_struct_members() {
    let src = "\
struct Rect {
    w,
    h,
}
impl Rect {
    function area() {
        return self.§
    }
}
";
    let items = complete_at(src);
    assert!(find(&items, "w").is_some(), "self 应看到字段");
    assert!(find(&items, "area").is_some(), "self 应看到兄弟方法");
}

#[test]
fn object_literal_fields_completion() {
    let items = complete_at("counter = {\n    n: 0,\n    inc: function() {\n        self.n += 1\n        return self.n\n    },\n}\ncounter.§\n");
    let n = find(&items, "n").expect("缺字段 n");
    assert_eq!(n.kind, ItemKind::Field);
    let inc = find(&items, "inc").expect("缺方法字段 inc");
    assert_eq!(inc.kind, ItemKind::Method);
}

#[test]
fn self_in_object_literal_method_sees_fields() {
    let items = complete_at("counter = {\n    n: 0,\n    inc: function() {\n        return self.§n\n    },\n}\n");
    assert!(find(&items, "n").is_some(), "对象方法内 self 应看到字段");
}

// ============ 容错 ============

#[test]
fn incomplete_document_still_completes() {
    // 半截语句、未闭合结构 —— 不 panic，给出可用补全
    let items = complete_at("function f() {\n    arr = [1, §");
    assert!(!items.is_empty());
    assert!(find(&items, "len").is_some(), "内置函数应可用");
}

#[test]
fn broken_statement_degrades_gracefully() {
    let items = complete_at("x = ?? ?\n§");
    assert!(!items.is_empty());
    assert!(find(&items, "println").is_some());
}

#[test]
fn multiline_chain_receiver() {
    // 方法链跨行：换行被词法器抑制，片段边界正确
    let items = complete_at("data = \"a,b,c\"\nout = data\n    .split(\",\")\n    .§");
    // split 返回 array → . 后给数组方法
    assert!(find(&items, "map").is_some());
    assert!(find(&items, "to_uppercase").is_none());
}

// ============ 类型推导规则 ============

#[test]
fn annotation_takes_priority() {
    let items = complete_at("x: number[] = \"whatever\"\n§");
    let x = find(&items, "x").expect("缺 x");
    assert_eq!(x.detail, "x: number[]", "标注优先于 RHS 推导");
}

#[test]
fn string_concat_propagation() {
    let items = complete_at("a = \"x\"\nb = a + 1\n§");
    assert_eq!(find(&items, "b").unwrap().detail, "b: string");
}

#[test]
fn builtin_global_function_returns() {
    let items = complete_at("n = num(\"3\")\ns = str(1)\nL = len([1])\n§");
    assert_eq!(find(&items, "n").unwrap().detail, "n: number");
    assert_eq!(find(&items, "s").unwrap().detail, "s: string");
    assert_eq!(find(&items, "L").unwrap().detail, "L: number");
}

#[test]
fn for_in_loop_variable_types() {
    let items = complete_at("for i in 0..n {\n    §\n}\n");
    let i = find(&items, "i").expect("缺循环变量");
    assert_eq!(i.detail, "i: number");

    let items = complete_at("for c in \"ab\" {\n    §\n}");
    let c = find(&items, "c").expect("缺循环变量 c");
    assert_eq!(c.detail, "c: string");
}

#[test]
fn reassignment_updates_type() {
    let items = complete_at("x = 1\nx = \"now string\"\n§");
    assert_eq!(find(&items, "x").unwrap().detail, "x: string");
}

// ============ 悬停 ============

#[test]
fn hover_variable_shows_inferred_type() {
    let h = hover_at("count = 41\ncount§").unwrap();
    assert!(h.signature.contains("count: number"), "got {}", h.signature);
}

#[test]
fn hover_user_function_signature() {
    let h = hover_at("function add(a: number, b) -> number {\n    return a + b\n}\nadd§").unwrap();
    assert!(h.signature.contains("function add(a: number, b) -> number"), "got {}", h.signature);
}

#[test]
fn hover_string_method() {
    let h = hover_at("s = \"x\"\ns.trim§()").unwrap();
    assert!(h.signature.contains("trim"), "got {}", h.signature);
    assert!(!h.doc.is_empty());
}

#[test]
fn hover_builtin_global() {
    let h = hover_at("print§ln(1)").unwrap();
    assert!(h.signature.contains("print"), "got {}", h.signature);
}

#[test]
fn hover_at_definition_site_shows_assigned_type() {
    // 光标就在 `n = 42` 的 n 上：词被哨兵替换，仍应显示将赋予的类型
    let h = hover_at("§n = 42\n").unwrap();
    assert!(h.signature.contains("n: number"), "got {}", h.signature);
}

#[test]
fn completion_with_cursor_inside_word() {
    // 光标在词中间（非成员位置）：词整体被哨兵替换，作用域照常
    let items = complete_at("count = 41\ntotal = cou§nt + 1\n");
    let c = find(&items, "count").expect("缺 count");
    assert_eq!(c.detail, "count: number");
}

#[test]
fn hover_struct_instance_member() {
    let src = "\
struct Point {
    x: number,
}
p = new Point(1, 2)
p.x§
";
    let h = hover_at(src).unwrap();
    assert!(h.signature.contains("Point.x"), "got {}", h.signature);
    assert!(h.signature.contains("number"));
}

// ============ UTF-16 位置换算 ============

#[test]
fn utf16_position_with_cjk_before_cursor() {
    // 行内有中文（每字 1 个 UTF-16 单元；补充一个 emoji 验证双单元）
    let src = "// 注释 🎉\nx = 1\n§";
    let items = complete_at(src);
    assert!(find(&items, "x").is_some());
}

#[test]
fn utf16_position_in_cjk_line() {
    // 光标所在行本身含中文 + emoji（双 UTF-16 单元），列换算必须按码元
    let src = "s = \"你好🎉\"\nt = 2\n你好测试§";
    let items = complete_at(src);
    assert!(find(&items, "s").is_some(), "UTF-16 列换算出错导致解析失败");
    assert!(find(&items, "t").is_some());
}

// ============ 冒烟：标签基本有序可用 ============

#[test]
fn global_completion_contains_keywords_and_builtins() {
    let items = complete_at("§");
    let ls = labels(&items);
    for kw in ["function", "struct", "impl", "interface", "is", "for", "while"] {
        assert!(ls.contains(&kw.to_string()), "缺关键字 {kw}");
    }
    for g in ["println", "len", "num", "input"] {
        assert!(ls.contains(&g.to_string()), "缺全局函数 {g}");
    }
    for c in ["EMPTY", "inf", "nan", "fs", "sys"] {
        assert!(ls.contains(&c.to_string()), "缺常量 {c}");
    }
    for cls in ["Map", "MaxHeap", "MinHeap", "Stack", "Queue"] {
        assert!(ls.contains(&cls.to_string()), "缺内置类 {cls}");
    }
}

// ============ 类型位置补全 ============

#[test]
fn type_completion_after_colon_and_arrow() {
    // 变量标注位置
    let items = complete_at("x: §");
    assert!(find(&items, "number").is_some(), "缺 number");
    assert!(find(&items, "string").is_some(), "缺 string");
    assert!(find(&items, "Map").is_some(), "缺 Map");
    assert!(find(&items, "println").is_none(), "类型位置不应有全局函数");
    assert!(find(&items, "if").is_none(), "类型位置不应有关键字");

    // 返回类型位置（语句残缺也能给类型表）
    let items = complete_at("function f(a) -> §");
    assert!(find(&items, "array").is_some(), "返回类型缺 array");

    // 参数标注位置
    let items = complete_at("function f(a: §");
    assert!(find(&items, "number").is_some(), "参数标注缺 number");

    // struct 字段标注位置
    let items = complete_at("struct S {\n    x: §\n}\n");
    assert!(find(&items, "string").is_some(), "字段标注缺 string");
}

#[test]
fn type_completion_includes_user_structs() {
    let items = complete_at("struct Point { x, y }\ninterface Shape { function area() }\nv: §");
    let p = find(&items, "Point").expect("缺用户 struct");
    assert_eq!(p.kind, ItemKind::Struct);
    let s = find(&items, "Shape").expect("缺接口");
    assert_eq!(s.kind, ItemKind::Interface);
    // 已输入前缀场景：`v: P§` 也命中
    let items = complete_at("struct Point { x, y }\nv: P§");
    assert!(find(&items, "Point").is_some(), "带前缀 P 应命中 Point");
}

#[test]
fn value_positions_not_hijacked_by_type_completion() {
    // 对象字面量值位置：仍是全局补全
    let items = complete_at("n = 1\no = { a: §");
    assert!(find(&items, "n").is_some(), "对象值位置应能看到变量 n");
    assert!(find(&items, "number").is_none(), "对象值位置不该给类型");

    // 三元假分支：仍是全局补全
    let items = complete_at("n = 1\nc = true ? x : §");
    assert!(find(&items, "n").is_some(), "三元分支应能看到变量 n");
    assert!(find(&items, "number").is_none(), "三元分支不该给类型");

    // 嵌套三元
    let items = complete_at("n = 1\nc = a ? b : d ? e : §");
    assert!(find(&items, "n").is_some(), "嵌套三元应能看到变量 n");
}
