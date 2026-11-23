/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! `workspace/symbol` lists the standard library's and the dependencies' symbols when nothing of the workspace
//! matches, as rust-analyzer falls back to libraries (prod-code, issue #6: `Extend` listed nothing, so a builder named
//! `Extend` was offered over the prelude's trait).

use super::failed_build_tests::{index_opts, write_native_and_app};
use super::lsp_server::Server;
use super::*;
use crate::indexer::driver::find_driver_binary;
use crate::indexer::orchestrate::try_run_indexing;
use serde_json::json;

#[test]
fn test_workspace_symbol_falls_back_to_the_libraries() {
    let _lock = lock_indexer_test();
    let Some(driver) = find_driver_binary() else { return };
    let dir = new_test_dir("ridx_library_symbols");
    let (ws, cas) = (dir.0.join("ws"), dir.0.join("cas"));
    write_native_and_app(&ws, "pub struct Gauge;\npub fn read(v: &mut Vec<u8>) { v.extend([1]); }\n");
    try_run_indexing(index_opts(&ws, &cas, &driver)).expect("index");
    let mut server = Server::start(&ws);
    let mut symbols = |query: &str| -> Vec<(String, String)> {
        let res = server.send(json!({ "method": "workspace/symbol", "params": { "query": query } }));
        res["result"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .map(|s| (s["name"].as_str().unwrap().to_string(), s["location"]["uri"].as_str().unwrap().to_string()))
            .collect()
    };
    let extend = symbols("Extend");
    assert!(extend.iter().any(|(n, uri)| n == "Extend" && uri.ends_with("/iter/traits/collect.rs")), "core's `Extend`: {extend:?}");
    let gauge = symbols("Gauge");
    assert_eq!(gauge.len(), 1, "a workspace match alone, no library search: {gauge:?}");
    assert!(gauge[0].1.ends_with("/app/src/lib.rs"), "{gauge:?}");
}
