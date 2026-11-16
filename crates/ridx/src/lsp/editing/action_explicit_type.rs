/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! rust-analyzer's `Add explicit type`: on `let name = …` with no annotation, writes the type the inlay hint shows
//! (`let rule: Flat = Flat(10);`). Offered only where the written type surely names the same thing: every name in it
//! is a primitive, a prelude type, or declared or imported at the file's own level; default type parameters are left
//! out as rust-analyzer leaves them (`HashSet<String>`, not `HashSet<String, RandomState>`).

use crate::overlay::OverlayManager;
use ridx_format::WorkspaceIndex;
use serde_json::{json, Value};

const PRIMITIVES: &[&str] = &["bool", "char", "str", "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128", "isize", "f32", "f64", "Self"];
const PRELUDE: &[&str] = &["String", "Vec", "Option", "Result", "Box", "Some", "None", "Ok", "Err"];
/// Default type parameters a written type leaves out.
const DEFAULT_PARAMS: &[&str] = &[", RandomState>", ", std::hash::RandomState>", ", Global>", ", std::alloc::Global>"];

/// The `Add explicit type` action at the cursor of an empty `range` on a `let` without a type, with its edit.
pub fn explicit_type_action(doc: &str, uri: &str, range: &Value, doc_version: Option<i32>, ws: &WorkspaceIndex, ov: &OverlayManager) -> Option<Value> {
    if range["start"] != range["end"] {
        return None;
    }
    let (line0, col) = (range["start"]["line"].as_u64()? as u32, range["start"]["character"].as_u64()? as usize);
    let line = doc.lines().nth(line0 as usize)?;
    let name_end = untyped_let_name_end(line)?;
    if col > name_end || !line.is_char_boundary(name_end) || !line[..name_end].is_ascii() {
        return None;
    }
    let msg = json!({ "params": { "textDocument": { "uri": uri }, "range": { "start": { "line": line0, "character": 0 }, "end": { "line": line0 + 1, "character": 0 } } } });
    let hints = crate::lsp::intelligence::hints::handle_inlay_hint(&msg, Some(ws), ov).ok()?;
    let hint = hints.as_array()?.iter().find(|h| h["kind"] == 1 && h["position"]["line"] == line0 && h["position"]["character"] == name_end as u64)?;
    let label = match &hint["label"] {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts.iter().filter_map(|p| p["value"].as_str()).collect(),
        _ => return None,
    };
    let mut ty = label.strip_prefix(':')?.trim().to_string();
    for d in DEFAULT_PARAMS {
        ty = ty.replace(d, ">");
    }
    if !nameable(&ty, doc) {
        return None;
    }
    let edit = json!({ "range": { "start": { "line": line0, "character": name_end }, "end": { "line": line0, "character": name_end } }, "newText": format!(": {ty}") });
    let mut changes = serde_json::Map::new();
    changes.insert(uri.to_string(), json!([edit]));
    let workspace_edit = match doc_version {
        Some(v) => json!({ "documentChanges": [{ "textDocument": { "uri": uri, "version": v }, "edits": [edit] }] }),
        None => json!({ "changes": changes }),
    };
    Some(json!({ "title": "Add explicit type", "kind": "refactor.rewrite", "edit": workspace_edit }))
}

/// The byte end of the name in `let [mut] name = …` when the binding has no type: `None` for a pattern, an annotation
/// or a `let` with no initializer.
fn untyped_let_name_end(line: &str) -> Option<usize> {
    let indent = line.len() - line.trim_start().len();
    let rest = line[indent..].strip_prefix("let ")?;
    let (skip, rest) = match rest.strip_prefix("mut ") {
        Some(r) => (4, r),
        None => (0, rest),
    };
    let name_len = rest.find(|c: char| !(c.is_alphanumeric() || c == '_')).unwrap_or(rest.len());
    if name_len == 0 || rest.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    let after = rest[name_len..].trim_start();
    (after.starts_with('=') && !after.starts_with("==")).then_some(indent + 4 + skip + name_len)
}

/// Whether every name the type text `ty` uses is seen at the file's own level, so that writing it compiles.
fn nameable(ty: &str, doc: &str) -> bool {
    if ty.is_empty() || ty.contains(['{', '?', '\'']) || ty.contains("impl ") || ty.split(|c: char| !(c.is_alphanumeric() || c == '_')).any(|w| w == "_") {
        return false;
    }
    let mut words = ty.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == ':')).filter(|w| !w.is_empty());
    words.all(|w| {
        let head = w.split("::").next().unwrap_or(w);
        if w.contains("::") {
            return matches!(head, "std" | "core" | "alloc" | "crate");
        }
        PRIMITIVES.contains(&w) || PRELUDE.contains(&w) || matches!(w, "dyn" | "mut" | "fn") || declared_or_imported(doc, w)
    })
}

/// Whether the file declares `name` (a struct, enum, union, trait or type alias) or imports it at its own level.
fn declared_or_imported(doc: &str, name: &str) -> bool {
    let declared = ["struct ", "enum ", "union ", "trait ", "type "]
        .iter()
        .any(|kw| doc.match_indices(&format!("{kw}{name}")).any(|(p, m)| !doc[p + m.len()..].starts_with(|c: char| c.is_alphanumeric() || c == '_')));
    declared || crate::lsp::intelligence::completion_import::name_imported(doc, name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_untyped_let_names() {
        assert_eq!(untyped_let_name_end("    let rule = Flat(10);"), Some(12));
        assert_eq!(untyped_let_name_end("    let mut out = HashSet::new();"), Some(15));
        assert_eq!(untyped_let_name_end("    let rule: Flat = Flat(10);"), None, "already typed");
        assert_eq!(untyped_let_name_end("    let (a, b) = pair;"), None, "a pattern");
        assert_eq!(untyped_let_name_end("    let x;"), None, "no initializer");
    }

    #[test]
    fn test_nameable_types() {
        let doc = "use std::collections::HashSet;\nuse crate::pricing::Flat;\npub struct Cart;\n";
        assert!(nameable("Flat", doc));
        assert!(nameable("HashSet<String>", doc));
        assert!(nameable("Vec<(u8, &str)>", doc));
        assert!(nameable("Option<Box<dyn std::error::Error>>", doc));
        assert!(nameable("Cart", doc));
        assert!(!nameable("HashMap<u8, u8>", doc), "not imported");
        assert!(!nameable("impl Iterator<Item = u8>", doc));
        assert!(!nameable("Vec<_>", doc));
        assert!(!nameable("&'a str", doc));
        assert!(!nameable("{closure}", doc));
    }
}
