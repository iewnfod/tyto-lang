# Tyto 语言参考

本文档完整描述 Tyto v0.1 的语法、语义与内置库。所有行为均与当前实现一一对应，未实现的内容不在此列。

- [词法](#词法)
- [类型与值](#类型与值)
- [类型标注（不检查）](#类型标注不检查)
- [运算符与表达式](#运算符与表达式)
- [语句](#语句)
- [函数与作用域](#函数与作用域)
- [对象](#对象)
- [struct 与 impl](#struct-与-impl)
- [interface 与 is](#interface-与-is)
- [内置类型：Array](#内置类型array)
- [内置类型：String](#内置类型string)
- [内置类型：Map](#内置类型map)
- [内置类型：MaxHeap / MinHeap](#内置类型maxheap--minheap)
- [内置类型：Stack / Queue](#内置类型stack--queue)
- [全局函数](#全局函数)
- [命名空间：fs / sys](#命名空间fs--sys)
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

关键字（共 18 个）：

```
function  return  if  else  while  for  in  break  continue
true  false  null  new
struct  impl  interface  is
```

> ⚠️ `let` 目前**不是**关键字（可作变量名），是后续版本的预留方向，新代码请避免使用。

命名惯例（语言不强制）：变量/函数 `snake_case`，类型（struct / interface）`CamelCase`，常量 `UPPER_SNAKE`。

### 字面量

| 种类 | 写法 | 备注 |
|---|---|---|
| 数字 | `42`、`3.14`、`1e3`、`2.5e-2`、`.5` | 单一 number 类型（f64）；`1.` 无效（点后必须有数字） |
| 字符串 | `"abc"` 或 `'abc'` | 转义：`\n \t \r \\ \" \' \0`；源码中的裸换行是错误（需写 `\n`） |
| 数组 | `[1, 2, 3]` | 允许尾逗号、嵌套、异构 |
| 对象 | `{x: 1, y: 2}` | 允许尾逗号；键可为标识符或字符串 |
| 布尔/空 | `true` `false` `null` | |

区间 `a..b`（不含 b）与 `a..=b`（含 b）**只能**出现在 for-in 头部与切片方括号内（见[切片](#切片)），不是独立的值。

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
| struct | `"struct"` | `struct P {...}` 定义本身（类型对象） |
| interface | `"interface"` | `interface I {...}` 定义本身（类型对象） |
| 实例 | struct 名（如 `"Point"`） | `new P(...)` 的结果；`type(p)` 返回 struct 名 |

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

## 类型标注（不检查）

Tyto **不是强类型语言**。类型标注是纯文档性质：让代码更清晰、为将来的类型推导保留信息，**运行时完全不检查**——标注与实际值不符不报错，删掉任何标注程序行为不变。

可标注的位置：

```
x: number = 1                                       // 变量（必须带 = 值）
function add(a: number, b: number) -> number {      // 参数 + 返回类型
    return a + b
}
double = function(x: number) -> number { return x * 2 }   // 匿名函数同样支持
struct Point {
    x: number,          // struct 字段
    y: number,
}
interface Shape {
    function area() -> number    // interface 签名里也接受（只保留方法名契约）
}
```

类型写法：标识符 + 可选泛型参数 + 可选 `[]` 后缀，如 `number`、`Point`、`number[]`、`Map<string, number>`。惯用基础名 `number` / `string` / `bool` / `null` / `any`，struct / interface 名，以及 `Array` / `Map` 等内置类名（语言不强制任何写法）。

规则细节：

- 变量标注只允许普通变量名 + `=`：`x: number = 1` 合法；`a[0]: number = 1`、`p.x: number = 1`、`x: number += 1`、缺 `=` 的单独 `x: number` 均为解析错误
- `->` 在 `)` 之后、`{` 之前，可跨行书写
- 标注会存入 AST 供工具使用；解释器只取参数/字段名，参数个数检查、作用域等一切语义照旧

---

## 运算符与表达式

### 优先级（从低到高）

| 级 | 运算符 | 备注 |
|---|---|---|
| 1 | `? :` | 三元，右结合：`a ? b : c ? d : e` |
| 2 | `??` | nullish 合并，短路，左结合 |
| 3 | `\|\|` | 短路 |
| 4 | `&&` | 短路 |
| 5 | `==  !=  is` | `is` 见 [interface 与 is](#interface-与-is) |
| 6 | `<  >  <=  >=` | 仅 number×number 或 string×string |
| 7 | `+  -` | |
| 8 | `*  /  %` | |
| 9 | `!x` `-x` | 一元，可叠：`- -x` |
| 10 | `f(...)` `a[i]` `a[i..j]` `a.b` `a?.b` | 后缀，从左到右链 |

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

### `??` nullish 合并

**仅当左侧是 `null`** 时取右侧，否则返回左侧——短路，左结合，优先级介于三元与 `||` 之间。

```
null ?? "x"    // "x"
0 ?? "x"       // 0    —— 假值不触发（区别于 ||）
"" ?? "x"      // ""
EMPTY ?? 1     // EMPTY —— 容器哨兵不参与
9 ?? boom()    // 9    —— 右侧不求值
p?.x ?? 0      // 可选链配合：p 为 null 时得 0
```

典型用途：给"可能为 null"的值兜底，而不吃掉 `0`/`""`/`false` 等合法假值。

### 赋值运算符

`=  +=  -=  *=  /=  %=  ??=`，目标可以是变量、索引（`a[0] += 1`）或对象字段（`p.n *= 2`）。`?.` 不能作为赋值目标。

`??=` 仅在旧值为 `null` 时才求值并写入右侧（右侧惰性）：

```
h = null
h ??= new MaxHeap()   // h 为 null → 构造
h ??= new MaxHeap()   // h 已存在 → 右侧不求值，保持原堆
```

### 索引

- `a[i]`：i 必须是**非负整数** number；数组越界（读/写）都报错，不支持负索引（负索引请用切片或 `slice`/`sub`）
- `s[i]`：返回单字符字符串（按字符，不是字节），越界报错
- 字符串不可变：`s[0] = "x"` 报错
- Map 不支持 `m[k]` 读写，请用 `get`/`insert` 方法

### 切片

`a[start..end]` 截取数组或字符串的一段，生成**新值**（数组是新数组，与原数组引用隔离）。端点可省略，`..` 不含 end，`..=` 含 end：

```
a = [1, 2, 3, 4]
a[1..3]      // [2, 3]     （不含下标 3）
a[1..=3]     // [2, 3, 4]  （含下标 3）
a[..2]       // [1, 2]     （省略起点 = 从 0 起）
a[2..]       // [3, 4]     （省略终点 = 到末尾止）
a[..]        // [1, 2, 3, 4]（全量拷贝）

s = "Hello"
s[1..3]      // "el"
s[..2]       // "He"
s[2..]       // "llo"
```

语义与 `slice()`/`sub()` 方法完全一致：

- **负下标从末尾数**：`a[-2..]` → `[3, 4]`，`s[..-1]` → `"Hell"`
- **越界截断**：`a[1..100]` → `[2, 3, 4]`，不报错
- **小数截断**：`a[0.9..2.7]` → `a[0..2]`
- 起点 ≥ 终点得**空值**（`a[2..1]` → `[]`），不报错
- 字符串按**字符**（Unicode 码点）切片：`"héllo"[1..3]` → `"él"`
- 端点可以是任意表达式：`a[n + 1..m]`；切片可继续链式：`a[1..][0]`、`s[..2].len()`

切片是表达式，**不能作为赋值目标**（`a[1..2] = x` 报错）；对非数组/字符串切片（如 number、map）报运行时错误。

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
- 参数与返回值可加类型标注（纯文档性质，见[类型标注](#类型标注不检查)）：

```
function add(a: number, b: number) -> number {
    return a + b
}
```

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

## struct 与 impl

对象字面量是"随手造"的世界；struct 是**固定形状**的世界：字段先声明后使用，读/写未声明的字段都会报错（typo 早暴露），方法集中放在 `impl` 里、所有实例共享。

```
struct Point {
    x,
    y,
}

impl Point {
    function new(x, y) {
        self.x = x
        self.y = y
    }
    function len() {
        return sqrt(self.x * self.x + self.y * self.y)
    }
    function scaled(k) {
        return new Point(self.x * k, self.y * k)
    }
}

p = new Point(3, 4)
println(p)          // Point {x: 3, y: 4}
println(p.len())    // 5
```

### 声明

- `struct Name { x, y }`：字段只写名字，也可带类型标注 `struct Point { x: number, y: number }`（纯文档性质，见[类型标注](#类型标注不检查)）；逗号或换行分隔，允许尾逗号；重复字段报运行时错误
- `struct` 是语句，执行到时在当前作用域定义绑定 `Name`（先定义后使用，同函数声明）
- 定义本身就是值：`println(Point)` → `<struct Point>`，`type(Point)` → `"struct"`

### 构造：`new Name(args)`

- 字段全部从 `null` 起步
- impl 里定义了 `new` 方法 → 调用 `Name::new(args)`（`self` 注入为新实例），**返回值忽略**，表达式的值始终是新实例
- 没定义 `new` → **按位置初始化**：参数按字段声明顺序赋值（`new Rect(3, 4)` 即 `w=3, h=4`）；参数多于字段数报错，无参则全 null
- `Name(args)` 不带 `new` 直接调用报错（提示加 `new`）

### 字段与方法

| 操作 | 行为 |
|---|---|
| `p.x` 读 | 字段不存在 → **报错**（含方法名也一样，除非取方法，见下） |
| `p.x = 5` 写 | 字段不存在 → **报错**，**不自动创建**（与对象的关键区别） |
| `p.x += 1` | 复合赋值可用（字段存在时） |
| `p?.x` | 可选链同对象语义（null 短路，非 null 严格读取） |
| `p.f(args)` | 先查字段（是函数则以 `self` 调用），再查 impl 方法表；都没有 → 报错 |
| `m = p.f` | 取出方法为普通函数，单独调用**不注入 `self`**（与对象方法一致） |
| `len(p)` | 字段数 |
| `has(p, "x")` | 是否有该字段 |
| `p == q` | 引用比较 |
| `println(p)` | `Point {x: 3, y: 4}`（空实例 `Point {}`） |

- 字段与方法是两个命名空间，访问时**字段优先**
- 方法内互调：`self.other()`；方法体内再构造同类：`new Point(...)`
- `impl` 的目标必须是已定义的 struct（`impl x { ... }` 其中 x 不是 struct → 报错）
- 同一 struct 可以多次 `impl`，后定义的同名方法**覆盖**旧的；实例持有 struct 定义的共享引用，后挂的方法对已有实例同样生效
- impl 中方法闭包捕获 impl 执行时的作用域（顶层 impl 即全局作用域）

struct 版链表见 `examples/shapes.tyto` 与 `tests/structs_test.rs`。

---

## interface 与 is

`interface` 是**结构化**的类型声明：只列出方法签名，不改变任何运行时行为。一个值是否"实现"了接口，由 `is` 在运行时按**方法集**判断——没有也不需要显式的"实现"声明。

```
interface Shape {
    function area()
    function describe()
}

struct Rect { w, h }
impl Rect {
    function area() { return self.w * self.h }
    function describe() { return "Rect " + self.w + "x" + self.h }
}

r = new Rect(3, 4)
println(r is Shape)   // true：Rect 的 impl 覆盖了 Shape 的全部方法

o = {area: function() { return 1 }, describe: function() { return "obj" }}
println(o is Shape)   // true：普通对象按函数字段结构化匹配
```

### interface 声明

- 方法签名只写 `function name(params)`，**没有函数体**（写了 `{` 是解析错误）
- 签名之间用逗号或换行分隔；空接口合法（`interface Any {}`）
- 执行到时定义绑定 `Name`（`type(I)` → `"interface"`，`println(I)` → `<interface Shape>`）；interface 不能被调用或构造

### `is` 运算符

优先级与 `==`/`!=` 同级、左结合：

```
if s is Shape && s.area() > 10 { ... }
println(p is T == true)
```

| 右侧 | 判定规则 |
|---|---|
| struct（如 `p is Point`） | **名义**：p 是该 struct 的实例 → true；其余值（含其它 struct 的实例）→ false |
| interface（如 `p is Shape`） | **结构化**：实例 → 其 struct 的 impl 方法表覆盖接口全部方法名；对象 → 这些名字的字段都是函数；其余类型 → false |
| 其它值 | **运行时报错**（右侧必须是 struct 或 interface） |

- 接口检查只看**方法名**，不看参数个数
- `is` 不做任何隐式转换；实例与 struct 名之间的判断恒为布尔值，不会报错

完整示例见 `examples/shapes.tyto`（异构集合 + `is` 过滤）。

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
| slice | `slice(start, end?) → array` | 新数组；**负下标从末尾数**，越界截断，小数截断（JS slice 语义）；等价于 `a[start..end]` 切片语法 |
| map | `map(f) → array` | `f(elem)`，返回新数组 |
| filter | `filter(f) → array` | `f(elem)` 真值保留 |
| fold | `fold(init, f) → value` | `f(acc, elem)` 依次折叠；init 在前（Rust 参数顺序） |

```
a = [3, 1, 2]
a.sort()                                        // [1, 2, 3]
a.sort(function(x, y) { return y - x })         // [3, 2, 1] 降序
[1, 2, 3, 4].slice(1, 3)                        // [2, 3]
[1, 2, 3, 4].slice(-2)                          // [3, 4]
[1, 2, 3, 4][1..3]                              // [2, 3]（切片语法）
[1, 2, 3].fold(0, function(s, x) { return s + x })   // 6
```

越界（读和写）报错；索引必须非负整数（截取一段请用切片语法）。

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
| sub | `sub(start, end?) → string` | 子串；**负下标从末尾数**、越界截断（同 slice 语义）；等价于 `s[start..end]` 切片语法 |
| replace | `replace(old, new) → string` | 替换**全部**出现 |
| chars | `chars() → array` | 拆成单字符数组 |

```
"  Hello, World  ".trim().sub(0, 5)     // "Hello"
"  Hello, World  ".trim()[..5]          // "Hello"（切片语法）
"hello world".index_of("world")          // 6
"a-b-c".split("-")                      // ["a", "b", "c"]
"abc".split("")                         // ["a", "b", "c"]
"na-x na-y".replace("na-", "")          // "x y"
```

`s[i]` 返回单字符字符串，越界报错；截取子串请用切片语法 `s[start..end]`。

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

`new` 能构造原生类（Map / MaxHeap / MinHeap / Stack / Queue）与用户 struct（见 [struct 与 impl](#struct-与-impl)）；`MaxHeap()` 直接调用会报错并提示加 `new`。经典双堆中位数用法见 `examples/median.tyto`。

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
| `fs` / `sys` | 命名空间对象（见下节） |

---

## 命名空间：fs / sys

文件与系统操作不走全局函数，收敛在两个预置命名空间对象里（v2 模块系统落地后无缝升级为真模块）。所有路径按 UTF-8 字符串处理。

### fs —— 文件

| 函数 | 签名 | 说明 |
|---|---|---|
| read_file | `fs.read_file(path) → string` | 整文件读入；不存在/非 UTF-8 报错 |
| read_lines | `fs.read_lines(path) → array` | 按行读入（去行尾换行；末尾换行不产生空尾元素） |
| write_file | `fs.write_file(path, content)` | 覆盖写；不存在则创建；**父目录必须存在**，否则报错 |
| append_file | `fs.append_file(path, content)` | 追加写；不存在则创建 |
| exists | `fs.exists(path) → bool` | 文件/目录存在性 |
| list_dir | `fs.list_dir(path) → array` | 目录内条目名（按名排序，含子目录） |

```
for line in fs.read_lines("data.txt") {
    println(num(line) * 2)
}
fs.write_file("out.txt", "done\n")
```

### sys —— 系统

| 函数 | 签名 | 说明 |
|---|---|---|
| shell | `sys.shell(cmd) → {status, stdout, stderr}` | `sh -c` 执行（Windows `cmd /C`）；**非零退出不报错**；stdout/stderr 原样保留（含换行，要干净自己 `.trim()`）；被信号杀死 status 为 -1 |
| get_env | `sys.get_env(name) → string \| null` | 环境变量；未设置返回 null |
| args | `sys.args() → array` | 脚本命令行参数：`tyto script.tyto a b` → `["a", "b"]` |

```
r = sys.shell("ls -la")
if r.status != 0 {
    println("出错: " + r.stderr)
}
println(r.stdout)

for a in sys.args() {
    println("参数: " + a)
}
```

---

## CLI 与 REPL

```sh
tyto script.tyto      # 运行脚本（后缀无所谓，解释器不检查）；额外的参数经 sys.args() 传给脚本
tyto                  # REPL
tyto lsp              # 语言服务器（stdio，编辑器插件用；补全/悬停的类型推导）
tyto --tokens f       # 调试：打印 token 流（行:列 + 类型），短参 -t
tyto --ast f          # 调试：打印 AST，短参 -a
tyto --time f         # 脚本执行完后输出耗时（stderr），短参 -T
```

选项可放在任意位置；以 `-` 开头的脚本参数需用 `--` 分隔（`tyto s.tyto -- -t` 中的 `-t` 会传给脚本而不是当成选项）。未知选项与缺文件的调试开关以退出码 2 结束。

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
| struct 未声明字段 | 读/写都报错（固定形状） | — | 编译错 |
| 实现接口 | 结构化（`is` 按方法集判断） | implements 声明 | impl 声明 |
| `this`/`self` | 仅 `obj.f()` 调用注入 | 动态 this | 显式 self 参数 |
| 字符串 replace | 替换全部 | 替换第一个 | 替换全部 |
| `round(-2.5)` | -3（away from zero） | -2 | -3 |
| 作用域赋值 | 向上命中已有绑定，否则当前层新建 | var/let 规则 | let/let mut 就地 |
| `&&`/`\|\|` 返回值 | 返回操作数 | 返回操作数 | 返回 bool |

---

*v2 方向（未实现）：单继承、模块导入、字符串插值、`match`。（struct / impl / interface / is 已在本文档落地。）*
