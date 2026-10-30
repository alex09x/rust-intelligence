/*
 * rust-intelligence — Compiler-backed, mmap-indexed code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

#![allow(dead_code)]

use ridx_format::{DefKind, IndexWriter, RawDefinition};
use std::fs::{self, File};
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static TEST_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Returns a unique, clean temporary directory for a test, automatically created.
pub fn unique_temp_dir(prefix: &str) -> PathBuf {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!("{}_{}_{}", prefix, std::process::id(), id));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).unwrap();
    p
}

/// Helper to build and write a simple .ridx index with a single function definition.
pub fn write_simple_index(crate_name: &str, crate_version: &str, unit_key: Option<&str>, file: &str, def_name: &str, def_path: &str, out_path: &Path) {
    if let Some(parent) = out_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let mut writer = IndexWriter::new(crate_name, crate_version);
    if let Some(uk) = unit_key {
        writer.set_unit_key(uk);
    }
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: def_name.to_string(),
        path: def_path.to_string(),
        signature: Some("fn()".to_string()),
        file: file.to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 5,
        end_col: 2,
    });
    let f = File::create(out_path).unwrap();
    writer.write_to(&mut BufWriter::new(f)).unwrap();
}
