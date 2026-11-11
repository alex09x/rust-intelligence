/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! The index's view of a struct's fields for `Convert to tuple struct`: every place the compiler resolved to each
//! field. A `.name` access is rewritten only where the index puts a reference to this struct's field, and the
//! conversion is offered only when every such reference lies inside one of its edits.

use super::action_struct_conv_fields::RawEdit;
use ridx_format::WorkspaceIndex;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

/// The indexed references to the struct's fields: byte offsets in the converted document, and those in other files
/// keyed by canonical path.
pub(crate) struct FieldRefs {
    pub sites: HashSet<usize>,
    pub other_files: BTreeMap<PathBuf, OtherFile>,
}

/// Another file the conversion edits: its path as the index names it (how the client knows it), its text, the
/// `.name` → `.N` edits, and the field references that are not accesses (labels in literals and patterns), which the
/// conversion of those literals must cover.
pub(crate) struct OtherFile {
    pub shown: String,
    pub src: String,
    pub accesses: Vec<RawEdit>,
    pub labels: Vec<usize>,
}

impl OtherFile {
    /// Line start offsets of the text.
    pub fn starts(&self) -> Vec<usize> {
        std::iter::once(0).chain(self.src.match_indices('\n').map(|(i, _)| i + 1)).collect()
    }
}

/// An LSP edit list for `edits` on `src`.
pub(crate) fn lsp_edits(src: &str, edits: &[RawEdit]) -> Vec<Value> {
    edits.iter().map(|(lo, hi, text)| lsp_edit(src, *lo, *hi, text)).collect()
}

/// Whether the receiver text before a `.name` (`text` ends right before that `.`) ends in a tuple index (`w.0`):
/// the field may be reached through `Deref` on a tuple struct, whose own `.N` a positional rewrite would hit.
pub(crate) fn receiver_ends_in_tuple_index(text: &str) -> bool {
    let t = text.trim_end();
    let digits = t.len() - t.trim_end_matches(|c: char| c.is_ascii_digit()).len();
    digits > 0 && t[..t.len() - digits].trim_end().ends_with('.')
}

/// LSP text edit replacing `src[lo..hi]` with `text`.
fn lsp_edit(src: &str, lo: usize, hi: usize, text: &str) -> Value {
    let pos = |b: usize| {
        let line_start = src[..b].rfind('\n').map_or(0, |p| p + 1);
        json!({ "line": src[..line_start].matches('\n').count(), "character": src[line_start..b].encode_utf16().count() })
    };
    json!({ "range": { "start": pos(lo), "end": pos(hi) }, "newText": text })
}

pub(crate) fn byte_to_line_col(doc: &str, starts: &[usize], byte: usize) -> (u32, u32) {
    let line = starts.partition_point(|&s| s <= byte) - 1;
    (line as u32 + 1, doc[starts[line]..byte].chars().count() as u32 + 1)
}

pub(crate) fn line_col_to_byte(doc: &str, starts: &[usize], line_1b: u32, col_1b: u32) -> Option<usize> {
    let start = *starts.get(line_1b.checked_sub(1)? as usize)?;
    let line = doc[start..].split('\n').next().unwrap_or("");
    let n = col_1b.checked_sub(1)? as usize;
    if n == line.chars().count() {
        return Some(start + line.len());
    }
    line.char_indices().nth(n).map(|(b, _)| start + b)
}

/// References to the fields whose names start at `field_bytes`, or `None` when the index cannot vouch for them:
/// a field it does not know, or a reference in another file that is not a plain `.name` access.
pub(crate) fn indexed_field_refs(ws: &WorkspaceIndex, file: &str, doc: &str, starts: &[usize], field_bytes: &[usize]) -> Option<FieldRefs> {
    let mut sites = HashSet::new();
    let mut other: BTreeMap<PathBuf, OtherFile> = BTreeMap::new();
    let canonical = std::fs::canonicalize(file).ok();
    for (idx, &fb) in field_bytes.iter().enumerate() {
        let (line, col) = byte_to_line_col(doc, starts, fb);
        let refs = ws.find_direct_references_at(file, line, col)?;
        if refs.target_line != line {
            return None;
        }
        let name_len = doc[fb..].find(|c: char| !(c.is_alphanumeric() || c == '_')).unwrap_or(doc.len() - fb);
        let name = &doc[fb..fb + name_len];
        for r in &refs.references {
            let canon = std::fs::canonicalize(r.resolved_path()).ok();
            if r.file == file || (canon.is_some() && canon == canonical) {
                sites.insert(line_col_to_byte(doc, starts, r.span.start_line(), r.span.start_col())?);
                continue;
            }
            // Keyed by the canonical path (one file reached through two crates is one file), named by the path the
            // index stores, which is how the client knows it.
            let path = canon?;
            let of = match other.entry(path) {
                std::collections::btree_map::Entry::Occupied(e) => e.into_mut(),
                std::collections::btree_map::Entry::Vacant(e) => {
                    let src = std::fs::read_to_string(e.key()).ok()?;
                    e.insert(OtherFile { shown: r.resolved_path().to_string_lossy().into_owned(), src, accesses: Vec::new(), labels: Vec::new() })
                }
            };
            let s = of.starts();
            let at = line_col_to_byte(&of.src, &s, r.span.start_line(), r.span.start_col())?;
            let src = &of.src;
            let before = src[..at].trim_end();
            if !src[at..].starts_with(name) {
                return None;
            }
            if !(before.ends_with('.') && !before.ends_with("..")) {
                // A label in a literal or pattern: rewritten with that literal, or the conversion is refused.
                of.labels.push(at);
                continue;
            }
            if src[at + name.len()..].trim_start().starts_with('(') || receiver_ends_in_tuple_index(&before[..before.len() - 1]) {
                return None;
            }
            if !of.accesses.iter().any(|e| e.0 == at) {
                of.accesses.push((at, at + name.len(), idx.to_string()));
            }
        }
    }
    Some(FieldRefs { sites, other_files: other })
}

/// Byte offsets, in this document, of the index's references to the struct (its name at `name_byte`).
pub(crate) fn struct_ref_sites(ws: &WorkspaceIndex, file: &str, doc: &str, starts: &[usize], name_byte: usize) -> Option<HashSet<usize>> {
    let (line, col) = byte_to_line_col(doc, starts, name_byte);
    let refs = ws.find_direct_references_at(file, line, col)?;
    let canonical = std::fs::canonicalize(file).ok();
    Some(
        refs.references
            .iter()
            .filter(|r| r.file == file || std::fs::canonicalize(r.resolved_path()).ok().is_some_and(|p| Some(&p) == canonical.as_ref()))
            .filter_map(|r| line_col_to_byte(doc, starts, r.span.start_line(), r.span.start_col()))
            .collect(),
    )
}

/// Byte offset of the struct's name in its declaration `[decl_start, open_delim)`.
pub(crate) fn struct_name_byte(masked: &str, decl_start: usize, open_delim: usize, name: &str) -> Option<usize> {
    let b = masked.as_bytes();
    let is_id = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let kw = decl_start + masked[decl_start..open_delim].find("struct")?;
    masked[kw + 6..open_delim].match_indices(name).map(|(p, _)| kw + 6 + p).find(|&p| !is_id(b[p - 1]) && b.get(p + name.len()).is_none_or(|c| !is_id(*c)))
}

pub(crate) fn skip_generic_args(s: &str) -> &str {
    let t = s.trim_start();
    let t = t.strip_prefix("::").unwrap_or(t);
    if !t.starts_with('<') {
        return s;
    }
    let mut depth = 0i32;
    for (i, c) in t.char_indices() {
        match c {
            '<' => depth += 1,
            '>' => {
                depth -= 1;
                if depth == 0 {
                    return &t[i + 1..];
                }
            }
            _ => {}
        }
    }
    s
}

/// Byte offset of each field's name inside the struct body `body` (starting at `body_start`), in declaration order.
pub(crate) fn field_name_bytes(masked_body: &str, body_start: usize, names: &[&str]) -> Option<Vec<usize>> {
    let b = masked_body.as_bytes();
    let is_id = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut from = 0;
    let mut out = Vec::with_capacity(names.len());
    for name in names {
        let rel = masked_body[from..].match_indices(name).map(|(p, _)| from + p).find(|&p| {
            (p == 0 || !is_id(b[p - 1])) && b.get(p + name.len()).is_none_or(|c| !is_id(*c)) && {
                let rest = masked_body[p + name.len()..].trim_start();
                rest.starts_with(':') && !rest.starts_with("::")
            }
        })?;
        out.push(body_start + rel);
        from = rel + name.len();
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::{field_name_bytes, receiver_ends_in_tuple_index, skip_generic_args, struct_name_byte};

    #[test]
    fn test_receiver_through_a_tuple_field_is_detected() {
        assert!(receiver_ends_in_tuple_index("        self.0"), "w.0.name may reach the field through Deref");
        assert!(receiver_ends_in_tuple_index("self.state\n            .0\n            "));
        assert!(!receiver_ends_in_tuple_index("self"));
        assert!(!receiver_ends_in_tuple_index("inner"));
        assert!(!receiver_ends_in_tuple_index("items[0]"));
    }

    #[test]
    fn test_locates_struct_and_field_names_and_skips_generic_arguments() {
        let src = "pub struct Wrap<T: Clone> { pub(crate) inner: T, count: usize }";
        let open = src.find('{').unwrap();
        assert_eq!(struct_name_byte(src, 0, open, "Wrap"), Some(11));
        let body = &src[open..];
        assert_eq!(field_name_bytes(body, open, &["inner", "count"]), Some(vec![src.find("inner").unwrap(), src.find("count").unwrap()]));
        assert_eq!(field_name_bytes(body, open, &["missing"]), None);
        assert_eq!(skip_generic_args("::<'a, Vec<u8>> {").trim_start(), "{");
        assert_eq!(skip_generic_args(" {"), " {");
    }
}
