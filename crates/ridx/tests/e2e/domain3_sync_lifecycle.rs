/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Domain 3: Document lifecycle synchronization tests (`didClose`, `didSave`,
//! and `OverlayManager` buffer registration).

use ridx::lsp::dispatch::dispatch_lsp_message;
use ridx::lsp::lifecycle::LifecycleManager;
use ridx::lsp::protocol::{path_to_uri, uri_to_path};
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx::overlay::OverlayManager;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::fs;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

struct CleanupDir(PathBuf);
impl Drop for CleanupDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn dispatch_msg(msg: Value, lifecycle: &mut LifecycleManager, multi_root: &mut MultiRootWorkspace, overlay_mgr: &mut OverlayManager) {
    let mut out = Cursor::new(Vec::new());
    let cancelled = Arc::new(Mutex::new(HashSet::new()));
    let _ = dispatch_lsp_message(msg, &mut out, lifecycle, multi_root, overlay_mgr, &cancelled, None);
}

#[test]
fn test_did_close_evicts_overlay() {
    let mut lifecycle = LifecycleManager::new();
    let mut multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();

    let uri = "file:///tmp/close_test.rs";
    let file = uri_to_path(uri);

    dispatch_msg(
        json!({
            "jsonrpc": "2.0", "method": "textDocument/didOpen",
            "params": { "textDocument": { "uri": uri, "version": 1, "text": "fn temp() {}\n" } }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
    );
    assert!(overlay_mgr.get(&file).is_some());

    dispatch_msg(
        json!({
            "jsonrpc": "2.0", "method": "textDocument/didClose",
            "params": { "textDocument": { "uri": uri } }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
    );
    assert!(overlay_mgr.get(&file).is_none());
    assert_eq!(overlay_mgr.open_count(), 0);
}

#[test]
fn test_did_close_unopened_document_is_noop() {
    let mut lifecycle = LifecycleManager::new();
    let mut multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();

    dispatch_msg(
        json!({
            "jsonrpc": "2.0", "method": "textDocument/didClose",
            "params": { "textDocument": { "uri": "file:///tmp/never_opened.rs" } }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
    );
    assert_eq!(overlay_mgr.open_count(), 0);
}

#[test]
fn test_did_close_invalidates_position_converter() {
    let mut lifecycle = LifecycleManager::new();
    let mut multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();

    let uri = "file:///tmp/inval_close.rs";
    let file = uri_to_path(uri);

    dispatch_msg(
        json!({
            "jsonrpc": "2.0", "method": "textDocument/didClose",
            "params": { "textDocument": { "uri": uri } }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
    );
    assert_eq!(lifecycle.conv.get_line(&file, 1), None);
}

#[test]
fn test_did_save_updates_disk_sync_state() {
    let temp_dir = std::env::temp_dir().join(format!("ridx_save_{}", std::process::id()));
    let _ = fs::create_dir_all(&temp_dir);
    let _guard = CleanupDir(temp_dir.clone());

    let file_path = temp_dir.join("saved_file.rs");
    fs::write(&file_path, "content on disk v1\n").unwrap();
    let uri = path_to_uri(file_path.to_str().unwrap());

    let mut lifecycle = LifecycleManager::new();
    let mut multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();

    dispatch_msg(
        json!({
            "jsonrpc": "2.0", "method": "textDocument/didOpen",
            "params": { "textDocument": { "uri": &uri, "version": 1, "text": "content in buffer v2\n" } }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
    );

    let file_str = file_path.to_str().unwrap();
    assert!(!overlay_mgr.get(file_str).unwrap().is_synced_with_disk());

    // Write buffer v2 to disk then send didSave
    fs::write(&file_path, "content in buffer v2\n").unwrap();
    dispatch_msg(
        json!({
            "jsonrpc": "2.0", "method": "textDocument/didSave",
            "params": { "textDocument": { "uri": &uri } }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
    );

    assert!(overlay_mgr.get(file_str).unwrap().is_synced_with_disk());
}

#[test]
fn test_did_save_triggers_flycheck() {
    let temp_dir = std::env::temp_dir().join(format!("ridx_flycheck_save_{}", std::process::id()));
    let _ = fs::create_dir_all(temp_dir.join("src"));
    let _guard = CleanupDir(temp_dir.clone());

    fs::write(temp_dir.join("Cargo.toml"), "[package]\nname = \"dummy\"\nversion = \"0.1.0\"\nedition = \"2021\"\n").unwrap();
    let lib_rs = temp_dir.join("src/lib.rs");
    fs::write(&lib_rs, "pub fn check_me() {}\n").unwrap();
    let uri = path_to_uri(lib_rs.to_str().unwrap());

    let mut lifecycle = LifecycleManager::new();
    let mut multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();

    let gen_before = lifecycle.flycheck.current_generation();
    dispatch_msg(
        json!({
            "jsonrpc": "2.0", "method": "textDocument/didSave",
            "params": { "textDocument": { "uri": &uri } }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
    );

    // Generation must have incremented on save trigger
    assert_eq!(lifecycle.flycheck.current_generation(), gen_before + 1);
}

#[test]
fn test_overlay_manager_open_count_and_file_paths() {
    let mut overlay_mgr = OverlayManager::new();
    assert_eq!(overlay_mgr.open_count(), 0);

    overlay_mgr.open_document("file:///f1.rs", "/f1.rs", 1, "fn a() {}".to_string());
    overlay_mgr.open_document("file:///f2.rs", "/f2.rs", 1, "fn b() {}".to_string());
    overlay_mgr.open_document("file:///f3.rs", "/f3.rs", 1, "fn c() {}".to_string());

    assert_eq!(overlay_mgr.open_count(), 3);
    let paths = overlay_mgr.open_file_paths();
    assert!(paths.contains(&"/f1.rs".to_string()));
    assert!(paths.contains(&"/f2.rs".to_string()));
    assert!(paths.contains(&"/f3.rs".to_string()));

    overlay_mgr.close_document("/f2.rs");
    assert_eq!(overlay_mgr.open_count(), 2);
    assert!(!overlay_mgr.open_file_paths().contains(&"/f2.rs".to_string()));
}

#[test]
fn test_overlay_rename_document_path() {
    let mut overlay_mgr = OverlayManager::new();
    overlay_mgr.open_document("file:///old/path.rs", "/old/path.rs", 1, "fn renamed() {}".to_string());

    overlay_mgr.rename("/old/path.rs", "file:///new/path.rs", "/new/path.rs");
    assert!(overlay_mgr.get("/old/path.rs").is_none());
    let new_ov = overlay_mgr.get("/new/path.rs").expect("must exist under new path");
    assert_eq!(new_ov.uri, "file:///new/path.rs");
    assert_eq!(new_ov.text, "fn renamed() {}");
}
