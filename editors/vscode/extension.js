const vscode = require('vscode');

// ============ 标准库数据表（补全与悬停共用） ============

const KEYWORDS = [
    'if', 'else', 'while', 'for', 'in', 'break', 'continue', 'return',
    'function', 'new', 'true', 'false', 'null', 'self',
];

// [名称, 签名, 文档]
const GLOBALS = [
    ['print', 'print(...args)', '输出，不换行；多参数以空格分隔'],
    ['println', 'println(...args)', '输出并换行；多参数以空格分隔'],
    ['input', 'input() → string | null', '读入一行；EOF 返回 null'],
    ['num', 'num(x) → number', '字符串/数字转数字；非法字符串报错'],
    ['str', 'str(x) → string', '任意值转字符串'],
    ['len', 'len(x) → number', '长度：数组 / 字符串 / 对象 / Map / 堆'],
    ['type', 'type(x) → string', '类型名：number/string/bool/null/array/object/map/maxheap/minheap/function'],
    ['has', 'has(obj, "field") → bool', '对象是否有某字段（Map 请用 .has() 方法）'],
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
    ['MaxHeap', 'new MaxHeap() / new MaxHeap(arr)', '最大堆（Rust BinaryHeap 原生实现）；push/pop/peek/len/isEmpty，空堆 pop/peek 返回 EMPTY'],
    ['MinHeap', 'new MinHeap() / new MinHeap(arr)', '最小堆；push/pop/peek/len/isEmpty，空堆 pop/peek 返回 EMPTY'],
    ['Map', 'new Map()', '保序哈希表；键限 number/string/bool/null；get/set/has/remove/len/keys/values'],
];

const CONSTANTS = [
    ['EMPTY', 'EMPTY', '哨兵值：空堆 pop()/peek() 的返回'],
    ['inf', 'inf', '正无穷（1 / 0）'],
    ['nan', 'nan', '非数（0 / 0）'],
];

// 无类型信息：`.` 后给出所有原生方法，detail 标注适用类型
const METHODS = [
    ['push', '.push(x)', 'array（多参数，返回新长度）/ maxheap / minheap（仅数字）'],
    ['pop', '.pop()', 'array（空→null）/ maxheap / minheap（空→EMPTY）'],
    ['peek', '.peek()', 'maxheap / minheap 堆顶；空返回 EMPTY'],
    ['len', '.len()', 'array / string / map / maxheap / minheap'],
    ['isEmpty', '.isEmpty()', 'array / string / map / maxheap / minheap'],
    ['contains', '.contains(x)', 'array / string 是否包含'],
    ['indexOf', '.indexOf(x)', 'array / string 首次出现的下标；未找到 -1'],
    ['join', '.join(sep?)', 'array 连接为字符串；默认 ","'],
    ['sort', '.sort() / .sort(f)', 'array 就地排序并返回自身；比较器 f(a,b)→number，负数在前'],
    ['reverse', '.reverse()', 'array 就地反转并返回自身'],
    ['slice', '.slice(start, end?)', 'array 子数组；负下标从末尾数'],
    ['map', '.map(f)', 'array → 新数组'],
    ['filter', '.filter(f)', 'array → 满足条件的新数组'],
    ['reduce', '.reduce(f, init)', 'array 折叠：f(acc, x)'],
    ['split', '.split(sep)', 'string 按分隔符拆为数组；sep 为空串按字符拆'],
    ['trim', '.trim()', 'string 去两端空白'],
    ['startsWith', '.startsWith(s)', 'string 前缀'],
    ['endsWith', '.endsWith(s)', 'string 后缀'],
    ['toUpper', '.toUpper()', 'string 转大写'],
    ['toLower', '.toLower()', 'string 转小写'],
    ['sub', '.sub(start, end?)', 'string 子串；负下标从末尾数'],
    ['replace', '.replace(old, new)', 'string 替换全部匹配'],
    ['chars', '.chars()', 'string → 单字符数组'],
    ['get', '.get(k)', 'map 取值；缺失返回 null'],
    ['set', '.set(k, v)', 'map 写入；返回自身可链式'],
    ['has', '.has(k)', 'map 是否有键'],
    ['remove', '.remove(k)', 'map 删除；返回是否删除'],
    ['keys', '.keys()', 'map → 键数组（保序）'],
    ['values', '.values()', 'map → 值数组'],
];

const GLOBAL_MAP = new Map(GLOBALS.map(([k, sig, doc]) => [k, { sig, doc }]));
const CLASS_MAP = new Map(CLASSES.map(([k, sig, doc]) => [k, { sig, doc }]));
const CONST_MAP = new Map(CONSTANTS.map(([k, sig, doc]) => [k, { sig, doc }]));
const METHOD_MAP = new Map(METHODS.map(([k, sig, doc]) => [k, { sig, doc }]));

// ============ 补全 ============

function completion(name, sig, doc, kind) {
    const item = new vscode.CompletionItem(name, kind);
    item.detail = sig;
    item.documentation = new vscode.MarkdownString(doc);
    return item;
}

function provideCompletionItems(document, position) {
    const prefix = document.lineAt(position).text.slice(0, position.character);
    const afterDot = /\.\s*\w*$/.test(prefix);

    if (afterDot) {
        return METHODS.map(([name, sig, doc]) =>
            completion(name, sig, doc, vscode.CompletionItemKind.Method));
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

// ============ 悬停文档 ============

function provideHover(document, position) {
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

function activate(context) {
    context.subscriptions.push(
        vscode.languages.registerCompletionItemProvider(
            'tyto',
            { provideCompletionItems },
            '.'
        ),
        vscode.languages.registerHoverProvider('tyto', { provideHover }),
        vscode.commands.registerCommand('tyto.runFile', runFile)
    );
}

function deactivate() {}

module.exports = { activate, deactivate };
