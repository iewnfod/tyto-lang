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
    ['Map', 'new Map()', '保序哈希表；键限 number/string/bool/null；get/set/has/remove/len/keys/values'],
];

const CONSTANTS = [
    ['EMPTY', 'EMPTY', '哨兵值：空堆/栈/队列 pop()/peek() 的返回'],
    ['inf', 'inf', '正无穷（1 / 0）'],
    ['nan', 'nan', '非数（0 / 0）'],
    ['fs', 'fs.*', '文件命名空间：read_file / read_lines / write_file / append_file / exists / list_dir'],
    ['sys', 'sys.*', '系统命名空间：shell / get_env / args'],
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
