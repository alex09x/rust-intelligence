/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use ridx::lsp::navigation::handle_document_highlight;
use ridx::lsp::protocol::PositionConverter;
use ridx::overlay::OverlayManager;
use ridx_format::{DefKind, IndexWriter, RawDefinition, RawReference, WorkspaceIndex};
use ridx_test_support::fixture::SyntheticWorkspaceFixture;
use serde_json::{json, Value};

fn make_pos_req(uri: &str, line: u32, col: u32) -> Value {
    json!({
        "params": {
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": col }
        }
    })
}

fn assert_range(range: &Value, start_line: u32, start_char: u32, end_line: u32, end_char: u32) {
    assert_eq!(range["start"]["line"], start_line);
    assert_eq!(range["start"]["character"], start_char);
    assert_eq!(range["end"]["line"], end_line);
    assert_eq!(range["end"]["character"], end_char);
}

#[test]
fn test_l1_document_highlight_dirty_overlay_skips_compiled_index_returns_empty() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_dirty", "0.1.0").unwrap();

    let (_, ids) = ws.add_source_file_with_defs("pkg_dirty", "src/lib.rs", "pub struct «CleanTarget»;\n", DefKind::Struct).unwrap();

    ws.add_refs_to_def("pkg_dirty", "src/lib.rs", "pub struct CleanTarget;\npub fn invoke() { let _ = «CleanTarget»; }\n", ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_dirty").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let file_path = ws.file_path("pkg_dirty", "src/lib.rs").unwrap();
    let file_str = file_path.to_str().unwrap();
    let uri = ws.file_uri("pkg_dirty", "src/lib.rs").unwrap();

    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    // 1. Control check: without overlay, query on CleanTarget returns 2 highlights (decl + ref)
    let req = make_pos_req(&uri, 0, 11);
    let res_control = handle_document_highlight(&req, Some(&index), &overlay_mgr, &mut conv);
    let items_control = res_control.as_array().expect("highlight array");
    assert_eq!(items_control.len(), 2);
    assert_range(&items_control[0]["range"], 0, 11, 0, 22);
    assert_range(&items_control[1]["range"], 1, 26, 1, 37);

    // 2. Open dirty overlay with modified unindexed text
    let dirty_text = "// edited content\npub fn unrelated() {}\n".to_string();
    overlay_mgr.open_with_provenance(&uri, file_str, 2, dirty_text, false);
    assert!(overlay_mgr.get(file_str).unwrap().is_dirty());

    // 3. Query documentHighlight on dirty buffer: compiled index is skipped, returns empty
    let res_dirty = handle_document_highlight(&req, Some(&index), &overlay_mgr, &mut conv);
    assert_eq!(res_dirty, json!([]));
}

#[test]
fn test_l1_document_highlight_overlay_local_binding_takes_precedence_over_compiled() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_prec", "0.1.0").unwrap();

    let (_, ids) = ws.add_source_file_with_defs("pkg_prec", "src/lib.rs", "pub struct «ShadowedItem»;\n", DefKind::Struct).unwrap();

    // Index compiled references matching the exact positions of the local bindings
    let full_code = "pub struct ShadowedItem;\npub fn run() {\n    let «ShadowedItem» = 42;\n    let _ = «ShadowedItem»;\n}\n";
    ws.add_refs_to_def("pkg_prec", "src/lib.rs", full_code, ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_prec").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let file_path = ws.file_path("pkg_prec", "src/lib.rs").unwrap();
    let file_str = file_path.to_str().unwrap();
    let uri = ws.file_uri("pkg_prec", "src/lib.rs").unwrap();

    // Open clean overlay matching disk content
    let clean_code = std::fs::read_to_string(&file_path).unwrap();
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    overlay_mgr.open_document(&uri, file_str, 1, clean_code);

    // 1. Query on the local binding at line 2 (0-based), character 8 ("    let ShadowedItem = 42;")
    // Local binding takes precedence: returns exactly 2 highlights from overlay (decl + use)
    // The compiled struct ShadowedItem on line 0 is NOT included due to early return
    let req_local = make_pos_req(&uri, 2, 8);
    let res_local = handle_document_highlight(&req_local, Some(&index), &overlay_mgr, &mut conv);
    let items_local = res_local.as_array().expect("highlight list");
    assert_eq!(items_local.len(), 2);
    assert_eq!(items_local[0]["kind"], 3);
    assert_range(&items_local[0]["range"], 2, 8, 2, 20);
    assert_eq!(items_local[1]["kind"], 1);
    assert_range(&items_local[1]["range"], 3, 12, 3, 24);

    // 2. Control query on compiled struct declaration at line 0, character 11
    // Proves compiled data exists: returns compiled decl + 2 compiled refs = 3 items
    let req_ctrl = make_pos_req(&uri, 0, 11);
    let res_ctrl = handle_document_highlight(&req_ctrl, Some(&index), &overlay_mgr, &mut conv);
    let items_ctrl = res_ctrl.as_array().expect("highlight list");
    assert_eq!(items_ctrl.len(), 3);
    assert_eq!(items_ctrl[0]["kind"], 3);
    assert_range(&items_ctrl[0]["range"], 0, 11, 0, 23);
    assert_eq!(items_ctrl[1]["kind"], 1);
    assert_range(&items_ctrl[1]["range"], 2, 8, 2, 20);
    assert_eq!(items_ctrl[2]["kind"], 1);
    assert_range(&items_ctrl[2]["range"], 3, 12, 3, 24);
}

#[test]
fn test_l1_document_highlight_target_zero_end_col_fallback_symbol_len() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_fb", "0.1.0").unwrap();

    let file_path = ws.file_path("pkg_fb", "src/lib.rs").unwrap();
    let file_str = file_path.to_str().unwrap();

    // Write source file to disk so PositionConverter can read lines if needed
    let code = "pub struct LegacyItem;\nfn use_item() { let _ = LegacyItem; }\n";
    ws.add_source_file("pkg_fb", "src/lib.rs", code).unwrap();

    // fixture helpers always emit end_col > 0; a raw shard is the only way to reach the fallback branch
    let mut writer = IndexWriter::new("pkg_fb", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Struct,
        visibility: 0,
        is_macro_expansion: false,
        file: file_str.to_string(),
        name: "LegacyItem".to_string(),
        start_line: 1,
        start_col: 12, // 1-based start column: "pub struct " is 11 chars
        end_line: 0,   // Intentionally 0
        end_col: 0,    // Intentionally 0 to trigger symbol len fallback
        path: "pkg_fb::LegacyItem".to_string(),
        signature: None,
    });
    writer.add_reference(RawReference {
        file: file_str.to_string(),
        start_line: 2,
        start_col: 25, // "fn use_item() { let _ = " is 24 chars -> col 25 (1-based)
        end_line: 2,
        end_col: 35, // 25 + 10 = 35
        target_def_id: Some(1),
        target_path: "pkg_fb::LegacyItem".to_string(),
        target_file: Some(file_str.to_string()),
        target_unit: Some("pkg_fb".to_string()),
        target_line: 1,
        target_col: 12,
        kind: DefKind::Struct,
        is_write: false,
        is_macro_expansion: false,
    });

    let shard_dir = ws.root_path().join("shard");
    std::fs::create_dir_all(&shard_dir).unwrap();
    let shard_path = shard_dir.join("pkg_fb.ridx");
    let mut f = std::fs::File::create(&shard_path).unwrap();
    writer.write_to(&mut f).unwrap();

    let index = WorkspaceIndex::open(&shard_path).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = format!("file://{}", file_str);

    // Cursor placed on the reference at line 1 (0-based), character 26 (inside LegacyItem)
    let req = make_pos_req(&uri, 1, 26);
    let res = handle_document_highlight(&req, Some(&index), &overlay_mgr, &mut conv);
    let items = res.as_array().expect("highlight list");
    assert_eq!(items.len(), 2);

    // Declaration: end_col fell back to start_col (11) + symbol_len (10) = 21
    assert_eq!(items[0]["kind"], 3);
    assert_range(&items[0]["range"], 0, 11, 0, 21);

    // Reference: line 1, char 24..34
    assert_eq!(items[1]["kind"], 1);
    assert_range(&items[1]["range"], 1, 24, 1, 34);
}

#[test]
fn test_l1_document_highlight_utf16_multibyte_declaration_and_references() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_utf16_hl", "0.1.0").unwrap();

    let (_, ids) = ws.add_source_file_with_defs("pkg_utf16_hl", "src/lib.rs", "/* 🚀 */ pub struct «Rocket»;\n", DefKind::Struct).unwrap();

    ws.add_refs_to_def("pkg_utf16_hl", "src/lib.rs", "/* 🚀 */ pub struct Rocket;\n/* 🦀 */ pub fn launch() { let _ = «Rocket»; }\n", ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_utf16_hl").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let uri = ws.file_uri("pkg_utf16_hl", "src/lib.rs").unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    // Column breakdown:
    // Line 0: "/* 🚀 */ pub struct Rocket;\n"
    //   Prefix: "/* " (3 chars, 3 UTF-16) + "🚀" (1 char, 2 UTF-16) + " */ pub struct " (15 chars, 15 UTF-16)
    //   Prefix chars = 19 -> 1-based char col = 20
    //   Prefix UTF-16 = 3 + 2 + 15 = 20 -> 0-based UTF-16 start col = 20
    //   "Rocket" length = 6 chars, 6 UTF-16 code units -> 0-based UTF-16 end col = 26
    //   Decl range: 0, 20, 0, 26
    //
    // Line 1: "/* 🦀 */ pub fn launch() { let _ = Rocket; }\n"
    //   Prefix: "/* " (3 chars, 3 UTF-16) + "🦀" (1 char, 2 UTF-16) + " */ pub fn launch() { let _ = " (30 chars, 30 UTF-16)
    //   Prefix chars = 34 -> 1-based char col = 35
    //   Prefix UTF-16 = 3 + 2 + 30 = 35 -> 0-based UTF-16 start col = 35
    //   "Rocket" length = 6 chars, 6 UTF-16 code units -> 0-based UTF-16 end col = 41
    //   Ref range: 1, 35, 1, 41

    // 1. Cursor on reference at line 1, character 37 (inside Rocket)
    let req_ref = make_pos_req(&uri, 1, 37);
    let res_ref = handle_document_highlight(&req_ref, Some(&index), &overlay_mgr, &mut conv);
    let items_ref = res_ref.as_array().expect("highlight list");
    assert_eq!(items_ref.len(), 2);
    assert_eq!(items_ref[0]["kind"], 3);
    assert_range(&items_ref[0]["range"], 0, 20, 0, 26);
    assert_eq!(items_ref[1]["kind"], 1);
    assert_range(&items_ref[1]["range"], 1, 35, 1, 41);

    // 2. Cursor on declaration at line 0, character 22 (inside Rocket)
    let req_decl = make_pos_req(&uri, 0, 22);
    let res_decl = handle_document_highlight(&req_decl, Some(&index), &overlay_mgr, &mut conv);
    let items_decl = res_decl.as_array().expect("highlight list");
    assert_eq!(items_decl.len(), 2);
    assert_eq!(items_decl[0]["kind"], 3);
    assert_range(&items_decl[0]["range"], 0, 20, 0, 26);
    assert_eq!(items_decl[1]["kind"], 1);
    assert_range(&items_decl[1]["range"], 1, 35, 1, 41);
}

#[test]
fn test_l1_document_highlight_cursor_on_compiled_reference_includes_decl() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_cur_ref", "0.1.0").unwrap();

    let (_, ids) = ws.add_source_file_with_defs("pkg_cur_ref", "src/lib.rs", "pub struct «TargetItem»;\n", DefKind::Struct).unwrap();

    ws.add_refs_to_def("pkg_cur_ref", "src/lib.rs", "pub struct TargetItem;\npub fn consume() { let _ = «TargetItem»; }\n", ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_cur_ref").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let uri = ws.file_uri("pkg_cur_ref", "src/lib.rs").unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    // Line 0: "pub struct TargetItem;\n" -> col 11..21
    // Line 1: "pub fn consume() { let _ = TargetItem; }\n" -> col 27..37
    // Cursor placed on the reference at line 1, character 29 (inside TargetItem)
    let req = make_pos_req(&uri, 1, 29);
    let res = handle_document_highlight(&req, Some(&index), &overlay_mgr, &mut conv);
    let items = res.as_array().expect("highlight list");

    assert_eq!(items.len(), 2);
    // Declaration is included with kind 3 (Write)
    assert_eq!(items[0]["kind"], 3);
    assert_range(&items[0]["range"], 0, 11, 0, 21);

    // Reference is included with kind 1 (Text)
    assert_eq!(items[1]["kind"], 1);
    assert_range(&items[1]["range"], 1, 27, 1, 37);
}

#[test]
fn test_l1_document_highlight_clean_overlay_input_column_via_overlay_conversion() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_clean_ov", "0.1.0").unwrap();

    let (_, ids) = ws.add_source_file_with_defs("pkg_clean_ov", "src/lib.rs", "/* 🚀 */ pub struct «SpaceStation»;\n", DefKind::Struct).unwrap();

    ws.add_refs_to_def("pkg_clean_ov", "src/lib.rs", "/* 🚀 */ pub struct SpaceStation;\n/* 🚀 */ pub fn dock() { let _ = «SpaceStation»; }\n", ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_clean_ov").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let file_path = ws.file_path("pkg_clean_ov", "src/lib.rs").unwrap();
    let file_str = file_path.to_str().unwrap();
    let uri = ws.file_uri("pkg_clean_ov", "src/lib.rs").unwrap();

    // Read saved clean text from disk
    let clean_text = std::fs::read_to_string(&file_path).unwrap();

    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    // Open clean overlay with is_compiled = true
    overlay_mgr.open_with_provenance(&uri, file_str, 1, clean_text, true);
    assert!(!overlay_mgr.get(file_str).unwrap().is_dirty());

    // Cursor on declaration: line 0, character 22
    // Line 0 breakdown:
    // Prefix: "/* " (3 chars, 3 UTF-16) + "🚀" (1 char, 2 UTF-16) + " */ pub struct " (15 chars, 15 UTF-16) = 19 chars, 20 UTF-16 units
    // "SpaceStation" spans UTF-16 cols 20..32, 1-based char cols 20..32
    // Cursor at UTF-16 22 is converted by overlay.utf16_col_to_char_col(1, 22) -> char col 22
    let req_decl = make_pos_req(&uri, 0, 22);
    let res_decl = handle_document_highlight(&req_decl, Some(&index), &overlay_mgr, &mut conv);
    let items_decl = res_decl.as_array().expect("highlight list");
    assert_eq!(items_decl.len(), 2);
    assert_eq!(items_decl[0]["kind"], 3);
    assert_range(&items_decl[0]["range"], 0, 20, 0, 32);
    assert_eq!(items_decl[1]["kind"], 1);
    assert_range(&items_decl[1]["range"], 1, 33, 1, 45);

    // Cursor on reference: line 1, character 35
    // Line 1 breakdown:
    // Prefix: "/* " (3) + "🚀" (1 char, 2 UTF-16) + " */ pub fn dock() { let _ = " (28) = 32 chars, 33 UTF-16 units
    // "SpaceStation" spans UTF-16 cols 33..45, 1-based char cols 33..45
    // Cursor at UTF-16 35 is converted by overlay.utf16_col_to_char_col(2, 35) -> char col 35
    let req_ref = make_pos_req(&uri, 1, 35);
    let res_ref = handle_document_highlight(&req_ref, Some(&index), &overlay_mgr, &mut conv);
    let items_ref = res_ref.as_array().expect("highlight list");
    assert_eq!(items_ref.len(), 2);
    assert_eq!(items_ref[0]["kind"], 3);
    assert_range(&items_ref[0]["range"], 0, 20, 0, 32);
    assert_eq!(items_ref[1]["kind"], 1);
    assert_range(&items_ref[1]["range"], 1, 33, 1, 45);
}
