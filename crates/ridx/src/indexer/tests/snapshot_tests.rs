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
use super::lsp_server::Server;
use super::*;
use crate::indexer::driver::find_driver_binary;
use crate::indexer::orchestrate::try_run_indexing;
use serde_json::json;

const LOADER: &str = "pub struct Loader { n: u32 }\nimpl Loader {\n    pub fn run(&mut self) -> u32 { self.finish(1) + native::native_api() }\n    fn finish(&mut self, x: u32) -> u32 { self.n + x }\n}\n";

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
    let line = first["range"]["start"]["line"].as_u64().unwrap() as usize;
    let text = fs::read_to_string(&target).unwrap();
    assert!(text.lines().nth(line).is_some_and(|l| l.contains("fn new")), "the line of `fn new` in the file opened: {first}");
    server.open(&target);
    let again = server.definition(&uri, 1, 34);
    assert_eq!(again, first, "the same place, with its file open");
}

const CART_INDEXED: &str = "pub mod model;\npub mod pricing;\nuse crate::pricing::Discount;\npub fn total(items: &[u64], rule: &dyn Discount) -> u64 {\n    items.iter().map(|p| rule.apply(*p)).sum()\n}\n#[cfg(test)]\nmod tests {\n    use super::*;\n    use std::collections::HashSet;\n    #[test]\n    fn t() {\n        assert_eq!(total(&[3], &crate::pricing::Flat(1)), 2);\n        let total_price = 3u64;\n        let x = pricing::Flat(total_price);\n        let _ = x.0;\n        let _seen: HashSet<u8> = HashSet::new();\n    }\n}\n";

/// In Zed on shop: `cart.rs` indexed with two lines more than it has now (reverted on disk), and the next reindex
/// failed on another module. Hover and jumps in `cart.rs` must keep working through the snapshot of the indexed text.
#[test]
fn test_a_reverted_file_keeps_the_index_after_a_failed_reindex() {
    let _lock = lock_indexer_test();
    let Some(driver) = find_driver_binary() else { return };
    let dir = new_test_dir("ridx_snapshot_failed");
    let (ws, cas) = (dir.0.join("ws"), dir.0.join("cas"));
    write_native_and_app(&ws, CART_INDEXED);
    super::failed_build_tests::write(&ws, "app/src/model.rs", "pub fn ok() -> u32 { 1 }\n");
    super::failed_build_tests::write(
        &ws,
        "app/src/pricing.rs",
        "pub trait Discount { fn apply(&self, p: u64) -> u64; }\npub struct Flat(pub u64);\nimpl Discount for Flat { fn apply(&self, p: u64) -> u64 { p - self.0 } }\n",
    );
    try_run_indexing(index_opts(&ws, &cas, &driver)).expect("index");

    let reverted = CART_INDEXED.replace("    use std::collections::HashSet;\n", "").replace("        let _seen: HashSet<u8> = HashSet::new();\n", "");
    let lib = ws.join("app/src/lib.rs");
    fs::write(&lib, &reverted).unwrap();
    super::failed_build_tests::write(&ws, "app/src/model.rs", "pub fn ok() -> u32 { 1 }\npub fn broken() -> u32 { \"x\" }\n");
    let _ = try_run_indexing(index_opts(&ws, &cas, &driver));

    let mut server = Server::start(&ws);
    let uri = server.open(&lib);
    let line = reverted.lines().position(|l| l.starts_with("pub fn total")).unwrap() as u32;
    let col = reverted.lines().nth(line as usize).unwrap().find("Discount").unwrap() as u32 + 2;
    let hover = server.send(json!({ "method": "textDocument/hover", "params": { "textDocument": { "uri": uri }, "position": { "line": line, "character": col } } }));
    assert!(hover["result"]["contents"]["value"].as_str().is_some_and(|v| v.contains("trait Discount")), "hover on `Discount`: {hover}");
    assert!(!server.definition(&uri, line, col).is_null(), "a jump to `Discount`");

    // Inside a binding whose next line on disk has a path at the same column: the index is read against the text it
    // compiled (the snapshot), not the newer file, which put `pricing::Flat` one line up (Zed showed `Product` there).
    let line = reverted.lines().position(|l| l.contains("let total_price")).unwrap() as u32;
    let hover = server.send(json!({ "method": "textDocument/hover", "params": { "textDocument": { "uri": uri }, "position": { "line": line, "character": 18 } } }));
    let text = hover["result"]["contents"]["value"].as_str().unwrap_or_default();
    assert!(text.contains("total_price") && text.contains("u64") && !text.contains("pricing"), "the binding, not the path below it: {hover}");
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
