/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! implementation lists the `impl` blocks of a trait, not the impl headers that only use it: the
//! trait as a generic argument (`Wrapper<Tr>`), as a bound (`impl<T: Tr>`) or in a `where` clause.

use crate::lsp::navigation::handle_implementation;
use crate::lsp::protocol::{path_to_uri, PositionConverter};
use crate::overlay::OverlayManager;
use ridx_format::{DefKind, IndexWriter, RawDefinition, RawReference, WorkspaceIndex};
use serde_json::json;
use std::fs;

const USES: &str = "impl Tr for S {}
impl Wrapper<Tr> for S {}
impl<T: Tr> Other for T {}
impl S where S: Tr {}
";

/// A reference to trait `t::Tr` on line `line` of `uses.rs`, over `Tr`.
fn reference(file: &str, trait_file: &str, line: u32, col: u32) -> RawReference {
    RawReference {
        file: file.to_string(),
        start_line: line,
        start_col: col,
        end_line: line,
        end_col: col + 2,
        target_def_id: Some(1),
        target_path: "t::Tr".to_string(),
        target_unit: None,
        target_file: Some(trait_file.to_string()),
        target_line: 1,
        target_col: 11,
        kind: DefKind::Trait,
        is_write: false,
        is_macro_expansion: false,
    }
}

#[test]
fn a_trait_used_by_an_impl_header_is_not_implemented_by_it() {
    let dir = std::env::temp_dir().join(format!("ridx_impl_head_{}", std::process::id()));
    let _ = fs::create_dir_all(&dir);
    let (trait_file, uses_file) = (dir.join("tr.rs"), dir.join("uses.rs"));
    fs::write(&trait_file, "pub trait Tr {}\n").unwrap();
    fs::write(&uses_file, USES).unwrap();
    let (t, u) = (trait_file.to_str().unwrap(), uses_file.to_str().unwrap());

    let mut w = IndexWriter::new("t", "0.1.0");
    w.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Trait,
        visibility: 0,
        is_macro_expansion: false,
        file: t.to_string(),
        name: "Tr".to_string(),
        start_line: 1,
        start_col: 11,
        end_line: 1,
        end_col: 13,
        path: "t::Tr".to_string(),
        signature: None,
    });
    // `Tr` sits at column 6 of line 1, 14 of line 2, 9 of line 3 and 17 of line 4.
    for (line, col) in [(1, 6), (2, 14), (3, 9), (4, 17)] {
        w.add_reference(reference(u, t, line, col));
    }
    w.write_to(&mut fs::File::create(dir.join("t.ridx")).unwrap()).unwrap();

    let ws = WorkspaceIndex::open(&dir).unwrap();
    let req = json!({"params": {
        "textDocument": { "uri": path_to_uri(t) },
        "position": { "line": 0, "character": 10 }
    }});
    let res = handle_implementation(&req, Some(&ws), &OverlayManager::new(), &mut PositionConverter::new());
    let mut lines: Vec<u64> = res.as_array().unwrap().iter().map(|l| l["range"]["start"]["line"].as_u64().unwrap()).collect();
    lines.sort();
    assert_eq!(lines, vec![0]);
    let _ = fs::remove_dir_all(&dir);
}
