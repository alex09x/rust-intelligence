/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! LSP navigation handlers: definition, declaration, type definition, and implementation.

pub mod declaration;
mod highlight_ext;
mod impl_line;
pub mod references;
pub mod type_definition;
mod type_definition_lookup;
pub(crate) mod type_definition_parse;
mod type_definition_qual;
mod type_definition_walk;

#[cfg(test)]
pub mod tests;

pub use declaration::handle_declaration;
pub use references::*;
pub use type_definition::handle_type_definition;

use crate::lsp::protocol::{path_to_uri, uri_to_path, PositionConverter};
use crate::overlay::OverlayManager;
use impl_line::{is_head_position, is_impl_header};
use ridx_format::{DefKind, WorkspaceIndex};
use serde_json::{json, Value};

pub fn handle_definition(msg: &Value, workspace: Option<&WorkspaceIndex>, overlay_mgr: &OverlayManager, conv: &mut PositionConverter) -> Value {
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
        let ov = overlay_opt.unwrap();
        let start_char = ov.char_col_to_utf16_col(def.start_line, def.start_col);
        let end_char = ov.char_col_to_utf16_col(def.end_line, def.end_col);
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
        let (query_line, query_col) = if let Some(overlay) = overlay_opt.filter(|o| o.is_dirty()) {
            if overlay.has_modified_imports() {
                (0, 0)
            } else if let Some((cl, cc)) = overlay.map_live_to_compiled(line_1based, col_1based) {
                (cl, cc)
            } else {
                (0, 0)
            }
        } else {
            (line_1based, col_1based)
        };

        if query_line > 0 {
            if let Some(def) = ws.find_definition_at(&file, query_line, query_col) {
                let resolved_path = def.resolved_path();
                if matches!(def.kind, DefKind::Mod | DefKind::Use | DefKind::ExternCrate) && !def.name.is_empty() {
                    if let Some(mod_file) = resolve_backing_module_file(ws, &def, &resolved_path) {
                        if let Some(mod_str) = mod_file.to_str() {
                            return json!({
                                "uri": path_to_uri(mod_str),
                                "range": {
                                    "start": { "line": 0, "character": 0 },
                                    "end": { "line": 0, "character": 0 }
                                }
                            });
                        }
                    }
                }
                let path_str = resolved_path.to_str().unwrap_or(def.file);
                let uri = path_to_uri(path_str);
                let (start_line, end_line, start_char, end_char) = if let Some(target_ov) = overlay_mgr.get(path_str).filter(|o| o.is_dirty()) {
                    let sl = target_ov.map_compiled_to_live(def.start_line, def.start_col);
                    let el = target_ov.map_compiled_to_live(def.end_line, def.end_col);
                    match (sl, el) {
                        (Some(s), Some(e)) => {
                            if !is_def_valid_in_overlay(target_ov, def.name, s.0, e.0) {
                                (0, 0, 0, 0)
                            } else {
                                let sc = target_ov.char_col_to_utf16_col(s.0, s.1);
                                let ec = target_ov.char_col_to_utf16_col(e.0, e.1);
                                (s.0, e.0, sc, ec)
                            }
                        }
                        _ => (0, 0, 0, 0),
                    }
                } else {
                    let sc = conv.char_col_to_utf16_col(path_str, def.start_line, def.start_col);
                    let ec = conv.char_col_to_utf16_col(path_str, def.end_line, def.end_col);
                    (def.start_line, def.end_line, sc, ec)
                };

                if start_line > 0 {
                    return json!({
                        "uri": uri,
                        "range": {
                            "start": { "line": start_line.saturating_sub(1), "character": start_char },
                            "end": { "line": end_line.saturating_sub(1), "character": end_char }
                        }
                    });
                }
            }
        }
    }

    // 3. Fallback: cross-file or live overlay definition lookup
    let ident = overlay_opt.and_then(|ov| ov.get_ident_at(line_1based, col_1based));
    let path_prefix = overlay_opt.and_then(|ov| ov.get_path_prefix_at(line_1based, col_1based));
    if let Some(ident_str) = ident {
        if let Some(def) = overlay_mgr.find_global_symbol_definition(&file, &ident_str, path_prefix.as_deref()) {
            let allow_hit = def.file == file || workspace.is_none() || overlay_opt.is_some_and(|o| o.is_dirty()) || overlay_mgr.get(&def.file).is_some_and(|o| o.is_dirty());
            if allow_hit {
                let path_str = &def.file;
                let uri = path_to_uri(path_str);
                let (start_char, end_char) = if let Some(target_ov) = overlay_mgr.get(path_str) {
                    (target_ov.char_col_to_utf16_col(def.start_line, def.start_col), target_ov.char_col_to_utf16_col(def.end_line, def.end_col))
                } else {
                    (conv.char_col_to_utf16_col(path_str, def.start_line, def.start_col), conv.char_col_to_utf16_col(path_str, def.end_line, def.end_col))
                };
                return json!({
                    "uri": uri,
                    "range": {
                        "start": { "line": def.start_line.saturating_sub(1), "character": start_char },
                        "end": { "line": def.end_line.saturating_sub(1), "character": end_char }
                    }
                });
            }
        }
    }

    Value::Null
}

fn resolve_backing_module_file(ws: &WorkspaceIndex, def: &ridx_format::ResolvedDefinition<'_>, decl_file: &std::path::Path) -> Option<std::path::PathBuf> {
    let mod_name = def.name;
    if def.kind == DefKind::Mod {
        let parent = decl_file.parent()?;
        let stem = decl_file.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        let mut dirs = Vec::new();
        if !stem.is_empty() && !matches!(stem, "lib" | "main" | "mod") {
            dirs.push(parent.join(stem));
        }
        dirs.push(parent.to_path_buf());
        for dir in &dirs {
            for cand in [dir.join(format!("{mod_name}.rs")), dir.join(mod_name).join("mod.rs")] {
                if cand != decl_file && cand.is_file() {
                    return Some(cand);
                }
            }
        }
        if let Some(reader) = def.reader_idx.and_then(|idx| ws.readers().get(idx)) {
            let raw_path = std::path::Path::new(def.file);
            if let Some(raw_parent) = raw_path.parent() {
                let raw_stem = raw_path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                let mut raw_dirs = Vec::new();
                if !raw_stem.is_empty() && !matches!(raw_stem, "lib" | "main" | "mod") {
                    raw_dirs.push(raw_parent.join(raw_stem));
                }
                raw_dirs.push(raw_parent.to_path_buf());
                for dir in &raw_dirs {
                    for cand in [dir.join(format!("{mod_name}.rs")), dir.join(mod_name).join("mod.rs")] {
                        if let Some(cs) = cand.to_str() {
                            if cs != def.file && reader.find_file_id(cs).is_some() {
                                return Some(cand);
                            }
                        }
                    }
                    let sub_prefix = format!("{}/", dir.join(mod_name).display());
                    if (0..reader.files_count()).any(|fid| reader.get_file(fid).starts_with(&sub_prefix)) {
                        return Some(dir.join(mod_name).join("mod.rs"));
                    }
                }
                if def.file.starts_with("library/") && def.start_line == def.end_line {
                    return Some(raw_parent.join(format!("{mod_name}.rs")));
                }
            }
        }
    }
    if decl_file.is_file() && def.start_line > 0 {
        if let Ok(src) = std::fs::read_to_string(decl_file) {
            if let Some(ln) = src.lines().nth(def.start_line.saturating_sub(1) as usize) {
                if let Some((_, after)) = ln.split_once("extern crate ") {
                    if let Some((real_crate, _)) = after.split_once(" as ") {
                        let rc = real_crate.trim().trim_end_matches(';').trim();
                        if let Some(ed) = ws.find_definition_by_path(rc) {
                            if ed.kind == DefKind::Mod {
                                return Some(ed.resolved_path());
                            }
                        }
                        for r in ws.readers() {
                            if r.crate_name() == rc && r.files_count() > 0 {
                                let f0 = r.get_file(0);
                                if !f0.is_empty() {
                                    return Some(ws.resolve_path_for_crate(rc, f0));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

fn is_ident_on_line(text: &str, line_starts: &[usize], line_1based: u32, name: &str) -> bool {
    let line_idx = line_1based.saturating_sub(1) as usize;
    if line_idx >= line_starts.len() {
        return false;
    }
    let start_byte = line_starts[line_idx];
    if start_byte > text.len() {
        return false;
    }
    let end_byte = if line_idx + 1 < line_starts.len() { line_starts[line_idx + 1].min(text.len()) } else { text.len() };
    if start_byte > end_byte {
        return false;
    }
    let line_str = &text[start_byte..end_byte];
    for (idx, _) in line_str.match_indices(name) {
        let before_ok = if idx == 0 {
            true
        } else {
            let c = line_str[..idx].chars().next_back().unwrap();
            !c.is_alphanumeric() && c != '_'
        };
        let after_idx = idx + name.len();
        let after_ok = if after_idx >= line_str.len() {
            true
        } else {
            let c = line_str[after_idx..].chars().next().unwrap();
            !c.is_alphanumeric() && c != '_'
        };
        if before_ok && after_ok {
            return true;
        }
    }
    false
}

pub(super) fn is_def_valid_in_overlay(target_ov: &crate::overlay::DocumentOverlay, name: &str, start_line: u32, end_line: u32) -> bool {
    if is_ident_on_line(&target_ov.text, &target_ov.line_starts, start_line, name) {
        return true;
    }
    let max_check = std::cmp::min(start_line + 3, end_line.max(start_line));
    for line in (start_line + 1)..=max_check {
        if is_ident_on_line(&target_ov.text, &target_ov.line_starts, line, name) {
            return true;
        }
    }
    false
}

pub fn handle_implementation(msg: &Value, workspace: Option<&WorkspaceIndex>, overlay_mgr: &OverlayManager, conv: &mut PositionConverter) -> Value {
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

    let mut keyed = Vec::new();
    if let Some(def) = ws.find_definition_at(&file, line_1based, col_1based) {
        let mut impl_refs = ws.find_implementations(&def, std::path::Path::new(&file));
        if impl_refs.is_empty() && !ws.has_compiler_impl_defs() {
            for r in ws.find_references(def.path) {
                let is_impl = if r.kind == DefKind::Impl {
                    true
                } else {
                    let resolved_path = r.resolved_path();
                    let path_str = resolved_path.to_str().unwrap_or(r.file);
                    if let Ok(content) = std::fs::read_to_string(path_str) {
                        let line_text = content.lines().nth(r.span.start_line().saturating_sub(1) as usize).unwrap_or("");
                        is_impl_header(line_text) && is_head_position(line_text, r.span.start_col().saturating_sub(1) as usize)
                    } else {
                        false
                    }
                };
                if is_impl {
                    impl_refs.push(r);
                }
            }
        }
        for r in impl_refs {
            let resolved_path = r.resolved_path();
            let path_str = resolved_path.to_str().unwrap_or(r.file);
            let uri = path_to_uri(path_str);
            let span = r.span;
            let start_line = span.start_line().saturating_sub(1);
            let end_line = span.end_line().saturating_sub(1);
            let start_char = conv.char_col_to_utf16_col(path_str, span.start_line(), span.start_col());
            let end_char = conv.char_col_to_utf16_col(path_str, span.end_line(), span.end_col());
            keyed.push((
                (uri.clone(), start_line, start_char, end_line, end_char),
                json!({
                    "uri": uri,
                    "range": {
                        "start": { "line": start_line, "character": start_char },
                        "end": { "line": end_line, "character": end_char }
                    }
                }),
            ));
        }
    }
    keyed.sort_by(|a, b| a.0.cmp(&b.0));
    keyed.dedup_by(|a, b| a.0 == b.0);
    let results: Vec<Value> = keyed.into_iter().map(|(_, v)| v).collect();

    json!(results)
}
