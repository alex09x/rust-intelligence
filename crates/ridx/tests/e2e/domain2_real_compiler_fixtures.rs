/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Domain 2: Real recorded cargo check JSONL fixtures tests.
//!
//! Validates parsing of real `cargo check --message-format=json` envelopes:
//! compiler-message unwrapping, expansion spans, machine-applicable suggestions,
//! multi-crate workspace relative paths, and clean build artifact filtering.

use ridx::lsp::presentation::compiler_message::parse_compiler_message;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

fn get_fixture_path(name: &str) -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir.join("tests/fixtures/diagnostics").join(name)
}

fn read_fixture_lines(name: &str) -> Vec<Value> {
    let path = get_fixture_path(name);
    let content = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("Failed to read fixture {}: {e}", path.display()));
    content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("Invalid JSON line in fixture"))
        .collect()
}

#[test]
fn test_parse_real_e0425_unresolved_name_fixture() {
    let lines = read_fixture_lines("e0425_unresolved_name.jsonl");
    assert_eq!(lines.len(), 3);

    let dummy_root = Path::new("/tmp/test_workspace");

    // Line 1: compiler-message with error E0425
    let res1 = parse_compiler_message(&lines[0], dummy_root);
    assert!(res1.is_some(), "Line 1 must parse into a diagnostic");
    let (uri, diag) = res1.unwrap();
    assert!(uri.ends_with("src/lib.rs"));
    assert_eq!(diag["severity"], 1); // Error
    assert_eq!(diag["code"], "E0425");
    assert_eq!(diag["source"], "rustc");
    assert!(diag["message"]
        .as_str()
        .unwrap()
        .contains("cannot find function `unknown_ident`"));
    assert_eq!(diag["range"]["start"]["line"], 1);
    assert_eq!(diag["range"]["start"]["character"], 4);
    assert_eq!(diag["range"]["end"]["line"], 1);
    assert_eq!(diag["range"]["end"]["character"], 17);

    // Line 2: failure-note with empty spans -> None
    let res2 = parse_compiler_message(&lines[1], dummy_root);
    assert!(res2.is_none(), "failure-note without spans must be ignored");

    // Line 3: build-finished -> None
    let res3 = parse_compiler_message(&lines[2], dummy_root);
    assert!(res3.is_none(), "build-finished envelope must be ignored");
}

#[test]
fn test_parse_real_unused_warning_with_suggestion_fixture() {
    let lines = read_fixture_lines("unused_warning_suggestion.jsonl");
    assert_eq!(lines.len(), 3);

    let dummy_root = Path::new("/tmp/test_workspace");

    // Line 1: compiler-message with warning unused_variables
    let res1 = parse_compiler_message(&lines[0], dummy_root);
    assert!(
        res1.is_some(),
        "Unused warning must parse into a diagnostic"
    );
    let (uri, diag) = res1.unwrap();
    assert!(uri.ends_with("src/lib.rs"));
    assert_eq!(diag["severity"], 2); // Warning
    assert_eq!(diag["code"], "unused_variables");
    let msg_str = diag["message"].as_str().unwrap();
    assert!(msg_str.contains("unused variable: `unused_val`"));
    assert!(msg_str.contains("prefix it with an underscore"));

    // Line 2: compiler-artifact -> None
    let res2 = parse_compiler_message(&lines[1], dummy_root);
    assert!(res2.is_none(), "compiler-artifact must be ignored");

    // Line 3: build-finished -> None
    let res3 = parse_compiler_message(&lines[2], dummy_root);
    assert!(res3.is_none(), "build-finished must be ignored");
}

#[test]
fn test_parse_real_macro_expansion_callsite_fixture() {
    let lines = read_fixture_lines("macro_expansion_callsite.jsonl");
    let dummy_root = Path::new("/tmp/test_workspace");

    // Line 1: compiler-message inside gen_err! macro with E0308 unwound to call site
    let res1 = parse_compiler_message(&lines[0], dummy_root);
    assert!(
        res1.is_some(),
        "Macro expansion compiler-message must parse"
    );
    let (uri, diag) = res1.unwrap();
    assert!(uri.ends_with("src/lib.rs"));
    assert_eq!(diag["severity"], 1);
    assert_eq!(diag["code"], "E0308");
    assert_eq!(diag["range"]["start"]["line"], 6); // Line 7 in file (0-based 6)
    assert_eq!(diag["range"]["start"]["character"], 4); // Column 5 (0-based 4)
    assert_eq!(diag["range"]["end"]["character"], 14); // Column 15 (0-based 14)

    // Non-primary definition span is captured in relatedInformation
    let related = diag["relatedInformation"]
        .as_array()
        .expect("Must have related info");
    assert!(
        related.len() >= 2,
        "Both secondary note and macro definition must be in relatedInformation"
    );
    assert!(related[0]["message"]
        .as_str()
        .unwrap()
        .contains("expected due to this"));
    assert_eq!(related[0]["location"]["range"]["start"]["line"], 2); // Line 3 in file (0-based 2)
    assert!(related[1]["message"]
        .as_str()
        .unwrap()
        .contains("in macro definition"));
    assert_eq!(related[1]["location"]["range"]["start"]["line"], 2);
}

#[test]
fn test_parse_real_multi_crate_subcrate_fixture() {
    let lines = read_fixture_lines("multi_crate_subcrate.jsonl");
    let workspace_root = Path::new("/home/user/workspace/demo");

    // Line 1: compiler-message for crates/subcrate/src/lib.rs
    let res1 = parse_compiler_message(&lines[0], workspace_root);
    assert!(res1.is_some(), "Multi-crate message must parse");
    let (uri, diag) = res1.unwrap();
    assert!(
        uri.ends_with("crates/subcrate/src/lib.rs"),
        "URI must preserve workspace-relative subcrate path: {uri}"
    );
    assert_eq!(diag["severity"], 1);
    assert_eq!(diag["code"], "E0425");
    assert_eq!(diag["range"]["start"]["line"], 1);
}

#[test]
fn test_parse_real_clean_build_artifacts_ignored_fixture() {
    let lines = read_fixture_lines("clean_build_artifacts.jsonl");
    let dummy_root = Path::new("/tmp/test_workspace");

    // All lines in clean build must be ignored (0 diagnostics)
    for (i, line) in lines.iter().enumerate() {
        let res = parse_compiler_message(line, dummy_root);
        assert!(
            res.is_none(),
            "Line {i} in clean build (reason={:?}) must produce None",
            line.get("reason")
        );
    }
}
