/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use super::*;
use crate::reader::IndexReader;
use crate::writer::IndexWriter;
use crate::{DefKind, RawDefinition};
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::path::PathBuf;

#[test]
fn test_relative_path_anchored_to_workspace_root_beats_dependency_suffix_match() {
    let temp_dir = tempfile::tempdir().unwrap();

    // 1. Dependency crate shard: `dep_crate` with file at `/deps/dep_crate/src/lib.rs`
    let dep_ridx = temp_dir.path().join("dep_crate.ridx");
    let mut dep_writer = IndexWriter::new("dep_crate", "0.1.0");
    dep_writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Struct,
        visibility: 0,
        is_macro_expansion: false,
        name: "DepHelper".to_string(),
        path: "dep_crate::DepHelper".to_string(),
        signature: None,
        file: "/deps/dep_crate/src/lib.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 5,
        end_col: 2,
    });
    let mut f_dep = File::create(&dep_ridx).unwrap();
    dep_writer.write_to(&mut f_dep).unwrap();
    drop(f_dep);

    // 2. Workspace root crate shard: `ws_crate` with file at `/workspace/src/lib.rs`
    let ws_ridx = temp_dir.path().join("ws_crate.ridx");
    let mut ws_writer = IndexWriter::new("ws_crate", "0.1.0");
    ws_writer.add_definition(RawDefinition {
        def_id: 10,
        kind: DefKind::Struct,
        visibility: 0,
        is_macro_expansion: false,
        name: "WsRoot".to_string(),
        path: "ws_crate::WsRoot".to_string(),
        signature: None,
        file: "/workspace/src/lib.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 5,
        end_col: 2,
    });
    let mut f_ws = File::create(&ws_ridx).unwrap();
    ws_writer.write_to(&mut f_ws).unwrap();
    drop(f_ws);

    let dep_reader = IndexReader::open(&dep_ridx).expect("open dep_crate index");
    let ws_reader = IndexReader::open(&ws_ridx).expect("open ws_crate index");

    // Both readers store absolute paths, so the exact lookup rejects the short "src/lib.rs":
    // routing has to anchor the relative path to the workspace root itself.
    assert!(dep_reader.find_file_id("src/lib.rs").is_none());
    assert!(ws_reader.find_file_id("src/lib.rs").is_none());

    // Dependency crate is placed at index 0, workspace root crate at index 1.
    // Without workspace anchoring, index 0 would match "src/lib.rs" first via suffix.
    let workspace = WorkspaceIndex {
        readers: vec![dep_reader, ws_reader],
        paths: vec![dep_ridx, ws_ridx],
        mtimes: vec![None, None],
        source_roots: vec![None, None],
        unit_keys: vec![Some("dep_crate 0.1.0 (lib)".into()), Some("ws_crate 0.1.0 (lib)".into())],
        unit_match: UnitMatchIndex::default(),
        path_index: super::PathIndex::default(),
        target_kinds: vec![None, None],
        reader_generations: vec![None, None],
        reader_dependencies: vec![Vec::new(), Vec::new()],
        reader_extern_units: vec![HashMap::new(), HashMap::new()],
        reader_file_hashes: vec![HashMap::new(), HashMap::new()],
        reader_is_dependency: vec![true, false],
        meta_paths: vec![None, None],
        meta_mtimes: vec![None, None],
        last_reloaded_files: HashSet::new(),
        root_dir: None,
        workspace_root: Some(PathBuf::from("/workspace")),
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

    // Query relative path "src/lib.rs"
    // Because workspace_root is /workspace, "src/lib.rs" anchors to /workspace/src/lib.rs.
    // Index 1 (ws_crate) has /workspace/src/lib.rs.
    // Index 0 (dep_crate) has /deps/dep_crate/src/lib.rs, which does NOT match the anchored path.
    let indices = workspace.reader_indices_for_file("src/lib.rs");
    assert_eq!(indices, vec![1], "relative path 'src/lib.rs' must anchor to workspace root (/workspace/src/lib.rs) and match ws_crate (index 1), not dep_crate (index 0)");

    let primary_idx = workspace.reader_index_for_file("src/lib.rs");
    assert_eq!(primary_idx, Some(1), "primary reader index must be ws_crate");

    let cname = workspace.crate_name_for_file("src/lib.rs");
    assert_eq!(cname, Some("ws_crate"), "crate name for 'src/lib.rs' must be ws_crate");

    let target_1 = workspace.target_file_for_reader(1, "src/lib.rs");
    assert_eq!(target_1.as_deref(), Some("/workspace/src/lib.rs"), "ws_crate must resolve 'src/lib.rs' to workspace-anchored path");
}
