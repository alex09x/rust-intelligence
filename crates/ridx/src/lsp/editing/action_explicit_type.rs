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
    // The hint cuts a long type with `…`; the action writes it whole, as rust-analyzer does.
    if ty.contains('…') {
        let name_start = line[..name_end].rfind(|c: char| !(c.is_alphanumeric() || c == '_')).map_or(0, |i| i + 1);
        let file = crate::lsp::protocol::uri_to_path(uri);
        ty = crate::lsp::intelligence::inlay::binding_type_full(ws, ov, &file, line0 + 1, name_start as u32 + 1, name_end as u32 + 1)?;
    }
    for d in DEFAULT_PARAMS {
        ty = ty.replace(d, ">");
    }
    if !nameable(&ty, doc, line0 as usize) {
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

/// Whether every name the type text `ty` uses is seen on line `line0` (0-based) of `doc`, so that writing it there
/// compiles: declared or imported at the file's own level, or imported in a block around the line.
fn nameable(ty: &str, doc: &str, line0: usize) -> bool {
    // `…` is the inlay hint's truncation of a long type: never text to write.
    if ty.is_empty() || ty.contains(['{', '?', '\'', '…']) || ty.contains("impl ") || ty.split(|c: char| !(c.is_alphanumeric() || c == '_')).any(|w| w == "_") {
        return false;
    }
    let mut words = ty.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == ':')).filter(|w| !w.is_empty());
    words.all(|w| {
        let head = w.split("::").next().unwrap_or(w);
        if w.contains("::") {
            return matches!(head, "std" | "core" | "alloc" | "crate" | "Self");
        }
        w.starts_with(|c: char| c.is_ascii_digit()) // an array length
            || PRIMITIVES.contains(&w)
            || PRELUDE.contains(&w)
            || matches!(w, "dyn" | "mut" | "fn" | "const")
            || declared_or_imported(doc, w)
            || imported_around(doc, line0, w)
    })
}

/// Whether a `use` in a block enclosing line `line0` of `doc` (a `mod tests`, a function) brings `name` in: in Zed,
/// `HashSet` imported in `mod tests` got no `Add explicit type` on a `let` in a test there.
fn imported_around(doc: &str, line0: usize, name: &str) -> bool {
    let masked = crate::lsp::editing::action_parse::mask_comments_and_strings(doc);
    let b = masked.as_bytes();
    let at: usize = doc.split_inclusive('\n').take(line0).map(str::len).sum();
    // Whether the block opened at `open` is still open at byte `at`.
    let encloses = |open: usize| {
        let mut depth = 0usize;
        for (i, &c) in b.iter().enumerate().skip(open) {
            match c {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return i > at;
                    }
                }
                _ => {}
            }
        }
        true
    };
    let mut open_braces: Vec<usize> = Vec::new();
    let mut p = 0;
    while p < b.len() {
        match b[p] {
            b'{' => open_braces.push(p),
            b'}' => {
                open_braces.pop();
            }
            b'u' if b[p..].starts_with(b"use ") && (p == 0 || !(b[p - 1].is_ascii_alphanumeric() || b[p - 1] == b'_')) => {
                if let Some(end) = masked[p..].find(';').map(|i| p + i) {
                    let in_scope = open_braces.last().is_some_and(|&open| open < at && encloses(open));
                    if in_scope && crate::lsp::intelligence::use_tree::use_leaves(&masked[p + 4..end]).iter().any(|l| l.name.as_deref() == Some(name)) {
                        return true;
                    }
                    p = end;
                }
            }
            _ => {}
        }
        p += 1;
    }
    false
}

/// Whether the file declares `name` (a struct, enum, union, trait or type alias) or imports it, at its own level: an
/// item starting at column 0, so neither a comment, a string nor an item of `mod tests` or of a function counts.
fn declared_or_imported(doc: &str, name: &str) -> bool {
    let declared = doc.lines().any(|line| {
        let item = ["pub(crate) ", "pub "].iter().find_map(|v| line.strip_prefix(v)).unwrap_or(line);
        ["struct ", "enum ", "union ", "trait ", "type "]
            .iter()
            .any(|kw| item.strip_prefix(kw).and_then(|rest| rest.strip_prefix(name)).is_some_and(|after| !after.starts_with(|c: char| c.is_alphanumeric() || c == '_')))
    });
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
        assert!(nameable("Flat", doc, 0));
        assert!(nameable("HashSet<String>", doc, 0));
        assert!(nameable("Vec<(u8, &str)>", doc, 0));
        assert!(nameable("Option<Box<dyn std::error::Error>>", doc, 0));
        assert!(nameable("Cart", doc, 0));
        assert!(!nameable("HashMap<u8, u8>", doc, 0), "not imported");
        assert!(!nameable("impl Iterator<Item = u8>", doc, 0));
        assert!(!nameable("Vec<_>", doc, 0));
        assert!(!nameable("&'a str", doc, 0));
        assert!(!nameable("{closure}", doc, 0));
        assert!(!nameable("Result<Option<String>, …>", doc, 0), "a truncated hint");
        assert!(!nameable("(String, String, String, …)", doc, 0));
        assert!(nameable("[u8; 32]", doc, 0), "an array length");
        assert!(nameable("*const u8", doc, 0));
        assert!(nameable("Self::Item", doc, 0));
        let nested = "// struct Commented;\nconst S: &str = \"struct InString\";\nmod tests {\n    struct Inner;\n}\nfn f() {\n    struct Local;\n}\npub(crate) struct Top;\n";
        assert!(!nameable("Commented", nested, 0), "a comment declares nothing");
        assert!(!nameable("InString", nested, 0), "nor a string");
        assert!(!nameable("Inner", nested, 0), "an item of `mod tests` is not seen outside it");
        assert!(!nameable("Local", nested, 0), "nor an item of a function");
        assert!(nameable("Top", nested, 0));
    }

    /// In Zed: `HashSet` imported in `mod tests` and a `let` in a test there got no `Add explicit type`.
    #[test]
    fn test_types_imported_in_a_block_around_the_line() {
        let doc = "pub struct Cart;\n#[cfg(test)]\nmod tests {\n    use std::collections::{HashMap, HashSet};\n    #[test]\n    fn t() {\n        use std::rc::Rc;\n        let tags = HashSet::<u8>::new();\n    }\n}\nfn outside() {\n    let x = 1;\n}\n";
        let at = |needle: &str| doc.lines().position(|l| l.contains(needle)).unwrap();
        assert!(nameable("HashSet<u8>", doc, at("let tags")), "the module's import");
        assert!(nameable("HashMap<u8, Rc<Cart>>", doc, at("let tags")), "and the function's own");
        assert!(!nameable("HashSet<u8>", doc, at("let x")), "not seen outside the module");
        assert!(!nameable("Rc<u8>", doc, at("let x")));
        let commented = "mod tests {\n    // use std::collections::HashSet;\n    fn t() {\n        let tags = 1;\n    }\n}\n";
        assert!(!nameable("HashSet<u8>", commented, 3), "a commented-out import brings nothing");
    }
}
