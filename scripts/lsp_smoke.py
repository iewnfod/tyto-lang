#!/usr/bin/env python3
"""tyto lsp 协议冒烟测试。

用真实 LSP 帧（Content-Length 分帧的 JSON-RPC over stdio）驱动 `tyto lsp`，
断言 initialize 握手、补全（成员过滤 / 变量类型 / UTF-16 位置）与悬停。

用法：
    cargo build
    python3 scripts/lsp_smoke.py [tyto 二进制路径]
"""

import json
import subprocess
import sys
import os

BIN = sys.argv[1] if len(sys.argv) > 1 else os.path.join(os.path.dirname(__file__), "..", "target", "debug", "tyto")

DOC_MEMBER = 's = "hello"\nm = new Map()\nobj = {\n    n: 1,\n}\np = new Point(1, 2)\nstruct Point {\n    x,\n    y,\n}\nimpl Point {\n    function len() -> number {\n        return 1\n    }\n}\nq = s.'

failures = []


def check(cond, label):
    status = "ok " if cond else "FAIL"
    print(f"[{status}] {label}")
    if not cond:
        failures.append(label)


class Client:
    def __init__(self):
        self.proc = subprocess.Popen(
            [BIN, "lsp"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        self.next_id = 1

    def send(self, method, params, notification=False):
        body = {"jsonrpc": "2.0", "method": method, "params": params}
        if not notification:
            body["id"] = self.next_id
            self.next_id += 1
        data = json.dumps(body).encode()
        self.proc.stdin.write(f"Content-Length: {len(data)}\r\n\r\n".encode() + data)
        self.proc.stdin.flush()
        return body.get("id")

    def read_message(self):
        headers = {}
        while True:
            line = self.proc.stdout.readline()
            if not line:
                raise EOFError("server closed stdout")
            line = line.decode().strip()
            if not line:
                break
            k, _, v = line.partition(":")
            headers[k.strip().lower()] = v.strip()
        length = int(headers["content-length"])
        return json.loads(self.proc.stdout.read(length))

    def wait_response(self, req_id, timeout_deadline=None):
        while True:
            msg = self.read_message()
            if msg.get("id") == req_id and ("result" in msg or "error" in msg):
                return msg


def uri_of(name):
    return f"file:///tmp/{name}.tyto"


def main():
    c = Client()

    # ---- initialize 握手 ----
    rid = c.send("initialize", {"processId": None, "rootUri": None, "capabilities": {}})
    resp = c.wait_response(rid)
    caps = resp.get("result", {}).get("capabilities", {})
    check(caps.get("textDocumentSync") == 1, "initialize: 全量同步 textDocumentSync=1")
    check(caps.get("completionProvider", {}).get("triggerCharacters") == ["."], "initialize: 补全触发字符 [.]")
    check(caps.get("hoverProvider") is True, "initialize: hoverProvider=true")
    c.send("initialized", {}, notification=True)

    # ---- 成员补全：字符串方法过滤 ----
    doc1 = 's = "hello"\nq = s.'
    c.send("textDocument/didOpen", notification=True, params={"textDocument": {"uri": uri_of("m1"), "languageId": "tyto", "version": 1, "text": doc1}})
    rid = c.send("textDocument/completion", {"textDocument": {"uri": uri_of("m1")}, "position": {"line": 1, "character": 6}})
    items = c.wait_response(rid)["result"]["items"]
    labels = [i["label"] for i in items]
    check("to_uppercase" in labels, "成员补全：string 方法在列")
    check("push" not in labels and "map" not in labels, "成员补全：array/堆方法被过滤")

    # ---- 全局补全：变量带类型 + 用户函数签名 + struct ----
    doc2 = "n = 42\nfunction add(a: number, b: number) -> number {\n    return a + b\n}\nstruct Point {\n    x,\n}\n"
    c.send("textDocument/didOpen", notification=True, params={"textDocument": {"uri": uri_of("m2"), "languageId": "tyto", "version": 1, "text": doc2}})
    rid = c.send("textDocument/completion", {"textDocument": {"uri": uri_of("m2")}, "position": {"line": 5, "character": 0}})
    items = c.wait_response(rid)["result"]["items"]
    by_label = {i["label"]: i for i in items}
    check(by_label.get("n", {}).get("detail") == "n: number", "全局补全：变量 n 显示 number")
    check(by_label.get("add", {}).get("detail") == "function add(a: number, b: number) -> number", "全局补全：函数完整签名")
    check("Point" in by_label, "全局补全：struct 名")
    check(by_label.get("println", {}).get("kind") == 3, "全局补全：内置函数 kind=Function")

    # ---- didChange 全量同步后类型更新 ----
    c.send("textDocument/didChange", notification=True, params={
        "textDocument": {"uri": uri_of("m2"), "version": 2},
        "contentChanges": [{"text": doc2 + "n = \"now string\"\n\n"}],
    })
    rid = c.send("textDocument/completion", {"textDocument": {"uri": uri_of("m2")}, "position": {"line": 8, "character": 0}})
    items = c.wait_response(rid)["result"]["items"]
    by_label = {i["label"]: i for i in items}
    check(by_label.get("n", {}).get("detail") == "n: string", "didChange 后重赋值类型更新为 string")

    # ---- 悬停：变量类型（引用处）----
    rid = c.send("textDocument/hover", {"textDocument": {"uri": uri_of("m2")}, "position": {"line": 0, "character": 1}})
    result = c.wait_response(rid)["result"]
    check(result is not None and "number" in result["contents"]["value"], "悬停：n 显示 number")

    # ---- 悬停：定义点（`n = 42` 的 n 上）----
    rid = c.send("textDocument/hover", {"textDocument": {"uri": uri_of("m2")}, "position": {"line": 0, "character": 0}})
    result = c.wait_response(rid)["result"]
    check(result is not None and "n: number" in result["contents"]["value"], "悬停：定义点 n 显示 number")

    # ---- UTF-16 位置：中文 + emoji 行内补全 ----
    doc3 = '// 注释 🎉\ns = "x"\ns2 = 1\n你好§marker\n'
    # 行 3「你好§marker」——把光标放在 marker 前（UTF-16 列 2：两个汉字各 1 单元）
    c.send("textDocument/didOpen", notification=True, params={"textDocument": {"uri": uri_of("m3"), "languageId": "tyto", "version": 1, "text": doc3}})
    rid = c.send("textDocument/completion", {"textDocument": {"uri": uri_of("m3")}, "position": {"line": 3, "character": 2}})
    items = c.wait_response(rid)["result"]["items"]
    labels = {i["label"] for i in items}
    check("s" in labels and "s2" in labels, "UTF-16 列换算：中文行后仍能拿到全部绑定")

    # ---- 关闭文档后补全为空 ----
    c.send("textDocument/didClose", notification=True, params={"textDocument": {"uri": uri_of("m1")}})
    rid = c.send("textDocument/completion", {"textDocument": {"uri": uri_of("m1")}, "position": {"line": 0, "character": 0}})
    items = c.wait_response(rid)["result"]["items"]
    check(items == [], "didClose 后文档无补全")

    # ---- 未知方法 → MethodNotFound，但不死 ----
    rid = c.send("textDocument/definition", {"textDocument": {"uri": uri_of("m2")}, "position": {"line": 0, "character": 0}})
    resp = c.wait_response(rid)
    check("error" in resp, "未知方法返回错误响应")

    # ---- shutdown / exit ----
    rid = c.send("shutdown", None)
    resp = c.wait_response(rid)
    check(resp.get("result") is None, "shutdown 返回 null")
    c.send("exit", None, notification=True)
    code = c.proc.wait(timeout=5)
    check(code == 0, f"exit 退出码 0（实际 {code}）")

    print()
    if failures:
        print(f"失败 {len(failures)} 项: {failures}")
        sys.exit(1)
    print("全部通过 ✔")


if __name__ == "__main__":
    main()
