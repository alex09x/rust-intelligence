/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! rust-analyzer's `Replace qualified path with use`: on `std::fs::read_to_string(f)` it imports the path and writes
//! every occurrence of it in the file by its last segment. The cursor picks the path: its segment and the ones before
//! it (`fs` in `std::fs::read_to_string` imports `std::fs`), the whole path from its first segment.
//!
//! Offered only where the result surely compiles: the path starts at a crate root (`std`, `core`, `alloc`, `crate`, a
//! dependency), names no associated item (no segment before the last is a type), carries no generic arguments and
//! is no macro; the name it brings in is not already used unqualified in the file, nor imported from elsewhere.

use super::action_parse::{line_starts_of, mask_comments_and_strings, utf16_pos_to_byte};
use super::action_raw_string::build_multi_edits_workspace_edit;
use ridx_format::{DefKind, WorkspaceIndex};
use serde_json::{json, Value};

fn is_ident(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// The `Replace qualified path with use` action at the cursor of an empty `range`, with its edit.
pub fn qualified_use_action(doc: &str, uri: &str, range: &Value, doc_version: Option<i32>, ws: &WorkspaceIndex) -> Option<Value> {
    if range["start"] != range["end"] {
        return None;
    }
    let (line0, col) = (range["start"]["line"].as_u64()? as u32, range["start"]["character"].as_u64()? as u32);
    let edits = qualified_use_edits(doc, line0, col, &|first| starts_at_crate_root(first, ws))?;
    Some(json!({
        "title": "Replace qualified path with use",
        "kind": "refactor.rewrite",
        "edit": build_multi_edits_workspace_edit(uri, edits, doc_version)
    }))
}

/// The text edits of the assist at the UTF-16 cursor `line0:col`; `crate_root` tells whether a first segment names
/// a crate.
pub(crate) fn qualified_use_edits(doc: &str, line0: u32, col: u32, crate_root: &dyn Fn(&str) -> bool) -> Option<Vec<Value>> {
    let starts = line_starts_of(doc);
    let cursor = utf16_pos_to_byte(doc, &starts, line0, col)?;
    let masked = mask_comments_and_strings(doc);
    let path = path_at(&masked, cursor)?;
    if crate::lsp::intelligence::completion_edits::in_use_item(&masked, cursor) || !crate_root(&path[0]) {
        return None;
    }
    // An associated item (`Type::new`, `Ordering::Less`) is no import target, nor a path through generics.
    if path[..path.len() - 1].iter().any(|s| s.starts_with(|c: char| c.is_ascii_uppercase())) {
        return None;
    }
    let text = path.join("::");
    let name = path.last()?.as_str();
    let occurrences = occurrences(&masked, &text);
    if occurrences.is_empty() || name_in_use(&masked, name, &occurrences, text.len()) {
        return None;
    }
    let mut edits: Vec<Value> = occurrences
        .iter()
        .map(|&at| {
            let first_seg_len = text.len() - name.len();
            json!({ "range": { "start": pos(doc, &starts, at), "end": pos(doc, &starts, at + first_seg_len) }, "newText": "" })
        })
        .collect();
    match crate::lsp::intelligence::auto_import::compute_auto_import_edit(doc, &text) {
        Some(import) => edits.insert(0, import),
        // Already imported under this very path: only the occurrences are rewritten.
        None if imports_exactly(&masked, &text) => {}
        None => return None,
    }
    Some(edits)
}

/// The path segments under the cursor that the assist imports: through the cursor's segment, or all of them when
/// the cursor is on the first one. `None` unless at least two segments remain.
fn path_at(masked: &str, cursor: usize) -> Option<Vec<String>> {
    let b = masked.as_bytes();
    let mut s = cursor.min(b.len());
    while s > 0 && is_ident(b[s - 1]) {
        s -= 1;
    }
    let mut e = cursor.min(b.len());
    while e < b.len() && is_ident(b[e]) {
        e += 1;
    }
    if s == e {
        return None;
    }
    // Walk back over `seg::` to the path's start, then forward over `::seg` to its end.
    let mut start = s;
    let mut before = Vec::new();
    while start >= 2 && &masked[start - 2..start] == "::" {
        let mut p = start - 2;
        while p > 0 && is_ident(b[p - 1]) {
            p -= 1;
        }
        if p == start - 2 {
            return None; // `::seg` from the crate root or after generics
        }
        before.push(masked[p..start - 2].to_string());
        start = p;
    }
    if start > 0 && (is_ident(b[start - 1]) || b[start - 1] == b'>' || b[start - 1] == b'.') {
        return None;
    }
    before.reverse();
    let mut after = Vec::new();
    let mut end = e;
    while masked[end..].starts_with("::") && end + 2 < b.len() && is_ident(b[end + 2]) {
        let mut q = end + 2;
        while q < b.len() && is_ident(b[q]) {
            q += 1;
        }
        after.push(masked[end + 2..q].to_string());
        end = q;
    }
    // A macro path, or one carrying generic arguments: not offered.
    if masked[end..].starts_with("::<") || masked[end..].trim_start().starts_with('!') {
        return None;
    }
    let mut segs = before;
    let at_first = segs.is_empty();
    segs.push(masked[s..e].to_string());
    if at_first {
        segs.extend(after);
    }
    (segs.len() >= 2).then_some(segs)
}

fn starts_at_crate_root(first: &str, ws: &WorkspaceIndex) -> bool {
    matches!(first, "std" | "core" | "alloc" | "crate") || ws.find_definition_by_path(first).is_some_and(|d| d.kind == DefKind::Mod && !d.path.contains("::"))
}

/// Byte offsets of `path` written as a whole path (not the tail of a longer one) outside `use` items.
fn occurrences(masked: &str, path: &str) -> Vec<usize> {
    let b = masked.as_bytes();
    masked
        .match_indices(path)
        .map(|(i, _)| i)
        .filter(|&i| {
            let starts_clean = i == 0 || !(is_ident(b[i - 1]) || (i >= 2 && &masked[i - 2..i] == "::"));
            let after = i + path.len();
            let ends_clean = after >= b.len() || !is_ident(b[after]);
            starts_clean && ends_clean && !masked[after..].starts_with("::<") && !crate::lsp::intelligence::completion_edits::in_use_item(masked, i)
        })
        .collect()
}

/// Whether `name` already means something unqualified in the file (a binding, an item, another import), so that
/// importing it would clash or change what an existing reference names.
fn name_in_use(masked: &str, name: &str, ours: &[usize], path_len: usize) -> bool {
    let b = masked.as_bytes();
    masked.match_indices(name).any(|(i, _)| {
        let end = i + name.len();
        let bounded = (i == 0 || !is_ident(b[i - 1])) && (end >= b.len() || !is_ident(b[end]));
        let in_our_path = ours.iter().any(|&o| i >= o && end <= o + path_len);
        bounded && !in_our_path && !(i >= 2 && &masked[i - 2..i] == "::") && !(i >= 1 && b[i - 1] == b'.')
    })
}

fn imports_exactly(masked: &str, path: &str) -> bool {
    masked.contains(&format!("use {path};"))
}

fn pos(doc: &str, starts: &[usize], byte: usize) -> Value {
    let line = starts.partition_point(|&s| s <= byte) - 1;
    let character: usize = doc[starts[line]..byte].encode_utf16().count();
    json!({ "line": line, "character": character })
}

#[cfg(test)]
#[path = "action_qualified_use_tests.rs"]
mod tests;
