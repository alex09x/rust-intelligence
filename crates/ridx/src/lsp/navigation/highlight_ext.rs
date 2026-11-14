/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! In-file lexical augmentation for `textDocument/documentHighlight`.

use crate::lsp::editing::is_rust_keyword;
use crate::lsp::protocol::PositionConverter;
use ridx_format::DefKind;
use serde_json::{json, Value};
use std::collections::HashSet;

fn opens_char_lit(chars: &[char], idx: usize) -> bool {
    match chars.get(idx + 1).copied() {
        Some('\\') => chars.get(idx + 2).is_some_and(|c| !matches!(c, '\n' | '\r')),
        Some(c) if !matches!(c, '\'' | '\n' | '\r') => chars.get(idx + 2) == Some(&'\''),
        _ => false,
    }
}

pub(crate) fn dedup_highlights(highlights: &mut Vec<Value>) {
    let mut seen = HashSet::new();
    highlights.retain(|h| {
        let sl = h["range"]["start"]["line"].as_u64().unwrap_or(0);
        let sc = h["range"]["start"]["character"].as_u64().unwrap_or(0);
        let el = h["range"]["end"]["line"].as_u64().unwrap_or(0);
        let ec = h["range"]["end"]["character"].as_u64().unwrap_or(0);
        seen.insert((sl, sc, el, ec))
    });
}

fn push_hl(file: &str, line_1based: u32, start_col_1based: u32, end_col_1based: u32, kind: u32, conv: &mut PositionConverter, out: &mut Vec<Value>) {
    let sc = conv.char_col_to_utf16_col(file, line_1based, start_col_1based);
    let ec = conv.char_col_to_utf16_col(file, line_1based, end_col_1based);
    let l0 = line_1based.saturating_sub(1);
    out.push(json!({
        "range": {
            "start": { "line": l0, "character": sc },
            "end": { "line": l0, "character": ec }
        },
        "kind": kind
    }));
}

fn scan_code_tokens<F>(text: &str, mut on_tok: F)
where
    F: FnMut(u32, u32, u32, &str, &str, &str, char),
{
    let mut in_block_comment = 0usize;
    let mut in_raw_hashes: Option<usize> = None;

    for (line_idx, line) in text.lines().enumerate() {
        let line_1based = (line_idx + 1) as u32;
        let chars: Vec<char> = line.chars().collect();
        let byte_offsets: Vec<usize> = line.char_indices().map(|(b, _)| b).collect();
        let mut i = 0;
        let mut in_string = false;
        let mut in_char = false;
        let mut escaped = false;

        while i < chars.len() {
            let ch = chars[i];
            let next = chars.get(i + 1).copied();

            if let Some(hashes) = in_raw_hashes {
                if ch == '"' && (0..hashes).all(|k| chars.get(i + 1 + k) == Some(&'#')) {
                    in_raw_hashes = None;
                    i += 1 + hashes;
                    continue;
                }
                i += 1;
                continue;
            }

            if in_block_comment > 0 {
                if ch == '/' && next == Some('*') {
                    in_block_comment += 1;
                    i += 2;
                    continue;
                }
                if ch == '*' && next == Some('/') {
                    in_block_comment -= 1;
                    i += 2;
                    continue;
                }
                i += 1;
                continue;
            }

            if in_string {
                if escaped {
                    escaped = false;
                } else if ch == '\\' {
                    escaped = true;
                } else if ch == '"' {
                    in_string = false;
                }
                i += 1;
                continue;
            }

            if in_char {
                if escaped {
                    escaped = false;
                } else if ch == '\\' {
                    escaped = true;
                } else if ch == '\'' {
                    in_char = false;
                }
                i += 1;
                continue;
            }

            if ch == '/' && next == Some('/') {
                break;
            }
            if ch == '/' && next == Some('*') {
                in_block_comment = 1;
                i += 2;
                continue;
            }
            if ch == 'r' && matches!(next, Some('"' | '#')) {
                let mut h = 0usize;
                let mut k = i + 1;
                while k < chars.len() && chars[k] == '#' {
                    h += 1;
                    k += 1;
                }
                if k < chars.len() && chars[k] == '"' {
                    in_raw_hashes = Some(h);
                    i = k + 1;
                    continue;
                }
            }
            if ch == '"' {
                in_string = true;
                i += 1;
                continue;
            }
            if ch == '\'' && opens_char_lit(&chars, i) {
                in_char = true;
                i += 1;
                continue;
            }

            if ch == '_' || ch.is_alphabetic() {
                let start_c = i;
                let start_b = byte_offsets[start_c];
                while i < chars.len() && (chars[i] == '_' || chars[i].is_alphanumeric()) {
                    i += 1;
                }
                let end_b = if i < byte_offsets.len() { byte_offsets[i] } else { line.len() };
                let tok = &line[start_b..end_b];
                let prefix = &line[..start_b];
                let suffix = &line[end_b..];
                on_tok(line_1based, (start_c + 1) as u32, (i + 1) as u32, tok, prefix, suffix, ' ');
                continue;
            }

            if matches!(ch, '{' | '}') {
                on_tok(line_1based, (i + 1) as u32, (i + 2) as u32, "", "", "", ch);
            }
            i += 1;
        }
    }
}

/// Adds the uses the index left out on top of the resolved `highlights`, read off the text by name. A name is not
/// a target, so a same-named item must be kept apart: a qualified token (`tokio_util::sync`, `io::Stderr::new`)
/// counts only when its qualifier is the target's parent module (or `crate`/`self`/`super`), and `Self` only inside
/// an `impl` whose header type the index already resolved to the target (not `impl … for Stderr` naming tokio's
/// own `Stderr` beside `std::io::Stderr`).
/// `target` is the target's name, its crate-relative path and its crate's name (the parent of a top-level item).
pub(crate) fn augment_resolved_highlights(
    file: &str,
    text: &str,
    (target_symbol, target_path, target_crate): (&str, &str, &str),
    target_kind: DefKind,
    conv: &mut PositionConverter,
    highlights: &mut Vec<Value>,
) {
    if target_symbol.is_empty() || highlights.is_empty() {
        return;
    }
    let is_type_like = matches!(target_kind, DefKind::Struct | DefKind::Enum | DefKind::Union | DefKind::TyAlias);
    if !is_type_like && !matches!(target_kind, DefKind::Mod | DefKind::AssocTy) {
        return;
    }
    let resolved: HashSet<(u64, u64)> =
        highlights.iter().map(|h| (h["range"]["start"]["line"].as_u64().unwrap_or(0), h["range"]["start"]["character"].as_u64().unwrap_or(0))).collect();
    let parent = target_path.rsplit("::").nth(1).unwrap_or(target_crate);
    let qualifier_fits = |prefix: &str| {
        let Some(p) = prefix.trim_end().strip_suffix("::") else { return true };
        let q = &p[p.trim_end_matches(|c: char| c.is_alphanumeric() || c == '_').len()..];
        q.is_empty() || q == parent || matches!(q, "crate" | "self" | "super" | "Self")
    };

    let mut impl_header_pending = false;
    let mut impl_in_where = false;
    let mut impl_target_matched = false;
    let mut impl_brace_depth = 0i32;

    scan_code_tokens(text, |line_1based, sc, ec, tok, prefix, suffix, brace| {
        if brace == '{' {
            if impl_header_pending {
                impl_header_pending = false;
                impl_in_where = false;
                if impl_target_matched {
                    impl_brace_depth = 1;
                }
            } else if impl_brace_depth > 0 {
                impl_brace_depth += 1;
            }
            return;
        }
        if brace == '}' {
            if impl_brace_depth > 0 {
                impl_brace_depth -= 1;
                if impl_brace_depth == 0 {
                    impl_target_matched = false;
                }
            }
            return;
        }
        if tok == "impl" && impl_brace_depth == 0 {
            impl_header_pending = true;
            impl_in_where = false;
            impl_target_matched = false;
            return;
        }
        if impl_header_pending {
            if tok == "for" {
                impl_target_matched = false;
                return;
            }
            if tok == "where" {
                impl_in_where = true;
                return;
            }
            if !impl_in_where && tok == target_symbol && !prefix.trim_end().ends_with(':') {
                let at = (u64::from(line_1based.saturating_sub(1)), u64::from(conv.char_col_to_utf16_col(file, line_1based, sc)));
                impl_target_matched = resolved.contains(&at);
            }
        }

        if target_kind == DefKind::Mod && tok == target_symbol {
            let p_trim = prefix.trim_end();
            let s_trim = suffix.trim_start();
            let shaped =
                (s_trim.starts_with("::") && !p_trim.ends_with('.')) || (p_trim.ends_with("::") && !s_trim.starts_with('(')) || p_trim.split_whitespace().last() == Some("mod");
            if shaped && qualifier_fits(prefix) {
                push_hl(file, line_1based, sc, ec, 1, conv, highlights);
            }
        } else if is_type_like {
            if (tok == "Self" && impl_brace_depth > 0) || (tok == target_symbol && suffix.trim_start().starts_with("::") && qualifier_fits(prefix)) {
                push_hl(file, line_1based, sc, ec, 1, conv, highlights);
            }
        } else if target_kind == DefKind::AssocTy && tok == target_symbol {
            let p_trim = prefix.trim_end();
            if p_trim.ends_with("::") || p_trim.split_whitespace().last() == Some("type") {
                push_hl(file, line_1based, sc, ec, 1, conv, highlights);
            }
        }
    });

    dedup_highlights(highlights);
}

pub(crate) fn fallback_cursor_highlights(file: &str, text: &str, line_1based: u32, col_1based: u32, conv: &mut PositionConverter, highlights: &mut Vec<Value>) {
    let mut cursor_tok: Option<(String, bool, bool)> = None;
    scan_code_tokens(text, |ln, sc, ec, tok, prefix, suffix, _| {
        if ln == line_1based && !tok.is_empty() && sc <= col_1based && col_1based <= ec {
            let p_trim = prefix.trim_end();
            let s_trim = suffix.trim_start();
            let is_qual = p_trim.ends_with("::") || p_trim.ends_with('.') || s_trim.starts_with("::");
            let is_upper = tok.chars().next().is_some_and(|c| c.is_ascii_uppercase());
            cursor_tok = Some((tok.to_string(), is_qual, is_upper));
        }
    });

    let Some((ident, is_qual, is_upper)) = cursor_tok else {
        return;
    };
    if is_rust_keyword(&ident) || (!is_qual && !is_upper) {
        return;
    }

    scan_code_tokens(text, |ln, sc, ec, tok, prefix, suffix, _| {
        if tok != ident {
            return;
        }
        if is_upper {
            push_hl(file, ln, sc, ec, 1, conv, highlights);
        } else {
            let p_trim = prefix.trim_end();
            let s_trim = suffix.trim_start();
            let last_kw = p_trim.split_whitespace().last().unwrap_or("");
            if p_trim.ends_with("::") || p_trim.ends_with('.') || s_trim.starts_with("::") || s_trim.starts_with('(') || matches!(last_kw, "fn" | "type" | "const" | "mod") {
                push_hl(file, ln, sc, ec, 1, conv, highlights);
            }
        }
    });

    dedup_highlights(highlights);
}
