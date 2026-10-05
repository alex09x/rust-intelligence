/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! LSP call hierarchy and type hierarchy handlers.

use super::protocol::{lsp_symbol_kind, path_to_uri, uri_to_path, PositionConverter};
use crate::overlay::OverlayManager;
use ridx_format::{DefKind, WorkspaceIndex};
use serde_json::{json, Value};

pub fn handle_prepare_call_hierarchy(
    msg: &Value,
    workspace: Option<&WorkspaceIndex>,
    overlay_mgr: &OverlayManager,
    conv: &mut PositionConverter,
) -> Value {
    let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
    let file = uri_to_path(uri);
    let line_0based = msg["params"]["position"]["line"].as_u64().unwrap_or(0) as u32;
    let utf16_col_0based = msg["params"]["position"]["character"].as_u64().unwrap_or(0) as u32;
    let line_1based = line_0based + 1;

    let overlay_opt = overlay_mgr.get(&file);
    let col_1based = if let Some(overlay) = overlay_opt {
        overlay.utf16_col_to_char_col(line_1based, utf16_col_0based)
    } else {
        conv.utf16_col_to_char_col(&file, line_1based, utf16_col_0based)
    };

    let mut items = Vec::new();

    // 1. Check overlay
    if let Some(def) = overlay_mgr.find_definition_at(&file, line_1based, col_1based) {
        if matches!(def.kind, DefKind::Fn | DefKind::AssocFn) {
            let sc = conv.char_col_to_utf16_col(&def.file, def.start_line, def.start_col);
            let ec = conv.char_col_to_utf16_col(&def.file, def.end_line, def.end_col);
            let range = json!({
                "start": { "line": def.start_line.saturating_sub(1), "character": sc },
                "end": { "line": def.end_line.saturating_sub(1), "character": ec }
            });
            items.push(json!({
                "name": def.name,
                "kind": lsp_symbol_kind(def.kind),
                "uri": path_to_uri(&def.file),
                "range": range,
                "selectionRange": range,
                "data": { "name": def.name }
            }));
            return json!(items);
        }
    }

    // 2. Check workspace
    if let Some(ws) = workspace {
        if let Some(def) = ws.find_definition_at(&file, line_1based, col_1based) {
            if matches!(def.kind, DefKind::Fn | DefKind::AssocFn) {
                let resolved_path = def.resolved_path();
                let path_str = resolved_path.to_str().unwrap_or(def.file);
                let uri = path_to_uri(path_str);
                let sc = conv.char_col_to_utf16_col(path_str, def.start_line, def.start_col);
                let ec = conv.char_col_to_utf16_col(path_str, def.end_line, def.end_col);
                let range = json!({
                    "start": { "line": def.start_line.saturating_sub(1), "character": sc },
                    "end": { "line": def.end_line.saturating_sub(1), "character": ec }
                });
                items.push(json!({
                    "name": def.name,
                    "kind": lsp_symbol_kind(def.kind),
                    "uri": uri,
                    "range": range,
                    "selectionRange": range,
                    "data": { "name": def.name, "path": def.path }
                }));
            }
        }
    }

    json!(items)
}

pub fn handle_call_hierarchy_incoming_calls(
    msg: &Value,
    workspace: Option<&WorkspaceIndex>,
    conv: &mut PositionConverter,
) -> Value {
    let item = &msg["params"]["item"];
    let name = item["name"].as_str().unwrap_or("");

    let Some(ws) = workspace else {
        return json!([]);
    };

    let mut incoming = Vec::new();
    let symbols = ws.search_symbols(name);

    for sym in symbols {
        if sym.name == name && matches!(sym.kind, DefKind::Fn | DefKind::AssocFn) {
            let refs = ws.find_references(sym.path);
            for r in refs {
                let r_path = r.resolved_path();
                let r_str = r_path.to_str().unwrap_or(r.file);
                let r_uri = path_to_uri(r_str);
                let span = r.span;
                let sc = conv.char_col_to_utf16_col(r_str, span.start_line(), span.start_col());
                let ec = conv.char_col_to_utf16_col(r_str, span.end_line(), span.end_col());
                let call_range = json!({
                    "start": { "line": span.start_line().saturating_sub(1), "character": sc },
                    "end": { "line": span.end_line().saturating_sub(1), "character": ec }
                });

                // Find the enclosing function definition in the referring file
                let caller_def = ws.search_symbols("").into_iter().find(|d| {
                    d.file == r.file
                        && matches!(d.kind, DefKind::Fn | DefKind::AssocFn)
                        && d.span.start_line() <= span.start_line()
                        && d.span.end_line() >= span.end_line()
                });

                let caller_item = if let Some(caller) = caller_def {
                    let c_path = caller.resolved_path();
                    let c_str = c_path.to_str().unwrap_or(caller.file);
                    let c_uri = path_to_uri(c_str);
                    let c_sc = conv.char_col_to_utf16_col(
                        c_str,
                        caller.span.start_line(),
                        caller.span.start_col(),
                    );
                    let c_ec = conv.char_col_to_utf16_col(
                        c_str,
                        caller.span.end_line(),
                        caller.span.end_col(),
                    );
                    let c_range = json!({
                        "start": { "line": caller.span.start_line().saturating_sub(1), "character": c_sc },
                        "end": { "line": caller.span.end_line().saturating_sub(1), "character": c_ec }
                    });
                    json!({
                        "name": caller.name,
                        "kind": lsp_symbol_kind(caller.kind),
                        "uri": c_uri,
                        "range": c_range,
                        "selectionRange": c_range
                    })
                } else {
                    json!({
                        "name": format!("fn in {}", r.file),
                        "kind": 12,
                        "uri": r_uri,
                        "range": call_range,
                        "selectionRange": call_range
                    })
                };

                incoming.push(json!({
                    "from": caller_item,
                    "fromRanges": [call_range]
                }));
            }
        }
    }

    json!(incoming)
}

pub fn handle_call_hierarchy_outgoing_calls(
    _msg: &Value,
    _workspace: Option<&WorkspaceIndex>,
) -> Value {
    json!([])
}

pub fn handle_prepare_type_hierarchy(
    msg: &Value,
    workspace: Option<&WorkspaceIndex>,
    overlay_mgr: &OverlayManager,
    conv: &mut PositionConverter,
) -> Value {
    let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
    let file = uri_to_path(uri);
    let line_0based = msg["params"]["position"]["line"].as_u64().unwrap_or(0) as u32;
    let utf16_col_0based = msg["params"]["position"]["character"].as_u64().unwrap_or(0) as u32;
    let line_1based = line_0based + 1;

    let overlay_opt = overlay_mgr.get(&file);
    let col_1based = if let Some(overlay) = overlay_opt {
        overlay.utf16_col_to_char_col(line_1based, utf16_col_0based)
    } else {
        conv.utf16_col_to_char_col(&file, line_1based, utf16_col_0based)
    };

    let mut items = Vec::new();

    // 1. Check overlay
    if let Some(def) = overlay_mgr.find_definition_at(&file, line_1based, col_1based) {
        if matches!(
            def.kind,
            DefKind::Struct | DefKind::Enum | DefKind::Trait | DefKind::Union | DefKind::TyAlias
        ) {
            let sc = conv.char_col_to_utf16_col(&def.file, def.start_line, def.start_col);
            let ec = conv.char_col_to_utf16_col(&def.file, def.end_line, def.end_col);
            let range = json!({
                "start": { "line": def.start_line.saturating_sub(1), "character": sc },
                "end": { "line": def.end_line.saturating_sub(1), "character": ec }
            });
            items.push(json!({
                "name": def.name,
                "kind": lsp_symbol_kind(def.kind),
                "uri": path_to_uri(&def.file),
                "range": range,
                "selectionRange": range,
                "data": { "name": def.name }
            }));
            return json!(items);
        }
    }

    if let Some(ws) = workspace {
        if let Some(def) = ws.find_definition_at(&file, line_1based, col_1based) {
            if matches!(
                def.kind,
                DefKind::Struct | DefKind::Enum | DefKind::Trait | DefKind::Union
            ) {
                let resolved_path = def.resolved_path();
                let path_str = resolved_path.to_str().unwrap_or(def.file);
                let uri = path_to_uri(path_str);
                let sc = conv.char_col_to_utf16_col(path_str, def.start_line, def.start_col);
                let ec = conv.char_col_to_utf16_col(path_str, def.end_line, def.end_col);
                let range = json!({
                    "start": { "line": def.start_line.saturating_sub(1), "character": sc },
                    "end": { "line": def.end_line.saturating_sub(1), "character": ec }
                });
                items.push(json!({
                    "name": def.name,
                    "kind": lsp_symbol_kind(def.kind),
                    "uri": uri,
                    "range": range,
                    "selectionRange": range,
                    "data": { "name": def.name, "path": def.path }
                }));
            }
        }
    }

    json!(items)
}

pub fn handle_type_hierarchy_supertypes(msg: &Value, workspace: Option<&WorkspaceIndex>) -> Value {
    let item = &msg["params"]["item"];
    let name = item["name"].as_str().unwrap_or("");

    let Some(ws) = workspace else {
        return json!([]);
    };

    let mut supertypes = Vec::new();

    // Look for traits implemented for this type (e.g. `impl Trait for Name`)
    let symbols = ws.search_symbols(name);
    for sym in symbols {
        if sym.name == name && matches!(sym.kind, DefKind::Struct | DefKind::Enum) {
            let refs = ws.find_references(sym.path);
            for r in refs {
                if r.kind == DefKind::Impl {
                    let r_path = r.resolved_path();
                    let r_str = r_path.to_str().unwrap_or(r.file);
                    let uri = path_to_uri(r_str);
                    let range = json!({
                        "start": { "line": r.span.start_line().saturating_sub(1), "character": r.span.start_col().saturating_sub(1) },
                        "end": { "line": r.span.end_line().saturating_sub(1), "character": r.span.end_col().saturating_sub(1) }
                    });
                    supertypes.push(json!({
                        "name": format!("impl for {}", name),
                        "kind": 11, // Interface/Trait
                        "uri": uri,
                        "range": range,
                        "selectionRange": range
                    }));
                }
            }
        }
    }

    json!(supertypes)
}

pub fn handle_type_hierarchy_subtypes(msg: &Value, workspace: Option<&WorkspaceIndex>) -> Value {
    let item = &msg["params"]["item"];
    let name = item["name"].as_str().unwrap_or("");

    let Some(ws) = workspace else {
        return json!([]);
    };

    let mut subtypes = Vec::new();

    // Look for types implementing this trait
    let symbols = ws.search_symbols(name);
    for sym in symbols {
        if sym.name == name && sym.kind == DefKind::Trait {
            let refs = ws.find_references(sym.path);
            for r in refs {
                if r.kind == DefKind::Impl {
                    let r_path = r.resolved_path();
                    let r_str = r_path.to_str().unwrap_or(r.file);
                    let uri = path_to_uri(r_str);
                    let range = json!({
                        "start": { "line": r.span.start_line().saturating_sub(1), "character": r.span.start_col().saturating_sub(1) },
                        "end": { "line": r.span.end_line().saturating_sub(1), "character": r.span.end_col().saturating_sub(1) }
                    });
                    subtypes.push(json!({
                        "name": format!("impl of {}", name),
                        "kind": 23, // Struct
                        "uri": uri,
                        "range": range,
                        "selectionRange": range
                    }));
                }
            }
        }
    }

    json!(subtypes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lsp::protocol::PositionConverter;
    use crate::overlay::OverlayManager;
    use ridx_format::{IndexWriter, RawDefinition, RawReference, WorkspaceIndex};
    use serde_json::json;
    use std::fs;
    use std::path::PathBuf;

    fn setup_test_hierarchy() -> (PathBuf, WorkspaceIndex, OverlayManager, PositionConverter) {
        let temp_dir =
            std::env::temp_dir().join(format!("ridx_test_hierarchy_{}", std::process::id()));
        let _ = fs::create_dir_all(temp_dir.join("src"));
        let src_file = temp_dir.join("src/lib.rs");
        let code = "pub fn add(a: i32, b: i32) -> i32 { a + b }\npub struct Foo { pub x: i32 }\npub trait Bar { fn bar(&self); }\nimpl Bar for Foo { fn bar(&self) {} }\n";
        let _ = fs::write(&src_file, code);

        let mut writer = IndexWriter::new("test_crate", "0.1.0");
        writer.add_definition(RawDefinition {
            def_id: 1,
            kind: DefKind::Fn,
            visibility: 0,
            is_macro_expansion: false,
            file: "src/lib.rs".to_string(),
            name: "add".to_string(),
            start_line: 1,
            start_col: 8,
            end_line: 1,
            end_col: 11,
            path: "test_crate::add".to_string(),
            signature: Some("fn(i32, i32) -> i32".to_string()),
        });
        writer.add_definition(RawDefinition {
            def_id: 2,
            kind: DefKind::Struct,
            visibility: 0,
            is_macro_expansion: false,
            file: "src/lib.rs".to_string(),
            name: "Foo".to_string(),
            start_line: 2,
            start_col: 12,
            end_line: 2,
            end_col: 15,
            path: "test_crate::Foo".to_string(),
            signature: None,
        });
        writer.add_definition(RawDefinition {
            def_id: 3,
            kind: DefKind::Trait,
            visibility: 0,
            is_macro_expansion: false,
            file: "src/lib.rs".to_string(),
            name: "Bar".to_string(),
            start_line: 3,
            start_col: 11,
            end_line: 3,
            end_col: 14,
            path: "test_crate::Bar".to_string(),
            signature: None,
        });
        writer.add_reference(RawReference {
            file: "src/lib.rs".to_string(),
            start_line: 4,
            start_col: 6,
            end_line: 4,
            end_col: 9,
            target_def_id: None,
            target_path: "test_crate::Bar".to_string(),
            target_unit: None,
            target_file: Some("src/lib.rs".to_string()),
            target_line: 3,
            target_col: 11,
            kind: DefKind::Impl,
            is_write: false,
            is_macro_expansion: false,
        });
        writer.add_reference(RawReference {
            file: "src/lib.rs".to_string(),
            start_line: 4,
            start_col: 14,
            end_line: 4,
            end_col: 17,
            target_def_id: None,
            target_path: "test_crate::Foo".to_string(),
            target_unit: None,
            target_file: Some("src/lib.rs".to_string()),
            target_line: 2,
            target_col: 12,
            kind: DefKind::Impl,
            is_write: false,
            is_macro_expansion: false,
        });

        writer.add_definition(RawDefinition {
            def_id: 4,
            kind: DefKind::Fn,
            visibility: 0,
            is_macro_expansion: false,
            file: "src/lib.rs".to_string(),
            name: "caller_fn".to_string(),
            start_line: 10,
            start_col: 8,
            end_line: 15,
            end_col: 2,
            path: "test_crate::caller_fn".to_string(),
            signature: Some("fn caller_fn()".to_string()),
        });
        writer.add_reference(RawReference {
            file: "src/lib.rs".to_string(),
            start_line: 11,
            start_col: 5,
            end_line: 11,
            end_col: 8,
            target_def_id: Some(1),
            target_path: "test_crate::add".to_string(),
            target_unit: None,
            target_file: Some("src/lib.rs".to_string()),
            target_line: 1,
            target_col: 8,
            kind: DefKind::Fn,
            is_write: false,
            is_macro_expansion: false,
        });
        // Unenclosed reference (caller_def == None)
        writer.add_reference(RawReference {
            file: "src/lib.rs".to_string(),
            start_line: 99,
            start_col: 5,
            end_line: 99,
            end_col: 8,
            target_def_id: Some(1),
            target_path: "test_crate::add".to_string(),
            target_unit: None,
            target_file: Some("src/lib.rs".to_string()),
            target_line: 1,
            target_col: 8,
            kind: DefKind::Fn,
            is_write: false,
            is_macro_expansion: false,
        });

        let ridx_file = temp_dir.join("index.ridx");
        let mut f = fs::File::create(&ridx_file).unwrap();
        writer.write_to(&mut f).unwrap();

        let ws = WorkspaceIndex::open(&ridx_file).unwrap();
        let overlay_mgr = OverlayManager::new();
        let conv = PositionConverter::new();

        (temp_dir, ws, overlay_mgr, conv)
    }

    #[test]
    fn test_call_and_type_hierarchy_comprehensive() {
        let (temp_dir, ws, mut overlay_mgr, mut conv) = setup_test_hierarchy();
        let file_path = temp_dir.join("src/lib.rs");
        let uri = format!("file://{}", file_path.to_str().unwrap());

        // 1. Prepare call hierarchy with workspace Fn
        let req = json!({
            "params": {
                "textDocument": { "uri": &uri },
                "position": { "line": 0, "character": 9 }
            }
        });
        let res = handle_prepare_call_hierarchy(&req, Some(&ws), &overlay_mgr, &mut conv);
        assert!(!res.as_array().unwrap().is_empty());

        // Miss
        let miss_req = json!({
            "params": {
                "textDocument": { "uri": &uri },
                "position": { "line": 99, "character": 0 }
            }
        });
        let miss_res = handle_prepare_call_hierarchy(&miss_req, Some(&ws), &overlay_mgr, &mut conv);
        assert_eq!(miss_res, json!([]));

        // 2. Prepare call hierarchy with overlay Fn
        overlay_mgr.open_document(
            &uri,
            file_path.to_str().unwrap(),
            1,
            "fn local_fn() {}\n".to_string(),
        );
        let req_ov = json!({
            "params": {
                "textDocument": { "uri": &uri },
                "position": { "line": 0, "character": 4 }
            }
        });
        let res_ov = handle_prepare_call_hierarchy(&req_ov, Some(&ws), &overlay_mgr, &mut conv);
        assert!(!res_ov.as_array().unwrap().is_empty());

        // 3. Incoming calls
        let inc_req = json!({
            "params": {
                "item": { "name": "Bar" }
            }
        });
        let inc_res = handle_call_hierarchy_incoming_calls(&inc_req, Some(&ws), &mut conv);
        assert!(inc_res.is_array());

        let inc_req2 = json!({
            "params": {
                "item": { "name": "add", "data": { "path": "test_crate::add" } }
            }
        });
        let inc_res2 = handle_call_hierarchy_incoming_calls(&inc_req2, Some(&ws), &mut conv);
        assert!(!inc_res2.as_array().unwrap().is_empty());

        let inc_none = handle_call_hierarchy_incoming_calls(&inc_req, None, &mut conv);
        assert_eq!(inc_none, json!([]));

        // 4. Outgoing calls
        let out_res = handle_call_hierarchy_outgoing_calls(&req, Some(&ws));
        assert_eq!(out_res, json!([]));

        // 5. Prepare type hierarchy with workspace Struct
        let type_req = json!({
            "params": {
                "textDocument": { "uri": "file://src/lib.rs" },
                "position": { "line": 1, "character": 12 }
            }
        });
        let type_res = handle_prepare_type_hierarchy(&type_req, Some(&ws), &overlay_mgr, &mut conv);
        assert!(!type_res.as_array().unwrap().is_empty());

        overlay_mgr.open_document(
            &uri,
            file_path.to_str().unwrap(),
            2,
            "struct LocalStruct;\n".to_string(),
        );
        let type_ov_req = json!({
            "params": {
                "textDocument": { "uri": &uri },
                "position": { "line": 0, "character": 8 }
            }
        });
        let type_ov_res =
            handle_prepare_type_hierarchy(&type_ov_req, Some(&ws), &overlay_mgr, &mut conv);
        assert!(!type_ov_res.as_array().unwrap().is_empty());

        // 6. Type hierarchy supertypes and subtypes
        let super_req = json!({
            "params": {
                "item": { "name": "Foo" }
            }
        });
        let super_res = handle_type_hierarchy_supertypes(&super_req, Some(&ws));
        assert!(super_res.is_array());
        let super_none = handle_type_hierarchy_supertypes(&super_req, None);
        assert_eq!(super_none, json!([]));

        let sub_req = json!({
            "params": {
                "item": { "name": "Bar" }
            }
        });
        let sub_res = handle_type_hierarchy_subtypes(&sub_req, Some(&ws));
        assert!(sub_res.is_array());
        let sub_none = handle_type_hierarchy_subtypes(&sub_req, None);
        assert_eq!(sub_none, json!([]));

        // Test non-matching def kinds for call and type hierarchy prepare (lines 57, 83, 221, 249)
        let mut ov_non_fn = OverlayManager::new();
        ov_non_fn.open_document(
            &uri,
            file_path.to_str().unwrap(),
            1,
            "struct MyStruct;\nfn my_func() {}\n".to_string(),
        );
        // Call hierarchy on a struct in overlay (line 57)
        let call_on_struct_ov = json!({
            "params": {
                "textDocument": { "uri": &uri },
                "position": { "line": 0, "character": 8 }
            }
        });
        assert_eq!(
            handle_prepare_call_hierarchy(&call_on_struct_ov, None, &ov_non_fn, &mut conv),
            json!([])
        );

        // Call hierarchy on a struct in workspace (line 83)
        let call_on_struct_ws = json!({
            "params": {
                "textDocument": { "uri": &uri },
                "position": { "line": 0, "character": 12 }
            }
        });
        let empty_mgr = OverlayManager::new();
        assert_eq!(
            handle_prepare_call_hierarchy(&call_on_struct_ws, Some(&ws), &empty_mgr, &mut conv),
            json!([])
        );

        // Type hierarchy on a fn in overlay (line 221)
        let type_on_fn_ov = json!({
            "params": {
                "textDocument": { "uri": &uri },
                "position": { "line": 1, "character": 4 }
            }
        });
        assert_eq!(
            handle_prepare_type_hierarchy(&type_on_fn_ov, None, &ov_non_fn, &mut conv),
            json!([])
        );

        // Type hierarchy on a fn in workspace (line 249)
        let type_on_fn_ws = json!({
            "params": {
                "textDocument": { "uri": &uri },
                "position": { "line": 19, "character": 8 }
            }
        });
        assert_eq!(
            handle_prepare_type_hierarchy(&type_on_fn_ws, Some(&ws), &empty_mgr, &mut conv),
            json!([])
        );

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
