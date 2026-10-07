use std::{cell::RefCell, collections::HashMap, rc::Rc};

use crate::{RtError, RtResult, Value};

pub type ScopeRef = Rc<RefCell<Scope>>;

/// 词法作用域链。
///
/// 读：沿链向上查找，找不到报错。
/// 赋值：先沿链向上找**已存在的绑定**（函数内 `h_low = ...` 命中全局的 `h_low`），
/// 找不到则在**当前作用域新建**（`val = ...` 成为局部变量）——Lua 式规则。
#[derive(Debug)]
pub struct Scope {
    vars: HashMap<String, Value>,
    parent: Option<ScopeRef>,
}

impl Scope {
    pub fn root() -> ScopeRef {
        Rc::new(RefCell::new(Scope { vars: HashMap::new(), parent: None }))
    }

    pub fn child(parent: ScopeRef) -> ScopeRef {
        Rc::new(RefCell::new(Scope { vars: HashMap::new(), parent: Some(parent) }))
    }
}

/// 在当前作用域定义/覆盖绑定
pub fn define(scope: &ScopeRef, name: impl Into<String>, value: Value) {
    scope.borrow_mut().vars.insert(name.into(), value);
}

/// 读变量：沿链向上
pub fn get(scope: &ScopeRef, name: &str) -> RtResult<Value> {
    let (found, parent) = {
        let s = scope.borrow();
        (s.vars.get(name).cloned(), s.parent.clone())
    };
    if let Some(v) = found {
        return Ok(v);
    }
    match parent {
        Some(p) => get(&p, name),
        None => Err(RtError::runtime(None, format!("undefined variable `{}`", name))),
    }
}

/// 赋值：向上找已存在绑定则写入，否则在**发起赋值的作用域**新建
pub fn assign(scope: &ScopeRef, name: &str, value: Value) {
    let mut cur = scope.clone();
    loop {
        if cur.borrow().vars.contains_key(name) {
            cur.borrow_mut().vars.insert(name.to_string(), value);
            return;
        }
        let parent = cur.borrow().parent.clone();
        match parent {
            Some(p) => cur = p,
            None => {
                define(scope, name, value);
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assign_penetrates_to_outer_binding() {
        let root = Scope::root();
        define(&root, "h", Value::Null);
        let child = Scope::child(root.clone());
        let grandchild = Scope::child(child.clone());

        assign(&grandchild, "h", Value::Num(42.0));
        assert_eq!(get(&root, "h").unwrap(), Value::Num(42.0));

        // 未在任何层定义 → 在最内层新建
        assign(&grandchild, "tmp", Value::Bool(true));
        assert_eq!(get(&grandchild, "tmp").unwrap(), Value::Bool(true));
        assert!(get(&child, "tmp").is_err());
        assert!(get(&root, "tmp").is_err());
    }

    #[test]
    fn get_reports_undefined() {
        let root = Scope::root();
        let err = get(&root, "nope").unwrap_err();
        assert!(matches!(err, RtError::Runtime { .. }));
    }
}
