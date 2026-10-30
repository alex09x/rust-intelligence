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

#[test]
fn test_container_scoped_trait_impl_references_do_not_cross_pollute_siblings() {
    let test_dir = new_test_dir("ridx_trait_impl_refs");
    let src_dir = test_dir.0.join("src");
    fs::create_dir_all(&src_dir).unwrap();
    let lib_rs = src_dir.join("lib.rs");
    fs::write(
        &lib_rs,
        "pub trait Trait {\n    fn m(&self);\n}\npub struct A;\nimpl Trait for A {\n    fn m(&self) {}\n}\npub struct B;\nimpl Trait for B {\n    fn m(&self) {}\n}\npub fn run(a: A, b: B) {\n    a.m();\n    b.m();\n}\n",
    )
    .unwrap();

    let mut w = IndexWriter::new("my_crate", "0.1.0");
    w.set_unit_key("my_crate-unit");

    // Trait definition (line 1) and Trait::m declaration (line 2)
    w.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Trait,
        visibility: 0,
        is_macro_expansion: false,
        name: "Trait".to_string(),
        path: "my_crate::Trait".to_string(),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: 1,
        start_col: 11,
        end_line: 1,
        end_col: 16,
    });
    w.add_definition_with_owners(
        RawDefinition {
            def_id: 2,
            kind: DefKind::AssocFn,
            visibility: 0,
            is_macro_expansion: false,
            name: "m".to_string(),
            path: "my_crate::Trait::m".to_string(),
            signature: Some("fn(&self)".to_string()),
            file: "src/lib.rs".to_string(),
            start_line: 2,
            start_col: 8,
            end_line: 2,
            end_col: 9,
        },
        vec!["my_crate::Trait".to_string()],
    );

    // Struct A (line 4) and impl Trait for A { fn m } (line 6)
    w.add_definition(RawDefinition {
        def_id: 3,
        kind: DefKind::Struct,
        visibility: 0,
        is_macro_expansion: false,
        name: "A".to_string(),
        path: "my_crate::A".to_string(),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: 4,
        start_col: 12,
        end_line: 4,
        end_col: 13,
    });
    w.add_deref("my_crate::A".to_string(), "my_crate::Trait".to_string());
    w.add_definition_with_owners(
        RawDefinition {
            def_id: 4,
            kind: DefKind::AssocFn,
            visibility: 0,
            is_macro_expansion: false,
            name: "m".to_string(),
            path: "my_crate::A::m".to_string(),
            signature: Some("fn(&self)".to_string()),
            file: "src/lib.rs".to_string(),
            start_line: 6,
            start_col: 8,
            end_line: 6,
            end_col: 9,
        },
        vec!["my_crate::A".to_string()],
    );

    // Struct B (line 8) and impl Trait for B { fn m } (line 10)
    w.add_definition(RawDefinition {
        def_id: 5,
        kind: DefKind::Struct,
        visibility: 0,
        is_macro_expansion: false,
        name: "B".to_string(),
        path: "my_crate::B".to_string(),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: 8,
        start_col: 12,
        end_line: 8,
        end_col: 13,
    });
    w.add_deref("my_crate::B".to_string(), "my_crate::Trait".to_string());
    w.add_definition_with_owners(
        RawDefinition {
            def_id: 6,
            kind: DefKind::AssocFn,
            visibility: 0,
            is_macro_expansion: false,
            name: "m".to_string(),
            path: "my_crate::B::m".to_string(),
            signature: Some("fn(&self)".to_string()),
            file: "src/lib.rs".to_string(),
            start_line: 10,
            start_col: 8,
            end_line: 10,
            end_col: 9,
        },
        vec!["my_crate::B".to_string()],
    );

    // Call to a.m() at line 13 col 7..8 -> references A::m
    w.add_reference(RawReference {
        file: "src/lib.rs".to_string(),
        start_line: 13,
        start_col: 7,
        end_line: 13,
        end_col: 8,
        target_def_id: Some(4),
        target_path: "my_crate::A::m".to_string(),
        target_unit: Some("my_crate-unit".to_string()),
        target_file: Some("src/lib.rs".to_string()),
        target_line: 6,
        target_col: 8,
        kind: DefKind::AssocFn,
        is_write: false,
        is_macro_expansion: false,
    });

    // Call to b.m() at line 14 col 7..8 -> references B::m
    w.add_reference(RawReference {
        file: "src/lib.rs".to_string(),
        start_line: 14,
        start_col: 7,
        end_line: 14,
        end_col: 8,
        target_def_id: Some(6),
        target_path: "my_crate::B::m".to_string(),
        target_unit: Some("my_crate-unit".to_string()),
        target_file: Some("src/lib.rs".to_string()),
        target_line: 10,
        target_col: 8,
        kind: DefKind::AssocFn,
        is_write: false,
        is_macro_expansion: false,
    });

    let ridx_file = test_dir.0.join("my_crate.ridx");
    let mut f = File::create(&ridx_file).unwrap();
    w.write_to(&mut f).unwrap();
    drop(f);

    let mut ws = WorkspaceIndex::open(&ridx_file).unwrap();
    ws.set_workspace_root(Some(test_dir.0.clone()));

    // 1. Query references on concrete impl `A::m` (line 6, col 8):
    // Must include Trait::m (line 2) and call `a.m()` (line 13),
    // 1. Query references on `A::m` (line 6, col 8):
    // Must include call `a.m()` (line 13), and MUST NOT include `B::m` (line 10),
    // call `b.m()` (line 14), or `Trait::m` (line 2).
    let refs_a = ws.find_references_at("src/lib.rs", 6, 8).expect("references for A::m");
    let lines_a: Vec<u32> = refs_a.references.iter().map(|r| r.span.start_line()).collect();
    assert_eq!(lines_a, vec![13], "A::m references must include only call a.m() at line 13 without sibling impl B::m (10) or b.m() (14)");

    // 2. Query references on trait declaration `Trait::m` (line 2, col 8):
    // Must expand through container `deref_entries` to include A::m (6), B::m (10), a.m() (13), b.m() (14).
    let refs_trait = ws.find_references_at("src/lib.rs", 2, 8).expect("references for Trait::m");
    let lines_trait: Vec<u32> = refs_trait.references.iter().map(|r| r.span.start_line()).collect();
    for expected_line in [6, 10, 13, 14] {
        assert!(lines_trait.contains(&expected_line), "Trait::m references must include line {expected_line}, got {lines_trait:?}");
    }
}

#[test]
fn test_field_vs_method_and_macro_callsite_reference_filtering() {
    let test_dir = new_test_dir("ridx_field_method_filter");
    let src_dir = test_dir.0.join("src");
    fs::create_dir_all(&src_dir).unwrap();
    let lib_rs = src_dir.join("lib.rs");
    fs::write(&lib_rs, "pub struct Glob { pub glob: String }\nimpl Glob {\n    pub fn glob(&self) -> &str { &self.glob }\n}\n").unwrap();

    let mut w = IndexWriter::new("globset", "0.1.0");
    w.set_unit_key("globset-unit");

    w.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Field,
        visibility: 0,
        is_macro_expansion: false,
        name: "glob".to_string(),
        path: "globset::Glob::glob".to_string(),
        signature: Some("String".to_string()),
        file: "src/lib.rs".to_string(),
        start_line: 1,
        start_col: 23,
        end_line: 1,
        end_col: 27,
    });
    w.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::AssocFn,
        visibility: 0,
        is_macro_expansion: false,
        name: "glob".to_string(),
        path: "globset::Glob::glob".to_string(),
        signature: Some("fn(&self) -> &str".to_string()),
        file: "src/lib.rs".to_string(),
        start_line: 3,
        start_col: 12,
        end_line: 3,
        end_col: 16,
    });

    // Field access `&self.glob` at line 3 col 35..39
    w.add_reference(RawReference {
        file: "src/lib.rs".to_string(),
        start_line: 3,
        start_col: 35,
        end_line: 3,
        end_col: 39,
        target_def_id: Some(1),
        target_path: "globset::Glob::glob".to_string(),
        target_unit: Some("globset-unit".to_string()),
        target_file: Some("src/lib.rs".to_string()),
        target_line: 1,
        target_col: 23,
        kind: DefKind::Field,
        is_write: false,
        is_macro_expansion: false,
    });

    // Method call `g.glob()` at line 10 col 5..9
    w.add_reference(RawReference {
        file: "src/lib.rs".to_string(),
        start_line: 10,
        start_col: 5,
        end_line: 10,
        end_col: 9,
        target_def_id: Some(2),
        target_path: "globset::Glob::glob".to_string(),
        target_unit: Some("globset-unit".to_string()),
        target_file: Some("src/lib.rs".to_string()),
        target_line: 3,
        target_col: 12,
        kind: DefKind::AssocFn,
        is_write: false,
        is_macro_expansion: false,
    });

    // Outer macro callsite span (width 25 != 4) with is_macro_expansion: true
    w.add_reference(RawReference {
        file: "src/lib.rs".to_string(),
        start_line: 20,
        start_col: 1,
        end_line: 20,
        end_col: 26,
        target_def_id: Some(2),
        target_path: "globset::Glob::glob".to_string(),
        target_unit: Some("globset-unit".to_string()),
        target_file: Some("src/lib.rs".to_string()),
        target_line: 3,
        target_col: 12,
        kind: DefKind::AssocFn,
        is_write: false,
        is_macro_expansion: true,
    });

    let ridx_file = test_dir.0.join("globset.ridx");
    let mut f = File::create(&ridx_file).unwrap();
    w.write_to(&mut f).unwrap();
    drop(f);

    let mut ws = WorkspaceIndex::open(&ridx_file).unwrap();
    ws.set_workspace_root(Some(test_dir.0.clone()));

    let method_refs = ws.find_references_at("src/lib.rs", 3, 12).expect("references for fn glob");
    let method_lines: Vec<u32> = method_refs.references.iter().map(|r| r.span.start_line()).collect();
    assert_eq!(method_lines, vec![10]);
}
