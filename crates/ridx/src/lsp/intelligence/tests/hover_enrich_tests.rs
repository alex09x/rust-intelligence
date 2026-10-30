/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use crate::lsp::intelligence::handle_hover;
use ridx_format::{DefKind, IndexWriter, RawDefinition, RawReference, WorkspaceIndex};
use serde_json::json;
use std::fs;

#[test]
fn test_hover_trait_full_signature_docs_provenance_and_blast_radius() {
    let temp_dir = std::env::temp_dir().join(format!("ridx_hover_enrich_{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp_dir);
    let _ = fs::create_dir_all(temp_dir.join("src"));
    let src_file = temp_dir.join("src/lib.rs");
    fs::write(
        &src_file,
        "/// A data structure that can be deserialized.\n/// Supports zero-copy borrowing via `'de`.\n#[cfg_attr(\n    not(no_diagnostic_namespace),\n    diagnostic::on_unimplemented(message = \"not satisfied\")\n)]\npub trait Deserialize<'de>: Sized {\n    fn deserialize(&self);\n}\n",
    )
    .unwrap();
    let src_str = src_file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("serde_core", "1.0.228");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Trait,
        visibility: 0,
        is_macro_expansion: false,
        file: "src/lib.rs".to_string(),
        name: "Deserialize".to_string(),
        start_line: 7,
        start_col: 11,
        end_line: 7,
        end_col: 22,
        path: "serde_core::de::Deserialize".to_string(),
        signature: None,
    });
    writer.add_reference(RawReference {
        file: "src/lib.rs".to_string(),
        start_line: 20,
        start_col: 10,
        end_line: 20,
        end_col: 21,
        target_def_id: Some(1),
        target_path: "serde_core::de::Deserialize".to_string(),
        target_unit: None,
        target_file: Some("src/lib.rs".to_string()),
        target_line: 7,
        target_col: 11,
        kind: DefKind::Trait,
        is_write: false,
        is_macro_expansion: false,
    });
    let ridx_file = temp_dir.join("serde_core.ridx");
    let mut f = fs::File::create(&ridx_file).unwrap();
    writer.write_to(&mut f).unwrap();
    drop(f);

    let mut ws = WorkspaceIndex::open(&ridx_file).unwrap();
    ws.set_workspace_root(Some(temp_dir.clone()));
    let overlay_mgr = crate::overlay::OverlayManager::new();
    let mut conv = crate::lsp::protocol::PositionConverter::new();

    let uri = format!("file://{src_str}");
    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 6, "character": 12 }
        }
    });
    let res = handle_hover(&req, Some(&ws), &overlay_mgr, &mut conv).unwrap();
    let val = res["contents"]["value"].as_str().unwrap();
    assert!(
        val.contains("pub trait Deserialize<'de>: Sized"),
        "expected full trait signature with lifetime and supertrait, got: {val}"
    );
    assert!(
        val.contains("A data structure that can be deserialized."),
        "expected extracted doc comment across multi-line attribute, got: {val}"
    );
    assert!(
        val.contains("**Provenance:** `serde_core::de::Deserialize` · crate `serde_core v1.0.228` (`src/lib.rs:7`)"),
        "expected provenance footer, got: {val}"
    );
    assert!(
        val.contains("**Blast Radius:** 1 direct reference across 1 file in 1 crate"),
        "expected blast radius footer, got: {val}"
    );
    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_blast_radius_deduplicates_file_across_shards_and_groups_package_targets() {
    let temp_dir =
        std::env::temp_dir().join(format!("ridx_hover_blast_dedup_{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp_dir);
    let ridx_dir = temp_dir.join("target/ridx");
    let _ = fs::create_dir_all(&ridx_dir);
    let _ = fs::create_dir_all(temp_dir.join("src"));
    let src_file = temp_dir.join("src/lib.rs");
    fs::write(&src_file, "pub struct SharedItem;\n").unwrap();

    // Shard 1: lib target (`src/lib.rs` gets file_id = 0)
    let mut w_lib = IndexWriter::new("my_pkg", "0.1.0");
    w_lib.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Struct,
        visibility: 0,
        is_macro_expansion: false,
        file: "src/lib.rs".to_string(),
        name: "SharedItem".to_string(),
        start_line: 1,
        start_col: 12,
        end_line: 1,
        end_col: 22,
        path: "my_pkg::SharedItem".to_string(),
        signature: None,
    });
    w_lib.add_reference(RawReference {
        file: "src/lib.rs".to_string(),
        start_line: 10,
        start_col: 5,
        end_line: 10,
        end_col: 15,
        target_def_id: Some(1),
        target_path: "my_pkg::SharedItem".to_string(),
        target_unit: None,
        target_file: Some("src/lib.rs".to_string()),
        target_line: 1,
        target_col: 12,
        kind: DefKind::Struct,
        is_write: false,
        is_macro_expansion: false,
    });
    w_lib
        .write_to(&mut fs::File::create(ridx_dir.join("my_pkg-lib.ridx")).unwrap())
        .unwrap();

    // Shard 2: unit-test target where `src/other.rs` is file_id = 0 and `src/lib.rs` is file_id = 1,
    // plus a second reference in `src/lib.rs` at line 25.
    let mut w_test = IndexWriter::new("my_pkg", "0.1.0");
    w_test.get_or_insert_file("src/other.rs");
    w_test.add_reference(RawReference {
        file: "src/lib.rs".to_string(),
        start_line: 10,
        start_col: 5,
        end_line: 10,
        end_col: 15,
        target_def_id: None,
        target_path: "my_pkg::SharedItem".to_string(),
        target_unit: None,
        target_file: Some("src/lib.rs".to_string()),
        target_line: 1,
        target_col: 12,
        kind: DefKind::Struct,
        is_write: false,
        is_macro_expansion: false,
    });
    w_test.add_reference(RawReference {
        file: "src/lib.rs".to_string(),
        start_line: 25,
        start_col: 5,
        end_line: 25,
        end_col: 15,
        target_def_id: None,
        target_path: "my_pkg::SharedItem".to_string(),
        target_unit: None,
        target_file: Some("src/lib.rs".to_string()),
        target_line: 1,
        target_col: 12,
        kind: DefKind::Struct,
        is_write: false,
        is_macro_expansion: false,
    });
    w_test
        .write_to(&mut fs::File::create(ridx_dir.join("my_pkg-test.ridx")).unwrap())
        .unwrap();

    let mut ws = WorkspaceIndex::open(&temp_dir).unwrap();
    ws.set_workspace_root(Some(temp_dir.clone()));
    let def = ws.find_definition_at("src/lib.rs", 1, 14).unwrap();
    let blast = ws.direct_blast_radius(&def);
    assert_eq!(
        blast.direct_refs, 2,
        "duplicate ref at line 10 must be counted once"
    );
    assert_eq!(
        blast.files, 1,
        "src/lib.rs across two shards with different file_ids must be counted once"
    );
    assert_eq!(
        blast.crates, 1,
        "lib and unit-test targets of the same package must be counted as 1 crate"
    );
    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_hover_doc_comments_with_cfg_attr_doc_and_derive() {
    let temp_dir =
        std::env::temp_dir().join(format!("ridx_hover_cfg_attr_doc_{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp_dir);
    let _ = fs::create_dir_all(temp_dir.join("src"));
    let src_file = temp_dir.join("src/lib.rs");
    fs::write(
        &src_file,
        "/// Doc line before derive.\n#[derive(Debug, Clone)]\n/// Doc line after derive.\n#[cfg_attr(not(no_doc), doc = \"Single-line cfg_attr doc.\")]\n#[cfg_attr(\n    docsrs,\n    doc = \"Multi-line cfg_attr doc.\"\n)]\npub struct SerdeAttrItem;\n",
    )
    .unwrap();
    let src_str = src_file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("serde_attr", "1.0.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Struct,
        visibility: 0,
        is_macro_expansion: false,
        file: "src/lib.rs".to_string(),
        name: "SerdeAttrItem".to_string(),
        start_line: 9,
        start_col: 12,
        end_line: 9,
        end_col: 25,
        path: "serde_attr::SerdeAttrItem".to_string(),
        signature: None,
    });
    let ridx_file = temp_dir.join("serde_attr.ridx");
    writer
        .write_to(&mut fs::File::create(&ridx_file).unwrap())
        .unwrap();

    let mut ws = WorkspaceIndex::open(&ridx_file).unwrap();
    ws.set_workspace_root(Some(temp_dir.clone()));
    let overlay_mgr = crate::overlay::OverlayManager::new();
    let mut conv = crate::lsp::protocol::PositionConverter::new();

    let uri = format!("file://{src_str}");
    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 8, "character": 14 }
        }
    });
    let res = handle_hover(&req, Some(&ws), &overlay_mgr, &mut conv).unwrap();
    let val = res["contents"]["value"].as_str().unwrap();
    assert!(
        val.contains("Doc line before derive.\nDoc line after derive.\nSingle-line cfg_attr doc.\nMulti-line cfg_attr doc."),
        "expected doc lines before/after #[derive] and from #[cfg_attr(..., doc = ...)], got: {val}"
    );
    let _ = fs::remove_dir_all(&temp_dir);
}
