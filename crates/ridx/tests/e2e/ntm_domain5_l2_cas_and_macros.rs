/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Compiler-grounded integration tests for macro expansion binding filtering,
//! CAS shard portability (closure path sanitization), and completion benchmarking on rich types.

use super::l2_harness::{fixture_dir, get_or_index_l2_fixture, pos_in_file};
use ridx::lsp::intelligence::completion::handle_completion;
use ridx::lsp::intelligence::hover::handle_hover;
use ridx::lsp::protocol::{path_to_uri, PositionConverter};
use ridx::overlay::OverlayManager;
use serde_json::json;
use std::time::Instant;

#[test]
fn test_l2_macro_expansion_no_synthetic_bindings() {
    let ws = get_or_index_l2_fixture("shorthand_and_reexports");
    let lib_file = fixture_dir("shorthand_and_reexports").join("src/lib.rs");
    let uri = path_to_uri(lib_file.to_str().unwrap());
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    // 1. Verify hover on `println` in `println!("{x}");`
    let (pl_line, pl_col) = pos_in_file(&lib_file, "println!(\"{x}\");", "println");
    let pl_req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": pl_line, "character": pl_col }
        }
    });

    let pl_res = handle_hover(&pl_req, Some(ws), &overlay_mgr, &mut conv).unwrap();
    if !pl_res.is_null() {
        let val = pl_res["contents"]["value"].as_str().unwrap();
        assert!(!val.contains("Arguments"), "Hover on println! must NOT leak compiler-synthetic Arguments binding, got: {val}");
        assert!(!val.contains("ArgumentV1"), "Hover on println! must NOT leak ArgumentV1 binding, got: {val}");
    }

    // 2. Verify hover on `vec` in `let v = vec![1u8];`
    let (vec_line, vec_col) = pos_in_file(&lib_file, "let v = vec![1u8];", "vec");
    let vec_req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": vec_line, "character": vec_col }
        }
    });

    let vec_res = handle_hover(&vec_req, Some(ws), &overlay_mgr, &mut conv).unwrap();
    if !vec_res.is_null() {
        let val = vec_res["contents"]["value"].as_str().unwrap();
        assert!(!val.contains("temp_vec"), "Hover on vec! must NOT leak internal temp_vec binding, got: {val}");
    }

    // 3. Verify user binding `v` retains exact inferred type Vec<u8>
    let (v_line, v_col) = pos_in_file(&lib_file, "let v = vec![1u8];", "v");
    let v_req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": v_line, "character": v_col }
        }
    });

    let v_res = handle_hover(&v_req, Some(ws), &overlay_mgr, &mut conv).unwrap();
    assert!(!v_res.is_null(), "Hover on binding v must return result");
    let val = v_res["contents"]["value"].as_str().unwrap();
    assert!(val.contains("Vec<u8>"), "Hover on user binding v must resolve to Vec<u8>, got: {val}");
}

#[test]
fn test_l2_cas_portability_and_closure_sanitization() {
    // Check both multi_crate (with vendor dependency) and shorthand_and_reexports
    let ws_multi = get_or_index_l2_fixture("multi_crate");
    let ws_short = get_or_index_l2_fixture("shorthand_and_reexports");

    for (ws_name, ws) in [("multi_crate", ws_multi), ("shorthand", ws_short)] {
        for (i, reader) in ws.readers().iter().enumerate() {
            for type_entry in reader.types() {
                let ty_str = reader.get_str(type_entry.type_offset());

                // Assert zero leakage of home directory or absolute paths
                assert!(!ty_str.contains("/home/") && !ty_str.contains("/Users/"), "Type string in {ws_name} reader {i} leaked absolute path: {ty_str}");

                // Assert closures are normalized without span path coordinates
                assert!(!ty_str.contains("{closure@"), "Type string in {ws_name} reader {i} contains unsanitized closure span: {ty_str}");
                assert!(!ty_str.contains("{async fn body@"), "Type string in {ws_name} reader {i} contains unsanitized async span: {ty_str}");
            }
        }
    }
}

#[test]
fn test_l2_rich_type_completion_latency_benchmark() {
    let ws = get_or_index_l2_fixture("shorthand_and_reexports");
    let lib_file = fixture_dir("shorthand_and_reexports").join("src/lib.rs");
    let uri = path_to_uri(lib_file.to_str().unwrap());
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    // 1. Benchmark completion on `it.` in `it.next()`
    let (it_line, it_col) = pos_in_file(&lib_file, "it.next()", "next");
    let it_req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": it_line, "character": it_col }
        }
    });

    let mut it_latencies = Vec::with_capacity(100);
    for _ in 0..100 {
        let t0 = Instant::now();
        let _ = handle_completion(&it_req, Some(ws), &overlay_mgr, &mut conv).unwrap();
        it_latencies.push(t0.elapsed());
    }
    it_latencies.sort();
    let it_p50 = it_latencies[50];
    let it_p95 = it_latencies[95];

    // 2. Benchmark completion on `make_point(1, 2)` receiver method lookup
    let (pt_line, pt_col) = pos_in_file(&lib_file, "make_point(1, 2);", "make_point");
    let pt_req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": pt_line, "character": pt_col }
        }
    });

    let mut pt_latencies = Vec::with_capacity(100);
    for _ in 0..100 {
        let t0 = Instant::now();
        let _ = handle_completion(&pt_req, Some(ws), &overlay_mgr, &mut conv).unwrap();
        pt_latencies.push(t0.elapsed());
    }
    pt_latencies.sort();
    let pt_p50 = pt_latencies[50];
    let pt_p95 = pt_latencies[95];

    eprintln!("BENCHMARK_COMPLETION_ITERATOR: p50={:?}, p95={:?}", it_p50, it_p95);
    eprintln!("BENCHMARK_COMPLETION_POINT: p50={:?}, p95={:?}", pt_p50, pt_p95);

    // The budget is held on our build nodes. GitHub's shared runners, with the whole suite running in parallel,
    // went past it now and then (the v0.3.14 tag's CI, the 0.3.12 cycle): there the numbers are printed only.
    if std::env::var_os("GITHUB_ACTIONS").is_some() {
        return;
    }
    assert!(it_p50.as_micros() < 5000, "Iterator completion p50 must be under 5ms, got {:?}", it_p50);
    assert!(pt_p50.as_micros() < 5000, "Point completion p50 must be under 5ms, got {:?}", pt_p50);
}

#[test]
fn test_out_of_bounds_line_completion_returns_empty() {
    let ws = get_or_index_l2_fixture("shorthand_and_reexports");
    let lib_file = fixture_dir("shorthand_and_reexports").join("src/lib.rs");
    let uri = path_to_uri(lib_file.to_str().unwrap());
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    // 1. Without overlay: position far beyond EOF returns empty list without panicking
    let oob_req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 99999, "character": 0 }
        }
    });
    let res = handle_completion(&oob_req, Some(ws), &overlay_mgr, &mut conv).unwrap();
    assert_eq!(res["items"].as_array().unwrap().len(), 0);
    assert_eq!(res["isIncomplete"], false);

    // 2. With overlay: position beyond overlay line count returns empty list without panicking
    overlay_mgr.open_document(&uri, lib_file.to_str().unwrap(), 1, "fn main() {}\n".to_string());
    let oob_overlay_req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 50, "character": 0 }
        }
    });
    let res_ov = handle_completion(&oob_overlay_req, Some(ws), &overlay_mgr, &mut conv).unwrap();
    assert_eq!(res_ov["items"].as_array().unwrap().len(), 0);
    assert_eq!(res_ov["isIncomplete"], false);
}

#[test]
fn test_l2_generic_param_completion_in_fn_and_impl_body() {
    let ws = get_or_index_l2_fixture("traits_and_generics");
    let lib_file = fixture_dir("traits_and_generics").join("src/lib.rs");
    let uri = path_to_uri(lib_file.to_str().unwrap());
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    // 1. Function body: call_generic<G: Greeter>
    let (g_line, g_col) = pos_in_file(&lib_file, "let _ = G::default_greet;", "G::");
    let g_param = ws.find_type_param_in_scope("src/lib.rs", "G", g_line + 1, g_col + 1);
    assert!(g_param.is_some(), "Generic parameter 'G' must be in scope inside call_generic fn body");
    let g_def = g_param.unwrap();
    assert_eq!(g_def.name, "G");
    assert_eq!(g_def.signature, Some("Greeter"));

    // Completion at G:: inside function body (cursor right after 'G::')
    let g_req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": g_line, "character": g_col + 3 }
        }
    });
    let g_res = handle_completion(&g_req, Some(ws), &overlay_mgr, &mut conv).unwrap();
    let g_items: Vec<&str> = g_res["items"].as_array().unwrap().iter().filter_map(|it| it["label"].as_str()).collect();
    assert!(g_items.contains(&"greet"), "Completion on G:: in fn body must include 'greet', got: {g_items:?}");
    assert!(g_items.contains(&"default_greet"), "Completion on G:: in fn body must include 'default_greet', got: {g_items:?}");

    // 2. Impl body: impl<W: Greeter> Wrapper<W>
    let (w_line, w_col) = pos_in_file(&lib_file, "let _ = W::default_greet;", "W::");
    let w_param = ws.find_type_param_in_scope("src/lib.rs", "W", w_line + 1, w_col + 1);
    assert!(w_param.is_some(), "Generic parameter 'W' must be in scope inside Wrapper<W> impl body");
    let w_def = w_param.unwrap();
    assert_eq!(w_def.name, "W");
    assert_eq!(w_def.signature, Some("Greeter"));

    // Completion at W:: inside impl body (cursor right after 'W::')
    let w_req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": w_line, "character": w_col + 3 }
        }
    });
    let w_res = handle_completion(&w_req, Some(ws), &overlay_mgr, &mut conv).unwrap();
    let w_items: Vec<&str> = w_res["items"].as_array().unwrap().iter().filter_map(|it| it["label"].as_str()).collect();
    assert!(w_items.contains(&"greet"), "Completion on W:: in impl body must include 'greet', got: {w_items:?}");
    assert!(w_items.contains(&"default_greet"), "Completion on W:: in impl body must include 'default_greet', got: {w_items:?}");
}
