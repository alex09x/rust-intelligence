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

mod cursor_literal;
pub mod declaration;
mod highlight_ext;
mod impl_line;
mod module_file;
pub(crate) mod primitive;
pub mod references;
mod references_gated;
mod references_inactive;
pub(crate) mod syntactic;
pub mod type_definition;
mod type_definition_index;
mod type_definition_live;
mod type_definition_lookup;
pub(crate) mod type_definition_parse;
mod type_definition_qual;
mod type_definition_self;
mod type_definition_walk;

#[cfg(test)]
pub mod tests;

pub use declaration::handle_declaration;
pub use references::*;
pub use type_definition::handle_type_definition;
pub(crate) use type_definition_live::{byte_offset_from_conv_or_overlay, clear_live_db_cache, live_db_for};

use crate::lsp::protocol::{path_to_uri, uri_to_path, PositionConverter};
use crate::overlay::OverlayManager;
use impl_line::{is_head_position, is_impl_header};
use ridx_format::{DefKind, WorkspaceIndex};
use serde_json::{json, Value};

pub fn handle_definition(msg: &Value, workspace: Option<&WorkspaceIndex>, overlay_mgr: &OverlayManager, conv: &mut PositionConverter) -> Value {
    let found = definition_at(msg, workspace, overlay_mgr, conv);
    syntactic::with_shorthand_field(found, msg, workspace, overlay_mgr, conv)
}

fn definition_at(msg: &Value, workspace: Option<&WorkspaceIndex>, overlay_mgr: &OverlayManager, conv: &mut PositionConverter) -> Value {
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

    // 0. What the syntax answers: control-flow keywords, labels, lifetimes, `mod foo;`, macro calls.
    if let Some(v) = syntactic::answer(&file, overlay_opt, line_0based, utf16_col_0based, workspace, conv) {
        return v;
    }

    // 1. Overlay definition. An item it finds by name yields to the index when the buffer is the indexed text: it
    // took `type T1` for the `const T1` a `match` pattern names. A local stays the buffer's: the compiler's span of
    // `let mut x` starts at `mut`, and a `for` pattern lies in the loop's desugaring.
    let clean = overlay_opt.is_none_or(|o| !o.is_dirty());
    let indexed = || clean && workspace.is_some_and(|ws| ws.find_definition_at(&file, line_1based, col_1based).is_some());
    if let Some(def) = overlay_mgr.find_definition_at(&file, line_1based, col_1based).filter(|d| d.kind == DefKind::Local || !indexed()) {
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
            if overlay.imports_changed_at(line_1based, col_1based) {
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
                let resolved_path = type_definition_parse::on_disk(def.resolved_path());
                if matches!(def.kind, DefKind::Mod | DefKind::Use | DefKind::ExternCrate) && !def.name.is_empty() {
                    if let Some(mod_file) = module_file::resolve_backing_module_file(ws, &def, &resolved_path) {
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

    // A binding the index did not resolve to a constant is its own definition, not an item of its name elsewhere
    // (`T1 => …` went to `trait T1`).
    if let Some(v) = overlay_opt.and_then(|o| syntactic::binding(&file, &o.text, line_0based, utf16_col_0based)) {
        return v;
    }
    // `Self` the index has no answer for: the type its `impl` names.
    let text = overlay_opt.map(|o| o.text.clone()).or_else(|| std::fs::read_to_string(&file).ok());
    if let Some(v) = text.and_then(|t| syntactic::self_type(&file, &t, line_0based, utf16_col_0based)) {
        return v;
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

    // 4. A primitive type (`usize`): its documentation module in `core`, as in rust-analyzer.
    let line_text = overlay_opt.and_then(|o| o.text.lines().nth(line_0based as usize)).map(str::to_string).or_else(|| conv.get_line(&file, line_1based).map(str::to_string));
    line_text.and_then(|l| primitive::definition(&l, col_1based)).unwrap_or(Value::Null)
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
                    let resolved_path = type_definition_parse::on_disk(r.resolved_path());
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
        // The text each impl's file has where the index placed it: the open buffer's when it is the compiled text, else
        // the file as saved; a buffer edited since is not narrowed (review of 465e02ad: the disk was read under a session
        // index of the buffer).
        let mut texts: std::collections::HashMap<String, Option<String>> = std::collections::HashMap::new();
        for r in &impl_refs {
            let path = type_definition_parse::on_disk(r.resolved_path()).to_string_lossy().to_string();
            texts.entry(path.clone()).or_insert_with(|| match overlay_mgr.get(&path) {
                Some(o) if o.is_dirty() => None,
                Some(o) => Some(o.text.clone()),
                None => std::fs::read_to_string(&path).ok(),
            });
        }
        let trees: std::collections::HashMap<&str, ridx_syntax::SyntaxTree<'_>> = texts.iter().filter_map(|(p, t)| Some((p.as_str(), ridx_syntax::parse(t.as_deref()?)))).collect();
        for r in impl_refs {
            let resolved_path = type_definition_parse::on_disk(r.resolved_path());
            let path_str = resolved_path.to_str().unwrap_or(r.file);
            let uri = path_to_uri(path_str);
            let span = r.span;
            let mut start_line = span.start_line().saturating_sub(1);
            let mut end_line = span.end_line().saturating_sub(1);
            let mut start_char = conv.char_col_to_utf16_col(path_str, span.start_line(), span.start_col());
            let mut end_char = conv.char_col_to_utf16_col(path_str, span.end_line(), span.end_col());
            // An impl header is placed at its self type, as rust-analyzer does (prod-code, m=20226); a derive's impl
            // stays at the derive.
            if let Some((text, tree)) = texts.get(path_str).and_then(|t| t.as_deref()).zip(trees.get(path_str)) {
                let line_start = text.split_inclusive('\n').take(start_line as usize).map(str::len).sum::<usize>();
                let start = line_start + text[line_start..].chars().take(span.start_col().saturating_sub(1) as usize).map(char::len_utf8).sum::<usize>();
                if let Some(ty) = impl_line::self_type_range(tree, start) {
                    let pos = |b: usize| {
                        let ls = text[..b].rfind('\n').map_or(0, |i| i + 1);
                        (text[..ls].matches('\n').count() as u32, text[ls..b].encode_utf16().count() as u32)
                    };
                    ((start_line, start_char), (end_line, end_char)) = (pos(ty.start), pos(ty.end));
                }
            }
            keyed.push((
                (uri.clone(), start_line, start_char, end_line, end_char),
                r.unit_key.unwrap_or(r.crate_name).to_string(),
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
    // The same impl found through several builds (units) of its crate is one implementation; several impls at one
    // place (a `#[derive(A, B)]`) are several, as rust-analyzer lists them: per place, as many as one unit holds.
    keyed.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    let mut results: Vec<Value> = Vec::new();
    let mut i = 0;
    while i < keyed.len() {
        let mut j = i;
        while j < keyed.len() && keyed[j].0 == keyed[i].0 {
            j += 1;
        }
        let group = &keyed[i..j];
        let mut per_unit: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
        for (_, unit, _) in group {
            *per_unit.entry(unit.as_str()).or_default() += 1;
        }
        let copies = per_unit.values().copied().max().unwrap_or(1);
        results.extend(std::iter::repeat_n(group[0].2.clone(), copies));
        i = j;
    }

    json!(results)
}
