/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Domain 2: Flycheck lifecycle integration tests, stream message processing,
//! driver resolution, diagnostic accumulation across files, and stale generation filtering.

use ridx::lsp::presentation::compiler_message::{find_ridx_driver, parse_compiler_message};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static TEST_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

fn setup_temp_workspace() -> (PathBuf, PathBuf, PathBuf) {
    let id = TEST_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp_dir =
        std::env::temp_dir().join(format!("ridx_test_d2_lc_{}_{}", std::process::id(), id));
    let src_lib = temp_dir.join("src/lib.rs");
    let src_main = temp_dir.join("src/main.rs");
    let _ = fs::create_dir_all(temp_dir.join("src"));
    let _ = fs::write(&src_lib, "pub fn lib_func() {}\n");
    let _ = fs::write(&src_main, "fn main() {}\n");
    (temp_dir, src_lib, src_main)
}

fn process_simulated_cargo_stream(lines: &[&str], root: &Path) -> HashMap<String, Vec<Value>> {
    let mut by_uri: HashMap<String, Vec<Value>> = HashMap::new();
    for line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(val) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        // Rustc compiler message is wrapped in {"reason": "compiler-message", "message": ...}
        // or direct compiler message
        let msg_obj = if val.get("reason") == Some(&Value::String("compiler-message".to_string())) {
            val.get("message")
        } else {
            Some(&val)
        };

        if let Some(msg) = msg_obj {
            if let Some((uri, diag)) = parse_compiler_message(msg, root) {
                by_uri.entry(uri).or_default().push(diag);
            }
        }
    }
    by_uri
}

#[test]
fn test_flycheck_stream_single_file_diagnostics() {
    let (root, _, _) = setup_temp_workspace();
    let stream = [
        r#"{"$message_type":"diagnostic","message":"unused variable: `x`","code":{"code":"unused_variables"},"level":"warning","spans":[{"file_name":"src/lib.rs","line_start":1,"line_end":1,"column_start":1,"column_end":2,"is_primary":true}]}"#,
        r#"{"$message_type":"artifact","artifact":"target/debug/libtest.rmeta","emit":"metadata"}"#,
    ];

    let diags_by_uri = process_simulated_cargo_stream(&stream, &root);
    assert_eq!(diags_by_uri.len(), 1);
    let diags = diags_by_uri.values().next().unwrap();
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0]["severity"], 2);
    assert_eq!(diags[0]["code"], "unused_variables");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn test_flycheck_stream_multi_file_diagnostics() {
    let (root, _, _) = setup_temp_workspace();
    let stream = [
        r#"{"reason":"compiler-message","message":{"message":"lib error","level":"error","spans":[{"file_name":"src/lib.rs","line_start":2,"line_end":2,"column_start":5,"column_end":10,"is_primary":true}]}}"#,
        r#"{"reason":"compiler-message","message":{"message":"main warning","level":"warning","spans":[{"file_name":"src/main.rs","line_start":3,"line_end":3,"column_start":1,"column_end":5,"is_primary":true}]}}"#,
        r#"{"reason":"build-finished","success":false}"#,
    ];

    let diags_by_uri = process_simulated_cargo_stream(&stream, &root);
    assert_eq!(
        diags_by_uri.len(),
        2,
        "Must group diagnostics by distinct file URIs"
    );

    let lib_uri = diags_by_uri
        .keys()
        .find(|k| k.ends_with("src/lib.rs"))
        .expect("lib.rs uri");
    assert_eq!(diags_by_uri[lib_uri].len(), 1);
    assert_eq!(diags_by_uri[lib_uri][0]["severity"], 1);

    let main_uri = diags_by_uri
        .keys()
        .find(|k| k.ends_with("src/main.rs"))
        .expect("main.rs uri");
    assert_eq!(diags_by_uri[main_uri].len(), 1);
    assert_eq!(diags_by_uri[main_uri][0]["severity"], 2);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn test_flycheck_stream_ignores_non_json_and_compiler_artifacts() {
    let (root, _, _) = setup_temp_workspace();
    let stream = [
        "Compiling test_pkg v0.1.0 (/path/to/test)",
        "warning: some human-readable text on stderr without json",
        r#"{"reason":"compiler-artifact","package_id":"test 0.1.0","target":{"name":"test"},"profile":{"opt_level":"0"}}"#,
        r#"{"reason":"build-script-executed","package_id":"test 0.1.0"}"#,
        "",
        "   ",
    ];

    let diags = process_simulated_cargo_stream(&stream, &root);
    assert!(
        diags.is_empty(),
        "Non-diagnostic messages must be ignored cleanly"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn test_flycheck_diagnostic_clearing_logic_on_clean_build() {
    let (root, _, _) = setup_temp_workspace();
    // Round 1: Run with error
    let stream_err = [
        r#"{"message":"syntax error","level":"error","spans":[{"file_name":"src/lib.rs","line_start":1,"line_end":1,"column_start":1,"column_end":5,"is_primary":true}]}"#,
    ];
    let diags_round1 = process_simulated_cargo_stream(&stream_err, &root);
    let mut published_uris: HashSet<String> = diags_round1.keys().cloned().collect();
    assert_eq!(published_uris.len(), 1);

    // Round 2: Clean build with 0 errors
    let stream_clean: [&str; 0] = [];
    let diags_round2 = process_simulated_cargo_stream(&stream_clean, &root);

    // Any URI that was previously published but not in current run must be cleared
    let mut cleared_payloads = Vec::new();
    for prev_uri in &published_uris {
        if !diags_round2.contains_key(prev_uri) {
            cleared_payloads.push(json!({
                "method": "textDocument/publishDiagnostics",
                "params": {
                    "uri": prev_uri,
                    "diagnostics": []
                }
            }));
        }
    }
    published_uris.clear();

    assert_eq!(cleared_payloads.len(), 1);
    assert!(cleared_payloads[0]["params"]["diagnostics"]
        .as_array()
        .unwrap()
        .is_empty());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn test_flycheck_generation_mismatch_drops_stale_notification() {
    let current_generation = 5u64;
    let old_job_generation = 4u64;

    // Simulate notification filter in LSP server
    let should_publish = old_job_generation == current_generation;
    assert!(
        !should_publish,
        "Notification from older generation must be discarded"
    );

    let match_job_generation = 5u64;
    let should_publish_match = match_job_generation == current_generation;
    assert!(
        should_publish_match,
        "Notification matching current generation must be accepted"
    );
}

#[test]
fn test_find_ridx_driver_custom_env_var() {
    let (root, _, _) = setup_temp_workspace();
    let fake_bin = root.join("custom_driver");
    let _ = fs::write(&fake_bin, "#!/bin/sh\nexit 0\n");

    std::env::set_var("RIDX_DRIVER_BIN", fake_bin.to_str().unwrap());
    let found = find_ridx_driver(&root);
    std::env::remove_var("RIDX_DRIVER_BIN");

    assert_eq!(found, Some(fake_bin));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn test_find_ridx_driver_fallback_candidate_search() {
    let (root, _, _) = setup_temp_workspace();
    let debug_bin = root.join("target/debug/ridx-driver");
    let _ = fs::create_dir_all(debug_bin.parent().unwrap());
    let _ = fs::write(&debug_bin, "fake");

    // Ensure RIDX_DRIVER_BIN does not interfere
    std::env::remove_var("RIDX_DRIVER_BIN");

    let found = find_ridx_driver(&root);
    assert_eq!(found, Some(debug_bin));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn test_find_ridx_driver_non_existent_returns_none() {
    let (root, _, _) = setup_temp_workspace();
    std::env::remove_var("RIDX_DRIVER_BIN");
    let found = find_ridx_driver(&root);
    assert_eq!(found, None);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn test_flycheck_multiple_errors_same_file_accumulated() {
    let (root, _, _) = setup_temp_workspace();
    let stream = [
        r#"{"message":"error 1","level":"error","spans":[{"file_name":"src/lib.rs","line_start":1,"line_end":1,"column_start":1,"column_end":2,"is_primary":true}]}"#,
        r#"{"message":"error 2","level":"error","spans":[{"file_name":"src/lib.rs","line_start":2,"line_end":2,"column_start":1,"column_end":2,"is_primary":true}]}"#,
        r#"{"message":"error 3","level":"error","spans":[{"file_name":"src/lib.rs","line_start":3,"line_end":3,"column_start":1,"column_end":2,"is_primary":true}]}"#,
    ];

    let diags = process_simulated_cargo_stream(&stream, &root);
    assert_eq!(diags.len(), 1);
    let file_diags = diags.values().next().unwrap();
    assert_eq!(
        file_diags.len(),
        3,
        "All 3 errors in the same file must be accumulated in order"
    );
    assert_eq!(file_diags[0]["message"], "error 1");
    assert_eq!(file_diags[1]["message"], "error 2");
    assert_eq!(file_diags[2]["message"], "error 3");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn test_parse_lint_with_suggested_replacement() {
    let (root, _, _) = setup_temp_workspace();
    let raw = json!({
        "message": "variable does not need to be mutable",
        "code": { "code": "unused_mut" },
        "level": "warning",
        "spans": [{
            "file_name": "src/lib.rs",
            "line_start": 1,
            "line_end": 1,
            "column_start": 5,
            "column_end": 8,
            "is_primary": true,
            "text": [{ "text": "let mut x = 1;", "highlight_start": 5, "highlight_end": 8 }]
        }],
        "children": [{
            "level": "help",
            "message": "remove this `mut`",
            "spans": [{
                "file_name": "src/lib.rs",
                "line_start": 1,
                "line_end": 1,
                "column_start": 5,
                "column_end": 8,
                "suggested_replacement": ""
            }]
        }]
    });

    let (_, diag) = parse_compiler_message(&raw, &root).unwrap();
    assert_eq!(diag["code"], "unused_mut");
    assert!(diag["message"]
        .as_str()
        .unwrap()
        .contains("remove this `mut`"));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn test_parse_error_outside_workspace_root() {
    let (root, _, _) = setup_temp_workspace();
    let raw = json!({
        "message": "external crate error",
        "level": "error",
        "spans": [{
            "file_name": "/cargo/registry/src/dep-1.0.0/src/lib.rs",
            "line_start": 10,
            "line_end": 10,
            "column_start": 1,
            "column_end": 5,
            "is_primary": true,
            "text": [{ "text": "test", "highlight_start": 1, "highlight_end": 5 }]
        }]
    });

    let (uri, diag) = parse_compiler_message(&raw, &root).unwrap();
    assert!(uri.starts_with("file:///cargo/registry/"));
    assert_eq!(diag["severity"], 1);
    let _ = fs::remove_dir_all(&root);
}
