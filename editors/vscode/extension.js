const vscode = require('vscode');
const { spawn } = require('child_process');

// ============ 标准库数据表（降级用：tyto lsp 不可用时保持基础体验） ============
// 与 Rust 端 src/analysis/builtins.rs 同源；完整类型推导见语言服务器。

const KEYWORDS = [
    'if', 'else', 'while', 'for', 'in', 'break', 'continue', 'return',
    'function', 'new', 'true', 'false', 'null', 'self',
    'struct', 'impl', 'interface', 'is', 'let', 'const',
];

// [名称, 签名, 文档]
const GLOBALS = [
    ['print', 'print(...args)', '输出，不换行；多参数以空格分隔'],
    ['println', 'println(...args)', '输出并换行；多参数以空格分隔'],
    ['input', 'input() → string | null', '读入一行；EOF 返回 null'],
    ['num', 'num(x) → number', '字符串/数字转数字；非法字符串报错'],
    ['str', 'str(x) → string', '任意值转字符串'],
    ['len', 'len(x) → number', '长度：数组 / 字符串 / 对象 / Map / 堆 / 栈 / 队列'],
    ['type', 'type(x) → string', '类型名：number/string/bool/null/array/object/map/maxheap/minheap/function'],
    ['has', 'has(obj, "field") → bool', '对象是否有某字段（Map 请用 .contains_key() 方法）'],
    ['floor', 'floor(x) → number', '向下取整'],
    ['ceil', 'ceil(x) → number', '向上取整'],
    ['round', 'round(x) → number', '四舍五入（half away from zero）'],
    ['abs', 'abs(x) → number', '绝对值'],
    ['sqrt', 'sqrt(x) → number', '平方根'],
    ['pow', 'pow(a, b) → number', '幂'],
    ['min', 'min(...) 或 min(arr) → number', '最小值'],
    ['max', 'max(...) 或 max(arr) → number', '最大值'],
];

const CLASSES = [
    ['MaxHeap', 'new MaxHeap() / new MaxHeap(arr)', '最大堆（Rust BinaryHeap 原生实现）；push/pop/peek/len/is_empty，空堆 pop/peek 返回 EMPTY'],
    ['MinHeap', 'new MinHeap() / new MinHeap(arr)', '最小堆；push/pop/peek/len/is_empty，空堆 pop/peek 返回 EMPTY'],
    ['Stack', 'new Stack() / new Stack(arr)', '栈（Vec，LIFO）；push/pop/peek/len/is_empty，空栈 pop/peek 返回 EMPTY；数组顺序 = 底→顶'],
    ['Queue', 'new Queue() / new Queue(arr)', '队列（VecDeque，FIFO）；push_back/pop_front/front/back/len/is_empty，空队列取值返回 EMPTY；数组顺序 = 队头→队尾'],
    ['Map', 'new Map()', '保序哈希表；键限 number/string/bool/null；get/insert/contains_key/remove/len/keys/values'],
];

const CONSTANTS = [
    ['EMPTY', 'EMPTY', '哨兵值：空堆/栈/队列 pop()/peek() 的返回'],
    ['inf', 'inf', '正无穷（1 / 0）'],
    ['nan', 'nan', '非数（0 / 0）'],
    ['fs', 'fs.*', '文件命名空间：read_file / read_lines / write_file / append_file / exists / list_dir'],
    ['sys', 'sys.*', '系统命名空间：shell / get_env / args'],
];

// 类型标注可用名（降级用；与 Rust 端 src/analysis/builtins.rs 的 TYPES 同源）
const TYPES = [
    ['number', 'number', '数字类型'],
    ['string', 'string', '字符串类型'],
    ['bool', 'bool', '布尔类型（也接受 boolean 写法）'],
    ['array', 'array', '数组类型；元素类型可加 `[]` 后缀标注，如 number[]'],
    ['map', 'map', '映射类型'],
    ['object', 'object', '对象字面量类型'],
    ['function', 'function', '函数类型'],
    ['any', 'any', '任意类型（标注缺省值）'],
    ['Array', 'Array', 'array 的别名写法'],
    ['Map', 'Map<K, V>', '保序哈希表类型，如 Map<string, number>'],
];

// 无类型信息：`.` 后给出所有原生方法，detail 标注适用类型
const METHODS = [
    ['push', '.push(x)', 'array（多参数，返回新长度）/ stack / maxheap / minheap（仅数字）'],
    ['pop', '.pop()', 'array（空→null）/ stack（空→EMPTY）/ maxheap / minheap（空→EMPTY）'],
    ['peek', '.peek()', 'stack 栈顶 / maxheap / minheap 堆顶；空返回 EMPTY'],
    ['push_back', '.push_back(x)', 'queue 入队（队尾）'],
    ['pop_front', '.pop_front()', 'queue 出队（队头）；空返回 EMPTY'],
    ['front', '.front()', 'queue 队头（不移除）；空返回 EMPTY'],
    ['back', '.back()', 'queue 队尾（不移除）；空返回 EMPTY'],
    ['len', '.len()', 'array / string / map / maxheap / minheap / stack / queue'],
    ['is_empty', '.is_empty()', 'array / string / map / maxheap / minheap / stack / queue'],
    // 命名空间 fs.* / sys.*（fs.foo() 形式调用）
    ['read_file', 'fs.read_file(path) → string', '整文件读入；不存在/非 UTF-8 报错'],
    ['read_lines', 'fs.read_lines(path) → array', '按行读入（去行尾换行）'],
    ['write_file', 'fs.write_file(path, content)', '覆盖写；父目录必须存在'],
    ['append_file', 'fs.append_file(path, content)', '追加写；不存在则创建'],
    ['exists', 'fs.exists(path) → bool', '文件/目录存在性'],
    ['list_dir', 'fs.list_dir(path) → array', '目录条目名（排序）'],
    ['shell', 'sys.shell(cmd) → {status, stdout, stderr}', 'sh -c 执行；非零退出不报错；输出原样'],
    ['get_env', 'sys.get_env(name) → string | null', '环境变量；未设置返回 null'],
    ['args', 'sys.args() → array', '脚本命令行参数'],
    ['contains', '.contains(x)', 'array / string 是否包含'],
    ['index_of', '.index_of(x)', 'array / string 首次出现的下标；未找到 -1'],
    ['join', '.join(sep?)', 'array 连接为字符串；默认 ","'],
    ['sort', '.sort() / .sort(f)', 'array 就地排序并返回自身；比较器 f(a,b)→number，负数在前'],
    ['reverse', '.reverse()', 'array 就地反转并返回自身'],
    ['slice', '.slice(start, end?)', 'array 子数组；负下标从末尾数'],
    ['map', '.map(f)', 'array → 新数组'],
    ['filter', '.filter(f)', 'array → 满足条件的新数组'],
    ['fold', '.fold(init, f)', 'array 折叠：f(acc, x)'],
    ['split', '.split(sep)', 'string 按分隔符拆为数组；sep 为空串按字符拆'],
    ['trim', '.trim()', 'string 去两端空白'],
    ['starts_with', '.starts_with(s)', 'string 前缀'],
    ['ends_with', '.ends_with(s)', 'string 后缀'],
    ['to_uppercase', '.to_uppercase()', 'string 转大写'],
    ['to_lowercase', '.to_lowercase()', 'string 转小写'],
    ['sub', '.sub(start, end?)', 'string 子串；负下标从末尾数'],
    ['replace', '.replace(old, new)', 'string 替换全部匹配'],
    ['chars', '.chars()', 'string → 单字符数组'],
    ['get', '.get(k)', 'map 取值；缺失返回 null'],
    ['insert', '.insert(k, v)', 'map 写入；返回自身可链式'],
    ['contains_key', '.contains_key(k)', 'map 是否有键'],
    ['remove', '.remove(k)', 'map 删除；返回是否删除'],
    ['keys', '.keys()', 'map → 键数组（保序）'],
    ['values', '.values()', 'map → 值数组'],
];

const GLOBAL_MAP = new Map(GLOBALS.map(([k, sig, doc]) => [k, { sig, doc }]));
const CLASS_MAP = new Map(CLASSES.map(([k, sig, doc]) => [k, { sig, doc }]));
const CONST_MAP = new Map(CONSTANTS.map(([k, sig, doc]) => [k, { sig, doc }]));
const METHOD_MAP = new Map(METHODS.map(([k, sig, doc]) => [k, { sig, doc }]));

// ============ 静态降级实现（原版逻辑） ============

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

// ============ 最小 LSP 客户端（tyto lsp，stdio + Content-Length 分帧） ============

const REQUEST_TIMEOUT_MS = 5000;

class LspClient {
    constructor(command, outputChannel) {
        this.pending = new Map(); // id → { resolve, reject, timer }
        this.nextId = 1;
        this.ready = false;
        this.dead = false;
        this.outputChannel = outputChannel;

        let child;
        try {
            child = spawn(command, ['lsp'], { stdio: ['pipe', 'pipe', 'pipe'] });
        } catch (e) {
            this.dead = true;
            return;
        }
        this.child = child;

        child.on('error', (e) => {
            this.outputChannel.appendLine(`无法启动语言服务器（${command} lsp）：${e.message}`);
            this.outputChannel.appendLine('补全/悬停已降级为静态模式。可设置 tyto.serverPath 指定 tyto 路径。');
            this.failAll(new Error('server error'));
            this.dead = true;
        });
        child.on('exit', (code) => {
            if (!this.dead) {
                this.outputChannel.appendLine(`语言服务器已退出（code ${code}），降级为静态模式。`);
            }
            this.failAll(new Error('server exited'));
            this.dead = true;
        });
        child.stderr.on('data', (d) => {
            this.outputChannel.append(d.toString());
        });

        // 分帧读：累积缓冲，解析 Content-Length 头 + JSON 体
        this.buffer = Buffer.alloc(0);
        child.stdout.on('data', (chunk) => {
            this.buffer = Buffer.concat([this.buffer, chunk]);
            this.drain();
        });

        this.initPromise = this.initialize();
    }

    initialize() {
        return this.request('initialize', {
            processId: process.pid,
            rootUri: null,
            capabilities: {},
        }).then((result) => {
            this.serverCapabilities = (result && result.capabilities) || {};
            this.notify('initialized', {});
            this.ready = true;
            this.outputChannel.appendLine(`语言服务器已就绪（${this.child.spawnargs.join(' ')}）`);
        });
    }

    drain() {
        for (;;) {
            const headerEnd = this.buffer.indexOf('\r\n\r\n');
            if (headerEnd < 0) return;
            const header = this.buffer.slice(0, headerEnd).toString();
            const m = /Content-Length:\s*(\d+)/i.exec(header);
            if (!m) {
                // 协议流损坏：丢弃缓冲，防止永久卡死
                this.buffer = Buffer.alloc(0);
                return;
            }
            const length = parseInt(m[1], 10);
            const bodyStart = headerEnd + 4;
            if (this.buffer.length < bodyStart + length) return; // 等待更多数据
            const body = this.buffer.slice(bodyStart, bodyStart + length);
            this.buffer = this.buffer.slice(bodyStart + length);
            let msg;
            try {
                msg = JSON.parse(body.toString());
            } catch {
                continue;
            }
            this.dispatch(msg);
        }
    }

    dispatch(msg) {
        if (msg.id !== undefined && (msg.result !== undefined || msg.error !== undefined)) {
            const entry = this.pending.get(msg.id);
            if (!entry) return;
            this.pending.delete(msg.id);
            clearTimeout(entry.timer);
            if (msg.error) {
                entry.reject(new Error(msg.error.message || 'LSP error'));
            } else {
                entry.resolve(msg.result);
            }
        }
        // 服务端主动通知：诊断推送给回调（activate 里映射到编辑器），日志仅展示
        if (msg.method === 'textDocument/publishDiagnostics') {
            if (this.onDiagnostics) this.onDiagnostics(msg.params);
            return;
        }
        if (msg.method === 'window/logMessage' && msg.params && msg.params.message) {
            this.outputChannel.appendLine(msg.params.message);
        }
    }

    failAll(err) {
        for (const [, entry] of this.pending) {
            clearTimeout(entry.timer);
            entry.reject(err);
        }
        this.pending.clear();
    }

    request(method, params) {
        if (this.dead) return Promise.reject(new Error('server dead'));
        const id = this.nextId++;
        const body = JSON.stringify({ jsonrpc: '2.0', id, method, params });
        return new Promise((resolve, reject) => {
            const timer = setTimeout(() => {
                this.pending.delete(id);
                reject(new Error(`LSP request timeout: ${method}`));
            }, REQUEST_TIMEOUT_MS);
            this.pending.set(id, { resolve, reject, timer });
            this.send(body);
        });
    }

    notify(method, params) {
        if (this.dead) return;
        this.send(JSON.stringify({ jsonrpc: '2.0', method, params }));
    }

    send(body) {
        this.child.stdin.write(`Content-Length: ${Buffer.byteLength(body)}\r\n\r\n${body}`);
    }
}

// LSP CompletionItemKind 数值 → vscode 枚举
const LSP_KIND = {
    1: vscode.CompletionItemKind.Text,
    2: vscode.CompletionItemKind.Method,
    3: vscode.CompletionItemKind.Function,
    5: vscode.CompletionItemKind.Field,
    6: vscode.CompletionItemKind.Variable,
    7: vscode.CompletionItemKind.Class,
    8: vscode.CompletionItemKind.Interface,
    14: vscode.CompletionItemKind.Keyword,
    21: vscode.CompletionItemKind.Constant,
    22: vscode.CompletionItemKind.Struct,
};

function mapCompletionItem(item) {
    const mapped = new vscode.CompletionItem(item.label, LSP_KIND[item.kind] || vscode.CompletionItemKind.Text);
    if (item.detail) mapped.detail = item.detail;
    if (item.documentation) {
        mapped.documentation = new vscode.MarkdownString(
            typeof item.documentation === 'string'
                ? item.documentation
                : item.documentation.value
        );
    }
    return mapped;
}

// ============ 运行当前文件 ============

function getTerminal() {
    const existing = vscode.window.terminals.find((t) => t.name === 'tyto-lang');
    if (existing) return existing;
    return vscode.window.createTerminal('tyto-lang');
}

function runFile() {
    const editor = vscode.window.activeTextEditor;
    if (!editor) {
        vscode.window.showWarningMessage('没有打开的编辑器');
        return;
    }
    if (editor.document.languageId !== 'tyto') {
        vscode.window.showWarningMessage('当前文件不是 tyto-lang 源文件');
        return;
    }
    editor.document.save().then((saved) => {
        if (!saved) return;
        const term = getTerminal();
        term.show(true);
        term.sendText(`tyto "${editor.document.fileName}"`);
    });
}

// ============ 激活 ============

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
