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
    TOKEN_TYPE_NAMESPACE, TOKEN_TYPE_STRUCT, TOKEN_TYPE_TYPE, TOKEN_TYPE_TYPE_PARAMETER, TOKEN_TYPE_VARIABLE,
};
use crate::overlay::DocumentOverlay;
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
            Some(capitalized_token(name))
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

pub(super) fn byte_to_utf16_col(line: &str, byte_idx: usize) -> u32 {
    if line.is_ascii() {
        byte_idx as u32
    } else {
        line[..byte_idx.min(line.len())].encode_utf16().count() as u32
    }
}

/// Token for a capitalized name the index may not hold: the standard library's common traits and enums keep their
/// kind, any other name reads as a struct.
fn capitalized_token(name: &str) -> (u32, u32) {
    match name {
        "Clone"
        | "Copy"
        | "Debug"
        | "Display"
        | "Default"
        | "PartialEq"
        | "Eq"
        | "PartialOrd"
        | "Ord"
        | "Hash"
        | "Hasher"
        | "Iterator"
        | "IntoIterator"
        | "DoubleEndedIterator"
        | "ExactSizeIterator"
        | "Extend"
        | "FromIterator"
        | "From"
        | "Into"
        | "TryFrom"
        | "TryInto"
        | "AsRef"
        | "AsMut"
        | "Borrow"
        | "BorrowMut"
        | "ToOwned"
        | "ToString"
        | "FromStr"
        | "Deref"
        | "DerefMut"
        | "Drop"
        | "Fn"
        | "FnMut"
        | "FnOnce"
        | "Send"
        | "Sync"
        | "Sized"
        | "Unpin"
        | "Future"
        | "IntoFuture"
        | "Error"
        | "Write"
        | "Read"
        | "BufRead"
        | "Seek"
        | "Any"
        | "Add"
        | "Sub"
        | "Mul"
        | "Div"
        | "Rem"
        | "Neg"
        | "Not"
        | "Index"
        | "IndexMut"
        | "Termination" => (TOKEN_TYPE_INTERFACE, 0),
        "Option" | "Result" | "Cow" | "Ordering" | "Bound" | "Entry" | "Poll" | "ControlFlow" | "Infallible" => (TOKEN_TYPE_ENUM, 0),
        _ => (TOKEN_TYPE_STRUCT, 0),
    }
}

/// `T`, `U2`: a lone uppercase letter, optionally followed by digits, names a type parameter by convention.
fn looks_like_type_param(ident: &str) -> bool {
    let b = ident.as_bytes();
    b[0].is_ascii_uppercase() && b[1..].iter().all(u8::is_ascii_digit)
}

/// Lexical class of an identifier from its neighbours, or `None` when the scan cannot tell.
fn classify_ident(ident: &str, after: &str, prev_word: &str, prev_dot: bool, fallback_variable: bool) -> Option<(u32, u32)> {
    let upper = ident.bytes().next().is_some_and(|b| b.is_ascii_uppercase());
    if after.starts_with('!') && !after.starts_with("!=") {
        Some((TOKEN_TYPE_MACRO, 0))
    } else if after.starts_with("::") {
        Some(if !upper {
            (TOKEN_TYPE_NAMESPACE, 0)
        } else if looks_like_type_param(ident) {
            (TOKEN_TYPE_TYPE_PARAMETER, 0)
        } else {
            capitalized_token(ident)
        })
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
    } else if upper && looks_like_type_param(ident) {
        Some((TOKEN_TYPE_TYPE_PARAMETER, 0))
    } else if upper {
        Some(capitalized_token(ident))
    } else if after.starts_with('(') {
        Some((if prev_dot { TOKEN_TYPE_METHOD } else { TOKEN_TYPE_FUNCTION }, 0))
    } else if fallback_variable {
        Some((TOKEN_TYPE_VARIABLE, 0))
    } else {
        None
    }
}

/// Classifies the identifiers of one line of code from byte `from` on. `scan` is the line to read (the line itself,
/// or a copy with comments and literals blanked, of the same byte length); `line` gives the UTF-16 columns.
/// `in_block_comment` carries `/* … */` state across lines; `fallback_variable` types otherwise unclassified
/// identifiers as variables. A macro call's `!` is its own `macro` token, as in rust-analyzer.
pub(super) fn scan_code_line(line: &str, scan: &str, from: usize, line_idx: u32, in_block_comment: &mut bool, fallback_variable: bool, tokens: &mut Vec<RawToken>) {
    let bytes = scan.as_bytes();
    let n = bytes.len();
    let mut i = from;
    let mut prev_word = "";
    let mut prev_dot = false;
    let mut push = |start: usize, end: usize, (tt, tm): (u32, u32)| {
        let col = byte_to_utf16_col(line, start);
        let length = byte_to_utf16_col(line, end).saturating_sub(col);
        if length > 0 {
            tokens.push(RawToken { line: line_idx, col, length, token_type: tt, token_modifiers: tm });
        }
    };

    while i < n {
        if *in_block_comment {
            if i + 1 < n && bytes[i] == b'*' && bytes[i + 1] == b'/' {
                *in_block_comment = false;
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
            *in_block_comment = true;
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
            let ident = &scan[start..i];
            if !is_rust_kw_or_prim(ident) {
                let after = scan[i..].trim_start();
                // `Self::Output`, `S::Error`: a capitalized name under `Self` or a type parameter is an associated type.
                let qualifier = scan[..start].strip_suffix("::").map(|p| p.rsplit(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).next().unwrap_or(""));
                let assoc_ty =
                    ident.as_bytes()[0].is_ascii_uppercase() && !after.starts_with('(') && qualifier.is_some_and(|q| q == "Self" || (!q.is_empty() && looks_like_type_param(q)));
                let class = if assoc_ty { Some((TOKEN_TYPE_TYPE, 0)) } else { classify_ident(ident, after, prev_word, prev_dot, fallback_variable) };
                if let Some(tok) = class {
                    push(start, i, tok);
                    if tok.0 == TOKEN_TYPE_MACRO {
                        let bang = n - after.len();
                        push(bang, bang + 1, (TOKEN_TYPE_MACRO, 0));
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

/// Lexical tokens for `text`: detached-parse symbols when the index has none for the file, then a classifying scan
/// of every code line.
pub fn scan_source_tokens(text: &str, use_detached_symbols: bool, has_compiled_tokens: bool, tokens: &mut Vec<RawToken>) {
    if use_detached_symbols && !has_compiled_tokens {
        let detached = DocumentOverlay::parse_detached(text.to_string());
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
    // Comments and literals, multi-line raw strings included, are blanked so their words are not read as code.
    let masked = crate::lsp::editing::action_parse::mask_comments_and_strings(text);
    let mut in_block_comment = false;
    for (line_idx, (line, scan)) in text.lines().zip(masked.lines()).enumerate() {
        if line.len() == scan.len() {
            scan_code_line(line, scan, 0, line_idx as u32, &mut in_block_comment, !has_compiled_tokens, tokens);
        }
    }
}
