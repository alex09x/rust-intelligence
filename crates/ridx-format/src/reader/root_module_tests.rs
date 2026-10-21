/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! A position on no name has no definition. The driver records the crate root as a `Mod`
//! definition with an empty name and the span of the whole file; the fallback to the smallest
//! definition containing the cursor must not answer it.

use super::cursor_precedence_tests::def;
use super::*;
use crate::format::DefKind;
use crate::writer::IndexWriter;
use std::path::{Path, PathBuf};

/// Writes `fx.ridx` into `dir`: the nameless crate-root module spanning the file (1:1-60:2),
/// `fn alpha` with the name span 10:4-10:9, and an inline `mod inner` recorded with the span of its
/// block 40:1-50:2.
pub(crate) fn write_fixture(dir: &Path) -> PathBuf {
    let path = dir.join("fx.ridx");
    let mut w = IndexWriter::new("fx", "0.1.0");
    let mut root = def(1, DefKind::Mod, "", (1, 1, 60, 2));
    root.path = String::new();
    w.add_definition(root);
    w.add_definition(def(2, DefKind::Fn, "alpha", (10, 4, 10, 9)));
    w.add_definition(def(3, DefKind::Mod, "inner", (40, 1, 50, 2)));
    let mut f = std::fs::File::create(&path).unwrap();
    w.write_to(&mut f).unwrap();
    path
}

static FIXTURE_COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn fixture() -> (PathBuf, IndexReader<Mmap>) {
    let n = FIXTURE_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_root_module_{}_{n}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = write_fixture(&dir);
    let reader = IndexReader::open(&path).unwrap();
    (dir, reader)
}

#[test]
fn a_position_on_no_name_has_no_definition() {
    let (dir, reader) = fixture();
    // Line 30 holds no name and lies in no item: only the crate-root module contains it.
    assert_eq!(reader.find_definition_at("src/lib.rs", 30, 1), None);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_name_inside_the_crate_root_module_still_answers() {
    let (dir, reader) = fixture();
    let on_alpha = reader.find_definition_at("src/lib.rs", 10, 6).unwrap();
    assert_eq!((on_alpha.path, on_alpha.kind), ("fx::alpha", DefKind::Fn));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_named_module_recorded_with_its_block_span_still_answers_inside_it() {
    let (dir, reader) = fixture();
    let in_inner = reader.find_definition_at("src/lib.rs", 45, 1).unwrap();
    assert_eq!((in_inner.path, in_inner.kind), ("fx::inner", DefKind::Mod));
    let _ = std::fs::remove_dir_all(dir);
}
