/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Hover on a `let` binding inside a `for` loop or an `async fn`: the compiler spans the loop's `into_iter` call and
//! the async body's `ResumeTy` over the whole body, so the binding lies inside a reference to something it is not.
//! The hover is the binding's, as rust-analyzer shows it.

use super::l2_harness::{fixture_dir, get_or_index_l2_fixture, pos_in_file};
use ridx::lsp::intelligence::hover::handle_hover;
use ridx::lsp::protocol::{path_to_uri, PositionConverter};
use ridx::overlay::OverlayManager;
use serde_json::json;

fn hover_at(snippet: &str, needle: &str) -> String {
    let ws = get_or_index_l2_fixture("traits_and_generics");
    let lib = fixture_dir("traits_and_generics").join("src/lib.rs");
    let (line, col) = pos_in_file(&lib, snippet, needle);
    let req = json!({ "params": { "textDocument": { "uri": path_to_uri(lib.to_str().unwrap()) }, "position": { "line": line, "character": col + 2 } } });
    let res = handle_hover(&req, Some(ws), &OverlayManager::new(), &mut PositionConverter::new()).unwrap();
    res["contents"]["value"].as_str().unwrap_or_default().to_string()
}

#[test]
fn test_l2_hover_on_a_binding_inside_a_for_loop_is_the_binding() {
    let val = hover_at("let mut loop_local: Vec<String>", "loop_local");
    assert!(val.contains("loop_local: Vec<String>"), "{val}");
    assert!(!val.contains("IntoIterator"), "{val}");
}

#[test]
fn test_l2_hover_on_a_binding_inside_an_async_fn_is_the_binding() {
    let val = hover_at("let mut async_local: Vec<u8>", "async_local");
    assert!(val.contains("async_local: Vec<u8>"), "{val}");
    assert!(!val.contains("ResumeTy"), "{val}");
}

#[test]
fn test_l2_hover_on_a_use_of_the_binding_inside_a_for_loop() {
    let val = hover_at("loop_local.push(item", "loop_local");
    assert!(val.contains("loop_local: Vec<String>"), "{val}");
}
