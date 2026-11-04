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

use ridx_format::WorkspaceIndex;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};

/// The indexed references to the struct's fields: byte offsets in the converted document, and the `.name` accesses in
/// other files as ready LSP edits to `.N`, keyed by URI.
pub(crate) struct FieldRefs {
    pub sites: HashSet<usize>,
    pub other_files: BTreeMap<String, Vec<Value>>,
}

/// Another file's path as the index names it, its text, and its `.name` edits as `start -> (end, field index)`.
type OtherFile = (String, String, BTreeMap<usize, (usize, usize)>);

/// LSP text edit replacing `src[lo..hi]` with `text`.
fn lsp_edit(src: &str, lo: usize, hi: usize, text: &str) -> Value {
    let pos = |b: usize| {
        let line_start = src[..b].rfind('\n').map_or(0, |p| p + 1);
        json!({ "line": src[..line_start].matches('\n').count(), "character": src[line_start..b].encode_utf16().count() })
    };
    json!({ "range": { "start": pos(lo), "end": pos(hi) }, "newText": text })
}

fn byte_to_line_col(doc: &str, starts: &[usize], byte: usize) -> (u32, u32) {
    let line = starts.partition_point(|&s| s <= byte) - 1;
    (line as u32 + 1, doc[starts[line]..byte].chars().count() as u32 + 1)
}

fn line_col_to_byte(doc: &str, starts: &[usize], line_1b: u32, col_1b: u32) -> Option<usize> {
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
    let mut other: BTreeMap<std::path::PathBuf, OtherFile> = BTreeMap::new();
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
            if !other.contains_key(&path) {
                let src = std::fs::read_to_string(&path).ok()?;
                other.insert(path.clone(), (r.resolved_path().to_string_lossy().into_owned(), src, BTreeMap::new()));
            }
            let (_, src, edits) = other.get_mut(&path)?;
            let s: Vec<usize> = std::iter::once(0).chain(src.match_indices('\n').map(|(i, _)| i + 1)).collect();
            let at = line_col_to_byte(src, &s, r.span.start_line(), r.span.start_col())?;
            // Only `recv.name` is rewritten elsewhere; anything else there (a literal, a pattern) is refused.
            let is_access = src[..at].trim_end().ends_with('.') && !src[..at].trim_end().ends_with("..") && src[at..].starts_with(name);
            if !is_access || src[at + name.len()..].trim_start().starts_with('(') {
                return None;
            }
            edits.insert(at, (at + name.len(), idx));
        }
    }
    let other_files = other
        .into_iter()
        .map(|(_, (shown, src, edits))| {
            let uri = crate::lsp::protocol::path_to_uri(&shown);
            (uri, edits.into_iter().map(|(lo, (hi, idx))| lsp_edit(&src, lo, hi, &idx.to_string())).collect())
        })
        .collect();
    Some(FieldRefs { sites, other_files })
}

/// Whether the struct (its name at `name_byte`) is built, destructured or given an `impl` outside the module text
/// `[mod_lo, mod_hi)` that the conversion rewrites. The index does not record field names inside struct literals, so
/// such sites show up only as references to the struct: one followed by `{` is a place the edits would miss; so is the header of an impl for the struct, followed by `{` too. `None` when the index does
/// not know the struct.
pub(crate) fn struct_shape_used_outside(ws: &WorkspaceIndex, file: &str, doc: &str, starts: &[usize], name_byte: usize, mod_lo: usize, mod_hi: usize) -> Option<bool> {
    let (line, col) = byte_to_line_col(doc, starts, name_byte);
    let refs = ws.find_direct_references_at(file, line, col)?;
    if refs.target_line != line {
        return None;
    }
    let canonical = std::fs::canonicalize(file).ok();
    let mut sources: std::collections::HashMap<std::path::PathBuf, Option<String>> = std::collections::HashMap::new();
    // A literal or pattern (`Name {`), or an impl whose self type is the struct (`impl … for Name {`), where
    // `Self { … }` may build it.
    let shape_site = |text: &str, at: usize| {
        let after = text[at..].trim_start_matches(|c: char| c.is_alphanumeric() || c == '_');
        skip_generic_args(after).trim_start().starts_with('{')
    };
    for r in &refs.references {
        let path = r.resolved_path();
        let same_file = r.file == file || std::fs::canonicalize(&path).ok().is_some_and(|p| Some(&p) == canonical.as_ref());
        let hit = if same_file {
            match line_col_to_byte(doc, starts, r.span.start_line(), r.span.start_col()) {
                Some(b) if (mod_lo..mod_hi).contains(&b) => false,
                Some(b) => shape_site(doc, b),
                None => true,
            }
        } else {
            match sources.entry(path.clone()).or_insert_with(|| std::fs::read_to_string(&path).ok()).as_deref() {
                Some(src) => {
                    let s: Vec<usize> = std::iter::once(0).chain(src.match_indices('\n').map(|(i, _)| i + 1)).collect();
                    line_col_to_byte(src, &s, r.span.start_line(), r.span.start_col()).is_none_or(|b| shape_site(src, b))
                }
                None => true,
            }
        };
        if hit {
            return Some(true);
        }
    }
    Some(false)
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

fn skip_generic_args(s: &str) -> &str {
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
    use super::{field_name_bytes, skip_generic_args, struct_name_byte};

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
