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

/// A file `doc` with the `macro_rules!` macros `defs` (name, 1-based line of `macro_rules!`) indexed.
fn indexed(doc: &str, defs: &[(&str, u32)]) -> (tempfile::TempDir, String, WorkspaceIndex) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    let lib = root.join("src/lib.rs");
    std::fs::write(&lib, doc).unwrap();
    let mut w = IndexWriter::new("w", "0.1.0");
    for (i, (name, line)) in defs.iter().enumerate() {
        w.add_definition(RawDefinition {
            def_id: i as u32 + 1,
            kind: DefKind::Macro,
            visibility: 0,
            is_macro_expansion: false,
            file: lib.to_string_lossy().to_string(),
            name: name.to_string(),
            start_line: *line,
            start_col: 14,
            end_line: *line,
            end_col: 14 + name.len() as u32,
            path: name.to_string(),
            signature: None,
        });
    }
    let shard = root.join("w.ridx");
    w.write_to(&mut std::fs::File::create(&shard).unwrap()).unwrap();
    let ws = WorkspaceIndex::open(&shard).unwrap();
    (dir, lib.to_string_lossy().to_string(), ws)
}

/// Review of d2cb318a: a call found by the syntax tree, a name the standard library also uses, completeness told by
/// the call syntax, an `expr` fragment not rendered as text.
#[test]
fn test_the_expansion_is_faithful_or_none() {
    let doc = "macro_rules! value {\n    () => { 1u8 };\n}\nmacro_rules! println {\n    () => { 3u8 };\n}\nmacro_rules! ne {\n    () => { 1 != 2 };\n}\nmacro_rules! raw {\n    () => { r#value!() };\n}\nmacro_rules! twice {\n    ($x:expr) => { $x * 2 };\n}\npub fn f() -> u8 {\n    value /* gap */ !() + println!()\n}\n";
    let (_d, file, ws) = indexed(doc, &[("value", 1), ("println", 4), ("ne", 7), ("raw", 10), ("twice", 13)]);
    let at = |s: &str| expand_macro_at(doc, &file, doc.rfind(s).unwrap(), &ws);
    assert_eq!(at("value /*"), Some(("value".to_string(), "1u8".to_string())));
    assert_eq!(at("println!()"), Some(("println".to_string(), "3u8".to_string())), "the workspace's own println");
    let calls = |name: &str, args: &str| {
        let d = format!("{doc}pub fn g() {{ let _ = {name}!({args}); }}\n");
        let (_d, file, ws) = indexed(&d, &[("value", 1), ("println", 4), ("ne", 7), ("raw", 10), ("twice", 13)]);
        expand_macro_at(&d, &file, d.rfind(&format!("{name}!")).unwrap(), &ws)
    };
    assert_eq!(calls("ne", ""), Some(("ne".to_string(), "1 != 2".to_string())), "`!=` is no call");
    assert_eq!(calls("raw", ""), None, "it calls `r#value!` again");
    assert_eq!(calls("twice", "1 + 2"), None, "an `expr` fragment");
}

/// Review of 94c450dc: a `ty` fragment is one node where it goes, as an `expr` one is; a qualified call is only the
/// macro the index resolved it to; arguments not closed by their delimiter are no call; the path's head is read from
/// its tokens.
#[test]
fn test_fragments_paths_and_unclosed_arguments() {
    let defs = "macro_rules! borrowed {\n    ($t:ty) => { &'static $t };\n}\nmacro_rules! id {\n    ($x:tt) => { $x };\n}\nmacro_rules! println {\n    () => { 3u8 };\n}\n";
    let expand = |body: &str, at: &str| {
        let d = format!("{defs}{body}");
        let (_d, file, ws) = indexed(&d, &[("borrowed", 1), ("id", 4), ("println", 7)]);
        expand_macro_at(&d, &file, d.rfind(at).unwrap(), &ws)
    };
    assert_eq!(expand("pub fn g() -> u8 { id!(12) }\n", "id!"), Some(("id".to_string(), "12".to_string())));
    assert_eq!(expand("pub fn g() { let _: borrowed!(dyn std::fmt::Debug + Send) = &1; }\n", "borrowed!"), None, "a `ty` fragment");
    assert_eq!(expand("pub fn g() { std::println!(\"S\"); }\n", "println!"), None, "the standard library's, by its path");
    for body in ["pub fn g() -> u8 { id!(12\n", "pub fn g() -> u8 { id!(\u{e9}\n", "pub fn g() -> u8 { id!(\n"] {
        assert_eq!(expand(body, "id!"), None, "{body}");
    }
    let (_d, file, ws) = indexed(defs, &[]);
    assert_ne!(super::crate_prefix(&ws, &file, "", "crate::", Some("dep")).as_deref(), Some("::crate"), "`crate /* c */ ::` is `crate::`");
}
