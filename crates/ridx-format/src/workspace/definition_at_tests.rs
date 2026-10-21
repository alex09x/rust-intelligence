/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! The workspace-level definition lookups apply the same precedence as the reader: a definition
//! at the cursor inside an enclosing reference's span answers instead of the reference.

use super::WorkspaceIndex;
use crate::format::DefKind;
use crate::reader::cursor_precedence_tests::write_fixture;
use std::path::PathBuf;

static FIXTURE_COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn workspace() -> (PathBuf, WorkspaceIndex) {
    let n = FIXTURE_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_ws_cursor_prec_{}_{n}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    write_fixture(&dir);
    let ws = WorkspaceIndex::open(&dir).unwrap();
    (dir, ws)
}

#[test]
fn workspace_definition_on_an_item_inside_a_macro_invocation_answers_the_item() {
    let (dir, ws) = workspace();
    let at_name = ws.find_definition_at("src/lib.rs", 5, 14).unwrap();
    assert_eq!((at_name.path, at_name.kind), ("fx::socket", DefKind::Fn));
    assert_eq!((at_name.start_line, at_name.start_col), (5, 12));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn reader_fallback_with_an_enclosing_reference_answers_the_item_inside_it() {
    let (dir, ws) = workspace();
    let reader = &ws.readers()[0];
    let file_id = reader.find_file_id("src/lib.rs").unwrap();
    let inner = reader
        .find_innermost_ref_at_file_id(file_id, 5, 14)
        .unwrap();
    assert_eq!(
        reader.get_str(inner.target_path_offset()),
        "alloc::sync::Arc"
    );
    let at_name = ws
        .find_def_at_in_reader(0, file_id, 5, 14, Some(inner))
        .unwrap();
    assert_eq!((at_name.path, at_name.kind), ("fx::socket", DefKind::Fn));
    // On no item the reference answers.
    let inner = reader
        .find_innermost_ref_at_file_id(file_id, 20, 1)
        .unwrap();
    let through_ref = ws
        .find_def_at_in_reader(0, file_id, 20, 1, Some(inner))
        .unwrap();
    assert_eq!(through_ref.path, "alloc::sync::Arc");
    let _ = std::fs::remove_dir_all(dir);
}
