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
use ridx_format::{DefKind, IndexWriter, RawDefinition};

/// Review of f60136bc: an index is current for a file while every other source of its crate is as it read it.
#[test]
fn test_a_crate_is_current_while_its_other_sources_are_as_read() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    let (main, child) = (root.join("src/main.rs"), root.join("src/child.rs"));
    let (main_text, child_text) = ("mod child;\nfn main() {}\n", "pub fn f() {}\n");
    std::fs::write(&main, main_text).unwrap();
    std::fs::write(&child, child_text).unwrap();
    let mut w = IndexWriter::new("w", "0.1.0");
    for (i, (file, name)) in [(&main, "main"), (&child, "f")].into_iter().enumerate() {
        let file = file.to_string_lossy().to_string();
        w.add_definition(RawDefinition {
            def_id: i as u32 + 1,
            kind: DefKind::Fn,
            visibility: 0,
            is_macro_expansion: false,
            file,
            name: name.to_string(),
            start_line: 1,
            start_col: 1,
            end_line: 1,
            end_col: 2,
            path: name.to_string(),
            signature: None,
        });
    }
    w.add_file_hash(&main.to_string_lossy(), ridx_format::normalized_source_hash(main_text.as_bytes()));
    w.add_file_hash(&child.to_string_lossy(), ridx_format::normalized_source_hash(child_text.as_bytes()));
    let shard = root.join("w.ridx");
    w.write_to(&mut std::fs::File::create(&shard).unwrap()).unwrap();
    let ws = WorkspaceIndex::open(&shard).unwrap();
    let ov = OverlayManager::new();
    let child_file = child.to_string_lossy().to_string();
    assert!(crates_current(&ws, &ov, &child_file));
    std::fs::write(&child, "pub fn f() { 1; }\n").unwrap();
    assert!(crates_current(&ws, &ov, &child_file), "the file the assist reads is its own");
    std::fs::write(&main, "macro_rules! println { () => { const N: i32 = 3; }; }\nmod child;\nfn main() {}\n").unwrap();
    assert!(!crates_current(&ws, &ov, &child_file), "a parent changed since");
    assert!(!crates_current(&ws, &ov, &root.join("src/new.rs").to_string_lossy()), "a file the index never read");
}
