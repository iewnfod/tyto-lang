//! LSP server（`tyto lsp`）：stdio 上的补全与悬停。
//!
//! 传输与生命周期交给 `lsp-server`（rust-analyzer 同款，同步线程模型）；
//! 消息体直接用 `serde_json::Value`——只用到协议的一小撮字段，
//! 不引入庞大的 `lsp-types` 类型库。
//!
//! 文档同步：全量（TextDocumentSyncKind::FULL），每次 didChange 收整份文本，
//! 对脚本规模的 .tyto 文件足够。
//!
//! 纪律：本进程 stdout **只能**走协议（lsp-server 接管）；调试输出一律
//! eprintln（stderr）或 `TYTO_LSP_LOG` 文件。

use std::collections::HashMap;
use std::io::Write;

use lsp_server::{Connection, Message, Notification, Request, Response};
use serde_json::{json, Value};

use crate::analysis::{self, CompleteItem, ItemKind};
use crate::{Lexer, Parser};

/// 入口：`tyto lsp`
pub fn run() -> ! {
    let (connection, io_threads) = Connection::stdio();

    // capabilities：全量同步 + 补全（`.` 触发）+ 悬停 + 跳转定义 + 语义着色
    //（lsp-server 的 initialize 会自动包一层 "capabilities"，这里只给内层）
    let semantic_types: Vec<&str> = crate::analysis::semantics::TOKEN_TYPES.to_vec();
    let capabilities = json!({
        "textDocumentSync": 1, // FULL
        "completionProvider": {
            "triggerCharacters": ["."],
            "resolveProvider": false,
        },
        "hoverProvider": true,
        "definitionProvider": true,
        "semanticTokensProvider": {
            "legend": {
                "tokenTypes": semantic_types,
                "tokenModifiers": [],
            },
            "full": true,
            "range": false,
        },
    });
    let init_params = match connection.initialize(capabilities) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("tyto lsp: initialize failed: {e}");
            std::process::exit(1);
        }
    };
    let _ = init_params;

    let server = ServerState {
        docs: HashMap::new(),
    };
    match main_loop(connection, server) {
        Ok(()) => {}
        Err(e) => eprintln!("tyto lsp: {e}"),
    }
    io_threads.join().ok();
    std::process::exit(0);
}

struct ServerState {
    /// uri → 当前全文（全量同步）
    docs: HashMap<String, String>,
}

fn main_loop(connection: Connection, mut state: ServerState) -> Result<(), String> {
    for msg in &connection.receiver {
        match msg {
            Message::Request(req) => {
                // shutdown 请求：true = 已处理并回包，循环返回（等 exit / 直接结束）
                if connection.handle_shutdown(&req).unwrap_or(false) {
                    return Ok(());
                }
                let resp = handle_request(&mut state, req);
                connection
                    .sender
                    .send(Message::Response(resp))
                    .map_err(|e| format!("send response: {e}"))?;
            }
            Message::Notification(not) => {
                if not.method == "exit" {
                    return Ok(());
                }
                // didOpen / didChange 之后推送诊断（语法错误 + 类型检查）
                if let Some(pub_not) = handle_notification(&mut state, not) {
                    connection
                        .sender
                        .send(Message::Notification(pub_not))
                        .map_err(|e| format!("send diagnostics: {e}"))?;
                }
            }
            // 本服务不发 server→client 请求，客户端响应忽略
            Message::Response(_) => {}
        }
    }
    Ok(())
}

fn handle_request(state: &mut ServerState, req: Request) -> Response {
    match req.method.as_str() {
        "textDocument/completion" => completion(state, &req),
        "textDocument/hover" => hover(state, &req),
        "textDocument/definition" => definition(state, &req),
        "textDocument/semanticTokens/full" => semantic_tokens_full(state, &req),
        _ => Response::new_err(
            req.id,
            lsp_server::ErrorCode::MethodNotFound as i32,
            format!("method not found: {}", req.method),
        ),
    }
}

fn handle_notification(state: &mut ServerState, not: Notification) -> Option<Notification> {
    match not.method.as_str() {
        "textDocument/didOpen" => {
            if let (Some(uri), Some(text)) = (
                not.params.pointer("/textDocument/uri").and_then(Value::as_str),
                not.params.pointer("/textDocument/text").and_then(Value::as_str),
            ) {
                state.docs.insert(uri.to_string(), text.to_string());
                return Some(publish_diagnostics(uri, text));
            }
            None
        }
        "textDocument/didChange" => {
            let Some(uri) = not
                .params
                .pointer("/textDocument/uri")
                .and_then(Value::as_str)
            else {
                return None;
            };
            // 全量同步：取最后一个 contentChanges 的 text
            if let Some(text) = not
                .params
                .pointer("/contentChanges")
                .and_then(Value::as_array)
                .and_then(|cs| cs.last())
                .and_then(|c| c.get("text"))
                .and_then(Value::as_str)
            {
                state.docs.insert(uri.to_string(), text.to_string());
                return Some(publish_diagnostics(uri, text));
            }
            None
        }
        "textDocument/didClose" => {
            if let Some(uri) = not
                .params
                .pointer("/textDocument/uri")
                .and_then(Value::as_str)
            {
                state.docs.remove(uri);
                // 关闭文档：清空诊断
                return Some(Notification::new(
                    "textDocument/publishDiagnostics".into(),
                    json!({ "uri": uri, "diagnostics": [] }),
                ));
            }
            None
        }
        _ => None,
    }
}

// ============ 诊断（语法错误 + 渐进类型检查） ============

/// 计算文档的全部诊断：词法 → 语法 → 类型（前者失败即止）
fn compute_diagnostics(text: &str) -> Vec<Value> {
    let map = analysis::tolerate::SourceMap::new(text);
    let mut items = Vec::new();

    let mut push = |span: crate::Span, sev: u8, msg: String, len: usize| {
        let (line, col) = map.from_span(span);
        items.push(json!({
            "range": {
                "start": { "line": line, "character": col },
                "end": { "line": line, "character": col + len },
            },
            "severity": sev,
            "source": "tyto",
            "message": msg,
        }));
    };

    let tokens = match Lexer::new(text).tokenize() {
        Ok(out) => out.tokens,
        Err(e) => {
            if let crate::RtError::Lex { span, message } = &e {
                push(*span, 1, format!("词法错误：{message}"), 1);
            }
            return items;
        }
    };
    let program = match Parser::new(tokens).parse_program() {
        Ok(p) => p,
        Err(e) => {
            if let crate::RtError::Parse { span, message } = &e {
                push(*span, 1, format!("语法错误：{message}"), 1);
            }
            return items;
        }
    };
    let out = crate::checker::check_program(&program);
    for d in out.diagnostics {
        let sev = match d.severity {
            crate::checker::diag::Severity::Error => 1,
            crate::checker::diag::Severity::Warning => 2,
        };
        push(d.span, sev, d.message, 1);
    }
    items
}

fn publish_diagnostics(uri: &str, text: &str) -> Notification {
    Notification::new(
        "textDocument/publishDiagnostics".into(),
        json!({ "uri": uri, "diagnostics": compute_diagnostics(text) }),
    )
}

/// 请求位置（0-based 行、UTF-16 列）
fn position(params: &Value) -> Option<(String, usize, usize)> {
    let uri = params.pointer("/textDocument/uri")?.as_str()?.to_string();
    let line = params.pointer("/position/line")?.as_u64()? as usize;
    let character = params.pointer("/position/character")?.as_u64()? as usize;
    Some((uri, line, character))
}

fn completion(state: &ServerState, req: &Request) -> Response {
    let Some((uri, line, character)) = position(&req.params) else {
        return Response::new_err(req.id.clone(), 0, "bad params".into());
    };
    let Some(text) = state.docs.get(&uri) else {
        return Response::new_ok(req.id.clone(), json!({ "isIncomplete": false, "items": [] }));
    };
    let items: Vec<Value> = analysis::complete(text, line, character)
        .iter()
        .map(completion_json)
        .collect();
    // 附加日志（可选文件）
    log_line(&format!("completion {}:{}:{} → {} items", uri, line, character, items.len()));
    Response::new_ok(req.id.clone(), json!({ "isIncomplete": false, "items": items }))
}

fn completion_json(item: &CompleteItem) -> Value {
    // CompletionItemKind 数值（协议常量）
    let kind = match item.kind {
        ItemKind::Keyword => 14,
        ItemKind::Function => 3,
        ItemKind::Class => 7,
        ItemKind::Constant => 21,
        ItemKind::Variable | ItemKind::Parameter => 6,
        ItemKind::Field => 5,
        ItemKind::Method => 2,
        ItemKind::Struct => 22,
        ItemKind::Interface => 8,
    };
    let mut v = json!({
        "label": item.label,
        "kind": kind,
        "detail": item.detail,
    });
    if !item.doc.is_empty() {
        v["documentation"] = json!({ "kind": "markdown", "value": item.doc });
    }
    v
}

fn hover(state: &ServerState, req: &Request) -> Response {
    let Some((uri, line, character)) = position(&req.params) else {
        return Response::new_err(req.id.clone(), 0, "bad params".into());
    };
    let Some(text) = state.docs.get(&uri) else {
        return Response::new_ok(req.id.clone(), Value::Null);
    };
    let result = analysis::hover(text, line, character).map(|h| {
        let mut md = format!("{}\n", h.signature);
        if !h.doc.is_empty() {
            md.push_str(&format!("\n{}", h.doc));
        }
        json!({ "contents": { "kind": "markdown", "value": md } })
    });
    Response::new_ok(req.id.clone(), result.unwrap_or(Value::Null))
}

/// 跳转定义：Location { uri, range }（uri 沿用请求的文档）
fn definition(state: &ServerState, req: &Request) -> Response {
    let Some((uri, line, character)) = position(&req.params) else {
        return Response::new_err(req.id.clone(), 0, "bad params".into());
    };
    let Some(text) = state.docs.get(&uri) else {
        return Response::new_ok(req.id.clone(), Value::Null);
    };
    let result = analysis::semantics::definition(text, line, character).map(|d| {
        json!({
            "uri": uri,
            "range": {
                "start": { "line": d.line, "character": d.col },
                "end": { "line": d.line, "character": d.col + d.len },
            }
        })
    });
    Response::new_ok(req.id.clone(), result.unwrap_or(Value::Null))
}

/// 语义着色（全量）：LSP 相对增量编码（deltaLine/deltaStartChar/length/type/modifiers）
fn semantic_tokens_full(state: &ServerState, req: &Request) -> Response {
    // 该请求不带 position，只读 uri
    let Some(uri) = req.params.pointer("/textDocument/uri").and_then(Value::as_str) else {
        return Response::new_err(req.id.clone(), 0, "bad params".into());
    };
    let empty = json!({ "data": [] });
    let Some(text) = state.docs.get(uri) else {
        return Response::new_ok(req.id.clone(), empty);
    };
    let toks = analysis::semantics::semantic_tokens(text);
    let mut data: Vec<u32> = Vec::with_capacity(toks.len() * 5);
    let mut prev_line = 0u32;
    let mut prev_col = 0u32;
    for t in &toks {
        let line = t.line as u32;
        let col = t.col as u32;
        let dline = line - prev_line;
        let dcol = if dline == 0 { col - prev_col } else { col };
        data.extend_from_slice(&[dline, dcol, t.len as u32, t.ty, 0]);
        prev_line = line;
        prev_col = col;
    }
    log_line(&format!("semanticTokens {} → {} tokens", uri, toks.len()));
    Response::new_ok(req.id.clone(), json!({ "data": data }))
}

/// 可选日志：TYTO_LSP_LOG=文件路径 时追加（调试用，绝不写 stdout）
fn log_line(s: &str) {
    if let Some(mut f) = std::env::var("TYTO_LSP_LOG")
        .ok()
        .and_then(|path| std::fs::OpenOptions::new().create(true).append(true).open(path).ok())
    {
        let _ = writeln!(f, "{s}");
    }
}
