/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! A loaded index asks the client to fetch again what it shows, once, and never after `shutdown`.

use crate::lsp::dispatch::{load_finished_index, reload_changed_index};
use crate::lsp::lifecycle::LifecycleManager;
use crate::lsp::workspace::MultiRootWorkspace;
use crate::overlay::OverlayManager;
use ridx_format::writer::IndexWriter;
use serde_json::json;
use std::path::Path;

fn write_index(path: &Path, file: &str, hash: u64) {
    let mut w = IndexWriter::new("demo", "0.1.0");
    w.get_or_insert_file(file);
    w.add_file_hash(file, hash);
    w.write_to(&mut std::fs::File::create(path).unwrap()).unwrap();
}

fn refreshes(out: &[u8]) -> usize {
    String::from_utf8_lossy(out).matches("workspace/inlayHint/refresh").count()
}

/// Review of aef1571e: an index written by `ridx index` by hand reloaded without telling the client, and a reindex
/// ending between `shutdown` and `exit` still wrote requests.
#[test]
fn test_reloaded_index_refreshes_the_client_once_and_not_after_shutdown() {
    let dir = tempfile::tempdir().unwrap();
    let (src, idx) = (dir.path().join("lib.rs"), dir.path().join("index.ridx"));
    std::fs::write(&src, "pub fn a() {}\n").unwrap();
    write_index(&idx, src.to_str().unwrap(), 1);

    let mut lifecycle = LifecycleManager::new();
    let init = json!({ "params": { "capabilities": { "workspace": { "inlayHint": { "refreshSupport": true } } } } });
    lifecycle.handle_initialize(&init).unwrap();
    let mut mr = MultiRootWorkspace::new();
    mr.add_root(idx.clone());
    let mut ov = OverlayManager::new();

    let mut out = Vec::new();
    reload_changed_index(&mut out, &mut lifecycle, &mut mr, &mut ov).unwrap();
    assert_eq!(refreshes(&out), 0, "nothing changed");

    std::thread::sleep(std::time::Duration::from_millis(20));
    write_index(&idx, src.to_str().unwrap(), 2);
    reload_changed_index(&mut out, &mut lifecycle, &mut mr, &mut ov).unwrap();
    assert_eq!(refreshes(&out), 1, "a new index");
    reload_changed_index(&mut out, &mut lifecycle, &mut mr, &mut ov).unwrap();
    assert_eq!(refreshes(&out), 1, "loaded once");

    lifecycle.handle_shutdown().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    write_index(&idx, src.to_str().unwrap(), 3);
    let mut after = Vec::new();
    reload_changed_index(&mut after, &mut lifecycle, &mut mr, &mut ov).unwrap();
    assert_eq!(refreshes(&after), 0, "nothing goes out after shutdown");
}

/// In Zed on hl-order-book-server, opened without an index, the index `ridx index` then built was never read: the
/// root had not joined the workspace, and nothing reloads a root that is not there.
#[test]
fn test_a_first_index_joins_the_workspace_when_its_run_ends() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let src = root.join("src/lib.rs");
    std::fs::create_dir_all(src.parent().unwrap()).unwrap();
    std::fs::write(&src, "pub fn a() {}\n").unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"demo\"\n").unwrap();

    let mut lifecycle = LifecycleManager::new();
    let init = json!({ "params": { "capabilities": { "workspace": { "inlayHint": { "refreshSupport": true } } } } });
    lifecycle.handle_initialize(&init).unwrap();
    let mut mr = MultiRootWorkspace::new();
    mr.open_from_folders(std::slice::from_ref(&root));
    assert!(mr.is_empty(), "no index yet: the root cannot be opened");
    let mut ov = OverlayManager::new();
    let file = src.to_str().unwrap();
    ov.open_with_source_hash(&format!("file://{file}"), file, 0, "pub fn a() {}\n".to_string(), mr.source_hash_for_file(file));
    assert!(ov.get(file).unwrap().compiled_text.is_none());
    assert!(ov.get(file).unwrap().unhashed_source, "no index records it yet");

    std::fs::create_dir_all(root.join("target/ridx")).unwrap();
    write_index(&root.join("target/ridx/demo.ridx"), file, ridx_format::normalized_source_hash(b"pub fn a() {}\n"));
    let mut out = Vec::new();
    load_finished_index(&mut out, &mut lifecycle, &mut mr, &mut ov, std::slice::from_ref(&root)).unwrap();
    assert_eq!(mr.len(), 1, "the first index is loaded");
    assert_eq!(refreshes(&out), 1, "and the client fetches again");
    // The E2E run on a copy of hl-order-book-server: the file opened before the index still answered nothing.
    assert_eq!(ov.get(file).unwrap().compiled_text.as_deref(), Some("pub fn a() {}\n"), "a buffer opened before takes the index as its base");
    assert!(!ov.get(file).unwrap().unhashed_source, "review of 30e62913: a workspace file again, not a library source");
    load_finished_index(&mut out, &mut lifecycle, &mut mr, &mut ov, std::slice::from_ref(&root)).unwrap();
    assert_eq!((mr.len(), refreshes(&out)), (1, 1), "a root is added once");
}
