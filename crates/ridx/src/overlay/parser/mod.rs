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

pub use super::OverlaySymbol;
use ridx_format::DefKind;

#[derive(Debug, Clone)]
pub(crate) struct Scope {
    pub(crate) id: usize,
    pub(crate) parent: Option<usize>,
    pub(crate) start_byte: usize,
    pub(crate) end_byte: usize,
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

pub(crate) struct OverlayParser<'a> {
    pub(crate) text: &'a str,
    line_starts: &'a [usize],
    pub(crate) scopes: Vec<Scope>,
    pub(crate) symbols: Vec<OverlaySymbol>,
    pub(crate) pending_fn_scope: Option<usize>,
}

impl<'a> OverlayParser<'a> {
    pub(crate) fn new(text: &'a str, line_starts: &'a [usize]) -> Self {
        Self {
            text,
            line_starts,
            scopes: Vec::new(),
            symbols: Vec::new(),
            pending_fn_scope: None,
        }
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
        self.scopes.push(Scope {
            id: 0,
            parent: None,
            start_byte: 0,
            end_byte: len,
        });

        let mut scope_stack = vec![0];

        while pos < len {
            // Skip whitespace and comments
            let b = bytes[pos];
            if b.is_ascii_whitespace() {
                pos += 1;
                continue;
            }

            // Line comment //
            if b == b'/' && pos + 1 < len && bytes[pos + 1] == b'/' {
                pos += 2;
                while pos < len && bytes[pos] != b'\n' {
                    pos += 1;
                }
                continue;
            }

            // Block comment /* ... */
            if b == b'/' && pos + 1 < len && bytes[pos + 1] == b'*' {
                let mut depth = 1usize;
                pos += 2;
                while pos + 1 < len {
                    if bytes[pos] == b'/' && bytes[pos + 1] == b'*' {
                        depth += 1;
                        pos += 2;
                    } else if bytes[pos] == b'*' && bytes[pos + 1] == b'/' {
                        depth -= 1;
                        pos += 2;
                        if depth == 0 {
                            break;
                        }
                    } else {
                        pos += 1;
                    }
                }
                if depth > 0 {
                    pos = len;
                }
                continue;
            }

            // String or char literal
            if b == b'"' || b == b'\'' {
                let quote = b;
                pos += 1;
                while pos < len && bytes[pos] != quote {
                    if bytes[pos] == b'\\' && pos + 1 < len {
                        pos += 2;
                    } else {
                        pos += 1;
                    }
                }
                if pos < len {
                    pos += 1;
                }
                continue;
            }

            // Scopes { ... }
            if b == b'{' {
                let sid = if let Some(fn_sid) = self.pending_fn_scope.take() {
                    fn_sid
                } else {
                    let new_id = self.scopes.len();
                    let parent = scope_stack.last().copied();
                    self.scopes.push(Scope {
                        id: new_id,
                        parent,
                        start_byte: pos,
                        end_byte: len,
                    });
                    new_id
                };
                scope_stack.push(sid);
                pos += 1;
                continue;
            }

            if b == b'}' {
                if let Some(id) = scope_stack.pop() {
                    if id < self.scopes.len() {
                        self.scopes[id].end_byte = pos + 1;
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
                            self.parse_named_item(&mut pos, DefKind::Const, current_scope);
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
        let mut cur = if is_raw {
            *pos + 2
        } else {
            *pos + first_char.len_utf8()
        };
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
