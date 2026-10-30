/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! References scoped to a unit include the consumers that reach the unit only through another
//! unit: a crate using an item through a re-export of the crate that depends on the defining one.

use super::sibling_tests::create_test_workspace;
use super::*;
use crate::format::DefKind;
use crate::writer::{IndexWriter, RawDefinition, RawReference};
use std::fs::File;
use std::path::{Path, PathBuf};

fn write_shard(dir: &Path, name: &str, unit: &str, with_def: bool, call_in: Option<&str>) -> PathBuf {
    let path = dir.join(format!("{name}.ridx"));
    let mut w = IndexWriter::new(name, "0.1.0");
    w.set_unit_key(unit);
    if with_def {
        w.add_definition(RawDefinition {
            def_id: 1,
            kind: DefKind::Fn,
            visibility: 0,
            is_macro_expansion: false,
            name: "f".to_string(),
            path: "a::f".to_string(),
            signature: Some("fn()".to_string()),
            file: "src/lib.rs".to_string(),
            start_line: 1,
            start_col: 8,
            end_line: 1,
            end_col: 9,
        });
    }
    if let Some(file) = call_in {
        w.add_reference(RawReference {
            file: file.to_string(),
            start_line: 5,
            start_col: 9,
            end_line: 5,
            end_col: 10,
            target_def_id: None,
            target_path: "a::f".to_string(),
            target_file: Some("src/lib.rs".to_string()),
            target_unit: Some("unit_a".to_string()),
            target_line: 1,
            target_col: 8,
            kind: DefKind::Fn,
            is_write: false,
            is_macro_expansion: false,
        });
    }
    w.write_to(&mut File::create(&path).unwrap()).unwrap();
    path
}

/// Units `a` (defines `a::f`), `b` (depends on `a`), `c` (depends on `b` only, calls `a::f`
/// through `b`'s re-export) and `d` (depends on an unrelated unit, holds the same reference).
fn chain_workspace() -> (tempfile::TempDir, WorkspaceIndex) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let a = write_shard(dir.path(), "a", "unit_a", true, None);
    let b = write_shard(dir.path(), "b", "unit_b", false, None);
    let c = write_shard(dir.path(), "c", "unit_c", false, Some("src/via_b.rs"));
    let d = write_shard(dir.path(), "d", "unit_d", false, Some("src/unrelated.rs"));
    let unit = |name: &str, path: &PathBuf| (path.as_path().to_path_buf(), Some(root.join(name)), Some(format!("unit_{name}")));
    let infos = [("a", &a), ("b", &b), ("c", &c), ("d", &d)].map(|(n, p)| unit(n, p));
    let mut ws = create_test_workspace(infos.iter().map(|(p, r, u)| (p.as_path(), r.clone(), u.clone(), Some(TargetKind::Lib), false)).collect(), root.clone());
    ws.reader_dependencies[1] = vec!["unit_a".to_string()];
    ws.reader_dependencies[2] = vec!["unit_b".to_string()];
    ws.reader_dependencies[3] = vec!["unit_other".to_string()];
    (dir, ws)
}

#[test]
fn a_consumer_reaching_the_unit_through_another_unit_is_found() {
    let (_dir, ws) = chain_workspace();
    let refs = ws.find_references_for_unit("a::f", Some("unit_a"));
    let files: Vec<&str> = refs.iter().map(|r| r.file).collect();
    assert_eq!(files, vec!["src/via_b.rs"]);
}

#[test]
fn a_unit_depending_on_neither_the_target_nor_a_unit_depending_on_it_stays_excluded() {
    let (_dir, ws) = chain_workspace();
    let refs = ws.find_references_for_unit("a::f", Some("unit_a"));
    assert!(refs.iter().all(|r| r.file != "src/unrelated.rs"));
}

/// Two builds of crate `a` (units `unit_a` and `unit_a2`, same version): `b` depends on the
/// second build only, `c` on `b`, and `c` refers to `a::f` of the first one.
fn sibling_build_workspace() -> (tempfile::TempDir, WorkspaceIndex) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let sub = dir.path().join("second_build");
    std::fs::create_dir(&sub).unwrap();
    let a = write_shard(dir.path(), "a", "unit_a", true, None);
    let a2 = write_shard(&sub, "a", "unit_a2", true, None);
    let b = write_shard(dir.path(), "b", "unit_b", false, None);
    let c = write_shard(dir.path(), "c", "unit_c", false, Some("src/via_b.rs"));
    let infos = [("a", &a), ("a2", &a2), ("b", &b), ("c", &c)].map(|(n, p)| (p.clone(), Some(root.join(n)), Some(format!("unit_{n}"))));
    let mut ws = create_test_workspace(infos.iter().map(|(p, r, u)| (p.as_path(), r.clone(), u.clone(), Some(TargetKind::Lib), false)).collect(), root.clone());
    ws.reader_dependencies[2] = vec!["unit_a2".to_string()];
    ws.reader_dependencies[3] = vec!["unit_b".to_string()];
    (dir, ws)
}

#[test]
fn a_consumer_reaching_another_build_of_the_target_crate_is_found() {
    let (_dir, ws) = sibling_build_workspace();
    let refs = ws.find_references_for_unit("a::f", Some("unit_a"));
    let files: Vec<&str> = refs.iter().map(|r| r.file).collect();
    assert_eq!(files, vec!["src/via_b.rs"]);
}
