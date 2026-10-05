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
    line_starts: Vec<usize>,
    symbols: Vec<OverlaySymbol>,
    scopes: Vec<Scope>,
    pub is_synced_with_disk: bool,
}

impl DocumentOverlay {
    /// Creates and parses a new DocumentOverlay from buffer text.
    pub fn new(uri: impl Into<String>, file_path: impl Into<String>, version: i32, text: String) -> Self {
        let uri = uri.into();
        let file_path = file_path.into();
        let content_hash = fnv1a_64(text.as_bytes());
        let mut overlay = Self {
            uri,
            file_path,
            version,
            content_hash,
            text,
            line_starts: Vec::new(),
            symbols: Vec::new(),
            scopes: Vec::new(),
            is_synced_with_disk: false,
        };
        overlay.reparse();
        overlay
    }

    /// Updates buffer text, version, and reparses symbols.
    pub fn update(&mut self, version: i32, text: String) {
        self.version = version;
        self.content_hash = fnv1a_64(text.as_bytes());
        self.text = text;
        self.is_synced_with_disk = false;
        self.reparse();
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
        let ident = self.get_ident_at(line, col)?;
        let byte_pos = self.line_col_to_byte(line, col);

        // Find enclosing scope
        let mut best_scope = 0;
        for s in &self.scopes {
            if byte_pos >= s.start_byte && byte_pos <= s.end_byte {
                best_scope = s.id;
            }
        }

        // Walk up scope chain to find matching symbol
        let mut cur = Some(best_scope);
        while let Some(sid) = cur {
            for sym in &self.symbols {
                if sym.scope_id == sid && sym.name == ident {
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
            cur = self.scopes.get(sid).and_then(|s| s.parent);
        }

        None
    }

    /// Returns the identifier at a given (line, col) in the buffer.
    pub fn get_ident_at(&self, line: u32, col: u32) -> Option<String> {
        let line_idx = line.saturating_sub(1) as usize;
        if line_idx >= self.line_starts.len() {
            return None;
        }
        let start = self.line_starts[line_idx];
        let end = if line_idx + 1 < self.line_starts.len() {
            self.line_starts[line_idx + 1]
        } else {
            self.text.len()
        };

        let line_text = &self.text[start..end];
        let col_idx = col.saturating_sub(1) as usize;
        if col_idx >= line_text.len() {
            return None;
        }

        let bytes = line_text.as_bytes();
        if !is_ident_part(bytes[col_idx]) {
            return None;
        }

        let mut left = col_idx;
        while left > 0 && is_ident_part(bytes[left - 1]) {
            left -= 1;
        }
        let mut right = col_idx;
        while right < bytes.len() && is_ident_part(bytes[right]) {
            right += 1;
        }

        Some(line_text[left..right].to_string())
    }

    fn line_col_to_byte(&self, line: u32, col: u32) -> usize {
        let line_idx = line.saturating_sub(1) as usize;
        if line_idx >= self.line_starts.len() {
            return self.text.len();
        }
        let start = self.line_starts[line_idx];
        (start + col.saturating_sub(1) as usize).min(self.text.len())
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

    #[allow(dead_code)]
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
            overlay.is_synced_with_disk = overlay.content_hash == disk_content_hash;
        }
    }

    #[allow(dead_code)]
    pub fn open_count(&self) -> usize {
        self.overlays.len()
    }
}

struct OverlayParser<'a> {
    text: &'a str,
    line_starts: &'a [usize],
    scopes: Vec<Scope>,
    symbols: Vec<OverlaySymbol>,
}

impl<'a> OverlayParser<'a> {
    fn new(text: &'a str, line_starts: &'a [usize]) -> Self {
        Self {
            text,
            line_starts,
            scopes: Vec::new(),
            symbols: Vec::new(),
        }
    }

    fn byte_to_line_col(&self, byte_offset: usize) -> (u32, u32) {
        let b = byte_offset.min(self.text.len());
        let line_idx = match self.line_starts.binary_search(&b) {
            Ok(idx) => idx,
            Err(idx) => idx.saturating_sub(1),
        };
        let line = (line_idx + 1) as u32;
        let line_start_byte = self.line_starts[line_idx];
        let col = (b.saturating_sub(line_start_byte) + 1) as u32;
        (line, col)
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
                let new_id = self.scopes.len();
                let parent = scope_stack.last().copied();
                self.scopes.push(Scope {
                    id: new_id,
                    parent,
                    start_byte: pos,
                    end_byte: len,
                });
                scope_stack.push(new_id);
                pos += 1;
                continue;
            }

            if b == b'}' {
                if let Some(id) = scope_stack.pop() {
                    if id < self.scopes.len() {
                        self.scopes[id].end_byte = pos;
                    }
                }
                pos += 1;
                continue;
            }

            // Identifiers / keywords
            if is_ident_start(b) {
                let start = pos;
                while pos < len && is_ident_part(bytes[pos]) {
                    pos += 1;
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
            } else {
                break;
            }
        }
    }

    fn parse_ident(&self, pos: &mut usize) -> Option<(&'a str, usize, usize)> {
        self.skip_whitespace_and_comments(pos);
        let bytes = self.text.as_bytes();
        let len = bytes.len();
        if *pos < len && is_ident_start(bytes[*pos]) {
            let start = *pos;
            while *pos < len && is_ident_part(bytes[*pos]) {
                *pos += 1;
            }
            Some((&self.text[start..*pos], start, *pos))
        } else {
            None
        }
    }

    fn parse_fn_item(&mut self, pos: &mut usize, current_scope: usize) {
        let Some((name, start, end)) = self.parse_ident(pos) else {
            return;
        };
        let (s_line, s_col) = self.byte_to_line_col(start);
        let (e_line, e_col) = self.byte_to_line_col(end);

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

        self.symbols.push(OverlaySymbol {
            name: name.to_string(),
            kind: DefKind::Fn,
            signature,
            start_line: s_line,
            start_col: s_col,
            end_line: e_line,
            end_col: e_col,
            scope_id: current_scope,
        });

        // Parse parameters inside `(...)`
        if let Some(open_paren) = self.text[start..sig_end].find('(') {
            if let Some(close_paren) = self.text[start + open_paren..sig_end].find(')') {
                let params_text = self.text[start + open_paren + 1..start + open_paren + close_paren].to_string();
                self.extract_params(&params_text, start + open_paren + 1, current_scope);
            }
        }
    }

    fn extract_params(&mut self, params_text: &str, base_byte: usize, scope_id: usize) {
        for part in params_text.split(',') {
            let trimmed = part.trim();
            if trimmed.is_empty() || trimmed == "self" || trimmed == "&self" || trimmed == "&mut self" {
                continue;
            }
            if let Some((p_name, _)) = trimmed.split_once(':') {
                let p_name = p_name.trim().trim_start_matches("mut ").trim();
                if !p_name.is_empty() && is_ident(p_name) {
                    if let Some(offset) = part.find(p_name) {
                        let abs_offset = base_byte + (part.as_ptr() as usize - params_text.as_ptr() as usize) + offset;
                        let (s_line, s_col) = self.byte_to_line_col(abs_offset);
                        let (e_line, e_col) = self.byte_to_line_col(abs_offset + p_name.len());
                        self.symbols.push(OverlaySymbol {
                            name: p_name.to_string(),
                            kind: DefKind::Field, // Local parameter
                            signature: Some(trimmed.to_string()),
                            start_line: s_line,
                            start_col: s_col,
                            end_line: e_line,
                            end_col: e_col,
                            scope_id,
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
        let (s_line, s_col) = self.byte_to_line_col(start);
        let (e_line, e_col) = self.byte_to_line_col(end);

        self.symbols.push(OverlaySymbol {
            name: name.to_string(),
            kind,
            signature: None,
            start_line: s_line,
            start_col: s_col,
            end_line: e_line,
            end_col: e_col,
            scope_id: current_scope,
        });
    }

    fn parse_let_binding(&mut self, pos: &mut usize, current_scope: usize) {
        self.skip_whitespace_and_comments(pos);
        // Optional `mut`
        if *pos + 3 < self.text.len() && &self.text[*pos..*pos + 3] == "mut" && self.text.as_bytes()[*pos + 3].is_ascii_whitespace() {
            *pos += 4;
        }

        let Some((name, start, end)) = self.parse_ident(pos) else {
            return;
        };
        let (s_line, s_col) = self.byte_to_line_col(start);
        let (e_line, e_col) = self.byte_to_line_col(end);

        self.symbols.push(OverlaySymbol {
            name: name.to_string(),
            kind: DefKind::Static, // Local variable
            signature: None,
            start_line: s_line,
            start_col: s_col,
            end_line: e_line,
            end_col: e_col,
            scope_id: current_scope,
        });
    }
}

#[inline]
fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

#[inline]
fn is_ident_part(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn is_ident(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.is_empty() || !is_ident_start(bytes[0]) {
        return false;
    }
    bytes[1..].iter().all(|&b| is_ident_part(b))
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn test_overlay_incomplete_syntax_tolerance() {
        // Incomplete code with unclosed brace and incomplete member expression
        let broken = "fn unclosed(val: &str) {\n    let partial = val.\n".to_string();
        let overlay = DocumentOverlay::new("file:///test/broken.rs", "src/broken.rs", 1, broken);

        // Parser does not panic and extracts symbols despite syntax errors
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
        assert!(mgr.get(file).unwrap().is_synced_with_disk);

        // User keeps typing while cargo check runs
        let text2 = "fn foo() { let x = 42; }\n".to_string();
        mgr.open_or_update(uri, file, 2, text2);
        assert!(!mgr.get(file).unwrap().is_synced_with_disk);

        mgr.close(file);
        assert_eq!(mgr.open_count(), 0);
    }
}

