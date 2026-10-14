/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Unit tests for sysroot reference routing and empty unit_key handling.

use super::*;
use crate::format::DefKind;
use crate::reader::IndexReader;
use crate::writer::{IndexWriter, RawDefinition, RawReference};
use std::fs::File;

#[test]
fn test_sysroot_references_without_unit_key_not_clobbered() {
    let temp_dir = tempfile::tempdir().unwrap();
    let ws_root = temp_dir.path().to_path_buf();

    // 1. Shard 1: core (sysroot, no unit key, crate="core")
    let core_idx_path = temp_dir.path().join("core.ridx");
    let mut core_writer = IndexWriter::new("core", "1.80.0");
    core_writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "panic".to_string(),
        path: "core::panic".to_string(),
        signature: None,
        file: "library/core/src/panicking.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 10,
    });
    let mut f_core = File::create(&core_idx_path).unwrap();
    core_writer.write_to(&mut f_core).unwrap();
    drop(f_core);

    // 2. Shard 2: alloc (sysroot, no unit key, crate="alloc")
    let alloc_idx_path = temp_dir.path().join("alloc.ridx");
    let mut alloc_writer = IndexWriter::new("alloc", "1.80.0");
    alloc_writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Struct,
        visibility: 0,
        is_macro_expansion: false,
        name: "Vec".to_string(),
        path: "alloc::vec::Vec".to_string(),
        signature: None,
        file: "library/alloc/src/vec/mod.rs".to_string(),
        start_line: 10,
        start_col: 1,
        end_line: 10,
        end_col: 15,
    });
    let mut f_alloc = File::create(&alloc_idx_path).unwrap();
    alloc_writer.write_to(&mut f_alloc).unwrap();
    drop(f_alloc);

    // 3. Shard 3: app crate (with unit key) referencing alloc::vec::Vec
    let app_idx_path = temp_dir.path().join("app.ridx");
    let mut app_writer = IndexWriter::new("app", "0.1.0");
    app_writer.set_unit_key("unit_app");
    app_writer.add_reference(RawReference {
        file: "src/main.rs".to_string(),
        start_line: 5,
        start_col: 10,
        end_line: 5,
        end_col: 13,
        target_def_id: None,
        target_path: "alloc::vec::Vec".to_string(),
        target_file: Some("library/alloc/src/vec/mod.rs".to_string()),
        target_unit: None,
        target_line: 10,
        target_col: 1,
        kind: DefKind::Struct,
        is_write: false,
        is_macro_expansion: false,
    });
    let mut f_app = File::create(&app_idx_path).unwrap();
    app_writer.write_to(&mut f_app).unwrap();
    drop(f_app);

    let ws = WorkspaceIndex {
        readers: vec![
            IndexReader::open(&core_idx_path).unwrap(),
            IndexReader::open(&alloc_idx_path).unwrap(),
            IndexReader::open(&app_idx_path).unwrap(),
        ],
        paths: vec![core_idx_path, alloc_idx_path, app_idx_path],
        mtimes: vec![None, None, None],
        source_roots: vec![None, None, Some(ws_root.clone())],
        unit_keys: vec![
            Some("".to_string()),
            Some("".to_string()),
            Some("unit_app".to_string()),
        ],
        unit_match: UnitMatchIndex::default(),
        path_index: super::PathIndex::default(),
        target_kinds: vec![None, None, Some(TargetKind::Bin)],
        reader_generations: vec![None, None, None],
        reader_dependencies: vec![Vec::new(), Vec::new(), vec!["alloc".to_string()]],
        reader_extern_units: vec![
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
        ],
        reader_file_hashes: vec![
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
        ],
        reader_is_dependency: vec![true, true, false],
        meta_paths: vec![None, None, None],
        meta_mtimes: vec![None, None, None],
        last_reloaded_files: std::collections::HashSet::new(),
        root_dir: None,
        workspace_root: Some(ws_root),
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
    };

    // Querying with Some("") (sentinel from definitions lacking unit_key) must NOT clobber target_crate to core
    let refs_empty_str = ws.find_references_for_unit("alloc::vec::Vec", Some(""));
    assert_eq!(
        refs_empty_str.len(),
        1,
        "Query with empty unit_key sentinel must preserve target_crate and locate app reference"
    );
    assert_eq!(refs_empty_str[0].file, "src/main.rs");

    // Querying with None must also locate the reference
    let refs_none = ws.find_references_for_unit("alloc::vec::Vec", None);
    assert_eq!(
        refs_none.len(),
        1,
        "Query with None unit_key must locate app reference"
    );
}

#[test]
fn test_workspace_references_exclude_dependency_and_sysroot_readers() {
    let temp_dir = tempfile::tempdir().unwrap();
    let ws_root = temp_dir.path().to_path_buf();

    // 1. Shard 1: alloc (sysroot, crate="alloc", reader_is_dependency=true)
    let alloc_idx_path = temp_dir.path().join("alloc.ridx");
    let mut alloc_writer = IndexWriter::new("alloc", "1.80.0");
    alloc_writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Struct,
        visibility: 0,
        is_macro_expansion: false,
        name: "Vec".to_string(),
        path: "alloc::vec::Vec".to_string(),
        signature: None,
        file: "library/alloc/src/vec/mod.rs".to_string(),
        start_line: 10,
        start_col: 1,
        end_line: 10,
        end_col: 15,
    });
    alloc_writer.add_reference(RawReference {
        file: "library/alloc/src/vec/mod.rs".to_string(),
        start_line: 25,
        start_col: 10,
        end_line: 25,
        end_col: 13,
        target_def_id: Some(1),
        target_path: "alloc::vec::Vec".to_string(),
        target_file: Some("library/alloc/src/vec/mod.rs".to_string()),
        target_unit: None,
        target_line: 10,
        target_col: 1,
        kind: DefKind::Struct,
        is_write: false,
        is_macro_expansion: false,
    });
    let mut f_alloc = File::create(&alloc_idx_path).unwrap();
    alloc_writer.write_to(&mut f_alloc).unwrap();
    drop(f_alloc);

    // 2. Shard 2: dep_crate (external dependency, crate="dep_crate", reader_is_dependency=true)
    let dep_idx_path = temp_dir.path().join("dep_crate.ridx");
    let mut dep_writer = IndexWriter::new("dep_crate", "0.1.0");
    dep_writer.set_unit_key("unit_dep");
    dep_writer.add_reference(RawReference {
        file: "src/dep_util.rs".to_string(),
        start_line: 15,
        start_col: 10,
        end_line: 15,
        end_col: 13,
        target_def_id: None,
        target_path: "alloc::vec::Vec".to_string(),
        target_file: Some("library/alloc/src/vec/mod.rs".to_string()),
        target_unit: None,
        target_line: 10,
        target_col: 1,
        kind: DefKind::Struct,
        is_write: false,
        is_macro_expansion: false,
    });
    let mut f_dep = File::create(&dep_idx_path).unwrap();
    dep_writer.write_to(&mut f_dep).unwrap();
    drop(f_dep);

    // 3. Shard 3: app crate (workspace crate, crate="app", reader_is_dependency=false)
    let app_idx_path = temp_dir.path().join("app.ridx");
    let mut app_writer = IndexWriter::new("app", "0.1.0");
    app_writer.set_unit_key("unit_app");
    app_writer.add_reference(RawReference {
        file: "src/main.rs".to_string(),
        start_line: 5,
        start_col: 10,
        end_line: 5,
        end_col: 13,
        target_def_id: None,
        target_path: "alloc::vec::Vec".to_string(),
        target_file: Some("library/alloc/src/vec/mod.rs".to_string()),
        target_unit: None,
        target_line: 10,
        target_col: 1,
        kind: DefKind::Struct,
        is_write: false,
        is_macro_expansion: false,
    });
    let mut f_app = File::create(&app_idx_path).unwrap();
    app_writer.write_to(&mut f_app).unwrap();
    drop(f_app);

    let ws = WorkspaceIndex {
        readers: vec![
            IndexReader::open(&alloc_idx_path).unwrap(),
            IndexReader::open(&dep_idx_path).unwrap(),
            IndexReader::open(&app_idx_path).unwrap(),
        ],
        paths: vec![alloc_idx_path, dep_idx_path, app_idx_path],
        mtimes: vec![None, None, None],
        source_roots: vec![
            None,
            Some(PathBuf::from("/cargo/registry/src/dep-0.1.0")),
            Some(ws_root.clone()),
        ],
        unit_keys: vec![
            Some("".to_string()),
            Some("unit_dep".to_string()),
            Some("unit_app".to_string()),
        ],
        unit_match: UnitMatchIndex::default(),
        path_index: super::PathIndex::default(),
        target_kinds: vec![None, Some(TargetKind::Lib), Some(TargetKind::Bin)],
        reader_generations: vec![None, None, None],
        reader_dependencies: vec![
            Vec::new(),
            vec!["alloc".to_string()],
            vec!["alloc".to_string(), "dep_crate".to_string()],
        ],
        reader_extern_units: vec![
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
        ],
        reader_file_hashes: vec![
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
        ],
        reader_is_dependency: vec![true, true, false],
        meta_paths: vec![None, None, None],
        meta_mtimes: vec![None, None, None],
        last_reloaded_files: std::collections::HashSet::new(),
        root_dir: None,
        workspace_root: Some(ws_root),
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
    };

    let refs = ws.find_references_for_unit("alloc::vec::Vec", None);
    assert_eq!(
        refs.len(),
        1,
        "Must return ONLY workspace references, excluding external dependency and sysroot references"
    );
    assert_eq!(refs[0].crate_name, "app");
    assert_eq!(refs[0].file, "src/main.rs");
}
