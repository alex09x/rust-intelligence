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

fn crate_root_ref(file: &str, line: u32) -> RawReference {
    RawReference {
        file: file.to_string(),
        start_line: line,
        start_col: 5,
        end_line: line,
        end_col: 7,
        target_def_id: None,
        target_path: "tt".to_string(),
        target_unit: Some("tt-unit".to_string()),
        target_file: Some("crates/tt/src/lib.rs".to_string()),
        target_line: 1,
        target_col: 1,
        kind: DefKind::Mod,
        is_write: false,
        is_macro_expansion: false,
    }
}

#[test]
fn test_a_crate_root_is_referenced_by_its_name_in_consumers() {
    let test_dir = new_test_dir("ridx_crate_root_refs");

    let mut tt = IndexWriter::new("tt", "0.1.0");
    tt.set_unit_key("tt-unit");
    tt.add_definition(RawDefinition {
        def_id: 0,
        kind: DefKind::Mod,
        visibility: 1,
        is_macro_expansion: false,
        name: "tt".to_string(),
        path: "tt".to_string(),
        signature: None,
        file: "crates/tt/src/lib.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 40,
        end_col: 2,
    });
    let mut f = File::create(test_dir.0.join("tt.ridx")).unwrap();
    tt.write_to(&mut f).unwrap();

    let mut mbe = IndexWriter::new("mbe", "0.1.0");
    mbe.set_unit_key("mbe-unit");
    mbe.add_reference(crate_root_ref("crates/mbe/src/lib.rs", 27));
    mbe.add_reference(crate_root_ref("crates/mbe/src/lib.rs", 155));
    let mut f = File::create(test_dir.0.join("mbe.ridx")).unwrap();
    mbe.write_to(&mut f).unwrap();

    let ws = WorkspaceIndex::open(&test_dir.0).unwrap();
    let refs = ws.find_references_at("crates/tt/src/lib.rs", 1, 1).expect("references of the crate root");
    let mut lines: Vec<(&str, u32)> = refs.references.iter().map(|r| (r.file, r.span.start_line())).collect();
    lines.sort();
    assert_eq!(lines, vec![("crates/mbe/src/lib.rs", 27), ("crates/mbe/src/lib.rs", 155)], "every `tt::` in the consumer");
}
