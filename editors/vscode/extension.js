// 扩展装配：启动 LSP 客户端、注册诊断/文档同步/补全/悬停/跳转/语义着色/运行命令。
// 数据表在 ./builtins.js，静态降级在 ./staticFallback.js，LSP 客户端在 ./lspClient.js，
// 运行命令在 ./commands.js。

const vscode = require('vscode');

const { staticCompletion, staticHover } = require('./staticFallback');
const { LspClient, mapCompletionItem } = require('./lspClient');
const { runFile } = require('./commands');

let client = null;

function docParams(document) {
    return { uri: document.uri.toString() };
}

function activate(context) {
    const outputChannel = vscode.window.createOutputChannel('Tyto LSP');

    // 启动语言服务器：tyto.serverPath 配置可覆盖（默认 PATH 上的 tyto）
    const serverPath = vscode.workspace.getConfiguration('tyto').get('serverPath') || 'tyto';
    client = new LspClient(serverPath, outputChannel);

    // 类型/语法诊断：publishDiagnostics → 编辑器红/黄波浪线（擦除语义：仅提示不阻塞运行）
    const diagnostics = vscode.languages.createDiagnosticCollection('tyto');
    const SEVERITY = {
        1: vscode.DiagnosticSeverity.Error,
        2: vscode.DiagnosticSeverity.Warning,
        3: vscode.DiagnosticSeverity.Information,
        4: vscode.DiagnosticSeverity.Hint,
    };
    client.onDiagnostics = (params) => {
        const items = (params && params.diagnostics) || [];
        diagnostics.set(vscode.Uri.parse(params.uri), items.map((d) => {
            const rng = d.range || { start: { line: 0, character: 0 }, end: { line: 0, character: 0 } };
            return new vscode.Diagnostic(
                new vscode.Range(
                    new vscode.Position(rng.start.line, rng.start.character),
                    new vscode.Position(rng.end.line, rng.end.character),
                ),
                d.message || '',
                SEVERITY[d.severity] || vscode.DiagnosticSeverity.Error,
            );
        }));
    };
    context.subscriptions.push(diagnostics);

    // 文档同步（全量）
    const sync = (document) => {
        if (!client || client.dead || document.languageId !== 'tyto' || !client.ready) return;
        client.notify('textDocument/didOpen', {
            textDocument: {
                uri: document.uri.toString(),
                languageId: 'tyto',
                version: document.version,
                text: document.getText(),
            },
        });
    };
    if (vscode.window.activeTextEditor) {
        sync(vscode.window.activeTextEditor.document);
    }
    context.subscriptions.push(
        vscode.workspace.onDidOpenTextDocument(sync),
        vscode.workspace.onDidChangeTextDocument((e) => {
            if (!client || client.dead || !client.ready) return;
            if (e.document.languageId !== 'tyto') return;
            client.notify('textDocument/didChange', {
                textDocument: { uri: e.document.uri.toString(), version: e.document.version },
                contentChanges: [{ text: e.document.getText() }],
            });
        }),
        vscode.workspace.onDidCloseTextDocument((doc) => {
            if (doc.languageId !== 'tyto') return;
            diagnostics.delete(doc.uri);
            if (!client || client.dead || !client.ready) return;
            client.notify('textDocument/didClose', { textDocument: { uri: doc.uri.toString() } });
        })
    );

    // 补全：服务器可用 → 转发；否则静态降级
    const provideCompletionItems = async (document, position) => {
        if (client && client.ready && !client.dead) {
            try {
                const result = await client.request('textDocument/completion', {
                    textDocument: { uri: document.uri.toString() },
                    position: { line: position.line, character: position.character },
                });
                const items = result && result.items ? result.items : result || [];
                return items.map(mapCompletionItem);
            } catch {
                // 超时/出错：落到静态降级
            }
        }
        return staticCompletion(document, position);
    };

    // 悬停：同上
    const provideHover = async (document, position) => {
        if (client && client.ready && !client.dead) {
            try {
                const result = await client.request('textDocument/hover', {
                    textDocument: { uri: document.uri.toString() },
                    position: { line: position.line, character: position.character },
                });
                if (result && result.contents && result.contents.value) {
                    return new vscode.Hover(result.contents.value);
                }
                return null;
            } catch {
                // 落到静态降级
            }
        }
        return staticHover(document, position);
    };

    // 跳转定义（Ctrl+点击）：无静态降级，服务不可用返回 null
    const provideDefinition = async (document, position) => {
        if (!client || !client.ready || client.dead) return null;
        try {
            const result = await client.request('textDocument/definition', {
                textDocument: { uri: document.uri.toString() },
                position: { line: position.line, character: position.character },
            });
            if (result && result.uri) {
                const { start, end } = result.range;
                return new vscode.Location(
                    vscode.Uri.parse(result.uri),
                    new vscode.Range(
                        new vscode.Position(start.line, start.character),
                        new vscode.Position(end.line, end.character)
                    )
                );
            }
            return null;
        } catch {
            return null;
        }
    };

    // 语义着色：等握手完成后注册（legend 从服务端 capabilities 读取，缺失用内置表）
    const SEMANTIC_LEGEND = [
        'variable', 'parameter', 'function', 'method', 'property',
        'struct', 'interface', 'class', 'namespace', 'type',
    ];
    const provideDocumentSemanticTokens = async (document) => {
        if (!client || !client.ready || client.dead) {
            return new vscode.SemanticTokens(new Uint32Array(0));
        }
        try {
            const result = await client.request('textDocument/semanticTokens/full', {
                textDocument: { uri: document.uri.toString() },
            });
            const data = (result && result.data) || [];
            return new vscode.SemanticTokens(new Uint32Array(data));
        } catch {
            return new vscode.SemanticTokens(new Uint32Array(0));
        }
    };
    client.initPromise.then(() => {
        if (client.dead) return; // 服务没起来：不注册，保持 TextMate 着色
        const legendTypes =
            (client.serverCapabilities &&
                client.serverCapabilities.semanticTokensProvider &&
                client.serverCapabilities.semanticTokensProvider.legend &&
                client.serverCapabilities.semanticTokensProvider.legend.tokenTypes) ||
            SEMANTIC_LEGEND;
        context.subscriptions.push(
            vscode.languages.registerDocumentSemanticTokensProvider(
                'tyto',
                { provideDocumentSemanticTokens },
                new vscode.SemanticTokensLegend(legendTypes)
            )
        );
    });

    context.subscriptions.push(
        outputChannel,
        vscode.languages.registerCompletionItemProvider(
            'tyto',
            { provideCompletionItems },
            '.'
        ),
        vscode.languages.registerHoverProvider('tyto', { provideHover }),
        vscode.languages.registerDefinitionProvider('tyto', { provideDefinition }),
        vscode.commands.registerCommand('tyto.runFile', runFile)
    );
}

function deactivate() {
    if (client && !client.dead) {
        try {
            client.notify('shutdown', null);
            client.notify('exit', null);
        } catch {
            // 进程已死则忽略
        }
    }
}

// 供脚本测试使用（scripts/lsp_client_test.js 用真实 tyto 二进制端到端驱动）
module.exports = { activate, deactivate, __test: { LspClient, mapCompletionItem } };
