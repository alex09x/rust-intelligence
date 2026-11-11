/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! LSP semantic tokens encoding, delta computation, and range filtering.

use crate::lsp::protocol::{uri_to_path, PositionConverter};
use crate::overlay::OverlayManager;
use ridx_format::{DefKind, WorkspaceIndex};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

pub const TOKEN_TYPE_NAMESPACE: u32 = 0;
pub const TOKEN_TYPE_TYPE: u32 = 1;
pub const TOKEN_TYPE_CLASS: u32 = 2;
pub const TOKEN_TYPE_ENUM: u32 = 3;
pub const TOKEN_TYPE_INTERFACE: u32 = 4;
pub const TOKEN_TYPE_STRUCT: u32 = 5;
pub const TOKEN_TYPE_TYPE_PARAMETER: u32 = 6;
pub const TOKEN_TYPE_PARAMETER: u32 = 7;
pub const TOKEN_TYPE_VARIABLE: u32 = 8;
pub const TOKEN_TYPE_PROPERTY: u32 = 9;
pub const TOKEN_TYPE_ENUM_MEMBER: u32 = 10;
pub const TOKEN_TYPE_FUNCTION: u32 = 11;
pub const TOKEN_TYPE_METHOD: u32 = 12;
pub const TOKEN_TYPE_MACRO: u32 = 13;
pub const TOKEN_TYPE_KEYWORD: u32 = 14;
pub const TOKEN_TYPE_COMMENT: u32 = 15;
pub const TOKEN_TYPE_STRING: u32 = 16;
pub const TOKEN_TYPE_NUMBER: u32 = 17;
/// rust-analyzer's `unresolvedReference`: a name that resolves to nothing.
pub const TOKEN_TYPE_UNRESOLVED: u32 = 18;
/// rust-analyzer's `generic`: a token of an attribute it does not analyze.
pub const TOKEN_TYPE_GENERIC: u32 = 19;

pub const MOD_DECLARATION: u32 = 1 << 0;
pub const MOD_DEFINITION: u32 = 1 << 1;
pub const MOD_READONLY: u32 = 1 << 2;
pub const MOD_STATIC: u32 = 1 << 3;

static RESULT_COUNTER: AtomicU64 = AtomicU64::new(1);
static SEMANTIC_CACHE: OnceLock<Mutex<HashMap<String, Vec<u32>>>> = OnceLock::new();

fn get_cache() -> &'static Mutex<HashMap<String, Vec<u32>>> {
    SEMANTIC_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawToken {
    pub line: u32,
    pub col: u32,
    pub length: u32,
    pub token_type: u32,
    pub token_modifiers: u32,
}

pub fn defkind_to_token(kind: DefKind) -> (u32, u32) {
    match kind {
        DefKind::Mod => (TOKEN_TYPE_NAMESPACE, 0),
        DefKind::TyAlias | DefKind::AssocTy | DefKind::OpaqueTy => (TOKEN_TYPE_TYPE, 0),
        DefKind::Enum => (TOKEN_TYPE_ENUM, 0),
        DefKind::Trait | DefKind::TraitAlias => (TOKEN_TYPE_INTERFACE, 0),
        DefKind::Struct | DefKind::Union => (TOKEN_TYPE_STRUCT, 0),
        DefKind::TyParam => (TOKEN_TYPE_TYPE_PARAMETER, 0),
        DefKind::Field => (TOKEN_TYPE_PROPERTY, 0),
        DefKind::Variant | DefKind::VariantCtor => (TOKEN_TYPE_ENUM_MEMBER, 0),
        DefKind::Fn => (TOKEN_TYPE_FUNCTION, MOD_DEFINITION),
        DefKind::AssocFn => (TOKEN_TYPE_METHOD, MOD_DEFINITION),
        DefKind::Macro => (TOKEN_TYPE_MACRO, 0),
        DefKind::Const | DefKind::AssocConst => (TOKEN_TYPE_VARIABLE, MOD_READONLY | MOD_STATIC),
        DefKind::Static => (TOKEN_TYPE_VARIABLE, MOD_STATIC),
        DefKind::Local => (TOKEN_TYPE_VARIABLE, 0),
        DefKind::ConstParam => (TOKEN_TYPE_VARIABLE, MOD_READONLY),
        _ => (TOKEN_TYPE_TYPE, 0),
    }
}

fn filter_non_overlapping(mut tokens: Vec<RawToken>) -> Vec<RawToken> {
    tokens.sort_by(|a, b| a.line.cmp(&b.line).then_with(|| a.length.cmp(&b.length)).then_with(|| a.col.cmp(&b.col)));

    let mut by_line: std::collections::BTreeMap<u32, Vec<RawToken>> = std::collections::BTreeMap::new();
    for tok in tokens {
        by_line.entry(tok.line).or_default().push(tok);
    }

    let mut result = Vec::new();
    for (_line, line_tokens) in by_line {
        let mut chosen: Vec<RawToken> = Vec::new();
        for tok in line_tokens {
            let tok_end = tok.col + tok.length;
            let overlaps = chosen.iter().any(|c| {
                let c_end = c.col + c.length;
                tok.col < c_end && tok_end > c.col
            });
            if !overlaps {
                chosen.push(tok);
            }
        }
        chosen.sort_by_key(|t| t.col);
        result.extend(chosen);
    }

    result.sort_by(|a, b| a.line.cmp(&b.line).then_with(|| a.col.cmp(&b.col)));
    result
}

pub fn collect_raw_tokens(file: &str, workspace: Option<&WorkspaceIndex>, overlay_mgr: &OverlayManager, conv: &mut PositionConverter) -> Vec<RawToken> {
    let mut tokens = Vec::new();
    let overlay_opt = overlay_mgr.get(file);
    let mut indexed = false;

    // 1. Overlay symbols (live buffers)
    if let Some(overlay) = overlay_opt {
        for sym in &overlay.symbols {
            if sym.start_line == sym.end_line {
                let line_0based = sym.start_line.saturating_sub(1);
                let col_0based = overlay.char_col_to_utf16_col(sym.start_line, sym.start_col);
                let length = overlay.char_col_to_utf16_col(sym.end_line, sym.end_col).saturating_sub(col_0based);
                if length > 0 {
                    let (token_type, token_modifiers) = defkind_to_token(sym.kind);
                    tokens.push(RawToken { line: line_0based, col: col_0based, length, token_type, token_modifiers });
                }
            }
        }
    }

    // 2. Workspace compiled symbols (definitions and references) from the readers that own the
    //    file, each asked for the path string it stores for it.
    if let Some(ws) = workspace {
        for idx in ws.reader_indices_for_file(file) {
            let Some(target) = ws.target_file_for_reader(idx, file) else {
                continue;
            };
            let reader = &ws.readers()[idx];

            if let Some(file_id) = reader.find_file_id(target.as_ref()) {
                indexed = true;
                for def in reader.definitions() {
                    let dk = DefKind::from(def.kind);
                    if matches!(dk, DefKind::Impl | DefKind::ForeignMod | DefKind::GlobalAsm | DefKind::AnonConst | DefKind::InlineConst | DefKind::Closure | DefKind::Other) {
                        continue;
                    }
                    if def.is_macro_expansion() && !matches!(dk, DefKind::Struct | DefKind::Enum | DefKind::Trait) {
                        continue;
                    }
                    if def.span.file_id == file_id && def.span.start_line == def.span.end_line {
                        let (sl, sc, length) = if let Some(overlay) = overlay_opt.filter(|o| o.is_dirty()) {
                            if let Some((live_l, live_c)) = overlay.map_compiled_to_live(def.span.start_line, def.span.start_col) {
                                let span_len = def.span.end_col.saturating_sub(def.span.start_col);
                                let l0 = live_l.saturating_sub(1);
                                let c0 = overlay.char_col_to_utf16_col(live_l, live_c);
                                let c1 = overlay.char_col_to_utf16_col(live_l, live_c + span_len);
                                (l0, c0, c1.saturating_sub(c0))
                            } else {
                                continue;
                            }
                        } else {
                            let sl = def.span.start_line.saturating_sub(1);
                            let sc = conv.char_col_to_utf16_col(file, def.span.start_line, def.span.start_col);
                            let ec = conv.char_col_to_utf16_col(file, def.span.end_line, def.span.end_col);
                            (sl, sc, ec.saturating_sub(sc))
                        };

                        if length > 0 {
                            let tok_pair = if dk == DefKind::Use {
                                super::tokens_scan::resolve_use_token(reader.get_str(def.name_offset()), length, reader)
                            } else {
                                Some(defkind_to_token(dk))
                            };
                            if let Some((tt, tm)) = tok_pair {
                                tokens.push(RawToken { line: sl, col: sc, length, token_type: tt, token_modifiers: tm });
                            }
                        }
                    }
                }

                for rf in reader.references() {
                    if rf.span.file_id == file_id && !rf.is_macro_expansion() && rf.span.start_line == rf.span.end_line {
                        let (sl, sc, length) = if let Some(overlay) = overlay_opt.filter(|o| o.is_dirty()) {
                            if let Some((live_l, live_c)) = overlay.map_compiled_to_live(rf.span.start_line, rf.span.start_col) {
                                let span_len = rf.span.end_col.saturating_sub(rf.span.start_col);
                                let l0 = live_l.saturating_sub(1);
                                let c0 = overlay.char_col_to_utf16_col(live_l, live_c);
                                let c1 = overlay.char_col_to_utf16_col(live_l, live_c + span_len);
                                (l0, c0, c1.saturating_sub(c0))
                            } else {
                                continue;
                            }
                        } else {
                            let sl = rf.span.start_line.saturating_sub(1);
                            let sc = conv.char_col_to_utf16_col(file, rf.span.start_line, rf.span.start_col);
                            let ec = conv.char_col_to_utf16_col(file, rf.span.end_line, rf.span.end_col);
                            (sl, sc, ec.saturating_sub(sc))
                        };

                        if length > 0 {
                            let (tt, _) = defkind_to_token(DefKind::from(rf.kind));
                            tokens.push(RawToken { line: sl, col: sc, length, token_type: tt, token_modifiers: 0 });
                        }
                    }
                }
            }
        }
    }

    let disk_text;
    let text = match overlay_opt {
        Some(ov) => Some(ov.text.as_str()),
        None => {
            disk_text = std::fs::read_to_string(file).ok();
            disk_text.as_deref()
        }
    };
    if let Some(text) = text {
        let unindexed = workspace.is_some() && !indexed && overlay_opt.is_none();
        // A target root its own `#![cfg(…)]` switches off compiles to an empty crate: indexed, with nothing in it.
        let compiled_empty = indexed && overlay_opt.is_none() && tokens.is_empty();
        let gated_off = (unindexed || compiled_empty) && super::tokens_detached::gated_off_by_inner_cfg(text);
        if gated_off {
            // rust-analyzer resolves what the crate root reaches and leaves the file's own names unresolved.
            let mut defs = Vec::new();
            super::tokens_detached::detached_definition_tokens(text, &mut defs);
            super::tokens_scan::scan_source_tokens(text, true, false, &mut tokens);
            super::tokens_attr::apply_attribute_rules(text, &mut tokens);
            super::tokens_gated::retype_gated_tokens(text, &defs, &mut tokens);
            super::tokens_doc::scan_doc_comments(text, None, &mut tokens);
            return filter_non_overlapping(tokens);
        }
        if unindexed && super::tokens_detached::outside_every_crate(file) {
            // A file no indexed crate includes: rust-analyzer types only its definition sites, by syntax.
            super::tokens_detached::detached_definition_tokens(text, &mut tokens);
            super::tokens_doc::scan_doc_comments(text, None, &mut tokens);
            return filter_non_overlapping(tokens);
        }
        // Lines the index has tokens on: an item behind `#[cfg(…)]` without any was left out of the build.
        let compiled: std::collections::HashSet<u32> =
            if indexed && overlay_opt.is_none() && text.contains("#[cfg(") { tokens.iter().map(|t| t.line).collect() } else { Default::default() };
        if workspace.is_some_and(|ws| ws.readers().len() > 2) {
            let has_compiled = !tokens.is_empty();
            super::tokens_scan::scan_source_tokens(text, overlay_opt.is_none(), has_compiled, &mut tokens);
        }
        super::tokens_attr::apply_attribute_rules(text, &mut tokens);
        if !compiled.is_empty() {
            let mut defs = Vec::new();
            super::tokens_detached::detached_definition_tokens(text, &mut defs);
            super::tokens_inactive::retype_inactive_items(text, &compiled, &defs, &mut tokens);
        }
        if let Some(ws) = workspace {
            super::tokens_fixture::scan_fixtures(file, text, ws, &mut tokens);
        }
        super::tokens_doc::scan_doc_comments(text, workspace, &mut tokens);
    }

    filter_non_overlapping(tokens)
}

pub fn encode_tokens_delta(tokens: &[RawToken]) -> Vec<u32> {
    let mut data = Vec::with_capacity(tokens.len() * 5);
    let mut prev_line = 0;
    let mut prev_col = 0;

    for tok in tokens {
        let delta_line = tok.line.saturating_sub(prev_line);
        let delta_col = if delta_line == 0 { tok.col.saturating_sub(prev_col) } else { tok.col };

        data.extend_from_slice(&[delta_line, delta_col, tok.length, tok.token_type, tok.token_modifiers]);
        prev_line = tok.line;
        prev_col = tok.col;
    }

    data
}

/// Handles `textDocument/semanticTokens/full` requests.
pub fn handle_semantic_tokens_full(msg: &Value, workspace: Option<&WorkspaceIndex>, overlay_mgr: &OverlayManager, conv: &mut PositionConverter) -> Value {
    let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
    let file = uri_to_path(uri);

    let raw_tokens = collect_raw_tokens(&file, workspace, overlay_mgr, conv);
    let data = encode_tokens_delta(&raw_tokens);

    let result_id = format!("ridx-st-{}", RESULT_COUNTER.fetch_add(1, Ordering::SeqCst));
    if let Ok(mut cache) = get_cache().lock() {
        if cache.len() > 256 {
            cache.clear();
        }
        cache.insert(result_id.clone(), data.clone());
    }

    json!({
        "resultId": result_id,
        "data": data
    })
}

/// Handles `textDocument/semanticTokens/full/delta` requests.
pub fn handle_semantic_tokens_delta(msg: &Value, workspace: Option<&WorkspaceIndex>, overlay_mgr: &OverlayManager, conv: &mut PositionConverter) -> Value {
    let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
    let prev_id = msg["params"]["previousResultId"].as_str().unwrap_or("");
    let file = uri_to_path(uri);

    let raw_tokens = collect_raw_tokens(&file, workspace, overlay_mgr, conv);
    let new_data = encode_tokens_delta(&raw_tokens);
    let new_id = format!("ridx-st-{}", RESULT_COUNTER.fetch_add(1, Ordering::SeqCst));

    let mut cache = get_cache().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(cached_data) = cache.get(prev_id).cloned() {
        if cached_data == new_data {
            cache.insert(new_id.clone(), new_data);
            return json!({
                "resultId": new_id,
                "edits": []
            });
        }

        // Find common prefix and suffix
        let mut prefix = 0;
        while prefix < cached_data.len() && prefix < new_data.len() && cached_data[prefix] == new_data[prefix] {
            prefix += 1;
        }

        let mut suffix = 0;
        while suffix < cached_data.len() - prefix && suffix < new_data.len() - prefix && cached_data[cached_data.len() - 1 - suffix] == new_data[new_data.len() - 1 - suffix] {
            suffix += 1;
        }

        let delete_count = (cached_data.len() - prefix - suffix) as u32;
        let insert_data = &new_data[prefix..new_data.len() - suffix];

        let edit = json!({
            "start": prefix,
            "deleteCount": delete_count,
            "data": insert_data
        });

        cache.insert(new_id.clone(), new_data);
        return json!({
            "resultId": new_id,
            "edits": [edit]
        });
    }

    cache.insert(new_id.clone(), new_data.clone());
    json!({
        "resultId": new_id,
        "data": new_data
    })
}

/// Handles `textDocument/semanticTokens/range` requests.
pub fn handle_semantic_tokens_range(msg: &Value, workspace: Option<&WorkspaceIndex>, overlay_mgr: &OverlayManager, conv: &mut PositionConverter) -> Value {
    let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
    let file = uri_to_path(uri);
    let start_line = msg["params"]["range"]["start"]["line"].as_u64().unwrap_or(0) as u32;
    let end_line = msg["params"]["range"]["end"]["line"].as_u64().unwrap_or(u32::MAX as u64) as u32;

    let raw_tokens = collect_raw_tokens(&file, workspace, overlay_mgr, conv);
    let in_range: Vec<RawToken> = raw_tokens.into_iter().filter(|t| t.line >= start_line && t.line <= end_line).collect();

    let data = encode_tokens_delta(&in_range);
    json!({ "data": data })
}
