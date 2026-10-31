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
    let (_dir, mut ws) = sibling_build_workspace();
    ws.rebuild_unit_match();
    let refs = ws.find_references_for_unit("a::f", Some("unit_a"));
    let files: Vec<&str> = refs.iter().map(|r| r.file).collect();
    assert_eq!(files, vec!["src/via_b.rs"]);
}

#[test]
fn submodule_name_colliding_with_other_shard_crate_name_preserves_defining_crate_and_local_resolution() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();

    // 1. Shard `core_lib` defines `submod::Item::act` in `src/submod.rs` (file_id 1)
    let p_core = dir.path().join("core_lib.ridx");
    let mut w_core = IndexWriter::new("core_lib", "0.1.0");
    w_core.set_unit_key("unit_core");
    w_core.get_or_insert_file("src/lib.rs");
    w_core.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::AssocFn,
        visibility: 0,
        is_macro_expansion: false,
        name: "act".to_string(),
        path: "submod::Item::act".to_string(),
        signature: Some("fn(&self)".to_string()),
        file: "src/submod.rs".to_string(),
        start_line: 20,
        start_col: 8,
        end_line: 20,
        end_col: 11,
    });
    w_core.add_reference(RawReference {
        file: "src/lib.rs".to_string(),
        start_line: 10,
        start_col: 5,
        end_line: 10,
        end_col: 8,
        target_def_id: Some(1),
        target_path: "submod::Item::act".to_string(),
        target_file: Some("src/submod.rs".to_string()),
        target_unit: Some("unit_core".to_string()),
        target_line: 20,
        target_col: 8,
        kind: DefKind::AssocFn,
        is_write: false,
        is_macro_expansion: false,
    });
    w_core.write_to(&mut File::create(&p_core).unwrap()).unwrap();

    // 2. Unrelated test binary shard named `submod` also defines `submod::Item::act` at a different line
    let p_sub = dir.path().join("submod.ridx");
    let mut w_sub = IndexWriter::new("submod", "0.1.0");
    w_sub.set_unit_key("unit_submod_test");
    w_sub.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::AssocFn,
        visibility: 0,
        is_macro_expansion: false,
        name: "act".to_string(),
        path: "submod::Item::act".to_string(),
        signature: Some("fn()".to_string()),
        file: "tests/submod.rs".to_string(),
        start_line: 99,
        start_col: 4,
        end_line: 99,
        end_col: 7,
    });
    w_sub.write_to(&mut File::create(&p_sub).unwrap()).unwrap();

    // 3. Consumer shard `app` depends on `unit_core` and references `core_lib::submod::Item::act`
    let p_app = dir.path().join("app.ridx");
    let mut w_app = IndexWriter::new("app", "0.1.0");
    w_app.set_unit_key("unit_app");
    w_app.add_reference(RawReference {
        file: "src/main.rs".to_string(),
        start_line: 7,
        start_col: 9,
        end_line: 7,
        end_col: 12,
        target_def_id: None,
        target_path: "core_lib::submod::Item::act".to_string(),
        target_file: Some("src/submod.rs".to_string()),
        target_unit: Some("unit_core".to_string()),
        target_line: 20,
        target_col: 8,
        kind: DefKind::AssocFn,
        is_write: false,
        is_macro_expansion: false,
    });
    w_app.write_to(&mut File::create(&p_app).unwrap()).unwrap();

    let mut ws = create_test_workspace(
        vec![
            (p_core.as_path(), Some(root.join("core_lib")), Some("unit_core".to_string()), Some(TargetKind::Lib), false),
            (p_sub.as_path(), Some(root.join("submod")), Some("unit_submod_test".to_string()), Some(TargetKind::Test), false),
            (p_app.as_path(), Some(root.join("app")), Some("unit_app".to_string()), Some(TargetKind::Bin), false),
        ],
        root.clone(),
    );
    ws.reader_dependencies[2] = vec!["unit_core".to_string()];
    ws.rebuild_unit_match();

    let loc_refs = ws.find_references_at(&root.join("core_lib/src/lib.rs").to_string_lossy(), 10, 6).unwrap();
    assert_eq!(loc_refs.target_file, "src/submod.rs");
    assert_eq!(loc_refs.target_line, 20);
    let ref_files: Vec<&str> = loc_refs.references.iter().map(|r| r.file).collect();
    assert!(ref_files.contains(&"src/lib.rs"));
    assert!(ref_files.contains(&"src/main.rs"));
}
