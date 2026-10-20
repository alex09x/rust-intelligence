/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! `IndexReader::find_file_id` names one stored file: the same string or the same path by
//! components. A longer path that merely ends with a stored one (another crate's `src/lib.rs`), a
//! shorter one, or a name that ends with the stored name mid-component must not match.

use super::IndexReader;
use crate::writer::IndexWriter;
use crate::{DefKind, RawDefinition};

fn reader_with_files(files: &[&str]) -> IndexReader<memmap2::Mmap> {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.keep().join("files.ridx");
    let mut writer = IndexWriter::new("files_crate", "0.1.0");
    for (i, file) in files.iter().enumerate() {
        writer.add_definition(RawDefinition {
            def_id: i as u32 + 1,
            kind: DefKind::Fn,
            visibility: 1,
            is_macro_expansion: false,
            name: format!("item_{i}"),
            path: format!("files_crate::item_{i}"),
            signature: None,
            file: file.to_string(),
            start_line: 1,
            start_col: 1,
            end_line: 1,
            end_col: 10,
        });
    }
    let mut f = std::fs::File::create(&path).unwrap();
    writer.write_to(&mut f).unwrap();
    drop(f);
    IndexReader::open(&path).unwrap()
}

#[test]
fn test_find_file_id_matches_only_the_stored_path() {
    let reader = reader_with_files(&["src/lib.rs", "src/mylib.rs"]);
    let lib = reader
        .find_file_id("src/lib.rs")
        .expect("stored path is found");
    assert_eq!(reader.get_file(lib), "src/lib.rs");
    assert_eq!(
        reader.get_file(reader.find_file_id("src/mylib.rs").unwrap()),
        "src/mylib.rs"
    );
    assert_eq!(
        reader.find_file_id("/ws/other/src/lib.rs"),
        None,
        "another crate's absolute path ends with src/lib.rs but is not this file"
    );
    assert_eq!(
        reader.find_file_id("lib.rs"),
        None,
        "a shorter path is not the stored file"
    );
    assert_eq!(
        reader.find_file_id("ylib.rs"),
        None,
        "a mid-component tail is not the stored file"
    );
    assert_eq!(reader.find_file_id("src/li"), None);
    assert_eq!(reader.find_file_id(""), None);
}

#[test]
fn test_find_file_id_accepts_the_same_path_spelled_differently() {
    let reader = reader_with_files(&["src/lib.rs"]);
    let lib = reader.find_file_id("src/lib.rs").unwrap();
    assert_eq!(
        reader.find_file_id("./src/lib.rs"),
        Some(lib),
        "leading ./ names the same file"
    );
    assert_eq!(
        reader.find_file_id("src//lib.rs"),
        Some(lib),
        "a doubled separator names the same file"
    );
    assert_eq!(reader.find_file_id("src/./lib.rs"), Some(lib));
}

#[test]
fn test_find_file_id_resolves_parent_dir_components_of_an_included_file() {
    // rust-analyzer's parser pulls its generated tests in with
    // `include!("../test_data/generated/runner.rs")`; rustc stores the path as spelled.
    let reader = reader_with_files(&[
        "crates/parser/src/../test_data/generated/runner.rs",
        "crates/parser/src/lib.rs",
    ]);
    let runner = reader
        .find_file_id("crates/parser/test_data/generated/runner.rs")
        .expect("the resolved path names the stored file");
    assert_eq!(
        reader.get_file(runner),
        "crates/parser/src/../test_data/generated/runner.rs"
    );
    assert_eq!(
        reader.find_file_id("crates/parser/src/../test_data/generated/runner.rs"),
        Some(runner),
        "the stored spelling still matches"
    );
    assert_eq!(
        reader.find_file_id("crates/parser/src/lib.rs"),
        Some(1),
        "the plain sibling is unaffected"
    );
    assert_eq!(
        reader.find_file_id("crates/parser/src/x/../lib.rs"),
        None,
        "a query spelled with .. is left to the filesystem-aware workspace lookup"
    );
    assert_eq!(
        reader.find_file_id("test_data/generated/runner.rs"),
        None,
        "a shorter path is still not the stored file"
    );
    assert_eq!(
        reader.find_file_id("crates/other/test_data/generated/runner.rs"),
        None,
        "another crate's path with the same tail does not match"
    );
}

#[test]
fn test_find_file_id_on_workspace_relative_and_sysroot_forms() {
    let reader = reader_with_files(&["serde/src/lib.rs", "library/core/src/lib.rs"]);
    assert_eq!(
        reader.get_file(reader.find_file_id("serde/src/lib.rs").unwrap()),
        "serde/src/lib.rs"
    );
    assert_eq!(
        reader.get_file(reader.find_file_id("library/core/src/lib.rs").unwrap()),
        "library/core/src/lib.rs"
    );
    assert_eq!(
        reader.find_file_id("src/lib.rs"),
        None,
        "the stored paths carry their crate prefix"
    );
    assert_eq!(
        reader.find_file_id("/ws/serde/src/lib.rs"),
        None,
        "absolute paths are resolved by the workspace, not here"
    );
}
