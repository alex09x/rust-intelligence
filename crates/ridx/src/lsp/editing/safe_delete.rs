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

/// Attributes under which something outside the source uses an item: the linker (a symbol, a section the loader runs:
/// `#[used]`, `#[link_section]`, review of f4f8b604), the compiler's proc-macro loader.
const EXTERNAL_USE: &[&str] =
    &["no_mangle", "export_name", "used", "link_section", "global_allocator", "panic_handler", "alloc_error_handler", "proc_macro", "proc_macro_derive", "proc_macro_attribute"];

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
    // A crate root's `main` (`r#main` too) is its target's entry point, used though nothing names it.
    let norm = ridx_syntax::sema_fn::norm_ident;
    if tree.kind(item) == K::Fn && norm(tree.text(name)) == "main" && tree.parent(item).is_some_and(|p| tree.kind(p) == K::SourceFile) {
        return Err("entryPoint");
    }
    // The item and everything it holds (a module's items: review of f4f8b604), by any spelling (`#[r#no_mangle]`).
    let inner: Vec<_> = std::iter::once(item).chain(tree.descendants(item)).collect();
    if inner.iter().any(|&n| tree.kind(n) == K::Attr && tree.tokens_of(n).iter().any(|t| EXTERNAL_USE.contains(&norm(t.text(doc))))) {
        return Err("externallyUsed");
    }
    // An impl names nothing a reference could point at, and code elsewhere may need it (a trait bound, a method).
    if inner.iter().skip(1).any(|&n| tree.kind(n) == K::Impl) {
        return Err("containsImpl");
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

/// The 1-based line and character column the compiler gives byte `b` of `doc`: a byte order mark is no character of
/// the first line to it (review of f4f8b604).
fn line_col(doc: &str, b: usize) -> (u32, u32) {
    let ls = doc[..b].rfind('\n').map_or(0, |i| i + 1);
    let bom = ls == 0 && doc.starts_with('\u{feff}');
    (doc[..ls].matches('\n').count() as u32 + 1, doc[ls..b].chars().count() as u32 + 1 - bom as u32)
}

/// The byte of `doc` at the compiler's 1-based `line` and character `col`.
fn byte_at(doc: &str, starts: &[usize], line: u32, col: u32) -> Option<usize> {
    let ls = *starts.get(line.saturating_sub(1) as usize)?;
    let skip = col.saturating_sub(1) as usize + (ls == 0 && doc.starts_with('\u{feff}')) as usize;
    Some(ls + doc[ls..].chars().take(skip).map(char::len_utf8).sum::<usize>())
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
    // Any buffer edited elsewhere: the index may not have its uses yet, and no reading of its text tells them all (a
    // macro's, a name spelled another way: review of f4f8b604).
    let unsaved =
        ov.open_file_paths().into_iter().filter(|k| ridx_format::canonicalize_best_effort(std::path::Path::new(k)) != canonical).any(|k| ov.get(&k).is_some_and(|o| o.is_dirty()));
    if unsaved {
        return refuse("unsavedUses", Vec::new());
    }
    // A file saved, changed on disk or added since the index was built may hold a use it lacks (prod-code, m=20268),
    // told by the sources' content, not by what the server was told of (review of 9effe92d).
    // The words a path to the item may hold: its names and the modules its file's path spells (`b` of `src/b.rs`).
    let module_words: Vec<String> = canonical.iter().map(|c| std::path::Path::new(c).file_stem().map_or(String::new(), |s| s.to_string_lossy().into_owned())).collect();
    let context: Vec<&str> = names.iter().copied().chain(module_words.iter().map(String::as_str)).filter(|w| !w.is_empty() && *w != "mod" && *w != "lib" && *w != "main").collect();
    if !super::safe_delete_fresh::uses_covered(ws, ov, &names, &context, (&canonical, plan.span.clone())) {
        return refuse("staleIndex", Vec::new());
    }
    let mut uses = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (i, r) in plan.names.iter().enumerate() {
        let (l, c) = line_col(&doc, r.start);
        // The definition the index has there, at its place in the very text it compiled; its name compared where both
        // spellings are ASCII (another normal form of a name is the compiler's, not a different item).
        let same_name = |s: &str| {
            let s = ridx_syntax::sema_fn::norm_ident(s);
            s == names[i] || !(s.is_ascii() && names[i].is_ascii())
        };
        let Some(found) = ws.find_references_at(&file, l, c).filter(|f| same_name(f.target_symbol) && f.target_line == l && f.target_col == c) else {
            return refuse(if i == 0 { "unknownSymbol" } else { "unknownMember" }, Vec::new());
        };
        // Every edge the index has to it: the definition's own (impls, aliases) and the path's, a macro's expansion
        // and a whole impl header included (review of f4f8b604: the first alone dropped some).
        let by_path = ws.find_references(found.target_path);
        let mut edges: Vec<super::safe_delete_edges::Edge> = found
            .references
            .iter()
            .chain(&by_path)
            .map(|rf| super::safe_delete_edges::Edge {
                path: crate::lsp::navigation::type_definition_parse::on_disk(rf.resolved_path()),
                start: (rf.span.start_line(), rf.span.start_col()),
                end: (rf.span.end_line(), rf.span.end_col()),
            })
            .collect();
        // And every edge the shards store to it, unfiltered: a macro's expansion too (review of 4009c2c4).
        // A use the shards hold but cannot place is still a use (review of b0b16e39).
        let Ok(stored) = super::safe_delete_edges::raw_edges(ws, &canonical, l, c, found.target_path) else {
            return refuse("unlocatedUses", Vec::new());
        };
        edges.extend(stored);
        for edge in edges {
            let path = ridx_format::canonicalize_best_effort(&edge.path);
            if !seen.insert((path.clone(), edge.start)) {
                continue;
            }
            let inside = path == canonical && byte_at(&doc, &starts, edge.start.0, edge.start.1).is_some_and(|b| plan.span.contains(&b));
            if !inside {
                let p = edge.path.to_string_lossy();
                // The compiler's columns of a first line after a byte order mark do not count it; the text converted
                // does, so the column is moved past it before converting (review of 4009c2c4; of b0b16e39: after it,
                // a character of two UTF-16 units would be cut).
                // Both from one text, the open buffer's when there is one (review of 6b91c9eb: a BOM-only edit leaves the
                // hash as it was while the disk's columns differ).
                let text = super::safe_delete_fresh::text_of(&edge.path, ov);
                let mut col = |(l, c): (u32, u32)| match &text {
                    Some(t) => {
                        let line = t.split('\n').nth(l.saturating_sub(1) as usize).unwrap_or("");
                        let skip = c.saturating_sub(1) as usize + (l == 1 && t.starts_with('\u{feff}')) as usize;
                        line.chars().take(skip).map(|ch| ch.len_utf16() as u32).sum()
                    }
                    None => conv.char_col_to_utf16_col(&p, l, c),
                };
                let (s, e) = (col(edge.start), col(edge.end));
                uses.push(json!({ "uri": crate::lsp::protocol::path_to_uri(&p), "range": { "start": { "line": edge.start.0 - 1, "character": s }, "end": { "line": edge.end.0 - 1, "character": e } } }));
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
