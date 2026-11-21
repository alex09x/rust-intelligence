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
//! workspace uses it, else a refusal with its uses (prod-code, m=20226: its safe delete and prune tools need it).
//!
//! It fails closed (review of d753d993): the index must have compiled the document's very text and know the item named
//! there, by name and place; every declaration the deletion takes (an inline module's items too) is checked, by the
//! index's references to that definition; an open buffer edited elsewhere naming one of them keeps it; and an entry
//! point, an item exported to the linker or a proc macro, a dependency's source, a trait's or a trait impl's item and a
//! `mod m;` are refused with their reason.

use super::action_parse::{byte_to_utf16_pos, line_starts_of, utf16_pos_to_byte};
use crate::lsp::protocol::{uri_to_path, PositionConverter};
use crate::overlay::OverlayManager;
use ridx_format::WorkspaceIndex;
use ridx_syntax::{SyntaxKind as K, SyntaxTree, TokenKind as T};
use serde_json::{json, Value};
use std::ops::Range;

/// The items a safe delete takes away whole.
const ITEMS: &[K] = &[K::Fn, K::Struct, K::Enum, K::Union, K::Trait, K::Const, K::Static, K::TypeAlias, K::MacroRules, K::Module];

/// Attributes under which something outside the source uses an item: the linker, the compiler's proc-macro loader.
const EXTERNAL_USE: &[&str] = &["no_mangle", "export_name", "proc_macro", "proc_macro_derive", "proc_macro_attribute"];

/// What deleting the item named at a position takes away: the bytes, and the name of every declaration in them (the
/// item first).
#[derive(Debug)]
pub(crate) struct Plan {
    pub span: Range<usize>,
    pub names: Vec<Range<usize>>,
}

/// The plan for the item whose name covers byte `offset` of `doc`, or why it is refused.
pub(crate) fn plan(doc: &str, offset: usize) -> Result<Plan, &'static str> {
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
    // A crate root's `main` is its target's entry point, used though nothing names it.
    if tree.kind(item) == K::Fn && tree.text(name) == "main" && tree.parent(item).is_some_and(|p| tree.kind(p) == K::SourceFile) {
        return Err("entryPoint");
    }
    if tree.children(item).filter(|&c| tree.kind(c) == K::Attr).any(|a| tree.tokens_of(a).iter().any(|t| EXTERNAL_USE.contains(&t.text(doc)))) {
        return Err("externallyUsed");
    }
    let mut names = vec![tree.range(name)];
    names.extend(tree.descendants(item).filter(|&d| tree.kind(d) == K::Name && d != name && tree.parent(d).is_some_and(|p| ITEMS.contains(&tree.kind(p)))).map(|d| tree.range(d)));
    Ok(Plan { span: lines_of(doc, &tree, tree.range(item)), names })
}

/// Whether `text` is an outer doc comment: `///` or `/** */`, not `////`, `/***` or `/**/` (review of d753d993).
fn outer_doc(text: &str) -> bool {
    (text.starts_with("///") && !text.starts_with("////")) || (text.starts_with("/**") && !text.starts_with("/***") && !text.starts_with("/**/"))
}

/// `range` grown over the doc comments right above it, then to whole lines when nothing else is on them, with one blank
/// line after it when one is before it too (no two blank lines left).
fn lines_of(doc: &str, tree: &SyntaxTree<'_>, range: Range<usize>) -> Range<usize> {
    let mut start = range.start;
    let docs: Vec<_> = tree.tokens.iter().filter(|t| matches!(t.kind, T::LineComment | T::BlockComment) && outer_doc(t.text(doc))).collect();
    for t in docs.into_iter().rev() {
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

/// The 1-based line and character column of byte `b` of `doc`.
fn line_col(doc: &str, b: usize) -> (u32, u32) {
    let ls = doc[..b].rfind('\n').map_or(0, |i| i + 1);
    (doc[..ls].matches('\n').count() as u32 + 1, doc[ls..b].chars().count() as u32 + 1)
}

/// The answer to `ridx/safeDelete`.
pub fn handle_safe_delete(msg: &Value, ws: Option<&WorkspaceIndex>, ov: &OverlayManager, conv: &mut PositionConverter) -> Value {
    let refuse = |reason: &str, uses: Vec<Value>| json!({ "refused": true, "reason": reason, "remainingUses": uses });
    let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
    let file = uri_to_path(uri);
    if super::atomic_write::is_readonly_dependency(std::path::Path::new(&file), ws) {
        return refuse("readOnly", Vec::new());
    }
    let (doc, version) = match ov.get(&file) {
        Some(o) => (o.text.clone(), Some(o.version)),
        None => (std::fs::read_to_string(&file).unwrap_or_default(), None),
    };
    let starts = line_starts_of(&doc);
    let (line, col) = (msg["params"]["position"]["line"].as_u64().unwrap_or(0) as u32, msg["params"]["position"]["character"].as_u64().unwrap_or(0) as u32);
    let Some(offset) = utf16_pos_to_byte(&doc, &starts, line, col) else { return refuse("notAnItem", Vec::new()) };
    let plan = match plan(&doc, offset) {
        Ok(plan) => plan,
        Err(why) => return refuse(why, Vec::new()),
    };
    let Some(ws) = ws else { return refuse("unknownSymbol", Vec::new()) };
    // The index compiled this very text: its coordinates are this text's.
    if !ws.units_agree_on_source(&file) || ws.source_hash_for_file(&file) != Some(ridx_format::normalized_source_hash(doc.as_bytes())) {
        return refuse("staleIndex", Vec::new());
    }
    let canonical = ridx_format::canonicalize_best_effort(std::path::Path::new(&file));
    let names: Vec<&str> = plan.names.iter().map(|r| ridx_syntax::sema_fn::norm_ident(&doc[r.clone()])).collect();
    // A buffer edited elsewhere, whose uses the index may not have yet, naming what would go.
    let unsaved = ov.open_file_paths().into_iter().filter(|k| ridx_format::canonicalize_best_effort(std::path::Path::new(k)) != canonical).any(|k| {
        ov.get(&k).is_some_and(|o| {
            o.is_dirty() && ridx_syntax::lex(&o.text).iter().any(|t| matches!(t.kind, T::Ident | T::RawIdent) && names.contains(&ridx_syntax::sema_fn::norm_ident(t.text(&o.text))))
        })
    });
    if unsaved {
        return refuse("unsavedUses", Vec::new());
    }
    let mut uses = Vec::new();
    for (i, r) in plan.names.iter().enumerate() {
        let (l, c) = line_col(&doc, r.start);
        // The definition the index has there, by its name and place: what it says of the uses is of this declaration.
        let Some(found) = ws.find_references_at(&file, l, c).filter(|f| ridx_syntax::sema_fn::norm_ident(f.target_symbol) == names[i] && f.target_line == l && f.target_col == c)
        else {
            return refuse(if i == 0 { "unknownSymbol" } else { "unknownMember" }, Vec::new());
        };
        for rf in &found.references {
            let path = crate::lsp::navigation::type_definition_parse::on_disk(rf.resolved_path());
            let inside = ridx_format::canonicalize_best_effort(&path) == canonical && {
                let b = starts
                    .get(rf.span.start_line().saturating_sub(1) as usize)
                    .map(|&ls| ls + doc[ls..].chars().take(rf.span.start_col().saturating_sub(1) as usize).map(char::len_utf8).sum::<usize>());
                b.is_some_and(|b| plan.span.contains(&b))
            };
            if !inside {
                let p = path.to_string_lossy();
                let (s, e) = (conv.char_col_to_utf16_col(&p, rf.span.start_line(), rf.span.start_col()), conv.char_col_to_utf16_col(&p, rf.span.end_line(), rf.span.end_col()));
                uses.push(json!({ "uri": crate::lsp::protocol::path_to_uri(&p), "range": { "start": { "line": rf.span.start_line() - 1, "character": s }, "end": { "line": rf.span.end_line() - 1, "character": e } } }));
            }
        }
    }
    if !uses.is_empty() {
        return refuse("used", uses);
    }
    let ((sl, sc), (el, ec)) = (byte_to_utf16_pos(&doc, &starts, plan.span.start), byte_to_utf16_pos(&doc, &starts, plan.span.end));
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
