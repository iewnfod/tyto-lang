# Tyto VSCode 插件

Tyto（.tyto）的语法高亮、补全、悬停文档与运行命令，零 npm 依赖。

## 安装

**打包安装：**

```sh
cd editors/vscode
npx @vscode/vsce package --allow-missing-repository
code --install-extension tyto-lang-0.0.1.vsix
```

**开发调试：** 在本目录用 VSCode 打开，按 F5 启动扩展开发主机（会自动加载本插件），新建 `.tyto` 文件即可验证。

## 功能

- **语法高亮**：关键字 / 字符串 / 数字 / 注释 / 运算符 / `self` / stdlib 函数与 `MaxHeap`/`MinHeap`/`Map`/`EMPTY` / 函数声明的函数名
- **补全**：关键字 + 全部 stdlib；输入 `.` 后补全原生方法（无类型信息，方法池按 detail 标注适用类型）
- **悬停文档**：stdlib 函数 / 方法 / 内置类的签名与说明
- **代码片段**：`fun` `fn` `if` `ife` `forin` `forr` `forc` `while` `prin` `counter`
- **运行**：命令面板 `tyto-lang: Run File` 或 `Ctrl+Alt+R`（macOS `Cmd+Alt+R`），在集成终端执行 `tyto <当前文件>`

  > 运行前需 `cargo install --path .` 安装 `tyto` 命令。

## 已知限制

无 LSP：没有诊断（错误检测）、跳转定义、重命名——这些需要语言服务器，等语言本体稳定后再考虑。
