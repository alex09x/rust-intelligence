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

    let (mut mr, mut ov, mut lc) = (MultiRootWorkspace::new(), OverlayManager::new(), LifecycleManager::new());
    let can = Arc::new(Mutex::new(HashSet::new()));
    assert!(mr.add_root(ws.clone()));
    let mut send = |msg: Value| {
        let mut out = Vec::new();
        dispatch_lsp_message(LspMessage::Client(msg), &mut out, &mut lc, &mut mr, &mut ov, &can, None).unwrap();
        response(&out)
    };
    send(json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "capabilities": {} } }));
    let uri = path_to_uri(lib.to_str().unwrap());
    send(json!({ "jsonrpc": "2.0", "method": "textDocument/didOpen", "params": { "textDocument": { "uri": uri, "languageId": "rust", "version": 1, "text": text } } }));

    // `finish`, declared on line 4 now, is called on line 3.
    let refs = send(json!({ "jsonrpc": "2.0", "id": 2, "method": "textDocument/references", "params": {
        "textDocument": { "uri": uri }, "position": { "line": 4, "character": 9 }, "context": { "includeDeclaration": true } } }));
    let lines: Vec<u64> = refs["result"].as_array().map(|a| a.iter().filter_map(|l| l["range"]["start"]["line"].as_u64()).collect()).unwrap_or_default();
    assert!(lines.contains(&4) && lines.contains(&3), "the declaration and the call, at their lines in the buffer: {refs}");
}
