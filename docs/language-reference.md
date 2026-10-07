# Tyto 语言参考

本文档完整描述 Tyto v0.1 的语法、语义与内置库。所有行为均与当前实现一一对应，未实现的内容不在此列。

- [词法](#词法)
- [类型与值](#类型与值)
- [运算符与表达式](#运算符与表达式)
- [语句](#语句)
- [函数与作用域](#函数与作用域)
- [对象](#对象)
- [内置类型：Array](#内置类型array)
- [内置类型：String](#内置类型string)
- [内置类型：Map](#内置类型map)
- [内置类型：MaxHeap / MinHeap](#内置类型maxheap--minheap)
- [内置类型：Stack / Queue](#内置类型stack--queue)
- [全局函数](#全局函数)
- [CLI 与 REPL](#cli-与-repl)
- [与 JS / Rust 的差异速查](#与-js--rust-的差异速查)

---

## 词法

### 注释

```
// 行注释（到行尾）
/* 块注释（不可嵌套） */
```

### 语句分隔

- **换行即语句结束**（Go 式）。也可以用 `;` 分隔，写不写随个人手感。
- 自动续行（换行被忽略）的三种情况：
  1. `(` `[` 内部换行（多行调用/数组）；
  2. 行尾是运算符、逗号、赋值号等「不能结尾」的 token（如 `a +` 换行 `b`）；
  3. **下一行以 `.` 开头**——方法链可以随便换行：

```
arr
    .map(f)
    .filter(g)
    .len()
```

### 标识符与关键字

`[a-zA-Z_][a-zA-Z0-9_]*`，大小写敏感。

关键字（共 14 个）：

```
function  return  if  else  while  for  in  break  continue
true  false  null  new
```

> ⚠️ `struct` / `impl` / `interface` / `is` / `let` 目前**不是**关键字（可作变量名），但它们是 v2 的预留方向，新代码请避免使用。

命名惯例（语言不强制）：变量/函数 `snake_case`，类型 `CamelCase`，常量 `UPPER_SNAKE`。

### 字面量

| 种类 | 写法 | 备注 |
|---|---|---|
| 数字 | `42`、`3.14`、`1e3`、`2.5e-2`、`.5` | 单一 number 类型（f64）；`1.` 无效（点后必须有数字） |
| 字符串 | `"abc"` 或 `'abc'` | 转义：`\n \t \r \\ \" \' \0`；源码中的裸换行是错误（需写 `\n`） |
| 数组 | `[1, 2, 3]` | 允许尾逗号、嵌套、异构 |
| 对象 | `{x: 1, y: 2}` | 允许尾逗号；键可为标识符或字符串 |
| 布尔/空 | `true` `false` `null` | |

区间 `a..b`（不含 b）与 `a..=b`（含 b）**只能**出现在 for-in 头部，不是独立的值。

---

## 类型与值

| 类型 | `type()` 返回 | 说明 |
|---|---|---|
| number | `"number"` | 唯一数字类型（f64），无整数/浮点之分 |
| string | `"string"` | UTF-8，不可变，按**字符**索引 |
| boolean | `"boolean"` | `true` / `false` |
| null | `"null"` | 空值 |
| empty | `"empty"` | `EMPTY` 哨兵（空堆 `pop()/peek()` 的返回） |
| array | `"array"` | 引用语义，可变，异构 |
| object | `"object"` | `{x: 1}`，字符串字段，保序 |
| map | `"map"` | `new Map()`，键限原始类型，保序 |
| maxheap / minheap | `"maxheap"` / `"minheap"` | 数字堆 |
| stack | `"stack"` | `new Stack()`，LIFO |
| queue | `"queue"` | `new Queue()`，FIFO |
| function | `"function"` | 闭包（`print` 等原生函数为 `"native function"`） |

### number 细则（JS 式）

- 整数值显示时无小数点：`println(2.0)` 输出 `2`，`println(2.5)` 输出 `2.5`
- `1 / 0` → `inf`，`-1 / 0` → `-inf`，`0 / 0` → `nan`（**不报错**）
- 全局预置 `inf`、`nan` 两个常量
- 整数运算仍是 f64：安全整数范围 ±2^53
- `%` 结果符号同被除数（`-7 % 3 == -1`）

### 真值表（JS 式）

以下为假，**其余一律为真**（包括 `[]`、`{}`、`"0"`）：

```
null   EMPTY   false   0   nan   ""
```

### `==` 与 `!=` 语义

- number/string/bool/null/EMPTY 按**值**比较（`1 == 1.0` 为 true；`nan == nan` 为 false）
- array/object/map/堆/function 按**引用**比较
- 跨类型恒为 false，**无任何隐式转换**（`1 == "1"` 为 false，`true == 1` 为 false）

---

## 运算符与表达式

### 优先级（从低到高）

| 级 | 运算符 | 备注 |
|---|---|---|
| 1 | `? :` | 三元，右结合：`a ? b : c ? d : e` |
| 2 | `\|\|` | 短路 |
| 3 | `&&` | 短路 |
| 4 | `==  !=` | |
| 5 | `<  >  <=  >=` | 仅 number×number 或 string×string |
| 6 | `+  -` | |
| 7 | `*  /  %` | |
| 8 | `!x` `-x` | 一元，可叠：`- -x` |
| 9 | `f(...)` `a[i]` `a.b` `a?.b` | 后缀，从左到右链 |

### `+` 的三重身份

```
1 + 2          // 3（数字加）
"a" + 1 + true // "a1true"（任一侧是字符串 → 拼接，值按显示格式转）
[1] + [2]      // [1, 2]（数组 + 数组 → 新数组拼接）
```

### `&&` / `||` 返回操作数（JS 式）

```
0 || "x"       // "x"
1 && 2         // 2
false && boom()// false —— 右侧不求值
```

### 赋值运算符

`=  +=  -=  *=  /=  %=`，目标可以是变量、索引（`a[0] += 1`）或对象字段（`p.n *= 2`）。`?.` 不能作为赋值目标。

### 索引

- `a[i]`：i 必须是**非负整数** number；数组越界（读/写）都报错，不支持负索引（负索引请用 `slice`/`sub`）
- `s[i]`：返回单字符字符串（按字符，不是字节），越界报错
- 字符串不可变：`s[0] = "x"` 报错
- Map 不支持 `m[k]` 读写，请用 `get`/`insert` 方法

---

## 语句

### 赋值与作用域规则（本语言的核心约定）

```
读变量：沿作用域链向上查找，找不到报错。
赋值：先沿链向上找「已存在的绑定」，找到就写入那一层；
      整条链都没有 → 在当前作用域新建。
```

这让你惯用的「顶层声明 + Init() 里赋值」伪代码模式原样可跑：

```
h_low = null

function init() {
    h_low = new MaxHeap()   // 命中全局的 h_low
    val = 1                 // 链上没有 val → 成为 Init 里的局部变量
}
```

代价：拼错的变量名会静默变成新局部变量。如果读到它报 `undefined variable`，多半是拼写问题。

块作用域：函数体、if/while/for 的体，各开一个子作用域。

### if / else if / else

```
if x > 10 {
    println("a")
} else if x > 3 {
    println("b")
} else {
    println("c")
}
```

条件不必是 bool——按真值表判断。`}` 和 `else` 可以分行。

### while

```
while !h.is_empty() {
    out.push(h.pop())
}
```

### for-in（数组 / 字符串 / 区间）

```
for x in [1, 2, 3] { ... }      // 数组，按索引迭代（迭代中 push 可见）
for c in "abc" { ... }          // 字符串，逐字符
for i in 0..n { ... }           // 0 到 n-1
for i in 1..=n { ... }          // 1 到 n（含端点）
```

- 区间端点必须是 number，步长恒为 +1（浮点端点也合法）
- Map 不能直接 for-in，用 `for k in m.keys()`
- 循环变量定义在循环自己的作用域；**整个循环共用一个绑定**（在闭包里捕获循环变量时注意，同 JS `var`）

### C 风格 for

```
for i = 0; i < n; i += 1 { ... }
for i = 0; ; i += 1 { ... }     // 条件段可空 = 恒真
for j = n; j > 0; { ... }       // 步进段可空
```

`continue` 仍会执行步进段，`break` 不会。

### break / continue / return

- `return` 可不带值（返回 null）；**顶层 `return` 是运行时错误**
- 三者都能从任意深的嵌套块中穿越出来

---

## 函数与作用域

### 声明与调用

```
function add(a, b) {
    return a + b
}
println(add(1, 2))     // 3
```

- **参数个数严格检查**：多了少了都报错
- 函数声明在**执行到时**定义（无提升），先定义后调用
- 递归天然可用（名字经闭包链查到）

### 匿名函数（一等公民）

```
double = function(x) { return x * 2 }
apply = function(f, v) { return f(v) }
println(apply(double, 21))     // 42

println([1, 2, 3].map(function(x) { return x * 2 }))   // [2, 4, 6]
```

### 闭包按引用捕获

```
function make_counter() {
    n = 0
    return function() { n += 1
return n }
}
c = make_counter()
println(c())   // 1
println(c())   // 2 —— 两次调用共享同一个 n
```

### `self` 绑定规则

只有 `obj.f(...)` 形式的调用会注入 `self`；把方法取出来单独调用则不绑定（引用 `self` 报 `undefined variable`）——比 JS 的动态 `this` 可预测。详见[对象](#对象)。

---

## 对象

`{}` 字面量凭空造对象（JS object 手感），字段：字符串键、保序、点访问的世界。

```
p = {
    x: 1,
    y: 2,
    inc: function() {
        self.n += 1
        return self.n
    },
}
```

| 操作 | 行为 |
|---|---|
| `p.x` 读 | 字段不存在 → **报错**（typo 早暴露，Rust 直觉） |
| `p.x = 5` 写 | 字段不存在 → **自动创建** |
| `p.x += 1` | 复合赋值可用 |
| `p?.x` | 仅当 `p` 为 null 时返回 null，否则同 `p.x`（注意：`EMPTY` 不会触发短路） |
| `p.f(args)` | 字段是函数 → 调用并注入 `self = p`；字段不是函数 → "not callable" |
| `len(p)` | 字段数 |
| `has(p, "x")` | 是否有该字段 |
| `p == q` | 引用比较 |
| `println(p)` | `{x: 1, y: 2}` |

对象没有任何内置方法（`len` 是全局函数）。Map 和对象是两个世界：Map 用 `get`/`insert`，对象用点访问，`p["x"]` 不支持。

方法内调用兄弟方法：`self.other()`。构造对象的函数式风格见 `examples/objects.tyto`（链表）。

---

## 内置类型：Array

字面量 `[1, "a", [2]]`；引用语义；`+` 拼接生成新数组；for-in 按索引迭代。

| 方法 | 签名 | 说明 |
|---|---|---|
| push | `push(x, ...) → number` | 追加所有参数，返回**新长度** |
| pop | `pop() → value` | 移除并返回末尾元素；空数组返回 `null` |
| len | `len() → number` | |
| is_empty | `is_empty() → bool` | |
| contains | `contains(x) → bool` | 按语言 `==` 语义 |
| index_of | `index_of(x) → number` | 首次出现下标；未找到 `-1` |
| join | `join(sep?) → string` | 默认分隔符 `,` |
| sort | `sort() → array` | 就地排序并返回自身（可链式）。无比较器时要求**全 number 或全 string**，否则报错 |
| sort | `sort(f) → array` | 比较器 `f(a, b) → number`，负数表示 a 在前；**稳定排序**；比较器内抛错会中断 |
| reverse | `reverse() → array` | 就地反转，返回自身 |
| slice | `slice(start, end?) → array` | 新数组；**负下标从末尾数**，越界截断，小数截断（JS slice 语义） |
| map | `map(f) → array` | `f(elem)`，返回新数组 |
| filter | `filter(f) → array` | `f(elem)` 真值保留 |
| fold | `fold(init, f) → value` | `f(acc, elem)` 依次折叠；init 在前（Rust 参数顺序） |

```
a = [3, 1, 2]
a.sort()                                        // [1, 2, 3]
a.sort(function(x, y) { return y - x })         // [3, 2, 1] 降序
[1, 2, 3, 4].slice(1, 3)                        // [2, 3]
[1, 2, 3, 4].slice(-2)                          // [3, 4]
[1, 2, 3].fold(0, function(s, x) { return s + x })   // 6
```

越界（读和写）报错；索引必须非负整数。

---

## 内置类型：String

不可变；按字符（Unicode 码点）索引与计长；`+` 拼接；比较按字典序。

| 方法 | 签名 | 说明 |
|---|---|---|
| len | `len() → number` | 字符数（不是字节数） |
| is_empty | `is_empty() → bool` | |
| split | `split(sep) → array` | `sep` 为空串时按字符拆分 |
| contains | `contains(s) → bool` | 子串 |
| index_of | `index_of(s) → number` | 首次出现的字符下标；未找到 `-1` |
| trim | `trim() → string` | 去两端空白 |
| starts_with | `starts_with(s) → bool` | |
| ends_with | `ends_with(s) → bool` | |
| to_uppercase | `to_uppercase() → string` | |
| to_lowercase | `to_lowercase() → string` | |
| sub | `sub(start, end?) → string` | 子串；**负下标从末尾数**、越界截断（同 slice 语义） |
| replace | `replace(old, new) → string` | 替换**全部**出现 |
| chars | `chars() → array` | 拆成单字符数组 |

```
"  Hello, World  ".trim().sub(0, 5)     // "Hello"
"hello world".index_of("world")          // 6
"a-b-c".split("-")                      // ["a", "b", "c"]
"abc".split("")                         // ["a", "b", "c"]
"na-x na-y".replace("na-", "")          // "x y"
```

`s[i]` 返回单字符字符串，越界报错。

---

## 内置类型：Map

保序哈希表。**键只能是 number / string / bool / null**（数组等做键报错）。

```
m = new Map()
m.insert("a", 1).insert("b", 2)      // set 返回自身，可链式
m.insert(1, "one")
m.insert(true, 3)

m.get("a")       // 1
m.get("zz")      // null（缺失返回 null，不报错）
m.contains_key("a")       // true
m.len()          // 4
m.keys()         // ["a", "b", 1, true] —— 按插入顺序
m.values()       // [1, 2, "one", 3]
m.remove("b")    // true（返回是否删除；保持其余键顺序）
```

- 数字键做规范化：`1` 与 `1.0` 是同一个键，`-0.0` 归为 `0`
- `for k in m.keys()` 遍历；判断键存在用 `contains_key`
- 显示格式 `Map {"a": 1, 1: "one"}`（键在容器内带引号）

---

## 内置类型：MaxHeap / MinHeap

原生 `BinaryHeap` 实现，只存 number。

```
h = new MaxHeap()          // 空；或 new MaxHeap([3, 1, 4]) 用数组初始化
h.push(3)
h.push(1)
h.peek()                   // 3（堆顶，不移除）
h.len()                    // 2
h.pop()                    // 3（最大者；MinHeap 为最小者）
```

| 方法 | 签名 | 说明 |
|---|---|---|
| push | `push(x) → null` | x 必须是 number |
| pop | `pop() → number` | 移除并返回堆顶；**空堆返回 `EMPTY`** |
| peek | `peek() → number` | 堆顶；空堆返回 `EMPTY` |
| len | `len() → number` | |
| is_empty | `is_empty() → bool` | |

`new` 只能构造原生类（Map / MaxHeap / MinHeap / Stack / Queue）；`MaxHeap()` 直接调用会报错并提示加 `new`。经典双堆中位数用法见 `examples/median.tyto`。

---

## 内置类型：Stack / Queue

栈（`Vec`，LIFO）与队列（`VecDeque`，FIFO）。元素可异构；空容器取值返回 `EMPTY`（与堆一致；数组的 `pop()` 空才返回 `null`）。

```
s = new Stack()          // 或 new Stack([1, 2, 3])（数组顺序 = 底→顶）
s.push(1)
s.push("two")
s.peek()                 // "two"（看顶，不移除）
s.pop()                  // "two"（移除并返回）
s.len()
s.is_empty()

q = new Queue()          // 或 new Queue(["a", "b"])（数组顺序 = 队头→队尾）
q.push_back(1)
q.front()                // 1（队头，不移除）
q.back()                 // 队尾
q.pop_front()            // 1（出队）
```

| 类型 | 方法 | 说明 |
|---|---|---|
| Stack | `push(x)` `pop()` `peek()` `len()` `is_empty()` | `pop`/`peek` 空时返回 `EMPTY` |
| Queue | `push_back(x)` `pop_front()` `front()` `back()` `len()` `is_empty()` | 命名对齐 Rust `VecDeque`；`pop_front`/`front`/`back` 空时返回 `EMPTY` |

显示格式：`Stack[1, 2, 3]`（顶在右）、`Queue[1, 2, 3]`（队头在左）。BFS 用法见 `examples/bfs.tyto`。

---

## 全局函数

### 输入输出

| 函数 | 签名 | 说明 |
|---|---|---|
| print | `print(...args)` | 输出，**不换行**；多参数以空格分隔 |
| println | `println(...args)` | 输出并换行；多参数以空格分隔 |
| input | `input() → string \| null` | 读入一行（去掉行尾换行）；EOF 返回 null |

```
n = num(input())            // 算法题标准起手式
line = input()
if line == null { println("结束") }
```

### 转换与信息

| 函数 | 签名 | 说明 |
|---|---|---|
| num | `num(x) → number` | number 原样返回；string 去空白后解析（非法报错） |
| str | `str(x) → string` | 按显示格式转字符串 |
| len | `len(x) → number` | 数组/字符串/对象/Map/堆；其余报错 |
| type | `type(x) → string` | 见[类型表](#类型与值) |
| has | `has(obj, "field") → bool` | 仅对象：是否有某字段（Map 请用 `contains_key` 方法） |

### 数学

| 函数 | 签名 | 说明 |
|---|---|---|
| floor / ceil | `(x) → number` | |
| round | `(x) → number` | 四舍五入（half away from zero：`round(2.5) == 3`、`round(-2.5) == -3`，与 JS 不同） |
| abs | `(x) → number` | |
| sqrt | `(x) → number` | |
| pow | `pow(a, b) → number` | |
| min / max | `min(a, b, ...) → number` | 多参数，或传单个数组 `min([3, 1, 2])`；空序列报错；要求全 number |

### 预置常量

| 名字 | 值 |
|---|---|
| `EMPTY` | 哨兵（空堆/空栈/空队列 的 pop/peek/pop_front/front/back 返回它；`== EMPTY` 判断；真值为假） |
| `inf` / `nan` | 正无穷 / 非数 |

---

## CLI 与 REPL

```sh
tyto script.tyto      # 运行脚本（后缀无所谓，解释器不检查）
tyto                  # REPL
tyto --tokens f       # 调试：打印 token 流（行:列 + 类型）
tyto --ast f          # 调试：打印 AST
```

REPL：全局作用域跨输入保持；未闭合的 `{`/`(`/字符串自动续行（提示符变 `..`）；**裸表达式回显求值结果**（字符串带引号显示）；Ctrl-D 退出。

错误一律带 `行:列` 与源码摘录箭头，文件模式下出错退出码为 1。

---

## 与 JS / Rust 的差异速查

| 主题 | Tyto | JS | Rust |
|---|---|---|---|
| 语句结尾 | 换行或 `;` | 分号/ASI | 分号 |
| `if` 括号 | 不写 | 必写 | 不写 |
| 数字 | 单一 number（f64） | number | 整数/浮点分家 |
| `1 == "1"` | false（无隐式转换） | true | — |
| 数组越界 | 报错 | undefined | panic |
| 读不存在的对象字段 | 报错 | undefined | 编译错 |
| `this`/`self` | 仅 `obj.f()` 调用注入 | 动态 this | 显式 self 参数 |
| 字符串 replace | 替换全部 | 替换第一个 | 替换全部 |
| `round(-2.5)` | -3（away from zero） | -2 | -3 |
| 作用域赋值 | 向上命中已有绑定，否则当前层新建 | var/let 规则 | let/let mut 就地 |
| `&&`/`\|\|` 返回值 | 返回操作数 | 返回操作数 | 返回 bool |

---

*v2 方向（未实现）：`struct`/`impl`/单继承、结构化 `interface` 与 `is`、`new T()` 自动调 `T::new()`、模块导入、字符串插值、`match`。*
