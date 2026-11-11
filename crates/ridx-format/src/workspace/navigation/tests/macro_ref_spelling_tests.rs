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

fn macro_ref(line: u32, col: u32, end_col: u32) -> RawReference {
    RawReference {
        file: "src/lib.rs".to_string(),
        start_line: line,
        start_col: col,
        end_line: line,
        end_col,
        target_def_id: Some(1),
        target_path: "my_crate::Thing".to_string(),
        target_unit: Some("my_crate-unit".to_string()),
        target_file: Some("src/lib.rs".to_string()),
        target_line: 2,
        target_col: 12,
        kind: DefKind::Struct,
        is_write: false,
        is_macro_expansion: true,
    }
}

#[test]
fn test_a_macro_reference_counts_only_where_the_source_spells_the_name() {
    let test_dir = new_test_dir("ridx_macro_ref_spelling");
    let src_dir = test_dir.0.join("src");
    fs::create_dir_all(&src_dir).unwrap();
    fs::write(src_dir.join("lib.rs"), "#[derive(Debug, Clone)]\npub struct Thing;\npub fn make() -> Vec<Thing> {\n    vec![Thing]\n}\n").unwrap();

    let mut w = IndexWriter::new("my_crate", "0.1.0");
    w.set_unit_key("my_crate-unit");
    w.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Struct,
        visibility: 0,
        is_macro_expansion: false,
        name: "Thing".to_string(),
        path: "my_crate::Thing".to_string(),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: 2,
        start_col: 12,
        end_line: 2,
        end_col: 17,
    });
    // The derive's generated `impl Debug for Thing`, at `Debug` in the attribute.
    w.add_reference(macro_ref(1, 10, 15));
    // `Thing` written inside `vec![…]`: expanded, yet spelled in the source.
    w.add_reference(macro_ref(4, 10, 15));

    let ridx_file = test_dir.0.join("my_crate.ridx");
    let mut f = File::create(&ridx_file).unwrap();
    w.write_to(&mut f).unwrap();
    drop(f);
    let mut ws = WorkspaceIndex::open(&ridx_file).unwrap();
    ws.set_workspace_root(Some(test_dir.0.clone()));

    let refs = ws.find_references_at("src/lib.rs", 2, 12).expect("references for Thing");
    let lines: Vec<u32> = refs.references.iter().map(|r| r.span.start_line()).collect();
    assert_eq!(lines, vec![4], "only the `Thing` written in `vec![…]` is a use: {lines:?}");
}
