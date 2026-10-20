/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Routing of a file to the shard that stores it must compare whole paths in the shard's own
//! coordinates: a root package whose stored `src/lib.rs` is a suffix of every nested crate's
//! `crates/<name>/src/lib.rs` must not claim those files, a file outside every source root has no
//! reader, and relative inputs are anchored to a root rather than matched by suffix.

use super::{UnitMatchIndex, WorkspaceIndex};
use crate::writer::IndexWriter;
use crate::{DefKind, RawDefinition};
use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};

/// Writes a shard whose file table holds `files`, each with one function at `(line, 1..10)`.
fn write_shard(dir: &Path, crate_name: &str, files: &[(&str, &str, u32)]) -> PathBuf {
    let path = dir.join(format!("{crate_name}.ridx"));
    let mut writer = IndexWriter::new(crate_name, "0.1.0");
    for (i, (file, item, line)) in files.iter().enumerate() {
        writer.add_definition(RawDefinition {
            def_id: i as u32 + 1,
            kind: DefKind::Fn,
            visibility: 1,
            is_macro_expansion: false,
            name: item.to_string(),
            path: format!("{crate_name}::{item}"),
            signature: None,
            file: file.to_string(),
            start_line: *line,
            start_col: 1,
            end_line: *line,
            end_col: 10,
        });
    }
    let mut f = File::create(&path).unwrap();
    writer.write_to(&mut f).unwrap();
    path
}

/// A workspace of `shards`: (shard path, source root, is_dependency), rooted at `ws_root`.
fn workspace(shards: &[(PathBuf, Option<&str>, bool)], ws_root: &str) -> WorkspaceIndex {
    let readers: Vec<_> = shards
        .iter()
        .map(|(p, _, _)| crate::reader::IndexReader::open(p).unwrap())
        .collect();
    let n = readers.len();
    let unit_keys: Vec<Option<String>> = (0..n).map(|i| Some(format!("unit{i}"))).collect();
    let reader_extern_units = vec![HashMap::new(); n];
    let unit_match = UnitMatchIndex::build(&readers, &unit_keys, &reader_extern_units);
    WorkspaceIndex {
        readers,
        paths: shards.iter().map(|(p, _, _)| p.clone()).collect(),
        mtimes: vec![None; n],
        source_roots: shards
            .iter()
            .map(|(_, r, _)| r.map(PathBuf::from))
            .collect(),
        unit_keys,
        unit_match,
        path_index: super::PathIndex::default(),
        target_kinds: vec![None; n],
        reader_generations: vec![None; n],
        reader_dependencies: vec![Vec::new(); n],
        reader_extern_units,
        reader_file_hashes: vec![HashMap::new(); n],
        reader_is_dependency: shards.iter().map(|(_, _, d)| *d).collect(),
        meta_paths: vec![None; n],
        meta_mtimes: vec![None; n],
        last_reloaded_files: std::collections::HashSet::new(),
        root_dir: None,
        workspace_root: Some(PathBuf::from(ws_root)),
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

fn target(ws: &WorkspaceIndex, idx: usize, file: &str) -> Option<String> {
    ws.target_file_for_reader(idx, file).map(|c| c.into_owned())
}

#[test]
fn test_root_package_shard_does_not_claim_nested_crate_file() {
    let dir = tempfile::tempdir().unwrap();
    let root = write_shard(dir.path(), "bevy", &[("src/lib.rs", "root_item", 3)]);
    let nested = write_shard(
        dir.path(),
        "bevy_time",
        &[("crates/time/src/lib.rs", "time_item", 10)],
    );
    let ws = workspace(
        &[
            (root, Some("/ws"), false),
            (nested, Some("/ws/crates/time"), false),
        ],
        "/ws",
    );

    assert_eq!(
        target(&ws, 0, "/ws/crates/time/src/lib.rs"),
        None,
        "stored src/lib.rs is not crates/time/src/lib.rs"
    );
    assert_eq!(
        target(&ws, 0, "/ws/src/lib.rs").as_deref(),
        Some("src/lib.rs")
    );
    assert_eq!(
        target(&ws, 1, "/ws/crates/time/src/lib.rs").as_deref(),
        Some("crates/time/src/lib.rs")
    );
    assert_eq!(
        target(&ws, 1, "/ws/src/lib.rs"),
        None,
        "the nested crate's root does not contain the workspace lib"
    );

    assert_eq!(
        ws.find_definition_at("/ws/crates/time/src/lib.rs", 3, 2).map(|d| d.name),
        None,
        "line 3 of the nested file is empty; the root crate's item at line 3 of its own lib must not answer"
    );
    assert_eq!(
        ws.find_definition_at("/ws/crates/time/src/lib.rs", 10, 2)
            .map(|d| d.name),
        Some("time_item")
    );
    assert_eq!(
        ws.find_definition_at("/ws/src/lib.rs", 3, 2)
            .map(|d| d.name),
        Some("root_item")
    );
}

#[test]
fn test_relative_inputs_anchor_to_a_root_and_match_whole_paths() {
    let dir = tempfile::tempdir().unwrap();
    let serde = write_shard(
        dir.path(),
        "serde",
        &[("serde/src/lib.rs", "serde_item", 2)],
    );
    let dep = write_shard(dir.path(), "dep", &[("src/lib.rs", "dep_item", 2)]);
    let ws = workspace(
        &[
            (serde, Some("/ws/serde"), false),
            (dep, Some("/deps/dep-1.0.0"), true),
        ],
        "/ws",
    );

    assert_eq!(
        target(&ws, 0, "serde/src/lib.rs").as_deref(),
        Some("serde/src/lib.rs"),
        "workspace-relative input"
    );
    assert_eq!(
        target(&ws, 0, "src/lib.rs").as_deref(),
        Some("serde/src/lib.rs"),
        "relative to the crate root, re-expressed"
    );
    assert_eq!(
        target(&ws, 0, "/ws/serde/src/lib.rs").as_deref(),
        Some("serde/src/lib.rs")
    );
    assert_eq!(
        target(&ws, 1, "src/lib.rs").as_deref(),
        Some("src/lib.rs"),
        "dependency paths are stored relative to the package"
    );
    assert_eq!(
        target(&ws, 1, "serde/src/lib.rs"),
        None,
        "another crate's path ends with src/lib.rs but is not the dependency's file"
    );
    assert_eq!(
        target(&ws, 1, "/deps/dep-1.0.0/src/lib.rs").as_deref(),
        Some("src/lib.rs")
    );
    assert_eq!(target(&ws, 1, "/ws/serde/src/lib.rs"), None);
    assert_eq!(
        ws.find_definition_at("serde/src/lib.rs", 2, 2)
            .map(|d| d.name),
        Some("serde_item")
    );
}

#[test]
fn test_file_outside_every_root_has_no_reader() {
    let dir = tempfile::tempdir().unwrap();
    let a = write_shard(dir.path(), "dep_a", &[("src/lib.rs", "a_item", 2)]);
    let b = write_shard(dir.path(), "dep_b", &[("src/lib.rs", "b_item", 2)]);
    let ws_crate = write_shard(
        dir.path(),
        "ws_crate",
        &[("ws_crate/src/lib.rs", "ws_item", 2)],
    );
    let ws = workspace(
        &[
            (a, Some("/deps/a-1.0.0"), true),
            (b, Some("/deps/b-2.0.0"), true),
            (ws_crate, Some("/ws/ws_crate"), false),
        ],
        "/ws",
    );

    assert_eq!(
        ws.reader_indices_for_file("/ws/tests/ui/src/lib.rs"),
        Vec::<usize>::new(),
        "a file under no source root is owned by nobody, even though its path ends with src/lib.rs"
    );
    assert_eq!(
        ws.find_definition_at("/ws/tests/ui/src/lib.rs", 2, 2)
            .map(|d| d.name),
        None
    );
    assert_eq!(
        ws.reader_indices_for_file("/ws/ws_crate/src/lib.rs"),
        vec![2]
    );
}

#[test]
fn test_new_file_under_a_root_is_routed_but_unowned() {
    let dir = tempfile::tempdir().unwrap();
    let serde = write_shard(
        dir.path(),
        "serde",
        &[("serde/src/lib.rs", "serde_item", 2)],
    );
    let ws = workspace(&[(serde, Some("/ws/serde"), false)], "/ws");

    assert_eq!(
        ws.reader_indices_for_file("/ws/serde/src/new.rs"),
        vec![0],
        "the crate's readers are candidates"
    );
    assert_eq!(
        target(&ws, 0, "/ws/serde/src/new.rs"),
        None,
        "but none of them stores the new file"
    );
    assert_eq!(
        ws.find_definition_at("/ws/serde/src/new.rs", 1, 1)
            .map(|d| d.name),
        None
    );
}

#[test]
fn test_sysroot_reader_only_answers_its_stored_paths() {
    let dir = tempfile::tempdir().unwrap();
    let core = write_shard(
        dir.path(),
        "core",
        &[("library/core/src/lib.rs", "core_item", 5)],
    );
    let ws = workspace(&[(core, None, true)], "/ws");

    assert_eq!(
        target(&ws, 0, "library/core/src/lib.rs").as_deref(),
        Some("library/core/src/lib.rs")
    );
    assert_eq!(
        target(&ws, 0, "/ws/core/src/lib.rs"),
        None,
        "a workspace file whose path ends with core/src/lib.rs is not the sysroot file"
    );
    assert_eq!(
        ws.find_definition_at("library/core/src/lib.rs", 5, 2)
            .map(|d| d.name),
        Some("core_item")
    );
}

#[test]
fn test_single_shard_opened_alone_resolves_against_its_directory() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/lib.rs"), "pub fn alone() {}\n").unwrap();
    let shard = write_shard(dir.path(), "alone", &[("src/lib.rs", "alone", 1)]);
    let ws = WorkspaceIndex::open(&shard).expect("open the shard on its own");
    let file = dir.path().join("src/lib.rs");
    let file = file.to_str().unwrap();

    assert_eq!(
        ws.workspace_root, None,
        "a lone shard records no workspace root"
    );
    assert_eq!(ws.reader_indices_for_file(file), vec![0]);
    assert_eq!(target(&ws, 0, file).as_deref(), Some("src/lib.rs"));
    assert_eq!(
        ws.find_definition_at(file, 1, 2).map(|d| d.name),
        Some("alone")
    );
    let elsewhere = dir.path().join("other/src/lib.rs");
    assert_eq!(
        target(&ws, 0, elsewhere.to_str().unwrap()),
        None,
        "a nested path is not the stored src/lib.rs"
    );
}
