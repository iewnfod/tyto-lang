//! checker::ty::Type ↔ analysis::infer::Ty 的双向换算（两套类型系统的唯一桥接点）。
//!
//! 背景：analysis 的 [`Ty`] 是编辑器视角的简化类型（无联合/泛型实参，Unknown
//! 兜底），checker 的 [`Type`] 是完整语义类型。内置表已单源化到 checker
//! （名字 + 签名 + 返回类型），编辑器侧的推导链经本桥取数；
//! 换不动的（Union / Object 形状 / TypeVar）按「宁可少知不可错知」回退 Unknown，
//! 展示层始终用表内签名串，不受此损益影响。

use std::rc::Rc;

use super::infer::Ty;
use crate::checker::ty::{FuncShape, Type};

/// checker 类型 → 编辑器类型（精确度有损：联合/对象形状/泛型实参 → Unknown）
pub fn checker_to_ty(t: &Type) -> Ty {
    match t {
        Type::Number => Ty::Number,
        Type::Str => Ty::Str,
        Type::Bool => Ty::Bool,
        Type::Null => Ty::Null,
        Type::Empty => Ty::Empty,
        Type::Array(_) => Ty::Array,
        Type::Map(..) => Ty::Map,
        Type::MaxHeap => Ty::MaxHeap,
        Type::MinHeap => Ty::MinHeap,
        Type::Stack => Ty::Stack,
        Type::Queue => Ty::Queue,
        // 语言只有 fs/sys 两个命名空间（checker 表同源），静态串直接映射
        Type::Namespace(ns) => Ty::Namespace(*ns),
        Type::StructDef(s) => Ty::StructDef(s.clone()),
        Type::InterfaceDef(s) => Ty::InterfaceDef(s.clone()),
        Type::Struct(s, _) => Ty::Struct(s.clone()),
        Type::Func(f) => Ty::Func(f.ret.as_ref().map(|r| Box::new(checker_to_ty(r)))),
        // Union / Object 形状 / TypeVar：编辑器 Ty 无对应表示
        Type::Union(_) | Type::Object(_) | Type::TypeVar(_) | Type::Any => Ty::Unknown,
    }
}

/// 编辑器类型 → checker 类型（内置容器补 Any 形参；仅用于内置表分派与查询）
pub fn ty_to_checker(t: &Ty) -> Type {
    match t {
        Ty::Number => Type::Number,
        Ty::Str => Type::Str,
        Ty::Bool => Type::Bool,
        Ty::Null => Type::Null,
        Ty::Empty => Type::Empty,
        Ty::Array => Type::Array(Box::new(Type::Any)),
        Ty::Map => Type::Map(Box::new(Type::Any), Box::new(Type::Any)),
        Ty::MaxHeap => Type::MaxHeap,
        Ty::MinHeap => Type::MinHeap,
        Ty::Stack => Type::Stack,
        Ty::Queue => Type::Queue,
        Ty::Namespace(ns) => Type::Namespace(*ns),
        Ty::Struct(s) => Type::Struct(s.clone(), Vec::new()),
        Ty::StructDef(s) => Type::StructDef(s.clone()),
        Ty::InterfaceDef(s) => Type::InterfaceDef(s.clone()),
        Ty::Func(ret) => Type::Func(Rc::new(FuncShape {
            params: Vec::new(),
            ret: ret.as_deref().map(ty_to_checker),
        })),
        Ty::Object(_) | Ty::Unknown => Type::Any,
    }
}
