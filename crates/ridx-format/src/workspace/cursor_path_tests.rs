/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Tests for cursor path segment, `Self::Assoc`, `T::Assoc`, and synthetic definition filtering.

use super::sibling_tests::create_test_workspace;
use super::*;
use crate::format::DefKind;
use crate::writer::{IndexWriter, RawDefinition, RawReference};
use std::fs::{self, File};

#[test]
fn synthetic_use_impl_and_tuple_field_defs_do_not_hijack_cursor_path() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let src_dir = root.join("src");
    fs::create_dir_all(&src_dir).unwrap();
    fs::write(src_dir.join("lib.rs"), "pub mod sub {\n    pub struct Bar;\n}\nuse crate::sub::Bar;\nstruct S(pub sub::Bar);\nimpl S {\n    fn f() -> sub::Bar { todo!() }\n}\n")
        .unwrap();

    let shard = root.join("k.ridx");
    let mut w = IndexWriter::new("k", "0.1.0");
    w.set_unit_key("unit_k");
    w.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Mod,
        visibility: 0,
        is_macro_expansion: false,
        name: "sub".into(),
        path: "k::sub".into(),
        signature: None,
        file: "src/lib.rs".into(),
        start_line: 1,
        start_col: 9,
        end_line: 3,
        end_col: 2,
    });
    w.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::Use,
        visibility: 0,
        is_macro_expansion: false,
        name: "{use#0}".into(),
        path: "k::{use#0}".into(),
        signature: None,
        file: "src/lib.rs".into(),
        start_line: 4,
        start_col: 1,
        end_line: 4,
        end_col: 21,
    });
    w.add_definition(RawDefinition {
        def_id: 3,
        kind: DefKind::Field,
        visibility: 0,
        is_macro_expansion: false,
        name: "0".into(),
        path: "k::S::0".into(),
        signature: None,
        file: "src/lib.rs".into(),
        start_line: 5,
        start_col: 10,
        end_line: 5,
        end_col: 22,
    });
    w.add_definition(RawDefinition {
        def_id: 4,
        kind: DefKind::Impl,
        visibility: 0,
        is_macro_expansion: false,
        name: "{impl#0}".into(),
        path: "k::{impl#0}".into(),
        signature: None,
        file: "src/lib.rs".into(),
        start_line: 6,
        start_col: 1,
        end_line: 8,
        end_col: 2,
    });
    w.write_to(&mut File::create(&shard).unwrap()).unwrap();

    let ws = create_test_workspace(vec![(&shard, Some(root.clone()), Some("unit_k".into()), Some(TargetKind::Lib), false)], root);

    let d_use = ws.find_definition_at("src/lib.rs", 4, 13).unwrap();
    assert_eq!((d_use.name, d_use.kind), ("sub", DefKind::Mod));

    let d_tuple = ws.find_definition_at("src/lib.rs", 5, 15).unwrap();
    assert_eq!((d_tuple.name, d_tuple.kind), ("sub", DefKind::Mod));

    let d_impl = ws.find_definition_at("src/lib.rs", 7, 16).unwrap();
    assert_eq!((d_impl.name, d_impl.kind), ("sub", DefKind::Mod));
}

#[test]
fn self_assoc_and_type_param_assoc_resolve_to_trait_and_impl_items() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let src_dir = root.join("src");
    fs::create_dir_all(&src_dir).unwrap();
    fs::write(
        src_dir.join("lib.rs"),
        "pub trait Visitor {\n    type Value;\n    fn visit(self) -> Self::Value;\n}\npub fn run<V: Visitor>(v: V) -> V::Value {\n    v.visit()\n}\n",
    )
    .unwrap();

    let shard = root.join("k.ridx");
    let mut w = IndexWriter::new("k", "0.1.0");
    w.set_unit_key("unit_k");
    w.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Trait,
        visibility: 0,
        is_macro_expansion: false,
        name: "Visitor".into(),
        path: "k::Visitor".into(),
        signature: Some("pub trait Visitor".into()),
        file: "src/lib.rs".into(),
        start_line: 1,
        start_col: 11,
        end_line: 1,
        end_col: 18,
    });
    w.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::AssocTy,
        visibility: 0,
        is_macro_expansion: false,
        name: "Value".into(),
        path: "k::Visitor::Value".into(),
        signature: None,
        file: "src/lib.rs".into(),
        start_line: 2,
        start_col: 10,
        end_line: 2,
        end_col: 15,
    });
    w.add_definition_with_owners(
        RawDefinition {
            def_id: 3,
            kind: DefKind::TyParam,
            visibility: 0,
            is_macro_expansion: false,
            name: "V".into(),
            path: "k::run::V".into(),
            signature: Some("V: Visitor".into()),
            file: "src/lib.rs".into(),
            start_line: 5,
            start_col: 12,
            end_line: 5,
            end_col: 13,
        },
        vec!["scope:5:1:7:2".into(), "k::Visitor".into()],
    );
    w.write_to(&mut File::create(&shard).unwrap()).unwrap();

    let ws = create_test_workspace(vec![(&shard, Some(root.clone()), Some("unit_k".into()), Some(TargetKind::Lib), false)], root);

    let self_val = ws.find_definition_at("src/lib.rs", 3, 29).unwrap();
    assert_eq!(self_val.path, "k::Visitor::Value");

    let tp_val = ws.find_definition_at("src/lib.rs", 5, 36).unwrap();
    assert_eq!(tp_val.path, "k::Visitor::Value");
}

#[test]
fn wide_loop_ref_yields_to_cursor_path_segment() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let src_dir = root.join("src");
    fs::create_dir_all(&src_dir).unwrap();
    fs::write(src_dir.join("lib.rs"), "pub mod sub {\n    pub fn make() -> Vec<i32> { vec![] }\n}\nfn demo() {\n    for _ in crate::sub::make() {\n    }\n}\n").unwrap();

    let shard = root.join("k.ridx");
    let mut w = IndexWriter::new("k", "0.1.0");
    w.set_unit_key("unit_k");
    w.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Mod,
        visibility: 0,
        is_macro_expansion: false,
        name: "sub".into(),
        path: "k::sub".into(),
        signature: None,
        file: "src/lib.rs".into(),
        start_line: 1,
        start_col: 9,
        end_line: 3,
        end_col: 2,
    });
    w.add_reference(RawReference {
        file: "src/lib.rs".into(),
        start_line: 5,
        start_col: 5,
        end_line: 6,
        end_col: 6,
        target_def_id: None,
        target_path: "core::iter::traits::collect::IntoIterator::into_iter".into(),
        target_file: Some("library/core/src/iter/traits/collect.rs".into()),
        target_unit: None,
        target_line: 312,
        target_col: 5,
        kind: DefKind::AssocFn,
        is_write: false,
        is_macro_expansion: false,
    });
    w.write_to(&mut File::create(&shard).unwrap()).unwrap();

    let ws = create_test_workspace(vec![(&shard, Some(root.clone()), Some("unit_k".into()), Some(TargetKind::Lib), false)], root);

    let def = ws.find_definition_at("src/lib.rs", 5, 22).unwrap();
    assert_eq!((def.path, def.kind), ("k::sub", DefKind::Mod));
}

#[test]
fn cursor_path_ignores_strings_raw_strings_and_comments_but_resolves_adjacent_code() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let src_dir = root.join("src");
    fs::create_dir_all(&src_dir).unwrap();
    let src = concat!(
        "pub mod inner {\n",
        "    pub struct IsTerminal;\n",
        "    pub struct Ambiguous;\n",
        "}\n",
        "#[deprecated(since = \"0.1.10\", note = \"use k::inner::IsTerminal instead\")]\n",
        "fn check() {\n",
        "    let _ = \"k::inner::Ambiguous: (17),\";\n",
        "    let _ = r#\"\n",
        "        use k::inner::IsTerminal;\n",
        "    \"#;\n",
        "    /* outer /* nested */ k::inner::IsTerminal */\n",
        "    // line comment k::inner::IsTerminal\n",
        "    let _: k::inner::IsTerminal;\n",
        "}\n",
    );
    fs::write(src_dir.join("lib.rs"), src).unwrap();

    let shard = root.join("k.ridx");
    let mut w = IndexWriter::new("k", "0.1.0");
    w.set_unit_key("unit_k");
    w.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Mod,
        visibility: 0,
        is_macro_expansion: false,
        name: "inner".into(),
        path: "k::inner".into(),
        signature: None,
        file: "src/lib.rs".into(),
        start_line: 1,
        start_col: 9,
        end_line: 4,
        end_col: 2,
    });
    w.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::Struct,
        visibility: 0,
        is_macro_expansion: false,
        name: "IsTerminal".into(),
        path: "k::inner::IsTerminal".into(),
        signature: None,
        file: "src/lib.rs".into(),
        start_line: 2,
        start_col: 16,
        end_line: 2,
        end_col: 26,
    });
    w.add_definition(RawDefinition {
        def_id: 3,
        kind: DefKind::Struct,
        visibility: 0,
        is_macro_expansion: false,
        name: "Ambiguous".into(),
        path: "k::inner::Ambiguous".into(),
        signature: None,
        file: "src/lib.rs".into(),
        start_line: 3,
        start_col: 16,
        end_line: 3,
        end_col: 25,
    });
    w.write_to(&mut File::create(&shard).unwrap()).unwrap();

    let ws = create_test_workspace(vec![(&shard, Some(root.clone()), Some("unit_k".into()), Some(TargetKind::Lib), false)], root);

    assert!(ws.find_definition_at("src/lib.rs", 5, 55).is_none());
    assert!(ws.find_references_at("src/lib.rs", 5, 48).is_none());
    assert!(ws.find_definition_at("src/lib.rs", 7, 25).is_none());
    assert!(ws.find_definition_at("src/lib.rs", 9, 24).is_none());
    assert!(ws.find_definition_at("src/lib.rs", 11, 38).is_none());
    assert!(ws.find_definition_at("src/lib.rs", 12, 32).is_none());

    let adj = ws.find_definition_at("src/lib.rs", 13, 24).unwrap();
    assert_eq!(adj.path, "k::inner::IsTerminal");
}
