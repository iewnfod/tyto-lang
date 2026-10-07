# Tyto

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
