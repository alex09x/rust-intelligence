/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! A file changed on disk after it was indexed and opened only then (a checkout, a pull, Zed reopening it) keeps the
//! index: the server maps the buffer through the source snapshot the indexer kept of the text it compiled.

use super::failed_build_tests::{index_opts, write_native_and_app};
use super::*;
use crate::indexer::driver::find_driver_binary;
use crate::indexer::orchestrate::try_run_indexing;
use crate::lsp::dispatch::dispatch_lsp_message;
use crate::lsp::lifecycle::LifecycleManager;
use crate::lsp::protocol::{path_to_uri, LspMessage};
use crate::lsp::workspace::MultiRootWorkspace;
use crate::overlay::OverlayManager;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

const LOADER: &str = "pub struct Loader { n: u32 }\nimpl Loader {\n    pub fn run(&mut self) -> u32 { self.finish(1) + native::native_api() }\n    fn finish(&mut self, x: u32) -> u32 { self.n + x }\n}\n";

fn response(out: &[u8]) -> Value {
    let s = String::from_utf8_lossy(out);
    s.split("Content-Length: ").filter_map(|part| part.find("\r\n\r\n").and_then(|i| serde_json::from_str::<Value>(part[i + 4..].trim()).ok())).last().unwrap_or(Value::Null)
}

/// A server driven through `dispatch_lsp_message`, opened on a workspace folder as an editor opens it.
struct Server {
    mr: MultiRootWorkspace,
    ov: OverlayManager,
    lc: LifecycleManager,
    can: Arc<Mutex<HashSet<Value>>>,
    next_id: u64,
}

impl Server {
    fn start(ws: &std::path::Path) -> Self {
        let mut s = Server { mr: MultiRootWorkspace::new(), ov: OverlayManager::new(), lc: LifecycleManager::new(), can: Arc::new(Mutex::new(HashSet::new())), next_id: 0 };
        let root = path_to_uri(ws.to_str().unwrap());
        s.send(json!({ "method": "initialize", "params": { "capabilities": {}, "rootUri": root, "workspaceFolders": [{ "uri": root, "name": "ws" }] } }));
        s.notify(json!({ "method": "initialized", "params": {} }));
        s
    }

    fn send(&mut self, mut msg: Value) -> Value {
        self.next_id += 1;
        msg["jsonrpc"] = json!("2.0");
        msg["id"] = json!(self.next_id);
        self.notify(msg)
    }

    fn notify(&mut self, mut msg: Value) -> Value {
        msg["jsonrpc"] = json!("2.0");
        let mut out = Vec::new();
        dispatch_lsp_message(LspMessage::Client(msg), &mut out, &mut self.lc, &mut self.mr, &mut self.ov, &self.can, None).unwrap();
        response(&out)
    }

    /// Opens `file` with its text on disk, as an editor does; its URI.
    fn open(&mut self, file: &std::path::Path) -> String {
        let uri = path_to_uri(file.to_str().unwrap());
        let text = fs::read_to_string(file).unwrap();
        self.notify(json!({ "method": "textDocument/didOpen", "params": { "textDocument": { "uri": uri, "languageId": "rust", "version": 1, "text": text } } }));
        uri
    }

    fn definition(&mut self, uri: &str, line: u32, character: u32) -> Value {
        self.send(json!({ "method": "textDocument/definition", "params": { "textDocument": { "uri": uri }, "position": { "line": line, "character": character } } }))["result"]
            .clone()
    }
}

/// After a jump into std the editor keeps that file open; the next jump into it lands as the first did. (The open
/// std buffer has no compiled text the workspace index records, and mapping a location through it gave nothing:
/// Cmd-click on `Vec::new` fell back to a list of every `new`.)
#[test]
fn test_a_jump_into_an_open_std_file_lands_as_into_a_closed_one() {
    let _lock = lock_indexer_test();
    let Some(driver) = find_driver_binary() else { return };
    let dir = new_test_dir("ridx_open_target");
    let (ws, cas) = (dir.0.join("ws"), dir.0.join("cas"));
    write_native_and_app(&ws, "pub fn make() -> Vec<String> {\n    let mut v: Vec<String> = Vec::new();\n    v.push(String::new());\n    v\n}\n");
    try_run_indexing(index_opts(&ws, &cas, &driver)).expect("index");
    let mut server = Server::start(&ws);
    let uri = server.open(&ws.join("app/src/lib.rs"));

    let first = server.definition(&uri, 1, 34); // `new` in `Vec::new()`
    assert!(!first.is_null(), "a jump into std");
    let Some(target) = first["uri"].as_str().and_then(|u| u.strip_prefix("file://")).map(std::path::PathBuf::from).filter(|p| p.is_file()) else {
        return; // no std sources on this host
    };
    server.open(&target);
    let again = server.definition(&uri, 1, 34);
    assert_eq!(again, first, "the same place, with its file open");
}

#[test]
fn test_a_file_changed_on_disk_before_it_is_opened_maps_onto_its_compiled_text() {
    let _lock = lock_indexer_test();
    let Some(driver) = find_driver_binary() else { return };
    let dir = new_test_dir("ridx_snapshot_open");
    let (ws, cas) = (dir.0.join("ws"), dir.0.join("cas"));
    write_native_and_app(&ws, LOADER);
    try_run_indexing(index_opts(&ws, &cas, &driver)).expect("index");

    // A line comes in above everything else on disk, with no reindex yet: each line of the index is one lower now.
    let lib = ws.join("app/src/lib.rs");
    let text = format!("// pulled in\n{LOADER}");
    fs::write(&lib, &text).unwrap();

    let mut server = Server::start(&ws);
    let uri = server.open(&lib);

    // `finish`, declared on line 4 now, is called on line 3.
    let refs = server.send(json!({ "jsonrpc": "2.0", "id": 2, "method": "textDocument/references", "params": {
        "textDocument": { "uri": uri }, "position": { "line": 4, "character": 9 }, "context": { "includeDeclaration": true } } }));
    let lines: Vec<u64> = refs["result"].as_array().map(|a| a.iter().filter_map(|l| l["range"]["start"]["line"].as_u64()).collect()).unwrap_or_default();
    assert!(lines.contains(&4) && lines.contains(&3), "the declaration and the call, at their lines in the buffer: {refs}");
}
