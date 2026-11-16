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

use crate::lsp::dispatch::reload_changed_index;
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
