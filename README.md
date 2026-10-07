# Tyto

个人脚本语言：动态类型、GC、无所有权。语法 = 「TS 的语义习惯 + Rust 的书写习惯」——`if` 不用括号、函数用 `function`、snake_case 命名、`new MaxHeap()` 构造、单一 `number` 类型（JS 式）。

名字来自仓鸮属学名 *Tyto*（猫头鹰，Tyto alba）——半夜写脚本的那位。也顺带致敬 T**y**peScript 与 rus**t**（原名 rt = rust & typescript）。

定位：快速写算法和小工具（python/bash 的位置，但写着顺手）。

**完整语言参考**：[docs/language-reference.md](docs/language-reference.md)——全部语法、语义规则与内置库的详细文档。

## 用法

```sh
cargo install --path .   # 安装 tyto 命令
tyto script.tyto         # 运行脚本（后缀 .tyto，避免与 .rt/.rtl 等老格式冲突）
tyto                     # REPL（Ctrl-D 退出）
tyto --tokens script.tyto  # 调试：token 流
tyto --ast script.tyto     # 调试：AST
```

## 语法速查

```rs
// 注释：// 行注释，/* */ 块注释；换行是语句分隔符（分号可写可不写）

// 变量：裸赋值。函数内赋值向上找已有绑定（命中全局），否则新建局部
h_low = null

// 函数：一等公民，闭包按引用捕获；匿名函数可做值
function Add(a, b) {
    return a + b
}
double = function(x) { return x * 2 }

// 分支：无括号 if / else if / else；三元（TS 习惯）
if x > 10 { ... } else if x > 3 { ... } else { ... }
y = x > 0 ? x : -x

// 循环：for-in（数组/字符串/range）、C 风格；break / continue
for x in [1, 2, 3] { ... }
for i in 0..n { ... }          // ..= 含端点
for i = 0; i < n; i += 1 { ... }
while cond { ... }

// 运算：+ - * / %，== != < > <= >=，&& || !（短路、返回操作数）
// number 是 JS 式单一类型：2 显示 2，2.5 显示 2.5，1/0 = inf，0/0 = nan
// + 遇字符串自动拼接（"a" + 1 == "a1"），数组 + 数组是拼接

// 数组 / 对象 / 可选链
xs = [1, 2, 3]
xs[0]                          // 越界报错（严格）
p = { x: 1, y: 2 }
p.x = 5                        // 字段自动创建
p?.x                           // p 为 null 时返回 null；读缺失字段报错
p.inc = function() { self.n += 1 }   // obj.f() 调用自动绑定 self

// 原生类型：new MaxHeap() / new MinHeap()（可选数组建堆）/ new Map() / new Stack() / new Queue()
h = new MaxHeap([3, 1])
h.push(4); h.pop(); h.peek(); h.len(); h.is_empty()   // 空堆 pop/peek 返回 EMPTY
m = new Map()                  // 键：number/string/bool/null，保序
m.insert(k, v).insert(k2, v2)  // insert 可链式
m.get(k)                       // 缺失返回 null
```

## 标准库

**全局**：`print`（不换行）/ `println`（换行，多参数空格分隔）、`input()`（一行→string，EOF→null）、`num(s)`、`str(x)`、`len(x)`、`type(x)`、`has(obj, "field")`、`floor` `ceil` `round` `abs` `sqrt` `pow` `min` `max`

**数组**：`push(…)`（返回新长度）`pop()`（空→null）`len` `is_empty` `contains` `index_of` `join(sep?)` `sort()` / `sort(f)`（比较器返回数字，负数在前；就地排序返回自身）`reverse` `slice(start, end?)`（负下标）`map(f)` `filter(f)` `fold(init, f)`

**字符串**（按字符索引）：`len` `is_empty` `split(sep)` `contains` `index_of` `trim` `starts_with` `ends_with` `to_uppercase` `to_lowercase` `sub(start, end?)` `replace(old, new)`（替换全部）`chars`

**Map**：`get` `insert` `contains_key` `remove` `len` `keys` `values` `is_empty`

**堆**（`BinaryHeap` 原生实现）：`push` `pop` `peek` `len` `is_empty`

**栈/队列**（Vec / VecDeque）：Stack `push` `pop` `peek` `len` `is_empty`；Queue `push_back` `pop_front` `front` `back` `len` `is_empty`（空容器取值返回 `EMPTY`）

## 语义要点

| 规则 | 行为 |
|---|---|
| 作用域 | 读沿链向上；赋值命中已有绑定（含全局），否则当前作用域新建（Lua 式）——顶层声明 + `Init()` 里赋值的伪代码模式直接可用 |
| `self` | 仅 `obj.f()` 调用注入；取出函数单独调用不绑定（比 JS 的动态 `this` 可预测） |
| `==` | 原始值按值、容器按引用、跨类型恒 false（无隐式转换，类似 `===`） |
| 真值 | `null` `false` `0` `nan` `""` `EMPTY` 为假，其余为真 |
| 字段读取 | 不存在报错（typo 早暴露），`?.` 兜底 |
| `EMPTY` | 全局哨兵（CAIE 伪代码习惯），空堆 `pop()/peek()` 返回它 |
| 常量预置 | `EMPTY` `inf` `nan` |

## 示例

见 [examples/](examples/)：`median.tyto`（双堆中位数，原样来自日常伪代码）、`bfs.tyto`（Queue + Map 最短路）、`fib.tyto`（Map 记忆化）、`word_count.tyto`、`objects.tyto`（self 链表）。仓库自带 `.vscode/settings.json`，把 `*.tyto` 关联为独立语言，不会被编辑器误认成 R。

## 编辑器支持

`editors/vscode/` 是官方 VSCode 插件（高亮 / 补全 / 悬停文档 / `Ctrl+Alt+R` 运行当前文件）：

```sh
cd editors/vscode
npx @vscode/vsce package --allow-missing-repository
code --install-extension tyto-lang-0.0.1.vsix
```

详见 [editors/vscode/README.md](editors/vscode/README.md)。

## v2 方向（已定设计，未实现）

- `struct` + `impl` 分离定义、单继承（字段+方法+构造链）
- 结构化 `interface`（纯契约，`is` 运行时检查，`impl for` 声明时早报错）
- `new T()` = 构造实例 + 自动调用 `T::new()`
- 模块导入、字符串插值 `` `sum is ${x}` ``、`match`、BigInt
