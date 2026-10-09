// 静态降级实现：语言服务器不可用时的补全与悬停（原版逻辑）。

const vscode = require('vscode');

const {
    KEYWORDS, GLOBALS, CLASSES, CONSTANTS, TYPES, METHODS,
    GLOBAL_MAP, CLASS_MAP, CONST_MAP, METHOD_MAP,
} = require('./builtins');

function completion(name, sig, doc, kind) {
    const item = new vscode.CompletionItem(name, kind);
    item.detail = sig;
    item.documentation = new vscode.MarkdownString(doc);
    return item;
}

function staticCompletion(document, position) {
    const prefix = document.lineAt(position).text.slice(0, position.character);
    const afterDot = /\.\s*\w*$/.test(prefix);

    if (afterDot) {
        return METHODS.map(([name, sig, doc]) =>
            completion(name, sig, doc, vscode.CompletionItemKind.Method));
    }

    // 类型标注位置：`:` / `->` 之后给类型名。
    // 三元 `? ... :` 不算；本行 `{` 比 `(` 更近的是对象字面量键值（跨行对象
    // 追踪不到，静态降级的已知局限——语言服务器在线时由 Rust 端精确判定）。
    const inType = /->\s*\w*$/.test(prefix)
        || (/:\s*\w*$/.test(prefix)
            && !/\?[^?:]*:\s*\w*$/.test(prefix)
            && !(prefix.lastIndexOf('{') > prefix.lastIndexOf('(')));
    if (inType) {
        return TYPES.map(([name, sig, doc]) =>
            completion(name, sig, doc,
                name === 'Map' ? vscode.CompletionItemKind.Class : vscode.CompletionItemKind.Keyword));
    }

    const items = [];
    for (const kw of KEYWORDS) {
        items.push(completion(kw, 'keyword', '', vscode.CompletionItemKind.Keyword));
    }
    for (const [name, sig, doc] of GLOBALS) {
        items.push(completion(name, sig, doc, vscode.CompletionItemKind.Function));
    }
    for (const [name, sig, doc] of CLASSES) {
        items.push(completion(name, sig, doc, vscode.CompletionItemKind.Class));
    }
    for (const [name, sig, doc] of CONSTANTS) {
        items.push(completion(name, sig, doc, vscode.CompletionItemKind.Constant));
    }
    return items;
}

function staticHover(document, position) {
    const range = document.getWordRangeAtPosition(position);
    if (!range) return null;
    const word = document.getText(range);

    // 光标前是否是 `.`（区分方法与全局）
    const lineStart = document.lineAt(position.line).text.slice(0, range.start.character);
    const isMethod = /\.\s*$/.test(lineStart);

    const entry = isMethod
        ? METHOD_MAP.get(word)
        : GLOBAL_MAP.get(word) || CLASS_MAP.get(word) || CONST_MAP.get(word);
    if (!entry) return null;
    return new vscode.Hover(`**${entry.sig}**\n\n${entry.doc}`);
}

module.exports = { staticCompletion, staticHover };
