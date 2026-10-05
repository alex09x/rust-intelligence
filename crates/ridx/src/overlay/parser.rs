//! Lexical scope tree and error-tolerant pure-Rust AST parser for document overlay.

use super::OverlaySymbol;
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
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if is_rust_ident_start(c) => chars.all(is_rust_ident_continue),
        _ => false,
    }
}

pub(crate) struct OverlayParser<'a> {
    text: &'a str,
    line_starts: &'a [usize],
    pub(crate) scopes: Vec<Scope>,
    pub(crate) symbols: Vec<OverlaySymbol>,
    pending_fn_scope: Option<usize>,
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

    fn byte_to_char_line_col(&self, byte_offset: usize) -> (u32, u32) {
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
                if is_rust_ident_start(first_char) {
                    let start = pos;
                    pos += first_char.len_utf8();
                    while pos < len {
                        let c = match self.text[pos..].chars().next() {
                            Some(c) => c,
                            None => break,
                        };
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
                continue;
            }

            pos += 1;
        }
    }

    fn skip_whitespace_and_comments(&self, pos: &mut usize) {
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

    fn parse_ident(&self, pos: &mut usize) -> Option<(&'a str, usize, usize)> {
        self.skip_whitespace_and_comments(pos);
        if *pos >= self.text.len() {
            return None;
        }
        let first_char = self.text[*pos..].chars().next()?;
        if !is_rust_ident_start(first_char) {
            return None;
        }
        let start = *pos;
        let mut cur = *pos + first_char.len_utf8();
        while cur < self.text.len() {
            let c = match self.text[cur..].chars().next() {
                Some(c) => c,
                None => break,
            };
            if is_rust_ident_continue(c) {
                cur += c.len_utf8();
            } else {
                break;
            }
        }
        *pos = cur;
        Some((&self.text[start..cur], start, cur))
    }

    fn parse_fn_item(&mut self, pos: &mut usize, current_scope: usize) {
        let Some((name, start, end)) = self.parse_ident(pos) else {
            return;
        };
        let (s_line, s_col) = self.byte_to_char_line_col(start);
        let (e_line, e_col) = self.byte_to_char_line_col(end);

        // Find signature up to `{` or `;`
        let sig_start = start;
        let bytes = self.text.as_bytes();
        let len = bytes.len();
        while *pos < len && bytes[*pos] != b'{' && bytes[*pos] != b';' {
            *pos += 1;
        }
        let sig_end = (*pos).min(len);
        let raw_sig = self.text[sig_start..sig_end].trim();
        let signature = if !raw_sig.is_empty() {
            Some(format!("fn {}", raw_sig))
        } else {
            None
        };

        // Add function symbol to enclosing outer scope
        self.symbols.push(OverlaySymbol {
            name: name.to_string(),
            kind: DefKind::Fn,
            signature,
            start_line: s_line,
            start_col: s_col,
            end_line: e_line,
            end_col: e_col,
            scope_id: current_scope,
            decl_start_byte: start,
            decl_end_byte: end,
            scope_start_byte: 0,
        });

        // Allocate a dedicated scope for this function's parameters and body
        let fn_scope_id = self.scopes.len();
        self.scopes.push(Scope {
            id: fn_scope_id,
            parent: Some(current_scope),
            start_byte: start,
            end_byte: len,
        });

        // Parse parameters inside `(...)` and assign them to fn_scope_id
        if let Some(open_paren) = self.text[start..sig_end].find('(') {
            if let Some(close_paren) = self.text[start + open_paren..sig_end].find(')') {
                let params_text =
                    self.text[start + open_paren + 1..start + open_paren + close_paren].to_string();
                self.extract_params(&params_text, start + open_paren + 1, fn_scope_id);
            }
        }

        // If followed by `{`, register pending_fn_scope so the body block reuses this scope
        if *pos < len && bytes[*pos] == b'{' {
            self.pending_fn_scope = Some(fn_scope_id);
        } else if *pos < len && bytes[*pos] == b';' {
            // Trait fn declaration without body
            self.scopes[fn_scope_id].end_byte = *pos + 1;
        }
    }

    fn extract_params(&mut self, params_text: &str, base_byte: usize, scope_id: usize) {
        for part in params_text.split(',') {
            let trimmed = part.trim();
            if trimmed.is_empty()
                || trimmed == "self"
                || trimmed == "&self"
                || trimmed == "&mut self"
            {
                continue;
            }
            if let Some((p_name, _)) = trimmed.split_once(':') {
                let mut p_name = p_name.trim();
                if let Some(after) = p_name.strip_prefix("mut") {
                    if after.chars().next().is_some_and(|c| c.is_whitespace()) {
                        p_name = after.trim();
                    }
                }
                if !p_name.is_empty() && is_ident(p_name) {
                    if let Some(offset) = part.find(p_name) {
                        let abs_offset = base_byte
                            + (part.as_ptr() as usize - params_text.as_ptr() as usize)
                            + offset;
                        let (s_line, s_col) = self.byte_to_char_line_col(abs_offset);
                        let (e_line, e_col) = self.byte_to_char_line_col(abs_offset + p_name.len());
                        self.symbols.push(OverlaySymbol {
                            name: p_name.to_string(),
                            kind: DefKind::Local,
                            signature: Some(trimmed.to_string()),
                            start_line: s_line,
                            start_col: s_col,
                            end_line: e_line,
                            end_col: e_col,
                            scope_id,
                            decl_start_byte: abs_offset,
                            decl_end_byte: abs_offset + p_name.len(),
                            scope_start_byte: base_byte,
                        });
                    }
                }
            }
        }
    }

    fn parse_named_item(&mut self, pos: &mut usize, kind: DefKind, current_scope: usize) {
        let Some((name, start, end)) = self.parse_ident(pos) else {
            return;
        };
        let (s_line, s_col) = self.byte_to_char_line_col(start);
        let (e_line, e_col) = self.byte_to_char_line_col(end);

        self.symbols.push(OverlaySymbol {
            name: name.to_string(),
            kind,
            signature: None,
            start_line: s_line,
            start_col: s_col,
            end_line: e_line,
            end_col: e_col,
            scope_id: current_scope,
            decl_start_byte: start,
            decl_end_byte: end,
            scope_start_byte: 0,
        });
    }

    fn parse_let_binding(&mut self, pos: &mut usize, current_scope: usize) {
        let decl_start = *pos;
        self.skip_whitespace_and_comments(pos);
        // Optional `mut` keyword (byte-safe UTF-8 check)
        let remainder = &self.text[*pos..];
        if let Some(after_mut) = remainder.strip_prefix("mut") {
            if after_mut.chars().next().is_some_and(|c| c.is_whitespace()) {
                *pos += 3;
                self.skip_whitespace_and_comments(pos);
            }
        }

        let Some((name, start, end)) = self.parse_ident(pos) else {
            return;
        };
        let (s_line, s_col) = self.byte_to_char_line_col(start);
        let (e_line, e_col) = self.byte_to_char_line_col(end);

        // Scan ahead to find the end of the entire let statement (';') or enclosing block end ('}').
        // In Rust, a let binding's scope starts AFTER its entire initializer statement.
        let bytes = self.text.as_bytes();
        let len = bytes.len();
        let mut cur = *pos;
        let mut brace_depth: usize = 0;
        let mut paren_depth: usize = 0;
        let mut bracket_depth: usize = 0;

        while cur < len {
            let b = bytes[cur];
            // Comments
            if b == b'/' && cur + 1 < len && bytes[cur + 1] == b'/' {
                cur += 2;
                while cur < len && bytes[cur] != b'\n' {
                    cur += 1;
                }
                continue;
            }
            if b == b'/' && cur + 1 < len && bytes[cur + 1] == b'*' {
                let mut depth = 1usize;
                cur += 2;
                while cur + 1 < len {
                    if bytes[cur] == b'/' && bytes[cur + 1] == b'*' {
                        depth += 1;
                        cur += 2;
                    } else if bytes[cur] == b'*' && bytes[cur + 1] == b'/' {
                        depth -= 1;
                        cur += 2;
                        if depth == 0 {
                            break;
                        }
                    } else {
                        cur += 1;
                    }
                }
                if depth > 0 {
                    cur = len;
                }
                continue;
            }
            // Strings and chars
            if b == b'"' || b == b'\'' {
                let quote = b;
                cur += 1;
                while cur < len && bytes[cur] != quote {
                    if bytes[cur] == b'\\' && cur + 1 < len {
                        cur += 2;
                    } else {
                        cur += 1;
                    }
                }
                if cur < len {
                    cur += 1;
                }
                continue;
            }
            // Delimiters
            if b == b'{' {
                brace_depth += 1;
                cur += 1;
                continue;
            }
            if b == b'}' {
                if brace_depth > 0 {
                    brace_depth -= 1;
                    cur += 1;
                    continue;
                } else {
                    // Closing brace of enclosing block: ends the let statement
                    break;
                }
            }
            if b == b'(' {
                paren_depth += 1;
                cur += 1;
                continue;
            }
            if b == b')' {
                paren_depth = paren_depth.saturating_sub(1);
                cur += 1;
                continue;
            }
            if b == b'[' {
                bracket_depth += 1;
                cur += 1;
                continue;
            }
            if b == b']' {
                bracket_depth = bracket_depth.saturating_sub(1);
                cur += 1;
                continue;
            }
            if b == b';' {
                if brace_depth == 0 && paren_depth == 0 && bracket_depth == 0 {
                    cur += 1;
                    break;
                }
                cur += 1;
                continue;
            }
            cur += 1;
        }

        let scope_start = cur;

        self.symbols.push(OverlaySymbol {
            name: name.to_string(),
            kind: DefKind::Local,
            signature: None,
            start_line: s_line,
            start_col: s_col,
            end_line: e_line,
            end_col: e_col,
            scope_id: current_scope,
            decl_start_byte: decl_start,
            decl_end_byte: end,
            scope_start_byte: scope_start,
        });
    }
}
