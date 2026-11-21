/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! `ridx/expandMacro`: a `macro_rules!` call expanded fully, or nothing.

use super::*;
use ridx_format::{IndexWriter, RawDefinition};

/// prod-code, m=20257: a macro in a parameter's type, expanded as source; one that calls a macro again is not.
#[test]
fn test_a_macro_call_is_expanded_fully_or_not_at_all() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    let lib = root.join("src/lib.rs");
    let doc = "macro_rules! my_ty {\n    () => { u32 };\n}\nmacro_rules! outer {\n    () => { my_ty!() };\n}\npub fn f(x: my_ty!()) -> u32 {\n    x\n}\npub fn g(x: outer!()) -> u32 {\n    x\n}\n";
    std::fs::write(&lib, doc).unwrap();
    let mut w = IndexWriter::new("w", "0.1.0");
    for (i, (name, line)) in [("my_ty", 1), ("outer", 4)].into_iter().enumerate() {
        w.add_definition(RawDefinition {
            def_id: i as u32 + 1,
            kind: DefKind::Macro,
            visibility: 0,
            is_macro_expansion: false,
            file: lib.to_string_lossy().to_string(),
            name: name.to_string(),
            start_line: line,
            start_col: 14,
            end_line: line,
            end_col: 14 + name.len() as u32,
            path: name.to_string(),
            signature: None,
        });
    }
    let shard = root.join("w.ridx");
    w.write_to(&mut std::fs::File::create(&shard).unwrap()).unwrap();
    let ws = WorkspaceIndex::open(&shard).unwrap();
    let file = lib.to_string_lossy().to_string();
    let at = doc.find("my_ty!()) -> u32").unwrap();
    assert_eq!(expand_macro_at(doc, &file, at, &ws), Some(("my_ty".to_string(), "u32".to_string())));
    assert_eq!(expand_macro_at(doc, &file, doc.find("outer!()) ").unwrap(), &ws), None, "it calls a macro again");
    assert_eq!(expand_macro_at(doc, &file, doc.find("x\n}\npub fn g").unwrap(), &ws), None, "no macro call there");
}
