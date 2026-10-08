#!/usr/bin/env node
// 插件端 LspClient 端到端测试：stub 掉 vscode 模块，用真实 `tyto lsp` 驱动
// extension.js 里的客户端类，验证 initialize / didChange / 补全 / 悬停 / 超时降级。
//
// 用法：cargo build && node scripts/lsp_client_test.js [tyto 路径]

'use strict';
const path = require('path');
const Module = require('module');

// ---- vscode 桩 ----
const stub = {
    CompletionItem: class {
        constructor(label, kind) { this.label = label; this.kind = kind; }
    },
    CompletionItemKind: Object.fromEntries(
        ['Text', 'Method', 'Function', 'Field', 'Variable', 'Class', 'Interface', 'Keyword', 'Constant', 'Struct']
            .map((k, i) => [k, i + 1])
    ),
    MarkdownString: class {
        constructor(v) { this.value = v; }
    },
    Hover: class {
        constructor(v) { this.contents = v; }
    },
    Location: class {
        constructor(uri, range) { this.uri = uri; this.range = range; }
    },
    Position: class {
        constructor(line, character) { this.line = line; this.character = character; }
    },
    Range: class {
        constructor(start, end) { this.start = start; this.end = end; }
    },
    Uri: { parse: (s) => s },
    SemanticTokens: class {
        constructor(data) { this.data = data; }
    },
    SemanticTokensLegend: class {
        constructor(tokenTypes) { this.tokenTypes = tokenTypes; }
    },
    window: {
        createOutputChannel: () => ({
            appendLine() {},
            append() {},
        }),
        terminals: [],
        createTerminal: () => ({}),
        showWarningMessage() {},
        activeTextEditor: null,
    },
    workspace: {
        onDidOpenTextDocument() {},
        onDidChangeTextDocument() {},
        onDidCloseTextDocument() {},
        getConfiguration: () => ({ get: () => 'tyto' }),
    },
    languages: {
        registerCompletionItemProvider() {},
        registerHoverProvider() {},
        registerDefinitionProvider() {},
        registerDocumentSemanticTokensProvider() {},
    },
    commands: { registerCommand() {} },
};
const origLoad = Module._load;
Module._load = function (request, ...rest) {
    if (request === 'vscode') return stub;
    return origLoad.call(this, request, ...rest);
};

const { __test } = require(path.join(__dirname, '..', 'editors', 'vscode', 'extension.js'));
const { LspClient, mapCompletionItem } = __test;

const BIN = process.argv[2] || path.join(__dirname, '..', 'target', 'debug', 'tyto');

const TOKEN_TYPES = ['variable', 'parameter', 'function', 'method', 'property',
    'struct', 'interface', 'class', 'namespace'];

function decodeDeltas(data) {
    const toks = [];
    let line = 0, col = 0;
    for (let i = 0; i + 4 < data.length; i += 5) {
        const [dline, dcol, , ty] = data.slice(i, i + 5);
        if (dline === 0) col += dcol;
        else { line += dline; col = dcol; }
        toks.push([line, col, TOKEN_TYPES[ty]]);
    }
    return toks;
}
let failures = 0;
const check = (cond, label) => {
    console.log(`[${cond ? 'ok ' : 'FAIL'}] ${label}`);
    if (!cond) failures++;
};

async function main() {
    const client = new LspClient(BIN, { appendLine() {}, append() {} });
    await client.initPromise;
    check(client.ready && !client.dead, '客户端 initialize 握手成功');

    const uri = 'file:///tmp/client_test.tyto';
    client.notify('textDocument/didOpen', {
        textDocument: { uri, languageId: 'tyto', version: 1, text: 's = "hi"\nq = s.' },
    });

    // 成员补全：string 方法
    let r = await client.request('textDocument/completion', {
        textDocument: { uri }, position: { line: 1, character: 6 },
    });
    const labels = (r.items || []).map((i) => i.label);
    check(labels.includes('to_uppercase') && !labels.includes('push'), '成员补全按类型过滤');

    // 全局补全：变量带类型
    client.notify('textDocument/didChange', {
        textDocument: { uri, version: 2 },
        contentChanges: [{ text: 'n = 1\nfn = function(a: number) -> number {\n    return a\n}\n' }],
    });
    r = await client.request('textDocument/completion', {
        textDocument: { uri }, position: { line: 4, character: 0 },
    });
    const byLabel = Object.fromEntries((r.items || []).map((i) => [i.label, i]));
    check(byLabel.n && byLabel.n.detail === 'n: number', '全局补全变量带推断类型');
    check(byLabel.fn && byLabel.fn.detail.includes('-> number'), '全局补全函数带签名');

    // 映射到 vscode CompletionItem
    const mapped = (r.items || []).slice(0, 5).map(mapCompletionItem);
    check(mapped.every((m) => typeof m.label === 'string' && m.kind !== undefined), 'LSP → vscode 项映射');

    // 悬停
    r = await client.request('textDocument/hover', {
        textDocument: { uri }, position: { line: 0, character: 1 },
    });
    check(r && r.contents && r.contents.value.includes('number'), '悬停返回 markdown');

    // 跳转定义：变量 → 声明处 Location
    r = await client.request('textDocument/definition', {
        textDocument: { uri }, position: { line: 0, character: 1 },
    });
    check(r && r.uri === uri && r.range.start.line === 0 && r.range.start.character === 0,
        '跳转定义返回 Location');

    // 语义着色：capabilities legend + 增量数据
    const legend = client.serverCapabilities.semanticTokensProvider.legend.tokenTypes;
    check(Array.isArray(legend) && legend.includes('variable') && legend.includes('method'),
        'serverCapabilities 带 semanticTokens legend');
    r = await client.request('textDocument/semanticTokens/full', {
        textDocument: { uri },
    });
    const toks = decodeDeltas(r.data);
    check(toks.some((t) => t[2] === 'variable'), '语义 token 含 variable');
    check(toks.some((t) => t[2] === 'parameter'), '语义 token 含 parameter（a 的声明）');

    // 关停
    await client.request('shutdown', null);
    client.notify('exit', null);
    await new Promise((res) => setTimeout(res, 300));
    check(client.dead, 'exit 后客户端标记 dead');

    console.log();
    if (failures) {
        console.log(`失败 ${failures} 项`);
        process.exit(1);
    }
    console.log('全部通过 ✔');
}

main().catch((e) => {
    console.error('测试崩溃:', e);
    process.exit(1);
});
