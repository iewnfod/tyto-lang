use std::{cell::RefCell, cmp::Ordering, collections::{BinaryHeap, VecDeque}, rc::Rc};

use indexmap::IndexMap;

use crate::{ast::Stmt, scope::ScopeRef};

/// 包装 f64 以便放进 BinaryHeap（f64 未实现 Ord，用 total_cmp）
#[derive(Clone, Copy, Debug)]
pub struct HeapVal(pub f64);

impl PartialEq for HeapVal {
    fn eq(&self, other: &Self) -> bool {
        self.0.total_cmp(&other.0) == Ordering::Equal
    }
}
impl Eq for HeapVal {}

impl PartialOrd for HeapVal {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for HeapVal {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.total_cmp(&other.0)
    }
}

/// Map 的合法键：number/string/bool/null。
/// 数字键做规范化：整数值统一为 Int，使得 1 与 1.0 哈希一致；-0.0 归为 0。
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum MapKey {
    Int(i64),
    Float(u64),
    Str(String),
    Bool(bool),
    Null,
}

impl MapKey {
    pub fn from_value(v: &Value) -> Option<MapKey> {
        match v {
            Value::Num(n) => {
                if *n == 0.0 {
                    Some(MapKey::Int(0))
                } else if n.fract() == 0.0 && n.abs() <= 9_007_199_254_740_992.0 {
                    Some(MapKey::Int(*n as i64))
                } else {
                    Some(MapKey::Float(n.to_bits()))
                }
            }
            Value::Str(s) => Some(MapKey::Str(s.clone())),
            Value::Bool(b) => Some(MapKey::Bool(*b)),
            Value::Null => Some(MapKey::Null),
            _ => None,
        }
    }

    pub fn to_display(&self) -> String {
        match self {
            MapKey::Int(i) => i.to_string(),
            MapKey::Float(bits) => fmt_num(f64::from_bits(*bits)),
            MapKey::Str(s) => format!("{:?}", s),
            MapKey::Bool(b) => b.to_string(),
            MapKey::Null => "null".into(),
        }
    }

    /// 键还原为值（keys() 用）
    pub fn to_value(&self) -> Value {
        match self {
            MapKey::Int(i) => Value::Num(*i as f64),
            MapKey::Float(bits) => Value::Num(f64::from_bits(*bits)),
            MapKey::Str(s) => Value::Str(s.clone()),
            MapKey::Bool(b) => Value::Bool(*b),
            MapKey::Null => Value::Null,
        }
    }
}

/// `{x: 1}` 匿名对象：字符串字段，保序
#[derive(Clone, Debug, Default)]
pub struct ObjObj {
    pub fields: IndexMap<String, Value>,
}

/// `new Map()`：任意原始类型键，保序
#[derive(Clone, Debug, Default)]
pub struct MapObj {
    pub entries: IndexMap<MapKey, Value>,
}

/// 用户函数：参数、函数体与其定义时的闭包环境
#[derive(Clone, Debug)]
pub struct FuncObj {
    pub name: String,
    pub params: Vec<String>,
    pub body: Rc<Stmt>,
    pub closure: ScopeRef,
}

/// 原生类（`new MaxHeap()` 中 `MaxHeap` 求值的结果）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeClass {
    Map,
    MaxHeap,
    MinHeap,
    Stack,
    Queue,
}

impl NativeClass {
    pub fn name(&self) -> &'static str {
        match self {
            NativeClass::Map => "Map",
            NativeClass::MaxHeap => "MaxHeap",
            NativeClass::MinHeap => "MinHeap",
            NativeClass::Stack => "Stack",
            NativeClass::Queue => "Queue",
        }
    }
}

/// JS 式数字显示：整数值不带小数点
pub fn fmt_num(n: f64) -> String {
    if n.is_nan() {
        "nan".into()
    } else if n.is_infinite() {
        if n > 0.0 { "inf".into() } else { "-inf".into() }
    } else if n == n.trunc() && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        format!("{}", n)
    }
}

#[derive(Clone, Debug)]
pub enum Value {
    Num(f64),
    Str(String),
    Bool(bool),
    Null,
    Empty,
    Array(Rc<RefCell<Vec<Value>>>),
    Obj(Rc<RefCell<ObjObj>>),
    Map(Rc<RefCell<MapObj>>),
    MaxHeap(Rc<RefCell<BinaryHeap<HeapVal>>>),
    MinHeap(Rc<RefCell<BinaryHeap<std::cmp::Reverse<HeapVal>>>>),
    Stack(Rc<RefCell<Vec<Value>>>),
    Queue(Rc<RefCell<VecDeque<Value>>>),
    NativeClass(NativeClass),
    NativeFn(&'static str),
    Func(Rc<FuncObj>),
}

impl Value {
    /// print 输出的形式：顶层字符串裸输出
    pub fn to_display(&self) -> String {
        match self {
            Value::Str(s) => s.clone(),
            _ => self.to_repr(),
        }
    }

    /// 调试/容器内嵌的形式：字符串带引号（Node 控制台风格）
    pub fn to_repr(&self) -> String {
        match self {
            Value::Num(n) => fmt_num(*n),
            Value::Str(s) => format!("{:?}", s),
            Value::Bool(b) => b.to_string(),
            Value::Null => "null".into(),
            Value::Empty => "EMPTY".into(),
            Value::Array(a) => {
                let items: Vec<String> = a.borrow().iter().map(|v| v.to_repr()).collect();
                format!("[{}]", items.join(", "))
            }
            Value::Obj(o) => {
                let fields = &o.borrow().fields;
                if fields.is_empty() {
                    return "{}".into();
                }
                let items: Vec<String> =
                    fields.iter().map(|(k, v)| format!("{}: {}", k, v.to_repr())).collect();
                format!("{{{}}}", items.join(", "))
            }
            Value::Map(m) => {
                let entries = &m.borrow().entries;
                if entries.is_empty() {
                    return "Map {}".into();
                }
                let items: Vec<String> = entries
                    .iter()
                    .map(|(k, v)| format!("{}: {}", k.to_display(), v.to_repr()))
                    .collect();
                format!("Map {{{}}}", items.join(", "))
            }
            Value::MaxHeap(h) => {
                // 堆内顺序不保证，打印时按有序输出便于调试
                let mut v: Vec<f64> = h.borrow().iter().map(|x| x.0).collect();
                v.sort_by(|a, b| b.total_cmp(a));
                let items: Vec<String> = v.into_iter().map(fmt_num).collect();
                format!("MaxHeap[{}]", items.join(", "))
            }
            Value::MinHeap(h) => {
                let mut v: Vec<f64> = h.borrow().iter().map(|x| x.0 .0).collect();
                v.sort_by(|a, b| a.total_cmp(b));
                let items: Vec<String> = v.into_iter().map(fmt_num).collect();
                format!("MinHeap[{}]", items.join(", "))
            }
            Value::Stack(s) => {
                // 从底到顶，顶在右
                let items: Vec<String> = s.borrow().iter().map(|v| v.to_repr()).collect();
                format!("Stack[{}]", items.join(", "))
            }
            Value::Queue(q) => {
                // 从队头到队尾
                let items: Vec<String> = q.borrow().iter().map(|v| v.to_repr()).collect();
                format!("Queue[{}]", items.join(", "))
            }
            Value::NativeClass(c) => format!("<class {}>", c.name()),
            Value::NativeFn(name) => format!("<native fn {}>", name),
            Value::Func(f) => {
                if f.name.is_empty() {
                    "<function>".into()
                } else {
                    format!("<function {}>", f.name)
                }
            }
        }
    }

    /// JS 式真值：null/false/0/nan/""/EMPTY 为假，其余为真（含 [] 与 {}）
    pub fn truthy(&self) -> bool {
        match self {
            Value::Null | Value::Empty => false,
            Value::Bool(b) => *b,
            Value::Num(n) => *n != 0.0 && !n.is_nan(),
            Value::Str(s) => !s.is_empty(),
            _ => true,
        }
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Num(_) => "number",
            Value::Str(_) => "string",
            Value::Bool(_) => "boolean",
            Value::Null => "null",
            Value::Empty => "empty",
            Value::Array(_) => "array",
            Value::Obj(_) => "object",
            Value::Map(_) => "map",
            Value::MaxHeap(_) => "maxheap",
            Value::MinHeap(_) => "minheap",
            Value::Stack(_) => "stack",
            Value::Queue(_) => "queue",
            Value::NativeClass(_) => "class",
            Value::NativeFn(_) => "native function",
            Value::Func(_) => "function",
        }
    }
}

/// `==` 语义：原始值按值比较，容器按引用比较，跨类型为 false（无隐式转换）
pub fn eq_value(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Num(x), Value::Num(y)) => x == y,
        (Value::Str(x), Value::Str(y)) => x == y,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Null, Value::Null) => true,
        (Value::Empty, Value::Empty) => true,
        (Value::Array(x), Value::Array(y)) => Rc::ptr_eq(x, y),
        (Value::Obj(x), Value::Obj(y)) => Rc::ptr_eq(x, y),
        (Value::Map(x), Value::Map(y)) => Rc::ptr_eq(x, y),
        (Value::MaxHeap(x), Value::MaxHeap(y)) => Rc::ptr_eq(x, y),
        (Value::MinHeap(x), Value::MinHeap(y)) => Rc::ptr_eq(x, y),
        (Value::Stack(x), Value::Stack(y)) => Rc::ptr_eq(x, y),
        (Value::Queue(x), Value::Queue(y)) => Rc::ptr_eq(x, y),
        (Value::NativeClass(x), Value::NativeClass(y)) => x == y,
        (Value::NativeFn(x), Value::NativeFn(y)) => x == y,
        (Value::Func(x), Value::Func(y)) => Rc::ptr_eq(x, y),
        _ => false,
    }
}

/// 测试/断言便利：与语言内 `==` 相同的语义
impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        eq_value(self, other)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_display_is_js_style() {
        assert_eq!(fmt_num(2.0), "2");
        assert_eq!(fmt_num(-3.0), "-3");
        assert_eq!(fmt_num(2.5), "2.5");
        assert_eq!(fmt_num(-0.0), "0");
        assert_eq!(fmt_num(0.1), "0.1");
        assert_eq!(fmt_num(f64::NAN), "nan");
        assert_eq!(fmt_num(f64::INFINITY), "inf");
        assert_eq!(fmt_num(f64::NEG_INFINITY), "-inf");
        assert_eq!(fmt_num(1e14), "100000000000000");
    }

    #[test]
    fn truthiness_is_js_style() {
        assert!(!Value::Null.truthy());
        assert!(!Value::Empty.truthy());
        assert!(!Value::Bool(false).truthy());
        assert!(!Value::Num(0.0).truthy());
        assert!(!Value::Num(f64::NAN).truthy());
        assert!(!Value::Str(String::new()).truthy());
        assert!(Value::Num(0.5).truthy());
        assert!(Value::Str("0".into()).truthy());
        assert!(Value::Array(Rc::new(RefCell::new(vec![]))).truthy());
        assert!(Value::Obj(Rc::new(RefCell::new(ObjObj::default()))).truthy());
    }

    #[test]
    fn map_key_normalizes_integral_floats() {
        let one = MapKey::from_value(&Value::Num(1.0)).unwrap();
        let one_again = MapKey::from_value(&Value::Num(1.0)).unwrap();
        let frac = MapKey::from_value(&Value::Num(1.5)).unwrap();
        let neg_zero = MapKey::from_value(&Value::Num(-0.0)).unwrap();
        assert_eq!(one, MapKey::Int(1));
        assert_eq!(one, one_again);
        assert_eq!(neg_zero, MapKey::Int(0));
        assert_eq!(frac, MapKey::Float(1.5f64.to_bits()));
        // 数组不能做键
        assert!(MapKey::from_value(&Value::Array(Rc::new(RefCell::new(vec![])))).is_none());
    }

    #[test]
    fn repr_quotes_strings_inside_containers() {
        let arr = Value::Array(Rc::new(RefCell::new(vec![
            Value::Num(1.0),
            Value::Str("a".into()),
        ])));
        assert_eq!(arr.to_repr(), "[1, \"a\"]");
        assert_eq!(Value::Str("a".into()).to_display(), "a");

        let obj = Value::Obj(Rc::new(RefCell::new(ObjObj {
            fields: IndexMap::from([
                ("x".to_string(), Value::Num(1.0)),
                ("y".to_string(), Value::Bool(true)),
            ]),
        })));
        assert_eq!(obj.to_repr(), "{x: 1, y: true}");

        let empty = Value::Obj(Rc::new(RefCell::new(ObjObj::default())));
        assert_eq!(empty.to_repr(), "{}");
    }

    #[test]
    fn equality_semantics() {
        assert!(eq_value(&Value::Num(1.0), &Value::Num(1.0)));
        assert!(eq_value(&Value::Null, &Value::Null));
        assert!(!eq_value(&Value::Num(1.0), &Value::Str("1".into())));
        assert!(!eq_value(&Value::Bool(true), &Value::Num(1.0)));

        let a = Rc::new(RefCell::new(vec![Value::Num(1.0)]));
        let same = a.clone();
        let other = Rc::new(RefCell::new(vec![Value::Num(1.0)]));
        assert!(eq_value(&Value::Array(a.clone()), &Value::Array(same)));
        assert!(!eq_value(&Value::Array(a.clone()), &Value::Array(other)));
    }

    #[test]
    fn heap_display_is_sorted() {
        let h = Value::MaxHeap(Rc::new(RefCell::new(BinaryHeap::from([
            HeapVal(1.0),
            HeapVal(3.0),
            HeapVal(2.0),
        ]))));
        assert_eq!(h.to_repr(), "MaxHeap[3, 2, 1]");
    }
}
