/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! LSP navigation handlers: definition, declaration, type definition, implementation, references, and highlights.

use super::protocol::{path_to_uri, uri_to_path, PositionConverter};
use crate::overlay::OverlayManager;
use ridx_format::{DefKind, WorkspaceIndex};
use serde_json::{json, Value};

pub fn handle_definition(
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

    // 1. Overlay definition
    if let Some(def) = overlay_mgr.find_definition_at(&file, line_1based, col_1based) {
        let uri = path_to_uri(&def.file);
        let start_char = if let Some(ov) = overlay_opt {
            ov.char_col_to_utf16_col(def.start_line, def.start_col)
        } else {
            conv.char_col_to_utf16_col(&def.file, def.start_line, def.start_col)
        };
        let end_char = if let Some(ov) = overlay_opt {
            ov.char_col_to_utf16_col(def.end_line, def.end_col)
        } else {
            conv.char_col_to_utf16_col(&def.file, def.end_line, def.end_col)
        };
        return json!({
            "uri": uri,
            "range": {
                "start": { "line": def.start_line.saturating_sub(1), "character": start_char },
                "end": { "line": def.end_line.saturating_sub(1), "character": end_char }
            }
        });
    }

    // 2. Workspace definition
    if let Some(ws) = workspace {
        let is_dirty = overlay_opt.map(|o| o.is_dirty()).unwrap_or(false);
        let (query_line, query_col) = if is_dirty {
            if let Some(overlay) = overlay_opt {
                if let Some((cl, cc)) = overlay.map_live_to_compiled(line_1based, col_1based) {
                    (cl, cc)
                } else {
                    return Value::Null;
                }
            } else {
                (line_1based, col_1based)
            }
        } else {
            (line_1based, col_1based)
        };

        if let Some(def) = ws.find_definition_at(&file, query_line, query_col) {
            let resolved_path = def.resolved_path();
            let path_str = resolved_path.to_str().unwrap_or(def.file);
            let uri = path_to_uri(path_str);
            let (start_line, start_col, end_line, end_col) =
                if let Some(target_ov) = overlay_mgr.get(path_str) {
                    if target_ov.is_dirty() {
                        let sl = target_ov
                            .map_compiled_to_live(def.start_line, def.start_col)
                            .unwrap_or((def.start_line, def.start_col));
                        let el = target_ov
                            .map_compiled_to_live(def.end_line, def.end_col)
                            .unwrap_or((def.end_line, def.end_col));
                        (sl.0, sl.1, el.0, el.1)
                    } else {
                        (def.start_line, def.start_col, def.end_line, def.end_col)
                    }
                } else {
                    (def.start_line, def.start_col, def.end_line, def.end_col)
                };

            let start_char = conv.char_col_to_utf16_col(path_str, start_line, start_col);
            let end_char = conv.char_col_to_utf16_col(path_str, end_line, end_col);
            return json!({
                "uri": uri,
                "range": {
                    "start": { "line": start_line.saturating_sub(1), "character": start_char },
                    "end": { "line": end_line.saturating_sub(1), "character": end_char }
                }
            });
        }
    }

    Value::Null
}

pub fn handle_declaration(
    msg: &Value,
    workspace: Option<&WorkspaceIndex>,
    overlay_mgr: &OverlayManager,
    conv: &mut PositionConverter,
) -> Value {
    handle_definition(msg, workspace, overlay_mgr, conv)
}

pub fn handle_type_definition(
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

    let Some(ws) = workspace else {
        return Value::Null;
    };

    let is_dirty = overlay_opt.map(|o| o.is_dirty()).unwrap_or(false);
    if is_dirty {
        return Value::Null;
    }

    if let Some(ty) = ws.type_at(&file, line_1based, col_1based) {
        if let Some(def) = ws.search_symbols(ty).into_iter().next() {
            let resolved_path = def.resolved_path();
            let path_str = resolved_path.to_str().unwrap_or(def.file);
            let uri = path_to_uri(path_str);
            let start_char =
                conv.char_col_to_utf16_col(path_str, def.span.start_line(), def.span.start_col());
            let end_char =
                conv.char_col_to_utf16_col(path_str, def.span.end_line(), def.span.end_col());
            return json!({
                "uri": uri,
                "range": {
                    "start": { "line": def.span.start_line().saturating_sub(1), "character": start_char },
                    "end": { "line": def.span.end_line().saturating_sub(1), "character": end_char }
                }
            });
        }
    }

    Value::Null
}

pub fn handle_implementation(
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

    let Some(ws) = workspace else {
        return json!([]);
    };

    let is_dirty = overlay_opt.map(|o| o.is_dirty()).unwrap_or(false);
    if is_dirty {
        return json!([]);
    }

    let mut results = Vec::new();
    if let Some(def) = ws.find_definition_at(&file, line_1based, col_1based) {
        let refs = ws.find_references(def.path);
        for r in refs {
            let is_impl = if r.kind == DefKind::Impl {
                true
            } else {
                let resolved_path = r.resolved_path();
                let path_str = resolved_path.to_str().unwrap_or(r.file);
                // Check whether the line contains `impl ` and not a trait bound `where ` or `<T:`
                if let Ok(content) = std::fs::read_to_string(path_str) {
                    let line_text = content
                        .lines()
                        .nth(r.span.start_line().saturating_sub(1) as usize)
                        .unwrap_or("");
                    let trimmed = line_text.trim();
                    trimmed.starts_with("impl")
                        || (trimmed.contains("impl ")
                            && !trimmed.contains("where ")
                            && !trimmed.contains("<T:"))
                } else {
                    false
                }
            };

            if is_impl {
                let resolved_path = r.resolved_path();
                let path_str = resolved_path.to_str().unwrap_or(r.file);
                let uri = path_to_uri(path_str);
                let span = r.span;
                let start_char =
                    conv.char_col_to_utf16_col(path_str, span.start_line(), span.start_col());
                let end_char =
                    conv.char_col_to_utf16_col(path_str, span.end_line(), span.end_col());
                results.push(json!({
                    "uri": uri,
                    "range": {
                        "start": { "line": span.start_line().saturating_sub(1), "character": start_char },
                        "end": { "line": span.end_line().saturating_sub(1), "character": end_char }
                    }
                }));
            }
        }
    }

    json!(results)
}

pub fn handle_references(
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
    let include_decl = msg["params"]["context"]["includeDeclaration"]
        .as_bool()
        .unwrap_or(true);

    let overlay_opt = overlay_mgr.get(&file);
    let col_1based = if let Some(overlay) = overlay_opt {
        overlay.utf16_col_to_char_col(line_1based, utf16_col_0based)
    } else {
        conv.utf16_col_to_char_col(&file, line_1based, utf16_col_0based)
    };

    let mut results = Vec::new();

    // 1. Overlay local references
    if let Some(overlay) = overlay_opt {
        let local_refs = overlay.find_local_binding_references(line_1based, col_1based);
        if !local_refs.is_empty() {
            for (sl, sc, el, ec) in local_refs {
                let start_char = overlay.char_col_to_utf16_col(sl, sc);
                let end_char = overlay.char_col_to_utf16_col(el, ec);
                results.push(json!({
                    "uri": uri,
                    "range": {
                        "start": { "line": sl.saturating_sub(1), "character": start_char },
                        "end": { "line": el.saturating_sub(1), "character": end_char }
                    }
                }));
            }
            return json!(results);
        }
    }

    // 2. Workspace compiled references
    if let Some(ws) = workspace {
        let is_dirty = overlay_opt.map(|o| o.is_dirty()).unwrap_or(false);
        if !is_dirty {
            if let Some(loc_refs) = ws.find_references_at(&file, line_1based, col_1based) {
                if include_decl
                    && !loc_refs.target_file.is_empty()
                    && loc_refs.target_file != "external"
                {
                    let decl_path = loc_refs.target_resolved_path();
                    let decl_str = decl_path.to_str().unwrap_or(loc_refs.target_file);
                    let decl_uri = path_to_uri(decl_str);
                    let sc = conv.char_col_to_utf16_col(
                        decl_str,
                        loc_refs.target_line,
                        loc_refs.target_col,
                    );
                    let ec = if loc_refs.target_end_line > 0 && loc_refs.target_end_col > 0 {
                        conv.char_col_to_utf16_col(
                            decl_str,
                            loc_refs.target_end_line,
                            loc_refs.target_end_col,
                        )
                    } else {
                        sc + (loc_refs.target_symbol.encode_utf16().count() as u32)
                    };
                    let sl = loc_refs.target_line.saturating_sub(1);
                    let el = if loc_refs.target_end_line > 0 {
                        loc_refs.target_end_line.saturating_sub(1)
                    } else {
                        sl
                    };
                    results.push(json!({
                        "uri": decl_uri,
                        "range": {
                            "start": { "line": sl, "character": sc },
                            "end": { "line": el, "character": ec }
                        }
                    }));
                }

                for r in loc_refs.references {
                    let r_path = r.resolved_path();
                    let r_str = r_path.to_str().unwrap_or(r.file);
                    let r_uri = path_to_uri(r_str);
                    let span = r.span;
                    let sc = conv.char_col_to_utf16_col(r_str, span.start_line(), span.start_col());
                    let ec = conv.char_col_to_utf16_col(r_str, span.end_line(), span.end_col());
                    results.push(json!({
                        "uri": r_uri,
                        "range": {
                            "start": { "line": span.start_line().saturating_sub(1), "character": sc },
                            "end": { "line": span.end_line().saturating_sub(1), "character": ec }
                        }
                    }));
                }
            }
        }
    }

    json!(results)
}

pub fn handle_document_highlight(
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

    let mut highlights = Vec::new();

    // 1. Overlay local references
    if let Some(overlay) = overlay_opt {
        let local_refs = overlay.find_local_binding_references(line_1based, col_1based);
        if !local_refs.is_empty() {
            for (idx, (sl, sc, el, ec)) in local_refs.into_iter().enumerate() {
                let start_char = overlay.char_col_to_utf16_col(sl, sc);
                let end_char = overlay.char_col_to_utf16_col(el, ec);
                let kind = if idx == 0 { 3 } else { 1 }; // Write (3) for decl, Read (1) for refs
                highlights.push(json!({
                    "range": {
                        "start": { "line": sl.saturating_sub(1), "character": start_char },
                        "end": { "line": el.saturating_sub(1), "character": end_char }
                    },
                    "kind": kind
                }));
            }
            return json!(highlights);
        }
    }

    // 2. Workspace compiled references in the current document
    if let Some(ws) = workspace {
        let is_dirty = overlay_opt.map(|o| o.is_dirty()).unwrap_or(false);
        if !is_dirty {
            if let Some(loc_refs) = ws.find_references_at(&file, line_1based, col_1based) {
                let decl_path = loc_refs.target_resolved_path();
                let decl_str = decl_path.to_str().unwrap_or(loc_refs.target_file);
                if decl_str == file || decl_str.ends_with(&file) || file.ends_with(decl_str) {
                    let sc = conv.char_col_to_utf16_col(
                        &file,
                        loc_refs.target_line,
                        loc_refs.target_col,
                    );
                    let ec = if loc_refs.target_end_line > 0 && loc_refs.target_end_col > 0 {
                        conv.char_col_to_utf16_col(
                            &file,
                            loc_refs.target_end_line,
                            loc_refs.target_end_col,
                        )
                    } else {
                        sc + (loc_refs.target_symbol.encode_utf16().count() as u32)
                    };
                    let sl = loc_refs.target_line.saturating_sub(1);
                    let el = if loc_refs.target_end_line > 0 {
                        loc_refs.target_end_line.saturating_sub(1)
                    } else {
                        sl
                    };
                    highlights.push(json!({
                        "range": {
                            "start": { "line": sl, "character": sc },
                            "end": { "line": el, "character": ec }
                        },
                        "kind": 3
                    }));
                }

                for r in loc_refs.references {
                    let r_path = r.resolved_path();
                    let r_str = r_path.to_str().unwrap_or(r.file);
                    if r_str == file || r_str.ends_with(&file) || file.ends_with(r_str) {
                        let span = r.span;
                        let sc =
                            conv.char_col_to_utf16_col(&file, span.start_line(), span.start_col());
                        let ec = conv.char_col_to_utf16_col(&file, span.end_line(), span.end_col());
                        highlights.push(json!({
                            "range": {
                                "start": { "line": span.start_line().saturating_sub(1), "character": sc },
                                "end": { "line": span.end_line().saturating_sub(1), "character": ec }
                            },
                            "kind": 1
                        }));
                    }
                }
            }
        }
    }

    json!(highlights)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lsp::protocol::PositionConverter;
    use crate::overlay::OverlayManager;
    use ridx_format::{IndexWriter, RawDefinition, RawReference, RawType, WorkspaceIndex};
    use serde_json::json;
    use std::fs;
    use std::path::PathBuf;

    fn setup_test_navigation() -> (PathBuf, WorkspaceIndex, OverlayManager, PositionConverter) {
        let temp_dir = std::env::temp_dir().join(format!("ridx_test_nav_{}", std::process::id()));
        let _ = fs::create_dir_all(temp_dir.join("src"));
        let src_file = temp_dir.join("src/lib.rs");
        let code = "pub fn foo() {}\npub struct Bar;\npub trait Baz {}\nimpl Baz for Bar {}\npub fn caller() { foo(); }\nunsafe impl Baz for () {}\nunsafe impl Baz for (u8,) where Self: Sized {}\nunsafe impl<T: Baz> for Option<T> {}\n";
        let _ = fs::write(&src_file, code);

        let mut writer = IndexWriter::new("test_nav", "0.1.0");
        writer.add_definition(RawDefinition {
            def_id: 1,
            kind: DefKind::Fn,
            visibility: 0,
            is_macro_expansion: false,
            file: "src/lib.rs".to_string(),
            name: "foo".to_string(),
            start_line: 1,
            start_col: 8,
            end_line: 1,
            end_col: 11,
            path: "test_nav::foo".to_string(),
            signature: Some("fn foo()".to_string()),
        });
        writer.add_definition(RawDefinition {
            def_id: 2,
            kind: DefKind::Struct,
            visibility: 0,
            is_macro_expansion: false,
            file: "src/lib.rs".to_string(),
            name: "Bar".to_string(),
            start_line: 2,
            start_col: 12,
            end_line: 2,
            end_col: 15,
            path: "test_nav::Bar".to_string(),
            signature: None,
        });
        writer.add_definition(RawDefinition {
            def_id: 3,
            kind: DefKind::Trait,
            visibility: 0,
            is_macro_expansion: false,
            file: "src/lib.rs".to_string(),
            name: "Baz".to_string(),
            start_line: 3,
            start_col: 11,
            end_line: 3,
            end_col: 14,
            path: "test_nav::Baz".to_string(),
            signature: None,
        });
        writer.add_reference(RawReference {
            file: "src/lib.rs".to_string(),
            start_line: 4,
            start_col: 6,
            end_line: 4,
            end_col: 9,
            target_def_id: None,
            target_path: "test_nav::Baz".to_string(),
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
            start_col: 6,
            end_line: 4,
            end_col: 9,
            target_def_id: None,
            target_path: "test_nav::Baz".to_string(),
            target_unit: None,
            target_file: Some("src/lib.rs".to_string()),
            target_line: 3,
            target_col: 11,
            kind: DefKind::Other,
            is_write: false,
            is_macro_expansion: false,
        });
        writer.add_reference(RawReference {
            file: "nonexistent_file_test.rs".to_string(),
            start_line: 1,
            start_col: 1,
            end_line: 1,
            end_col: 5,
            target_def_id: None,
            target_path: "test_nav::Baz".to_string(),
            target_unit: None,
            target_file: Some("src/lib.rs".to_string()),
            target_line: 3,
            target_col: 11,
            kind: DefKind::Other,
            is_write: false,
            is_macro_expansion: false,
        });
        // Line 6: unsafe impl Baz for () {} -> is_impl = true (contains "impl ")
        writer.add_reference(RawReference {
            file: "src/lib.rs".to_string(),
            start_line: 6,
            start_col: 13,
            end_line: 6,
            end_col: 16,
            target_def_id: None,
            target_path: "test_nav::Baz".to_string(),
            target_unit: None,
            target_file: Some("src/lib.rs".to_string()),
            target_line: 3,
            target_col: 11,
            kind: DefKind::Other,
            is_write: false,
            is_macro_expansion: false,
        });
        // Line 7: unsafe impl Baz for (u8,) where Self: Sized {} -> contains "where " -> is_impl = false
        writer.add_reference(RawReference {
            file: "src/lib.rs".to_string(),
            start_line: 7,
            start_col: 13,
            end_line: 7,
            end_col: 16,
            target_def_id: None,
            target_path: "test_nav::Baz".to_string(),
            target_unit: None,
            target_file: Some("src/lib.rs".to_string()),
            target_line: 3,
            target_col: 11,
            kind: DefKind::Other,
            is_write: false,
            is_macro_expansion: false,
        });
        // Line 8: unsafe impl<T: Baz> for Option<T> {} -> contains "<T:" -> is_impl = false
        writer.add_reference(RawReference {
            file: "src/lib.rs".to_string(),
            start_line: 8,
            start_col: 16,
            end_line: 8,
            end_col: 19,
            target_def_id: None,
            target_path: "test_nav::Baz".to_string(),
            target_unit: None,
            target_file: Some("src/lib.rs".to_string()),
            target_line: 3,
            target_col: 11,
            kind: DefKind::Other,
            is_write: false,
            is_macro_expansion: false,
        });
        writer.add_definition(RawDefinition {
            def_id: 4,
            kind: DefKind::Fn,
            visibility: 0,
            is_macro_expansion: false,
            file: "src/lib.rs".to_string(),
            name: "zero_end".to_string(),
            start_line: 1,
            start_col: 1,
            end_line: 0,
            end_col: 0,
            path: "test_nav::zero_end".to_string(),
            signature: None,
        });
        writer.add_reference(RawReference {
            file: "src/lib.rs".to_string(),
            start_line: 5,
            start_col: 1,
            end_line: 5,
            end_col: 8,
            target_def_id: Some(4),
            target_path: "test_nav::zero_end".to_string(),
            target_unit: None,
            target_file: Some("src/lib.rs".to_string()),
            target_line: 1,
            target_col: 1,
            kind: DefKind::Fn,
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
            target_path: "test_nav::Bar".to_string(),
            target_unit: None,
            target_file: Some("src/lib.rs".to_string()),
            target_line: 2,
            target_col: 12,
            kind: DefKind::Fn,
            is_write: false,
            is_macro_expansion: false,
        });
        writer.add_reference(RawReference {
            file: "src/lib.rs".to_string(),
            start_line: 5,
            start_col: 19,
            end_line: 5,
            end_col: 22,
            target_def_id: None,
            target_path: "test_nav::foo".to_string(),
            target_unit: None,
            target_file: Some("src/lib.rs".to_string()),
            target_line: 1,
            target_col: 8,
            kind: DefKind::Fn,
            is_write: false,
            is_macro_expansion: false,
        });
        writer.add_reference(RawReference {
            file: "src/caller.rs".to_string(),
            start_line: 1,
            start_col: 1,
            end_line: 1,
            end_col: 4,
            target_def_id: None,
            target_path: "test_nav::foo".to_string(),
            target_unit: None,
            target_file: Some("src/lib.rs".to_string()),
            target_line: 1,
            target_col: 8,
            kind: DefKind::Fn,
            is_write: false,
            is_macro_expansion: false,
        });
        writer.add_type(RawType {
            file: "src/lib.rs".to_string(),
            start_line: 2,
            start_col: 12,
            end_line: 2,
            end_col: 15,
            type_name: "test_nav::Bar".to_string(),
        });

        let ridx_file = temp_dir.join("index.ridx");
        let mut f = fs::File::create(&ridx_file).unwrap();
        writer.write_to(&mut f).unwrap();

        let mut ws = WorkspaceIndex::open(&ridx_file).unwrap();
        ws.set_workspace_root(Some(temp_dir.clone()));
        let overlay_mgr = OverlayManager::new();
        let conv = PositionConverter::new();

        (temp_dir, ws, overlay_mgr, conv)
    }

    #[test]
    fn test_navigation_handlers_comprehensive() {
        let (temp_dir, ws, mut overlay_mgr, mut conv) = setup_test_navigation();
        let file_path = temp_dir.join("src/lib.rs");
        let uri = format!("file://{}", file_path.to_str().unwrap());

        // 1. Definition (workspace hit)
        let def_req = json!({
            "params": {
                "textDocument": { "uri": &uri },
                "position": { "line": 4, "character": 20 }
            }
        });
        let def_res = handle_definition(&def_req, Some(&ws), &overlay_mgr, &mut conv);
        assert!(!def_res.is_null());

        // Declaration
        let decl_res = handle_declaration(&def_req, Some(&ws), &overlay_mgr, &mut conv);
        assert!(!decl_res.is_null());

        // Definition (overlay hit)
        overlay_mgr.open_document(
            &uri,
            file_path.to_str().unwrap(),
            1,
            "fn local() {}\nfn run() { local(); }\n".to_string(),
        );
        let ov_def_req = json!({
            "params": {
                "textDocument": { "uri": &uri },
                "position": { "line": 1, "character": 12 }
            }
        });
        let ov_def_res = handle_definition(&ov_def_req, Some(&ws), &overlay_mgr, &mut conv);
        assert!(!ov_def_res.is_null());

        // Definition when target overlay is dirty
        let base_code = "pub fn foo() {}\npub struct Bar;\npub trait Baz {}\nimpl Baz for Bar {}\npub fn caller() { foo(); }\n";
        let mut dirty_mgr = OverlayManager::new();
        dirty_mgr.open_document(&uri, file_path.to_str().unwrap(), 1, base_code.to_string());
        dirty_mgr.update(
            file_path.to_str().unwrap(),
            2,
            format!("{}// dirty addition\n", base_code),
        );
        let def_dirty = handle_definition(&def_req, Some(&ws), &dirty_mgr, &mut conv);
        assert!(!def_dirty.is_null());

        let decl_dirty = handle_declaration(&def_req, Some(&ws), &dirty_mgr, &mut conv);
        assert!(!decl_dirty.is_null());
        let empty_mgr = OverlayManager::new();
        assert_eq!(
            handle_declaration(&def_req, None, &empty_mgr, &mut conv),
            Value::Null
        );

        let def_dirty_unmapped = json!({
            "params": {
                "textDocument": { "uri": &uri },
                "position": { "line": 9999, "character": 0 }
            }
        });
        assert_eq!(
            handle_definition(&def_dirty_unmapped, Some(&ws), &dirty_mgr, &mut conv),
            Value::Null
        );

        // 2. Type definition
        let type_def_req = json!({
            "params": {
                "textDocument": { "uri": &uri },
                "position": { "line": 1, "character": 13 }
            }
        });
        // With dirty overlay, returns Null
        overlay_mgr.update(
            file_path.to_str().unwrap(),
            2,
            "// dirty buffer\n".to_string(),
        );
        assert_eq!(
            handle_type_definition(&type_def_req, Some(&ws), &overlay_mgr, &mut conv),
            Value::Null
        );
        assert_eq!(
            handle_implementation(&type_def_req, Some(&ws), &overlay_mgr, &mut conv),
            json!([])
        );
        let refs_dirty_req = json!({
            "params": {
                "textDocument": { "uri": &uri },
                "position": { "line": 0, "character": 9 }
            }
        });
        assert_eq!(
            handle_references(&refs_dirty_req, Some(&ws), &overlay_mgr, &mut conv),
            json!([])
        );
        assert_eq!(
            handle_document_highlight(&refs_dirty_req, Some(&ws), &overlay_mgr, &mut conv),
            json!([])
        );
        // Without overlay
        let clean_mgr = OverlayManager::new();
        let td_res = handle_type_definition(&type_def_req, Some(&ws), &clean_mgr, &mut conv);
        assert!(!td_res.is_null());
        assert_eq!(
            handle_type_definition(&type_def_req, None, &clean_mgr, &mut conv),
            Value::Null
        );

        // 3. Implementation
        let impl_req = json!({
            "params": {
                "textDocument": { "uri": &uri },
                "position": { "line": 2, "character": 12 }
            }
        });
        let impl_res = handle_implementation(&impl_req, Some(&ws), &clean_mgr, &mut conv);
        assert!(impl_res.is_array());
        assert_eq!(
            handle_implementation(&impl_req, None, &clean_mgr, &mut conv),
            json!([])
        );

        // 4. References with includeDeclaration: true (lines 316, 322)
        let refs_req = json!({
            "params": {
                "textDocument": { "uri": &uri },
                "position": { "line": 0, "character": 9 },
                "context": { "includeDeclaration": true }
            }
        });
        let refs_res = handle_references(&refs_req, Some(&ws), &clean_mgr, &mut conv);
        assert!(refs_res.is_array());
        assert_eq!(
            handle_references(&refs_req, None, &clean_mgr, &mut conv),
            json!([])
        );

        // 5. Document highlight (lines 416, 422)
        let hl_res = handle_document_highlight(&refs_req, Some(&ws), &clean_mgr, &mut conv);
        assert!(hl_res.is_array());
        assert_eq!(
            handle_document_highlight(&refs_req, None, &clean_mgr, &mut conv),
            json!([])
        );

        // 6. Definition with non-dirty overlay for target (line 93)
        let mut non_dirty_mgr = OverlayManager::new();
        non_dirty_mgr.open_document(&uri, file_path.to_str().unwrap(), 1, base_code.to_string());
        non_dirty_mgr.open_document("src/lib.rs", "src/lib.rs", 1, base_code.to_string());
        let def_clean_ov = handle_definition(&def_req, Some(&ws), &non_dirty_mgr, &mut conv);
        assert!(!def_clean_ov.is_null());

        // 7. Definition with dirty overlay for target (lines 82-89)
        let mut dirty_target_mgr = OverlayManager::new();
        dirty_target_mgr.open_document(&uri, file_path.to_str().unwrap(), 1, base_code.to_string());
        dirty_target_mgr.update(
            file_path.to_str().unwrap(),
            2,
            format!("// extra line\n{}", base_code),
        );
        dirty_target_mgr.open_document("src/lib.rs", "src/lib.rs", 1, base_code.to_string());
        dirty_target_mgr.update("src/lib.rs", 2, format!("// extra line\n{}", base_code));

        let caller_path = temp_dir.join("src/caller.rs");
        let caller_uri = format!("file://{}", caller_path.to_str().unwrap());
        let def_caller_req = json!({
            "params": {
                "textDocument": { "uri": &caller_uri },
                "position": { "line": 0, "character": 2 }
            }
        });
        let def_dirty = handle_definition(&def_caller_req, Some(&ws), &dirty_target_mgr, &mut conv);
        assert!(!def_dirty.is_null());
        assert_eq!(def_dirty["range"]["start"]["line"], 1);

        // 8. References and highlight with target_end_line: 0 (lines 314, 320, 329, 414, 420, 429)
        let refs_zero_end_req = json!({
            "params": {
                "textDocument": { "uri": &uri },
                "position": { "line": 4, "character": 3 },
                "context": { "includeDeclaration": true }
            }
        });
        let refs_ze = handle_references(&refs_zero_end_req, Some(&ws), &clean_mgr, &mut conv);
        assert_eq!(refs_ze.as_array().unwrap().len(), 2);

        let hl_ze = handle_document_highlight(&refs_zero_end_req, Some(&ws), &clean_mgr, &mut conv);
        assert_eq!(hl_ze.as_array().unwrap().len(), 2);

        // 9. Local binding references and highlights in overlay (lines 275-286, 378-390)
        let mut local_ov_mgr = OverlayManager::new();
        let local_code = "fn test() {\n    let my_var = 1;\n    let _ = my_var;\n}\n";
        local_ov_mgr.open_document(
            &uri,
            file_path.to_str().unwrap(),
            10,
            local_code.to_string(),
        );
        let local_ref_req = json!({
            "params": {
                "textDocument": { "uri": &uri },
                "position": { "line": 1, "character": 9 }
            }
        });
        let local_refs = handle_references(&local_ref_req, Some(&ws), &local_ov_mgr, &mut conv);
        assert_eq!(local_refs.as_array().unwrap().len(), 2);

        let local_hl =
            handle_document_highlight(&local_ref_req, Some(&ws), &local_ov_mgr, &mut conv);
        assert_eq!(local_hl.as_array().unwrap().len(), 2);
        assert_eq!(local_hl[0]["kind"], 3);
        assert_eq!(local_hl[1]["kind"], 1);

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
