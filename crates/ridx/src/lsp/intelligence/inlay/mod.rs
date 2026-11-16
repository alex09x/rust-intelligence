/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! LSP inlay hints with rust-analyzer's default set: binding types, parameter names, method-chain types and
//! closing-brace labels.

mod bindings;
mod chaining;
mod closing;
mod params;
pub mod type_label;

use crate::lsp::error::LspResult;
use crate::lsp::protocol::uri_to_path;
use crate::overlay::OverlayManager;
use ridx_format::WorkspaceIndex;
use serde_json::{json, Value};

/// A type the index records for a span (1-based lines and columns).
pub struct TypeSpan<'a> {
    pub start: (u32, u32),
    pub end: (u32, u32),
    pub ty: &'a str,
}

/// A document's text, its masked copy (comments and literals blanked, same offsets) and line starts.
pub struct SourceMap<'a> {
    pub text: &'a str,
    pub masked: &'a str,
    line_starts: Vec<usize>,
}

impl<'a> SourceMap<'a> {
    pub fn new(text: &'a str, masked: &'a str) -> Self {
        let mut line_starts = vec![0];
        line_starts.extend(text.match_indices('\n').map(|(i, _)| i + 1));
        Self { text, masked, line_starts }
    }

    /// 0-based line and 0-based char column of byte `offset`.
    pub fn line_col(&self, offset: usize) -> (u32, u32) {
        let line = self.line_starts.partition_point(|&s| s <= offset).saturating_sub(1);
        let start = self.line_starts[line];
        (line as u32, self.text[start..offset.min(self.text.len())].chars().count() as u32)
    }

    /// 0-based line and UTF-16 column of byte `offset`.
    pub fn position(&self, offset: usize) -> (u32, u32) {
        let line = self.line_starts.partition_point(|&s| s <= offset).saturating_sub(1);
        let start = self.line_starts[line];
        (line as u32, self.text[start..offset.min(self.text.len())].encode_utf16().count() as u32)
    }

    /// The byte offset where 0-based `line` starts.
    pub fn line_start(&self, line: usize) -> Option<usize> {
        self.line_starts.get(line).copied()
    }

    pub fn line_count(&self) -> usize {
        self.line_starts.len()
    }
}

/// One inlay hint at a byte offset of the document.
pub struct Hint {
    pub line: u32,
    pub character: u32,
    pub label: String,
    /// LSP kind: 1 type, 2 parameter, none for other hints.
    pub kind: Option<u8>,
    pub padding_left: bool,
    pub padding_right: bool,
}

impl Hint {
    pub fn new(src: &SourceMap<'_>, offset: usize, label: String, kind: Option<u8>) -> Self {
        let (line, character) = src.position(offset);
        Self { line, character, label, kind, padding_left: false, padding_right: false }
    }

    fn to_json(&self) -> Value {
        let mut v =
            json!({ "position": { "line": self.line, "character": self.character }, "label": self.label, "paddingLeft": self.padding_left, "paddingRight": self.padding_right });
        if let Some(k) = self.kind {
            v["kind"] = json!(k);
        }
        v
    }
}

pub fn handle_inlay_hint(msg: &Value, workspace: Option<&WorkspaceIndex>, overlay_mgr: &OverlayManager) -> LspResult<Value> {
    let (uri, start_line, _start_col, end_line, _end_col) = crate::lsp::params::text_document_range(msg)?;
    let file = uri_to_path(uri);
    let text = match overlay_mgr.get(&file) {
        Some(ov) => ov.text.clone(),
        None => match std::fs::read_to_string(&file) {
            Ok(t) => t,
            Err(_) => return Ok(json!([])),
        },
    };
    let masked = crate::lsp::editing::action_parse::mask_comments_and_strings(&text);
    let src = SourceMap::new(&text, &masked);
    let lines = (start_line, end_line);

    // The index's types sit on the lines of the text it compiled; in an edited buffer they move to the live lines
    // (a span on a line the edit changed or removed is dropped), and a binding on a new line asks the live engine.
    let line_map = overlay_mgr.get(&file).filter(|o| o.is_dirty()).and_then(|o| o.line_map());
    let compiled_lines = overlay_mgr.get(&file).and_then(|o| o.compiled_text.as_ref()).map_or(0, |t| t.lines().count());
    let index_lines = src.line_count().max(compiled_lines) as u32;
    let types: Vec<TypeSpan<'_>> = workspace
        .map(|ws| {
            ws.types_in_lines(&file, 1, index_lines)
                .into_iter()
                .filter_map(|(start, end, ty)| match &line_map {
                    Some(lm) => Some(TypeSpan { start: (lm.to_live(start.0)?, start.1), end: (lm.to_live(end.0)?, end.1), ty }),
                    None => Some(TypeSpan { start, end, ty }),
                })
                .collect()
        })
        .unwrap_or_default();
    let live_type = |b_start: usize, b_end: usize| -> Option<String> {
        let (ws, lm) = (workspace?, line_map.as_ref()?);
        let (l0, _) = src.line_col(b_start);
        if lm.to_baseline(l0 + 1).is_some() || src.masked[b_end..].trim_start().starts_with(':') {
            return None;
        }
        let db = crate::lsp::navigation::live_db_for(ws);
        let mut db = db.lock().unwrap_or_else(|e| e.into_inner());
        db.type_at(ws, &file, b_start as u32, Some(overlay_mgr)).map(|t| t.display)
    };

    // The hints read source text with offsets from a masked copy; a slip must cost the hints, not the server.
    let collected = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut hints = bindings::binding_hints(&src, &types, lines, &live_type);
        hints.extend(chaining::chaining_hints(&src, &types, lines));
        hints.extend(closing::closing_brace_hints(&src, lines));
        if let Some(ws) = workspace {
            let to_index = |live: u32| line_map.as_ref().map_or(Some(live), |lm| lm.to_baseline(live));
            hints.extend(params::parameter_hints(&src, ws, &file, lines, &to_index));
        }
        hints
    }));
    let Ok(mut hints) = collected else { return Ok(json!([])) };
    hints.sort_by_key(|h| (h.line, h.character));
    Ok(json!(hints.iter().map(Hint::to_json).collect::<Vec<_>>()))
}
