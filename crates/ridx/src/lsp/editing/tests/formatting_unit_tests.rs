/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Unit tests for document formatting, range formatting, and edition detection.

use crate::lsp::editing::format::{find_crate_edition, handle_formatting, handle_range_formatting};
use crate::lsp::protocol::path_to_uri;
use crate::overlay::OverlayManager;
use ridx_test_support::apply_text_edits;
use serde_json::json;
use std::fs;

fn has_rustfmt() -> bool {
    std::process::Command::new("rustfmt").arg("--version").output().map(|o| o.status.success()).unwrap_or(false)
}

#[test]
fn test_find_crate_edition_detection() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    // Boundary sentinel to isolate tests from host filesystem
    fs::create_dir_all(root.join(".git")).unwrap();

    // Standard edition in package
    let cargo1 = root.join("pkg1/Cargo.toml");
    fs::create_dir_all(cargo1.parent().unwrap()).unwrap();
    fs::write(&cargo1, "[package]\nname = \"pkg1\"\nedition = \"2024\"\n").unwrap();
    let src1 = root.join("pkg1/src/lib.rs");
    fs::create_dir_all(src1.parent().unwrap()).unwrap();
    fs::write(&src1, "").unwrap();
    assert_eq!(find_crate_edition(src1.to_str().unwrap()), "2024");

    // Workspace inherited edition
    let ws_cargo = root.join("ws/Cargo.toml");
    fs::create_dir_all(ws_cargo.parent().unwrap()).unwrap();
    fs::write(&ws_cargo, "[workspace.package]\nedition = \"2018\"\n").unwrap();
    let sub_cargo = root.join("ws/crates/sub/Cargo.toml");
    fs::create_dir_all(sub_cargo.parent().unwrap()).unwrap();
    fs::write(&sub_cargo, "[package]\nname = \"sub\"\nedition.workspace = true\n").unwrap();
    let sub_src = root.join("ws/crates/sub/src/lib.rs");
    fs::create_dir_all(sub_src.parent().unwrap()).unwrap();
    fs::write(&sub_src, "").unwrap();
    assert_eq!(find_crate_edition(sub_src.to_str().unwrap()), "2018");

    // Package without explicit edition defaults to 2021
    let no_ed_cargo = root.join("pkg_no_ed/Cargo.toml");
    fs::create_dir_all(no_ed_cargo.parent().unwrap()).unwrap();
    fs::write(&no_ed_cargo, "[package]\nname = \"pkg_no_ed\"\n").unwrap();
    let no_ed_src = root.join("pkg_no_ed/src/lib.rs");
    fs::create_dir_all(no_ed_src.parent().unwrap()).unwrap();
    fs::write(&no_ed_src, "").unwrap();
    assert_eq!(find_crate_edition(no_ed_src.to_str().unwrap()), "2021");

    // Default fallback when no Cargo.toml exists
    let lone_file = root.join("standalone.rs");
    fs::write(&lone_file, "").unwrap();
    assert_eq!(find_crate_edition(lone_file.to_str().unwrap()), "2021");
}

#[test]
fn test_formatting_whole_document() {
    if !has_rustfmt() {
        eprintln!("Skipping formatting test: rustfmt is not installed or not in PATH");
        return;
    }

    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("lib.rs");
    let file_str = file.to_str().unwrap();
    let uri = path_to_uri(file_str);

    let unformatted = "fn   add(  x:i32,y:i32  )->i32{x+y}\n";
    fs::write(&file, unformatted).unwrap();

    // The editor formats a document it opened.
    let mut overlay_mgr = OverlayManager::new();
    overlay_mgr.open_document(&uri, file_str, 1, unformatted.to_string());
    let req = json!({
        "params": {
            "textDocument": { "uri": &uri }
        }
    });

    let res = handle_formatting(&req, &overlay_mgr);
    let edits = res.as_array().expect("Formatting must return array of TextEdits");
    assert!(!edits.is_empty(), "Must return at least one formatting edit");

    let formatted = apply_text_edits(unformatted, edits);
    assert_eq!(formatted, "fn add(x: i32, y: i32) -> i32 {\n    x + y\n}\n");
    syn::parse_file(&formatted).expect("Formatted code must be valid Rust");
}

#[test]
fn test_formatting_already_formatted_returns_empty() {
    if !has_rustfmt() {
        eprintln!("Skipping formatting test: rustfmt is not installed or not in PATH");
        return;
    }

    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("lib.rs");
    let file_str = file.to_str().unwrap();
    let uri = path_to_uri(file_str);

    let formatted = "fn add(x: i32, y: i32) -> i32 {\n    x + y\n}\n";
    fs::write(&file, formatted).unwrap();

    let overlay_mgr = OverlayManager::new();
    let req = json!({
        "params": {
            "textDocument": { "uri": &uri }
        }
    });

    let res = handle_formatting(&req, &overlay_mgr);
    let edits = res.as_array().expect("Formatting must return array of TextEdits");
    assert!(edits.is_empty(), "Already formatted document must return empty edits");
}

#[test]
fn test_formatting_dirty_overlay() {
    if !has_rustfmt() {
        eprintln!("Skipping formatting test: rustfmt is not installed or not in PATH");
        return;
    }

    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("lib.rs");
    let file_str = file.to_str().unwrap();
    let uri = path_to_uri(file_str);

    fs::write(&file, "/* clean on disk */\n").unwrap();

    let dirty_code = "fn   test(  a:bool  )->bool{!a}\n";
    let mut overlay_mgr = OverlayManager::new();
    overlay_mgr.open_document(&uri, file_str, 1, dirty_code.to_string());

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri }
        }
    });

    let res = handle_formatting(&req, &overlay_mgr);
    let edits = res.as_array().expect("Formatting array");
    assert!(!edits.is_empty());

    let formatted = apply_text_edits(dirty_code, edits);
    assert_eq!(formatted, "fn test(a: bool) -> bool {\n    !a\n}\n");
    syn::parse_file(&formatted).expect("Valid Rust");
}

#[test]
fn test_range_formatting_statements() {
    if !has_rustfmt() {
        eprintln!("Skipping formatting test: rustfmt is not installed or not in PATH");
        return;
    }

    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("lib.rs");
    let file_str = file.to_str().unwrap();
    let uri = path_to_uri(file_str);

    let code = "fn compute() {\n    let   a=1;\n    let   b=2;\n    println!(\"{}\", a + b);\n}\n";
    fs::write(&file, code).unwrap();

    let overlay_mgr = OverlayManager::new();
    // Range format lines 1..=2 (`let a=1; let b=2;`)
    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "range": {
                "start": { "line": 1, "character": 0 },
                "end": { "line": 2, "character": 14 }
            }
        }
    });

    let res = handle_range_formatting(&req, &overlay_mgr);
    let edits = res.as_array().expect("Range formatting array");
    assert!(!edits.is_empty(), "Range formatting should return edits");

    let formatted = apply_text_edits(code, edits);
    assert!(formatted.contains("let a = 1;"));
    assert!(formatted.contains("let b = 2;"));
    syn::parse_file(&formatted).expect("Valid Rust");
}

#[test]
fn test_range_formatting_out_of_bounds() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("lib.rs");
    let file_str = file.to_str().unwrap();
    let uri = path_to_uri(file_str);

    fs::write(&file, "fn ok() {}\n").unwrap();
    let overlay_mgr = OverlayManager::new();

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "range": {
                "start": { "line": 50, "character": 0 },
                "end": { "line": 60, "character": 0 }
            }
        }
    });

    let res = handle_range_formatting(&req, &overlay_mgr);
    assert_eq!(res, json!([]));
}
