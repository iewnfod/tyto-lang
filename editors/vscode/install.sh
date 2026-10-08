#!/usr/bin/env bash
# 构建 VSCode 插件并安装：打包 .vsix 后调用 code --install-extension
# 用法：editors/vscode/install.sh
set -euo pipefail

cd "$(dirname "$0")"

# 从 package.json 读取 name/version，vsce 输出文件名为 <name>-<version>.vsix
name=$(node -p "require('./package.json').name")
version=$(node -p "require('./package.json').version")
vsix="${name}-${version}.vsix"

echo "==> 打包 ${name} v${version}"
rm -f "$vsix"
npx --yes @vscode/vsce package --allow-missing-repository

# 找一个可用的编辑器命令（code / code-oss / codium）
code_bin=""
for c in code code-oss codium; do
    if command -v "$c" >/dev/null 2>&1; then
        code_bin="$c"
        break
    fi
done

if [ -z "$code_bin" ]; then
    echo "==> 未找到 code 命令，请手动安装：code --install-extension $vsix" >&2
    exit 1
fi

echo "==> 安装到 ${code_bin}"
"$code_bin" --install-extension "$vsix"
echo "==> 完成，重启 VSCode 后生效"
