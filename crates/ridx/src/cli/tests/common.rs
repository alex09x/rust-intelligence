/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Common test fixtures and mock builders for CLI tests.

use std::fs;
use std::path::PathBuf;

pub(crate) struct TestDir(pub(crate) PathBuf);
impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub(crate) fn lock_cli_test() -> std::sync::MutexGuard<'static, ()> {
    crate::indexer::tests::lock_indexer_test()
}

pub(crate) fn new_test_dir(prefix: &str) -> TestDir {
    static CNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let id = CNT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let path = std::env::temp_dir().join(format!("{}_{}_{}", prefix, std::process::id(), id));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    TestDir(path)
}

pub(crate) fn create_indexed_workspace() -> (TestDir, String) {
    use ridx_format::{DefKind, IndexWriter, RawDefinition, RawReference, RawType};

    let test_dir = new_test_dir("ridx_cli_ws");
    let index_file = test_dir.0.join("test_crate.ridx");

    let mut writer = IndexWriter::new("test_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "test_fn".to_string(),
        path: "test_crate::test_fn".to_string(),
        signature: Some("fn()".to_string()),
        file: "src/lib.rs".to_string(),
        start_line: 10,
        start_col: 1,
        end_line: 12,
        end_col: 2,
    });

    writer.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::Const,
        visibility: 0,
        is_macro_expansion: false,
        name: "NO_SIG".to_string(),
        path: "test_crate::NO_SIG".to_string(),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: 15,
        start_col: 1,
        end_line: 15,
        end_col: 10,
    });

    writer.add_reference(RawReference {
        file: "src/main.rs".to_string(),
        start_line: 20,
        start_col: 5,
        end_line: 20,
        end_col: 12,
        target_def_id: Some(1),
        target_path: "test_crate::test_fn".to_string(),
        target_unit: None,
        target_file: Some("src/lib.rs".to_string()),
        target_line: 10,
        target_col: 1,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });

    writer.add_type(RawType {
        file: "src/lib.rs".to_string(),
        start_line: 10,
        start_col: 1,
        end_line: 10,
        end_col: 8,
        type_name: "fn()".to_string(),
    });

    let mut file = fs::File::create(&index_file).unwrap();
    writer.write_to(&mut file).unwrap();

    let dir_str = test_dir.0.display().to_string();
    (test_dir, dir_str)
}
