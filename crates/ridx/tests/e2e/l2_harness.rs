/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Compiler-grounded L2 test harness: indexes real fixture workspaces with `ridx-driver`
//! and caches the resulting `WorkspaceIndex` instances for millisecond-fast test runs.

use ridx::indexer::{try_run_indexing, IndexOptions};
use ridx_format::WorkspaceIndex;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

static L2_CACHE: OnceLock<Mutex<HashMap<String, &'static WorkspaceIndex>>> = OnceLock::new();

fn get_cache() -> &'static Mutex<HashMap<String, &'static WorkspaceIndex>> {
    L2_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Locates the root of the rust-intelligence repository.
pub fn repo_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .unwrap_or(manifest_dir)
}

/// Returns the path to the fixture directory.
pub fn fixture_dir(name: &str) -> PathBuf {
    repo_root().join("crates/ridx/tests/fixtures").join(name)
}

/// Dynamic text position resolver: locates `needle` inside `snippet` in `file`,
/// returning exact 0-based line and 0-based UTF-16 character column.
pub fn pos_in_file(file: &Path, snippet: &str, needle: &str) -> (u32, u32) {
    let content = std::fs::read_to_string(file)
        .unwrap_or_else(|e| panic!("Failed to read file {:?} for position lookup: {e}", file));
    pos_in_str(&content, snippet, needle)
}

static DRIVER_BIN: OnceLock<PathBuf> = OnceLock::new();

/// Ensures `ridx-driver` is compiled fresh for L2 tests (built once per process).
pub fn ensure_fresh_driver() -> PathBuf {
    DRIVER_BIN
        .get_or_init(|| {
            let root = repo_root();
            let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
            let base_target = std::env::var_os("CARGO_TARGET_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| root.join("target"));
            let target_dir = base_target.join("ridx-l2-driver");
            let status = std::process::Command::new(cargo)
                .args(["build", "-p", "ridx-driver", "--target-dir"])
                .arg(&target_dir)
                .current_dir(&root)
                .status()
                .expect("Failed to execute cargo build -p ridx-driver");
            assert!(status.success(), "Failed to build ridx-driver for L2 tests");
            let bin = target_dir.join("debug/ridx-driver");
            assert!(bin.is_file(), "ridx-driver binary not found at {:?}", bin);
            bin
        })
        .clone()
}

/// Dynamic text position resolver on a string buffer.
pub fn pos_in_str(content: &str, snippet: &str, needle: &str) -> (u32, u32) {
    let first = content
        .find(snippet)
        .unwrap_or_else(|| panic!("snippet {:?} not found in content", snippet));
    let last = content.rfind(snippet).unwrap();
    assert_eq!(
        first, last,
        "snippet {:?} is not unique in content",
        snippet
    );
    let needle_offset = snippet
        .find(needle)
        .unwrap_or_else(|| panic!("needle {:?} not found in snippet {:?}", needle, snippet));
    let abs_byte = first + needle_offset;

    let before = &content[..abs_byte];
    let line = before
        .lines()
        .count()
        .saturating_sub(if before.ends_with('\n') { 0 } else { 1 }) as u32;
    let line_start_byte = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
    let line_slice = &before[line_start_byte..];
    let col = line_slice.encode_utf16().count() as u32;

    (line, col)
}

/// Ensures the fixture is indexed with `ridx-driver` and returns a reference to its `WorkspaceIndex`.
pub fn get_or_index_l2_fixture(name: &str) -> &'static WorkspaceIndex {
    let mut cache = get_cache().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(&ws) = cache.get(name) {
        return ws;
    }

    let fix_dir = fixture_dir(name);
    assert!(
        fix_dir.is_dir(),
        "L2 fixture directory not found: {:?}",
        fix_dir
    );

    // Isolate test cache away from ~/.cache/ridx/v2/objects without modifying global process env
    let test_cache_dir = repo_root().join("target/ridx-test-cache");
    let _ = std::fs::create_dir_all(&test_cache_dir);

    let driver_bin = ensure_fresh_driver();

    let opts = IndexOptions {
        path: fix_dir.clone(),
        out_dir_opt: None,
        package: None,
        release: false,
        features: None,
        no_default_features: false,
        all_features: false,
        target: None,
        cache_dir: Some(test_cache_dir.clone()),
        driver_bin: Some(driver_bin),
    };

    try_run_indexing(opts).unwrap_or_else(|e| {
        panic!("Indexing L2 fixture {name} failed: {e}");
    });

    let mut ws = WorkspaceIndex::open(&fix_dir).unwrap_or_else(|e| {
        panic!(
            "Failed to open indexed WorkspaceIndex at {:?}: {e}",
            fix_dir
        );
    });

    if !ws.has_sysroot() {
        let cas_key_opt = ws.sysroot_cas_key().map(str::to_string);
        if let Some(ref cas_key) = cas_key_opt {
            ws.attach_sysroot_shards_for_key_in(&test_cache_dir.join("sysroot"), cas_key);
        }
        if !ws.has_sysroot() {
            if let Ok(entries) = std::fs::read_dir(test_cache_dir.join("sysroot")) {
                for entry in entries.flatten() {
                    if entry.path().is_dir() {
                        if let Some(key) = entry.file_name().to_str() {
                            ws.attach_sysroot_shards_for_key_in(
                                &test_cache_dir.join("sysroot"),
                                key,
                            );
                            if ws.has_sysroot() {
                                break;
                            }
                        }
                    }
                }
            }
        }
    }

    if !ws.has_sysroot() {
        panic!("L2 fixture {name}: driver did not build or attach real sysroot shards");
    }

    let leaked: &'static WorkspaceIndex = Box::leak(Box::new(ws));
    cache.insert(name.to_string(), leaked);
    leaked
}
