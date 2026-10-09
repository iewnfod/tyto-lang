// 最小 LSP 客户端：spawn `tyto lsp`，stdio + Content-Length 分帧，
// 进程崩溃/退出时置 dead（由 activate 降级到静态模式）。

const vscode = require('vscode');
const { spawn } = require('child_process');

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

module.exports = { LspClient, mapCompletionItem };
