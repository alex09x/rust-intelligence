/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Lexical scanner and compile-safe string-literal rewrites (`Add #`, `Remove #`, `Rewrite as regular/raw string`).

use super::action_parse::byte_to_utf16_pos;
pub use super::action_parse_lex::{find_string_lit_at_cursor, is_inside_attr_or_extern_abi, ScannedStringLit};
use serde_json::{json, Value};

pub const KIND_STR_ADD_HASH: &str = "refactor.str_add_hash";
pub const KIND_STR_REMOVE_HASH: &str = "refactor.str_remove_hash";
pub const KIND_STR_TO_REGULAR: &str = "refactor.str_to_regular";
pub const KIND_STR_TO_RAW: &str = "refactor.str_to_raw";

pub fn is_string_assist_data_kind(k: &str) -> bool {
    matches!(k, KIND_STR_ADD_HASH | KIND_STR_REMOVE_HASH | KIND_STR_TO_REGULAR | KIND_STR_TO_RAW)
}

#[allow(clippy::too_many_arguments)]
pub fn collect_string_literal_assists<F>(
    doc_text: &str,
    masked_doc: &str,
    starts: &[usize],
    uri: &str,
    cursor_line0: u32,
    cursor_col_u16: u32,
    cursor_byte: usize,
    doc_version: Option<i32>,
    matches_kind: &F,
) -> Vec<Value>
where
    F: Fn(&str) -> bool,
{
    let Some(lit) = find_string_lit_at_cursor(doc_text, cursor_byte) else {
        return Vec::new();
    };
    if is_inside_attr_or_extern_abi(masked_doc, lit.start_byte) {
        return Vec::new();
    }
    let mut out = Vec::new();
    if lit.is_raw {
        if matches_kind("refactor") {
            if let Some(rep) = build_str_replacement(doc_text, &lit, KIND_STR_ADD_HASH) {
                out.push(make_range_assist_action(
                    uri,
                    "Add #",
                    "refactor",
                    KIND_STR_ADD_HASH,
                    cursor_line0,
                    cursor_col_u16,
                    doc_text,
                    starts,
                    lit.start_byte,
                    lit.end_byte,
                    &rep,
                    doc_version,
                ));
            }
        }
        if matches_kind("refactor.rewrite") {
            if let Some(rep) = build_str_replacement(doc_text, &lit, KIND_STR_REMOVE_HASH) {
                out.push(make_range_assist_action(
                    uri,
                    "Remove #",
                    "refactor.rewrite",
                    KIND_STR_REMOVE_HASH,
                    cursor_line0,
                    cursor_col_u16,
                    doc_text,
                    starts,
                    lit.start_byte,
                    lit.end_byte,
                    &rep,
                    doc_version,
                ));
            }
            if let Some(rep) = build_str_replacement(doc_text, &lit, KIND_STR_TO_REGULAR) {
                out.push(make_range_assist_action(
                    uri,
                    "Rewrite as regular string",
                    "refactor.rewrite",
                    KIND_STR_TO_REGULAR,
                    cursor_line0,
                    cursor_col_u16,
                    doc_text,
                    starts,
                    lit.start_byte,
                    lit.end_byte,
                    &rep,
                    doc_version,
                ));
            }
        }
    } else if matches_kind("refactor.rewrite") {
        if let Some(rep) = build_str_replacement(doc_text, &lit, KIND_STR_TO_RAW) {
            out.push(make_range_assist_action(
                uri,
                "Rewrite as raw string",
                "refactor.rewrite",
                KIND_STR_TO_RAW,
                cursor_line0,
                cursor_col_u16,
                doc_text,
                starts,
                lit.start_byte,
                lit.end_byte,
                &rep,
                doc_version,
            ));
        }
    }
    out
}

pub fn resolve_string_assist(data_kind: &str, doc_text: &str, masked_doc: &str, starts: &[usize], uri: &str, cursor_byte: usize, doc_version: Option<i32>) -> Option<Value> {
    let lit = find_string_lit_at_cursor(doc_text, cursor_byte)?;
    if is_inside_attr_or_extern_abi(masked_doc, lit.start_byte) {
        return None;
    }
    let rep = build_str_replacement(doc_text, &lit, data_kind)?;
    Some(build_single_range_workspace_edit(uri, doc_text, starts, lit.start_byte, lit.end_byte, &rep, doc_version))
}

fn build_str_replacement(doc_text: &str, lit: &ScannedStringLit, data_kind: &str) -> Option<String> {
    let body = &doc_text[lit.body_start..lit.body_end];
    match data_kind {
        KIND_STR_ADD_HASH if lit.is_raw && lit.hashes < 255 => {
            let h = "#".repeat(lit.hashes + 1);
            Some(format!("{}r{h}\"{body}\"{h}", lit.prefix))
        }
        KIND_STR_REMOVE_HASH if lit.is_raw && lit.hashes >= 1 => {
            let new_h = lit.hashes - 1;
            let forbidden = format!("\"{}", "#".repeat(new_h));
            if body.contains(&forbidden) {
                return None;
            }
            let h = "#".repeat(new_h);
            Some(format!("{}r{h}\"{body}\"{h}", lit.prefix))
        }
        KIND_STR_TO_REGULAR if lit.is_raw => {
            let mut esc = String::with_capacity(body.len() + 8);
            for ch in body.chars() {
                match ch {
                    '\\' => esc.push_str("\\\\"),
                    '"' => esc.push_str("\\\""),
                    '\n' => esc.push_str("\\n"),
                    '\r' => esc.push_str("\\r"),
                    '\t' => esc.push_str("\\t"),
                    '\0' => esc.push_str("\\0"),
                    _ => esc.push(ch),
                }
            }
            Some(format!("{}\"{esc}\"", lit.prefix))
        }
        KIND_STR_TO_RAW if !lit.is_raw => {
            let unescaped = unescape_regular_string(body, lit.prefix == "c")?;
            let hashes = min_hashes_needed(&unescaped)?;
            let h = "#".repeat(hashes);
            Some(format!("{}r{h}\"{unescaped}\"{h}", lit.prefix))
        }
        _ => None,
    }
}

fn unescape_regular_string(raw_body: &str, is_cstr: bool) -> Option<String> {
    let mut out = String::with_capacity(raw_body.len());
    let mut chars = raw_body.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            if ch == '\r' || (is_cstr && ch == '\0') {
                return None;
            }
            out.push(ch);
            continue;
        }
        match chars.next()? {
            '\\' => out.push('\\'),
            '"' => out.push('"'),
            '\'' => out.push('\''),
            'n' => out.push('\n'),
            't' => out.push('\t'),
            '\n' => {
                while chars.peek().is_some_and(|c| matches!(c, ' ' | '\t' | '\n' | '\r')) {
                    chars.next();
                }
            }
            _ => return None,
        }
    }
    Some(out)
}

fn min_hashes_needed(content: &str) -> Option<usize> {
    if !content.contains('"') {
        return Some(0);
    }
    for n in 1..255usize {
        let pat = format!("\"{}", "#".repeat(n));
        if !content.contains(&pat) {
            return Some(n);
        }
    }
    None
}

#[allow(clippy::too_many_arguments)]
pub fn make_range_assist_action(
    uri: &str,
    title: &str,
    lsp_kind: &str,
    data_kind: &str,
    cursor_line0: u32,
    cursor_col_u16: u32,
    doc_text: &str,
    starts: &[usize],
    rep_start: usize,
    rep_end: usize,
    new_text: &str,
    doc_version: Option<i32>,
) -> Value {
    let edit_obj = build_single_range_workspace_edit(uri, doc_text, starts, rep_start, rep_end, new_text, doc_version);
    json!({
        "title": title,
        "kind": lsp_kind,
        "data": {
            "uri": uri,
            "version": doc_version,
            "kind": data_kind,
            "cursor": { "line": cursor_line0, "character": cursor_col_u16 }
        },
        "edit": edit_obj
    })
}

pub fn build_single_range_workspace_edit(uri: &str, doc_text: &str, starts: &[usize], rep_start: usize, rep_end: usize, new_text: &str, doc_version: Option<i32>) -> Value {
    let (sl, sc) = byte_to_utf16_pos(doc_text, starts, rep_start);
    let (el, ec) = byte_to_utf16_pos(doc_text, starts, rep_end);
    build_multi_edits_workspace_edit(
        uri,
        vec![json!({
            "range": { "start": { "line": sl, "character": sc }, "end": { "line": el, "character": ec } },
            "newText": new_text
        })],
        doc_version,
    )
}

pub fn build_multi_edits_workspace_edit(uri: &str, edits: Vec<Value>, doc_version: Option<i32>) -> Value {
    let mut edit_obj = json!({ "changes": { uri: edits.clone() } });
    if let Some(ver) = doc_version {
        edit_obj["documentChanges"] = json!([{
            "textDocument": { "uri": uri, "version": ver },
            "edits": edits
        }]);
    }
    edit_obj
}
