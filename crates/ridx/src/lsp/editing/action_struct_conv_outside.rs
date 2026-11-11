/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! `Convert to tuple struct` outside the struct's module. The index's references to the struct find, in other files,
//! the literals and patterns that build or destructure it (`Name { .. }`) and the impls whose `Self { .. }` may; each
//! is rewritten to `Name(..)`. The conversion is refused when a field label there is left outside every rewritten
//! literal, when the struct is aliased (`type A = Name`, whose literals name no reference to it), or when such a site
//! is in the struct's own file but outside its module.

use super::action_parse::mask_comments_and_strings;
use super::action_parse_decl::find_header_terminator;
use super::action_parse_items::{contains_word, skip_balanced};
use super::action_struct_conv_fields::{splice, RawEdit};
use super::action_struct_conv_helpers::{convert_brace_init_to_tuple, is_brace_struct_literal_or_pat, skip_optional_turbofish};
use super::action_struct_conv_index::{line_col_to_byte, lsp_edits, skip_generic_args, Canon, OtherFile};
use ridx_format::LocationReferences;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

/// The struct: its document, the index's references to it, and the module text `[mod_lo, mod_hi)` the in-module
/// conversion rewrites.
pub(crate) struct StructAt<'a> {
    pub file: &'a str,
    pub doc: &'a str,
    pub starts: &'a [usize],
    pub refs: &'a LocationReferences<'a>,
    pub module: (usize, usize),
}

/// Edits for the other files, as LSP edits keyed by URI; `None` when a site cannot be rewritten safely.
pub(crate) fn convert_outside(at: &StructAt, mut files: BTreeMap<PathBuf, OtherFile>, field_index: &HashMap<&str, usize>, n_fields: usize) -> Option<BTreeMap<String, Vec<Value>>> {
    let mut canon_of = Canon::default();
    let canonical = canon_of.get(std::path::Path::new(at.file));
    let doc_masked = mask_comments_and_strings(at.doc);
    let mut masked: HashMap<PathBuf, String> = HashMap::new();
    // Per file, the literals to rewrite as `(edit start, opening brace)`.
    let mut literals: BTreeMap<PathBuf, Vec<(usize, usize)>> = BTreeMap::new();
    for r in &at.refs.references {
        let path = r.resolved_path();
        let canon = if r.file == at.file { canonical.clone() } else { canon_of.get(&path) };
        if r.file == at.file || (canon.is_some() && canon == canonical) {
            let b = line_col_to_byte(at.doc, at.starts, r.span.start_line(), r.span.start_col())?;
            if (at.module.0..at.module.1).contains(&b) {
                if aliases(&doc_masked, b) {
                    return None;
                }
                continue;
            }
            if aliases(&doc_masked, b) || !shape_sites(at.doc, &doc_masked, b).is_empty() || is_impl_header(&doc_masked, b) {
                return None;
            }
            continue;
        }
        let canon = canon?;
        if !files.contains_key(&canon) {
            files.insert(canon.clone(), OtherFile::read(&canon, path.to_string_lossy().into_owned())?);
        }
        let of = &files[&canon];
        let m = masked.entry(canon.clone()).or_insert_with(|| mask_comments_and_strings(&of.src));
        let b = line_col_to_byte(&of.src, &of.starts, r.span.start_line(), r.span.start_col())?;
        if aliases(m, b) {
            return None;
        }
        literals.entry(canon).or_default().extend(shape_sites(&of.src, m, b));
    }
    let mut out = BTreeMap::new();
    for (path, of) in files {
        let m = masked.remove(&path).unwrap_or_else(|| mask_comments_and_strings(&of.src));
        let mut lits = literals.remove(&path).unwrap_or_default();
        lits.sort_unstable();
        lits.dedup();
        let mut accesses = of.accesses;
        let mut edits: Vec<RawEdit> = Vec::new();
        for (lo, k) in lits {
            let end = skip_balanced(m.as_bytes(), k, b'{', b'}')?;
            if edits.iter().any(|e| lo < e.1 && e.0 < end) {
                return None;
            }
            let (inner_lo, inner_hi) = (k + 1, end - 1);
            let nested: Vec<RawEdit> = accesses.iter().filter(|e| e.0 >= inner_lo && e.1 <= inner_hi).cloned().collect();
            accesses.retain(|e| !(e.0 >= inner_lo && e.1 <= inner_hi));
            let inner = splice(&of.src[inner_lo..inner_hi], inner_lo, &nested);
            let inner_m = splice(&m[inner_lo..inner_hi], inner_lo, &nested);
            edits.push((lo, end, convert_brace_init_to_tuple(&inner, &inner_m, n_fields, field_index)?));
        }
        if !of.labels.iter().all(|&l| edits.iter().any(|e| l >= e.0 && l < e.1)) {
            return None;
        }
        edits.extend(accesses);
        edits.sort_by_key(|e| e.0);
        if edits.windows(2).any(|w| w[1].0 < w[0].1) {
            return None;
        }
        if !edits.is_empty() {
            out.insert(crate::lsp::protocol::path_to_uri(&of.shown), lsp_edits(&of.src, &edits));
        }
    }
    Some(out)
}

/// Whether the struct cannot be followed from the reference at `b`: the target of a free type alias
/// (`type A<T> = Name<T>;`, whose literals refer to the alias), or a bare path handed to a macro
/// (`from_id![(Id, crate::Name)]`, which may build it out of sight). An associated type in an impl
/// (`type Map = Name<E>;`) is not an alias here: literals through it (`Self::Map { .. }`) refer to the struct.
pub(crate) fn aliases(masked: &str, b: usize) -> bool {
    let name_end = b + masked[b..].len() - masked[b..].trim_start_matches(|c: char| c.is_alphanumeric() || c == '_').len();
    let after = skip_generic_args(&masked[name_end..]).trim_start();
    if after.starts_with([',', ')', ']']) && in_macro_call(masked, b) {
        return true;
    }
    let before = masked[..b].trim_end();
    let before = before.strip_suffix("::").map_or(before, |p| p.trim_end_matches(|c: char| c.is_alphanumeric() || c == '_' || c == ':'));
    let Some(head) = before.trim_end().strip_suffix('=') else { return false };
    let stmt_lo = head.rfind([';', '{', '}']).map_or(0, |p| p + 1);
    if !contains_word(&head[stmt_lo..], "type") {
        return false;
    }
    // The block the alias sits in: an impl or trait body makes it an associated type.
    match enclosing_open(masked, stmt_lo) {
        Some(open) => {
            let header = &masked[masked[..open].rfind([';', '{', '}']).map_or(0, |p| p + 1)..open];
            !(contains_word(header, "impl") || contains_word(header, "trait"))
        }
        None => true,
    }
}

/// The unmatched `{` (or `(`/`[`) that encloses byte `b`, if any.
fn enclosing_open(masked: &str, b: usize) -> Option<usize> {
    let mb = masked.as_bytes();
    let mut depth = 0i32;
    let mut i = b;
    while i > 0 {
        i -= 1;
        match mb[i] {
            b'}' | b')' | b']' => depth += 1,
            b'{' | b'(' | b'[' if depth == 0 => return Some(i),
            b'{' | b'(' | b'[' => depth -= 1,
            _ => {}
        }
    }
    None
}

/// Whether byte `b` lies in the arguments of a macro call (`name!(..)`, `name![..]`, `name! {..}`).
fn in_macro_call(masked: &str, b: usize) -> bool {
    let mut at = b;
    while let Some(open) = enclosing_open(masked, at) {
        if masked[..open].trim_end().ends_with('!') {
            return true;
        }
        at = open;
    }
    false
}

/// Whether the reference at `b` names the self type of an impl header (`impl Trait for Name<T> where … {`).
pub(crate) fn is_impl_header(masked: &str, b: usize) -> bool {
    let stmt_lo = masked[..b].rfind([';', '{', '}']).map_or(0, |p| p + 1);
    let head = &masked[stmt_lo..b];
    // The self type is the last type of the header: `{` or `where` follows it (not `>` of `impl From<Name> for X`).
    let name_end = b + masked[b..].len() - masked[b..].trim_start_matches(|c: char| c.is_alphanumeric() || c == '_').len();
    let rest = skip_generic_args(&masked[name_end..]).trim_start();
    let ends_header = rest.starts_with('{') || (rest.starts_with("where") && !rest[5..].starts_with(|c: char| c.is_alphanumeric() || c == '_'));
    ends_header && contains_word(head, "impl") && !head.contains("->") && !contains_word(head, "fn")
}

/// The literals or patterns to rewrite for the reference at `b`: itself when it builds or destructures the struct,
/// or each `Self { .. }` in the impl it heads, as `(edit start, opening brace)`.
fn shape_sites(src: &str, masked: &str, b: usize) -> Vec<(usize, usize)> {
    let mb = masked.as_bytes();
    let name_end = b + masked[b..].len() - masked[b..].trim_start_matches(|c: char| c.is_alphanumeric() || c == '_').len();
    if is_impl_header(masked, b) {
        let Some(bo) = find_header_terminator(mb, name_end).filter(|&p| mb.get(p) == Some(&b'{')) else { return Vec::new() };
        let Some(bc) = skip_balanced(mb, bo, b'{', b'}') else { return Vec::new() };
        return self_literals(src, masked, bo + 1, bc - 1);
    }
    let (_, after_tf) = skip_optional_turbofish(src, masked, name_end);
    let k = masked.len() - masked[after_tf..].trim_start().len();
    if mb.get(k) == Some(&b'{') && is_brace_struct_literal_or_pat(masked, 0, b, k, &[]) {
        vec![(after_tf, k)]
    } else {
        Vec::new()
    }
}

/// `Self { .. }` literals and patterns in the impl body `[lo, hi)`.
fn self_literals(src: &str, masked: &str, lo: usize, hi: usize) -> Vec<(usize, usize)> {
    let mb = masked.as_bytes();
    let is_id = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    masked[lo..hi]
        .match_indices("Self")
        .map(|(p, _)| lo + p)
        .filter(|&p| !is_id(mb[p - 1]) && mb.get(p + 4).is_some_and(|&c| !is_id(c)) && !masked[..p].trim_end().ends_with("::"))
        .filter_map(|p| {
            let (_, after_tf) = skip_optional_turbofish(src, masked, p + 4);
            let k = after_tf + masked[after_tf..].len() - masked[after_tf..].trim_start().len();
            (mb.get(k) == Some(&b'{') && is_brace_struct_literal_or_pat(masked, lo, p, k, &[])).then_some((after_tf, k))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{aliases, is_impl_header, shape_sites};

    #[test]
    fn test_finds_literals_patterns_and_self_literals_of_impls() {
        let s = "fn f() -> Name { let n = Name { a: 1 }; let Name { a } = n; m::Name::<u8> { a } }\nimpl<T> Tr for Name<T> where T: Copy { fn g() -> Self { Self { a: 0 } } }";
        let at = |needle: &str, nth: usize| s.match_indices(needle).nth(nth).unwrap().0;
        assert!(shape_sites(s, s, at("Name", 0)).is_empty(), "a return type is not a literal");
        assert_eq!(shape_sites(s, s, at("Name", 1)), vec![(at("Name", 1) + 4, at("Name", 1) + 5)]);
        assert_eq!(shape_sites(s, s, at("Name", 2)).len(), 1);
        assert_eq!(shape_sites(s, s, at("Name", 3)), vec![(at("Name", 3) + 10, at("Name", 3) + 11)]);
        let imp = at("Name", 4);
        assert!(is_impl_header(s, imp));
        assert_eq!(shape_sites(s, s, imp), vec![(s.rfind("Self {").unwrap() + 4, s.rfind("Self {").unwrap() + 5)]);
    }

    #[test]
    fn test_detects_type_aliases() {
        let s = "type A<T> = crate::m::Name<T>; let x: Name = y;";
        assert!(aliases(s, s.find("Name").unwrap()));
        assert!(!aliases(s, s.rfind("Name").unwrap()));
        let s = "impl<E> Ser for Name<E> { type Map = Name<E>; }";
        assert!(!aliases(s, s.rfind("Name").unwrap()), "an associated type is not an alias");
        let s = "from_id![(hir_def::UnionId, crate::Name), (A, B)]; f(Name::new(), x);";
        assert!(aliases(s, s.find("Name").unwrap()), "a bare path handed to a macro");
        assert!(!aliases(s, s.rfind("Name").unwrap()));
    }
}
