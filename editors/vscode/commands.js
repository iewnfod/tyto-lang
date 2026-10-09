// 运行当前文件：保存后在集成终端执行 `tyto "<file>"`（复用同名终端）。

const vscode = require('vscode');

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

module.exports = { runFile };
