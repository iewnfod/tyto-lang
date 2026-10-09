# AGENTS.md

面向 AI 编码代理（与人）的仓库工作指南。改代码前先读本文。

## 项目概述

**tyto-lang**：Rust 实现的渐进类型解释型语言。二进制名 `tyto`。
语言规范见 `docs/language-reference.md`（约 900 行，中文）。

- 入口：`src/main.rs` → `src/cli.rs`（手动参数解析，无 clap）→ 各命令
- 语言流水线：`lexer → parser → ast → interpreter`（运行）+ `checker`（诊断）
- 编辑器智能：`analysis`（补全/悬停/着色/跳转）经 `src/lsp.rs` 暴露为 LSP，VSCode 插件在 `editors/vscode/`

## 常用命令

```bash
cargo build                      # 构建（CI 同款）
cargo test                       # 全部单测 + 集成测试（CI 同款）
cargo run -- script.tyto         # 运行脚本
cargo run -- check script.tyto   # 类型检查（诊断不阻塞运行）
cargo run -- lsp                 # 语言服务器（stdio）

python3 scripts/lsp_smoke.py     # LSP 协议冒烟（CI 同款，需先 cargo build）
node scripts/lsp_client_test.js  # VSCode 插件 LspClient 端到端（需先 cargo build）

cd editors/vscode && vsce package  # 打包插件（install.sh 可探测安装）
```

CI（`.github/workflows/test.yml`）：`cargo build` + `cargo test` + `python3 scripts/lsp_smoke.py`。

## 架构与模块职责

```
源码 ──► lexer ──► parser ──► ast ─┬─► interpreter ──► 值/输出（运行时类型擦除）
                                  ├─► checker ──► 诊断（红线/警告，永不阻塞执行）
                                  └─► analysis ──► 补全/悬停/着色/跳转 ──► lsp.rs ──► VSCode
natives：内置库运行时实现（arrays/strings/maps/heaps/collections/files/system/fns）
```

| 模块 | 职责 |
|---|---|
| `src/lexer/` | 词法：`lexer.rs` 主循环与扫描、`token.rs` TokenKind/Keyword/运算符表、`tests.rs` 单测 |
| `src/parser/` | 语法（递归下降+优先级爬升）：`mod.rs` 结构与入口、`cursor.rs` 游标原语、`types.rs` 类型标注、`stmts.rs` 语句/声明、`exprs.rs` 表达式、`tests.rs` 单测 |
| `src/ast.rs` | AST 数据定义（Expr/Stmt/TypeAst），纯数据，四方共用 |
| `src/checker/` | 渐进类型检查器：`mod.rs` 状态与入口、`stmts.rs` 语句检查、`assign.rs` 赋值检查、`infer.rs` **表达式推导引擎（唯一实现）**、`call.rs` 调用/构造检查、`unify.rs` 泛型求解、`ty.rs` 类型表示与兼容性、`cursor.rs` **光标模式**（编辑器在容错哨兵 AST 上驱动引擎 + 无标注返回推导 `infer_func_ret`）、`builtins.rs` **内置表单一事实来源**、`registry.rs` struct 注册表、`diag.rs` 诊断 |
| `src/interpreter/` | 树遍历解释器：`mod.rs` Interpreter/Flow/全局安装、`exprs.rs` 表达式求值、`stmts.rs` 语句执行、`calls.rs` 调用与构造、`tests/` 端到端单测（basics/objects/structs） |
| `src/natives/` | 内置库运行时（方法分派 `call_method`、全局函数、fs/sys） |
| `src/analysis/` | 编辑器分析：`mod.rs` complete/hover 门面、`complete_items.rs`/`hover_info.rs` 条目构造、`ty_view.rs` 编辑器类型视图（展示名 `editor_display`/标注薄委托/联合解析/签名串）、`scope/` 光标作用域快照（`walker.rs` 走查器——驱动 checker 引擎**镜像链**，绑定与表达式类型全部单源到引擎）、`semantics/` 着色（`names.rs` token 扫描索引 + `highlight.rs` AST 走查，同样驱动引擎）与 `definition.rs` 跳转、`tolerate.rs` 容错解析与位置换算、`builtins.rs` 对 checker 事实源的查询层 + 纯展示数据（关键字/类型名/常量） |
| `src/lsp.rs` | LSP 服务器（lsp-server + 手写 JSON；编排 analysis + checker） |
| `src/cli.rs` / `src/main.rs` / `src/repl.rs` | CLI 参数、命令执行体、交互 REPL |
| `src/value.rs` / `src/scope.rs` / `src/error.rs` | 运行时值、运行时作用域链、错误与 Span |
| `editors/vscode/` | 插件：`extension.js` 装配、`lspClient.js` 手写 LSP 客户端、`builtins.js` 静态降级数据表、`staticFallback.js` 降级补全/悬停、`commands.js` 运行命令 |
| `tests/` | 集成测试（按功能分文件：checker/analysis/semantics/stdlib/structs/examples/fs_sys + `builtins_consistency_test.rs` 表对齐守护 + `engine_consistency_test.rs` **引擎单源守护**：光标路径与全文件检查的类型一致性） |

## 依赖纪律（改代码前必读）

1. **checker 是核心事实来源**：struct 注册表（`checker::registry`）、内置表（`checker::builtins`）与**类型推导引擎**（`checker::Checker`）只有一份，analysis 单向引用 checker。
2. **checker 永不依赖 analysis**；**interpreter 运行时类型擦除，永不依赖 checker**（标注纯文档性质）。
3. **类型表示与推导均已单源**：唯一类型表示是 `checker::ty::Type`（编辑器「推不出」由 `Any` 承担）；唯一推导实现是 checker 引擎——编辑器侧（walker/highlight）持引擎**镜像链**成对登记（压栈/绑定/ambient 替换两侧必须同步），编辑器只剩展示辅助（`analysis/ty_view.rs`）。新增推导规则只改 checker，两侧自动一致。
4. `lsp.rs` 同时编排 analysis 与 checker，但两者各走各路径，互不调用。

## 同步点清单（改一处必须同步的地方）

| 改什么 | 必须同步 |
|---|---|
| 内置方法/全局函数/fs·sys 函数/内置类（`src/natives/` 实现） | `src/checker/builtins.rs`（唯一 Rust 侧事实源：名字/签名/文档/返回类型）；`editors/vscode/builtins.js`（JS 降级表） |
| 语义 token 类型 | `src/analysis/semantics/mod.rs` 的 `TOKEN_TYPES` ↔ `editors/vscode/extension.js` 的 `SEMANTIC_LEGEND` ↔ `scripts/lsp_smoke.py` 的 `TOKEN_TYPES`（三处必须逐项一致） |
| CompletionItemKind 映射 | `src/lsp.rs` `completion_json` ↔ `editors/vscode/lspClient.js` `LSP_KIND` |
| 语言关键字 | `src/lexer/token.rs` `Keyword` ↔ `src/analysis/builtins.rs` `KEYWORDS` ↔ `editors/vscode/builtins.js` |

守卫：`tests/builtins_consistency_test.rs` 校验 analysis 展示表与 checker 事实源对齐；`tests/engine_consistency_test.rs` 校验光标路径与全文件检查的类型一致（镜像链防漂移）；`checker/builtins.rs` 内单测校验表条目完整（名字/签名/文档/arity）。

## 代码风格约定

- 注释与文档一律**中文**；模块头用 `//!`，条目文档讲「为什么」。
- 大文件内用 `// ============ 分节 ============` 注释分组——**分节注释即拆分边界**。
- 拆大文件的既定模式（参考 parser/interpreter/checker）：
  - 目录模块 + 同一类型的多个 `impl` 块分散在子文件；跨子模块调用的方法用 `pub(super)`/`pub(crate)`。
  - 内嵌 `#[cfg(test)] mod tests` 超过 ~200 行时外移为同级 `tests.rs`（或 `tests/` 目录），在 mod.rs 声明 `#[cfg(test)] mod tests;`。
- 测试：单元测试贴实现；集成测试在 `tests/`（analysis/semantics 测试用 `§` 作光标标记，`at()` 辅助定位）。
- 错误消息面向用户（会显示在红线/CLI），保持现有措辞风格，勿随意改动（测试断言消息子串）。

## 文件尺寸约定

- 源文件（不含测试）**~400 行软上限、500 行硬上限**；超限先外移测试，再按分节注释拆子模块。
- 纯数据表（AST/内置表）与测试文件可适当放宽，但仍按主题分文件。
- 新增代码放对模块：见上表职责，拿不准就贴着最相似的现有文件放。

## 已知债务（有意不做 / 待办）

- 无标注函数的返回推导（`Checker::infer_func_ret`）目前只服务编辑器光标路径，**尚未接入诊断路径**（`check_program` 对无标注函数的调用仍回 Any）——接入属后续独立工作。
- 镜像链是成对簿记（编辑器作用域 + 引擎作用域），walker/highlight 两侧的压栈与登记必须同步；`tests/engine_consistency_test.rs` 是漂移防线。后续若要走 trait 抽象引擎（TypeEnv）消除簿记，属独立重构。
- `extension.js` 的 JS 降级表无法与 Rust 侧共享代码（无构建管线），靠同步清单 + 冒烟测试约束。
- LSP 尚无 rename/formatting/signatureHelp（见 `editors/vscode/README.md` 已知限制）。
- `editors/vscode/` 下历史 `.vsix` 打包产物未清理（是否删除由维护者决定）。
