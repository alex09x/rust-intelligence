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
use ridx_format::DefKind;
use ridx_test_support::fixture::SyntheticWorkspaceFixture;

/// `doc` with the item named at `$0` deleted, or why not.
fn deleted(doc: &str) -> Result<String, &'static str> {
    let offset = doc.find("$0").unwrap();
    let doc = doc.replacen("$0", "", 1);
    let span = item_span(&doc, offset)?;
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
}

#[test]
fn test_what_its_own_deletion_would_break_is_refused() {
    assert_eq!(deleted("pub trait T {\n    fn $0f(&self);\n}\n"), Err("traitItem"));
    assert_eq!(deleted("pub trait T {\n    fn f(&self);\n}\npub struct S;\nimpl T for S {\n    fn $0f(&self) {}\n}\n"), Err("traitItem"));
    assert_eq!(deleted("mod $0other;\n"), Err("moduleFile"), "its file would stay");
    assert_eq!(deleted("pub fn f() {\n    let $0x = 1;\n}\n"), Err("notAnItem"));
}

/// prod-code, m=20226: an item used is refused with its uses, an unused one gets its edit.
#[test]
fn test_the_uses_keep_an_item() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_del", "0.1.0").unwrap();
    let (_, ids) = ws.add_source_file_with_defs("pkg_del", "src/lib.rs", "pub struct «Used»;\npub struct «Unused»;\n", DefKind::Struct).unwrap();
    let code = "pub struct Used;\npub struct Unused;\n«impl Used» {}\n";
    ws.add_source_file_with_impl_refs("pkg_del", "src/lib.rs", code, "pkg_del", ids[0]).unwrap();
    let index = WorkspaceIndex::open(ws.build_crate_index("pkg_del").unwrap()).unwrap();
    let uri = ws.file_uri("pkg_del", "src/lib.rs").unwrap();
    let (ov, mut conv) = (OverlayManager::new(), PositionConverter::new());
    let at = |line: u32| json!({ "params": { "textDocument": { "uri": uri }, "position": { "line": line, "character": 12 } } });
    let used = handle_safe_delete(&at(0), Some(&index), &ov, &mut conv);
    assert_eq!((used["refused"].as_bool(), used["reason"].as_str()), (Some(true), Some("used")), "{used}");
    assert_eq!(used["remainingUses"][0]["range"]["start"]["line"], 2, "{used}");
    let unused = handle_safe_delete(&at(1), Some(&index), &ov, &mut conv);
    assert_eq!(unused["edit"]["changes"][&uri][0]["range"], json!({ "start": { "line": 1, "character": 0 }, "end": { "line": 2, "character": 0 } }), "{unused}");
    let unknown = handle_safe_delete(&at(1), None, &ov, &mut conv);
    assert_eq!(unknown["reason"], "unknownSymbol", "no index: its uses are not known");
}
