//! 内置表一致性守护：analysis（编辑器展示）与 checker（事实来源）之间的
//! 名字集合对齐。数据本体已单源化到 `checker::builtins`（方法/全局/命名空间
//! 的名字、签名、返回类型），本测试守护剩余的展示数据不漂移：
//! - CLASSES（analysis 展示表）的名字必须与 checker 的内置类知识一致；
//! - CONSTANTS 中的值常量必须有 checker 类型、命名空间常量必须有函数表；
//! - checker 全局函数必须能从 analysis 侧查到完整签名与文档。

use tyto_lang::analysis::builtins;
use tyto_lang::checker;
use tyto_lang::checker::ty::Type;

fn probe_tys() -> Vec<Type> {
    vec![
        Type::Array(Box::new(Type::Any)),
        Type::Str,
        Type::Map(Box::new(Type::Any), Box::new(Type::Any)),
        Type::MaxHeap,
        Type::MinHeap,
        Type::Stack,
        Type::Queue,
    ]
}

/// CLASSES 展示表 ↔ checker 内置类知识：名字集合一致
#[test]
fn classes_match_checker() {
    for (name, sig, doc) in builtins::CLASSES {
        assert!(checker::builtins::is_builtin_class(name), "CLASSES 中的 {name} 不是 checker 认可的内置类");
        assert!(checker::builtins::class_instance(name).is_some());
        assert!(!sig.is_empty() && !doc.is_empty());
    }
    // 反向：checker 认可的类都在展示表里（当前五个）
    for name in ["Map", "MaxHeap", "MinHeap", "Stack", "Queue"] {
        assert!(
            builtins::CLASSES.iter().any(|(n, _, _)| *n == name),
            "内置类 {name} 缺少编辑器展示条目"
        );
    }
}

/// CONSTANTS：值常量有 checker 类型；命名空间常量有函数表
#[test]
fn constants_match_checker() {
    for (name, _, _) in builtins::CONSTANTS {
        match *name {
            "EMPTY" | "inf" | "nan" => {
                assert!(checker::builtins::constant_ty(name).is_some());
            }
            "fs" | "sys" => {
                assert!(builtins::namespace_fns(name).is_some());
                assert!(!builtins::namespace_fns(name).unwrap().is_empty());
            }
            other => panic!("CONSTANTS 出现未知常量 {other}"),
        }
    }
}

/// checker 全局函数经 analysis 查询层完整可查（名字/签名/文档/类型）
#[test]
fn globals_queryable_from_analysis() {
    for f in builtins::all_globals() {
        assert!(!f.sig.is_empty(), "{} 缺签名", f.name);
        assert!(!f.doc.is_empty(), "{} 缺文档", f.name);
        let q = builtins::global_fn(f.name).expect(f.name);
        assert_eq!(q.sig, f.sig);
    }
}

/// 方法表：每种内置接收者都能取到非空方法表，条目三件套齐全
#[test]
fn method_tables_complete() {
    for ty in probe_tys() {
        let table = builtins::methods_for(&ty)
            .unwrap_or_else(|| panic!("{ty:?} 没有方法表"));
        assert!(!table.is_empty());
        for m in table {
            assert!(!m.name.is_empty());
            assert!(m.sig.starts_with('.'));
            assert!(!m.doc.is_empty());
        }
    }
    assert!(!builtins::all_methods().is_empty());
}

/// 方法表直查：每种内置接收者的 len 都返回 number（类型表示已单源，无桥接层）
#[test]
fn builtin_method_returns_keep_kinds() {
    for ty in probe_tys() {
        let back = builtins::method_return(&ty, "len");
        assert_eq!(back, Some(Type::Number), "{ty:?}.len 应为 number");
    }
}
