//! struct / interface 注册表：全文件收集类型声明（对编辑中的代码宽容）。
//!
//! 从 analysis::scope 迁入（核心模块化：checker 是事实来源，analysis 引用之）。
//! FuncInfo 保留 TypeAst 形式的标注（检查期由 checker 按需转换）。

use std::collections::HashMap;
use std::rc::Rc;

use crate::ast::{Param, Stmt, TypeAst};
use crate::Span;

/// struct / interface 注册表项
#[derive(Debug, Clone)]
pub struct StructInfo {
    pub name: String,
    pub interface: bool,
    /// struct 的泛型参数（`struct Box<T>` → ["T"]）
    pub type_params: Vec<String>,
    pub fields: Vec<Param>,
    pub methods: Vec<FuncInfo>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct FuncInfo {
    pub name: String,
    pub params: Vec<Param>,
    pub type_params: Vec<String>,
    pub ret: Option<TypeAst>,
    pub body: Rc<Stmt>,
}

impl StructInfo {
    pub fn detail(&self) -> String {
        if self.interface {
            format!(
                "interface {} {{ {} }}",
                self.name,
                self.methods.iter().map(|m| m.name.as_str()).collect::<Vec<_>>().join(", ")
            )
        } else {
            format!(
                "struct {} {{ {} }}",
                self.name,
                self.fields.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", ")
            )
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct StructRegistry {
    map: HashMap<String, StructInfo>,
}

impl StructRegistry {
    pub fn get(&self, name: &str) -> Option<&StructInfo> {
        self.map.get(name)
    }
    pub fn contains(&self, name: &str) -> bool {
        self.map.contains_key(name)
    }
    pub fn iter(&self) -> impl Iterator<Item = &StructInfo> {
        self.map.values()
    }
    fn entry(&mut self, info: StructInfo) {
        self.map.insert(info.name.clone(), info);
    }
}

/// 全文件收集 struct / impl / interface（不看光标位置，声明序优先）
pub fn collect_registry(program: &Stmt) -> StructRegistry {
    let mut reg = StructRegistry::default();
    walk_types(program, &mut reg);
    reg
}

fn walk_types(stmt: &Stmt, reg: &mut StructRegistry) {
    match stmt {
        Stmt::Struct { name, type_params, fields, .. } => {
            reg.entry(StructInfo {
                name: name.clone(),
                interface: false,
                type_params: type_params.clone(),
                fields: fields.clone(),
                methods: Vec::new(),
                span: stmt.span(),
            });
        }
        Stmt::Interface { name, methods, .. } => {
            reg.entry(StructInfo {
                name: name.clone(),
                interface: true,
                type_params: Vec::new(),
                fields: Vec::new(),
                methods: methods
                    .iter()
                    .map(|m| FuncInfo {
                        name: m.name.clone(),
                        params: m.params.clone(),
                        type_params: Vec::new(),
                        ret: m.ret.clone(),
                        body: Rc::new(Stmt::Block { stmts: Vec::new(), span: Span::default() }),
                    })
                    .collect(),
                span: stmt.span(),
            });
        }
        Stmt::Impl { target, type_params, methods, .. } => {
            // struct 可能尚未声明（编辑中）：先造空壳，后续声明补字段。
            // 空壳的泛型参数先用 impl 声明兜底（struct 声明出现后覆盖）。
            let entry = reg.map.entry(target.clone()).or_insert_with(|| StructInfo {
                name: target.clone(),
                interface: false,
                type_params: type_params.clone(),
                fields: Vec::new(),
                methods: Vec::new(),
                span: stmt.span(),
            });
            for m in methods {
                if let Stmt::FuncDecl { name, params, type_params, ret, body, .. } = m {
                    // 同名方法覆盖（与运行时一致）
                    entry.methods.retain(|old| old.name != *name);
                    entry.methods.push(FuncInfo {
                        name: name.clone(),
                        params: params.clone(),
                        type_params: type_params.clone(),
                        ret: ret.clone(),
                        body: body.clone(),
                    });
                }
            }
        }
        Stmt::Block { stmts, .. } => stmts.iter().for_each(|s| walk_types(s, reg)),
        Stmt::If { then_block, else_block, .. } => {
            walk_types(then_block, reg);
            if let Some(e) = else_block {
                walk_types(e, reg);
            }
        }
        Stmt::While { body, .. } | Stmt::ForIn { body, .. } | Stmt::ForC { body, .. } => {
            walk_types(body, reg)
        }
        _ => {}
    }
}
