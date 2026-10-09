//! 光标处可见绑定的收集：AST 遍历 + 作用域链。
//!
//! 核心思路：在光标处插入哨兵标识符后解析（见 [`super::tolerate`]），
//! 然后沿 AST **走到哨兵的路径**收集作用域——语句按「span ≤ 哨兵」顺序登记，
//! 遇到容器语句（函数体 / if / while / for / impl / 含函数字面量的表达式）
//! 则下钻一层。链上赋值语义与运行时一致：命中已有绑定就更新那一层的类型。
//!
//! self 不做特判：impl 方法与对象字面量方法在进入时把 `self` 作为
//! 普通绑定插入作用域，推导沿链自然查到。
//!
//! 类型推导已单源到 checker 引擎：Walker 持引擎镜像链（[`crate::checker::Checker`]
//! 光标模式）成对登记，绑定类型与表达式类型全部由引擎给出。
//!
//! 函数体内编辑时的全局启发式：语言惯用形是「顶层 `h = null` + init() 内
//! `h = new MaxHeap()`」——运行期函数在顶层跑完后才被调用，故光标在函数体内
//! 时，全局层改用「挖掉光标语句后的完整文件」（ambient）的顶层绑定。
//!
//! 结构：数据类型与入口在本模块；走查器在 [`walker`]；叶子登记
//! [`register_leaf`] 由着色走查（semantics）共用。

mod walker;

use std::collections::HashMap;
use std::rc::Rc;

use crate::ast::{AssignOp, Expr, Stmt};
use crate::checker::ty::{FuncShape, Type};
use crate::checker::Checker;
use crate::Span;

use super::ty_view::{assign_result_ty, editor_display, func_sig};
use super::ItemKind;
use self::walker::{register_globals_ambient, Walker};

// struct/interface 注册表已迁入核心 checker（事实来源单一化），这里转发
pub use crate::checker::registry::{
    collect_registry, FuncInfo, StructInfo, StructRegistry,
};

pub const SENTINEL: &str = "__tyto_cx__";

/// 一个可见绑定（变量 / 函数 / struct / interface）
#[derive(Debug, Clone)]
pub struct Binding {
    pub name: String,
    pub ty: Type,
    pub kind: ItemKind,
    /// 展示签名：变量 `x: T`、函数完整签名、struct 字段列表
    pub detail: String,
    pub doc: String,
    /// 声明处名字的位置（跳转定义用；未知为 default）
    pub span: Span,
}

/// 走到哨兵后的作用域快照
pub struct ScopeSnapshot {
    /// 外→内的作用域链（最内层可能为空——光标所在块还没有语句）
    pub scopes: Vec<HashMap<String, Binding>>,
    /// 光标是否在（至少一层）函数体内
    pub inside_func: bool,
    /// 成员补全/悬停：哨兵 Member 接收者的类型（快照期由引擎预推导）
    pub receiver_ty: Option<Type>,
    /// 光标引擎实例（镜像作用域链随快照冻结；impl 方法返回推导等按需查询）
    pub(crate) ck: Checker,
    pub structs: StructRegistry,
}

/// 推导上下文：作用域链（外→内）+ struct 注册表
pub struct Ctx<'a> {
    pub scopes: &'a [HashMap<String, Binding>],
    pub structs: &'a StructRegistry,
}

impl<'a> Ctx<'a> {
    /// 沿链（内→外）查绑定
    pub fn lookup(&self, name: &str) -> Option<&Binding> {
        self.scopes.iter().rev().find_map(|s| s.get(name))
    }
}

/// 走到哨兵（`s`），收集作用域链与接收者。
///
/// `program`：插好哨兵并修复闭括号后解析出的 AST；
/// `ambient`：挖掉光标语句后的完整文件 AST（函数体内全局启发式用），可为 None。
///
/// 接收者类型在快照期末尾由引擎一次性推出，消费方免查。
pub fn collect_at_cursor(
    program: &Stmt,
    s: Span,
    ambient: Option<&Stmt>,
    structs: &StructRegistry,
) -> ScopeSnapshot {
    let mut walker = Walker {
        structs,
        inside_func: false,
        receiver: None,
        s,
        ck: Checker::for_cursor(structs.clone()),
    };
    let mut scopes: Vec<HashMap<String, Binding>> = vec![HashMap::new()];
    walker.walk_stmts(top_stmts(program), &mut scopes);

    // 函数体内编辑：全局层换成 ambient（整个文件减去光标语句）的顶层绑定。
    // 引擎镜像链同步换——ambient 用独立引擎跑，避免污染主镜像
    if let Some(amb) = ambient.filter(|_| walker.inside_func) {
        let mut whole: Vec<HashMap<String, Binding>> = vec![HashMap::new()];
        let mut amb_ck = Checker::for_cursor(structs.clone());
        register_globals_ambient(top_stmts(amb), &mut whole, &mut amb_ck);
        if let Some(g) = whole.first() {
            scopes[0] = g.clone();
        }
        if let Some(g) = amb_ck.scopes.first() {
            walker.ck.scopes[0] = g.clone();
        }
    }

    // 接收者类型快照期预推导（引擎唯一推导，消费方免查）
    let receiver_ty = walker.receiver.as_ref().map(|e| walker.ck.infer(e));

    ScopeSnapshot {
        scopes,
        inside_func: walker.inside_func,
        receiver_ty,
        ck: walker.ck,
        structs: structs.clone(),
    }
}

fn top_stmts(program: &Stmt) -> &[Stmt] {
    match program {
        Stmt::Block { stmts, .. } => stmts,
        _ => &[],
    }
}

/// 链上语义：命中已有绑定 → 更新那一层；否则插入最内层
pub(crate) fn chain_insert(scopes: &mut [HashMap<String, Binding>], name: &str, binding: Binding) {
    for scope in scopes.iter_mut().rev() {
        if scope.contains_key(name) {
            scope.insert(name.to_string(), binding);
            return;
        }
    }
    scopes.last_mut().unwrap().insert(name.to_string(), binding);
}

fn chain_find<'b>(scopes: &'b [HashMap<String, Binding>], name: &str) -> Option<&'b Binding> {
    scopes.iter().rev().find_map(|s| s.get(name))
}

/// 叶子语句（赋值 / 函数声明 / struct / interface）的绑定登记 + 声明位置记录。
/// 光标走查（scope::Walker）与全文档着色走查（semantics::Highlighter）共用。
/// 类型推导经引擎（`ck`），编辑器绑定与引擎镜像成对登记。
pub(crate) fn register_leaf(
    ck: &mut Checker,
    stmt: &Stmt,
    scopes: &mut Vec<HashMap<String, Binding>>,
) {
    match stmt {
        Stmt::Assign { target, op, value, ann, decl, span } => {
            let Expr::Ident(name, tspan) = target else {
                // 对象字段 / 索引赋值不引入新绑定
                return;
            };
            let rhs = ck.infer(value);
            let new_ty = if let Some(a) = ann {
                ck.ann_ty(a)
            } else if *op != AssignOp::Set {
                match chain_find(scopes, name) {
                    Some(old) => assign_result_ty(*op, &old.ty, &rhs),
                    None => rhs,
                }
            } else {
                rhs
            };
            let _ = decl; // let/const 对编辑器绑定无差别（运行时才区分）
            // 跳转定义指向**首个**声明处：重赋值保留旧 span
            let dspan = chain_find(scopes, name)
                .map(|old| old.span)
                .filter(|s| *s != Span::default())
                .unwrap_or(*tspan);
            let b = Binding {
                name: name.clone(),
                ty: new_ty.clone(),
                kind: ItemKind::Variable,
                detail: format!("{}: {}", name, editor_display(&new_ty)),
                doc: String::new(),
                // 赋值目标的名字位置：AST 精确
                span: dspan,
            };
            let _ = span;
            chain_insert(scopes, name, b);
            ck.mirror_define(name, new_ty, None);
        }
        Stmt::FuncDecl { name, params, type_params, ret, body, span } => {
            // 返回类型：标注优先，否则引擎从 body 的 return 推导
            let ret_ann = ret.as_ref().map(|a| ck.ann_ty(a));
            let ret_ty = ret_ann.or_else(|| ck.infer_func_ret(params, body));
            chain_insert(
                scopes,
                name,
                Binding {
                    name: name.clone(),
                    // 展示形状不带参数（完整签名由 func_sig 给出）
                    ty: Type::Func(Rc::new(FuncShape { params: Vec::new(), ret: ret_ty.clone() })),
                    kind: ItemKind::Function,
                    detail: func_sig(name, params, ret.as_ref()),
                    doc: String::new(),
                    // `function name`：名字在关键字 + 空格后（len("function ") == 9）
                    span: Span::new(span.line, span.col + 9),
                },
            );
            // 引擎镜像带完整参数形状与原始签名（调用点推导/泛型求解用）
            ck.mirror_define_func(name, params, type_params, ret, ret_ty);
        }
        Stmt::Struct { name, fields, span, .. } => {
            chain_insert(
                scopes,
                name,
                Binding {
                    name: name.clone(),
                    ty: Type::StructDef(name.clone()),
                    kind: ItemKind::Struct,
                    detail: format!(
                        "struct {} {{ {} }}",
                        name,
                        fields.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", ")
                    ),
                    doc: String::new(),
                    span: Span::new(span.line, span.col + 7),
                },
            );
            ck.mirror_define(name, Type::StructDef(name.clone()), None);
        }
        Stmt::Interface { name, methods, span } => {
            chain_insert(
                scopes,
                name,
                Binding {
                    name: name.clone(),
                    ty: Type::InterfaceDef(name.clone()),
                    kind: ItemKind::Interface,
                    detail: format!(
                        "interface {} {{ {} }}",
                        name,
                        methods.iter().map(|m| m.name.as_str()).collect::<Vec<_>>().join(", ")
                    ),
                    doc: String::new(),
                    span: Span::new(span.line, span.col + 10),
                },
            );
            ck.mirror_define(name, Type::InterfaceDef(name.clone()), None);
        }
        _ => {}
    }
}
