/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! `ridx/safeDelete {textDocument, position}`: the edit deleting the item named at the position when nothing in the
//! workspace uses it, else a refusal with its uses (prod-code, m=20226: its safe delete and prune tools need it). The
//! uses are the ones `textDocument/references` gives, the session's unsaved text included; an item the index does not
//! know, a trait's item and a trait impl's (whose deletion breaks the trait or its impls) are refused with a reason.

use super::action_parse::{byte_to_utf16_pos, line_starts_of, utf16_pos_to_byte};
use crate::lsp::protocol::{uri_to_path, PositionConverter};
use crate::overlay::OverlayManager;
use ridx_format::WorkspaceIndex;
use ridx_syntax::{SyntaxKind as K, SyntaxTree, TokenKind as T};
use serde_json::{json, Value};
use std::ops::Range;

/// The items a safe delete takes away whole.
const ITEMS: &[K] = &[K::Fn, K::Struct, K::Enum, K::Union, K::Trait, K::Const, K::Static, K::TypeAlias, K::MacroRules, K::Module];

/// Why an item named at byte `offset` of `doc` cannot be deleted by itself, or the bytes its deletion takes: the item,
/// its doc comments, and its lines when nothing else is on them.
pub(crate) fn item_span(doc: &str, offset: usize) -> Result<Range<usize>, &'static str> {
    let tree = ridx_syntax::parse(doc);
    let at = |o: usize| Some(tree.covering(o)).filter(|&n| tree.kind(n) == K::Name);
    let name = at(offset).or_else(|| offset.checked_sub(1).and_then(at)).ok_or("notAnItem")?;
    let item = tree.parent(name).filter(|&i| ITEMS.contains(&tree.kind(i))).ok_or("notAnItem")?;
    if let Some(owner) = tree.ancestors(item).find(|&a| matches!(tree.kind(a), K::Trait | K::Impl)) {
        if tree.kind(owner) == K::Trait || tree.own_tokens(owner).any(|(_, t)| t.kind == T::For) {
            return Err("traitItem");
        }
    }
    // `mod m;` leaves its file behind.
    if tree.kind(item) == K::Module && tree.child(item, K::ItemList).is_none() {
        return Err("moduleFile");
    }
    Ok(lines_of(doc, &tree, tree.range(item)))
}

/// `range` grown over the doc comments right above it, then to whole lines when nothing else is on them, with one blank
/// line after it when one is before it too (no two blank lines left).
fn lines_of(doc: &str, tree: &SyntaxTree<'_>, range: Range<usize>) -> Range<usize> {
    let mut start = range.start;
    let docs = tree.tokens.iter().filter(|t| t.kind == T::LineComment || t.kind == T::BlockComment).filter(|t| t.text(doc).starts_with("///") || t.text(doc).starts_with("/**"));
    for t in docs.collect::<Vec<_>>().into_iter().rev() {
        if (t.end as usize) <= start && doc[t.end as usize..start].trim().is_empty() {
            start = t.start as usize;
        }
    }
    let line_start = doc[..start].rfind('\n').map_or(0, |i| i + 1);
    let line_end = doc[range.end..].find('\n').map_or(doc.len(), |i| range.end + i + 1);
    if !doc[line_start..start].trim().is_empty() || !doc[range.end..line_end].trim().is_empty() {
        let rest = &doc[range.end..];
        return start..range.end + (rest.len() - rest.trim_start_matches([' ', '\t']).len());
    }
    let blank_before = line_start == 0 || doc[..line_start - 1].rsplit('\n').next().is_some_and(|l| l.trim().is_empty());
    let next_end = doc[line_end..].find('\n').map(|i| line_end + i + 1);
    match next_end {
        Some(e) if blank_before && doc[line_end..e].trim().is_empty() => line_start..e,
        _ => line_start..line_end,
    }
}

/// The answer to `ridx/safeDelete`.
pub fn handle_safe_delete(msg: &Value, ws: Option<&WorkspaceIndex>, ov: &OverlayManager, conv: &mut PositionConverter) -> Value {
    let refuse = |reason: &str, uses: Value| json!({ "refused": true, "reason": reason, "remainingUses": uses });
    let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
    let file = uri_to_path(uri);
    let (doc, version) = match ov.get(&file) {
        Some(o) => (o.text.clone(), Some(o.version)),
        None => (std::fs::read_to_string(&file).unwrap_or_default(), None),
    };
    let starts = line_starts_of(&doc);
    let (line, col) = (msg["params"]["position"]["line"].as_u64().unwrap_or(0) as u32, msg["params"]["position"]["character"].as_u64().unwrap_or(0) as u32);
    let Some(offset) = utf16_pos_to_byte(&doc, &starts, line, col) else { return refuse("notAnItem", json!([])) };
    let span = match item_span(&doc, offset) {
        Ok(span) => span,
        Err(why) => return refuse(why, json!([])),
    };
    // Known to the index, else its uses are not known; every reference the index has of it is a use (an impl of it, a
    // `use` of it, a call), the definition's own name aside.
    let name_start = doc[..offset].rfind(|c: char| !(c.is_alphanumeric() || c == '_')).map_or(0, |i| i + 1);
    let char_col = doc[starts[line as usize]..name_start].chars().count() as u32;
    let Some((ws, def)) = ws.and_then(|w| w.find_definition_at(&file, line + 1, char_col + 1).map(|d| (w, d))) else {
        return refuse("unknownSymbol", json!([]));
    };
    let canonical = ridx_format::canonicalize_best_effort(std::path::Path::new(&file));
    let uses: Vec<Value> = ws
        .find_references(def.path)
        .iter()
        .filter(|r| !(r.span.start_line() == line + 1 && r.span.start_col() == char_col + 1 && ridx_format::canonicalize_best_effort(&r.resolved_path()) == canonical))
        .map(|r| {
            let path = crate::lsp::navigation::type_definition_parse::on_disk(r.resolved_path());
            let p = path.to_string_lossy();
            let (s, e) = (conv.char_col_to_utf16_col(&p, r.span.start_line(), r.span.start_col()), conv.char_col_to_utf16_col(&p, r.span.end_line(), r.span.end_col()));
            json!({ "uri": crate::lsp::protocol::path_to_uri(&p), "range": { "start": { "line": r.span.start_line() - 1, "character": s }, "end": { "line": r.span.end_line() - 1, "character": e } } })
        })
        .collect();
    if !uses.is_empty() {
        return refuse("used", json!(uses));
    }
    let ((sl, sc), (el, ec)) = (byte_to_utf16_pos(&doc, &starts, span.start), byte_to_utf16_pos(&doc, &starts, span.end));
    let edit = json!({ "range": { "start": { "line": sl, "character": sc }, "end": { "line": el, "character": ec } }, "newText": "" });
    let edit = match version {
        Some(v) => json!({ "documentChanges": [{ "textDocument": { "uri": uri, "version": v }, "edits": [edit] }] }),
        None => json!({ "changes": { uri: [edit] } }),
    };
    json!({ "edit": edit })
}

#[cfg(test)]
#[path = "tests/safe_delete_tests.rs"]
mod tests;
