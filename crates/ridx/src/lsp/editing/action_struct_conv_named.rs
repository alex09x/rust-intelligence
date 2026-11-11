/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! `Convert to named struct` through the index: every reference to the struct and to its fields, in any file, is
//! known, so `.0` becomes `.field1`, `Name(a, b)` and `Self(a, b)` (built or matched) become `Name { field1: a, … }`,
//! and the conversion is refused where a reference cannot be rewritten (the constructor used as a function value,
//! a type alias, a path handed to a macro, a literal with `..`).

use super::action_parse::{mask_comments_and_strings, AdtFieldsKind, EnclosingContext, ItemKindInfo, ParsedCursorItem};
use super::action_parse_decl::find_header_terminator;
use super::action_parse_items::skip_balanced;
use super::action_struct_conv::ConversionEdits;
use super::action_struct_conv_attrs::{outer_attrs_allow_shape_change, AttrCtx, Shape};
use super::action_struct_conv_fields::{splice, RawEdit};
use super::action_struct_conv_helpers::{convert_tuple_init_to_brace, parse_tuple_fields_clean, skip_optional_turbofish};
use super::action_struct_conv_index::{byte_to_line_col, line_col_to_byte, lsp_edits, struct_name_byte, Canon};
use super::action_struct_conv_outside::{aliases, is_impl_header};
use ridx_format::WorkspaceIndex;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// A file the conversion edits: the document itself or another one.
struct Work {
    shown: String,
    src: String,
    masked: String,
    starts: Vec<usize>,
    edits: Vec<RawEdit>,
    /// `(edit start, opening parenthesis)` of each `Name(..)` / `Self(..)` to rewrite.
    ctors: Vec<(usize, usize)>,
}

impl Work {
    fn new(src: String, shown: String) -> Self {
        let masked = mask_comments_and_strings(&src);
        let starts = std::iter::once(0).chain(src.match_indices('\n').map(|(i, _)| i + 1)).collect();
        Work { shown, src, masked, starts, edits: Vec::new(), ctors: Vec::new() }
    }
}

/// Byte offset where each tuple field's type starts in the declaration's `( … )` at `open`, in order.
fn tuple_field_starts(masked: &str, open: usize, close: usize) -> Vec<usize> {
    let b = masked.as_bytes();
    let mut out = Vec::new();
    let (mut depth, mut seg) = (0i32, open + 1);
    for k in open + 1..=close {
        let c = if k == close { b',' } else { b[k] };
        let arrow = c == b'>' && b[k - 1] == b'-';
        match c {
            b'(' | b'[' | b'{' | b'<' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b'>' if !arrow => depth -= 1,
            b',' if depth == 0 => {
                let text = &masked[seg..k];
                let lead = text.len() - text.trim_start().len();
                if !text.trim().is_empty() {
                    let t = text.trim_start();
                    let rest = super::action_adt::strip_field_vis(t);
                    out.push(seg + lead + (t.len() - rest.len()) + (rest.len() - rest.trim_start().len()));
                }
                seg = k + 1;
            }
            _ => {}
        }
    }
    out
}

/// Whether the reference at `b` (its name ending at `after`) uses the constructor as a value (`.map(Name)`,
/// `let f = Name;`) rather than naming the type: only a type position is accepted.
fn is_value_use(masked: &str, b: usize, after: usize) -> bool {
    let next = masked[after..].trim_start();
    if next.starts_with("::") || next.starts_with('<') {
        return false;
    }
    // The statement's start, past the groups of an import (`use a::{B, C}`).
    let (mut at, mut lo) = (b, 0);
    while let Some(p) = masked[..at].rfind([';', '{', '}']) {
        if !(masked.as_bytes()[p] == b'{' && masked[..p].trim_end().ends_with("::")) {
            lo = p + 1;
            break;
        }
        at = p;
    }
    let stmt = masked[lo..b].trim_start();
    if stmt.starts_with("use ") || stmt.starts_with("pub use ") || stmt.starts_with("pub(crate) use ") {
        return false;
    }
    if follows_type_prefix(masked, b) {
        return false;
    }
    let prev = path_prefix_stripped(masked, b);
    // An attribute's arguments (`#[require(A, Name)]`) name types.
    let mut at = b;
    while let Some(o) = unmatched_open_at(masked, at) {
        if masked.as_bytes()[o] == b'[' && masked[..o].trim_end().ends_with('#') || masked[..o].trim_end().ends_with("#!") {
            return false;
        }
        at = o;
    }
    if prev.ends_with(',') || prev.ends_with('(') {
        let Some(o) = unmatched_open_at(masked, b) else { return true };
        let before = masked[..o].trim_end();
        return match masked.as_bytes()[o] {
            b'<' => false,
            // An attribute's arguments (`#[require(A, Name)]`) name types.
            b'[' => !before.ends_with('#') && !before.ends_with("#!"),
            // `Fn(Name) -> R`, and a tuple variant's fields (`Variant(Name)` in an `enum` body).
            b'(' => !(["Fn", "FnMut", "FnOnce"].iter().any(|f| before.ends_with(f)) || in_enum_body(masked, o)),
            _ => true,
        };
    }
    true
}

/// The text before `b` with a path qualifier (`a::b::`) left out.
fn path_prefix_stripped(masked: &str, b: usize) -> &str {
    let mut prev = masked[..b].trim_end();
    while let Some(p) = prev.strip_suffix("::") {
        prev = p.trim_end_matches(|c: char| c.is_alphanumeric() || c == '_').trim_end();
    }
    prev
}

/// Whether what precedes `b` makes it a type (`: Name`, `-> Name`, `&Name`, `impl X for Name`).
fn follows_type_prefix(masked: &str, b: usize) -> bool {
    let prev = path_prefix_stripped(masked, b);
    [":", "->", "<", "&", "mut", "as", "for", "impl", "dyn", "+", "'static"].iter().any(|p| prev.ends_with(p)) && !prev.ends_with("::")
}

/// Whether the parenthesis at `o` opens a tuple variant's fields: its enclosing block is an `enum`'s body.
fn in_enum_body(masked: &str, o: usize) -> bool {
    let Some(brace) = unmatched_open_at(masked, o).filter(|&p| masked.as_bytes()[p] == b'{') else { return false };
    let head = &masked[masked[..brace].rfind([';', '{', '}']).map_or(0, |p| p + 1)..brace];
    head.split(|c: char| !(c.is_alphanumeric() || c == '_')).any(|w| w == "enum")
}

/// The innermost bracket (`(`, `[`, `{` or a generic `<`) left open before `b` and its offset; `->` and `=>` are no
/// brackets.
fn unmatched_open_at(masked: &str, b: usize) -> Option<usize> {
    let mb = masked.as_bytes();
    let mut depth = 0i32;
    for i in (0..b).rev() {
        let c = mb[i];
        let arrow = c == b'>' && i > 0 && matches!(mb[i - 1], b'-' | b'=');
        match c {
            b')' | b']' | b'}' => depth += 1,
            b'>' if !arrow => depth += 1,
            b'(' | b'[' | b'{' | b'<' if depth == 0 => return Some(i),
            b'(' | b'[' | b'{' | b'<' => depth -= 1,
            _ => {}
        }
    }
    None
}

pub fn build_convert_to_named_struct_indexed(
    doc_text: &str,
    masked_doc: &str,
    starts: &[usize],
    item: &ParsedCursorItem,
    ws: &WorkspaceIndex,
    file: &str,
) -> Option<ConversionEdits> {
    if !matches!(item.enclosing, EnclosingContext::None) {
        return None;
    }
    let ItemKindInfo::Adt { adt_kw: "struct", where_src, fields_kind: AdtFieldsKind::Tuple(raw_fields), open_delim_byte, end_byte, .. } = &item.kind else {
        return None;
    };
    let ctx = AttrCtx { ws, file, doc: doc_text, starts };
    if !where_src.trim().is_empty() || !outer_attrs_allow_shape_change(doc_text, item.first_attr_byte, item.decl_start_byte, Some(&ctx), Shape::Named) {
        return None;
    }
    let fields = parse_tuple_fields_clean(raw_fields)?;
    let close = skip_balanced(masked_doc.as_bytes(), *open_delim_byte, b'(', b')')? - 1;
    let field_starts = tuple_field_starts(masked_doc, *open_delim_byte, close);
    if fields.is_empty() || fields.len() > 16 || field_starts.len() != fields.len() {
        return None;
    }
    let name_byte = struct_name_byte(masked_doc, item.decl_start_byte, *open_delim_byte, &item.name)?;
    let (nl, nc) = byte_to_line_col(doc_text, starts, name_byte);
    let struct_refs = ws.find_direct_references_at(file, nl, nc).filter(|r| r.target_line == nl)?;

    let mut canon = Canon::default();
    let doc_key = canon.get(std::path::Path::new(file))?;
    let mut works: BTreeMap<PathBuf, Work> = BTreeMap::new();
    works.insert(doc_key.clone(), Work::new(doc_text.to_string(), file.to_string()));
    // A reference's file (the document, or another one read once) and byte offset there.
    let mut locate = |works: &mut BTreeMap<PathBuf, Work>, rfile: &str, resolved: PathBuf, line: u32, col: u32| -> Option<(PathBuf, usize)> {
        let path = if rfile == file { doc_key.clone() } else { canon.get(&resolved)? };
        if !works.contains_key(&path) {
            works.insert(path.clone(), Work::new(std::fs::read_to_string(&path).ok()?, resolved.to_string_lossy().into_owned()));
        }
        let w = &works[&path];
        Some((path.clone(), line_col_to_byte(&w.src, &w.starts, line, col)?))
    };

    // `.N` → `.fieldN+1` at every indexed access of a field.
    for (idx, &fs) in field_starts.iter().enumerate() {
        let (fl, fc) = byte_to_line_col(doc_text, starts, fs);
        // The field's position is also its type's (`Handle` in `S(Handle<Image>)`): the field is found by its path.
        let at_field = ws.find_direct_references_at(file, fl, fc).filter(|r| r.target_line == fl && r.target_kind == ridx_format::DefKind::Field);
        let refs = match at_field {
            Some(r) => r.references,
            None => ws.find_references_for_unit(&format!("{}::{idx}", struct_refs.target_path), struct_refs.target_unit_key),
        };
        let n = idx.to_string();
        for r in &refs {
            let (path, at) = locate(&mut works, r.file, r.resolved_path(), r.span.start_line(), r.span.start_col())?;
            let w = works.get_mut(&path)?;
            // A derive's code (`self.0.clone()` at `Clone`) follows the new shape by itself.
            if r.is_macro_expansion && !w.src[at..].starts_with(&n) {
                continue;
            }
            let before = w.masked[..at].trim_end();
            if !(before.ends_with('.') && !before.ends_with("..")) || !w.src[at..].starts_with(&n) || w.src[at + n.len()..].starts_with(|c: char| c.is_ascii_digit()) {
                return None;
            }
            w.edits.push((at, at + n.len(), format!("field{}", idx + 1)));
        }
    }

    // Constructors and tuple patterns of the struct, and `Self(..)` in its impls.
    for r in &struct_refs.references {
        let (path, b) = locate(&mut works, r.file, r.resolved_path(), r.span.start_line(), r.span.start_col())?;
        let w = works.get_mut(&path)?;
        let (src, m) = (&w.src, &w.masked);
        if (path == doc_key && b == name_byte) || (r.is_macro_expansion && !src[b..].starts_with(item.name.as_str())) {
            continue;
        }
        if aliases(m, b) {
            return None;
        }
        let name_end = b + m[b..].len() - m[b..].trim_start_matches(|c: char| c.is_alphanumeric() || c == '_').len();
        if is_impl_header(m, b) {
            let mb = m.as_bytes();
            let bo = find_header_terminator(mb, name_end).filter(|&p| mb.get(p) == Some(&b'{'))?;
            let bc = skip_balanced(mb, bo, b'{', b'}')?;
            let is_id = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
            for (p, _) in m[bo..bc].match_indices("Self").map(|(p, s)| (bo + p, s)) {
                if is_id(mb[p - 1]) || mb.get(p + 4).is_some_and(|&c| is_id(c)) || m[..p].trim_end().ends_with("::") {
                    continue;
                }
                let k = p + 4 + m[p + 4..].len() - m[p + 4..].trim_start().len();
                match mb.get(k) {
                    Some(b'(') => w.ctors.push((p + 4, k)),
                    Some(b'{') => return None,
                    _ => {}
                }
            }
            continue;
        }
        let (_, after_tf) = skip_optional_turbofish(src, m, name_end);
        let k = after_tf + m[after_tf..].len() - m[after_tf..].trim_start().len();
        match m.as_bytes().get(k) {
            Some(b'(') => w.ctors.push((after_tf, k)),
            // `-> Name {` opens a body; elsewhere a brace after a tuple struct's name is no expression to rewrite.
            Some(b'{') if !follows_type_prefix(m, b) => return None,
            Some(b'{') => {}
            _ if is_value_use(m, b, after_tf) => return None,
            _ => {}
        }
    }

    let mut decl_text = String::new();
    let names: Vec<String> =
        fields.iter().enumerate().map(|(i, (vis, ty))| if vis.is_empty() { format!("field{}: {ty}", i + 1) } else { format!("{vis} field{}: {ty}", i + 1) }).collect();
    decl_text.push_str(&format!(" {{ {} }}", names.join(", ")));
    let mut doc_edits = Vec::new();
    let mut other = BTreeMap::new();
    for (path, mut w) in works {
        if path == doc_key {
            w.edits.push((*open_delim_byte, *end_byte, decl_text.clone()));
        }
        w.ctors.sort_unstable();
        w.ctors.dedup();
        let mut edits: Vec<RawEdit> = Vec::new();
        let mut accesses = std::mem::take(&mut w.edits);
        for (lo, k) in w.ctors.clone() {
            let end = skip_balanced(w.masked.as_bytes(), k, b'(', b')')?;
            let (inner_lo, inner_hi) = (k + 1, end - 1);
            let nested: Vec<RawEdit> = accesses.iter().filter(|e| e.0 >= inner_lo && e.1 <= inner_hi).cloned().collect();
            accesses.retain(|e| !(e.0 >= inner_lo && e.1 <= inner_hi));
            let inner = splice(&w.src[inner_lo..inner_hi], inner_lo, &nested);
            let inner_m = splice(&w.masked[inner_lo..inner_hi], inner_lo, &nested);
            edits.push((lo, end, convert_tuple_init_to_brace(&inner, &inner_m, fields.len())?));
        }
        edits.extend(accesses);
        edits.sort_by_key(|e| e.0);
        // Nested constructors (`Name(Name(x))`) or edits that overlap cannot be applied side by side.
        if edits.windows(2).any(|e| e[1].0 < e[0].1) {
            return None;
        }
        if path == doc_key {
            doc_edits = lsp_edits(&w.src, &edits);
        } else if !edits.is_empty() {
            other.insert(crate::lsp::protocol::path_to_uri(&w.shown), lsp_edits(&w.src, &edits));
        }
    }
    Some((doc_edits, other))
}

/// `Convert to named struct`: through the index when it knows every reference, else within the struct's module.
pub(crate) fn convert_to_named(doc_text: &str, masked_doc: &str, starts: &[usize], item: &ParsedCursorItem, ws: &WorkspaceIndex, file: &str) -> Option<ConversionEdits> {
    // A visible named field needs a doc comment where the crate denies `missing_docs`; a tuple field does not.
    if let ItemKindInfo::Adt { fields_kind: AdtFieldsKind::Tuple(raw), .. } = &item.kind {
        let any_vis = parse_tuple_fields_clean(raw).is_some_and(|f| f.iter().any(|(vis, _)| !vis.is_empty()));
        if any_vis && super::action_crate_checks::crate_denies_missing_docs(masked_doc, file) {
            return None;
        }
    }
    build_convert_to_named_struct_indexed(doc_text, masked_doc, starts, item, ws, file)
        .or_else(|| super::action_struct_conv::build_convert_to_named_struct(doc_text, masked_doc, starts, item).map(|e| (e, BTreeMap::new())))
}

#[cfg(test)]
mod tests {
    use super::{is_value_use, tuple_field_starts};

    #[test]
    fn finds_field_types_and_value_uses() {
        let decl = "struct S(pub usize, Vec<(u8, u8)>);";
        let open = decl.find('(').unwrap();
        let starts = tuple_field_starts(decl, open, decl.rfind(')').unwrap());
        assert_eq!(starts.iter().map(|&s| &decl[s..s + 3]).collect::<Vec<_>>(), vec!["usi", "Vec"]);
        let decl = "struct P<C>(PhantomData<fn() -> C>, u8);";
        let open = decl.find('(').unwrap();
        assert_eq!(tuple_field_starts(decl, open, decl.rfind(')').unwrap()).len(), 2, "`->` closes no generic list");
        let src = "let ids = v.map(Id); fn f(x: &Id) -> Option<Id> { None } use crate::m::{Id, Other}; let h: HashMap<u8, Id>;";
        let at = |n: usize| src.match_indices("Id").nth(n).unwrap().0;
        assert!(is_value_use(src, at(0), at(0) + 2), "a constructor passed as a function");
        assert!(!is_value_use(src, at(1), at(1) + 2) && !is_value_use(src, at(2), at(2) + 2), "types");
        assert!(!is_value_use(src, at(3), at(3) + 2), "an import");
        assert!(!is_value_use(src, at(4), at(4) + 2), "a generic argument");
        let src = "fn f(g: &impl Fn(Id) -> bool) {} #[require(A, Id)] struct C; enum E { V(Id), W } let x = Some(Id);";
        let at = |n: usize| src.match_indices("Id").nth(n).unwrap().0;
        assert!(!is_value_use(src, at(0), at(0) + 2), "a closure type's parameter");
        assert!(!is_value_use(src, at(1), at(1) + 2), "an attribute's type argument");
        assert!(!is_value_use(src, at(2), at(2) + 2), "a tuple variant's field");
        assert!(is_value_use(src, at(3), at(3) + 2), "a constructor as an argument");
    }
}
