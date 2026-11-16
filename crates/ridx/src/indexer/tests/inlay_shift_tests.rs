/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Parameter-name hints follow the edit: in Zed on shop, after a line above moved the calls of a test down by one,
//! `name:` / `price:` showed on the line below the call they belong to (the index was asked at the live line) and the
//! call itself had none.

use super::failed_build_tests::{index_opts, write_native_and_app};
use super::lsp_server::Server;
use super::*;
use crate::indexer::driver::find_driver_binary;
use crate::indexer::orchestrate::try_run_indexing;
use serde_json::json;

const APP: &str = "pub fn price(name: &str, cents: u64) -> u64 {\n    name.len() as u64 + cents\n}\npub fn run() -> u64 {\n    let a = price(\"tea\", 1000);\n    let b = 7;\n    a + b + native::native_api() as u64\n}\n";

#[test]
fn test_parameter_hints_follow_lines_added_above() {
    let _lock = lock_indexer_test();
    let Some(driver) = find_driver_binary() else { return };
    let dir = new_test_dir("ridx_inlay_shift");
    let (ws, cas) = (dir.0.join("ws"), dir.0.join("cas"));
    write_native_and_app(&ws, APP);
    try_run_indexing(index_opts(&ws, &cas, &driver)).expect("index");
    let mut server = Server::start(&ws);
    let edited = format!("// a line typed above\n{APP}");
    let uri = server.open_with(&ws.join("app/src/lib.rs"), &edited);
    let res = server.send(json!({ "method": "textDocument/inlayHint", "params": { "textDocument": { "uri": uri }, "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 9, "character": 0 } } } }));
    let params: Vec<(u64, String)> = res["result"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|h| h["kind"] == 2)
        .map(|h| (h["position"]["line"].as_u64().unwrap(), h["label"].as_str().unwrap_or_default().to_string()))
        .collect();
    let call_line = edited.lines().position(|l| l.contains("price(\"tea\"")).unwrap() as u64;
    assert_eq!(params, [(call_line, "name:".to_string()), (call_line, "cents:".to_string())], "on the call's live line: {res}");
}
