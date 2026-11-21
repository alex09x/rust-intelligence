/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! `ridx/safeDelete`: what it deletes, what it refuses, and the uses that keep an item.

use super::*;
use crate::lsp::editing::tests::code_action_assists_tests::assert_rustc_compiles;
use ridx_format::{DefKind, IndexWriter, RawDefinition, RawReference};

/// `doc` with the item named at `$0` deleted, or why not.
fn deleted(doc: &str) -> Result<String, &'static str> {
    let offset = doc.find("$0").unwrap();
    let doc = doc.replacen("$0", "", 1);
    let span = plan(&doc, offset)?.span;
    let mut out = doc.clone();
    out.replace_range(span, "");
    Ok(out)
}

#[test]
fn test_an_item_goes_with_its_doc_and_its_lines() {
    let got = deleted("pub fn a() {}\n\n/// Twice `x`.\n#[inline]\npub fn $0twice(x: u32) -> u32 {\n    x * 2\n}\n\npub fn b() {}\n").unwrap();
    assert_eq!(got, "pub fn a() {}\n\npub fn b() {}\n");
    assert_rustc_compiles(&got, "safe delete");
    let got = deleted("pub struct S;\nimpl S {\n    pub fn $0one() -> u8 {\n        1\n    }\n    pub fn two() -> u8 {\n        2\n    }\n}\n").unwrap();
    assert_eq!(got, "pub struct S;\nimpl S {\n    pub fn two() -> u8 {\n        2\n    }\n}\n", "a method of an inherent impl");
    assert_eq!(deleted("pub const $0A: u8 = 1; pub const B: u8 = 2;\n").unwrap(), "pub const B: u8 = 2;\n", "not alone on its line");
    assert_eq!(deleted("mod $0inner {\n    pub fn f() {}\n}\n").unwrap(), "");
    // Review of d753d993: ordinary comments that look like doc comments stay.
    assert_eq!(deleted("//// separator\n/*** block */\npub fn $0f() {}\n").unwrap(), "//// separator\n/*** block */\n");
}

#[test]
fn test_what_its_own_deletion_would_break_is_refused() {
    assert_eq!(deleted("pub trait T {\n    fn $0f(&self);\n}\n"), Err("traitItem"));
    assert_eq!(deleted("pub trait T {\n    fn f(&self);\n}\npub struct S;\nimpl T for S {\n    fn $0f(&self) {}\n}\n"), Err("traitItem"));
    assert_eq!(deleted("mod $0other;\n"), Err("moduleFile"), "its file would stay");
    assert_eq!(deleted("pub fn f() {\n    let $0x = 1;\n}\n"), Err("notAnItem"));
    // Review of d753d993: used though nothing in the source names it.
    assert_eq!(deleted("fn $0main() {}\n"), Err("entryPoint"));
    assert_eq!(deleted("#[no_mangle]\npub extern \"C\" fn $0hook() {}\n"), Err("externallyUsed"));
}

/// An inline module's declarations are all checked.
#[test]
fn test_a_container_names_its_members() {
    let doc = "mod m {\n    pub fn child() {}\n    pub struct S;\n}\n";
    let names: Vec<&str> = plan(doc, 4).unwrap().names.iter().map(|r| &doc[r.clone()]).collect();
    assert_eq!(names, ["m", "child", "S"]);
}

/// A workspace whose `src/lib.rs` is `lib` with `defs` (name, kind, 1-based line and column), indexed from that very
/// text, and whose `src/other.rs` holds `refs` (1-based line and column, the index of the definition used).
fn workspace(lib: &str, defs: &[(&str, DefKind, u32, u32)], refs: &[(u32, u32, usize)]) -> (tempfile::TempDir, std::path::PathBuf, WorkspaceIndex) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    let (lib_path, other) = (root.join("src/lib.rs"), root.join("src/other.rs"));
    std::fs::write(&lib_path, lib).unwrap();
    let mut w = IndexWriter::new("w", "0.1.0");
    w.add_file_hash(&lib_path.to_string_lossy(), ridx_format::normalized_source_hash(lib.as_bytes()));
    let paths: Vec<String> =
        defs.iter().enumerate().map(|(i, d)| defs.iter().take(i).filter(|m| m.1 == DefKind::Mod).map(|m| format!("{}::", m.0)).collect::<String>() + d.0).collect();
    for (i, (name, kind, line, col)) in defs.iter().enumerate() {
        let path = paths[i].clone();
        w.add_definition(RawDefinition {
            def_id: i as u32 + 1,
            kind: *kind,
            visibility: 0,
            is_macro_expansion: false,
            file: lib_path.to_string_lossy().to_string(),
            name: name.to_string(),
            start_line: *line,
            start_col: *col,
            end_line: *line,
            end_col: col + name.chars().count() as u32,
            path,
            signature: None,
        });
    }
    for (line, col, def) in refs {
        let (name, kind, tl, tc) = defs[*def];
        w.add_reference(RawReference {
            file: other.to_string_lossy().to_string(),
            start_line: *line,
            start_col: *col,
            end_line: *line,
            end_col: col + name.len() as u32,
            target_def_id: Some(*def as u32 + 1),
            target_path: paths[*def].clone(),
            target_file: Some(lib_path.to_string_lossy().to_string()),
            target_unit: None,
            target_line: tl,
            target_col: tc,
            kind,
            is_write: false,
            is_macro_expansion: false,
        });
    }
    let shard = root.join("w.ridx");
    w.write_to(&mut std::fs::File::create(&shard).unwrap()).unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    (dir, lib_path, index)
}

fn at(lib: &std::path::Path, line: u32, character: u32) -> Value {
    json!({ "params": { "textDocument": { "uri": format!("file://{}", lib.display()) }, "position": { "line": line, "character": character } } })
}

/// prod-code, m=20226: an item used is refused with its uses, an unused one gets its edit.
#[test]
fn test_the_uses_keep_an_item() {
    let lib = "pub fn used() {}\npub fn unused() {}\n";
    let (_d, path, index) = workspace(lib, &[("used", DefKind::Fn, 1, 8), ("unused", DefKind::Fn, 2, 8)], &[(3, 5, 0)]);
    let (ov, mut conv) = (OverlayManager::new(), PositionConverter::new());
    let used = handle_safe_delete(&at(&path, 0, 8), Some(&index), &ov, &mut conv);
    assert_eq!((used["reason"].as_str(), used["remainingUses"][0]["range"]["start"]["line"].as_u64()), (Some("used"), Some(2)), "{used}");
    let unused = handle_safe_delete(&at(&path, 1, 8), Some(&index), &ov, &mut conv);
    let uri = format!("file://{}", path.display());
    assert_eq!(unused["edit"]["changes"][&uri][0]["range"], json!({ "start": { "line": 1, "character": 0 }, "end": { "line": 2, "character": 0 } }), "{unused}");
    assert_eq!(handle_safe_delete(&at(&path, 1, 8), None, &ov, &mut conv)["reason"], "unknownSymbol", "no index: its uses are not known");
}

/// Review of d753d993: the index must be of this text and know this item; a member's use keeps its container; an
/// unsaved caller elsewhere keeps the item; a name with a combining mark is no panic.
#[test]
fn test_safe_delete_fails_closed() {
    let lib = "pub fn used() {}\npub fn unused() {}\n";
    let (_d, path, index) = workspace(lib, &[("used", DefKind::Fn, 1, 8), ("unused", DefKind::Fn, 2, 8)], &[(3, 5, 0)]);
    let mut conv = PositionConverter::new();
    let mut ov = OverlayManager::new();
    let uri = format!("file://{}", path.display());
    ov.open_document(&uri, &path.to_string_lossy(), 2, "pub fn unused() {}\npub fn used() {}\n".to_string());
    assert_eq!(handle_safe_delete(&at(&path, 0, 8), Some(&index), &ov, &mut conv)["reason"], "staleIndex", "the index is of another text");
    let mut ov = OverlayManager::new();
    let other = path.with_file_name("g.rs");
    ov.open_document(&format!("file://{}", other.display()), &other.to_string_lossy(), 1, "pub fn g() {}\n".to_string());
    ov.apply_content_changes(&other.to_string_lossy(), 2, &[json!({ "text": "pub fn g() { crate::unused() }\n" })]);
    assert_eq!(handle_safe_delete(&at(&path, 1, 8), Some(&index), &ov, &mut conv)["reason"], "unsavedUses");

    let lib = "mod m {\n    pub fn child() {}\n}\n";
    let (_d, path, index) = workspace(lib, &[("m", DefKind::Mod, 1, 5), ("child", DefKind::Fn, 2, 12)], &[(3, 5, 1)]);
    let got = handle_safe_delete(&at(&path, 0, 4), Some(&index), &OverlayManager::new(), &mut conv);
    assert_eq!(got["reason"], "used", "its child is used: {got}");

    let lib = "pub fn a\u{0301}b() {}\n";
    let (_d, path, _index) = workspace(lib, &[], &[]);
    assert_eq!(handle_safe_delete(&at(&path, 0, 9), None, &OverlayManager::new(), &mut conv)["reason"], "unknownSymbol");
}
