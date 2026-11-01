/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Source-level fallback and supplemental token scanning for `textDocument/semanticTokens/*`.

use super::tokens::{
    defkind_to_token, RawToken, MOD_DEFINITION, TOKEN_TYPE_ENUM, TOKEN_TYPE_ENUM_MEMBER, TOKEN_TYPE_FUNCTION, TOKEN_TYPE_INTERFACE, TOKEN_TYPE_MACRO, TOKEN_TYPE_METHOD,
    TOKEN_TYPE_NAMESPACE, TOKEN_TYPE_STRUCT, TOKEN_TYPE_VARIABLE,
};
use crate::overlay::{DocumentOverlay, OverlayManager};
use ridx_format::{DefKind, IndexReader};

pub fn resolve_use_token<D: AsRef<[u8]>>(name: &str, span_len: u32, reader: &IndexReader<D>) -> Option<(u32, u32)> {
    if name.is_empty() || span_len != name.len() as u32 {
        return None;
    }
    for d in reader.definitions() {
        let dk = DefKind::from(d.kind);
        if !matches!(dk, DefKind::Use | DefKind::Impl | DefKind::Local | DefKind::Field | DefKind::TyParam | DefKind::Other) && reader.get_str(d.name_offset()) == name {
            return Some(defkind_to_token(dk));
        }
    }
    if name.bytes().next().is_some_and(|b| b.is_ascii_uppercase()) {
        if name.ends_with("Ext") || name.ends_with("Trait") {
            Some((TOKEN_TYPE_INTERFACE, 0))
        } else {
            Some((TOKEN_TYPE_STRUCT, 0))
        }
    } else if name.contains('_') {
        Some((TOKEN_TYPE_FUNCTION, 0))
    } else {
        Some((TOKEN_TYPE_NAMESPACE, 0))
    }
}

fn is_rust_kw_or_prim(ident: &str) -> bool {
    matches!(
        ident,
        "as" | "async"
            | "await"
            | "box"
            | "break"
            | "const"
            | "continue"
            | "crate"
            | "dyn"
            | "else"
            | "enum"
            | "extern"
            | "false"
            | "fn"
            | "for"
            | "if"
            | "impl"
            | "in"
            | "let"
            | "loop"
            | "macro_rules"
            | "match"
            | "mod"
            | "move"
            | "mut"
            | "pub"
            | "ref"
            | "return"
            | "self"
            | "Self"
            | "static"
            | "struct"
            | "super"
            | "trait"
            | "true"
            | "type"
            | "union"
            | "unsafe"
            | "use"
            | "where"
            | "while"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "u128"
            | "usize"
            | "i8"
            | "i16"
            | "i32"
            | "i64"
            | "i128"
            | "isize"
            | "f32"
            | "f64"
            | "bool"
            | "char"
            | "str"
            | "_"
    )
}

fn byte_to_utf16_col(line: &str, byte_idx: usize) -> u32 {
    if line.is_ascii() {
        byte_idx as u32
    } else {
        line[..byte_idx.min(line.len())].encode_utf16().count() as u32
    }
}

pub fn scan_source_tokens(file: &str, overlay_mgr: &OverlayManager, has_compiled_tokens: bool, tokens: &mut Vec<RawToken>) {
    let text_opt = if let Some(ov) = overlay_mgr.get(file) { Some(ov.text.clone()) } else { std::fs::read_to_string(file).ok() };
    let Some(text) = text_opt else {
        return;
    };

    if !has_compiled_tokens && overlay_mgr.get(file).is_none() {
        let detached = DocumentOverlay::parse_detached(text.clone());
        for sym in &detached.symbols {
            if sym.start_line == sym.end_line {
                let l0 = sym.start_line.saturating_sub(1);
                let c0 = detached.char_col_to_utf16_col(sym.start_line, sym.start_col);
                let len = detached.char_col_to_utf16_col(sym.end_line, sym.end_col).saturating_sub(c0);
                if len > 0 {
                    let (tt, tm) = defkind_to_token(sym.kind);
                    tokens.push(RawToken { line: l0, col: c0, length: len, token_type: tt, token_modifiers: tm });
                }
            }
        }
    }

    let mut in_block_comment = false;
    for (line_idx, line) in text.lines().enumerate() {
        let bytes = line.as_bytes();
        let n = bytes.len();
        let mut i = 0;
        let mut prev_word = "";
        let mut prev_dot = false;

        while i < n {
            if in_block_comment {
                if i + 1 < n && bytes[i] == b'*' && bytes[i + 1] == b'/' {
                    in_block_comment = false;
                    i += 2;
                } else {
                    i += 1;
                }
                continue;
            }
            if i + 1 < n && bytes[i] == b'/' && bytes[i + 1] == b'/' {
                break;
            }
            if i + 1 < n && bytes[i] == b'/' && bytes[i + 1] == b'*' {
                in_block_comment = true;
                i += 2;
                continue;
            }
            if bytes[i] == b'"' {
                i += 1;
                while i < n {
                    if bytes[i] == b'\\' {
                        i += 2;
                    } else if bytes[i] == b'"' {
                        i += 1;
                        break;
                    } else {
                        i += 1;
                    }
                }
                continue;
            }
            if bytes[i] == b'.' {
                prev_dot = true;
                i += 1;
                continue;
            }
            if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
                let start = i;
                i += 1;
                while i < n && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                let ident = &line[start..i];
                if !is_rust_kw_or_prim(ident) {
                    let after = line[i..].trim_start();
                    let tok_opt = if after.starts_with('!') && !after.starts_with("!=") {
                        Some((TOKEN_TYPE_MACRO, 0))
                    } else if after.starts_with("::") {
                        if ident.bytes().next().is_some_and(|b| b.is_ascii_uppercase()) {
                            Some((TOKEN_TYPE_STRUCT, 0))
                        } else {
                            Some((TOKEN_TYPE_NAMESPACE, 0))
                        }
                    } else if matches!(prev_word, "let" | "mut" | "for") {
                        Some((TOKEN_TYPE_VARIABLE, 0))
                    } else if prev_word == "fn" {
                        Some((TOKEN_TYPE_FUNCTION, MOD_DEFINITION))
                    } else if prev_word == "struct" || prev_word == "union" {
                        Some((TOKEN_TYPE_STRUCT, 0))
                    } else if prev_word == "enum" {
                        Some((TOKEN_TYPE_ENUM, 0))
                    } else if prev_word == "trait" {
                        Some((TOKEN_TYPE_INTERFACE, 0))
                    } else if prev_word == "mod" {
                        Some((TOKEN_TYPE_NAMESPACE, 0))
                    } else if matches!(ident, "Some" | "None" | "Ok" | "Err") {
                        Some((TOKEN_TYPE_ENUM_MEMBER, 0))
                    } else if ident.bytes().next().is_some_and(|b| b.is_ascii_uppercase()) {
                        Some((TOKEN_TYPE_STRUCT, 0))
                    } else if after.starts_with('(') {
                        Some((if prev_dot { TOKEN_TYPE_METHOD } else { TOKEN_TYPE_FUNCTION }, 0))
                    } else if !has_compiled_tokens {
                        Some((TOKEN_TYPE_VARIABLE, 0))
                    } else {
                        None
                    };
                    if let Some((tt, tm)) = tok_opt {
                        let col = byte_to_utf16_col(line, start);
                        let end_col = byte_to_utf16_col(line, i);
                        let length = end_col.saturating_sub(col);
                        if length > 0 {
                            tokens.push(RawToken { line: line_idx as u32, col, length, token_type: tt, token_modifiers: tm });
                        }
                    }
                }
                prev_word = ident;
                prev_dot = false;
                continue;
            }
            if !bytes[i].is_ascii_whitespace() {
                prev_dot = false;
            }
            i += 1;
        }
    }
}
