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
use crate::lsp::error::{LspError, LspResult};
use crate::lsp::protocol::{path_to_uri, uri_to_path, PositionConverter};
use crate::overlay::OverlayManager;
use ridx_format::WorkspaceIndex;
use serde_json::{json, Value};
use std::collections::HashMap;

pub use super::prepare_rename::handle_prepare_rename;

fn to_utf16_col(
    ov: Option<&crate::overlay::DocumentOverlay>,
    conv: &mut PositionConverter,
    file: &str,
    line: u32,
    col: u32,
) -> u32 {
    ov.map(|o| o.char_col_to_utf16_col(line, col))
        .unwrap_or_else(|| conv.char_col_to_utf16_col(file, line, col))
}

pub fn handle_rename(
    msg: &Value,
    workspace: Option<&WorkspaceIndex>,
    overlay_mgr: &OverlayManager,
    conv: &mut PositionConverter,
) -> LspResult<Value> {
    let new_name = msg["params"]["newName"].as_str().unwrap_or("");
    if !is_valid_rust_ident(new_name) {
        return Err(LspError::request_failed(format!(
            "Invalid name '{new_name}': not a valid Rust identifier"
        )));
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
            changes_map.insert(uri.to_string(), edits.clone());
            let version = json!(overlay.version);
            let doc_changes = vec![json!({
                "textDocument": { "uri": uri, "version": version },
                "edits": edits
            })];
            return Ok(json!({
                "changes": changes_map,
                "documentChanges": doc_changes
            }));
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
                if target_name.is_empty() {
                    return Err(LspError::request_failed(
                        "No renameable symbol found at this position",
                    ));
                }
                if loc_refs.is_external(ws) {
                    return Err(LspError::request_failed(format!(
                        "Cannot rename external symbol '{target_name}'"
                    )));
                }
                if ws.has_unmerged_reexport_references(loc_refs.target_path, &loc_refs.references) {
                    return Err(LspError::request_failed(
                        "symbol is re-exported; cross-crate references may be incomplete",
                    ));
                }

                let decl_path = loc_refs.target_resolved_path();
                let decl_str = decl_path.to_str().unwrap_or(loc_refs.target_file);
                let mut decl_final_line = None;

                // Add declaration edit with exact identifier span
                if !loc_refs.target_file.is_empty() && loc_refs.target_file != "external" {
                    let decl_uri = path_to_uri(decl_str);
                    let decl_ov = overlay_mgr.get(decl_str);
                    if let Some(dov) = decl_ov {
                        if dov.is_dirty() && dov.has_modified_imports() {
                            return Err(LspError::request_failed(
                                "Cannot rename symbol in file with modified imports; save document first",
                            ));
                        }
                    }

                    let mapped_line = if let Some(dov) = decl_ov {
                        if dov.is_dirty() {
                            match dov.map_compiled_to_live(loc_refs.target_line, loc_refs.target_col) {
                                Some(pos) => pos.0,
                                None => return Err(LspError::request_failed(
                                    "Cannot map symbol position in modified file",
                                )),
                            }
                        } else {
                            loc_refs.target_line
                        }
                    } else {
                        loc_refs.target_line
                    };

                    let decl_lines_vec: Vec<String> = if let Some(dov) = decl_ov {
                        dov.text.lines().map(|s| s.to_string()).collect()
                    } else {
                        std::fs::read_to_string(decl_str)
                            .map(|t| t.lines().map(|s| s.to_string()).collect())
                            .unwrap_or_default()
                    };
                    let decl_lines_ref: Vec<&str> =
                        decl_lines_vec.iter().map(|s| s.as_str()).collect();

                    let (final_line, ident_col) = match find_declaration_ident_in_lines(
                        &decl_lines_ref,
                        mapped_line,
                        target_name,
                        loc_refs.target_kind,
                    ) {
                        Some(pos) => pos,
                        None => return Err(LspError::request_failed(
                            "Cannot find declaration identifier at mapped position",
                        )),
                    };
                    decl_final_line = Some(final_line);

                    let end_col = ident_col + target_name.chars().count() as u32;
                    if let Some(dov) = decl_ov {
                        if dov.get_ident_at(final_line, ident_col).as_deref() != Some(target_name) {
                            return Err(LspError::request_failed(format!(
                                "Identifier at mapped location does not match '{target_name}'",
                            )));
                        }
                    }

                    let sc = to_utf16_col(decl_ov, conv, decl_str, final_line, ident_col);
                    let ec = to_utf16_col(decl_ov, conv, decl_str, final_line, end_col);
                    let sl = final_line.saturating_sub(1);
                    changes_map.entry(decl_uri).or_default().push(json!({
                        "range": {
                            "start": { "line": sl, "character": sc },
                            "end": { "line": sl, "character": ec }
                        },
                        "newText": new_name
                    }));
                }

                // Add references edits with live buffer remapping and safety checks
                let mut skipped_macro_refs = 0usize;
                for r in loc_refs.references {
                    let r_path = r.resolved_path();
                    if ws.is_external_path(&r_path) {
                        continue;
                    }
                    let r_str = r_path.to_str().unwrap_or(r.file);
                    let r_uri = path_to_uri(r_str);

                    let ref_ov = overlay_mgr.get(r_str);
                    let (mapped_line, mapped_col) = if let Some(rov) = ref_ov {
                        if rov.is_dirty() {
                            if rov.has_modified_imports() {
                                return Err(LspError::request_failed(
                                    "Cannot rename symbol in caller file with modified imports; save document first",
                                ));
                            }
                            let pos = match rov.map_compiled_to_live(r.span.start_line(), r.span.start_col()) {
                                Some(p) => p,
                                None => return Err(LspError::request_failed(
                                    "Cannot map symbol position in modified file",
                                )),
                            };
                            if let Some(local_def) = rov.find_definition_at(pos.0, pos.1) {
                                if r_str != decl_str
                                    || decl_final_line.map(|fl| local_def.start_line != fl).unwrap_or(true)
                                {
                                    return Err(LspError::request_failed(
                                        "Symbol shadowed by local definition",
                                    ));
                                }
                            }
                            pos
                        } else {
                            (r.span.start_line(), r.span.start_col())
                        }
                    } else {
                        (r.span.start_line(), r.span.start_col())
                    };

                    let occ_name = if let Some(rov) = ref_ov {
                        rov.get_ident_at(mapped_line, mapped_col)
                    } else {
                        std::fs::read_to_string(r_str).ok().and_then(|t| {
                            t.lines()
                                .nth(mapped_line.saturating_sub(1) as usize)
                                .and_then(|line| {
                                    extract_ident_at_col(line, mapped_col).map(|s| s.to_string())
                                })
                        })
                    };

                    let occ_str = match occ_name {
                        Some(s) if !s.is_empty() => s,
                        _ => {
                            if r.is_macro_expansion
                                || span_covers_other_identifier(
                                    ref_ov.is_none().then_some(r_str),
                                    (
                                        r.span.start_line(),
                                        r.span.start_col(),
                                        r.span.end_line(),
                                        r.span.end_col(),
                                    ),
                                )
                            {
                                skipped_macro_refs += 1;
                                continue;
                            }
                            return Err(LspError::request_failed(
                                "Failed to locate identifier at reference position",
                            ));
                        }
                    };

                    // Index records item definition spans starting at item keywords (e.g. `fn`, `mod`,
                    // `pub`, `struct`) rather than identifier tokens. The declaration edit is already
                    // applied above at the exact identifier column. Skipping references whose token does
                    // not match target_name is safe and necessary: it avoids corrupting keyword tokens
                    // and handles macro expansions where the reference span points elsewhere.
                    if occ_str != target_name {
                        if r.is_macro_expansion {
                            skipped_macro_refs += 1;
                        }
                        continue;
                    }

                    let end_col = mapped_col + occ_str.chars().count() as u32;
                    let sc = to_utf16_col(ref_ov, conv, r_str, mapped_line, mapped_col);
                    let ec = to_utf16_col(ref_ov, conv, r_str, mapped_line, end_col);
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

                if skipped_macro_refs > 0 {
                    eprintln!(
                        "[ridx:rename] Skipped {skipped_macro_refs} macro expansion reference(s) where identifier could not be located in source"
                    );
                }
            }
        }
    }

    if changes_map.is_empty() {
        return Err(LspError::request_failed(
            "No renameable symbol found at this position",
        ));
    }

    validate_and_dedup_edits(&mut changes_map)?;

    let mut doc_changes = Vec::new();
    for (doc_uri, edits) in &changes_map {
        let doc_path = uri_to_path(doc_uri);
        let version = overlay_mgr
            .get(&doc_path)
            .map(|ov| json!(ov.version))
            .unwrap_or(Value::Null);

        doc_changes.push(json!({
            "textDocument": { "uri": doc_uri, "version": version },
            "edits": edits
        }));
    }

    Ok(json!({
        "changes": changes_map,
        "documentChanges": doc_changes
    }))
}

pub(crate) fn validate_and_dedup_edits(
    changes_map: &mut HashMap<String, Vec<Value>>,
) -> Result<(), LspError> {
    for (doc_uri, edits) in changes_map {
        edits.sort_by(|a, b| {
            let a_sl = a["range"]["start"]["line"].as_u64().unwrap_or(0);
            let b_sl = b["range"]["start"]["line"].as_u64().unwrap_or(0);
            let a_sc = a["range"]["start"]["character"].as_u64().unwrap_or(0);
            let b_sc = b["range"]["start"]["character"].as_u64().unwrap_or(0);
            (a_sl, a_sc).cmp(&(b_sl, b_sc))
        });
        let mut deduped: Vec<Value> = Vec::with_capacity(edits.len());
        for edit in edits.drain(..) {
            if let Some(prev) = deduped.last() {
                let prev_el = prev["range"]["end"]["line"].as_u64().unwrap_or(0);
                let prev_ec = prev["range"]["end"]["character"].as_u64().unwrap_or(0);
                let cur_sl = edit["range"]["start"]["line"].as_u64().unwrap_or(0);
                let cur_sc = edit["range"]["start"]["character"].as_u64().unwrap_or(0);
                if (cur_sl, cur_sc) < (prev_el, prev_ec) {
                    if prev["range"] == edit["range"] {
                        // Exact duplicate range, safe to skip
                        continue;
                    }
                    let doc_file = uri_to_path(doc_uri);
                    return Err(LspError::internal_error(format!(
                        "Overlapping rename edits in file: {doc_file}"
                    )));
                }
            }
            deduped.push(edit);
        }
        *edits = deduped;
    }
    Ok(())
}

