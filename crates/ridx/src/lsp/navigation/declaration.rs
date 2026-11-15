/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! LSP `textDocument/declaration` handler: resolves trait-impl associated items to their
//! trait declaration, delegates standard definitions to `handle_definition`, and supports
//! declaration tokens inside multi-line raw string test fixtures.

use super::{handle_definition, is_def_valid_in_overlay};
use crate::lsp::protocol::{path_to_uri, uri_to_path, PositionConverter};
use crate::overlay::OverlayManager;
use ridx_format::WorkspaceIndex;
use serde_json::{json, Value};

const DECL_KEYWORDS: &[&str] = &["fn", "struct", "enum", "trait", "type", "union", "const", "static", "mod"];

pub fn handle_declaration(msg: &Value, workspace: Option<&WorkspaceIndex>, overlay_mgr: &OverlayManager, conv: &mut PositionConverter) -> Value {
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

    if let Some(ws) = workspace {
        let (query_line, query_col) = if let Some(overlay) = overlay_opt.filter(|o| o.is_dirty()) {
            if overlay.has_modified_imports() {
                (0, 0)
            } else {
                overlay.map_live_to_compiled(line_1based, col_1based).unwrap_or((0, 0))
            }
        } else {
            (line_1based, col_1based)
        };

        if query_line > 0 {
            if let Some(def) = ws.find_definition_at(&file, query_line, query_col) {
                if let Some(trait_decl) = ws.find_trait_item_declaration(&def) {
                    let resolved_path = super::type_definition_parse::on_disk(trait_decl.resolved_path());
                    let path_str = resolved_path.to_str().unwrap_or(trait_decl.file);
                    let target_uri = path_to_uri(path_str);
                    let (start_line, end_line, start_char, end_char) = if let Some(target_ov) = overlay_mgr.get(path_str).filter(|o| o.is_dirty()) {
                        match (target_ov.map_compiled_to_live(trait_decl.start_line, trait_decl.start_col), target_ov.map_compiled_to_live(trait_decl.end_line, trait_decl.end_col))
                        {
                            (Some(s), Some(e)) if is_def_valid_in_overlay(target_ov, trait_decl.name, s.0, e.0) => {
                                (s.0, e.0, target_ov.char_col_to_utf16_col(s.0, s.1), target_ov.char_col_to_utf16_col(e.0, e.1))
                            }
                            _ => (0, 0, 0, 0),
                        }
                    } else {
                        let sc = conv.char_col_to_utf16_col(path_str, trait_decl.start_line, trait_decl.start_col);
                        let ec = conv.char_col_to_utf16_col(path_str, trait_decl.end_line, trait_decl.end_col);
                        (trait_decl.start_line, trait_decl.end_line, sc, ec)
                    };
                    if start_line > 0 {
                        return json!({
                            "uri": target_uri,
                            "range": {
                                "start": { "line": start_line.saturating_sub(1), "character": start_char },
                                "end": { "line": end_line.saturating_sub(1), "character": end_char }
                            }
                        });
                    }
                }
            }
        }
    }

    let res = handle_definition(msg, workspace, overlay_mgr, conv);
    if !res.is_null() {
        return res;
    }

    if let Some((sc, ec)) = raw_string_fixture_decl_cols(&file, line_1based, col_1based, overlay_mgr) {
        let start_u16 = conv.char_col_to_utf16_col(&file, line_1based, sc);
        let end_u16 = conv.char_col_to_utf16_col(&file, line_1based, ec);
        return json!({
            "uri": path_to_uri(&file),
            "range": {
                "start": { "line": line_0based, "character": start_u16 },
                "end": { "line": line_0based, "character": end_u16 }
            }
        });
    }
    Value::Null
}

fn raw_string_fixture_decl_cols(file: &str, line_1based: u32, col_1based: u32, overlay_mgr: &OverlayManager) -> Option<(u32, u32)> {
    let owned;
    let text = if let Some(ov) = overlay_mgr.get(file) {
        ov.text.as_str()
    } else {
        owned = std::fs::read_to_string(file).ok()?;
        owned.as_str()
    };
    let line_idx = (line_1based as usize).checked_sub(1)?;
    let mut line_start_byte = 0usize;
    let mut cur_line = 0usize;
    for (idx, b) in text.bytes().enumerate() {
        if cur_line == line_idx {
            line_start_byte = idx;
            break;
        }
        if b == b'\n' {
            cur_line += 1;
        }
    }
    if cur_line != line_idx || !is_in_multiline_raw_string(&text[..line_start_byte]) {
        return None;
    }
    let rest = &text[line_start_byte..];
    let line_str = rest.split('\n').next()?.trim_end_matches('\r');
    let chars: Vec<(usize, char)> = line_str.char_indices().collect();
    let idx = (col_1based as usize).checked_sub(1).filter(|&i| i < chars.len())?;
    let is_id = |c: char| c.is_ascii_alphanumeric() || c == '_';
    if !is_id(chars[idx].1) {
        return None;
    }
    let mut s = idx;
    while s > 0 && is_id(chars[s - 1].1) {
        s -= 1;
    }
    let mut e = idx + 1;
    while e < chars.len() && is_id(chars[e].1) {
        e += 1;
    }
    let before = line_str[..chars[s].0].trim_end();
    let has_decl_kw = DECL_KEYWORDS.iter().any(|&kw| before.strip_suffix(kw).is_some_and(|prefix| prefix.is_empty() || !prefix.chars().next_back().is_some_and(is_id)));
    has_decl_kw.then_some(((s as u32) + 1, (e as u32) + 1))
}

fn is_in_multiline_raw_string(prefix: &str) -> bool {
    let bytes = prefix.as_bytes();
    let n = bytes.len();
    let mut i = 0;
    while i < n {
        let ch = bytes[i];
        if ch == b'/' && i + 1 < n && bytes[i + 1] == b'/' {
            while i < n && bytes[i] != b'\n' {
                i += 1;
            }
        } else if ch == b'/' && i + 1 < n && bytes[i + 1] == b'*' {
            let mut depth = 1usize;
            i += 2;
            while i < n && depth > 0 {
                if i + 1 < n && bytes[i] == b'/' && bytes[i + 1] == b'*' {
                    depth += 1;
                    i += 2;
                } else if i + 1 < n && bytes[i] == b'*' && bytes[i + 1] == b'/' {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
        } else if matches!(ch, b'r' | b'b' | b'c') && (i == 0 || !(bytes[i - 1].is_ascii_alphanumeric() || matches!(bytes[i - 1], b'_' | b'\'' | b'"'))) {
            let mut j = if matches!(ch, b'b' | b'c') {
                if i + 1 < n && bytes[i + 1] == b'r' {
                    i + 2
                } else {
                    i += 1;
                    continue;
                }
            } else {
                i + 1
            };
            let mut hashes = 0usize;
            while j < n && bytes[j] == b'#' {
                hashes += 1;
                j += 1;
            }
            if j < n && bytes[j] == b'"' {
                j += 1;
                let mut closed = false;
                while j < n {
                    if bytes[j] == b'"' && j + hashes < n && bytes[j + 1..j + 1 + hashes].iter().all(|&b| b == b'#') {
                        j += 1 + hashes;
                        closed = true;
                        break;
                    }
                    j += 1;
                }
                if !closed {
                    return true;
                }
                i = j;
            } else {
                i += 1;
            }
        } else if ch == b'"' {
            i += 1;
            while i < n && bytes[i] != b'"' {
                i += if bytes[i] == b'\\' { 2 } else { 1 };
            }
            i += 1;
        } else {
            i += 1;
        }
    }
    false
}
