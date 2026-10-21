/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! The workspace-level definition lookups never answer the nameless crate-root module either.

use super::WorkspaceIndex;
use crate::format::DefKind;
use crate::reader::root_module_tests::write_fixture;
use std::path::PathBuf;

static FIXTURE_COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn workspace() -> (PathBuf, WorkspaceIndex) {
    let n = FIXTURE_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_ws_root_module_{}_{n}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    write_fixture(&dir);
    let ws = WorkspaceIndex::open(&dir).unwrap();
    (dir, ws)
}

#[test]
fn workspace_definition_on_no_name_is_none() {
    let (dir, ws) = workspace();
    assert_eq!(ws.find_definition_at("src/lib.rs", 30, 1), None);
    let on_alpha = ws.find_definition_at("src/lib.rs", 10, 6).unwrap();
    assert_eq!((on_alpha.path, on_alpha.kind), ("fx::alpha", DefKind::Fn));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn reader_fallback_on_no_name_is_none() {
    let (dir, ws) = workspace();
    let file_id = ws.readers()[0].find_file_id("src/lib.rs").unwrap();
    assert!(ws.find_def_at_in_reader(0, file_id, 30, 1, None).is_none());
    let in_inner = ws.find_def_at_in_reader(0, file_id, 45, 1, None).unwrap();
    assert_eq!((in_inner.path, in_inner.kind), ("fx::inner", DefKind::Mod));
    let _ = std::fs::remove_dir_all(dir);
}
