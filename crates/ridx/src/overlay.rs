//! Fast in-memory document overlay for live buffer indexing (Variant B / As-You-Type).
//!
//! Maintains an error-tolerant, pure-Rust AST and lexical scope index for open,
//! unsaved editor buffers. Resolves local variables, function parameters, and newly
//! declared items within milliseconds, even when the buffer contains incomplete syntax
//! or compilation errors.

use ridx_format::{fnv1a_64, DefKind};
use std::collections::HashMap;

/// Representation of a symbol in the live document overlay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlaySymbol {
    pub name: String,
    pub kind: DefKind,
    pub signature: Option<String>,
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
    pub scope_id: usize,
    pub decl_start_byte: usize,
    pub decl_end_byte: usize,
    pub scope_start_byte: usize,
}

/// Representation of a resolved definition within the live buffer overlay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlayDefinition {
    pub name: String,
    pub kind: DefKind,
    pub signature: Option<String>,
    pub file: String,
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
}

#[derive(Debug, Clone)]
struct Scope {
    id: usize,
    parent: Option<usize>,
    start_byte: usize,
    end_byte: usize,
}

/// In-memory overlay for a single active document buffer.
pub struct DocumentOverlay {
    #[allow(dead_code)]
    pub uri: String,
    pub file_path: String,
    pub version: i32,
    pub content_hash: u64,
    pub text: String,
    pub compiled_text: Option<String>,
    pub disk_text: Option<String>,
    line_starts: Vec<usize>,
    symbols: Vec<OverlaySymbol>,
    scopes: Vec<Scope>,
}

impl DocumentOverlay {
    /// Creates and parses a new DocumentOverlay from buffer text.
    pub fn new(uri: impl Into<String>, file_path: impl Into<String>, version: i32, text: String) -> Self {
        let uri = uri.into();
        let file_path = file_path.into();
        let content_hash = fnv1a_64(text.as_bytes());
        let disk_text = std::fs::read_to_string(&file_path).ok();
        let compiled_text = disk_text.clone().or_else(|| Some(text.clone()));
        let mut overlay = Self {
            uri,
            file_path,
            version,
            content_hash,
            text,
            compiled_text,
            disk_text,
            line_starts: Vec::new(),
            symbols: Vec::new(),
            scopes: Vec::new(),
        };
        overlay.reparse();
        overlay
    }

    /// Updates buffer text, version, and reparses symbols.
    pub fn update(&mut self, version: i32, text: String) {
        self.version = version;
        self.content_hash = fnv1a_64(text.as_bytes());
        self.text = text;
        self.reparse();
    }

    /// Whether this buffer has unsaved or unindexed edits relative to compiled index.
    #[allow(dead_code)]
    pub fn is_dirty(&self) -> bool {
        self.compiled_text.as_deref() != Some(&self.text)
    }

    /// Whether this buffer matches the content currently saved on disk.
    #[allow(dead_code)]
    pub fn is_synced_with_disk(&self) -> bool {
        self.disk_text.as_deref() == Some(&self.text)
    }

    /// Advances the compiled index baseline when a newly built index is published.
    pub fn advance_compiled_baseline(&mut self, text: String) {
        self.compiled_text = Some(text);
    }

    /// Maps a 1-based line number from the live overlay buffer to the corresponding
    /// 1-based line number in the compiled disk snapshot.
    ///
    /// Returns `Some(compiled_line)` if the line is in a proven unchanged region,
    /// or `None` if the line falls in an edited, inserted, or modified region.
    #[allow(dead_code)]
    pub fn map_line_to_compiled(&self, line: u32) -> Option<u32> {
        if self.compiled_text.as_deref() == Some(&self.text) {
            return Some(line);
        }
        let compiled_text = self.compiled_text.as_ref()?;

        let curr_lines: Vec<&str> = self.text.lines().collect();
        let compiled_lines: Vec<&str> = compiled_text.lines().collect();

        let l_idx = (line.saturating_sub(1)) as usize;
        if l_idx >= curr_lines.len() {
            return None;
        }

        // Count common prefix lines
        let mut prefix_common = 0;
        while prefix_common < curr_lines.len()
            && prefix_common < compiled_lines.len()
            && curr_lines[prefix_common] == compiled_lines[prefix_common]
        {
            prefix_common += 1;
        }

        // If line is in proven unchanged prefix
        if l_idx < prefix_common {
            return Some(line);
        }

        // Count common suffix lines
        let mut suffix_common = 0;
        while suffix_common < curr_lines.len()
            && suffix_common < compiled_lines.len()
            && curr_lines[curr_lines.len() - 1 - suffix_common]
                == compiled_lines[compiled_lines.len() - 1 - suffix_common]
        {
            suffix_common += 1;
        }

        // If line is in proven unchanged suffix
        if l_idx >= curr_lines.len().saturating_sub(suffix_common) {
            let offset_from_end = curr_lines.len() - 1 - l_idx;
            if offset_from_end < compiled_lines.len() {
                let compiled_line_idx = compiled_lines.len() - 1 - offset_from_end;
                return Some((compiled_line_idx + 1) as u32);
            }
        }

        // The line is in the modified/inserted region
        None
    }

    /// Maps a 1-based line number from the compiled index snapshot to the corresponding
    /// 1-based line number in the current live overlay buffer.
    ///
    /// Returns `Some(live_line)` if the line is in a proven unchanged region,
    /// or `None` if the line was deleted, replaced, or modified by unsaved edits.
    pub fn map_compiled_line_to_live(&self, compiled_line: u32) -> Option<u32> {
        if self.compiled_text.as_deref() == Some(&self.text) {
            return Some(compiled_line);
        }
        let compiled_text = self.compiled_text.as_ref()?;

        let curr_lines: Vec<&str> = self.text.lines().collect();
        let compiled_lines: Vec<&str> = compiled_text.lines().collect();

        let c_idx = (compiled_line.saturating_sub(1)) as usize;
        if c_idx >= compiled_lines.len() {
            return None;
        }

        // Count common prefix lines
        let mut prefix_common = 0;
        while prefix_common < curr_lines.len()
            && prefix_common < compiled_lines.len()
            && curr_lines[prefix_common] == compiled_lines[prefix_common]
        {
            prefix_common += 1;
        }

        if c_idx < prefix_common {
            return Some(compiled_line);
        }

        // Count common suffix lines
        let mut suffix_common = 0;
        while suffix_common < curr_lines.len()
            && suffix_common < compiled_lines.len()
            && curr_lines[curr_lines.len() - 1 - suffix_common]
                == compiled_lines[compiled_lines.len() - 1 - suffix_common]
        {
            suffix_common += 1;
        }

        if c_idx >= compiled_lines.len().saturating_sub(suffix_common) {
            let offset_from_end = compiled_lines.len() - 1 - c_idx;
            if offset_from_end < curr_lines.len() {
                let live_line_idx = curr_lines.len() - 1 - offset_from_end;
                return Some((live_line_idx + 1) as u32);
            }
        }

        None
    }

    /// Converts a 0-based UTF-16 character column on a 1-based line to a 1-based Unicode character column.
    pub fn utf16_col_to_char_col(&self, line_1based: u32, utf16_col_0based: u32) -> u32 {
        let line_idx = line_1based.saturating_sub(1) as usize;
        if line_idx >= self.line_starts.len() {
            return utf16_col_0based + 1;
        }
        let start = self.line_starts[line_idx];
        let end = if line_idx + 1 < self.line_starts.len() {
            self.line_starts[line_idx + 1]
        } else {
            self.text.len()
        };
        let line_str = &self.text[start..end];

        let mut utf16_count = 0;
        let mut char_count = 0;
        for c in line_str.chars() {
            if utf16_count >= utf16_col_0based {
                break;
            }
            utf16_count += c.len_utf16() as u32;
            char_count += 1;
        }
        char_count + 1
    }

    /// Converts a 1-based Unicode character column on a 1-based line to a 0-based UTF-16 character column.
    pub fn char_col_to_utf16_col(&self, line_1based: u32, char_col_1based: u32) -> u32 {
        let line_idx = line_1based.saturating_sub(1) as usize;
        if line_idx >= self.line_starts.len() {
            return char_col_1based.saturating_sub(1);
        }
        let start = self.line_starts[line_idx];
        let end = if line_idx + 1 < self.line_starts.len() {
            self.line_starts[line_idx + 1]
        } else {
            self.text.len()
        };
        let line_str = &self.text[start..end];

        let target_char = char_col_1based.saturating_sub(1);
        let mut utf16_count = 0;
        for (i, c) in line_str.chars().enumerate() {
            if (i as u32) >= target_char {
                break;
            }
            utf16_count += c.len_utf16() as u32;
        }
        utf16_count
    }

    /// Converts a byte offset to 1-based line and 1-based character column.
    #[allow(dead_code)]
    pub fn byte_to_char_line_col(&self, byte_offset: usize) -> (u32, u32) {
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

    /// Converts 1-based line and 1-based character column to byte offset.
    pub fn line_char_col_to_byte(&self, line: u32, col: u32) -> usize {
        let line_idx = line.saturating_sub(1) as usize;
        if line_idx >= self.line_starts.len() {
            return self.text.len();
        }
        let start = self.line_starts[line_idx];
        let end = if line_idx + 1 < self.line_starts.len() {
            self.line_starts[line_idx + 1]
        } else {
            self.text.len()
        };
        let line_str = &self.text[start..end];

        let target_char = col.saturating_sub(1) as usize;
        let mut byte_offset = start;
        for (i, c) in line_str.chars().enumerate() {
            if i >= target_char {
                break;
            }
            byte_offset += c.len_utf8();
        }
        byte_offset
    }

    /// Re-indexes line boundaries, lexical scopes, and declarations.
    fn reparse(&mut self) {
        self.line_starts.clear();

        // 1. Build line index
        self.line_starts.push(0);
        for (i, &b) in self.text.as_bytes().iter().enumerate() {
            if b == b'\n' {
                self.line_starts.push(i + 1);
            }
        }

        // 2. Parse lexical scopes and symbols using error-tolerant token scanner
        let mut parser = OverlayParser::new(&self.text, &self.line_starts);
        parser.parse();
        self.scopes = parser.scopes;
        self.symbols = parser.symbols;
    }
}

fn strip_trailing_whitespace_and_comments(s: &str) -> &str {
    let mut slice = s;
    loop {
        let trimmed = slice.trim_end();
        if trimmed.ends_with("*/") {
            if let Some(start) = trimmed.rfind("/*") {
                slice = &trimmed[..start];
                continue;
            }
        }
        if let Some(last_nl) = trimmed.rfind('\n') {
            let last_line = &trimmed[last_nl + 1..];
            if let Some(comment_pos) = last_line.find("//") {
                let before = &last_line[..comment_pos];
                slice = &trimmed[..last_nl + 1 + before.len()];
                continue;
            }
        } else if let Some(comment_pos) = trimmed.find("//") {
            slice = &trimmed[..comment_pos];
            continue;
        }
        return trimmed;
    }
}

fn strip_leading_whitespace_and_comments(s: &str) -> &str {
    let mut slice = s;
    loop {
        let trimmed = slice.trim_start();
        if trimmed.starts_with("/*") {
            if let Some(end) = trimmed.find("*/") {
                slice = &trimmed[end + 2..];
                continue;
            }
        }
        if trimmed.starts_with("//") {
            if let Some(end) = trimmed.find('\n') {
                slice = &trimmed[end + 1..];
                continue;
            } else {
                return "";
            }
        }
        return trimmed;
    }
}

impl DocumentOverlay {
    /// Finds a symbol definition at a cursor location within the active buffer.
    pub fn find_definition_at(&self, line: u32, col: u32) -> Option<OverlayDefinition> {
        // 1. Direct hit on an existing overlay definition span
        for sym in &self.symbols {
            if line >= sym.start_line && line <= sym.end_line && col >= sym.start_col && col <= sym.end_col {
                return Some(OverlayDefinition {
                    name: sym.name.clone(),
                    kind: sym.kind,
                    signature: sym.signature.clone(),
                    file: self.file_path.clone(),
                    start_line: sym.start_line,
                    start_col: sym.start_col,
                    end_line: sym.end_line,
                    end_col: sym.end_col,
                });
            }
        }

        // 2. Identifier at cursor location -> resolve to visible binding in lexical scope
        let (ident, start_b, end_b) = self.get_ident_and_span_at(line, col)?;
        let byte_pos = self.line_char_col_to_byte(line, col);

        let line_idx = line.saturating_sub(1) as usize;
        if line_idx >= self.line_starts.len() {
            return None;
        }
        let line_start = self.line_starts[line_idx];
        let ident_start = line_start + start_b;
        let ident_end = line_start + end_b;

        // If the token is part of a qualified path (e.g. `foo::bar` or `foo.bar` or `bar::baz`),
        // across newlines or comments, the single-file lexical overlay lacks path/type semantics:
        // abstain (return None) so the compiled index can resolve it accurately.
        let prefix = strip_trailing_whitespace_and_comments(&self.text[..ident_start]);
        if prefix.ends_with("::") || prefix.ends_with('.') {
            return None;
        }
        let suffix = strip_leading_whitespace_and_comments(&self.text[ident_end..]);
        if suffix.starts_with("::") {
            return None;
        }

        // Find innermost enclosing scope
        let mut best_scope = 0;
        let mut min_scope_len = usize::MAX;
        for s in &self.scopes {
            if byte_pos >= s.start_byte && byte_pos <= s.end_byte {
                let scope_len = s.end_byte.saturating_sub(s.start_byte);
                if scope_len < min_scope_len {
                    min_scope_len = scope_len;
                    best_scope = s.id;
                }
            }
        }

        // Walk up scope chain to find nearest visible matching symbol
        let mut cur = Some(best_scope);
        while let Some(sid) = cur {
            let mut best_match: Option<&OverlaySymbol> = None;
            for sym in &self.symbols {
                if sym.scope_id == sid && sym.name == ident {
                    let is_visible = match sym.kind {
                        DefKind::Fn
                        | DefKind::Struct
                        | DefKind::Enum
                        | DefKind::Trait
                        | DefKind::TyAlias
                        | DefKind::Const
                        | DefKind::Mod => true,
                        _ => sym.scope_start_byte <= byte_pos,
                    };
                    if is_visible {
                        if let Some(prev) = best_match {
                            if sym.scope_start_byte > prev.scope_start_byte {
                                best_match = Some(sym);
                            }
                        } else {
                            best_match = Some(sym);
                        }
                    }
                }
            }
            if let Some(sym) = best_match {
                return Some(OverlayDefinition {
                    name: sym.name.clone(),
                    kind: sym.kind,
                    signature: sym.signature.clone(),
                    file: self.file_path.clone(),
                    start_line: sym.start_line,
                    start_col: sym.start_col,
                    end_line: sym.end_line,
                    end_col: sym.end_col,
                });
            }
            cur = self.scopes.get(sid).and_then(|s| s.parent);
        }

        None
    }

    /// Returns the identifier at a given 1-based (line, char_col) in the buffer.
    #[allow(dead_code)]
    pub fn get_ident_at(&self, line: u32, col: u32) -> Option<String> {
        self.get_ident_and_span_at(line, col).map(|(s, _, _)| s)
    }

    /// Returns the identifier and its byte start and end offset within the line.
    pub fn get_ident_and_span_at(&self, line: u32, col: u32) -> Option<(String, usize, usize)> {
        let byte_pos = self.line_char_col_to_byte(line, col);
        let line_idx = line.saturating_sub(1) as usize;
        if line_idx >= self.line_starts.len() {
            return None;
        }
        let line_start = self.line_starts[line_idx];
        let line_end = if line_idx + 1 < self.line_starts.len() {
            self.line_starts[line_idx + 1]
        } else {
            self.text.len()
        };
        let line_text = &self.text[line_start..line_end];
        let col_byte = byte_pos.saturating_sub(line_start).min(line_text.len());

        let char_indices: Vec<(usize, char)> = line_text.char_indices().collect();
        if char_indices.is_empty() {
            return None;
        }

        let mut target_idx = None;
        for (i, &(idx, c)) in char_indices.iter().enumerate() {
            if idx <= col_byte && col_byte < idx + c.len_utf8() {
                target_idx = Some(i);
                break;
            }
        }
        if target_idx.is_none() && col_byte >= line_text.len() {
            target_idx = Some(char_indices.len().saturating_sub(1));
        }

        let idx = target_idx?;
        let c = char_indices[idx].1;
        if !is_rust_ident_continue(c) {
            if idx > 0 && is_rust_ident_continue(char_indices[idx - 1].1) {
                let mut start_i = idx - 1;
                while start_i > 0 && is_rust_ident_continue(char_indices[start_i - 1].1) {
                    start_i -= 1;
                }
                let mut end_i = idx;
                while end_i < char_indices.len() && is_rust_ident_continue(char_indices[end_i].1) {
                    end_i += 1;
                }
                if is_rust_ident_start(char_indices[start_i].1) {
                    let start_b = char_indices[start_i].0;
                    let end_b = if end_i < char_indices.len() {
                        char_indices[end_i].0
                    } else {
                        line_text.len()
                    };
                    return Some((line_text[start_b..end_b].to_string(), start_b, end_b));
                }
            }
            return None;
        }

        let mut start_i = idx;
        while start_i > 0 && is_rust_ident_continue(char_indices[start_i - 1].1) {
            start_i -= 1;
        }
        let mut end_i = idx + 1;
        while end_i < char_indices.len() && is_rust_ident_continue(char_indices[end_i].1) {
            end_i += 1;
        }

        if is_rust_ident_start(char_indices[start_i].1) {
            let start_b = char_indices[start_i].0;
            let end_b = if end_i < char_indices.len() {
                char_indices[end_i].0
            } else {
                line_text.len()
            };
            Some((line_text[start_b..end_b].to_string(), start_b, end_b))
        } else {
            None
        }
    }

    /// Returns all top-level and item symbols for LSP documentSymbol.
    pub fn document_symbols(&self) -> &[OverlaySymbol] {
        &self.symbols
    }
}

/// Thread-safe manager for all open document overlays in an active LSP session.
#[derive(Default)]
pub struct OverlayManager {
    overlays: HashMap<String, DocumentOverlay>,
}

impl OverlayManager {
    pub fn new() -> Self {
        Self {
            overlays: HashMap::new(),
        }
    }

    pub fn open_or_update(&mut self, uri: &str, file_path: &str, version: i32, text: String) {
        if let Some(existing) = self.overlays.get_mut(file_path) {
            existing.update(version, text);
        } else {
            let overlay = DocumentOverlay::new(uri, file_path, version, text);
            self.overlays.insert(file_path.to_string(), overlay);
        }
    }

    pub fn close(&mut self, file_path: &str) {
        self.overlays.remove(file_path);
    }

    pub fn get(&self, file_path: &str) -> Option<&DocumentOverlay> {
        self.overlays.get(file_path)
    }

    pub fn find_definition_at(&self, file_path: &str, line: u32, col: u32) -> Option<OverlayDefinition> {
        let overlay = self.overlays.get(file_path)?;
        overlay.find_definition_at(line, col)
    }

    pub fn document_symbols(&self, file_path: &str) -> Option<&[OverlaySymbol]> {
        self.overlays.get(file_path).map(|o| o.document_symbols())
    }

    pub fn reconcile_with_disk(&mut self, file_path: &str, disk_content_hash: u64) {
        if let Some(overlay) = self.overlays.get_mut(file_path) {
            if overlay.content_hash == disk_content_hash {
                overlay.disk_text = Some(overlay.text.clone());
            } else if let Ok(disk) = std::fs::read_to_string(file_path) {
                overlay.disk_text = Some(disk);
            }
        }
    }

    #[allow(dead_code)]
    pub fn map_compiled_line_to_live(&self, file_path: &str, compiled_line: u32) -> Option<u32> {
        self.overlays.get(file_path)?.map_compiled_line_to_live(compiled_line)
    }

    #[allow(dead_code)]
    pub fn advance_compiled_baseline(&mut self, file_path: &str, text: String) {
        if let Some(overlay) = self.overlays.get_mut(file_path) {
            overlay.advance_compiled_baseline(text);
        }
    }

    #[allow(dead_code)]
    pub fn open_count(&self) -> usize {
        self.overlays.len()
    }

    pub fn open_file_paths(&self) -> Vec<String> {
        self.overlays.keys().cloned().collect()
    }
}

struct OverlayParser<'a> {
    text: &'a str,
    line_starts: &'a [usize],
    scopes: Vec<Scope>,
    symbols: Vec<OverlaySymbol>,
    pending_fn_scope: Option<usize>,
}

impl<'a> OverlayParser<'a> {
    fn new(text: &'a str, line_starts: &'a [usize]) -> Self {
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

    fn parse(&mut self) {
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
                pos += 2;
                while pos + 1 < len && !(bytes[pos] == b'*' && bytes[pos + 1] == b'/') {
                    pos += 1;
                }
                pos = (pos + 2).min(len);
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
                *pos += 2;
                while *pos + 1 < len && !(bytes[*pos] == b'*' && bytes[*pos + 1] == b'/') {
                    *pos += 1;
                }
                *pos = (*pos + 2).min(len);
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
                let params_text = self.text[start + open_paren + 1..start + open_paren + close_paren].to_string();
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
            if trimmed.is_empty() || trimmed == "self" || trimmed == "&self" || trimmed == "&mut self" {
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
                        let abs_offset = base_byte + (part.as_ptr() as usize - params_text.as_ptr() as usize) + offset;
                        let (s_line, s_col) = self.byte_to_char_line_col(abs_offset);
                        let (e_line, e_col) = self.byte_to_char_line_col(abs_offset + p_name.len());
                        self.symbols.push(OverlaySymbol {
                            name: p_name.to_string(),
                            kind: DefKind::Field, // Local parameter
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
        // We track nested braces, parentheses, and brackets so that inner semicolons/braces
        // (such as in block initializers `let x = { let t = 2; x };`) do not end the outer let scope early.
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
                cur += 2;
                while cur + 1 < len && !(bytes[cur] == b'*' && bytes[cur + 1] == b'/') {
                    cur += 1;
                }
                cur = (cur + 2).min(len);
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
            kind: DefKind::Static, // Local variable
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_multibyte_non_identifier_is_safe() {
        let code = "fn f() { 🚀 }\n".to_string();
        let overlay = DocumentOverlay::new("file:///test/emoji.rs", "src/emoji.rs", 1, code);
        assert!(!overlay.symbols.is_empty());
        let def = overlay.find_definition_at(1, 4).expect("should resolve f");
        assert_eq!(def.name, "f");
    }

    #[test]
    fn test_shadow_initializer_sees_previous_binding() {
        let code = "fn test() {\n    let x = 1;\n    let x = x + 1;\n    x;\n}\n".to_string();
        let overlay = DocumentOverlay::new("file:///test/shadow_init.rs", "src/shadow_init.rs", 1, code);
        // Cursor on `x` in `x + 1` at line 3, col 13
        let def = overlay.find_definition_at(3, 13).expect("should resolve to outer x");
        assert_eq!(def.name, "x");
        assert_eq!(def.start_line, 2);

        // Cursor on `x;` on line 4, col 5 -> resolves to second `let x = x + 1` on line 3
        let def2 = overlay.find_definition_at(4, 5).expect("should resolve to inner x");
        assert_eq!(def2.name, "x");
        assert_eq!(def2.start_line, 3);
    }

    #[test]
    fn test_qualified_name_does_not_resolve_lexically() {
        let code = "fn f() {}\nfn use_it() { external::f(); }\n".to_string();
        let overlay = DocumentOverlay::new("file:///test/qual.rs", "src/qual.rs", 1, code);
        // Querying external::f on line 2, col 25 (the `f` in `external::f`) should abstain (None)
        let def = overlay.find_definition_at(2, 25);
        assert!(def.is_none(), "overlay must abstain for qualified path segment external::f");
    }

    #[test]
    fn test_nested_initializer_keeps_outer_binding() {
        let code = "fn test() {\n    let x = 1;\n    let x = { let t = 2; x };\n    x;\n}\n".to_string();
        let overlay = DocumentOverlay::new("file:///test/nested_init.rs", "src/nested_init.rs", 1, code);
        // Cursor on `x` inside block `{ let t = 2; x }` on line 3, col 27 -> resolves to line 2 (outer x)
        let def = overlay.find_definition_at(3, 27).expect("should resolve to outer x");
        assert_eq!(def.name, "x");
        assert_eq!(def.start_line, 2);

        // Cursor on `x;` on line 4, col 5 -> resolves to line 3 (inner x)
        let def2 = overlay.find_definition_at(4, 5).expect("should resolve to inner x");
        assert_eq!(def2.name, "x");
        assert_eq!(def2.start_line, 3);
    }

    #[test]
    fn test_multiline_qualified_path_abstains() {
        let code = "fn f() {}\nfn run() { external::\n    f();\n}\n".to_string();
        let overlay = DocumentOverlay::new("file:///test/multiline_qual.rs", "src/multiline_qual.rs", 1, code);
        // Querying `f` on line 3, col 5 should abstain (None) because of `external::` on previous line
        let def = overlay.find_definition_at(3, 5);
        assert!(def.is_none(), "overlay must abstain for multiline qualified path external::\\n    f()");
    }

    #[test]
    fn test_save_does_not_advance_compiled_baseline() {
        let initial_code = "fn a() {}\nfn b() {}\n".to_string();
        let mut overlay = DocumentOverlay::new("file:///test/save.rs", "src/save.rs", 1, initial_code);
        // User inserts a new line at top
        let edited_code = "// new line\nfn a() {}\nfn b() {}\n".to_string();
        overlay.update(2, edited_code.clone());
        // In edited buffer, line 3 (fn b) maps to compiled line 2
        assert_eq!(overlay.map_line_to_compiled(3), Some(2));

        // Now simulate didSave with the edited text hash
        let _hash = fnv1a_64(edited_code.as_bytes());
        overlay.disk_text = Some(edited_code.clone());
        // Since no compilation occurred, compiled baseline is still initial_code!
        assert_eq!(overlay.map_line_to_compiled(3), Some(2));
        assert!(overlay.is_dirty());
    }

    #[test]
    fn test_overlay_local_variables_and_params() {
        let code = "fn compute(input: i32) {\n    let result = input * 2;\n    result\n}\n".to_string();
        let overlay = DocumentOverlay::new("file:///test/main.rs", "src/main.rs", 1, code);

        // 1. Cursor on `result` in expression on line 3, col 5
        let def = overlay.find_definition_at(3, 5).expect("should find local variable 'result'");
        assert_eq!(def.name, "result");
        assert_eq!(def.start_line, 2);
        assert_eq!(def.kind, DefKind::Static); // local let

        // 2. Cursor on `input` in expression on line 2, col 19
        let def_param = overlay.find_definition_at(2, 19).expect("should find parameter 'input'");
        assert_eq!(def_param.name, "input");
        assert_eq!(def_param.start_line, 1);
        assert_eq!(def_param.kind, DefKind::Field); // param

        // 3. Cursor on function name `compute` at line 1, col 4
        let def_fn = overlay.find_definition_at(1, 4).expect("should find function 'compute'");
        assert_eq!(def_fn.name, "compute");
        assert_eq!(def_fn.kind, DefKind::Fn);
        assert_eq!(def_fn.start_line, 1);
    }

    #[test]
    fn test_unicode_binding_does_not_panic() {
        let code = "fn f() {\n    let привет = 1;\n    let мир = привет;\n}\n".to_string();
        let overlay = DocumentOverlay::new("file:///test/unicode.rs", "src/unicode.rs", 1, code);

        assert!(!overlay.symbols.is_empty());
        let def = overlay.find_definition_at(3, 16).expect("should resolve 'привет'");
        assert_eq!(def.name, "привет");
        assert_eq!(def.start_line, 2);

        let mir = overlay.find_definition_at(3, 9).expect("should resolve 'мир'");
        assert_eq!(mir.name, "мир");
        assert_eq!(mir.start_line, 3);
    }

    #[test]
    fn test_function_parameter_scope_isolation() {
        let code = "fn a(x: i32) {}\nfn b(x: i32) { x; }\n".to_string();
        let overlay = DocumentOverlay::new("file:///test/scopes.rs", "src/scopes.rs", 1, code);

        // Cursor on `x;` inside b: line 2, col 16
        let def = overlay.find_definition_at(2, 16).expect("should find x in b");
        assert_eq!(def.name, "x");
        // Parameter in b is on line 2, NOT line 1!
        assert_eq!(def.start_line, 2);
    }

    #[test]
    fn test_lexical_binding_shadowing_and_visibility() {
        let code = "fn test() {\n    let x = 1;\n    let x = 2;\n    x;\n}\n".to_string();
        let overlay = DocumentOverlay::new("file:///test/shadow.rs", "src/shadow.rs", 1, code);

        // Cursor on `x;` on line 4, col 5 -> should resolve to second `let x = 2` on line 3
        let def = overlay.find_definition_at(4, 5).expect("should resolve nearest shadowed binding");
        assert_eq!(def.name, "x");
        assert_eq!(def.start_line, 3);
    }

    #[test]
    fn test_overlay_edit_mapping() {
        let saved_text = "fn foo() {\n    call_a();\n    call_b();\n}\n".to_string();
        let mut overlay = DocumentOverlay::new("file:///test/edit.rs", "src/edit.rs", 1, saved_text.clone());
        overlay.disk_text = Some(saved_text);

        assert_eq!(overlay.map_line_to_compiled(2), Some(2));

        // Insert a new line at line 2:
        let edited = "fn foo() {\n    // new comment\n    call_a();\n    call_b();\n}\n".to_string();
        overlay.update(2, edited);

        // Line 1 (unchanged prefix) maps to line 1
        assert_eq!(overlay.map_line_to_compiled(1), Some(1));
        // Line 2 (newly inserted comment) is modified region -> None
        assert_eq!(overlay.map_line_to_compiled(2), None);
        // Line 3 (`call_a()`) is in unchanged suffix -> maps to line 2 in compiled snapshot!
        assert_eq!(overlay.map_line_to_compiled(3), Some(2));
        // Line 4 (`call_b()`) -> maps to line 3 in compiled snapshot!
        assert_eq!(overlay.map_line_to_compiled(4), Some(3));
    }

    #[test]
    fn test_overlay_position_conversion_with_unicode() {
        let code = "let привет = 42;\n".to_string();
        let overlay = DocumentOverlay::new("file:///test/conv.rs", "src/conv.rs", 1, code);

        // 'let ' = 4 chars / 4 utf-16
        // 'привет' = 6 Cyrillic chars (12 utf-8 bytes, 6 utf-16 code units)
        // utf-16 col 4 -> 1-based char col 5 (the start of 'привет')
        let char_col = overlay.utf16_col_to_char_col(1, 4);
        assert_eq!(char_col, 5);

        let utf16_col = overlay.char_col_to_utf16_col(1, 5);
        assert_eq!(utf16_col, 4);
    }

    #[test]
    fn test_overlay_incomplete_syntax_tolerance() {
        let broken = "fn unclosed(val: &str) {\n    let partial = val.\n".to_string();
        let overlay = DocumentOverlay::new("file:///test/broken.rs", "src/broken.rs", 1, broken);

        assert!(!overlay.symbols.is_empty());
        let fn_sym = overlay.symbols.iter().find(|s| s.name == "unclosed").expect("should extract function name");
        assert_eq!(fn_sym.kind, DefKind::Fn);

        let let_sym = overlay.symbols.iter().find(|s| s.name == "partial").expect("should extract incomplete let binding");
        assert_eq!(let_sym.start_line, 2);
    }

    #[test]
    fn test_overlay_manager_reconciliation() {
        let mut mgr = OverlayManager::new();
        let uri = "file:///test/live.rs";
        let file = "src/live.rs";
        let text1 = "fn foo() {}\n".to_string();

        mgr.open_or_update(uri, file, 1, text1.clone());
        assert_eq!(mgr.open_count(), 1);

        let hash1 = fnv1a_64(text1.as_bytes());
        mgr.reconcile_with_disk(file, hash1);
        assert!(mgr.get(file).unwrap().is_synced_with_disk());

        // User keeps typing while cargo check runs
        let text2 = "fn foo() { let x = 42; }\n".to_string();
        mgr.open_or_update(uri, file, 2, text2);
        assert!(!mgr.get(file).unwrap().is_synced_with_disk());

        mgr.close(file);
        assert_eq!(mgr.open_count(), 0);
    }
}
