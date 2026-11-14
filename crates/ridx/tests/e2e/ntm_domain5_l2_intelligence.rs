/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Compiler-grounded L2 integration tests for Domain 5: Language Intelligence.
//! Validates hover, inlay hints, receiver-isolated completion, and signatureHelp
//! on real compiler-indexed fixture workspaces.

use super::l2_harness::{fixture_dir, get_or_index_l2_fixture, pos_in_file};
use ridx::lsp::intelligence::completion::handle_completion;
use ridx::lsp::intelligence::hints::handle_inlay_hint;
use ridx::lsp::intelligence::hover::handle_hover;
use ridx::lsp::intelligence::signature::handle_signature_help;
use ridx::lsp::protocol::{path_to_uri, PositionConverter};
use ridx::overlay::OverlayManager;
use serde_json::json;
use std::time::Instant;

#[test]
fn test_l2_hover_on_inferred_vec() {
    let ws = get_or_index_l2_fixture("shorthand_and_reexports");
    let lib_file = fixture_dir("shorthand_and_reexports").join("src/lib.rs");
    let uri = path_to_uri(lib_file.to_str().unwrap());
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    // Query hover on `v` in `let v = vec![1u8];`
    let (line, col) = pos_in_file(&lib_file, "let v = vec![1u8];", "v");
    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": line, "character": col }
        }
    });

    let res = handle_hover(&req, Some(ws), &overlay_mgr, &mut conv).unwrap();
    assert!(!res.is_null(), "Hover on inferred vec must return result");
    let val = res["contents"]["value"].as_str().unwrap();
    assert!(val.contains("Vec<u8>"), "Hover on vec![1u8] must contain Vec<u8>, got: {val}");
}

#[test]
fn test_l2_hover_on_assoc_type_extract() {
    let ws = get_or_index_l2_fixture("traits_and_generics");
    let lib_file = fixture_dir("traits_and_generics").join("src/lib.rs");
    let uri = path_to_uri(lib_file.to_str().unwrap());
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let (line, col) = pos_in_file(&lib_file, "process_container<C: Container<Item = i32>>(c: &C) -> i32 {\n    c.extract()", "extract");
    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": line, "character": col }
        }
    });

    let res = handle_hover(&req, Some(ws), &overlay_mgr, &mut conv).unwrap();
    assert!(!res.is_null(), "Hover on c.extract() must return result");
    let val = res["contents"]["value"].as_str().unwrap();
    assert!(val.contains("i32"), "Hover on c.extract() must resolve inferred return type i32 via Item = i32, got: {val}");
}

#[test]
fn test_l2_hover_on_generic_trait_method() {
    let ws = get_or_index_l2_fixture("traits_and_generics");
    let lib_file = fixture_dir("traits_and_generics").join("src/lib.rs");
    let uri = path_to_uri(lib_file.to_str().unwrap());
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    // Query hover on generic `greet` in `g.greet()` inside `call_generic`
    let (line, col) = pos_in_file(&lib_file, "g.greet()", "greet");
    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": line, "character": col }
        }
    });

    let res = handle_hover(&req, Some(ws), &overlay_mgr, &mut conv).unwrap();
    assert!(!res.is_null(), "Hover on g.greet() must return result");
    let val = res["contents"]["value"].as_str().unwrap();
    assert!(val.contains("fn(&'a Self) -> &'static str") || val.contains("fn greet"), "Hover on generic g.greet() must show trait method signature, got: {val}");
}

#[test]
fn test_l2_inlay_hint_utf16_with_emoji() {
    let ws = get_or_index_l2_fixture("shorthand_and_reexports");
    let lib_file = fixture_dir("shorthand_and_reexports").join("src/lib.rs");
    let uri = path_to_uri(lib_file.to_str().unwrap());
    let overlay_mgr = OverlayManager::new();

    // Find line of `/* 🦀 */ let p = make_point(1, 2);`
    let (line, _) = pos_in_file(&lib_file, "/* 🦀 */ let p = make_point(1, 2);", "let p");
    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "range": {
                "start": { "line": line, "character": 0 },
                "end": { "line": line + 1, "character": 0 }
            }
        }
    });

    let res = handle_inlay_hint(&req, Some(ws), &overlay_mgr).unwrap();
    let hints = res.as_array().expect("Inlay hints response must be an array");

    let p_hint = hints.iter().find(|h| h["label"].as_str().unwrap_or("").contains("Point")).expect("Must find inlay hint for Point");

    assert_eq!(p_hint["position"]["line"], line);
    // Line content: `    /* 🦀 */ let p = make_point(1, 2);`
    // UTF-16 code units: 4 (spaces) + 3 ('/* ') + 2 (🦀) + 4 (' */ ') + 4 ('let ') + 1 ('p') = 18
    let char_pos = p_hint["position"]["character"].as_u64().unwrap() as u32;
    assert_eq!(char_pos, 18, "Inlay hint must be positioned at character 18 in UTF-16");
    assert_eq!(p_hint["label"], ": Point");
}

#[test]
fn test_l2_completion_receiver_isolation() {
    let ws = get_or_index_l2_fixture("multi_crate");
    let app_file = fixture_dir("multi_crate").join("crates/app_crate/src/lib.rs");
    let uri = path_to_uri(app_file.to_str().unwrap());
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    // Line 28: `    ext_svc.execute() + local_svc.execute()`
    // Trigger completion immediately after `ext_svc.`
    let (line, col) = pos_in_file(&app_file, "ext_svc.execute()", "execute");
    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": line, "character": col }
        }
    });

    let res = handle_completion(&req, Some(ws), &overlay_mgr, &mut conv).unwrap();
    let items = res["items"].as_array().expect("Completion items must be an array");

    let execute_item = items.iter().find(|item| item["label"] == "execute").expect("execute method must be suggested for ext_svc.");

    // The detail is the method's signature, as rust-analyzer gives it.
    let detail = execute_item["detail"].as_str().unwrap();
    assert_eq!(detail, "fn(&self) -> u32");

    // Cross-crate receiver isolation: `ext_svc` is core_lib's `CoreService`, not the app crate's same-named type
    // (field `local_val`). Before a call's parentheses only methods complete, as in rust-analyzer.
    let labels: Vec<&str> = items.iter().filter_map(|item| item["label"].as_str()).collect();
    assert!(!labels.contains(&"local_val"), "the app crate's field must not leak into ext_svc. completion: {labels:?}");
    assert!(!labels.contains(&"id"), "a field is no method: {labels:?}");
}

#[test]
fn test_l2_completion_generic_trait_bound() {
    let ws = get_or_index_l2_fixture("traits_and_generics");
    let lib_file = fixture_dir("traits_and_generics").join("src/lib.rs");
    let uri = path_to_uri(lib_file.to_str().unwrap());
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    // Line 51: `    g.greet()` -> trigger completion after `g.`
    let (line, col) = pos_in_file(&lib_file, "g.greet()", "greet");
    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": line, "character": col }
        }
    });

    let res = handle_completion(&req, Some(ws), &overlay_mgr, &mut conv).unwrap();
    let items = res["items"].as_array().expect("Completion items must be an array");

    // A bound's methods are offered with their signatures as detail, as rust-analyzer does (the bound's own trait
    // needs no `(as Greeter)` label: it is in scope).
    let greet_item = items.iter().find(|item| item["label"] == "greet").expect("greet method must be suggested for g. where G: Greeter");
    assert_eq!(greet_item["detail"].as_str().unwrap(), "fn(&self) -> &'static str");

    let default_greet_item = items.iter().find(|item| item["label"] == "default_greet").expect("default_greet method must be suggested for g. where G: Greeter");
    assert_eq!(default_greet_item["detail"].as_str().unwrap(), "fn(&self) -> &'static str");
}

#[test]
fn test_l2_signature_help_cross_crate_and_trait() {
    let ws_multi = get_or_index_l2_fixture("multi_crate");
    let app_file = fixture_dir("multi_crate").join("crates/app_crate/src/lib.rs");
    let app_uri = path_to_uri(app_file.to_str().unwrap());
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    // 1. Cross-crate constructor call: `ExternalCoreService::new(42)`
    let (line_new, col_new) = pos_in_file(&app_file, "ExternalCoreService::new(42)", "42");
    let req_new = json!({
        "params": {
            "textDocument": { "uri": &app_uri },
            "position": { "line": line_new, "character": col_new }
        }
    });
    let res_new = handle_signature_help(&req_new, Some(ws_multi), &overlay_mgr, &mut conv).unwrap();
    assert!(!res_new.is_null(), "Signature help for ExternalCoreService::new must be found");
    let sig_new = res_new["signatures"][0]["label"].as_str().unwrap();
    assert!(sig_new.contains("fn(u32) -> CoreService") || sig_new.contains("pub fn new"), "Expected CoreService::new signature, got: {sig_new}");

    // 2. Trait method call: `c.extract()`
    let ws_trait = get_or_index_l2_fixture("traits_and_generics");
    let trait_file = fixture_dir("traits_and_generics").join("src/lib.rs");
    let trait_uri = path_to_uri(trait_file.to_str().unwrap());

    let (line_ext, col_ext) = pos_in_file(&trait_file, "process_container<C: Container<Item = i32>>(c: &C) -> i32 {\n    c.extract()", ")");
    let req_ext = json!({
        "params": {
            "textDocument": { "uri": &trait_uri },
            "position": { "line": line_ext, "character": col_ext }
        }
    });
    let res_ext = handle_signature_help(&req_ext, Some(ws_trait), &overlay_mgr, &mut conv).unwrap();
    assert!(!res_ext.is_null(), "Signature help for c.extract() must be found");
    let sig_ext = res_ext["signatures"][0]["label"].as_str().unwrap();
    assert!(
        sig_ext.contains("Container>::Item")
            || sig_ext.contains("fn extract")
            || sig_ext.contains("Self::Item")
            || sig_ext.contains("fn(&'a C) -> i32")
            || sig_ext.contains("-> i32"),
        "Expected Container::extract signature, got: {sig_ext}"
    );

    // 3. Generic trait method call: `g.greet()`
    let (line_greet, col_greet) = pos_in_file(&trait_file, "g.greet()", ")");
    let req_greet = json!({
        "params": {
            "textDocument": { "uri": &trait_uri },
            "position": { "line": line_greet, "character": col_greet }
        }
    });
    let res_greet = handle_signature_help(&req_greet, Some(ws_trait), &overlay_mgr, &mut conv).unwrap();
    assert!(!res_greet.is_null(), "Signature help for g.greet() must be found");
    let sig_greet = res_greet["signatures"][0]["label"].as_str().unwrap();
    assert!(sig_greet.contains("fn(&self)") || sig_greet.contains("fn(&'a Self)") || sig_greet.contains("fn greet"), "Expected Greeter::greet signature, got: {sig_greet}");
}

#[test]
fn test_l2_completion_latency_benchmark() {
    let ws = get_or_index_l2_fixture("multi_crate");
    let app_file = fixture_dir("multi_crate").join("crates/app_crate/src/lib.rs");
    let uri = path_to_uri(app_file.to_str().unwrap());
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    // 1. Dot-completion benchmark after `ext_svc.`
    let (dot_line, dot_col) = pos_in_file(&app_file, "ext_svc.execute()", "execute");
    let dot_req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": dot_line, "character": dot_col }
        }
    });

    let mut dot_latencies = Vec::with_capacity(100);
    for _ in 0..100 {
        let t0 = Instant::now();
        let _ = handle_completion(&dot_req, Some(ws), &overlay_mgr, &mut conv).unwrap();
        dot_latencies.push(t0.elapsed());
    }
    dot_latencies.sort();
    let dot_p50 = dot_latencies[50];
    let dot_p95 = dot_latencies[95];

    // 2. Colon-completion benchmark after `ExternalCoreService::`
    let (colon_line, colon_col) = pos_in_file(&app_file, "ExternalCoreService::new", "new");
    let colon_req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": colon_line, "character": colon_col }
        }
    });

    let mut colon_latencies = Vec::with_capacity(100);
    for _ in 0..100 {
        let t0 = Instant::now();
        let _ = handle_completion(&colon_req, Some(ws), &overlay_mgr, &mut conv).unwrap();
        colon_latencies.push(t0.elapsed());
    }
    colon_latencies.sort();
    let colon_p50 = colon_latencies[50];
    let colon_p95 = colon_latencies[95];

    eprintln!("BENCHMARK_COMPLETION_DOT: p50={:?}, p95={:?}", dot_p50, dot_p95);
    eprintln!("BENCHMARK_COMPLETION_COLON: p50={:?}, p95={:?}", colon_p50, colon_p95);

    // Enforce sub-5ms latency for scoped completions (<100µs measured)
    assert!(dot_p50.as_micros() < 5000, "Dot completion p50 must be under 5ms, got {:?}", dot_p50);
    assert!(colon_p50.as_micros() < 5000, "Colon completion p50 must be under 5ms, got {:?}", colon_p50);
}
