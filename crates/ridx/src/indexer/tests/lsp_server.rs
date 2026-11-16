/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! A server driven through `dispatch_lsp_message` over a real index, opened on a workspace folder as an editor opens
//! it: the tests of what an editor sees after indexing.

use crate::lsp::dispatch::dispatch_lsp_message;
use crate::lsp::lifecycle::LifecycleManager;
use crate::lsp::protocol::{path_to_uri, LspMessage};
use crate::lsp::workspace::MultiRootWorkspace;
use crate::overlay::OverlayManager;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};

fn response(out: &[u8]) -> Value {
    let s = String::from_utf8_lossy(out);
    s.split("Content-Length: ").filter_map(|part| part.find("\r\n\r\n").and_then(|i| serde_json::from_str::<Value>(part[i + 4..].trim()).ok())).last().unwrap_or(Value::Null)
}

pub(super) struct Server {
    mr: MultiRootWorkspace,
    ov: OverlayManager,
    lc: LifecycleManager,
    can: Arc<Mutex<HashSet<Value>>>,
    next_id: u64,
}

impl Server {
    pub(super) fn start(ws: &Path) -> Self {
        let mut s = Server { mr: MultiRootWorkspace::new(), ov: OverlayManager::new(), lc: LifecycleManager::new(), can: Arc::new(Mutex::new(HashSet::new())), next_id: 0 };
        let root = path_to_uri(ws.to_str().unwrap());
        s.send(json!({ "method": "initialize", "params": { "capabilities": {}, "rootUri": root, "workspaceFolders": [{ "uri": root, "name": "ws" }] } }));
        s.notify(json!({ "method": "initialized", "params": {} }));
        s
    }

    pub(super) fn send(&mut self, mut msg: Value) -> Value {
        self.next_id += 1;
        msg["jsonrpc"] = json!("2.0");
        msg["id"] = json!(self.next_id);
        self.notify(msg)
    }

    pub(super) fn notify(&mut self, mut msg: Value) -> Value {
        msg["jsonrpc"] = json!("2.0");
        let mut out = Vec::new();
        dispatch_lsp_message(LspMessage::Client(msg), &mut out, &mut self.lc, &mut self.mr, &mut self.ov, &self.can, None).unwrap();
        response(&out)
    }

    /// Opens `file` with its text on disk, as an editor does; its URI.
    pub(super) fn open(&mut self, file: &Path) -> String {
        let text = fs::read_to_string(file).unwrap();
        self.open_with(file, &text)
    }

    /// Opens `file` with the buffer text `text` (typed since the index was built); its URI.
    pub(super) fn open_with(&mut self, file: &Path, text: &str) -> String {
        let uri = path_to_uri(file.to_str().unwrap());
        self.notify(json!({ "method": "textDocument/didOpen", "params": { "textDocument": { "uri": uri, "languageId": "rust", "version": 1, "text": text } } }));
        uri
    }

    pub(super) fn definition(&mut self, uri: &str, line: u32, character: u32) -> Value {
        self.send(json!({ "method": "textDocument/definition", "params": { "textDocument": { "uri": uri }, "position": { "line": line, "character": character } } }))["result"]
            .clone()
    }

    /// The titles and actions offered at `line:character` (an empty range).
    pub(super) fn code_actions(&mut self, uri: &str, line: u32, character: u32) -> Vec<Value> {
        let at = json!({ "line": line, "character": character });
        let res = self.send(
            json!({ "method": "textDocument/codeAction", "params": { "textDocument": { "uri": uri }, "range": { "start": at, "end": at }, "context": { "diagnostics": [] } } }),
        );
        res["result"].as_array().cloned().unwrap_or_default()
    }

    /// The completion items at `line:character`.
    pub(super) fn completion(&mut self, uri: &str, line: u32, character: u32) -> Vec<Value> {
        let res = self.send(json!({ "method": "textDocument/completion", "params": { "textDocument": { "uri": uri }, "position": { "line": line, "character": character } } }));
        let r = &res["result"];
        r["items"].as_array().or_else(|| r.as_array()).cloned().unwrap_or_default()
    }
}
