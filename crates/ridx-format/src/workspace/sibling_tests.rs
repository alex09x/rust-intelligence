/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Unit tests for sibling workspace units (lib and lib test) reference routing and deduplication.

use super::*;
use crate::format::DefKind;
use crate::reader::IndexReader;
use crate::writer::{IndexWriter, RawDefinition, RawReference};
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::path::{Path, PathBuf};

#[allow(clippy::type_complexity)]
fn create_test_workspace(
    readers_info: Vec<(
        &Path,
        Option<PathBuf>,
        Option<String>,
        Option<TargetKind>,
        bool,
    )>,
    workspace_root: PathBuf,
) -> WorkspaceIndex {
    let mut readers = Vec::with_capacity(readers_info.len());
    let mut paths = Vec::with_capacity(readers_info.len());
    let mut mtimes = Vec::with_capacity(readers_info.len());
    let mut source_roots = Vec::with_capacity(readers_info.len());
    let mut unit_keys = Vec::with_capacity(readers_info.len());
    let mut target_kinds = Vec::with_capacity(readers_info.len());
    let mut reader_generations = Vec::with_capacity(readers_info.len());
    let mut reader_dependencies = Vec::with_capacity(readers_info.len());
    let mut reader_extern_units = Vec::with_capacity(readers_info.len());
    let mut reader_file_hashes = Vec::with_capacity(readers_info.len());
    let mut reader_is_dependency = Vec::with_capacity(readers_info.len());
    let mut meta_paths = Vec::with_capacity(readers_info.len());
    let mut meta_mtimes = Vec::with_capacity(readers_info.len());

    for (p, sroot, ukey, tkind, is_dep) in readers_info {
        readers.push(IndexReader::open(p).expect("failed to open test index"));
        paths.push(p.to_path_buf());
        mtimes.push(None);
        source_roots.push(sroot);
        unit_keys.push(ukey);
        target_kinds.push(tkind);
        reader_generations.push(None);
        reader_dependencies.push(Vec::new());
        reader_extern_units.push(HashMap::new());
        reader_file_hashes.push(HashMap::new());
        reader_is_dependency.push(is_dep);
        meta_paths.push(None);
        meta_mtimes.push(None);
    }

    let unit_match = UnitMatchIndex::build(&readers, &unit_keys, &reader_extern_units);
    WorkspaceIndex {
        readers,
        paths,
        mtimes,
        source_roots,
        unit_keys,
        unit_match,
        target_kinds,
        reader_generations,
        reader_dependencies,
        reader_extern_units,
        reader_file_hashes,
        reader_is_dependency,
        meta_paths,
        meta_mtimes,
        last_reloaded_files: HashSet::new(),
        root_dir: None,
        workspace_root: Some(workspace_root),
        manifest_path: None,
        manifest_mtime: None,
        manifest_len: None,
        manifest_hash: None,
        manifest_generation: None,
        manifest_ino: None,
        active_variant_id: None,
        sysroot_cas_key: None,
        instance_id: 1,
        generation: 1,
    }
}

#[test]
fn test_sibling_units_reference_only_in_test_shard() {
    let temp_dir = tempfile::tempdir().unwrap();
    let ws_root = temp_dir.path().to_path_buf();
    let pkg_root = ws_root.join("crates/my_pkg");

    // 1. Lib shard: contains definition of `compute`
    let lib_idx_path = temp_dir.path().join("my_pkg_lib.ridx");
    let mut lib_writer = IndexWriter::new("my_pkg", "0.1.0");
    lib_writer.set_unit_key("unit_lib");
    lib_writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "compute".to_string(),
        path: "my_pkg::compute".to_string(),
        signature: Some("fn(i32) -> i32".to_string()),
        file: "src/lib.rs".to_string(),
        start_line: 1,
        start_col: 8,
        end_line: 1,
        end_col: 15,
    });
    let mut f_lib = File::create(&lib_idx_path).unwrap();
    lib_writer.write_to(&mut f_lib).unwrap();
    drop(f_lib);

    // 2. Lib test shard: same package, reference ONLY in a test file (e.g. tests/test_run.rs)
    let test_idx_path = temp_dir.path().join("my_pkg_test.ridx");
    let mut test_writer = IndexWriter::new("my_pkg", "0.1.0");
    test_writer.set_unit_key("unit_test");
    test_writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "compute".to_string(),
        path: "my_pkg::compute".to_string(),
        signature: Some("fn(i32) -> i32".to_string()),
        file: "src/lib.rs".to_string(),
        start_line: 1,
        start_col: 8,
        end_line: 1,
        end_col: 15,
    });
    test_writer.add_reference(RawReference {
        file: "src/runtime/tests/inject.rs".to_string(),
        start_line: 10,
        start_col: 5,
        end_line: 10,
        end_col: 12,
        target_def_id: Some(1),
        target_path: "my_pkg::compute".to_string(),
        target_file: Some("src/lib.rs".to_string()),
        target_unit: None,
        target_line: 1,
        target_col: 8,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    let mut f_test = File::create(&test_idx_path).unwrap();
    test_writer.write_to(&mut f_test).unwrap();
    drop(f_test);

    let ws = create_test_workspace(
        vec![
            (
                &lib_idx_path,
                Some(pkg_root.clone()),
                Some("unit_lib".to_string()),
                Some(TargetKind::Lib),
                false,
            ),
            (
                &test_idx_path,
                Some(pkg_root.clone()),
                Some("unit_test".to_string()),
                Some(TargetKind::Test),
                false,
            ),
        ],
        ws_root,
    );

    let refs = ws.find_references_for_unit("my_pkg::compute", Some("unit_lib"));
    assert_eq!(
        refs.len(),
        1,
        "Reference in sibling test shard must be found when querying unit_lib"
    );
    assert_eq!(refs[0].file, "src/runtime/tests/inject.rs");
    assert_eq!(refs[0].span.start_line(), 10);
    assert_eq!(refs[0].span.start_col(), 5);
}

#[test]
fn test_sibling_units_reference_in_common_file_deduplicated() {
    let temp_dir = tempfile::tempdir().unwrap();
    let ws_root = temp_dir.path().to_path_buf();
    let pkg_root = ws_root.join("crates/my_pkg");

    // 1. Lib shard: definition and reference in common file `src/lib.rs`
    let lib_idx_path = temp_dir.path().join("my_pkg_lib.ridx");
    let mut lib_writer = IndexWriter::new("my_pkg", "0.1.0");
    lib_writer.set_unit_key("unit_lib");
    lib_writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "compute".to_string(),
        path: "my_pkg::compute".to_string(),
        signature: Some("fn(i32) -> i32".to_string()),
        file: "src/lib.rs".to_string(),
        start_line: 1,
        start_col: 8,
        end_line: 1,
        end_col: 15,
    });
    lib_writer.add_reference(RawReference {
        file: "src/lib.rs".to_string(),
        start_line: 5,
        start_col: 10,
        end_line: 5,
        end_col: 17,
        target_def_id: Some(1),
        target_path: "my_pkg::compute".to_string(),
        target_file: Some("src/lib.rs".to_string()),
        target_unit: None,
        target_line: 1,
        target_col: 8,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    let mut f_lib = File::create(&lib_idx_path).unwrap();
    lib_writer.write_to(&mut f_lib).unwrap();
    drop(f_lib);

    // 2. Lib test shard: same package, duplicates definition and reference in `src/lib.rs`
    let test_idx_path = temp_dir.path().join("my_pkg_test.ridx");
    let mut test_writer = IndexWriter::new("my_pkg", "0.1.0");
    test_writer.set_unit_key("unit_test");
    test_writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "compute".to_string(),
        path: "my_pkg::compute".to_string(),
        signature: Some("fn(i32) -> i32".to_string()),
        file: "src/lib.rs".to_string(),
        start_line: 1,
        start_col: 8,
        end_line: 1,
        end_col: 15,
    });
    test_writer.add_reference(RawReference {
        file: "src/lib.rs".to_string(),
        start_line: 5,
        start_col: 10,
        end_line: 5,
        end_col: 17,
        target_def_id: Some(1),
        target_path: "my_pkg::compute".to_string(),
        target_file: Some("src/lib.rs".to_string()),
        target_unit: None,
        target_line: 1,
        target_col: 8,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    let mut f_test = File::create(&test_idx_path).unwrap();
    test_writer.write_to(&mut f_test).unwrap();
    drop(f_test);

    let ws = create_test_workspace(
        vec![
            (
                &lib_idx_path,
                Some(pkg_root.clone()),
                Some("unit_lib".to_string()),
                Some(TargetKind::Lib),
                false,
            ),
            (
                &test_idx_path,
                Some(pkg_root.clone()),
                Some("unit_test".to_string()),
                Some(TargetKind::Test),
                false,
            ),
        ],
        ws_root,
    );

    let refs = ws.find_references_for_unit("my_pkg::compute", Some("unit_lib"));
    assert_eq!(
        refs.len(),
        1,
        "Reference in common file src/lib.rs present in both lib and lib test must be deduplicated to exactly one"
    );
    assert_eq!(refs[0].file, "src/lib.rs");
    assert_eq!(refs[0].span.start_line(), 5);
    assert_eq!(refs[0].span.start_col(), 10);
}

#[test]
fn test_unrelated_package_with_same_crate_name_is_excluded() {
    let temp_dir = tempfile::tempdir().unwrap();
    let ws_root = temp_dir.path().to_path_buf();
    let pkg_root_a = ws_root.join("packages/pkg_a");
    let pkg_root_b = ws_root.join("packages/pkg_b");

    // 1. Package A (lib): defines `my_pkg::compute`
    let lib_a_path = temp_dir.path().join("pkg_a.ridx");
    let mut writer_a = IndexWriter::new("my_pkg", "0.1.0");
    writer_a.set_unit_key("unit_pkg_a");
    writer_a.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "compute".to_string(),
        path: "my_pkg::compute".to_string(),
        signature: Some("fn(i32) -> i32".to_string()),
        file: "src/lib.rs".to_string(),
        start_line: 1,
        start_col: 8,
        end_line: 1,
        end_col: 15,
    });
    let mut f_a = File::create(&lib_a_path).unwrap();
    writer_a.write_to(&mut f_a).unwrap();
    drop(f_a);

    // 2. Package B: distinct package, same crate name, different source_root
    // Has a reference to `my_pkg::compute`
    let lib_b_path = temp_dir.path().join("pkg_b.ridx");
    let mut writer_b = IndexWriter::new("my_pkg", "0.2.0");
    writer_b.set_unit_key("unit_pkg_b");
    writer_b.add_reference(RawReference {
        file: "src/other.rs".to_string(),
        start_line: 20,
        start_col: 4,
        end_line: 20,
        end_col: 11,
        target_def_id: None,
        target_path: "my_pkg::compute".to_string(),
        target_file: None,
        target_unit: None,
        target_line: 0,
        target_col: 0,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    let mut f_b = File::create(&lib_b_path).unwrap();
    writer_b.write_to(&mut f_b).unwrap();
    drop(f_b);

    let ws = create_test_workspace(
        vec![
            (
                &lib_a_path,
                Some(pkg_root_a),
                Some("unit_pkg_a".to_string()),
                Some(TargetKind::Lib),
                false,
            ),
            (
                &lib_b_path,
                Some(pkg_root_b),
                Some("unit_pkg_b".to_string()),
                Some(TargetKind::Lib),
                false,
            ),
        ],
        ws_root,
    );

    let refs = ws.find_references_for_unit("my_pkg::compute", Some("unit_pkg_a"));
    assert_eq!(
        refs.len(),
        0,
        "Distinct package B with the same crate_name but different source_root must NOT be treated as sibling"
    );
}
