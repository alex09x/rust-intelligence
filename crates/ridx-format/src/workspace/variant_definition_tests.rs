/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! The definition of a use of an enum variant, which the index holds no definition for: it is the
//! target the reference records, not the struct or import of the same name in the same file.

use super::sibling_tests::create_test_workspace;
use super::*;
use crate::format::DefKind;
use crate::writer::{IndexWriter, RawDefinition, RawReference};
use std::fs::File;
use std::path::PathBuf;

/// Unit `unit_a` (crate `a`) has `struct Foo` on line 1 of `src/nodes.rs` and the variant
/// `Ty::Foo`, with no definition, on line 5. Unit `unit_b` (crate `b`, depending on `a`) has a
/// reference on line 3 of `src/use.rs` to `ref_path` at `ref_line` of `src/nodes.rs`.
fn workspace(ref_path: &str, ref_line: u32) -> (tempfile::TempDir, WorkspaceIndex, PathBuf) {
    workspace_of_kind(ref_path, ref_line, DefKind::Variant)
}

fn workspace_of_kind(
    ref_path: &str,
    ref_line: u32,
    kind: DefKind,
) -> (tempfile::TempDir, WorkspaceIndex, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let a = dir.path().join("a.ridx");
    let mut w = IndexWriter::new("a", "0.1.0");
    w.set_unit_key("unit_a");
    w.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Struct,
        visibility: 0,
        is_macro_expansion: false,
        name: "Foo".to_string(),
        path: "a::Foo".to_string(),
        signature: None,
        file: "src/nodes.rs".to_string(),
        start_line: 1,
        start_col: 12,
        end_line: 1,
        end_col: 15,
    });
    w.write_to(&mut File::create(&a).unwrap()).unwrap();
    let b = dir.path().join("b.ridx");
    let mut w = IndexWriter::new("b", "0.1.0");
    w.set_unit_key("unit_b");
    w.add_reference(RawReference {
        file: "src/use.rs".to_string(),
        start_line: 3,
        start_col: 5,
        end_line: 3,
        end_col: 8,
        target_def_id: None,
        target_path: ref_path.to_string(),
        target_file: Some("src/nodes.rs".to_string()),
        target_unit: Some("unit_a".to_string()),
        target_line: ref_line,
        target_col: 5,
        kind,
        is_write: false,
        is_macro_expansion: false,
    });
    w.write_to(&mut File::create(&b).unwrap()).unwrap();
    let infos = [("a", &a), ("b", &b)]
        .map(|(n, p)| (p.clone(), Some(root.join(n)), Some(format!("unit_{n}"))));
    let mut ws = create_test_workspace(
        infos
            .iter()
            .map(|(p, r, u)| {
                (
                    p.as_path(),
                    r.clone(),
                    u.clone(),
                    Some(TargetKind::Lib),
                    false,
                )
            })
            .collect(),
        root.clone(),
    );
    ws.reader_dependencies[1] = vec!["unit_a".to_string()];
    (dir, ws, root)
}

#[test]
fn a_use_of_a_variant_is_not_taken_for_the_struct_of_the_same_name() {
    let (_dir, ws, _root) = workspace("a::Ty::Foo", 5);
    let def = ws.find_definition_at("src/use.rs", 3, 6).unwrap();
    assert_eq!(
        (def.path, def.kind, def.start_line),
        ("a::Ty::Foo", DefKind::Variant, 5)
    );
}

#[test]
fn the_definition_of_a_variant_use_is_rooted_in_the_unit_that_holds_it() {
    let (_dir, ws, root) = workspace("a::Ty::Foo", 5);
    let def = ws.find_definition_at("src/use.rs", 3, 6).unwrap();
    assert_eq!(def.source_root, Some(root.join("a").as_path()));
    assert_eq!(def.reader_idx, Some(0));
}

#[test]
fn a_target_on_the_line_of_the_struct_is_still_found_by_file_and_name() {
    let (_dir, ws, _root) = workspace("a::reexport::Foo", 1);
    let def = ws.find_definition_at("src/use.rs", 3, 6).unwrap();
    assert_eq!(
        (def.path, def.kind, def.start_line),
        ("a::Foo", DefKind::Struct, 1)
    );
}

#[test]
fn a_target_that_is_no_variant_keeps_the_match_by_file_and_name_whatever_its_line() {
    let (_dir, ws, _root) = workspace_of_kind("a::reexport::Foo", 99, DefKind::Struct);
    let def = ws.find_definition_at("src/use.rs", 3, 6).unwrap();
    assert_eq!(
        (def.path, def.kind, def.start_line),
        ("a::Foo", DefKind::Struct, 1)
    );
}
