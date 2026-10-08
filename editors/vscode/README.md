# Tyto VSCode 插件

Tyto（.tyto）的语法高亮、**类型推导补全**、悬停文档与运行命令，零 npm 依赖。

## 架构：插件是桥，`tyto lsp` 是大脑

补全与悬停由 Rust 端语言服务器提供（复用 tyto 的词法器/解析器做真正的类型推导），插件只做转发；服务器不可用时自动降级为内置静态表，保持基础体验：

```
VSCode 插件（薄桥接，~150 行手写 LSP 客户端）
   │  spawn `tyto lsp`（stdio + Content-Length 分帧）
   ▼
tyto lsp（常驻进程：作用域收集 + 类型推导 + 内置表单一事实源）
```

## 安装

**前置**：`cargo install --path .` 安装 `tyto` 命令（补全/悬停需要；PATH 找不到时可设置 `tyto.serverPath` 指定路径）。

**打包安装：**

```sh
./editors/vscode/install.sh
```

脚本会自动打包并安装（自动探测 `code` / `code-oss` / `codium`）。等价的手动步骤：

```sh
cd editors/vscode
npx @vscode/vsce package --allow-missing-repository
code --install-extension tyto-lang-0.0.2.vsix
```

**开发调试：** 在本目录用 VSCode 打开，按 F5 启动扩展开发主机（会自动加载本插件），新建 `.tyto` 文件即可验证。服务器日志见输出面板「Tyto LSP」。

## 功能

- **语法高亮**（TextMate）：关键字 / 字符串 / 数字 / 注释 / 运算符 / `self` / stdlib 函数与 `MaxHeap`/`MinHeap`/`Map`/`EMPTY` / 函数声明的函数名
- **语义着色**（`tyto lsp`，叠加在 TextMate 之上）：基于类型推导给变量 / 参数 / 函数 / 方法 / 属性 / struct / interface / 内置类（`new Map()` 的 `Map`）/ 命名空间（`fs`/`sys`）着色；推不出的标识符保持静态高亮，宁可少色不可错色；编辑中途（语法不完整）时对可解析前缀照常着色
- **类型推导补全**（`tyto lsp`）：
  - 变量带推断类型：`n: number`、`m: map`、`p: Point`
  - 用户函数完整签名：`function add(a: number, b) -> number`（返回类型从标注或 return 语句推导）
  - `.` 后按接收者类型过滤：字符串给 `split`/`trim`，数组给 `map`/`fold`，Map 给 `insert`/`keys`，struct 实例给字段 + impl 方法，对象字面量给已知字段，`fs.`/`sys.` 给命名空间函数；类型未知时回退全量方法表
  - 作用域感知：函数内可见参数/局部变量/全局；`self.` 在 impl 方法与对象字面量方法内给对应成员
  - 惯用形支持：顶层 `h = null` + init() 内 `h = new MaxHeap()`，在函数体内编辑时全局变量按整个文件的最终赋值推导
- **跳转定义**（Ctrl+点击 / F12）：变量 → 声明处（多次赋值指向首个声明）、函数/struct/interface → 名字处、struct 字段与方法 → 声明处、参数 → 签名处、`self` → struct 声明处；内置符号无跳转
- **悬停文档**：变量显示推断类型（定义点也支持）、用户函数签名、struct 字段/方法、stdlib 函数与方法的签名说明
- **代码片段**：`fun` `fn` `if` `ife` `forin` `forr` `forc` `while` `prin` `counter`
- **运行**：命令面板 `tyto-lang: Run File` 或 `Ctrl+Alt+R`（macOS `Cmd+Alt+R`），在集成终端执行 `tyto <当前文件>`

## 降级行为

`tyto` 未安装 / `tyto.serverPath` 无效 / 服务器启动失败或中途退出 / 请求超时（5s）——任一情况：补全与悬停静默降级为静态表、语义着色保持 TextMate 高亮（不注册语义 provider 或返回空 token）、跳转定义返回空（VSCode 无操作）。输出面板「Tyto LSP」会记录原因。

## 测试

```sh
cargo test                          # Rust 全部测试（含 analysis 38 项 + semantics 17 项）
python3 scripts/lsp_smoke.py        # LSP 协议冒烟（补全/悬停/定义/语义着色/关停）
node scripts/lsp_client_test.js     # 插件端 LspClient 端到端（真实 tyto 二进制）
```

## 已知限制

- 无诊断（错误检测）/ 重命名——后续在 `tyto lsp` 上按需扩展
- 类型推导是编辑器近似：异构数组元素、运行时分支类型合并等场景会落到 unknown（此时给全量方法表、成员不着色）
- 对象字面量字段跳转按「就近向上的同名键」定位，多个对象含同名字段时可能跳到相近的那个（struct 字段/方法按 owner 精确匹配）
- `tyto.serverPath` 修改后需重启窗口生效
