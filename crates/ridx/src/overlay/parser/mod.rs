/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Lexical scope tree and error-tolerant pure-Rust AST parser for document overlay.

pub mod items;
pub(crate) mod trivia;

pub use super::OverlaySymbol;
use ridx_format::DefKind;

#[derive(Debug, Clone)]
pub(crate) struct Scope {
    pub(crate) id: usize,
    pub(crate) parent: Option<usize>,
    pub(crate) start_byte: usize,
    pub(crate) end_byte: usize,
    pub(crate) sel_start_byte: usize,
    pub(crate) body_start_byte: Option<usize>,
}

#[inline]
pub fn is_rust_ident_start(c: char) -> bool {
    c == '_' || c.is_alphabetic()
}

#[inline]
pub fn is_rust_ident_continue(c: char) -> bool {
    c == '_' || c.is_alphanumeric()
}

pub fn is_ident(s: &str) -> bool {
    let s = s.strip_prefix("r#").unwrap_or(s);
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if is_rust_ident_start(c) => chars.all(is_rust_ident_continue),
        _ => false,
    }
}

pub(crate) fn skip_string_or_char_literal(text: &str, pos: &mut usize) -> bool {
    let bytes = text.as_bytes();
    let len = bytes.len();
    if *pos >= len {
        return false;
    }
    let b = bytes[*pos];

    // Raw string r"..." / r#"..."# or byte/C raw string br"..." / cr"..."
    let raw_start = if b == b'r' && *pos + 1 < len && (bytes[*pos + 1] == b'"' || bytes[*pos + 1] == b'#') {
        Some(*pos + 1)
    } else if (b == b'b' || b == b'c') && *pos + 2 < len && bytes[*pos + 1] == b'r' && (bytes[*pos + 2] == b'"' || bytes[*pos + 2] == b'#') {
        Some(*pos + 2)
    } else {
        None
    };
    if let Some(mut h_idx) = raw_start {
        let mut hashes = 0usize;
        while h_idx < len && bytes[h_idx] == b'#' {
            hashes += 1;
            h_idx += 1;
        }
        if h_idx < len && bytes[h_idx] == b'"' {
            let mut i = h_idx + 1;
            while i < len {
                if bytes[i] == b'"' && i + hashes < len && (0..hashes).all(|k| bytes[i + 1 + k] == b'#') {
                    *pos = i + 1 + hashes;
                    return true;
                }
                i += 1;
            }
            *pos = len;
            return true;
        }
    }

    if b == b'"' {
        let mut i = *pos + 1;
        while i < len && bytes[i] != b'"' {
            if bytes[i] == b'\\' && i + 1 < len {
                i += 2;
            } else {
                i += 1;
            }
        }
        *pos = (i + 1).min(len);
        return true;
    }

    if b == b'\'' {
        let j = *pos + 1;
        if j + 1 < len && bytes[j] == b'\\' && bytes[j + 1] != b'\n' && bytes[j + 1] != b'\r' {
            let esc_len = text[j + 1..].chars().next().map_or(1, char::len_utf8);
            let mut k = j + 1 + esc_len;
            while k < len && bytes[k] != b'\'' && bytes[k] != b'\n' && bytes[k] != b'\r' {
                k += 1;
            }
            if k < len && bytes[k] == b'\'' {
                *pos = k + 1;
                return true;
            }
        } else if let Some(ch) = text[j..].chars().next() {
            if ch != '\'' && ch != '\\' && ch != '\n' && ch != '\r' {
                let end_quote = j + ch.len_utf8();
                if end_quote < len && bytes[end_quote] == b'\'' {
                    *pos = end_quote + 1;
                    return true;
                }
            }
        }

        // Lifetime `'ident` / `'r#ident` or standalone `'`
        let mut k = if text[j..].starts_with("r#") { j + 2 } else { j };
        if let Some(first_char) = text[k..].chars().next() {
            if is_rust_ident_start(first_char) {
                k += first_char.len_utf8();
                while let Some(c) = text[k..].chars().next() {
                    if is_rust_ident_continue(c) {
                        k += c.len_utf8();
                    } else {
                        break;
                    }
                }
                *pos = k;
                return true;
            }
        }
        *pos = j;
        return true;
    }

    false
}

pub(crate) struct OverlayParser<'a> {
    pub(crate) text: &'a str,
    line_starts: &'a [usize],
    pub(crate) scopes: Vec<Scope>,
    pub(crate) symbols: Vec<OverlaySymbol>,
    pub(crate) pending_fn_scope: Option<usize>,
    /// Comment and string/char literal spans in text order, recorded by the main parse loop.
    pub(crate) opaque: Vec<(usize, usize)>,
}

impl<'a> OverlayParser<'a> {
    pub(crate) fn new(text: &'a str, line_starts: &'a [usize]) -> Self {
        Self { text, line_starts, scopes: Vec::new(), symbols: Vec::new(), pending_fn_scope: None, opaque: Vec::new() }
    }

    pub(crate) fn byte_to_char_line_col(&self, byte_offset: usize) -> (u32, u32) {
        let b = byte_offset.min(self.text.len());
        let line_idx = match self.line_starts.binary_search(&b) {
            Ok(idx) => idx,
            Err(idx) => idx.saturating_sub(1),
        };
        let line = (line_idx + 1) as u32;
        let line_start_byte = self.line_starts[line_idx];
        let line_prefix = &self.text[line_start_byte..b];
        let char_col = (line_prefix.chars().count() + 1) as u32;
        (line, char_col)
    }

    pub(crate) fn parse(&mut self) {
        let bytes = self.text.as_bytes();
        let len = bytes.len();
        let mut pos = 0;

        // Root scope
        self.scopes.push(Scope { id: 0, parent: None, start_byte: 0, end_byte: len, sel_start_byte: 0, body_start_byte: None });

        let mut scope_stack = vec![0];

        while pos < len {
            let prev = pos;
            self.skip_whitespace_and_comments(&mut pos);
            if pos != prev {
                self.record_comment_spans(prev, pos);
                continue;
            }
            let b = bytes[pos];

            // Raw string, string, char literal, or lifetime `'ident`
            if skip_string_or_char_literal(self.text, &mut pos) {
                self.record_literal_span(prev, pos);
                continue;
            }

            // Scopes { ... }
            if b == b'{' {
                let sid = if let Some(fn_sid) = self.pending_fn_scope.take() {
                    if fn_sid < self.scopes.len() {
                        self.scopes[fn_sid].body_start_byte = Some(pos);
                    }
                    fn_sid
                } else {
                    let new_id = self.scopes.len();
                    let parent = scope_stack.last().copied();
                    self.scopes.push(Scope { id: new_id, parent, start_byte: pos, end_byte: len, sel_start_byte: pos, body_start_byte: None });
                    new_id
                };
                scope_stack.push(sid);
                pos += 1;
                continue;
            }

            if b == b'}' {
                if scope_stack.len() > 1 {
                    if let Some(id) = scope_stack.pop() {
                        if id < self.scopes.len() {
                            self.scopes[id].end_byte = pos + 1;
                        }
                    }
                }
                pos += 1;
                continue;
            }

            // Identifiers / keywords
            if let Some(first_char) = self.text[pos..].chars().next() {
                let is_raw = self.text[pos..].starts_with("r#");
                if is_raw || is_rust_ident_start(first_char) {
                    let start = pos;
                    if is_raw {
                        pos += 2;
                    } else {
                        pos += first_char.len_utf8();
                    }
                    while pos < len {
                        let c = self.text[pos..].chars().next().unwrap();
                        if is_rust_ident_continue(c) {
                            pos += c.len_utf8();
                        } else {
                            break;
                        }
                    }
                    let token = &self.text[start..pos];
                    let current_scope = scope_stack.last().copied().unwrap_or(0);

                    match token {
                        "fn" => {
                            self.parse_fn_item(&mut pos, current_scope);
                        }
                        "struct" => {
                            self.parse_named_item(&mut pos, DefKind::Struct, current_scope);
                        }
                        "union" => {
                            self.parse_named_item(&mut pos, DefKind::Union, current_scope);
                        }
                        "enum" => {
                            self.parse_named_item(&mut pos, DefKind::Enum, current_scope);
                        }
                        "trait" => {
                            self.parse_named_item(&mut pos, DefKind::Trait, current_scope);
                        }
                        "type" => {
                            self.parse_named_item(&mut pos, DefKind::TyAlias, current_scope);
                        }
                        "const" => {
                            let mut check_pos = pos;
                            if let Some((next_tok, _, _)) = self.parse_ident(&mut check_pos) {
                                if next_tok == "fn" {
                                    pos = check_pos;
                                    self.parse_fn_item(&mut pos, current_scope);
                                    continue;
                                }
                            }
                            let prev_non_ws = self.text[..start].bytes().rev().find(|b| !b.is_ascii_whitespace());
                            if !matches!(prev_non_ws, Some(b'*' | b'<' | b',')) {
                                self.parse_named_item(&mut pos, DefKind::Const, current_scope);
                            }
                        }
                        "static" => {
                            self.parse_named_item(&mut pos, DefKind::Static, current_scope);
                        }
                        "mod" => {
                            self.parse_named_item(&mut pos, DefKind::Mod, current_scope);
                        }
                        "let" => {
                            self.parse_let_binding(&mut pos, current_scope);
                        }
                        _ => {}
                    }
                    continue;
                }
                pos += first_char.len_utf8();
            }
        }
    }

    pub(crate) fn skip_whitespace_and_comments(&self, pos: &mut usize) {
        let bytes = self.text.as_bytes();
        let len = bytes.len();
        while *pos < len {
            let b = bytes[*pos];
            if b.is_ascii_whitespace() {
                *pos += 1;
            } else if b == b'/' && *pos + 1 < len && bytes[*pos + 1] == b'/' {
                *pos += 2;
                while *pos < len && bytes[*pos] != b'\n' {
                    *pos += 1;
                }
            } else if b == b'/' && *pos + 1 < len && bytes[*pos + 1] == b'*' {
                let mut depth = 1usize;
                *pos += 2;
                while *pos + 1 < len {
                    if bytes[*pos] == b'/' && bytes[*pos + 1] == b'*' {
                        depth += 1;
                        *pos += 2;
                    } else if bytes[*pos] == b'*' && bytes[*pos + 1] == b'/' {
                        depth -= 1;
                        *pos += 2;
                        if depth == 0 {
                            break;
                        }
                    } else {
                        *pos += 1;
                    }
                }
                if depth > 0 {
                    *pos = len;
                }
            } else {
                break;
            }
        }
    }

    pub(crate) fn parse_ident(&self, pos: &mut usize) -> Option<(&'a str, usize, usize)> {
        self.skip_whitespace_and_comments(pos);
        if *pos >= self.text.len() {
            return None;
        }
        let is_raw = self.text[*pos..].starts_with("r#");
        let first_char = self.text[*pos..].chars().next()?;
        if !is_raw && !is_rust_ident_start(first_char) {
            return None;
        }
        let start = *pos;
        let mut cur = if is_raw { *pos + 2 } else { *pos + first_char.len_utf8() };
        while cur < self.text.len() {
            let c = self.text[cur..].chars().next().unwrap();
            if is_rust_ident_continue(c) {
                cur += c.len_utf8();
            } else {
                break;
            }
        }
        *pos = cur;
        Some((&self.text[start..cur], start, cur))
    }
}
