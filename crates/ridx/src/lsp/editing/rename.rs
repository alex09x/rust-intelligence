/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Symbol rename operations, identifier validation, and linked editing ranges.

use super::ident::*;
use crate::lsp::protocol::{path_to_uri, uri_to_path, PositionConverter};
use crate::overlay::OverlayManager;
use ridx_format::WorkspaceIndex;
use serde_json::{json, Value};
use std::collections::HashMap;

pub fn handle_prepare_rename(
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

    if let Some(overlay) = overlay_opt {
        let refs = overlay.find_local_binding_references(line_1based, col_1based);
        if let Some((sl, sc, el, ec)) = refs.iter().find(|(sl, sc, el, ec)| {
            line_1based >= *sl && line_1based <= *el && col_1based >= *sc && col_1based <= *ec
        }) {
            let start_char = overlay.char_col_to_utf16_col(*sl, *sc);
            let end_char = overlay.char_col_to_utf16_col(*el, *ec);
            let ident = overlay.get_ident_at(*sl, *sc).unwrap_or_default();
            return json!({
                "range": {
                    "start": { "line": sl.saturating_sub(1), "character": start_char },
                    "end": { "line": el.saturating_sub(1), "character": end_char }
                },
                "placeholder": ident
            });
        }
    }

    if let Some(ws) = workspace {
        let is_dirty = overlay_opt.map(|o| o.is_dirty()).unwrap_or(false);
        if !is_dirty {
            if let Some(loc_refs) = ws.find_references_at(&file, line_1based, col_1based) {
                let target_name = loc_refs
                    .target_symbol
                    .rsplit("::")
                    .next()
                    .unwrap_or(loc_refs.target_symbol);

                let line_str = if let Some(ov) = overlay_opt {
                    let idx = line_0based as usize;
                    ov.text.lines().nth(idx).map(|s| s.to_string())
                } else {
                    std::fs::read_to_string(&file)
                        .ok()
                        .and_then(|t| t.lines().nth(line_0based as usize).map(|s| s.to_string()))
                };

                let ident_col = line_str
                    .as_deref()
                    .and_then(|s| find_declaration_ident_col(s, target_name, loc_refs.target_kind))
                    .unwrap_or(col_1based);

                let sc = if let Some(ov) = overlay_opt {
                    ov.char_col_to_utf16_col(line_1based, ident_col)
                } else {
                    conv.char_col_to_utf16_col(&file, line_1based, ident_col)
                };
                let ec = sc + target_name.chars().count() as u32;

                return json!({
                    "range": {
                        "start": { "line": line_0based, "character": sc },
                        "end": { "line": line_0based, "character": ec }
                    },
                    "placeholder": target_name
                });
            }
        }
    }

    Value::Null
}

pub fn handle_rename(
    msg: &Value,
    workspace: Option<&WorkspaceIndex>,
    overlay_mgr: &OverlayManager,
    conv: &mut PositionConverter,
) -> Value {
    let new_name = msg["params"]["newName"].as_str().unwrap_or("");
    if !is_valid_rust_ident(new_name) {
        return json!({ "changes": {} });
    }

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

    let mut changes_map: HashMap<String, Vec<Value>> = HashMap::new();

    // 1. Rename overlay local variable / parameter
    if let Some(overlay) = overlay_opt {
        let local_refs = overlay.find_local_binding_references(line_1based, col_1based);
        if !local_refs.is_empty() {
            let mut edits = Vec::new();
            for (sl, sc, el, ec) in local_refs {
                let start_char = overlay.char_col_to_utf16_col(sl, sc);
                let end_char = overlay.char_col_to_utf16_col(el, ec);
                let edit = json!({
                    "range": {
                        "start": { "line": sl.saturating_sub(1), "character": start_char },
                        "end": { "line": el.saturating_sub(1), "character": end_char }
                    },
                    "newText": new_name
                });
                if !edits.iter().any(|e: &Value| e["range"] == edit["range"]) {
                    edits.push(edit);
                }
            }
            changes_map.insert(uri.to_string(), edits);
            return json!({ "changes": changes_map });
        }
    }

    // 2. Renaming a compiled symbol across the workspace
    if let Some(ws) = workspace {
        let is_dirty = overlay_opt.map(|o| o.is_dirty()).unwrap_or(false);
        if !is_dirty {
            if let Some(loc_refs) = ws.find_references_at(&file, line_1based, col_1based) {
                let target_name = loc_refs
                    .target_symbol
                    .rsplit("::")
                    .next()
                    .unwrap_or(loc_refs.target_symbol);

                // Add declaration edit with exact identifier span
                if !loc_refs.target_file.is_empty() && loc_refs.target_file != "external" {
                    let decl_path = loc_refs.target_resolved_path();
                    let decl_str = decl_path.to_str().unwrap_or(loc_refs.target_file);
                    let decl_uri = path_to_uri(decl_str);

                    let decl_ov = overlay_mgr.get(decl_str);
                    if let Some(dov) = decl_ov {
                        if dov.is_dirty() && dov.has_modified_imports() {
                            return json!({ "changes": {} });
                        }
                    }

                    let (mapped_line, mapped_col) = if let Some(dov) = decl_ov {
                        if dov.is_dirty() {
                            match dov
                                .map_compiled_to_live(loc_refs.target_line, loc_refs.target_col)
                            {
                                Some(pos) => pos,
                                None => return json!({ "changes": {} }),
                            }
                        } else {
                            (loc_refs.target_line, loc_refs.target_col)
                        }
                    } else {
                        (loc_refs.target_line, loc_refs.target_col)
                    };

                    let line_content = if let Some(dov) = decl_ov {
                        dov.text
                            .lines()
                            .nth(mapped_line.saturating_sub(1) as usize)
                            .map(|s| s.to_string())
                    } else {
                        std::fs::read_to_string(decl_str).ok().and_then(|t| {
                            t.lines()
                                .nth(mapped_line.saturating_sub(1) as usize)
                                .map(|s| s.to_string())
                        })
                    };

                    let ident_col = line_content
                        .as_deref()
                        .and_then(|s| {
                            find_declaration_ident_col(s, target_name, loc_refs.target_kind)
                        })
                        .unwrap_or(mapped_col);

                    let name_len = target_name.chars().count() as u32;
                    let end_col = ident_col + name_len;

                    if let Some(dov) = decl_ov {
                        if dov.get_ident_at(mapped_line, ident_col).as_deref() != Some(target_name)
                        {
                            return json!({ "changes": {} });
                        }
                    }

                    let sc = if let Some(dov) = decl_ov {
                        dov.char_col_to_utf16_col(mapped_line, ident_col)
                    } else {
                        conv.char_col_to_utf16_col(decl_str, mapped_line, ident_col)
                    };
                    let ec = if let Some(dov) = decl_ov {
                        dov.char_col_to_utf16_col(mapped_line, end_col)
                    } else {
                        conv.char_col_to_utf16_col(decl_str, mapped_line, end_col)
                    };

                    let sl = mapped_line.saturating_sub(1);
                    let edits = changes_map.entry(decl_uri).or_default();
                    let edit = json!({
                        "range": {
                            "start": { "line": sl, "character": sc },
                            "end": { "line": sl, "character": ec }
                        },
                        "newText": new_name
                    });
                    if !edits.iter().any(|e| e["range"] == edit["range"]) {
                        edits.push(edit);
                    }
                }

                // Add references edits with live buffer remapping and safety checks
                for r in loc_refs.references {
                    let r_path = r.resolved_path();
                    let r_str = r_path.to_str().unwrap_or(r.file);
                    let r_uri = path_to_uri(r_str);

                    let ref_ov = overlay_mgr.get(r_str);
                    if let Some(rov) = ref_ov {
                        if rov.is_dirty() && rov.has_modified_imports() {
                            return json!({ "changes": {} });
                        }
                    }

                    let (mapped_line, mapped_col) = if let Some(rov) = ref_ov {
                        if rov.is_dirty() {
                            match rov.map_compiled_to_live(r.span.start_line(), r.span.start_col())
                            {
                                Some(pos) => pos,
                                None => return json!({ "changes": {} }),
                            }
                        } else {
                            (r.span.start_line(), r.span.start_col())
                        }
                    } else {
                        (r.span.start_line(), r.span.start_col())
                    };

                    let name_len = target_name.chars().count() as u32;
                    let end_col = mapped_col + name_len;

                    if let Some(rov) = ref_ov {
                        if rov.get_ident_at(mapped_line, mapped_col).as_deref() != Some(target_name)
                        {
                            return json!({ "changes": {} });
                        }
                    }

                    let sc = if let Some(rov) = ref_ov {
                        rov.char_col_to_utf16_col(mapped_line, mapped_col)
                    } else {
                        conv.char_col_to_utf16_col(r_str, mapped_line, mapped_col)
                    };
                    let ec = if let Some(rov) = ref_ov {
                        rov.char_col_to_utf16_col(mapped_line, end_col)
                    } else {
                        conv.char_col_to_utf16_col(r_str, mapped_line, end_col)
                    };

                    let sl = mapped_line.saturating_sub(1);
                    let edits = changes_map.entry(r_uri).or_default();
                    let edit = json!({
                        "range": {
                            "start": { "line": sl, "character": sc },
                            "end": { "line": sl, "character": ec }
                        },
                        "newText": new_name
                    });
                    if !edits.iter().any(|e| e["range"] == edit["range"]) {
                        edits.push(edit);
                    }
                }
            }
        }
    }

    let mut doc_changes = Vec::new();
    for (doc_uri, edits) in &changes_map {
        doc_changes.push(json!({
            "textDocument": {
                "uri": doc_uri,
                "version": Value::Null
            },
            "edits": edits
        }));
    }

    json!({
        "changes": changes_map,
        "documentChanges": doc_changes
    })
}

pub fn handle_linked_editing_range(
    msg: &Value,
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

    if let Some(overlay) = overlay_opt {
        let refs = overlay.find_local_binding_references(line_1based, col_1based);
        if !refs.is_empty() {
            let mut ranges = Vec::new();
            for (sl, sc, el, ec) in refs {
                let start_char = overlay.char_col_to_utf16_col(sl, sc);
                let end_char = overlay.char_col_to_utf16_col(el, ec);
                ranges.push(json!({
                    "start": { "line": sl.saturating_sub(1), "character": start_char },
                    "end": { "line": el.saturating_sub(1), "character": end_char }
                }));
            }
            return json!({ "ranges": ranges });
        }
    }

    Value::Null
}
