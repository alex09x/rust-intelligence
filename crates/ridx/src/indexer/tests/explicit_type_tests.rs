/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! `Add explicit type` over a real index: on `let rule = Flat(10);` Zed offered no code action at all, where
//! rust-analyzer writes the type its inlay hint shows. Offered only when the written type names what it shows.

use super::failed_build_tests::{index_opts, write_native_and_app};
use super::lsp_server::Server;
use super::*;
use crate::indexer::driver::find_driver_binary;
use crate::indexer::orchestrate::try_run_indexing;

const APP: &str = "use std::collections::HashSet;\npub struct Flat(pub u64);\npub fn make() -> usize {\n    let rule = Flat(10);\n    let mut seen = HashSet::new();\n    seen.insert(rule.0);\n    let typed: u64 = 1;\n    let map = std::collections::HashMap::<u8, u8>::new();\n    seen.len() + typed as usize + map.len() + native::native_api() as usize\n}\n";

#[test]
fn test_add_explicit_type_writes_the_hinted_type_when_it_names_it() {
    let _lock = lock_indexer_test();
    let Some(driver) = find_driver_binary() else { return };
    let dir = new_test_dir("ridx_explicit_type");
    let (ws, cas) = (dir.0.join("ws"), dir.0.join("cas"));
    write_native_and_app(&ws, APP);
    try_run_indexing(index_opts(&ws, &cas, &driver)).expect("index");
    let mut server = Server::start(&ws);
    let uri = server.open(&ws.join("app/src/lib.rs"));

    let mut explicit = |line: u32, character: u32| {
        server.code_actions(&uri, line, character).into_iter().find(|a| a["title"] == "Add explicit type").map(|a| {
            let edits = a["edit"]["documentChanges"][0]["edits"].as_array().or_else(|| a["edit"]["changes"][uri.as_str()].as_array()).cloned().unwrap_or_default();
            (edits[0]["range"]["start"].clone(), edits[0]["newText"].as_str().unwrap_or_default().to_string())
        })
    };

    let (at, text) = explicit(3, 9).expect("offered on `rule`");
    assert_eq!((at, text.as_str()), (serde_json::json!({ "line": 3, "character": 12 }), ": Flat"));
    let (_, text) = explicit(4, 12).expect("offered on `seen`");
    assert_eq!(text, ": HashSet<u64>", "the default hasher left out");
    assert!(explicit(6, 9).is_none(), "already typed");
    assert!(explicit(7, 9).is_none(), "HashMap is not imported: writing it would not compile");
}
