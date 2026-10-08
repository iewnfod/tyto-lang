# TyTo

**完整语言参考**：[docs/language-reference.md](docs/language-reference.md)——全部语法、语义规则与内置库的详细文档。

## 用法

```sh
cargo install --path .     # 安装 tyto 命令
tyto script.tyto           # 运行脚本（后缀 .tyto，避免与 .rt/.rtl 等老格式冲突）
tyto                       # REPL（Ctrl-D 退出）
tyto --help                # 完整 CLI 说明
```

## 示例

见 [examples/](examples/)：`median.tyto`（双堆中位数，原样来自日常伪代码）、`bfs.tyto`（Queue + Map 最短路）、`fib.tyto`（Map 记忆化）、`word_count.tyto`、`objects.tyto`（self 链表）。仓库自带 `.vscode/settings.json`，把 `*.tyto` 关联为独立语言，不会被编辑器误认成 R。

## 编辑器支持

`editors/vscode/` 是官方 VSCode 插件（高亮 / **类型推导补全** / 悬停文档 / `Ctrl+Alt+R` 运行当前文件）：

```sh
cargo install --path .        # 先装 tyto（补全/悬停的语言服务器）
./editors/vscode/install.sh   # 一键打包并安装
```

补全与悬停由 `tyto lsp` 语言服务器提供：变量带推断类型、用户函数完整签名、`.` 后按接收者类型过滤方法、struct 字段/方法与 `self` 感知；服务器不可用时插件自动降级为静态表。详见 [editors/vscode/README.md](editors/vscode/README.md)。

## v2 方向（部分已实现）

已实现：

- `struct` + `impl` 分离定义（字段固定、方法共享、`new` 构造）
- 结构化 `interface`（纯契约声明，`is` 运行时按方法集检查）
- `new T()` = 构造实例 + 自动调用 `T::new()`（无 `new` 方法时按位置初始化字段）
- 可选类型标注：`x: number = 1`、参数 `a: number`、返回 `-> number`、struct 字段（纯文档性质，运行时不检查）

规划中：

- 单继承（字段+方法+构造链）、`impl for` 声明时早报错
- 模块导入、字符串插值 `` `sum is ${x}` ``、`match`、BigInt
